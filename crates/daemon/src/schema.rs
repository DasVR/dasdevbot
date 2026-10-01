//! Phase 1 tables. Chats cannot be marked for replication. The provider
//! ledger has one writer, a fencing epoch, a budget window, and a slot cap.

use rusqlite::Connection;

use crate::Result;

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS leader_lease (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            holder TEXT NOT NULL,
            fencing INTEGER NOT NULL,
            until_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS workers (
            id TEXT PRIMARY KEY,
            role TEXT NOT NULL,
            seen_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS chats (
            id TEXT PRIMARY KEY,
            device_node TEXT NOT NULL,
            title TEXT NOT NULL,
            replicate_to_server INTEGER NOT NULL DEFAULT 0 CHECK (replicate_to_server = 0)
        );

        CREATE TABLE IF NOT EXISTS chat_messages (
            id TEXT PRIMARY KEY,
            chat_id TEXT NOT NULL,
            body TEXT NOT NULL,
            replicate_to_server INTEGER NOT NULL DEFAULT 0 CHECK (replicate_to_server = 0)
        );

        CREATE TABLE IF NOT EXISTS provider_caps (
            provider TEXT PRIMARY KEY,
            cap_tokens INTEGER NOT NULL,
            spent_tokens INTEGER NOT NULL,
            reserved_tokens INTEGER NOT NULL DEFAULT 0,
            window_kind TEXT NOT NULL,
            reset_at_ms INTEGER,
            concurrency_cap INTEGER NOT NULL,
            in_flight INTEGER NOT NULL DEFAULT 0,
            fencing_epoch INTEGER NOT NULL,
            writer_id TEXT NOT NULL,
            writer_until_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS budget_reservations (
            job_id TEXT PRIMARY KEY,
            teammate_id TEXT NOT NULL,
            provider TEXT NOT NULL,
            tokens INTEGER NOT NULL,
            teammate_charged INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS decision_nonces (
            nonce TEXT PRIMARY KEY,
            approval_id TEXT NOT NULL,
            payload_hash TEXT NOT NULL,
            purpose TEXT NOT NULL,
            expires_at INTEGER NOT NULL,
            consumed INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS teammate_provider_policy (
            teammate_id TEXT NOT NULL,
            provider TEXT NOT NULL,
            ordinal INTEGER NOT NULL,
            PRIMARY KEY (teammate_id, provider)
        );

        CREATE TABLE IF NOT EXISTS scoped_grants (
            grant_id TEXT PRIMARY KEY,
            teammate_id TEXT NOT NULL,
            resource TEXT NOT NULL,
            verbs TEXT NOT NULL,
            max_tier TEXT NOT NULL,
            expires_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS harness_checkpoints (
            job_id TEXT PRIMARY KEY,
            phase TEXT NOT NULL,
            paused INTEGER NOT NULL,
            step INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS audit_log (
            seq INTEGER PRIMARY KEY,
            prev_hash TEXT NOT NULL,
            payload_hash TEXT NOT NULL,
            tip_hash TEXT NOT NULL,
            kind TEXT NOT NULL,
            payload TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            signature TEXT NOT NULL
        );

        CREATE TRIGGER IF NOT EXISTS audit_log_no_update
        BEFORE UPDATE ON audit_log
        BEGIN
            SELECT RAISE(ABORT, 'audit log is append-only');
        END;

        CREATE TRIGGER IF NOT EXISTS audit_log_no_delete
        BEFORE DELETE ON audit_log
        BEGIN
            SELECT RAISE(ABORT, 'audit log is append-only');
        END;

        CREATE TABLE IF NOT EXISTS hello_public_key (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            blob_type INTEGER NOT NULL,
            public_key TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        ",
    )?;
    ensure_column(conn, "jobs", "assigned_worker", "TEXT")?;
    ensure_column(conn, "jobs", "quota_until_ms", "INTEGER")?;
    ensure_column(conn, "jobs", "admit_provider", "TEXT")?;
    ensure_column(conn, "agents", "conservative_max", "INTEGER")?;
    ensure_column(
        conn,
        "budget_reservations",
        "charged_tokens",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        conn,
        "budget_reservations",
        "teammate_charged",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(conn, "audit_log", "signature", "TEXT NOT NULL DEFAULT ''")?;
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
