//! Lichess accuracy, reproduced from `lila/modules/analyse/src/main/AccuracyPercent.scala`.

use crate::score::{CP_INITIAL, Score};
use crate::winpct::win_percent_cp;

/// Accuracy of one move from the mover's win% before and after (0..100).
pub fn move_accuracy(before: f64, after: f64) -> f64 {
    if after >= before {
        return 100.0;
    }
    let win_diff = before - after;
    let raw = 103.1668100711649 * (-0.04354415386753951 * win_diff).exp() - 3.166924740191411;
    (raw + 1.0).clamp(0.0, 100.0)
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GameAccuracy {
    pub white: Option<f64>,
    pub black: Option<f64>,
}

/// Game accuracy per colour. `scores` are White-POV evals of the position
/// after each ply, in order; `None` where no eval exists.
/// `white_starts` is true when the first ply is a White move.
pub fn game_accuracy(white_starts: bool, scores: &[Option<Score>]) -> GameAccuracy {
    game_accuracy_from(white_starts, scores, Some(CP_INITIAL))
}

pub fn game_accuracy_from(
    white_starts: bool,
    scores: &[Option<Score>],
    initial_cp: Option<i32>,
) -> GameAccuracy {
    let all: Vec<Option<f64>> = std::iter::once(initial_cp)
        .chain(scores.iter().map(|s| s.map(|s| s.ceiled_cp())))
        .map(|c| c.map(win_percent_cp))
        .collect();
    if all.len() < 2 {
        return GameAccuracy { white: None, black: None };
    }
    let window_size = (scores.len() / 10).clamp(2, 8);

    // windows = (windowSize.atMost(n) - 2) copies of the first window, then sliding windows
    let mut windows: Vec<&[Option<f64>]> = Vec::new();
    let first_len = window_size.min(all.len());
    let copies = first_len.saturating_sub(2);
    for _ in 0..copies {
        windows.push(&all[..first_len]);
    }
    if all.len() >= window_size {
        for w in all.windows(window_size) {
            windows.push(w);
        }
    } else {
        // Scala's sliding on a shorter list yields the whole list once.
        windows.push(&all[..]);
    }
    let weights: Vec<Option<f64>> = windows
        .iter()
        .map(|w| {
            let vals: Option<Vec<f64>> = w.iter().copied().collect();
            vals.map(|v| std_dev(&v).clamp(0.5, 12.0))
        })
        .collect();

    // (accuracy, weight, is_white)
    let mut per_move: Vec<(Option<(f64, f64)>, bool)> = Vec::new();
    for (i, pair) in all.windows(2).enumerate() {
        let Some(weight) = weights.get(i).copied() else { break };
        let is_white = (i % 2 == 0) == white_starts;
        let entry = match (pair[0], pair[1], weight) {
            (Some(p), Some(n), Some(w)) => {
                let acc = if is_white { move_accuracy(p, n) } else { move_accuracy(n, p) };
                Some((acc, w))
            }
            _ => None,
        };
        per_move.push((entry, is_white));
    }

    let colour = |white: bool| -> Option<f64> {
        let xs: Vec<(f64, f64)> = per_move
            .iter()
            .filter(|(_, c)| *c == white)
            .filter_map(|(e, _)| *e)
            .collect();
        let weighted = weighted_mean(&xs)?;
        let harmonic = harmonic_mean(&xs.iter().map(|(a, _)| *a).collect::<Vec<_>>())?;
        Some((weighted + harmonic) / 2.0)
    };
    GameAccuracy { white: colour(true), black: colour(false) }
}

fn std_dev(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    (xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n).sqrt()
}

fn weighted_mean(xs: &[(f64, f64)]) -> Option<f64> {
    let total_w: f64 = xs.iter().map(|(_, w)| w).sum();
    if xs.is_empty() || total_w == 0.0 {
        return None;
    }
    Some(xs.iter().map(|(v, w)| v * w).sum::<f64>() / total_w)
}

fn harmonic_mean(xs: &[f64]) -> Option<f64> {
    if xs.is_empty() {
        return None;
    }
    // scalalib Maths.harmonicMean: n / sum(1 / max(1, x))
    Some(xs.len() as f64 / xs.iter().map(|x| 1.0 / x.max(1.0)).sum::<f64>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfect_move() {
        assert_eq!(move_accuracy(50.0, 50.0), 100.0);
        assert_eq!(move_accuracy(50.0, 60.0), 100.0);
    }

    #[test]
    fn drop_reduces_accuracy() {
        let a = move_accuracy(50.0, 40.0);
        assert!(a > 60.0 && a < 70.0, "{a}");
        assert_eq!(move_accuracy(100.0, 0.0), 0.0);
        // tiny drop still gets the +1 bonus but never above 100
        assert!(move_accuracy(50.0, 49.99) <= 100.0);
    }

    #[test]
    fn perfect_game_is_near_100() {
        let scores: Vec<Option<Score>> = (0..40).map(|_| Some(Score::Cp(15))).collect();
        let acc = game_accuracy(true, &scores);
        assert!(acc.white.unwrap() > 99.0);
        assert!(acc.black.unwrap() > 99.0);
    }

    #[test]
    fn blunders_hurt_the_blunderer() {
        // White blunders at ply 11 (index 10) from +0.2 to -5
        let mut scores: Vec<Option<Score>> = (0..30).map(|_| Some(Score::Cp(20))).collect();
        for s in scores.iter_mut().skip(10) {
            *s = Some(Score::Cp(-500));
        }
        let acc = game_accuracy(true, &scores);
        assert!(acc.white.unwrap() < acc.black.unwrap());
    }
}
