use serde::{Deserialize, Serialize};
use shakmaty::Color;

/// Engine score. Unless stated otherwise it is from **White's** point of view.
/// `Mate(n)`: n > 0 means White mates in n; n < 0 means Black mates in |n|.
/// A checkmated position is stored as `Mate(±1)` in the winner's favour (see
/// [`Score::checkmated`]) so that sign-based rules keep working.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Score {
    Cp(i32),
    Mate(i32),
}

pub const CP_CEILING: i32 = 1000;
pub const CP_INITIAL: i32 = 15;

impl Score {
    pub fn negate(self) -> Score {
        match self {
            Score::Cp(c) => Score::Cp(-c),
            Score::Mate(m) => Score::Mate(-m),
        }
    }

    /// Flip a side-to-move score into White's point of view.
    pub fn from_pov(self, pov: Color) -> Score {
        match pov {
            Color::White => self,
            Color::Black => self.negate(),
        }
    }

    /// Express a White-POV score from `pov`'s point of view.
    pub fn to_pov(self, pov: Color) -> Score {
        self.from_pov(pov)
    }

    /// Score of a position where `loser` is checkmated, White POV.
    pub fn checkmated(loser: Color) -> Score {
        match loser {
            Color::White => Score::Mate(-1),
            Color::Black => Score::Mate(1),
        }
    }

    pub fn cp(self) -> Option<i32> {
        match self {
            Score::Cp(c) => Some(c),
            Score::Mate(_) => None,
        }
    }

    pub fn mate(self) -> Option<i32> {
        match self {
            Score::Mate(m) => Some(m),
            Score::Cp(_) => None,
        }
    }

    /// Lichess `forceAsCp`: mates become ±1000, cp is clamped to ±1000.
    pub fn ceiled_cp(self) -> i32 {
        match self {
            Score::Cp(c) => c.clamp(-CP_CEILING, CP_CEILING),
            Score::Mate(m) => {
                if m >= 0 {
                    CP_CEILING
                } else {
                    -CP_CEILING
                }
            }
        }
    }

    /// Human form, White POV: "+1.23", "-0.40", "#3", "#-2".
    pub fn display(self) -> String {
        match self {
            Score::Cp(c) => format!("{:+.2}", c as f64 / 100.0),
            Score::Mate(m) => format!("#{m}"),
        }
    }
}
