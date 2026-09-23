//! Hosted mode: one shared password, exchanged for an HttpOnly session cookie.
//! Local mode (the default) has no login.

use axum::Json;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::state::AppState;

pub const COOKIE: &str = "chessgpt_session";

pub fn session_token(password: &str, salt: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(salt);
    h.update(password.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Per-install salt so tokens differ between installs; stored in the data dir.
pub fn load_salt(data_dir: &std::path::Path) -> std::io::Result<Vec<u8>> {
    let p = data_dir.join("session.salt");
    match std::fs::read(&p) {
        Ok(b) if b.len() >= 16 => Ok(b),
        _ => {
            let salt: [u8; 32] = rand::random();
            std::fs::write(&p, salt)?;
            Ok(salt.to_vec())
        }
    }
}

fn constant_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn cookie_value(headers: &HeaderMap) -> Option<String> {
    headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).find_map(|c| {
        c.split(';').map(str::trim).find_map(|kv| kv.strip_prefix(&format!("{COOKIE}=")).map(String::from))
    })
}

pub async fn require_session(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let Some(expected) = &state.session_token else {
        return next.run(req).await;
    };
    let path = req.uri().path();
    let open = !path.starts_with("/api/") || path == "/api/login" || path == "/api/health" || path == "/api/session";
    if open || cookie_value(req.headers()).is_some_and(|v| constant_eq(&v, expected)) {
        return next.run(req).await;
    }
    (StatusCode::UNAUTHORIZED, Json(api_types::ApiError { error: "login required".into() })).into_response()
}

#[derive(Deserialize)]
pub struct Login {
    password: String,
}

pub async fn login(State(state): State<AppState>, Json(l): Json<Login>) -> Response {
    let (Some(expected), Some(pw)) = (&state.session_token, &state.config.auth_password) else {
        return StatusCode::NO_CONTENT.into_response();
    };
    if !constant_eq(&l.password, pw) {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        return (StatusCode::UNAUTHORIZED, Json(api_types::ApiError { error: "wrong password".into() })).into_response();
    }
    let secure = if state.config.public_url.as_deref().is_some_and(|u| u.starts_with("https://")) { "; Secure" } else { "" };
    let cookie = format!("{COOKIE}={expected}; HttpOnly; SameSite=Lax; Path=/; Max-Age=2592000{secure}");
    (StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response()
}

#[derive(serde::Serialize)]
pub struct Session {
    required: bool,
    authenticated: bool,
}

pub async fn session(State(state): State<AppState>, headers: HeaderMap) -> Json<Session> {
    let authenticated = match &state.session_token {
        None => true,
        Some(t) => cookie_value(&headers).is_some_and(|v| constant_eq(&v, t)),
    };
    Json(Session { required: state.session_token.is_some(), authenticated })
}
