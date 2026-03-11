use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: String,
    pub group_folder: String,
    pub prompt: String,
    pub schedule_type: String,
    pub schedule_value: String,
    pub context_mode: String,
    pub context_session: Option<String>,
    pub next_run: Option<String>,
    pub last_run: Option<String>,
    pub last_result: Option<String>,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRunLog {
    pub id: i64,
    pub task_id: String,
    pub run_at: String,
    pub duration_ms: u64,
    pub status: String,
    pub result: Option<String>,
    pub error: Option<String>,
}

pub fn create_task(conn: &Connection, task: &ScheduledTask) -> Result<()> {
    conn.execute(
        "INSERT INTO scheduled_tasks (id, group_folder, prompt, schedule_type, schedule_value, context_mode, context_session, next_run, last_run, last_result, status, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            task.id,
            task.group_folder,
            task.prompt,
            task.schedule_type,
            task.schedule_value,
            task.context_mode,
            task.context_session,
            task.next_run,
            task.last_run,
            task.last_result,
            task.status,
            task.created_at,
        ],
    )?;
    Ok(())
}

pub fn get_task(conn: &Connection, id: &str) -> Result<Option<ScheduledTask>> {
    let mut stmt = conn.prepare(
        "SELECT id, group_folder, prompt, schedule_type, schedule_value, context_mode, context_session, next_run, last_run, last_result, status, created_at \
         FROM scheduled_tasks WHERE id = ?1",
    )?;
    let result = stmt.query_row(params![id], |row| {
        Ok(ScheduledTask {
            id: row.get(0)?,
            group_folder: row.get(1)?,
            prompt: row.get(2)?,
            schedule_type: row.get(3)?,
            schedule_value: row.get(4)?,
            context_mode: row.get(5)?,
            context_session: row.get(6)?,
            next_run: row.get(7)?,
            last_run: row.get(8)?,
            last_result: row.get(9)?,
            status: row.get(10)?,
            created_at: row.get(11)?,
        })
    });
    match result {
        Ok(task) => Ok(Some(task)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn get_all_tasks(conn: &Connection) -> Result<Vec<ScheduledTask>> {
    let mut stmt = conn.prepare(
        "SELECT id, group_folder, prompt, schedule_type, schedule_value, context_mode, context_session, next_run, last_run, last_result, status, created_at \
         FROM scheduled_tasks ORDER BY created_at DESC",
    )?;
    let tasks = stmt
        .query_map([], |row| {
            Ok(ScheduledTask {
                id: row.get(0)?,
                group_folder: row.get(1)?,
                prompt: row.get(2)?,
                schedule_type: row.get(3)?,
                schedule_value: row.get(4)?,
                context_mode: row.get(5)?,
                context_session: row.get(6)?,
                next_run: row.get(7)?,
                last_run: row.get(8)?,
                last_result: row.get(9)?,
                status: row.get(10)?,
                created_at: row.get(11)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(tasks)
}

pub fn get_due_tasks(conn: &Connection) -> Result<Vec<ScheduledTask>> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut stmt = conn.prepare(
        "SELECT id, group_folder, prompt, schedule_type, schedule_value, context_mode, context_session, next_run, last_run, last_result, status, created_at \
         FROM scheduled_tasks WHERE next_run <= ?1 AND status = 'active'",
    )?;
    let tasks = stmt
        .query_map(params![now], |row| {
            Ok(ScheduledTask {
                id: row.get(0)?,
                group_folder: row.get(1)?,
                prompt: row.get(2)?,
                schedule_type: row.get(3)?,
                schedule_value: row.get(4)?,
                context_mode: row.get(5)?,
                context_session: row.get(6)?,
                next_run: row.get(7)?,
                last_run: row.get(8)?,
                last_result: row.get(9)?,
                status: row.get(10)?,
                created_at: row.get(11)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(tasks)
}

pub fn update_task_status(conn: &Connection, id: &str, status: &str) -> Result<()> {
    conn.execute(
        "UPDATE scheduled_tasks SET status = ?1 WHERE id = ?2",
        params![status, id],
    )?;
    Ok(())
}

pub fn set_task_running(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "UPDATE scheduled_tasks SET status = 'running' WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

pub fn update_task_after_run(
    conn: &Connection,
    id: &str,
    next_run: Option<&str>,
    result: Option<&str>,
) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE scheduled_tasks SET last_run = ?1, next_run = ?2, last_result = ?3, status = 'active' WHERE id = ?4",
        params![now, next_run, result, id],
    )?;
    Ok(())
}

pub fn delete_task(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM task_run_logs WHERE task_id = ?1",
        params![id],
    )?;
    conn.execute(
        "DELETE FROM scheduled_tasks WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

pub fn log_task_run(
    conn: &Connection,
    task_id: &str,
    duration_ms: u64,
    status: &str,
    result: Option<&str>,
    error: Option<&str>,
) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO task_run_logs (task_id, run_at, duration_ms, status, result, error) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![task_id, now, duration_ms as i64, status, result, error],
    )?;
    Ok(())
}

pub fn get_task_logs(conn: &Connection, task_id: &str) -> Result<Vec<TaskRunLog>> {
    let mut stmt = conn.prepare(
        "SELECT id, task_id, run_at, duration_ms, status, result, error \
         FROM task_run_logs WHERE task_id = ?1 ORDER BY run_at DESC",
    )?;
    let logs = stmt
        .query_map(params![task_id], |row| {
            Ok(TaskRunLog {
                id: row.get(0)?,
                task_id: row.get(1)?,
                run_at: row.get(2)?,
                duration_ms: row.get::<_, i64>(3)? as u64,
                status: row.get(4)?,
                result: row.get(5)?,
                error: row.get(6)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(logs)
}
