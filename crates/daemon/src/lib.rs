//! dasdevbotd library surface. The binary is a thin CLI over [`serve`].

mod audit_log;
mod caps;
mod hello_key;
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
pub use signature::{DECISION_PURPOSE, UNDO_PURPOSE};
pub use server::{serve, url_exposes_bearer};
pub use store::Store;
pub use turn::audit_dev_env;

#[cfg(windows)]
pub use hello_key::sign_approval_message;

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
    /// Per-launch secrets. The window label is derived from these, not from JSON.
    pub window_secrets: WindowSecrets,
}

/// One secret per shell window, minted at process start.
pub struct WindowSecrets {
    pub card: String,
    pub main: String,
    pub settings: String,
}

impl WindowSecrets {
    pub fn resolve(&self, presented: &str) -> Option<&'static str> {
        if tokens_equal(&self.card, presented) {
            Some(dasdevbot_core::CARD_WINDOW)
        } else if tokens_equal(&self.main, presented) {
            Some(dasdevbot_core::MAIN_WINDOW)
        } else if tokens_equal(&self.settings, presented) {
            Some(dasdevbot_core::SETTINGS_WINDOW)
        } else {
            None
        }
    }
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

/// Sibling of `--data`. Authorization does not read this path.
pub fn role_path(data: &Path) -> PathBuf {
    sibling(data, ".role")
}

#[cfg(unix)]
const ROLE_FILE: &str = "/etc/dasdevbot/role";
#[cfg(windows)]
const ROLE_FILE: &str = r"C:\ProgramData\dasdevbot\role";

/// Fixed role file. `serve --role` is the other source, from the unit argv.
pub fn configured_role_path() -> &'static Path {
    Path::new(ROLE_FILE)
}

pub fn load_role_file() -> Result<String> {
    let path = configured_role_path();
    if !path.is_file() {
        return Err(Error::Forbidden("daemon role is not configured".into()));
    }
    if role_file_is_unsafe(path) {
        return Err(Error::Forbidden(
            "role file must be root-owned and not group- or world-writable".into(),
        ));
    }
    let role = fs::read_to_string(path)?.trim().to_string();
    if role.is_empty() {
        return Err(Error::Forbidden("daemon role is not configured".into()));
    }
    Ok(role)
}

pub fn audit_key_path(data: &Path) -> PathBuf {
    sibling(data, ".audit-key")
}

pub fn audit_tip_path(data: &Path) -> PathBuf {
    sibling(data, ".audit-tip")
}

pub fn window_secret_path(data: &Path, label: &str) -> PathBuf {
    sibling(data, &format!(".window-{label}"))
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
    let mut store = Store::open(&config.data)?;
    let audit_seed = load_or_create_audit_seed(&config.data, &store)?;
    if !audit_log::verify(&store, &audit_seed)? {
        eprintln!("dasdevbotd: audit log failed verification; refusing to start");
        return Err(Error::Forbidden("audit log failed verification".into()));
    }
    audit_log::audit_grant_seed(&mut store, wall_ms(), &audit_seed)?;
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
    let window_secrets = mint_window_secrets(&config.data)?;
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
        window_secrets,
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

fn mint_window_secrets(data: &Path) -> Result<WindowSecrets> {
    let card = mint_token();
    let main = mint_token();
    let settings = mint_token();
    write_private(window_secret_path(data, "card"), card.as_bytes())?;
    write_private(window_secret_path(data, "main"), main.as_bytes())?;
    write_private(window_secret_path(data, "settings"), settings.as_bytes())?;
    Ok(WindowSecrets {
        card,
        main,
        settings,
    })
}

pub(crate) fn tokens_equal(left: &str, right: &str) -> bool {
    use subtle::ConstantTimeEq;
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    bool::from(left.ct_eq(right))
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

/// True unless the file is root-owned, not owned by this process, and not
/// group- or world-writable. A root caller therefore always refuses the file.
/// A metadata error and non-unix targets fail closed: the file is unsafe.
pub(crate) fn role_file_is_unsafe(path: &Path) -> bool {
    #[cfg(unix)]
    {
        let Ok(meta) = fs::metadata(path) else {
            return true;
        };
        // geteuid is a libc read of the process uid. It cannot unwind.
        let uid = unsafe { libc::geteuid() };
        meta.uid() != 0 || meta.uid() == uid || meta.mode() & 0o022 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        true
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

#[cfg(test)]
pub(crate) fn test_window_secrets() -> WindowSecrets {
    WindowSecrets {
        card: "c".repeat(64),
        main: "m".repeat(64),
        settings: "s".repeat(64),
    }
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
        assert_eq!(minted.role, "executor");
        let data = dir.join("minted.sqlite");
        assert!(!role_path(&data).exists());
        let owned = dir.join("owned.role");
        fs::write(&owned, "executor").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&owned).unwrap().permissions();
            perms.set_mode(0o666);
            fs::set_permissions(&owned, perms).unwrap();
        }
        assert!(role_file_is_unsafe(&owned));
        assert_ne!(role_path(&data), configured_role_path());

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
    fn a_caller_owned_or_missing_role_file_is_unsafe() {
        let dir = std::env::temp_dir().join(format!("dasdevbot-role-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let mine = dir.join("mine.role");
        fs::write(&mine, "executor").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&mine, fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(role_file_is_unsafe(&mine));
        assert!(role_file_is_unsafe(&dir.join("missing.role")));
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
    fn a_deleted_database_cannot_mint_a_new_audit_genesis() {
        let dir = std::env::temp_dir().join(format!("dasdevbot-tip-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let data = dir.join("db.sqlite");
        let mut store = Store::open(&data).unwrap();
        audit_log::append(&mut store, "one", "alpha", 10, &[5u8; 32]).unwrap();
        let tip = fs::read_to_string(audit_tip_path(&data)).unwrap();
        assert!(!tip.trim().is_empty());
        drop(store);
        let display = data.display().to_string();
        fs::remove_file(&data).unwrap();
        let _ = fs::remove_file(format!("{display}-wal"));
        let _ = fs::remove_file(format!("{display}-shm"));
        match Store::open(&data) {
            Err(err) => assert!(err.to_string().contains("prior tip"), "{err}"),
            Ok(_) => panic!("a deleted database minted a new audit genesis"),
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
