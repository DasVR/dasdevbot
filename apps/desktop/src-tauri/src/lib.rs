//! Card-window decisions and settings-window secret entry. The session bearer
//! never reaches the webview (#36 H1): the shell reads it and attaches it to
//! its own daemon calls (daemon_http, the shell socket).
//! The Tauri window label only selects which per-launch secret to send.
//! The daemon derives the window from that secret. The shell does not open
//! SQLite. Every command is a line on the daemon's local socket.
//! See docs/shell-ipc.md.
//!
//! The shell forms (full, companion, pill) live in `shell_form` and
//! `geometry`. Approval cards are decided only in the `card` window: the main
//! window can ask to show it (`open_card_window`) but holds no decision
//! capability.

mod daemon_child;
mod daemon_http;
mod geometry;
mod native_sight;
mod shell_form;
mod snap;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

struct ShellState {
    data: PathBuf,
}

#[tauri::command]
async fn sign_decision(
    window: tauri::Window,
    state: tauri::State<'_, ShellState>,
    id: String,
    decision: String,
    reason: Option<String>,
) -> Result<(), String> {
    let label = window.label().to_string();
    let data = state.data.clone();
    daemon_http::off_main(move || {
        finish_signed(
            &data,
            &label,
            json!({
                "op": "decide",
                "approval_id": id,
                "decision": decision,
                "reason": reason,
            }),
        )
    })
    .await
}

#[tauri::command]
async fn undo_decision(
    window: tauri::Window,
    state: tauri::State<'_, ShellState>,
    id: String,
) -> Result<(), String> {
    let label = window.label().to_string();
    let data = state.data.clone();
    daemon_http::off_main(move || {
        finish_signed(
            &data,
            &label,
            json!({
                "op": "undo",
                "approval_id": id,
            }),
        )
    })
    .await
}

#[tauri::command]
async fn set_secret(
    window: tauri::Window,
    state: tauri::State<'_, ShellState>,
    handle: String,
    value: String,
) -> Result<SecretStored, String> {
    let label = window.label().to_string();
    let data = state.data.clone();
    let response = daemon_http::off_main(move || {
        signed_body(
            &data,
            &label,
            json!({
                "op": "secret",
                "name": handle,
                "value": value,
            }),
        )
    })
    .await?;
    let last4 = response["result"]["last4"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    Ok(SecretStored { last4 })
}

#[derive(serde::Serialize)]
struct SecretStored {
    last4: String,
}

fn window_secret(data: &Path, label: &str) -> Result<String, String> {
    let text = std::fs::read_to_string(dasdevbotd::window_secret_path(data, label))
        .map_err(|err| err.to_string())?;
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("window secret is missing".into());
    }
    Ok(text)
}

fn signed_body(data: &Path, label: &str, mut body: Value) -> Result<Value, String> {
    body["window_secret"] = Value::String(window_secret(data, label)?);
    transact(data, body)
}

fn finish_signed(data: &Path, label: &str, body: Value) -> Result<(), String> {
    match signed_body(data, label, body.clone()) {
        Ok(_) => Ok(()),
        Err(err) if err.contains("requires a Windows Hello signature") => {
            complete_hello(data, label, &body)
        }
        Err(err) => Err(err),
    }
}

#[cfg(windows)]
fn complete_hello(data: &Path, label: &str, body: &Value) -> Result<(), String> {
    let approval_id = body["approval_id"].as_str().unwrap_or("");
    let decision = body
        .get("decision")
        .and_then(|value| value.as_str())
        .unwrap_or("approve");
    let reason = body
        .get("reason")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let purpose = if body["op"] == "undo" {
        dasdevbotd::UNDO_PURPOSE
    } else {
        dasdevbotd::DECISION_PURPOSE
    };
    let prepared = signed_body(
        data,
        label,
        json!({
            "op": "prepare",
            "approval_id": approval_id,
            "decision": decision,
            "reason": reason,
            "purpose": purpose,
        }),
    )?;
    let result = &prepared["result"];
    if result["hello_enrolled"].as_bool() != Some(true) {
        signed_body(data, label, json!({"op": "hello-enroll"}))?;
    }
    let message = result["message"].as_str().unwrap_or("");
    let prompt = result["prompt"].as_str().unwrap_or("");
    let nonce = result["nonce"].as_str().unwrap_or("").to_string();
    let signature = dasdevbotd::sign_approval_message(message, prompt)?;
    let mut again = body.clone();
    again["client_signature"] = Value::String(signature);
    again["client_nonce"] = Value::String(nonce);
    signed_body(data, label, again)?;
    Ok(())
}

#[cfg(not(windows))]
fn complete_hello(_data: &Path, _label: &str, _body: &Value) -> Result<(), String> {
    Err("external tier is denied without Windows Hello".into())
}

fn daemon_data() -> PathBuf {
    if let Ok(path) = std::env::var("DASDEVBOT_DATA") {
        return PathBuf::from(path);
    }
    #[cfg(feature = "demo-daemon")]
    if let Some(path) = demo::data_path() {
        return path;
    }
    PathBuf::from("data/dasdevbot.sqlite")
}

/// Every shell-socket line (decide, undo, prepare, hello-enroll, secret)
/// goes through here.
fn transact(data: &Path, body: Value) -> Result<Value, String> {
    transact_gated(data, body, &daemon_child::ready, &exchange)
}

/// #54 / #36 (M2 class): a failed or exited bundled daemon fails closed. The
/// gate runs before the bearer is read and before the pipe or socket is
/// opened, so nothing is written to whatever else holds the pipe name. The
/// server PID/SID check stays on #54.
fn transact_gated(
    data: &Path,
    mut body: Value,
    ready: &dyn Fn() -> Result<(), String>,
    send: &dyn Fn(&Path, &Value) -> Result<Value, String>,
) -> Result<Value, String> {
    ready()?;
    let token = std::fs::read_to_string(dasdevbotd::session_token_path(data))
        .map_err(|err| err.to_string())?;
    body["token"] = Value::String(token.trim().to_string());
    send(data, &body)
}

fn exchange(data: &Path, body: &Value) -> Result<Value, String> {
    #[cfg(unix)]
    {
        let stream = std::os::unix::net::UnixStream::connect(dasdevbotd::shell_socket_path(data))
            .map_err(|err| err.to_string())?;
        write_and_read(stream, body)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // SECURITY_IDENTIFICATION: a pipe server may identify us, not impersonate us.
        const SECURITY_IDENTIFICATION: u32 = 0x0001_0000;
        let pipe = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .security_qos_flags(SECURITY_IDENTIFICATION)
            .open(dasdevbotd::shell_pipe_name(data))
            .map_err(|err| err.to_string())?;
        write_and_read(pipe, body)
    }
}

fn write_and_read(mut stream: impl std::io::Read + Write, body: &Value) -> Result<Value, String> {
    serde_json::to_writer(&mut stream, body).map_err(|err| err.to_string())?;
    stream.write_all(b"\n").map_err(|err| err.to_string())?;
    stream.flush().map_err(|err| err.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|err| err.to_string())?;
    let parsed: Value = serde_json::from_str(&line).map_err(|err| err.to_string())?;
    if parsed["ok"].as_bool() == Some(true) {
        Ok(parsed)
    } else {
        Err(parsed["error"]
            .as_str()
            .unwrap_or("daemon refused the request")
            .to_string())
    }
}

fn open_shell() -> ShellState {
    ShellState {
        data: daemon_data(),
    }
}

/// The session bearer, for the shell's own daemon calls only. No command
/// returns it and no script puts it in a page (#36 H1).
pub(crate) fn read_session_token() -> Result<String, String> {
    if let Ok(token) = std::env::var("DASDEVBOT_TOKEN") {
        let token = token.trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }
    let path = token_file();
    let token = std::fs::read_to_string(&path)
        .map_err(|_| format!("session token file is missing ({})", path.display()))?;
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err("session token file is empty".into());
    }
    Ok(token)
}

fn token_file() -> PathBuf {
    if let Ok(path) = std::env::var("DASDEVBOT_TOKEN_FILE") {
        return PathBuf::from(path);
    }
    dasdevbotd::session_token_path(&daemon_data())
}

/// The label of the only window that may sign or undo a decision.
const CARD_WINDOW: &str = "card";

/// Show and focus the card window. Showing it decides nothing: the card window
/// still needs the evidence on screen, the hold, and its own IPC capability.
#[tauri::command]
fn open_card_window(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    let card = app
        .get_webview_window(CARD_WINDOW)
        .ok_or_else(|| "card window is missing".to_string())?;
    card.show().map_err(|err| err.to_string())?;
    card.unminimize().map_err(|err| err.to_string())?;
    card.set_focus().map_err(|err| err.to_string())
}

/// Forwards sight changes to the page, and the card window hides instead of
/// closing, so it can be shown again.
fn keep_card_window(window: &tauri::Window, event: &tauri::WindowEvent) {
    native_sight::forward(window, event);
    if window.label() != CARD_WINDOW {
        return;
    }
    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        let _ = window.hide();
    }
}

/// The demo installer's daemon. Built only with `--features demo-daemon`.
///
/// Studio Director's demo ruling: the sidecar is `dasdevbotd` built with
/// `--no-default-features` (no iroh UDP bind), started as
/// `serve --role executor --provider mock` on 127.0.0.1:8787. No
/// `--allow-remote`, no `--web`, no `--dev-env-secrets`, and no secret is
/// bundled. It runs only while the app runs: no service and no startup entry.
#[cfg(feature = "demo-daemon")]
mod demo {
    use std::path::PathBuf;
    use std::process::{Child, Command};

    /// `%LOCALAPPDATA%\net.dasdev.dasdevbot\dasdevbot.sqlite`, which the
    /// uninstaller's opt-in "Also delete my data" box removes.
    pub fn data_path() -> Option<PathBuf> {
        let base = std::env::var_os("LOCALAPPDATA")?;
        Some(
            PathBuf::from(base)
                .join("net.dasdev.dasdevbot")
                .join("dasdevbot.sqlite"),
        )
    }

    pub fn args(data: &std::path::Path) -> Vec<String> {
        vec![
            "serve".into(),
            "--role".into(),
            "executor".into(),
            "--provider".into(),
            "mock".into(),
            "--bind".into(),
            "127.0.0.1:8787".into(),
            "--data".into(),
            data.display().to_string(),
        ]
    }

    pub fn start(data: &std::path::Path) -> Result<Child, String> {
        let exe = std::env::current_exe().map_err(|err| err.to_string())?;
        let dir = exe
            .parent()
            .ok_or_else(|| "app directory is missing".to_string())?;
        let name = if cfg!(windows) {
            "dasdevbotd.exe"
        } else {
            "dasdevbotd"
        };
        if let Some(parent) = data.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let mut command = Command::new(dir.join(name));
        command.args(args(data)).env_clear();
        for keep in [
            "SystemRoot",
            "LOCALAPPDATA",
            "APPDATA",
            "USERPROFILE",
            "TEMP",
            "TMP",
        ] {
            if let Some(value) = std::env::var_os(keep) {
                command.env(keep, value);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command.spawn().map_err(|err| err.to_string())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .manage(open_shell())
        .invoke_handler(tauri::generate_handler![
            sign_decision,
            undo_decision,
            set_secret,
            open_card_window,
            daemon_http::daemon_snapshot,
            daemon_http::daemon_emit_demo,
            shell_form::prepare_shell_form,
            shell_form::shell_metrics,
            shell_form::set_shell_bounds,
            shell_form::window_minimize,
            shell_form::window_toggle_maximize,
            shell_form::window_close,
            snap::snap_maximize_rect
        ])
        .on_window_event(keep_card_window)
        .setup(|app| {
            use tauri::Manager;
            // #36 M2/M3: keep the spawn result. A failed or exited daemon
            // is an error state the page shows; nothing else on the port is
            // used. On Windows the child is in a kill-on-close Job Object.
            #[cfg(feature = "demo-daemon")]
            daemon_child::install(daemon_child::Supervisor::from_spawn(demo::start(
                &daemon_data(),
            )));
            if let Some(window) = app.get_webview_window("main") {
                shell_form::apply_full_chrome(&window);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building dasdevbot");
    app.run(|_app, _event| {
        #[cfg(feature = "demo-daemon")]
        if let tauri::RunEvent::Exit = _event {
            if let Some(supervisor) = daemon_child::installed() {
                supervisor.shutdown();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    fn assert_handler<F>(_handler: F)
    where
        F: Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static,
    {
    }

    #[test]
    fn the_handler_registers_the_decision_commands() {
        assert_handler(tauri::generate_handler![
            super::sign_decision,
            super::undo_decision,
            super::set_secret,
            super::open_card_window,
            super::daemon_http::daemon_snapshot,
            super::daemon_http::daemon_emit_demo,
            super::shell_form::prepare_shell_form,
            super::shell_form::shell_metrics,
            super::shell_form::set_shell_bounds,
            super::shell_form::window_minimize,
            super::shell_form::window_toggle_maximize,
            super::shell_form::window_close,
            super::snap::snap_maximize_rect
        ]);
    }

    fn capability(name: &str) -> serde_json::Value {
        let path = format!("{}/capabilities/{name}.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn permissions_for(window: &str) -> Vec<String> {
        let dir = format!("{}/capabilities", env!("CARGO_MANIFEST_DIR"));
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let name = entry.unwrap().path();
            let stem = name.file_stem().unwrap().to_string_lossy().to_string();
            let cap = capability(&stem);
            let windows: Vec<&str> = cap["windows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap())
                .collect();
            if windows.contains(&window) || windows.contains(&"*") {
                for permission in cap["permissions"].as_array().unwrap() {
                    out.push(permission.as_str().unwrap().to_string());
                }
            }
        }
        out
    }

    #[cfg(feature = "demo-daemon")]
    #[test]
    fn the_demo_daemon_is_loopback_mock_executor() {
        let args = super::demo::args(std::path::Path::new("d/dasdevbot.sqlite"));
        assert_eq!(
            args,
            [
                "serve",
                "--role",
                "executor",
                "--provider",
                "mock",
                "--bind",
                "127.0.0.1:8787",
                "--data",
                "d/dasdevbot.sqlite"
            ]
        );
        for banned in ["--allow-remote", "--web", "--dev-env-secrets", "--token"] {
            assert!(!args.iter().any(|arg| arg == banned), "{banned}");
        }
    }

    /// A sync `#[tauri::command]` runs on the main thread in Tauri 2. Every
    /// command that touches the daemon (HTTP, the shell socket, Windows Hello)
    /// or the disk must be async and run its blocking work off the main thread.
    #[test]
    fn daemon_commands_never_block_the_main_thread() {
        let dir = format!("{}/src", env!("CARGO_MANIFEST_DIR"));
        let mut source = String::new();
        for file in ["lib.rs", "daemon_http.rs"] {
            source.push_str(&std::fs::read_to_string(format!("{dir}/{file}")).unwrap());
        }
        for name in [
            "daemon_snapshot",
            "daemon_emit_demo",
            "sign_decision",
            "undo_decision",
            "set_secret",
        ] {
            let sync = [format!("fn {name}("), format!("pub(crate) fn {name}(")];
            let lines: Vec<&str> = source.lines().collect();
            for (index, line) in lines.iter().enumerate() {
                let trimmed = line.trim_start();
                if sync
                    .iter()
                    .any(|needle| trimmed.starts_with(needle.as_str()))
                {
                    let above = lines[..index].iter().rev().find(|l| !l.trim().is_empty());
                    assert!(
                        above.is_none_or(|l| !l.contains("#[tauri::command]")),
                        "{name} is a sync tauri command and would block the main thread"
                    );
                }
            }
            assert!(
                source.contains(&format!("async fn {name}(")),
                "{name} must be an async command"
            );
        }
    }

    /// #36 H1: no command hands the bearer to a page, no init script writes
    /// it into one, and no window is granted the old permission.
    #[test]
    fn the_session_bearer_never_reaches_a_webview() {
        let dir = format!("{}/src", env!("CARGO_MANIFEST_DIR"));
        let mut source = String::new();
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|ext| ext == "rs") {
                source.push_str(&std::fs::read_to_string(path).unwrap());
            }
        }
        let banned = [
            ["fn session", "_token("].concat(),
            ["js_init", "_script("].concat(),
            ["__DASDEVBOT", "_TOKEN"].concat(),
            ["initialization", "_script("].concat(),
        ];
        for needle in &banned {
            assert!(
                !source.contains(needle.as_str()),
                "{needle} is back in src-tauri"
            );
        }
        for window in ["main", "card", "settings", "voice"] {
            assert!(
                !permissions_for(window)
                    .iter()
                    .any(|p| p == "allow-session-token"),
                "{window}"
            );
        }
        let permissions = format!("{}/permissions", env!("CARGO_MANIFEST_DIR"));
        for entry in std::fs::read_dir(permissions).unwrap() {
            let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            assert!(!text.contains("session_token"), "{text}");
        }
        let api = format!("{}/../src/lib/api.ts", env!("CARGO_MANIFEST_DIR"));
        let api = std::fs::read_to_string(api).unwrap();
        for needle in [
            ["__DASDEVBOT", "_TOKEN"].concat(),
            ["dasdevbot", "-token"].concat(),
            ["session", "_token"].concat(),
            "Authorization".to_string(),
            "Bearer".to_string(),
        ] {
            assert!(!api.contains(needle.as_str()), "api.ts still has {needle}");
        }
    }

    /// The daemon commands are the main window's (and the card's snapshot),
    /// never the voice or settings window's.
    #[test]
    fn the_daemon_http_commands_are_scoped_to_main_and_card() {
        assert!(permissions_for("main")
            .iter()
            .any(|p| p == "allow-daemon-snapshot"));
        assert!(permissions_for("main")
            .iter()
            .any(|p| p == "allow-daemon-emit-demo"));
        assert!(permissions_for("card")
            .iter()
            .any(|p| p == "allow-daemon-snapshot"));
        assert!(!permissions_for("card")
            .iter()
            .any(|p| p == "allow-daemon-emit-demo"));
        for window in ["settings", "voice"] {
            let granted = permissions_for(window);
            assert!(
                !granted.iter().any(|p| p.starts_with("allow-daemon-")),
                "{window}"
            );
        }
    }

    /// The shell's CSP is byte-identical to the daemon's (server.rs).
    #[test]
    fn the_csp_is_byte_identical_to_the_daemons() {
        let path = format!("{}/tauri.conf.json", env!("CARGO_MANIFEST_DIR"));
        let conf: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let csp = conf["app"]["security"]["csp"].as_str().unwrap();
        let server = include_str!("../../../../crates/daemon/src/server.rs");
        let line = server
            .lines()
            .find(|line| line.starts_with("const CONTENT_SECURITY_POLICY: &str = "))
            .unwrap();
        let daemon = line
            .trim_start_matches("const CONTENT_SECURITY_POLICY: &str = \"")
            .trim_end_matches("\";");
        assert_eq!(csp, daemon);
    }

    /// #54 / #36: with a failed or exited bundled daemon, no decision, undo
    /// or secret line is sent. The pipe/socket is never opened, nothing is
    /// written, and the bearer and window secret are never put on a line.
    #[test]
    fn a_failed_or_exited_daemon_sends_no_pipe_line() {
        use serde_json::{json, Value};
        use std::cell::Cell;
        use std::path::Path;
        let dir = std::env::temp_dir().join(format!("dasdevbot-pipe-gate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let data = dir.join("db.sqlite");
        std::fs::write(
            dasdevbotd::session_token_path(&data),
            "0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        for label in ["card", "settings"] {
            std::fs::write(
                dasdevbotd::window_secret_path(&data, label),
                "window-secret-for-test",
            )
            .unwrap();
        }

        #[cfg(unix)]
        let listener = {
            let path = dasdevbotd::shell_socket_path(&data);
            let _ = std::fs::remove_file(&path);
            let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
            listener.set_nonblocking(true).unwrap();
            listener
        };

        let failed = crate::daemon_child::Supervisor::from_spawn(Err("no such file".into()));
        let exited = {
            #[cfg(unix)]
            let child = std::process::Command::new("sh")
                .args(["-c", "exit 1"])
                .spawn()
                .unwrap();
            #[cfg(windows)]
            let child = std::process::Command::new("cmd")
                .args(["/C", "exit 1"])
                .spawn()
                .unwrap();
            let supervisor = crate::daemon_child::Supervisor::from_spawn(Ok(child));
            let start = std::time::Instant::now();
            while supervisor.check().is_ok() {
                assert!(start.elapsed() < std::time::Duration::from_secs(5));
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            supervisor
        };
        let supervisors = [&failed, &exited];

        let lines = [
            (
                "card",
                json!({"op": "decide", "approval_id": "ap_x", "decision": "approve", "reason": null}),
            ),
            ("card", json!({"op": "undo", "approval_id": "ap_x"})),
            (
                "settings",
                json!({"op": "secret", "name": "ollama", "value": "not-a-real-key"}),
            ),
        ];
        for supervisor in supervisors {
            for (label, body) in &lines {
                let opened = Cell::new(0);
                let ready = || supervisor.check();
                let send = |_: &Path, _: &Value| -> Result<Value, String> {
                    opened.set(opened.get() + 1);
                    Err("the pipe must not be opened".into())
                };
                // The same path the commands take: signed_body adds the window
                // secret, then transact adds the bearer and opens the pipe.
                let mut line = body.clone();
                line["window_secret"] = Value::String(super::window_secret(&data, label).unwrap());
                let refused = super::transact_gated(&data, line, &ready, &send).unwrap_err();
                assert!(refused.contains("bundled daemon"), "{refused}");
                assert_eq!(opened.get(), 0, "{} opened the pipe", body["op"]);
                // And the real exchange is never reached: no connection on the socket.
                #[cfg(unix)]
                assert!(matches!(
                    listener.accept(),
                    Err(ref err) if err.kind() == std::io::ErrorKind::WouldBlock
                ));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every shell-socket line passes the gate: `exchange` is called only
    /// through `transact_gated`, and the commands all reach it via `transact`.
    #[test]
    fn every_pipe_caller_goes_through_the_gate() {
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        let body = source.split("#[cfg(test)]\nmod tests").next().unwrap();
        assert_eq!(
            body.matches("exchange(").count(),
            1,
            "only the fn definition"
        );
        assert!(body.contains("transact_gated(data, body, &daemon_child::ready, &exchange)"));
        let gated = body.split("fn transact_gated(").nth(1).unwrap();
        let ready = gated.find("ready()?;").unwrap();
        let send = gated.find("send(data, &body)").unwrap();
        let token = gated.find("session_token_path").unwrap();
        assert!(ready < token && token < send);
    }

    #[test]
    fn only_the_card_window_can_decide() {
        for window in ["main", "settings", "voice"] {
            let granted = permissions_for(window);
            assert!(
                !granted.iter().any(|p| p == "allow-sign-decision"),
                "{window}"
            );
            assert!(
                !granted.iter().any(|p| p == "allow-undo-decision"),
                "{window}"
            );
        }
        let card = permissions_for("card");
        assert!(card.iter().any(|p| p == "allow-sign-decision"));
        assert!(card.iter().any(|p| p == "allow-undo-decision"));
        assert!(!card.iter().any(|p| p.starts_with("allow-set-shell")));
        assert!(permissions_for("main")
            .iter()
            .any(|p| p == "allow-open-card-window"));
    }

    #[test]
    fn the_csp_is_mains_and_bundling_is_per_user_nsis() {
        let path = format!("{}/tauri.conf.json", env!("CARGO_MANIFEST_DIR"));
        let conf: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let csp = conf["app"]["security"]["csp"].as_str().unwrap();
        assert!(csp.contains("default-src 'self'"));
        assert!(csp.contains("frame-ancestors 'none'"));
        assert!(!csp.contains('*'));
        let bundle = &conf["bundle"];
        assert_eq!(bundle["targets"], serde_json::json!(["nsis"]));
        assert_eq!(bundle["createUpdaterArtifacts"], false);
        assert_eq!(bundle["windows"]["nsis"]["installMode"], "currentUser");
        assert!(conf["plugins"].as_object().is_none_or(|p| p.is_empty()));
        let labels: Vec<&str> = conf["app"]["windows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["label"].as_str().unwrap())
            .collect();
        assert_eq!(labels, ["main", "card", "settings", "voice"]);
        let main = &conf["app"]["windows"][0];
        assert_eq!(
            main["minWidth"].as_f64(),
            Some(crate::shell_form::FULL_MIN_WIDTH)
        );
        assert_eq!(
            main["minHeight"].as_f64(),
            Some(crate::shell_form::FULL_MIN_HEIGHT)
        );
    }
}
