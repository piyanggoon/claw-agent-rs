//! Extended tests for web API routes.
//!
//! Covers: Auth, Groups, History, Sessions (extended), Tasks (CRUD),
//! Notifications (extended), Soul (extended), Search, Files, and Chat status.

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

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Build an AppState with in-memory SQLite, temp soul dir, and temp groups dir.
///
/// The groups_dir is set to a temporary directory so that group CRUD tests work
/// without touching the real filesystem. A `default/soul` template directory is
/// created so that `create_group` has something to copy from.
fn setup_app() -> (TempDir, axum::Router, Arc<Mutex<rusqlite::Connection>>) {
    let tmp = TempDir::new().unwrap();

    // Soul dir lives inside a "main" group
    let groups_dir = tmp.path().join("groups");
    let main_group_dir = groups_dir.join("main");
    let soul_dir = main_group_dir.join("soul");
    std::fs::create_dir_all(soul_dir.join("memory")).unwrap();
    std::fs::write(soul_dir.join("SOUL.md"), "# Soul\n").unwrap();
    std::fs::write(soul_dir.join("MEMORY.md"), "# Memory\n").unwrap();

    // Create a default template group so `create_group` can copy from it
    let default_dir = groups_dir.join("default");
    let default_soul = default_dir.join("soul");
    std::fs::create_dir_all(default_soul.join("memory")).unwrap();
    std::fs::write(default_soul.join("SOUL.md"), "# Default Soul\n").unwrap();
    std::fs::write(default_soul.join("IDENTITY.md"), "# Identity\n").unwrap();

    let soul = Arc::new(SoulManager::new(&soul_dir));
    let memory = Arc::new(MemoryManager::new(soul.clone()));

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
        .unwrap();
    db::schema::initialize_db(&conn).unwrap();
    let db_conn = Arc::new(Mutex::new(conn));

    let (notification_tx, _) = broadcast::channel::<NotificationEvent>(64);
    let (chat_tx, _) = broadcast::channel::<ChatMessageEvent>(64);
    let (task_events_tx, _) = broadcast::channel::<serde_json::Value>(64);

    // Data dir (for uploads)
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let mut config = ClawConfig::from_env();
    config.groups_dir = groups_dir;
    config.main_group = "main".to_string();
    config.data_dir = data_dir;
    config.auth_enabled = false; // Disable auth in tests by default

    let state = AppState {
        db: db_conn.clone(),
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
    (tmp, router, db_conn)
}

/// Helper: parse a response body into `serde_json::Value`.
async fn parse_body(response: axum::http::Response<Body>) -> serde_json::Value {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

/// Helper: make a JSON POST request.
fn json_post(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::post(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

/// Helper: make a JSON PATCH request.
fn json_patch(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::patch(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

/// Helper: make a JSON PUT request.
fn json_put(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::put(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

/// Helper: make a GET request.
fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
}

/// Helper: make a DELETE request.
fn delete(uri: &str) -> Request<Body> {
    Request::delete(uri).body(Body::empty()).unwrap()
}

// ═══════════════════════════════════════════════════════════════════════════════
// AUTH
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn auth_status() {
    let (_tmp, app, _db) = setup_app();
    let resp = app.oneshot(get("/api/auth/status")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["auth_enabled"], false);
}

#[tokio::test]
async fn auth_login() {
    let (_tmp, app, _db) = setup_app();
    let resp = app
        .oneshot(json_post("/api/auth/login", json!({"password": "secret"})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["ok"], true);
}

#[tokio::test]
async fn auth_logout() {
    let (_tmp, app, _db) = setup_app();
    let resp = app
        .oneshot(json_post("/api/auth/logout", json!({})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["ok"], true);
}

#[tokio::test]
async fn auth_verify() {
    let (_tmp, app, _db) = setup_app();
    let resp = app.oneshot(get("/api/auth/verify")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["ok"], true);
}

// ═══════════════════════════════════════════════════════════════════════════════
// GROUPS
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn groups_list_empty() {
    let (_tmp, app, _db) = setup_app();
    // The groups dir contains "main" and "default", but list_groups skips "default"
    // and the only other directory is "main". If the handler also skips the main group
    // we might get an empty list; but the handler only skips "default".
    let resp = app.oneshot(get("/api/groups")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert!(body["groups"].is_array());
}

#[tokio::test]
async fn groups_create_and_list() {
    let (_tmp, app, _db) = setup_app();

    // Create a new group
    let resp = app
        .clone()
        .oneshot(json_post(
            "/api/groups",
            json!({
                "name": "Research",
                "folder": "research",
                "trigger": "cron"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["group"]["name"], "Research");
    assert_eq!(body["group"]["folder"], "research");
    assert_eq!(body["group"]["trigger"], "cron");

    // List groups should now include "research"
    let resp = app.oneshot(get("/api/groups")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    let groups = body["groups"].as_array().unwrap();
    let folders: Vec<&str> = groups
        .iter()
        .map(|g| g["folder"].as_str().unwrap())
        .collect();
    assert!(folders.contains(&"research"));
}

#[tokio::test]
async fn groups_create_duplicate_fails() {
    let (_tmp, app, _db) = setup_app();

    // Create group "test-group"
    let resp = app
        .clone()
        .oneshot(json_post(
            "/api/groups",
            json!({"name": "Test", "folder": "test-group"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Attempt to create the same group again — should fail with 409 Conflict
    let resp = app
        .oneshot(json_post(
            "/api/groups",
            json!({"name": "Test", "folder": "test-group"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn groups_delete() {
    let (_tmp, app, _db) = setup_app();

    // Create, then delete
    let resp = app
        .clone()
        .oneshot(json_post(
            "/api/groups",
            json!({"name": "Temp", "folder": "temp-group"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = app
        .clone()
        .oneshot(delete("/api/groups/temp-group"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["ok"], true);
}

#[tokio::test]
async fn groups_delete_default_fails() {
    let (_tmp, app, _db) = setup_app();
    let resp = app
        .oneshot(delete("/api/groups/default"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn groups_delete_main_fails() {
    let (_tmp, app, _db) = setup_app();
    let resp = app
        .oneshot(delete("/api/groups/main"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn groups_delete_nonexistent_fails() {
    let (_tmp, app, _db) = setup_app();
    let resp = app
        .oneshot(delete("/api/groups/nonexistent-group"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ═══════════════════════════════════════════════════════════════════════════════
// HISTORY
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn history_empty() {
    let (_tmp, app, _db) = setup_app();
    let resp = app.oneshot(get("/api/history")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["messages"].as_array().unwrap().len(), 0);
    assert_eq!(body["hasMore"], false);
    assert_eq!(body["total"], 0);
}

#[tokio::test]
async fn history_with_session_and_messages() {
    let (_tmp, app, db) = setup_app();

    // Seed a session and messages
    let session_id = "test-session-1";
    {
        let conn = db.lock().await;
        db::sessions::create_session(&conn, session_id, Some("Test Session"))
            .unwrap();
        db::messages::store_message(
            &conn, "msg-1", "thread-1", "user", "hello world", Some(session_id), None,
        )
        .unwrap();
        db::messages::store_message(
            &conn, "msg-2", "thread-1", "assistant", "hi there", Some(session_id), None,
        )
        .unwrap();
    }

    // Query history for that session
    let resp = app
        .oneshot(get(&format!("/api/history?session={session_id}")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(body["total"], 2);
}

#[tokio::test]
async fn history_delete() {
    let (_tmp, app, db) = setup_app();

    // Seed data
    {
        let conn = db.lock().await;
        db::sessions::create_session(&conn, "s1", None).unwrap();
        db::messages::store_message(&conn, "m1", "t1", "user", "test", Some("s1"), None)
            .unwrap();
    }

    // Delete all history
    let resp = app.clone().oneshot(delete("/api/history")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["ok"], true);

    // Verify empty
    let resp = app.oneshot(get("/api/history?session=s1")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["messages"].as_array().unwrap().len(), 0);
}

// ═══════════════════════════════════════════════════════════════════════════════
// SESSIONS (extended)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn sessions_list_wrapped_in_sessions_key() {
    let (_tmp, app, _db) = setup_app();
    let resp = app.oneshot(get("/api/sessions")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    // Must be wrapped in {sessions: [...]}
    assert!(body["sessions"].is_array());
}

#[tokio::test]
async fn sessions_rename() {
    let (_tmp, app, db) = setup_app();

    let sid = "rename-session-1";
    {
        let conn = db.lock().await;
        db::sessions::create_session(&conn, sid, Some("Original Title")).unwrap();
    }

    let resp = app
        .clone()
        .oneshot(json_patch(
            &format!("/api/sessions/{sid}"),
            json!({"title": "New Title"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["session"]["title"], "New Title");
    assert_eq!(body["session"]["id"], sid);
}

#[tokio::test]
async fn sessions_delete_all() {
    let (_tmp, app, db) = setup_app();

    // Create two sessions
    {
        let conn = db.lock().await;
        db::sessions::create_session(&conn, "s-a", None).unwrap();
        db::sessions::create_session(&conn, "s-b", None).unwrap();
    }

    let resp = app.clone().oneshot(delete("/api/sessions")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["ok"], true);

    // Verify empty
    let resp = app.oneshot(get("/api/sessions")).await.unwrap();
    let body = parse_body(resp).await;
    assert!(body["sessions"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn session_with_message_count() {
    let (_tmp, app, db) = setup_app();

    let sid = "counted-session";
    {
        let conn = db.lock().await;
        db::sessions::create_session(&conn, sid, Some("Counted")).unwrap();
        db::messages::store_message(&conn, "cm-1", "t1", "user", "one", Some(sid), None)
            .unwrap();
        db::messages::store_message(
            &conn, "cm-2", "t1", "assistant", "two", Some(sid), None,
        )
        .unwrap();
        db::messages::store_message(&conn, "cm-3", "t1", "user", "three", Some(sid), None)
            .unwrap();
    }

    // List sessions — each entry should have message_count
    let resp = app.oneshot(get("/api/sessions")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    let sessions = body["sessions"].as_array().unwrap();
    let s = sessions.iter().find(|s| s["id"] == sid).unwrap();
    assert_eq!(s["message_count"], 3);
}

#[tokio::test]
async fn session_get_includes_messages() {
    let (_tmp, app, db) = setup_app();

    let sid = "detail-session";
    {
        let conn = db.lock().await;
        db::sessions::create_session(&conn, sid, Some("Detail")).unwrap();
        db::messages::store_message(&conn, "dm-1", "t1", "user", "hi", Some(sid), None)
            .unwrap();
    }

    let resp = app
        .oneshot(get(&format!("/api/sessions/{sid}")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["id"], sid);
    assert_eq!(body["message_count"], 1);
    assert!(body["messages"].is_array());
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
}

// ═══════════════════════════════════════════════════════════════════════════════
// TASKS (CRUD)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn tasks_create_and_get() {
    let (_tmp, app, _db) = setup_app();

    // Create a task
    let resp = app
        .clone()
        .oneshot(json_post(
            "/api/tasks",
            json!({
                "prompt": "Say hello every day",
                "schedule_type": "cron",
                "schedule_value": "0 9 * * *"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    let task_id = body["task"]["id"].as_str().unwrap().to_string();
    assert_eq!(body["task"]["prompt"], "Say hello every day");
    assert_eq!(body["task"]["schedule_type"], "cron");
    assert_eq!(body["task"]["status"], "active");

    // Get the task by id
    let resp = app
        .oneshot(get(&format!("/api/tasks/{task_id}")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["task"]["id"], task_id);
}

#[tokio::test]
async fn tasks_list_after_create() {
    let (_tmp, app, _db) = setup_app();

    // Create a task
    app.clone()
        .oneshot(json_post(
            "/api/tasks",
            json!({
                "prompt": "Check weather",
                "schedule_type": "interval",
                "schedule_value": "3600000"
            }),
        ))
        .await
        .unwrap();

    // List tasks
    let resp = app.oneshot(get("/api/tasks")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    let tasks = body["tasks"].as_array().unwrap();
    assert!(!tasks.is_empty());
    assert_eq!(tasks[0]["prompt"], "Check weather");
}

#[tokio::test]
async fn tasks_update_status() {
    let (_tmp, app, _db) = setup_app();

    // Create
    let resp = app
        .clone()
        .oneshot(json_post(
            "/api/tasks",
            json!({
                "prompt": "Update me",
                "schedule_type": "once",
                "schedule_value": "2026-12-31T23:59:59"
            }),
        ))
        .await
        .unwrap();
    let body = parse_body(resp).await;
    let task_id = body["task"]["id"].as_str().unwrap().to_string();

    // Patch status
    let resp = app
        .oneshot(json_patch(
            &format!("/api/tasks/{task_id}"),
            json!({"status": "paused"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["task"]["status"], "paused");
}

#[tokio::test]
async fn tasks_delete() {
    let (_tmp, app, _db) = setup_app();

    // Create
    let resp = app
        .clone()
        .oneshot(json_post(
            "/api/tasks",
            json!({
                "prompt": "Delete me",
                "schedule_type": "delay",
                "schedule_value": "60000"
            }),
        ))
        .await
        .unwrap();
    let body = parse_body(resp).await;
    let task_id = body["task"]["id"].as_str().unwrap().to_string();

    // Delete
    let resp = app
        .clone()
        .oneshot(delete(&format!("/api/tasks/{task_id}")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["ok"], true);

    // Verify gone
    let resp = app
        .oneshot(get(&format!("/api/tasks/{task_id}")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn tasks_pause() {
    let (_tmp, app, db) = setup_app();

    // Seed a task directly
    let task_id = "pause-task-1";
    {
        let conn = db.lock().await;
        db::tasks::create_task(
            &conn,
            &db::tasks::ScheduledTask {
                id: task_id.to_string(),
                group_folder: "main".to_string(),
                prompt: "test".to_string(),
                schedule_type: "interval".to_string(),
                schedule_value: "60000".to_string(),
                context_mode: "group".to_string(),
                context_session: None,
                next_run: None,
                last_run: None,
                last_result: None,
                status: "active".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        )
        .unwrap();
    }

    let resp = app
        .oneshot(json_post(&format!("/api/tasks/{task_id}/pause"), json!({})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["ok"], true);

    // Verify status in DB
    let conn = db.lock().await;
    let task = db::tasks::get_task(&conn, task_id).unwrap().unwrap();
    assert_eq!(task.status, "paused");
}

#[tokio::test]
async fn tasks_resume() {
    let (_tmp, app, db) = setup_app();

    let task_id = "resume-task-1";
    {
        let conn = db.lock().await;
        db::tasks::create_task(
            &conn,
            &db::tasks::ScheduledTask {
                id: task_id.to_string(),
                group_folder: "main".to_string(),
                prompt: "test".to_string(),
                schedule_type: "interval".to_string(),
                schedule_value: "60000".to_string(),
                context_mode: "group".to_string(),
                context_session: None,
                next_run: None,
                last_run: None,
                last_result: None,
                status: "paused".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        )
        .unwrap();
    }

    let resp = app
        .oneshot(json_post(&format!("/api/tasks/{task_id}/resume"), json!({})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["ok"], true);

    let conn = db.lock().await;
    let task = db::tasks::get_task(&conn, task_id).unwrap().unwrap();
    assert_eq!(task.status, "active");
}

#[tokio::test]
async fn tasks_cancel() {
    let (_tmp, app, db) = setup_app();

    let task_id = "cancel-task-1";
    {
        let conn = db.lock().await;
        db::tasks::create_task(
            &conn,
            &db::tasks::ScheduledTask {
                id: task_id.to_string(),
                group_folder: "main".to_string(),
                prompt: "cancel me".to_string(),
                schedule_type: "cron".to_string(),
                schedule_value: "0 0 * * *".to_string(),
                context_mode: "group".to_string(),
                context_session: None,
                next_run: None,
                last_run: None,
                last_result: None,
                status: "active".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        )
        .unwrap();
    }

    let resp = app
        .oneshot(json_post(
            &format!("/api/tasks/{task_id}/cancel"),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["ok"], true);

    // Task should be deleted
    let conn = db.lock().await;
    let task = db::tasks::get_task(&conn, task_id).unwrap();
    assert!(task.is_none());
}

#[tokio::test]
async fn tasks_logs_for_task() {
    let (_tmp, app, db) = setup_app();

    let task_id = "logs-task-1";
    {
        let conn = db.lock().await;
        db::tasks::create_task(
            &conn,
            &db::tasks::ScheduledTask {
                id: task_id.to_string(),
                group_folder: "main".to_string(),
                prompt: "log test".to_string(),
                schedule_type: "interval".to_string(),
                schedule_value: "60000".to_string(),
                context_mode: "group".to_string(),
                context_session: None,
                next_run: None,
                last_run: None,
                last_result: None,
                status: "active".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        )
        .unwrap();
        // Insert a log entry
        db::tasks::log_task_run(&conn, task_id, 150, "success", Some("done"), None)
            .unwrap();
    }

    let resp = app
        .oneshot(get(&format!("/api/tasks/{task_id}/logs")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    let logs = body["logs"].as_array().unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0]["task_id"], task_id);
    assert_eq!(logs[0]["status"], "success");
}

#[tokio::test]
async fn tasks_logs_for_nonexistent_task() {
    let (_tmp, app, _db) = setup_app();
    let resp = app
        .oneshot(get("/api/tasks/no-such-task/logs"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn tasks_all_logs() {
    let (_tmp, app, db) = setup_app();

    // Create a task and log entries
    let task_id = "all-logs-task";
    {
        let conn = db.lock().await;
        db::tasks::create_task(
            &conn,
            &db::tasks::ScheduledTask {
                id: task_id.to_string(),
                group_folder: "main".to_string(),
                prompt: "all logs test".to_string(),
                schedule_type: "interval".to_string(),
                schedule_value: "60000".to_string(),
                context_mode: "group".to_string(),
                context_session: None,
                next_run: None,
                last_run: None,
                last_result: None,
                status: "active".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        )
        .unwrap();
        db::tasks::log_task_run(&conn, task_id, 100, "success", Some("ok"), None)
            .unwrap();
        db::tasks::log_task_run(&conn, task_id, 200, "error", None, Some("timeout"))
            .unwrap();
    }

    let resp = app.oneshot(get("/api/tasks/logs")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    let logs = body["logs"].as_array().unwrap();
    assert_eq!(logs.len(), 2);
}

#[tokio::test]
async fn tasks_get_nonexistent() {
    let (_tmp, app, _db) = setup_app();
    let resp = app
        .oneshot(get("/api/tasks/nonexistent-task-id"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn tasks_delete_nonexistent() {
    let (_tmp, app, _db) = setup_app();
    let resp = app
        .oneshot(delete("/api/tasks/nonexistent-task-id"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ═══════════════════════════════════════════════════════════════════════════════
// NOTIFICATIONS (extended)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn notifications_list_with_unread_count() {
    let (_tmp, app, db) = setup_app();

    // Seed a notification
    {
        let conn = db.lock().await;
        db::notifications::create_notification(
            &conn, "notif-1", "Test Alert", "Something happened", "info", "system", None,
        )
        .unwrap();
    }

    let resp = app.oneshot(get("/api/notifications")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    let notifs = body["notifications"].as_array().unwrap();
    assert_eq!(notifs.len(), 1);
    assert_eq!(notifs[0]["title"], "Test Alert");
    assert_eq!(body["unreadCount"], 1);
}

#[tokio::test]
async fn notifications_mark_read() {
    let (_tmp, app, db) = setup_app();

    {
        let conn = db.lock().await;
        db::notifications::create_notification(
            &conn, "notif-2", "Read Me", "Please", "info", "system", None,
        )
        .unwrap();
    }

    // Mark as read (PATCH)
    let resp = app
        .clone()
        .oneshot(json_patch("/api/notifications/notif-2/read", json!({})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["ok"], true);

    // Verify unread count is now 0
    let resp = app.oneshot(get("/api/notifications")).await.unwrap();
    let body = parse_body(resp).await;
    assert_eq!(body["unreadCount"], 0);
}

#[tokio::test]
async fn notifications_mark_all_read() {
    let (_tmp, app, db) = setup_app();

    // Create multiple unread notifications
    {
        let conn = db.lock().await;
        db::notifications::create_notification(
            &conn, "n-a", "Alert A", "msg a", "info", "system", None,
        )
        .unwrap();
        db::notifications::create_notification(
            &conn, "n-b", "Alert B", "msg b", "warning", "task", None,
        )
        .unwrap();
        db::notifications::create_notification(
            &conn, "n-c", "Alert C", "msg c", "error", "agent", None,
        )
        .unwrap();
    }

    // Verify 3 unread
    let resp = app.clone().oneshot(get("/api/notifications")).await.unwrap();
    let body = parse_body(resp).await;
    assert_eq!(body["unreadCount"], 3);

    // Mark all read (POST)
    let resp = app
        .clone()
        .oneshot(json_post("/api/notifications/read-all", json!({})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["ok"], true);

    // Verify 0 unread
    let resp = app.oneshot(get("/api/notifications")).await.unwrap();
    let body = parse_body(resp).await;
    assert_eq!(body["unreadCount"], 0);
}

#[tokio::test]
async fn notifications_delete() {
    let (_tmp, app, db) = setup_app();

    {
        let conn = db.lock().await;
        db::notifications::create_notification(
            &conn, "del-notif", "Delete Me", "bye", "info", "system", None,
        )
        .unwrap();
    }

    let resp = app
        .clone()
        .oneshot(delete("/api/notifications/del-notif"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify notification is gone
    let resp = app.oneshot(get("/api/notifications")).await.unwrap();
    let body = parse_body(resp).await;
    assert_eq!(body["notifications"].as_array().unwrap().len(), 0);
}

// ═══════════════════════════════════════════════════════════════════════════════
// SOUL (extended)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn soul_list_files() {
    let (_tmp, app, _db) = setup_app();

    let resp = app.oneshot(get("/api/soul")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    let files = body["files"].as_array().unwrap();
    // Should contain SOUL.md and MEMORY.md at minimum
    let paths: Vec<&str> = files.iter().map(|f| f["path"].as_str().unwrap()).collect();
    assert!(paths.contains(&"SOUL.md"));
    assert!(paths.contains(&"MEMORY.md"));
}

#[tokio::test]
async fn soul_write_and_read() {
    let (_tmp, app, _db) = setup_app();

    // Write
    let resp = app
        .clone()
        .oneshot(json_put(
            "/api/soul/TEST.md",
            json!({"content": "# Test File\nHello world\n"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["ok"], true);
    assert_eq!(body["filename"], "TEST.md");
    assert!(body["size"].as_u64().unwrap() > 0);

    // Read back
    let resp = app.oneshot(get("/api/soul/TEST.md")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["filename"], "TEST.md");
    assert!(body["content"].as_str().unwrap().contains("Hello world"));
}

#[tokio::test]
async fn soul_read_nonexistent() {
    let (_tmp, app, _db) = setup_app();
    let resp = app
        .oneshot(get("/api/soul/NONEXISTENT.md"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn soul_delete_bootstrap() {
    let (_tmp, app, _db) = setup_app();

    // First create BOOTSTRAP.md
    let resp = app
        .clone()
        .oneshot(json_put(
            "/api/soul/BOOTSTRAP.md",
            json!({"content": "# Bootstrap\nSetup\n"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Delete it
    let resp = app.oneshot(delete("/api/soul/BOOTSTRAP.md")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["ok"], true);
}

#[tokio::test]
async fn soul_delete_non_bootstrap_forbidden() {
    let (_tmp, app, _db) = setup_app();
    let resp = app.oneshot(delete("/api/soul/SOUL.md")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn soul_memory_search() {
    let (_tmp, app, _db) = setup_app();

    let resp = app
        .oneshot(get("/api/soul/memory/search"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    // The results field should exist (string from memory recall)
    assert!(body.get("results").is_some());
}

#[tokio::test]
async fn soul_memory_search_with_query() {
    let (_tmp, app, _db) = setup_app();

    let resp = app
        .oneshot(get("/api/soul/memory/search?q=test&days=3"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert!(body.get("results").is_some());
}

#[tokio::test]
async fn soul_memory_daily_logs() {
    let (_tmp, app, _db) = setup_app();

    let resp = app
        .oneshot(get("/api/soul/memory/daily"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert!(body["logs"].is_array());
}

#[tokio::test]
async fn soul_memory_daily_logs_with_file() {
    let (tmp, app, _db) = setup_app();

    // Create a daily log file in the memory subdirectory
    let memory_dir = tmp.path().join("groups/main/soul/memory");
    std::fs::write(
        memory_dir.join("2026-03-12.md"),
        "# Daily Log\n- Something happened\n",
    )
    .unwrap();

    let resp = app
        .oneshot(get("/api/soul/memory/daily"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    let logs = body["logs"].as_array().unwrap();
    assert!(!logs.is_empty());
    let dates: Vec<&str> = logs.iter().map(|l| l["date"].as_str().unwrap()).collect();
    assert!(dates.contains(&"2026-03-12"));
}

// ═══════════════════════════════════════════════════════════════════════════════
// SEARCH
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn search_empty_query() {
    let (_tmp, app, _db) = setup_app();
    let resp = app.oneshot(get("/api/search")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["results"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn search_no_match() {
    let (_tmp, app, _db) = setup_app();
    let resp = app
        .oneshot(get("/api/search?q=xyznonexistent"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["results"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn search_with_results() {
    let (_tmp, app, db) = setup_app();

    // Seed messages with searchable content
    let sid = "search-session";
    {
        let conn = db.lock().await;
        db::sessions::create_session(&conn, sid, Some("Search Test")).unwrap();
        db::messages::store_message(
            &conn, "sm-1", "t1", "user", "hello world greeting", Some(sid), None,
        )
        .unwrap();
        db::messages::store_message(
            &conn, "sm-2", "t1", "assistant", "hello back to you", Some(sid), None,
        )
        .unwrap();
    }

    let resp = app.oneshot(get("/api/search?q=hello")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    let results = body["results"].as_array().unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0]["sessionId"], sid);
}

// ═══════════════════════════════════════════════════════════════════════════════
// FILES
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn file_serve_nonexistent() {
    let (_tmp, app, _db) = setup_app();
    let resp = app
        .oneshot(get("/api/file?path=/tmp/nonexistent_file_xyz.txt"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn file_serve_existing() {
    let (tmp, app, _db) = setup_app();

    // Create a file to serve
    let test_file = tmp.path().join("test-serve.txt");
    std::fs::write(&test_file, "file content here").unwrap();

    let resp = app
        .oneshot(get(&format!(
            "/api/file?path={}",
            test_file.to_string_lossy()
        )))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(std::str::from_utf8(&body).unwrap(), "file content here");
}

#[tokio::test]
async fn upload_file_multipart() {
    let (_tmp, app, _db) = setup_app();

    // Build a simple multipart body
    let boundary = "----TestBoundary123";
    let body_str = format!(
        "--{boundary}\r\n\
         Content-Disposition: form-data; name=\"file\"; filename=\"test.txt\"\r\n\
         Content-Type: text/plain\r\n\
         \r\n\
         Hello from upload!\r\n\
         --{boundary}--\r\n"
    );

    let resp = app
        .oneshot(
            Request::post("/api/upload")
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body_str))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    let files = body["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["filename"], "test.txt");
    assert!(files[0]["path"].as_str().unwrap().contains("test.txt"));
}

// ═══════════════════════════════════════════════════════════════════════════════
// CHAT (extended)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn chat_status_no_active_runs() {
    let (_tmp, app, _db) = setup_app();
    let resp = app.oneshot(get("/api/chat/status")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["running"], false);
    assert_eq!(body["runs"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn chat_status_with_active_run() {
    // Rebuild with a seeded run_sessions map so chat_status reports running=true
    let tmp = TempDir::new().unwrap();
    let groups_dir = tmp.path().join("groups");
    let soul_dir = groups_dir.join("main/soul");
    std::fs::create_dir_all(soul_dir.join("memory")).unwrap();
    std::fs::write(soul_dir.join("SOUL.md"), "# Soul\n").unwrap();
    std::fs::write(soul_dir.join("MEMORY.md"), "# Memory\n").unwrap();

    let soul = Arc::new(SoulManager::new(&soul_dir));
    let memory = Arc::new(MemoryManager::new(soul.clone()));
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
        .unwrap();
    db::schema::initialize_db(&conn).unwrap();
    let db_conn = Arc::new(Mutex::new(conn));
    let (notification_tx, _) = broadcast::channel::<NotificationEvent>(64);
    let (chat_tx, _) = broadcast::channel::<ChatMessageEvent>(64);
    let (task_events_tx, _) = broadcast::channel::<serde_json::Value>(64);

    let mut config = ClawConfig::from_env();
    config.groups_dir = groups_dir;
    config.main_group = "main".to_string();
    config.data_dir = tmp.path().join("data");
    config.auth_enabled = false; // Disable auth in tests

    let run_sessions = Arc::new(DashMap::new());
    run_sessions.insert("run-abc".to_string(), "session-xyz".to_string());

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
        run_sessions,
        custom_events: Arc::new(DashMap::new()),
        task_events_tx,
        run_accumulators: Arc::new(DashMap::new()),
    };

    let router = build_router(state);

    let resp = router
        .oneshot(get("/api/chat/status"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["running"], true);
    let runs = body["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
}
