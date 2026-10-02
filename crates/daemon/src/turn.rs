use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;

use dasdevbot_core::{
    claim_mode, decide, kind, Admission, ClaimMode, EffectClass, GateInput, Policy, RunEnd,
    SignalRead, RETRY_CAP,
};
use dasdevbot_proto::EmitRequest;
use serde_json::{json, Value};

use crate::audit_log;
use crate::caps::{
    self, admit_job, commit_attempt, park_job, release_reservation, reserve_attempt,
    settle_failed_attempt,
};
use crate::harness::{self, HarnessRun};
use crate::provider::{
    Completion, CompletionRequest, ProviderError, RetryCost, OLLAMA_BUSY_MAX_MS,
};
use crate::store::{AppendOutcome, Job, NewApproval, Store};
use crate::topology;
use crate::{wall_ms, App, Error, Result};

const LEASE_MS: u64 = 120_000;
const CALL_MAX_TOKENS: u32 = 512;

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
        {
            let mut store = app.store.lock().expect("store");
            let now = wall_ms();
            if let Err(err) = topology::tick(&mut store, &app.role, &app.worker_id, now) {
                eprintln!("dasdevbotd topology: {err}");
            }
            if let Err(err) = caps::release_quota_waits(&store, now) {
                eprintln!("dasdevbotd quota: {err}");
            }
            if let Err(err) = caps::release_slot_waits(&store) {
                eprintln!("dasdevbotd slot: {err}");
            }
        }
        loop {
            let job = {
                let store = app.store.lock().expect("store");
                let claimed = match claim_mode(&app.role) {
                    ClaimMode::None => Ok(None),
                    ClaimMode::AssignedOnly => {
                        store.claim_assigned(&app.worker_id, wall_ms(), LEASE_MS)
                    }
                    ClaimMode::IncludeUnassigned => {
                        store.claim_at(&app.worker_id, wall_ms(), LEASE_MS)
                    }
                };
                match claimed {
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
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(()) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
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
    let (prepared, request, estimate) = {
        let mut store = app.store.lock().expect("store");
        let Some(prepared) = prepare(&mut store, &app.worker_id, job, &app.audit_seed)? else {
            return Ok(());
        };
        if prepared.class == EffectClass::Destructive {
            deny_destructive(&mut store, app, job, &prepared)?;
            return Ok(());
        }
        if prepared.class == EffectClass::External && external_tier_is_denied() {
            deny_external(&mut store, app, job, &prepared)?;
            return Ok(());
        }
        let request = CompletionRequest {
            model: String::new(),
            system: prepared.persona.clone(),
            user: prepared.user_message.clone(),
            max_tokens: CALL_MAX_TOKENS,
        };
        let estimate = app.provider.attempt_worst_case(&request);
        if !admit_turn(&mut store, app, job, &prepared.agent_id, estimate)? {
            return Ok(());
        }
        if !store.heartbeat_at(&job.id, &app.worker_id, wall_ms(), LEASE_MS)? {
            release_reservation(&mut store, &job.id, &app.worker_id, wall_ms())?;
            return Ok(());
        }
        (prepared, request, estimate)
    };
    let Some(completion) = run_attempts(app, job, &request, estimate)? else {
        return Ok(());
    };
    let mut store = app.store.lock().expect("store");
    if !store.heartbeat_at(&job.id, &app.worker_id, wall_ms(), LEASE_MS)? {
        let actual = completion
            .input_tokens
            .saturating_add(completion.output_tokens);
        commit_attempt(&mut store, &job.id, &app.worker_id, actual, wall_ms())?;
        release_reservation(&mut store, &job.id, &app.worker_id, wall_ms())?;
        return Ok(());
    }
    let actual = completion
        .input_tokens
        .saturating_add(completion.output_tokens);
    if let Err(err) = commit_attempt(&mut store, &job.id, &app.worker_id, actual, wall_ms()) {
        release_reservation(&mut store, &job.id, &app.worker_id, wall_ms())?;
        return Err(err);
    }
    let class = prepared.class;
    let grant = store.has_grant(&prepared.agent_id, class.as_str(), wall_ms())?;
    let outcome = decide(
        GateInput {
            grant,
            class,
            budget_remaining: true,
            tainted: prepared.tainted,
        },
        phase1_policy(),
    );
    if outcome != dasdevbot_core::GateOutcome::Ask {
        let payload = json!({
            "job_id": job.id,
            "effect_class": class.as_str(),
            "outcome": "deny",
            "tainted": prepared.tainted,
        })
        .to_string();
        audit_log::append(
            &mut store,
            kind::GATE_DENIED,
            &payload,
            wall_ms(),
            &app.audit_seed,
        )?;
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
        release_reservation(&mut store, &job.id, &app.worker_id, wall_ms())?;
        store.fail_leased(&job.id, &app.worker_id)?;
        return Ok(());
    }
    let action = prepared.action.as_str();
    let purpose = prepared.purpose.clone();
    let draft = completion.text.clone();
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
    release_reservation(&mut store, &job.id, &app.worker_id, wall_ms())?;
    eprintln!("dasdevbotd approval requested id={approval_id}");
    Ok(())
}

fn run_attempts(
    app: &App,
    job: &Job,
    request: &CompletionRequest,
    estimate: u64,
) -> Result<Option<Completion>> {
    let parked = AtomicBool::new(false);
    let completion = Mutex::new(None);
    let last_error: Mutex<Option<ProviderError>> = Mutex::new(None);
    let cli_version = Mutex::new(String::new());
    let end = harness::execute(
        &app.store,
        HarnessRun {
            job_id: &job.id,
            lease_owner: Some(&app.worker_id),
            retry_cap: RETRY_CAP,
            now_ms: wall_ms(),
            audit_seed: &app.audit_seed,
            cli_version: &cli_version,
        },
        || true,
        || {
            let mut charge = |_cost: &RetryCost| {
                let mut store = app.store.lock().expect("store");
                settle_failed_attempt(&mut store, &job.id, &app.worker_id, wall_ms())
                    .map_err(|err| ProviderError::Failed(err.to_string()))?;
                let held =
                    reserve_attempt(&mut store, &job.id, &app.worker_id, estimate, wall_ms())
                        .map_err(|err| ProviderError::Failed(err.to_string()))?;
                if held {
                    Ok(())
                } else {
                    release_reservation(&mut store, &job.id, &app.worker_id, wall_ms())
                        .map_err(|err| ProviderError::Failed(err.to_string()))?;
                    pause_for(&mut store, app, job, "reservation failed", None)
                        .map_err(|err| ProviderError::Failed(err.to_string()))?;
                    parked.store(true, Ordering::SeqCst);
                    Err(ProviderError::Failed("reservation failed".into()))
                }
            };
            match app.provider.complete(request, &mut charge) {
                Ok(done) => {
                    let text = done.text.clone();
                    *completion.lock().expect("completion") = Some(done);
                    Ok(text)
                }
                Err(err) => {
                    if parked.load(Ordering::SeqCst) {
                        return Err(dasdevbot_core::ProviderStop::Limit);
                    }
                    match stop_after_provider_error(app, job, &err, &cli_version) {
                        Ok(stop) => {
                            *last_error.lock().expect("provider error") = Some(err);
                            Err(stop)
                        }
                        Err(failure) => {
                            *last_error.lock().expect("provider error") =
                                Some(ProviderError::Failed(failure.to_string()));
                            Err(dasdevbot_core::ProviderStop::Fault)
                        }
                    }
                }
            }
        },
    )?;
    match end {
        RunEnd::Done { .. } => completion
            .into_inner()
            .expect("completion")
            .ok_or_else(|| Error::BadRequest("provider returned no completion".into()))
            .map(Some),
        RunEnd::Paused { .. } => Ok(None),
        RunEnd::FailedClosed { .. } => Err(last_error
            .into_inner()
            .expect("provider error")
            .unwrap_or_else(|| ProviderError::Failed("harness failed closed".into()))
            .into()),
    }
}

fn stop_after_provider_error(
    app: &App,
    job: &Job,
    err: &ProviderError,
    cli_version: &Mutex<String>,
) -> Result<dasdevbot_core::ProviderStop> {
    let mut store = app.store.lock().expect("store");
    if let ProviderError::ToolUseAttempted {
        cli_version: version,
        event,
    } = err
    {
        *cli_version.lock().expect("cli version") = version.clone();
        let payload = json!({
            "job_id": job.id,
            "cli_version": version,
            "event": event,
        })
        .to_string();
        store.append_at(
            wall_ms(),
            "runtime",
            kind::TOOL_USE_BLOCKED,
            &payload,
            &format!("tool-use:{}:{}", job.id, job.attempt),
            None,
        )?;
    }
    settle_failed_attempt(&mut store, &job.id, &app.worker_id, wall_ms())?;
    let stop = provider_stop(err);
    release_reservation(&mut store, &job.id, &app.worker_id, wall_ms())?;
    let mapped = match &stop {
        ProviderStop::Pause { reason, until_ms } => {
            pause_for(&mut store, app, job, reason, *until_ms)?;
            match err {
                ProviderError::Busy(_) => dasdevbot_core::ProviderStop::Busy,
                ProviderError::LimitReached(_) => dasdevbot_core::ProviderStop::Limit,
                ProviderError::Failed(_) => dasdevbot_core::ProviderStop::Limit,
                ProviderError::Unavailable(_) | ProviderError::ToolUseAttempted { .. } => {
                    dasdevbot_core::ProviderStop::Fault
                }
            }
        }
        ProviderStop::Fail => match err {
            ProviderError::ToolUseAttempted { .. } => {
                dasdevbot_core::ProviderStop::ToolUseAttempted
            }
            ProviderError::Unavailable(_)
            | ProviderError::Failed(_)
            | ProviderError::Busy(_)
            | ProviderError::LimitReached(_) => dasdevbot_core::ProviderStop::Fault,
        },
    };
    Ok(mapped)
}

fn phase1_policy() -> Policy {
    // External is denied by the policy itself, not by a failing verifier.
    Policy::phase1()
}

fn external_tier_is_denied() -> bool {
    phase1_policy().deny_external
}

enum ProviderStop {
    Pause {
        reason: String,
        until_ms: Option<u64>,
    },
    Fail,
}

fn provider_stop(err: &ProviderError) -> ProviderStop {
    match err {
        ProviderError::LimitReached(limit) => ProviderStop::Pause {
            reason: "limit_reached".into(),
            until_ms: Some(limit_pause_until(limit.resets_at, wall_ms())),
        },
        ProviderError::Busy(busy) if busy.terminal => ProviderStop::Pause {
            reason: "busy".into(),
            until_ms: Some(wall_ms().saturating_add(OLLAMA_BUSY_MAX_MS)),
        },
        ProviderError::Failed(message) if message == "reservation failed" => ProviderStop::Pause {
            reason: "reservation failed".into(),
            until_ms: None,
        },
        ProviderError::Unavailable(_)
        | ProviderError::Failed(_)
        | ProviderError::Busy(_)
        | ProviderError::ToolUseAttempted { .. } => ProviderStop::Fail,
    }
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

fn pause_for(
    store: &mut Store,
    app: &App,
    job: &Job,
    reason: &str,
    until_ms: Option<u64>,
) -> Result<()> {
    let payload = json!({
        "job_id": job.id,
        "reason": reason,
        "until_ms": until_ms,
    })
    .to_string();
    store.append_at(
        wall_ms(),
        "runtime",
        kind::JOB_PAUSED,
        &payload,
        &format!("job-paused:{}:{}", job.id, job.attempt),
        None,
    )?;
    park_job(
        store,
        &job.id,
        &app.worker_id,
        "paused",
        app.provider.id(),
        until_ms,
    )?;
    Ok(())
}

pub fn audit_dev_env(app: &App, role: &str) -> Result<()> {
    let payload = json!({
        "flag": "--dev-env-secrets",
        "role": role,
    })
    .to_string();
    let mut store = app.store.lock().expect("store");
    store.append_at(
        wall_ms(),
        "runtime",
        kind::DEV_ENV,
        &payload,
        &format!("secret-dev-env:{}", uuid::Uuid::new_v4()),
        None,
    )?;
    audit_log::append(
        &mut store,
        kind::DEV_ENV,
        &payload,
        wall_ms(),
        &app.audit_seed,
    )?;
    Ok(())
}

fn deny_destructive(store: &mut Store, app: &App, job: &Job, prepared: &Prepared) -> Result<()> {
    let approval = NewApproval {
        job_id: job.id.clone(),
        agent_id: prepared.agent_id.clone(),
        thread_id: prepared.thread_id.clone(),
        effect_class: prepared.class.as_str().into(),
        action: prepared.action.clone(),
        purpose: format!(
            "Force-push {}. This rewrites the remote branch.",
            prepared.evidence_ref
        ),
        draft: format!("git push --force origin {}", prepared.evidence_ref),
        evidence: prepared.evidence.clone(),
        evidence_repo: prepared.evidence_repo.clone(),
        evidence_ref: prepared.evidence_ref.clone(),
        evidence_event_id: prepared.evidence_event_id.clone(),
        evidence_kind: prepared.evidence_kind.clone(),
        provider: "none".into(),
        model: String::new(),
        usage_kind: "none".into(),
        input_tokens: 0,
        output_tokens: 0,
        micro_usd: 0,
        ledger_note: "phase 1 denies destructive work".into(),
        project: prepared.project.clone(),
    };
    store.record_gate_denial(
        wall_ms(),
        &app.worker_id,
        &approval,
        "destructive is denied in phase 1",
        &app.audit_seed,
    )?;
    Ok(())
}

fn deny_external(store: &mut Store, app: &App, job: &Job, prepared: &Prepared) -> Result<()> {
    let approval = NewApproval {
        job_id: job.id.clone(),
        agent_id: prepared.agent_id.clone(),
        thread_id: prepared.thread_id.clone(),
        effect_class: prepared.class.as_str().into(),
        action: prepared.action.clone(),
        purpose: format!(
            "External effect on {} is denied until Windows Hello verifies the signer.",
            prepared.evidence_repo
        ),
        draft: format!(
            "post_pr_comment on {} at {} is denied",
            prepared.evidence_repo, prepared.evidence_ref
        ),
        evidence: prepared.evidence.clone(),
        evidence_repo: prepared.evidence_repo.clone(),
        evidence_ref: prepared.evidence_ref.clone(),
        evidence_event_id: prepared.evidence_event_id.clone(),
        evidence_kind: prepared.evidence_kind.clone(),
        provider: "none".into(),
        model: String::new(),
        usage_kind: "none".into(),
        input_tokens: 0,
        output_tokens: 0,
        micro_usd: 0,
        ledger_note: "phase 1 denies external work until user verification is real".into(),
        project: prepared.project.clone(),
    };
    store.record_gate_denial(
        wall_ms(),
        &app.worker_id,
        &approval,
        "external is denied until Windows Hello is the production verifier",
        &app.audit_seed,
    )?;
    Ok(())
}

fn admit_turn(
    store: &mut Store,
    app: &App,
    job: &Job,
    agent_id: &str,
    estimate: u64,
) -> Result<bool> {
    let decision = admit_job(
        store,
        caps::AdmitRequest {
            role: &app.role,
            writer_id: &app.worker_id,
            teammate_id: agent_id,
            providers: &[(app.provider.id(), SignalRead::Absent)],
            estimate: Some(estimate),
            now_ms: wall_ms(),
            job_id: &job.id,
        },
    );
    match decision {
        Admission::Admit { .. } => Ok(true),
        Admission::WaitingOnQuota { provider, until_ms } => {
            park_wait(
                store,
                app,
                job,
                kind::WAITING_ON_QUOTA,
                "waiting_on_quota",
                &provider,
                until_ms,
            )?;
            Ok(false)
        }
        Admission::AskOwner { provider } => {
            park_wait(
                store,
                app,
                job,
                kind::WAITING_ON_QUOTA,
                "waiting_on_quota",
                &provider,
                None,
            )?;
            Ok(false)
        }
        Admission::WaitingOnSlot { provider } => {
            park_wait(
                store,
                app,
                job,
                kind::WAITING_ON_SLOT,
                "waiting_on_slot",
                &provider,
                None,
            )?;
            Ok(false)
        }
        Admission::Deny { reason } => {
            let payload = json!({
                "agent_id": agent_id,
                "job_id": job.id,
                "reason": format!("{reason:?}"),
                "reserve": estimate,
            })
            .to_string();
            if reason == dasdevbot_core::DenyReason::TeammateBudget {
                store.append_at(
                    wall_ms(),
                    "runtime",
                    kind::JOB_PAUSED,
                    &payload,
                    &format!("job-paused:{}", job.id),
                    None,
                )?;
                park_job(
                    store,
                    &job.id,
                    &app.worker_id,
                    "paused",
                    app.provider.id(),
                    None,
                )?;
                return Ok(false);
            }
            store.append_at(
                wall_ms(),
                "runtime",
                kind::BUDGET_DENIED,
                &payload,
                &format!("budget-denied:{}", job.id),
                None,
            )?;
            store.fail_leased(&job.id, &app.worker_id)?;
            Ok(false)
        }
    }
}

fn park_wait(
    store: &mut Store,
    app: &App,
    job: &Job,
    event_kind: &str,
    status: &str,
    provider: &str,
    until_ms: Option<u64>,
) -> Result<()> {
    let payload = json!({
        "job_id": job.id,
        "provider": provider,
        "until_ms": until_ms,
    })
    .to_string();
    store.append_at(
        wall_ms(),
        "runtime",
        event_kind,
        &payload,
        &format!("{status}:{}", job.id),
        None,
    )?;
    park_job(store, &job.id, &app.worker_id, status, provider, until_ms)?;
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
    class: EffectClass,
    action: String,
}

fn prepare(
    store: &mut Store,
    owner: &str,
    job: &Job,
    audit_seed: &[u8; 32],
) -> Result<Option<Prepared>> {
    let agent = store.agent(&job.agent_id)?;
    let body: Value = serde_json::from_str(&job.payload).unwrap_or_else(|_| json!({}));
    let thread_id = body
        .get("thread_id")
        .and_then(|v| v.as_str())
        .unwrap_or(&job.id)
        .to_string();
    if !store.heartbeat_at(&job.id, owner, wall_ms(), LEASE_MS)? {
        return Ok(None);
    }
    let payload = body.get("payload").cloned().unwrap_or(json!({}));
    let tainted = body
        .get("tainted")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let kind_name = body.get("kind").and_then(|v| v.as_str()).unwrap_or("event");
    let Some((class, action)) = classify(kind_name) else {
        deny_unknown(store, owner, job, kind_name, audit_seed)?;
        return Ok(None);
    };
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
        purpose: match class {
            EffectClass::WriteLocal => format!(
                "Edit the local workspace for {repo} at {reference}. Nothing is written until you approve, and phase 0 does not write it at all."
            ),
            _ => format!(
                "Post a review comment on {repo} at {reference}. Nothing is sent until you approve, and phase 0 does not send it at all."
            ),
        },
        thread_id,
        tainted,
        class,
        action: action.to_string(),
    }))
}

/// The daemon classifies the event kind. A field on the payload does not.
/// Unknown kinds are denied. They are not treated as external.
fn classify(kind: &str) -> Option<(EffectClass, &'static str)> {
    match kind {
        "repo.force_push" => Some((EffectClass::Destructive, "force_push")),
        "repo.push" => Some((EffectClass::External, "post_pr_comment")),
        "workspace.write" => Some((EffectClass::WriteLocal, "edit")),
        _ => None,
    }
}

fn deny_unknown(
    store: &mut Store,
    owner: &str,
    job: &Job,
    kind_name: &str,
    audit_seed: &[u8; 32],
) -> Result<()> {
    let approval = NewApproval {
        job_id: job.id.clone(),
        agent_id: job.agent_id.clone(),
        thread_id: job.id.clone(),
        effect_class: "unknown".into(),
        action: "deny".into(),
        purpose: format!("Unknown event kind {kind_name}."),
        draft: format!("denied unknown event kind {kind_name}"),
        evidence: String::new(),
        evidence_repo: String::new(),
        evidence_ref: String::new(),
        evidence_event_id: String::new(),
        evidence_kind: kind_name.to_string(),
        provider: "none".into(),
        model: String::new(),
        usage_kind: "none".into(),
        input_tokens: 0,
        output_tokens: 0,
        micro_usd: 0,
        ledger_note: "unknown event kind is denied".into(),
        project: String::new(),
    };
    store.record_gate_denial(
        wall_ms(),
        owner,
        &approval,
        "unknown event kind",
        audit_seed,
    )?;
    Ok(())
}

fn tokens_i64(input: u64, output: u64) -> i64 {
    input.saturating_add(output).min(i64::MAX as u64) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{Completion, LlmProvider, ProviderError, RetryCost};
    use crate::store::Store;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    struct Boom {
        calls: Arc<AtomicUsize>,
    }

    impl LlmProvider for Boom {
        fn complete(
            &self,
            _req: &CompletionRequest,
            _charge: &mut dyn FnMut(&RetryCost) -> std::result::Result<(), ProviderError>,
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
            .enqueue_job(
                "tiny",
                "job-tiny",
                "{\"tainted\":true,\"kind\":\"workspace.write\"}",
                0,
            )
            .unwrap()
            .unwrap();
        let job = store.claim_at("owner", 10, 5_000).unwrap().unwrap();
        assert_eq!(job.id, job_id);
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = Boom {
            calls: Arc::clone(&calls),
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
            token: "0123456789abcdef0123456789abcdef".into(),
            data: std::path::PathBuf::from("data/dasdevbot.sqlite"),
            audit_seed: [9u8; 32],
            window_secrets: crate::test_window_secrets(),
        };
        run_turn(&app, &job).unwrap();
        assert_eq!(app.provider.as_ref().id(), "boom");
        let store = app.store.lock().unwrap();
        assert_eq!(
            store.job_status(&job_id).unwrap().as_deref(),
            Some("paused")
        );
        assert_eq!(store.agent("tiny").unwrap().tokens_spent, 0);
        let events = store.recent_events(10).unwrap();
        assert!(events.iter().any(|event| event.kind == kind::JOB_PAUSED));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    struct BusyOnce {
        calls: Arc<AtomicUsize>,
        held: Arc<AtomicU64>,
    }

    impl LlmProvider for BusyOnce {
        fn complete(
            &self,
            req: &CompletionRequest,
            charge: &mut dyn FnMut(&RetryCost) -> std::result::Result<(), ProviderError>,
        ) -> std::result::Result<Completion, ProviderError> {
            self.held
                .store(self.attempt_worst_case(req), Ordering::SeqCst);
            self.calls.fetch_add(1, Ordering::SeqCst);
            charge(&RetryCost {
                attempt: 1,
                backoff_ms: 0,
                budget_tokens: 1,
            })?;
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(ProviderError::Failed("second attempt failed".into()))
        }

        fn id(&self) -> &'static str {
            "mock"
        }

        fn detail(&self) -> String {
            "busy once".into()
        }
    }

    #[test]
    fn a_retry_reserves_the_worst_case_not_one_token() {
        let store = Store::open_memory().unwrap();
        store
            .insert_agent("wide", "Wide", "persona", "proj", 100_000)
            .unwrap();
        let job_id = store
            .enqueue_job(
                "wide",
                "job-busy",
                "{\"tainted\":true,\"kind\":\"workspace.write\"}",
                0,
            )
            .unwrap()
            .unwrap();
        let job = store.claim_at("owner", 10, 5_000).unwrap().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let held = Arc::new(AtomicU64::new(0));
        let provider = BusyOnce {
            calls: Arc::clone(&calls),
            held: Arc::clone(&held),
        };
        let (wake, _rx) = mpsc::channel();
        let app = App {
            store: Mutex::new(store),
            provider: Arc::new(provider),
            web_root: None,
            role: "executor".into(),
            worker_id: "owner".into(),
            wake,
            endpoint_id: Mutex::new(None),
            token: "0123456789abcdef0123456789abcdef".into(),
            data: std::path::PathBuf::from("data/dasdevbot.sqlite"),
            audit_seed: [9u8; 32],
            window_secrets: crate::test_window_secrets(),
        };
        let err = run_turn(&app, &job).unwrap_err();
        assert!(err.to_string().contains("second attempt failed"), "{err}");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let worst = held.load(Ordering::SeqCst);
        assert!(worst > u64::from(CALL_MAX_TOKENS));
        assert_ne!(worst, 1);
        let store = app.store.lock().unwrap();
        let spent: i64 = store
            .connection()
            .query_row(
                "SELECT spent_tokens FROM provider_caps WHERE provider = 'mock'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(spent, worst as i64 * 2);
        let reserved: i64 = store
            .connection()
            .query_row(
                "SELECT reserved_tokens FROM provider_caps WHERE provider = 'mock'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(reserved, 0);
        assert_eq!(job.id, job_id);
        assert_eq!(store.agent("wide").unwrap().tokens_spent, worst as i64 * 2);
    }

    struct HoldOnce {
        calls: Arc<AtomicUsize>,
    }

    impl LlmProvider for HoldOnce {
        fn complete(
            &self,
            _req: &CompletionRequest,
            charge: &mut dyn FnMut(&RetryCost) -> std::result::Result<(), ProviderError>,
        ) -> std::result::Result<Completion, ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            charge(&RetryCost {
                attempt: 1,
                backoff_ms: 0,
                budget_tokens: 1,
            })?;
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(ProviderError::Failed("should have paused".into()))
        }

        fn attempt_worst_case(&self, _req: &CompletionRequest) -> u64 {
            1_000
        }

        fn id(&self) -> &'static str {
            "mock"
        }

        fn detail(&self) -> String {
            "hold once".into()
        }
    }

    #[test]
    fn a_retry_hold_that_does_not_fit_pauses_the_job() {
        let store = Store::open_memory().unwrap();
        store
            .insert_agent("wide", "Wide", "persona", "proj", 100_000)
            .unwrap();
        store
            .connection()
            .execute(
                "UPDATE provider_caps
                 SET cap_tokens = 1500, spent_tokens = 0, reserved_tokens = 0,
                     fencing_epoch = 1, writer_id = 'bootstrap', writer_until_ms = 0
                 WHERE provider = 'mock'",
                [],
            )
            .unwrap();
        let job_id = store
            .enqueue_job(
                "wide",
                "job-pause",
                "{\"tainted\":true,\"kind\":\"workspace.write\"}",
                0,
            )
            .unwrap()
            .unwrap();
        let job = store.claim_at("owner", 10, 5_000).unwrap().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = HoldOnce {
            calls: Arc::clone(&calls),
        };
        let (wake, _rx) = mpsc::channel();
        let app = App {
            store: Mutex::new(store),
            provider: Arc::new(provider),
            web_root: None,
            role: "executor".into(),
            worker_id: "owner".into(),
            wake,
            endpoint_id: Mutex::new(None),
            token: "0123456789abcdef0123456789abcdef".into(),
            data: std::path::PathBuf::from("data/dasdevbot.sqlite"),
            audit_seed: [9u8; 32],
            window_secrets: crate::test_window_secrets(),
        };
        run_turn(&app, &job).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let store = app.store.lock().unwrap();
        assert_eq!(
            store.job_status(&job_id).unwrap().as_deref(),
            Some("paused")
        );
        assert_eq!(store.agent("wide").unwrap().tokens_spent, 1_000);
        let events = store.recent_events(20).unwrap();
        assert!(events.iter().any(|event| event.kind == kind::JOB_PAUSED));
        assert!(!events.iter().any(|event| event.kind == kind::JOB_FAILED));
    }

    #[test]
    fn an_unknown_event_kind_is_denied() {
        let store = Store::open_memory().unwrap();
        let job_id = store
            .enqueue_job("reviewer", "job-unknown", "{\"kind\":\"nope\"}", 0)
            .unwrap()
            .unwrap();
        let job = store.claim_at("owner", 10, 5_000).unwrap().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = Boom {
            calls: Arc::clone(&calls),
        };
        let (wake, _rx) = mpsc::channel();
        let app = App {
            store: Mutex::new(store),
            provider: Arc::new(provider),
            web_root: None,
            role: "executor".into(),
            worker_id: "owner".into(),
            wake,
            endpoint_id: Mutex::new(None),
            token: "0123456789abcdef0123456789abcdef".into(),
            data: std::path::PathBuf::from("data/dasdevbot.sqlite"),
            audit_seed: [9u8; 32],
            window_secrets: crate::test_window_secrets(),
        };
        run_turn(&app, &job).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let store = app.store.lock().unwrap();
        assert_eq!(
            store.job_status(&job_id).unwrap().as_deref(),
            Some("failed")
        );
        let approvals = store.approvals().unwrap();
        assert_eq!(approvals[0].effect_class, "unknown");
        assert_eq!(approvals[0].status, "denied");
    }

    fn mock_app(store: Store, data: std::path::PathBuf) -> App {
        let (wake, _rx) = mpsc::channel();
        App {
            store: Mutex::new(store),
            provider: Arc::new(crate::provider::MockProvider::new()),
            web_root: None,
            role: "executor".into(),
            worker_id: "owner".into(),
            wake,
            endpoint_id: Mutex::new(None),
            token: "0123456789abcdef0123456789abcdef".into(),
            data,
            audit_seed: [9u8; 32],
            window_secrets: crate::test_window_secrets(),
        }
    }

    fn run_mock_job(kind_name: &str, payload: Value) -> App {
        let store = Store::open_memory().unwrap();
        let body = json!({
            "kind": kind_name,
            "tainted": true,
            // A provider or a payload cannot pick the tier.
            "effect_class": "write_local",
            "tier": "internal",
            "payload": payload,
        });
        store
            .enqueue_job(
                "reviewer",
                &format!("job-mock-{kind_name}"),
                &body.to_string(),
                0,
            )
            .unwrap()
            .unwrap();
        let job = store.claim_at("owner", 10, 5_000).unwrap().unwrap();
        let app = mock_app(store, std::path::PathBuf::from("data/dasdevbot.sqlite"));
        run_turn(&app, &job).unwrap();
        app
    }

    #[test]
    fn a_mock_card_gets_its_tier_from_the_action_type_only() {
        let claims = json!({
            "repo": "DasVR/NIL",
            "ref": "phase0",
            "effect_class": "write_local",
            "tier": "internal",
            "provider": "mock",
        });
        for (kind_name, class) in [
            ("repo.push", "external"),
            ("repo.force_push", "destructive"),
        ] {
            let app = run_mock_job(kind_name, claims.clone());
            let store = app.store.lock().unwrap();
            let approvals = store.approvals().unwrap();
            assert_eq!(approvals.len(), 1, "{kind_name}");
            assert_eq!(approvals[0].effect_class, class, "{kind_name}");
            assert_eq!(approvals[0].status, "denied", "{kind_name}");
            assert_eq!(approvals[0].provider, "none", "{kind_name}");
        }
        let app = run_mock_job(
            "workspace.write",
            json!({
                "repo": "DasVR/NIL",
                "ref": "phase0",
                "effect_class": "external",
                "tier": "destructive",
                "action": "force_push",
            }),
        );
        let store = app.store.lock().unwrap();
        let approvals = store.approvals().unwrap();
        assert_eq!(approvals.len(), 1);
        assert_eq!(approvals[0].effect_class, "write_local");
        assert_eq!(approvals[0].action, "edit");
        assert_eq!(approvals[0].status, "pending");
        assert_eq!(approvals[0].provider, "mock");
    }

    #[test]
    fn approving_a_mock_card_has_no_effect() {
        use crate::ipc::{sign_decision, SignRequest};
        use crate::secrets::{MemorySecrets, APPROVAL_KEY_NAME};
        use crate::verify_user::TestVerifier;

        let data_dir =
            std::env::temp_dir().join(format!("dasdevbot-mock-effects-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&data_dir).unwrap();
        let store = Store::open_memory().unwrap();
        store
            .enqueue_job(
                "reviewer",
                "job-mock-effects",
                &json!({
                    "kind": "workspace.write",
                    "tainted": true,
                    "payload": {"repo": "DasVR/NIL", "ref": "phase0"},
                })
                .to_string(),
                0,
            )
            .unwrap()
            .unwrap();
        let job = store.claim_at("owner", 10, 5_000).unwrap().unwrap();
        let app = mock_app(store, data_dir.join("dasdevbot.sqlite"));
        run_turn(&app, &job).unwrap();
        let mut store = app.store.lock().unwrap();
        let card = store.approvals().unwrap().remove(0);
        assert_eq!(card.provider, "mock");
        assert_eq!(card.effect_class, "write_local");
        let keys = MemorySecrets::new();
        keys.insert(
            APPROVAL_KEY_NAME,
            "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff",
        );
        // Hello is unchanged: a refused verification does not approve the card.
        let refused = sign_decision(
            &mut store,
            SignRequest {
                window: dasdevbot_core::CARD_WINDOW,
                voice: false,
                approval_id: &card.id,
                decision: "approve",
                reason: None,
                now_ms: 2_000,
                fencing: 7,
                secrets: &keys,
                verifier: &TestVerifier { allow: false },
                audit_seed: &[9u8; 32],
                client_signature: None,
                client_nonce: None,
            },
        );
        assert!(refused.is_err());
        assert_eq!(store.approval_status(&card.id).unwrap(), "pending");
        let signed = sign_decision(
            &mut store,
            SignRequest {
                window: dasdevbot_core::CARD_WINDOW,
                voice: false,
                approval_id: &card.id,
                decision: "approve",
                reason: None,
                now_ms: 2_000,
                fencing: 7,
                secrets: &keys,
                verifier: &TestVerifier { allow: true },
                audit_seed: &[9u8; 32],
                client_signature: None,
                client_nonce: None,
            },
        )
        .unwrap();
        assert_eq!(signed.status, "approved");
        let committed = store.commit_due(u64::MAX / 4).unwrap();
        assert_eq!(committed, vec![card.id.clone()]);
        let payload: String = store
            .connection()
            .query_row(
                "SELECT payload FROM events WHERE kind = ?1",
                [kind::APPROVAL_COMMITTED],
                |row| row.get(0),
            )
            .unwrap();
        let body: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(body["executed"], false);
        assert_eq!(app.provider.id(), "mock");
        // Nothing was written next to the data path, let alone outside it.
        let written: Vec<_> = std::fs::read_dir(&data_dir).unwrap().collect();
        assert!(written.is_empty(), "{written:?}");
        std::fs::remove_dir_all(&data_dir).unwrap();
    }
}
