use chess_core::Score;
use serde::{Deserialize, Serialize};

/// Search limit. Any combination may be set; the engine stops at whichever
/// is reached first. At least one must be set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Limit {
    pub depth: Option<u32>,
    pub movetime_ms: Option<u64>,
    pub nodes: Option<u64>,
}

impl Limit {
    pub fn depth(d: u32) -> Limit {
        Limit { depth: Some(d), ..Default::default() }
    }
    pub fn movetime(ms: u64) -> Limit {
        Limit { movetime_ms: Some(ms), ..Default::default() }
    }
    pub fn depth_or_time(d: u32, ms: u64) -> Limit {
        Limit { depth: Some(d), movetime_ms: Some(ms), nodes: None }
    }
    pub fn is_empty(&self) -> bool {
        self.depth.is_none() && self.movetime_ms.is_none() && self.nodes.is_none()
    }
    pub(crate) fn go_command(&self) -> String {
        let mut s = String::from("go");
        if let Some(d) = self.depth {
            s += &format!(" depth {d}");
        }
        if let Some(t) = self.movetime_ms {
            s += &format!(" movetime {t}");
        }
        if let Some(n) = self.nodes {
            s += &format!(" nodes {n}");
        }
        if self.is_empty() {
            s += " depth 18";
        }
        s
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Priority {
    /// Whole-game batch analysis.
    #[default]
    Batch,
    /// A person is waiting on this (board analysis, coach tool call).
    Interactive,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisRequest {
    pub fen: String,
    pub multipv: u32,
    pub limit: Limit,
    pub priority: Priority,
}

/// One principal variation. Scores are **White POV**.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PvLine {
    /// 1-based MultiPV rank.
    pub rank: u32,
    pub depth: u32,
    pub score: Score,
    /// Win/draw/loss per mille, White POV.
    pub wdl: Option<[u32; 3]>,
    pub pv: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Terminal {
    Checkmate,
    Stalemate,
}

/// Engine output for one position, partial or final.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Analysis {
    pub fen: String,
    pub depth: u32,
    pub multipv: u32,
    pub lines: Vec<PvLine>,
    pub nodes: u64,
    pub nps: u64,
    pub time_ms: u64,
    pub best_move: Option<String>,
    /// Set when the position is already over; `lines` is then empty.
    pub terminal: Option<Terminal>,
    pub done: bool,
    pub engine: String,
}

impl Analysis {
    /// White-POV score of the position (best line, or terminal result).
    pub fn score(&self) -> Option<Score> {
        self.lines.first().map(|l| l.score)
    }
}

#[derive(Debug, thiserror::Error, Clone)]
pub enum EngineError {
    #[error("engine binary not found at {0}")]
    NotFound(String),
    #[error("failed to start engine: {0}")]
    Spawn(String),
    #[error("engine protocol error: {0}")]
    Protocol(String),
    #[error("engine process died")]
    Died,
    #[error("invalid position: {0}")]
    BadFen(String),
    #[error("analysis cancelled")]
    Cancelled,
    #[error("engine pool is shut down")]
    Shutdown,
}
