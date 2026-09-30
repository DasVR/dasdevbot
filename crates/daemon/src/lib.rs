//! dasdevbotd library surface. The binary is a thin CLI over [`serve`].

mod audit_log;
mod caps;
mod harness;
pub mod ipc;
mod ownership_store;
mod provider;
mod schema;
pub mod secrets;
mod server;
mod shell_ipc;
mod signature;
mod store;
mod surface;
mod topology;
mod turn;
mod verify_user;

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

pub use audit_log::{audit_tool_use_attempted, verify as verify_audit};
pub use caps::{admit_job, install_cap};
pub use harness::execute as execute_harness;
pub use ownership_store::{insert_chat, server_batch, JobMeta};
pub use provider::{
    open_provider, parse_sha256_list, CompletionRequest, LlmProvider, MockProvider, ProviderError,
    ProviderKind, ProviderSettings,
};
pub use secrets::{
    plan_secret_set, prompt_secret_from_tty, read_piped_secret, CommandKind, KeyringHandle,
    SecretHandle, SecretSource,
};
pub use server::{serve, url_exposes_bearer};
pub use store::Store;
pub use turn::audit_dev_env;

const MIN_TOKEN_BYTES: usize = 32;

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
    #[error("bind: {0}")]
    Bind(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
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
    pub data: PathBuf,
    pub audit_seed: [u8; 32],
}

pub struct Config {
    pub data: PathBuf,
    pub web_root: Option<PathBuf>,
    pub role: String,
    /// Operator token (`--token` or `DASDEVBOT_TOKEN`). Generated when absent.
    pub token: Option<String>,
}

pub fn build_and_worker(config: Config, provider: Box<dyn LlmProvider>) -> Result<Arc<App>> {
    let (app, rx) = build_app(config, provider)?;
    shell_ipc::spawn(Arc::clone(&app))?;
    turn::spawn_worker(Arc::clone(&app), rx);
    Ok(app)
}

pub fn session_token_path(data: &Path) -> PathBuf {
    sibling(data, ".token")
}

pub fn role_path(data: &Path) -> PathBuf {
    sibling(data, ".role")
}

pub fn audit_key_path(data: &Path) -> PathBuf {
    sibling(data, ".audit-key")
}

pub fn shell_socket_path(data: &Path) -> PathBuf {
    sibling(data, ".shell.sock")
}

pub fn shell_port_path(data: &Path) -> PathBuf {
    sibling(data, ".shell.port")
}

fn sibling(data: &Path, suffix: &str) -> PathBuf {
    let mut name = data
        .file_name()
        .map(|file| file.to_os_string())
        .unwrap_or_default();
    name.push(suffix);
    data.with_file_name(name)
}

pub fn build_app(
    config: Config,
    provider: Box<dyn LlmProvider>,
) -> Result<(Arc<App>, mpsc::Receiver<()>)> {
    let store = Store::open(&config.data)?;
    let audit_seed = load_or_create_audit_seed(&config.data, &store)?;
    if !audit_log::verify(&store, &audit_seed)? {
        eprintln!("dasdevbotd: audit log failed verification; refusing to start");
        return Err(Error::Forbidden("audit log failed verification".into()));
    }
    let node = store.node_id().to_string();
    let (wake, rx) = mpsc::channel();
    let token = match config.token {
        Some(token) if !token.trim().is_empty() => {
            let token = token.trim().to_string();
            if token.len() < MIN_TOKEN_BYTES {
                return Err(Error::BadRequest(
                    "bearer token must be at least 32 random bytes".into(),
                ));
            }
            token
        }
        _ => mint_token(),
    };
    persist_token(&config.data, &token)?;
    persist_role(&config.data, &config.role)?;
    warn_role_file(&role_path(&config.data));
    let app = Arc::new(App {
        worker_id: format!("{node}:worker-1"),
        role: config.role,
        web_root: config.web_root,
        provider: Arc::from(provider),
        store: Mutex::new(store),
        wake,
        endpoint_id: Mutex::new(None),
        token,
        data: config.data,
        audit_seed,
    });
    Ok((app, rx))
}

fn mint_token() -> String {
    let mut bytes = [0u8; MIN_TOKEN_BYTES];
    getrandom::getrandom(&mut bytes).expect("os rng");
    hex_encode(&bytes)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

/// Write `*` into the data directory only when this process creates it.
/// A bare filename resolves to `.`, and an existing directory is left alone.
pub(crate) fn ensure_data_gitignore(data: &Path) -> Result<()> {
    let Some(dir) = data
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty() && *parent != Path::new("."))
    else {
        return Ok(());
    };
    if dir.exists() {
        return Ok(());
    }
    fs::create_dir_all(dir)?;
    let ignore = dir.join(".gitignore");
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    match options.open(&ignore) {
        Ok(mut file) => {
            file.write_all(b"*\n")?;
            Ok(())
        }
        Err(err) if err.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn persist_token(data: &Path, token: &str) -> Result<()> {
    write_private(session_token_path(data), token.as_bytes())
}

fn persist_role(data: &Path, role: &str) -> Result<()> {
    write_private(role_path(data), role.as_bytes())
}

fn load_or_create_audit_seed(data: &Path, store: &Store) -> Result<[u8; 32]> {
    let path = audit_key_path(data);
    if path.exists() {
        let text = fs::read_to_string(&path)?;
        return signature::decode_seed(text.trim())
            .ok_or_else(|| Error::Forbidden("audit key is not a 32-byte seed".into()));
    }
    let rows: i64 = store
        .connection()
        .query_row("SELECT COUNT(*) FROM audit_log", [], |row| row.get(0))?;
    if rows > 0 {
        eprintln!("dasdevbotd: audit log has rows but the audit key is missing; refusing to start");
        return Err(Error::Forbidden("audit key is missing".into()));
    }
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("os rng");
    write_private(path, signature::encode_seed(&bytes).as_bytes())?;
    Ok(bytes)
}

fn warn_role_file(path: &Path) {
    if role_file_is_unsafe(path) {
        eprintln!(
            "dasdevbotd: role file {} is writable by this process; it should be root-owned and not group- or world-writable",
            path.display()
        );
    }
}

pub(crate) fn role_file_is_unsafe(path: &Path) -> bool {
    #[cfg(unix)]
    {
        let Ok(meta) = fs::metadata(path) else {
            return false;
        };
        // geteuid is a libc read of the process uid. It cannot unwind.
        let uid = unsafe { libc::geteuid() };
        meta.uid() == uid || meta.mode() & 0o022 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

pub(crate) fn write_private(path: PathBuf, bytes: &[u8]) -> Result<()> {
    ensure_data_gitignore(&path)?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    Ok(())
}

pub fn signature_seed(text: &str) -> Result<[u8; 32]> {
    signature::decode_seed(text)
        .ok_or_else(|| Error::Forbidden("audit key is not a 32-byte seed".into()))
}

pub fn append_audit(store: &mut Store, kind: &str, payload: &str, seed: &[u8; 32]) -> Result<()> {
    audit_log::append(store, kind, payload, wall_ms(), seed)?;
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
    fn token_must_be_at_least_32_random_bytes() {
        let dir = std::env::temp_dir().join(format!("dasdevbot-token-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let short = build_app(
            Config {
                data: dir.join("short.sqlite"),
                web_root: None,
                role: "server".into(),
                token: Some("a".repeat(MIN_TOKEN_BYTES - 1)),
            },
            Box::new(MockProvider::new()),
        );
        match short {
            Err(err) => assert!(err.to_string().contains("32"), "{err}"),
            Ok(_) => panic!("short token was accepted"),
        }

        let (minted, _) = build_app(
            Config {
                data: dir.join("minted.sqlite"),
                web_root: None,
                role: "executor".into(),
                token: None,
            },
            Box::new(MockProvider::new()),
        )
        .unwrap();
        assert_eq!(minted.token.len(), MIN_TOKEN_BYTES * 2);
        let role = fs::read_to_string(role_path(&dir.join("minted.sqlite"))).unwrap();
        assert_eq!(role, "executor");
        assert!(role_file_is_unsafe(&role_path(&dir.join("minted.sqlite"))));

        let supplied = "b".repeat(MIN_TOKEN_BYTES);
        let (app, _) = build_app(
            Config {
                data: dir.join("supplied.sqlite"),
                web_root: None,
                role: "executor".into(),
                token: Some(format!("  {supplied}  ")),
            },
            Box::new(MockProvider::new()),
        )
        .unwrap();
        assert_eq!(app.token, supplied);
    }

    #[test]
    fn a_broken_audit_log_refuses_to_start() {
        let dir = std::env::temp_dir().join(format!("dasdevbot-audit-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let data = dir.join("db.sqlite");
        let mut store = Store::open(&data).unwrap();
        let seed = [3u8; 32];
        audit_log::append(&mut store, "one", "alpha", 10, &seed).unwrap();
        store
            .connection()
            .execute(
                "INSERT INTO audit_log (seq, prev_hash, payload_hash, tip_hash, kind, payload, created_at, signature)
                 VALUES (2, 'genesis', 'nope', 'nope', 'forged', 'gamma', 30, '00')",
                [],
            )
            .unwrap();
        drop(store);
        fs::write(audit_key_path(&data), signature::encode_seed(&seed)).unwrap();
        let started = build_app(
            Config {
                data,
                web_root: None,
                role: "executor".into(),
                token: Some("0123456789abcdef0123456789abcdef".into()),
            },
            Box::new(MockProvider::new()),
        );
        match started {
            Err(err) => assert!(err.to_string().contains("audit"), "{err}"),
            Ok(_) => panic!("a forged audit log was accepted"),
        }
    }

    #[test]
    fn star_gitignore_is_written_only_for_a_data_dir_this_process_creates() {
        let root = std::env::temp_dir().join(format!("dasdevbot-ignore-{}", uuid::Uuid::new_v4()));
        let created = root.join("created");
        Store::open(&created.join("db.sqlite")).unwrap();
        assert_eq!(
            fs::read_to_string(created.join(".gitignore")).unwrap(),
            "*\n"
        );

        let existing = root.join("existing");
        fs::create_dir_all(&existing).unwrap();
        Store::open(&existing.join("db.sqlite")).unwrap();
        assert!(
            !existing.join(".gitignore").exists(),
            "an existing data dir must not gain a gitignore"
        );

        let kept = root.join("kept");
        fs::create_dir_all(&kept).unwrap();
        fs::write(kept.join(".gitignore"), "keep-me\n").unwrap();
        Store::open(&kept.join("db.sqlite")).unwrap();
        assert_eq!(
            fs::read_to_string(kept.join(".gitignore")).unwrap(),
            "keep-me\n"
        );
    }

    #[test]
    fn bare_data_path_does_not_write_a_gitignore_into_dot() {
        let scratch = std::env::temp_dir().join(format!("dasdevbot-dot-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&scratch).unwrap();
        let saved = std::env::current_dir().unwrap();
        let _restore = CurrentDirGuard(saved);
        std::env::set_current_dir(&scratch).unwrap();

        ensure_data_gitignore(Path::new("bare.sqlite")).unwrap();
        ensure_data_gitignore(Path::new("./bare.sqlite")).unwrap();
        assert!(
            !scratch.join(".gitignore").exists(),
            "a bare --data path must not write .gitignore into ."
        );
    }

    struct CurrentDirGuard(PathBuf);

    impl Drop for CurrentDirGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.0);
        }
    }
}
