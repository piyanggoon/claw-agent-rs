use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use super::middleware::auth_middleware;
use super::routes;
use super::state::AppState;

/// Build the full Axum router with all API routes, middleware, and shared state.
///
/// Routes are split into two groups:
/// - **Public routes**: health, auth endpoints — no authentication required.
/// - **Protected routes**: everything else — guarded by `auth_middleware` when
///   `AUTH_ENABLED=1`.
pub fn build_router(state: AppState) -> Router {
    // ── Public routes (no auth required) ────────────────────────────────
    let public_routes = Router::new()
        .route("/api/health", axum::routing::get(|| async { "ok" }))
        .route(
            "/api/auth/status",
            axum::routing::get(routes::auth::auth_status),
        )
        .route(
            "/api/auth/login",
            axum::routing::post(routes::auth::login),
        )
        .route(
            "/api/auth/logout",
            axum::routing::post(routes::auth::logout),
        )
        .route(
            "/api/auth/verify",
            axum::routing::get(routes::auth::verify),
        );

    // ── Protected routes (auth middleware when AUTH_ENABLED=1) ───────────
    let protected_routes = Router::new()
        // ── Chat routes ─────────────────────────────────────────────
        .route(
            "/api/chat",
            axum::routing::post(routes::chat::create_chat),
        )
        .route(
            "/api/chat/status",
            axum::routing::get(routes::chat::chat_status),
        )
        .route(
            "/api/chat/stream/{run_id}",
            axum::routing::get(routes::chat::stream_events),
        )
        .route(
            "/api/chat/respond",
            axum::routing::post(routes::chat::respond_to_question),
        )
        .route(
            "/api/chat/stop",
            axum::routing::post(routes::chat::stop_agent),
        )
        // ── Session routes ──────────────────────────────────────────
        .route(
            "/api/sessions",
            axum::routing::get(routes::sessions::list_sessions)
                .delete(routes::sessions::delete_all_sessions),
        )
        .route(
            "/api/sessions/{id}",
            axum::routing::get(routes::sessions::get_session)
                .patch(routes::sessions::rename_session)
                .delete(routes::sessions::delete_session),
        )
        // ── History routes ──────────────────────────────────────────
        .route(
            "/api/history",
            axum::routing::get(routes::history::get_history)
                .delete(routes::history::delete_history),
        )
        // ── Task routes ─────────────────────────────────────────────
        .route(
            "/api/tasks",
            axum::routing::get(routes::tasks::list_tasks)
                .post(routes::tasks::create_task),
        )
        .route(
            "/api/tasks/logs",
            axum::routing::get(routes::tasks::all_task_logs),
        )
        .route(
            "/api/tasks/events",
            axum::routing::get(routes::tasks::task_events_stream),
        )
        .route(
            "/api/tasks/{id}",
            axum::routing::get(routes::tasks::get_task)
                .patch(routes::tasks::update_task)
                .delete(routes::tasks::delete_task),
        )
        .route(
            "/api/tasks/{id}/pause",
            axum::routing::post(routes::tasks::pause_task),
        )
        .route(
            "/api/tasks/{id}/resume",
            axum::routing::post(routes::tasks::resume_task),
        )
        .route(
            "/api/tasks/{id}/cancel",
            axum::routing::post(routes::tasks::cancel_task),
        )
        .route(
            "/api/tasks/{id}/logs",
            axum::routing::get(routes::tasks::task_logs),
        )
        // ── Notification routes ─────────────────────────────────────
        .route(
            "/api/notifications",
            axum::routing::get(routes::notifications::list_notifications),
        )
        .route(
            "/api/notifications/{id}/read",
            axum::routing::patch(routes::notifications::mark_read),
        )
        .route(
            "/api/notifications/read-all",
            axum::routing::post(routes::notifications::mark_all_read),
        )
        .route(
            "/api/notifications/{id}",
            axum::routing::delete(routes::notifications::delete_notification),
        )
        // ── Soul routes ─────────────────────────────────────────────
        .route(
            "/api/soul",
            axum::routing::get(routes::soul::list_soul_files),
        )
        .route(
            "/api/soul/memory/search",
            axum::routing::get(routes::soul::search_memory),
        )
        .route(
            "/api/soul/memory/daily",
            axum::routing::get(routes::soul::list_daily_logs),
        )
        .route(
            "/api/soul/{*filename}",
            axum::routing::get(routes::soul::read_soul)
                .put(routes::soul::write_soul)
                .delete(routes::soul::delete_soul),
        )
        // ── Groups routes ───────────────────────────────────────────
        .route(
            "/api/groups",
            axum::routing::get(routes::groups::list_groups)
                .post(routes::groups::create_group),
        )
        .route(
            "/api/groups/{folder}",
            axum::routing::delete(routes::groups::delete_group),
        )
        // ── Search routes ───────────────────────────────────────────
        .route(
            "/api/search",
            axum::routing::get(routes::search::search),
        )
        // ── File routes ─────────────────────────────────────────────
        .route(
            "/api/file",
            axum::routing::get(routes::files::serve_file),
        )
        .route(
            "/api/upload",
            axum::routing::post(routes::files::upload_file),
        )
        // ── Auth middleware ──────────────────────────────────────────
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // ── Merge and apply global middleware ────────────────────────────────
    Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Start the HTTP server on the given port.
///
/// This function blocks until the server is shut down.
pub async fn start_server(state: AppState, port: u16) -> anyhow::Result<()> {
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    tracing::info!("Claw server listening on port {port}");
    axum::serve(listener, app).await?;
    Ok(())
}
