//! Rating bands for the coach: how to talk to a player, how long the lines
//! may be, and which moments are worth their time. (Engine budgets use the
//! coarser `EloTier`.)

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Band {
    /// Under 1000.
    Novice,
    /// 1000 to 1399.
    Developing,
    /// 1400 to 1799.
    Club,
    /// 1800 to 2099.
    StrongClub,
    /// 2100 to 2399.
    Expert,
    /// 2400 and up.
    Master,
}

impl Band {
    pub fn from_elo(elo: u32) -> Band {
        match elo {
            0..1000 => Band::Novice,
            1000..1400 => Band::Developing,
            1400..1800 => Band::Club,
            1800..2100 => Band::StrongClub,
            2100..2400 => Band::Expert,
            _ => Band::Master,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Band::Novice => "new to the game",
            Band::Developing => "developing player",
            Band::Club => "club player",
            Band::StrongClub => "strong club player",
            Band::Expert => "expert",
            Band::Master => "master-level player",
        }
    }

    /// How to write for this player.
    pub fn guidance(self) -> &'static str {
        match self {
            Band::Novice => {
                "Use plain, friendly words and no jargon. Most games at this level are decided by pieces left undefended and by missed checks and captures, so focus on those: say which piece was loose or what the threat was. If you use a chess term, explain it in a few words. Show at most two or three moves of any line."
            }
            Band::Developing => {
                "Keep the language simple. Name basic tactics (fork, pin, skewer, hanging piece) and explain them briefly. Put safety first: checks, captures and threats before plans. Short lines only."
            }
            Band::Club => {
                "Standard chess vocabulary is fine (pin, outpost, open file, weak square, pawn break). Give the key idea and the concrete reason it works or fails, and connect it to a plan the player can reuse."
            }
            Band::StrongClub => {
                "Assume solid tactics. Be concrete: name the critical line, then the positional factors behind the evaluation, such as piece activity, pawn structure and king safety."
            }
            Band::Expert => {
                "Be concise and precise. Discuss the critical variation, move-order points and prophylaxis. Skip basic definitions."
            }
            Band::Master => {
                "Write as one strong player to another: dense and exact, focused on the critical line and the nuance that decided it."
            }
        }
    }

    pub fn length_hint(self) -> &'static str {
        match self {
            Band::Novice => "Two or three short sentences.",
            Band::Developing => "Two or three sentences.",
            Band::Club => "Two to four sentences.",
            Band::StrongClub => "Up to five sentences.",
            Band::Expert | Band::Master => "Up to six dense sentences.",
        }
    }

    /// Longest engine line to show, in moves (plies).
    pub fn max_line(self) -> usize {
        match self {
            Band::Novice => 3,
            Band::Developing => 4,
            Band::Club => 6,
            Band::StrongClub => 8,
            Band::Expert | Band::Master => 10,
        }
    }

    /// Lowest error class worth a key moment: players under 1400 lose to
    /// mistakes and blunders, so inaccuracies would only be noise.
    pub fn min_severity(self) -> f64 {
        match self {
            Band::Novice | Band::Developing => 2.0,
            _ => 1.0,
        }
    }

    /// How many moments to explain per game.
    pub fn max_moments(self) -> usize {
        match self {
            Band::Novice => 5,
            Band::Developing => 6,
            _ => crate::key_moments::DEFAULT_MAX,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_cover_the_range() {
        assert_eq!(Band::from_elo(0), Band::Novice);
        assert_eq!(Band::from_elo(999), Band::Novice);
        assert_eq!(Band::from_elo(1000), Band::Developing);
        assert_eq!(Band::from_elo(1799), Band::Club);
        assert_eq!(Band::from_elo(2400), Band::Master);
        assert!(Band::Novice.max_line() < Band::Master.max_line());
    }
}
