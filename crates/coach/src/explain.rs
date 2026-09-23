//! LLM explanations of key moments and the end-of-game review, with
//! verification, one corrective retry, and sentence stripping as last resort.

use api_types::{BetterMove, Explanation, GameReview, MoveEval, Verification, VerificationStatus};
use chess_core::position::parse_fen;
use llm::{ChatRequest, JsonSchema, Message, Part, Provider, StopReason, Usage, send_with_retry};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::analysis::MomentContext;
use crate::prompts::{self, GameContext};
use crate::tags::is_tag;
use crate::verify::{Findings, Grounding, norm, repair_better_move, strip_sentences};

#[derive(Debug, thiserror::Error)]
pub enum ExplainError {
    #[error(transparent)]
    Llm(#[from] llm::LlmError),
    #[error("the model declined to answer{}", .0.as_ref().map(|c| format!(" ({c})")).unwrap_or_default())]
    Refused(Option<String>),
    #[error("the model ran out of output tokens")]
    Truncated,
    #[error("the model did not return valid JSON: {0}")]
    BadJson(String),
}

#[derive(Debug, Clone, Deserialize)]
struct BetterDraft {
    san: String,
    #[serde(default)]
    line_san: Vec<String>,
    #[serde(default)]
    reason: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Draft {
    headline: String,
    why_it_matters: String,
    better_move: Option<BetterDraft>,
    #[serde(default)]
    concept_tags: Vec<String>,
    #[serde(default)]
    takeaway: String,
    #[serde(default)]
    mentioned_moves: Vec<String>,
}

pub struct Explained {
    pub explanation: Explanation,
    pub request: Value,
    pub response: Value,
    pub usage: Usage,
}

fn parse<T: for<'de> Deserialize<'de>>(text: &str) -> Result<T, String> {
    let v = llm::json::extract_object(text).ok_or_else(|| "no JSON object found".to_string())?;
    serde_json::from_value(v).map_err(|e| e.to_string())
}

fn add_usage(total: &mut Usage, u: &Usage) {
    total.input_tokens += u.input_tokens;
    total.output_tokens += u.output_tokens;
    total.cache_read_tokens += u.cache_read_tokens;
    total.cache_write_tokens += u.cache_write_tokens;
}

pub fn moment_grounding(ctx: &GameContext, m: &MoveEval, mc: &MomentContext) -> Grounding {
    let mut g = Grounding::new();
    if let Ok(p) = parse_fen(&mc.fen_before) {
        g.add_position(&p);
    }
    if let Ok(p) = parse_fen(&mc.fen_after) {
        g.add_position(&p);
    }
    for (_, l) in &mc.lines_before {
        g.add_line(l);
    }
    if let Some((_, l)) = &mc.refutation {
        g.add_line(l);
    }
    g.add_line(&m.best_line_san);
    g.add_line(std::slice::from_ref(&m.san));
    g.add_history(&ctx.moves_san);
    g
}

/// Verify a draft; returns the explanation fields and findings.
fn check_draft(
    ctx: &GameContext,
    m: &MoveEval,
    mc: &MomentContext,
    d: &Draft,
) -> (Findings, Option<BetterMove>) {
    let g = moment_grounding(ctx, m, mc);
    let mut f = Findings::default();
    let before = parse_fen(&mc.fen_before).expect("valid fen");
    let candidates: Vec<String> = mc.lines_before.iter().filter_map(|(_, l)| l.first().cloned()).collect();
    let better = d.better_move.as_ref().and_then(|b| {
        if norm(&b.san) == norm(&m.san) {
            return None;
        }
        let mut bm = BetterMove { san: b.san.clone(), line_san: b.line_san.clone(), reason: b.reason.clone() };
        repair_better_move(&g, &before, &candidates, &mut bm, prompts::max_line(ctx.elo), &mut f);
        Some(bm)
    });
    for text in [&d.headline, &d.why_it_matters, &d.takeaway] {
        f.scan_text(&g, text);
    }
    if let Some(b) = &better {
        f.scan_text(&g, &b.reason);
    }
    f.scan_moves(&g, &d.mentioned_moves);
    (f, better)
}

pub async fn explain_moment(
    provider: &dyn Provider,
    ctx: &GameContext,
    m: &MoveEval,
    mc: &MomentContext,
) -> Result<Explained, ExplainError> {
    let system = prompts::explain_system(ctx);
    let mut messages = vec![Message::user(prompts::explain_moment(ctx, m, mc))];
    let mut usage = Usage::default();
    let mut raw_texts = vec![];
    let schema = JsonSchema { name: "move_explanation".into(), schema: prompts::explanation_schema() };

    let mut corrected = false;
    for attempt in 0..3 {
        let mut req = ChatRequest::new(system.clone(), messages.clone());
        req.json_schema = Some(schema.clone());
        let c = send_with_retry(provider, &req, None).await?;
        add_usage(&mut usage, &c.usage);
        match &c.stop {
            StopReason::Refusal(cat) => return Err(ExplainError::Refused(cat.clone())),
            StopReason::MaxTokens => return Err(ExplainError::Truncated),
            _ => {}
        }
        let text = c.message.text();
        raw_texts.push(text.clone());
        let draft: Draft = match parse(&text) {
            Ok(d) => d,
            Err(e) if attempt < 2 => {
                messages.push(c.message.clone());
                messages.push(Message::user(format!(
                    "That reply was not a single JSON object matching the schema ({e}). Reply again with only the JSON object."
                )));
                continue;
            }
            Err(e) => return Err(ExplainError::BadJson(e)),
        };
        let (f, better) = check_draft(ctx, m, mc, &draft);
        if !f.hard.is_empty() && !corrected && attempt < 2 {
            corrected = true;
            messages.push(c.message.clone());
            messages.push(Message::user(format!(
                "These moves are illegal in the relevant positions or not in the engine data: {}. \
                 Rewrite the explanation using only moves from ENGINE LINES, REFUTATION, GAME MOVES or LEGAL MOVES, copied exactly as written there.",
                f.hard.join(", ")
            )));
            continue;
        }
        let explanation = finish(m, &draft, better, &f, provider, &c.model);
        return Ok(Explained {
            explanation,
            request: json!({"system": system, "messages": messages, "schema": schema.name}),
            response: json!({"texts": raw_texts, "model": c.model}),
            usage,
        });
    }
    unreachable!("loop returns within three attempts")
}

fn finish(
    m: &MoveEval,
    d: &Draft,
    better: Option<BetterMove>,
    f: &Findings,
    provider: &dyn Provider,
    model: &str,
) -> Explanation {
    let mut stripped = false;
    let mut clean = |s: &str| {
        let (out, changed) = strip_sentences(s, &f.hard);
        stripped |= changed;
        out
    };
    let headline = clean(&d.headline);
    let why = clean(&d.why_it_matters);
    let takeaway = clean(&d.takeaway);
    let better = better.and_then(|mut b| {
        if f.hard.iter().any(|h| norm(h) == norm(&b.san)) {
            return None;
        }
        b.reason = clean(&b.reason);
        Some(b)
    });
    let mentioned: Vec<String> = d
        .mentioned_moves
        .iter()
        .filter(|mv| !f.hard.iter().any(|h| norm(h) == norm(mv)))
        .cloned()
        .collect();
    let tags: Vec<String> = d.concept_tags.iter().filter(|t| is_tag(t)).take(3).cloned().collect();
    let verification = f.to_verification(stripped);
    Explanation {
            ply: m.ply,
            headline,
            why_it_matters: why,
            better_move: better,
            concept_tags: tags,
            takeaway,
            mentioned_moves: mentioned,
            verification,
        provider: provider.kind().to_string(),
        model: model.to_string(),
    }
}

#[derive(Debug, Deserialize)]
struct ReviewDraft {
    text: String,
    #[serde(default)]
    themes: Vec<String>,
}

pub async fn review_game(
    provider: &dyn Provider,
    ctx: &GameContext,
    moves: &[MoveEval],
    white_accuracy: Option<f64>,
    black_accuracy: Option<f64>,
    explanations: &[Explanation],
) -> Result<(GameReview, Usage), ExplainError> {
    let acc = |a: Option<f64>| a.map(|a| format!("{a:.1}%")).unwrap_or_else(|| "n/a".into());
    let count = |side: api_types::Side, c: api_types::Classification| {
        moves.iter().filter(|m| m.mover == side && m.classification == c).count()
    };
    use api_types::{Classification as C, Side as S};
    let mut notes = String::new();
    for e in explanations {
        let Some(m) = moves.iter().find(|m| m.ply == e.ply) else { continue };
        notes.push_str(&format!(
            "- ply {} {} by {:?}: {:?}. {} Takeaway: {} Tags: {}\n",
            m.ply,
            m.san,
            m.mover,
            m.classification,
            e.headline,
            e.takeaway,
            e.concept_tags.join(", ")
        ));
    }
    if notes.is_empty() {
        for m in moves.iter().filter(|m| m.is_key_moment) {
            notes.push_str(&format!("- ply {} {} by {:?}: {:?}\n", m.ply, m.san, m.mover, m.classification));
        }
    }
    let user = format!(
        "ACCURACY: White {} (blunders {}, mistakes {}, inaccuracies {}); Black {} (blunders {}, mistakes {}, inaccuracies {}).\n\nKEY MOMENTS\n{}\nWrite the review.",
        acc(white_accuracy),
        count(S::White, C::Blunder) + count(S::White, C::MissedWin),
        count(S::White, C::Mistake),
        count(S::White, C::Inaccuracy),
        acc(black_accuracy),
        count(S::Black, C::Blunder) + count(S::Black, C::MissedWin),
        count(S::Black, C::Mistake),
        count(S::Black, C::Inaccuracy),
        if notes.is_empty() { "(no key moments)\n".to_string() } else { notes }
    );
    let mut req = ChatRequest::new(prompts::review_system(ctx), vec![Message::user(user)]);
    req.json_schema = Some(JsonSchema { name: "game_review".into(), schema: prompts::review_schema() });
    let c = send_with_retry(provider, &req, None).await?;
    match &c.stop {
        StopReason::Refusal(cat) => return Err(ExplainError::Refused(cat.clone())),
        StopReason::MaxTokens => return Err(ExplainError::Truncated),
        _ => {}
    }
    let d: ReviewDraft = parse(&c.message.text()).map_err(ExplainError::BadJson)?;
    let mut g = Grounding::new();
    g.add_history(&ctx.moves_san);
    for e in explanations {
        g.add_line(&e.mentioned_moves);
        if let Some(b) = &e.better_move {
            g.add_line(&b.line_san);
        }
    }
    let mut f = Findings::default();
    f.scan_text(&g, &d.text);
    // Legal-but-unbacked moves are not acceptable in a review: no position context.
    let mut bad = f.hard.clone();
    bad.extend(f.unverified.clone());
    let (text, stripped) = strip_sentences(&d.text, &bad);
    let verification = if stripped {
        Verification {
            status: VerificationStatus::Partial,
            issues: vec![format!("removed sentences mentioning {}", bad.join(", "))],
            unverified_moves: vec![],
            rejected_moves: bad.clone(),
        }
    } else {
        Verification { status: VerificationStatus::Ok, issues: vec![], unverified_moves: vec![], rejected_moves: vec![] }
    };
    let themes = d.themes.into_iter().filter(|t| is_tag(t)).take(3).collect();
    Ok((GameReview { text, themes, verification }, c.usage))
}

/// Drop provider-internal parts from a message for logging.
pub fn visible(m: &Message) -> Message {
    Message {
        role: m.role,
        parts: m.parts.iter().filter(|p| !matches!(p, Part::Opaque { .. })).cloned().collect(),
    }
}
