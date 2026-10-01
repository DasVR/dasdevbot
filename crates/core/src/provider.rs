//! Provider-agnostic teammate interface. The stub does not call a network.
//! Real adapters live outside this crate so they can land on their own branch.

use crate::admission::SignalRead;

/// Attempts after the first call. The driver stops here. It does not spin.
pub const RETRY_CAP: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderStop {
    /// Mid-run limit. Pause at the last checkpoint.
    Limit,
    /// Transient failure. Counted as a charged retry.
    Retry,
    /// Terminal busy. Pause. Do not call again.
    Busy,
    /// The provider tried to use a tool. Fail closed. The audit row has no content.
    ToolUseAttempted,
    Fault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    Provider,
    Ledger,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEnd {
    Done { attempts: u32, text: String },
    Paused { attempts: u32, reason: PauseReason },
    FailedClosed { attempts: u32, tool_use: bool },
}

/// Audit body for [`ProviderStop::ToolUseAttempted`]. Always empty.
pub fn tool_use_audit_payload() -> &'static str {
    ""
}

/// Run provider attempts. Every attempt, including each retry, charges the
/// ledger before the call. Busy and the retry cap pause. Tool use fails closed.
pub fn drive_provider(
    retry_cap: u32,
    mut charge: impl FnMut() -> bool,
    mut attempt: impl FnMut() -> Result<String, ProviderStop>,
) -> RunEnd {
    let mut attempts = 0u32;
    loop {
        if !charge() {
            return RunEnd::Paused {
                attempts,
                reason: PauseReason::Ledger,
            };
        }
        attempts = attempts.saturating_add(1);
        match attempt() {
            Ok(text) => {
                return RunEnd::Done { attempts, text };
            }
            Err(ProviderStop::Retry) if attempts < retry_cap => {}
            Err(ProviderStop::Retry) | Err(ProviderStop::Busy) | Err(ProviderStop::Limit) => {
                return RunEnd::Paused {
                    attempts,
                    reason: PauseReason::Provider,
                };
            }
            Err(ProviderStop::ToolUseAttempted) => {
                return RunEnd::FailedClosed {
                    attempts,
                    tool_use: true,
                };
            }
            Err(ProviderStop::Fault) => {
                return RunEnd::FailedClosed {
                    attempts,
                    tool_use: false,
                };
            }
        }
    }
}

pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn limit_signal(&self) -> SignalRead;
    fn complete(&self, prompt: &str) -> Result<String, ProviderStop>;
}

#[derive(Debug, Default)]
pub struct StubProvider;

impl Provider for StubProvider {
    fn name(&self) -> &str {
        "stub"
    }

    fn limit_signal(&self) -> SignalRead {
        SignalRead::Absent
    }

    fn complete(&self, prompt: &str) -> Result<String, ProviderStop> {
        Ok(format!("[stub] {} chars", prompt.chars().count()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stub_is_labeled_and_does_not_invent_a_limit() {
        let provider = StubProvider;
        let text = provider.complete("hello").unwrap();
        assert!(text.starts_with("[stub]"));
        assert_eq!(provider.limit_signal(), SignalRead::Absent);
        assert_eq!(provider.name(), "stub");
    }

    #[test]
    fn every_retry_is_charged_and_the_cap_pauses() {
        let mut charges = 0u32;
        let mut calls = 0u32;
        let end = drive_provider(
            2,
            || {
                charges += 1;
                true
            },
            || {
                calls += 1;
                Err(ProviderStop::Retry)
            },
        );
        assert_eq!(charges, 2);
        assert_eq!(calls, 2);
        assert_eq!(
            end,
            RunEnd::Paused {
                attempts: 2,
                reason: PauseReason::Provider,
            }
        );
    }

    #[test]
    fn terminal_busy_pauses_after_one_call() {
        let mut calls = 0u32;
        let end = drive_provider(
            RETRY_CAP,
            || true,
            || {
                calls += 1;
                Err(ProviderStop::Busy)
            },
        );
        assert_eq!(calls, 1);
        assert_eq!(
            end,
            RunEnd::Paused {
                attempts: 1,
                reason: PauseReason::Provider,
            }
        );
    }

    #[test]
    fn tool_use_fails_closed_with_an_empty_audit() {
        let mut charges = 0u32;
        let end = drive_provider(
            RETRY_CAP,
            || {
                charges += 1;
                true
            },
            || Err(ProviderStop::ToolUseAttempted),
        );
        assert_eq!(charges, 1);
        assert_eq!(
            end,
            RunEnd::FailedClosed {
                attempts: 1,
                tool_use: true,
            }
        );
        assert!(tool_use_audit_payload().is_empty());
    }
}
