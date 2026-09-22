//! Lichess win-percentage model (scalachess `WinPercent`).

use crate::score::{CP_CEILING, Score};

const MULTIPLIER: f64 = -0.00368208;

/// Winning chances on the −1..+1 scale for a raw centipawn value (no ceiling).
/// This is what Lichess move judgements use.
pub fn winning_chances(cp: i32) -> f64 {
    (2.0 / (1.0 + (MULTIPLIER * cp as f64).exp()) - 1.0).clamp(-1.0, 1.0)
}

/// Win percentage 0..100 for a centipawn value, with the ±1000 ceiling applied.
pub fn win_percent_cp(cp: i32) -> f64 {
    50.0 + 50.0 * winning_chances(cp.clamp(-CP_CEILING, CP_CEILING))
}

/// Win percentage 0..100 for White given a White-POV score.
pub fn win_percent(score: Score) -> f64 {
    win_percent_cp(score.ceiled_cp())
}

/// Win percentage for `mover` given a White-POV score.
pub fn win_percent_for(score: Score, mover: shakmaty::Color) -> f64 {
    let w = win_percent(score);
    match mover {
        shakmaty::Color::White => w,
        shakmaty::Color::Black => 100.0 - w,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_is_fifty() {
        assert_eq!(win_percent_cp(0), 50.0);
    }

    #[test]
    fn ceiling_applies() {
        assert_eq!(win_percent_cp(5000), win_percent_cp(1000));
        assert_eq!(win_percent_cp(-5000), win_percent_cp(-1000));
        let w = win_percent_cp(1000);
        assert!((w - 97.5).abs() < 0.2, "{w}");
        assert_eq!(win_percent(Score::Mate(3)), w);
        assert!((win_percent(Score::Mate(-3)) - (100.0 - w)).abs() < 1e-9);
    }

    #[test]
    fn symmetric() {
        for cp in [-800, -300, -50, 17, 250, 999] {
            let s = win_percent_cp(cp) + win_percent_cp(-cp);
            assert!((s - 100.0).abs() < 1e-9);
        }
    }

    #[test]
    fn raw_chances_not_ceiled() {
        assert!(winning_chances(3000) > winning_chances(1000));
    }
}
