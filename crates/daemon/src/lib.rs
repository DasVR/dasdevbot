//! dasdevbotd library surface. The binary is a thin CLI over [`serve`].

mod provider;
mod server;
mod store;
mod turn;

use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};

pub use provider::{from_env, smoke_xai, LlmProvider, MockProvider};
pub use server::serve;
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
}

pub struct Config {
    pub data: PathBuf,
    pub web_root: Option<PathBuf>,
    pub role: String,
}

pub fn build_and_worker(config: Config) -> Result<Arc<App>> {
    let (app, rx) = build_app(config, from_env())?;
    turn::spawn_worker(Arc::clone(&app), rx);
    Ok(app)
}

pub fn build_app(
    config: Config,
    provider: Box<dyn LlmProvider>,
) -> Result<(Arc<App>, mpsc::Receiver<()>)> {
    let store = Store::open(&config.data)?;
    let node = store.node_id().to_string();
    let (wake, rx) = mpsc::channel();
    let app = Arc::new(App {
        worker_id: format!("{node}:worker-1"),
        role: config.role,
        web_root: config.web_root,
        provider: Arc::from(provider),
        store: Mutex::new(store),
        wake,
        endpoint_id: Mutex::new(None),
    });
    Ok((app, rx))
}

pub fn wall_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
