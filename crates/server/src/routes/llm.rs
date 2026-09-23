//! The in-browser coach model: a tab holds the device stream open and
//! answers the requests it receives (see `crate::relay`).

use std::convert::Infallible;

use axum::Json;
use axum::extract::{Path, Query};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::Stream;
use serde::Deserialize;
use serde_json::Value;

use crate::auth::UserState;
use crate::error::{ApiResult, AppError};

#[derive(Deserialize)]
pub struct DeviceQuery {
    model: String,
}

/// Register this tab's loaded model and stream coach requests to it.
pub async fn device(
    UserState(state, _): UserState,
    Query(q): Query<DeviceQuery>,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    let offered = state.config.device_model.as_ref().map(|m| m.id.as_str());
    if offered != Some(q.model.as_str()) {
        return Err(AppError::bad_request("this site does not offer that browser model"));
    }
    let (guard, rx) = state.relay.connect(state.user_key(), q.model);
    let stream = futures::stream::unfold((rx, guard), |(mut rx, guard)| async move {
        let r = rx.recv().await?;
        let ev = Event::default().event("request").data(serde_json::to_string(&r).unwrap_or_default());
        Some((Ok(ev), (rx, guard)))
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// The browser's answer to one relayed request: a `chat.completion` object,
/// or `{"error": "..."}`.
pub async fn relay_reply(
    UserState(state, _): UserState,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    if !state.relay.reply(state.user_key(), &id, body) {
        return Err(AppError::bad_request("unknown or expired request"));
    }
    Ok(Json(serde_json::json!({"ok": true})))
}
