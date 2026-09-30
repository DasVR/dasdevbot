//! Claude Pro through the official Claude Code CLI.
//!
//! The supported version is pinned. Tools are disabled. The prompt is stdin.
//! `--dangerously-skip-permissions` is never passed, and `claude setup-token`
//! output is never read. Stream events other than `rate_limit_event` and
//! `result` are dropped and the raw stream is never logged.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

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
}

impl RateStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Warning => "warning",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VersionStamp {
    mtime: Option<SystemTime>,
}

#[derive(Debug)]
pub struct ClaudeCli {
    bin: PathBuf,
    model: Option<String>,
    checked: Mutex<Option<VersionStamp>>,
}

impl ClaudeCli {
    pub fn open(bin: PathBuf, model: Option<String>) -> Result<Self, ProviderError> {
        let bin = resolve_executable(bin)?;
        ensure_version(&bin)?;
        Ok(Self {
            checked: Mutex::new(Some(version_stamp(&bin))),
            bin,
            model,
        })
    }

    pub fn complete(
        &self,
        req: &CompletionRequest,
        _charge: &mut dyn FnMut(&super::RetryCost) -> Result<(), ProviderError>,
    ) -> Result<Completion, ProviderError> {
        self.ensure_current()?;
        let dir = fresh_workdir()?;
        let _cleanup = DirGuard(dir.clone());
        let prompt_file = write_system_prompt(&dir, &req.system)?;
        let args = claude_command_args(self.model.as_deref(), &prompt_file);
        refuse_forbidden_args(&args)?;
        let stdout = run_cli(&self.bin, &args, &req.user, &dir)?;
        interpret_stream(&stdout, self.model.as_deref())
    }

    fn ensure_current(&self) -> Result<(), ProviderError> {
        let stamp = version_stamp(&self.bin);
        let mut cached = self.checked.lock().expect("claude version");
        if stamp.mtime.is_some() && cached.as_ref() == Some(&stamp) {
            return Ok(());
        }
        ensure_version(&self.bin)?;
        *cached = Some(stamp);
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
        "--system-prompt-file".to_string(),
        system_prompt_file.display().to_string(),
    ];
    if let Some(model) = model.filter(|model| !model.is_empty()) {
        args.push("--model".to_string());
        args.push(model.to_string());
    }
    args
}

pub fn child_env_from(vars: &[(&str, &str)]) -> Vec<(String, String)> {
    const KEEP: &[&str] = &["HOME", "USER", "LANG", "TMPDIR"];
    let mut out: Vec<(String, String)> = vars
        .iter()
        .filter(|(key, _)| KEEP.contains(key))
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect();
    out.push(("PATH".to_string(), CLAUDE_CHILD_PATH.to_string()));
    out.push(("DISABLE_AUTOUPDATER".to_string(), "1".to_string()));
    out.push(("DISABLE_UPDATES".to_string(), "1".to_string()));
    out
}

fn child_env() -> Vec<(String, String)> {
    let vars: Vec<(String, String)> = ["HOME", "USER", "LANG", "TMPDIR"]
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
    child_env_from(&borrowed)
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

fn version_stamp(bin: &Path) -> VersionStamp {
    let mtime = fs::metadata(bin).ok().and_then(|meta| meta.modified().ok());
    VersionStamp { mtime }
}

fn version_output(bin: &Path) -> Result<Output, ProviderError> {
    let mut attempt = 0;
    loop {
        match Command::new(bin).arg("--version").output() {
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

fn ensure_version(bin: &Path) -> Result<(), ProviderError> {
    let output = version_output(bin)?;
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

fn write_system_prompt(dir: &Path, system: &str) -> Result<PathBuf, ProviderError> {
    let path = dir.join(SYSTEM_PROMPT_NAME);
    fs::write(&path, system).map_err(|err| {
        ProviderError::Failed(format!(
            "could not write the claude system prompt ({})",
            err.kind()
        ))
    })?;
    Ok(path)
}

fn run_cli(bin: &Path, args: &[String], prompt: &str, dir: &Path) -> Result<String, ProviderError> {
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
    for (key, value) in child_env() {
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
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(err) = stdin.write_all(prompt.as_bytes()) {
            kill_group(&mut child);
            drop(out_handle);
            drop(err_handle);
            return Err(ProviderError::Failed(format!(
                "claude CLI stdin failed ({})",
                err.kind()
            )));
        }
    }
    let collected = match read_stdout_lines(&mut child, &rx) {
        Ok(text) => text,
        Err(err) => {
            drop(out_handle);
            drop(err_handle);
            return Err(err);
        }
    };
    let _ = out_handle.join();
    let _ = err_handle.join();
    let _ = child.wait();
    Ok(collected)
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

fn fresh_workdir() -> Result<PathBuf, ProviderError> {
    let dir = std::env::temp_dir().join(format!("dasdevbot-claude-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&dir).map_err(|err| {
        ProviderError::Failed(format!(
            "could not create a claude working directory ({})",
            err.kind()
        ))
    })?;
    Ok(dir)
}

struct DirGuard(PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
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
        if rate.status == RateStatus::Rejected {
            return Err(ProviderError::LimitReached(LimitReached {
                message: "claude-cli usage limit reached".into(),
                resets_at: rate.resets_at,
            }));
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
    for line in stream.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(kind) = value.get("type").and_then(|item| item.as_str()) else {
            continue;
        };
        match kind {
            "rate_limit_event" => {
                if let Some(observed) = parse_rate_event(&value) {
                    rate = Some(prefer_rate(rate, observed));
                }
            }
            "result" => {
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
            _ => {}
        }
    }
    let Some(mut parsed) = result else {
        if let Some(rate) = rate.filter(|rate| rate.status == RateStatus::Rejected) {
            return Err(ProviderError::LimitReached(LimitReached {
                message: "claude-cli usage limit reached".into(),
                resets_at: rate.resets_at,
            }));
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
        RateStatus::Rejected => 2,
    }
}

fn parse_rate_event(value: &Value) -> Option<RateObservation> {
    let obj = value.as_object()?;
    if obj.len() != 2 || obj.get("type").and_then(|item| item.as_str()) != Some("rate_limit_event")
    {
        return None;
    }
    let info = obj.get("rate_limit_info")?.as_object()?;
    if info.is_empty()
        || info
            .keys()
            .any(|key| !matches!(key.as_str(), "status" | "resetsAt" | "utilization"))
    {
        return None;
    }
    let status = match info.get("status").and_then(|item| item.as_str())? {
        "allowed" => RateStatus::Allowed,
        "allowed_warning" | "warning" => RateStatus::Warning,
        "rejected" => RateStatus::Rejected,
        _ => return None,
    };
    let resets_at = match info.get("resetsAt") {
        None => None,
        Some(item) => Some(item.as_i64()?),
    };
    let utilization_pct = match info.get("utilization") {
        None => None,
        Some(item) => {
            let fraction = item.as_f64()?;
            if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
                return None;
            }
            Some(fraction * 100.0)
        }
    };
    Some(RateObservation {
        status,
        resets_at,
        utilization_pct,
    })
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
    use std::time::{Duration, Instant};

    fn write_cli(dir: &Path, body: &str) -> PathBuf {
        let bin = dir.join("claude");
        fs::write(&bin, body).unwrap();
        let mut perms = fs::metadata(&bin).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bin, perms).unwrap();
        bin
    }

    fn no_charge(_: &crate::provider::RetryCost) -> Result<(), ProviderError> {
        Ok(())
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
        let env = child_env_from(&[
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
        ]);
        assert_eq!(
            env.iter()
                .find(|(key, _)| key == "PATH")
                .map(|(_, value)| value.as_str()),
            Some(CLAUDE_CHILD_PATH)
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
                    | "CLAUDE_CONFIG_DIR"
            )
        }));
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
    fn lax_rate_events_and_result_text_do_not_invent_a_limit() {
        let stream = r#"
{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1700000000,"utilization":1.2},"extra":true}
{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","utilization":1.2}}
{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","mystery":1}}
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

    #[test]
    fn version_mismatch_does_not_run_a_prompt() {
        let dir =
            std::env::temp_dir().join(format!("dasdevbot-claude-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let ran = dir.join("ran");
        let bin = write_cli(
            &dir,
            &format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"9.9.9\"; exit 0; fi\ntouch '{}'\n",
                ran.display()
            ),
        );
        let err = ClaudeCli::open(bin.clone(), None).unwrap_err();
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
        let script = format!(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "2.1.285 (Claude Code)"
  exit 0
fi
cat >/dev/null
if [ "$DISABLE_AUTOUPDATER" != "1" ] || [ "$DISABLE_UPDATES" != "1" ] || [ "$PATH" != "/usr/bin:/bin" ]; then
  touch "{executed}"
  exit 2
fi
if [ -n "$OLLAMA_API_KEY" ] || [ -n "$ANTHROPIC_API_KEY" ] || [ -n "$DASDEVBOT_TOKEN" ] || [ -n "$CLAUDE_CODE_OAUTH_TOKEN" ]; then
  touch "{executed}"
  exit 2
fi
printf '%s\0' "$@" > "{argv}"
pwd > "{cwd}"
cp system-prompt.txt "{prompt}"
printf '%s\n' '{{"type":"rate_limit_event","rate_limit_info":{{"status":"allowed","resetsAt":1700000000,"utilization":0.1}}}}'
printf '%s\n' '{{"type":"result","result":"ok","usage":{{"input_tokens":2,"output_tokens":1}}}}'
"#,
            executed = executed.display(),
            argv = argv_file.display(),
            cwd = cwd_file.display(),
            prompt = prompt_copy.display(),
        );
        let bin = write_cli(&dir, &script);
        let cli = ClaudeCli::open(bin, None).unwrap();
        assert!(cli.bin.is_absolute());
        assert_eq!(cli.bin, fs::canonicalize(dir.join("claude")).unwrap());
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
        let prompt = cwd.join(SYSTEM_PROMPT_NAME);
        assert_eq!(
            fs::read_to_string(&prompt_copy).unwrap(),
            "PERSONA_SENTINEL"
        );
        assert_eq!(
            recorded_args(&argv_file),
            claude_command_args(None, &prompt)
        );
        let raw = String::from_utf8_lossy(&fs::read(&argv_file).unwrap()).into_owned();
        assert!(!raw.contains("PERSONA_SENTINEL"));
        assert!(!raw.contains("Reply with the single word ok."));
        assert!(logs
            .iter()
            .all(|line| !line.contains("Reply with the single word ok.")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tool_use_kills_the_grandchild_process_group() {
        let dir =
            std::env::temp_dir().join(format!("dasdevbot-claude-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let pidfile = dir.join("grandchild.pid");
        let survived = dir.join("survived");
        let script = format!(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "2.1.285 (Claude Code)"
  exit 0
fi
sh -c 'echo $$ > "{pidfile}"; sleep 30; touch "{survived}"' &
i=0
while [ ! -s "{pidfile}" ]; do
  i=$((i + 1))
  if [ "$i" -gt 100 ]; then
    exit 3
  fi
  sleep 0.05
done
printf '%s\n' '{{"type":"assistant","message":{{"content":[{{"type":"tool_use","name":"Bash","input":{{"command":"SENTINEL_TOOL_BODY"}}}}]}}}}'
wait
"#,
            pidfile = pidfile.display(),
            survived = survived.display(),
        );
        let bin = write_cli(&dir, &script);
        let cli = ClaudeCli::open(bin, None).unwrap();
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

    #[test]
    fn version_is_rechecked_when_the_binary_changes() {
        let dir =
            std::env::temp_dir().join(format!("dasdevbot-claude-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let ran = dir.join("ran");
        let bin = write_cli(
            &dir,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"2.1.285 (Claude Code)\"; exit 0; fi\nexit 0\n",
        );
        let cli = ClaudeCli::open(bin.clone(), None).unwrap();
        write_cli(
            &dir,
            &format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"9.9.9\"; exit 0; fi\ntouch '{}'\n",
                ran.display()
            ),
        );
        set_mtime(&bin, 1_700_000_000);
        let err = cli
            .complete(
                &CompletionRequest {
                    model: String::new(),
                    system: String::new(),
                    user: "should not run".into(),
                    max_tokens: 16,
                },
                &mut no_charge,
            )
            .unwrap_err();
        assert!(
            matches!(err, ProviderError::Failed(_)) && err.to_string().contains(CLAUDE_CLI_VERSION),
            "{err}"
        );
        assert!(!ran.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    /// Local only. CI must not hold a Claude login. This checks `claude --version`
    /// when that binary is already installed and does not run a completion.
    #[test]
    #[ignore = "local-only: requires the pinned Claude Code CLI and must not use a login"]
    fn local_claude_version_matches_pin() {
        let cli = ClaudeCli::open(PathBuf::from("claude"), None).unwrap();
        assert!(cli.detail().contains(CLAUDE_CLI_VERSION));
    }
}
