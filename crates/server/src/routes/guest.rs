//! The coach for guests (no account). Guests keep their games in the browser
//! and run the coach model there too (WebGPU), so the server only builds the
//! prompt, relays it to the guest's own tab and verifies the answer, exactly
//! as for accounts. A guest is a random id the browser keeps; it names the
//! relay channel and nothing else is stored.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use api_types::{DeviceModel, EloTier, Explanation, MoveEval, Side};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use coach::analysis::deep_pass;
use coach::explain::explain_moment;
use coach::prompts::GameContext;
use futures::Stream;
use serde::Deserialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::error::{ApiResult, AppError};
use crate::relay::Relay;
use crate::state::AppState;

/// Explanations per guest (and per address) per hour.
const EXPLAINS_PER_HOUR: usize = 40;

fn guest_key(id: &str) -> ApiResult<String> {
    let ok = (16..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !ok {
        return Err(AppError::bad_request("bad guest id"));
    }
    Ok(format!("guest:{id}"))
}

fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("cf-connecting-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').next().unwrap_or("").trim().to_string())
        .unwrap_or_default()
}

/// Sliding one-hour window per key.
fn allow(key: &str, per_hour: usize) -> bool {
    static SEEN: OnceLock<Mutex<HashMap<String, Vec<Instant>>>> = OnceLock::new();
    let mut seen = SEEN.get_or_init(Default::default).lock().unwrap();
    let hour = Duration::from_secs(3600);
    seen.retain(|_, v| {
        v.retain(|t| t.elapsed() < hour);
        !v.is_empty()
    });
    let v = seen.entry(key.to_string()).or_default();
    if v.len() >= per_hour {
        return false;
    }
    v.push(Instant::now());
    true
}

/// The browser model this site offers (guests get the server-free meta otherwise).
pub async fn meta(State(state): State<AppState>) -> Json<Option<DeviceModel>> {
    Json(state.config.device_model.clone())
}

#[derive(Deserialize)]
pub struct DeviceQuery {
    model: String,
    guest: String,
}

/// A guest tab with the model loaded: stream coach requests to it.
pub async fn device(
    State(state): State<AppState>,
    Query(q): Query<DeviceQuery>,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    let key = guest_key(&q.guest)?;
    let offered = state.config.device_model.as_ref().map(|m| m.id.as_str());
    if offered != Some(q.model.as_str()) {
        return Err(AppError::bad_request("this site does not offer that browser model"));
    }
    let (guard, rx) = state.relay.connect(&key, q.model);
    let stream = futures::stream::unfold((rx, guard), |(mut rx, guard)| async move {
        let r = rx.recv().await?;
        let ev = Event::default().event("request").data(serde_json::to_string(&r).unwrap_or_default());
        Some((Ok(ev), (rx, guard)))
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[derive(Deserialize)]
pub struct GuestQuery {
    guest: String,
}

pub async fn relay_reply(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<GuestQuery>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    if !state.relay.reply(&guest_key(&q.guest)?, &id, body) {
        return Err(AppError::bad_request("unknown or expired request"));
    }
    Ok(Json(serde_json::json!({"ok": true})))
}

#[derive(Deserialize)]
pub struct ExplainRequest {
    guest: String,
    pgn: String,
    ply: u32,
    elo: u32,
    user_side: Option<Side>,
    /// The browser's analysis of the game.
    moves: Vec<MoveEval>,
}

/// Explain one moment of a guest's game with the coach model in their tab.
pub async fn explain(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(r): Json<ExplainRequest>,
) -> ApiResult<Json<Explanation>> {
    let key = guest_key(&r.guest)?;
    let Some(model) = state.relay.device_model(&key) else {
        return Err(AppError(StatusCode::CONFLICT, "Turn on the coach in Settings first: it runs in this browser.".into()));
    };
    if !allow(&key, EXPLAINS_PER_HOUR) || !allow(&format!("ip:{}", client_ip(&headers)), EXPLAINS_PER_HOUR) {
        return Err(AppError(StatusCode::TOO_MANY_REQUESTS, "That's a lot of explanations for one hour. Try again later.".into()));
    }
    let game = chess_core::pgn::parse_pgn(&r.pgn).map_err(|e| AppError::bad_request(e.to_string()))?;
    let m = r
        .moves
        .iter()
        .find(|m| m.ply == r.ply)
        .cloned()
        .ok_or_else(|| AppError::bad_request("that move has not been analysed yet"))?;
    let elo = r.elo.clamp(100, 3500);
    let tier = EloTier::from_elo(elo);
    let mcs = deep_pass(&state.pool, &game, &[r.ply], tier.multipv(), tier.batch(), &CancellationToken::new())
        .await
        .map_err(|e| AppError::unavailable(e.to_string()))?;
    let mc = mcs.into_iter().next().ok_or_else(|| AppError::bad_request("no such ply"))?;
    let ctx = GameContext::new(&game, r.user_side, elo, None);
    let provider = Arc::new(Relay { hub: state.relay.clone(), user: key, model, fallback: None, dead: Default::default() });
    let x = explain_moment(provider.as_ref(), &ctx, &m, &mc)
        .await
        .map_err(|e| AppError(StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(x.explanation))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_ids_are_checked() {
        assert_eq!(guest_key("0f8fad5b-d9cb-469f-a165-70867728950e").unwrap(), "guest:0f8fad5b-d9cb-469f-a165-70867728950e");
        assert!(guest_key("short").is_err());
        assert!(guest_key("../../etc/passwd-aaaaaaaaaaaa").is_err());
        assert!(guest_key(&"a".repeat(65)).is_err());
    }

    #[test]
    fn hourly_allowance_runs_out() {
        let key = "test:allowance";
        assert!((0..3).all(|_| allow(key, 3)));
        assert!(!allow(key, 3));
        assert!(allow("test:other", 3));
    }
}
