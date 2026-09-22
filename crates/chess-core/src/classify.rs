//! Move judgements. The three Lichess judgements are reproduced exactly from
//! `lila/modules/tree/src/main/Advice.scala`; the richer [`Classification`]
//! layered on top is chessgpt's own (see docs/classification.md).

use serde::{Deserialize, Serialize};
use shakmaty::Color;

use crate::score::Score;
use crate::winpct::{win_percent_for, winning_chances};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Judgement {
    Inaccuracy,
    Mistake,
    Blunder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Classification {
    Book,
    Best,
    Excellent,
    Good,
    Inaccuracy,
    Mistake,
    Blunder,
    MissedWin,
}

impl Classification {
    pub fn glyph(self) -> &'static str {
        match self {
            Classification::Book => "",
            Classification::Best => "!!",
            Classification::Excellent => "!",
            Classification::Good => "",
            Classification::Inaccuracy => "?!",
            Classification::Mistake => "?",
            Classification::Blunder => "??",
            Classification::MissedWin => "⌀",
        }
    }

    pub fn is_error(self) -> bool {
        matches!(
            self,
            Classification::Inaccuracy
                | Classification::Mistake
                | Classification::Blunder
                | Classification::MissedWin
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Classification::Book => "book",
            Classification::Best => "best",
            Classification::Excellent => "excellent",
            Classification::Good => "good",
            Classification::Inaccuracy => "inaccuracy",
            Classification::Mistake => "mistake",
            Classification::Blunder => "blunder",
            Classification::MissedWin => "missed_win",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum MateSequence {
    MateCreated,
    MateLost,
}

/// Lichess judgement for a move by `mover`, given White-POV scores of the
/// position before and after the move. Same order as lila: cp advice first,
/// then mate advice.
pub fn lichess_judgement(prev: Score, cur: Score, mover: Color) -> Option<Judgement> {
    cp_advice(prev, cur, mover).or_else(|| mate_advice(prev, cur, mover).map(|(_, j)| j))
}

fn cp_advice(prev: Score, cur: Score, mover: Color) -> Option<Judgement> {
    // lila uses the raw (un-ceiled) cp here.
    let (p, c) = (prev.cp()?, cur.cp()?);
    let d = winning_chances(c) - winning_chances(p);
    let delta = match mover {
        Color::White => -d,
        Color::Black => d,
    };
    if delta >= 0.3 {
        Some(Judgement::Blunder)
    } else if delta >= 0.2 {
        Some(Judgement::Mistake)
    } else if delta >= 0.1 {
        Some(Judgement::Inaccuracy)
    } else {
        None
    }
}

pub fn mate_sequence(prev: Score, cur: Score, mover: Color) -> Option<MateSequence> {
    let (p, c) = (prev.to_pov(mover), cur.to_pov(mover));
    match (p, c) {
        (Score::Cp(_), Score::Mate(n)) if n < 0 => Some(MateSequence::MateCreated),
        (Score::Mate(m), Score::Cp(_)) if m > 0 => Some(MateSequence::MateLost),
        (Score::Mate(m), Score::Mate(n)) if m > 0 && n < 0 => Some(MateSequence::MateLost),
        _ => None,
    }
}

fn mate_advice(prev: Score, cur: Score, mover: Color) -> Option<(MateSequence, Judgement)> {
    let seq = mate_sequence(prev, cur, mover)?;
    let prev_cp = prev.to_pov(mover).cp().unwrap_or(0);
    let cur_cp = cur.to_pov(mover).cp().unwrap_or(0);
    let j = match seq {
        MateSequence::MateCreated if prev_cp < -999 => Judgement::Inaccuracy,
        MateSequence::MateCreated if prev_cp < -700 => Judgement::Mistake,
        MateSequence::MateCreated => Judgement::Blunder,
        MateSequence::MateLost if cur_cp > 999 => Judgement::Inaccuracy,
        MateSequence::MateLost if cur_cp > 700 => Judgement::Mistake,
        MateSequence::MateLost => Judgement::Blunder,
    };
    Some((seq, j))
}

/// Everything needed to classify one played move.
#[derive(Debug, Clone)]
pub struct MoveInput<'a> {
    pub mover: Color,
    /// White-POV score of the position before the move.
    pub prev: Score,
    /// White-POV score of the position after the move.
    pub cur: Score,
    /// Engine's best move (UCI) in the position before the move, if known.
    pub best_uci: Option<&'a str>,
    pub played_uci: &'a str,
    /// The position after the move is a named opening position and every
    /// earlier move was book too.
    pub is_book: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MoveVerdict {
    pub classification: Classification,
    pub lichess_judgement: Option<Judgement>,
    /// Mover's win% before and after the move (0..100, ceiled cp).
    pub win_before: f64,
    pub win_after: f64,
    /// Drop of the mover's win% expressed on the −1..1 winning-chances scale
    /// (ceiled cp). Positive = the move made things worse.
    pub delta_wc: f64,
}

/// Win% the mover must have had for a Mistake/Blunder to count as a missed win.
pub const MISSED_WIN_FROM: f64 = 85.0;
/// A non-best move that loses less than this (−1..1 scale) is "excellent".
pub const EXCELLENT_BELOW: f64 = 0.02;

pub fn judge(input: &MoveInput<'_>) -> MoveVerdict {
    let win_before = win_percent_for(input.prev, input.mover);
    let win_after = win_percent_for(input.cur, input.mover);
    let delta_wc = (win_before - win_after) / 50.0;
    let lj = lichess_judgement(input.prev, input.cur, input.mover);
    let played_best = input.best_uci.is_some_and(|b| b == input.played_uci);

    let classification = if input.is_book && lj.is_none() {
        Classification::Book
    } else if let Some(j) = lj {
        let had_win = win_before >= MISSED_WIN_FROM
            || input.prev.to_pov(input.mover).mate().is_some_and(|m| m > 0);
        match j {
            Judgement::Mistake | Judgement::Blunder if had_win && win_after < MISSED_WIN_FROM => {
                Classification::MissedWin
            }
            Judgement::Inaccuracy => Classification::Inaccuracy,
            Judgement::Mistake => Classification::Mistake,
            Judgement::Blunder => Classification::Blunder,
        }
    } else if played_best {
        Classification::Best
    } else if delta_wc < EXCELLENT_BELOW {
        Classification::Excellent
    } else {
        Classification::Good
    };

    MoveVerdict {
        classification,
        lichess_judgement: lj,
        win_before,
        win_after,
        delta_wc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Color::*;

    /// Smallest cp drop from `from` for which a White move loses at least
    /// `target` winning chances.
    fn cp_for_drop(from: i32, target: f64) -> i32 {
        let base = winning_chances(from);
        (from - 3000..=from)
            .rev()
            .find(|&c| base - winning_chances(c) >= target)
            .unwrap()
    }

    #[test]
    fn cp_thresholds_are_exact() {
        for (t, j) in [
            (0.1, Judgement::Inaccuracy),
            (0.2, Judgement::Mistake),
            (0.3, Judgement::Blunder),
        ] {
            let c = cp_for_drop(0, t);
            assert_eq!(lichess_judgement(Score::Cp(0), Score::Cp(c), White), Some(j));
            let above = lichess_judgement(Score::Cp(0), Score::Cp(c + 1), White);
            assert_ne!(above, Some(j), "threshold {t}");
        }
    }

    #[test]
    fn black_pov_mirrors() {
        assert_eq!(
            lichess_judgement(Score::Cp(0), Score::Cp(400), Black),
            Some(Judgement::Blunder)
        );
        assert_eq!(lichess_judgement(Score::Cp(0), Score::Cp(-400), Black), None);
    }

    #[test]
    fn mate_created() {
        assert_eq!(
            lichess_judgement(Score::Cp(-1200), Score::Mate(-4), White),
            Some(Judgement::Inaccuracy)
        );
        assert_eq!(
            lichess_judgement(Score::Cp(-800), Score::Mate(-4), White),
            Some(Judgement::Mistake)
        );
        assert_eq!(
            lichess_judgement(Score::Cp(0), Score::Mate(-4), White),
            Some(Judgement::Blunder)
        );
        assert_eq!(
            lichess_judgement(Score::Cp(0), Score::Mate(3), Black),
            Some(Judgement::Blunder)
        );
    }

    #[test]
    fn mate_lost_and_delayed() {
        assert_eq!(
            lichess_judgement(Score::Mate(2), Score::Cp(1500), White),
            Some(Judgement::Inaccuracy)
        );
        assert_eq!(
            lichess_judgement(Score::Mate(2), Score::Cp(300), White),
            Some(Judgement::Blunder)
        );
        assert_eq!(
            lichess_judgement(Score::Mate(2), Score::Mate(-5), White),
            Some(Judgement::Blunder)
        );
        assert_eq!(lichess_judgement(Score::Mate(2), Score::Mate(6), White), None);
        assert_eq!(lichess_judgement(Score::Cp(50), Score::Mate(3), White), None);
    }

    #[test]
    fn classification_layers() {
        let base = MoveInput {
            mover: White,
            prev: Score::Cp(30),
            cur: Score::Cp(25),
            best_uci: Some("e2e4"),
            played_uci: "e2e4",
            is_book: false,
        };
        assert_eq!(judge(&base).classification, Classification::Best);
        let v = judge(&MoveInput { played_uci: "d2d4", ..base.clone() });
        assert_eq!(v.classification, Classification::Excellent);
        // +0.30 -> -0.10 loses 0.074 winning chances: below Lichess' 0.1, above "excellent"
        let v = judge(&MoveInput { played_uci: "g2g4", cur: Score::Cp(-10), ..base.clone() });
        assert_eq!(v.classification, Classification::Good);
        // +0.30 -> -0.40 loses 0.129: Lichess inaccuracy
        let v = judge(&MoveInput { played_uci: "g2g4", cur: Score::Cp(-40), ..base.clone() });
        assert_eq!(v.classification, Classification::Inaccuracy);
        let v = judge(&MoveInput { is_book: true, played_uci: "d2d4", ..base.clone() });
        assert_eq!(v.classification, Classification::Book);
        let v = judge(&MoveInput {
            prev: Score::Cp(900),
            cur: Score::Cp(0),
            played_uci: "a2a3",
            ..base.clone()
        });
        assert_eq!(v.classification, Classification::MissedWin);
        assert_eq!(v.lichess_judgement, Some(Judgement::Blunder));
        let v = judge(&MoveInput {
            prev: Score::Mate(2),
            cur: Score::Cp(150),
            played_uci: "a2a3",
            ..base
        });
        assert_eq!(v.classification, Classification::MissedWin);
    }
}
