//! Session bearer for the webview, card-window decisions, and settings-window secret entry.
//! The Tauri window label only selects which per-launch secret to send.
//! The daemon derives the window from that secret. The shell does not open
//! SQLite. Every command is a line on the daemon's local socket.
//! See docs/shell-ipc.md.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

struct ShellState {
    data: PathBuf,
}

#[tauri::command]
fn sign_decision(
    window: tauri::Window,
    state: tauri::State<'_, ShellState>,
    id: String,
    decision: String,
    reason: Option<String>,
) -> Result<(), String> {
    let label = window.label().to_string();
    finish_signed(
        &state.data,
        &label,
        json!({
            "op": "decide",
            "approval_id": id,
            "decision": decision,
            "reason": reason,
        }),
    )
}

#[tauri::command]
fn undo_decision(
    window: tauri::Window,
    state: tauri::State<'_, ShellState>,
    id: String,
) -> Result<(), String> {
    let label = window.label().to_string();
    finish_signed(
        &state.data,
        &label,
        json!({
            "op": "undo",
            "approval_id": id,
        }),
    )
}

#[tauri::command]
fn set_secret(
    window: tauri::Window,
    state: tauri::State<'_, ShellState>,
    handle: String,
    value: String,
) -> Result<SecretStored, String> {
    let label = window.label().to_string();
    let response = signed_body(
        &state.data,
        &label,
        json!({
            "op": "secret",
            "name": handle,
            "value": value,
        }),
    )?;
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
    std::env::var("DASDEVBOT_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/dasdevbot.sqlite"))
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
fn session_token() -> Result<String, String> {
    read_session_token()
}

fn read_session_token() -> Result<String, String> {
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
    PathBuf::from("data/dasdevbot.sqlite.token")
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(session_plugin())
        .manage(open_shell())
        .invoke_handler(tauri::generate_handler![
            session_token,
            sign_decision,
            undo_decision,
            set_secret
        ])
        .run(tauri::generate_context!())
        .expect("error while running dasdevbot");
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
            super::set_secret
        ]);
    }
}
