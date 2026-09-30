//! Session bearer for the webview, card-window decisions, and settings-window secret entry.
//! The window label comes from the Tauri runtime. The shell does not open
//! SQLite. Every command is a line on the daemon's local socket.

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
    transact(
        &state.data,
        json!({
            "op": "decide",
            "approval_id": id,
            "decision": decision,
            "reason": reason,
            "window": label,
        }),
    )?;
    Ok(())
}

#[tauri::command]
fn undo_decision(
    window: tauri::Window,
    state: tauri::State<'_, ShellState>,
    id: String,
) -> Result<(), String> {
    let label = window.label().to_string();
    transact(
        &state.data,
        json!({
            "op": "undo",
            "approval_id": id,
            "window": label,
        }),
    )?;
    Ok(())
}

#[tauri::command]
fn set_secret(
    window: tauri::Window,
    state: tauri::State<'_, ShellState>,
    handle: String,
    value: String,
) -> Result<SecretStored, String> {
    let label = window.label().to_string();
    let response = transact(
        &state.data,
        json!({
            "op": "secret",
            "name": handle,
            "value": value,
            "window": label,
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
        let port_text = std::fs::read_to_string(dasdevbotd::shell_port_path(data))
            .map_err(|err| err.to_string())?;
        let port: u16 = port_text
            .trim()
            .parse()
            .map_err(|_| "shell port is not a number".to_string())?;
        let stream = std::net::TcpStream::connect(("127.0.0.1", port)).map_err(|err| err.to_string())?;
        write_and_read(stream, body)
    }
}

fn write_and_read(
    mut stream: impl std::io::Read + Write,
    body: &Value,
) -> Result<Value, String> {
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
    ShellState { data: daemon_data() }
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
    #[test]
    fn the_handler_registers_the_decision_commands() {
        let _handler = tauri::generate_handler![
            super::sign_decision,
            super::undo_decision,
            super::set_secret
        ];
    }
}
