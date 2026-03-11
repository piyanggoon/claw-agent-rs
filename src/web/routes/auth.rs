//! Authentication routes — login, logout, status, verify.
//!
//! Token format (compatible with SoulClaw frontend):
//! ```text
//! {payloadB64url}.{signatureB64url}
//! ```
//! where `payloadB64url` is the URL-safe Base64 encoding of a JSON payload
//! `{ "exp": <unix_ms> }`, and `signatureB64url` is the URL-safe Base64 encoding
//! of `HMAC-SHA256(AUTH_SECRET, payloadB64url)`.
//!
//! When `AUTH_ENABLED=0` (default), all auth endpoints return success stubs.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::json;
use sha2::Sha256;

use super::super::state::AppState;

/// Token validity duration: 7 days (in milliseconds).
const TOKEN_EXPIRY_MS: u64 = 7 * 24 * 60 * 60 * 1000;

type HmacSha256 = Hmac<Sha256>;

// ─── Request types ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct LoginRequest {
    pub password: Option<String>,
}

#[derive(Deserialize)]
pub struct VerifyQuery {
    pub token: Option<String>,
}

// ─── Token helpers ──────────────────────────────────────────────────────────

/// Generate a signed token using HMAC-SHA256.
///
/// Returns `{payloadB64url}.{signatureB64url}`.
pub fn generate_token(secret: &str) -> String {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let exp = now_ms + TOKEN_EXPIRY_MS;

    let payload = json!({"exp": exp});
    let payload_b64 = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(payload_b64.as_bytes());
    let sig = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());

    format!("{payload_b64}.{sig}")
}

/// Verify a token's HMAC signature and check expiration.
///
/// Returns `true` if the token is valid and not expired.
pub fn verify_token(token: &str, secret: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 2 {
        return false;
    }
    let (payload_b64, sig_b64) = (parts[0], parts[1]);

    // Verify signature
    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(payload_b64.as_bytes());

    let expected_sig = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    if sig_b64 != expected_sig {
        return false;
    }

    // Decode payload and check expiration
    let payload_bytes = match URL_SAFE_NO_PAD.decode(payload_b64) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let payload: serde_json::Value = match serde_json::from_slice(&payload_bytes) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let exp = match payload["exp"].as_u64() {
        Some(e) => e,
        None => return false,
    };

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    exp > now_ms
}

// ─── Route handlers ─────────────────────────────────────────────────────────

/// GET /api/auth/status
///
/// Returns whether authentication is enabled.
pub async fn auth_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({ "auth_enabled": state.config.auth_enabled }))
}

/// POST /api/auth/login
///
/// When auth is disabled: returns `{ ok: true, token: "none" }`.
/// When auth is enabled: validates password and returns a signed token.
pub async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let config = &state.config;

    // When auth is disabled, always succeed
    if !config.auth_enabled {
        return Ok(Json(json!({"ok": true, "token": "none"})));
    }

    // Validate password
    let password = body.password.unwrap_or_default();
    let expected_password = config.auth_password.as_deref().unwrap_or("");

    if expected_password.is_empty() {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "AUTH_ENABLED=1 but AUTH_PASSWORD is not set"})),
        ));
    }

    if password != expected_password {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "Invalid password"})),
        ));
    }

    // Generate signed token
    let token = generate_token(&config.auth_secret);
    Ok(Json(json!({"ok": true, "token": token})))
}

/// POST /api/auth/logout
///
/// Always returns success — token invalidation is handled by the frontend
/// clearing the cookie.
pub async fn logout() -> Json<serde_json::Value> {
    Json(json!({"ok": true}))
}

/// GET /api/auth/verify
///
/// When auth is disabled: always returns `{ ok: true }`.
/// When auth is enabled: validates the token from query parameter.
pub async fn verify(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<VerifyQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let config = &state.config;

    // When auth is disabled, always succeed
    if !config.auth_enabled {
        return Ok(Json(json!({"ok": true})));
    }

    let token = match &query.token {
        Some(t) if !t.is_empty() => t,
        _ => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({"ok": false, "error": "No token provided"})),
            ));
        }
    };

    if verify_token(token, &config.auth_secret) {
        Ok(Json(json!({"ok": true})))
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok": false, "error": "Invalid or expired token"})),
        ))
    }
}

// ─── Unit tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_and_verify_token() {
        let secret = "claw-auth-mypassword-secret-key";
        let token = generate_token(secret);
        assert!(verify_token(&token, secret));
    }

    #[test]
    fn test_wrong_secret_fails() {
        let token = generate_token("correct-secret");
        assert!(!verify_token(&token, "wrong-secret"));
    }

    #[test]
    fn test_expired_token() {
        let secret = "test-secret";
        // Manually create an expired token
        let exp = 0u64; // Unix epoch = long expired
        let payload = json!({"exp": exp});
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());

        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(payload_b64.as_bytes());
        let sig = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());

        let token = format!("{payload_b64}.{sig}");
        assert!(!verify_token(&token, secret));
    }

    #[test]
    fn test_malformed_token() {
        let secret = "test-secret";
        assert!(!verify_token("not-a-valid-token", secret));
        assert!(!verify_token("", secret));
        assert!(!verify_token("a.b.c", secret));
    }

    #[test]
    fn test_tampered_payload() {
        let secret = "test-secret";
        let token = generate_token(secret);
        let parts: Vec<&str> = token.split('.').collect();

        // Tamper with the payload
        let tampered_payload = URL_SAFE_NO_PAD.encode(
            json!({"exp": 9999999999999u64}).to_string().as_bytes(),
        );
        let tampered_token = format!("{}.{}", tampered_payload, parts[1]);
        assert!(!verify_token(&tampered_token, secret));
    }

    #[test]
    fn test_empty_secret() {
        let token = generate_token("");
        assert!(verify_token(&token, ""));
    }
}
