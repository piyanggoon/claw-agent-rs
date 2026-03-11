use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use crate::db::tasks;
use crate::web::state::AppState;

// ── Handlers ─────────────────────────────────────────────────────────────────

/// GET /api/tasks
///
/// Returns all scheduled tasks ordered by creation date descending.
pub async fn list_tasks(
    State(state): State<AppState>,
) -> Result<Json<Vec<tasks::ScheduledTask>>, (StatusCode, String)> {
    let db = state.db.lock().await;
    let result = tasks::get_all_tasks(&db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(result))
}

/// POST /api/tasks/:id/pause
///
/// Pauses an active task so it won't be picked up by the scheduler.
pub async fn pause_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let db = state.db.lock().await;

    // Verify the task exists.
    tasks::get_task(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, format!("task {id} not found")))?;

    tasks::update_task_status(&db, &id, "paused")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::OK)
}

/// POST /api/tasks/:id/resume
///
/// Resumes a paused task, making it eligible for scheduling again.
pub async fn resume_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let db = state.db.lock().await;

    tasks::get_task(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, format!("task {id} not found")))?;

    tasks::update_task_status(&db, &id, "active")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::OK)
}

/// POST /api/tasks/:id/cancel
///
/// Permanently deletes a scheduled task and its run logs.
pub async fn cancel_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let db = state.db.lock().await;

    tasks::get_task(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, format!("task {id} not found")))?;

    tasks::delete_task(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::OK)
}

/// GET /api/tasks/:id/logs
///
/// Returns the execution history (run logs) for a specific task.
pub async fn task_logs(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<tasks::TaskRunLog>>, (StatusCode, String)> {
    let db = state.db.lock().await;

    // Verify the task exists.
    tasks::get_task(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, format!("task {id} not found")))?;

    let logs = tasks::get_task_logs(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(logs))
}
