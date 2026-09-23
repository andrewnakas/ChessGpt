//! chessgpt's OAuth 2.1 authorization server, so Claude and ChatGPT can
//! connect to `/mcp` on behalf of a signed-in user.
//!
//! - Discovery: RFC 9728 protected-resource metadata, RFC 8414 server metadata
//! - Clients: Client ID Metadata Documents (preferred) and dynamic registration (RFC 7591)
//! - PKCE S256 only, public clients (`token_endpoint_auth_method: none`)
//! - RFC 8707 resource indicators bind tokens to `/mcp`; RFC 9207 `iss` on redirects
//! - Rotating refresh tokens

use std::time::Duration;

use axum::Form;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use db::{OAuthClient, OAuthGrant, hash_token, random_token};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::{base_url, cookie_value, enc, pkce_challenge};
use crate::mcp::SCOPE;
use crate::state::AppState;

fn oauth_error(status: StatusCode, error: &str, description: &str) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({"error": error, "error_description": description})),
    )
        .into_response()
}

// ------------------------------------------------------------ discovery

pub async fn protected_resource(State(state): State<AppState>, headers: HeaderMap) -> Json<Value> {
    let base = base_url(&state, &headers);
    Json(json!({
        "resource": format!("{base}/mcp"),
        "authorization_servers": [base],
        "scopes_supported": [SCOPE],
        "bearer_methods_supported": ["header"],
        "resource_name": "chessgpt",
        "resource_documentation": format!("{base}/connect")
    }))
}

pub async fn authorization_server(State(state): State<AppState>, headers: HeaderMap) -> Json<Value> {
    let base = base_url(&state, &headers);
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "registration_endpoint": format!("{base}/oauth/register"),
        "revocation_endpoint": format!("{base}/oauth/revoke"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": [SCOPE, "offline_access"],
        "client_id_metadata_document_supported": true,
        "authorization_response_iss_parameter_supported": true,
        "service_documentation": format!("{base}/connect")
    }))
}

/// ChatGPT app domain verification, when configured.
pub async fn openai_challenge() -> Response {
    match std::env::var("CHESSGPT_OPENAI_CHALLENGE") {
        Ok(v) if !v.trim().is_empty() => v.trim().to_string().into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

// ------------------------------------------------------------ clients

fn valid_redirect(uri: &str) -> bool {
    uri.starts_with("https://") || is_loopback(uri)
}

fn is_loopback(uri: &str) -> bool {
    ["http://localhost", "http://127.0.0.1", "http://[::1]"].iter().any(|p| uri.starts_with(p))
}

/// Registered redirect matches exactly; loopback redirects match on any port.
pub fn redirect_allowed(registered: &[String], uri: &str) -> bool {
    let strip_port = |u: &str| -> String {
        let Some(rest) = u.split_once("://").map(|x| x.1) else { return u.into() };
        let (host, path) = rest.split_once('/').map(|(h, p)| (h, format!("/{p}"))).unwrap_or((rest, String::new()));
        let host = if host.starts_with('[') { host.split(']').next().unwrap_or(host).to_string() + "]" } else { host.split(':').next().unwrap_or(host).to_string() };
        format!("http://{host}{path}")
    };
    registered.iter().any(|r| r == uri || (is_loopback(r) && is_loopback(uri) && strip_port(r) == strip_port(uri)))
}

#[derive(Deserialize)]
pub struct RegisterRequest {
    client_name: Option<String>,
    redirect_uris: Vec<String>,
    #[serde(flatten)]
    rest: serde_json::Map<String, Value>,
}

/// RFC 7591 dynamic client registration (public clients only).
pub async fn register(State(state): State<AppState>, Json(r): Json<RegisterRequest>) -> Response {
    if r.redirect_uris.is_empty() || !r.redirect_uris.iter().all(|u| valid_redirect(u)) {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_redirect_uri", "redirect URIs must be https or loopback");
    }
    let id = format!("dcr_{}", &random_token()[..24]);
    let name = r.client_name.clone().unwrap_or_else(|| "MCP client".into());
    let meta = Value::Object(r.rest.clone());
    match state.db.register_client(&id, &name, &r.redirect_uris, &meta).await {
        Ok(c) => (
            StatusCode::CREATED,
            Json(json!({
                "client_id": c.client_id,
                "client_name": c.client_name,
                "redirect_uris": c.redirect_uris,
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
                "token_endpoint_auth_method": "none",
                "client_id_issued_at": db::now_ms() / 1000
            })),
        )
            .into_response(),
        Err(e) => oauth_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error", &e.to_string()),
    }
}

/// Resolve a client: a stored (registered) one, or a Client ID Metadata
/// Document fetched from its https URL.
async fn resolve_client(state: &AppState, client_id: &str) -> Result<OAuthClient, String> {
    if !client_id.starts_with("https://") {
        return state.db.client(client_id).await.map_err(|e| e.to_string())?.ok_or_else(|| "unknown client".to_string());
    }
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let doc: Value = match http.get(client_id).header(header::ACCEPT, "application/json").send().await {
        Ok(r) if r.status().is_success() => r.json().await.map_err(|e| format!("client metadata is not JSON: {e}"))?,
        Ok(r) => return Err(format!("client metadata returned HTTP {}", r.status())),
        Err(e) => {
            // Fall back to a copy we fetched before.
            return state
                .db
                .client(client_id)
                .await
                .ok()
                .flatten()
                .ok_or_else(|| format!("could not fetch client metadata: {e}"));
        }
    };
    if doc["client_id"].as_str() != Some(client_id) {
        return Err("client metadata client_id does not match its URL".into());
    }
    let redirects: Vec<String> = doc["redirect_uris"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    if redirects.is_empty() || !redirects.iter().all(|u| valid_redirect(u)) {
        return Err("client metadata has no valid redirect_uris".into());
    }
    let name = doc["client_name"].as_str().unwrap_or("MCP client").to_string();
    state.db.register_client(client_id, &name, &redirects, &doc).await.map_err(|e| e.to_string())
}

fn client_host(c: &OAuthClient) -> String {
    let src = if c.client_id.starts_with("https://") {
        c.client_id.clone()
    } else {
        c.redirect_uris.first().cloned().unwrap_or_default()
    };
    src.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or("unknown").to_string()
}

// ------------------------------------------------------------ authorize

#[derive(Deserialize, Clone)]
pub struct AuthorizeQuery {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    state: Option<String>,
    scope: Option<String>,
    resource: Option<String>,
}

impl AuthorizeQuery {
    fn to_query(&self) -> String {
        let mut parts = vec![];
        for (k, v) in [
            ("response_type", &self.response_type),
            ("client_id", &self.client_id),
            ("redirect_uri", &self.redirect_uri),
            ("code_challenge", &self.code_challenge),
            ("code_challenge_method", &self.code_challenge_method),
            ("state", &self.state),
            ("scope", &self.scope),
            ("resource", &self.resource),
        ] {
            if let Some(v) = v {
                parts.push(format!("{k}={}", enc(v)));
            }
        }
        parts.join("&")
    }
}

struct Checked {
    client: OAuthClient,
    redirect_uri: String,
    challenge: String,
    resource: Option<String>,
    scope: String,
}

/// Validate an authorization request. Errors that must not redirect (bad
/// client or redirect URI) come back as `Err(page)`.
async fn check(state: &AppState, base: &str, q: &AuthorizeQuery) -> Result<Checked, Response> {
    let page = |msg: &str| Html(error_page(msg)).into_response();
    let Some(client_id) = q.client_id.as_deref() else { return Err(page("Missing client_id.")) };
    let client = resolve_client(state, client_id).await.map_err(|e| page(&format!("Unknown app: {e}")))?;
    let Some(redirect_uri) = q.redirect_uri.clone() else { return Err(page("Missing redirect_uri.")) };
    if !redirect_allowed(&client.redirect_uris, &redirect_uri) {
        return Err(page("This app's redirect address is not registered."));
    }
    let back = |err: &str, desc: &str| {
        let mut u = format!("{redirect_uri}{}error={err}&error_description={}", if redirect_uri.contains('?') { "&" } else { "?" }, enc(desc));
        if let Some(s) = &q.state {
            u.push_str(&format!("&state={}", enc(s)));
        }
        u.push_str(&format!("&iss={}", enc(base)));
        Redirect::to(&u).into_response()
    };
    if q.response_type.as_deref() != Some("code") {
        return Err(back("unsupported_response_type", "only response_type=code is supported"));
    }
    let Some(challenge) = q.code_challenge.clone().filter(|c| c.len() >= 43) else {
        return Err(back("invalid_request", "PKCE code_challenge is required"));
    };
    if q.code_challenge_method.as_deref() != Some("S256") {
        return Err(back("invalid_request", "code_challenge_method must be S256"));
    }
    let resource = q.resource.clone().map(|r| r.trim_end_matches('/').to_string());
    if let Some(r) = &resource
        && r != &format!("{base}/mcp")
    {
        return Err(back("invalid_target", "unknown resource"));
    }
    let scope = SCOPE.to_string();
    Ok(Checked { client, redirect_uri, challenge, resource, scope })
}

fn csrf_for(session: &str) -> String {
    hash_token(&format!("csrf:{session}"))[..32].to_string()
}

pub async fn authorize(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<AuthorizeQuery>) -> Response {
    let base = base_url(&state, &headers);
    let checked = match check(&state, &base, &q).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    // Who is signing in? Local mode: the single local user.
    let session = cookie_value(&headers);
    let account = match (&session, state.accounts_enabled()) {
        (_, false) => None,
        (Some(t), true) => state.db.session_user(t).await.ok().flatten(),
        (None, true) => None,
    };
    if state.accounts_enabled() && account.is_none() {
        let ret = format!("/oauth/authorize?{}", q.to_query());
        return Redirect::to(&format!("/login?return={}", enc(&ret))).into_response();
    }
    let who = account.as_ref().map(|a| a.display_name.clone()).unwrap_or_else(|| "this computer's chessgpt".into());
    let csrf = csrf_for(session.as_deref().unwrap_or("local"));
    Html(consent_page(&checked.client.client_name, &client_host(&checked.client), &who, &q, &csrf)).into_response()
}

#[derive(Deserialize)]
pub struct Decision {
    decision: String,
    csrf: String,
    #[serde(flatten)]
    q: AuthorizeQueryForm,
}

#[derive(Deserialize)]
pub struct AuthorizeQueryForm {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    state: Option<String>,
    scope: Option<String>,
    resource: Option<String>,
}

pub async fn decide(State(state): State<AppState>, headers: HeaderMap, Form(d): Form<Decision>) -> Response {
    let q = AuthorizeQuery {
        response_type: d.q.response_type,
        client_id: d.q.client_id,
        redirect_uri: d.q.redirect_uri,
        code_challenge: d.q.code_challenge,
        code_challenge_method: d.q.code_challenge_method,
        state: d.q.state,
        scope: d.q.scope,
        resource: d.q.resource,
    };
    let base = base_url(&state, &headers);
    let checked = match check(&state, &base, &q).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let session = cookie_value(&headers);
    let user_id = if state.accounts_enabled() {
        match &session {
            Some(t) => match state.db.session_user(t).await.ok().flatten() {
                Some(a) => a.id,
                None => return Html(error_page("Your session expired. Start connecting again.")).into_response(),
            },
            None => return Html(error_page("Please sign in first.")).into_response(),
        }
    } else {
        state.db.user_id().to_string()
    };
    if d.csrf != csrf_for(session.as_deref().unwrap_or("local")) {
        return Html(error_page("This form expired. Start connecting again.")).into_response();
    }
    let sep = if checked.redirect_uri.contains('?') { "&" } else { "?" };
    let state_param = q.state.as_ref().map(|s| format!("&state={}", enc(s))).unwrap_or_default();
    if d.decision != "allow" {
        return Redirect::to(&format!(
            "{}{sep}error=access_denied{state_param}&iss={}",
            checked.redirect_uri,
            enc(&base)
        ))
        .into_response();
    }
    let code = match state
        .db
        .create_code(
            &checked.client.client_id,
            &user_id,
            &checked.redirect_uri,
            &checked.challenge,
            checked.resource.as_deref(),
            &checked.scope,
        )
        .await
    {
        Ok(c) => c,
        Err(e) => return Html(error_page(&e.to_string())).into_response(),
    };
    Redirect::to(&format!("{}{sep}code={code}{state_param}&iss={}", checked.redirect_uri, enc(&base))).into_response()
}

// ------------------------------------------------------------ token

#[derive(Deserialize)]
pub struct TokenRequest {
    grant_type: String,
    code: Option<String>,
    redirect_uri: Option<String>,
    client_id: Option<String>,
    code_verifier: Option<String>,
    refresh_token: Option<String>,
    resource: Option<String>,
}

pub async fn token(State(state): State<AppState>, headers: HeaderMap, Form(t): Form<TokenRequest>) -> Response {
    let base = base_url(&state, &headers);
    let tokens = match t.grant_type.as_str() {
        "authorization_code" => {
            let (Some(code), Some(verifier)) = (t.code.as_deref(), t.code_verifier.as_deref()) else {
                return oauth_error(StatusCode::BAD_REQUEST, "invalid_request", "code and code_verifier are required");
            };
            let Some((grant, redirect, challenge)) = state.db.take_code(code).await.ok().flatten() else {
                return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "code is invalid, used or expired");
            };
            if t.client_id.as_deref().is_some_and(|c| c != grant.client_id)
                || t.redirect_uri.as_deref().is_some_and(|r| r != redirect)
                || pkce_challenge(verifier) != challenge
            {
                return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "code verification failed");
            }
            if let Some(r) = &t.resource
                && Some(r.trim_end_matches('/').to_string()) != grant.resource
                && r.trim_end_matches('/') != format!("{base}/mcp")
            {
                return oauth_error(StatusCode::BAD_REQUEST, "invalid_target", "resource mismatch");
            }
            let grant = OAuthGrant { resource: grant.resource.or_else(|| Some(format!("{base}/mcp"))), ..grant };
            state.db.issue_tokens(&grant).await
        }
        "refresh_token" => {
            let (Some(rt), Some(cid)) = (t.refresh_token.as_deref(), t.client_id.as_deref()) else {
                return oauth_error(StatusCode::BAD_REQUEST, "invalid_request", "refresh_token and client_id are required");
            };
            match state.db.refresh(rt, cid).await {
                Ok(Some(tok)) => Ok(tok),
                Ok(None) => return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "refresh token is invalid"),
                Err(e) => Err(e),
            }
        }
        _ => return oauth_error(StatusCode::BAD_REQUEST, "unsupported_grant_type", "use authorization_code or refresh_token"),
    };
    match tokens {
        Ok(tok) => (
            [(header::CACHE_CONTROL, "no-store")],
            Json(json!({
                "access_token": tok.access_token,
                "token_type": "Bearer",
                "expires_in": tok.expires_in,
                "refresh_token": tok.refresh_token,
                "scope": tok.scope
            })),
        )
            .into_response(),
        Err(e) => oauth_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error", &e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct RevokeRequest {
    token: String,
}

pub async fn revoke(State(state): State<AppState>, Form(r): Form<RevokeRequest>) -> StatusCode {
    let _ = state.db.revoke_token(&r.token).await;
    StatusCode::OK
}

// ------------------------------------------------------------ pages

fn h(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

const PAGE_STYLE: &str = "body{font-family:system-ui,-apple-system,'Segoe UI',sans-serif;background:#f6f5f2;color:#1d1b18;display:flex;justify-content:center;padding:8vh 16px;margin:0}
.card{background:#fff;border:1px solid #dedad2;border-radius:12px;padding:24px 28px;max-width:420px;width:100%;box-shadow:0 4px 16px rgb(0 0 0/.06)}
h1{font-size:20px;margin:0 0 12px}ul{padding-left:20px;margin:10px 0 18px}li{margin:4px 0}.host{color:#6b665d;font-size:13px}
.row{display:flex;gap:10px}button{font:inherit;border-radius:8px;padding:9px 16px;border:1px solid #dedad2;background:#f0eee9;cursor:pointer}
button.allow{background:#b7791f;border-color:#b7791f;color:#fff;font-weight:600}
@media(prefers-color-scheme:dark){body{background:#161512;color:#e9e6df}.card{background:#211f1c;border-color:#3a3733}.host{color:#a39e94}button{background:#2a2825;border-color:#3a3733;color:inherit}}";

fn consent_page(client: &str, host: &str, who: &str, q: &AuthorizeQuery, csrf: &str) -> String {
    let hidden = [
        ("response_type", &q.response_type),
        ("client_id", &q.client_id),
        ("redirect_uri", &q.redirect_uri),
        ("code_challenge", &q.code_challenge),
        ("code_challenge_method", &q.code_challenge_method),
        ("state", &q.state),
        ("scope", &q.scope),
        ("resource", &q.resource),
    ]
    .iter()
    .filter_map(|(k, v)| v.as_ref().map(|v| format!("<input type=\"hidden\" name=\"{k}\" value=\"{}\">", h(v))))
    .collect::<String>();
    format!(
        "<!doctype html><html><head><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'>\
<title>Connect {client} to chessgpt</title><style>{PAGE_STYLE}</style></head><body><form class=card method=post action=/oauth/authorize>\
<h1>Connect {client} to chessgpt?</h1><div class=host>Requested by {host}. Signed in as {who}.</div>\
<p>{client} will be able to:</p><ul><li>Run Stockfish analysis for you</li><li>See your saved games and analyses</li>\
<li>Save and analyse games in your library</li><li>Import your games from Lichess and Chess.com</li></ul>\
<p class=host>It cannot see your password or AI keys. You can disconnect it anytime in chessgpt Settings.</p>\
{hidden}<input type=hidden name=csrf value=\"{csrf}\"><div class=row><button class=allow name=decision value=allow>Allow</button>\
<button name=decision value=deny>Cancel</button></div></form></body></html>",
        client = h(client),
        host = h(host),
        who = h(who),
    )
}

fn error_page(msg: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'>\
<title>chessgpt</title><style>{PAGE_STYLE}</style></head><body><div class=card><h1>Could not connect</h1><p>{}</p>\
<p><a href=/connect>How to connect chessgpt</a></p></div></body></html>",
        h(msg)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirects() {
        let reg = vec!["https://claude.ai/api/mcp/auth_callback".to_string(), "http://localhost/callback".to_string()];
        assert!(redirect_allowed(&reg, "https://claude.ai/api/mcp/auth_callback"));
        assert!(!redirect_allowed(&reg, "https://claude.ai/api/mcp/auth_callback/x"));
        assert!(!redirect_allowed(&reg, "https://evil.com/cb"));
        assert!(redirect_allowed(&reg, "http://localhost:53123/callback"), "loopback ignores port");
        assert!(!redirect_allowed(&reg, "http://localhost:53123/other"));
        assert!(valid_redirect("https://chatgpt.com/connector_platform_oauth_redirect"));
        assert!(!valid_redirect("http://evil.com/cb"));
    }

    #[test]
    fn consent_escapes() {
        let q = AuthorizeQuery {
            response_type: Some("code".into()),
            client_id: Some("x".into()),
            redirect_uri: Some("https://a/\"><script>".into()),
            code_challenge: None,
            code_challenge_method: None,
            state: None,
            scope: None,
            resource: None,
        };
        let p = consent_page("<b>Evil</b>", "a", "me", &q, "c");
        assert!(!p.contains("<script>") && !p.contains("<b>Evil"));
    }
}
