use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use super::routes;
use super::state::AppState;

/// Build the full Axum router with all API routes, middleware, and shared state.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        // ── Chat routes ──────────────────────────────────────────────
        .route(
            "/api/chat",
            axum::routing::post(routes::chat::create_chat),
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
            "/api/chat/stop/{run_id}",
            axum::routing::post(routes::chat::stop_agent),
        )
        // ── Session routes ───────────────────────────────────────────
        .route(
            "/api/sessions",
            axum::routing::get(routes::sessions::list_sessions),
        )
        .route(
            "/api/sessions/{id}",
            axum::routing::get(routes::sessions::get_session)
                .delete(routes::sessions::delete_session),
        )
        // ── Task routes ──────────────────────────────────────────────
        .route(
            "/api/tasks",
            axum::routing::get(routes::tasks::list_tasks),
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
        // ── Notification routes ──────────────────────────────────────
        .route(
            "/api/notifications",
            axum::routing::get(routes::notifications::list_notifications),
        )
        .route(
            "/api/notifications/{id}/read",
            axum::routing::post(routes::notifications::mark_read),
        )
        .route(
            "/api/notifications/read-all",
            axum::routing::post(routes::notifications::mark_all_read),
        )
        .route(
            "/api/notifications/{id}",
            axum::routing::delete(routes::notifications::delete_notification),
        )
        // ── Soul routes ──────────────────────────────────────────────
        .route(
            "/api/soul/memory/search",
            axum::routing::get(routes::soul::search_memory),
        )
        .route(
            "/api/soul/{*filename}",
            axum::routing::get(routes::soul::read_soul)
                .put(routes::soul::write_soul),
        )
        // ── Health ───────────────────────────────────────────────────
        .route("/api/health", axum::routing::get(|| async { "ok" }))
        // ── Middleware ────────────────────────────────────────────────
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
