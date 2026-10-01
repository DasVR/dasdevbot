//! OS keychain access for provider credentials.
//!
//! The secret value is only exposed to build an `Authorization` header.
//! `Debug`, `Display`, errors, and log lines redact it.

use std::fmt;
use std::io::{self, IsTerminal, Read};

use keyring::Entry;

pub const SERVICE: &str = "dasdevbotd";
pub const OLLAMA_SECRET_NAME: &str = "ollama";
pub const APPROVAL_KEY_NAME: &str = "approval-key";
pub const DEV_ENV_WARNING: &str =
    "warning: --dev-env-secrets is on; OLLAMA_API_KEY may be read from the environment";
pub const DEV_ENV_SERVER_REFUSAL: &str = "refusing --dev-env-secrets on the server role";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandKind {
    Serve,
    SmokeModel,
    SecretSet,
    Emit,
}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("{0}")]
    Refused(String),
    #[error("{0}")]
    Unavailable(String),
}

impl SecretError {
    pub fn refused(text: impl Into<String>) -> Self {
        Self::Refused(text.into())
    }

    pub fn unavailable(text: impl Into<String>) -> Self {
        Self::Unavailable(text.into())
    }

    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}

pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

pub fn last4(secret: &Secret) -> String {
    let raw = secret.expose();
    let count = raw.chars().count();
    if count <= 4 {
        return "*".repeat(count);
    }
    raw.chars().skip(count - 4).collect()
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret([redacted])")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

pub fn scrub(text: &str, secret: &Secret) -> String {
    let raw = secret.expose();
    if raw.is_empty() {
        return text.to_string();
    }
    text.replace(raw, "[redacted]")
}

pub trait SecretHandle: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<Secret>, SecretError>;
    fn set(&self, name: &str, secret: &Secret) -> Result<(), SecretError>;
}

pub struct KeyringHandle;

impl SecretHandle for KeyringHandle {
    fn get(&self, name: &str) -> Result<Option<Secret>, SecretError> {
        let entry =
            Entry::new(SERVICE, name).map_err(|err| SecretError::unavailable(store_error(&err)))?;
        match entry.get_password() {
            Ok(value) if !value.is_empty() => Ok(Some(Secret::new(value))),
            Ok(_) => Ok(None),
            Err(err) if is_missing(&err) => Ok(None),
            Err(err) => Err(SecretError::unavailable(store_error(&err))),
        }
    }

    fn set(&self, name: &str, secret: &Secret) -> Result<(), SecretError> {
        let entry =
            Entry::new(SERVICE, name).map_err(|err| SecretError::refused(store_error(&err)))?;
        entry
            .set_password(secret.expose())
            .map_err(|err| SecretError::refused(scrub(&store_error(&err), secret)))
    }
}

fn store_error(err: &keyring::Error) -> String {
    err.to_string()
}

fn is_missing(err: &keyring::Error) -> bool {
    matches!(err, keyring::Error::NoEntry)
}

#[cfg(test)]
#[derive(Default)]
pub struct MemorySecrets {
    values: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

#[cfg(test)]
impl MemorySecrets {
    pub fn new() -> Self {
        Self {
            values: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    pub fn insert(&self, name: impl Into<String>, value: impl Into<String>) {
        self.values
            .lock()
            .expect("secrets")
            .insert(name.into(), value.into());
    }
}

#[cfg(test)]
impl SecretHandle for MemorySecrets {
    fn get(&self, name: &str) -> Result<Option<Secret>, SecretError> {
        Ok(self
            .values
            .lock()
            .expect("secrets")
            .get(name)
            .filter(|value| !value.is_empty())
            .map(|value| Secret::new(value.clone())))
    }

    fn set(&self, name: &str, secret: &Secret) -> Result<(), SecretError> {
        self.insert(name, secret.expose());
        Ok(())
    }
}

pub trait EnvLookup {
    fn get(&self, name: &str) -> Option<String>;
}

pub struct ProcessEnv;

impl EnvLookup for ProcessEnv {
    fn get(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

#[cfg(test)]
pub struct MapEnv {
    values: std::collections::HashMap<String, String>,
}

#[cfg(test)]
impl MapEnv {
    pub fn new(values: std::collections::HashMap<String, String>) -> Self {
        Self { values }
    }
}

#[cfg(test)]
impl EnvLookup for MapEnv {
    fn get(&self, name: &str) -> Option<String> {
        self.values.get(name).cloned()
    }
}

#[derive(Debug)]
pub struct KeyDecision {
    pub key: Option<Secret>,
    pub warn_dev_env: bool,
}

pub fn check_dev_env(command: CommandKind, role: &str, enabled: bool) -> Result<(), SecretError> {
    if !enabled {
        return Ok(());
    }
    match command {
        CommandKind::Serve => match role {
            "server" => Err(SecretError::refused(DEV_ENV_SERVER_REFUSAL)),
            "device" | "display" => Ok(()),
            other => Err(SecretError::refused(format!(
                "role must be device, server, or display, got {other}"
            ))),
        },
        CommandKind::SmokeModel => Ok(()),
        CommandKind::SecretSet => Err(SecretError::refused(
            "secret set does not read secrets from the environment",
        )),
        CommandKind::Emit => Err(SecretError::refused(
            "--dev-env-secrets does not apply to emit",
        )),
    }
}

/// Keyring first. `OLLAMA_API_KEY` is read only when `dev_env` is set and the command allows it.
pub fn resolve_ollama_key(
    handle: &dyn SecretHandle,
    dev_env: bool,
    command: CommandKind,
    role: &str,
    env: &dyn EnvLookup,
) -> Result<KeyDecision, SecretError> {
    check_dev_env(command, role, dev_env)?;
    let stored = handle.get(OLLAMA_SECRET_NAME)?;
    if stored.is_some() {
        return Ok(KeyDecision {
            key: stored,
            warn_dev_env: dev_env,
        });
    }
    if !dev_env {
        return Ok(KeyDecision {
            key: None,
            warn_dev_env: false,
        });
    }
    let from_env = env
        .get("OLLAMA_API_KEY")
        .filter(|value| !value.is_empty())
        .map(Secret::new);
    Ok(KeyDecision {
        key: from_env,
        warn_dev_env: true,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum SecretSource {
    Tty,
    Stdin,
}

#[derive(Debug)]
pub struct SecretSetPlan {
    pub name: String,
    pub source: SecretSource,
}

pub fn plan_secret_set(args: &[String]) -> Result<SecretSetPlan, SecretError> {
    let mut name: Option<String> = None;
    let mut stdin = false;
    let mut rejected_value = false;
    for arg in args {
        match arg.as_str() {
            "--stdin" => stdin = true,
            "--dev-env-secrets" => {
                return Err(SecretError::refused(
                    "secret set does not read secrets from the environment",
                ));
            }
            other if other.starts_with('-') => {
                return Err(SecretError::refused(format!("unknown argument {other}")));
            }
            other => {
                if name.is_none() {
                    name = Some(other.to_string());
                } else {
                    rejected_value = true;
                }
            }
        }
    }
    if rejected_value {
        return Err(SecretError::refused(
            "secret set does not accept the secret on the command line",
        ));
    }
    let name = name.ok_or_else(|| SecretError::refused("secret set needs a name"))?;
    validate_secret_name(&name)?;
    let source = if stdin {
        SecretSource::Stdin
    } else {
        SecretSource::Tty
    };
    Ok(SecretSetPlan { name, source })
}

fn validate_secret_name(name: &str) -> Result<(), SecretError> {
    let lower = name.to_ascii_lowercase();
    if lower.contains("setup-token") || lower.contains("setup_token") {
        return Err(SecretError::refused(
            "refusing a claude setup-token; the claude-cli provider does not accept one",
        ));
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(SecretError::refused("secret set needs a name"));
    };
    if !first.is_ascii_lowercase() {
        return Err(SecretError::refused(
            "secret name must start with a lowercase letter",
        ));
    }
    if name.len() > 64
        || !chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
    {
        return Err(SecretError::refused(
            "secret name must be lowercase letters, digits, hyphens, or underscores",
        ));
    }
    Ok(())
}

pub fn secret_from_text(text: &str) -> Result<Secret, SecretError> {
    let trimmed = text.trim_end_matches(['\r', '\n']);
    if trimmed.is_empty() {
        return Err(SecretError::refused("secret is empty"));
    }
    if trimmed.contains('\n') || trimmed.contains('\r') {
        return Err(SecretError::refused("secret must be a single line"));
    }
    Ok(Secret::new(trimmed))
}

pub fn stdin_secret_allowed(stdin_is_terminal: bool) -> Result<(), SecretError> {
    if stdin_is_terminal {
        Err(SecretError::refused(
            "--stdin only accepts a pipe, not a terminal",
        ))
    } else {
        Ok(())
    }
}

pub fn prompt_secret_from_tty(prompt: &str) -> Result<Secret, SecretError> {
    match rpassword::prompt_password(prompt) {
        Ok(value) => secret_from_text(&value),
        Err(err) => Err(SecretError::refused(format!(
            "secret set could not read a no-echo TTY prompt ({})",
            err.kind()
        ))),
    }
}

pub fn read_piped_secret() -> Result<Secret, SecretError> {
    stdin_secret_allowed(io::stdin().is_terminal())?;
    let mut buf = String::new();
    io::stdin()
        .read_to_string(&mut buf)
        .map_err(|err| SecretError::refused(format!("could not read --stdin ({})", err.kind())))?;
    secret_from_text(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn secret_set_rejects_argv_and_setup_tokens_without_echoing_them() {
        let err = plan_secret_set(&["ollama".into(), "SENTINEL_SECRET".into()]).unwrap_err();
        assert!(!err.to_string().contains("SENTINEL_SECRET"));
        assert!(err.to_string().contains("command line"));
        let err = plan_secret_set(&["claude-setup-token".into()]).unwrap_err();
        assert!(err.to_string().contains("setup-token"));
        let err = plan_secret_set(&["--dev-env-secrets".into(), "ollama".into()]).unwrap_err();
        assert!(err.to_string().contains("environment"));
        let plan = plan_secret_set(&["ollama".into(), "--stdin".into()]).unwrap();
        assert_eq!(plan.source, SecretSource::Stdin);
        assert_eq!(plan.name, "ollama");
    }

    #[test]
    fn dev_env_is_off_by_default_and_refused_on_the_server_role() {
        let env = MapEnv::new(HashMap::from([(
            "OLLAMA_API_KEY".into(),
            "SENTINEL_KEY".into(),
        )]));
        let secrets = MemorySecrets::new();
        let decision =
            resolve_ollama_key(&secrets, false, CommandKind::SmokeModel, "server", &env).unwrap();
        assert!(decision.key.is_none());
        assert!(!decision.warn_dev_env);
        let err =
            resolve_ollama_key(&secrets, true, CommandKind::Serve, "server", &env).unwrap_err();
        assert!(!err.to_string().contains("SENTINEL_KEY"));
        assert_eq!(err.to_string(), DEV_ENV_SERVER_REFUSAL);
        assert!(!DEV_ENV_WARNING.contains("SENTINEL_KEY"));
        let decision =
            resolve_ollama_key(&secrets, true, CommandKind::SmokeModel, "server", &env).unwrap();
        assert_eq!(decision.key.unwrap().expose(), "SENTINEL_KEY");
        assert!(decision.warn_dev_env);
    }

    #[test]
    fn secret_debug_is_redacted() {
        let secret = Secret::new("SENTINEL_KEY");
        assert!(!format!("{secret:?}").contains("SENTINEL_KEY"));
        assert!(!format!("{secret}").contains("SENTINEL_KEY"));
        assert_eq!(scrub("bearer SENTINEL_KEY", &secret), "bearer [redacted]");
        assert!(stdin_secret_allowed(true).is_err());
        assert!(stdin_secret_allowed(false).is_ok());
    }
}
