//! chessgpt as a remote MCP server: Claude and ChatGPT users add
//! `https://chessgpt.com/mcp` and their own assistant (on their own
//! subscription) calls Stockfish and their chessgpt library through these
//! tools. Every result links back to the site.

pub mod widget;

use std::time::Duration;

use api_types::{AnalyseGameRequest, Classification, GameSource, ImportRequest, ImportResponse, Side};
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use coach::tools::{ToolEnv, run_tool};
use coach::verify::Grounding;
use engine::EloTier;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, Implementation, ListResourcesResult, MetaObject, PaginatedRequestParams,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource, ResourceContents,
    ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, RoleServer, ServerHandler, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::base_url;
use crate::state::AppState;

pub const WIDGET_URI: &str = "ui://chessgpt/board-v1.html";
pub const SCOPE: &str = "chess";

/// Who an MCP request acts for (set by [`require_token`]).
#[derive(Clone, Debug)]
pub struct McpUser(pub String);

// ------------------------------------------------------------ auth layer

fn resource_metadata_url(base: &str) -> String {
    format!("{base}/.well-known/oauth-protected-resource/mcp")
}

fn challenge(base: &str, error: Option<&str>) -> Response {
    let mut v = format!("Bearer resource_metadata=\"{}\", scope=\"{SCOPE}\"", resource_metadata_url(base));
    if let Some(e) = error {
        v.push_str(&format!(", error=\"{e}\""));
    }
    let mut r = (StatusCode::UNAUTHORIZED, "authorization required").into_response();
    if let Ok(h) = HeaderValue::from_str(&v) {
        r.headers_mut().insert(header::WWW_AUTHENTICATE, h);
    }
    r
}

/// Bearer-token gate in front of `/mcp` (hosted mode). Local mode acts as
/// the single local user.
pub async fn require_token(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    if !state.accounts_enabled() {
        let id = state.db.user_id().to_string();
        req.extensions_mut().insert(McpUser(id));
        return next.run(req).await;
    }
    let base = base_url(&state, req.headers());
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer ").or_else(|| h.strip_prefix("bearer ")))
        .map(str::trim)
        .map(String::from);
    let Some(token) = token else { return challenge(&base, None) };
    let grant = match state.db.access_grant(&token).await {
        Ok(Some(g)) => g,
        Ok(None) => return challenge(&base, Some("invalid_token")),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    // RFC 8707 audience check: tokens bound to another resource are refused.
    if let Some(res) = &grant.resource
        && res.trim_end_matches('/') != format!("{base}/mcp")
    {
        return challenge(&base, Some("invalid_token"));
    }
    req.extensions_mut().insert(McpUser(grant.user_id));
    next.run(req).await
}

// ------------------------------------------------------------ service

pub fn service(state: &AppState) -> StreamableHttpService<ChessMcp, LocalSessionManager> {
    let mut hosts = vec!["localhost".to_string(), "127.0.0.1".to_string(), "::1".to_string()];
    if let Some(u) = &state.config.public_url
        && let Some(host) = u.split("://").nth(1).map(|h| h.trim_end_matches('/'))
    {
        hosts.push(host.split(':').next().unwrap_or(host).to_string());
        hosts.push(host.to_string());
    }
    // Extra hostnames the server is reached through (e.g. the Cloudflare
    // Tunnel origin behind the edge Worker).
    if let Ok(extra) = std::env::var("CHESSGPT_ALLOWED_HOSTS") {
        hosts.extend(extra.split(',').map(|h| h.trim().to_string()).filter(|h| !h.is_empty()));
    }
    let mut cfg = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None);
    cfg = if state.config.public_url.is_some() || !state.accounts_enabled() {
        cfg.with_allowed_hosts(hosts)
    } else {
        tracing::warn!("CHESSGPT_PUBLIC_URL is not set: the MCP endpoint accepts any Host header");
        cfg.disable_allowed_hosts()
    };
    let st = state.clone();
    StreamableHttpService::new(move || Ok(ChessMcp::new(st.clone())), Default::default(), cfg)
}

#[derive(Clone)]
pub struct ChessMcp {
    state: AppState,
    tool_router: ToolRouter<Self>,
}

fn ui_meta() -> MetaObject {
    let v = json!({ "ui": { "resourceUri": WIDGET_URI }, "openai/outputTemplate": WIDGET_URI });
    MetaObject(v.as_object().cloned().unwrap_or_default())
}

fn ui_meta_with_status(invoking: &str, invoked: &str) -> MetaObject {
    let mut m = ui_meta();
    m.0.insert("openai/toolInvocation/invoking".into(), json!(invoking));
    m.0.insert("openai/toolInvocation/invoked".into(), json!(invoked));
    m
}

/// Human summary first, then the JSON (for clients that ignore structuredContent).
fn result(summary: String, data: Value) -> CallToolResult {
    let mut r = CallToolResult::success(vec![ContentBlock::text(summary), ContentBlock::text(data.to_string())]);
    r.structured_content = Some(data);
    r
}

fn fail(msg: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(msg.into())])
}

struct Ctx {
    state: AppState,
    base: String,
}

impl ChessMcp {
    pub fn new(state: AppState) -> ChessMcp {
        ChessMcp { state, tool_router: Self::tool_router() }
    }

    fn ctx(&self, rc: &RequestContext<RoleServer>) -> Result<Ctx, ErrorData> {
        let parts = rc
            .extensions
            .get::<axum::http::request::Parts>()
            .ok_or_else(|| ErrorData::internal_error("missing request context", None))?;
        let user = parts
            .extensions
            .get::<McpUser>()
            .ok_or_else(|| ErrorData::invalid_request("not signed in", None))?;
        Ok(Ctx { state: self.state.scoped(&user.0), base: base_url(&self.state, &parts.headers) })
    }

    async fn env(&self, c: &Ctx) -> ToolEnv {
        let s = c.state.db.settings().await.ok();
        let elo = s.as_ref().map(|s| s.elo).unwrap_or(1500);
        let explorer = match (s.as_ref().is_some_and(|s| s.explorer_enabled), c.state.db.lichess_token().await.ok().flatten()) {
            (true, Some(t)) => Some((c.state.importers.clone(), t)),
            _ => None,
        };
        ToolEnv { pool: c.state.pool.clone(), tier: EloTier::from_elo(elo), explorer, game: None }
    }

    async fn run_coach_tool(&self, rc: &RequestContext<RoleServer>, name: &str, input: Value) -> Result<CallToolResult, ErrorData> {
        let c = self.ctx(rc)?;
        let env = self.env(&c).await;
        let mut g = Grounding::new();
        let out = run_tool(&env, &mut g, "mcp", name, &input).await;
        if out.view.is_error {
            return Ok(fail(out.content));
        }
        let fen = input["fen"].as_str().unwrap_or_default();
        let url = format!("{}/board?fen={}", c.base, crate::auth::enc(fen));
        let mut data = out.view.data.unwrap_or_else(|| json!({}));
        if let Some(o) = data.as_object_mut() {
            o.insert("kind".into(), json!(name));
            o.insert("fen".into(), json!(o.get("final_fen").cloned().unwrap_or(json!(fen))));
            o.insert("start_fen".into(), json!(fen));
            o.insert("url".into(), json!(url));
        }
        Ok(result(format!("{}\nOpen on chessgpt: {url}", out.content.trim_end()), data))
    }
}

// ------------------------------------------------------------ tool inputs

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PositionArgs {
    /// Position in FEN.
    pub fen: String,
    /// How many best lines to return (1-5, default 3).
    pub lines: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LineArgs {
    /// Starting position in FEN.
    pub fen: String,
    /// SAN moves to play in order, e.g. ["e4", "e5", "Nf3"].
    pub moves: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FenArgs {
    /// Position in FEN.
    pub fen: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Database {
    Masters,
    Lichess,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OpeningArgs {
    /// Position in FEN.
    pub fen: String,
    /// "masters" for over-the-board master games, "lichess" for online games.
    pub database: Database,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Color {
    White,
    Black,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GameArgs {
    /// Full PGN of the game. Give this or `lichess_url`.
    pub pgn: Option<String>,
    /// A lichess.org game link or 8-character id. Give this or `pgn`.
    pub lichess_url: Option<String>,
    /// Which side the user played, if known.
    pub my_color: Option<Color>,
    /// The user's rating; defaults to the rating saved on chessgpt.
    pub my_rating: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListArgs {
    /// How many recent games (default 10, max 50).
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GameIdArgs {
    /// Game id from `my_games` or `analyze_game`.
    pub game_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Site {
    Lichess,
    Chesscom,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ImportArgs {
    /// Where to import from.
    pub site: Site,
    /// Username on that site.
    pub username: String,
    /// How many recent games (default 10, max 50).
    pub max: Option<u32>,
}

fn side_of(c: &Option<Color>) -> Option<Side> {
    c.as_ref().map(|c| match c {
        Color::White => Side::White,
        Color::Black => Side::Black,
    })
}

// ------------------------------------------------------------ tools

#[tool_router]
impl ChessMcp {
    #[tool(
        name = "analyze_position",
        title = "Analyze a position",
        description = "Run Stockfish on a chess position (FEN). Returns the best lines in SAN with evaluations from White's point of view, and shows the board. Call this before judging any position or recommending a move; never guess evaluations.",
        annotations(title = "Analyze a position", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        meta = ui_meta_with_status("Running Stockfish…", "Analysis ready")
    )]
    async fn analyze_position(&self, Parameters(a): Parameters<PositionArgs>, rc: RequestContext<RoleServer>) -> Result<CallToolResult, ErrorData> {
        self.run_coach_tool(&rc, "analyse_position", json!({"fen": a.fen, "lines": a.lines})).await
    }

    #[tool(
        name = "check_line",
        title = "Check a line",
        description = "Play a sequence of SAN moves from a FEN, confirm every move is legal, and evaluate the final position with Stockfish. Use this to verify any line of more than two moves before showing it to the user. If a move is illegal the result says which one and lists the legal moves there.",
        annotations(title = "Check a line", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        meta = ui_meta_with_status("Checking the line…", "Line checked")
    )]
    async fn check_line(&self, Parameters(a): Parameters<LineArgs>, rc: RequestContext<RoleServer>) -> Result<CallToolResult, ErrorData> {
        self.run_coach_tool(&rc, "play_line", json!({"fen": a.fen, "moves": a.moves})).await
    }

    #[tool(
        name = "legal_moves",
        title = "List legal moves",
        description = "List every legal move in a position in SAN, with checks and captures called out.",
        annotations(title = "List legal moves", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn legal_moves(&self, Parameters(a): Parameters<FenArgs>, rc: RequestContext<RoleServer>) -> Result<CallToolResult, ErrorData> {
        let c = self.ctx(&rc)?;
        let env = self.env(&c).await;
        let out = run_tool(&env, &mut Grounding::new(), "mcp", "legal_moves", &json!({"fen": a.fen})).await;
        Ok(if out.view.is_error { fail(out.content) } else { CallToolResult::success(vec![ContentBlock::text(out.content)]) })
    }

    #[tool(
        name = "opening_explorer",
        title = "Opening explorer",
        description = "Look up a position in the Lichess opening explorer: opening name and the most played moves with results. Needs the user to have signed in to chessgpt with Lichess.",
        annotations(title = "Opening explorer", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    async fn opening_explorer(&self, Parameters(a): Parameters<OpeningArgs>, rc: RequestContext<RoleServer>) -> Result<CallToolResult, ErrorData> {
        let c = self.ctx(&rc)?;
        let env = self.env(&c).await;
        if env.explorer.is_none() {
            return Ok(fail(format!(
                "The opening explorer needs a Lichess sign-in. Ask the user to sign in with Lichess at {}/login, then try again.",
                c.base
            )));
        }
        let db = match a.database {
            Database::Masters => "masters",
            Database::Lichess => "lichess",
        };
        let out = run_tool(&env, &mut Grounding::new(), "mcp", "opening_lookup", &json!({"fen": a.fen, "database": db})).await;
        Ok(if out.view.is_error { fail(out.content) } else { CallToolResult::success(vec![ContentBlock::text(out.content)]) })
    }

    #[tool(
        name = "analyze_game",
        title = "Analyze a game",
        description = "Save a game (PGN or Lichess link) to the user's chessgpt library and analyse every move with Stockfish, classifying inaccuracies, mistakes and blunders exactly like Lichess. Returns accuracy, the key moments with the engine's better move and line, and a link to the full interactive review on chessgpt.com. Explain the key moments to the user at their rating, citing only the moves and lines returned here.",
        annotations(title = "Analyze a game", read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = true),
        meta = ui_meta_with_status("Analysing every move…", "Game analysed")
    )]
    async fn analyze_game(&self, Parameters(a): Parameters<GameArgs>, rc: RequestContext<RoleServer>) -> Result<CallToolResult, ErrorData> {
        let c = self.ctx(&rc)?;
        let req = match (&a.pgn, &a.lichess_url) {
            (Some(p), _) if !p.trim().is_empty() => ImportRequest::Pgn { pgn: p.clone() },
            (_, Some(u)) if !u.trim().is_empty() => ImportRequest::LichessGame { id: u.clone() },
            _ => return Ok(fail("Give either `pgn` or `lichess_url`.")),
        };
        let game_id = match import(&c.state, req).await {
            Ok(mut r) if !r.games.is_empty() => r.games.remove(0).id,
            Ok(r) => return Ok(fail(format!("Could not read the game: {}", r.errors.join("; ")))),
            Err(e) => return Ok(fail(e)),
        };
        let req = AnalyseGameRequest { elo: a.my_rating, user_side: side_of(&a.my_color), explain: false, force: false };
        let started = match crate::routes::games::start_analysis(&c.state, game_id.clone(), &req).await {
            Ok(s) => s,
            Err(e) => return Ok(fail(e.1)),
        };
        // Stay under proxy timeouts (Cloudflare closes requests at 100 s).
        let analysis = match crate::jobs::wait_done(&c.state, &started.analysis_id, Duration::from_secs(75)).await {
            Ok(an) => an,
            Err(e) => return Ok(fail(e)),
        };
        if !analysis.status.is_finished() {
            let url = format!("{}/analyse/{}", c.base, game_id);
            return Ok(result(
                format!("The analysis is still running. Call game_report with game_id {game_id} in about a minute, or open {url}"),
                json!({"kind": "game", "game_id": game_id, "status": "running", "url": url}),
            ));
        }
        game_report(&c, &game_id, Some(analysis)).await
    }

    #[tool(
        name = "game_report",
        title = "Game report",
        description = "Get the stored analysis of one of the user's games: accuracy, every key moment with the position, the move played, its classification, and the engine's better line, plus a link to the review on chessgpt.com.",
        annotations(title = "Game report", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        meta = ui_meta()
    )]
    async fn game_report(&self, Parameters(a): Parameters<GameIdArgs>, rc: RequestContext<RoleServer>) -> Result<CallToolResult, ErrorData> {
        let c = self.ctx(&rc)?;
        game_report(&c, &a.game_id, None).await
    }

    #[tool(
        name = "my_games",
        title = "My games",
        description = "List the user's most recent games saved on chessgpt, with result, opening and accuracy.",
        annotations(title = "My games", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn my_games(&self, Parameters(a): Parameters<ListArgs>, rc: RequestContext<RoleServer>) -> Result<CallToolResult, ErrorData> {
        let c = self.ctx(&rc)?;
        let games = c.state.db.list_games(a.limit.unwrap_or(10).clamp(1, 50), 0).await.map_err(db_err)?;
        let rows: Vec<Value> = games
            .iter()
            .map(|g| {
                json!({
                    "game_id": g.id, "white": g.white, "black": g.black, "result": g.result,
                    "opening": g.opening, "date": g.date, "user_side": g.user_side,
                    "white_accuracy": g.white_accuracy.map(round1), "black_accuracy": g.black_accuracy.map(round1),
                    "analysed": g.analysis_status.is_some_and(|s| s == api_types::JobStatus::Done),
                    "url": format!("{}/analyse/{}", c.base, g.id)
                })
            })
            .collect();
        let text = if rows.is_empty() {
            format!("No games saved yet. Import some with import_games or at {}/import.", c.base)
        } else {
            games
                .iter()
                .map(|g| format!("- {} vs {} {} ({}) id {}", g.white, g.black, g.result, g.opening.as_deref().unwrap_or("?"), g.id))
                .collect::<Vec<_>>()
                .join("\n")
        };
        Ok(result(text, json!({ "kind": "games", "games": rows, "url": format!("{}/", c.base) })))
    }

    #[tool(
        name = "import_games",
        title = "Import recent games",
        description = "Import the user's recent games from Lichess or Chess.com into their chessgpt library. Lichess import needs the user to have signed in to chessgpt with Lichess.",
        annotations(title = "Import recent games", read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    async fn import_games(&self, Parameters(a): Parameters<ImportArgs>, rc: RequestContext<RoleServer>) -> Result<CallToolResult, ErrorData> {
        let c = self.ctx(&rc)?;
        let max = Some(a.max.unwrap_or(10).clamp(1, 50));
        let req = match a.site {
            Site::Lichess => ImportRequest::Lichess { username: a.username, max },
            Site::Chesscom => ImportRequest::Chesscom { username: a.username, max },
        };
        match import(&c.state, req).await {
            Ok(r) => Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                "Imported {} new game(s) ({} already saved). Latest: {}. Library: {}/",
                r.games.len() as u32 - r.duplicates,
                r.duplicates,
                r.games.first().map(|g| format!("{} vs {} (id {})", g.white, g.black, g.id)).unwrap_or_default(),
                c.base
            ))])),
            Err(e) => Ok(fail(e)),
        }
    }

    #[tool(
        name = "my_weaknesses",
        title = "My weaknesses",
        description = "Summarise the user's recurring mistakes across their analysed games on chessgpt: tactical motifs they missed or allowed (forks, pins, hanging pieces...), error rates by game phase, and their estimated playing strength. Use it to suggest what to study.",
        annotations(title = "My weaknesses", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn my_weaknesses(&self, rc: RequestContext<RoleServer>) -> Result<CallToolResult, ErrorData> {
        let c = self.ctx(&rc)?;
        let db = &c.state.db;
        let motifs = db.mistake_motif_counts().await.map_err(db_err)?;
        let phases = db.phase_counts().await.map_err(db_err)?;
        let games = db.progress_games().await.map_err(db_err)?;
        let moves: u32 = phases.iter().map(|(_, m, _)| m).sum();
        let text = if motifs.is_empty() && phases.is_empty() {
            "No analysed mistakes yet. Analyse a few of the user's games first (analyze_game), with their side set.".to_string()
        } else {
            let mut by_tag: std::collections::BTreeMap<&str, (i64, i64)> = Default::default();
            for (t, kind, n) in &motifs {
                let e = by_tag.entry(t.as_str()).or_default();
                if kind == "allowed" { e.1 += n } else { e.0 += n }
            }
            let mut tags: Vec<_> = by_tag.into_iter().collect();
            tags.sort_by_key(|(_, (m, a))| -(m + a));
            let t: Vec<String> = tags
                .iter()
                .take(8)
                .map(|(t, (m, a))| format!("{} (missed {m}, allowed {a})", coach::tags::label(t)))
                .collect();
            let p: Vec<String> = phases
                .iter()
                .map(|(ph, m, e)| format!("{}: {e} errors in {m} moves", ph.as_str()))
                .collect();
            let est: Vec<u32> = games.iter().filter_map(|g| g.estimate).collect();
            let strength = chess_core::rating::rolling(&est)
                .map(|(r, m)| format!("\nEstimated playing strength from move quality (last {} games): about {r} ± {m}.", est.len().min(chess_core::rating::ROLLING_GAMES)))
                .unwrap_or_default();
            format!(
                "Across {} analysed games ({moves} of the user's moves).\nBy motif: {}\nBy phase: {}{strength}",
                games.len(),
                if t.is_empty() { "none found".into() } else { t.join(", ") },
                p.join("; ")
            )
        };
        let themes: Vec<Value> = motifs.iter().map(|(t, k, n)| json!({"tag": t, "kind": k, "count": n})).collect();
        let phases_json: Vec<Value> =
            phases.iter().map(|(p, m, e)| json!({"phase": p.as_str(), "moves": m, "errors": e})).collect();
        Ok(result(text, json!({ "kind": "weaknesses", "themes": themes, "phases": phases_json, "url": format!("{}/progress", c.base) })))
    }
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn db_err(e: db::DbError) -> ErrorData {
    ErrorData::internal_error(e.to_string(), None)
}

async fn import(state: &AppState, req: ImportRequest) -> Result<ImportResponse, String> {
    let mut out = ImportResponse { games: vec![], duplicates: 0, errors: vec![] };
    use crate::routes::games::store;
    match req {
        ImportRequest::Pgn { pgn } => {
            for chunk in importers::split_pgns(&pgn).into_iter().take(1) {
                store(state, GameSource::Pgn, None, chunk, &mut out).await;
            }
        }
        ImportRequest::LichessGame { id } => {
            let g = state.importers.lichess_game(&id).await.map_err(|e| e.to_string())?;
            store(state, g.source, Some(g.source_id), g.pgn, &mut out).await;
        }
        ImportRequest::Lichess { username, max } => {
            let token = state.db.lichess_token().await.map_err(|e| e.to_string())?;
            let games = state
                .importers
                .lichess_user(&username, max.unwrap_or(10), token.as_deref())
                .await
                .map_err(|e| e.to_string())?;
            for g in games {
                store(state, g.source, Some(g.source_id), g.pgn, &mut out).await;
            }
        }
        ImportRequest::Chesscom { username, max } => {
            let games = state.importers.chesscom_user(&username, max.unwrap_or(10)).await.map_err(|e| e.to_string())?;
            for g in games {
                store(state, g.source, Some(g.source_id), g.pgn, &mut out).await;
            }
        }
        ImportRequest::Fen { .. } => return Err("use analyze_position for a single position".into()),
    }
    Ok(out)
}

fn class_word(c: Classification) -> &'static str {
    match c {
        Classification::Book => "book",
        Classification::Best => "best",
        Classification::Excellent => "excellent",
        Classification::Good => "good",
        Classification::Inaccuracy => "inaccuracy",
        Classification::Mistake => "mistake",
        Classification::Blunder => "blunder",
        Classification::MissedWin => "missed win",
    }
}

async fn game_report(c: &Ctx, game_id: &str, analysis: Option<api_types::GameAnalysis>) -> Result<CallToolResult, ErrorData> {
    let game = match c.state.db.game(game_id).await {
        Ok(g) => g,
        Err(db::DbError::NotFound) => return Ok(fail("No game with that id in the user's library.")),
        Err(e) => return Err(db_err(e)),
    };
    let analysis = match analysis {
        Some(a) => Some(a),
        None => c.state.db.latest_analysis(game_id).await.map_err(db_err)?,
    };
    let url = format!("{}/analyse/{}", c.base, game_id);
    let s = &game.summary;
    let Some(a) = analysis.filter(|a| !a.moves.is_empty()) else {
        return Ok(result(
            format!("{} vs {} is saved but not analysed yet. Call analyze_game, or open {url}", s.white, s.black),
            json!({"kind": "game", "game_id": game_id, "url": url}),
        ));
    };
    let moments: Vec<Value> = a
        .key_moments
        .iter()
        .filter_map(|ply| {
            let m = a.moves.iter().find(|m| m.ply == *ply)?;
            let pm = game.parsed.moves.get(*ply as usize - 1)?;
            let n = pm.fen_before.split(' ').nth(5).unwrap_or("1");
            let label = if m.mover == Side::White { format!("{n}. {}", m.san) } else { format!("{n}... {}", m.san) };
            Some(json!({
                "ply": m.ply, "move": label, "san": m.san, "uci": m.uci,
                "side": m.mover, "classification": class_word(m.classification),
                "fen_before": pm.fen_before, "fen_after": pm.fen_after,
                "win_chance_before": round1(m.win_before), "win_chance_after": round1(m.win_after),
                "eval_after": m.score,
                "best_move": m.best_san, "best_uci": m.best_uci,
                "best_line": m.best_line_san.iter().take(8).collect::<Vec<_>>(),
                "phase": m.phase,
                "url": format!("{url}?ply={}", m.ply)
            }))
        })
        .collect();
    let count = |side: Side, cl: &[Classification]| a.moves.iter().filter(|m| m.mover == side && cl.contains(&m.classification)).count();
    let errs = [Classification::Blunder, Classification::MissedWin];
    let mut text = format!(
        "{} ({}) vs {} ({}), {}. {}\nAccuracy: White {:.1}%, Black {:.1}%. White: {} blunders, {} mistakes. Black: {} blunders, {} mistakes.\nKey moments:\n",
        s.white,
        s.white_elo.map(|e| e.to_string()).unwrap_or_else(|| "?".into()),
        s.black,
        s.black_elo.map(|e| e.to_string()).unwrap_or_else(|| "?".into()),
        s.result,
        s.opening.as_deref().unwrap_or(""),
        a.white_accuracy.unwrap_or(0.0),
        a.black_accuracy.unwrap_or(0.0),
        count(Side::White, &errs),
        count(Side::White, &[Classification::Mistake]),
        count(Side::Black, &errs),
        count(Side::Black, &[Classification::Mistake]),
    );
    for m in &moments {
        text.push_str(&format!(
            "- {} ({}): {}. Engine preferred {} with {}.\n",
            m["move"].as_str().unwrap_or(""),
            m["side"].as_str().unwrap_or(""),
            m["classification"].as_str().unwrap_or(""),
            m["best_move"].as_str().unwrap_or("?"),
            m["best_line"].as_array().map(|l| l.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" ")).unwrap_or_default()
        ));
    }
    text.push_str(&format!("Full interactive review: {url}"));
    let first_fen = moments.first().and_then(|m| m["fen_before"].as_str()).unwrap_or(&game.parsed.start_fen).to_string();
    let data = json!({
        "kind": "game",
        "game_id": game_id,
        "white": s.white, "black": s.black, "result": s.result, "opening": s.opening,
        "user_side": a.user_side,
        "white_accuracy": a.white_accuracy.map(round1), "black_accuracy": a.black_accuracy.map(round1),
        "rating_used": a.elo,
        "key_moments": moments,
        "fen": first_fen,
        "url": url
    });
    Ok(result(text, data))
}

// ------------------------------------------------------------ handler

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ChessMcp {
    fn get_info(&self) -> ServerConfig {
        let mut info = Implementation::new("chessgpt", env!("CARGO_PKG_VERSION"));
        info.title = Some("chessgpt chess coach".into());
        info.website_url = self.state.config.public_url.clone();
        info.description = Some("Stockfish analysis and your chessgpt game library.".into());
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_resources().build())
            .with_server_info(info)
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let mut r = Resource::new(WIDGET_URI, "chess-board");
        r.title = Some("Chess board".into());
        r.mime_type = Some(widget::MIME.into());
        Ok(ListResourcesResult::with_all_items(vec![r]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        if request.uri != WIDGET_URI {
            return Err(ErrorData::invalid_params(format!("unknown resource {}", request.uri), None));
        }
        let base = context
            .extensions
            .get::<axum::http::request::Parts>()
            .map(|p| base_url(&self.state, &p.headers))
            .unwrap_or_default();
        let meta = json!({
            "ui": { "prefersBorder": true, "csp": { "connectDomains": [], "resourceDomains": [] } },
            "openai/widgetDescription": "An interactive chess board showing the engine's lines or the game's key moments.",
            "openai/widgetPrefersBorder": true,
            "openai/widgetCSP": { "connect_domains": [], "resource_domains": [] }
        });
        let contents = ResourceContents::TextResourceContents {
            uri: WIDGET_URI.into(),
            mime_type: Some(widget::MIME.into()),
            text: widget::html(&base),
            meta: Some(MetaObject(meta.as_object().cloned().unwrap_or_default())),
        };
        Ok(ReadResourceResponse::Complete(ReadResourceResult::new(vec![contents])))
    }
}

const INSTRUCTIONS: &str = "chessgpt gives you Stockfish and the user's chessgpt game library. \
Never guess chess evaluations or lines: call analyze_position before judging a position, and check_line before \
presenting any line longer than two moves. Only mention moves that appear in tool results or that check_line \
confirmed. Evaluations are from White's point of view; translate them for the user's side. \
To review a game, call analyze_game with the PGN or Lichess link, then explain the key moments at the user's \
rating and end with the chessgpt.com link from the result so they can replay it on the interactive board.";
