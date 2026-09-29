use std::path::Path;

use dasdevbot_core::{HlcTimestamp, HybridClock, TokenBudget, EVENT_VERSION};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::{Error, Result};

const REVIEWER_PERSONA: &str = "\
You are Reviewer, a teammate for the DasVR/NIL repository.
When woken, draft a short review of the push in the user message.
Three to six sentences: what you were told, one risk, and a recommendation.
You cannot post the review yourself. A human approves any external effect.
Do not invent a diff you were not given.
";

pub struct Store {
    conn: Connection,
    clock: HybridClock,
    node_id: String,
}

#[derive(Debug, Clone)]
pub struct StoredEvent {
    pub id: String,
    pub version: u32,
    pub hlc: HlcTimestamp,
    pub source: String,
    pub kind: String,
    pub payload: String,
    pub idempotency_key: String,
    pub thread_id: String,
}

#[derive(Debug, Clone)]
pub enum AppendOutcome {
    Created(StoredEvent),
    Replay(StoredEvent),
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: String,
    pub agent_id: String,
    pub status: String,
    pub lease_owner: Option<String>,
    pub lease_until_ms: Option<i64>,
    pub idempotency_key: String,
    pub payload: String,
    pub attempt: i64,
}

#[derive(Debug, Clone)]
pub struct AgentRow {
    pub id: String,
    pub name: String,
    pub persona: String,
    pub project: String,
    pub token_cap: i64,
    pub tokens_spent: i64,
}

#[derive(Debug, Clone)]
pub struct ApprovalRow {
    pub id: String,
    pub job_id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub thread_id: String,
    pub effect_class: String,
    pub action: String,
    pub purpose: String,
    pub draft: String,
    pub evidence: String,
    pub status: String,
    pub provider: String,
    pub model: String,
    pub usage_kind: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub micro_usd: i64,
}

#[derive(Debug, Clone)]
pub struct LedgerRow {
    pub id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub project: String,
    pub provider: String,
    pub model: String,
    pub usage_kind: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub micro_usd: i64,
    pub note: String,
}

pub struct NewApproval {
    pub job_id: String,
    pub agent_id: String,
    pub thread_id: String,
    pub effect_class: String,
    pub action: String,
    pub purpose: String,
    pub draft: String,
    pub evidence: String,
    pub provider: String,
    pub model: String,
    pub usage_kind: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub micro_usd: i64,
    pub ledger_note: String,
    pub project: String,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(path)?;
        Self::from_conn(conn)
    }

    fn from_conn(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(3))?;
        // In-memory connections used by tests reject WAL. File databases use it.
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        migrate(&conn)?;
        let node_id = ensure_node_id(&conn)?;
        seed(&conn)?;
        let (millis, counter) = latest_hlc(&conn)?;
        let clock = HybridClock::resume(node_id.clone(), millis, counter);
        Ok(Self {
            conn,
            clock,
            node_id,
        })
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    #[cfg(test)]
    pub(crate) fn open_memory() -> Result<Self> {
        Self::from_conn(Connection::open_in_memory()?)
    }

    pub fn append_at(
        &mut self,
        wall_ms: u64,
        source: &str,
        kind: &str,
        payload: &str,
        idempotency_key: &str,
        thread_id: Option<&str>,
    ) -> Result<AppendOutcome> {
        if let Some(existing) = self.event_by_key(idempotency_key)? {
            if existing.payload == payload {
                return Ok(AppendOutcome::Replay(existing));
            }
            return Err(Error::IdempotencyConflict(idempotency_key.to_string()));
        }
        let hlc = self.clock.tick(wall_ms);
        let id = Uuid::new_v4().to_string();
        let thread = thread_id.unwrap_or(&id).to_string();
        self.conn.execute(
            "INSERT INTO events (
                id, version, hlc_millis, hlc_counter, hlc_node, source, kind,
                payload, idempotency_key, thread_id, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                id,
                EVENT_VERSION,
                hlc.millis as i64,
                hlc.counter,
                hlc.node,
                source,
                kind,
                payload,
                idempotency_key,
                thread,
                wall_ms as i64,
            ],
        )?;
        Ok(AppendOutcome::Created(StoredEvent {
            id,
            version: EVENT_VERSION,
            hlc,
            source: source.to_string(),
            kind: kind.to_string(),
            payload: payload.to_string(),
            idempotency_key: idempotency_key.to_string(),
            thread_id: thread,
        }))
    }

    pub fn event_by_key(&self, key: &str) -> Result<Option<StoredEvent>> {
        self.conn
            .query_row(
                "SELECT id, version, hlc_millis, hlc_counter, hlc_node, source, kind,
                        payload, idempotency_key, thread_id
                 FROM events WHERE idempotency_key = ?1",
                [key],
                map_event,
            )
            .optional()
            .map_err(Error::from)
    }

    pub fn event_count(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .map_err(Error::from)
    }

    pub fn recent_events(&self, limit: i64) -> Result<Vec<StoredEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, version, hlc_millis, hlc_counter, hlc_node, source, kind,
                    payload, idempotency_key, thread_id
             FROM events
             ORDER BY hlc_millis DESC, hlc_counter DESC, id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], map_event)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn enqueue_job(
        &self,
        agent_id: &str,
        idempotency_key: &str,
        payload: &str,
        now_ms: u64,
    ) -> Result<Option<String>> {
        let id = Uuid::new_v4().to_string();
        match self.conn.execute(
            "INSERT INTO jobs (id, agent_id, status, idempotency_key, payload, attempt, created_at)
             VALUES (?1, ?2, 'pending', ?3, ?4, 0, ?5)",
            params![id, agent_id, idempotency_key, payload, now_ms as i64],
        ) {
            Ok(_) => Ok(Some(id)),
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Ok(None)
            }
            Err(err) => Err(err.into()),
        }
    }

    pub fn job_ids_for_event_key(&self, event_key: &str) -> Result<Vec<String>> {
        let like = format!("job:{event_key}:%");
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM jobs WHERE idempotency_key = ?1 OR idempotency_key LIKE ?2 ORDER BY created_at")?;
        let rows = stmt.query_map(params![format!("job:{event_key}"), like], |row| row.get(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn job_count(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
            .map_err(Error::from)
    }

    pub fn job_count_for_agent(&self, agent_id: &str) -> Result<i64> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE agent_id = ?1",
                [agent_id],
                |row| row.get(0),
            )
            .map_err(Error::from)
    }

    /// Claim the oldest pending job, or a leased job whose lease has expired.
    pub fn claim_at(&self, owner: &str, now_ms: u64, lease_for_ms: u64) -> Result<Option<Job>> {
        let until = now_ms.saturating_add(lease_for_ms) as i64;
        match self.conn.query_row(
            "UPDATE jobs
             SET status = 'leased',
                 lease_owner = ?1,
                 lease_until_ms = ?2,
                 attempt = attempt + 1
             WHERE id = (
                 SELECT id FROM jobs
                 WHERE status = 'pending'
                    OR (status = 'leased' AND lease_until_ms <= ?3)
                 ORDER BY created_at ASC, id ASC
                 LIMIT 1
             )
             RETURNING id, agent_id, status, lease_owner, lease_until_ms, idempotency_key, payload, attempt",
            params![owner, until, now_ms as i64],
            map_job,
        ) {
            Ok(job) => Ok(Some(job)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub fn heartbeat_at(
        &self,
        job_id: &str,
        owner: &str,
        now_ms: u64,
        lease_for_ms: u64,
    ) -> Result<bool> {
        let until = now_ms.saturating_add(lease_for_ms) as i64;
        let changed = self.conn.execute(
            "UPDATE jobs SET lease_until_ms = ?1
             WHERE id = ?2 AND lease_owner = ?3 AND status = 'leased'",
            params![until, job_id, owner],
        )?;
        Ok(changed == 1)
    }

    pub fn complete(&self, job_id: &str, owner: &str) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE jobs
             SET status = 'done', lease_owner = NULL, lease_until_ms = NULL
             WHERE id = ?1 AND lease_owner = ?2 AND status = 'leased'",
            params![job_id, owner],
        )?;
        Ok(changed == 1)
    }

    pub fn job_status(&self, job_id: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT status FROM jobs WHERE id = ?1", [job_id], |row| {
                row.get(0)
            })
            .optional()
            .map_err(Error::from)
    }

    pub fn lease_until(&self, job_id: &str) -> Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT lease_until_ms FROM jobs WHERE id = ?1",
                [job_id],
                |row| row.get(0),
            )
            .map_err(Error::from)
    }

    pub fn agent(&self, id: &str) -> Result<AgentRow> {
        self.conn
            .query_row(
                "SELECT id, name, persona, project, token_cap, tokens_spent FROM agents WHERE id = ?1",
                [id],
                map_agent,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("agent {id}")))
    }

    pub fn agents(&self) -> Result<Vec<AgentRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, persona, project, token_cap, tokens_spent FROM agents ORDER BY name",
        )?;
        let rows = stmt.query_map([], map_agent)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn insert_agent(
        &self,
        id: &str,
        name: &str,
        persona: &str,
        project: &str,
        token_cap: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO agents (id, name, persona, project, token_cap, tokens_spent)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            params![id, name, persona, project, token_cap],
        )?;
        Ok(())
    }

    pub fn agent_status(&self, agent_id: &str) -> Result<String> {
        let leased: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM jobs WHERE agent_id = ?1 AND status = 'leased'",
            [agent_id],
            |row| row.get(0),
        )?;
        if leased > 0 {
            return Ok("working".into());
        }
        let waiting: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM jobs WHERE agent_id = ?1 AND status = 'waiting_approval'",
            [agent_id],
            |row| row.get(0),
        )?;
        if waiting > 0 {
            Ok("blocked".into())
        } else {
            Ok("idle".into())
        }
    }

    pub fn has_grant(&self, agent_id: &str, effect_class: &str) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM grants WHERE agent_id = ?1 AND effect_class = ?2",
            params![agent_id, effect_class],
            |row| row.get(0),
        )?;
        Ok(n > 0)
    }

    pub fn rules_for_kind(&self, kind: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT agent_id FROM rules WHERE kind = ?1 ORDER BY id")?;
        let rows = stmt.query_map([kind], |row| row.get(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn budget_of(&self, agent_id: &str) -> Result<TokenBudget> {
        let agent = self.agent(agent_id)?;
        Ok(TokenBudget {
            cap: agent.token_cap.max(0) as u64,
            spent: agent.tokens_spent.max(0) as u64,
        })
    }

    pub fn add_spend(&self, agent_id: &str, tokens: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE agents SET tokens_spent = tokens_spent + ?1 WHERE id = ?2",
            params![tokens, agent_id],
        )?;
        Ok(())
    }

    pub fn fail_leased(&self, job_id: &str, owner: &str) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE jobs
             SET status = 'failed', lease_owner = NULL, lease_until_ms = NULL
             WHERE id = ?1 AND lease_owner = ?2 AND status = 'leased'",
            params![job_id, owner],
        )?;
        Ok(changed == 1)
    }

    pub fn record_approval_and_wait(
        &mut self,
        wall_ms: u64,
        owner: &str,
        approval: NewApproval,
        request_payload: &str,
        ledger_payload: &str,
    ) -> Result<String> {
        let request_hlc = self.clock.tick(wall_ms);
        let ledger_hlc = self.clock.tick(wall_ms);
        let approval_id = Uuid::new_v4().to_string();
        let request_event = Uuid::new_v4().to_string();
        let ledger_event = Uuid::new_v4().to_string();
        let ledger_id = Uuid::new_v4().to_string();
        let tx = self.conn.transaction()?;
        let owned: i64 = tx.query_row(
            "SELECT COUNT(*) FROM jobs WHERE id = ?1 AND lease_owner = ?2 AND status = 'leased'",
            params![approval.job_id, owner],
            |row| row.get(0),
        )?;
        if owned != 1 {
            return Err(Error::BadRequest(
                "job lease was lost before the approval was recorded".into(),
            ));
        }
        let request_key = format!("approval-requested:{approval_id}");
        let ledger_key = format!("ledger:{ledger_id}");
        insert_event(
            &tx,
            EventInsert {
                id: &request_event,
                hlc: &request_hlc,
                source: "runtime",
                kind: dasdevbot_core::kind::APPROVAL_REQUESTED,
                payload: request_payload,
                idempotency_key: &request_key,
                thread_id: &approval.thread_id,
                wall_ms,
            },
        )?;
        insert_event(
            &tx,
            EventInsert {
                id: &ledger_event,
                hlc: &ledger_hlc,
                source: "runtime",
                kind: dasdevbot_core::kind::LEDGER_POSTED,
                payload: ledger_payload,
                idempotency_key: &ledger_key,
                thread_id: &approval.thread_id,
                wall_ms,
            },
        )?;
        tx.execute(
            "INSERT INTO approvals (
                id, job_id, agent_id, thread_id, effect_class, action, purpose, draft, evidence,
                status, provider, model, usage_kind, input_tokens, output_tokens, micro_usd, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'pending', ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                approval_id,
                approval.job_id,
                approval.agent_id,
                approval.thread_id,
                approval.effect_class,
                approval.action,
                approval.purpose,
                approval.draft,
                approval.evidence,
                approval.provider,
                approval.model,
                approval.usage_kind,
                approval.input_tokens,
                approval.output_tokens,
                approval.micro_usd,
                wall_ms as i64,
            ],
        )?;
        tx.execute(
            "UPDATE agents SET tokens_spent = tokens_spent + ?1 WHERE id = ?2",
            params![
                approval.input_tokens.saturating_add(approval.output_tokens),
                approval.agent_id,
            ],
        )?;
        tx.execute(
            "INSERT INTO ledger (
                id, agent_id, project, provider, model, usage_kind, input_tokens, output_tokens,
                micro_usd, note, event_id, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                ledger_id,
                approval.agent_id,
                approval.project,
                approval.provider,
                approval.model,
                approval.usage_kind,
                approval.input_tokens,
                approval.output_tokens,
                approval.micro_usd,
                approval.ledger_note,
                ledger_event,
                wall_ms as i64,
            ],
        )?;
        let changed = tx.execute(
            "UPDATE jobs
             SET status = 'waiting_approval', lease_owner = NULL, lease_until_ms = NULL
             WHERE id = ?1 AND lease_owner = ?2 AND status = 'leased'",
            params![approval.job_id, owner],
        )?;
        if changed != 1 {
            return Err(Error::BadRequest(
                "failed to park the job on approval".into(),
            ));
        }
        tx.commit()?;
        Ok(approval_id)
    }

    pub fn approvals(&self) -> Result<Vec<ApprovalRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.id, a.job_id, a.agent_id, agents.name, a.thread_id, a.effect_class, a.action,
                    a.purpose, a.draft, a.evidence, a.status, a.provider, a.model, a.usage_kind,
                    a.input_tokens, a.output_tokens, a.micro_usd
             FROM approvals a
             JOIN agents ON agents.id = a.agent_id
             ORDER BY a.created_at DESC",
        )?;
        let rows = stmt.query_map([], map_approval)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn ledger(&self) -> Result<Vec<LedgerRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT l.id, l.agent_id, agents.name, l.project, l.provider, l.model, l.usage_kind,
                    l.input_tokens, l.output_tokens, l.micro_usd, l.note
             FROM ledger l
             JOIN agents ON agents.id = l.agent_id
             ORDER BY l.created_at DESC",
        )?;
        let rows = stmt.query_map([], map_ledger)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Record approve or deny. The external effect is not executed.
    pub fn decide_approval(
        &mut self,
        approval_id: &str,
        decision: &str,
        wall_ms: u64,
    ) -> Result<(String, String)> {
        if decision != "approve" && decision != "deny" {
            return Err(Error::BadRequest("decision must be approve or deny".into()));
        }
        let status = if decision == "approve" {
            "approved"
        } else {
            "denied"
        };
        let hlc = self.clock.tick(wall_ms);
        let event_id = Uuid::new_v4().to_string();
        let tx = self.conn.transaction()?;
        let (current, job_id, thread_id, agent_id): (String, String, String, String) = tx
            .query_row(
                "SELECT status, job_id, thread_id, agent_id FROM approvals WHERE id = ?1",
                [approval_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => {
                    Error::NotFound(format!("approval {approval_id}"))
                }
                other => Error::Sqlite(other),
            })?;
        if current != "pending" {
            let existing = tx
                .query_row(
                    "SELECT id FROM events WHERE idempotency_key = ?1",
                    [format!("approval-decided:{approval_id}")],
                    |row| row.get::<_, String>(0),
                )
                .unwrap_or_else(|_| String::new());
            return Ok((current, existing));
        }
        let payload = serde_json::json!({
            "approval_id": approval_id,
            "decision": decision,
            "executed": false,
            "reason": "phase 0 records the decision and does not perform the external effect",
        })
        .to_string();
        let decision_key = format!("approval-decided:{approval_id}");
        insert_event(
            &tx,
            EventInsert {
                id: &event_id,
                hlc: &hlc,
                source: "human",
                kind: dasdevbot_core::kind::APPROVAL_DECIDED,
                payload: &payload,
                idempotency_key: &decision_key,
                thread_id: &thread_id,
                wall_ms,
            },
        )?;
        tx.execute(
            "UPDATE approvals SET status = ?1 WHERE id = ?2 AND status = 'pending'",
            params![status, approval_id],
        )?;
        tx.execute(
            "UPDATE jobs SET status = 'done', lease_owner = NULL, lease_until_ms = NULL
             WHERE id = ?1 AND status = 'waiting_approval'",
            params![job_id],
        )?;
        let _ = agent_id;
        tx.commit()?;
        Ok((status.to_string(), event_id))
    }
}

struct EventInsert<'a> {
    id: &'a str,
    hlc: &'a HlcTimestamp,
    source: &'a str,
    kind: &'a str,
    payload: &'a str,
    idempotency_key: &'a str,
    thread_id: &'a str,
    wall_ms: u64,
}

fn insert_event(tx: &rusqlite::Transaction<'_>, event: EventInsert<'_>) -> Result<()> {
    tx.execute(
        "INSERT INTO events (
            id, version, hlc_millis, hlc_counter, hlc_node, source, kind,
            payload, idempotency_key, thread_id, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            event.id,
            EVENT_VERSION,
            event.hlc.millis as i64,
            event.hlc.counter,
            event.hlc.node,
            event.source,
            event.kind,
            event.payload,
            event.idempotency_key,
            event.thread_id,
            event.wall_ms as i64,
        ],
    )?;
    Ok(())
}

fn map_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredEvent> {
    let millis: i64 = row.get(2)?;
    let counter: u32 = row.get(3)?;
    let node: String = row.get(4)?;
    Ok(StoredEvent {
        id: row.get(0)?,
        version: row.get(1)?,
        hlc: HlcTimestamp {
            millis: millis.max(0) as u64,
            counter,
            node,
        },
        source: row.get(5)?,
        kind: row.get(6)?,
        payload: row.get(7)?,
        idempotency_key: row.get(8)?,
        thread_id: row.get(9)?,
    })
}

fn map_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<Job> {
    Ok(Job {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        status: row.get(2)?,
        lease_owner: row.get(3)?,
        lease_until_ms: row.get(4)?,
        idempotency_key: row.get(5)?,
        payload: row.get(6)?,
        attempt: row.get(7)?,
    })
}

fn map_agent(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentRow> {
    Ok(AgentRow {
        id: row.get(0)?,
        name: row.get(1)?,
        persona: row.get(2)?,
        project: row.get(3)?,
        token_cap: row.get(4)?,
        tokens_spent: row.get(5)?,
    })
}

fn map_approval(row: &rusqlite::Row<'_>) -> rusqlite::Result<ApprovalRow> {
    Ok(ApprovalRow {
        id: row.get(0)?,
        job_id: row.get(1)?,
        agent_id: row.get(2)?,
        agent_name: row.get(3)?,
        thread_id: row.get(4)?,
        effect_class: row.get(5)?,
        action: row.get(6)?,
        purpose: row.get(7)?,
        draft: row.get(8)?,
        evidence: row.get(9)?,
        status: row.get(10)?,
        provider: row.get(11)?,
        model: row.get(12)?,
        usage_kind: row.get(13)?,
        input_tokens: row.get(14)?,
        output_tokens: row.get(15)?,
        micro_usd: row.get(16)?,
    })
}

fn map_ledger(row: &rusqlite::Row<'_>) -> rusqlite::Result<LedgerRow> {
    Ok(LedgerRow {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        agent_name: row.get(2)?,
        project: row.get(3)?,
        provider: row.get(4)?,
        model: row.get(5)?,
        usage_kind: row.get(6)?,
        input_tokens: row.get(7)?,
        output_tokens: row.get(8)?,
        micro_usd: row.get(9)?,
        note: row.get(10)?,
    })
}

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS events (
            id TEXT PRIMARY KEY,
            version INTEGER NOT NULL,
            hlc_millis INTEGER NOT NULL,
            hlc_counter INTEGER NOT NULL,
            hlc_node TEXT NOT NULL,
            source TEXT NOT NULL,
            kind TEXT NOT NULL,
            payload TEXT NOT NULL,
            idempotency_key TEXT NOT NULL UNIQUE,
            thread_id TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_events_hlc
            ON events (hlc_millis, hlc_counter, hlc_node);

        CREATE TRIGGER IF NOT EXISTS events_no_update
        BEFORE UPDATE ON events
        BEGIN
            SELECT RAISE(ABORT, 'events are append-only');
        END;

        CREATE TRIGGER IF NOT EXISTS events_no_delete
        BEFORE DELETE ON events
        BEGIN
            SELECT RAISE(ABORT, 'events are append-only');
        END;

        CREATE TABLE IF NOT EXISTS agents (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            persona TEXT NOT NULL,
            project TEXT NOT NULL,
            token_cap INTEGER NOT NULL,
            tokens_spent INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS rules (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            agent_id TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS grants (
            agent_id TEXT NOT NULL,
            effect_class TEXT NOT NULL,
            PRIMARY KEY (agent_id, effect_class)
        );

        CREATE TABLE IF NOT EXISTS jobs (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            status TEXT NOT NULL,
            lease_owner TEXT,
            lease_until_ms INTEGER,
            idempotency_key TEXT NOT NULL UNIQUE,
            payload TEXT NOT NULL,
            attempt INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            FOREIGN KEY (agent_id) REFERENCES agents(id)
        );

        CREATE INDEX IF NOT EXISTS idx_jobs_claim
            ON jobs (status, created_at);

        CREATE TABLE IF NOT EXISTS approvals (
            id TEXT PRIMARY KEY,
            job_id TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            thread_id TEXT NOT NULL,
            effect_class TEXT NOT NULL,
            action TEXT NOT NULL,
            purpose TEXT NOT NULL,
            draft TEXT NOT NULL,
            evidence TEXT NOT NULL,
            status TEXT NOT NULL,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            usage_kind TEXT NOT NULL,
            input_tokens INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL,
            micro_usd INTEGER NOT NULL,
            created_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS ledger (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            project TEXT NOT NULL,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            usage_kind TEXT NOT NULL,
            input_tokens INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL,
            micro_usd INTEGER NOT NULL,
            note TEXT NOT NULL,
            event_id TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        ",
    )?;
    Ok(())
}

fn ensure_node_id(conn: &Connection) -> Result<String> {
    if let Some(existing) = conn
        .query_row("SELECT value FROM meta WHERE key = 'node_id'", [], |row| {
            row.get::<_, String>(0)
        })
        .optional()?
    {
        return Ok(existing);
    }
    let id = format!("node-{}", &Uuid::new_v4().to_string()[..8]);
    conn.execute(
        "INSERT INTO meta (key, value) VALUES ('node_id', ?1)",
        [&id],
    )?;
    Ok(id)
}

fn seed(conn: &Connection) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO agents (id, name, persona, project, token_cap, tokens_spent)
         VALUES ('reviewer', 'Reviewer', ?1, 'DasVR/NIL', 8000, 0)",
        [REVIEWER_PERSONA],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO rules (id, kind, agent_id) VALUES ('rule-repo-push-reviewer', 'repo.push', 'reviewer')",
        [],
    )?;
    for class in ["read", "write_local", "external"] {
        conn.execute(
            "INSERT OR IGNORE INTO grants (agent_id, effect_class) VALUES ('reviewer', ?1)",
            [class],
        )?;
    }
    Ok(())
}

fn latest_hlc(conn: &Connection) -> Result<(u64, u32)> {
    let row = conn
        .query_row(
            "SELECT hlc_millis, hlc_counter FROM events
             ORDER BY hlc_millis DESC, hlc_counter DESC LIMIT 1",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, u32>(1)?)),
        )
        .optional()?;
    Ok(match row {
        Some((millis, counter)) => (millis.max(0) as u64, counter),
        None => (0, 0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> Store {
        let conn = Connection::open_in_memory().unwrap();
        Store::from_conn(conn).unwrap()
    }

    #[test]
    fn event_log_is_append_only_and_idempotent() {
        let mut store = memory();
        let created = store
            .append_at(1_000, "demo", "repo.push", "{\"n\":1}", "k1", None)
            .unwrap();
        let AppendOutcome::Created(first) = created else {
            panic!("expected a new event");
        };
        assert_eq!(first.hlc.counter, 0);
        let again = store
            .append_at(1_000, "demo", "repo.push", "{\"n\":1}", "k1", None)
            .unwrap();
        let AppendOutcome::Replay(replayed) = again else {
            panic!("expected a replay");
        };
        assert_eq!(replayed.id, first.id);
        assert_eq!(store.event_count().unwrap(), 1);

        let err = store
            .append_at(1_001, "demo", "repo.push", "{\"n\":2}", "k1", None)
            .unwrap_err();
        assert!(matches!(err, Error::IdempotencyConflict(_)));

        let second = store
            .append_at(1_000, "demo", "repo.push", "{}", "k2", None)
            .unwrap();
        let AppendOutcome::Created(second) = second else {
            panic!("expected second event");
        };
        assert!(second.hlc > first.hlc);

        let delete_err = store.conn.execute("DELETE FROM events", []).unwrap_err();
        assert!(delete_err.to_string().contains("append-only"));
        let update_err = store
            .conn
            .execute("UPDATE events SET kind = 'nope'", [])
            .unwrap_err();
        assert!(update_err.to_string().contains("append-only"));
    }

    #[test]
    fn job_lease_claim_heartbeat_complete_and_reclaim() {
        let store = memory();
        let job_id = store
            .enqueue_job("reviewer", "job-1", "{}", 0)
            .unwrap()
            .unwrap();

        let claimed = store.claim_at("worker-a", 1_000, 500).unwrap().unwrap();
        assert_eq!(claimed.id, job_id);
        assert_eq!(claimed.lease_owner.as_deref(), Some("worker-a"));
        assert_eq!(claimed.attempt, 1);
        assert!(store.claim_at("worker-b", 1_200, 500).unwrap().is_none());

        assert!(store
            .heartbeat_at(&job_id, "worker-a", 1_300, 1_000)
            .unwrap());
        assert_eq!(store.lease_until(&job_id).unwrap(), Some(2_300));
        assert!(store.claim_at("worker-b", 2_000, 500).unwrap().is_none());

        let reclaimed = store.claim_at("worker-b", 2_300, 500).unwrap().unwrap();
        assert_eq!(reclaimed.lease_owner.as_deref(), Some("worker-b"));
        assert_eq!(reclaimed.attempt, 2);
        assert!(!store.complete(&job_id, "worker-a").unwrap());
        assert!(store.complete(&job_id, "worker-b").unwrap());
        assert_eq!(store.job_status(&job_id).unwrap().as_deref(), Some("done"));
        assert!(store.claim_at("worker-c", 9_000, 500).unwrap().is_none());
    }

    #[test]
    fn agents_are_rows_and_only_the_matching_event_wakes_one() {
        let mut store = memory();
        store
            .insert_agent("scribe", "Scribe", "notes", "DasVR/NIL", 1000)
            .unwrap();
        store
            .insert_agent("scout", "Scout", "look", "DasVR/NIL", 1000)
            .unwrap();
        assert!(store.agents().unwrap().len() >= 3);
        assert_eq!(store.job_count().unwrap(), 0);
        assert_eq!(store.agent("reviewer").unwrap().tokens_spent, 0);

        let event = store
            .append_at(
                50,
                "demo",
                "repo.push",
                "{\"repo\":\"DasVR/NIL\"}",
                "push-1",
                None,
            )
            .unwrap();
        let AppendOutcome::Created(event) = event else {
            panic!("created");
        };
        let agents = store.rules_for_kind(&event.kind).unwrap();
        assert_eq!(agents, vec!["reviewer".to_string()]);
        let job = store
            .enqueue_job(
                "reviewer",
                &format!("job:{}:reviewer", event.idempotency_key),
                &event.payload,
                50,
            )
            .unwrap();
        assert!(job.is_some());
        assert_eq!(store.job_count().unwrap(), 1);
        assert_eq!(store.job_count_for_agent("scribe").unwrap(), 0);
        assert_eq!(store.job_count_for_agent("scout").unwrap(), 0);
        assert_eq!(store.agent_status("reviewer").unwrap(), "idle");
        let _ = store.claim_at("w", 60, 1_000).unwrap().unwrap();
        assert_eq!(store.agent_status("reviewer").unwrap(), "working");
    }
}
