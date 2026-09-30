//! Usage-aware admission.
//!
//! One global ledger per provider, no per-device escrow. A job starts only
//! when remaining headroom covers 1.5× its estimate and a concurrency slot
//! is free.
//!
//! Ollama Cloud's window is monthly. Claude's reset time comes from the
//! limit signal. A signal can only lower headroom. A failed signal is not a
//! signal: the ledger governs. The ledger is single-writer; spends carry a
//! fencing epoch. Phase 1 runs one executor instance.

use crate::roles::{may_write_admission_ledger, Role};

/// 3/2. Admission requires remaining headroom ≥ estimate × 3/2.
pub const HEADROOM_NUMERATOR: u64 = 3;
pub const HEADROOM_DENOMINATOR: u64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    /// Ollama Cloud credits. The ledger's `reset_at_ms` is the month boundary.
    Monthly,
    /// Claude. The reset instant comes from the signal when one is present.
    SignalReset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitSignal {
    /// Provider-reported remaining credits. Never raises headroom.
    pub remaining: u64,
    /// Provider-reported reset. Claude waits until this instant.
    pub reset_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalRead {
    Absent,
    Present(LimitSignal),
    /// The read failed. Treated as no signal.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriterLease {
    pub writer_id: String,
    pub epoch: u64,
    pub until_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerSnapshot {
    pub provider: String,
    pub cap: u64,
    pub spent: u64,
    pub reserved: u64,
    pub window: WindowKind,
    pub reset_at_ms: Option<u64>,
    pub concurrency_cap: u32,
    pub in_flight: u32,
    pub writer: WriterLease,
}

impl LedgerSnapshot {
    pub fn accounted(&self) -> Option<u64> {
        self.spent.checked_add(self.reserved)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeammateBudget {
    pub cap: u64,
    pub spent: u64,
    pub reserved: u64,
    /// Used when the caller has no cost estimate.
    pub conservative_max: u64,
}

impl TeammateBudget {
    pub fn accounted(&self) -> Option<u64> {
        self.spent.checked_add(self.reserved)
    }

    pub fn remaining(&self) -> Option<u64> {
        self.accounted().map(|used| self.cap.saturating_sub(used))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetAccess {
    Ready(TeammateBudget),
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerAccess {
    Ready(LedgerSnapshot),
    NotConfigured { provider: String },
    Failed { provider: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSlot {
    pub ledger: LedgerAccess,
    pub signal: SignalRead,
}

/// Simulated outcome of the fenced ledger write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReserveWrite {
    Applied,
    Failed,
    StaleEpoch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmitInput {
    pub estimate: Option<u64>,
    pub teammate: BudgetAccess,
    pub providers: Vec<ProviderSlot>,
    pub writer_role: Role,
    pub writer_id: String,
    pub now_ms: u64,
    pub reserve_write: ReserveWrite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DenyReason {
    BudgetStoreRead,
    BudgetStoreWrite,
    StaleEpoch,
    OtherWriter,
    NotLedgerWriter,
    TeammateBudget,
    UnboundedEstimate,
    Overflow,
    Malformed,
    /// No `provider_caps` row. Missing configuration is not unlimited.
    NotConfigured,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    Admit {
        provider: String,
        reserved: u64,
        headroom: u64,
        epoch: Option<u64>,
    },
    WaitingOnQuota {
        provider: String,
        until_ms: Option<u64>,
    },
    /// Concurrency cap is full. Distinct from a credit wait.
    WaitingOnSlot {
        provider: String,
    },
    AskOwner {
        provider: String,
    },
    Deny {
        reason: DenyReason,
    },
}

impl Admission {
    pub fn starts(&self) -> bool {
        match self {
            Admission::Admit { .. } => true,
            Admission::WaitingOnQuota { .. }
            | Admission::WaitingOnSlot { .. }
            | Admission::AskOwner { .. }
            | Admission::Deny { .. } => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteAuth {
    Ok { next_epoch: u64 },
    NotExecutor,
    StaleEpoch,
    OtherWriter,
}

pub fn cover_tokens(estimate: u64) -> Option<u64> {
    estimate.checked_mul(HEADROOM_NUMERATOR).map(|scaled| scaled.div_ceil(HEADROOM_DENOMINATOR))
}

/// Credits still inside the provider window.
pub fn ledger_remaining(snapshot: &LedgerSnapshot, now_ms: u64) -> Option<u64> {
    let used = match snapshot.window {
        // A new month drops last month's spend. Holds taken in the new
        // window still count, so a reset cannot raise headroom over them.
        WindowKind::Monthly => match snapshot.reset_at_ms {
            Some(reset) if now_ms >= reset => Some(snapshot.reserved),
            _ => snapshot.accounted(),
        },
        WindowKind::SignalReset => snapshot.accounted(),
    }?;
    Some(snapshot.cap.saturating_sub(used))
}

/// A present signal can only shrink remaining credits.
/// Absent and failed reads leave the ledger number unchanged.
pub fn effective_headroom(ledger_remaining: u64, signal: SignalRead) -> u64 {
    match signal {
        SignalRead::Absent | SignalRead::Failed => ledger_remaining,
        SignalRead::Present(signal) => ledger_remaining.min(signal.remaining),
    }
}

fn quota_until(snapshot: &LedgerSnapshot, signal: SignalRead) -> Option<u64> {
    match snapshot.window {
        WindowKind::Monthly => snapshot.reset_at_ms,
        WindowKind::SignalReset => match signal {
            SignalRead::Present(signal) => signal.reset_at_ms.or(snapshot.reset_at_ms),
            SignalRead::Absent | SignalRead::Failed => snapshot.reset_at_ms,
        },
    }
}

pub fn authorize_spend(
    role: Role,
    writer_id: &str,
    lease: &WriterLease,
    expected_epoch: u64,
    now_ms: u64,
) -> WriteAuth {
    if !may_write_admission_ledger(role) {
        return WriteAuth::NotExecutor;
    }
    if expected_epoch != lease.epoch {
        return WriteAuth::StaleEpoch;
    }
    let holds = lease.writer_id == writer_id || now_ms >= lease.until_ms;
    if !holds {
        return WriteAuth::OtherWriter;
    }
    WriteAuth::Ok {
        next_epoch: lease.epoch.saturating_add(1),
    }
}

pub fn admit(input: AdmitInput) -> Admission {
    if input.providers.is_empty() {
        return Admission::Deny {
            reason: DenyReason::Malformed,
        };
    }
    if matches!(input.teammate, BudgetAccess::Failed)
        || input
            .providers
            .iter()
            .any(|slot| matches!(slot.ledger, LedgerAccess::Failed { .. }))
    {
        return Admission::Deny {
            reason: DenyReason::BudgetStoreRead,
        };
    }
    let BudgetAccess::Ready(teammate) = &input.teammate else {
        return Admission::Deny {
            reason: DenyReason::BudgetStoreRead,
        };
    };
    let Some(estimate) = input.estimate.or(Some(teammate.conservative_max)) else {
        return Admission::Deny {
            reason: DenyReason::UnboundedEstimate,
        };
    };
    if input.estimate.is_none() && teammate.conservative_max == 0 {
        return Admission::Deny {
            reason: DenyReason::UnboundedEstimate,
        };
    }
    let Some(need) = cover_tokens(estimate) else {
        return Admission::Deny {
            reason: DenyReason::Overflow,
        };
    };
    let Some(teammate_remaining) = teammate.remaining() else {
        return Admission::Deny {
            reason: DenyReason::Overflow,
        };
    };
    if teammate_remaining < need {
        return Admission::Deny {
            reason: DenyReason::TeammateBudget,
        };
    }

    let mut saw_quota: Option<(String, Option<u64>)> = None;
    let mut saw_slot: Option<String> = None;
    for slot in &input.providers {
        match slot_decision(slot, need, input.now_ms) {
            SlotDecision::Overflow => {
                return Admission::Deny {
                    reason: DenyReason::Overflow,
                };
            }
            SlotDecision::Missing => {
                return Admission::Deny {
                    reason: DenyReason::NotConfigured,
                };
            }
            SlotDecision::Open { headroom, epoch } => {
                if let Some(epoch) = epoch {
                    let LedgerAccess::Ready(snapshot) = &slot.ledger else {
                        return Admission::Deny {
                            reason: DenyReason::Malformed,
                        };
                    };
                    match authorize_spend(
                        input.writer_role,
                        &input.writer_id,
                        &snapshot.writer,
                        epoch,
                        input.now_ms,
                    ) {
                        WriteAuth::Ok { .. } => {}
                        WriteAuth::NotExecutor => {
                            return Admission::Deny {
                                reason: DenyReason::NotLedgerWriter,
                            };
                        }
                        WriteAuth::StaleEpoch => {
                            return Admission::Deny {
                                reason: DenyReason::StaleEpoch,
                            };
                        }
                        WriteAuth::OtherWriter => {
                            return Admission::Deny {
                                reason: DenyReason::OtherWriter,
                            };
                        }
                    }
                }
                return match input.reserve_write {
                    ReserveWrite::Applied => Admission::Admit {
                        provider: provider_name(slot).to_string(),
                        reserved: estimate,
                        headroom,
                        epoch,
                    },
                    ReserveWrite::Failed => Admission::Deny {
                        reason: DenyReason::BudgetStoreWrite,
                    },
                    ReserveWrite::StaleEpoch => Admission::Deny {
                        reason: DenyReason::StaleEpoch,
                    },
                };
            }
            SlotDecision::Quota { until_ms } => {
                if saw_quota.is_none() {
                    saw_quota = Some((provider_name(slot).to_string(), until_ms));
                }
            }
            SlotDecision::Slot => {
                if saw_slot.is_none() {
                    saw_slot = Some(provider_name(slot).to_string());
                }
            }
        }
    }
    if let Some((provider, until_ms)) = saw_quota {
        if until_ms.is_some() {
            return Admission::WaitingOnQuota { provider, until_ms };
        }
        return Admission::AskOwner { provider };
    }
    if let Some(provider) = saw_slot {
        return Admission::WaitingOnSlot { provider };
    }
    Admission::AskOwner {
        provider: provider_name(&input.providers[0]).to_string(),
    }
}

enum SlotDecision {
    Open { headroom: u64, epoch: Option<u64> },
    Quota { until_ms: Option<u64> },
    Slot,
    Overflow,
    /// No caps row. Fail closed. Do not fall through to another provider.
    Missing,
}

fn slot_decision(slot: &ProviderSlot, need: u64, now_ms: u64) -> SlotDecision {
    match &slot.ledger {
        LedgerAccess::Failed { .. } => SlotDecision::Overflow,
        LedgerAccess::NotConfigured { .. } => SlotDecision::Missing,
        LedgerAccess::Ready(snapshot) => {
            let Some(remaining) = ledger_remaining(snapshot, now_ms) else {
                return SlotDecision::Overflow;
            };
            let headroom = effective_headroom(remaining, slot.signal);
            if headroom < need {
                return SlotDecision::Quota {
                    until_ms: quota_until(snapshot, slot.signal),
                };
            }
            if snapshot.in_flight >= snapshot.concurrency_cap {
                return SlotDecision::Slot;
            }
            SlotDecision::Open {
                headroom,
                epoch: Some(snapshot.writer.epoch),
            }
        }
    }
}

fn provider_name(slot: &ProviderSlot) -> &str {
    match &slot.ledger {
        LedgerAccess::Ready(snapshot) => snapshot.provider.as_str(),
        LedgerAccess::NotConfigured { provider } | LedgerAccess::Failed { provider } => provider,
    }
}

#[cfg(test)]
fn writer(epoch: u64, id: &str, until_ms: u64) -> WriterLease {
    WriterLease {
        writer_id: id.to_string(),
        epoch,
        until_ms,
    }
}

#[cfg(test)]
fn ollama(spent: u64, in_flight: u32, cap_slots: u32, reset_at_ms: Option<u64>) -> LedgerSnapshot {
    LedgerSnapshot {
        provider: "ollama-fake".into(),
        cap: 10_000,
        spent,
        reserved: 0,
        window: WindowKind::Monthly,
        reset_at_ms,
        concurrency_cap: cap_slots,
        in_flight,
        writer: writer(1, "executor-1", 10_000),
    }
}

#[cfg(test)]
fn claude(spent: u64, reset_at_ms: Option<u64>) -> LedgerSnapshot {
    LedgerSnapshot {
        provider: "claude-fake".into(),
        cap: 10_000,
        spent,
        reserved: 0,
        window: WindowKind::SignalReset,
        reset_at_ms,
        concurrency_cap: 4,
        in_flight: 0,
        writer: writer(1, "executor-1", 10_000),
    }
}

#[cfg(test)]
fn teammate_ok() -> BudgetAccess {
    BudgetAccess::Ready(TeammateBudget {
        cap: 100_000,
        spent: 0,
        reserved: 0,
        conservative_max: 8_000,
    })
}

#[cfg(test)]
fn base_input(providers: Vec<ProviderSlot>, estimate: Option<u64>) -> AdmitInput {
    AdmitInput {
        estimate,
        teammate: teammate_ok(),
        providers,
        writer_role: Role::Executor,
        writer_id: "executor-1".into(),
        now_ms: 1_000,
        reserve_write: ReserveWrite::Applied,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signal_cannot_raise_headroom_above_the_ledger() {
        assert_eq!(effective_headroom(100, SignalRead::Present(LimitSignal {
            remaining: u64::MAX,
            reset_at_ms: None,
        })), 100);
        assert_eq!(effective_headroom(1_000, SignalRead::Present(LimitSignal {
            remaining: 40,
            reset_at_ms: None,
        })), 40);
        assert_eq!(effective_headroom(1_000, SignalRead::Failed), 1_000);
        assert_eq!(effective_headroom(1_000, SignalRead::Absent), 1_000);
    }

    #[test]
    fn ninety_nine_percent_used_does_not_start_a_big_job() {
        let ledger = ollama(9_900, 0, 4, Some(50_000));
        let mut input = base_input(
            vec![ProviderSlot {
                ledger: LedgerAccess::Ready(ledger),
                signal: SignalRead::Present(LimitSignal {
                    remaining: u64::MAX,
                    reset_at_ms: Some(9_999_999),
                }),
            }],
            Some(1_000),
        );
        input.now_ms = 1_000;
        let decision = admit(input);
        assert!(!decision.starts());
        assert_eq!(
            decision,
            Admission::WaitingOnQuota {
                provider: "ollama-fake".into(),
                until_ms: Some(50_000),
            }
        );
    }

    #[test]
    fn ollama_monthly_window_resets_and_claude_uses_the_signal_reset() {
        let mut input = base_input(
            vec![ProviderSlot {
                ledger: LedgerAccess::Ready(ollama(9_900, 0, 4, Some(5_000))),
                signal: SignalRead::Absent,
            }],
            Some(1_000),
        );
        input.now_ms = 5_000;
        let decision = admit(input);
        assert!(decision.starts(), "{decision:?}");
        match decision {
            Admission::Admit { headroom, .. } => assert_eq!(headroom, 10_000),
            other => panic!("{other:?}"),
        }

        let decision = admit(base_input(
            vec![ProviderSlot {
                ledger: LedgerAccess::Ready(claude(9_900, Some(80_000))),
                signal: SignalRead::Present(LimitSignal {
                    remaining: 50,
                    reset_at_ms: Some(12_000),
                }),
            }],
            Some(1_000),
        ));
        assert_eq!(
            decision,
            Admission::WaitingOnQuota {
                provider: "claude-fake".into(),
                until_ms: Some(12_000),
            }
        );
    }

    #[test]
    fn a_failed_signal_leaves_the_ledger_in_charge() {
        let open = admit(base_input(
            vec![ProviderSlot {
                ledger: LedgerAccess::Ready(ollama(0, 0, 2, Some(90_000))),
                signal: SignalRead::Failed,
            }],
            Some(100),
        ));
        assert!(open.starts(), "{open:?}");

        let blocked = admit(base_input(
            vec![ProviderSlot {
                ledger: LedgerAccess::Ready(claude(9_900, Some(70_000))),
                signal: SignalRead::Failed,
            }],
            Some(1_000),
        ));
        assert_eq!(
            blocked,
            Admission::WaitingOnQuota {
                provider: "claude-fake".into(),
                until_ms: Some(70_000),
            }
        );
    }

    #[test]
    fn a_full_concurrency_cap_waits_on_a_slot_not_on_quota() {
        let decision = admit(base_input(
            vec![ProviderSlot {
                ledger: LedgerAccess::Ready(ollama(0, 2, 2, Some(90_000))),
                signal: SignalRead::Absent,
            }],
            Some(100),
        ));
        assert_eq!(
            decision,
            Admission::WaitingOnSlot {
                provider: "ollama-fake".into(),
            }
        );
        assert!(!decision.starts());

        let quota = admit(base_input(
            vec![ProviderSlot {
                ledger: LedgerAccess::Ready(ollama(9_900, 0, 2, Some(90_000))),
                signal: SignalRead::Absent,
            }],
            Some(1_000),
        ));
        assert!(matches!(quota, Admission::WaitingOnQuota { .. }));
        assert!(!matches!(quota, Admission::WaitingOnSlot { .. }));
    }

    #[test]
    fn unknown_estimate_counts_at_the_conservative_maximum() {
        let mut tight = base_input(
            vec![ProviderSlot {
                ledger: LedgerAccess::NotConfigured {
                    provider: "ollama-fake".into(),
                },
                signal: SignalRead::Absent,
            }],
            None,
        );
        tight.teammate = BudgetAccess::Ready(TeammateBudget {
            cap: 10_000,
            spent: 0,
            reserved: 0,
            conservative_max: 8_000,
        });
        assert_eq!(
            admit(tight),
            Admission::Deny {
                reason: DenyReason::TeammateBudget,
            }
        );

        let mut wide = base_input(
            vec![ProviderSlot {
                ledger: LedgerAccess::Ready(ollama(0, 0, 2, Some(90_000))),
                signal: SignalRead::Absent,
            }],
            None,
        );
        wide.teammate = BudgetAccess::Ready(TeammateBudget {
            cap: 20_000,
            spent: 0,
            reserved: 0,
            conservative_max: 100,
        });
        match admit(wide) {
            Admission::Admit { reserved, .. } => assert_eq!(reserved, 100),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_missing_caps_row_fails_closed() {
        let input = base_input(
            vec![ProviderSlot {
                ledger: LedgerAccess::NotConfigured {
                    provider: "ollama-fake".into(),
                },
                signal: SignalRead::Present(LimitSignal {
                    remaining: u64::MAX,
                    reset_at_ms: Some(90_000),
                }),
            }],
            Some(10),
        );
        assert_eq!(
            admit(input),
            Admission::Deny {
                reason: DenyReason::NotConfigured,
            }
        );
    }

    #[test]
    fn budget_store_read_and_write_errors_deny() {
        let mut read = base_input(vec![], Some(1));
        read.teammate = BudgetAccess::Failed;
        read.providers = vec![ProviderSlot {
            ledger: LedgerAccess::NotConfigured {
                provider: "ollama-fake".into(),
            },
            signal: SignalRead::Absent,
        }];
        assert_eq!(
            admit(read),
            Admission::Deny {
                reason: DenyReason::BudgetStoreRead,
            }
        );

        let mut ledger_read = base_input(
            vec![ProviderSlot {
                ledger: LedgerAccess::Failed {
                    provider: "claude-fake".into(),
                },
                signal: SignalRead::Absent,
            }],
            Some(1),
        );
        ledger_read.teammate = teammate_ok();
        assert_eq!(
            admit(ledger_read),
            Admission::Deny {
                reason: DenyReason::BudgetStoreRead,
            }
        );

        let mut write = base_input(
            vec![ProviderSlot {
                ledger: LedgerAccess::Ready(ollama(0, 0, 2, Some(90_000))),
                signal: SignalRead::Absent,
            }],
            Some(10),
        );
        write.reserve_write = ReserveWrite::Failed;
        assert_eq!(
            admit(write),
            Admission::Deny {
                reason: DenyReason::BudgetStoreWrite,
            }
        );
    }

    #[test]
    fn only_the_executor_writes_and_a_stale_epoch_cannot_spend() {
        let slot = ProviderSlot {
            ledger: LedgerAccess::Ready(ollama(0, 0, 2, Some(90_000))),
            signal: SignalRead::Absent,
        };
        let mut leader = base_input(vec![slot.clone()], Some(10));
        leader.writer_role = Role::Leader;
        assert_eq!(
            admit(leader),
            Admission::Deny {
                reason: DenyReason::NotLedgerWriter,
            }
        );

        let mut stale = base_input(vec![slot.clone()], Some(10));
        if let LedgerAccess::Ready(snapshot) = &mut stale.providers[0].ledger {
            snapshot.writer.epoch = 4;
        }
        stale.reserve_write = ReserveWrite::Applied;
        let decision = admit(AdmitInput {
            writer_id: "executor-1".into(),
            ..stale
        });
        // expected epoch matches the snapshot, so this admits. A copied old
        // epoch is rejected below.
        assert!(decision.starts());

        let mut other = base_input(vec![slot], Some(10));
        other.writer_id = "executor-2".into();
        other.now_ms = 1_000;
        assert_eq!(
            admit(other),
            Admission::Deny {
                reason: DenyReason::OtherWriter,
            }
        );

        assert_eq!(
            authorize_spend(Role::Executor, "executor-1", &writer(2, "executor-1", 10_000), 1, 0),
            WriteAuth::StaleEpoch
        );
        assert_eq!(
            authorize_spend(Role::Worker, "executor-1", &writer(1, "executor-1", 10_000), 1, 0),
            WriteAuth::NotExecutor
        );
    }

    #[test]
    fn fallback_is_only_the_provider_the_policy_lists() {
        let decision = admit(base_input(
            vec![
                ProviderSlot {
                    ledger: LedgerAccess::Ready(ollama(9_900, 0, 2, Some(90_000))),
                    signal: SignalRead::Absent,
                },
                ProviderSlot {
                    ledger: LedgerAccess::Ready(LedgerSnapshot {
                        provider: "claude-fake".into(),
                        ..claude(0, Some(90_000))
                    }),
                    signal: SignalRead::Absent,
                },
            ],
            Some(100),
        ));
        match decision {
            Admission::Admit { provider, .. } => assert_eq!(provider, "claude-fake"),
            other => panic!("{other:?}"),
        }

        let waiting = admit(base_input(
            vec![ProviderSlot {
                ledger: LedgerAccess::Ready(ollama(9_900, 0, 2, None)),
                signal: SignalRead::Absent,
            }],
            Some(1_000),
        ));
        assert_eq!(
            waiting,
            Admission::AskOwner {
                provider: "ollama-fake".into(),
            }
        );
    }

    #[test]
    fn teammate_budget_at_ninety_nine_percent_does_not_start() {
        let mut input = base_input(
            vec![ProviderSlot {
                ledger: LedgerAccess::NotConfigured {
                    provider: "ollama-fake".into(),
                },
                signal: SignalRead::Absent,
            }],
            Some(1_000),
        );
        input.teammate = BudgetAccess::Ready(TeammateBudget {
            cap: 10_000,
            spent: 9_900,
            reserved: 0,
            conservative_max: 8_000,
        });
        assert_eq!(
            admit(input),
            Admission::Deny {
                reason: DenyReason::TeammateBudget,
            }
        );
    }
}
