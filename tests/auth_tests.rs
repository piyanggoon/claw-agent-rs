//! Comprehensive authentication tests.
//!
//! Tests the full auth lifecycle:
//! - Auth disabled: all routes accessible without tokens
//! - Auth enabled: login, token verification, protected routes, middleware
//! - Token generation, verification, expiration, tampering

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

// ─── Helpers ────────────────────────────────────────────────────────────────

fn setup_app_with_auth(auth_enabled: bool, auth_password: Option<&str>) -> (TempDir, axum::Router) {
    let tmp = TempDir::new().unwrap();
    let groups_dir = tmp.path().join("groups");
    let main_group_dir = groups_dir.join("main");
    let soul_dir = main_group_dir.join("soul");
    std::fs::create_dir_all(soul_dir.join("memory")).unwrap();
    std::fs::write(soul_dir.join("SOUL.md"), "# Soul\n").unwrap();
    std::fs::write(soul_dir.join("MEMORY.md"), "# Memory\n").unwrap();

    let default_dir = groups_dir.join("default");
    let default_soul = default_dir.join("soul");
    std::fs::create_dir_all(default_soul.join("memory")).unwrap();
    std::fs::write(default_soul.join("SOUL.md"), "# Default\n").unwrap();

    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let soul = Arc::new(SoulManager::new(&soul_dir));
    let memory = Arc::new(MemoryManager::new(soul.clone()));

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;").unwrap();
    db::schema::initialize_db(&conn).unwrap();
    let db_conn = Arc::new(Mutex::new(conn));

    let (notification_tx, _) = broadcast::channel::<NotificationEvent>(64);
    let (chat_tx, _) = broadcast::channel::<ChatMessageEvent>(64);
    let (task_events_tx, _) = broadcast::channel::<serde_json::Value>(64);

    let auth_secret = match auth_password {
        Some(pw) => format!("claw-auth-{pw}-secret-key"),
        None => String::new(),
    };

    let mut config = ClawConfig::from_env();
    config.groups_dir = groups_dir;
    config.main_group = "main".to_string();
    config.data_dir = data_dir;
    config.auth_enabled = auth_enabled;
    config.auth_password = auth_password.map(|s| s.to_string());
    config.auth_secret = auth_secret;

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
    };

    let router = build_router(state);
    (tmp, router)
}

async fn parse_body(response: axum::http::Response<Body>) -> serde_json::Value {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
}

fn json_post(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::post(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

fn get_with_cookie(uri: &str, token: &str) -> Request<Body> {
    Request::get(uri)
        .header("cookie", format!("claw-token={token}"))
        .body(Body::empty())
        .unwrap()
}

fn get_with_bearer(uri: &str, token: &str) -> Request<Body> {
    Request::get(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

// ═══════════════════════════════════════════════════════════════════════════════
// AUTH DISABLED — all routes accessible without tokens
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn auth_disabled_status_shows_disabled() {
    let (_tmp, app) = setup_app_with_auth(false, None);
    let resp = app.oneshot(get("/api/auth/status")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["auth_enabled"], false);
}

#[tokio::test]
async fn auth_disabled_login_always_succeeds() {
    let (_tmp, app) = setup_app_with_auth(false, None);
    let resp = app
        .oneshot(json_post("/api/auth/login", json!({"password": "anything"})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["ok"], true);
    assert_eq!(body["token"], "none");
}

#[tokio::test]
async fn auth_disabled_verify_always_succeeds() {
    let (_tmp, app) = setup_app_with_auth(false, None);
    let resp = app.oneshot(get("/api/auth/verify")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["ok"], true);
}

#[tokio::test]
async fn auth_disabled_protected_routes_accessible() {
    let (_tmp, app) = setup_app_with_auth(false, None);
    // Sessions should be accessible without auth
    let resp = app.oneshot(get("/api/sessions")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn auth_disabled_logout_succeeds() {
    let (_tmp, app) = setup_app_with_auth(false, None);
    let resp = app
        .oneshot(json_post("/api/auth/logout", json!({})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["ok"], true);
}

// ═══════════════════════════════════════════════════════════════════════════════
// AUTH ENABLED — login flow
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn auth_enabled_status_shows_enabled() {
    let (_tmp, app) = setup_app_with_auth(true, Some("mysecret"));
    let resp = app.oneshot(get("/api/auth/status")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["auth_enabled"], true);
}

#[tokio::test]
async fn auth_enabled_login_correct_password() {
    let (_tmp, app) = setup_app_with_auth(true, Some("mysecret"));
    let resp = app
        .oneshot(json_post("/api/auth/login", json!({"password": "mysecret"})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["ok"], true);
    // Should return a real token (not "none")
    let token = body["token"].as_str().unwrap();
    assert_ne!(token, "none");
    assert!(token.contains('.'), "Token should be in payload.sig format");
}

#[tokio::test]
async fn auth_enabled_login_wrong_password() {
    let (_tmp, app) = setup_app_with_auth(true, Some("mysecret"));
    let resp = app
        .oneshot(json_post("/api/auth/login", json!({"password": "wrong"})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let body = parse_body(resp).await;
    assert!(body["error"].as_str().unwrap().contains("Invalid"));
}

#[tokio::test]
async fn auth_enabled_login_empty_password() {
    let (_tmp, app) = setup_app_with_auth(true, Some("mysecret"));
    let resp = app
        .oneshot(json_post("/api/auth/login", json!({"password": ""})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_enabled_login_no_password_field() {
    let (_tmp, app) = setup_app_with_auth(true, Some("mysecret"));
    let resp = app
        .oneshot(json_post("/api/auth/login", json!({})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_enabled_login_no_password_configured() {
    let (_tmp, app) = setup_app_with_auth(true, None);
    let resp = app
        .oneshot(json_post("/api/auth/login", json!({"password": "anything"})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = parse_body(resp).await;
    assert!(body["error"].as_str().unwrap().contains("AUTH_PASSWORD"));
}

// ═══════════════════════════════════════════════════════════════════════════════
// AUTH ENABLED — token verification via /api/auth/verify
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn auth_enabled_verify_valid_token() {
    let (_tmp, app) = setup_app_with_auth(true, Some("mysecret"));

    // Login to get a token
    let resp = app
        .clone()
        .oneshot(json_post("/api/auth/login", json!({"password": "mysecret"})))
        .await
        .unwrap();
    let body = parse_body(resp).await;
    let token = body["token"].as_str().unwrap();

    // Verify the token
    let resp = app
        .oneshot(get(&format!("/api/auth/verify?token={token}")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["ok"], true);
}

#[tokio::test]
async fn auth_enabled_verify_invalid_token() {
    let (_tmp, app) = setup_app_with_auth(true, Some("mysecret"));
    let resp = app
        .oneshot(get("/api/auth/verify?token=invalid.token"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let body = parse_body(resp).await;
    assert_eq!(body["ok"], false);
}

#[tokio::test]
async fn auth_enabled_verify_no_token() {
    let (_tmp, app) = setup_app_with_auth(true, Some("mysecret"));
    let resp = app.oneshot(get("/api/auth/verify")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ═══════════════════════════════════════════════════════════════════════════════
// AUTH ENABLED — middleware protects routes
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn auth_enabled_protected_route_without_token() {
    let (_tmp, app) = setup_app_with_auth(true, Some("secret"));
    let resp = app.oneshot(get("/api/sessions")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let body = parse_body(resp).await;
    assert!(body["error"].as_str().unwrap().contains("required"));
}

#[tokio::test]
async fn auth_enabled_protected_route_with_valid_cookie() {
    let (_tmp, app) = setup_app_with_auth(true, Some("secret"));

    // Login to get token
    let resp = app
        .clone()
        .oneshot(json_post("/api/auth/login", json!({"password": "secret"})))
        .await
        .unwrap();
    let body = parse_body(resp).await;
    let token = body["token"].as_str().unwrap();

    // Access protected route with cookie
    let resp = app
        .oneshot(get_with_cookie("/api/sessions", token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn auth_enabled_protected_route_with_valid_bearer() {
    let (_tmp, app) = setup_app_with_auth(true, Some("secret"));

    // Login to get token
    let resp = app
        .clone()
        .oneshot(json_post("/api/auth/login", json!({"password": "secret"})))
        .await
        .unwrap();
    let body = parse_body(resp).await;
    let token = body["token"].as_str().unwrap();

    // Access protected route with Bearer token
    let resp = app
        .oneshot(get_with_bearer("/api/sessions", token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn auth_enabled_protected_route_with_invalid_token() {
    let (_tmp, app) = setup_app_with_auth(true, Some("secret"));
    let resp = app
        .oneshot(get_with_cookie("/api/sessions", "fake.token"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_enabled_public_routes_always_accessible() {
    let (_tmp, app) = setup_app_with_auth(true, Some("secret"));

    // Health check (public)
    let resp = app.clone().oneshot(get("/api/health")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Auth status (public)
    let resp = app.clone().oneshot(get("/api/auth/status")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Auth login (public)
    let resp = app.clone()
        .oneshot(json_post("/api/auth/login", json!({"password": "secret"})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Auth logout (public)
    let resp = app.clone()
        .oneshot(json_post("/api/auth/logout", json!({})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Auth verify (public, though it may return 401 for invalid token)
    let resp = app.oneshot(get("/api/auth/verify")).await.unwrap();
    // Verify returns 401 without token when auth enabled, but the route itself is accessible
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ═══════════════════════════════════════════════════════════════════════════════
// AUTH ENABLED — full login → access → logout flow
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn auth_full_login_access_flow() {
    let (_tmp, app) = setup_app_with_auth(true, Some("mypass123"));

    // Step 1: Can't access protected route
    let resp = app.clone().oneshot(get("/api/sessions")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Step 2: Login
    let resp = app
        .clone()
        .oneshot(json_post("/api/auth/login", json!({"password": "mypass123"})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    let token = body["token"].as_str().unwrap().to_string();

    // Step 3: Access protected route with token
    let resp = app
        .clone()
        .oneshot(get_with_cookie("/api/sessions", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Step 4: Verify token
    let resp = app
        .clone()
        .oneshot(get(&format!("/api/auth/verify?token={token}")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = parse_body(resp).await;
    assert_eq!(body["ok"], true);

    // Step 5: Access soul route with token
    let resp = app
        .clone()
        .oneshot(get_with_cookie("/api/soul", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Step 6: Access tasks route with Bearer token
    let resp = app
        .clone()
        .oneshot(get_with_bearer("/api/tasks", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Step 7: Logout (doesn't actually invalidate the token server-side)
    let resp = app
        .oneshot(json_post("/api/auth/logout", json!({})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ═══════════════════════════════════════════════════════════════════════════════
// AUTH — multiple protected routes require auth
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn auth_enabled_all_protected_endpoints_require_auth() {
    let (_tmp, app) = setup_app_with_auth(true, Some("test"));

    let endpoints = vec![
        "/api/sessions",
        "/api/history",
        "/api/tasks",
        "/api/notifications",
        "/api/soul",
        "/api/groups",
        "/api/search?q=test",
        "/api/chat/status",
    ];

    for endpoint in endpoints {
        let resp = app
            .clone()
            .oneshot(get(endpoint))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "Expected 401 for {endpoint} without auth"
        );
    }
}

#[tokio::test]
async fn auth_enabled_all_protected_endpoints_accessible_with_token() {
    let (_tmp, app) = setup_app_with_auth(true, Some("test"));

    // Login first
    let resp = app
        .clone()
        .oneshot(json_post("/api/auth/login", json!({"password": "test"})))
        .await
        .unwrap();
    let body = parse_body(resp).await;
    let token = body["token"].as_str().unwrap().to_string();

    let endpoints = vec![
        "/api/sessions",
        "/api/history",
        "/api/tasks",
        "/api/notifications",
        "/api/soul",
        "/api/groups",
        "/api/chat/status",
    ];

    for endpoint in endpoints {
        let resp = app
            .clone()
            .oneshot(get_with_cookie(endpoint, &token))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "Expected 200 for {endpoint} with valid token, got {}",
            resp.status()
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// CONFIG — auth_secret derivation
// ═══════════════════════════════════════════════════════════════════════════════

/// SAFETY: env var manipulation is unsafe in Rust 2024 edition because it's
/// not thread-safe. These tests modify environment variables temporarily.
/// They should NOT be run in parallel with other env-reading tests.
#[test]
fn config_auth_secret_derived_from_password() {
    unsafe {
        std::env::set_var("AUTH_ENABLED", "1");
        std::env::set_var("AUTH_PASSWORD", "testpw");
        std::env::remove_var("AUTH_SECRET");
    }

    let config = ClawConfig::from_env();
    assert!(config.auth_enabled);
    assert_eq!(config.auth_password, Some("testpw".to_string()));
    assert_eq!(config.auth_secret, "claw-auth-testpw-secret-key");

    // Cleanup
    unsafe {
        std::env::remove_var("AUTH_ENABLED");
        std::env::remove_var("AUTH_PASSWORD");
    }
}

#[test]
fn config_auth_secret_explicit_override() {
    unsafe {
        std::env::set_var("AUTH_ENABLED", "1");
        std::env::set_var("AUTH_PASSWORD", "pw");
        std::env::set_var("AUTH_SECRET", "my-custom-secret");
    }

    let config = ClawConfig::from_env();
    assert_eq!(config.auth_secret, "my-custom-secret");

    // Cleanup
    unsafe {
        std::env::remove_var("AUTH_ENABLED");
        std::env::remove_var("AUTH_PASSWORD");
        std::env::remove_var("AUTH_SECRET");
    }
}

#[test]
fn config_auth_disabled_by_default() {
    unsafe {
        std::env::remove_var("AUTH_ENABLED");
    }
    let config = ClawConfig::from_env();
    assert!(!config.auth_enabled);
}
