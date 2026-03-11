use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::web::state::AppState;

// ── Query / Request types ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct MemorySearchQuery {
    pub q: Option<String>,
    pub days: Option<u32>,
}

#[derive(Deserialize)]
pub struct SoulGroupQuery {
    pub group: Option<String>,
}

#[derive(Deserialize)]
pub struct WriteSoulBody {
    pub content: String,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// GET /api/soul — list all soul files
pub async fn list_soul_files(
    State(state): State<AppState>,
    Query(params): Query<SoulGroupQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let soul_dir = if let Some(group) = &params.group {
        state.config.groups_dir.join(group).join("soul")
    } else {
        state.config.soul_dir()
    };

    let mut files = Vec::new();
    if soul_dir.exists() {
        collect_soul_files(&soul_dir, &soul_dir, &mut files);
    }

    Ok(Json(json!({ "files": files })))
}

fn collect_soul_files(
    base: &std::path::Path,
    dir: &std::path::Path,
    files: &mut Vec<serde_json::Value>,
) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_soul_files(base, &path, files);
            } else if path.is_file() {
                let relative = path.strip_prefix(base).unwrap_or(&path);
                let name = relative.to_string_lossy().to_string();
                let meta = std::fs::metadata(&path);
                let (size, modified_at) = if let Ok(m) = meta {
                    let size = m.len();
                    let modified = m.modified().ok().map(|t| {
                        let dt: chrono::DateTime<chrono::Utc> = t.into();
                        dt.to_rfc3339()
                    });
                    (size, modified)
                } else {
                    (0, None)
                };
                files.push(json!({
                    "path": name,
                    "name": path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                    "size": size,
                    "modified_at": modified_at,
                }));
            }
        }
    }
}

/// GET /api/soul/*filename — read a soul file
pub async fn read_soul(
    State(state): State<AppState>,
    Path(filename): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
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

    Ok(Json(json!({
        "filename": filename,
        "content": content,
        "path": filename,
    })))
}

/// PUT /api/soul/*filename — write a soul file (JSON body: { content })
pub async fn write_soul(
    State(state): State<AppState>,
    Path(filename): Path<String>,
    Json(body): Json<WriteSoulBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    state
        .soul
        .write(&filename, &body.content)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Get file metadata after write
    let soul_path = state.config.soul_dir().join(&filename);
    let (size, modified_at) = if let Ok(m) = std::fs::metadata(&soul_path) {
        let modified = m.modified().ok().map(|t| {
            let dt: chrono::DateTime<chrono::Utc> = t.into();
            dt.to_rfc3339()
        });
        (m.len(), modified)
    } else {
        (body.content.len() as u64, None)
    };

    Ok(Json(json!({
        "ok": true,
        "filename": filename,
        "size": size,
        "modified_at": modified_at,
    })))
}

/// DELETE /api/soul/*filename — delete a soul file (BOOTSTRAP.md only)
pub async fn delete_soul(
    State(state): State<AppState>,
    Path(filename): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Safety: only allow deleting BOOTSTRAP.md
    let base_name = std::path::Path::new(&filename)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    if base_name != "BOOTSTRAP.md" {
        return Err((
            StatusCode::FORBIDDEN,
            "Only BOOTSTRAP.md can be deleted".to_string(),
        ));
    }

    let soul_path = state.config.soul_dir().join(&filename);
    if soul_path.exists() {
        std::fs::remove_file(&soul_path)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    Ok(Json(json!({ "ok": true })))
}

/// GET /api/soul/memory/search
pub async fn search_memory(
    State(state): State<AppState>,
    Query(params): Query<MemorySearchQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let days = params.days.unwrap_or(7);
    let results = state
        .memory
        .recall(params.q.as_deref(), days)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({ "results": results })))
}

/// GET /api/soul/memory/daily — list daily log files
pub async fn list_daily_logs(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let memory_dir = state.config.soul_dir().join("memory");
    let mut logs = Vec::new();

    if memory_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&memory_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(name) = path.file_name() {
                        let filename = name.to_string_lossy().to_string();
                        if filename.ends_with(".md") {
                            let meta = std::fs::metadata(&path);
                            let (size, modified_at) = if let Ok(m) = meta {
                                let modified = m.modified().ok().map(|t| {
                                    let dt: chrono::DateTime<chrono::Utc> = t.into();
                                    dt.to_rfc3339()
                                });
                                (m.len(), modified)
                            } else {
                                (0, None)
                            };
                            // Extract date from filename like "2026-03-12.md"
                            let date = filename.trim_end_matches(".md").to_string();
                            logs.push(json!({
                                "filename": filename,
                                "date": date,
                                "size": size,
                                "modified_at": modified_at,
                            }));
                        }
                    }
                }
            }
        }
    }

    // Sort by date descending
    logs.sort_by(|a, b| {
        let da = a["date"].as_str().unwrap_or("");
        let db_val = b["date"].as_str().unwrap_or("");
        db_val.cmp(da)
    });

    Ok(Json(json!({ "logs": logs })))
}
