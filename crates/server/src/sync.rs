//! Keep a user's library in step with their Lichess and Chess.com accounts:
//! import new games, mark the user's side (from the usernames in Settings),
//! and analyse the newest few so Progress and Train stay current.

use std::time::Duration;

use api_types::{AnalyseGameRequest, SyncReport};
use chess_core::pgn::parse_pgn;
use db::NewGame;

use crate::routes::games::start_analysis;
use crate::state::AppState;

/// Games fetched per account per sync.
const FETCH: u32 = 20;
/// New games analysed per sync (the engine is shared by everyone).
const ANALYSE: usize = 5;
pub const INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// Sync one user (`state` scoped to them).
pub async fn sync_user(state: &AppState) -> SyncReport {
    let mut report = SyncReport { username: None, imported: 0, analysing: 0, errors: vec![] };
    let settings = match state.db.settings().await {
        Ok(s) => s,
        Err(e) => {
            report.errors.push(e.to_string());
            return report;
        }
    };
    let mut fetched = vec![];
    if let Some(u) = settings.chesscom_username.as_deref() {
        match state.importers.chesscom_user(u, FETCH).await {
            Ok(g) => fetched.extend(g),
            Err(e) => report.errors.push(format!("Chess.com: {e}")),
        }
    }
    if let Some(u) = settings.lichess_username.as_deref() {
        let token = state.db.lichess_token().await.ok().flatten();
        match state.importers.lichess_user(u, FETCH, token.as_deref()).await {
            Ok(g) => fetched.extend(g),
            Err(e) => report.errors.push(format!("Lichess: {e}")),
        }
    }
    let mut new_games = vec![];
    for g in fetched {
        let Ok(parsed) = parse_pgn(&g.pgn) else { continue };
        let new = NewGame { source: g.source, source_id: Some(g.source_id), pgn: g.pgn, parsed };
        match state.db.insert_game(new).await {
            Ok((summary, true)) => new_games.push(summary),
            Ok((_, false)) => {}
            Err(e) => report.errors.push(e.to_string()),
        }
    }
    report.imported = new_games.len() as u32;
    // Newest first; only games where we know which side the user played.
    new_games.sort_by(|a, b| b.date.cmp(&a.date));
    for g in new_games.iter().filter(|g| g.user_side.is_some() && g.ply_count > 0).take(ANALYSE) {
        let req = AnalyseGameRequest { elo: None, user_side: None, explain: false, force: false };
        match start_analysis(state, g.id.clone(), &req).await {
            Ok(_) => report.analysing += 1,
            Err(e) => report.errors.push(e.1),
        }
    }
    report
}

/// Sync every user with a linked account now and then.
pub fn spawn_periodic(state: AppState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(INTERVAL).await;
            let users = match state.db.linked_users().await {
                Ok(u) => u,
                Err(e) => {
                    tracing::warn!("sync: {e}");
                    continue;
                }
            };
            for user in users {
                let r = sync_user(&state.scoped(&user)).await;
                if r.imported > 0 || !r.errors.is_empty() {
                    tracing::info!("sync {user}: {} new, {} analysing, errors {:?}", r.imported, r.analysing, r.errors);
                }
                // Be gentle with the public APIs.
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    });
}
