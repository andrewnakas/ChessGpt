//! Prompt rendering. Templates live in `prompts/*.md` and are versioned; the
//! version string is stored with every explanation.

use api_types::{Classification, EloTier, Judgement, MoveEval, Score, Side};
use chess_core::motifs::{line_story, move_motifs, position_motifs};
use chess_core::pgn::ParsedGame;
use chess_core::position::{features, legal_sans, numbered_line, parse_fen, san_to_move};
use chess_core::winpct::win_percent;
use serde_json::{Value, json};
use shakmaty::{Chess, Position};

use crate::analysis::MomentContext;
use crate::bands::Band;
use crate::tags::tag_names;

pub const EXPLAIN_VERSION: &str = "explain.v2";
pub const REVIEW_VERSION: &str = "review.v2";

const EXPLAIN_SYSTEM: &str = include_str!("../prompts/explain_system.v2.md");
const EXPLAIN_MOMENT: &str = include_str!("../prompts/explain_moment.v2.md");
const REVIEW_SYSTEM: &str = include_str!("../prompts/review.v1.md");

/// Everything about the game that stays constant across its prompts.
#[derive(Debug, Clone)]
pub struct GameContext {
    pub white: String,
    pub black: String,
    pub white_elo: Option<u32>,
    pub black_elo: Option<u32>,
    pub result: String,
    pub opening: Option<String>,
    pub user_side: Option<Side>,
    pub elo: u32,
    pub tier: EloTier,
    pub start_fen: String,
    pub moves_san: Vec<String>,
    /// The student's most frequent mistake patterns across their games.
    pub recurring: Vec<String>,
}

impl GameContext {
    pub fn new(game: &ParsedGame, user_side: Option<Side>, elo: u32, opening: Option<String>) -> GameContext {
        GameContext {
            white: game.tag("White").unwrap_or("White").to_string(),
            black: game.tag("Black").unwrap_or("Black").to_string(),
            white_elo: game.elo(Side::White),
            black_elo: game.elo(Side::Black),
            result: game.tag("Result").unwrap_or("*").to_string(),
            opening: opening.or_else(|| game.tag("Opening").map(String::from)),
            user_side,
            elo,
            tier: EloTier::from_elo(elo),
            start_fen: game.start_fen.clone(),
            moves_san: game.moves.iter().map(|m| m.san.clone()).collect(),
            recurring: vec![],
        }
    }

    pub fn header(&self) -> String {
        let elo = |e: Option<u32>| e.map(|e| e.to_string()).unwrap_or_else(|| "unrated".into());
        format!(
            "White: {} ({}). Black: {} ({}). Result: {}. Opening: {}.",
            self.white,
            elo(self.white_elo),
            self.black,
            elo(self.black_elo),
            self.result,
            self.opening.as_deref().unwrap_or("unknown")
        )
    }

    pub fn numbered_moves(&self) -> String {
        match parse_fen(&self.start_fen) {
            Ok(p) if !self.moves_san.is_empty() => numbered_line(&p, &self.moves_san),
            _ => "(no moves)".into(),
        }
    }

    fn user_side_text(&self) -> &'static str {
        match self.user_side {
            Some(Side::White) => "White",
            Some(Side::Black) => "Black",
            None => "an unknown side (treat both sides as instructive)",
        }
    }
}

/// How to write for a player of this rating.
pub fn level_guidance(elo: u32) -> &'static str {
    Band::from_elo(elo).guidance()
}

/// Longest line (in plies) to show a player of this rating.
pub fn max_line(elo: u32) -> usize {
    Band::from_elo(elo).max_line()
}

pub fn fmt_score(s: Score) -> String {
    match s {
        Score::Cp(c) => format!("{:+.2}", c as f64 / 100.0),
        Score::Mate(m) if m > 0 => format!("White mates in {m}"),
        Score::Mate(m) => format!("Black mates in {}", -m),
    }
}

fn render(template: &str, vars: &[(&str, String)]) -> String {
    let mut s = template.to_string();
    for (k, v) in vars {
        s = s.replace(&format!("{{{{{k}}}}}"), v);
    }
    debug_assert!(!s.contains("{{"), "unfilled placeholder in prompt: {s}");
    s
}

pub fn explain_system(ctx: &GameContext) -> String {
    render(
        EXPLAIN_SYSTEM,
        &[
            ("elo", ctx.elo.to_string()),
            ("tier_label", Band::from_elo(ctx.elo).label().into()),
            ("level_guidance", level_guidance(ctx.elo).into()),
            ("length_hint", Band::from_elo(ctx.elo).length_hint().into()),
            ("max_line", max_line(ctx.elo).to_string()),
            ("user_side", ctx.user_side_text().into()),
            ("tags", tag_names().join(", ")),
            ("game_header", ctx.header()),
            ("game_moves", ctx.numbered_moves()),
            ("recurring", recurring_text(&ctx.recurring)),
        ],
    )
}

fn recurring_text(r: &[String]) -> String {
    if r.is_empty() {
        return String::new();
    }
    format!(
        "\nRECURRING PATTERNS in this student's analysed games: {}. When this moment is another example of one of them, say so in one short clause (\"this is the same loose-piece problem as before\"), without inventing details about other games.\n",
        r.join(", ")
    )
}

fn move_label(pos: &Chess, san: &str) -> String {
    let n = pos.fullmoves().get();
    match pos.turn() {
        shakmaty::Color::White => format!("{n}. {san}"),
        shakmaty::Color::Black => format!("{n}... {san}"),
    }
}

fn class_word(c: Classification) -> &'static str {
    match c {
        Classification::Book => "a book move",
        Classification::Best => "the engine's best move",
        Classification::Excellent => "excellent",
        Classification::Good => "good",
        Classification::Inaccuracy => "an inaccuracy",
        Classification::Mistake => "a mistake",
        Classification::Blunder => "a blunder",
        Classification::MissedWin => "a missed win",
    }
}

fn bullet(lines: &[String]) -> String {
    if lines.is_empty() {
        return "- (none)".into();
    }
    lines.iter().map(|l| format!("- {l}")).collect::<Vec<_>>().join("\n")
}

pub fn explain_moment(ctx: &GameContext, m: &MoveEval, mc: &MomentContext) -> String {
    let before = parse_fen(&mc.fen_before).expect("valid fen");
    let after = parse_fen(&mc.fen_after).expect("valid fen");
    let plies = max_line(ctx.elo) + 2;
    let engine_lines = if mc.lines_before.is_empty() {
        "(none)".to_string()
    } else {
        mc.lines_before
            .iter()
            .enumerate()
            .map(|(i, (s, l))| {
                let l: Vec<String> = l.iter().take(plies).cloned().collect();
                format!("{}. [{}] {}", i + 1, fmt_score(*s), numbered_line(&before, &l))
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let refutation = if after.is_checkmate() {
        "(none: the move gives checkmate)".to_string()
    } else if after.is_stalemate() {
        "(none: the move gives stalemate)".to_string()
    } else {
        match &mc.refutation {
            Some((s, l)) if !l.is_empty() => {
                let l: Vec<String> = l.iter().take(plies).cloned().collect();
                format!("[{}] {}", fmt_score(*s), numbered_line(&after, &l))
            }
            _ => "(none)".to_string(),
        }
    };
    let played_motifs = san_to_move(&before, &m.san)
        .map(|mv| move_motifs(&before, mv).into_iter().map(|h| h.text).collect::<Vec<_>>())
        .unwrap_or_default();
    let refutation_story = match &mc.refutation {
        Some((_, l)) if !after.is_game_over() => line_story(&after, l, plies),
        _ => vec![],
    };
    let best_story = mc.lines_before.first().map(|(_, l)| line_story(&before, l, plies)).unwrap_or_default();
    let themes_before: Vec<String> = position_motifs(&before).into_iter().map(|h| h.text).collect();
    let (white_before, white_after) = match m.mover {
        Side::White => (m.win_before, m.win_after),
        Side::Black => (100.0 - m.win_before, 100.0 - m.win_after),
    };
    let eval_before = mc.lines_before.first().map(|(s, _)| *s).unwrap_or_else(|| {
        // fall back to the batch eval implied by win% if the deep pass was empty
        Score::Cp(0)
    });
    let side = crate::analysis::side_name(m.mover);
    let student_marker = if ctx.user_side == Some(m.mover) { " (the student)" } else { "" };
    let judgement = match m.lichess_judgement {
        Some(j) if !matches!(m.classification, Classification::Inaccuracy | Classification::Mistake | Classification::Blunder) => {
            format!(
                " (Lichess would call it {})",
                match j {
                    Judgement::Inaccuracy => "an inaccuracy",
                    Judgement::Mistake => "a mistake",
                    Judgement::Blunder => "a blunder",
                }
            )
        }
        _ => String::new(),
    };
    render(
        EXPLAIN_MOMENT,
        &[
            ("move_label", move_label(&before, &m.san)),
            ("side", side.into()),
            ("student_marker", student_marker.into()),
            ("played_san", m.san.clone()),
            ("classification", class_word(m.classification).into()),
            ("judgement", judgement),
            ("fen_before", mc.fen_before.clone()),
            ("eval_before", fmt_score(eval_before)),
            ("eval_after", fmt_score(m.score)),
            ("white_win_before", format!("{white_before:.0}")),
            ("white_win_after", format!("{white_after:.0}")),
            ("change", format!("{:+.0}", m.win_after - m.win_before)),
            ("engine_lines", engine_lines),
            ("refutation", refutation),
            ("played_motifs", bullet(&played_motifs)),
            ("refutation_story", bullet(&refutation_story)),
            ("best_story", bullet(&best_story)),
            ("themes_before", bullet(&themes_before)),
            ("facts_before", bullet(&features(&before))),
            ("facts_after", bullet(&features(&after))),
            ("legal_moves", legal_sans(&before).join(", ")),
        ],
    )
}

pub fn explanation_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["headline", "why_it_matters", "better_move", "concept_tags", "takeaway", "mentioned_moves"],
        "properties": {
            "headline": {"type": "string", "description": "One short sentence naming what happened."},
            "why_it_matters": {"type": "string", "description": "The concrete reason, grounded in the engine lines."},
            "better_move": {
                "anyOf": [
                    {"type": "null"},
                    {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["san", "line_san", "reason"],
                        "properties": {
                            "san": {"type": "string", "description": "First move of an engine line, SAN."},
                            "line_san": {"type": "array", "items": {"type": "string"}, "description": "Prefix of that engine line, SAN."},
                            "reason": {"type": "string"}
                        }
                    }
                ]
            },
            "concept_tags": {"type": "array", "items": {"type": "string", "enum": tag_names()}},
            "takeaway": {"type": "string", "description": "One practical rule of thumb."},
            "mentioned_moves": {"type": "array", "items": {"type": "string"}, "description": "Every SAN move written above."}
        }
    })
}

pub fn review_system(ctx: &GameContext) -> String {
    render(
        REVIEW_SYSTEM,
        &[
            ("elo", ctx.elo.to_string()),
            ("tier_label", Band::from_elo(ctx.elo).label().into()),
            ("user_side", ctx.user_side_text().into()),
            ("level_guidance", level_guidance(ctx.elo).into()),
            ("tags", tag_names().join(", ")),
            ("game_header", ctx.header()),
            ("game_moves", ctx.numbered_moves()),
        ],
    )
}

pub fn review_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["text", "themes"],
        "properties": {
            "text": {"type": "string"},
            "themes": {"type": "array", "items": {"type": "string", "enum": tag_names()}}
        }
    })
}

/// White's win% for a White-POV score (for prompts and tests).
pub fn white_win(s: Score) -> f64 {
    win_percent(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recurring_patterns_only_when_known() {
        assert_eq!(recurring_text(&[]), "");
        let t = recurring_text(&["hanging piece (12 times)".into()]);
        assert!(t.contains("RECURRING PATTERNS") && t.contains("hanging piece (12 times)"), "{t}");
    }
}
