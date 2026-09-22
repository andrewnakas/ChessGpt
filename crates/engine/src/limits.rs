//! Search budgets by player strength. Classification stability, not engine
//! strength, drives depth: weaker players get faster feedback.

use serde::{Deserialize, Serialize};

use crate::types::Limit;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum EloTier {
    /// Under 1200.
    Beginner,
    /// 1200 to 1799.
    Intermediate,
    /// 1800 to 2199.
    Advanced,
    /// 2200 and up.
    Expert,
}

impl EloTier {
    pub fn from_elo(elo: u32) -> EloTier {
        match elo {
            0..1200 => EloTier::Beginner,
            1200..1800 => EloTier::Intermediate,
            1800..2200 => EloTier::Advanced,
            _ => EloTier::Expert,
        }
    }

    /// Per-position budget for whole-game analysis.
    pub fn batch(self) -> Limit {
        match self {
            EloTier::Beginner => Limit::depth_or_time(16, 400),
            EloTier::Intermediate => Limit::depth_or_time(18, 600),
            EloTier::Advanced => Limit::depth_or_time(20, 900),
            EloTier::Expert => Limit::depth_or_time(22, 1500),
        }
    }

    /// Budget for interactive analysis, coach tools, and key-moment deep passes.
    pub fn interactive(self) -> Limit {
        match self {
            EloTier::Beginner => Limit::depth_or_time(20, 2000),
            EloTier::Intermediate => Limit::depth_or_time(22, 3000),
            EloTier::Advanced => Limit::depth_or_time(24, 4000),
            EloTier::Expert => Limit::depth_or_time(26, 6000),
        }
    }

    pub fn multipv(self) -> u32 {
        match self {
            EloTier::Beginner | EloTier::Intermediate => 3,
            EloTier::Advanced => 4,
            EloTier::Expert => 5,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EloTier::Beginner => "beginner (under 1200)",
            EloTier::Intermediate => "intermediate (1200-1800)",
            EloTier::Advanced => "advanced (1800-2200)",
            EloTier::Expert => "expert (2200+)",
        }
    }
}
