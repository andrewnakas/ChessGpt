//! Linking the user's Lichess / Chess.com accounts for automatic game sync.

use api_types::{ConnectRequest, SyncReport};
use axum::Json;

use crate::auth::UserState;
use crate::error::{ApiResult, AppError};
use crate::sync::sync_user;

/// Link a Chess.com account by username (Chess.com offers no self-service
/// sign-in; the games are public), then import and analyse recent games.
pub async fn connect_chesscom(UserState(state, _): UserState, Json(req): Json<ConnectRequest>) -> ApiResult<Json<SyncReport>> {
    let wanted = req.username.trim();
    if wanted.is_empty() || wanted.len() > 50 || !wanted.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(AppError::bad_request("that doesn't look like a Chess.com username"));
    }
    let username = state.importers.chesscom_player(wanted).await?;
    state.db.set_chesscom_username(Some(&username)).await?;
    state.db.mark_side_by_name(&username).await?;
    let mut report = sync_user(&state).await;
    report.username = Some(username);
    Ok(Json(report))
}

pub async fn disconnect_chesscom(UserState(state, _): UserState) -> ApiResult<Json<serde_json::Value>> {
    state.db.set_chesscom_username(None).await?;
    Ok(Json(serde_json::json!({"ok": true})))
}

/// Fetch new games from the linked accounts now.
pub async fn sync_now(UserState(state, _): UserState) -> ApiResult<Json<SyncReport>> {
    Ok(Json(sync_user(&state).await))
}
