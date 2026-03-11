use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::web::state::AppState;

// ── Query / Response types ───────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct MemorySearchQuery {
    /// Search query string. If omitted, returns all memory content.
    pub q: Option<String>,
    /// Number of recent daily log days to include (default: 7).
    pub days: Option<u32>,
}

#[derive(Serialize)]
pub struct MemorySearchResponse {
    pub results: String,
}

#[derive(Serialize)]
pub struct SoulFileResponse {
    pub filename: String,
    pub content: String,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// GET /api/soul/*filename
///
/// Reads a soul file from disk and returns its content as JSON.
///
/// Examples:
/// - `GET /api/soul/SOUL.md`
/// - `GET /api/soul/memory/2026-03-12.md`
pub async fn read_soul(
    State(state): State<AppState>,
    Path(filename): Path<String>,
) -> Result<Json<SoulFileResponse>, (StatusCode, String)> {
    let content = state
        .soul
        .read(&filename)
        .await
        .map_err(|e| {
            if e.to_string().contains("failed to read soul file") {
                (StatusCode::NOT_FOUND, format!("soul file not found: {filename}"))
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
            }
        })?;

    Ok(Json(SoulFileResponse { filename, content }))
}

/// PUT /api/soul/*filename
///
/// Writes content to a soul file. The request body is treated as raw text
/// (the entire body becomes the file content).
///
/// Examples:
/// - `PUT /api/soul/SOUL.md` with body containing the markdown content
pub async fn write_soul(
    State(state): State<AppState>,
    Path(filename): Path<String>,
    body: Bytes,
) -> Result<StatusCode, (StatusCode, String)> {
    let content = String::from_utf8(body.to_vec()).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("invalid UTF-8 in request body: {e}"),
        )
    })?;

    state
        .soul
        .write(&filename, &content)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::OK)
}

/// GET /api/soul/memory/search
///
/// Searches MEMORY.md and recent daily logs for matching content.
///
/// Query parameters:
/// - `q` (optional): search query for case-insensitive substring matching.
/// - `days` (optional, default 7): how many days of daily logs to include.
pub async fn search_memory(
    State(state): State<AppState>,
    Query(params): Query<MemorySearchQuery>,
) -> Result<Json<MemorySearchResponse>, (StatusCode, String)> {
    let days = params.days.unwrap_or(7);
    let results = state
        .memory
        .recall(params.q.as_deref(), days)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(MemorySearchResponse { results }))
}
