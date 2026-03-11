use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSession {
    pub id: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub created_at: String,
    pub last_message_at: Option<String>,
}

pub fn create_session(conn: &Connection, id: &str, title: Option<&str>) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO web_sessions (id, title, created_at, last_message_at) VALUES (?1, ?2, ?3, ?4)",
        params![id, title, now, now],
    )?;
    Ok(())
}

pub fn get_sessions(conn: &Connection) -> Result<Vec<WebSession>> {
    let mut stmt =
        conn.prepare("SELECT id, title, summary, created_at, last_message_at FROM web_sessions ORDER BY last_message_at DESC")?;
    let sessions = stmt
        .query_map([], |row| {
            Ok(WebSession {
                id: row.get(0)?,
                title: row.get(1)?,
                summary: row.get(2)?,
                created_at: row.get(3)?,
                last_message_at: row.get(4)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(sessions)
}

pub fn get_session(conn: &Connection, id: &str) -> Result<Option<WebSession>> {
    let mut stmt =
        conn.prepare("SELECT id, title, summary, created_at, last_message_at FROM web_sessions WHERE id = ?1")?;
    let result = stmt.query_row(params![id], |row| {
        Ok(WebSession {
            id: row.get(0)?,
            title: row.get(1)?,
            summary: row.get(2)?,
            created_at: row.get(3)?,
            last_message_at: row.get(4)?,
        })
    });
    match result {
        Ok(session) => Ok(Some(session)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn update_session_title(conn: &Connection, id: &str, title: &str) -> Result<()> {
    conn.execute(
        "UPDATE web_sessions SET title = ?1 WHERE id = ?2",
        params![title, id],
    )?;
    Ok(())
}

pub fn delete_session(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM web_sessions WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn touch_session(conn: &Connection, id: &str) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE web_sessions SET last_message_at = ?1 WHERE id = ?2",
        params![now, id],
    )?;
    Ok(())
}
