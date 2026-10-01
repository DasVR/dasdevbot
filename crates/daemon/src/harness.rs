//! Harness persistence. The caller reserves the worst-case estimate before
//! each attempt. Busy and the retry cap pause on the saved checkpoint.
//! Tool use fails the job closed. Status updates require the lease owner.

use std::sync::Mutex;

use dasdevbot_core::{drive_provider, HarnessState, Phase, ProviderStop, RunEnd, Step};

#[cfg(test)]
use dasdevbot_core::Provider;
use rusqlite::OptionalExtension;

use crate::audit_log::{audit_tool_use_attempted, audit_tool_use_blocked};
use crate::store::Store;
use crate::{Error, Result};

pub struct HarnessRun<'a> {
    pub job_id: &'a str,
    pub lease_owner: Option<&'a str>,
    pub retry_cap: u32,
    pub now_ms: u64,
    pub audit_seed: &'a [u8; 32],
    pub cli_version: &'a Mutex<String>,
}

pub fn execute(
    store: &Mutex<Store>,
    run: HarnessRun<'_>,
    charge: impl FnMut() -> bool,
    attempt: impl FnMut() -> std::result::Result<String, ProviderStop>,
) -> Result<RunEnd> {
    let mut state = {
        let store = store.lock().expect("store");
        if let Some(saved) = load(&store, run.job_id)? {
            if saved.paused {
                return Ok(RunEnd::Paused {
                    attempts: 0,
                    reason: dasdevbot_core::PauseReason::Provider,
                });
            }
        }
        let mut state = HarnessState::wake();
        state = state.advance(Step::Begin).map_err(harness_err)?;
        state = state.advance(Step::Planned).map_err(harness_err)?;
        state = state.advance(Step::Checkpoint).map_err(harness_err)?;
        save(&store, run.job_id, &state, run.now_ms)?;
        state
    };
    let end = drive_provider(run.retry_cap, charge, attempt);
    let mut store = store.lock().expect("store");
    match &end {
        RunEnd::Paused { .. } => {
            state = state.on_limit();
            save(&store, run.job_id, &state, run.now_ms)?;
            fence_job(&store, run.job_id, run.lease_owner, "paused")?;
        }
        RunEnd::FailedClosed { tool_use: true, .. } => {
            audit_tool_use_attempted(&mut store, run.now_ms, run.audit_seed)?;
            let version = run.cli_version.lock().expect("cli version").clone();
            audit_tool_use_blocked(&mut store, run.now_ms, run.audit_seed, run.job_id, &version)?;
            fence_job(&store, run.job_id, run.lease_owner, "failed")?;
        }
        RunEnd::FailedClosed {
            tool_use: false, ..
        } => {
            fence_job(&store, run.job_id, run.lease_owner, "failed")?;
        }
        RunEnd::Done { .. } => {
            state = state.resume();
            state = state.advance(Step::Finish).unwrap_or(state);
            save(&store, run.job_id, &state, run.now_ms)?;
        }
    }
    Ok(end)
}

fn harness_err(_: dasdevbot_core::HarnessError) -> Error {
    Error::BadRequest("harness".into())
}

fn fence_job(store: &Store, job_id: &str, lease_owner: Option<&str>, status: &str) -> Result<()> {
    let current: Option<String> = store
        .connection()
        .query_row("SELECT status FROM jobs WHERE id = ?1", [job_id], |row| {
            row.get(0)
        })
        .optional()?;
    let Some(current) = current else {
        return Ok(());
    };
    if current == status {
        return Ok(());
    }
    let Some(owner) = lease_owner else {
        return Err(Error::Forbidden(
            "job status update requires the lease owner".into(),
        ));
    };
    let changed = store.connection().execute(
        "UPDATE jobs
         SET status = ?1, lease_owner = NULL, lease_until_ms = NULL
         WHERE id = ?2 AND lease_owner = ?3 AND status = 'leased'",
        rusqlite::params![status, job_id, owner],
    )?;
    if changed != 1 {
        return Err(Error::Forbidden("job status update lost the lease".into()));
    }
    Ok(())
}

fn save(store: &Store, job_id: &str, state: &HarnessState, now_ms: u64) -> Result<()> {
    store.connection().execute(
        "INSERT INTO harness_checkpoints (job_id, phase, paused, step, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(job_id) DO UPDATE SET
            phase = excluded.phase,
            paused = excluded.paused,
            step = excluded.step,
            updated_at = excluded.updated_at",
        rusqlite::params![
            job_id,
            state.phase.as_str(),
            i64::from(state.paused),
            state.step as i64,
            now_ms as i64,
        ],
    )?;
    Ok(())
}

pub fn load(store: &Store, job_id: &str) -> Result<Option<HarnessState>> {
    let row = store
        .connection()
        .query_row(
            "SELECT phase, paused, step FROM harness_checkpoints WHERE job_id = ?1",
            [job_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((phase, paused, step)) = row else {
        return Ok(None);
    };
    let Some(phase) = Phase::parse(&phase) else {
        return Err(Error::BadRequest("bad harness phase".into()));
    };
    Ok(Some(HarnessState {
        phase,
        paused: paused != 0,
        step: step.max(0) as u64,
    }))
}

#[cfg(test)]
struct Script {
    stops: Vec<ProviderStop>,
    calls: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
impl dasdevbot_core::Provider for Script {
    fn name(&self) -> &str {
        "fake"
    }

    fn limit_signal(&self) -> dasdevbot_core::SignalRead {
        dasdevbot_core::SignalRead::Absent
    }

    fn complete(&self, _prompt: &str) -> std::result::Result<String, ProviderStop> {
        let index = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        match self.stops.get(index) {
            Some(stop) => Err(*stop),
            None => Ok("[fake]".into()),
        }
    }
}

#[cfg(test)]
fn run(
    store: &Mutex<Store>,
    job_id: &str,
    provider: &Script,
    retry_cap: u32,
    now_ms: u64,
    mut charge: impl FnMut() -> bool,
) -> Result<RunEnd> {
    let version = Mutex::new("test-cli".into());
    execute(
        store,
        HarnessRun {
            job_id,
            lease_owner: None,
            retry_cap,
            now_ms,
            audit_seed: &[8u8; 32],
            cli_version: &version,
        },
        &mut charge,
        || provider.complete(""),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dasdevbot_core::RETRY_CAP;

    #[test]
    fn busy_and_retry_cap_pause_at_the_checkpoint_and_do_not_loop() {
        let store = Mutex::new(Store::open_memory().unwrap());
        let busy = Script {
            stops: vec![ProviderStop::Busy],
            calls: std::sync::atomic::AtomicUsize::new(0),
        };
        let mut charges = 0u32;
        let end = run(&store, "job-busy", &busy, RETRY_CAP, 10, || {
            charges += 1;
            true
        })
        .unwrap();
        assert!(matches!(end, RunEnd::Paused { attempts: 1, .. }));
        assert_eq!(charges, 1);
        assert_eq!(busy.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        let saved = load(&store.lock().unwrap(), "job-busy").unwrap().unwrap();
        assert!(saved.paused);
        assert_eq!(saved.phase, Phase::Checkpoint);
        let again = run(&store, "job-busy", &busy, RETRY_CAP, 20, || {
            charges += 1;
            true
        })
        .unwrap();
        assert!(matches!(again, RunEnd::Paused { attempts: 0, .. }));
        assert_eq!(busy.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(charges, 1);

        let retry = Script {
            stops: vec![
                ProviderStop::Retry,
                ProviderStop::Retry,
                ProviderStop::Retry,
            ],
            calls: std::sync::atomic::AtomicUsize::new(0),
        };
        let mut charges = 0u32;
        let end = run(&store, "job-retry", &retry, 2, 30, || {
            charges += 1;
            true
        })
        .unwrap();
        assert_eq!(charges, 2);
        assert_eq!(retry.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(matches!(end, RunEnd::Paused { attempts: 2, .. }));
        let saved = load(&store.lock().unwrap(), "job-retry").unwrap().unwrap();
        assert!(saved.paused);
        assert_eq!(saved.phase, Phase::Checkpoint);
    }

    #[test]
    fn tool_use_fails_closed_and_the_audit_has_no_content() {
        let dir = std::env::temp_dir().join(format!("dasdevbot-harness-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("db.sqlite");
        let store = Mutex::new(Store::open(&path).unwrap());
        let provider = Script {
            stops: vec![ProviderStop::ToolUseAttempted],
            calls: std::sync::atomic::AtomicUsize::new(0),
        };
        let end = run(&store, "job-tool", &provider, RETRY_CAP, 5, || true).unwrap();
        assert_eq!(
            end,
            RunEnd::FailedClosed {
                attempts: 1,
                tool_use: true,
            }
        );
        let payload: String = store
            .lock()
            .unwrap()
            .connection()
            .query_row(
                "SELECT payload FROM audit_log WHERE kind = 'provider.tool_use_attempted'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(payload.is_empty());
        let blocked: String = store
            .lock()
            .unwrap()
            .connection()
            .query_row(
                "SELECT payload FROM audit_log WHERE kind = 'provider.tool_use_blocked'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(blocked.contains("\"job_id\":\"job-tool\""), "{blocked}");
        assert!(
            blocked.contains("\"cli_version\":\"test-cli\""),
            "{blocked}"
        );
        assert!(!blocked.contains("SECRET"));
        drop(store);
        let store = Store::open(&path).unwrap();
        let saved = load(&store, "job-tool").unwrap().unwrap();
        assert_eq!(saved.phase, Phase::Checkpoint);
        assert!(!saved.paused);
    }

    #[test]
    fn a_status_update_without_the_lease_is_refused() {
        let store = Mutex::new(Store::open_memory().unwrap());
        let job_id = {
            let guard = store.lock().unwrap();
            guard
                .enqueue_job("reviewer", "job-lease", "{}", 1)
                .unwrap()
                .unwrap()
        };
        {
            let guard = store.lock().unwrap();
            guard.claim_at("holder", 2, 60_000).unwrap().unwrap();
        }
        let provider = Script {
            stops: vec![ProviderStop::Fault],
            calls: std::sync::atomic::AtomicUsize::new(0),
        };
        let version = std::sync::Mutex::new(String::new());
        let err = execute(
            &store,
            HarnessRun {
                job_id: &job_id,
                lease_owner: Some("other"),
                retry_cap: 1,
                now_ms: 3,
                audit_seed: &[8u8; 32],
                cli_version: &version,
            },
            || true,
            || provider.complete(""),
        )
        .unwrap_err();
        assert!(err.to_string().contains("lease"));
        let status = store.lock().unwrap().job_status(&job_id).unwrap();
        assert_eq!(status.as_deref(), Some("leased"));
    }
}
