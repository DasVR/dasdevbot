use std::sync::mpsc;
use std::thread::JoinHandle;

use dasdevbot_core::{decide, kind, EffectClass, GateInput, Policy};
use dasdevbot_proto::EmitRequest;
use serde_json::{json, Value};

use crate::provider::{CompletionRequest, RESERVE_TOKENS};
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
    // Claim until the queue is empty, then park. The channel buffers a wake that
    // arrives between the empty claim and recv, so a notification cannot be lost
    // and the thread does not poll.
    loop {
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
                eprintln!("dasdevbotd turn failed: {err}");
                let mut store = app.store.lock().expect("store");
                let payload = json!({"job_id": job.id, "error": err.to_string()}).to_string();
                let key = format!("job-failed:{}:{}", job.id, job.attempt);
                let _ =
                    store.append_at(wall_ms(), "runtime", kind::JOB_FAILED, &payload, &key, None);
                let _ = store.fail_leased(&job.id, &app.worker_id);
            }
        }
        if rx.recv().is_err() {
            break;
        }
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
    let completion = app.provider.complete(&CompletionRequest {
        model: String::new(),
        system: prepared.persona,
        user: prepared.user_message,
        max_tokens: 512,
    })?;
    let mut store = app.store.lock().expect("store");
    if !store.heartbeat_at(&job.id, &app.worker_id, wall_ms(), LEASE_MS)? {
        return Ok(());
    }
    let grant = store.has_grant(&prepared.agent_id, EffectClass::External.as_str())?;
    let outcome = decide(
        GateInput {
            grant,
            class: EffectClass::External,
            budget_remaining: true,
            tainted: prepared.tainted,
        },
        Policy::phase0(),
    );
    if outcome != dasdevbot_core::GateOutcome::Ask {
        let payload = json!({
            "job_id": job.id,
            "effect_class": "external",
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
    let request_payload = json!({
        "job_id": job.id,
        "action": "post_pr_comment",
        "effect_class": "external",
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
    })
    .to_string();
    let approval = NewApproval {
        job_id: job.id.clone(),
        agent_id: prepared.agent_id.clone(),
        thread_id: prepared.thread_id,
        effect_class: "external".into(),
        action: "post_pr_comment".into(),
        purpose: prepared.purpose,
        draft: completion.text,
        evidence: prepared.evidence,
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
    purpose: String,
    thread_id: String,
    tainted: bool,
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
        purpose: format!(
            "Post a review comment on {repo} at {reference}. Nothing is sent until you approve, and phase 0 does not send it at all."
        ),
        thread_id,
        tainted,
    }))
}

fn tokens_i64(input: u64, output: u64) -> i64 {
    input.saturating_add(output).min(i64::MAX as u64) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{Completion, LlmProvider, ProviderError};
    use crate::store::Store;
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
            Err(ProviderError("provider should not be called".into()))
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
}
