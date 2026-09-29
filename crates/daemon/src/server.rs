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
    let server = Server::http(bind).map_err(|err| Error::Bind(err.to_string()))?;
    eprintln!(
        "dasdevbotd ready bind={} provider={} sync={}",
        server.server_addr(),
        app.provider.id(),
        TRANSPORT
    );
    let _ = std::io::Write::flush(&mut std::io::stderr());
    serve_incoming(app, server);
    Ok(())
}

pub fn serve_incoming(app: Arc<App>, server: Server) {
    for mut request in server.incoming_requests() {
        let response = handle(&app, &mut request);
        let _ = request.respond(response);
    }
}

pub fn handle(app: &App, request: &mut Request) -> Response<Cursor<Vec<u8>>> {
    match dispatch(app, request) {
        Ok(response) => response,
        Err(err) => {
            let code = match &err {
                Error::NotFound(_) => 404,
                Error::BadRequest(_) | Error::IdempotencyConflict(_) => 400,
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

fn dispatch(app: &App, request: &mut Request) -> Result<Response<Cursor<Vec<u8>>>> {
    if request.method() == &Method::Options {
        return Ok(empty(204));
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
    let approvals = store
        .approvals()?
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
    let events = store
        .recent_events(30)?
        .into_iter()
        .map(|event| EventView {
            id: event.id,
            version: event.version,
            hlc: event.hlc.to_string(),
            source: event.source,
            kind: event.kind,
            thread_id: event.thread_id,
            idempotency_key: event.idempotency_key,
        })
        .collect();
    Ok(Snapshot {
        protocol: PROTOCOL_VERSION,
        role: app.role.clone(),
        node: store.node_id().to_string(),
        provider: app.provider.id().to_string(),
        provider_detail: app.provider.detail(),
        sync: TRANSPORT.to_string(),
        agents,
        approvals,
        ledger,
        events,
    })
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
        return file_response(&candidate);
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
        file_response(&index)
    } else {
        json_response(
            404,
            &ErrorBody {
                error: "not found".into(),
            },
        )
    }
}

fn file_response(path: &Path) -> Response<Cursor<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => with_headers(
            Response::from_data(bytes).with_status_code(StatusCode(200)),
            mime(path),
        ),
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

fn json_response(code: u16, body: &impl serde::Serialize) -> Response<Cursor<Vec<u8>>> {
    let bytes = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    with_headers(
        Response::from_data(bytes).with_status_code(StatusCode(code)),
        "application/json",
    )
}

fn empty(code: u16) -> Response<Cursor<Vec<u8>>> {
    with_headers(
        Response::from_data(Vec::new()).with_status_code(StatusCode(code)),
        "text/plain",
    )
}

fn with_headers(
    response: Response<Cursor<Vec<u8>>>,
    content_type: &str,
) -> Response<Cursor<Vec<u8>>> {
    response
        .with_header(header("Content-Type", content_type))
        .with_header(header("Access-Control-Allow-Origin", "*"))
        .with_header(header("Access-Control-Allow-Methods", "GET, POST, OPTIONS"))
        .with_header(header("Access-Control-Allow-Headers", "Content-Type"))
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
            },
            Box::new(MockProvider::new()),
        )
        .unwrap();
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
        assert_eq!(health["sync"], "stub");

        let emitted = agent
            .post(&format!("http://{addr}/v1/events"))
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
        let agents = snap["agents"].as_array().unwrap();
        let reviewer = agents.iter().find(|a| a["id"] == "reviewer").unwrap();
        assert_eq!(reviewer["status"], "blocked");
        assert!(reviewer["tokens_spent"].as_u64().unwrap() > 0);
        assert!(!snap["ledger"].as_array().unwrap().is_empty());
        assert_eq!(snap["ledger"][0]["usage_kind"], "estimated");
        assert_eq!(snap["ledger"][0]["micro_usd"], 0);

        let approval_id = approval["id"].as_str().unwrap();
        let decision = agent
            .post(&format!(
                "http://{addr}/v1/approvals/{approval_id}/decision"
            ))
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
            .send_json(json!({}))
            .unwrap()
            .into_json::<dasdevbot_proto::UndoResponse>()
            .unwrap();
        assert_eq!(undone.status, "pending");

        let denied = agent
            .post(&format!(
                "http://{addr}/v1/approvals/{approval_id}/decision"
            ))
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

    fn wait_ok(agent: &ureq::Agent, url: &str) -> serde_json::Value {
        let start = Instant::now();
        loop {
            if let Ok(response) = agent.get(url).call() {
                return response.into_json().unwrap();
            }
            if start.elapsed() > Duration::from_secs(5) {
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
