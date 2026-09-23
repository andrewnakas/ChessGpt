use std::convert::Infallible;

use api_types::{
    ChatEvent, ChatMessage, ChatRole, ChatThread, ChatThreadDetail, Classification, CreateThreadRequest, EloTier,
    SendMessageRequest,
};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use coach::chat::{TurnInput, run_turn};
use coach::prompts::GameContext;
use coach::tools::{GameToolContext, ToolEnv};
use futures::Stream;
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::error::{ApiResult, AppError};
use crate::routes::engine::async_stream;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ThreadQuery {
    game_id: Option<String>,
}

pub async fn list(State(state): State<AppState>, Query(q): Query<ThreadQuery>) -> ApiResult<Json<Vec<ChatThread>>> {
    Ok(Json(state.db.list_threads(q.game_id.as_deref()).await?))
}

pub async fn create(State(state): State<AppState>, Json(r): Json<CreateThreadRequest>) -> ApiResult<Json<ChatThread>> {
    if let Some(g) = &r.game_id {
        state.db.game_summary(g).await?;
    }
    let title = r.title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "New chat".into());
    Ok(Json(state.db.create_thread(r.game_id.as_deref(), &title).await?))
}

pub async fn detail(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<ChatThreadDetail>> {
    Ok(Json(state.db.thread_detail(&id).await?))
}

pub async fn delete(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<StatusCode> {
    state.db.delete_thread(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn game_tool_context(state: &AppState, game_id: &str, elo: u32) -> Option<GameToolContext> {
    let g = state.db.game(game_id).await.ok()?;
    let ctx = GameContext::new(&g.parsed, g.summary.user_side, elo, g.summary.opening.clone());
    let mut key_moments = vec![];
    if let Ok(Some(a)) = state.db.latest_analysis(game_id).await {
        for ply in &a.key_moments {
            let Some(m) = a.moves.iter().find(|m| m.ply == *ply) else { continue };
            let n = (m.ply + 1) / 2;
            let label = if m.ply % 2 == 1 { format!("{n}. {}", m.san) } else { format!("{n}... {}", m.san) };
            let mut line = format!(
                "- {label} ({:?}, {}): engine preferred {}",
                m.mover,
                match m.classification {
                    Classification::MissedWin => "missed win".to_string(),
                    c => format!("{c:?}").to_lowercase(),
                },
                m.best_san.as_deref().unwrap_or("?")
            );
            if let Some(e) = a.explanations.iter().find(|e| e.ply == *ply) {
                line.push_str(&format!(". Coach: {}", e.headline));
            }
            key_moments.push(line);
        }
        if let (Some(w), Some(b)) = (a.white_accuracy, a.black_accuracy) {
            key_moments.insert(0, format!("Accuracy: White {w:.0}%, Black {b:.0}%"));
        }
    }
    Some(GameToolContext {
        header: ctx.header(),
        numbered_moves: ctx.numbered_moves(),
        moves_san: ctx.moves_san.clone(),
        key_moments,
    })
}

fn ev(e: &ChatEvent) -> Result<Event, Infallible> {
    Ok(Event::default().data(serde_json::to_string(e).unwrap_or_default()))
}

/// Ask the coach. Streams tokens, tool calls, the verification report, and
/// finally the stored assistant message.
pub async fn send(
    State(state): State<AppState>,
    Path(thread_id): Path<String>,
    Json(req): Json<SendMessageRequest>,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    if req.text.trim().is_empty() {
        return Err(AppError::bad_request("empty message"));
    }
    chess_core::position::parse_fen(&req.fen).map_err(|e| AppError::bad_request(e.to_string()))?;
    let thread = state.db.thread(&thread_id).await?;
    let provider = state.provider().await?;
    let settings = state.db.settings().await?;
    let elo = req.elo.unwrap_or(settings.elo).clamp(100, 3500);
    let explorer = match (settings.explorer_enabled, state.db.lichess_token().await?) {
        (true, Some(t)) => Some((state.importers.clone(), t)),
        _ => None,
    };
    let (game, start_fen) = match &thread.game_id {
        Some(g) => {
            let start = state.db.game(g).await.ok().map(|r| r.parsed.start_fen);
            (game_tool_context(&state, g, elo).await, start)
        }
        None => (None, None),
    };
    let env = ToolEnv { pool: state.pool.clone(), tier: EloTier::from_elo(elo), explorer, game };
    let history: Vec<llm::Message> = state
        .db
        .thread_ir(&thread_id)
        .await?
        .into_iter()
        .filter_map(|v| serde_json::from_value::<Vec<llm::Message>>(v).ok())
        .flatten()
        .collect();

    let user_msg = ChatMessage {
        id: db::new_id(),
        role: ChatRole::User,
        text: req.text.trim().to_string(),
        tool_calls: vec![],
        verification: None,
        fen: Some(req.fen.clone()),
        ply: req.ply,
        created_at: db::now_ms(),
    };
    let first_question = history.is_empty() && thread.title == "New chat";

    let stream = async_stream(move |tx| async move {
        let _ = tx.send(ChatEvent::UserMessage { message: user_msg.clone() }).await;
        let (etx, mut erx) = mpsc::unbounded_channel::<ChatEvent>();
        let fwd_tx = tx.clone();
        let forward = tokio::spawn(async move {
            while let Some(e) = erx.recv().await {
                if fwd_tx.send(e).await.is_err() {
                    break;
                }
            }
        });
        let input = TurnInput {
            history,
            text: req.text.clone(),
            fen: req.fen.clone(),
            ply: req.ply,
            move_path: req.move_path.clone(),
            elo,
            start_fen,
        };
        let result = run_turn(provider.as_ref(), &env, input, &etx).await;
        drop(etx);
        let _ = forward.await;
        match result {
            Ok(r) => {
                let assistant = ChatMessage {
                    id: db::new_id(),
                    role: ChatRole::Assistant,
                    text: r.text.clone(),
                    tool_calls: r.tool_calls.clone(),
                    verification: Some(r.verification.clone()),
                    fen: Some(req.fen.clone()),
                    ply: req.ply,
                    created_at: db::now_ms(),
                };
                let user_ir = serde_json::to_value(&r.new_messages[..1]).unwrap_or_default();
                let rest_ir = serde_json::to_value(&r.new_messages[1..]).unwrap_or_default();
                let stored = async {
                    state.db.append_message(&thread_id, &user_msg, &user_ir, (None, None)).await?;
                    state
                        .db
                        .append_message(
                            &thread_id,
                            &assistant,
                            &rest_ir,
                            (Some(r.usage.input_tokens), Some(r.usage.output_tokens)),
                        )
                        .await?;
                    if first_question {
                        let mut title: String = req.text.trim().chars().take(60).collect();
                        if req.text.trim().chars().count() > 60 {
                            title.push('…');
                        }
                        state.db.rename_thread(&thread_id, &title).await?;
                    }
                    Ok::<_, db::DbError>(())
                }
                .await;
                match stored {
                    Ok(()) => {
                        let _ = tx.send(ChatEvent::Done { message: assistant }).await;
                    }
                    Err(e) => {
                        let _ = tx.send(ChatEvent::Error { message: format!("could not save the reply: {e}") }).await;
                    }
                }
            }
            Err(e) => {
                let _ = tx.send(ChatEvent::Error { message: e.to_string() }).await;
            }
        }
    });
    Ok(Sse::new(futures::StreamExt::map(stream, |e| ev(&e))).keep_alive(KeepAlive::default()))
}
