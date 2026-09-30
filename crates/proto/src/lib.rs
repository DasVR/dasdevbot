//! Protocol version 1.
//!
//! Phase 0 carries these messages as JSON over loopback HTTP. The same shapes
//! are what a later Tauri shell or an iroh stream should send. Bumping
//! `PROTOCOL_VERSION` is the compatibility signal.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Health {
    pub protocol: u32,
    pub ok: bool,
    pub ready: bool,
    pub role: String,
    pub node: String,
    pub provider: String,
    pub provider_detail: String,
    pub sync: String,
    /// iroh node id when `serve` bound an endpoint. Absent when p2p is off.
    pub endpoint_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmitRequest {
    pub source: String,
    pub kind: String,
    pub payload: Value,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmitResponse {
    pub protocol: u32,
    pub created: bool,
    pub event_id: String,
    pub thread_id: String,
    pub jobs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRequest {
    pub decision: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionResponse {
    pub protocol: u32,
    pub approval_id: String,
    pub status: String,
    pub event_id: String,
    pub executed: bool,
    pub committed: bool,
    /// Epoch milliseconds when the undo window closes. Absent once the decision is final.
    #[serde(default)]
    pub undo_until: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoResponse {
    pub protocol: u32,
    pub approval_id: String,
    pub status: String,
    pub event_id: String,
}

/// Structured evidence for the approval card. `evidence_text` on the approval
/// keeps the older free-text line for clients that still read it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceView {
    pub repo: String,
    /// Pull-request line, mock order `repo`, `pr`, `ref`, `event`. Empty when the ask has no PR.
    #[serde(default)]
    pub pr: String,
    /// Digits from the leading `#n` in `pr`, used in the undo subline.
    #[serde(default)]
    pub pr_number: String,
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub event_id: String,
    #[serde(default)]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentView {
    pub id: String,
    pub name: String,
    pub project: String,
    pub persona: String,
    pub token_cap: u64,
    pub tokens_spent: u64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalView {
    pub id: String,
    pub job_id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub thread_id: String,
    pub effect_class: String,
    pub action: String,
    pub purpose: String,
    pub draft: String,
    pub evidence: EvidenceView,
    /// Legacy free-text evidence. Structured fields live on `evidence`.
    pub evidence_text: String,
    pub status: String,
    pub provider: String,
    pub model: String,
    pub usage_kind: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub micro_usd: i64,
    /// Epoch milliseconds the approval was created.
    pub created_at: u64,
    /// Epoch milliseconds the pending approval expires. Null once it is no longer pending.
    #[serde(default)]
    pub expires_at: Option<u64>,
    /// Epoch milliseconds the decision was recorded. Null while pending.
    #[serde(default)]
    pub decided_at: Option<u64>,
    #[serde(default)]
    pub decision_event_id: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    /// False while the decision is still inside the undo window.
    pub committed: bool,
    /// Epoch milliseconds when the undo window closes.
    #[serde(default)]
    pub undo_until: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerView {
    pub id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub project: String,
    pub provider: String,
    pub model: String,
    pub usage_kind: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub micro_usd: i64,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventView {
    pub id: String,
    pub version: u32,
    pub hlc: String,
    pub source: String,
    pub kind: String,
    pub thread_id: String,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub protocol: u32,
    pub role: String,
    pub node: String,
    pub provider: String,
    pub provider_detail: String,
    pub sync: String,
    pub endpoint_id: Option<String>,
    pub agents: Vec<AgentView>,
    pub approvals: Vec<ApprovalView>,
    pub ledger: Vec<LedgerView>,
    pub events: Vec<EventView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: String,
}
