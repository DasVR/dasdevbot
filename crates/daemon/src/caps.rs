//! Global per-provider ledger. One executor writes it. Phase 1 runs one
//! instance. Ollama windows are monthly; Claude reset comes from the signal.
//! A full slot cap waits as `waiting_on_slot`, not `waiting_on_quota`.

use dasdevbot_core::{
    admit, parse_role, Admission, AdmitInput, BudgetAccess, DenyReason, LedgerAccess,
    LedgerSnapshot, ProviderSlot, ReserveWrite, Role, SignalRead, TeammateBudget, WindowKind,
    WriterLease,
};
use rusqlite::{params, OptionalExtension};

use crate::store::Store;
use crate::Result;

const WRITER_TTL_MS: u64 = 30_000;
const MONTH_MS: u64 = 30 * 24 * 60 * 60 * 1000;

#[derive(Debug)]
enum CasError {
    Stale,
    Slot,
    Write,
}

pub struct AdmitRequest<'a> {
    pub role: &'a str,
    pub writer_id: &'a str,
    pub teammate_id: &'a str,
    pub providers: &'a [(&'a str, SignalRead)],
    pub estimate: Option<u64>,
    pub now_ms: u64,
    pub job_id: &'a str,
}

pub struct CapInstall<'a> {
    pub role: Role,
    pub writer_id: &'a str,
    pub provider: &'a str,
    pub cap_tokens: u64,
    pub window: WindowKind,
    pub reset_at_ms: Option<u64>,
    pub concurrency_cap: u32,
    pub now_ms: u64,
}

struct ReservationCommit<'a> {
    role: Role,
    writer_id: &'a str,
    provider: &'a str,
    epoch: Option<u64>,
    reserved: u64,
    now_ms: u64,
    job_id: &'a str,
    teammate_id: &'a str,
}

pub fn admit_job(store: &mut Store, request: AdmitRequest<'_>) -> Admission {
    let Some(role) = parse_role(request.role) else {
        return Admission::Deny {
            reason: DenyReason::Malformed,
        };
    };
    let teammate = match load_teammate(store, request.teammate_id) {
        Ok(budget) => BudgetAccess::Ready(budget),
        Err(_) => {
            return Admission::Deny {
                reason: DenyReason::BudgetStoreRead,
            };
        }
    };
    let mut slots = Vec::with_capacity(request.providers.len());
    for (provider, signal) in request.providers {
        match load_ledger(store, provider) {
            Ok(ledger) => slots.push(ProviderSlot {
                ledger,
                signal: *signal,
            }),
            Err(_) => {
                return Admission::Deny {
                    reason: DenyReason::BudgetStoreRead,
                };
            }
        }
    }
    let decision = admit(AdmitInput {
        estimate: request.estimate,
        teammate,
        providers: slots,
        writer_role: role,
        writer_id: request.writer_id.to_string(),
        now_ms: request.now_ms,
        reserve_write: ReserveWrite::Applied,
    });
    match decision {
        Admission::Admit {
            provider,
            reserved,
            headroom,
            epoch,
        } => match commit(
            store,
            ReservationCommit {
                role,
                writer_id: request.writer_id,
                provider: &provider,
                epoch,
                reserved,
                now_ms: request.now_ms,
                job_id: request.job_id,
                teammate_id: request.teammate_id,
            },
        ) {
            Ok(()) => Admission::Admit {
                provider,
                reserved,
                headroom,
                epoch,
            },
            Err(CasError::Stale) => Admission::Deny {
                reason: DenyReason::StaleEpoch,
            },
            Err(CasError::Slot) => Admission::WaitingOnSlot { provider },
            Err(CasError::Write) => Admission::Deny {
                reason: DenyReason::BudgetStoreWrite,
            },
        },
        other => other,
    }
}

pub fn install_cap(store: &mut Store, cap: CapInstall<'_>) -> Result<()> {
    if !dasdevbot_core::may_write_admission_ledger(cap.role) {
        return Err(crate::Error::Forbidden(
            "only the executor writes the admission ledger".into(),
        ));
    }
    let window_kind = match cap.window {
        WindowKind::Monthly => "monthly",
        WindowKind::SignalReset => "signal_reset",
    };
    store.connection().execute(
        "INSERT INTO provider_caps (
            provider, cap_tokens, spent_tokens, reserved_tokens, window_kind, reset_at_ms,
            concurrency_cap, in_flight, fencing_epoch, writer_id, writer_until_ms
         ) VALUES (?1, ?2, 0, 0, ?3, ?4, ?5, 0, 1, ?6, ?7)",
        params![
            cap.provider,
            cap.cap_tokens as i64,
            window_kind,
            cap.reset_at_ms.map(|ms| ms as i64),
            cap.concurrency_cap,
            cap.writer_id,
            cap.now_ms.saturating_add(WRITER_TTL_MS) as i64,
        ],
    )?;
    Ok(())
}

fn commit(store: &mut Store, held: ReservationCommit<'_>) -> std::result::Result<(), CasError> {
    if held.epoch.is_some() && !dasdevbot_core::may_write_admission_ledger(held.role) {
        return Err(CasError::Write);
    }
    let tx = store
        .connection_mut()
        .unchecked_transaction()
        .map_err(|_| CasError::Write)?;
    tx.execute(
        "INSERT INTO budget_reservations (job_id, teammate_id, provider, tokens)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            held.job_id,
            held.teammate_id,
            held.provider,
            held.reserved as i64
        ],
    )
    .map_err(|_| CasError::Write)?;
    if let Some(epoch) = held.epoch {
        let until = held.now_ms.saturating_add(WRITER_TTL_MS) as i64;
        let changed = tx
            .execute(
                "UPDATE provider_caps
                 SET reserved_tokens = CASE
                         WHEN window_kind = 'monthly' AND reset_at_ms IS NOT NULL AND reset_at_ms <= ?6
                         THEN ?1
                         ELSE reserved_tokens + ?1
                     END,
                     spent_tokens = CASE
                         WHEN window_kind = 'monthly' AND reset_at_ms IS NOT NULL AND reset_at_ms <= ?6
                         THEN 0
                         ELSE spent_tokens
                     END,
                     reset_at_ms = CASE
                         WHEN window_kind = 'monthly' AND reset_at_ms IS NOT NULL AND reset_at_ms <= ?6
                         THEN ?6 + ?7
                         ELSE reset_at_ms
                     END,
                     in_flight = in_flight + 1,
                     fencing_epoch = fencing_epoch + 1,
                     writer_id = ?2,
                     writer_until_ms = ?3
                 WHERE provider = ?4
                   AND fencing_epoch = ?5
                   AND in_flight < concurrency_cap
                   AND (writer_id = ?2 OR writer_until_ms <= ?6)",
                params![
                    held.reserved as i64,
                    held.writer_id,
                    until,
                    held.provider,
                    epoch as i64,
                    held.now_ms as i64,
                    MONTH_MS as i64,
                ],
            )
            .map_err(|_| CasError::Write)?;
        if changed != 1 {
            let state = tx
                .query_row(
                    "SELECT fencing_epoch, in_flight, concurrency_cap, writer_id, writer_until_ms
                     FROM provider_caps WHERE provider = ?1",
                    [held.provider],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, i64>(4)?,
                        ))
                    },
                )
                .map_err(|_| CasError::Write)?;
            let epoch_now = state.0 as u64;
            let in_flight = state.1 as u32;
            let cap = state.2 as u32;
            let holder = state.3;
            let until_ms = state.4 as u64;
            if epoch_now != epoch {
                return Err(CasError::Stale);
            }
            if in_flight >= cap {
                return Err(CasError::Slot);
            }
            if holder != held.writer_id && held.now_ms < until_ms {
                return Err(CasError::Stale);
            }
            return Err(CasError::Write);
        }
    }
    tx.commit().map_err(|_| CasError::Write)?;
    Ok(())
}

/// Hold `tokens` for one more attempt on the job's existing reservation.
///
/// The first attempt is held by [`admit_job`]. Each retry calls this with the
/// provider's worst-case estimate. `Ok(false)` means the hold did not fit;
/// the epoch is unchanged and the caller pauses. A lost fence is an error.
pub fn reserve_attempt(
    store: &mut Store,
    job_id: &str,
    writer_id: &str,
    tokens: u64,
    now_ms: u64,
) -> Result<bool> {
    if tokens == 0 {
        return Err(crate::Error::BadRequest(
            "an attempt reserve needs a worst-case estimate".into(),
        ));
    }
    let row = store
        .connection()
        .query_row(
            "SELECT provider FROM budget_reservations WHERE job_id = ?1",
            [job_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(provider) = row else {
        return Err(crate::Error::Forbidden("no reservation to extend".into()));
    };
    let hold = tokens as i64;
    let tx = store.connection_mut().unchecked_transaction()?;
    let epoch: i64 = tx
        .query_row(
            "SELECT fencing_epoch FROM provider_caps WHERE provider = ?1",
            [&provider],
            |row| row.get(0),
        )
        .map_err(|_| crate::Error::Forbidden("missing caps row".into()))?;
    let changed = tx.execute(
        "UPDATE provider_caps
         SET reserved_tokens = reserved_tokens + ?1,
             fencing_epoch = fencing_epoch + 1,
             writer_until_ms = ?2
         WHERE provider = ?3 AND fencing_epoch = ?4 AND writer_id = ?5
           AND cap_tokens - spent_tokens - reserved_tokens >= ?1",
        params![
            hold,
            now_ms.saturating_add(WRITER_TTL_MS) as i64,
            provider,
            epoch,
            writer_id,
        ],
    )?;
    if changed != 1 {
        let state: Option<(i64, String)> = tx
            .query_row(
                "SELECT fencing_epoch, writer_id FROM provider_caps WHERE provider = ?1",
                [&provider],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((epoch_now, holder)) = state else {
            return Err(crate::Error::Forbidden("missing caps row".into()));
        };
        if epoch_now != epoch || holder != writer_id {
            return Err(crate::Error::Forbidden(
                "ledger reserve lost the fencing epoch".into(),
            ));
        }
        return Ok(false);
    }
    let teammate: String = tx.query_row(
        "SELECT teammate_id FROM budget_reservations WHERE job_id = ?1",
        [job_id],
        |row| row.get(0),
    )?;
    charge_teammate(&tx, &teammate, hold)?;
    tx.execute(
        "UPDATE budget_reservations
         SET tokens = tokens + ?1, teammate_charged = teammate_charged + ?1
         WHERE job_id = ?2",
        params![hold, job_id],
    )?;
    tx.commit()?;
    Ok(true)
}

/// The call committed. Spend the measured tokens and release the unused hold.
pub fn commit_attempt(
    store: &mut Store,
    job_id: &str,
    writer_id: &str,
    actual: u64,
    now_ms: u64,
) -> Result<()> {
    let row = store
        .connection()
        .query_row(
            "SELECT provider, tokens FROM budget_reservations WHERE job_id = ?1",
            [job_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    let Some((provider, hold)) = row else {
        return Err(crate::Error::Forbidden("no reservation to commit".into()));
    };
    if hold <= 0 {
        return Err(crate::Error::Forbidden(
            "commit requires a held worst-case estimate".into(),
        ));
    }
    let actual = actual as i64;
    let tx = store.connection_mut().unchecked_transaction()?;
    let epoch: i64 = tx.query_row(
        "SELECT fencing_epoch FROM provider_caps WHERE provider = ?1",
        [&provider],
        |row| row.get(0),
    )?;
    let changed = tx.execute(
        "UPDATE provider_caps
         SET spent_tokens = spent_tokens + ?1,
             reserved_tokens = MAX(reserved_tokens - ?2, 0),
             fencing_epoch = fencing_epoch + 1,
             writer_until_ms = ?3
         WHERE provider = ?4 AND fencing_epoch = ?5 AND writer_id = ?6",
        params![
            actual,
            hold,
            now_ms.saturating_add(WRITER_TTL_MS) as i64,
            provider,
            epoch,
            writer_id,
        ],
    )?;
    if changed != 1 {
        return Err(crate::Error::Forbidden(
            "ledger commit lost the fencing epoch".into(),
        ));
    }
    tx.execute(
        "UPDATE budget_reservations SET tokens = 0, charged_tokens = ?1 WHERE job_id = ?2",
        params![actual, job_id],
    )?;
    tx.commit()?;
    Ok(())
}

/// The call did not commit. The held worst case stays spent so the attempt is not free.
pub fn settle_failed_attempt(
    store: &mut Store,
    job_id: &str,
    writer_id: &str,
    now_ms: u64,
) -> Result<()> {
    let row = store
        .connection()
        .query_row(
            "SELECT provider, tokens, teammate_id, teammate_charged FROM budget_reservations WHERE job_id = ?1",
            [job_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((provider, hold, teammate, teammate_charged)) = row else {
        return Ok(());
    };
    if hold <= 0 {
        return Ok(());
    }
    let tx = store.connection_mut().unchecked_transaction()?;
    let epoch: i64 = tx.query_row(
        "SELECT fencing_epoch FROM provider_caps WHERE provider = ?1",
        [&provider],
        |row| row.get(0),
    )?;
    let changed = tx.execute(
        "UPDATE provider_caps
         SET spent_tokens = spent_tokens + ?1,
             reserved_tokens = MAX(reserved_tokens - ?1, 0),
             fencing_epoch = fencing_epoch + 1,
             writer_until_ms = ?2
         WHERE provider = ?3 AND fencing_epoch = ?4 AND writer_id = ?5",
        params![
            hold,
            now_ms.saturating_add(WRITER_TTL_MS) as i64,
            provider,
            epoch,
            writer_id,
        ],
    )?;
    if changed != 1 {
        return Err(crate::Error::Forbidden(
            "ledger settle lost the fencing epoch".into(),
        ));
    }
    let uncharged = hold.saturating_sub(teammate_charged);
    charge_teammate(&tx, &teammate, uncharged)?;
    tx.execute(
        "UPDATE budget_reservations
         SET tokens = 0, charged_tokens = charged_tokens + ?1, teammate_charged = 0
         WHERE job_id = ?2",
        params![hold, job_id],
    )?;
    tx.commit()?;
    Ok(())
}

fn charge_teammate(tx: &rusqlite::Transaction<'_>, teammate_id: &str, tokens: i64) -> Result<()> {
    if tokens <= 0 {
        return Ok(());
    }
    tx.execute(
        "UPDATE agents SET tokens_spent = tokens_spent + ?1 WHERE id = ?2",
        params![tokens, teammate_id],
    )?;
    Ok(())
}

pub fn release_reservation(
    store: &mut Store,
    job_id: &str,
    writer_id: &str,
    now_ms: u64,
) -> Result<()> {
    let row = store
        .connection()
        .query_row(
            "SELECT provider, tokens FROM budget_reservations WHERE job_id = ?1",
            [job_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    let Some((provider, tokens)) = row else {
        return Ok(());
    };
    let tx = store.connection_mut().unchecked_transaction()?;
    let epoch: Option<i64> = tx
        .query_row(
            "SELECT fencing_epoch FROM provider_caps WHERE provider = ?1",
            [&provider],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(epoch) = epoch {
        let changed = tx.execute(
            "UPDATE provider_caps
             SET reserved_tokens = MAX(reserved_tokens - ?1, 0),
                 in_flight = MAX(in_flight - 1, 0),
                 fencing_epoch = fencing_epoch + 1,
                 writer_until_ms = ?2
             WHERE provider = ?3 AND fencing_epoch = ?4 AND writer_id = ?5",
            params![
                tokens,
                now_ms.saturating_add(WRITER_TTL_MS) as i64,
                provider,
                epoch,
                writer_id,
            ],
        )?;
        if changed != 1 {
            return Err(crate::Error::Forbidden(
                "ledger release lost the fencing epoch".into(),
            ));
        }
    }
    tx.execute(
        "DELETE FROM budget_reservations WHERE job_id = ?1",
        [job_id],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn park_job(
    store: &Store,
    job_id: &str,
    owner: &str,
    status: &str,
    provider: &str,
    until_ms: Option<u64>,
) -> Result<bool> {
    let changed = store.connection().execute(
        "UPDATE jobs
         SET status = ?1, lease_owner = NULL, lease_until_ms = NULL,
             admit_provider = ?2, quota_until_ms = ?3
         WHERE id = ?4 AND lease_owner = ?5 AND status = 'leased'",
        params![
            status,
            provider,
            until_ms.map(|ms| ms as i64),
            job_id,
            owner
        ],
    )?;
    Ok(changed == 1)
}

pub fn release_quota_waits(store: &Store, now_ms: u64) -> Result<()> {
    store.connection().execute(
        "UPDATE jobs
         SET status = 'pending', quota_until_ms = NULL
         WHERE status = 'waiting_on_quota'
           AND quota_until_ms IS NOT NULL
           AND quota_until_ms <= ?1",
        [now_ms as i64],
    )?;
    Ok(())
}

pub fn release_slot_waits(store: &Store) -> Result<()> {
    store.connection().execute(
        "UPDATE jobs
         SET status = 'pending'
         WHERE status = 'waiting_on_slot'
           AND admit_provider IN (
               SELECT provider FROM provider_caps WHERE in_flight < concurrency_cap
           )",
        [],
    )?;
    Ok(())
}

fn load_teammate(store: &Store, teammate_id: &str) -> Result<TeammateBudget> {
    let row = store.connection().query_row(
        "SELECT token_cap, tokens_spent, COALESCE(conservative_max, token_cap),
                (SELECT COALESCE(SUM(tokens), 0) FROM budget_reservations WHERE teammate_id = ?1)
         FROM agents WHERE id = ?1",
        [teammate_id],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        },
    )?;
    Ok(TeammateBudget {
        cap: row.0.max(0) as u64,
        spent: row.1.max(0) as u64,
        reserved: row.3.max(0) as u64,
        conservative_max: row.2.max(0) as u64,
    })
}

fn load_ledger(store: &Store, provider: &str) -> Result<LedgerAccess> {
    let row = store
        .connection()
        .query_row(
            "SELECT cap_tokens, spent_tokens, reserved_tokens, window_kind, reset_at_ms,
                    concurrency_cap, in_flight, fencing_epoch, writer_id, writer_until_ms
             FROM provider_caps WHERE provider = ?1",
            [provider],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, i64>(9)?,
                ))
            },
        )
        .optional()?;
    let Some(row) = row else {
        return Ok(LedgerAccess::NotConfigured {
            provider: provider.to_string(),
        });
    };
    let window = match row.3.as_str() {
        "monthly" => WindowKind::Monthly,
        "signal_reset" => WindowKind::SignalReset,
        _ => return Err(crate::Error::BadRequest("unknown ledger window".into())),
    };
    Ok(LedgerAccess::Ready(LedgerSnapshot {
        provider: provider.to_string(),
        cap: row.0.max(0) as u64,
        spent: row.1.max(0) as u64,
        reserved: row.2.max(0) as u64,
        window,
        reset_at_ms: row.4.map(|ms| ms.max(0) as u64),
        concurrency_cap: row.5.max(0) as u32,
        in_flight: row.6.max(0) as u32,
        writer: WriterLease {
            epoch: row.7.max(0) as u64,
            writer_id: row.8,
            until_ms: row.9.max(0) as u64,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dasdevbot_core::SignalRead;

    fn memory() -> Store {
        Store::open_memory().unwrap()
    }

    fn request<'a>(
        writer: &'a str,
        providers: &'a [(&'a str, SignalRead)],
        estimate: u64,
        now: u64,
        job: &'a str,
    ) -> AdmitRequest<'a> {
        AdmitRequest {
            role: "executor",
            writer_id: writer,
            teammate_id: "mate",
            providers,
            estimate: Some(estimate),
            now_ms: now,
            job_id: job,
        }
    }

    fn cap<'a>(role: Role, writer: &'a str, slots: u32) -> CapInstall<'a> {
        CapInstall {
            role,
            writer_id: writer,
            provider: "ollama-fake",
            cap_tokens: 10_000,
            window: WindowKind::Monthly,
            reset_at_ms: Some(90_000),
            concurrency_cap: slots,
            now_ms: 1_000,
        }
    }

    #[test]
    fn fencing_epoch_stops_a_second_writer_and_slots_are_separate_from_quota() {
        let mut store = memory();
        store
            .insert_agent("mate", "Teammate", "persona", "proj", 100_000)
            .unwrap();
        let err = install_cap(&mut store, cap(Role::Leader, "leader", 1));
        assert!(err.is_err());
        install_cap(&mut store, cap(Role::Executor, "executor-1", 1)).unwrap();

        let first = admit_job(
            &mut store,
            request(
                "executor-1",
                &[("ollama-fake", SignalRead::Absent)],
                100,
                1_000,
                "job-a",
            ),
        );
        assert!(first.starts(), "{first:?}");

        let second = admit_job(
            &mut store,
            request(
                "executor-1",
                &[("ollama-fake", SignalRead::Absent)],
                100,
                1_100,
                "job-b",
            ),
        );
        assert_eq!(
            second,
            Admission::WaitingOnSlot {
                provider: "ollama-fake".into(),
            }
        );

        let stale = admit_job(
            &mut store,
            request(
                "executor-2",
                &[("ollama-fake", SignalRead::Absent)],
                100,
                1_200,
                "job-c",
            ),
        );
        assert!(!stale.starts(), "{stale:?}");
        assert!(!matches!(stale, Admission::WaitingOnQuota { .. }));

        release_reservation(&mut store, "job-a", "executor-1", 1_300).unwrap();
        release_slot_waits(&store).unwrap();
        let third = admit_job(
            &mut store,
            request(
                "executor-1",
                &[("ollama-fake", SignalRead::Failed)],
                100,
                1_400,
                "job-d",
            ),
        );
        assert!(third.starts(), "{third:?}");
    }

    #[test]
    fn provider_cap_table_has_no_device_column() {
        let store = memory();
        let mut stmt = store
            .connection()
            .prepare("PRAGMA table_info(provider_caps)")
            .unwrap();
        let names: Vec<String> = stmt
            .query_map([], |row| row.get(1))
            .unwrap()
            .map(|name| name.unwrap())
            .collect();
        assert!(names.contains(&"fencing_epoch".to_string()));
        assert!(names.contains(&"window_kind".to_string()));
        assert!(names.contains(&"concurrency_cap".to_string()));
        assert!(!names.iter().any(|name| name.contains("device")));
    }

    #[test]
    fn a_missing_caps_row_denies_and_spend_is_charged_then_reconciled() {
        let mut store = memory();
        store
            .insert_agent("mate", "Teammate", "persona", "proj", 100_000)
            .unwrap();
        let missing = admit_job(
            &mut store,
            request(
                "executor-1",
                &[("no-such-provider", SignalRead::Absent)],
                100,
                1_000,
                "job-missing",
            ),
        );
        assert_eq!(
            missing,
            Admission::Deny {
                reason: DenyReason::NotConfigured,
            }
        );

        install_cap(&mut store, cap(Role::Executor, "executor-1", 2)).unwrap();
        let admitted = admit_job(
            &mut store,
            request(
                "executor-1",
                &[("ollama-fake", SignalRead::Absent)],
                700,
                1_000,
                "job-spend",
            ),
        );
        assert!(admitted.starts(), "{admitted:?}");
        let worst = 700i64;
        let (spent, reserved): (i64, i64) = store
            .connection()
            .query_row(
                "SELECT spent_tokens, reserved_tokens FROM provider_caps WHERE provider = 'ollama-fake'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(spent, 0);
        assert_eq!(reserved, worst);

        commit_attempt(&mut store, "job-spend", "executor-1", 40, 1_100).unwrap();
        let (spent, reserved, held): (i64, i64, i64) = store
            .connection()
            .query_row(
                "SELECT caps.spent_tokens, caps.reserved_tokens, res.tokens
                 FROM provider_caps caps, budget_reservations res
                 WHERE caps.provider = 'ollama-fake' AND res.job_id = 'job-spend'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(spent, 40);
        assert_eq!(reserved, 0);
        assert_eq!(held, 0);

        assert!(reserve_attempt(&mut store, "job-spend", "executor-1", 700, 1_200).unwrap());
        let reserved: i64 = store
            .connection()
            .query_row(
                "SELECT reserved_tokens FROM provider_caps WHERE provider = 'ollama-fake'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(reserved, worst);

        store
            .connection()
            .execute(
                "UPDATE provider_caps SET spent_tokens = cap_tokens - 100 WHERE provider = 'ollama-fake'",
                [],
            )
            .unwrap();
        assert!(!reserve_attempt(&mut store, "job-spend", "executor-1", 700, 1_250).unwrap());
        let reserved_after: i64 = store
            .connection()
            .query_row(
                "SELECT reserved_tokens FROM provider_caps WHERE provider = 'ollama-fake'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(reserved_after, worst);

        release_reservation(&mut store, "job-spend", "executor-1", 1_300).unwrap();
        let (spent, in_flight): (i64, i64) = store
            .connection()
            .query_row(
                "SELECT spent_tokens, in_flight FROM provider_caps WHERE provider = 'ollama-fake'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(spent, 9_900);
        assert_eq!(in_flight, 0);
    }
}
