//! Append-only audit log. Each row's hash covers the previous tip, so a
//! forged insert breaks [`verify`].
//!
//! The signing key sits in `<data>.audit-key`, beside the database. Anyone who
//! can replace the database can replace that key. There is no external witness.
//! The tip hash is also written to `<data>.audit-tip`. A new genesis is refused
//! when that sidecar still holds a prior tip, so deleting the database does not
//! start a fresh chain.

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
    if let Some(path) = store.audit_tip_path() {
        // Keep the Hello pin that shares the sidecar.
        let mut sidecar = read_sidecar(path)?;
        sidecar.tip = Some(tip_hash);
        write_sidecar(path, &sidecar)?;
    }
    Ok(seq)
}

/// Contents of `<data>.audit-tip`. Line 1 is the tip (may be empty). An
/// optional `hello-pin <fingerprint>` line pins the Windows Hello key. The old
/// one-line file is a bare tip.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Sidecar {
    pub tip: Option<String>,
    pub hello_pin: Option<String>,
}

const HELLO_PIN_PREFIX: &str = "hello-pin ";

pub(crate) fn read_sidecar(path: &std::path::Path) -> Result<Sidecar> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Sidecar::default()),
        Err(err) => return Err(err.into()),
    };
    let mut sidecar = Sidecar::default();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if let Some(pin) = line.strip_prefix(HELLO_PIN_PREFIX) {
            let pin = pin.trim();
            if !pin.is_empty() {
                sidecar.hello_pin = Some(pin.to_string());
            }
        } else if index == 0 && !line.is_empty() {
            sidecar.tip = Some(line.to_string());
        } else if !line.is_empty() {
            return Err(crate::Error::Forbidden(
                "audit tip sidecar has an unknown line".into(),
            ));
        }
    }
    Ok(sidecar)
}

pub(crate) fn write_sidecar(path: &std::path::Path, sidecar: &Sidecar) -> Result<()> {
    let mut text = sidecar.tip.clone().unwrap_or_default();
    text.push('\n');
    if let Some(pin) = &sidecar.hello_pin {
        text.push_str(HELLO_PIN_PREFIX);
        text.push_str(pin);
        text.push('\n');
    }
    crate::write_private(path.to_path_buf(), text.as_bytes())
}

/// The pinned Hello fingerprint. Memory stores keep it on the store.
pub(crate) fn read_hello_pin(store: &Store) -> Result<Option<String>> {
    match store.audit_tip_path() {
        Some(path) => Ok(read_sidecar(path)?.hello_pin),
        None => Ok(store.memory_hello_pin().map(str::to_string)),
    }
}

/// Set or clear the pin. The tip in the sidecar is kept.
pub(crate) fn write_hello_pin(store: &mut Store, pin: Option<&str>) -> Result<()> {
    match store.audit_tip_path() {
        Some(path) => {
            let mut sidecar = read_sidecar(path)?;
            sidecar.hello_pin = pin.map(str::to_string);
            write_sidecar(path, &sidecar)
        }
        None => {
            store.set_memory_hello_pin(pin.map(str::to_string));
            Ok(())
        }
    }
}

fn link_hash(seq: i64, prev_hash: &str, kind: &str, created_at: i64, payload: &str) -> String {
    blake3::hash(format!("{seq}|{prev_hash}|{kind}|{created_at}|{payload}").as_bytes())
        .to_hex()
        .to_string()
}

/// Refuse to open a file database whose log was deleted while a prior tip
/// sidecar still exists, and refuse a sidecar that does not match the log.
pub fn bind_tip(store: &Store) -> Result<()> {
    let Some(path) = store.audit_tip_path() else {
        return Ok(());
    };
    let mut sidecar = read_sidecar(path)?;
    let db_tip = latest_tip(store)?;
    match (db_tip, sidecar.tip.clone()) {
        (None, Some(_)) => Err(crate::Error::Forbidden(
            "refusing a new audit genesis; a prior tip exists".into(),
        )),
        (Some(db), Some(side)) if db != side => Err(crate::Error::Forbidden(
            "audit tip sidecar does not match the log".into(),
        )),
        (Some(db), None) => {
            sidecar.tip = Some(db);
            write_sidecar(path, &sidecar)
        }
        _ => Ok(()),
    }
}

fn latest_tip(store: &Store) -> Result<Option<String>> {
    store
        .connection()
        .query_row(
            "SELECT tip_hash FROM audit_log ORDER BY seq DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(crate::Error::from)
}

/// One audit row per seeded grant. Later startups do not append the same payload again.
pub fn audit_grant_seed(store: &mut Store, now_ms: u64, audit_seed: &[u8; 32]) -> Result<()> {
    let grants = store.grants()?;
    for (agent_id, effect_class, expires_at) in grants {
        let payload = serde_json::json!({
            "agent_id": agent_id,
            "effect_class": effect_class,
            "expires_at": expires_at,
        })
        .to_string();
        let count: i64 = store.connection().query_row(
            "SELECT COUNT(*) FROM audit_log WHERE kind = 'grant.seeded' AND payload = ?1",
            [&payload],
            |row| row.get(0),
        )?;
        if count == 0 {
            append(store, "grant.seeded", &payload, now_ms, audit_seed)?;
        }
    }
    Ok(())
}
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
    job_id: &str,
    cli_version: &str,
) -> Result<i64> {
    let payload = serde_json::json!({
        "job_id": job_id,
        "cli_version": cli_version,
    })
    .to_string();
    append(store, "provider.tool_use_blocked", &payload, now_ms, audit_seed)
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
    fn the_sidecar_reads_the_old_format_and_keeps_both_fields() {
        let dir = std::env::temp_dir().join(format!("dasdevbot-sidecar-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("db.sqlite.audit-tip");
        std::fs::write(&path, "abc123\n").unwrap();
        assert_eq!(
            read_sidecar(&path).unwrap(),
            Sidecar {
                tip: Some("abc123".into()),
                hello_pin: None
            }
        );
        let both = Sidecar {
            tip: Some("abc123".into()),
            hello_pin: Some("f00d".into()),
        };
        write_sidecar(&path, &both).unwrap();
        assert_eq!(read_sidecar(&path).unwrap(), both);
        let pin_only = Sidecar {
            tip: None,
            hello_pin: Some("f00d".into()),
        };
        write_sidecar(&path, &pin_only).unwrap();
        assert_eq!(read_sidecar(&path).unwrap(), pin_only);
        std::fs::write(&path, "abc123\nsomething else\n").unwrap();
        assert!(read_sidecar(&path).is_err());
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
