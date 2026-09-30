use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::Duration;

use dasdevbot_core::{decide, kind, EffectClass, GateInput, Policy};
use dasdevbot_proto::EmitRequest;
use serde_json::{json, Value};

use crate::provider::{
    CompletionRequest, ProviderError, RetryCost, OLLAMA_BUSY_MAX_MS, RESERVE_TOKENS,
};
use crate::store::{AppendOutcome, Job, NewApproval, Store};
use crate::{wall_ms, App, Error, Result};

const LEASE_MS: u64 = 120_000;

pub struct Ingested {
    pub created: bool,
    pub event_id: String,
    pub thread_id: String,
    pub jobs: Vec<String>,
}

pub fn spawn_worker(app: std::sync::Arc<App>, rx: mpsc::Receiver<()>) -> JoinHandle<()> {
    std::thread::spawn(move || worker_loop(app, rx))
}

fn worker_loop(app: std::sync::Arc<App>, rx: mpsc::Receiver<()>) {
    // Claim until the queue is empty, then park. A short timeout also sweeps
    // approval expiry and the undo-window commit point while the queue is idle.
    // The channel buffers a wake that arrives between the empty claim and recv,
    // so a notification cannot be lost.
    loop {
        sweep_approvals(&app);
        loop {
            let job = {
                let store = app.store.lock().expect("store");
                match store.claim_at(&app.worker_id, wall_ms(), LEASE_MS) {
                    Ok(job) => job,
                    Err(err) => {
                        eprintln!("dasdevbotd claim: {err}");
                        None
                    }
                }
            };
            let Some(job) = job else {
                break;
            };
            eprintln!("dasdevbotd turn agent={} job={}", job.agent_id, job.id);
            if let Err(err) = run_turn(&app, &job) {
                record_turn_failure(&app, &job, &err);
            }
        }
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(()) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn record_turn_failure(app: &App, job: &Job, err: &Error) {
    eprintln!("dasdevbotd turn failed: {err}");
    let mut store = app.store.lock().expect("store");
    let payload = json!({"job_id": job.id, "error": err.to_string()}).to_string();
    let key = format!("job-failed:{}:{}", job.id, job.attempt);
    let _ = store.append_at(wall_ms(), "runtime", kind::JOB_FAILED, &payload, &key, None);
    let _ = store.fail_leased(&job.id, &app.worker_id);
}

fn sweep_approvals(app: &App) {
    let mut store = app.store.lock().expect("store");
    if let Err(err) = store.sweep(wall_ms()) {
        eprintln!("dasdevbotd sweep: {err}");
    }
}

pub fn ingest(store: &mut Store, request: &EmitRequest, now_ms: u64) -> Result<Ingested> {
    let payload = serde_json::to_string(&request.payload)
        .map_err(|err| Error::BadRequest(err.to_string()))?;
    let key = request
        .idempotency_key
        .clone()
        .filter(|key| !key.trim().is_empty())
        .unwrap_or_else(|| format!("event-{}", uuid::Uuid::new_v4()));
    match store.append_at(now_ms, &request.source, &request.kind, &payload, &key, None)? {
        AppendOutcome::Replay(event) => {
            let jobs = store.job_ids_for_event_key(&event.idempotency_key)?;
            Ok(Ingested {
                created: false,
                event_id: event.id,
                thread_id: event.thread_id,
                jobs,
            })
        }
        AppendOutcome::Created(event) => {
            let mut jobs = Vec::new();
            for agent_id in store.rules_for_kind(&event.kind)? {
                let job_key = format!("job:{}:{agent_id}", event.idempotency_key);
                let job_payload = json!({
                    "event_id": event.id,
                    "thread_id": event.thread_id,
                    "kind": event.kind,
                    "tainted": event.kind == kind::REPO_PUSH,
                    "payload": request.payload,
                })
                .to_string();
                if let Some(id) = store.enqueue_job(&agent_id, &job_key, &job_payload, now_ms)? {
                    jobs.push(id);
                }
            }
            Ok(Ingested {
                created: true,
                event_id: event.id,
                thread_id: event.thread_id,
                jobs,
            })
        }
    }
}

fn run_turn(app: &App, job: &Job) -> Result<()> {
    let prepared = {
        let mut store = app.store.lock().expect("store");
        prepare(&mut store, app, job)?
    };
    let Some(prepared) = prepared else {
        return Ok(());
    };
    let completion = match app.provider.complete(&CompletionRequest {
        model: String::new(),
        system: prepared.persona,
        user: prepared.user_message,
        max_tokens: 512,
    }) {
        Ok(completion) => completion,
        Err(err) => {
            let mut store = app.store.lock().expect("store");
            return match settle_turn_error(&mut store, job, &app.worker_id, &err, wall_ms())? {
                TurnErrorAction::Pause { .. } => Ok(()),
                TurnErrorAction::Fail => Err(Error::Provider(err)),
            };
        }
    };
    let mut store = app.store.lock().expect("store");
    if !store.heartbeat_at(&job.id, &app.worker_id, wall_ms(), LEASE_MS)? {
        return Ok(());
    }
    charge_retry_costs(
        &mut store,
        &prepared.agent_id,
        job,
        &completion.retry_costs,
        wall_ms(),
    )?;
    let class = if prepared.forced {
        EffectClass::Destructive
    } else {
        EffectClass::External
    };
    let grant = store.has_grant(&prepared.agent_id, class.as_str())?;
    let outcome = decide(
        GateInput {
            grant,
            class,
            budget_remaining: true,
            tainted: prepared.tainted,
        },
        Policy::phase0(),
    );
    if outcome != dasdevbot_core::GateOutcome::Ask {
        let payload = json!({
            "job_id": job.id,
            "effect_class": class.as_str(),
            "outcome": "deny",
            "tainted": prepared.tainted,
        })
        .to_string();
        store.append_at(
            wall_ms(),
            "runtime",
            kind::JOB_FAILED,
            &payload,
            &format!("gate-deny:{}", job.id),
            Some(&prepared.thread_id),
        )?;
        store.add_spend(
            &prepared.agent_id,
            tokens_i64(completion.input_tokens, completion.output_tokens),
        )?;
        store.fail_leased(&job.id, &app.worker_id)?;
        return Ok(());
    }
    let action = if prepared.forced {
        "force_push"
    } else {
        "post_pr_comment"
    };
    let purpose = if prepared.forced {
        format!(
            "Force-push {}. This rewrites the remote branch.",
            prepared.evidence_ref
        )
    } else {
        prepared.purpose.clone()
    };
    let draft = if prepared.forced {
        format!("git push --force origin {}", prepared.evidence_ref)
    } else {
        completion.text.clone()
    };
    let request_payload = json!({
        "job_id": job.id,
        "action": action,
        "effect_class": class.as_str(),
        "tainted": prepared.tainted,
        "provider": completion.provider,
    })
    .to_string();
    let ledger_payload = json!({
        "provider": completion.provider,
        "model": completion.model,
        "usage_kind": completion.usage_kind,
        "input_tokens": completion.input_tokens,
        "output_tokens": completion.output_tokens,
        "micro_usd": completion.micro_usd,
        "note": completion.note,
        "quota": completion.usage.quota_detail(),
        "headroom_remaining": completion.usage.headroom.remaining_credit(),
    })
    .to_string();
    let approval = NewApproval {
        job_id: job.id.clone(),
        agent_id: prepared.agent_id.clone(),
        thread_id: prepared.thread_id,
        effect_class: class.as_str().into(),
        action: action.into(),
        purpose,
        draft,
        evidence: prepared.evidence,
        evidence_repo: prepared.evidence_repo,
        evidence_ref: prepared.evidence_ref,
        evidence_event_id: prepared.evidence_event_id,
        evidence_kind: prepared.evidence_kind,
        provider: completion.provider,
        model: completion.model,
        usage_kind: completion.usage_kind,
        input_tokens: completion.input_tokens as i64,
        output_tokens: completion.output_tokens as i64,
        micro_usd: completion.micro_usd,
        ledger_note: completion.note,
        project: prepared.project,
    };
    let approval_id = store.record_approval_and_wait(
        wall_ms(),
        &app.worker_id,
        approval,
        &request_payload,
        &ledger_payload,
    )?;
    eprintln!("dasdevbotd approval requested id={approval_id}");
    Ok(())
}

struct Prepared {
    agent_id: String,
    persona: String,
    project: String,
    user_message: String,
    evidence: String,
    evidence_repo: String,
    evidence_ref: String,
    evidence_event_id: String,
    evidence_kind: String,
    purpose: String,
    thread_id: String,
    tainted: bool,
    forced: bool,
}

fn prepare(store: &mut Store, app: &App, job: &Job) -> Result<Option<Prepared>> {
    let agent = store.agent(&job.agent_id)?;
    let budget = store.budget_of(&job.agent_id)?;
    let body: Value = serde_json::from_str(&job.payload).unwrap_or_else(|_| json!({}));
    let thread_id = body
        .get("thread_id")
        .and_then(|v| v.as_str())
        .unwrap_or(&job.id)
        .to_string();
    if !budget.can_reserve(RESERVE_TOKENS) {
        let payload = json!({
            "agent_id": agent.id,
            "token_cap": budget.cap,
            "tokens_spent": budget.spent,
            "reserve": RESERVE_TOKENS,
        })
        .to_string();
        store.append_at(
            wall_ms(),
            "runtime",
            kind::BUDGET_DENIED,
            &payload,
            &format!("budget-denied:{}", job.id),
            Some(&thread_id),
        )?;
        store.fail_leased(&job.id, &app.worker_id)?;
        return Ok(None);
    }
    if !store.heartbeat_at(&job.id, &app.worker_id, wall_ms(), LEASE_MS)? {
        return Ok(None);
    }
    let payload = body.get("payload").cloned().unwrap_or(json!({}));
    let tainted = body
        .get("tainted")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let forced = payload
        .get("forced")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let kind_name = body.get("kind").and_then(|v| v.as_str()).unwrap_or("event");
    let event_id = body.get("event_id").and_then(|v| v.as_str()).unwrap_or("");
    let repo = payload
        .get("repo")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let reference = payload
        .get("ref")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    Ok(Some(Prepared {
        agent_id: agent.id,
        persona: agent.persona,
        project: agent.project,
        user_message: format!(
            "Event kind: {kind_name}\nPayload:\n{}",
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
        ),
        evidence: format!("repo {repo}\nref {reference}\nevent {event_id}"),
        evidence_repo: repo.to_string(),
        evidence_ref: reference.to_string(),
        evidence_event_id: event_id.to_string(),
        evidence_kind: kind_name.to_string(),
        purpose: format!(
            "Post a review comment on {repo} at {reference}. Nothing is sent until you approve, and phase 0 does not send it at all."
        ),
        thread_id,
        tainted,
        forced,
    }))
}

#[derive(Debug, PartialEq, Eq)]
pub enum TurnErrorAction {
    Pause { until_ms: u64 },
    Fail,
}

/// Pause a leased job on a terminal provider limit, or leave a hard failure for the worker.
pub fn settle_turn_error(
    store: &mut Store,
    job: &Job,
    owner: &str,
    err: &ProviderError,
    now_ms: u64,
) -> Result<TurnErrorAction> {
    match err {
        ProviderError::LimitReached(limit) => {
            let until_ms = limit_pause_until(limit.resets_at, now_ms);
            pause_job(store, job, owner, until_ms, "limit_reached", now_ms)?;
            Ok(TurnErrorAction::Pause { until_ms })
        }
        ProviderError::Busy(busy) if busy.terminal => {
            charge_retry_costs(store, &job.agent_id, job, &busy.retry_costs, now_ms)?;
            let until_ms = now_ms.saturating_add(OLLAMA_BUSY_MAX_MS);
            pause_job(store, job, owner, until_ms, "busy", now_ms)?;
            Ok(TurnErrorAction::Pause { until_ms })
        }
        ProviderError::Unavailable(_)
        | ProviderError::Failed(_)
        | ProviderError::Busy(_)
        | ProviderError::ToolUseAttempted => Ok(TurnErrorAction::Fail),
    }
}

fn pause_job(
    store: &mut Store,
    job: &Job,
    owner: &str,
    until_ms: u64,
    reason: &str,
    now_ms: u64,
) -> Result<()> {
    let payload = json!({
        "job_id": job.id,
        "reason": reason,
        "until_ms": until_ms,
    })
    .to_string();
    store.append_at(
        now_ms,
        "runtime",
        kind::JOB_PAUSED,
        &payload,
        &format!("job-paused:{}:{}", job.id, job.attempt),
        None,
    )?;
    store.pause_leased(&job.id, owner, until_ms)?;
    Ok(())
}

fn charge_retry_costs(
    store: &mut Store,
    agent_id: &str,
    job: &Job,
    costs: &[RetryCost],
    now_ms: u64,
) -> Result<()> {
    for cost in costs {
        let tokens = cost.budget_tokens.min(i64::MAX as u64) as i64;
        store.add_spend(agent_id, tokens)?;
        let payload = json!({
            "job_id": job.id,
            "attempt": cost.attempt,
            "backoff_ms": cost.backoff_ms,
            "budget_tokens": cost.budget_tokens,
        })
        .to_string();
        store.append_at(
            now_ms,
            "runtime",
            kind::PROVIDER_COST,
            &payload,
            &format!("provider-cost:{}:{}:{}", job.id, job.attempt, cost.attempt),
            None,
        )?;
    }
    Ok(())
}

fn limit_pause_until(resets_at: Option<i64>, now_ms: u64) -> u64 {
    const POLICY_MS: u64 = 60_000;
    let Some(resets_at) = resets_at.filter(|value| *value > 0) else {
        return now_ms.saturating_add(POLICY_MS);
    };
    let resets = resets_at as u64;
    let until_ms = if resets > 1_000_000_000_000 {
        resets
    } else {
        resets.saturating_mul(1_000)
    };
    if until_ms > now_ms {
        until_ms
    } else {
        now_ms.saturating_add(POLICY_MS)
    }
}

fn tokens_i64(input: u64, output: u64) -> i64 {
    input.saturating_add(output).min(i64::MAX as u64) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{
        Busy, Completion, Headroom, LimitReached, LlmProvider, ProviderError, QuotaSignal,
        RetryCost, UsageReport,
    };
    use crate::store::{Job, Store};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    struct Boom {
        calls: AtomicUsize,
    }

    impl LlmProvider for Boom {
        fn complete(
            &self,
            _req: &CompletionRequest,
        ) -> std::result::Result<Completion, ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(ProviderError::Failed(
                "provider should not be called".into(),
            ))
        }

        fn id(&self) -> &'static str {
            "boom"
        }

        fn detail(&self) -> String {
            "boom".into()
        }
    }

    #[test]
    fn a_capped_agent_halts_without_calling_the_provider() {
        let store = Store::open_memory().unwrap();
        store
            .insert_agent("tiny", "Tiny", "small", "proj", 10)
            .unwrap();
        let job_id = store
            .enqueue_job("tiny", "job-tiny", "{\"tainted\":true}", 0)
            .unwrap()
            .unwrap();
        let job = store.claim_at("owner", 10, 5_000).unwrap().unwrap();
        assert_eq!(job.id, job_id);
        let provider = Boom {
            calls: AtomicUsize::new(0),
        };
        let (wake, _rx) = mpsc::channel();
        let app = App {
            store: Mutex::new(store),
            provider: Arc::new(provider),
            web_root: None,
            role: "server".into(),
            worker_id: "owner".into(),
            wake,
            endpoint_id: Mutex::new(None),
            token: "test-token".into(),
        };
        run_turn(&app, &job).unwrap();
        assert_eq!(app.provider.as_ref().id(), "boom");
        let store = app.store.lock().unwrap();
        assert_eq!(
            store.job_status(&job_id).unwrap().as_deref(),
            Some("failed")
        );
        assert_eq!(store.agent("tiny").unwrap().tokens_spent, 0);
        let events = store.recent_events(10).unwrap();
        assert!(events.iter().any(|event| event.kind == kind::BUDGET_DENIED));
    }

    fn leased_job(store: &Store) -> Job {
        store.claim_at("owner", 1_000, 5_000).unwrap().unwrap()
    }

    fn app_with(store: Store, provider: impl LlmProvider + 'static) -> App {
        let (wake, _rx) = mpsc::channel();
        App {
            store: Mutex::new(store),
            provider: Arc::new(provider),
            web_root: None,
            role: "server".into(),
            worker_id: "owner".into(),
            wake,
            endpoint_id: Mutex::new(None),
            token: "turn-test-token-0123456789abcdef0123".into(),
        }
    }

    #[test]
    fn limit_reached_pauses_until_the_reset_time() {
        let store = Store::open_memory().unwrap();
        store
            .insert_agent("limit-agent", "Limit", "persona", "proj", 10_000)
            .unwrap();
        store
            .enqueue_job("limit-agent", "job-limit", "{}", 0)
            .unwrap();
        let job = leased_job(&store);
        let err = ProviderError::LimitReached(LimitReached {
            message: "claude-cli usage limit reached".into(),
            resets_at: Some(2_000_000_000),
        });
        let mut store = store;
        let action = settle_turn_error(&mut store, &job, "owner", &err, 1_000).unwrap();
        assert_eq!(
            action,
            TurnErrorAction::Pause {
                until_ms: 2_000_000_000_000
            }
        );
        assert_eq!(
            store.job_status(&job.id).unwrap().as_deref(),
            Some("leased")
        );
        assert_eq!(store.lease_until(&job.id).unwrap(), Some(2_000_000_000_000));
        let events = store.recent_events(10).unwrap();
        assert!(events.iter().any(|event| event.kind == kind::JOB_PAUSED));
        assert!(events.iter().all(|event| event.kind != kind::JOB_FAILED));
    }

    #[test]
    fn terminal_busy_charges_retries_then_pauses() {
        let store = Store::open_memory().unwrap();
        store
            .insert_agent("busy-agent", "Busy", "persona", "proj", 10_000)
            .unwrap();
        store
            .enqueue_job("busy-agent", "job-busy", "{}", 0)
            .unwrap();
        let job = leased_job(&store);
        let err = ProviderError::Busy(Busy {
            message: "ollama cloud is busy".into(),
            retry_costs: vec![
                RetryCost {
                    attempt: 1,
                    backoff_ms: 2_000,
                    budget_tokens: 1,
                },
                RetryCost {
                    attempt: 2,
                    backoff_ms: 4_000,
                    budget_tokens: 1,
                },
            ],
            terminal: true,
        });
        let mut store = store;
        let action = settle_turn_error(&mut store, &job, "owner", &err, 10_000).unwrap();
        assert_eq!(action, TurnErrorAction::Pause { until_ms: 70_000 });
        assert_eq!(store.agent("busy-agent").unwrap().tokens_spent, 2);
        assert_eq!(
            store.job_status(&job.id).unwrap().as_deref(),
            Some("leased")
        );
        assert_eq!(store.lease_until(&job.id).unwrap(), Some(70_000));
        let events = store.recent_events(10).unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == kind::PROVIDER_COST)
                .count(),
            2
        );
        assert!(events.iter().any(|event| event.kind == kind::JOB_PAUSED));
        assert!(events.iter().all(|event| event.kind != kind::JOB_FAILED));
    }

    #[test]
    fn tool_use_fails_the_job() {
        struct ToolUse;
        impl LlmProvider for ToolUse {
            fn complete(
                &self,
                _req: &CompletionRequest,
            ) -> std::result::Result<Completion, ProviderError> {
                Err(ProviderError::ToolUseAttempted)
            }
            fn id(&self) -> &'static str {
                "claude-cli"
            }
            fn detail(&self) -> String {
                "claude-cli".into()
            }
        }
        let store = Store::open_memory().unwrap();
        store
            .insert_agent("tool-agent", "Tool", "persona", "proj", 10_000)
            .unwrap();
        let job_id = store
            .enqueue_job("tool-agent", "job-tool", "{}", 0)
            .unwrap()
            .unwrap();
        let job = leased_job(&store);
        let app = app_with(store, ToolUse);
        let err = run_turn(&app, &job).unwrap_err();
        record_turn_failure(&app, &job, &err);
        let store = app.store.lock().unwrap();
        assert_eq!(
            store.job_status(&job_id).unwrap().as_deref(),
            Some("failed")
        );
        let events = store.recent_events(10).unwrap();
        assert!(events.iter().any(|event| event.kind == kind::JOB_FAILED));
        assert!(events.iter().all(|event| event.kind != kind::JOB_PAUSED));
    }

    #[test]
    fn successful_retries_are_charged_before_the_turn_spend() {
        struct Fixed {
            completion: Completion,
        }
        impl LlmProvider for Fixed {
            fn complete(
                &self,
                _req: &CompletionRequest,
            ) -> std::result::Result<Completion, ProviderError> {
                Ok(self.completion.clone())
            }
            fn id(&self) -> &'static str {
                "ollama"
            }
            fn detail(&self) -> String {
                "ollama".into()
            }
        }
        let completion = Completion {
            text: "draft".into(),
            model: "gemma4:31b".into(),
            provider: "ollama".into(),
            usage_kind: "absent".into(),
            input_tokens: 4,
            output_tokens: 0,
            micro_usd: 0,
            note: "note".into(),
            usage: UsageReport {
                input_tokens: Some(4),
                output_tokens: Some(0),
                cached_input_tokens: None,
                quota: QuotaSignal::Absent {
                    detail: "none".into(),
                },
                headroom: Headroom::unknown(),
            },
            retry_costs: vec![
                RetryCost {
                    attempt: 1,
                    backoff_ms: 2_000,
                    budget_tokens: 1,
                },
                RetryCost {
                    attempt: 2,
                    backoff_ms: 4_000,
                    budget_tokens: 1,
                },
            ],
        };
        let store = Store::open_memory().unwrap();
        store
            .insert_agent("cost-agent", "Cost", "persona", "proj", 10_000)
            .unwrap();
        let job_id = store
            .enqueue_job("cost-agent", "job-cost", "{\"tainted\":true}", 0)
            .unwrap()
            .unwrap();
        let job = leased_job(&store);
        let app = app_with(store, Fixed { completion });
        run_turn(&app, &job).unwrap();
        let store = app.store.lock().unwrap();
        assert_eq!(store.agent("cost-agent").unwrap().tokens_spent, 6);
        assert_eq!(
            store.job_status(&job_id).unwrap().as_deref(),
            Some("failed")
        );
        let events = store.recent_events(20).unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == kind::PROVIDER_COST)
                .count(),
            2
        );
    }
}
