//! dasdevbotd library surface. The binary is a thin CLI over [`serve`].

mod provider;
mod server;
mod store;
mod turn;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

pub use provider::{from_env, smoke_xai, LlmProvider, MockProvider};
pub use server::{serve, url_exposes_bearer};
pub use store::Store;

use provider::ProviderError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("idempotency conflict for key {0}")]
    IdempotencyConflict(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("bind: {0}")]
    Bind(String),
    #[error(transparent)]
    Provider(#[from] ProviderError),
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct App {
    pub store: Mutex<Store>,
    pub provider: Arc<dyn LlmProvider>,
    pub web_root: Option<PathBuf>,
    pub role: String,
    pub worker_id: String,
    pub wake: mpsc::Sender<()>,
    /// Set by [`serve`] after the iroh endpoint binds. Empty when p2p is off.
    pub endpoint_id: Mutex<Option<String>>,
    /// Per-launch bearer required on mutating routes. Not returned by the API.
    pub token: String,
}

pub struct Config {
    pub data: PathBuf,
    pub web_root: Option<PathBuf>,
    pub role: String,
    /// Operator token (`--token` or `DASDEVBOT_TOKEN`). Generated when absent.
    pub token: Option<String>,
}

pub fn build_and_worker(config: Config) -> Result<Arc<App>> {
    let provider = from_env();
    let (app, rx) = build_app(config, provider)?;
    turn::spawn_worker(Arc::clone(&app), rx);
    Ok(app)
}

/// `--allow-remote` stays closed unless the operator chose the bearer.
pub fn require_explicit_token(allow_remote: bool, token: Option<&str>) -> Result<()> {
    if !allow_remote {
        return Ok(());
    }
    match token {
        Some(token) if !token.trim().is_empty() => Ok(()),
        _ => Err(Error::BadRequest(
            "refusing --allow-remote unless a bearer token is set with --token or DASDEVBOT_TOKEN"
                .into(),
        )),
    }
}

pub fn session_token_path(data: &Path) -> PathBuf {
    let mut name = data
        .file_name()
        .map(|file| file.to_os_string())
        .unwrap_or_default();
    name.push(".token");
    data.with_file_name(name)
}

pub fn build_app(
    config: Config,
    provider: Box<dyn LlmProvider>,
) -> Result<(Arc<App>, mpsc::Receiver<()>)> {
    let store = Store::open(&config.data)?;
    let node = store.node_id().to_string();
    let (wake, rx) = mpsc::channel();
    let token = match config.token {
        Some(token) if !token.trim().is_empty() => token,
        _ => mint_token(),
    };
    persist_token(&config.data, &token)?;
    let app = Arc::new(App {
        worker_id: format!("{node}:worker-1"),
        role: config.role,
        web_root: config.web_root,
        provider: Arc::from(provider),
        store: Mutex::new(store),
        wake,
        endpoint_id: Mutex::new(None),
        token,
    });
    Ok((app, rx))
}

fn mint_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn persist_token(data: &Path, token: &str) -> Result<()> {
    let path = session_token_path(data);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    file.write_all(token.as_bytes())?;
    Ok(())
}

pub fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_remote_requires_an_explicit_token() {
        assert!(require_explicit_token(false, None).is_ok());
        assert!(require_explicit_token(true, None).is_err());
        assert!(require_explicit_token(true, Some("  ")).is_err());
        assert!(require_explicit_token(true, Some("operator-token")).is_ok());
    }
}
