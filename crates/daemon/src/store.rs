use std::path::Path;

use dasdevbot_core::{HlcTimestamp, HybridClock, TokenBudget, EVENT_VERSION};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::{Error, Result};

/// How long a recorded decision stays undoable before it is final.
pub const UNDO_WINDOW_MS: u64 = 6_000;

/// How long a pending approval waits before it expires.
/// The spec requires a TTL and does not set the length; 15 minutes is the phase-0 value.
pub const APPROVAL_TTL_MS: u64 = 15 * 60 * 1000;

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
    pub evidence_repo: String,
    pub evidence_ref: String,
    pub evidence_event_id: String,
    pub evidence_kind: String,
    pub status: String,
    pub provider: String,
    pub model: String,
    pub usage_kind: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub micro_usd: i64,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub decided_at: Option<i64>,
    pub decision_event_id: Option<String>,
    pub reason: Option<String>,
    pub commit_due_ms: Option<i64>,
    pub committed: i64,
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
    pub evidence_repo: String,
    pub evidence_ref: String,
    pub evidence_event_id: String,
    pub evidence_kind: String,
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
        crate::ensure_data_gitignore(path)?;
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

    pub fn event_by_id(&self, id: &str) -> Result<Option<StoredEvent>> {
        self.conn
            .query_row(
                "SELECT id, version, hlc_millis, hlc_counter, hlc_node, source, kind,
                        payload, idempotency_key, thread_id
                 FROM events WHERE id = ?1",
                [id],
                map_event,
            )
            .optional()
            .map_err(Error::from)
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
        let expires_at = wall_ms.saturating_add(APPROVAL_TTL_MS) as i64;
        tx.execute(
            "INSERT INTO approvals (
                id, job_id, agent_id, thread_id, effect_class, action, purpose, draft, evidence,
                evidence_repo, evidence_ref, evidence_event_id, evidence_kind,
                status, provider, model, usage_kind, input_tokens, output_tokens, micro_usd,
                created_at, expires_at, committed
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                ?10, ?11, ?12, ?13,
                'pending', ?14, ?15, ?16, ?17, ?18, ?19,
                ?20, ?21, 0
             )",
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
                approval.evidence_repo,
                approval.evidence_ref,
                approval.evidence_event_id,
                approval.evidence_kind,
                approval.provider,
                approval.model,
                approval.usage_kind,
                approval.input_tokens,
                approval.output_tokens,
                approval.micro_usd,
                wall_ms as i64,
                expires_at,
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
                    a.input_tokens, a.output_tokens, a.micro_usd,
                    a.evidence_repo, a.evidence_ref, a.evidence_event_id, a.evidence_kind,
                    a.decided_at, a.decision_event_id, a.reason, a.commit_due_ms, a.committed,
                    a.expires_at, a.created_at
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

    /// Record approve or deny as pending-commit. The external effect waits for [`Self::commit_due`].
    pub fn decide_approval(
        &mut self,
        approval_id: &str,
        decision: &str,
        reason: Option<&str>,
        wall_ms: u64,
    ) -> Result<DecisionRecord> {
        if decision != "approve" && decision != "deny" {
            return Err(Error::BadRequest("decision must be approve or deny".into()));
        }
        self.expire_due(wall_ms)?;
        let reason = clean_reason(reason);
        let status = if decision == "approve" {
            "approved"
        } else {
            "denied"
        };
        let current = self.approval_decision_row(approval_id)?;
        if current.status != "pending" {
            if current.status == "expired" {
                return Err(Error::BadRequest(
                    "approval expired before a decision".into(),
                ));
            }
            return Ok(DecisionRecord {
                status: current.status,
                event_id: current.decision_event_id.unwrap_or_default(),
                executed: false,
                committed: current.committed != 0,
                undo_until: undo_until_ms(current.committed, current.commit_due_ms),
            });
        }
        let commit_due = wall_ms.saturating_add(UNDO_WINDOW_MS);
        let hlc = self.clock.tick(wall_ms);
        let event_id = Uuid::new_v4().to_string();
        let payload = serde_json::json!({
            "approval_id": approval_id,
            "decision": decision,
            "reason": reason,
            "executed": false,
            "pending_commit": true,
            "commit_due_ms": commit_due,
        })
        .to_string();
        let decision_key = format!("approval-decided:{approval_id}:{event_id}");
        let tx = self.conn.transaction()?;
        insert_event(
            &tx,
            EventInsert {
                id: &event_id,
                hlc: &hlc,
                source: "human",
                kind: dasdevbot_core::kind::APPROVAL_DECIDED,
                payload: &payload,
                idempotency_key: &decision_key,
                thread_id: &current.thread_id,
                wall_ms,
            },
        )?;
        let changed = tx.execute(
            "UPDATE approvals
             SET status = ?1, decided_at = ?2, decision_event_id = ?3, reason = ?4,
                 commit_due_ms = ?5, committed = 0
             WHERE id = ?6 AND status = 'pending'",
            params![
                status,
                wall_ms as i64,
                event_id,
                reason,
                commit_due as i64,
                approval_id,
            ],
        )?;
        if changed != 1 {
            return Err(Error::BadRequest("approval was no longer pending".into()));
        }
        tx.commit()?;
        Ok(DecisionRecord {
            status: status.to_string(),
            event_id,
            executed: false,
            committed: false,
            undo_until: Some(commit_due),
        })
    }

    /// Revert a pending-commit decision. Outside the window the decision is committed instead.
    pub fn undo_approval(&mut self, approval_id: &str, wall_ms: u64) -> Result<DecisionRecord> {
        self.commit_due(wall_ms)?;
        let current = self.approval_decision_row(approval_id)?;
        if current.status != "approved" && current.status != "denied" {
            return Err(Error::BadRequest("approval is not awaiting commit".into()));
        }
        if current.committed != 0 {
            return Err(Error::BadRequest("undo window has closed".into()));
        }
        let due = current.commit_due_ms.unwrap_or(0);
        if due <= wall_ms as i64 {
            return Err(Error::BadRequest("undo window has closed".into()));
        }
        let hlc = self.clock.tick(wall_ms);
        let event_id = Uuid::new_v4().to_string();
        let payload = serde_json::json!({
            "approval_id": approval_id,
            "reverted_decision": current.status,
            "reverted_event_id": current.decision_event_id,
        })
        .to_string();
        let key = format!("approval-undone:{approval_id}:{event_id}");
        let tx = self.conn.transaction()?;
        insert_event(
            &tx,
            EventInsert {
                id: &event_id,
                hlc: &hlc,
                source: "human",
                kind: dasdevbot_core::kind::APPROVAL_UNDONE,
                payload: &payload,
                idempotency_key: &key,
                thread_id: &current.thread_id,
                wall_ms,
            },
        )?;
        let changed = tx.execute(
            "UPDATE approvals
             SET status = 'pending', decided_at = NULL, decision_event_id = NULL,
                 reason = NULL, commit_due_ms = NULL, committed = 0
             WHERE id = ?1 AND committed = 0 AND status IN ('approved', 'denied')",
            params![approval_id],
        )?;
        if changed != 1 {
            return Err(Error::BadRequest("undo window has closed".into()));
        }
        tx.commit()?;
        Ok(DecisionRecord {
            status: "pending".into(),
            event_id,
            executed: false,
            committed: false,
            undo_until: None,
        })
    }

    /// Expire pending approvals past their TTL and commit decisions whose undo window has closed.
    pub fn sweep(&mut self, wall_ms: u64) -> Result<()> {
        self.expire_due(wall_ms)?;
        self.commit_due(wall_ms)?;
        Ok(())
    }

    pub fn expire_due(&mut self, wall_ms: u64) -> Result<Vec<String>> {
        let due: Vec<(String, String, String)> = {
            let mut stmt = self.conn.prepare(
                "SELECT id, job_id, thread_id FROM approvals
                 WHERE status = 'pending'
                   AND COALESCE(expires_at, created_at + ?1) <= ?2",
            )?;
            let rows = stmt.query_map(params![APPROVAL_TTL_MS as i64, wall_ms as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut expired = Vec::new();
        for (id, job_id, thread_id) in due {
            let hlc = self.clock.tick(wall_ms);
            let event_id = Uuid::new_v4().to_string();
            let payload = serde_json::json!({
                "approval_id": id,
                "reason": "ttl",
            })
            .to_string();
            let key = format!("approval-expired:{id}");
            let tx = self.conn.transaction()?;
            insert_event(
                &tx,
                EventInsert {
                    id: &event_id,
                    hlc: &hlc,
                    source: "runtime",
                    kind: dasdevbot_core::kind::APPROVAL_EXPIRED,
                    payload: &payload,
                    idempotency_key: &key,
                    thread_id: &thread_id,
                    wall_ms,
                },
            )?;
            tx.execute(
                "UPDATE approvals SET status = 'expired',
                    decided_at = ?1,
                    decision_event_id = ?2,
                    expires_at = COALESCE(expires_at, ?1)
                 WHERE id = ?3 AND status = 'pending'",
                params![wall_ms as i64, event_id, id],
            )?;
            tx.execute(
                "UPDATE jobs SET status = 'done', lease_owner = NULL, lease_until_ms = NULL
                 WHERE id = ?1 AND status = 'waiting_approval'",
                params![job_id],
            )?;
            tx.commit()?;
            expired.push(id);
        }
        Ok(expired)
    }

    /// Finalize decisions whose undo window has closed.
    ///
    /// This is the commit point. [`perform_commit_effect`] is where a later phase
    /// posts to GitHub. Phase 0 returns false and records that nothing was posted.
    pub fn commit_due(&mut self, wall_ms: u64) -> Result<Vec<String>> {
        let due: Vec<(String, String, String, String, String)> = {
            let mut stmt = self.conn.prepare(
                "SELECT id, job_id, thread_id, status, COALESCE(decision_event_id, '')
                 FROM approvals
                 WHERE committed = 0
                   AND status IN ('approved', 'denied')
                   AND commit_due_ms IS NOT NULL
                   AND commit_due_ms <= ?1",
            )?;
            let rows = stmt.query_map([wall_ms as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut committed = Vec::new();
        for (id, job_id, thread_id, status, decision_event_id) in due {
            let executed = perform_commit_effect(&id, &status);
            let hlc = self.clock.tick(wall_ms);
            let event_id = Uuid::new_v4().to_string();
            let payload = serde_json::json!({
                "approval_id": id,
                "decision": status,
                "executed": executed,
                "commit": true,
            })
            .to_string();
            let key = format!("approval-committed:{id}:{decision_event_id}");
            let tx = self.conn.transaction()?;
            insert_event(
                &tx,
                EventInsert {
                    id: &event_id,
                    hlc: &hlc,
                    source: "runtime",
                    kind: dasdevbot_core::kind::APPROVAL_COMMITTED,
                    payload: &payload,
                    idempotency_key: &key,
                    thread_id: &thread_id,
                    wall_ms,
                },
            )?;
            tx.execute(
                "UPDATE approvals SET committed = 1 WHERE id = ?1 AND committed = 0",
                params![id],
            )?;
            tx.execute(
                "UPDATE jobs SET status = 'done', lease_owner = NULL, lease_until_ms = NULL
                 WHERE id = ?1 AND status = 'waiting_approval'",
                params![job_id],
            )?;
            tx.commit()?;
            committed.push(id);
        }
        Ok(committed)
    }

    fn approval_decision_row(&self, approval_id: &str) -> Result<DecisionRow> {
        self.conn
            .query_row(
                "SELECT status, job_id, thread_id, decision_event_id, commit_due_ms, committed
                 FROM approvals WHERE id = ?1",
                [approval_id],
                |row| {
                    Ok(DecisionRow {
                        status: row.get(0)?,
                        job_id: row.get(1)?,
                        thread_id: row.get(2)?,
                        decision_event_id: row.get(3)?,
                        commit_due_ms: row.get(4)?,
                        committed: row.get(5)?,
                    })
                },
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => {
                    Error::NotFound(format!("approval {approval_id}"))
                }
                other => Error::Sqlite(other),
            })
    }
}

/// Outcome of decide or undo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionRecord {
    pub status: String,
    pub event_id: String,
    pub executed: bool,
    pub committed: bool,
    pub undo_until: Option<u64>,
}

struct DecisionRow {
    status: String,
    #[allow(dead_code)]
    job_id: String,
    thread_id: String,
    decision_event_id: Option<String>,
    commit_due_ms: Option<i64>,
    committed: i64,
}

fn undo_until_ms(committed: i64, commit_due_ms: Option<i64>) -> Option<u64> {
    if committed != 0 {
        return None;
    }
    commit_due_ms.and_then(|ms| if ms > 0 { Some(ms as u64) } else { None })
}

fn clean_reason(reason: Option<&str>) -> Option<String> {
    reason
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// Phase 0 commit hook. Returns whether an external side effect ran.
/// A later phase posts to GitHub here, and only here, after the undo window.
fn perform_commit_effect(_approval_id: &str, _decision: &str) -> bool {
    false
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
        evidence_repo: row.get(17)?,
        evidence_ref: row.get(18)?,
        evidence_event_id: row.get(19)?,
        evidence_kind: row.get(20)?,
        decided_at: row.get(21)?,
        decision_event_id: row.get(22)?,
        reason: row.get(23)?,
        commit_due_ms: row.get(24)?,
        committed: row.get(25)?,
        expires_at: row.get(26)?,
        created_at: row.get(27)?,
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
            evidence_repo TEXT NOT NULL DEFAULT '',
            evidence_ref TEXT NOT NULL DEFAULT '',
            evidence_event_id TEXT NOT NULL DEFAULT '',
            evidence_kind TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            usage_kind TEXT NOT NULL,
            input_tokens INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL,
            micro_usd INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            expires_at INTEGER,
            decided_at INTEGER,
            decision_event_id TEXT,
            reason TEXT,
            commit_due_ms INTEGER,
            committed INTEGER NOT NULL DEFAULT 0
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
    ensure_column(
        conn,
        "approvals",
        "evidence_repo",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        conn,
        "approvals",
        "evidence_ref",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        conn,
        "approvals",
        "evidence_event_id",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        conn,
        "approvals",
        "evidence_kind",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(conn, "approvals", "expires_at", "INTEGER")?;
    ensure_column(conn, "approvals", "decided_at", "INTEGER")?;
    ensure_column(conn, "approvals", "decision_event_id", "TEXT")?;
    ensure_column(conn, "approvals", "reason", "TEXT")?;
    ensure_column(conn, "approvals", "commit_due_ms", "INTEGER")?;
    ensure_column(conn, "approvals", "committed", "INTEGER NOT NULL DEFAULT 0")?;
    Ok(())
}

fn ensure_column(conn: &Connection, table: &str, column: &str, decl: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for name in names {
        if name? == column {
            return Ok(());
        }
    }
    conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"),
        [],
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
    for class in ["read", "write_local", "external", "destructive"] {
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

    fn pending_approval(store: &mut Store, now: u64) -> String {
        let job_key = format!("job-approval-{}", uuid::Uuid::new_v4());
        let job_id = store
            .enqueue_job("reviewer", &job_key, "{}", now)
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
                    thread_id: format!("thread-{now}"),
                    effect_class: "external".into(),
                    action: "post_pr_comment".into(),
                    purpose: "Post a review comment.".into(),
                    draft: "draft".into(),
                    evidence: "repo DasVR/NIL\nref phase0\nevent ev_test".into(),
                    evidence_repo: "DasVR/NIL".into(),
                    evidence_ref: "phase0".into(),
                    evidence_event_id: "ev_test".into(),
                    evidence_kind: "repo.push".into(),
                    provider: "mock".into(),
                    model: "mock-review-v0".into(),
                    usage_kind: "estimated".into(),
                    input_tokens: 10,
                    output_tokens: 4,
                    micro_usd: 0,
                    ledger_note: "test".into(),
                    project: "DasVR/NIL".into(),
                },
                "{}",
                "{}",
            )
            .unwrap()
    }

    fn row(store: &Store, id: &str) -> ApprovalRow {
        store
            .approvals()
            .unwrap()
            .into_iter()
            .find(|approval| approval.id == id)
            .unwrap()
    }

    #[test]
    fn undo_inside_the_window_reverts_to_pending() {
        let mut store = memory();
        let now = 1_000_000;
        let id = pending_approval(&mut store, now);
        let decided = store
            .decide_approval(&id, "approve", None, now + 10)
            .unwrap();
        assert_eq!(decided.status, "approved");
        assert!(!decided.committed);
        assert_eq!(decided.undo_until, Some(now + 10 + UNDO_WINDOW_MS));
        assert_eq!(
            store
                .job_status(&row(&store, &id).job_id)
                .unwrap()
                .as_deref(),
            Some("waiting_approval")
        );

        let undone = store.undo_approval(&id, now + 1_000).unwrap();
        assert_eq!(undone.status, "pending");
        let approval = row(&store, &id);
        assert_eq!(approval.status, "pending");
        assert!(approval.reason.is_none());
        assert!(approval.decision_event_id.is_none());
        assert_eq!(approval.committed, 0);
        assert_eq!(store.agent_status("reviewer").unwrap(), "blocked");
        let events = store.recent_events(20).unwrap();
        assert!(events
            .iter()
            .any(|event| event.kind == dasdevbot_core::kind::APPROVAL_UNDONE));
    }

    #[test]
    fn undo_outside_the_window_commits_and_rejects() {
        let mut store = memory();
        let now = 2_000_000;
        let id = pending_approval(&mut store, now);
        store
            .decide_approval(&id, "deny", Some("no"), now + 5)
            .unwrap();
        let outside = now + 5 + UNDO_WINDOW_MS;
        let err = store.undo_approval(&id, outside).unwrap_err();
        assert!(err.to_string().contains("window"));
        let approval = row(&store, &id);
        assert_eq!(approval.status, "denied");
        assert_eq!(approval.committed, 1);
        assert_eq!(approval.reason.as_deref(), Some("no"));
        assert_eq!(
            store.job_status(&approval.job_id).unwrap().as_deref(),
            Some("done")
        );
        assert_eq!(store.agent_status("reviewer").unwrap(), "idle");
        let events = store.recent_events(20).unwrap();
        let committed = events
            .iter()
            .find(|event| event.kind == dasdevbot_core::kind::APPROVAL_COMMITTED)
            .unwrap();
        assert!(committed.payload.contains("\"executed\":false"));
        assert!(committed.payload.contains("\"commit\":true"));
        let again = store.undo_approval(&id, outside + 50).unwrap_err();
        assert!(again.to_string().contains("window"));
    }

    #[test]
    fn reason_round_trips_and_blank_is_absent() {
        let mut store = memory();
        let now = 3_000_000;
        let id = pending_approval(&mut store, now);
        store
            .decide_approval(
                &id,
                "deny",
                Some("  Not worth a comment on a phase-0 branch  "),
                now + 1,
            )
            .unwrap();
        assert_eq!(
            row(&store, &id).reason.as_deref(),
            Some("Not worth a comment on a phase-0 branch")
        );
        store.undo_approval(&id, now + 2).unwrap();
        assert!(row(&store, &id).reason.is_none());
        store
            .decide_approval(&id, "approve", Some("   "), now + 3)
            .unwrap();
        let approval = row(&store, &id);
        assert_eq!(approval.status, "approved");
        assert!(approval.reason.is_none());
        assert_eq!(approval.evidence_repo, "DasVR/NIL");
        assert_eq!(approval.evidence_ref, "phase0");
        assert_eq!(approval.evidence_event_id, "ev_test");
    }

    #[test]
    fn pending_approval_expires_after_ttl() {
        let mut store = memory();
        let now = 4_000_000;
        let id = pending_approval(&mut store, now);
        store.sweep(now + APPROVAL_TTL_MS - 1).unwrap();
        assert_eq!(row(&store, &id).status, "pending");
        store.sweep(now + APPROVAL_TTL_MS).unwrap();
        let approval = row(&store, &id);
        assert_eq!(approval.status, "expired");
        assert!(approval.decided_at.is_some());
        assert!(approval.decision_event_id.is_some());
        assert_eq!(
            store.job_status(&approval.job_id).unwrap().as_deref(),
            Some("done")
        );
        assert_eq!(store.agent_status("reviewer").unwrap(), "idle");
        let events = store.recent_events(20).unwrap();
        assert!(events
            .iter()
            .any(|event| event.kind == dasdevbot_core::kind::APPROVAL_EXPIRED));
        let err = store
            .decide_approval(&id, "approve", None, now + APPROVAL_TTL_MS + 1)
            .unwrap_err();
        assert!(err.to_string().contains("expired"));
    }

    #[test]
    fn decision_after_expiry_is_rejected_before_a_sweep() {
        let mut store = memory();
        let now = 4_000_000;
        let id = pending_approval(&mut store, now);
        let err = store
            .decide_approval(&id, "approve", None, now + APPROVAL_TTL_MS)
            .unwrap_err();
        assert!(err.to_string().contains("expired"));
        assert_eq!(row(&store, &id).status, "expired");
        assert_eq!(
            store
                .job_status(&row(&store, &id).job_id)
                .unwrap()
                .as_deref(),
            Some("done")
        );
    }
}
