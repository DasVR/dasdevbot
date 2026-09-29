//! Domain types for the five kernel nouns that exist in phase 0:
//! event (via the clock that stamps it), job lease, agent budget, and the gate.
//! Persistence and the network live in the daemon.

mod budget;
mod gate;
mod hlc;
mod lease;

pub use budget::TokenBudget;
pub use gate::{decide, EffectClass, GateInput, GateOutcome, Policy};
pub use hlc::{HlcTimestamp, HybridClock};
pub use lease::{is_expired, Lease};

pub const EVENT_VERSION: u32 = 1;

pub mod kind {
    pub const REPO_PUSH: &str = "repo.push";
    pub const APPROVAL_REQUESTED: &str = "approval.requested";
    pub const APPROVAL_DECIDED: &str = "approval.decided";
    pub const APPROVAL_UNDONE: &str = "approval.undone";
    pub const APPROVAL_COMMITTED: &str = "approval.committed";
    pub const APPROVAL_EXPIRED: &str = "approval.expired";
    pub const LEDGER_POSTED: &str = "ledger.posted";
    pub const BUDGET_DENIED: &str = "budget.denied";
    pub const JOB_FAILED: &str = "job.failed";
}
