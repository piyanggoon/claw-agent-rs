use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use crate::db::notifications;
use crate::web::state::AppState;

// ── Query parameters ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct NotificationQuery {
    /// When `true`, only return unread notifications.
    pub unread: Option<bool>,
    /// Maximum number of notifications to return (default: 50).
    pub limit: Option<u32>,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// GET /api/notifications
///
/// Returns notifications ordered by creation date descending. Supports
/// filtering to only unread notifications via `?unread=true`.
pub async fn list_notifications(
    State(state): State<AppState>,
    Query(params): Query<NotificationQuery>,
) -> Result<Json<Vec<notifications::Notification>>, (StatusCode, String)> {
    let db = state.db.lock().await;
    let unread_only = params.unread.unwrap_or(false);
    let result = notifications::get_notifications(&db, unread_only, params.limit)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(result))
}

/// POST /api/notifications/:id/read
///
/// Marks a single notification as read.
pub async fn mark_read(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let db = state.db.lock().await;
    notifications::mark_read(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::OK)
}

/// POST /api/notifications/read-all
///
/// Marks all unread notifications as read in a single operation.
pub async fn mark_all_read(
    State(state): State<AppState>,
) -> Result<StatusCode, (StatusCode, String)> {
    let db = state.db.lock().await;
    notifications::mark_all_read(&db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::OK)
}

/// DELETE /api/notifications/:id
///
/// Permanently deletes a notification.
pub async fn delete_notification(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let db = state.db.lock().await;
    notifications::delete_notification(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::OK)
}
