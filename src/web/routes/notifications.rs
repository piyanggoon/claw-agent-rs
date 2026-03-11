use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::db::notifications;
use crate::web::state::AppState;

#[derive(Deserialize)]
pub struct NotificationQuery {
    pub unread: Option<bool>,
    pub limit: Option<u32>,
}

/// GET /api/notifications → { notifications: [...], unreadCount }
pub async fn list_notifications(
    State(state): State<AppState>,
    Query(params): Query<NotificationQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db = state.db.lock().await;
    let unread_only = params.unread.unwrap_or(false);
    let result = notifications::get_notifications(&db, unread_only, params.limit)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let unread_count = notifications::unread_count(&db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({
        "notifications": result,
        "unreadCount": unread_count,
    })))
}

/// PATCH /api/notifications/:id/read
pub async fn mark_read(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db = state.db.lock().await;
    notifications::mark_read(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({"ok": true})))
}

/// POST /api/notifications/read-all
pub async fn mark_all_read(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db = state.db.lock().await;
    notifications::mark_all_read(&db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({"ok": true})))
}

/// DELETE /api/notifications/:id
pub async fn delete_notification(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db = state.db.lock().await;
    notifications::delete_notification(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({"ok": true})))
}
