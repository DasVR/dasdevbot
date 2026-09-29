//! Phase 0 does not link iroh.
//!
//! The event log is the source of truth. When sync lands, devices append
//! locally with hybrid logical clocks and the server assigns order. Leases
//! and idempotency keys already live in the daemon so a later transport does
//! not have to invent them.
//!
//! iroh is left out of this spike on purpose: the task allows a stub, and the
//! binary-size budget should be measured on the kernel (SQLite, the queue,
//! one provider call, loopback IPC) before the QUIC stack is added.

pub const TRANSPORT: &str = "stub";

pub fn linked() -> bool {
    false
}

pub fn describe() -> &'static str {
    "sync stub: iroh is not linked in phase 0"
}
