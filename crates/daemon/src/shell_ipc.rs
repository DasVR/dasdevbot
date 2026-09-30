//! Local socket the desktop shell uses. The shell does not open SQLite.
//! The daemon verifies the user, signs, and commits.

use std::io::{BufRead, BufReader, Write};
use std::sync::Arc;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use serde_json::json;

use crate::ipc::{self, SignRequest, UndoRequest};
use crate::secrets::{KeyringHandle, Secret, SecretHandle};
use crate::verify_user::{PlatformVerifier, UserVerifier};
use crate::{wall_ms, App, Error};

pub fn spawn(app: Arc<App>) -> crate::Result<()> {
    #[cfg(unix)]
    {
        spawn_unix(app)
    }
    #[cfg(windows)]
    {
        spawn_tcp(app)
    }
}

#[cfg(unix)]
fn spawn_unix(app: Arc<App>) -> crate::Result<()> {
        use std::os::unix::net::UnixListener;
    let path = crate::shell_socket_path(&app.data);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let app = Arc::clone(&app);
            std::thread::spawn(move || serve_client(app, stream));
        }
    });
    Ok(())
}

#[cfg(windows)]
fn spawn_tcp(app: Arc<App>) -> crate::Result<()> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    crate::write_private(crate::shell_port_path(&app.data), port.to_string().as_bytes())?;
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let app = Arc::clone(&app);
            std::thread::spawn(move || serve_client(app, stream));
        }
    });
    Ok(())
}

fn serve_client(app: Arc<App>, stream: impl std::io::Read + Write) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let verifier = PlatformVerifier;
    let keys = KeyringHandle;
    let response = handle_line(&app, &verifier, &keys, &line);
    let _ = writeln!(reader.get_mut(), "{response}");
}

pub fn handle_line(
    app: &App,
    verifier: &dyn UserVerifier,
    secrets: &dyn SecretHandle,
    line: &str,
) -> String {
    let request: serde_json::Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(_) => return err_json(&Error::BadRequest("shell request is not json".into())),
    };
    match dispatch(app, verifier, secrets, &request) {
        Ok(value) => json!({"ok": true, "result": value}).to_string(),
        Err(err) => err_json(&err),
    }
}

fn err_json(err: &Error) -> String {
    json!({"ok": false, "error": err.to_string()}).to_string()
}

fn dispatch(
    app: &App,
    verifier: &dyn UserVerifier,
    secrets: &dyn SecretHandle,
    request: &serde_json::Value,
) -> crate::Result<serde_json::Value> {
    let token = request.get("token").and_then(|value| value.as_str()).unwrap_or("");
    if token != app.token {
        return Err(Error::Unauthorized);
    }
    let op = request.get("op").and_then(|value| value.as_str()).unwrap_or("");
    match op {
        "decide" => decide(app, verifier, secrets, request),
        "undo" => undo(app, verifier, secrets, request),
        "secret" => secret(app, secrets, request),
        _ => Err(Error::BadRequest("unknown shell op".into())),
    }
}

fn decide(
    app: &App,
    verifier: &dyn UserVerifier,
    secrets: &dyn SecretHandle,
    request: &serde_json::Value,
) -> crate::Result<serde_json::Value> {
    let approval_id = required(request, "approval_id")?;
    let decision = required(request, "decision")?;
    let window = required(request, "window")?;
    let reason = request.get("reason").and_then(|value| value.as_str());
    let mut store = app.store.lock().expect("store");
    let fencing = store.leader_fencing()?;
    let record = ipc::sign_decision(
        &mut store,
        SignRequest {
            window: &window,
            voice: false,
            approval_id: &approval_id,
            decision: &decision,
            reason,
            now_ms: wall_ms(),
            fencing,
            secrets,
            verifier,
            audit_seed: &app.audit_seed,
        },
    )?;
    Ok(json!({"status": record.status}))
}

fn undo(
    app: &App,
    verifier: &dyn UserVerifier,
    secrets: &dyn SecretHandle,
    request: &serde_json::Value,
) -> crate::Result<serde_json::Value> {
    let approval_id = required(request, "approval_id")?;
    let window = required(request, "window")?;
    let mut store = app.store.lock().expect("store");
    let fencing = store.leader_fencing()?;
    let record = ipc::undo_decision(
        &mut store,
        UndoRequest {
            window: &window,
            voice: false,
            approval_id: &approval_id,
            now_ms: wall_ms(),
            fencing,
            secrets,
            verifier,
            audit_seed: &app.audit_seed,
        },
    )?;
    Ok(json!({"status": record.status}))
}

fn secret(
    app: &App,
    secrets: &dyn SecretHandle,
    request: &serde_json::Value,
) -> crate::Result<serde_json::Value> {
    let window = required(request, "window")?;
    let name = required(request, "name")?;
    let value = required(request, "value")?;
    let secret = Secret::new(value);
    let mut store = app.store.lock().expect("store");
    let tail = ipc::store_secret(
        &app.role,
        &window,
        &name,
        &secret,
        secrets,
        &mut store,
        &app.audit_seed,
    )?;
    Ok(json!({"last4": tail}))
}

fn required(request: &serde_json::Value, field: &str) -> crate::Result<String> {
    request
        .get(field)
        .and_then(|value| value.as_str())
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Error::BadRequest(format!("shell request needs {field}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{MemorySecrets, APPROVAL_KEY_NAME};
    use crate::store::NewApproval;
    use crate::verify_user::TestVerifier;
    use crate::{build_app, Config, MockProvider};
    use dasdevbot_core::CARD_WINDOW;

    #[test]
    fn the_shell_line_does_not_commit_without_user_verification() {
        let dir = std::env::temp_dir().join(format!("dasdevbot-shell-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let data = dir.join("db.sqlite");
        let (app, _rx) = build_app(
            Config {
                data: data.clone(),
                web_root: None,
                role: "executor".into(),
                token: Some("0123456789abcdef0123456789abcdef".into()),
            },
            Box::new(MockProvider::new()),
        )
        .unwrap();
        let keys = MemorySecrets::new();
        keys.insert(
            APPROVAL_KEY_NAME,
            "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff",
        );
        let id = {
            let mut store = app.store.lock().unwrap();
            let now = crate::wall_ms();
            let job_id = store
                .enqueue_job("reviewer", "job-shell", "{}", now)
                .unwrap()
                .unwrap();
            store.claim_at("owner", now, 60_000).unwrap().unwrap();
            store
                .record_approval_and_wait(
                    now,
                    "owner",
                    NewApproval {
                        job_id,
                        agent_id: "reviewer".into(),
                        thread_id: "thread".into(),
                        effect_class: "write_local".into(),
                        action: "edit".into(),
                        purpose: "purpose".into(),
                        draft: "draft".into(),
                        evidence: "evidence".into(),
                        evidence_repo: "DasVR/NIL".into(),
                        evidence_ref: "phase0".into(),
                        evidence_event_id: "ev".into(),
                        evidence_kind: "workspace.write".into(),
                        provider: "fake".into(),
                        model: "fake".into(),
                        usage_kind: "estimated".into(),
                        input_tokens: 1,
                        output_tokens: 1,
                        micro_usd: 0,
                        ledger_note: "fake".into(),
                        project: "DasVR/NIL".into(),
                    },
                    "{}",
                    "{}",
                )
                .unwrap()
        };
        let denied = handle_line(
            &app,
            &TestVerifier { allow: false },
            &keys,
            &json!({
                "op": "decide",
                "token": app.token,
                "approval_id": id,
                "decision": "approve",
                "window": CARD_WINDOW,
            })
            .to_string(),
        );
        assert!(denied.contains("user verification"), "{denied}");
        assert_eq!(
            app.store.lock().unwrap().approval_status(&id).unwrap(),
            "pending"
        );
        let accepted = handle_line(
            &app,
            &TestVerifier { allow: true },
            &keys,
            &json!({
                "op": "decide",
                "token": app.token,
                "approval_id": id,
                "decision": "approve",
                "window": CARD_WINDOW,
            })
            .to_string(),
        );
        assert!(accepted.contains("\"ok\":true"), "{accepted}");
        assert_eq!(
            app.store.lock().unwrap().approval_status(&id).unwrap(),
            "approved"
        );
        let missing = handle_line(
            &app,
            &TestVerifier { allow: true },
            &keys,
            &json!({
                "op": "decide",
                "approval_id": id,
                "decision": "deny",
                "window": CARD_WINDOW,
            })
            .to_string(),
        );
        assert!(missing.contains("unauthorized") || missing.contains("ok\":false"), "{missing}");
    }
}
