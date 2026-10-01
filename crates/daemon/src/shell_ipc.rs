//! Local socket the desktop shell uses. The shell does not open SQLite.
//! The daemon verifies the user, signs, and commits.

use std::io::{BufRead, BufReader, Write};
use std::sync::Arc;

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::UnixListener;
#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, FromRawHandle};

use serde_json::json;

use crate::ipc::{self, PrepareRequest, SignRequest, UndoRequest};
use crate::secrets::{KeyringHandle, Secret, SecretHandle};
use crate::signature::{DECISION_PURPOSE, UNDO_PURPOSE};
use crate::verify_user::{PlatformVerifier, UserVerifier};
use crate::{wall_ms, App, Error};

/// The connecting process, as seen by the OS. Only `is_self` peers are served.
#[derive(Clone, Copy, Debug)]
pub struct Peer {
    pub is_self: bool,
}

pub fn spawn(app: Arc<App>) -> crate::Result<()> {
    #[cfg(unix)]
    {
        spawn_unix(app)
    }
    #[cfg(windows)]
    {
        spawn_pipe(app)
    }
}

#[cfg(unix)]
fn spawn_unix(app: Arc<App>) -> crate::Result<()> {
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
            std::thread::spawn(move || {
                let peer = unix_peer(&stream);
                serve_client(app, peer, stream);
            });
        }
    });
    Ok(())
}

#[cfg(unix)]
fn unix_peer(stream: &std::os::unix::net::UnixStream) -> Peer {
    // geteuid is a libc read of the process uid. It cannot fail.
    let own = unsafe { libc::geteuid() };
    Peer {
        is_self: peer_uid(stream) == Some(own),
    }
}

#[cfg(target_os = "linux")]
fn peer_uid(stream: &std::os::unix::net::UnixStream) -> Option<u32> {
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        )
    };
    if rc != 0 {
        None
    } else {
        Some(cred.uid)
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
fn peer_uid(_stream: &std::os::unix::net::UnixStream) -> Option<u32> {
    None
}

/// Owner-only named pipe. The first instance must be ours, so a squatter
/// that created the name first makes startup fail.
#[cfg(windows)]
fn spawn_pipe(app: Arc<App>) -> crate::Result<()> {
    let name = crate::win_acl::wide(crate::shell_pipe_name(&app.data).as_ref());
    let own = crate::win_acl::current_user()?;
    let descriptor = crate::win_acl::owner_only(&own)?;
    let mut pipe = create_pipe(&name, &descriptor, true)?;
    std::thread::spawn(move || loop {
        if connect_pipe(&pipe).is_err() {
            // Drop the broken instance and start a fresh one.
            match next_pipe(&name, &descriptor) {
                Some(next) => pipe = next,
                None => return,
            }
            continue;
        }
        // The next instance exists before this one is served.
        let Some(next) = next_pipe(&name, &descriptor) else {
            return;
        };
        let served = std::mem::replace(&mut pipe, next);
        let peer = pipe_peer(&served, &own);
        let app = Arc::clone(&app);
        std::thread::spawn(move || {
            let served = serve_client(app, peer, served);
            // Wait for the client to read the reply before the handle closes.
            let _ = served.sync_all();
        });
    });
    Ok(())
}

#[cfg(windows)]
fn create_pipe(
    name: &[u16],
    descriptor: &crate::win_acl::Descriptor,
    first: bool,
) -> std::io::Result<std::fs::File> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
    use windows::Win32::System::Pipes::{
        CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
        PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    let mut open_mode = PIPE_ACCESS_DUPLEX;
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let attributes = descriptor.attributes();
    // SAFETY: name is NUL-terminated, and attributes and the descriptor it
    // points at outlive the call.
    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            4096,
            4096,
            0,
            Some(&attributes),
        )
    };
    if handle.is_invalid() {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: handle is a fresh pipe handle owned by nothing else.
    Ok(unsafe { std::fs::File::from_raw_handle(handle.0) })
}

/// A new instance, retrying while the system is short of resources.
#[cfg(windows)]
fn next_pipe(name: &[u16], descriptor: &crate::win_acl::Descriptor) -> Option<std::fs::File> {
    for _ in 0..50 {
        match create_pipe(name, descriptor, false) {
            Ok(pipe) => return Some(pipe),
            Err(err) => {
                eprintln!("dasdevbotd: shell pipe instance failed: {err}");
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
    }
    eprintln!("dasdevbotd: shell pipe stopped");
    None
}

#[cfg(windows)]
fn connect_pipe(pipe: &std::fs::File) -> windows::core::Result<()> {
    use windows::Win32::Foundation::{ERROR_PIPE_CONNECTED, HANDLE};
    use windows::Win32::System::Pipes::ConnectNamedPipe;
    // SAFETY: the handle is a live, synchronous pipe instance owned by pipe.
    match unsafe { ConnectNamedPipe(HANDLE(pipe.as_raw_handle()), None) } {
        // A client that connected before this call is still a connection.
        Err(err) if err.code() == ERROR_PIPE_CONNECTED.to_hresult() => Ok(()),
        other => other,
    }
}

/// The client process must run as the daemon's user. Any failure is foreign.
#[cfg(windows)]
fn pipe_peer(pipe: &std::fs::File, own: &crate::win_acl::UserSid) -> Peer {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Pipes::GetNamedPipeClientProcessId;
    let mut pid = 0u32;
    // SAFETY: the handle is a connected pipe owned by pipe; pid is a local.
    let known = unsafe { GetNamedPipeClientProcessId(HANDLE(pipe.as_raw_handle()), &mut pid) };
    let is_self = known.is_ok()
        && pid != 0
        && crate::win_acl::process_user(pid).is_ok_and(|user| user.equals(own));
    Peer { is_self }
}

fn serve_client<S: std::io::Read + Write>(app: Arc<App>, peer: Peer, stream: S) -> S {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_ok() {
        let verifier = PlatformVerifier;
        let keys = KeyringHandle;
        let response = handle_line(&app, &verifier, &keys, peer, &line);
        let _ = writeln!(reader.get_mut(), "{response}");
    }
    reader.into_inner()
}

pub fn handle_line(
    app: &App,
    verifier: &dyn UserVerifier,
    secrets: &dyn SecretHandle,
    peer: Peer,
    line: &str,
) -> String {
    let request: serde_json::Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(_) => return err_json(&Error::BadRequest("shell request is not json".into())),
    };
    match dispatch(app, verifier, secrets, peer, &request) {
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
    peer: Peer,
    request: &serde_json::Value,
) -> crate::Result<serde_json::Value> {
    if !peer.is_self {
        return Err(Error::Unauthorized);
    }
    let token = request
        .get("token")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    if !crate::tokens_equal(token, &app.token) {
        return Err(Error::Unauthorized);
    }
    let op = request
        .get("op")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    match op {
        "decide" => decide(app, verifier, secrets, request),
        "undo" => undo(app, verifier, secrets, request),
        "secret" => secret(app, secrets, request),
        "prepare" => prepare(app, request),
        "hello-enroll" => hello_enroll(app, request),
        "hello-reset" => hello_reset(app, verifier, request),
        _ => Err(Error::BadRequest("unknown shell op".into())),
    }
}

fn window_of(app: &App, request: &serde_json::Value) -> crate::Result<&'static str> {
    let secret = request
        .get("window_secret")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    app.window_secrets
        .resolve(secret)
        .ok_or(Error::Unauthorized)
}

fn decide(
    app: &App,
    verifier: &dyn UserVerifier,
    secrets: &dyn SecretHandle,
    request: &serde_json::Value,
) -> crate::Result<serde_json::Value> {
    let window = window_of(app, request)?;
    let approval_id = required(request, "approval_id")?;
    let decision = required(request, "decision")?;
    let reason = request.get("reason").and_then(|value| value.as_str());
    let client_signature = request
        .get("client_signature")
        .and_then(|value| value.as_str());
    let client_nonce = request.get("client_nonce").and_then(|value| value.as_str());
    let mut store = app.store.lock().expect("store");
    let fencing = store.leader_fencing()?;
    let record = ipc::sign_decision(
        &mut store,
        SignRequest {
            window,
            voice: false,
            approval_id: &approval_id,
            decision: &decision,
            reason,
            now_ms: wall_ms(),
            fencing,
            secrets,
            verifier,
            audit_seed: &app.audit_seed,
            client_signature,
            client_nonce,
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
    let window = window_of(app, request)?;
    let approval_id = required(request, "approval_id")?;
    let client_signature = request
        .get("client_signature")
        .and_then(|value| value.as_str());
    let client_nonce = request.get("client_nonce").and_then(|value| value.as_str());
    let mut store = app.store.lock().expect("store");
    let fencing = store.leader_fencing()?;
    let record = ipc::undo_decision(
        &mut store,
        UndoRequest {
            window,
            voice: false,
            approval_id: &approval_id,
            now_ms: wall_ms(),
            fencing,
            secrets,
            verifier,
            audit_seed: &app.audit_seed,
            client_signature,
            client_nonce,
        },
    )?;
    Ok(json!({"status": record.status}))
}

fn secret(
    app: &App,
    secrets: &dyn SecretHandle,
    request: &serde_json::Value,
) -> crate::Result<serde_json::Value> {
    let window = window_of(app, request)?;
    let name = required(request, "name")?;
    let value = required(request, "value")?;
    let secret = Secret::new(value);
    let mut store = app.store.lock().expect("store");
    let tail = ipc::store_secret(
        &app.role,
        window,
        &name,
        &secret,
        secrets,
        &mut store,
        &app.audit_seed,
    )?;
    Ok(json!({"last4": tail}))
}

fn prepare(app: &App, request: &serde_json::Value) -> crate::Result<serde_json::Value> {
    let window = window_of(app, request)?;
    let approval_id = required(request, "approval_id")?;
    let decision = request
        .get("decision")
        .and_then(|value| value.as_str())
        .unwrap_or("approve");
    let reason = request
        .get("reason")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let purpose = request
        .get("purpose")
        .and_then(|value| value.as_str())
        .unwrap_or(DECISION_PURPOSE);
    if purpose != DECISION_PURPOSE && purpose != UNDO_PURPOSE {
        return Err(Error::BadRequest("unknown signature purpose".into()));
    }
    let mut store = app.store.lock().expect("store");
    let fencing = store.leader_fencing()?;
    ipc::prepare_signature(
        &mut store,
        PrepareRequest {
            approval_id: &approval_id,
            decision,
            reason,
            window,
            fencing,
            now_ms: wall_ms(),
            purpose,
        },
    )
}

fn hello_enroll(app: &App, request: &serde_json::Value) -> crate::Result<serde_json::Value> {
    let _window = window_of(app, request)?;
    #[cfg(not(windows))]
    {
        Err(Error::Forbidden(
            "external tier is denied without Windows Hello".into(),
        ))
    }
    #[cfg(windows)]
    {
        let (blob_type, public_key) =
            crate::hello_key::hello_public_key_material().map_err(Error::Forbidden)?;
        let mut store = app.store.lock().expect("store");
        store.enroll_hello_public_key(blob_type, &public_key, wall_ms(), &app.audit_seed)?;
        Ok(json!({"enrolled": true}))
    }
}

/// The consent text for a Hello reset names the action.
const HELLO_RESET_PROMPT: &str = "reset the Windows Hello approval key";

/// Settings window only, with a reason and a verified user. The reset is
/// audited before the key and its pin are dropped.
fn hello_reset(
    app: &App,
    verifier: &dyn UserVerifier,
    request: &serde_json::Value,
) -> crate::Result<serde_json::Value> {
    let window = window_of(app, request)?;
    if window != dasdevbot_core::SETTINGS_WINDOW {
        return Err(Error::Forbidden(
            "only the settings window may reset the Windows Hello key".into(),
        ));
    }
    let reason = request
        .get("reason")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| Error::Forbidden("a Hello reset needs a reason".into()))?;
    verifier
        .verify_user(HELLO_RESET_PROMPT)
        .map_err(|err| Error::Forbidden(format!("user verification failed: {err}")))?;
    let mut store = app.store.lock().expect("store");
    store.reset_hello_enrollment(reason, wall_ms(), &app.audit_seed)?;
    Ok(json!({"reset": true}))
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

    const SELF: Peer = Peer { is_self: true };
    const FOREIGN: Peer = Peer { is_self: false };

    #[cfg(target_os = "linux")]
    #[test]
    fn a_socket_pair_peer_is_self() {
        let (left, _right) = std::os::unix::net::UnixStream::pair().unwrap();
        assert!(unix_peer(&left).is_self);
    }

    #[test]
    fn the_pipe_name_is_stable_per_data_path() {
        let one = crate::shell_pipe_name(std::path::Path::new("a/db.sqlite"));
        assert!(one.starts_with(r"\\.\pipe\dasdevbot-"), "{one}");
        assert_eq!(one.len(), r"\\.\pipe\dasdevbot-".len() + 32);
        assert_eq!(
            one,
            crate::shell_pipe_name(std::path::Path::new("a/db.sqlite"))
        );
        assert_ne!(
            one,
            crate::shell_pipe_name(std::path::Path::new("b/db.sqlite"))
        );
    }

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
        let peer = SELF;
        let spoofed = handle_line(
            &app,
            &TestVerifier { allow: true },
            &keys,
            peer,
            &json!({
                "op": "decide",
                "token": app.token,
                "approval_id": id,
                "decision": "approve",
                "window": "card",
            })
            .to_string(),
        );
        assert!(spoofed.contains("unauthorized"), "{spoofed}");
        assert_eq!(
            app.store.lock().unwrap().approval_status(&id).unwrap(),
            "pending"
        );
        let foreign = handle_line(
            &app,
            &TestVerifier { allow: true },
            &keys,
            FOREIGN,
            &json!({
                "op": "decide",
                "token": app.token,
                "approval_id": id,
                "decision": "approve",
                "window_secret": app.window_secrets.card,
            })
            .to_string(),
        );
        assert!(foreign.contains("unauthorized"), "{foreign}");
        let denied = handle_line(
            &app,
            &TestVerifier { allow: false },
            &keys,
            peer,
            &json!({
                "op": "decide",
                "token": app.token,
                "approval_id": id,
                "decision": "approve",
                "window_secret": app.window_secrets.card,
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
            peer,
            &json!({
                "op": "decide",
                "token": app.token,
                "approval_id": id,
                "decision": "approve",
                "window_secret": app.window_secrets.card,
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
            peer,
            &json!({
                "op": "decide",
                "approval_id": id,
                "decision": "deny",
                "window_secret": app.window_secrets.card,
            })
            .to_string(),
        );
        assert!(
            missing.contains("unauthorized") || missing.contains("ok\":false"),
            "{missing}"
        );
    }

    #[test]
    fn a_hello_reset_needs_settings_a_reason_and_the_user() {
        use crate::hello_key::X509_SPKI_BLOB;
        let dir = std::env::temp_dir().join(format!("dasdevbot-reset-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let (app, _rx) = build_app(
            Config {
                data: dir.join("db.sqlite"),
                web_root: None,
                role: "executor".into(),
                token: Some("0123456789abcdef0123456789abcdef".into()),
            },
            Box::new(MockProvider::new()),
        )
        .unwrap();
        let keys = MemorySecrets::new();
        let peer = SELF;
        app.store
            .lock()
            .unwrap()
            .enroll_hello_public_key(X509_SPKI_BLOB, "aa", 10, &app.audit_seed)
            .unwrap();
        let reset = |allow: bool, window_secret: &str, reason: &str| {
            handle_line(
                &app,
                &TestVerifier { allow },
                &keys,
                peer,
                &json!({
                    "op": "hello-reset",
                    "token": app.token,
                    "window_secret": window_secret,
                    "reason": reason,
                })
                .to_string(),
            )
        };
        let resets = || {
            app.store
                .lock()
                .unwrap()
                .connection()
                .query_row(
                    "SELECT COUNT(*) FROM audit_log WHERE kind = 'hello.reset'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        };
        let card = reset(true, &app.window_secrets.card, "lost device");
        assert!(card.contains("settings window"), "{card}");
        let unknown = reset(true, "not-a-window", "lost device");
        assert!(unknown.contains("unauthorized"), "{unknown}");
        let empty = reset(true, &app.window_secrets.settings, "   ");
        assert!(empty.contains("reason"), "{empty}");
        let denied = reset(false, &app.window_secrets.settings, "lost device");
        assert!(denied.contains("user verification"), "{denied}");
        assert_eq!(resets(), 0);
        let refused = app
            .store
            .lock()
            .unwrap()
            .enroll_hello_public_key(X509_SPKI_BLOB, "bb", 20, &app.audit_seed)
            .unwrap_err();
        assert!(
            refused.to_string().contains("pinned fingerprint"),
            "{refused}"
        );
        let accepted = reset(true, &app.window_secrets.settings, "lost device");
        assert!(accepted.contains("\"ok\":true"), "{accepted}");
        assert_eq!(resets(), 1);
        let mut store = app.store.lock().unwrap();
        store
            .enroll_hello_public_key(X509_SPKI_BLOB, "bb", 30, &app.audit_seed)
            .unwrap();
        assert_eq!(
            store.hello_public_key().unwrap(),
            Some((X509_SPKI_BLOB, "bb".to_string()))
        );
        assert!(crate::audit_log::verify(&store, &app.audit_seed).unwrap());
    }

    /// The real pipe: an own-user client is served. The squatter refusal
    /// (FILE_FLAG_FIRST_PIPE_INSTANCE) needs a real Windows host to check.
    #[cfg(windows)]
    #[test]
    fn the_named_pipe_serves_the_same_user() {
        use std::io::{BufRead, BufReader, Write};
        let dir = std::env::temp_dir().join(format!("dasdevbot-pipe-{}", uuid::Uuid::new_v4()));
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
        spawn(Arc::clone(&app)).unwrap();
        let mut pipe = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(crate::shell_pipe_name(&data))
            .unwrap();
        let line = json!({"op": "no-such-op", "token": app.token}).to_string();
        writeln!(pipe, "{line}").unwrap();
        let mut reply = String::new();
        BufReader::new(pipe).read_line(&mut reply).unwrap();
        // A foreign peer would get "unauthorized" before the op is looked at.
        assert!(reply.contains("unknown shell op"), "{reply}");
    }
}
