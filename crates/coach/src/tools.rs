//! Tools the coach can call during chat. Each tool validates its input,
//! runs deterministic code (engine, move generator, explorer), and records
//! what it returned in the turn's [`Grounding`] so the answer can be verified.

use api_types::{EloTier, ToolCallView};
use chess_core::position::{legal_sans, numbered_line, parse_fen, play_sans, pv_to_san, to_fen};
use engine::{AnalysisRequest, EnginePool, Limit, Priority};
use importers::{ExplorerDb, Importers};
use llm::ToolDef;
use serde_json::{Value, json};
use shakmaty::{Color, Position};

use crate::analysis::position_score;
use crate::prompts::fmt_score;
use crate::verify::Grounding;

/// A game the chat is attached to.
#[derive(Debug, Clone)]
pub struct GameToolContext {
    pub header: String,
    pub numbered_moves: String,
    pub moves_san: Vec<String>,
    /// One line per key moment, prepared by the server.
    pub key_moments: Vec<String>,
}

#[derive(Clone)]
pub struct ToolEnv {
    pub pool: EnginePool,
    pub tier: EloTier,
    /// Lichess explorer client and token, when the user enabled it.
    pub explorer: Option<(Importers, String)>,
    pub game: Option<GameToolContext>,
}

fn nullable(t: &str, desc: &str) -> Value {
    json!({"anyOf": [{"type": t}, {"type": "null"}], "description": desc})
}

pub fn tool_defs(env: &ToolEnv) -> Vec<ToolDef> {
    let fen = json!({"type": "string", "description": "Position in FEN."});
    let mut defs = vec![
        ToolDef {
            name: "analyse_position".into(),
            description: "Run Stockfish on a position. Returns the best lines with evaluations (White's point of view) in SAN. \
                          Call this before judging any position or move."
                .into(),
            input_schema: json!({
                "type": "object", "additionalProperties": false, "required": ["fen", "lines"],
                "properties": {"fen": fen, "lines": nullable("integer", "How many best lines (1-5, default 3).")}
            }),
        },
        ToolDef {
            name: "play_line".into(),
            description: "Play a sequence of SAN moves from a position, check that every move is legal, and evaluate the final \
                          position with Stockfish. Call this to verify any line longer than two moves before presenting it."
                .into(),
            input_schema: json!({
                "type": "object", "additionalProperties": false, "required": ["fen", "moves"],
                "properties": {"fen": fen, "moves": {"type": "array", "items": {"type": "string"}, "description": "SAN moves in order."}}
            }),
        },
        ToolDef {
            name: "legal_moves".into(),
            description: "List every legal move in a position in SAN, with the checks and captures called out.".into(),
            input_schema: json!({
                "type": "object", "additionalProperties": false, "required": ["fen"],
                "properties": {"fen": fen}
            }),
        },
    ];
    if env.explorer.is_some() {
        defs.push(ToolDef {
            name: "opening_lookup".into(),
            description: "Look up a position in the Lichess opening explorer: the opening name and the moves played most often, \
                          with results. Use for opening questions."
                .into(),
            input_schema: json!({
                "type": "object", "additionalProperties": false, "required": ["fen", "database"],
                "properties": {"fen": fen, "database": {"type": "string", "enum": ["masters", "lichess"], "description": "masters = over-the-board master games; lichess = online games near the player's rating."}}
            }),
        });
    }
    if env.game.is_some() {
        defs.push(ToolDef {
            name: "game_context".into(),
            description: "Get the student's current game: players, all moves, and the key moments found by the analysis. \
                          Use when the question refers to the game (\"why did I lose\", \"move 23\")."
                .into(),
            input_schema: json!({"type": "object", "additionalProperties": false, "required": [], "properties": {}}),
        });
    }
    defs
}

pub struct ToolOutcome {
    /// What the model reads.
    pub content: String,
    pub view: ToolCallView,
}

fn err(id: &str, name: &str, input: &Value, msg: String) -> ToolOutcome {
    ToolOutcome {
        content: format!("ERROR: {msg}"),
        view: ToolCallView {
            id: id.into(),
            name: name.into(),
            input: input.clone(),
            summary: msg,
            is_error: true,
            data: None,
        },
    }
}

fn side_to_move(c: Color) -> &'static str {
    match c {
        Color::White => "White",
        Color::Black => "Black",
    }
}

pub fn validate_input(defs: &[ToolDef], name: &str, input: &Value) -> Result<(), String> {
    if let Some(raw) = llm::invalid_json_input(input) {
        return Err(format!("{{\"INVALID_JSON\": {}}}", Value::String(raw.to_string())));
    }
    let def = defs.iter().find(|d| d.name == name).ok_or_else(|| format!("unknown tool {name}"))?;
    let validator = jsonschema::validator_for(&def.input_schema).map_err(|e| e.to_string())?;
    let errors: Vec<String> = validator.iter_errors(input).map(|e| e.to_string()).collect();
    if errors.is_empty() { Ok(()) } else { Err(format!("invalid input: {}", errors.join("; "))) }
}

pub async fn run_tool(env: &ToolEnv, g: &mut Grounding, id: &str, name: &str, input: &Value) -> ToolOutcome {
    let defs = tool_defs(env);
    if let Err(e) = validate_input(&defs, name, input) {
        return err(id, name, input, e);
    }
    match name {
        "analyse_position" => analyse(env, g, id, input).await,
        "play_line" => play_line(env, g, id, input).await,
        "legal_moves" => legal(g, id, input),
        "opening_lookup" => opening(env, g, id, input).await,
        "game_context" => game_context(env, g, id, input),
        _ => err(id, name, input, format!("unknown tool {name}")),
    }
}

async fn analyse(env: &ToolEnv, g: &mut Grounding, id: &str, input: &Value) -> ToolOutcome {
    let name = "analyse_position";
    let fen = input["fen"].as_str().unwrap_or_default();
    let pos = match parse_fen(fen) {
        Ok(p) => p,
        Err(e) => return err(id, name, input, e.to_string()),
    };
    let multipv = input["lines"].as_u64().unwrap_or(3).clamp(1, 5) as u32;
    let a = match env
        .pool
        .analyse(AnalysisRequest {
            fen: to_fen(&pos),
            multipv,
            limit: env.tier.interactive(),
            priority: Priority::Interactive,
        })
        .await
    {
        Ok(a) => a,
        Err(e) => return err(id, name, input, e.to_string()),
    };
    g.add_position(&pos);
    if let Some(t) = a.terminal {
        let s = position_score(&a, pos.turn());
        return ToolOutcome {
            content: format!("The position is already over: {t:?} ({}).", fmt_score(s)),
            view: ToolCallView {
                id: id.into(),
                name: name.into(),
                input: input.clone(),
                summary: format!("{t:?}"),
                is_error: false,
                data: Some(json!({"fen": a.fen, "lines": []})),
            },
        };
    }
    let mut text = format!("Position: {} ({} to move). Stockfish depth {}.\n", a.fen, side_to_move(pos.turn()), a.depth);
    let mut lines = vec![];
    for l in &a.lines {
        let san = pv_to_san(&pos, &l.pv);
        let shown: Vec<String> = san.iter().take(12).cloned().collect();
        g.add_line(&shown);
        text.push_str(&format!("{}. [{}] {}\n", l.rank, fmt_score(l.score), numbered_line(&pos, &shown)));
        lines.push(json!({"score": l.score, "san": shown, "uci": l.pv.iter().take(12).collect::<Vec<_>>(), "wdl": l.wdl}));
    }
    let summary = a
        .lines
        .first()
        .map(|l| {
            let first = pv_to_san(&pos, &l.pv[..1.min(l.pv.len())]);
            format!("{} best, {} (depth {})", first.first().cloned().unwrap_or_default(), fmt_score(l.score), a.depth)
        })
        .unwrap_or_else(|| "no lines".into());
    ToolOutcome {
        content: text,
        view: ToolCallView {
            id: id.into(),
            name: name.into(),
            input: input.clone(),
            summary,
            is_error: false,
            data: Some(json!({"fen": a.fen, "depth": a.depth, "lines": lines})),
        },
    }
}

async fn play_line(env: &ToolEnv, g: &mut Grounding, id: &str, input: &Value) -> ToolOutcome {
    let name = "play_line";
    let fen = input["fen"].as_str().unwrap_or_default();
    let pos = match parse_fen(fen) {
        Ok(p) => p,
        Err(e) => return err(id, name, input, e.to_string()),
    };
    let moves: Vec<String> = input["moves"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    g.add_position(&pos);
    let (end, line) = match play_sans(&pos, &moves) {
        Ok(r) => r,
        Err(chess_core::position::ChessError::IllegalMove { index, mv }) => {
            let (reached, ok) = play_sans(&pos, &moves[..index]).expect("prefix is legal");
            let legal = legal_sans(&reached);
            g.add_position(&reached);
            g.add_line(&ok.san);
            return err(
                id,
                name,
                input,
                format!(
                    "move {} ({mv}) is illegal after {}. Legal moves there: {}",
                    index + 1,
                    if ok.san.is_empty() { "the start position".to_string() } else { numbered_line(&pos, &ok.san) },
                    legal.join(", ")
                ),
            );
        }
        Err(e) => return err(id, name, input, e.to_string()),
    };
    g.add_line(&line.san);
    g.add_position(&end);
    let a = env
        .pool
        .analyse(AnalysisRequest {
            fen: line.final_fen.clone(),
            multipv: 1,
            limit: Limit::depth_or_time(20, 2500),
            priority: Priority::Interactive,
        })
        .await;
    let mut text = format!("Line is legal: {}\nFinal position: {}\n", numbered_line(&pos, &line.san), line.final_fen);
    let mut eval = None;
    let mut cont = vec![];
    match a {
        Ok(a) => {
            let s = position_score(&a, end.turn());
            eval = Some(s);
            if let Some(t) = a.terminal {
                text.push_str(&format!("The line ends the game: {t:?}.\n"));
            } else if let Some(l) = a.lines.first() {
                cont = pv_to_san(&end, &l.pv).into_iter().take(8).collect::<Vec<_>>();
                g.add_line(&cont);
                text.push_str(&format!(
                    "Evaluation: {} (depth {}). Best continuation: {}\n",
                    fmt_score(s),
                    a.depth,
                    numbered_line(&end, &cont)
                ));
            }
        }
        Err(e) => text.push_str(&format!("(engine unavailable: {e})\n")),
    }
    ToolOutcome {
        content: text,
        view: ToolCallView {
            id: id.into(),
            name: name.into(),
            input: input.clone(),
            summary: format!(
                "{} legal{}",
                line.san.join(" "),
                eval.map(|s| format!(", {}", fmt_score(s))).unwrap_or_default()
            ),
            is_error: false,
            data: Some(json!({"fen": fen, "san": line.san, "uci": line.uci, "final_fen": line.final_fen, "score": eval, "continuation": cont})),
        },
    }
}

fn legal(g: &mut Grounding, id: &str, input: &Value) -> ToolOutcome {
    let name = "legal_moves";
    let fen = input["fen"].as_str().unwrap_or_default();
    let pos = match parse_fen(fen) {
        Ok(p) => p,
        Err(e) => return err(id, name, input, e.to_string()),
    };
    g.add_position(&pos);
    let all = legal_sans(&pos);
    let checks: Vec<&String> = all.iter().filter(|s| s.ends_with('+') || s.ends_with('#')).collect();
    let captures: Vec<&String> = all.iter().filter(|s| s.contains('x')).collect();
    let text = format!(
        "{} to move, {} legal moves: {}\nChecks: {}\nCaptures: {}",
        side_to_move(pos.turn()),
        all.len(),
        all.join(", "),
        if checks.is_empty() { "none".into() } else { checks.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ") },
        if captures.is_empty() { "none".into() } else { captures.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ") },
    );
    ToolOutcome {
        content: text,
        view: ToolCallView {
            id: id.into(),
            name: name.into(),
            input: input.clone(),
            summary: format!("{} legal moves", all.len()),
            is_error: false,
            data: None,
        },
    }
}

async fn opening(env: &ToolEnv, g: &mut Grounding, id: &str, input: &Value) -> ToolOutcome {
    let name = "opening_lookup";
    let Some((client, token)) = &env.explorer else {
        return err(id, name, input, "the opening explorer is not enabled".into());
    };
    let fen = input["fen"].as_str().unwrap_or_default();
    let pos = match parse_fen(fen) {
        Ok(p) => p,
        Err(e) => return err(id, name, input, e.to_string()),
    };
    let db = if input["database"] == "lichess" { ExplorerDb::Lichess } else { ExplorerDb::Masters };
    let ex = match client.explorer(&to_fen(&pos), db, None, token).await {
        Ok(e) => e,
        Err(e) => return err(id, name, input, e.to_string()),
    };
    g.add_position(&pos);
    let total = |w: u64, d: u64, b: u64| (w + d + b).max(1) as f64;
    let mut text = format!(
        "{} database. Opening: {}. Games: {}.\n",
        if db == ExplorerDb::Masters { "Masters" } else { "Lichess" },
        ex.opening.as_ref().map(|o| format!("{} {}", o.eco, o.name)).unwrap_or_else(|| "unnamed".into()),
        ex.white + ex.draws + ex.black
    );
    for m in ex.moves.iter().take(8) {
        let n = total(m.white, m.draws, m.black);
        g.add_line(std::slice::from_ref(&m.san));
        text.push_str(&format!(
            "- {}: {} games, White wins {:.0}%, draws {:.0}%, Black wins {:.0}%\n",
            m.san,
            m.white + m.draws + m.black,
            100.0 * m.white as f64 / n,
            100.0 * m.draws as f64 / n,
            100.0 * m.black as f64 / n
        ));
    }
    ToolOutcome {
        content: text,
        view: ToolCallView {
            id: id.into(),
            name: name.into(),
            input: input.clone(),
            summary: ex.opening.map(|o| o.name).unwrap_or_else(|| format!("{} moves", ex.moves.len())),
            is_error: false,
            data: Some(json!({"moves": ex.moves.iter().take(8).map(|m| json!({"san": m.san, "uci": m.uci, "games": m.white + m.draws + m.black})).collect::<Vec<_>>()})),
        },
    }
}

fn game_context(env: &ToolEnv, g: &mut Grounding, id: &str, input: &Value) -> ToolOutcome {
    let name = "game_context";
    let Some(gc) = &env.game else {
        return err(id, name, input, "no game is attached to this conversation".into());
    };
    g.add_history(&gc.moves_san);
    let text = format!(
        "{}\nMoves: {}\nKey moments:\n{}",
        gc.header,
        gc.numbered_moves,
        if gc.key_moments.is_empty() { "(analysis not run yet)".to_string() } else { gc.key_moments.join("\n") }
    );
    ToolOutcome {
        content: text,
        view: ToolCallView {
            id: id.into(),
            name: name.into(),
            input: input.clone(),
            summary: format!("{} key moments", gc.key_moments.len()),
            is_error: false,
            data: None,
        },
    }
}
