use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::web::state::AppState;

#[derive(Deserialize)]
pub struct FileQuery {
    pub path: String,
}

/// GET /api/file?path= — serve a file
pub async fn serve_file(
    Query(params): Query<FileQuery>,
) -> Result<Response<Body>, (StatusCode, String)> {
    let path = std::path::Path::new(&params.path);

    if !path.exists() {
        return Err((StatusCode::NOT_FOUND, "file not found".to_string()));
    }

    let bytes = tokio::fs::read(path).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let content_type = match path.extension().and_then(|e| e.to_str()) {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        Some("json") => "application/json",
        Some("js") => "application/javascript",
        Some("css") => "text/css",
        Some("html" | "htm") => "text/html",
        Some("md" | "txt") => "text/plain",
        _ => "application/octet-stream",
    };

    Ok(Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "public, max-age=3600")
        .body(Body::from(bytes))
        .unwrap())
}

/// POST /api/upload — file upload via multipart
pub async fn upload_file(
    State(state): State<AppState>,
    mut multipart: axum::extract::Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let upload_dir = state.config.data_dir.join("uploads");
    std::fs::create_dir_all(&upload_dir)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut files = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let filename = field.file_name()
            .map(|f| f.to_string())
            .unwrap_or_else(|| format!("{}.bin", uuid::Uuid::new_v4()));

        let data = field.bytes().await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

        let unique_name = format!("{}_{}", uuid::Uuid::new_v4(), filename);
        let dest = upload_dir.join(&unique_name);

        tokio::fs::write(&dest, &data).await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        files.push(json!({
            "filename": filename,
            "path": dest.to_string_lossy(),
            "url": format!("/api/file?path={}", dest.to_string_lossy()),
        }));
    }

    Ok(Json(json!({"files": files})))
}
