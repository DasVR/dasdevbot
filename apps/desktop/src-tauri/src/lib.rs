use std::path::PathBuf;

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
        .invoke_handler(tauri::generate_handler![session_token])
        .run(tauri::generate_context!())
        .expect("error while running dasdevbot");
}
