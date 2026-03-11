use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

use crate::db::messages;
use crate::db::sessions;
use crate::web::state::AppState;

// ── Response types ───────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct SessionWithMessages {
    #[serde(flatten)]
    pub session: sessions::WebSession,
    pub messages: Vec<messages::StoredMessage>,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// GET /api/sessions
///
/// Returns all web sessions ordered by most recently active first.
pub async fn list_sessions(
    State(state): State<AppState>,
) -> Result<Json<Vec<sessions::WebSession>>, (StatusCode, String)> {
    let db = state.db.lock().await;
    let result = sessions::get_sessions(&db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(result))
}

/// GET /api/sessions/:id
///
/// Returns a single session along with all its messages.
pub async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SessionWithMessages>, (StatusCode, String)> {
    let db = state.db.lock().await;

    let session = sessions::get_session(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, format!("session {id} not found")))?;

    let msgs = messages::get_messages_by_session(&db, &id, None, None)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(SessionWithMessages {
        session,
        messages: msgs,
    }))
}

/// DELETE /api/sessions/:id
///
/// Deletes a session and all of its associated messages.
pub async fn delete_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let db = state.db.lock().await;

    // Delete messages first to satisfy foreign-key-like semantics.
    messages::delete_messages_by_session(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    sessions::delete_session(&db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::OK)
}
