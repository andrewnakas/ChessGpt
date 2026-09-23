//! Choose the handful of moments worth explaining.

use api_types::{Classification, MoveEval, Phase, Side};

pub const DEFAULT_MAX: usize = 8;
/// Mover's win% below which the game was already lost before the move.
pub const ALREADY_LOST: f64 = 10.0;

fn severity(c: Classification) -> f64 {
    match c {
        Classification::Blunder | Classification::MissedWin => 3.0,
        Classification::Mistake => 2.0,
        Classification::Inaccuracy => 1.0,
        _ => 0.0,
    }
}

/// White's win% after each ply.
fn white_win(m: &MoveEval) -> f64 {
    match m.mover {
        Side::White => m.win_after,
        Side::Black => 100.0 - m.win_after,
    }
}

/// The ply after which the side that lost never again held a winning chance
/// above 50%. `None` for draws or unfinished games.
pub fn turning_point(moves: &[MoveEval], result: &str) -> Option<u32> {
    let loser = match result {
        "1-0" => Side::Black,
        "0-1" => Side::White,
        _ => return None,
    };
    let loser_win = |m: &MoveEval| match loser {
        Side::White => white_win(m),
        Side::Black => 100.0 - white_win(m),
    };
    let last_good = moves.iter().rposition(|m| loser_win(m) > 50.0);
    let idx = match last_good {
        Some(i) if i + 1 < moves.len() => i + 1,
        Some(_) => return None,
        None => 0,
    };
    moves.get(idx).map(|m| m.ply)
}

/// Pick up to `max` key plies, sorted by ply.
///
/// Scoring: errors by severity x win-chance drop (x1.5 for the user's own
/// moves), ignoring errors made from already-lost positions; plus the turning point
/// and the largest swing in each phase. Two errors by the same player within
/// two plies collapse to the larger.
pub fn select(moves: &[MoveEval], user_side: Option<Side>, result: &str, max: usize) -> Vec<u32> {
    let weight = |m: &MoveEval| {
        let own = user_side.is_none_or(|s| s == m.mover);
        let base = severity(m.classification) * m.delta_wc.max(0.0);
        if own && user_side.is_some() { base * 1.5 } else { base }
    };
    // Errors from an already-lost position teach little: skip them.
    let mut scored: Vec<(f64, u32, Side)> = moves
        .iter()
        .filter(|m| m.classification.is_error() && m.win_before >= ALREADY_LOST)
        .map(|m| (weight(m), m.ply, m.mover))
        .collect();

    if let Some(tp) = turning_point(moves, result)
        && !scored.iter().any(|(_, p, _)| *p == tp)
        && let Some(m) = moves.iter().find(|m| m.ply == tp)
        && m.delta_wc > 0.05
        && m.win_before >= ALREADY_LOST
    {
        scored.push((weight(m).max(0.5), tp, m.mover));
    }
    for phase in [Phase::Opening, Phase::Middlegame, Phase::Endgame] {
        if let Some(m) = moves
            .iter()
            .filter(|m| m.phase == phase && m.delta_wc > 0.1 && m.win_before >= ALREADY_LOST)
            .max_by(|a, b| a.delta_wc.total_cmp(&b.delta_wc))
            && !scored.iter().any(|(_, p, _)| *p == m.ply)
        {
            scored.push((weight(m).max(0.3), m.ply, m.mover));
        }
    }

    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut chosen: Vec<(u32, Side)> = vec![];
    for (_, ply, side) in scored {
        if chosen.len() >= max {
            break;
        }
        if chosen.iter().any(|(p, s)| *s == side && p.abs_diff(ply) <= 2) {
            continue;
        }
        chosen.push((ply, side));
    }
    let mut plies: Vec<u32> = chosen.into_iter().map(|(p, _)| p).collect();
    plies.sort_unstable();
    plies
}

#[cfg(test)]
mod tests {
    use super::*;
    use api_types::Score;

    fn mv(ply: u32, class: Classification, delta: f64, white_win_after: f64) -> MoveEval {
        let mover = if ply % 2 == 1 { Side::White } else { Side::Black };
        MoveEval {
            ply,
            mover,
            san: "x".into(),
            uci: "a1a2".into(),
            score: Score::Cp(0),
            depth: 18,
            best_uci: None,
            best_san: None,
            best_line_san: vec![],
            classification: class,
            lichess_judgement: None,
            win_before: 50.0,
            win_after: if mover == Side::White { white_win_after } else { 100.0 - white_win_after },
            delta_wc: delta,
            accuracy: 90.0,
            phase: if ply < 20 { Phase::Opening } else { Phase::Middlegame },
            is_key_moment: false,
        }
    }

    #[test]
    fn picks_errors_and_prefers_user() {
        let mut moves: Vec<MoveEval> = (1..=40).map(|p| mv(p, Classification::Good, 0.0, 55.0)).collect();
        moves[9] = mv(10, Classification::Blunder, 0.4, 80.0); // black blunder
        moves[10] = mv(11, Classification::Inaccuracy, 0.12, 75.0);
        moves[20] = mv(21, Classification::Mistake, 0.25, 60.0); // white mistake
        moves[21] = mv(22, Classification::Inaccuracy, 0.1, 62.0);
        let k = select(&moves, Some(Side::White), "1-0", 8);
        assert!(k.contains(&10) && k.contains(&21), "{k:?}");
        let k = select(&moves, Some(Side::White), "1-0", 2);
        assert_eq!(k.len(), 2);
        assert!(k.contains(&10), "blunders always rank high: {k:?}");
    }

    #[test]
    fn same_player_errors_close_together_collapse() {
        let mut moves: Vec<MoveEval> = (1..=30).map(|p| mv(p, Classification::Good, 0.0, 50.0)).collect();
        moves[10] = mv(11, Classification::Mistake, 0.2, 40.0);
        moves[12] = mv(13, Classification::Blunder, 0.35, 10.0);
        let k = select(&moves, None, "0-1", 8);
        assert!(k.contains(&13) && !k.contains(&11), "{k:?}");
    }

    #[test]
    fn turning_point_found() {
        let mut moves: Vec<MoveEval> = (1..=20).map(|p| mv(p, Classification::Good, 0.0, 45.0)).collect();
        for m in moves.iter_mut().skip(7) {
            let w = 80.0;
            m.win_after = if m.mover == Side::White { w } else { 100.0 - w };
        }
        assert_eq!(turning_point(&moves, "1-0"), Some(8));
        assert_eq!(turning_point(&moves, "1/2-1/2"), None);
    }
}
