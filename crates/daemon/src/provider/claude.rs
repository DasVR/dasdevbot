//! Claude Pro through the official Claude Code CLI.
//!
//! The supported version is pinned. Tools are disabled. The prompt is stdin.
//! `--dangerously-skip-permissions` is never passed, and `claude setup-token`
//! output is never read. Stream events other than `rate_limit_event` and
//! `result` are dropped and the raw stream is never logged.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{
    log_provider, Completion, CompletionRequest, Headroom, LimitReached, ProviderError,
    QuotaSignal, UsageReport,
};

pub const CLAUDE_CLI_VERSION: &str = "2.1.285";

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

#[derive(Debug)]
pub struct ClaudeCli {
    bin: PathBuf,
    model: Option<String>,
}

impl ClaudeCli {
    pub fn open(bin: PathBuf, model: Option<String>) -> Result<Self, ProviderError> {
        ensure_version(&bin)?;
        Ok(Self { bin, model })
    }

    pub fn complete(&self, req: &CompletionRequest) -> Result<Completion, ProviderError> {
        let args = claude_command_args(self.model.as_deref(), &req.system);
        refuse_forbidden_args(&args)?;
        let stdout = run_cli(&self.bin, &args, &req.user)?;
        interpret_stream(&stdout, self.model.as_deref())
    }

    pub fn detail(&self) -> String {
        match &self.model {
            Some(model) => format!("claude-cli {CLAUDE_CLI_VERSION} model {model}"),
            None => format!("claude-cli {CLAUDE_CLI_VERSION}"),
        }
    }
}

pub fn claude_command_args(model: Option<&str>, system: &str) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--tools".to_string(),
        String::new(),
        "--strict-mcp-config".to_string(),
        "--disable-slash-commands".to_string(),
        "--no-session-persistence".to_string(),
    ];
    if !system.is_empty() {
        args.push("--system-prompt".to_string());
        args.push(system.to_string());
    }
    if let Some(model) = model.filter(|model| !model.is_empty()) {
        args.push("--model".to_string());
        args.push(model.to_string());
    }
    args
}

#[cfg(test)]
pub fn child_env_from(vars: &[(&str, &str)]) -> Vec<(String, String)> {
    const KEEP: &[&str] = &["HOME", "PATH", "USER", "LANG", "TMPDIR"];
    vars.iter()
        .filter(|(key, _)| KEEP.contains(key))
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect()
}

fn child_env() -> Vec<(String, String)> {
    ["HOME", "PATH", "USER", "LANG", "TMPDIR"]
        .into_iter()
        .filter_map(|key| {
            std::env::var(key)
                .ok()
                .map(|value| (key.to_string(), value))
        })
        .collect()
}

fn refuse_forbidden_args(args: &[String]) -> Result<(), ProviderError> {
    for arg in args {
        if arg == "--dangerously-skip-permissions" || arg.contains("setup-token") {
            return Err(ProviderError::Failed(
                "refusing to pass --dangerously-skip-permissions or a setup-token to claude".into(),
            ));
        }
    }
    Ok(())
}

fn ensure_version(bin: &Path) -> Result<(), ProviderError> {
    let output = Command::new(bin).arg("--version").output().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            ProviderError::Unavailable(format!("claude CLI not found at {}", bin.display()))
        } else {
            ProviderError::Failed(format!("claude --version failed to start ({})", err.kind()))
        }
    })?;
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

fn run_cli(bin: &Path, args: &[String], prompt: &str) -> Result<String, ProviderError> {
    let dir = fresh_workdir()?;
    let _cleanup = DirGuard(dir.clone());
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .current_dir(&dir)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
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
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(err) = stdin.write_all(prompt.as_bytes()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ProviderError::Failed(format!(
                "claude CLI stdin failed ({})",
                err.kind()
            )));
        }
    }
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
    let collected = match read_stdout_lines(&mut child, &rx) {
        Ok(text) => text,
        Err(err) => {
            let _ = out_handle.join();
            let _ = err_handle.join();
            return Err(err);
        }
    };
    let _ = out_handle.join();
    let _ = err_handle.join();
    let _ = child.wait();
    Ok(collected)
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
                if line_has_tool_use(&line) {
                    log_provider("claude-cli event=tool_use");
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ProviderError::ToolUseAttempted);
                }
                collected.push_str(&line);
                collected.push('\n');
            }
            Ok(Err(err)) => {
                return Err(ProviderError::Failed(format!(
                    "claude CLI stdout failed ({})",
                    err.kind()
                )));
            }
            Err(RecvTimeoutError::Timeout) => {
                if started.elapsed() >= limit {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ProviderError::Failed("claude CLI timed out".into()));
                }
            }
            Err(RecvTimeoutError::Disconnected) => match child.try_wait() {
                Ok(Some(_)) => return Ok(collected),
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
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

pub fn line_has_tool_use(line: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
        return false;
    };
    value_has_tool_use(&value)
}

fn value_has_tool_use(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            if map.get("type").and_then(|item| item.as_str()) == Some("tool_use") {
                return true;
            }
            map.values().any(value_has_tool_use)
        }
        Value::Array(items) => items.iter().any(value_has_tool_use),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}

fn stream_has_tool_use(stream: &str) -> bool {
    stream.lines().any(line_has_tool_use)
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
    if stream_has_tool_use(stream) {
        log_provider("claude-cli event=tool_use");
        return Err(ProviderError::ToolUseAttempted);
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
    if is_limit_text(&parsed.text) {
        return Err(ProviderError::LimitReached(LimitReached {
            message: "claude-cli usage limit reached".into(),
            resets_at: parsed.rate.as_ref().and_then(|rate| rate.resets_at),
        }));
    }
    if is_logged_out(&parsed.text) {
        return Err(ProviderError::Unavailable(
            "claude-cli is not logged in".into(),
        ));
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
    let info = value.get("rate_limit_info")?.as_object()?;
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
            if !fraction.is_finite() || fraction < 0.0 {
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

fn is_limit_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("usage limit reached")
        || lower.contains("rate limit")
        || lower.contains("you've hit your")
        || lower.contains("context limit reached")
        || lower.contains("too many requests")
}

fn is_logged_out(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("not logged in")
        || lower.contains("not authenticated")
        || lower.contains("/login")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{start_log, take_log};
    use std::os::unix::fs::PermissionsExt;

    fn write_cli(dir: &Path, body: &str) -> PathBuf {
        let bin = dir.join("claude");
        fs::write(&bin, body).unwrap();
        let mut perms = fs::metadata(&bin).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bin, perms).unwrap();
        bin
    }

    #[test]
    fn command_disables_tools_and_keeps_the_prompt_off_argv() {
        let args = claude_command_args(Some("opus"), "be brief");
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--output-format", "stream-json"]));
        assert!(args.iter().any(|arg| arg == "--verbose"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--tools" && pair[1].is_empty()));
        assert!(!args.iter().any(|arg| arg == "json"));
        assert!(!args
            .iter()
            .any(|arg| arg == "--dangerously-skip-permissions"));
        assert!(args.iter().all(|arg| !arg.contains("setup-token")));
        assert!(args
            .iter()
            .all(|arg| arg != "Reply with the single word ok."));
        let env = child_env_from(&[
            ("HOME", "/home/dev"),
            ("PATH", "/usr/bin"),
            ("USER", "dev"),
            ("LANG", "C"),
            ("TMPDIR", "/tmp"),
            ("OLLAMA_API_KEY", "SENTINEL_KEY"),
            ("CLAUDE_CODE_OAUTH_TOKEN", "SENTINEL_OAUTH"),
            ("ANTHROPIC_API_KEY", "SENTINEL_ANTHROPIC"),
            ("DASDEVBOT_TOKEN", "SENTINEL_TOKEN"),
        ]);
        assert!(env.iter().all(|(key, _)| {
            !matches!(
                key.as_str(),
                "OLLAMA_API_KEY"
                    | "CLAUDE_CODE_OAUTH_TOKEN"
                    | "ANTHROPIC_API_KEY"
                    | "DASDEVBOT_TOKEN"
            )
        }));
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
{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1700000000,"utilization":1.2}}
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
        assert!(matches!(err, ProviderError::ToolUseAttempted));
        assert!(!err.to_string().contains("SENTINEL_TOOL_BODY"));
        assert_eq!(logs, vec!["claude-cli event=tool_use".to_string()]);
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

    #[test]
    fn disabled_tools_do_not_execute_and_a_tool_use_kills_the_process() {
        let dir =
            std::env::temp_dir().join(format!("dasdevbot-claude-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let executed = dir.join("executed");
        let survived = dir.join("survived");
        let script = format!(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "2.1.285 (Claude Code)"
  exit 0
fi
prev=""
tools_ok=0
bad=0
for arg in "$@"; do
  if [ "$prev" = "--tools" ]; then
    if [ -z "$arg" ]; then tools_ok=1; else bad=1; fi
  fi
  if [ "$arg" = "--dangerously-skip-permissions" ]; then bad=1; fi
  case "$arg" in *setup-token*) bad=1 ;; esac
  prev="$arg"
done
if [ "$bad" -eq 1 ] || [ "$tools_ok" -eq 0 ]; then
  touch "{executed}"
  exit 2
fi
printf '%s\n' '{{"type":"rate_limit_event","rate_limit_info":{{"status":"allowed","resetsAt":1700000000,"utilization":0.1}}}}'
printf '%s\n' '{{"type":"result","result":"ok","usage":{{"input_tokens":2,"output_tokens":1}}}}'
"#,
            executed = executed.display()
        );
        let bin = write_cli(&dir, &script);
        let cli = ClaudeCli::open(bin, None).unwrap();
        start_log();
        let completion = cli
            .complete(&CompletionRequest {
                model: String::new(),
                system: String::new(),
                user: "Reply with the single word ok.".into(),
                max_tokens: 16,
            })
            .unwrap();
        let logs = take_log();
        assert_eq!(completion.text, "ok");
        assert!(!executed.exists());
        assert!(logs
            .iter()
            .all(|line| !line.contains("Reply with the single word ok.")));

        let killer = format!(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "2.1.285 (Claude Code)"
  exit 0
fi
printf '%s\n' '{{"type":"assistant","message":{{"content":[{{"type":"tool_use","name":"Bash","input":{{"command":"SENTINEL_TOOL_BODY"}}}}]}}}}'
sleep 30
touch "{survived}"
"#,
            survived = survived.display()
        );
        let bin = write_cli(&dir, &killer);
        let cli = ClaudeCli::open(bin, None).unwrap();
        start_log();
        let err = cli
            .complete(&CompletionRequest {
                model: String::new(),
                system: String::new(),
                user: "please run a tool".into(),
                max_tokens: 16,
            })
            .unwrap_err();
        let logs = take_log();
        assert!(matches!(err, ProviderError::ToolUseAttempted));
        assert!(!err.to_string().contains("SENTINEL_TOOL_BODY"));
        assert!(logs.iter().all(|line| !line.contains("SENTINEL_TOOL_BODY")));
        assert!(!survived.exists());
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
