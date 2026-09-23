//! Estimated playing strength ("played like ~1450") from the quality of one
//! side's moves in one game. A linear model over move-quality features,
//! fitted on Lichess games with server evals (`chessgpt-lab fit-rating`).
//! One game is noisy; average several with [`rolling`].

use crate::classify::Judgement;
use crate::position::Phase;

/// One move by the side being rated.
#[derive(Debug, Clone, Copy)]
pub struct MoveStat {
    /// Mover's win% before and after the move (0..100).
    pub win_before: f64,
    pub win_after: f64,
    /// Lichess-style move accuracy (0..100).
    pub accuracy: f64,
    pub judgement: Option<Judgement>,
    pub phase: Phase,
}

pub const FEATURES: usize = 12;

/// Feature names, in `features()` order (for reports).
pub const FEATURE_NAMES: [&str; FEATURES] = [
    "intercept",
    "accuracy",
    "accuracy_sq",
    "blunder_rate",
    "mistake_rate",
    "inaccuracy_rate",
    "mean_win_loss",
    "opening_accuracy",
    "middlegame_accuracy",
    "endgame_accuracy",
    "ln_base_seconds",
    "ln_moves",
];

/// Fitted by `chessgpt-lab fit-rating` (see `fixtures/rating/README.md`).
#[rustfmt::skip]
pub const COEF: [f64; FEATURES] = [
    2880.071714, -20.878259, -8.014965, -283.347639, -467.532986, -439.788865,
    -93.947935, 19.652107, 1.372252, -1.961966, -114.731339, 169.704593,
];

/// Per-game estimates shrink toward the training average (a noisy predictor
/// does that): estimate ≈ a + b × true rating. Averages are corrected by
/// inverting this line (fit in `chessgpt-lab fit-rating`).
pub const CALIBRATION: (f64, f64) = (1280.145, 0.22304);
/// Spread of one game's estimate around that line.
const RESIDUAL_SD: f64 = 183.0;
/// Games averaged by [`rolling`].
pub const ROLLING_GAMES: usize = 20;

pub const MIN_MOVES: usize = 10;

fn mean(v: impl Iterator<Item = f64>) -> Option<f64> {
    let (s, n) = v.fold((0.0, 0usize), |(s, n), x| (s + x, n + 1));
    (n > 0).then(|| s / n as f64)
}

/// The model's inputs for one side's moves; None for games too short to rate.
/// `base_seconds` is the clock's starting time (None: unknown, taken as 5 min).
pub fn features(moves: &[MoveStat], base_seconds: Option<u32>) -> Option<[f64; FEATURES]> {
    if moves.len() < MIN_MOVES {
        return None;
    }
    let n = moves.len() as f64;
    let acc = mean(moves.iter().map(|m| m.accuracy))?;
    let rate = |j: Judgement| moves.iter().filter(|m| m.judgement == Some(j)).count() as f64 / n;
    let loss = mean(moves.iter().map(|m| (m.win_before - m.win_after).clamp(0.0, 50.0)))?;
    let phase_acc = |p: Phase| mean(moves.iter().filter(|m| m.phase == p).map(|m| m.accuracy)).unwrap_or(acc);
    let base = base_seconds.unwrap_or(300).max(1) as f64;
    Some([
        1.0,
        acc,
        acc * acc / 100.0,
        rate(Judgement::Blunder),
        rate(Judgement::Mistake),
        rate(Judgement::Inaccuracy),
        loss,
        phase_acc(Phase::Opening),
        phase_acc(Phase::Middlegame),
        phase_acc(Phase::Endgame),
        base.ln(),
        n.ln(),
    ])
}

/// Estimated rating for one game, or None if too short to say.
pub fn estimate(moves: &[MoveStat], base_seconds: Option<u32>) -> Option<u32> {
    let f = features(moves, base_seconds)?;
    let y: f64 = f.iter().zip(COEF.iter()).map(|(x, c)| x * c).sum();
    Some(y.clamp(400.0, 3000.0).round() as u32)
}

/// Combine recent per-game estimates (newest last) into a rating: the mean
/// of the last [`ROLLING_GAMES`], corrected for shrinkage, rounded to 25.
/// Returns (rating, ± margin of about one standard error).
pub fn rolling(estimates: &[u32]) -> Option<(u32, u32)> {
    let recent = &estimates[estimates.len().saturating_sub(ROLLING_GAMES)..];
    let m = mean(recent.iter().map(|e| *e as f64))?;
    let (a, b) = CALIBRATION;
    let r = ((m - a) / b).clamp(400.0, 3000.0);
    let margin = RESIDUAL_SD / b / (recent.len() as f64).sqrt();
    Some((((r / 25.0).round() * 25.0) as u32, ((margin / 25.0).round() * 25.0) as u32))
}

/// Performance rating over games against rated opponents: the average
/// opponent rating plus 400 × (wins − losses) / games, the usual linear
/// approximation. `results` holds (opponent rating, score 1 / 0.5 / 0).
pub fn performance(results: &[(u32, f64)]) -> Option<u32> {
    if results.is_empty() {
        return None;
    }
    let n = results.len() as f64;
    let avg = results.iter().map(|(r, _)| *r as f64).sum::<f64>() / n;
    let net: f64 = results.iter().map(|(_, s)| 2.0 * s - 1.0).sum();
    Some((avg + 400.0 * net / n).clamp(100.0, 3500.0).round() as u32)
}

/// Parse a PGN `TimeControl` tag ("300+3") into its base seconds.
pub fn base_seconds(time_control: &str) -> Option<u32> {
    time_control.split('+').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(acc: f64, j: Option<Judgement>) -> MoveStat {
        MoveStat { win_before: 50.0, win_after: 50.0 - (100.0 - acc) / 4.0, accuracy: acc, judgement: j, phase: Phase::Middlegame }
    }

    #[test]
    fn features_need_enough_moves() {
        assert!(features(&[stat(90.0, None); 9], None).is_none());
        let f = features(&[stat(90.0, None); 20], Some(180)).unwrap();
        assert_eq!(f[0], 1.0);
        assert_eq!(f[1], 90.0);
        assert_eq!(f[9], 90.0, "missing endgame takes the overall accuracy");
        assert!((f[10] - 180f64.ln()).abs() < 1e-9);
    }

    #[test]
    fn better_play_rates_higher() {
        let strong: Vec<MoveStat> = (0..30).map(|_| stat(95.0, None)).collect();
        let mut weak: Vec<MoveStat> = (0..30).map(|_| stat(70.0, None)).collect();
        weak[3].judgement = Some(Judgement::Blunder);
        weak[9].judgement = Some(Judgement::Blunder);
        let (s, w) = (estimate(&strong, Some(300)).unwrap(), estimate(&weak, Some(300)).unwrap());
        assert!(s > w + 150, "strong {s} vs weak {w}");
    }

    #[test]
    fn rolling_undoes_shrinkage() {
        let (a, b) = CALIBRATION;
        let shrunk = (a + b * 1200.0).round() as u32;
        let mut v = vec![400; 5];
        v.extend([shrunk; 20]);
        let (r, margin) = rolling(&v).unwrap();
        assert!((1175..=1225).contains(&r), "{r}");
        assert!(margin > 100 && margin < 250, "{margin}");
        assert_eq!(rolling(&[]), None);
        assert_eq!(base_seconds("180+2"), Some(180));
        assert_eq!(performance(&[(1500, 1.0), (1500, 0.0), (1600, 0.5)]), Some(1533));
        assert_eq!(performance(&[(1500, 1.0)]), Some(1900));
    }
}
