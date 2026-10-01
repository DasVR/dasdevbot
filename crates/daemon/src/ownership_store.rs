//! Device chats stay in this SQLite. The server replica is job metadata only.

use rusqlite::params;

use crate::store::Store;
use crate::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobMeta {
    pub id: String,
    pub teammate_id: String,
    pub status: String,
}

pub fn insert_chat(store: &Store, id: &str, device_node: &str, title: &str) -> Result<()> {
    store.connection().execute(
        "INSERT INTO chats (id, device_node, title, replicate_to_server) VALUES (?1, ?2, ?3, 0)",
        params![id, device_node, title],
    )?;
    Ok(())
}

/// `include_device_chats` is ignored. Chats are never part of the server batch.
pub fn server_batch(store: &Store, _include_device_chats: bool) -> Result<Vec<JobMeta>> {
    let mut stmt = store
        .connection()
        .prepare("SELECT id, agent_id, status FROM jobs ORDER BY created_at, id")?;
    let rows = stmt.query_map([], |row| {
        Ok(JobMeta {
            id: row.get(0)?,
            teammate_id: row.get(1)?,
            status: row.get(2)?,
        })
    })?;
    let mut jobs = Vec::new();
    for row in rows {
        jobs.push(row?);
    }
    Ok(jobs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chats_are_not_in_the_server_batch_and_cannot_be_flagged_for_replication() {
        let store = Store::open_memory().unwrap();
        let title = "private-chat-title-9f3a";
        insert_chat(&store, "chat-1", "device-a", title).unwrap();
        let err = store.connection().execute(
            "INSERT INTO chats (id, device_node, title, replicate_to_server) VALUES ('x', 'd', 't', 1)",
            [],
        );
        assert!(err.is_err());
        let job = store
            .enqueue_job("reviewer", "job-meta", "{}", 1)
            .unwrap()
            .unwrap();
        let batch = server_batch(&store, true).unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].id, job);
        let rendered = format!("{batch:?}");
        assert!(!rendered.contains(title));
    }
}
