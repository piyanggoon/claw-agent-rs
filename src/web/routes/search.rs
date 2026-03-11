use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::web::state::AppState;

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
}

/// GET /api/search?q=
pub async fn search(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let query = params.q.unwrap_or_default();
    if query.is_empty() {
        return Ok(Json(json!({"results": []})));
    }

    let db = state.db.lock().await;
    let pattern = format!("%{query}%");
    let mut stmt = db.prepare(
        "SELECT m.web_session_id, m.content, m.timestamp, COUNT(*) as match_count \
         FROM messages m \
         WHERE m.content LIKE ?1 AND m.web_session_id IS NOT NULL \
         GROUP BY m.web_session_id \
         ORDER BY MAX(m.timestamp) DESC \
         LIMIT 50"
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let results: Vec<serde_json::Value> = stmt.query_map(
        rusqlite::params![pattern],
        |row| {
            let session_id: String = row.get(0)?;
            let content: String = row.get(1)?;
            let timestamp: String = row.get(2)?;
            let match_count: u32 = row.get(3)?;
            let preview = if content.len() > 200 {
                format!("{}...", &content[..200])
            } else {
                content
            };
            Ok(json!({
                "sessionId": session_id,
                "sessionDate": timestamp,
                "matchCount": match_count,
                "preview": preview,
            }))
        }
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .filter_map(|r| r.ok())
    .collect();

    Ok(Json(json!({"results": results})))
}
