use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessage {
    pub id: String,
    pub thread_id: String,
    pub role: String,
    pub content: String,
    pub timestamp: String,
    pub metadata: Option<String>,
    pub web_session_id: Option<String>,
}

pub fn store_message(
    conn: &Connection,
    id: &str,
    thread_id: &str,
    role: &str,
    content: &str,
    web_session_id: Option<&str>,
    metadata: Option<&str>,
) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO messages (id, thread_id, role, content, timestamp, metadata, web_session_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, thread_id, role, content, now, metadata, web_session_id],
    )?;
    Ok(())
}

pub fn get_messages_by_session(
    conn: &Connection,
    web_session_id: &str,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<StoredMessage>> {
    let limit_val = limit.unwrap_or(100);
    let offset_val = offset.unwrap_or(0);
    let mut stmt = conn.prepare(
        "SELECT id, thread_id, role, content, timestamp, metadata, web_session_id \
         FROM messages WHERE web_session_id = ?1 ORDER BY timestamp ASC LIMIT ?2 OFFSET ?3",
    )?;
    let messages = stmt
        .query_map(params![web_session_id, limit_val, offset_val], |row| {
            Ok(StoredMessage {
                id: row.get(0)?,
                thread_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                timestamp: row.get(4)?,
                metadata: row.get(5)?,
                web_session_id: row.get(6)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(messages)
}

pub fn get_messages_by_thread(
    conn: &Connection,
    thread_id: &str,
) -> Result<Vec<StoredMessage>> {
    let mut stmt = conn.prepare(
        "SELECT id, thread_id, role, content, timestamp, metadata, web_session_id \
         FROM messages WHERE thread_id = ?1 ORDER BY timestamp ASC",
    )?;
    let messages = stmt
        .query_map(params![thread_id], |row| {
            Ok(StoredMessage {
                id: row.get(0)?,
                thread_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                timestamp: row.get(4)?,
                metadata: row.get(5)?,
                web_session_id: row.get(6)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(messages)
}

pub fn delete_messages_by_session(conn: &Connection, web_session_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM messages WHERE web_session_id = ?1",
        params![web_session_id],
    )?;
    Ok(())
}

pub fn count_messages(conn: &Connection, web_session_id: &str) -> Result<u32> {
    let count: u32 = conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE web_session_id = ?1",
        params![web_session_id],
        |row| row.get(0),
    )?;
    Ok(count)
}
