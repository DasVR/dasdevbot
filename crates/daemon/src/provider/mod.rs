//! Model providers. Ollama Cloud is the default. The mock provider is for tests.

mod claude;
mod ollama;

use std::cell::RefCell;
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::secrets::{
    resolve_ollama_key, CommandKind, EnvLookup, KeyringHandle, ProcessEnv, SecretHandle,
    DEV_ENV_WARNING,
};

pub use claude::ClaudeCli;
pub use ollama::{OllamaCloud, OllamaLocal, OLLAMA_BUSY_MAX_MS};

/// Tokens reserved before a provider call. A turn whose agent cannot cover this does not call.
pub const RESERVE_TOKENS: u64 = 256;

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub model: String,
    pub system: String,
    pub user: String,
    pub max_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct Completion {
    pub text: String,
    pub model: String,
    pub provider: String,
    pub usage_kind: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub micro_usd: i64,
    pub note: String,
    pub usage: UsageReport,
    pub retry_costs: Vec<RetryCost>,
}

impl Completion {
    fn from_usage(
        text: impl Into<String>,
        model: impl Into<String>,
        provider: impl Into<String>,
        usage_kind: &str,
        micro_usd: i64,
        note: impl Into<String>,
        usage: UsageReport,
    ) -> Self {
        Self {
            text: text.into(),
            model: model.into(),
            provider: provider.into(),
            usage_kind: usage_kind.to_string(),
            input_tokens: usage.input_tokens.unwrap_or(0),
            output_tokens: usage.output_tokens.unwrap_or(0),
            micro_usd,
            note: note.into(),
            usage,
            retry_costs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryCost {
    pub attempt: u32,
    pub backoff_ms: u64,
    pub budget_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct UsageReport {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub quota: QuotaSignal,
    pub headroom: Headroom,
}

impl UsageReport {
    pub fn quota_detail(&self) -> &str {
        match &self.quota {
            QuotaSignal::Absent { detail } | QuotaSignal::Reported { detail } => detail,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotaSignal {
    Absent { detail: String },
    Reported { detail: String },
}

/// Remaining monthly Ollama credit. A later observation can only lower it.
#[derive(Debug, Clone, PartialEq)]
pub struct Headroom {
    remaining_credit: Option<f64>,
}

impl Headroom {
    pub fn unknown() -> Self {
        Self {
            remaining_credit: None,
        }
    }

    pub fn remaining_credit(&self) -> Option<f64> {
        self.remaining_credit
    }

    pub fn lower(&mut self, observed: Option<f64>) {
        let Some(observed) =
            observed.filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        else {
            return;
        };
        self.remaining_credit = Some(match self.remaining_credit {
            Some(current) => current.min(observed),
            None => observed,
        });
    }
}

impl Default for Headroom {
    fn default() -> Self {
        Self::unknown()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LimitReached {
    pub message: String,
    pub resets_at: Option<i64>,
}

impl fmt::Display for LimitReached {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.resets_at {
            Some(resets_at) => write!(formatter, "{} resets_at={resets_at}", self.message),
            None => write!(formatter, "{} resets_at=unavailable", self.message),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Busy {
    pub message: String,
    pub retry_costs: Vec<RetryCost>,
    pub terminal: bool,
}

impl fmt::Display for Busy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.terminal {
            write!(formatter, "{} terminal", self.message)
        } else {
            write!(formatter, "{}", self.message)
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    Failed(String),
    #[error("{0}")]
    LimitReached(LimitReached),
    #[error("{0}")]
    Busy(Busy),
    #[error("claude-cli tool_use attempted")]
    ToolUseAttempted,
}

pub trait LlmProvider: Send + Sync {
    fn complete(&self, req: &CompletionRequest) -> Result<Completion, ProviderError>;
    fn id(&self) -> &'static str;
    fn detail(&self) -> String;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderKind {
    Ollama,
    OllamaLocal,
    ClaudeCli,
}

impl ProviderKind {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "ollama" => Ok(Self::Ollama),
            "ollama-local" => Ok(Self::OllamaLocal),
            "claude-cli" => Ok(Self::ClaudeCli),
            other => Err(format!(
                "provider must be ollama, ollama-local, or claude-cli, got {other}"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::OllamaLocal => "ollama-local",
            Self::ClaudeCli => "claude-cli",
        }
    }
}

pub struct ProviderSettings {
    pub kind: ProviderKind,
    pub model: Option<String>,
    pub dev_env_secrets: bool,
    pub command: CommandKind,
    pub role: String,
}

pub fn open_provider(settings: &ProviderSettings) -> Result<Box<dyn LlmProvider>, ProviderError> {
    open_with(settings, &KeyringHandle, &ProcessEnv)
}

pub fn open_with(
    settings: &ProviderSettings,
    secrets: &dyn SecretHandle,
    env: &dyn EnvLookup,
) -> Result<Box<dyn LlmProvider>, ProviderError> {
    match settings.kind {
        ProviderKind::Ollama => {
            let decision = resolve_ollama_key(
                secrets,
                settings.dev_env_secrets,
                settings.command,
                &settings.role,
                env,
            )
            .map_err(|err| {
                if err.is_unavailable() {
                    ProviderError::Unavailable(err.to_string())
                } else {
                    ProviderError::Failed(err.to_string())
                }
            })?;
            if decision.warn_dev_env {
                log_provider(DEV_ENV_WARNING);
            }
            let Some(key) = decision.key else {
                return Err(ProviderError::Unavailable(
                    "ollama api key is not available; run `dasdevbotd secret set ollama`".into(),
                ));
            };
            Ok(Box::new(OllamaCloud::new(key, settings.model.clone())))
        }
        ProviderKind::OllamaLocal => Ok(Box::new(OllamaLocal::new(settings.model.clone()))),
        ProviderKind::ClaudeCli => {
            let provider = ClaudeCli::open(PathBuf::from("claude"), settings.model.clone())?;
            Ok(Box::new(provider))
        }
    }
}

impl LlmProvider for OllamaCloud {
    fn complete(&self, req: &CompletionRequest) -> Result<Completion, ProviderError> {
        OllamaCloud::complete(self, req)
    }

    fn id(&self) -> &'static str {
        "ollama"
    }

    fn detail(&self) -> String {
        OllamaCloud::detail(self)
    }
}

impl LlmProvider for OllamaLocal {
    fn complete(&self, req: &CompletionRequest) -> Result<Completion, ProviderError> {
        OllamaLocal::complete(self, req)
    }

    fn id(&self) -> &'static str {
        "ollama-local"
    }

    fn detail(&self) -> String {
        OllamaLocal::detail(self)
    }
}

impl LlmProvider for ClaudeCli {
    fn complete(&self, req: &CompletionRequest) -> Result<Completion, ProviderError> {
        ClaudeCli::complete(self, req)
    }

    fn id(&self) -> &'static str {
        "claude-cli"
    }

    fn detail(&self) -> String {
        ClaudeCli::detail(self)
    }
}

pub struct MockProvider {
    pub calls: AtomicUsize,
}

impl MockProvider {
    pub fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmProvider for MockProvider {
    fn complete(&self, req: &CompletionRequest) -> Result<Completion, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let text = mock_draft();
        let input_tokens = estimate_tokens(&req.system) + estimate_tokens(&req.user);
        let output_tokens = estimate_tokens(&text);
        Ok(Completion::from_usage(
            text,
            "mock-review-v0",
            "mock",
            "estimated",
            0,
            "mock provider for tests; token counts are char/4 estimates; no charge; no quota signal",
            UsageReport {
                input_tokens: Some(input_tokens),
                output_tokens: Some(output_tokens),
                cached_input_tokens: None,
                quota: QuotaSignal::Absent {
                    detail: "mock provider does not report quota".into(),
                },
                headroom: Headroom::unknown(),
            },
        ))
    }

    fn id(&self) -> &'static str {
        "mock"
    }

    fn detail(&self) -> String {
        "mock provider for tests".into()
    }
}

fn mock_draft() -> String {
    "If refresh() rejects on a 401, the handoff lock is never released. \
Wrap it in try/finally so the next session can take the lock."
        .into()
}

pub fn estimate_tokens(text: &str) -> u64 {
    let chars = text.chars().count() as u64;
    if chars == 0 {
        0
    } else {
        chars.div_ceil(4)
    }
}

thread_local! {
    static LOG_CAPTURE: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

pub(crate) fn log_provider(line: &str) {
    LOG_CAPTURE.with(|slot| {
        if let Some(buf) = slot.borrow_mut().as_mut() {
            buf.push(line.to_string());
        }
    });
    eprintln!("{line}");
}

#[cfg(test)]
pub(crate) fn start_log() {
    LOG_CAPTURE.with(|slot| *slot.borrow_mut() = Some(Vec::new()));
}

#[cfg(test)]
pub(crate) fn take_log() -> Vec<String> {
    LOG_CAPTURE.with(|slot| slot.borrow_mut().take().unwrap_or_default())
}
