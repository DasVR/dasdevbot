//! Domain types for the five kernel nouns that exist in phase 0:
//! event (via the clock that stamps it), job lease, agent budget, and the gate.
//! Persistence and the network live in the daemon.

mod admission;
mod budget;
mod decision;
mod election;
mod gate;
mod grant;
mod harness;
mod hlc;
mod lease;
mod ownership;
mod provider;
mod roles;
mod secret;

pub use admission::{
    admit, authorize_spend, cover_tokens, effective_headroom, ledger_remaining, AdmitInput,
    Admission, BudgetAccess, DenyReason, LedgerAccess, LedgerSnapshot, LimitSignal, ProviderSlot,
    ReserveWrite, SignalRead, TeammateBudget, WindowKind, WriterLease, WriteAuth,
    HEADROOM_DENOMINATOR, HEADROOM_NUMERATOR,
};
pub use budget::TokenBudget;
pub use decision::{
    authorize_decision, authorize_secret_window, DecisionDeny, Surface, CARD_WINDOW, MAIN_WINDOW,
    SETTINGS_WINDOW,
};
pub use election::{claim as claim_leader, dispatch_allowed, Claim, LeaderLease};
pub use gate::{decide, EffectClass, GateInput, GateOutcome, Policy};
pub use grant::{issue_expiry, GrantError, EXTERNAL_EXPIRY_MS, READ_EXPIRY_MS, WRITE_LOCAL_EXPIRY_MS};
pub use harness::{HarnessError, HarnessState, JobLife, Phase, Step};
pub use hlc::{HlcTimestamp, HybridClock};
pub use lease::{is_expired, Lease};
pub use ownership::{device_to_server_classes, DataOwner, OwnedData, OWNERSHIP};
pub use provider::{
    drive_provider, tool_use_audit_payload, PauseReason, Provider, ProviderStop, RunEnd,
    StubProvider, RETRY_CAP,
};
pub use roles::{
    authorize_secret_name, claim_mode, claims_jobs, may_hold_github_credential,
    may_seek_leadership, may_write_admission_ledger, parse_role, ClaimMode, Role, SecretRoleError,
};
pub use secret::SecretValue;

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
    pub const JOB_PAUSED: &str = "job.paused";
    pub const PROVIDER_COST: &str = "provider.cost";
    pub const TOOL_USE_BLOCKED: &str = "provider.tool_use_blocked";
    pub const DEV_ENV: &str = "secret.dev_env";
    pub const GATE_DENIED: &str = "gate.denied";
    pub const WAITING_ON_QUOTA: &str = "waiting_on_quota";
    pub const WAITING_ON_SLOT: &str = "waiting_on_slot";
}
