use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub task_id: Option<String>,
    pub source: String,
    pub title: String,
    pub message: String,
    pub level: String,
    pub read: bool,
    pub created_at: String,
}

pub fn create_notification(
    conn: &Connection,
    id: &str,
    title: &str,
    message: &str,
    level: &str,
    source: &str,
    task_id: Option<&str>,
) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO notifications (id, task_id, source, title, message, level, read, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
        params![id, task_id, source, title, message, level, now],
    )?;
    Ok(())
}

pub fn get_notifications(
    conn: &Connection,
    unread_only: bool,
    limit: Option<u32>,
) -> Result<Vec<Notification>> {
    let limit_val = limit.unwrap_or(50);
    let sql = if unread_only {
        "SELECT id, task_id, source, title, message, level, read, created_at \
         FROM notifications WHERE read = 0 ORDER BY created_at DESC LIMIT ?1"
    } else {
        "SELECT id, task_id, source, title, message, level, read, created_at \
         FROM notifications ORDER BY created_at DESC LIMIT ?1"
    };
    let mut stmt = conn.prepare(sql)?;
    let notifications = stmt
        .query_map(params![limit_val], |row| {
            Ok(Notification {
                id: row.get(0)?,
                task_id: row.get(1)?,
                source: row.get(2)?,
                title: row.get(3)?,
                message: row.get(4)?,
                level: row.get(5)?,
                read: row.get::<_, i32>(6)? != 0,
                created_at: row.get(7)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(notifications)
}

pub fn mark_read(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "UPDATE notifications SET read = 1 WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

pub fn mark_all_read(conn: &Connection) -> Result<()> {
    conn.execute("UPDATE notifications SET read = 1 WHERE read = 0", [])?;
    Ok(())
}

pub fn delete_notification(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM notifications WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn unread_count(conn: &Connection) -> Result<u32> {
    let count: u32 = conn.query_row(
        "SELECT COUNT(*) FROM notifications WHERE read = 0",
        [],
        |row| row.get(0),
    )?;
    Ok(count)
}
