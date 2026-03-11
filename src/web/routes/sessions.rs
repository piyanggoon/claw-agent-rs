use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::db::{messages, sessions};
use crate::web::state::AppState;

#[derive(Deserialize)]
pub struct RenameRequest {
    pub title: String,
}

/// GET /api/sessions → { sessions: [...] }
pub async fn list_sessions(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db = state.db.lock().await;
    let result = sessions::get_sessions(&db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let enriched: Vec<serde_json::Value> = result.iter().map(|s| {
        let count = messages::count_messages(&db, &s.id).unwrap_or(0);
        json!({
            "id": s.id,
            "title": s.title,
            "summary": s.summary,
            "created_at": s.created_at,
            "last_message_at": s.last_message_at,
            "message_count": count,
        })
    }).collect();
    Ok(Json(json!({"sessions": enriched})))
}

/// GET /api/sessions/:id
pub async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db = state.db.lock().await;
    let session = sessions::get_session(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, format!("session {id} not found")))?;
    let msgs = messages::get_messages_by_session(&db, &id, None, None)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let count = messages::count_messages(&db, &id).unwrap_or(0);
    Ok(Json(json!({
        "id": session.id,
        "title": session.title,
        "summary": session.summary,
        "created_at": session.created_at,
        "last_message_at": session.last_message_at,
        "message_count": count,
        "messages": msgs,
    })))
}

/// PATCH /api/sessions/:id — rename
pub async fn rename_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<RenameRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db = state.db.lock().await;
    sessions::update_session_title(&db, &id, &body.title)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let session = sessions::get_session(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, format!("session {id} not found")))?;
    let count = messages::count_messages(&db, &id).unwrap_or(0);
    Ok(Json(json!({
        "session": {
            "id": session.id,
            "title": session.title,
            "summary": session.summary,
            "created_at": session.created_at,
            "last_message_at": session.last_message_at,
            "message_count": count,
        }
    })))
}

/// DELETE /api/sessions — delete all
pub async fn delete_all_sessions(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db = state.db.lock().await;
    db.execute("DELETE FROM messages", [])
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    db.execute("DELETE FROM web_sessions", [])
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({"ok": true})))
}

/// DELETE /api/sessions/:id
pub async fn delete_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db = state.db.lock().await;
    messages::delete_messages_by_session(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    sessions::delete_session(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({"ok": true})))
}
