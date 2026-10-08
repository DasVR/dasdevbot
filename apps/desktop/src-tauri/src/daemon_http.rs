//! Read the snapshot and send the demo event for the webview.
//!
//! A bundled webview runs on the Tauri origin, and the daemon sends no CORS
//! headers, so the page cannot fetch `/v1` itself. These two commands make the
//! loopback HTTP call from the shell process instead. The address is fixed to
//! 127.0.0.1:8787. The bearer is read here and never handed to the page for
//! this path. Decisions do not go through HTTP: `/decision` and `/undo` stay 403
//! and the card window signs over the socket.

use std::time::Duration;

const DAEMON: &str = "http://127.0.0.1:8787";

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(Duration::from_millis(500))
        .timeout(Duration::from_secs(5))
        .build()
}

fn http_error(err: ureq::Error) -> String {
    match err {
        ureq::Error::Status(code, response) => {
            let body = response.into_string().unwrap_or_default();
            let message = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|value| value["error"].as_str().map(str::to_string))
                .unwrap_or_else(|| format!("daemon answered {code}"));
            message
        }
        ureq::Error::Transport(_) => "the daemon is not running on 127.0.0.1:8787".into(),
    }
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
    let response = agent()
        .get(&format!("{DAEMON}/v1/snapshot"))
        .call()
        .map_err(http_error)?;
    response.into_json().map_err(|err| err.to_string())
}

/// The scripted force-push (the C1 row's cue) is dev-only for now (CD ruling
/// c, UX 2). This is the one gate: widen it here, e.g. to
/// `cfg!(any(debug_assertions, feature = "demo-daemon"))` for the demo NSIS.
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
    let token = crate::read_session_token()?;
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
    agent()
        .post(&format!("{DAEMON}/v1/events"))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(body)
        .map_err(http_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
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
