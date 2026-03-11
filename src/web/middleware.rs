//! Authentication middleware for protected API routes.
//!
//! When `AUTH_ENABLED=1`, this middleware validates the `claw-token` cookie
//! or `Authorization: Bearer <token>` header on every request to protected routes.
//!
//! Public routes (auth endpoints, health check) bypass this middleware.

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use super::routes::auth::verify_token;
use super::state::AppState;

/// Extract the auth token from the request.
///
/// Looks for:
/// 1. `Cookie: claw-token=<token>` header
/// 2. `Authorization: Bearer <token>` header
fn extract_token(req: &Request<Body>) -> Option<String> {
    // Try Cookie header first
    if let Some(cookie_header) = req.headers().get("cookie") {
        if let Ok(cookie_str) = cookie_header.to_str() {
            for cookie in cookie_str.split(';') {
                let cookie = cookie.trim();
                if let Some(token) = cookie.strip_prefix("claw-token=") {
                    let token = token.trim();
                    if !token.is_empty() {
                        return Some(token.to_string());
                    }
                }
            }
        }
    }

    // Fall back to Authorization header
    if let Some(auth_header) = req.headers().get("authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if let Some(token) = auth_str.strip_prefix("Bearer ") {
                let token = token.trim();
                if !token.is_empty() {
                    return Some(token.to_string());
                }
            }
        }
    }

    None
}

/// Auth middleware that validates tokens when auth is enabled.
///
/// If `auth_enabled` is `false`, all requests pass through.
/// If `auth_enabled` is `true`, the request must carry a valid token
/// via cookie or Authorization header.
pub async fn auth_middleware(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let config = &state.config;

    // If auth is not enabled, pass through
    if !config.auth_enabled {
        return next.run(req).await;
    }

    // Extract and verify token
    let token = match extract_token(&req) {
        Some(t) => t,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Authentication required"})),
            )
                .into_response();
        }
    };

    if !verify_token(&token, &config.auth_secret) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "Invalid or expired token"})),
        )
            .into_response();
    }

    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header;

    fn make_request_with_cookie(token: &str) -> Request<Body> {
        Request::builder()
            .uri("/api/chat")
            .header("cookie", format!("claw-token={token}"))
            .body(Body::empty())
            .unwrap()
    }

    fn make_request_with_bearer(token: &str) -> Request<Body> {
        Request::builder()
            .uri("/api/chat")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    }

    fn make_request_no_auth() -> Request<Body> {
        Request::builder()
            .uri("/api/chat")
            .body(Body::empty())
            .unwrap()
    }

    #[test]
    fn extract_token_from_cookie() {
        let req = make_request_with_cookie("abc123.def456");
        assert_eq!(extract_token(&req), Some("abc123.def456".to_string()));
    }

    #[test]
    fn extract_token_from_bearer() {
        let req = make_request_with_bearer("abc123.def456");
        assert_eq!(extract_token(&req), Some("abc123.def456".to_string()));
    }

    #[test]
    fn extract_token_none() {
        let req = make_request_no_auth();
        assert_eq!(extract_token(&req), None);
    }

    #[test]
    fn extract_token_from_multi_cookie() {
        let req = Request::builder()
            .uri("/api/chat")
            .header("cookie", "other=value; claw-token=mytoken.sig; another=x")
            .body(Body::empty())
            .unwrap();
        assert_eq!(extract_token(&req), Some("mytoken.sig".to_string()));
    }

    #[test]
    fn extract_token_cookie_priority_over_bearer() {
        let req = Request::builder()
            .uri("/api/chat")
            .header("cookie", "claw-token=from-cookie.sig")
            .header(header::AUTHORIZATION, "Bearer from-bearer.sig")
            .body(Body::empty())
            .unwrap();
        // Cookie should take priority
        assert_eq!(extract_token(&req), Some("from-cookie.sig".to_string()));
    }

    #[test]
    fn extract_token_empty_cookie_falls_through() {
        let req = Request::builder()
            .uri("/api/chat")
            .header("cookie", "claw-token=")
            .header(header::AUTHORIZATION, "Bearer fallback.sig")
            .body(Body::empty())
            .unwrap();
        assert_eq!(extract_token(&req), Some("fallback.sig".to_string()));
    }
}
