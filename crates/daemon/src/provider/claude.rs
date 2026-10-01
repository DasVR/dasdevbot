//! Claude Pro through the official Claude Code CLI.
//!
//! The supported version is pinned. Tools are disabled. The prompt is stdin.
//! The child gets `HOME={claude_home}` and a dedicated
//! `CLAUDE_CONFIG_DIR={claude_home}/claude-config`, and runs with `--restricted`,
//! `--safe-mode`, `--setting-sources project,local` and `disableAllHooks`, so no
//! settings-file hook or `apiKeyHelper` command runs.
//! `--dangerously-skip-permissions` is never passed, and `claude setup-token`
//! output is never read. Stream events other than `rate_limit_event` and
//! `result` are dropped and the raw stream is never logged.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use serde_json::Value;

use super::{
    log_provider, Completion, CompletionRequest, Headroom, LimitReached, ProviderError,
    QuotaSignal, UsageReport,
};

pub const CLAUDE_CLI_VERSION: &str = "2.1.285";
/// Documented empty MCP server map. `--strict-mcp-config` then ignores every other MCP config.
pub const EMPTY_MCP_CONFIG: &str = r#"{"mcpServers":{}}"#;
/// Documented `--disallowedTools` value that removes every tool, including MCP tools.
pub const DISALLOWED_TOOLS: &str = "*";
pub const CLAUDE_CHILD_PATH: &str = "/usr/bin:/bin";
pub const SYSTEM_PROMPT_NAME: &str = "system-prompt.txt";
/// CLI reference: `--setting-sources` is a comma-separated list of `user`, `project`, and `local`.
/// There is no documented empty or `none` value. An empty string was reported broken from
/// CLI 2.1.59 onward, and 2.1.285 is newer than that, so this stays `project,local` to omit
/// `user`. The per-call cwd is a mode-0700 directory under `claude_home`, so project and local
/// files are not loaded from `/tmp` or from the operator's home.
pub const SETTING_SOURCES: &str = "project,local";
/// CLI reference: `--settings` accepts inline JSON and overrides file settings for the session.
/// The hooks guide uses this to set `disableAllHooks`.
pub const DISABLE_HOOKS_SETTINGS: &str = r#"{"disableAllHooks":true}"#;
/// Dedicated `CLAUDE_CONFIG_DIR` under `claude_home`. The child never reads `~/.claude`,
/// the operator's config dir, or an inherited `CLAUDE_CONFIG_DIR`. Log in once with the
/// same value so the credentials land here (deploy/ubuntu/README.md).
pub const CLAUDE_CONFIG_DIR_NAME: &str = "claude-config";
/// Flags that must be on every completion argv. `complete` fails closed if one is missing.
///
/// Checked offline against 2.1.285 under `unshare -rn` (no network), with a planted
/// SessionStart/UserPromptSubmit hook and an `apiKeyHelper` command in the config dir and
/// in the cwd's `.claude/settings.json`:
/// - no flags: every hook and the project `apiKeyHelper` ran.
/// - `--setting-sources project,local` alone: the project hooks still ran.
/// - `--settings '{"disableAllHooks":true}'`: no hook ran, but the project `apiKeyHelper` ran.
/// - `--safe-mode`: no hook ran, but the project `apiKeyHelper` ran.
/// - `--restricted`: nothing ran. It ignores user, project and local settings files.
///
/// The full set below ran nothing and was accepted together.
pub const REQUIRED_ISOLATION_ARGS: &[&str] = &[
    "--strict-mcp-config",
    "--disable-slash-commands",
    "--no-session-persistence",
    "--safe-mode",
    "--restricted",
];

const STDIN_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

const NO_RATE: &str = "claude-cli stream had no rate_limit_event. No account quota is invented.";

#[derive(Debug, Clone, PartialEq)]
pub struct RateObservation {
    pub status: RateStatus,
    pub resets_at: Option<i64>,
    pub utilization_pct: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateStatus {
    Allowed,
    Warning,
    Rejected,
    /// A `rate_limit_event` whose status could not be read. Treated like `Rejected`
    /// for scheduling: the job pauses and nothing is assumed to be allowed.
    Unknown,
}

impl RateStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Warning => "warning",
            Self::Rejected => "rejected",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileIdentity {
    inode: u64,
    size: u64,
    mtime_sec: i64,
    mtime_nsec: i64,
    ctime_sec: i64,
    ctime_nsec: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HashedFile {
    path: PathBuf,
    identity: FileIdentity,
    sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PinStamp {
    files: Vec<HashedFile>,
}

#[derive(Debug)]
pub struct ClaudeCli {
    model: Option<String>,
    claude_home: PathBuf,
    /// Snapshot of the bytes hashed at startup. Every exec uses this fd.
    memfd: File,
    checked: Mutex<PinStamp>,
}

impl ClaudeCli {
    pub fn open(
        bin: PathBuf,
        model: Option<String>,
        claude_home: PathBuf,
        expected_sha256: &[[u8; 32]],
    ) -> Result<Self, ProviderError> {
        let bin = resolve_executable(bin)?;
        let claude_home = require_claude_home(claude_home)?;
        require_config_dir(&claude_home)?;
        warn_install_owner(&bin);
        let identity = file_identity(&bin)?;
        let (memfd, sha256) = snapshot_memfd(&bin)?;
        if let Some(expected) = expected_sha256.first() {
            if expected != &sha256 {
                return Err(ProviderError::Failed(
                    "claude CLI sha256 does not match claude_sha256".into(),
                ));
            }
        }
        if expected_sha256.len() > 1 {
            return Err(ProviderError::Failed(
                "claude_sha256 has more digests than the resolved binary".into(),
            ));
        }
        let files = vec![HashedFile {
            path: bin.clone(),
            identity,
            sha256,
        }];
        ensure_version(&exec_path(&memfd), &claude_home)?;
        Ok(Self {
            checked: Mutex::new(PinStamp { files }),
            claude_home,
            memfd,
            model,
        })
    }

    pub fn complete(
        &self,
        req: &CompletionRequest,
        _charge: &mut dyn FnMut(&super::RetryCost) -> Result<(), ProviderError>,
    ) -> Result<Completion, ProviderError> {
        self.ensure_current()?;
        let dir = fresh_workdir(&self.claude_home)?;
        let prompt_file = write_system_prompt(&self.claude_home, &req.system)?;
        let _cleanup = DirGuard {
            cwd: dir.clone(),
            prompt: prompt_file.clone(),
        };
        let args = claude_command_args(self.model.as_deref(), &prompt_file);
        refuse_forbidden_args(&args)?;
        require_isolation_args(&args)?;
        require_config_dir(&self.claude_home)?;
        let stdout = run_cli(
            &exec_path(&self.memfd),
            &args,
            &req.user,
            &dir,
            &self.claude_home,
        )?;
        interpret_stream(&stdout, self.model.as_deref())
    }

    fn ensure_current(&self) -> Result<(), ProviderError> {
        let mut cached = self.checked.lock().expect("claude version");
        let mut refreshed = Vec::new();
        for file in &cached.files {
            let identity = file_identity(&file.path)?;
            if identity == file.identity {
                refreshed.push(identity);
                continue;
            }
            let sha256 = sha256_file(&file.path)?;
            if sha256 != file.sha256 {
                return Err(ProviderError::Failed(
                    "claude CLI sha256 changed since startup".into(),
                ));
            }
            refreshed.push(identity);
        }
        for (file, identity) in cached.files.iter_mut().zip(refreshed) {
            file.identity = identity;
        }
        Ok(())
    }

    pub fn detail(&self) -> String {
        match &self.model {
            Some(model) => format!("claude-cli {CLAUDE_CLI_VERSION} model {model}"),
            None => format!("claude-cli {CLAUDE_CLI_VERSION}"),
        }
    }
}

pub fn claude_command_args(model: Option<&str>, system_prompt_file: &Path) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--tools".to_string(),
        String::new(),
        "--disallowedTools".to_string(),
        DISALLOWED_TOOLS.to_string(),
        "--strict-mcp-config".to_string(),
        "--mcp-config".to_string(),
        EMPTY_MCP_CONFIG.to_string(),
        "--max-turns".to_string(),
        "1".to_string(),
        "--disable-slash-commands".to_string(),
        "--no-session-persistence".to_string(),
        "--safe-mode".to_string(),
        "--restricted".to_string(),
        "--setting-sources".to_string(),
        SETTING_SOURCES.to_string(),
        "--settings".to_string(),
        DISABLE_HOOKS_SETTINGS.to_string(),
        "--system-prompt-file".to_string(),
        system_prompt_file.display().to_string(),
    ];
    if let Some(model) = model.filter(|model| !model.is_empty()) {
        args.push("--model".to_string());
        args.push(model.to_string());
    }
    args
}

pub fn child_env_from(claude_home: &Path, vars: &[(&str, &str)]) -> Vec<(String, String)> {
    const KEEP: &[&str] = &["USER", "LANG", "TMPDIR"];
    let mut out: Vec<(String, String)> = vars
        .iter()
        .filter(|(key, _)| KEEP.contains(key))
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect();
    out.push(("HOME".to_string(), claude_home.display().to_string()));
    out.push((
        "CLAUDE_CONFIG_DIR".to_string(),
        claude_config_dir(claude_home).display().to_string(),
    ));
    out.push(("PATH".to_string(), CLAUDE_CHILD_PATH.to_string()));
    out.push(("DISABLE_AUTOUPDATER".to_string(), "1".to_string()));
    out.push(("DISABLE_UPDATES".to_string(), "1".to_string()));
    out
}

fn child_env(claude_home: &Path) -> Vec<(String, String)> {
    let vars: Vec<(String, String)> = ["USER", "LANG", "TMPDIR"]
        .into_iter()
        .filter_map(|key| {
            std::env::var(key)
                .ok()
                .map(|value| (key.to_string(), value))
        })
        .collect();
    let borrowed: Vec<(&str, &str)> = vars
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    child_env_from(claude_home, &borrowed)
}

pub fn claude_config_dir(claude_home: &Path) -> PathBuf {
    claude_home.join(CLAUDE_CONFIG_DIR_NAME)
}

/// Creates `{claude_home}/claude-config` mode 0700, or accepts an existing one only when it
/// is a real directory (not a symlink) owned by this process and mode 0700.
fn require_config_dir(claude_home: &Path) -> Result<PathBuf, ProviderError> {
    let dir = claude_config_dir(claude_home);
    match fs::symlink_metadata(&dir) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&dir)
                .map_err(|err| {
                    ProviderError::Failed(format!(
                        "could not create the claude config dir ({})",
                        err.kind()
                    ))
                })?;
            set_mode(&dir, 0o700)?;
        }
        Err(err) => {
            return Err(ProviderError::Failed(format!(
                "claude config dir could not be read ({})",
                err.kind()
            )));
        }
        Ok(meta) => {
            if !meta.file_type().is_dir() {
                return Err(ProviderError::Failed(
                    "refusing the claude config dir because it is not a directory".into(),
                ));
            }
            let euid = unsafe { libc::geteuid() };
            if meta.uid() != euid {
                return Err(ProviderError::Failed(
                    "refusing the claude config dir because it is not owned by this process".into(),
                ));
            }
            if meta.mode() & 0o777 != 0o700 {
                return Err(ProviderError::Failed(
                    "refusing the claude config dir because it is not mode 0700".into(),
                ));
            }
        }
    }
    Ok(dir)
}

fn require_isolation_args(args: &[String]) -> Result<(), ProviderError> {
    let has_pair = |flag: &str, value: &str| {
        args.windows(2)
            .any(|pair| pair[0] == flag && pair[1] == value)
    };
    let ok = REQUIRED_ISOLATION_ARGS
        .iter()
        .all(|flag| args.iter().any(|arg| arg == flag))
        && has_pair("--setting-sources", SETTING_SOURCES)
        && has_pair("--settings", DISABLE_HOOKS_SETTINGS)
        && has_pair("--mcp-config", EMPTY_MCP_CONFIG)
        && has_pair("--disallowedTools", DISALLOWED_TOOLS)
        && has_pair("--tools", "");
    if ok {
        Ok(())
    } else {
        Err(ProviderError::Failed(
            "refusing a claude argv without the isolation flags".into(),
        ))
    }
}

fn refuse_forbidden_args(args: &[String]) -> Result<(), ProviderError> {
    let mut previous = "";
    for arg in args {
        let bypass = arg == "--dangerously-skip-permissions"
            || arg == "--allow-dangerously-skip-permissions"
            || arg == "--permission-mode=bypassPermissions"
            || (previous == "--permission-mode" && arg == "bypassPermissions")
            || arg.contains("setup-token");
        if bypass {
            return Err(ProviderError::Failed(
                "refusing a claude permissions bypass or a setup-token".into(),
            ));
        }
        previous = arg;
    }
    Ok(())
}

fn resolve_executable(bin: PathBuf) -> Result<PathBuf, ProviderError> {
    let candidate = if bin.is_absolute() {
        bin
    } else {
        let path_var = std::env::var_os("PATH").unwrap_or_default();
        let mut found = None;
        for dir in std::env::split_paths(&path_var) {
            let full = dir.join(&bin);
            if full.is_file() {
                found = Some(full);
                break;
            }
        }
        found.ok_or_else(|| {
            ProviderError::Unavailable(format!("claude CLI not found at {}", bin.display()))
        })?
    };
    if !candidate.is_file() {
        return Err(ProviderError::Unavailable(format!(
            "claude CLI not found at {}",
            candidate.display()
        )));
    }
    fs::canonicalize(&candidate).map_err(|err| {
        ProviderError::Failed(format!(
            "claude CLI path could not be resolved ({})",
            err.kind()
        ))
    })
}

fn require_claude_home(path: PathBuf) -> Result<PathBuf, ProviderError> {
    if !path.is_dir() {
        return Err(ProviderError::Failed(format!(
            "claude_home is not a directory ({})",
            path.display()
        )));
    }
    let canonical = fs::canonicalize(&path).map_err(|err| {
        ProviderError::Failed(format!(
            "claude_home could not be resolved ({})",
            err.kind()
        ))
    })?;
    if personal_home_rejected(&canonical) {
        return Err(ProviderError::Failed(
            "refusing claude_home because it is the invoking user's home".into(),
        ));
    }
    let meta = fs::metadata(&canonical).map_err(|err| {
        ProviderError::Failed(format!("claude_home could not be read ({})", err.kind()))
    })?;
    let euid = unsafe { libc::geteuid() };
    if meta.uid() != euid {
        return Err(ProviderError::Failed(
            "refusing claude_home because it is not owned by this process".into(),
        ));
    }
    if meta.mode() & 0o777 != 0o700 {
        return Err(ProviderError::Failed(
            "refusing claude_home because it is not mode 0700".into(),
        ));
    }
    Ok(canonical)
}

/// A normal user's home and `$HOME` are refused. A system account may use its own
/// passwd home only when its uid is below 1000 and `pw_shell` is `nologin` or `false`.
fn personal_home_rejected(canonical: &Path) -> bool {
    let euid = unsafe { libc::geteuid() };
    let ruid = unsafe { libc::getuid() };
    let own = passwd_account(euid);
    let real = passwd_account(ruid);
    let own_home = own
        .as_ref()
        .and_then(|account| fs::canonicalize(&account.home).ok());
    let real_home = real
        .as_ref()
        .and_then(|account| fs::canonicalize(&account.home).ok());
    let env = std::env::var_os("HOME").and_then(|path| fs::canonicalize(path).ok());
    let is_own = own_home.as_deref() == Some(canonical);
    let shell = own.as_ref().map(|account| account.shell.as_str());
    if is_own && service_home_exception(euid, ruid, shell) {
        return false;
    }
    is_own || real_home.as_deref() == Some(canonical) || env.as_deref() == Some(canonical)
}

fn service_home_exception(euid: u32, ruid: u32, shell: Option<&str>) -> bool {
    ruid == euid && euid < 1000 && shell.is_some_and(shell_is_nologin_or_false)
}

fn shell_is_nologin_or_false(shell: &str) -> bool {
    match Path::new(shell).file_name().and_then(|name| name.to_str()) {
        Some("nologin" | "false") => true,
        Some(_) | None => false,
    }
}

struct PasswdAccount {
    home: PathBuf,
    shell: String,
}

fn passwd_account(uid: u32) -> Option<PasswdAccount> {
    let mut pwd = unsafe { std::mem::zeroed::<libc::passwd>() };
    let mut buf = vec![0u8; 16 * 1024];
    let mut result = std::ptr::null_mut();
    let rc = unsafe {
        libc::getpwuid_r(
            uid,
            &mut pwd,
            buf.as_mut_ptr().cast::<libc::c_char>(),
            buf.len(),
            &mut result,
        )
    };
    if rc != 0 || result.is_null() {
        return None;
    }
    let home = unsafe { std::ffi::CStr::from_ptr(pwd.pw_dir) }
        .to_str()
        .ok()
        .map(PathBuf::from)?;
    let shell = unsafe { std::ffi::CStr::from_ptr(pwd.pw_shell) }
        .to_str()
        .ok()?
        .to_string();
    Some(PasswdAccount { home, shell })
}

fn warn_install_owner(path: &Path) {
    warn_root_owned(path);
    let mut cursor = path.parent();
    while let Some(dir) = cursor {
        if dir.as_os_str().is_empty() || dir == Path::new("/") {
            break;
        }
        warn_root_owned(dir);
        cursor = dir.parent();
    }
}

fn warn_root_owned(path: &Path) {
    let Ok(meta) = fs::metadata(path) else {
        return;
    };
    if meta.uid() != 0 {
        log_provider(&format!(
            "claude-cli install path is not root-owned at {}",
            path.display()
        ));
    }
    if meta.is_dir() && meta.mode() & 0o022 != 0 {
        log_provider(&format!(
            "claude-cli install path is group or world writable at {}",
            path.display()
        ));
    }
}

fn file_identity(path: &Path) -> Result<FileIdentity, ProviderError> {
    let meta = fs::metadata(path).map_err(|err| {
        ProviderError::Failed(format!("claude CLI metadata failed ({})", err.kind()))
    })?;
    Ok(FileIdentity {
        inode: meta.ino(),
        size: meta.len(),
        mtime_sec: meta.mtime(),
        mtime_nsec: meta.mtime_nsec(),
        ctime_sec: meta.ctime(),
        ctime_nsec: meta.ctime_nsec(),
    })
}

fn sha256_file(path: &Path) -> Result<[u8; 32], ProviderError> {
    let mut file = File::open(path).map_err(|err| {
        ProviderError::Failed(format!("claude CLI could not be hashed ({})", err.kind()))
    })?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let read = file.read(&mut buf).map_err(|err| {
            ProviderError::Failed(format!("claude CLI could not be hashed ({})", err.kind()))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(hasher.finalize().into())
}

fn snapshot_memfd(path: &Path) -> Result<(File, [u8; 32]), ProviderError> {
    let mut source = File::open(path).map_err(|err| {
        ProviderError::Failed(format!("claude CLI could not be hashed ({})", err.kind()))
    })?;
    let name = std::ffi::CString::new("dasdevbot-claude")
        .map_err(|_| ProviderError::Failed("claude CLI could not be snapshotted".into()))?;
    let raw =
        unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
    if raw < 0 {
        return Err(ProviderError::Failed(
            "claude CLI could not be snapshotted".into(),
        ));
    }
    let mut memfd = unsafe { File::from_raw_fd(raw) };
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    let mut checked_magic = false;
    loop {
        let read = source.read(&mut buf).map_err(|err| {
            ProviderError::Failed(format!("claude CLI could not be hashed ({})", err.kind()))
        })?;
        if read == 0 {
            break;
        }
        if !checked_magic {
            if read < 4 || buf[..4] != *b"\x7fELF" {
                return Err(ProviderError::Failed(
                    "refusing a claude script target; only a native ELF binary is accepted".into(),
                ));
            }
            checked_magic = true;
        }
        hasher.update(&buf[..read]);
        memfd.write_all(&buf[..read]).map_err(|err| {
            ProviderError::Failed(format!(
                "claude CLI could not be snapshotted ({})",
                err.kind()
            ))
        })?;
    }
    if !checked_magic {
        return Err(ProviderError::Failed(
            "refusing a claude script target; only a native ELF binary is accepted".into(),
        ));
    }
    memfd.seek(SeekFrom::Start(0)).map_err(|err| {
        ProviderError::Failed(format!(
            "claude CLI could not be snapshotted ({})",
            err.kind()
        ))
    })?;
    let fd = memfd.as_raw_fd();
    let sealed = unsafe {
        libc::fcntl(
            fd,
            libc::F_ADD_SEALS,
            libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL,
        )
    };
    if sealed < 0 {
        return Err(ProviderError::Failed(
            "claude CLI snapshot could not be sealed".into(),
        ));
    }
    // CLOEXEC stays set. exec of /proc/self/fd/N loads the ELF, then the kernel
    // closes this fd, so the CLI and its descendants do not inherit the snapshot.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(ProviderError::Failed(
            "claude CLI snapshot could not be marked close-on-exec".into(),
        ));
    }
    Ok((memfd, hasher.finalize().into()))
}

fn exec_path(memfd: &File) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", memfd.as_raw_fd()))
}

fn version_output(bin: &Path, claude_home: &Path) -> Result<Output, ProviderError> {
    let mut attempt = 0;
    loop {
        let mut cmd = Command::new(bin);
        cmd.arg("--version")
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // pre_exec forces fork. The snapshot fd is inherited across the fork and
        // closed on exec because it is CLOEXEC, including in any descendant.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        for (key, value) in child_env(claude_home) {
            cmd.env(key, value);
        }
        match cmd.output() {
            Ok(output) => return Ok(output),
            Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 5 => {
                attempt += 1;
                thread::sleep(Duration::from_millis(20));
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(ProviderError::Unavailable(format!(
                    "claude CLI not found at {}",
                    bin.display()
                )));
            }
            Err(err) => {
                return Err(ProviderError::Failed(format!(
                    "claude --version failed to start ({})",
                    err.kind()
                )));
            }
        }
    }
}

fn ensure_version(bin: &Path, claude_home: &Path) -> Result<(), ProviderError> {
    let output = version_output(bin, claude_home)?;
    if !output.status.success() {
        return Err(ProviderError::Failed(
            "claude --version exited without printing the supported version".into(),
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let found = text.split_whitespace().next().unwrap_or("");
    if found != CLAUDE_CLI_VERSION {
        return Err(ProviderError::Failed(format!(
            "claude CLI version {found} does not match the supported version {CLAUDE_CLI_VERSION}"
        )));
    }
    Ok(())
}

fn write_system_prompt(claude_home: &Path, system: &str) -> Result<PathBuf, ProviderError> {
    let parent = claude_home.join("prompts");
    ensure_private_dir(&parent)?;
    let path = parent.join(format!("{}-{SYSTEM_PROMPT_NAME}", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|err| {
            ProviderError::Failed(format!(
                "could not write the claude system prompt ({})",
                err.kind()
            ))
        })?;
    file.write_all(system.as_bytes()).map_err(|err| {
        ProviderError::Failed(format!(
            "could not write the claude system prompt ({})",
            err.kind()
        ))
    })?;
    Ok(path)
}

fn ensure_private_dir(path: &Path) -> Result<(), ProviderError> {
    fs::create_dir_all(path).map_err(|err| {
        ProviderError::Failed(format!(
            "could not create {} ({})",
            path.display(),
            err.kind()
        ))
    })?;
    set_mode(path, 0o700)
}

fn set_mode(path: &Path, mode: u32) -> Result<(), ProviderError> {
    let mut perms = fs::metadata(path)
        .map_err(|err| {
            ProviderError::Failed(format!(
                "could not read {} ({})",
                path.display(),
                err.kind()
            ))
        })?
        .permissions();
    perms.set_mode(mode);
    fs::set_permissions(path, perms).map_err(|err| {
        ProviderError::Failed(format!(
            "could not set the mode on {} ({})",
            path.display(),
            err.kind()
        ))
    })
}

fn run_cli(
    bin: &Path,
    args: &[String],
    prompt: &str,
    dir: &Path,
    claude_home: &Path,
) -> Result<String, ProviderError> {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .current_dir(dir)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    for (key, value) in child_env(claude_home) {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            ProviderError::Unavailable(format!("claude CLI not found at {}", bin.display()))
        } else {
            ProviderError::Failed(format!("claude CLI failed to start ({})", err.kind()))
        }
    })?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let err_handle = thread::spawn(move || drain_pipe(stderr));
    let (tx, rx) = mpsc::channel();
    let out_handle = thread::spawn(move || {
        let Some(stdout) = stdout else {
            return;
        };
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    if let Err(err) = write_prompt(&mut child, prompt) {
        drop(out_handle);
        drop(err_handle);
        return Err(err);
    }
    let collected = match read_stdout_lines(&mut child, &rx) {
        Ok(text) => text,
        Err(err) => {
            drop(out_handle);
            drop(err_handle);
            return Err(err);
        }
    };
    kill_group(&mut child);
    drop(out_handle);
    drop(err_handle);
    Ok(collected)
}

fn write_prompt(child: &mut Child, prompt: &str) -> Result<(), ProviderError> {
    let Some(mut stdin) = child.stdin.take() else {
        return Ok(());
    };
    let prompt = prompt.to_string();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = stdin.write_all(prompt.as_bytes());
        let _ = tx.send(result);
    });
    match rx.recv_timeout(STDIN_WRITE_TIMEOUT) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => {
            kill_group(child);
            Err(ProviderError::Failed(format!(
                "claude CLI stdin failed ({})",
                err.kind()
            )))
        }
        Err(RecvTimeoutError::Timeout) => {
            kill_group(child);
            Err(ProviderError::Failed("claude CLI stdin timed out".into()))
        }
        Err(RecvTimeoutError::Disconnected) => {
            kill_group(child);
            Err(ProviderError::Failed(
                "claude CLI stdin failed (disconnected)".into(),
            ))
        }
    }
}

fn kill_group(child: &mut Child) {
    let pid = child.id() as i32;
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    let _ = child.wait();
}

fn read_stdout_lines(
    child: &mut Child,
    rx: &mpsc::Receiver<std::io::Result<String>>,
) -> Result<String, ProviderError> {
    let started = Instant::now();
    let limit = Duration::from_secs(120);
    let mut collected = String::new();
    loop {
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(Ok(line)) => {
                if let Some(event) = tool_event_name(&line) {
                    log_provider(&format!("claude-cli event={event}"));
                    kill_group(child);
                    return Err(ProviderError::ToolUseAttempted {
                        cli_version: CLAUDE_CLI_VERSION.into(),
                        event: event.into(),
                    });
                }
                collected.push_str(&line);
                collected.push('\n');
            }
            Ok(Err(err)) => {
                kill_group(child);
                return Err(ProviderError::Failed(format!(
                    "claude CLI stdout failed ({})",
                    err.kind()
                )));
            }
            Err(RecvTimeoutError::Timeout) => {
                if started.elapsed() >= limit {
                    kill_group(child);
                    return Err(ProviderError::Failed("claude CLI timed out".into()));
                }
            }
            Err(RecvTimeoutError::Disconnected) => match child.try_wait() {
                Ok(Some(_)) => return Ok(collected),
                Ok(None) => {
                    kill_group(child);
                    return Err(ProviderError::Failed(
                        "claude CLI stdout closed before the process exited".into(),
                    ));
                }
                Err(err) => {
                    return Err(ProviderError::Failed(format!(
                        "claude CLI wait failed ({})",
                        err.kind()
                    )));
                }
            },
        }
    }
}

fn drain_pipe(pipe: Option<impl Read>) {
    let Some(mut pipe) = pipe else {
        return;
    };
    let mut buf = [0u8; 1024];
    while pipe.read(&mut buf).unwrap_or(0) > 0 {}
}

fn tool_event_name(line: &str) -> Option<&'static str> {
    let value = serde_json::from_str::<Value>(line.trim()).ok()?;
    value_tool_event(&value)
}

fn value_tool_event(value: &Value) -> Option<&'static str> {
    match value {
        Value::Object(map) => {
            if let Some(kind) = map.get("type").and_then(|item| item.as_str()) {
                if kind == "tool_use" {
                    return Some("tool_use");
                }
                if kind == "tool_result" {
                    return Some("tool_result");
                }
            }
            map.values().find_map(value_tool_event)
        }
        Value::Array(items) => items.iter().find_map(value_tool_event),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn stream_tool_event(stream: &str) -> Option<&'static str> {
    stream.lines().find_map(tool_event_name)
}

fn fresh_workdir(claude_home: &Path) -> Result<PathBuf, ProviderError> {
    let parent = claude_home.join("claude-cwd");
    ensure_private_dir(&parent)?;
    let dir = parent.join(uuid::Uuid::new_v4().to_string());
    fs::create_dir(&dir).map_err(|err| {
        ProviderError::Failed(format!(
            "could not create a claude working directory ({})",
            err.kind()
        ))
    })?;
    set_mode(&dir, 0o700)?;
    Ok(dir)
}

struct DirGuard {
    cwd: PathBuf,
    prompt: PathBuf,
}

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.cwd);
        let _ = fs::remove_file(&self.prompt);
    }
}

struct ParsedStream {
    text: String,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    rate: Option<RateObservation>,
}

/// Parse a recorded stream-json transcript. Every event other than
/// `rate_limit_event` and `result` is discarded.
pub fn interpret_stream(stream: &str, model: Option<&str>) -> Result<Completion, ProviderError> {
    if let Some(event) = stream_tool_event(stream) {
        log_provider(&format!("claude-cli event={event}"));
        return Err(ProviderError::ToolUseAttempted {
            cli_version: CLAUDE_CLI_VERSION.into(),
            event: event.into(),
        });
    }
    let parsed = parse_stream(stream)?;
    if let Some(rate) = &parsed.rate {
        log_provider(&rate_log_line(rate));
        if let Some(limit) = pause_for(rate) {
            return Err(ProviderError::LimitReached(limit));
        }
    }
    let (quota, note) = match &parsed.rate {
        Some(rate) => {
            let detail = rate_detail(rate);
            (
                QuotaSignal::Reported {
                    detail: detail.clone(),
                },
                format!("claude-cli {detail}"),
            )
        }
        None => (
            QuotaSignal::Absent {
                detail: NO_RATE.into(),
            },
            NO_RATE.to_string(),
        ),
    };
    let usage_kind = if parsed.input_tokens.is_some() || parsed.output_tokens.is_some() {
        "provider"
    } else {
        "absent"
    };
    Ok(Completion::from_usage(
        parsed.text,
        model.unwrap_or("claude-cli"),
        "claude-cli",
        usage_kind,
        0,
        note,
        UsageReport {
            input_tokens: parsed.input_tokens,
            output_tokens: parsed.output_tokens,
            cached_input_tokens: None,
            quota,
            headroom: Headroom::unknown(),
        },
    ))
}

fn parse_stream(stream: &str) -> Result<ParsedStream, ProviderError> {
    let mut rate: Option<RateObservation> = None;
    let mut result: Option<ParsedStream> = None;
    let mut result_error = false;
    for line in stream.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            // stream-json escapes model text inside one JSON line, so a line that does
            // not parse came from the CLI itself. A broken rate event pauses.
            if line.contains("\"rate_limit_event\"") {
                rate = Some(prefer_rate(rate, RateObservation::unknown()));
            }
            continue;
        };
        if is_rate_event(&value) {
            rate = Some(prefer_rate(rate, parse_rate_event(&value)));
            continue;
        }
        if value.get("type").and_then(|item| item.as_str()) != Some("result") {
            continue;
        }
        if value.get("is_error").and_then(Value::as_bool) == Some(true) {
            // The text of an error result (for example a logout notice) is
            // never used as a draft and never logged.
            result_error = true;
            result = None;
            continue;
        }
        if result_error {
            continue;
        }
        if let Some(text) = value.get("result").and_then(|item| item.as_str()) {
            let text = text.trim().to_string();
            if text.is_empty() {
                continue;
            }
            result = Some(ParsedStream {
                text,
                input_tokens: value["usage"]["input_tokens"].as_u64(),
                output_tokens: value["usage"]["output_tokens"].as_u64(),
                rate: None,
            });
        }
    }
    let Some(mut parsed) = result else {
        if let Some(limit) = rate.as_ref().and_then(pause_for) {
            return Err(ProviderError::LimitReached(limit));
        }
        if result_error {
            return Err(ProviderError::Failed(
                "claude-cli result reported an error".into(),
            ));
        }
        return Err(ProviderError::Failed(
            "claude-cli returned no result".into(),
        ));
    };
    parsed.rate = rate;
    Ok(parsed)
}

fn prefer_rate(current: Option<RateObservation>, next: RateObservation) -> RateObservation {
    match current {
        Some(current) if severity(current.status) > severity(next.status) => current,
        _ => next,
    }
}

fn severity(status: RateStatus) -> u8 {
    match status {
        RateStatus::Allowed => 0,
        RateStatus::Warning => 1,
        RateStatus::Unknown => 2,
        RateStatus::Rejected => 3,
    }
}

/// `Rejected` and `Unknown` pause the job. `Allowed` and `Warning` do not.
fn pause_for(rate: &RateObservation) -> Option<LimitReached> {
    let message = match rate.status {
        RateStatus::Rejected => "claude-cli usage limit reached",
        RateStatus::Unknown => "claude-cli rate_limit_event was not understood; pausing",
        RateStatus::Allowed | RateStatus::Warning => return None,
    };
    Some(LimitReached {
        message: message.into(),
        resets_at: rate.resets_at,
    })
}

impl RateObservation {
    fn unknown() -> Self {
        Self {
            status: RateStatus::Unknown,
            resets_at: None,
            utilization_pct: None,
        }
    }
}

const RATE_INFO_KEYS: &[&str] = &[
    "rate_limit_info",
    "rateLimitInfo",
    "rate_limit",
    "rateLimit",
];
const RESETS_AT_KEYS: &[&str] = &["resetsAt", "resets_at", "resetAt", "reset_at"];
const UTILIZATION_KEYS: &[&str] = &["utilization", "utilization_fraction"];

/// A top-level `{"type":"rate_limit_event"}`, or a `system` event whose subtype names a
/// rate limit. Only the CLI emits these as separate stream lines.
fn is_rate_event(value: &Value) -> bool {
    let kind = value.get("type").and_then(Value::as_str);
    let subtype = value.get("subtype").and_then(Value::as_str);
    kind == Some("rate_limit_event")
        || (kind == Some("system") && matches!(subtype, Some("rate_limit_event" | "rate_limit")))
}

/// Tolerant read of a rate event. Never fails and never drops the event:
/// - The info object may be `rate_limit_info`, `rateLimitInfo`, `rate_limit` or `rateLimit`,
///   or the fields may sit on the event itself. Unknown and extra keys are ignored.
/// - `status` is matched case-insensitively. A missing, non-string or unrecognized status is
///   [`RateStatus::Unknown`], which pauses.
/// - `resetsAt` (or `resets_at`, `resetAt`, `reset_at`) may be an integer, a float or a
///   numeric string. Anything else, or a value that is not positive, is dropped; the status is
///   kept.
/// - `utilization` outside 0..=1, or not a finite number, drops only the percentage.
///
/// Re-validate the field set against a stream captured from Claude Code 2.1.285.
fn parse_rate_event(value: &Value) -> RateObservation {
    let Some(event) = value.as_object() else {
        return RateObservation::unknown();
    };
    let info = RATE_INFO_KEYS
        .iter()
        .find_map(|key| event.get(*key))
        .and_then(Value::as_object)
        .unwrap_or(event);
    let status = match info.get("status").and_then(Value::as_str) {
        Some(text) => rate_status(text),
        None => RateStatus::Unknown,
    };
    let resets_at = RESETS_AT_KEYS
        .iter()
        .find_map(|key| info.get(*key))
        .and_then(number_i64)
        .filter(|value| *value > 0);
    let utilization_pct = UTILIZATION_KEYS
        .iter()
        .find_map(|key| info.get(*key))
        .and_then(number_f64)
        .filter(|fraction| fraction.is_finite() && (0.0..=1.0).contains(fraction))
        .map(|fraction| fraction * 100.0);
    RateObservation {
        status,
        resets_at,
        utilization_pct,
    }
}

fn rate_status(text: &str) -> RateStatus {
    match text.trim().to_ascii_lowercase().as_str() {
        "allowed" | "ok" => RateStatus::Allowed,
        "allowed_warning" | "warning" | "warn" => RateStatus::Warning,
        "rejected" | "limited" | "blocked" | "exceeded" | "denied" => RateStatus::Rejected,
        _ => RateStatus::Unknown,
    }
}

fn number_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64().or_else(|| {
            number
                .as_f64()
                .filter(|float| float.is_finite() && float.abs() < 9.0e15)
                .map(|float| float as i64)
        }),
        Value::String(text) => {
            let text = text.trim();
            text.parse::<i64>().ok().or_else(|| {
                text.parse::<f64>()
                    .ok()
                    .filter(|float| float.is_finite() && float.abs() < 9.0e15)
                    .map(|float| float as i64)
            })
        }
        _ => None,
    }
}

fn number_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// Hex SHA-256 of the native Claude ELF. One digest is accepted. A second digest
/// fails because a script interpreter is not part of the pin.
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

fn rate_log_line(rate: &RateObservation) -> String {
    let resets = rate
        .resets_at
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_string());
    let pct = rate
        .utilization_pct
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_string());
    format!(
        "claude-cli rate_limit status={} resets_at={resets} utilization_pct={pct}",
        rate.status.as_str()
    )
}

fn rate_detail(rate: &RateObservation) -> String {
    let resets = rate
        .resets_at
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unavailable".to_string());
    match rate.utilization_pct {
        Some(pct) => format!(
            "rate_limit status={} resets_at={resets} utilization_pct={pct}",
            rate.status.as_str()
        ),
        None => format!(
            "rate_limit status={} resets_at={resets}; no utilization percentage was present",
            rate.status.as_str()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{start_log, take_log};
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Stdio};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    fn fake_claude_elf() -> &'static Path {
        static ELF: OnceLock<PathBuf> = OnceLock::new();
        ELF.get_or_init(|| {
            let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fake_claude.c");
            let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fake-claude");
            let status = Command::new("cc")
                .args(["-O2", "-o"])
                .arg(&out)
                .arg(&src)
                .status()
                .expect("cc");
            assert!(status.success(), "fake claude failed to compile");
            out
        })
        .as_path()
    }

    fn write_cli(dir: &Path, config: &str) -> PathBuf {
        let bin = dir.join("claude");
        let mut bytes = fs::read(fake_claude_elf()).unwrap();
        bytes.extend_from_slice(b"\nDASDEVBOT_FAKE_CFG\n");
        bytes.extend_from_slice(config.as_bytes());
        if !config.ends_with('\n') {
            bytes.push(b'\n');
        }
        fs::write(&bin, &bytes).unwrap();
        let mut perms = fs::metadata(&bin).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bin, perms).unwrap();
        bin
    }

    fn no_charge(_: &crate::provider::RetryCost) -> Result<(), ProviderError> {
        Ok(())
    }

    fn private_home(path: &Path) {
        fs::create_dir_all(path).unwrap();
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o700);
        fs::set_permissions(path, perms).unwrap();
    }

    fn open_at(dir: &Path, bin: PathBuf) -> ClaudeCli {
        let home = dir.join("claude-home");
        private_home(&home);
        ClaudeCli::open(bin, None, home, &[]).unwrap()
    }

    #[test]
    fn command_args_are_exact_and_the_persona_stays_off_argv() {
        let prompt = Path::new("/tmp/dasdevbot-claude/system-prompt.txt");
        let persona = "be brief PERSONA_SENTINEL";
        let args = claude_command_args(Some("opus"), prompt);
        assert_eq!(
            args,
            vec![
                "-p".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                "--verbose".to_string(),
                "--tools".to_string(),
                String::new(),
                "--disallowedTools".to_string(),
                "*".to_string(),
                "--strict-mcp-config".to_string(),
                "--mcp-config".to_string(),
                r#"{"mcpServers":{}}"#.to_string(),
                "--max-turns".to_string(),
                "1".to_string(),
                "--disable-slash-commands".to_string(),
                "--no-session-persistence".to_string(),
                "--safe-mode".to_string(),
                "--restricted".to_string(),
                "--setting-sources".to_string(),
                "project,local".to_string(),
                "--settings".to_string(),
                r#"{"disableAllHooks":true}"#.to_string(),
                "--system-prompt-file".to_string(),
                prompt.display().to_string(),
                "--model".to_string(),
                "opus".to_string(),
            ]
        );
        assert!(args
            .iter()
            .all(|arg| arg != persona && !arg.contains("PERSONA")));
        let bare = claude_command_args(None, prompt);
        assert!(!bare.iter().any(|arg| arg == "--model" || arg == "opus"));
        let home = Path::new("/var/lib/dasdevbot");
        let env = child_env_from(
            home,
            &[
                ("HOME", "/home/dev"),
                ("PATH", "/usr/local/bin:/usr/bin"),
                ("USER", "dev"),
                ("LANG", "C"),
                ("TMPDIR", "/tmp"),
                ("OLLAMA_API_KEY", "SENTINEL_KEY"),
                ("CLAUDE_CODE_OAUTH_TOKEN", "SENTINEL_OAUTH"),
                ("ANTHROPIC_API_KEY", "SENTINEL_ANTHROPIC"),
                ("DASDEVBOT_TOKEN", "SENTINEL_TOKEN"),
                ("CLAUDE_CONFIG_DIR", "/home/dev/.claude"),
            ],
        );
        assert_eq!(
            env.iter()
                .find(|(key, _)| key == "PATH")
                .map(|(_, value)| value.as_str()),
            Some(CLAUDE_CHILD_PATH)
        );
        assert_eq!(
            env.iter()
                .find(|(key, _)| key == "HOME")
                .map(|(_, value)| value.as_str()),
            Some("/var/lib/dasdevbot")
        );
        assert_eq!(env.iter().filter(|(key, _)| key == "HOME").count(), 1);
        assert_eq!(
            env.iter()
                .filter(|(key, _)| key == "CLAUDE_CONFIG_DIR")
                .map(|(_, value)| value.as_str())
                .collect::<Vec<_>>(),
            vec!["/var/lib/dasdevbot/claude-config"]
        );
        assert!(env
            .iter()
            .any(|(key, value)| key == "DISABLE_AUTOUPDATER" && value == "1"));
        assert!(env
            .iter()
            .any(|(key, value)| key == "DISABLE_UPDATES" && value == "1"));
        assert!(env.iter().all(|(key, _)| {
            !matches!(
                key.as_str(),
                "OLLAMA_API_KEY"
                    | "CLAUDE_CODE_OAUTH_TOKEN"
                    | "ANTHROPIC_API_KEY"
                    | "DASDEVBOT_TOKEN"
            )
        }));
        assert!(env.iter().all(|(_, value)| value != "/home/dev/.claude"));
    }

    #[test]
    fn an_argv_missing_any_isolation_flag_is_refused() {
        let full = claude_command_args(None, Path::new("system-prompt.txt"));
        assert!(require_isolation_args(&full).is_ok());
        for drop in REQUIRED_ISOLATION_ARGS {
            let args: Vec<String> = full.iter().filter(|arg| arg != drop).cloned().collect();
            assert!(require_isolation_args(&args).is_err(), "missing {drop}");
        }
        for (flag, value) in [
            ("--setting-sources", SETTING_SOURCES),
            ("--settings", DISABLE_HOOKS_SETTINGS),
            ("--mcp-config", EMPTY_MCP_CONFIG),
            ("--disallowedTools", DISALLOWED_TOOLS),
        ] {
            let mut args = full.clone();
            let at = args.iter().position(|arg| arg == flag).unwrap();
            args[at + 1] = format!("{value}x");
            assert!(require_isolation_args(&args).is_err(), "changed {flag}");
            let mut args = full.clone();
            args.remove(at + 1);
            args.remove(at);
            assert!(require_isolation_args(&args).is_err(), "missing {flag}");
        }
        let mut sources = full.clone();
        let at = sources
            .iter()
            .position(|arg| arg == "--setting-sources")
            .unwrap();
        sources[at + 1] = "user,project,local".into();
        assert!(require_isolation_args(&sources).is_err());
    }

    #[test]
    fn the_config_dir_is_created_private_and_a_symlink_is_refused() {
        let dir =
            std::env::temp_dir().join(format!("dasdevbot-claude-test-{}", uuid::Uuid::new_v4()));
        let home = dir.join("claude-home");
        private_home(&home);
        let made = require_config_dir(&home).unwrap();
        assert_eq!(made, home.join(CLAUDE_CONFIG_DIR_NAME));
        assert_eq!(fs::metadata(&made).unwrap().mode() & 0o777, 0o700);
        let mut perms = fs::metadata(&made).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&made, perms).unwrap();
        assert!(require_config_dir(&home).is_err());
        fs::remove_dir(&made).unwrap();
        let elsewhere = dir.join("operator-dot-claude");
        private_home(&elsewhere);
        std::os::unix::fs::symlink(&elsewhere, &made).unwrap();
        let err = require_config_dir(&home).unwrap_err();
        assert!(err.to_string().contains("not a directory"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bypass_flags_are_refused() {
        let base = claude_command_args(None, Path::new("system-prompt.txt"));
        assert!(refuse_forbidden_args(&base).is_ok());
        let cases = [
            vec!["--allow-dangerously-skip-permissions".to_string()],
            vec![
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
            ],
            vec!["--permission-mode=bypassPermissions".to_string()],
            vec!["--dangerously-skip-permissions".to_string()],
            vec!["claude-setup-token".to_string()],
        ];
        for extra in cases {
            let mut args = base.clone();
            args.extend(extra);
            let err = refuse_forbidden_args(&args).unwrap_err();
            assert!(
                matches!(err, ProviderError::Failed(ref message) if message.contains("bypass") || message.contains("setup-token")),
                "{err}"
            );
        }
    }

    #[test]
    fn stream_keeps_rate_and_result_and_drops_other_content() {
        let stream = r#"
{"type":"rate_limit_event","rate_limit_info":{"status":"allowed_warning","resetsAt":1700000000,"utilization":0.5}}
{"type":"assistant","message":{"content":[{"type":"text","text":"SENTINEL_DROPPED"}]}}
{"type":"result","result":"ok","usage":{"input_tokens":3,"output_tokens":1}}
"#;
        start_log();
        let completion = interpret_stream(stream, None).unwrap();
        let logs = take_log();
        assert_eq!(completion.text, "ok");
        assert_eq!(completion.input_tokens, 3);
        assert_eq!(completion.output_tokens, 1);
        assert!(logs.iter().any(|line| line.contains("status=warning")));
        assert!(logs.iter().all(|line| !line.contains("SENTINEL_DROPPED")));
        assert!(!completion.note.contains("SENTINEL_DROPPED"));
    }

    #[test]
    fn rejected_rate_is_limit_reached_without_the_result_text() {
        let stream = r#"
{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1700000000,"utilization":1.0}}
{"type":"result","result":"PROMPT_ECHO should not leak","usage":{"input_tokens":1,"output_tokens":1}}
"#;
        let err = interpret_stream(stream, None).unwrap_err();
        match err {
            ProviderError::LimitReached(limit) => {
                assert_eq!(limit.resets_at, Some(1_700_000_000));
                let text = limit.to_string();
                assert!(text.contains("resets_at=1700000000"));
                assert!(!text.contains("PROMPT_ECHO"));
            }
            other => panic!("expected limit, got {other}"),
        }
    }

    #[test]
    fn tool_use_fixture_fails_closed_without_logging_content() {
        let stream = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"SENTINEL_TOOL_BODY"}}]}}"#;
        start_log();
        let err = interpret_stream(stream, None).unwrap_err();
        let logs = take_log();
        assert!(matches!(
            err,
            ProviderError::ToolUseAttempted { ref event, .. } if event == "tool_use"
        ));
        assert!(!err.to_string().contains("SENTINEL_TOOL_BODY"));
        assert!(!err.to_string().contains("Bash"));
        assert_eq!(logs, vec!["claude-cli event=tool_use".to_string()]);
    }

    #[test]
    fn tool_result_fixture_fails_closed_without_logging_content() {
        let stream = r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"SENTINEL_TOOL_RESULT"}]}}"#;
        start_log();
        let err = interpret_stream(stream, None).unwrap_err();
        let logs = take_log();
        assert!(matches!(
            err,
            ProviderError::ToolUseAttempted { ref event, .. } if event == "tool_result"
        ));
        assert!(!err.to_string().contains("SENTINEL_TOOL_RESULT"));
        assert_eq!(logs, vec!["claude-cli event=tool_result".to_string()]);
    }

    #[test]
    fn out_of_range_utilization_keeps_rejected_status() {
        let stream = r#"
{"type":"rate_limit_event","session_id":"sess","uuid":"abc","rate_limit_info":{"status":"rejected","resetsAt":1700000000,"utilization":1.2,"extra":true},"request_id":"r"}
{"type":"result","result":"PROMPT_ECHO should not leak","usage":{"input_tokens":1,"output_tokens":1}}
"#;
        start_log();
        let err = interpret_stream(stream, None).unwrap_err();
        let logs = take_log();
        match err {
            ProviderError::LimitReached(limit) => {
                assert_eq!(limit.resets_at, Some(1_700_000_000));
                let text = limit.to_string();
                assert!(!text.contains("PROMPT_ECHO"));
                assert!(!text.contains("1.2"));
                assert!(!text.contains("120"));
            }
            other => panic!("expected limit, got {other}"),
        }
        assert!(logs.iter().any(|line| {
            line.contains("status=rejected") && line.contains("utilization_pct=none")
        }));
        assert!(logs.iter().all(|line| {
            !line.contains("PROMPT_ECHO")
                && !line.contains("1.2")
                && !line.contains("120")
                && !line.contains("sess")
                && !line.contains("abc")
        }));
    }

    #[test]
    fn out_of_range_utilization_keeps_allowed_status() {
        let stream = r#"
{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","utilization":1.5}}
{"type":"result","result":"ok","usage":{"input_tokens":1,"output_tokens":1}}
"#;
        start_log();
        let completion = interpret_stream(stream, None).unwrap();
        let logs = take_log();
        assert_eq!(completion.text, "ok");
        assert!(logs.iter().any(|line| {
            line.contains("status=allowed") && line.contains("utilization_pct=none")
        }));
        assert!(logs
            .iter()
            .all(|line| !line.contains("1.5") && !line.contains("150")));
    }

    #[test]
    fn result_text_alone_does_not_invent_a_limit() {
        let stream = r#"
{"type":"result","result":"usage limit reached and not logged in","usage":{"input_tokens":1,"output_tokens":1}}
"#;
        start_log();
        let completion = interpret_stream(stream, None).unwrap();
        let logs = take_log();
        assert_eq!(completion.text, "usage limit reached and not logged in");
        assert!(completion.note.contains("no rate_limit_event"));
        assert!(logs.iter().all(|line| !line.contains("usage limit")));
        assert!(logs.iter().all(|line| !line.contains("not logged in")));
    }

    fn expect_pause(stream: &str) -> LimitReached {
        match interpret_stream(stream, None) {
            Err(ProviderError::LimitReached(limit)) => limit,
            Ok(completion) => panic!("rate event passed silently: {}", completion.text),
            Err(other) => panic!("expected a pause, got {other}"),
        }
    }

    #[test]
    fn an_unreadable_rate_event_pauses_instead_of_passing() {
        let result = r#"{"type":"result","result":"PROMPT_ECHO","usage":{"input_tokens":1,"output_tokens":1}}"#;
        let events = [
            // Unknown status, extra fields.
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"nope","mystery":1}}"#,
            // Status missing.
            r#"{"type":"rate_limit_event","rate_limit_info":{"resetsAt":1700000000}}"#,
            // Status is not a string.
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":3}}"#,
            // No info object and no status on the event.
            r#"{"type":"rate_limit_event"}"#,
            // Info is not an object.
            r#"{"type":"rate_limit_event","rate_limit_info":"rejected"}"#,
            // A truncated line from the CLI.
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allo"#,
        ];
        for event in events {
            start_log();
            let limit = expect_pause(&format!("{event}\n{result}\n"));
            let logs = take_log();
            assert!(limit.message.contains("not understood"), "{event}");
            assert!(!limit.to_string().contains("PROMPT_ECHO"));
            assert!(logs.iter().all(|line| !line.contains("PROMPT_ECHO")));
            assert!(
                logs.iter().any(|line| line.contains("status=unknown")),
                "{event}: {logs:?}"
            );
            // Without a result line it still pauses, not "no result".
            let limit = expect_pause(&format!("{event}\n"));
            assert!(limit.message.contains("not understood"), "{event}");
        }
    }

    #[test]
    fn a_later_allowed_event_does_not_clear_an_unknown_or_rejected_one() {
        let result =
            r#"{"type":"result","result":"ok","usage":{"input_tokens":1,"output_tokens":1}}"#;
        let unknown = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"?"}}"#;
        let rejected = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1700000000}}"#;
        let allowed = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed"}}"#;
        let limit = expect_pause(&format!("{unknown}\n{allowed}\n{result}\n"));
        assert!(limit.message.contains("not understood"));
        let limit = expect_pause(&format!("{rejected}\n{unknown}\n{allowed}\n{result}\n"));
        assert_eq!(limit.message, "claude-cli usage limit reached");
        assert_eq!(limit.resets_at, Some(1_700_000_000));
    }

    #[test]
    fn alternate_rate_event_shapes_are_read() {
        let result =
            r#"{"type":"result","result":"ok","usage":{"input_tokens":1,"output_tokens":1}}"#;
        let rejected = [
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"REJECTED","resets_at":"1700000000"}}"#,
            r#"{"type":"rate_limit_event","rateLimitInfo":{"status":"rejected","resetAt":1700000000.0}}"#,
            r#"{"type":"rate_limit_event","rate_limit":{"status":" Rejected ","reset_at":1700000000}}"#,
            r#"{"type":"rate_limit_event","status":"rejected","resetsAt":1700000000,"rateLimitType":"five_hour"}"#,
            r#"{"type":"system","subtype":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1700000000}}"#,
        ];
        for event in rejected {
            let limit = expect_pause(&format!("{event}\n{result}\n"));
            assert_eq!(limit.message, "claude-cli usage limit reached", "{event}");
            assert_eq!(limit.resets_at, Some(1_700_000_000), "{event}");
        }
        let warning = r#"{"type":"rate_limit_event","rateLimitInfo":{"status":"allowed_warning","utilization":"0.25","isUsingOverage":false,"overageStatus":"rejected"}}"#;
        start_log();
        let completion = interpret_stream(&format!("{warning}\n{result}\n"), None).unwrap();
        let logs = take_log();
        assert_eq!(completion.text, "ok");
        assert!(logs
            .iter()
            .any(|line| line.contains("status=warning") && line.contains("utilization_pct=25")));
    }

    #[test]
    fn bad_reset_and_utilization_values_drop_only_that_field() {
        let result =
            r#"{"type":"result","result":"ok","usage":{"input_tokens":1,"output_tokens":1}}"#;
        for reset in [r#""soon""#, "-5", "0", "null", "{}", "[1]", "1e300", "true"] {
            let event = format!(
                r#"{{"type":"rate_limit_event","rate_limit_info":{{"status":"rejected","resetsAt":{reset},"utilization":"high"}}}}"#
            );
            let limit = expect_pause(&format!("{event}\n{result}\n"));
            assert_eq!(limit.message, "claude-cli usage limit reached", "{reset}");
            assert_eq!(limit.resets_at, None, "{reset}");
        }
        let allowed = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":"x","utilization":-0.1}}"#;
        let completion = interpret_stream(&format!("{allowed}\n{result}\n"), None).unwrap();
        assert_eq!(completion.text, "ok");
    }

    #[test]
    fn an_error_result_is_not_a_draft() {
        let stream = r#"
{"type":"system","subtype":"init","apiKeySource":"none"}
{"type":"result","subtype":"success","is_error":true,"result":"Not logged in SENTINEL_ERROR_TEXT","usage":{"input_tokens":0,"output_tokens":0}}
"#;
        start_log();
        let err = interpret_stream(stream, None).unwrap_err();
        let logs = take_log();
        assert!(
            matches!(err, ProviderError::Failed(ref message) if message == "claude-cli result reported an error"),
            "{err}"
        );
        assert!(logs
            .iter()
            .all(|line| !line.contains("SENTINEL_ERROR_TEXT")));
        let ok_after_error = format!(
            "{}\n{}\n",
            r#"{"type":"result","is_error":true,"result":"x"}"#,
            r#"{"type":"result","is_error":false,"result":"ok"}"#
        );
        assert!(interpret_stream(&ok_after_error, None).is_err());
    }

    #[test]
    fn envelope_fields_do_not_drop_a_real_rate_event() {
        let stream = r#"
{"type":"rate_limit_event","session_id":"sess-1","uuid":"6ba7b810-9dad-11d1-80b4-00c04fd430c8","rate_limit_info":{"status":"rejected","resetsAt":1700000000,"utilization":1.0,"window":"month"},"request_id":"req-9"}
{"type":"result","result":"PROMPT_ECHO should not leak","usage":{"input_tokens":1,"output_tokens":1}}
"#;
        start_log();
        let err = interpret_stream(stream, None).unwrap_err();
        let logs = take_log();
        match err {
            ProviderError::LimitReached(limit) => {
                assert_eq!(limit.resets_at, Some(1_700_000_000));
                assert!(!limit.to_string().contains("PROMPT_ECHO"));
                assert!(!limit.to_string().contains("sess-1"));
            }
            other => panic!("expected limit, got {other}"),
        }
        assert!(logs.iter().all(|line| !line.contains("sess-1")));
        assert!(logs.iter().all(|line| !line.contains("6ba7b810")));
        assert!(logs.iter().any(|line| line.contains("status=rejected")));
    }

    #[test]
    fn version_mismatch_does_not_run_a_prompt() {
        let dir =
            std::env::temp_dir().join(format!("dasdevbot-claude-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let ran = dir.join("ran");
        let bin = write_cli(&dir, &format!("mode=bad_version\nran={}\n", ran.display()));
        let home = dir.join("claude-home");
        private_home(&home);
        let err = ClaudeCli::open(bin.clone(), None, home, &[]).unwrap_err();
        let script = fs::read_to_string(&bin).unwrap_or_default();
        assert!(
            matches!(err, ProviderError::Failed(_)) && err.to_string().contains(CLAUDE_CLI_VERSION),
            "{err}\n{script}"
        );
        assert!(!ran.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    fn recorded_args(path: &Path) -> Vec<String> {
        let raw = fs::read(path).unwrap();
        let mut parts: Vec<String> = raw
            .split(|byte| *byte == 0)
            .map(|chunk| String::from_utf8(chunk.to_vec()).unwrap())
            .collect();
        if parts.last().is_some_and(|item| item.is_empty()) {
            parts.pop();
        }
        parts
    }

    fn set_mtime(path: &Path, secs: i64) {
        let c_path = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        let times = [
            libc::timespec {
                tv_sec: secs,
                tv_nsec: 0,
            },
            libc::timespec {
                tv_sec: secs,
                tv_nsec: 0,
            },
        ];
        let rc = unsafe { libc::utimensat(libc::AT_FDCWD, c_path.as_ptr(), times.as_ptr(), 0) };
        assert_eq!(rc, 0, "{}", std::io::Error::last_os_error());
    }

    fn process_alive(pid: i32) -> bool {
        PathBuf::from(format!("/proc/{pid}")).exists()
    }

    #[test]
    fn disabled_tools_record_the_exact_argv() {
        let dir =
            std::env::temp_dir().join(format!("dasdevbot-claude-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let executed = dir.join("executed");
        let argv_file = dir.join("argv");
        let cwd_file = dir.join("cwd");
        let prompt_copy = dir.join("prompt-copy");
        let mode_file = dir.join("mode");
        let leak = dir.join("memfd-leaked");
        let config = format!(
            "mode=argv\nexecuted={}\nargv={}\ncwd={}\nmode_file={}\nprompt_copy={}\nleak={}\n",
            executed.display(),
            argv_file.display(),
            cwd_file.display(),
            mode_file.display(),
            prompt_copy.display(),
            leak.display(),
        );
        let bin = write_cli(&dir, &config);
        let cli = open_at(&dir, bin);
        let resolved = cli.checked.lock().expect("stamp").files[0].path.clone();
        assert!(resolved.is_absolute());
        assert_eq!(resolved, fs::canonicalize(dir.join("claude")).unwrap());
        start_log();
        let completion = cli
            .complete(
                &CompletionRequest {
                    model: String::new(),
                    system: "PERSONA_SENTINEL".into(),
                    user: "Reply with the single word ok.".into(),
                    max_tokens: 16,
                },
                &mut no_charge,
            )
            .unwrap();
        let logs = take_log();
        assert_eq!(completion.text, "ok");
        assert!(!executed.exists());
        let cwd = PathBuf::from(fs::read_to_string(&cwd_file).unwrap().trim());
        let args = recorded_args(&argv_file);
        let prompt = PathBuf::from(
            args.iter()
                .skip_while(|arg| arg.as_str() != "--system-prompt-file")
                .nth(1)
                .expect("system prompt path"),
        );
        assert!(!prompt.starts_with(&cwd));
        let mode_line = fs::read_to_string(&mode_file).unwrap();
        let mut mode_parts = mode_line.split_whitespace();
        assert_eq!(mode_parts.next(), Some("700"));
        assert_eq!(
            mode_parts.next().and_then(|text| text.parse::<u32>().ok()),
            Some(unsafe { libc::geteuid() })
        );
        assert_eq!(
            fs::read_to_string(&prompt_copy).unwrap(),
            "PERSONA_SENTINEL"
        );
        assert_eq!(args, claude_command_args(None, &prompt));
        let raw = String::from_utf8_lossy(&fs::read(&argv_file).unwrap()).into_owned();
        assert!(!raw.contains("PERSONA_SENTINEL"));
        assert!(!raw.contains("Reply with the single word ok."));
        assert!(logs
            .iter()
            .all(|line| !line.contains("Reply with the single word ok.")));
        assert!(!leak.exists(), "snapshot fd was inherited");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tool_use_kills_the_grandchild_process_group() {
        let dir =
            std::env::temp_dir().join(format!("dasdevbot-claude-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let pidfile = dir.join("grandchild.pid");
        let survived = dir.join("survived");
        let config = format!(
            "mode=grandchild\npidfile={}\nsurvived={}\n",
            pidfile.display(),
            survived.display(),
        );
        let bin = write_cli(&dir, &config);
        let cli = open_at(&dir, bin);
        start_log();
        let started = Instant::now();
        let err = cli
            .complete(
                &CompletionRequest {
                    model: String::new(),
                    system: String::new(),
                    user: "please run a tool".into(),
                    max_tokens: 16,
                },
                &mut no_charge,
            )
            .unwrap_err();
        let elapsed = started.elapsed();
        let logs = take_log();
        assert!(
            elapsed < Duration::from_secs(5),
            "grandchild kill took {elapsed:?}"
        );
        assert!(matches!(
            err,
            ProviderError::ToolUseAttempted { ref event, .. } if event == "tool_use"
        ));
        assert!(!err.to_string().contains("SENTINEL_TOOL_BODY"));
        assert!(logs.iter().all(|line| !line.contains("SENTINEL_TOOL_BODY")));
        let pid: i32 = fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while process_alive(pid) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(!process_alive(pid), "grandchild {pid} survived the kill");
        assert!(!survived.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    fn ok_config(leak: &Path) -> String {
        format!("mode=ok\nleak={}\n", leak.display())
    }

    fn sample_request(user: &str) -> CompletionRequest {
        CompletionRequest {
            model: String::new(),
            system: String::new(),
            user: user.into(),
            max_tokens: 16,
        }
    }

    #[test]
    fn a_changed_binary_fails_closed_without_replacing_the_digest() {
        let dir =
            std::env::temp_dir().join(format!("dasdevbot-claude-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let ran = dir.join("ran");
        let bin = write_cli(&dir, &ok_config(&dir.join("memfd-leaked")));
        let cli = open_at(&dir, bin.clone());
        let digest = cli.checked.lock().expect("stamp").files[0].sha256;
        write_cli(&dir, &format!("mode=bad_version\nran={}\n", ran.display()));
        set_mtime(&bin, 1_700_000_000);
        let err = cli
            .complete(&sample_request("should not run"), &mut no_charge)
            .unwrap_err();
        assert!(
            err.to_string().contains("sha256 changed since startup"),
            "{err}"
        );
        assert!(!ran.exists());
        assert_eq!(cli.checked.lock().expect("stamp").files[0].sha256, digest);
        let err = cli
            .complete(&sample_request("still should not run"), &mut no_charge)
            .unwrap_err();
        assert!(
            err.to_string().contains("sha256 changed since startup"),
            "{err}"
        );
        assert!(!ran.exists());
        assert_eq!(cli.checked.lock().expect("stamp").files[0].sha256, digest);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_mtime_change_with_the_same_bytes_keeps_the_digest() {
        let dir =
            std::env::temp_dir().join(format!("dasdevbot-claude-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let leak = dir.join("memfd-leaked");
        let bin = write_cli(&dir, &ok_config(&leak));
        let cli = open_at(&dir, bin.clone());
        let digest = cli.checked.lock().expect("stamp").files[0].sha256;
        set_mtime(&bin, 1_700_000_000);
        let completion = cli
            .complete(
                &sample_request("Reply with the single word ok."),
                &mut no_charge,
            )
            .unwrap();
        assert_eq!(completion.text, "ok");
        assert!(!leak.exists(), "snapshot fd was inherited");
        assert_eq!(cli.checked.lock().expect("stamp").files[0].sha256, digest);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn configured_sha256_is_checked_at_open() {
        let dir =
            std::env::temp_dir().join(format!("dasdevbot-claude-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let home = dir.join("claude-home");
        private_home(&home);
        let bin = write_cli(&dir, "mode=ok\n");
        let err = ClaudeCli::open(bin.clone(), None, home.clone(), &[[0x11; 32]]).unwrap_err();
        assert!(
            err.to_string().contains("does not match claude_sha256"),
            "{err}"
        );
        let digest = sha256_file(&bin).unwrap();
        let cli = ClaudeCli::open(bin.clone(), None, home.clone(), &[digest]).unwrap();
        assert_eq!(cli.checked.lock().expect("stamp").files[0].sha256, digest);
        let err = ClaudeCli::open(bin, None, home, &[digest, digest]).unwrap_err();
        assert!(
            err.to_string()
                .contains("more digests than the resolved binary"),
            "{err}"
        );
        assert!(parse_sha256_list("abcd").is_err());
        assert!(parse_sha256_list(&hex_digest(&digest)).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    fn hex_digest(bytes: &[u8; 32]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn install_owner_warns_about_parent_directories() {
        let dir =
            std::env::temp_dir().join(format!("dasdevbot-claude-owner-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let bin = write_cli(&dir, "mode=ok\n");
        start_log();
        warn_install_owner(&bin);
        let logs = take_log();
        let parent = dir.display().to_string();
        assert!(
            logs.iter()
                .any(|line| line.contains("not root-owned") && line.contains(&parent)),
            "{logs:?}"
        );
        assert!(
            logs.iter()
                .any(|line| line.contains("group or world writable")),
            "{logs:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn claude_home_refuses_personal_unowned_and_loose_modes() {
        let home = std::env::var("HOME").expect("HOME");
        let err = require_claude_home(PathBuf::from(&home)).unwrap_err();
        assert!(err.to_string().contains("invoking user's home"), "{err}");
        let err = require_claude_home(PathBuf::from("/usr")).unwrap_err();
        assert!(err.to_string().contains("not owned"), "{err}");
        let dir =
            std::env::temp_dir().join(format!("dasdevbot-claude-home-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let err = require_claude_home(dir.clone()).unwrap_err();
        assert!(err.to_string().contains("0700"), "{err}");
        private_home(&dir);
        require_claude_home(dir.clone()).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cwd_ancestors_are_the_service_dir_not_temp() {
        let state = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/claude-state-{}",
            uuid::Uuid::new_v4()
        ));
        private_home(&state);
        let home = state.join("claude-home");
        private_home(&home);
        let planted = std::env::temp_dir().join("CLAUDE.md");
        let created_plant = !planted.exists();
        if created_plant {
            fs::write(&planted, "SENTINEL_ANCESTOR\n").unwrap();
        }
        let found_file = state.join("claude-md-found");
        let cwd_file = state.join("cwd");
        let mode_file = state.join("mode");
        let prompt_copy = state.join("prompt-copy");
        let config = format!(
            "mode=cwd\ncwd={}\nmode_file={}\nprompt_copy={}\nfound={}\n",
            cwd_file.display(),
            mode_file.display(),
            prompt_copy.display(),
            found_file.display(),
        );
        let bin = write_cli(&state, &config);
        let cli = ClaudeCli::open(bin, None, home, &[]).unwrap();
        let completion = cli
            .complete(
                &CompletionRequest {
                    model: String::new(),
                    system: "PERSONA_SENTINEL".into(),
                    user: "Reply with the single word ok.".into(),
                    max_tokens: 16,
                },
                &mut no_charge,
            )
            .unwrap();
        assert_eq!(completion.text, "ok");
        assert!(
            !found_file.exists(),
            "CLAUDE.md in an ancestor was visible from the cwd"
        );
        let cwd = PathBuf::from(fs::read_to_string(&cwd_file).unwrap().trim());
        let state = fs::canonicalize(&state).unwrap();
        let temp = fs::canonicalize(std::env::temp_dir()).unwrap();
        assert!(cwd.starts_with(&state), "{}", cwd.display());
        assert!(cwd.ancestors().all(|dir| dir != temp));
        let mode_line = fs::read_to_string(&mode_file).unwrap();
        let mut mode_parts = mode_line.split_whitespace();
        assert_eq!(mode_parts.next(), Some("700"));
        let euid = unsafe { libc::geteuid() };
        assert_eq!(
            mode_parts.next().and_then(|text| text.parse::<u32>().ok()),
            Some(euid)
        );
        let prompt = fs::read_to_string(&prompt_copy).unwrap();
        assert_eq!(prompt, "PERSONA_SENTINEL");
        let mut cursor = cwd.parent().map(Path::to_path_buf);
        let mut saw_state = false;
        while let Some(dir) = cursor {
            if dir == state {
                saw_state = true;
            }
            if dir.starts_with(&state) {
                let meta = fs::metadata(&dir).unwrap();
                assert_eq!(meta.uid(), euid, "{}", dir.display());
                assert_eq!(meta.mode() & 0o777, 0o700, "{}", dir.display());
            } else {
                break;
            }
            if dir == state {
                break;
            }
            cursor = dir.parent().map(Path::to_path_buf);
        }
        assert!(saw_state);
        if created_plant {
            let _ = fs::remove_file(&planted);
        }
        let _ = fs::remove_dir_all(&state);
    }

    #[test]
    fn a_script_target_is_refused() {
        let dir =
            std::env::temp_dir().join(format!("dasdevbot-claude-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let home = dir.join("claude-home");
        private_home(&home);
        let bin = dir.join("claude");
        fs::write(&bin, "#!/bin/sh\necho 2.1.285\n").unwrap();
        let err = ClaudeCli::open(bin, None, home.clone(), &[]).unwrap_err();
        assert!(err.to_string().contains("native ELF"), "{err}");
        let js = dir.join("cli.js");
        fs::write(&js, "console.log('2.1.285')\n").unwrap();
        let err = ClaudeCli::open(js, None, home, &[]).unwrap_err();
        assert!(err.to_string().contains("native ELF"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn elf_magic_is_checked_on_the_snapshotted_bytes() {
        let dir =
            std::env::temp_dir().join(format!("dasdevbot-claude-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let script = dir.join("claude");
        fs::write(&script, "#!/bin/sh\necho 2.1.285\n").unwrap();
        let err = snapshot_memfd(&script).unwrap_err();
        assert!(err.to_string().contains("native ELF"), "{err}");
        let empty = dir.join("empty");
        fs::write(&empty, b"").unwrap();
        let err = snapshot_memfd(&empty).unwrap_err();
        assert!(err.to_string().contains("native ELF"), "{err}");
        let elf = write_cli(&dir, "mode=ok\n");
        let (mut memfd, _) = snapshot_memfd(&elf).unwrap();
        let mut header = [0u8; 4];
        memfd.read_exact(&mut header).unwrap();
        assert_eq!(header, *b"\x7fELF");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn service_home_exception_requires_a_nologin_shell() {
        assert!(service_home_exception(42, 42, Some("/usr/sbin/nologin")));
        assert!(service_home_exception(42, 42, Some("/bin/false")));
        assert!(service_home_exception(42, 42, Some("nologin")));
        assert!(!service_home_exception(42, 42, Some("/bin/bash")));
        assert!(!service_home_exception(
            42,
            42,
            Some("/usr/sbin/nologin.sh")
        ));
        assert!(!service_home_exception(
            1000,
            1000,
            Some("/usr/sbin/nologin")
        ));
        assert!(!service_home_exception(42, 0, Some("/usr/sbin/nologin")));
        assert!(!service_home_exception(42, 42, None));
        assert!(shell_is_nologin_or_false("/sbin/nologin"));
        assert!(shell_is_nologin_or_false("/usr/bin/false"));
        assert!(!shell_is_nologin_or_false("/bin/sh"));
    }

    fn plant_settings(dir: &Path, markers: &Path, tag: &str) {
        fs::create_dir_all(dir).unwrap();
        let helper = markers.join(format!("{tag}-api-key-helper"));
        let hook = markers.join(format!("{tag}-hook"));
        fs::write(
            dir.join("settings.json"),
            format!(
                r#"{{"apiKeyHelper":"touch {}","hooks":{{"SessionStart":[{{"hooks":[{{"type":"command","command":"touch {}"}}]}}]}}}}"#,
                helper.display(),
                hook.display()
            ),
        )
        .unwrap();
    }

    fn fired(markers: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(markers)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// Plants a hook and an `apiKeyHelper` command in every settings file the fixture
    /// models: the operator's `~/.claude`, `claude_home/.claude` (the CLI's default for
    /// `HOME`), the dedicated `claude_home/claude-config`, and a project root above the
    /// per-call cwd. The fixture runs them with no isolation, and none run through
    /// `ClaudeCli`.
    #[test]
    fn a_hostile_home_hook_does_not_write_a_marker() {
        let dir =
            std::env::temp_dir().join(format!("dasdevbot-claude-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let markers = dir.join("markers");
        fs::create_dir(&markers).unwrap();
        let operator = dir.join("operator-home");
        plant_settings(&operator.join(".claude"), &markers, "operator");
        let home = dir.join("claude-home");
        private_home(&home);
        plant_settings(&home.join(".claude"), &markers, "home-dot-claude");
        let config = home.join(CLAUDE_CONFIG_DIR_NAME);
        private_home(&config);
        plant_settings(&config, &markers, "config-dir");
        let project = home.join("claude-cwd");
        private_home(&project);
        fs::create_dir(project.join(".git")).unwrap();
        plant_settings(&project.join(".claude"), &markers, "project");

        let home_record = dir.join("child-home");
        let bin = write_cli(
            &dir,
            &format!("mode=hostile\nhome_record={}\n", home_record.display()),
        );
        // Control 1: the operator's own environment, no flags.
        let status = Command::new(&bin)
            .current_dir(&project)
            .env_clear()
            .env("HOME", &operator)
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            fired(&markers),
            vec![
                "operator-api-key-helper",
                "operator-hook",
                "project-api-key-helper",
                "project-hook"
            ]
        );
        for name in fired(&markers) {
            fs::remove_file(markers.join(name)).unwrap();
        }
        // Control 2: the child env without the isolation flags still reads the dedicated
        // config dir and the project, so the flags are what stop them.
        let mut control = Command::new(&bin);
        control
            .current_dir(&project)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (key, value) in child_env_from(&home, &[]) {
            control.env(key, value);
        }
        assert!(control.status().unwrap().success());
        assert_eq!(
            fired(&markers),
            vec![
                "config-dir-api-key-helper",
                "config-dir-hook",
                "project-api-key-helper",
                "project-hook"
            ]
        );
        for name in fired(&markers) {
            fs::remove_file(markers.join(name)).unwrap();
        }

        let cli = ClaudeCli::open(bin, None, home.clone(), &[]).unwrap();
        let completion = cli
            .complete(
                &CompletionRequest {
                    model: String::new(),
                    system: String::new(),
                    user: "Reply with the single word ok.".into(),
                    max_tokens: 16,
                },
                &mut no_charge,
            )
            .unwrap();
        assert_eq!(completion.text, "ok");
        assert_eq!(fired(&markers), Vec::<String>::new());
        let recorded = fs::read_to_string(&home_record).unwrap();
        let canonical = fs::canonicalize(&home).unwrap();
        let mut lines = recorded.lines();
        assert_eq!(lines.next(), Some(canonical.display().to_string().as_str()));
        assert_eq!(
            lines.next(),
            Some(
                canonical
                    .join(CLAUDE_CONFIG_DIR_NAME)
                    .display()
                    .to_string()
                    .as_str()
            )
        );
        assert!(!recorded.contains(&operator.display().to_string()));
        let _ = fs::remove_dir_all(&dir);
    }

    /// Local only. CI must not hold a Claude login. This checks `claude --version`
    /// when that binary is already installed and does not run a completion.
    #[test]
    #[ignore = "local-only: requires the pinned Claude Code CLI and must not use a login"]
    fn local_claude_version_matches_pin() {
        let dir =
            std::env::temp_dir().join(format!("dasdevbot-claude-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let home = dir.join("claude-home");
        private_home(&home);
        let cli = ClaudeCli::open(PathBuf::from("claude"), None, home, &[]).unwrap();
        assert!(cli.detail().contains(CLAUDE_CLI_VERSION));
        let _ = fs::remove_dir_all(&dir);
    }

    /// Local only. Runs the real CLI with a hook and an `apiKeyHelper` planted in
    /// `claude_home/.claude`, the dedicated `claude_home/claude-config`, and a project root
    /// above the per-call cwd. CI must not hold a Claude login. A missing login fails the
    /// completion; no marker may appear either way. Run it without network access, for
    /// example under `unshare -rn`, so it cannot make a real call.
    #[test]
    #[ignore = "local-only: requires the pinned Claude Code CLI and must not use the operator home"]
    fn local_claude_does_not_run_a_hostile_home_hook() {
        let dir =
            std::env::temp_dir().join(format!("dasdevbot-claude-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let markers = dir.join("markers");
        fs::create_dir(&markers).unwrap();
        let home = dir.join("claude-home");
        private_home(&home);
        plant_settings(&home.join(".claude"), &markers, "home-dot-claude");
        let config = home.join(CLAUDE_CONFIG_DIR_NAME);
        private_home(&config);
        plant_settings(&config, &markers, "config-dir");
        let project = home.join("claude-cwd");
        private_home(&project);
        fs::create_dir(project.join(".git")).unwrap();
        plant_settings(&project.join(".claude"), &markers, "project");
        let cli = ClaudeCli::open(PathBuf::from("claude"), None, home, &[]).unwrap();
        let _ = cli.complete(
            &CompletionRequest {
                model: String::new(),
                system: String::new(),
                user: "Reply with the single word ok.".into(),
                max_tokens: 16,
            },
            &mut no_charge,
        );
        assert_eq!(fired(&markers), Vec::<String>::new());
        let _ = fs::remove_dir_all(&dir);
    }
}
