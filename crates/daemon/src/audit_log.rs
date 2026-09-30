//! Append-only audit log. Each row's hash covers the previous tip, so a
//! forged insert breaks [`verify`]. Tool-use failures record an empty payload.

use dasdevbot_core::tool_use_audit_payload;
use rusqlite::{params, OptionalExtension};

use crate::signature::{sign_bytes, verify_bytes};
use crate::store::Store;
use crate::Result;

pub fn append(
    store: &mut Store,
    kind: &str,
    payload: &str,
    now_ms: u64,
    audit_seed: &[u8; 32],
) -> Result<i64> {
    let tx = store.connection_mut().unchecked_transaction()?;
    let prev = tx
        .query_row(
            "SELECT seq, tip_hash FROM audit_log ORDER BY seq DESC LIMIT 1",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let (seq, prev_hash) = match prev {
        Some((seq, tip)) => (seq + 1, tip),
        None => (1, "genesis".to_string()),
    };
    let created_at = now_ms as i64;
    let payload_hash = blake3::hash(payload.as_bytes()).to_hex().to_string();
    let tip_hash = link_hash(seq, &prev_hash, kind, created_at, payload);
    let signature = sign_bytes(audit_seed, tip_hash.as_bytes());
    tx.execute(
        "INSERT INTO audit_log (seq, prev_hash, payload_hash, tip_hash, kind, payload, created_at, signature)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            seq,
            prev_hash,
            payload_hash,
            tip_hash,
            kind,
            payload,
            created_at,
            signature
        ],
    )?;
    tx.commit()?;
    Ok(seq)
}

fn link_hash(seq: i64, prev_hash: &str, kind: &str, created_at: i64, payload: &str) -> String {
    blake3::hash(format!("{seq}|{prev_hash}|{kind}|{created_at}|{payload}").as_bytes())
        .to_hex()
        .to_string()
}

/// Walk the chain. An empty log is valid. A tip read error is an error,
/// not a new genesis row (that path lives in [`append`]).
pub fn verify(store: &Store, audit_seed: &[u8; 32]) -> Result<bool> {
    let mut stmt = store.connection().prepare(
        "SELECT seq, prev_hash, tip_hash, kind, payload, created_at, signature
         FROM audit_log ORDER BY seq ASC",
    )?;
    let mut rows = stmt.query([])?;
    let mut expected_prev = "genesis".to_string();
    let mut expected_seq = 1i64;
    while let Some(row) = rows.next()? {
        let seq: i64 = row.get(0)?;
        let prev_hash: String = row.get(1)?;
        let tip_hash: String = row.get(2)?;
        let kind: String = row.get(3)?;
        let payload: String = row.get(4)?;
        let created_at: i64 = row.get(5)?;
        let signature: String = row.get(6)?;
        if seq != expected_seq || prev_hash != expected_prev {
            return Ok(false);
        }
        let linked = link_hash(seq, &prev_hash, &kind, created_at, &payload);
        if linked != tip_hash || !verify_bytes(audit_seed, tip_hash.as_bytes(), &signature) {
            return Ok(false);
        }
        expected_prev = tip_hash;
        expected_seq += 1;
    }
    Ok(true)
}

/// No content: the payload is the empty audit body, not the prompt or tool args.
pub fn audit_tool_use_attempted(
    store: &mut Store,
    now_ms: u64,
    audit_seed: &[u8; 32],
) -> Result<i64> {
    append(
        store,
        "provider.tool_use_attempted",
        tool_use_audit_payload(),
        now_ms,
        audit_seed,
    )
}

pub fn audit_tool_use_blocked(
    store: &mut Store,
    now_ms: u64,
    audit_seed: &[u8; 32],
) -> Result<i64> {
    append(
        store,
        "provider.tool_use_blocked",
        tool_use_audit_payload(),
        now_ms,
        audit_seed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: [u8; 32] = [9u8; 32];

    #[test]
    fn tool_use_audit_has_no_content_and_the_log_is_append_only() {
        let mut store = Store::open_memory().unwrap();
        let secret = "tool-args-SECRET";
        let seq = audit_tool_use_attempted(&mut store, 10, &SEED).unwrap();
        assert_eq!(seq, 1);
        let payload: String = store
            .connection()
            .query_row("SELECT payload FROM audit_log WHERE seq = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(payload.is_empty());
        assert!(!payload.contains(secret));
        let err = store
            .connection()
            .execute("UPDATE audit_log SET payload = 'x' WHERE seq = 1", [])
            .unwrap_err();
        assert!(err.to_string().contains("append-only"));
        let err = store
            .connection()
            .execute("DELETE FROM audit_log WHERE seq = 1", [])
            .unwrap_err();
        assert!(err.to_string().contains("append-only"));
    }

    #[test]
    fn the_hash_chain_verifies_and_a_forged_row_does_not() {
        let mut store = Store::open_memory().unwrap();
        assert!(verify(&store, &SEED).unwrap());
        append(&mut store, "one", "alpha", 10, &SEED).unwrap();
        append(&mut store, "two", "beta", 20, &SEED).unwrap();
        assert!(verify(&store, &SEED).unwrap());
        assert!(!verify(&store, &[1u8; 32]).unwrap());
        store
            .connection()
            .execute(
                "INSERT INTO audit_log (seq, prev_hash, payload_hash, tip_hash, kind, payload, created_at, signature)
                 VALUES (3, 'genesis', 'nope', 'nope', 'forged', 'gamma', 30, '00')",
                [],
            )
            .unwrap();
        assert!(!verify(&store, &SEED).unwrap());
    }
}
