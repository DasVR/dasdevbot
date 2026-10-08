//! Session bearer for the webview, card-window decisions, and settings-window secret entry.
//! The Tauri window label only selects which per-launch secret to send.
//! The daemon derives the window from that secret. The shell does not open
//! SQLite. Every command is a line on the daemon's local socket.
//! See docs/shell-ipc.md.
//!
//! The shell forms (full, companion, pill) live in `shell_form` and
//! `geometry`. Approval cards are decided only in the `card` window: the main
//! window can ask to show it (`open_card_window`) but holds no decision
//! capability.

mod daemon_http;
mod geometry;
mod shell_form;

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

fn transact(data: &Path, mut body: Value) -> Result<Value, String> {
    let token = std::fs::read_to_string(dasdevbotd::session_token_path(data))
        .map_err(|err| err.to_string())?;
    body["token"] = Value::String(token.trim().to_string());
    exchange(data, &body)
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

/// Session bearer for the webview. The daemon never embeds this in HTML.
#[tauri::command]
async fn session_token() -> Result<String, String> {
    daemon_http::off_main(read_session_token).await
}

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

/// Runs before the page parses. Assigns `window` because Tauri wraps the script in a function.
fn session_init_script() -> String {
    let literal = match read_session_token() {
        Ok(token) => serde_json::to_string(&token).unwrap_or_else(|_| "\"\"".to_string()),
        Err(_) => "\"\"".to_string(),
    };
    format!("window.__DASDEVBOT_TOKEN = {literal};")
}

fn session_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("dasdevbot-session")
        .js_init_script(session_init_script())
        .build()
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

/// The card window hides instead of closing, so it can be shown again.
fn keep_card_window(window: &tauri::Window, event: &tauri::WindowEvent) {
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
    use std::sync::Mutex;

    pub struct Daemon(pub Mutex<Option<Child>>);

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
        .plugin(session_plugin())
        .manage(open_shell())
        .invoke_handler(tauri::generate_handler![
            session_token,
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
            shell_form::window_close
        ])
        .on_window_event(keep_card_window)
        .setup(|app| {
            use tauri::Manager;
            #[cfg(feature = "demo-daemon")]
            {
                let child = demo::start(&daemon_data()).ok();
                app.manage(demo::Daemon(std::sync::Mutex::new(child)));
            }
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
            use tauri::Manager;
            if let Some(state) = _app.try_state::<demo::Daemon>() {
                if let Some(mut child) = state.0.lock().expect("daemon").take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
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
            super::shell_form::window_close
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
            "session_token",
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
    }
}
