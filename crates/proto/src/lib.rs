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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionResponse {
    pub protocol: u32,
    pub approval_id: String,
    pub status: String,
    pub event_id: String,
    pub executed: bool,
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
    pub evidence: String,
    pub status: String,
    pub provider: String,
    pub model: String,
    pub usage_kind: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub micro_usd: i64,
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
    pub agents: Vec<AgentView>,
    pub approvals: Vec<ApprovalView>,
    pub ledger: Vec<LedgerView>,
    pub events: Vec<EventView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: String,
}
