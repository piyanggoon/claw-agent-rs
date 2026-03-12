//! Tests for web API routes using Axum's built-in test utilities.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use dashmap::DashMap;
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::{broadcast, Mutex};
use tower::ServiceExt; // for oneshot()

use claw_agent_rs::config::ClawConfig;
use claw_agent_rs::context::{ChatMessageEvent, NotificationEvent};
use claw_agent_rs::db;
use claw_agent_rs::memory::MemoryManager;
use claw_agent_rs::scheduler::SchedulerHandle;
use claw_agent_rs::soul::SoulManager;
use claw_agent_rs::web::{server::build_router, state::AppState};

fn setup_app() -> (TempDir, axum::Router) {
    let tmp = TempDir::new().unwrap();
    let soul_dir = tmp.path().join("soul");
    std::fs::create_dir_all(soul_dir.join("memory")).unwrap();
    std::fs::write(soul_dir.join("SOUL.md"), "# Soul\n").unwrap();
    std::fs::write(soul_dir.join("MEMORY.md"), "# Memory\n").unwrap();

    let soul = Arc::new(SoulManager::new(&soul_dir));
    let memory = Arc::new(MemoryManager::new(soul.clone()));

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;").unwrap();
    db::schema::initialize_db(&conn).unwrap();
    let db_conn = Arc::new(Mutex::new(conn));

    let (notification_tx, _) = broadcast::channel::<NotificationEvent>(64);
    let (chat_tx, _) = broadcast::channel::<ChatMessageEvent>(64);

    let (task_events_tx, _) = broadcast::channel::<serde_json::Value>(64);

    // Ensure auth is disabled in tests so all routes are accessible
    let mut config = ClawConfig::from_env();
    config.auth_enabled = false;

    let state = AppState {
        db: db_conn,
        soul,
        memory,
        config: Arc::new(config),
        scheduler: Arc::new(SchedulerHandle::new()),
        active_runs: Arc::new(DashMap::new()),
        abort_handles: Arc::new(DashMap::new()),
        notification_tx,
        chat_tx,
        pending_questions: Arc::new(DashMap::new()),
        run_sessions: Arc::new(DashMap::new()),
        custom_events: Arc::new(DashMap::new()),
        task_events_tx,
        run_accumulators: Arc::new(DashMap::new()),
    };

    let router = build_router(state);
    (tmp, router)
}

// ─── Health ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn health_check() {
    let (_tmp, app) = setup_app();
    let response = app
        .oneshot(Request::get("/api/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ─── Sessions ────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_sessions_empty() {
    let (_tmp, app) = setup_app();
    let response = app
        .oneshot(Request::get("/api/sessions").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(parsed["sessions"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn get_session_not_found() {
    let (_tmp, app) = setup_app();
    let response = app
        .oneshot(Request::get("/api/sessions/nonexistent").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ─── Notifications ───────────────────────────────────────────────────────

#[tokio::test]
async fn list_notifications_empty() {
    let (_tmp, app) = setup_app();
    let response = app
        .oneshot(Request::get("/api/notifications").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ─── Tasks ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_tasks_empty() {
    let (_tmp, app) = setup_app();
    let response = app
        .oneshot(Request::get("/api/tasks").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ─── Soul ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn read_soul_file() {
    let (_tmp, app) = setup_app();
    let response = app
        .oneshot(Request::get("/api/soul/SOUL.md").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("Soul"));
}

#[tokio::test]
async fn write_soul_file() {
    let (_tmp, app) = setup_app();

    // Write (now JSON body with "content" field)
    let write_body = json!({"content": "# Tools\n- OS: macOS\n"});
    let response = app.clone()
        .oneshot(
            Request::put("/api/soul/TOOLS.md")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&write_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Read back
    let response = app
        .oneshot(Request::get("/api/soul/TOOLS.md").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(String::from_utf8(body.to_vec()).unwrap().contains("macOS"));
}

// ─── Chat ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn chat_respond_to_missing_question() {
    let (_tmp, app) = setup_app();

    let body = json!({"question_id": "nonexistent", "response": "test"});
    let response = app
        .oneshot(
            Request::post("/api/chat/respond")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn stop_missing_run() {
    let (_tmp, app) = setup_app();
    let body = json!({"runId": "nonexistent-run-id"});
    let response = app
        .oneshot(
            Request::post("/api/chat/stop")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn stream_missing_run() {
    let (_tmp, app) = setup_app();
    let response = app
        .oneshot(
            Request::get("/api/chat/stream/nonexistent-run-id")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
