use std::collections::HashSet;
use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::Arc;

use dasdevbot_proto::{
    AgentView, ApprovalView, DecisionRequest, DecisionResponse, EmitRequest, EmitResponse,
    ErrorBody, EventView, EvidenceView, Health, LedgerView, Snapshot, UndoResponse,
    PROTOCOL_VERSION,
};
use dasdevbot_sync::TRANSPORT;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::turn;
use crate::{wall_ms, App, Error, Result};

pub fn serve(app: Arc<App>, bind: &str) -> Result<()> {
    let local = dasdevbot_sync::bind_local().map_err(Error::Bind)?;
    if let Some(id) = local.node_id.clone() {
        *app.endpoint_id.lock().expect("endpoint") = Some(id);
    }
    let server = Server::http(bind).map_err(|err| Error::Bind(err.to_string()))?;
    let endpoint = local.node_id.as_deref().unwrap_or("-");
    eprintln!(
        "dasdevbotd ready bind={} provider={} sync={} endpoint={}",
        server.server_addr(),
        app.provider.id(),
        TRANSPORT,
        endpoint
    );
    let _ = std::io::Write::flush(&mut std::io::stderr());
    serve_incoming(app, server);
    Ok(())
}

pub fn serve_incoming(app: Arc<App>, server: Server) {
    let port = bound_port(&server);
    for mut request in server.incoming_requests() {
        let response = handle(&app, &mut request, port);
        let _ = request.respond(response);
    }
}

pub fn handle(app: &App, request: &mut Request, port: u16) -> Response<Cursor<Vec<u8>>> {
    match dispatch(app, request, port) {
        Ok(response) => response,
        Err(err) => {
            let code = match &err {
                Error::NotFound(_) => 404,
                Error::BadRequest(_) | Error::IdempotencyConflict(_) => 400,
                Error::Unauthorized => 401,
                Error::Forbidden(_) => 403,
                _ => 500,
            };
            json_response(
                code,
                &ErrorBody {
                    error: err.to_string(),
                },
            )
        }
    }
}

fn dispatch(app: &App, request: &mut Request, port: u16) -> Result<Response<Cursor<Vec<u8>>>> {
    if url_exposes_bearer(request.url(), &app.token) {
        return Err(Error::Forbidden(
            "bearer token must be sent only in the Authorization header".into(),
        ));
    }
    if !host_is_loopback(request, port) {
        return Err(Error::Forbidden("rejected host".into()));
    }
    if is_mutating(request.method()) {
        if !origin_is_allowed(request, port) {
            return Err(Error::Forbidden("rejected origin".into()));
        }
        if !bearer_matches(request, &app.token) {
            return Err(Error::Unauthorized);
        }
    }
    let path = request.url().split('?').next().unwrap_or("/").to_string();
    let matched = path.clone();
    match (request.method(), matched.as_str()) {
        (&Method::Get, "/v1/health") => Ok(json_response(200, &health(app))),
        (&Method::Get, "/v1/snapshot") => {
            let mut store = app.store.lock().expect("store");
            store.sweep(wall_ms())?;
            Ok(json_response(200, &snapshot(app, &store)?))
        }
        (&Method::Post, "/v1/events") => {
            let body = read_body(request)?;
            let req: EmitRequest = serde_json::from_str(&body)
                .map_err(|err| Error::BadRequest(format!("invalid JSON: {err}")))?;
            if req.source.trim().is_empty() || req.kind.trim().is_empty() {
                return Err(Error::BadRequest("source and kind are required".into()));
            }
            let ingested = {
                let mut store = app.store.lock().expect("store");
                turn::ingest(&mut store, &req, wall_ms())?
            };
            if ingested.created && !ingested.jobs.is_empty() {
                let _ = app.wake.send(());
            }
            Ok(json_response(
                200,
                &EmitResponse {
                    protocol: PROTOCOL_VERSION,
                    created: ingested.created,
                    event_id: ingested.event_id,
                    thread_id: ingested.thread_id,
                    jobs: ingested.jobs,
                },
            ))
        }
        (&Method::Post, approval_path)
            if approval_path.starts_with("/v1/approvals/")
                && approval_path.ends_with("/decision") =>
        {
            let id = approval_path
                .trim_start_matches("/v1/approvals/")
                .trim_end_matches("/decision")
                .trim_matches('/')
                .to_string();
            if id.is_empty() || id.contains('/') {
                return Err(Error::BadRequest("missing approval id".into()));
            }
            let body = read_body(request)?;
            let req: DecisionRequest = serde_json::from_str(&body)
                .map_err(|err| Error::BadRequest(format!("invalid JSON: {err}")))?;
            let recorded = {
                let mut store = app.store.lock().expect("store");
                store.decide_approval(&id, &req.decision, req.reason.as_deref(), wall_ms())?
            };
            Ok(json_response(
                200,
                &DecisionResponse {
                    protocol: PROTOCOL_VERSION,
                    approval_id: id,
                    status: recorded.status,
                    event_id: recorded.event_id,
                    executed: recorded.executed,
                    committed: recorded.committed,
                    undo_until: recorded.undo_until,
                },
            ))
        }
        (&Method::Post, approval_path)
            if approval_path.starts_with("/v1/approvals/") && approval_path.ends_with("/undo") =>
        {
            let id = approval_path
                .trim_start_matches("/v1/approvals/")
                .trim_end_matches("/undo")
                .trim_matches('/')
                .to_string();
            if id.is_empty() || id.contains('/') {
                return Err(Error::BadRequest("missing approval id".into()));
            }
            let _ = read_body(request)?;
            let recorded = {
                let mut store = app.store.lock().expect("store");
                store.undo_approval(&id, wall_ms())?
            };
            Ok(json_response(
                200,
                &UndoResponse {
                    protocol: PROTOCOL_VERSION,
                    approval_id: id,
                    status: recorded.status,
                    event_id: recorded.event_id,
                },
            ))
        }
        (&Method::Get, static_path) if !static_path.starts_with("/v1/") => {
            Ok(serve_static(app, static_path))
        }
        _ => Err(Error::NotFound(path)),
    }
}

fn health(app: &App) -> Health {
    let node = app.store.lock().expect("store").node_id().to_string();
    Health {
        protocol: PROTOCOL_VERSION,
        ok: true,
        ready: true,
        role: app.role.clone(),
        node,
        provider: app.provider.id().to_string(),
        provider_detail: app.provider.detail(),
        sync: TRANSPORT.to_string(),
        endpoint_id: app.endpoint_id.lock().expect("endpoint").clone(),
    }
}

fn snapshot(app: &App, store: &crate::store::Store) -> Result<Snapshot> {
    let agents = store
        .agents()?
        .into_iter()
        .map(|agent| {
            let status = store
                .agent_status(&agent.id)
                .unwrap_or_else(|_| "idle".into());
            AgentView {
                id: agent.id,
                name: agent.name,
                project: agent.project,
                persona: agent.persona,
                token_cap: agent.token_cap.max(0) as u64,
                tokens_spent: agent.tokens_spent.max(0) as u64,
                status,
            }
        })
        .collect();
    let approval_rows = store.approvals()?;
    let events = referenced_events(store, &approval_rows)?
        .into_iter()
        .map(event_view)
        .collect();
    let approvals = approval_rows
        .into_iter()
        .map(|row| {
            let evidence = evidence_view(&row);
            let pending = row.status == "pending";
            let committed = row.committed != 0;
            ApprovalView {
                id: row.id,
                job_id: row.job_id,
                agent_id: row.agent_id,
                agent_name: row.agent_name,
                thread_id: row.thread_id,
                effect_class: row.effect_class,
                action: row.action,
                purpose: row.purpose,
                draft: row.draft,
                evidence,
                evidence_text: row.evidence,
                status: row.status,
                provider: row.provider,
                model: row.model,
                usage_kind: row.usage_kind,
                input_tokens: row.input_tokens.max(0) as u64,
                output_tokens: row.output_tokens.max(0) as u64,
                micro_usd: row.micro_usd,
                created_at: row.created_at.max(0) as u64,
                expires_at: optional_ms(row.expires_at, pending),
                decided_at: optional_ms(row.decided_at, true),
                decision_event_id: row.decision_event_id,
                reason: row.reason,
                committed,
                undo_until: if committed {
                    None
                } else {
                    optional_ms(row.commit_due_ms, true)
                },
            }
        })
        .collect();
    let ledger = store
        .ledger()?
        .into_iter()
        .map(|row| LedgerView {
            id: row.id,
            agent_id: row.agent_id,
            agent_name: row.agent_name,
            project: row.project,
            provider: row.provider,
            model: row.model,
            usage_kind: row.usage_kind,
            input_tokens: row.input_tokens.max(0) as u64,
            output_tokens: row.output_tokens.max(0) as u64,
            micro_usd: row.micro_usd,
            note: row.note,
        })
        .collect();
    Ok(Snapshot {
        protocol: PROTOCOL_VERSION,
        role: app.role.clone(),
        node: store.node_id().to_string(),
        provider: app.provider.id().to_string(),
        provider_detail: app.provider.detail(),
        sync: TRANSPORT.to_string(),
        endpoint_id: app.endpoint_id.lock().expect("endpoint").clone(),
        agents,
        approvals,
        ledger,
        events,
    })
}

fn event_view(event: crate::store::StoredEvent) -> EventView {
    EventView {
        id: event.id,
        version: event.version,
        hlc: event.hlc.to_string(),
        source: event.source,
        kind: event.kind,
        thread_id: event.thread_id,
        idempotency_key: event.idempotency_key,
    }
}

/// Recent events, plus any evidence, decision, or `approval.requested` row the
/// card names, so a displayed `ev_####` is always a row in the stream.
fn referenced_events(
    store: &crate::store::Store,
    approvals: &[crate::store::ApprovalRow],
) -> Result<Vec<crate::store::StoredEvent>> {
    let mut events = store.recent_events(30)?;
    let mut seen: HashSet<String> = events.iter().map(|event| event.id.clone()).collect();
    let mut extra = Vec::new();
    for row in approvals {
        remember(store, &mut seen, &mut extra, &row.evidence_event_id)?;
        if let Some(id) = row.decision_event_id.as_deref() {
            remember(store, &mut seen, &mut extra, id)?;
        }
        let key = format!("approval-requested:{}", row.id);
        if let Some(event) = store.event_by_key(&key)? {
            if seen.insert(event.id.clone()) {
                extra.push(event);
            }
        }
    }
    events.append(&mut extra);
    events.sort_by(|left, right| {
        right
            .hlc
            .cmp(&left.hlc)
            .then_with(|| right.id.cmp(&left.id))
    });
    Ok(events)
}

fn remember(
    store: &crate::store::Store,
    seen: &mut HashSet<String>,
    extra: &mut Vec<crate::store::StoredEvent>,
    id: &str,
) -> Result<()> {
    if id.is_empty() || !seen.insert(id.to_string()) {
        return Ok(());
    }
    if let Some(event) = store.event_by_id(id)? {
        extra.push(event);
    }
    Ok(())
}

fn evidence_view(row: &crate::store::ApprovalRow) -> EvidenceView {
    if !row.evidence_repo.is_empty()
        || !row.evidence_ref.is_empty()
        || !row.evidence_event_id.is_empty()
    {
        return EvidenceView {
            repo: row.evidence_repo.clone(),
            git_ref: row.evidence_ref.clone(),
            event_id: row.evidence_event_id.clone(),
            kind: row.evidence_kind.clone(),
        };
    }
    let mut repo = String::new();
    let mut git_ref = String::new();
    let mut event_id = String::new();
    for line in row.evidence.lines() {
        if let Some(rest) = line.strip_prefix("repo ") {
            repo = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("ref ") {
            git_ref = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("event ") {
            event_id = rest.trim().to_string();
        }
    }
    EvidenceView {
        repo,
        git_ref,
        event_id,
        kind: String::new(),
    }
}

fn optional_ms(value: Option<i64>, include: bool) -> Option<u64> {
    if !include {
        return None;
    }
    value.and_then(|ms| if ms >= 0 { Some(ms as u64) } else { None })
}

fn serve_static(app: &App, path: &str) -> Response<Cursor<Vec<u8>>> {
    let Some(root) = app.web_root.as_ref() else {
        return json_response(
            404,
            &ErrorBody {
                error: "no web root; build apps/desktop and pass --web, or use the API".into(),
            },
        );
    };
    let rel = path.trim_start_matches('/');
    let candidate = if rel.is_empty() {
        root.join("index.html")
    } else if rel.contains("..") {
        return json_response(
            400,
            &ErrorBody {
                error: "bad path".into(),
            },
        );
    } else {
        root.join(rel)
    };
    if candidate.is_file() {
        return file_response(app, &candidate);
    }
    if Path::new(rel).extension().is_some() {
        return json_response(
            404,
            &ErrorBody {
                error: "not found".into(),
            },
        );
    }
    let index = root.join("index.html");
    if index.is_file() {
        file_response(app, &index)
    } else {
        json_response(
            404,
            &ErrorBody {
                error: "not found".into(),
            },
        )
    }
}

fn file_response(app: &App, path: &Path) -> Response<Cursor<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => {
            let body = if path.extension().and_then(|ext| ext.to_str()) == Some("html") {
                inject_session_token(&bytes, &app.token)
            } else {
                bytes
            };
            with_headers(
                Response::from_data(body).with_status_code(StatusCode(200)),
                mime(path),
            )
        }
        Err(_) => json_response(
            404,
            &ErrorBody {
                error: "not found".into(),
            },
        ),
    }
}

fn mime(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

fn read_body(request: &mut Request) -> Result<String> {
    let mut body = String::new();
    request
        .as_reader()
        .take(1_048_576)
        .read_to_string(&mut body)?;
    Ok(body)
}

fn inject_session_token(bytes: &[u8], token: &str) -> Vec<u8> {
    let Ok(html) = std::str::from_utf8(bytes) else {
        return bytes.to_vec();
    };
    let meta = format!(
        "<meta name=\"dasdevbot-token\" content=\"{}\">",
        escape_html(token)
    );
    let injected = if let Some(start) = html.find("<meta name=\"dasdevbot-token\"") {
        match html[start..].find('>') {
            Some(end_rel) => {
                let end = start + end_rel + 1;
                let mut out = String::with_capacity(html.len() + meta.len());
                out.push_str(&html[..start]);
                out.push_str(&meta);
                out.push_str(&html[end..]);
                out
            }
            None => html.to_string(),
        }
    } else if let Some(at) = html.find("<head>") {
        let split = at + "<head>".len();
        let mut out = String::with_capacity(html.len() + meta.len());
        out.push_str(&html[..split]);
        out.push_str(&meta);
        out.push_str(&html[split..]);
        out
    } else {
        format!("{meta}{html}")
    };
    injected.into_bytes()
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn bound_port(server: &Server) -> u16 {
    server
        .server_addr()
        .to_string()
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse().ok())
        .unwrap_or(0)
}

fn is_mutating(method: &Method) -> bool {
    matches!(
        method,
        Method::Post | Method::Put | Method::Patch | Method::Delete
    )
}

/// DNS rebinding sends the attacker's hostname while the socket is loopback.
fn host_is_loopback(request: &Request, port: u16) -> bool {
    let ip = format!("127.0.0.1:{port}");
    let name = format!("localhost:{port}");
    let mut saw = false;
    for header in request.headers() {
        if !header.field.equiv("Host") {
            continue;
        }
        saw = true;
        let value = header.value.as_str().trim();
        if value != ip && value != name {
            return false;
        }
    }
    saw
}

/// A browser cross-origin POST carries `Origin`. Non-browser clients omit it.
fn origin_is_allowed(request: &Request, port: u16) -> bool {
    let ip = format!("http://127.0.0.1:{port}");
    let name = format!("http://localhost:{port}");
    for header in request.headers() {
        if !header.field.equiv("Origin") {
            continue;
        }
        let value = header.value.as_str().trim();
        if value != ip && value != name {
            return false;
        }
    }
    true
}

/// True when the request target carries a bearer in the path or query.
///
/// The error path must not echo `url` or `token`. Callers reject the request
/// and accept the secret only from the Authorization header.
pub fn url_exposes_bearer(url: &str, token: &str) -> bool {
    if !token.is_empty() {
        if url.contains(token) {
            return true;
        }
        let decoded = percent_decode(url);
        if decoded.contains(token) || percent_decode(&decoded).contains(token) {
            return true;
        }
    }
    query_names_a_credential(url)
}

fn query_names_a_credential(url: &str) -> bool {
    let Some((_, query)) = url.split_once('?') else {
        return false;
    };
    let query = query.split('#').next().unwrap_or(query);
    query.split('&').any(|pair| {
        let name = pair.split_once('=').map(|(name, _)| name).unwrap_or(pair);
        let name = percent_decode(&percent_decode(name));
        matches!(
            name.to_ascii_lowercase().as_str(),
            "token" | "access_token" | "bearer"
        )
    })
}

fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (from_hex(bytes[index + 1]), from_hex(bytes[index + 2])) {
                out.push((hi << 4) | lo);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn bearer_matches(request: &Request, token: &str) -> bool {
    for header in request.headers() {
        if !header.field.equiv("Authorization") {
            continue;
        }
        let Some(presented) = presented_bearer(header.value.as_str()) else {
            continue;
        };
        if constant_time_eq(presented, token) {
            return true;
        }
    }
    false
}

fn presented_bearer(value: &str) -> Option<&str> {
    let (scheme, rest) = value.trim().split_once(' ')?;
    if scheme.eq_ignore_ascii_case("bearer") {
        Some(rest.trim())
    } else {
        None
    }
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

fn json_response(code: u16, body: &impl serde::Serialize) -> Response<Cursor<Vec<u8>>> {
    let bytes = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    with_headers(
        Response::from_data(bytes).with_status_code(StatusCode(code)),
        "application/json",
    )
}

fn with_headers(
    response: Response<Cursor<Vec<u8>>>,
    content_type: &str,
) -> Response<Cursor<Vec<u8>>> {
    response
        .with_header(header("Content-Type", content_type))
        .with_header(header("Cache-Control", "no-store"))
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static header")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::MockProvider;
    use crate::turn::spawn_worker;
    use crate::Config;
    use serde_json::json;
    use std::time::{Duration, Instant};

    #[test]
    fn ipc_round_trip_approval_is_recorded_and_not_executed() {
        let dir = std::env::temp_dir().join(format!("dasdevbot-ipc-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let (app, rx) = crate::build_app(
            Config {
                data: dir.join("db.sqlite"),
                web_root: None,
                role: "server".into(),
                token: None,
            },
            Box::new(MockProvider::new()),
        )
        .unwrap();
        let token = app.token.clone();
        spawn_worker(Arc::clone(&app), rx);
        let server = Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_string();
        std::thread::spawn(move || serve_incoming(app, server));

        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(2))
            .build();
        let health_url = format!("http://{addr}/v1/health");
        let health = wait_ok(&agent, &health_url);
        assert_eq!(health["protocol"], 1);
        assert_eq!(health["ready"], true);
        assert_eq!(health["provider"], "mock");
        assert_eq!(health["sync"], dasdevbot_sync::TRANSPORT);
        assert!(health["endpoint_id"].is_null());

        let emitted = agent
            .post(&format!("http://{addr}/v1/events"))
            .set("Authorization", &format!("Bearer {token}"))
            .send_json(json!({
                "source": "demo",
                "kind": "repo.push",
                "payload": {"repo": "DasVR/NIL", "ref": "phase0", "subject": "simulated push"},
                "idempotency_key": "ipc-push-1"
            }))
            .unwrap()
            .into_json::<EmitResponse>()
            .unwrap();
        assert!(emitted.created);
        assert_eq!(emitted.jobs.len(), 1);

        let replay = agent
            .post(&format!("http://{addr}/v1/events"))
            .set("Authorization", &format!("Bearer {token}"))
            .send_json(json!({
                "source": "demo",
                "kind": "repo.push",
                "payload": {"repo": "DasVR/NIL", "ref": "phase0", "subject": "simulated push"},
                "idempotency_key": "ipc-push-1"
            }))
            .unwrap()
            .into_json::<EmitResponse>()
            .unwrap();
        assert!(!replay.created);
        assert_eq!(replay.event_id, emitted.event_id);

        let snap = wait_approval(&agent, &format!("http://{addr}/v1/snapshot"));
        assert_eq!(snap["provider"], "mock");
        let approval = &snap["approvals"][0];
        assert_eq!(approval["status"], "pending");
        assert_eq!(approval["effect_class"], "external");
        assert_eq!(approval["action"], "post_pr_comment");
        assert_eq!(approval["evidence"]["repo"], "DasVR/NIL");
        assert_eq!(approval["evidence"]["ref"], "phase0");
        assert_eq!(approval["evidence"]["kind"], "repo.push");
        assert!(!approval["evidence"]["event_id"]
            .as_str()
            .unwrap()
            .is_empty());
        assert!(approval["evidence_text"]
            .as_str()
            .unwrap()
            .contains("DasVR/NIL"));
        assert_eq!(approval["provider"], "mock");
        assert!(approval["draft"].as_str().unwrap().contains("refresh()"));
        let evidence_id = approval["evidence"]["event_id"].as_str().unwrap();
        let approval_id = approval["id"].as_str().unwrap();
        let request_key = format!("approval-requested:{approval_id}");
        let events = snap["events"].as_array().unwrap();
        assert!(events.iter().any(|event| event["id"] == evidence_id));
        assert!(events
            .iter()
            .any(|event| event["idempotency_key"] == request_key));
        let agents = snap["agents"].as_array().unwrap();
        let reviewer = agents.iter().find(|a| a["id"] == "reviewer").unwrap();
        assert_eq!(reviewer["status"], "blocked");
        assert!(reviewer["tokens_spent"].as_u64().unwrap() > 0);
        assert!(!snap["ledger"].as_array().unwrap().is_empty());
        assert_eq!(snap["ledger"][0]["usage_kind"], "estimated");
        assert_eq!(snap["ledger"][0]["micro_usd"], 0);

        let decision = agent
            .post(&format!(
                "http://{addr}/v1/approvals/{approval_id}/decision"
            ))
            .set("Authorization", &format!("Bearer {token}"))
            .send_json(json!({"decision": "approve"}))
            .unwrap()
            .into_json::<DecisionResponse>()
            .unwrap();
        assert_eq!(decision.status, "approved");
        assert!(!decision.executed);
        assert!(!decision.committed);
        assert!(decision.undo_until.is_some());
        assert!(!decision.event_id.is_empty());

        let after: serde_json::Value = agent
            .get(&format!("http://{addr}/v1/snapshot"))
            .call()
            .unwrap()
            .into_json()
            .unwrap();
        assert_eq!(after["approvals"][0]["status"], "approved");
        assert_eq!(after["approvals"][0]["committed"], false);
        assert_eq!(
            after["agents"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["id"] == "reviewer")
                .unwrap()["status"],
            "blocked"
        );

        let undone = agent
            .post(&format!("http://{addr}/v1/approvals/{approval_id}/undo"))
            .set("Authorization", &format!("Bearer {token}"))
            .send_json(json!({}))
            .unwrap()
            .into_json::<dasdevbot_proto::UndoResponse>()
            .unwrap();
        assert_eq!(undone.status, "pending");

        let denied = agent
            .post(&format!(
                "http://{addr}/v1/approvals/{approval_id}/decision"
            ))
            .set("Authorization", &format!("Bearer {token}"))
            .send_json(json!({
                "decision": "deny",
                "reason": "Not worth a comment on a phase-0 branch"
            }))
            .unwrap()
            .into_json::<DecisionResponse>()
            .unwrap();
        assert_eq!(denied.status, "denied");
        assert!(!denied.executed);
        assert!(!denied.committed);

        let reasoned: serde_json::Value = agent
            .get(&format!("http://{addr}/v1/snapshot"))
            .call()
            .unwrap()
            .into_json()
            .unwrap();
        assert_eq!(reasoned["approvals"][0]["status"], "denied");
        assert_eq!(
            reasoned["approvals"][0]["reason"],
            "Not worth a comment on a phase-0 branch"
        );
        assert_eq!(
            reasoned["agents"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["id"] == "reviewer")
                .unwrap()["status"],
            "blocked"
        );
        let kinds: Vec<&str> = reasoned["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|event| event["kind"].as_str())
            .collect();
        assert!(kinds.contains(&"repo.push"));
        assert!(kinds.contains(&"approval.requested"));
        assert!(kinds.contains(&"approval.decided"));
        assert!(kinds.contains(&"approval.undone"));
        assert!(kinds.contains(&"ledger.posted"));
        assert!(!kinds.contains(&"approval.committed"));
    }

    #[test]
    fn forced_push_snapshot_effect_class_is_destructive() {
        let dir = std::env::temp_dir().join(format!("dasdevbot-forced-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let (app, rx) = crate::build_app(
            Config {
                data: dir.join("db.sqlite"),
                web_root: None,
                role: "server".into(),
                token: None,
            },
            Box::new(MockProvider::new()),
        )
        .unwrap();
        let token = app.token.clone();
        spawn_worker(Arc::clone(&app), rx);
        let server = Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_string();
        std::thread::spawn(move || serve_incoming(app, server));

        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(2))
            .build();
        let health = wait_ok(&agent, &format!("http://{addr}/v1/health"));
        assert_eq!(health["ready"], true);

        let emitted = agent
            .post(&format!("http://{addr}/v1/events"))
            .set("Authorization", &format!("Bearer {token}"))
            .send_json(json!({
                "source": "demo",
                "kind": "repo.push",
                "payload": {
                    "repo": "DasVR/NIL",
                    "ref": "phase0",
                    "subject": "simulated push",
                    "forced": true
                },
                "idempotency_key": "ipc-forced-1"
            }))
            .unwrap()
            .into_json::<EmitResponse>()
            .unwrap();
        assert!(emitted.created);
        assert_eq!(emitted.jobs.len(), 1);

        let snap = wait_approval(&agent, &format!("http://{addr}/v1/snapshot"));
        let approval = &snap["approvals"][0];
        assert_eq!(approval["effect_class"], "destructive");
        assert_eq!(approval["action"], "force_push");
        assert_eq!(approval["draft"], "git push --force origin phase0");
        assert_eq!(approval["status"], "pending");
    }

    #[test]
    fn cross_origin_post_to_the_decision_route_is_rejected() {
        let dir = std::env::temp_dir().join(format!("dasdevbot-cors-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let (app, rx) = crate::build_app(
            Config {
                data: dir.join("db.sqlite"),
                web_root: None,
                role: "server".into(),
                token: Some("desktop-only-token".into()),
            },
            Box::new(MockProvider::new()),
        )
        .unwrap();
        let token = app.token.clone();
        spawn_worker(Arc::clone(&app), rx);
        let server = Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_string();
        let port = addr.rsplit_once(':').unwrap().1;
        std::thread::spawn(move || serve_incoming(app, server));

        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(2))
            .build();
        let _ = wait_ok(&agent, &format!("http://{addr}/v1/health"));
        agent
            .post(&format!("http://{addr}/v1/events"))
            .set("Authorization", &format!("Bearer {token}"))
            .send_json(json!({
                "source": "demo",
                "kind": "repo.push",
                "payload": {"repo": "DasVR/NIL", "ref": "phase0", "subject": "simulated push"},
                "idempotency_key": "cors-push-1"
            }))
            .unwrap();
        let snap = wait_approval(&agent, &format!("http://{addr}/v1/snapshot"));
        let approval_id = snap["approvals"][0]["id"].as_str().unwrap();
        let decision_url = format!("http://{addr}/v1/approvals/{approval_id}/decision");

        let denied = agent
            .post(&decision_url)
            .set("Authorization", &format!("Bearer {token}"))
            .set("Origin", "https://evil.example")
            .send_json(json!({"decision": "approve"}))
            .unwrap_err();
        match denied {
            ureq::Error::Status(403, response) => {
                assert!(response.header("Access-Control-Allow-Origin").is_none());
                let body = response.into_string().unwrap();
                assert!(body.contains("rejected origin"), "{body}");
            }
            other => panic!("cross-origin decision was not forbidden: {other}"),
        }

        let rebound = agent
            .post(&decision_url)
            .set("Authorization", &format!("Bearer {token}"))
            .set("Host", &format!("evil.example:{port}"))
            .send_json(json!({"decision": "approve"}))
            .unwrap_err();
        match rebound {
            ureq::Error::Status(403, response) => {
                let body = response.into_string().unwrap();
                assert!(body.contains("rejected host"), "{body}");
            }
            other => panic!("rebinding decision was not forbidden: {other}"),
        }

        let missing = agent
            .post(&decision_url)
            .send_json(json!({"decision": "approve"}))
            .unwrap_err();
        match missing {
            ureq::Error::Status(401, _) => {}
            other => panic!("decision without a bearer was not rejected: {other}"),
        }

        let options = agent.request("OPTIONS", &decision_url).call();
        match options {
            Err(ureq::Error::Status(status, response)) => {
                assert_ne!(status, 204);
                assert!(response.header("Access-Control-Allow-Origin").is_none());
            }
            Ok(response) => panic!("preflight succeeded with {}", response.status()),
            Err(err) => panic!("preflight failed closed the wrong way: {err}"),
        }

        let still = agent
            .get(&format!("http://{addr}/v1/snapshot"))
            .call()
            .unwrap()
            .into_json::<serde_json::Value>()
            .unwrap();
        assert_eq!(still["approvals"][0]["status"], "pending");
        assert!(still["approvals"][0]["expires_at"].as_u64().is_some());
    }

    #[test]
    fn desktop_html_receives_the_token_and_health_does_not() {
        let dir = std::env::temp_dir().join(format!("dasdevbot-html-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("index.html"),
            "<!doctype html><head><meta name=\"dasdevbot-token\" content=\"\"></head><body>ok</body>",
        )
        .unwrap();
        let (app, _rx) = crate::build_app(
            Config {
                data: dir.join("db.sqlite"),
                web_root: Some(dir.clone()),
                role: "server".into(),
                token: Some("html-only-token".into()),
            },
            Box::new(MockProvider::new()),
        )
        .unwrap();
        let server = Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_string();
        std::thread::spawn(move || serve_incoming(app, server));
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(2))
            .build();
        let _ = wait_ok(&agent, &format!("http://{addr}/v1/health"));
        let page = agent
            .get(&format!("http://{addr}/"))
            .call()
            .unwrap()
            .into_string()
            .unwrap();
        assert!(page.contains("content=\"html-only-token\""));
        assert_eq!(page.matches("dasdevbot-token").count(), 1);
        let health = agent
            .get(&format!("http://{addr}/v1/health"))
            .call()
            .unwrap()
            .into_string()
            .unwrap();
        assert!(!health.contains("html-only-token"));
        assert!(agent
            .get(&format!("http://{addr}/v1/health"))
            .call()
            .unwrap()
            .header("Access-Control-Allow-Origin")
            .is_none());
    }

    #[test]
    fn bearer_in_the_query_string_is_rejected_and_not_echoed() {
        let dir = std::env::temp_dir().join(format!("dasdevbot-query-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let (app, rx) = crate::build_app(
            Config {
                data: dir.join("db.sqlite"),
                web_root: None,
                role: "server".into(),
                token: Some("desktop-only-token".into()),
            },
            Box::new(MockProvider::new()),
        )
        .unwrap();
        let token = app.token.clone();
        spawn_worker(Arc::clone(&app), rx);
        let server = Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_string();
        std::thread::spawn(move || serve_incoming(app, server));

        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(2))
            .build();
        let _ = wait_ok(&agent, &format!("http://{addr}/v1/health"));
        agent
            .post(&format!("http://{addr}/v1/events"))
            .set("Authorization", &format!("Bearer {token}"))
            .send_json(json!({
                "source": "demo",
                "kind": "repo.push",
                "payload": {"repo": "DasVR/NIL", "ref": "phase0", "subject": "simulated push"},
                "idempotency_key": "query-push-1"
            }))
            .unwrap();
        let snap = wait_approval(&agent, &format!("http://{addr}/v1/snapshot"));
        let approval_id = snap["approvals"][0]["id"].as_str().unwrap();

        let leaked = agent
            .post(&format!(
                "http://{addr}/v1/approvals/{approval_id}/decision?token={token}"
            ))
            .set("Authorization", &format!("Bearer {token}"))
            .send_json(json!({"decision": "approve"}))
            .unwrap_err();
        match leaked {
            ureq::Error::Status(403, response) => {
                let body = response.into_string().unwrap();
                assert!(body.contains("Authorization"), "{body}");
                assert!(!body.contains(&token), "response echoed the bearer");
            }
            other => panic!("query-string bearer was not forbidden: {other}"),
        }

        let named = agent
            .post(&format!(
                "http://{addr}/v1/approvals/{approval_id}/decision?access_token=not-the-secret"
            ))
            .set("Authorization", &format!("Bearer {token}"))
            .send_json(json!({"decision": "approve"}))
            .unwrap_err();
        match named {
            ureq::Error::Status(403, response) => {
                let body = response.into_string().unwrap();
                assert!(
                    !body.contains("not-the-secret"),
                    "response echoed the query"
                );
                assert!(!body.contains(&token), "response echoed the bearer");
            }
            other => panic!("access_token query was not forbidden: {other}"),
        }

        let still = agent
            .get(&format!("http://{addr}/v1/snapshot"))
            .call()
            .unwrap()
            .into_json::<serde_json::Value>()
            .unwrap();
        assert_eq!(still["approvals"][0]["status"], "pending");
    }

    #[test]
    fn url_credential_check_covers_path_query_and_percent_encoding() {
        let token = "desktop-only-token";
        assert!(!url_exposes_bearer("/v1/health", token));
        assert!(url_exposes_bearer(&format!("/v1/{token}/decision"), token));
        assert!(url_exposes_bearer("/v1/health?token=other", token));
        assert!(url_exposes_bearer("/v1/health?Access_Token=other", token));
        assert!(url_exposes_bearer("/v1/health?bearer", token));
        assert!(url_exposes_bearer(
            "/v1/health?token=%64esktop-only-token",
            token
        ));
        assert!(!url_exposes_bearer("/v1/health?note=phase0", token));
        assert!(!url_exposes_bearer("/v1/health", ""));
    }

    #[test]
    fn serve_binds_before_health() {
        let dir = std::env::temp_dir().join(format!("dasdevbot-bind-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let app = crate::build_and_worker(Config {
            data: dir.join("db.sqlite"),
            web_root: None,
            role: "server".into(),
            token: Some("bind-test-token".into()),
        })
        .unwrap();
        let bind = format!("127.0.0.1:{port}");
        std::thread::spawn(move || {
            serve(app, &bind).unwrap();
        });

        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(2))
            .build();
        let health = wait_ok(&agent, &format!("http://127.0.0.1:{port}/v1/health"));
        assert_eq!(health["sync"], dasdevbot_sync::TRANSPORT);
        if dasdevbot_sync::linked() {
            let id = health["endpoint_id"].as_str().expect("endpoint id");
            assert!(id.len() > 8, "{id}");
        } else {
            assert!(health["endpoint_id"].is_null());
        }
    }

    fn wait_ok(agent: &ureq::Agent, url: &str) -> serde_json::Value {
        let start = Instant::now();
        loop {
            if let Ok(response) = agent.get(url).call() {
                return response.into_json().unwrap();
            }
            if start.elapsed() > Duration::from_secs(20) {
                panic!("daemon did not answer {url}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_approval(agent: &ureq::Agent, url: &str) -> serde_json::Value {
        let start = Instant::now();
        loop {
            let snap: serde_json::Value = agent.get(url).call().unwrap().into_json().unwrap();
            if snap["approvals"]
                .as_array()
                .map(|rows| !rows.is_empty())
                .unwrap_or(false)
            {
                return snap;
            }
            if start.elapsed() > Duration::from_secs(5) {
                panic!("approval was not created: {snap}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
