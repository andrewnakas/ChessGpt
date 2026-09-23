//! Engine pass over a whole game: evaluate every position, classify every
//! move the Lichess way, compute accuracy. Then a deeper MultiPV pass on the
//! key moments so explanations can cite real alternatives and refutations.

use api_types::{MoveEval, Score, Side};
use chess_core::accuracy::{game_accuracy, move_accuracy};
use chess_core::book::book_prefix;
use chess_core::classify::{MoveInput, judge};
use chess_core::pgn::ParsedGame;
use chess_core::position::{fen_key, parse_fen, phase, pv_to_san};
use engine::{Analysis, AnalysisRequest, EngineError, EnginePool, Limit, Priority, Terminal};
use futures::stream::{FuturesOrdered, StreamExt};
use shakmaty::{Color, Position};
use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
pub enum AnalysisError {
    #[error("engine: {0}")]
    Engine(#[from] EngineError),
    #[error("invalid game: {0}")]
    Game(String),
    #[error("cancelled")]
    Cancelled,
}

/// White-POV score for a finished engine analysis, handling mate/stalemate.
pub fn position_score(a: &Analysis, turn: Color) -> Score {
    match a.terminal {
        Some(Terminal::Checkmate) => Score::checkmated(turn),
        Some(Terminal::Stalemate) => Score::Cp(0),
        None => a.score().unwrap_or(Score::Cp(0)),
    }
}

pub struct EnginePass {
    pub start_score: Score,
    pub moves: Vec<MoveEval>,
    pub white_accuracy: Option<f64>,
    pub black_accuracy: Option<f64>,
}

/// Evaluate every position of the game (use `EloTier::batch()` for `limit`).
/// `on_move` is called in ply order as soon as each move can be classified.
pub async fn engine_pass(
    pool: &EnginePool,
    game: &ParsedGame,
    limit: Limit,
    cancel: &CancellationToken,
    mut on_move: impl FnMut(&MoveEval),
) -> Result<EnginePass, AnalysisError> {
    let fens: Vec<String> = std::iter::once(game.start_fen.clone())
        .chain(game.moves.iter().map(|m| m.fen_after.clone()))
        .collect();
    let positions = fens
        .iter()
        .map(|f| parse_fen(f).map_err(|e| AnalysisError::Game(e.to_string())))
        .collect::<Result<Vec<_>, _>>()?;
    let keys_after: Vec<String> = positions.iter().skip(1).map(fen_key).collect();
    let (book, _) = book_prefix(&keys_after);

    // Queue every position at once so all engine workers stay busy; results
    // are consumed in order.
    let mut pending = FuturesOrdered::new();
    for f in &fens {
        let req = AnalysisRequest { fen: f.clone(), multipv: 1, limit, priority: Priority::Batch };
        let pool = pool.clone();
        pending.push_back(async move { pool.analyse(req).await });
    }

    let mut analyses: Vec<Analysis> = Vec::with_capacity(fens.len());
    let mut moves = Vec::with_capacity(game.moves.len());
    while let Some(res) = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(AnalysisError::Cancelled),
        r = pending.next() => r,
    } {
        analyses.push(res?);
        let i = analyses.len() - 1;
        if i == 0 {
            continue;
        }
        let pm = &game.moves[i - 1];
        let before = &positions[i - 1];
        let (a_before, a_after) = (&analyses[i - 1], &analyses[i]);
        let prev = position_score(a_before, before.turn());
        let cur = position_score(a_after, positions[i].turn());
        let mover: Color = pm.mover.into();
        let best_uci = a_before.best_move.clone();
        let v = judge(&MoveInput {
            mover,
            prev,
            cur,
            best_uci: best_uci.as_deref(),
            played_uci: &pm.uci,
            is_book: book[i - 1],
        });
        let best_line_uci = a_before.lines.first().map(|l| l.pv.clone()).unwrap_or_default();
        let best_line_san = pv_to_san(before, &best_line_uci);
        let m = MoveEval {
            ply: pm.ply,
            mover: pm.mover,
            san: pm.san.clone(),
            uci: pm.uci.clone(),
            score: cur,
            depth: a_after.depth,
            best_san: best_line_san.first().cloned(),
            best_uci,
            best_line_san: best_line_san.into_iter().take(10).collect(),
            classification: v.classification,
            lichess_judgement: v.lichess_judgement,
            win_before: v.win_before,
            win_after: v.win_after,
            delta_wc: v.delta_wc,
            accuracy: move_accuracy(v.win_before, v.win_after),
            phase: phase(before),
            is_key_moment: false,
        };
        on_move(&m);
        moves.push(m);
    }

    let start_score = position_score(&analyses[0], positions[0].turn());
    let scores: Vec<Option<Score>> = moves.iter().map(|m| Some(m.score)).collect();
    let acc = game_accuracy(game.white_starts(), &scores);
    Ok(EnginePass { start_score, moves, white_accuracy: acc.white, black_accuracy: acc.black })
}

/// Engine context for one key moment, from the deep pass.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MomentContext {
    pub ply: u32,
    pub fen_before: String,
    pub fen_after: String,
    /// MultiPV lines in the position before the move (SAN, White-POV score).
    pub lines_before: Vec<(Score, Vec<String>)>,
    /// Best line for the opponent after the played move.
    pub refutation: Option<(Score, Vec<String>)>,
}

/// Deeper MultiPV analysis around each key moment, run concurrently (use
/// `EloTier::multipv()` / `EloTier::interactive()`).
pub async fn deep_pass(
    pool: &EnginePool,
    game: &ParsedGame,
    plies: &[u32],
    multipv: u32,
    limit: Limit,
    cancel: &CancellationToken,
) -> Result<Vec<MomentContext>, AnalysisError> {
    let mut futs = FuturesOrdered::new();
    for &ply in plies {
        let pm = game
            .moves
            .get(ply as usize - 1)
            .ok_or_else(|| AnalysisError::Game(format!("no ply {ply}")))?
            .clone();
        let pool = pool.clone();
        futs.push_back(async move {
            let before = pool.analyse(AnalysisRequest {
                fen: pm.fen_before.clone(),
                multipv,
                limit,
                priority: Priority::Batch,
            });
            let after = pool.analyse(AnalysisRequest {
                fen: pm.fen_after.clone(),
                multipv: 1,
                limit,
                priority: Priority::Batch,
            });
            let (b, a) = tokio::join!(before, after);
            Ok::<_, EngineError>((pm, b?, a?))
        });
    }
    let mut out = vec![];
    while let Some(r) = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(AnalysisError::Cancelled),
        r = futs.next() => r,
    } {
        let (pm, b, a) = r?;
        let pos_b = parse_fen(&pm.fen_before).map_err(|e| AnalysisError::Game(e.to_string()))?;
        let pos_a = parse_fen(&pm.fen_after).map_err(|e| AnalysisError::Game(e.to_string()))?;
        let lines_before = b.lines.iter().map(|l| (l.score, pv_to_san(&pos_b, &l.pv))).collect();
        let refutation = a.lines.first().map(|l| (l.score, pv_to_san(&pos_a, &l.pv)));
        out.push(MomentContext {
            ply: pm.ply,
            fen_before: pm.fen_before,
            fen_after: pm.fen_after,
            lines_before,
            refutation: refutation.or_else(|| {
                // Checkmate after the played move: no refutation line.
                a.terminal.map(|_| (position_score(&a, pos_a.turn()), vec![]))
            }),
        });
    }
    Ok(out)
}

pub fn side_name(s: Side) -> &'static str {
    match s {
        Side::White => "White",
        Side::Black => "Black",
    }
}
