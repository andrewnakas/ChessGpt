//! "Find the better move" puzzles from the player's own mistakes.

use chess_core::position::{parse_fen, pv_to_san};
use chess_core::winpct::win_percent_for;
use engine::{AnalysisRequest, EngineError, EnginePool, Limit, Priority};
use shakmaty::Position;

/// Mover's win% by which the best move must beat the second best, so the
/// puzzle has one clear answer.
pub const MIN_GAP: f64 = 15.0;

#[derive(Debug, Clone, PartialEq)]
pub struct PuzzleDraft {
    pub fen: String,
    /// The move to find (UCI). Other moves count as misses.
    pub solution_uci: Vec<String>,
    /// The engine's line after the solution, shown once solved (SAN).
    pub line_san: Vec<String>,
}

/// A puzzle from the position before a mistake, if the engine's best move
/// clearly stands out and differs from the move played.
pub async fn from_mistake(
    pool: &EnginePool,
    fen_before: &str,
    played_uci: &str,
    depth: u32,
) -> Result<Option<PuzzleDraft>, EngineError> {
    let Ok(pos) = parse_fen(fen_before) else { return Ok(None) };
    if pos.legal_moves().len() < 2 {
        return Ok(None);
    }
    let a = pool
        .analyse(AnalysisRequest { fen: fen_before.into(), multipv: 2, limit: Limit::depth(depth), priority: Priority::Batch })
        .await?;
    let (Some(best), Some(second)) = (a.lines.first(), a.lines.get(1)) else { return Ok(None) };
    let Some(first) = best.pv.first() else { return Ok(None) };
    let gap = win_percent_for(best.score, pos.turn()) - win_percent_for(second.score, pos.turn());
    if first == played_uci || gap < MIN_GAP {
        return Ok(None);
    }
    Ok(Some(PuzzleDraft {
        fen: fen_before.into(),
        solution_uci: vec![first.clone()],
        line_san: pv_to_san(&pos, &best.pv).into_iter().take(6).collect(),
    }))
}
