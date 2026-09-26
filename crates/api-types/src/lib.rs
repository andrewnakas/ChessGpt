//! Every type that crosses the HTTP boundary. `cargo xtask gen-types` writes
//! them to `web/src/lib/api/types.ts`; never edit that file by hand.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub use chess_core::accuracy::GameAccuracy;
pub use chess_core::book::Opening;
pub use chess_core::classify::{Classification, Judgement};
pub use chess_core::pgn::PlyMove;
pub use chess_core::position::{Phase, Side};
pub use chess_core::score::Score;
pub use engine::limits::EloTier;
pub use engine::types::{Limit, Terminal};

// ---------------------------------------------------------------- engine

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct LineDto {
    pub rank: u32,
    pub depth: u32,
    /// White POV.
    pub score: Score,
    pub wdl: Option<[u32; 3]>,
    pub pv_uci: Vec<String>,
    pub pv_san: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct EngineAnalysis {
    pub fen: String,
    pub depth: u32,
    pub lines: Vec<LineDto>,
    pub nodes: u64,
    pub nps: u64,
    pub time_ms: u64,
    pub terminal: Option<Terminal>,
    pub done: bool,
    pub engine: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EngineEvent {
    Update { analysis: EngineAnalysis },
    Done { analysis: EngineAnalysis },
    Error { message: String },
}

// ---------------------------------------------------------------- games

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GameSource {
    Pgn,
    Fen,
    Lichess,
    Chesscom,
}

impl GameSource {
    pub fn as_str(self) -> &'static str {
        match self {
            GameSource::Pgn => "pgn",
            GameSource::Fen => "fen",
            GameSource::Lichess => "lichess",
            GameSource::Chesscom => "chesscom",
        }
    }
    pub fn parse(s: &str) -> GameSource {
        match s {
            "fen" => GameSource::Fen,
            "lichess" => GameSource::Lichess,
            "chesscom" => GameSource::Chesscom,
            _ => GameSource::Pgn,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Running => "running",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
        }
    }
    pub fn parse(s: &str) -> JobStatus {
        match s {
            "running" => JobStatus::Running,
            "done" => JobStatus::Done,
            "failed" => JobStatus::Failed,
            "cancelled" => JobStatus::Cancelled,
            _ => JobStatus::Queued,
        }
    }
    pub fn is_finished(self) -> bool {
        matches!(self, JobStatus::Done | JobStatus::Failed | JobStatus::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct GameSummary {
    pub id: String,
    pub source: GameSource,
    pub source_id: Option<String>,
    pub white: String,
    pub black: String,
    pub white_elo: Option<u32>,
    pub black_elo: Option<u32>,
    pub result: String,
    pub date: Option<String>,
    pub time_control: Option<String>,
    pub eco: Option<String>,
    pub opening: Option<String>,
    /// Which side the app's user played, when known.
    pub user_side: Option<Side>,
    pub ply_count: u32,
    pub analysis_status: Option<JobStatus>,
    pub white_accuracy: Option<f64>,
    pub black_accuracy: Option<f64>,
    pub imported_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct GameDetail {
    pub summary: GameSummary,
    pub pgn: String,
    pub start_fen: String,
    pub moves: Vec<PlyMove>,
    pub analysis: Option<GameAnalysis>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct MoveEval {
    pub ply: u32,
    pub mover: Side,
    pub san: String,
    pub uci: String,
    /// White-POV eval of the position after this move.
    pub score: Score,
    pub depth: u32,
    /// Engine's choice in the position before the move.
    pub best_uci: Option<String>,
    pub best_san: Option<String>,
    pub best_line_san: Vec<String>,
    pub classification: Classification,
    pub lichess_judgement: Option<Judgement>,
    /// Mover's win% before and after (0..100).
    pub win_before: f64,
    pub win_after: f64,
    pub delta_wc: f64,
    pub accuracy: f64,
    pub phase: Phase,
    pub is_key_moment: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct BetterMove {
    pub san: String,
    pub line_san: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    /// Every move mentioned is legal and backed by an engine line.
    Ok,
    /// Offending text was removed or flagged.
    Partial,
    /// Could not be verified (e.g. free-text answer from a weak model).
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct Verification {
    pub status: VerificationStatus,
    pub issues: Vec<String>,
    /// SAN moves the text mentions that are legal but not in any engine line.
    pub unverified_moves: Vec<String>,
    /// SAN moves that are illegal or invented (removed where possible).
    #[serde(default)]
    pub rejected_moves: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct Explanation {
    pub ply: u32,
    pub headline: String,
    pub why_it_matters: String,
    pub better_move: Option<BetterMove>,
    pub concept_tags: Vec<String>,
    pub takeaway: String,
    pub mentioned_moves: Vec<String>,
    pub verification: Verification,
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct GameReview {
    pub text: String,
    pub themes: Vec<String>,
    pub verification: Verification,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct GameAnalysis {
    pub id: String,
    pub game_id: String,
    pub status: JobStatus,
    pub elo: u32,
    pub tier: EloTier,
    pub user_side: Option<Side>,
    pub engine: String,
    /// White-POV eval of the starting position.
    pub start_score: Option<Score>,
    pub white_accuracy: Option<f64>,
    pub black_accuracy: Option<f64>,
    /// Estimated rating each side played at in this game (see chess_core::rating).
    #[serde(default)]
    pub white_estimate: Option<u32>,
    #[serde(default)]
    pub black_estimate: Option<u32>,
    pub moves: Vec<MoveEval>,
    pub key_moments: Vec<u32>,
    pub explanations: Vec<Explanation>,
    pub review: Option<GameReview>,
    pub error: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct AnalyseGameRequest {
    /// Explain for this rating; defaults to the user's setting.
    pub elo: Option<u32>,
    pub user_side: Option<Side>,
    /// Ask the LLM to explain key moments (needs a provider).
    pub explain: bool,
    /// Discard any previous analysis and start over.
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct AnalyseGameResponse {
    pub analysis_id: String,
    pub status: JobStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum JobStage {
    Engine,
    DeepPass,
    Explaining,
    Review,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JobEvent {
    Snapshot { analysis: GameAnalysis },
    Progress { stage: JobStage, done: u32, total: u32 },
    Move { eval: MoveEval },
    Accuracy { white: Option<f64>, black: Option<f64> },
    KeyMoments { plies: Vec<u32> },
    ExplanationStarted { ply: u32 },
    Explanation { explanation: Explanation },
    ExplanationFailed { ply: u32, message: String },
    Review { review: GameReview },
    Done { analysis: GameAnalysis },
    Error { message: String },
}

// ---------------------------------------------------------------- progress

/// One analysed game in the user's progress history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ProgressGame {
    pub game_id: String,
    pub date: Option<String>,
    pub imported_at: i64,
    pub user_side: Side,
    pub opponent: String,
    pub user_elo: Option<u32>,
    pub opponent_elo: Option<u32>,
    /// 1, 0.5 or 0 for the user; None if unfinished.
    pub score: Option<f64>,
    pub time_control: Option<String>,
    pub accuracy: Option<f64>,
    /// Estimated rating the user played at in this game.
    pub estimate: Option<u32>,
    /// The user's moves, and how many were mistakes or blunders.
    pub moves: u32,
    pub errors: u32,
}

/// How often the user's mistakes involve one motif.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct MotifRate {
    pub tag: String,
    pub label: String,
    /// Mistakes where the better line used this motif (the user missed it).
    pub missed: u32,
    /// Mistakes that let the opponent use it.
    pub allowed: u32,
    pub per_100_moves: f64,
    /// Typical rate for players of the user's rating band, when known.
    pub baseline_per_100: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct PhaseRate {
    pub phase: Phase,
    pub moves: u32,
    pub errors: u32,
    pub per_100_moves: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct Progress {
    /// Oldest first.
    pub games: Vec<ProgressGame>,
    /// Rating from the last 20 per-game estimates (corrected for shrinkage).
    pub rolling_estimate: Option<u32>,
    /// ± margin of that rating (about one standard error).
    pub estimate_margin: Option<u32>,
    /// Performance rating over the last 20 games with a rated opponent.
    pub performance: Option<u32>,
    pub user_moves: u32,
    /// Most frequent first.
    pub motifs: Vec<MotifRate>,
    pub phases: Vec<PhaseRate>,
}

// ---------------------------------------------------------------- drills

/// How a drill item trains its technique.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DrillKind {
    /// Recognise it: answer a question about the position, no move played.
    Spot,
    /// Play it: find the moves.
    Find,
    /// Stop it: a careless move here walks into the technique; play a safe one.
    Defend,
    /// Use it: play the position out against the bot.
    Playout,
}

/// A recognition question. Tap a square (`answer_squares`) or pick one of
/// `choices` (`answer_choice`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct DrillQuiz {
    pub question: String,
    pub choices: Vec<String>,
    pub answer_choice: Option<u32>,
    pub answer_squares: Vec<String>,
    /// A move to draw on the board (UCI).
    pub arrow_uci: Option<String>,
    /// Shown after answering.
    pub explanation: String,
}

/// Play `moves` of your own moves against the bot and keep your winning
/// chances at or above `min_win_pct`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct PlayoutGoal {
    pub moves: u32,
    pub min_win_pct: f64,
    pub start_win_pct: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct DrillItem {
    pub id: String,
    pub kind: DrillKind,
    /// "variant" (the user's own position, transformed), "bank" (a Lichess puzzle).
    pub origin: String,
    pub fen: String,
    pub prompt: String,
    /// Find: the solver's moves and the replies (UCI).
    pub solution_uci: Vec<String>,
    /// Defend: every first move that holds.
    pub accept_uci: Vec<String>,
    /// Defend: the tempting move that walks into it, and the punishing line (SAN).
    pub trap_san: Vec<String>,
    /// The engine's line from the solution, for after solving (SAN).
    pub line_san: Vec<String>,
    pub quiz: Option<DrillQuiz>,
    pub goal: Option<PlayoutGoal>,
    /// Mildest first.
    pub hints: Vec<String>,
    /// Lichess puzzle rating, for bank items.
    pub rating: Option<u32>,
    /// Not attempted yet: None.
    pub solved: Option<bool>,
}

/// What a drill set was made from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DrillSource {
    /// A mistake in an analysed game.
    Mistake { analysis_id: String, ply: u32 },
    /// A training puzzle.
    Puzzle { id: String },
    /// A technique by name (a concept tag).
    Theme { tag: String },
    /// The user's weakest technique.
    Weakest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct DrillSet {
    pub id: String,
    /// Concept tag.
    pub technique: String,
    pub label: String,
    pub source: DrillSource,
    /// The game rating the set was pitched at.
    pub rating: u32,
    pub items: Vec<DrillItem>,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct DrillAttempt {
    pub solved: bool,
    pub ms: u32,
    pub hints_used: u32,
}

/// A motif behind one of the user's mistakes in an analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct MistakeMotif {
    pub ply: u32,
    /// Concept tag.
    pub tag: String,
    /// "missed" (the better line used it), "allowed" (the opponent's reply uses it), "coach".
    pub kind: String,
}

/// Drill results for one technique over the last 30 days.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct TechniqueMastery {
    pub tag: String,
    pub label: String,
    pub attempted: u32,
    pub solved: u32,
    /// Solved without hints.
    pub clean: u32,
    pub median_ms: Option<u32>,
}

/// The techniques that can be drilled, with the user's results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct DrillOverview {
    pub techniques: Vec<TechniqueMastery>,
    /// Weakest first; what the weekly plan drills.
    pub focus: Vec<String>,
    /// Sets finished in the last 7 days, against the weekly goal.
    pub week_done: u32,
    pub week_goal: u32,
    pub recent: Vec<DrillSetSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct DrillSetSummary {
    pub id: String,
    pub technique: String,
    pub label: String,
    pub items: u32,
    pub attempted: u32,
    pub solved: u32,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

// ---------------------------------------------------------------- linked accounts

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ConnectRequest {
    pub username: String,
}

/// What a sync of the user's Lichess / Chess.com games did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct SyncReport {
    /// The account name as the site spells it.
    pub username: Option<String>,
    /// Games new to the library.
    pub imported: u32,
    /// Of those, how many are being analysed now.
    pub analysing: u32,
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------- puzzles

/// A training puzzle: find the move from a position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct Puzzle {
    pub id: String,
    /// "mistake": a position from the user's own game.
    pub source: String,
    pub game_id: Option<String>,
    pub analysis_id: Option<String>,
    pub ply: Option<u32>,
    pub fen: String,
    pub solution_uci: Vec<String>,
    /// The engine's line from the solution on (SAN), for after solving.
    pub line_san: Vec<String>,
    pub themes: Vec<String>,
    /// When non-empty, any of these first moves solves it (defence puzzles).
    pub accept_uci: Vec<String>,
    pub reps: u32,
    pub lapses: u32,
    pub due_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct PuzzleAttempt {
    pub solved: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct PuzzleQueue {
    pub due: Vec<Puzzle>,
    pub total: u32,
    pub due_count: u32,
    /// Solved at least twice in a row.
    pub learned: u32,
}

// ---------------------------------------------------------------- import

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum ImportRequest {
    Pgn { pgn: String },
    Fen { fen: String },
    /// A lichess game id or URL.
    LichessGame { id: String },
    Lichess { username: String, max: Option<u32> },
    Chesscom { username: String, max: Option<u32> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ImportResponse {
    pub games: Vec<GameSummary>,
    /// Games already imported before.
    pub duplicates: u32,
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------- chat

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ToolCallView {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
    pub summary: String,
    pub is_error: bool,
    /// Structured result for the board (engine lines, played line).
    #[ts(type = "unknown")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ChatMessage {
    pub id: String,
    pub role: ChatRole,
    pub text: String,
    pub tool_calls: Vec<ToolCallView>,
    pub verification: Option<Verification>,
    pub fen: Option<String>,
    pub ply: Option<u32>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ChatThread {
    pub id: String,
    pub game_id: Option<String>,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ChatThreadDetail {
    pub thread: ChatThread,
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct CreateThreadRequest {
    pub game_id: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct SendMessageRequest {
    pub text: String,
    /// Board position the question is about.
    pub fen: String,
    pub ply: Option<u32>,
    /// SAN moves from the game start to `fen`, when on a game.
    pub move_path: Vec<String>,
    pub elo: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatEvent {
    UserMessage { message: ChatMessage },
    TextDelta { text: String },
    Thinking { text: String },
    ToolCall { id: String, name: String, input: serde_json::Value },
    ToolResult { call: ToolCallView },
    Verification { verification: Verification },
    Done { message: ChatMessage },
    Error { message: String },
}

// ---------------------------------------------------------------- settings

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Anthropic,
    Openai,
    Openrouter,
    Ollama,
    /// Any OpenAI-compatible server (LM Studio, llama.cpp, vLLM, ...).
    OpenaiCompatible,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Openai => "openai",
            ProviderKind::Openrouter => "openrouter",
            ProviderKind::Ollama => "ollama",
            ProviderKind::OpenaiCompatible => "openai_compatible",
        }
    }
    pub fn parse(s: &str) -> Option<ProviderKind> {
        Some(match s {
            "anthropic" => ProviderKind::Anthropic,
            "openai" => ProviderKind::Openai,
            "openrouter" => ProviderKind::Openrouter,
            "ollama" => ProviderKind::Ollama,
            "openai_compatible" => ProviderKind::OpenaiCompatible,
            _ => return None,
        })
    }
    pub fn default_base_url(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "https://api.anthropic.com",
            ProviderKind::Openai => "https://api.openai.com/v1",
            ProviderKind::Openrouter => "https://openrouter.ai/api/v1",
            ProviderKind::Ollama => "http://localhost:11434/v1",
            ProviderKind::OpenaiCompatible => "http://localhost:1234/v1",
        }
    }
    pub fn default_model(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "claude-opus-5",
            ProviderKind::Openai => "gpt-5",
            ProviderKind::Openrouter => "anthropic/claude-opus-5",
            ProviderKind::Ollama => "qwen3:8b",
            ProviderKind::OpenaiCompatible => "local-model",
        }
    }
    pub fn needs_key(self) -> bool {
        matches!(self, ProviderKind::Anthropic | ProviderKind::Openai | ProviderKind::Openrouter)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct Provider {
    pub id: String,
    pub kind: ProviderKind,
    pub label: String,
    pub base_url: String,
    pub model: String,
    pub has_key: bool,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ProviderInput {
    pub kind: ProviderKind,
    pub label: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    /// `None` keeps the stored key on update; empty string clears it.
    pub api_key: Option<String>,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ProviderTestResult {
    pub ok: bool,
    pub message: String,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct Settings {
    pub elo: u32,
    pub lichess_username: Option<String>,
    pub chesscom_username: Option<String>,
    /// Allow calls to the Lichess opening explorer (needs a Lichess token).
    pub explorer_enabled: bool,
    /// A Lichess personal API token is stored. Lichess requires one for
    /// username game export and the opening explorer.
    pub has_lichess_token: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct SettingsInput {
    pub elo: u32,
    pub lichess_username: Option<String>,
    pub chesscom_username: Option<String>,
    pub explorer_enabled: bool,
    /// `None` keeps the stored token; empty string clears it.
    pub lichess_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct Meta {
    pub version: String,
    pub engine: String,
    pub engine_threads: u32,
    pub engine_workers: u32,
    pub mode: String,
    pub has_provider: bool,
    /// The site operator supplies the AI model; users never configure keys.
    pub managed_provider: bool,
    /// e.g. "Claude (claude-opus-5)" when managed.
    pub provider_label: Option<String>,
    /// Today's share of the site's AI budget already used, 0..1, when capped.
    pub budget_used: Option<f64>,
    /// A coach model the browser can download and run itself (WebGPU).
    pub device_model: Option<DeviceModel>,
}

/// An in-browser coach model (WebLLM / MLC format).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct DeviceModel {
    /// MLC model id, e.g. "Qwen3-4B-q4f16_1-MLC".
    pub id: String,
    /// Weights location for a model outside WebLLM's built-in list.
    pub url: Option<String>,
    /// Compiled model library (.wasm) for such a model.
    pub lib_url: Option<String>,
    /// Approximate download size.
    pub size_mb: u32,
}

/// A coach request for the browser's model: an OpenAI Chat Completions body.
/// Answer with `POST /api/llm/relay/{id}` and the `chat.completion` object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct LlmRelayRequest {
    pub id: String,
    pub body: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ApiError {
    pub error: String,
}

// ---------------------------------------------------------------- codegen

macro_rules! decls {
    ($cfg:expr; $($t:ty),* $(,)?) => {{
        let mut out = String::new();
        $(
            out.push_str("export ");
            out.push_str(&<$t as TS>::decl($cfg));
            out.push_str("\n\n");
        )*
        out
    }};
}

/// All TypeScript declarations as one module.
pub fn typescript() -> String {
    let cfg = ts_rs::Config::new().with_large_int("number");
    let body = decls!(&cfg;
        Score, Side, Phase, Classification, Judgement, Opening, PlyMove, GameAccuracy,
        EloTier, Limit, Terminal,
        LineDto, EngineAnalysis, EngineEvent,
        GameSource, JobStatus, GameSummary, GameDetail, MoveEval, BetterMove,
        VerificationStatus, Verification, Explanation, GameReview, GameAnalysis,
        AnalyseGameRequest, AnalyseGameResponse, JobStage, JobEvent,
        ImportRequest, ImportResponse, DeviceModel, LlmRelayRequest,
        ProgressGame, MotifRate, PhaseRate, Progress, Puzzle, PuzzleAttempt, PuzzleQueue, ConnectRequest, SyncReport,
        DrillKind, DrillQuiz, PlayoutGoal, DrillItem, DrillSource, DrillSet, DrillAttempt, TechniqueMastery,
        DrillOverview, DrillSetSummary, MistakeMotif,
        ChatRole, ToolCallView, ChatMessage, ChatThread, ChatThreadDetail,
        CreateThreadRequest, SendMessageRequest, ChatEvent,
        ProviderKind, Provider, ProviderInput, ProviderTestResult, Settings, SettingsInput, Meta, ApiError,
    );
    format!(
        "// GENERATED by `cargo xtask gen-types` from crates/api-types. Do not edit.\n\
         /* eslint-disable */\n\n\
         export type JsonValue = number | string | boolean | Array<JsonValue> | {{ [key in string]?: JsonValue }} | null;\n\n{body}"
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn typescript_is_generated() {
        let ts = super::typescript();
        assert!(ts.contains("export type Score ="), "{}", &ts[..400.min(ts.len())]);
        assert!(ts.contains("export type JobEvent ="));
        assert!(!ts.contains("bigint"));
    }
}
