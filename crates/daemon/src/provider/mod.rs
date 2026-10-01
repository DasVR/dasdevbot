//! Model providers. Ollama Cloud is the default. The mock provider is for tests
//! and for local `serve --provider mock` demos. It is never a fallback.

#[cfg(unix)]
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

#[cfg(unix)]
pub use claude::ClaudeCli;
pub use ollama::{OllamaCloud, OllamaLocal, OLLAMA_BUSY_MAX_MS};

/// Parse a comma-separated list of SHA-256 hex digests of the native Claude ELF.
/// The provider accepts one digest. A second digest fails because a script
/// interpreter is not part of the pin.
pub fn parse_sha256_list(text: &str) -> Result<Vec<[u8; 32]>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("claude_sha256 is empty".into());
    }
    let mut out = Vec::new();
    for part in text.split(',') {
        let part = part.trim();
        if part.len() != 64 || !part.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("claude_sha256 must be 64 hex characters".into());
        }
        let mut bytes = [0u8; 32];
        for (byte, chunk) in bytes.iter_mut().zip(part.as_bytes().chunks(2)) {
            let hex = std::str::from_utf8(chunk)
                .map_err(|_| "claude_sha256 must be 64 hex characters".to_string())?;
            *byte = u8::from_str_radix(hex, 16)
                .map_err(|_| "claude_sha256 must be 64 hex characters".to_string())?;
        }
        out.push(bytes);
    }
    Ok(out)
}

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
    ToolUseAttempted { cli_version: String, event: String },
}

pub trait LlmProvider: Send + Sync {
    fn complete(
        &self,
        req: &CompletionRequest,
        charge: &mut dyn FnMut(&RetryCost) -> Result<(), ProviderError>,
    ) -> Result<Completion, ProviderError>;
    fn id(&self) -> &'static str;
    fn detail(&self) -> String;
    /// Tokens one attempt can spend. The admission ledger reserves this before
    /// every attempt, including the first and each retry.
    fn attempt_worst_case(&self, req: &CompletionRequest) -> u64 {
        attempt_worst_case_tokens(req, 0)
    }
}

/// Input estimate, plus `max_tokens`, plus any per-attempt accounting the provider adds.
pub fn attempt_worst_case_tokens(req: &CompletionRequest, retry_accounting: u64) -> u64 {
    estimate_tokens(&req.system)
        .saturating_add(estimate_tokens(&req.user))
        .saturating_add(u64::from(req.max_tokens))
        .saturating_add(retry_accounting)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderKind {
    Ollama,
    OllamaLocal,
    ClaudeCli,
    /// Demo only. Chosen only by an explicit `--provider mock`, only for `serve`
    /// on a local role (device or executor). Never on the server role, and
    /// nothing selects it when another provider fails.
    Mock,
}

impl ProviderKind {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "ollama" => Ok(Self::Ollama),
            "ollama-local" => Ok(Self::OllamaLocal),
            "claude-cli" => Ok(Self::ClaudeCli),
            "mock" => Ok(Self::Mock),
            other => Err(format!(
                "provider must be ollama, ollama-local, claude-cli, or mock, got {other}"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::OllamaLocal => "ollama-local",
            Self::ClaudeCli => "claude-cli",
            Self::Mock => "mock",
        }
    }
}

pub struct ProviderSettings {
    pub kind: ProviderKind,
    pub model: Option<String>,
    pub dev_env_secrets: bool,
    pub command: CommandKind,
    pub role: String,
    /// HOME for the Claude CLI. Required for `claude-cli`. The process home is not used.
    pub claude_home: Option<PathBuf>,
    /// SHA-256 of the native Claude ELF. Required for `serve` on the server role.
    pub claude_sha256: Vec<[u8; 32]>,
}

#[cfg(unix)]
fn claude_sha256_required(settings: &ProviderSettings) -> bool {
    settings.kind == ProviderKind::ClaudeCli
        && settings.command == CommandKind::Serve
        && settings.role == "server"
        && settings.claude_sha256.is_empty()
}

pub const MOCK_DEMO_WARNING: &str =
    "dasdevbotd provider=mock: demo only; drafts are canned, nothing calls a model or the network";

/// `--provider mock` runs only for `serve` on a local role. The device role
/// takes it explicitly; the executor role is the one that writes the admission
/// ledger, so a single-box demo that should draft cards runs as executor.
fn mock_allowed(settings: &ProviderSettings) -> Result<(), String> {
    if settings.role == "server" {
        return Err("provider mock is demo-only and refused on the server role".into());
    }
    if settings.command != CommandKind::Serve {
        return Err("provider mock is demo-only and runs only under serve".into());
    }
    if settings.role != "device" && settings.role != "executor" {
        return Err(format!(
            "provider mock is demo-only and runs only on the device or executor role, got {}",
            settings.role
        ));
    }
    Ok(())
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
        ProviderKind::Mock => {
            // No key lookup, no env read, no client: the mock never leaves the process.
            let _ = (secrets, env);
            mock_allowed(settings).map_err(ProviderError::Failed)?;
            log_provider(MOCK_DEMO_WARNING);
            Ok(Box::new(MockProvider::new()))
        }
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
            #[cfg(not(unix))]
            {
                let _ = settings;
                Err(ProviderError::Failed(
                    "claude-cli is only available on unix".into(),
                ))
            }
            #[cfg(unix)]
            {
                if claude_sha256_required(settings) {
                    return Err(ProviderError::Failed(
                        "claude_sha256 is required for the service role".into(),
                    ));
                }
                let home = settings.claude_home.clone().ok_or_else(|| {
                    ProviderError::Failed(
                        "claude_home is required; refusing to load the process home".into(),
                    )
                })?;
                let provider = ClaudeCli::open(
                    PathBuf::from("claude"),
                    settings.model.clone(),
                    home,
                    &settings.claude_sha256,
                )?;
                Ok(Box::new(provider))
            }
        }
    }
}

impl LlmProvider for OllamaCloud {
    fn complete(
        &self,
        req: &CompletionRequest,
        charge: &mut dyn FnMut(&RetryCost) -> Result<(), ProviderError>,
    ) -> Result<Completion, ProviderError> {
        OllamaCloud::complete(self, req, charge)
    }

    fn id(&self) -> &'static str {
        "ollama"
    }

    fn detail(&self) -> String {
        OllamaCloud::detail(self)
    }

    fn attempt_worst_case(&self, req: &CompletionRequest) -> u64 {
        attempt_worst_case_tokens(req, ollama::OLLAMA_BUSY_RETRY_BUDGET_TOKENS)
    }
}

impl LlmProvider for OllamaLocal {
    fn complete(
        &self,
        req: &CompletionRequest,
        charge: &mut dyn FnMut(&RetryCost) -> Result<(), ProviderError>,
    ) -> Result<Completion, ProviderError> {
        OllamaLocal::complete(self, req, charge)
    }

    fn id(&self) -> &'static str {
        "ollama-local"
    }

    fn detail(&self) -> String {
        OllamaLocal::detail(self)
    }
}

#[cfg(unix)]
impl LlmProvider for ClaudeCli {
    fn complete(
        &self,
        req: &CompletionRequest,
        charge: &mut dyn FnMut(&RetryCost) -> Result<(), ProviderError>,
    ) -> Result<Completion, ProviderError> {
        ClaudeCli::complete(self, req, charge)
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
    fn complete(
        &self,
        req: &CompletionRequest,
        _charge: &mut dyn FnMut(&RetryCost) -> Result<(), ProviderError>,
    ) -> Result<Completion, ProviderError> {
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
        "mock provider (demo, no network)".into()
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn the_server_role_requires_claude_sha256_before_starting() {
        let missing = ProviderSettings {
            kind: ProviderKind::ClaudeCli,
            model: None,
            dev_env_secrets: false,
            command: CommandKind::Serve,
            role: "server".into(),
            claude_home: Some(PathBuf::from("/var/lib/dasdevbot")),
            claude_sha256: Vec::new(),
        };
        let opened = open_with(&missing, &KeyringHandle, &ProcessEnv);
        let Err(err) = opened else {
            panic!("server role started without claude_sha256");
        };
        assert!(
            err.to_string().contains("claude_sha256 is required"),
            "{err}"
        );
        let mut smoke = missing;
        smoke.command = CommandKind::SmokeModel;
        assert!(!claude_sha256_required(&smoke));
        let mut device = smoke;
        device.command = CommandKind::Serve;
        device.role = "device".into();
        assert!(!claude_sha256_required(&device));
    }
}

#[cfg(test)]
mod mock_tests {
    use super::*;
    use crate::secrets::{Secret, SecretError};

    /// Panics on any use, so a test proves the mock never reads a key or the env.
    struct Untouchable;

    impl SecretHandle for Untouchable {
        fn get(&self, name: &str) -> Result<Option<Secret>, SecretError> {
            panic!("mock provider read secret {name}");
        }

        fn set(&self, name: &str, _secret: &Secret) -> Result<(), SecretError> {
            panic!("mock provider wrote secret {name}");
        }
    }

    impl EnvLookup for Untouchable {
        fn get(&self, name: &str) -> Option<String> {
            panic!("mock provider read env {name}");
        }
    }

    /// No stored key and no env: the real provider is unavailable.
    struct Empty;

    impl SecretHandle for Empty {
        fn get(&self, _name: &str) -> Result<Option<Secret>, SecretError> {
            Ok(None)
        }

        fn set(&self, _name: &str, _secret: &Secret) -> Result<(), SecretError> {
            Ok(())
        }
    }

    impl EnvLookup for Empty {
        fn get(&self, _name: &str) -> Option<String> {
            None
        }
    }

    fn settings(kind: ProviderKind, command: CommandKind, role: &str) -> ProviderSettings {
        ProviderSettings {
            kind,
            model: None,
            dev_env_secrets: false,
            command,
            role: role.into(),
            claude_home: None,
            claude_sha256: Vec::new(),
        }
    }

    fn refusal(kind: ProviderKind, command: CommandKind, role: &str) -> String {
        match open_with(&settings(kind, command, role), &Untouchable, &Untouchable) {
            Ok(provider) => panic!("{} opened for {role}", provider.id()),
            Err(err) => err.to_string(),
        }
    }

    #[test]
    fn mock_is_refused_on_the_server_role() {
        assert_eq!(ProviderKind::parse("mock"), Ok(ProviderKind::Mock));
        let err = refusal(ProviderKind::Mock, CommandKind::Serve, "server");
        assert_eq!(
            err,
            "provider mock is demo-only and refused on the server role"
        );
        let err = refusal(ProviderKind::Mock, CommandKind::SmokeModel, "server");
        assert!(err.contains("refused on the server role"), "{err}");
    }

    #[test]
    fn mock_runs_only_under_serve_on_a_local_role() {
        for role in ["leader", "worker", "display"] {
            let err = refusal(ProviderKind::Mock, CommandKind::Serve, role);
            assert!(
                err.contains("only on the device or executor role"),
                "{role}: {err}"
            );
        }
        let err = refusal(ProviderKind::Mock, CommandKind::SmokeModel, "device");
        assert!(err.contains("only under serve"), "{err}");
        for role in ["device", "executor"] {
            start_log();
            let provider = open_with(
                &settings(ProviderKind::Mock, CommandKind::Serve, role),
                &Untouchable,
                &Untouchable,
            )
            .unwrap();
            let log = take_log();
            assert_eq!(provider.id(), "mock");
            assert!(
                log.iter().any(|line| line.contains("provider=mock")),
                "{log:?}"
            );
        }
    }

    #[test]
    fn mock_is_never_a_fallback_when_another_provider_fails() {
        for role in ["device", "server"] {
            let opened = open_with(
                &settings(ProviderKind::Ollama, CommandKind::Serve, role),
                &Empty,
                &Empty,
            );
            match opened {
                Ok(provider) => panic!("ollama without a key opened {}", provider.id()),
                Err(ProviderError::Unavailable(_)) => {}
                Err(other) => panic!("unexpected error {other}"),
            }
        }
        for name in ["", "Mock", "MOCK", "mock ", "demo", "none"] {
            assert!(ProviderKind::parse(name).is_err(), "{name:?} parsed");
        }
    }

    #[test]
    fn the_mock_provider_has_no_network_or_process_path() {
        let source = include_str!("mod.rs");
        let start = source.find("pub struct MockProvider").unwrap();
        let end = source.find("pub fn estimate_tokens").unwrap();
        let mock = &source[start..end];
        for banned in [
            "ureq",
            "reqwest",
            "std::net",
            "TcpStream",
            "UdpSocket",
            "std::process",
            "Command::",
            "http",
            "OllamaCloud",
            "OllamaLocal",
            "ClaudeCli",
        ] {
            assert!(!mock.contains(banned), "mock provider mentions {banned}");
        }
        let provider = MockProvider::new();
        let completion = provider
            .complete(
                &CompletionRequest {
                    model: String::new(),
                    system: "s".into(),
                    user: "u".into(),
                    max_tokens: 16,
                },
                &mut |_| panic!("the mock charges nothing"),
            )
            .unwrap();
        assert_eq!(completion.provider, "mock");
        assert_eq!(completion.micro_usd, 0);
    }
}
