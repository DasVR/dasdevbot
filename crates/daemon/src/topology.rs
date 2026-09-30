//! Leader lease in SQLite. The leader assigns work. Workers only claim their own.

use dasdevbot_core::{claim_leader, dispatch_allowed, may_seek_leadership, parse_role, Claim, LeaderLease};
use rusqlite::{params, OptionalExtension};

use crate::store::Store;
use crate::Result;

const LEADER_TTL_MS: u64 = 30_000;

pub fn tick(store: &mut Store, role: &str, instance: &str, now_ms: u64) -> Result<()> {
    store.connection().execute(
        "INSERT INTO workers (id, role, seen_ms) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET role = excluded.role, seen_ms = excluded.seen_ms",
        params![instance, role, now_ms as i64],
    )?;
    let Some(parsed) = parse_role(role) else {
        return Ok(());
    };
    if !may_seek_leadership(parsed) {
        return Ok(());
    }
    let current = read_lease(store)?;
    let Claim::Held(lease) = claim_leader(current, instance, now_ms, LEADER_TTL_MS) else {
        return Ok(());
    };
    write_lease(store, &lease)?;
    if dispatch_allowed(&lease, instance, lease.fencing, now_ms) {
        let target = pick_worker(store)?.unwrap_or_else(|| instance.to_string());
        store.connection().execute(
            "UPDATE jobs SET assigned_worker = ?1
             WHERE status = 'pending' AND assigned_worker IS NULL",
            [target],
        )?;
    }
    Ok(())
}

fn read_lease(store: &Store) -> Result<Option<LeaderLease>> {
    store
        .connection()
        .query_row(
            "SELECT holder, fencing, until_ms FROM leader_lease WHERE id = 1",
            [],
            |row| {
                Ok(LeaderLease {
                    holder: row.get(0)?,
                    fencing: row.get::<_, i64>(1)? as u64,
                    until_ms: row.get::<_, i64>(2)? as u64,
                })
            },
        )
        .optional()
        .map_err(crate::Error::from)
}

fn write_lease(store: &Store, lease: &LeaderLease) -> Result<()> {
    store.connection().execute(
        "INSERT INTO leader_lease (id, holder, fencing, until_ms) VALUES (1, ?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET
            holder = excluded.holder,
            fencing = excluded.fencing,
            until_ms = excluded.until_ms",
        params![lease.holder, lease.fencing as i64, lease.until_ms as i64],
    )?;
    Ok(())
}

fn pick_worker(store: &Store) -> Result<Option<String>> {
    store
        .connection()
        .query_row(
            "SELECT id FROM workers WHERE role = 'worker' ORDER BY seen_ms DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(crate::Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_leader_assigns_work_and_a_worker_cannot_take_someone_elses_job() {
        let mut store = Store::open_memory().unwrap();
        let job = store
            .enqueue_job("reviewer", "job-dispatch", "{}", 10)
            .unwrap()
            .unwrap();
        tick(&mut store, "worker", "worker-b", 10).unwrap();
        assert!(store.claim_assigned("worker-b", 10, 1_000).unwrap().is_none());
        tick(&mut store, "leader", "leader-a", 20).unwrap();
        tick(&mut store, "leader", "leader-b", 30).unwrap();
        let holder: String = store
            .connection()
            .query_row("SELECT holder FROM leader_lease WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(holder, "leader-a");
        let assigned: String = store
            .connection()
            .query_row(
                "SELECT assigned_worker FROM jobs WHERE id = ?1",
                [&job],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(assigned, "worker-b");
        assert!(store.claim_assigned("worker-a", 40, 1_000).unwrap().is_none());
        assert!(store.claim_assigned("worker-b", 40, 1_000).unwrap().is_some());
    }
}
