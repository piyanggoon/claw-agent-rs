use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::db::{messages, sessions};
use crate::web::state::AppState;

#[derive(Deserialize)]
pub struct HistoryQuery {
    pub limit: Option<u32>,
    pub session: Option<String>,
    pub date: Option<String>,
    pub before: Option<String>,
    pub paginate: Option<String>,
}

/// GET /api/history
///
/// Returns messages for a session. The `metadata` field is returned as a raw
/// JSON string so the frontend can parse it with `JSON.parse(m.metadata)`.
///
/// - Without `before`: Returns the N most recent messages.
/// - With `before` (ISO timestamp): Returns N messages older than `before`.
/// - `hasMore`: true if there are older messages to load.
/// - `sessionStats`: aggregated from ALL messages in the session (not just the page).
pub async fn get_history(
    State(state): State<AppState>,
    Query(params): Query<HistoryQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db = state.db.lock().await;
    let limit = params.limit.unwrap_or(100);

    let session_id = params.session.or(params.date).or_else(|| {
        sessions::get_sessions(&db).ok()
            .and_then(|s| s.first().map(|s| s.id.clone()))
    });

    let msgs = if let Some(sid) = &session_id {
        messages::get_messages_paginated(&db, sid, limit, params.before.as_deref())
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        Vec::new()
    };

    let total = if let Some(sid) = &session_id {
        messages::count_messages(&db, sid).unwrap_or(0)
    } else {
        0
    };

    // Check if there are older messages (for "load more" / infinite scroll)
    let has_more = if !msgs.is_empty() {
        if let Some(sid) = &session_id {
            let oldest_ts = &msgs[0].timestamp; // msgs are chronological (oldest first)
            messages::has_messages_before(&db, sid, oldest_ts).unwrap_or(false)
        } else {
            false
        }
    } else {
        false
    };

    // Session stats from ALL messages (not just the current page)
    let (total_cost, total_input, total_output, turns) = if let Some(sid) = &session_id {
        messages::get_session_stats(&db, sid).unwrap_or((0.0, 0, 0, 0))
    } else {
        (0.0, 0, 0, 0)
    };

    let formatted: Vec<serde_json::Value> = msgs.iter().map(|m| {
        // Return message with metadata as raw JSON string (frontend parses it)
        json!({
            "id": m.id,
            "role": m.role,
            "content": m.content,
            "timestamp": m.timestamp,
            "web_session_id": m.web_session_id,
            "metadata": m.metadata,
        })
    }).collect();

    Ok(Json(json!({
        "messages": formatted,
        "hasMore": has_more,
        "total": total,
        "sessionStats": {
            "totalCost": total_cost,
            "totalInput": total_input,
            "totalOutput": total_output,
            "turns": turns,
            "lastModel": null,
        }
    })))
}

/// DELETE /api/history
pub async fn delete_history(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db = state.db.lock().await;
    db.execute("DELETE FROM messages", [])
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({"ok": true})))
}
