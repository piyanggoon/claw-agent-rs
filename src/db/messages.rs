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

/// Get paginated messages for a session.
///
/// - Without `before`: Returns the N most recent messages in chronological order.
/// - With `before` (timestamp): Returns the N messages older than `before` in
///   chronological order (for infinite scroll / load more).
///
/// Note: `before` is a **timestamp** (ISO string), NOT a message ID.
/// The SoulClaw frontend sends the timestamp of the oldest displayed message.
pub fn get_messages_paginated(
    conn: &Connection,
    web_session_id: &str,
    limit: u32,
    before: Option<&str>,
) -> Result<Vec<StoredMessage>> {
    let (sql, params_vec): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(before_ts) = before {
        // Cursor-based: get N messages older than the given timestamp
        (
            "SELECT id, thread_id, role, content, timestamp, metadata, web_session_id \
             FROM messages WHERE web_session_id = ?1 AND timestamp < ?2 \
             ORDER BY timestamp DESC LIMIT ?3",
            vec![
                Box::new(web_session_id.to_string()),
                Box::new(before_ts.to_string()),
                Box::new(limit),
            ],
        )
    } else {
        // First page: get the N most recent messages (DESC then reverse)
        (
            "SELECT id, thread_id, role, content, timestamp, metadata, web_session_id \
             FROM messages WHERE web_session_id = ?1 \
             ORDER BY timestamp DESC LIMIT ?2",
            vec![
                Box::new(web_session_id.to_string()),
                Box::new(limit),
            ],
        )
    };
    let mut stmt = conn.prepare(sql)?;
    let params_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
    let mut messages: Vec<StoredMessage> = stmt
        .query_map(params_refs.as_slice(), |row| {
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
    // Reverse DESC results to get chronological order (oldest first)
    messages.reverse();
    Ok(messages)
}

/// Check if there are messages older than the given timestamp in a session.
pub fn has_messages_before(
    conn: &Connection,
    web_session_id: &str,
    before_ts: &str,
) -> Result<bool> {
    let count: u32 = conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE web_session_id = ?1 AND timestamp < ?2",
        params![web_session_id, before_ts],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

pub fn count_messages(conn: &Connection, web_session_id: &str) -> Result<u32> {
    let count: u32 = conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE web_session_id = ?1",
        params![web_session_id],
        |row| row.get(0),
    )?;
    Ok(count)
}

/// Get session stats aggregated from ALL assistant messages in a session.
pub fn get_session_stats(conn: &Connection, web_session_id: &str) -> Result<(f64, u64, u64, u64)> {
    let mut stmt = conn.prepare(
        "SELECT metadata FROM messages WHERE web_session_id = ?1 AND role = 'assistant' AND metadata IS NOT NULL",
    )?;
    let mut total_cost: f64 = 0.0;
    let mut total_input: u64 = 0;
    let mut total_output: u64 = 0;
    let mut turns: u64 = 0;

    let rows = stmt.query_map(params![web_session_id], |row| {
        let meta: String = row.get(0)?;
        Ok(meta)
    })?;

    for row in rows.flatten() {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&row) {
            if let Some(rm) = parsed.get("resultMeta") {
                total_cost += rm.get("costUsd").and_then(|v| v.as_f64()).unwrap_or(0.0);
                total_input += rm.get("inputTokens").and_then(|v| v.as_u64()).unwrap_or(0);
                total_output += rm.get("outputTokens").and_then(|v| v.as_u64()).unwrap_or(0);
                turns += rm.get("numTurns").and_then(|v| v.as_u64()).unwrap_or(0);
            }
        }
    }

    Ok((total_cost, total_input, total_output, turns))
}
