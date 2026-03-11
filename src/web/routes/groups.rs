use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::web::state::AppState;

#[derive(Deserialize)]
pub struct CreateGroupRequest {
    pub name: String,
    pub folder: String,
    pub trigger: Option<String>,
}

/// GET /api/groups
pub async fn list_groups(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let groups_dir = &state.config.groups_dir;
    let mut groups = Vec::new();

    let entries = std::fs::read_dir(groups_dir)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    for entry in entries {
        let entry = entry.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        if !entry.path().is_dir() {
            continue;
        }
        let folder = entry.file_name().to_string_lossy().into_owned();
        if folder == "default" {
            continue;
        }
        groups.push(json!({
            "name": folder.clone(),
            "folder": folder,
            "trigger": "direct",
        }));
    }

    Ok(Json(json!({"groups": groups})))
}

/// POST /api/groups — create a new group from default template
pub async fn create_group(
    State(state): State<AppState>,
    Json(body): Json<CreateGroupRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let groups_dir = &state.config.groups_dir;
    let target = groups_dir.join(&body.folder);

    if target.exists() {
        return Err((StatusCode::CONFLICT, format!("group '{}' already exists", body.folder)));
    }

    // Copy from default template
    let default_dir = groups_dir.join("default");
    let default_soul = default_dir.join("soul");

    std::fs::create_dir_all(target.join("soul"))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Copy soul files
    if default_soul.exists() {
        if let Ok(entries) = std::fs::read_dir(&default_soul) {
            for entry in entries.flatten() {
                if entry.path().is_file() {
                    let _ = std::fs::copy(entry.path(), target.join("soul").join(entry.file_name()));
                }
            }
        }
    }

    // Copy group-level files
    if let Ok(entries) = std::fs::read_dir(&default_dir) {
        for entry in entries.flatten() {
            if entry.path().is_file() {
                let _ = std::fs::copy(entry.path(), target.join(entry.file_name()));
            }
        }
    }

    std::fs::create_dir_all(target.join("soul").join("memory"))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let trigger = body.trigger.unwrap_or_else(|| "direct".to_string());

    Ok(Json(json!({
        "group": {
            "name": body.name,
            "folder": body.folder,
            "trigger": trigger,
        }
    })))
}

/// DELETE /api/groups/:folder
pub async fn delete_group(
    State(state): State<AppState>,
    Path(folder): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if folder == "default" || folder == state.config.main_group {
        return Err((StatusCode::BAD_REQUEST, "cannot delete default or main group".to_string()));
    }

    let target = state.config.groups_dir.join(&folder);
    if !target.exists() {
        return Err((StatusCode::NOT_FOUND, format!("group '{folder}' not found")));
    }

    std::fs::remove_dir_all(&target)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({"ok": true})))
}
