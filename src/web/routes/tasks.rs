use std::convert::Infallible;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::Json;
use futures::stream::Stream;
use serde::Deserialize;
use serde_json::json;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::db::tasks;
use crate::scheduler::engine::calculate_initial_next_run;
use crate::web::state::AppState;

#[derive(Deserialize)]
pub struct TaskQuery {
    pub group: Option<String>,
}

#[derive(Deserialize)]
pub struct TaskLogQuery {
    pub limit: Option<u32>,
}

#[derive(Deserialize)]
pub struct CreateTaskRequest {
    pub prompt: String,
    pub schedule_type: String,
    pub schedule_value: String,
    pub group_folder: Option<String>,
    pub context_mode: Option<String>,
    pub web_session_id: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateTaskRequest {
    pub status: Option<String>,
}

/// GET /api/tasks → { tasks: [...] }
pub async fn list_tasks(
    State(state): State<AppState>,
    Query(params): Query<TaskQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db = state.db.lock().await;
    let all = tasks::get_all_tasks(&db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let filtered: Vec<_> = if let Some(group) = &params.group {
        all.into_iter().filter(|t| &t.group_folder == group).collect()
    } else {
        all
    };
    Ok(Json(json!({"tasks": filtered})))
}

/// POST /api/tasks — create task
pub async fn create_task(
    State(state): State<AppState>,
    Json(body): Json<CreateTaskRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let group = body.group_folder.unwrap_or_else(|| state.config.main_group.clone());
    let context_mode = body.context_mode.unwrap_or_else(|| "group".to_string());
    let next_run = calculate_initial_next_run(&body.schedule_type, &body.schedule_value);

    let task = tasks::ScheduledTask {
        id: id.clone(),
        group_folder: group,
        prompt: body.prompt,
        schedule_type: body.schedule_type,
        schedule_value: body.schedule_value,
        context_mode,
        context_session: body.web_session_id,
        next_run,
        last_run: None,
        last_result: None,
        status: "active".to_string(),
        created_at: now,
    };

    {
        let db = state.db.lock().await;
        tasks::create_task(&db, &task)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    state.scheduler.notify_new_task();

    // Emit task event
    let _ = state.task_events_tx.send(json!({
        "type": "task_created",
        "task_id": id,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }));

    Ok(Json(json!({"task": task})))
}

/// GET /api/tasks/:id → { task: {...} }
pub async fn get_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db = state.db.lock().await;
    let task = tasks::get_task(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, format!("task {id} not found")))?;
    Ok(Json(json!({"task": task})))
}

/// PATCH /api/tasks/:id — update status
pub async fn update_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateTaskRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db = state.db.lock().await;
    let task = tasks::get_task(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, format!("task {id} not found")))?;

    if let Some(status) = &body.status {
        tasks::update_task_status(&db, &id, status)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    let updated = tasks::get_task(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .unwrap_or(task);

    let _ = state.task_events_tx.send(json!({
        "type": "task_updated",
        "task_id": id,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }));

    Ok(Json(json!({"task": updated})))
}

/// DELETE /api/tasks/:id
pub async fn delete_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db = state.db.lock().await;
    tasks::get_task(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, format!("task {id} not found")))?;
    tasks::delete_task(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let _ = state.task_events_tx.send(json!({
        "type": "task_deleted",
        "task_id": id,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }));

    Ok(Json(json!({"ok": true})))
}

/// POST /api/tasks/:id/pause
pub async fn pause_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db = state.db.lock().await;
    tasks::update_task_status(&db, &id, "paused")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let _ = state.task_events_tx.send(json!({
        "type": "task_paused",
        "task_id": id,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }));
    Ok(Json(json!({"ok": true})))
}

/// POST /api/tasks/:id/resume
pub async fn resume_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db = state.db.lock().await;
    tasks::update_task_status(&db, &id, "active")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let _ = state.task_events_tx.send(json!({
        "type": "task_resumed",
        "task_id": id,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }));
    Ok(Json(json!({"ok": true})))
}

/// POST /api/tasks/:id/cancel
pub async fn cancel_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db = state.db.lock().await;
    tasks::delete_task(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let _ = state.task_events_tx.send(json!({
        "type": "task_cancelled",
        "task_id": id,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }));
    Ok(Json(json!({"ok": true})))
}

/// GET /api/tasks/:id/logs → { logs: [...] }
pub async fn task_logs(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<TaskLogQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db = state.db.lock().await;
    tasks::get_task(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, format!("task {id} not found")))?;
    let logs = tasks::get_task_logs(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let limit = params.limit.unwrap_or(20) as usize;
    let limited: Vec<_> = logs.into_iter().take(limit).collect();
    Ok(Json(json!({"logs": limited})))
}

/// GET /api/tasks/logs — all task logs
pub async fn all_task_logs(
    State(state): State<AppState>,
    Query(params): Query<TaskLogQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db = state.db.lock().await;
    let limit = params.limit.unwrap_or(50);
    let mut stmt = db.prepare(
        "SELECT id, task_id, run_at, duration_ms, status, result, error \
         FROM task_run_logs ORDER BY run_at DESC LIMIT ?1"
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let logs: Vec<serde_json::Value> = stmt.query_map(
        rusqlite::params![limit],
        |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "task_id": row.get::<_, String>(1)?,
                "run_at": row.get::<_, String>(2)?,
                "duration_ms": row.get::<_, i64>(3)?,
                "status": row.get::<_, String>(4)?,
                "result": row.get::<_, Option<String>>(5)?,
                "error": row.get::<_, Option<String>>(6)?,
            }))
        }
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .filter_map(|r| r.ok())
    .collect();
    Ok(Json(json!({"logs": logs})))
}

/// GET /api/tasks/events — SSE stream for real-time task events
pub async fn task_events_stream(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.task_events_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(json) => {
            let data = serde_json::to_string(&json).unwrap_or_default();
            Some(Ok(Event::default().data(data)))
        }
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(30))
            .text("keep-alive"),
    )
}
