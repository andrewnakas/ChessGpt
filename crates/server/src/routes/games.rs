use std::convert::Infallible;

use api_types::{
    AnalyseGameRequest, AnalyseGameResponse, EloTier, Explanation, GameAnalysis, GameDetail, GameSource, GameSummary,
    ImportRequest, ImportResponse, JobEvent, JobStatus, Side,
};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use chess_core::pgn::{ParsedGame, parse_pgn, parse_pgn_many};
use coach::analysis::deep_pass;
use coach::explain::explain_moment;
use coach::prompts::{EXPLAIN_VERSION, GameContext};
use db::{NewAnalysis, NewGame};
use futures::Stream;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

use crate::error::{ApiResult, AppError};
use crate::jobs::{self, JobParams};
use crate::routes::engine::async_stream;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct Paging {
    limit: Option<u32>,
    offset: Option<u32>,
}

pub async fn list(State(state): State<AppState>, Query(p): Query<Paging>) -> ApiResult<Json<Vec<GameSummary>>> {
    Ok(Json(state.db.list_games(p.limit.unwrap_or(100).min(500), p.offset.unwrap_or(0)).await?))
}

pub async fn detail(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<GameDetail>> {
    let g = state.db.game(&id).await?;
    let analysis = state.db.latest_analysis(&id).await?;
    Ok(Json(GameDetail {
        start_fen: g.parsed.start_fen.clone(),
        moves: g.parsed.moves.clone(),
        summary: g.summary,
        pgn: g.pgn,
        analysis,
    }))
}

pub async fn delete(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<StatusCode> {
    state.db.delete_game(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct SideBody {
    side: Option<Side>,
}

pub async fn set_side(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<SideBody>,
) -> ApiResult<Json<GameSummary>> {
    state.db.set_user_side(&id, b.side).await?;
    Ok(Json(state.db.game_summary(&id).await?))
}

fn movetext_hash(g: &ParsedGame) -> String {
    let mut h = Sha256::new();
    h.update(g.start_fen.as_bytes());
    for m in &g.moves {
        h.update(m.uci.as_bytes());
    }
    for t in ["White", "Black", "Date", "Result"] {
        h.update(g.tag(t).unwrap_or("").as_bytes());
    }
    h.finalize().iter().take(12).map(|b| format!("{b:02x}")).collect()
}

async fn store(
    state: &AppState,
    source: GameSource,
    source_id: Option<String>,
    pgn: String,
    out: &mut ImportResponse,
) {
    match parse_pgn(&pgn) {
        Ok(parsed) => {
            let source_id = source_id.or_else(|| Some(movetext_hash(&parsed)));
            match state.db.insert_game(NewGame { source, source_id, pgn, parsed }).await {
                Ok((g, true)) => out.games.push(g),
                Ok((g, false)) => {
                    out.duplicates += 1;
                    out.games.push(g);
                }
                Err(e) => out.errors.push(e.to_string()),
            }
        }
        Err(e) => out.errors.push(e.to_string()),
    }
}

pub async fn import(State(state): State<AppState>, Json(req): Json<ImportRequest>) -> ApiResult<Json<ImportResponse>> {
    let mut out = ImportResponse { games: vec![], duplicates: 0, errors: vec![] };
    match req {
        ImportRequest::Pgn { pgn } => {
            let games = parse_pgn_many(&pgn).map_err(|e| AppError::bad_request(e.to_string()))?;
            if games.len() > 500 {
                return Err(AppError::bad_request("at most 500 games per paste"));
            }
            for chunk in importers::split_pgns(&pgn) {
                store(&state, GameSource::Pgn, None, chunk, &mut out).await;
            }
        }
        ImportRequest::Fen { fen } => {
            let g = ParsedGame::from_fen(&fen).map_err(|e| AppError::bad_request(e.to_string()))?;
            let pgn = format!("[Event \"Position\"]\n[SetUp \"1\"]\n[FEN \"{}\"]\n\n*\n", g.start_fen);
            store(&state, GameSource::Fen, Some(g.start_fen.clone()), pgn, &mut out).await;
        }
        ImportRequest::LichessGame { id } => {
            let g = state.importers.lichess_game(&id).await?;
            store(&state, g.source, Some(g.source_id), g.pgn, &mut out).await;
        }
        ImportRequest::Lichess { username, max } => {
            let token = state.db.lichess_token().await?;
            let games = state.importers.lichess_user(&username, max.unwrap_or(20), token.as_deref()).await?;
            for g in games {
                store(&state, g.source, Some(g.source_id), g.pgn, &mut out).await;
            }
        }
        ImportRequest::Chesscom { username, max } => {
            let games = state.importers.chesscom_user(&username, max.unwrap_or(20)).await?;
            for g in games {
                store(&state, g.source, Some(g.source_id), g.pgn, &mut out).await;
            }
        }
    }
    if out.games.is_empty() && !out.errors.is_empty() {
        return Err(AppError::bad_request(out.errors.join("; ")));
    }
    Ok(Json(out))
}

pub async fn analyse(
    State(state): State<AppState>,
    Path(game_id): Path<String>,
    Json(req): Json<AnalyseGameRequest>,
) -> ApiResult<Json<AnalyseGameResponse>> {
    let game = state.db.game_summary(&game_id).await?;
    if game.ply_count == 0 {
        return Err(AppError::bad_request("this game has no moves; open it on the board to analyse the position"));
    }
    if !req.force
        && let Some(a) = state.db.latest_analysis(&game_id).await?
        && matches!(a.status, JobStatus::Done | JobStatus::Running | JobStatus::Queued)
    {
        if a.status == JobStatus::Queued && state.jobs.get(&a.id).is_none() {
            jobs::spawn(state.clone(), JobParams { analysis_id: a.id.clone(), explain: req.explain });
        }
        return Ok(Json(AnalyseGameResponse { analysis_id: a.id, status: a.status }));
    }
    let settings = state.db.settings().await?;
    let elo = req.elo.unwrap_or(settings.elo).clamp(100, 3500);
    let user_side = req.user_side.or(game.user_side);
    if req.user_side.is_some() && req.user_side != game.user_side {
        state.db.set_user_side(&game_id, req.user_side).await?;
    }
    let id = state
        .db
        .create_analysis(&NewAnalysis {
            game_id,
            elo,
            tier: EloTier::from_elo(elo),
            user_side,
            engine: state.pool.engine_name().to_string(),
        })
        .await?;
    jobs::spawn(state.clone(), JobParams { analysis_id: id.clone(), explain: req.explain });
    Ok(Json(AnalyseGameResponse { analysis_id: id, status: JobStatus::Queued }))
}

pub async fn get_analysis(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<GameAnalysis>> {
    Ok(Json(state.db.analysis(&id).await?))
}

pub async fn cancel(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<StatusCode> {
    match state.jobs.get(&id) {
        Some(h) => {
            h.cancel.cancel();
            Ok(StatusCode::ACCEPTED)
        }
        None => Err(AppError::not_found("no running job with that id")),
    }
}

fn ev(e: &JobEvent) -> Result<Event, Infallible> {
    Ok(Event::default().data(serde_json::to_string(e).unwrap_or_default()))
}

/// Server-sent events for an analysis: a snapshot first, then live updates
/// until the job finishes.
pub async fn events(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    // Subscribe before reading the snapshot so no event falls in between.
    let rx = state.jobs.get(&id).map(|h| h.tx.subscribe());
    let snap = jobs::snapshot(&state, &id).await?;
    let stream = async_stream(move |tx| async move {
        let finished = snap.status.is_finished();
        if tx.send(JobEvent::Snapshot { analysis: snap.clone() }).await.is_err() {
            return;
        }
        let Some(mut rx) = rx else {
            if finished {
                let _ = tx.send(JobEvent::Done { analysis: snap }).await;
            }
            return;
        };
        loop {
            match rx.recv().await {
                Ok(e) => {
                    let end = matches!(e, JobEvent::Done { .. });
                    if tx.send(e).await.is_err() || end {
                        return;
                    }
                }
                Err(RecvError::Lagged(_)) => {
                    if let Ok(a) = jobs::snapshot(&state, &id).await
                        && tx.send(JobEvent::Snapshot { analysis: a }).await.is_err()
                    {
                        return;
                    }
                }
                Err(RecvError::Closed) => {
                    if let Ok(a) = jobs::snapshot(&state, &id).await {
                        let _ = tx.send(JobEvent::Done { analysis: a }).await;
                    }
                    return;
                }
            }
        }
    });
    Ok(Sse::new(futures::StreamExt::map(stream, |e| ev(&e))).keep_alive(KeepAlive::default()))
}

/// Explain any single move on demand (not only key moments).
pub async fn explain_ply(
    State(state): State<AppState>,
    Path((id, ply)): Path<(String, u32)>,
) -> ApiResult<Json<Explanation>> {
    let a = state.db.analysis(&id).await?;
    let m = a
        .moves
        .iter()
        .find(|m| m.ply == ply)
        .cloned()
        .ok_or_else(|| AppError::bad_request("that move has not been analysed yet"))?;
    let provider = state.provider().await?;
    let game = state.db.game(&a.game_id).await?;
    let tier = EloTier::from_elo(a.elo);
    let ctx = GameContext::new(&game.parsed, a.user_side, a.elo, game.summary.opening.clone());
    let mcs = deep_pass(&state.pool, &game.parsed, &[ply], tier.multipv(), tier.interactive(), &CancellationToken::new())
        .await
        .map_err(|e| AppError::unavailable(e.to_string()))?;
    let mc = mcs.into_iter().next().ok_or_else(|| AppError::bad_request("no such ply"))?;
    let x = explain_moment(provider.as_ref(), &ctx, &m, &mc)
        .await
        .map_err(|e| AppError(StatusCode::BAD_GATEWAY, e.to_string()))?;
    state
        .db
        .put_explanation(
            &id,
            &x.explanation,
            EXPLAIN_VERSION,
            &x.request,
            &x.response,
            Some(x.usage.input_tokens),
            Some(x.usage.output_tokens),
        )
        .await?;
    Ok(Json(x.explanation))
}
