//! Accounts in hosted mode: email + password, "Sign in with Lichess" or
//! "Sign in with Chess.com",
//! browser sessions in an HttpOnly cookie. Local mode has a single user and
//! no login.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{FromRequestParts, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use db::{Account, random_token};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{ApiResult, AppError};
use crate::state::AppState;

pub const COOKIE: &str = "chessgpt_session";

/// Request state scoped to the signed-in user: `state.db` only sees their data.
pub struct UserState(pub AppState, pub Option<Account>);

impl FromRequestParts<AppState> for UserState {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        if !state.accounts_enabled() {
            return Ok(UserState(state.clone(), None));
        }
        let unauthorized = || AppError(StatusCode::UNAUTHORIZED, "sign in required".into());
        let token = cookie_value(&parts.headers).ok_or_else(unauthorized)?;
        let acct = state.db.session_user(&token).await?.ok_or_else(unauthorized)?;
        Ok(UserState(state.scoped(&acct.id), Some(acct)))
    }
}

pub fn cookie_value(headers: &HeaderMap) -> Option<String> {
    headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).find_map(|c| {
        c.split(';').map(str::trim).find_map(|kv| kv.strip_prefix(&format!("{COOKIE}=")).map(String::from))
    })
}

fn session_cookie(state: &AppState, token: &str, max_age: i64) -> String {
    let secure = if state.config.public_url.as_deref().is_some_and(|u| u.starts_with("https://")) { "; Secure" } else { "" };
    format!("{COOKIE}={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age}{secure}")
}

async fn start_session(state: &AppState, acct: &Account) -> ApiResult<String> {
    let token = state.db.create_session(&acct.id).await?;
    Ok(session_cookie(state, &token, db::accounts_session_seconds()))
}

/// Public origin of this server, e.g. `https://chessgpt.com`.
pub fn base_url(state: &AppState, headers: &HeaderMap) -> String {
    if let Some(u) = &state.config.public_url {
        return u.trim_end_matches('/').to_string();
    }
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("localhost:8080");
    let proto = headers.get("x-forwarded-proto").and_then(|h| h.to_str().ok()).unwrap_or("http");
    format!("{proto}://{host}")
}

#[derive(Serialize)]
pub struct Session {
    accounts: bool,
    account: Option<Account>,
    lichess_login: bool,
    chesscom_login: bool,
}

pub async fn session(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Session>> {
    let account = match (state.accounts_enabled(), cookie_value(&headers)) {
        (true, Some(t)) => state.db.session_user(&t).await?,
        _ => None,
    };
    Ok(Json(Session {
        accounts: state.accounts_enabled(),
        account,
        lichess_login: state.accounts_enabled(),
        chesscom_login: state.accounts_enabled() && state.config.chesscom_client_id.is_some(),
    }))
}

#[derive(Deserialize)]
pub struct Register {
    email: String,
    password: String,
    name: Option<String>,
}

pub async fn register(State(state): State<AppState>, Json(r): Json<Register>) -> ApiResult<Response> {
    if !state.accounts_enabled() {
        return Err(AppError::bad_request("accounts are only used in hosted mode"));
    }
    let acct = state.db.register(&r.email, &r.password, r.name.as_deref()).await?;
    let cookie = start_session(&state, &acct).await?;
    Ok(([(header::SET_COOKIE, cookie)], Json(acct)).into_response())
}

#[derive(Deserialize)]
pub struct Login {
    email: String,
    password: String,
}

pub async fn login(State(state): State<AppState>, Json(l): Json<Login>) -> ApiResult<Response> {
    match state.db.verify_login(&l.email, &l.password).await {
        Ok(acct) => {
            let cookie = start_session(&state, &acct).await?;
            Ok(([(header::SET_COOKIE, cookie)], Json(acct)).into_response())
        }
        Err(db::DbError::NotFound) => {
            tokio::time::sleep(Duration::from_millis(400)).await;
            Err(AppError(StatusCode::UNAUTHORIZED, "wrong email or password".into()))
        }
        Err(e) => Err(e.into()),
    }
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(t) = cookie_value(&headers) {
        state.db.delete_session(&t).await?;
    }
    Ok(([(header::SET_COOKIE, session_cookie(&state, "", 0))], StatusCode::NO_CONTENT).into_response())
}

/// Delete the signed-in account and everything stored for it.
pub async fn delete_account(UserState(state, acct): UserState) -> ApiResult<Response> {
    let Some(acct) = acct else { return Err(AppError::bad_request("accounts are only used in hosted mode")) };
    state.db.delete_account(&acct.id).await?;
    Ok(([(header::SET_COOKIE, session_cookie(&state, "", 0))], StatusCode::NO_CONTENT).into_response())
}

// ------------------------------------------------------------ Lichess OAuth

/// In-flight Lichess sign-ins: state -> (PKCE verifier, return path, started).
#[derive(Default)]
pub struct PendingLogins(Mutex<HashMap<String, (String, String, Instant)>>);

impl PendingLogins {
    fn put(&self, state: &str, verifier: &str, ret: &str) {
        let mut m = self.0.lock().unwrap();
        m.retain(|_, v| v.2.elapsed() < Duration::from_secs(600));
        m.insert(state.into(), (verifier.into(), ret.into(), Instant::now()));
    }
    fn take(&self, state: &str) -> Option<(String, String)> {
        self.0.lock().unwrap().remove(state).filter(|v| v.2.elapsed() < Duration::from_secs(600)).map(|v| (v.0, v.1))
    }
}

pub fn pkce_challenge(verifier: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Only same-site relative paths are allowed as a post-login destination.
pub fn safe_return(r: Option<&str>) -> String {
    match r {
        Some(p) if p.starts_with('/') && !p.starts_with("//") && !p.contains('\\') => p.to_string(),
        _ => "/".to_string(),
    }
}

#[derive(Deserialize)]
pub struct LichessStart {
    #[serde(rename = "return")]
    ret: Option<String>,
}

pub async fn lichess_start(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<LichessStart>) -> Response {
    let base = base_url(&state, &headers);
    let verifier = random_token();
    let st = random_token();
    state.pending.put(&st, &verifier, &safe_return(q.ret.as_deref()));
    let client_id = client_id_for(&base);
    let url = format!(
        "https://lichess.org/oauth?response_type=code&client_id={}&redirect_uri={}&code_challenge_method=S256&code_challenge={}&state={}",
        enc(&client_id),
        enc(&format!("{base}/auth/lichess/callback")),
        pkce_challenge(&verifier),
        st
    );
    Redirect::to(&url).into_response()
}

fn client_id_for(base: &str) -> String {
    base.split("://").nth(1).unwrap_or("chessgpt").to_string()
}

pub fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[derive(Deserialize)]
pub struct LichessCallback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

pub async fn lichess_callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<LichessCallback>,
) -> Response {
    let fail = |m: &str| Redirect::to(&format!("/login?error={}", enc(m))).into_response();
    if let Some(e) = q.error {
        return fail(&format!("Lichess sign-in was cancelled ({e})"));
    }
    let (Some(code), Some(st)) = (q.code, q.state) else { return fail("missing code") };
    let Some((verifier, ret)) = state.pending.take(&st) else { return fail("sign-in expired, try again") };
    let base = base_url(&state, &headers);
    let client = reqwest::Client::new();
    let tok: serde_json::Value = match client
        .post("https://lichess.org/api/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("redirect_uri", &format!("{base}/auth/lichess/callback")),
            ("client_id", &client_id_for(&base)),
        ])
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r.json().await.unwrap_or_default(),
        Ok(r) => return fail(&format!("Lichess refused the sign-in (HTTP {})", r.status())),
        Err(e) => return fail(&format!("could not reach Lichess: {e}")),
    };
    let Some(access) = tok["access_token"].as_str() else { return fail("Lichess returned no token") };
    let acct: serde_json::Value = match client.get("https://lichess.org/api/account").bearer_auth(access).send().await {
        Ok(r) if r.status().is_success() => r.json().await.unwrap_or_default(),
        _ => return fail("could not read your Lichess account"),
    };
    let (Some(id), Some(username)) = (acct["id"].as_str(), acct["username"].as_str()) else {
        return fail("could not read your Lichess account");
    };
    let account = match state.db.lichess_login(id, username, access).await {
        Ok(a) => a,
        Err(e) => return fail(&e.to_string()),
    };
    match start_session(&state, &account).await {
        Ok(cookie) => ([(header::SET_COOKIE, cookie)], Redirect::to(&ret)).into_response(),
        Err(e) => fail(&e.1),
    }
}

// ---------------------------------------------------------- Chess.com OAuth

// Chess.com's OpenID Connect server (https://oauth.chess.com/.well-known/openid-configuration).
// Client credentials come from Chess.com's OAuth application program.
const CHESSCOM_ISSUER: &str = "https://oauth.chess.com";

pub async fn chesscom_start(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<LichessStart>) -> Response {
    let Some(client_id) = state.config.chesscom_client_id.clone() else {
        return Redirect::to(&format!("/login?error={}", enc("Chess.com sign-in is not set up on this server"))).into_response();
    };
    let base = base_url(&state, &headers);
    let verifier = random_token();
    let st = random_token();
    state.pending.put(&st, &verifier, &safe_return(q.ret.as_deref()));
    // The state doubles as the OIDC nonce: both are single use and random.
    let url = format!(
        "{CHESSCOM_ISSUER}/authorize?response_type=code&client_id={}&redirect_uri={}&scope=openid%20profile&code_challenge_method=S256&code_challenge={}&state={st}&nonce={st}",
        enc(&client_id),
        enc(&format!("{base}/auth/chesscom/callback")),
        pkce_challenge(&verifier),
    );
    Redirect::to(&url).into_response()
}

pub async fn chesscom_callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<LichessCallback>,
) -> Response {
    let fail = |m: &str| Redirect::to(&format!("/login?error={}", enc(m))).into_response();
    let Some(client_id) = state.config.chesscom_client_id.clone() else {
        return fail("Chess.com sign-in is not set up on this server");
    };
    if let Some(e) = q.error {
        return fail(&format!("Chess.com sign-in was cancelled ({e})"));
    }
    let (Some(code), Some(st)) = (q.code, q.state) else { return fail("missing code") };
    let Some((verifier, ret)) = state.pending.take(&st) else { return fail("sign-in expired, try again") };
    let base = base_url(&state, &headers);
    let redirect_uri = format!("{base}/auth/chesscom/callback");
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("code_verifier", verifier.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("client_id", client_id.as_str()),
    ];
    if let Some(secret) = &state.config.chesscom_client_secret {
        form.push(("client_secret", secret.as_str()));
    }
    let tok: serde_json::Value = match reqwest::Client::new()
        .post(format!("{CHESSCOM_ISSUER}/token"))
        .header(header::USER_AGENT, concat!("chessgpt/", env!("CARGO_PKG_VERSION"), " (+https://chessgpt.com)"))
        .form(&form)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r.json().await.unwrap_or_default(),
        Ok(r) => return fail(&format!("Chess.com refused the sign-in (HTTP {})", r.status())),
        Err(e) => return fail(&format!("could not reach Chess.com: {e}")),
    };
    // The id_token came straight from the token endpoint over TLS, so its
    // claims are checked but its signature need not be (OIDC Core 3.1.3.7).
    let claims = tok["id_token"].as_str().and_then(|t| chesscom_claims(t, &client_id, &st, db::now_ms() / 1000));
    let Some((sub, username)) = claims else { return fail("could not read your Chess.com account") };
    let account = match state.db.chesscom_login(&sub, &username).await {
        Ok(a) => a,
        Err(e) => return fail(&e.to_string()),
    };
    match start_session(&state, &account).await {
        Ok(cookie) => ([(header::SET_COOKIE, cookie)], Redirect::to(&ret)).into_response(),
        Err(e) => fail(&e.1),
    }
}

/// `(sub, preferred_username)` from a Chess.com id_token, if it is for us,
/// unexpired and carries our nonce.
fn chesscom_claims(id_token: &str, client_id: &str, nonce: &str, now_secs: i64) -> Option<(String, String)> {
    use base64::Engine;
    let payload = id_token.split('.').nth(1)?;
    let c: serde_json::Value =
        serde_json::from_slice(&base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?).ok()?;
    let aud_ok = match &c["aud"] {
        serde_json::Value::String(a) => a == client_id,
        serde_json::Value::Array(a) => a.iter().any(|v| v == client_id),
        _ => false,
    };
    let ok = c["iss"] == CHESSCOM_ISSUER && aud_ok && c["exp"].as_i64()? > now_secs && c["nonce"] == nonce;
    let sub = c["sub"].as_str().filter(|s| !s.is_empty())?;
    let username = c["preferred_username"].as_str().filter(|s| !s.is_empty())?;
    ok.then(|| (sub.to_string(), username.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_rfc7636_example() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn chesscom_id_token_claims_are_checked() {
        use base64::Engine;
        let tok = |c: serde_json::Value| {
            let b = |v: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v);
            format!("{}.{}.sig", b(br#"{"alg":"RS256"}"#), b(c.to_string().as_bytes()))
        };
        let good = serde_json::json!({
            "iss": "https://oauth.chess.com", "aud": "cid", "sub": "42", "exp": 2000,
            "nonce": "n1", "preferred_username": "MagnusCarlsen"
        });
        assert_eq!(chesscom_claims(&tok(good.clone()), "cid", "n1", 1000), Some(("42".into(), "MagnusCarlsen".into())));
        let mut arr = good.clone();
        arr["aud"] = serde_json::json!(["other", "cid"]);
        assert!(chesscom_claims(&tok(arr), "cid", "n1", 1000).is_some());
        assert!(chesscom_claims(&tok(good.clone()), "other", "n1", 1000).is_none(), "wrong audience");
        assert!(chesscom_claims(&tok(good.clone()), "cid", "n2", 1000).is_none(), "wrong nonce");
        assert!(chesscom_claims(&tok(good.clone()), "cid", "n1", 3000).is_none(), "expired");
        let mut iss = good;
        iss["iss"] = "https://evil.example".into();
        assert!(chesscom_claims(&tok(iss), "cid", "n1", 1000).is_none(), "wrong issuer");
        assert!(chesscom_claims("garbage", "cid", "n1", 1000).is_none());
    }

    #[test]
    fn return_paths_are_local() {
        assert_eq!(safe_return(Some("/analyse/x")), "/analyse/x");
        assert_eq!(safe_return(Some("//evil.com")), "/");
        assert_eq!(safe_return(Some("https://evil.com")), "/");
        assert_eq!(safe_return(None), "/");
    }
}
