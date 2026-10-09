//! Read the snapshot and send the demo event for the webview.
//!
//! A bundled webview runs on the Tauri origin, and the daemon sends no CORS
//! headers, so the page cannot fetch `/v1` itself. These two commands make the
//! loopback HTTP call from the shell process instead. The address is fixed to
//! 127.0.0.1:8787. Decisions do not go through HTTP: `/decision` and `/undo`
//! stay 403 and the card window signs over the socket.
//!
//! #36 H1: the bearer is read here and never handed to the page.
//!
//! #36 M2, authenticated both ways:
//! - Before anything is sent, the bundled daemon must be running
//!   (`daemon_child::ready`). A failed or exited spawn is an error, never a
//!   fallback to whatever else listens on the port.
//! - The daemon proves it holds this launch's bearer (`http_proof`) on a
//!   handshake that carries no bearer. Only then does the shell send the
//!   bearer, on every call including the snapshot.
//! - Every response is checked against a fresh proof over its exact status and
//!   body before the shell trusts it.

use std::io::Read;
use std::time::Duration;

use dasdevbotd::http_proof::{self, CHALLENGE_HEADER, PROOF_HEADER};

const DAEMON: &str = "http://127.0.0.1:8787";

/// Largest response body the shell reads (the snapshot is bounded far below this).
const MAX_BODY: u64 = 8 * 1024 * 1024;

pub(crate) const UNPROVEN: &str = "The process on 127.0.0.1:8787 did not prove it is this app's daemon. dasdevbot did not send it anything.";
const NOT_RUNNING: &str = "the daemon is not running on 127.0.0.1:8787";

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(Duration::from_millis(500))
        .timeout(Duration::from_secs(5))
        .build()
}

/// One answered request: status, proof header and body bytes.
pub(crate) struct Answer {
    pub status: u16,
    pub proof: Option<String>,
    pub body: Vec<u8>,
}

fn answer(response: ureq::Response) -> Result<Answer, String> {
    let status = response.status();
    let proof = response.header(PROOF_HEADER).map(str::to_string);
    let mut body = Vec::new();
    response
        .into_reader()
        .take(MAX_BODY)
        .read_to_end(&mut body)
        .map_err(|err| err.to_string())?;
    Ok(Answer {
        status,
        proof,
        body,
    })
}

fn send(request: ureq::Request, json: Option<&serde_json::Value>) -> Result<Answer, String> {
    let sent = match json {
        Some(body) => request.send_json(body),
        None => request.call(),
    };
    match sent {
        Ok(response) => answer(response),
        Err(ureq::Error::Status(_, response)) => answer(response),
        Err(ureq::Error::Transport(_)) => Err(NOT_RUNNING.into()),
    }
}

/// Check the proof, then turn the answer into JSON or the daemon's error.
/// An unproven answer is never parsed, so a squatter's text never reaches the page.
pub(crate) fn trusted(
    token: &str,
    challenge: &str,
    request_line: &str,
    answer: Answer,
) -> Result<serde_json::Value, String> {
    if !http_proof::verify(
        token,
        challenge,
        request_line,
        answer.status,
        &answer.body,
        answer.proof.as_deref(),
    ) {
        return Err(UNPROVEN.into());
    }
    let value: Option<serde_json::Value> = serde_json::from_slice(&answer.body).ok();
    if (200..300).contains(&answer.status) {
        return value.ok_or_else(|| "the daemon sent a body that is not JSON".to_string());
    }
    Err(value
        .as_ref()
        .and_then(|value| value["error"].as_str().map(str::to_string))
        .unwrap_or_else(|| format!("daemon answered {}", answer.status)))
}

/// The authenticated loopback call every command makes (#36 M2).
fn call(
    method: &str,
    path: &str,
    json: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    crate::daemon_child::ready()?;
    let token = crate::read_session_token()?;
    call_checked(
        DAEMON,
        &token,
        method,
        path,
        json,
        &crate::daemon_child::ready,
    )
}

/// Tests: the same call with no bundled child to re-check.
#[cfg(test)]
fn call_at(
    base: &str,
    token: &str,
    method: &str,
    path: &str,
    json: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    call_checked(base, token, method, path, json, &|| Ok(()))
}

/// `recheck` runs after the daemon's proof verifies and before the bearer
/// request goes out (SD, #46): if the bundled child exited in between, the
/// bearer is never sent.
fn call_checked(
    base: &str,
    token: &str,
    method: &str,
    path: &str,
    json: Option<serde_json::Value>,
    recheck: &dyn Fn() -> Result<(), String>,
) -> Result<serde_json::Value, String> {
    let token = token.to_string();
    let agent = agent();

    // 1. The daemon proves itself before the bearer leaves this process.
    let challenge = http_proof::new_challenge();
    let hello = send(
        agent
            .get(&format!("{base}/v1/health"))
            .set(CHALLENGE_HEADER, &challenge),
        None,
    )?;
    trusted(&token, &challenge, "GET /v1/health", hello)?;
    recheck()?;

    // 2. The call itself, with the bearer and a fresh challenge.
    let challenge = http_proof::new_challenge();
    let request = agent
        .request(method, &format!("{base}{path}"))
        .set("Authorization", &format!("Bearer {token}"))
        .set(CHALLENGE_HEADER, &challenge);
    let reply = send(request, json.as_ref())?;
    trusted(&token, &challenge, &format!("{method} {path}"), reply)
}

/// The event kind for the demo buttons. Only the kinds main routes to a teammate.
pub(crate) fn demo_kind(forced: bool) -> &'static str {
    if forced {
        "repo.force_push"
    } else {
        "repo.push"
    }
}

/// Run blocking loopback work off the main thread. A sync `#[tauri::command]`
/// runs on the main (UI) thread in Tauri 2, so a slow daemon froze every
/// window for up to the HTTP timeout.
pub(crate) async fn off_main<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|err| err.to_string())?
}

#[tauri::command]
pub(crate) async fn daemon_snapshot() -> Result<serde_json::Value, String> {
    off_main(snapshot_blocking).await
}

fn snapshot_blocking() -> Result<serde_json::Value, String> {
    call("GET", "/v1/snapshot", None)
}

/// The scripted force-push (the C1 row's cue) is dev-only (CD ruling c, UX 2).
/// It is never widened to the `demo-daemon` feature (#36, SD).
pub(crate) const FORCED_DEMO_ALLOWED: bool = cfg!(debug_assertions);

const FORCED_REFUSED: &str = "the scripted force-push exists only in dev builds";

/// Refuse `forced: true` unless the gate allows it, before any HTTP call.
pub(crate) fn check_forced(forced: bool, allowed: bool) -> Result<(), String> {
    if forced && !allowed {
        return Err(FORCED_REFUSED.into());
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn daemon_emit_demo(forced: bool) -> Result<(), String> {
    check_forced(forced, FORCED_DEMO_ALLOWED)?;
    off_main(move || emit_blocking(forced)).await
}

fn emit_blocking(forced: bool) -> Result<(), String> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let body = serde_json::json!({
        "source": "demo",
        "kind": demo_kind(forced),
        "payload": {
            "repo": "DasVR/NIL",
            "ref": "phase0",
            "subject": "simulated push",
            "note": "phase 0 attaches no diff",
        },
        "idempotency_key": format!("ui-{nonce}"),
    });
    call("POST", "/v1/events", Some(body))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use dasdevbotd::http_proof::{self, PROOF_HEADER};

    /// A loopback HTTP server for the M2 tests. `prove` decides whether it
    /// answers with a valid proof (this app's daemon) or none (a squatter).
    /// It records every request it receives.
    fn fake_daemon(
        token: Option<&'static str>,
        status: u16,
        body: &'static str,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = std::sync::Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut head = String::new();
                let mut length = 0usize;
                let mut challenge = String::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    let lower = line.to_ascii_lowercase();
                    if let Some(value) = lower.strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                    if let Some(value) = lower.strip_prefix("x-dasdevbot-challenge:") {
                        challenge = value.trim().to_string();
                    }
                    head.push_str(&line);
                }
                let mut sent = vec![0u8; length];
                let _ = std::io::Read::read_exact(&mut reader, &mut sent);
                head.push_str(&String::from_utf8_lossy(&sent));
                let line = head.lines().next().unwrap_or("").to_string();
                let mut parts = line.split(' ');
                let request_line = format!(
                    "{} {}",
                    parts.next().unwrap_or(""),
                    parts.next().unwrap_or("").split('?').next().unwrap_or("")
                );
                log.lock().unwrap().push(head);
                let proof = token
                    .map(|token| {
                        format!(
                            "{PROOF_HEADER}: {}\r\n",
                            http_proof::proof(
                                token,
                                &challenge,
                                &request_line,
                                status,
                                body.as_bytes()
                            )
                        )
                    })
                    .unwrap_or_default();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{proof}Connection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (base, seen)
    }

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn a_squatter_without_the_proof_never_receives_the_bearer() {
        let (base, seen) = fake_daemon(None, 200, r#"{"approvals":[]}"#);
        let refused = super::call_at(
            &base,
            TOKEN,
            "POST",
            "/v1/events",
            Some(serde_json::json!({})),
        );
        assert_eq!(refused, Err(super::UNPROVEN.to_string()));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "only the no-bearer handshake was sent");
        assert!(seen[0].starts_with("GET /v1/health "));
        assert!(!seen[0].contains(TOKEN));
        assert!(!seen[0].to_ascii_lowercase().contains("authorization"));
    }

    /// SD on #46: the child is checked again after the probe verifies. If it
    /// has stopped by then, the bearer request never goes out.
    #[test]
    fn a_child_that_stops_after_the_probe_never_gets_the_bearer_request() {
        let (base, seen) = fake_daemon(Some(TOKEN), 200, "{}");
        let checks = std::sync::atomic::AtomicUsize::new(0);
        let stopped = || {
            checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err("The bundled daemon stopped (exit code: 1).".to_string())
        };
        assert_eq!(
            super::call_checked(&base, TOKEN, "GET", "/v1/snapshot", None, &stopped),
            Err("The bundled daemon stopped (exit code: 1).".to_string())
        );
        assert_eq!(checks.load(std::sync::atomic::Ordering::SeqCst), 1);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "only the probe went out");
        assert!(seen[0].starts_with("GET /v1/health "));
        assert!(!seen[0].contains(TOKEN));

        // And the real gate: a failed supervisor refuses at the re-check too.
        let failed = crate::daemon_child::Supervisor::from_spawn(Err("gone".into()));
        let (base, seen) = fake_daemon(Some(TOKEN), 200, "{}");
        let recheck = || failed.check();
        assert!(super::call_checked(&base, TOKEN, "GET", "/v1/snapshot", None, &recheck).is_err());
        assert!(!seen
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.contains(TOKEN)));
    }

    #[test]
    fn a_proof_made_with_another_token_is_refused() {
        let (base, seen) = fake_daemon(Some("ffffffffffffffffffffffffffffffff"), 200, "{}");
        assert_eq!(
            super::call_at(&base, TOKEN, "GET", "/v1/snapshot", None),
            Err(super::UNPROVEN.to_string())
        );
        assert!(!seen
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.contains(TOKEN)));
    }

    #[test]
    fn this_apps_daemon_gets_the_bearer_on_every_call_including_the_snapshot() {
        let (base, seen) = fake_daemon(Some(TOKEN), 200, r#"{"approvals":[]}"#);
        let snapshot = super::call_at(&base, TOKEN, "GET", "/v1/snapshot", None).unwrap();
        assert_eq!(snapshot["approvals"], serde_json::json!([]));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen[0].starts_with("GET /v1/health "));
        assert!(!seen[0].contains(TOKEN), "the handshake carries no bearer");
        assert!(seen[1].starts_with("GET /v1/snapshot "));
        assert!(seen[1].contains(&format!("Bearer {TOKEN}")));
    }

    #[test]
    fn a_proven_error_is_shown_and_an_unproven_body_is_never_parsed() {
        let (base, _) = fake_daemon(Some(TOKEN), 403, r#"{"error":"rejected origin"}"#);
        assert_eq!(
            super::call_at(
                &base,
                TOKEN,
                "POST",
                "/v1/events",
                Some(serde_json::json!({}))
            ),
            Err("rejected origin".to_string())
        );
        let answer = super::Answer {
            status: 200,
            proof: Some("0".repeat(64)),
            body: br#"{"approvals":[{"id":"fake"}]}"#.to_vec(),
        };
        assert_eq!(
            super::trusted(
                TOKEN,
                &http_proof::new_challenge(),
                "GET /v1/snapshot",
                answer
            ),
            Err(super::UNPROVEN.to_string())
        );
    }

    #[test]
    fn nothing_listening_reads_as_not_running() {
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", free.local_addr().unwrap());
        drop(free);
        assert_eq!(
            super::call_at(&base, TOKEN, "GET", "/v1/snapshot", None),
            Err(super::NOT_RUNNING.to_string())
        );
    }

    /// End to end against the real daemon (mock provider, executor role).
    #[test]
    fn the_real_daemon_proves_itself_and_decisions_stay_403() {
        let dir = std::env::temp_dir().join(format!("dasdevbot-shell-m2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let app = dasdevbotd::build_and_worker(
            dasdevbotd::Config {
                data: dir.join("db.sqlite"),
                web_root: None,
                role: "executor".into(),
                token: Some(TOKEN.into()),
            },
            Box::new(dasdevbotd::MockProvider::new()),
        )
        .unwrap();
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = free.local_addr().unwrap();
        drop(free);
        let bind = addr.to_string();
        std::thread::spawn(move || dasdevbotd::serve(app, &bind));
        let base = format!("http://{addr}");
        let start = std::time::Instant::now();
        let snapshot = loop {
            match super::call_at(&base, TOKEN, "GET", "/v1/snapshot", None) {
                Ok(snapshot) => break snapshot,
                Err(err)
                    if err == super::NOT_RUNNING
                        && start.elapsed() < std::time::Duration::from_secs(10) =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(err) => panic!("{err}"),
            }
        };
        assert!(snapshot["approvals"].is_array());
        let emitted = super::call_at(
            &base,
            TOKEN,
            "POST",
            "/v1/events",
            Some(serde_json::json!({
                "source": "demo",
                "kind": "session.note",
                "payload": {"note": "m2"},
                "idempotency_key": "m2-e2e",
            })),
        )
        .unwrap();
        assert_eq!(emitted["created"], true);
        assert_eq!(
            super::call_at(
                &base,
                TOKEN,
                "POST",
                "/v1/approvals/ap_x/decision",
                Some(serde_json::json!({}))
            ),
            Err("approval decisions are Tauri IPC only".to_string())
        );
        assert_eq!(
            super::call_at(
                &base,
                TOKEN,
                "POST",
                "/v1/approvals/ap_x/undo",
                Some(serde_json::json!({}))
            ),
            Err("approval decisions are Tauri IPC only".to_string())
        );
        assert_eq!(
            super::call_at(
                &base,
                "ffffffffffffffffffffffffffffffff",
                "GET",
                "/v1/snapshot",
                None
            ),
            Err(super::UNPROVEN.to_string())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_scripted_force_push_stays_dev_only() {
        assert_eq!(super::FORCED_DEMO_ALLOWED, cfg!(debug_assertions));
        let source = include_str!("daemon_http.rs");
        let gate = [
            "pub(crate) const FORCED_DEMO_ALLOWED: bool = ",
            "cfg!(debug_assertions);",
        ]
        .concat();
        assert!(source.contains(&gate));
    }

    #[test]
    fn the_daemon_address_is_loopback_and_the_kinds_are_mains() {
        assert!(super::DAEMON.starts_with("http://127.0.0.1:"));
        assert_eq!(super::demo_kind(false), "repo.push");
        assert_eq!(super::demo_kind(true), "repo.force_push");
    }

    #[test]
    fn the_forced_push_is_refused_unless_the_gate_allows_it() {
        assert_eq!(super::check_forced(false, false), Ok(()));
        assert_eq!(super::check_forced(true, true), Ok(()));
        assert_eq!(
            super::check_forced(true, false),
            Err(super::FORCED_REFUSED.to_string())
        );
    }

    /// `cargo test --release`: a release build refuses before any HTTP call.
    #[cfg(not(debug_assertions))]
    #[test]
    fn a_release_build_refuses_the_scripted_force_push() {
        let refused = tauri::async_runtime::block_on(super::daemon_emit_demo(true));
        assert_eq!(refused, Err(super::FORCED_REFUSED.to_string()));
    }
}
