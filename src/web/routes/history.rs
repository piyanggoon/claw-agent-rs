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

    let has_more = msgs.len() == limit as usize;

    let formatted: Vec<serde_json::Value> = msgs.iter().map(|m| {
        let mut msg = json!({
            "id": m.id,
            "role": m.role,
            "content": m.content,
            "timestamp": m.timestamp,
            "web_session_id": m.web_session_id,
        });
        if let Some(meta) = &m.metadata {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(meta) {
                // Merge metadata fields into the message
                if let Some(tool_calls) = parsed.get("toolCalls") {
                    msg["toolCalls"] = tool_calls.clone();
                }
                if let Some(result_meta) = parsed.get("resultMeta") {
                    msg["resultMeta"] = result_meta.clone();
                }
            }
        }
        msg
    }).collect();

    Ok(Json(json!({
        "messages": formatted,
        "hasMore": has_more,
        "total": total,
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
