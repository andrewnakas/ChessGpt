//! Technique drills: one idea (a concept tag such as "fork") trained several
//! ways. Recognise it, find it in fresh positions (the player's own position
//! transformed, and rated Lichess puzzles), then stop it when it's aimed at
//! you. Every item is checked by the engine or the motif detectors, so the
//! set is correct without an LLM.

use std::collections::HashSet;

use api_types::{DrillItem, DrillKind, DrillQuiz, PlayoutGoal};
use chess_core::motifs::{Motif, move_motifs, solver_motifs};
use chess_core::position::{hanging_pieces, parse_fen, pv_to_san, role_name, uci_to_move, uci_to_san};
use chess_core::variants::all_variants;
use chess_core::winpct::win_percent_for;
use engine::{AnalysisRequest, EngineError, EnginePool, Limit, Priority};
use shakmaty::{Chess, Position};

use crate::bank::{Bank, BankPuzzle, PUZZLE_RATING_OFFSET, TECHNIQUES, confirms, mix, motif_for, themes_for};
use crate::puzzles::MIN_GAP;
use crate::tags::label;

/// Search depth for checking generated positions.
pub const DEPTH: u32 = 16;
/// A defence holds when it keeps the mover within this many win% of the best move.
pub const DEFEND_MARGIN: f64 = 6.0;

/// The player's own position the set starts from.
#[derive(Debug, Clone)]
pub struct Seed {
    pub fen: String,
    /// Missed: the move they should have found. Allowed: the move they played,
    /// which walked into the technique.
    pub solution_uci: Vec<String>,
    pub allowed: bool,
}

#[derive(Debug, Clone)]
pub struct DrillRequest {
    pub technique: String,
    /// The player's game rating.
    pub rating: u32,
    pub seed: Option<Seed>,
    /// Bank puzzles already drilled recently.
    pub exclude: HashSet<String>,
    /// Varies the picks between sets.
    pub rng: u64,
}

/// An item and, for bank items, the Lichess puzzle it came from.
#[derive(Debug, Clone)]
pub struct ItemDraft {
    pub item: DrillItem,
    pub bank_id: Option<String>,
}

/// Can this technique be drilled?
pub fn drillable(tag: &str) -> bool {
    themes_for(tag).is_some()
}

fn blank(kind: DrillKind, origin: &str, fen: String, prompt: String) -> DrillItem {
    DrillItem {
        id: String::new(),
        kind,
        origin: origin.into(),
        fen,
        prompt,
        solution_uci: vec![],
        accept_uci: vec![],
        trap_san: vec![],
        line_san: vec![],
        quiz: None,
        goal: None,
        hints: vec![],
        rating: None,
        solved: None,
    }
}

fn side_name(pos: &Chess) -> &'static str {
    if pos.turn() == shakmaty::Color::White { "White" } else { "Black" }
}

/// Do the solver's moves along `line` use the technique? Techniques the
/// detectors don't know pass.
fn uses(technique: &str, pos: &Chess, line: &[String]) -> bool {
    let Some(themes) = themes_for(technique) else { return false };
    let found = solver_motifs(pos, line);
    themes.iter().any(|t| confirms(t, &found))
}

/// The detectors' one-line statement of the technique in the first move, if any.
fn motif_text(pos: &Chess, uci: &str, technique: &str) -> Option<String> {
    let want = motif_for(technique)?;
    let m = uci_to_move(pos, uci)?;
    move_motifs(pos, m)
        .into_iter()
        .find(|h| h.motif == want || (want == Motif::MatingAttack && h.motif == Motif::BackRank))
        .map(|h| {
            let mut c = h.text.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        })
}

/// Graded hints for finding `uci`: the idea, the piece, the move.
fn find_hints(pos: &Chess, uci: &str, technique: &str) -> Vec<String> {
    let mut h = vec![format!("Look for a {}.", label(technique).to_lowercase())];
    if let Some(m) = uci_to_move(pos, uci) {
        if let Some(from) = m.from() {
            h.push(format!("Move your {} on {from}.", role_name(m.role())));
        }
    }
    if let Some(san) = uci_to_san(pos, uci) {
        h.push(format!("Play {san}."));
    }
    h
}

/// Best line and its win% lead over the second best, for the side to move.
async fn best_two(pool: &EnginePool, fen: &str, depth: u32) -> Result<Option<(Vec<String>, f64)>, EngineError> {
    let Ok(pos) = parse_fen(fen) else { return Ok(None) };
    if pos.legal_moves().len() < 2 {
        return Ok(None);
    }
    let a = pool
        .analyse(AnalysisRequest { fen: fen.into(), multipv: 2, limit: Limit::depth(depth), priority: Priority::Interactive })
        .await?;
    let (Some(best), Some(second)) = (a.lines.first(), a.lines.get(1)) else { return Ok(None) };
    if best.pv.is_empty() {
        return Ok(None);
    }
    let gap = win_percent_for(best.score, pos.turn()) - win_percent_for(second.score, pos.turn());
    Ok(Some((best.pv.clone(), gap)))
}

/// Up to `n` transformed copies of the player's own position that still have
/// one clear best move, the same one, using the technique.
pub async fn variant_items(
    pool: &EnginePool,
    seed: &Seed,
    technique: &str,
    n: usize,
) -> Result<Vec<DrillItem>, EngineError> {
    let mut out = vec![];
    for v in all_variants(&seed.fen, &seed.solution_uci) {
        if out.len() >= n {
            break;
        }
        let Some((pv, gap)) = best_two(pool, &v.fen, DEPTH).await? else { continue };
        let pos = parse_fen(&v.fen).expect("variants are legal");
        if pv[0] != v.solution[0] || gap < MIN_GAP || !uses(technique, &pos, &pv[..pv.len().min(5)]) {
            continue;
        }
        let mut item = blank(
            DrillKind::Find,
            "variant",
            v.fen.clone(),
            format!("Your own position, redrawn. {} to move: find the {}.", side_name(&pos), label(technique).to_lowercase()),
        );
        item.hints = find_hints(&pos, &pv[0], technique);
        item.solution_uci = vec![pv[0].clone()];
        item.line_san = pv_to_san(&pos, &pv).into_iter().take(6).collect();
        out.push(item);
    }
    Ok(out)
}

/// A bank puzzle as a find-the-moves item.
pub fn find_item(p: &BankPuzzle, technique: &str) -> Option<DrillItem> {
    let (fen, solution) = p.solving()?;
    let pos = parse_fen(&fen).ok()?;
    let mut item = blank(
        DrillKind::Find,
        "bank",
        fen,
        format!("{} to move: find the {}.", side_name(&pos), label(technique).to_lowercase()),
    );
    item.hints = find_hints(&pos, solution.first()?, technique);
    item.line_san = pv_to_san(&pos, &solution);
    item.solution_uci = solution;
    item.rating = Some(p.rating.max(0) as u32);
    Some(item)
}

/// A position where `trap` walks into the technique: play a move that holds.
/// Every move within `DEFEND_MARGIN` of the best is accepted, and the trap must
/// not be. `trap_line` is the trap and its punishment (UCI); without one the
/// engine finds the punishment, which must use the technique.
pub async fn defend_at(
    pool: &EnginePool,
    fen: &str,
    trap: &str,
    trap_line: Option<&[String]>,
    technique: &str,
    origin: &str,
) -> Result<Option<DrillItem>, EngineError> {
    let Ok(pos) = parse_fen(fen) else { return Ok(None) };
    let Some(trap_move) = uci_to_move(&pos, trap) else { return Ok(None) };
    let a = pool
        .analyse(AnalysisRequest { fen: fen.into(), multipv: 6, limit: Limit::depth(DEPTH - 2), priority: Priority::Interactive })
        .await?;
    let Some(best) = a.lines.first() else { return Ok(None) };
    let best_wp = win_percent_for(best.score, pos.turn());
    let accept: Vec<String> = a
        .lines
        .iter()
        .filter(|l| win_percent_for(l.score, pos.turn()) >= best_wp - DEFEND_MARGIN)
        .filter_map(|l| l.pv.first().cloned())
        .collect();
    if accept.is_empty() || accept.iter().any(|u| u == trap) || accept.len() >= pos.legal_moves().len() {
        return Ok(None);
    }
    let line: Vec<String> = match trap_line {
        Some(l) => l[..l.len().min(4)].to_vec(),
        None => {
            let mut after = pos.clone();
            after.play_unchecked(trap_move);
            let fen_after = chess_core::position::to_fen(&after);
            let r = pool
                .analyse(AnalysisRequest { fen: fen_after, multipv: 1, limit: Limit::depth(DEPTH - 2), priority: Priority::Interactive })
                .await?;
            let Some(pv) = r.lines.first().map(|l| l.pv.clone()) else { return Ok(None) };
            // The punishment must be the technique, or the lesson is something else.
            if !uses(technique, &after, &pv[..pv.len().min(3)]) {
                return Ok(None);
            }
            std::iter::once(trap.to_string()).chain(pv.into_iter().take(3)).collect()
        }
    };
    let trap_san = pv_to_san(&pos, &line);
    let lead = if origin == "variant" { "Your own position, redrawn. " } else { "" };
    let mut item = blank(
        DrillKind::Defend,
        origin,
        fen.into(),
        format!(
            "{lead}{} to move. A natural-looking move here runs into a {}. Play a move that keeps you safe.",
            side_name(&pos),
            label(technique).to_lowercase()
        ),
    );
    item.hints = vec!["Before you move, ask what your opponent could do next.".into()];
    if trap_san.len() >= 2 {
        item.hints.push(format!("{} would run into {}.", trap_san[0], trap_san[1]));
    }
    item.line_san = pv_to_san(&pos, &best.pv).into_iter().take(6).collect();
    item.accept_uci = accept;
    item.trap_san = trap_san;
    Ok(Some(item))
}

/// The position before the move that walked into a bank puzzle.
pub async fn defend_item(pool: &EnginePool, p: &BankPuzzle, technique: &str) -> Result<Option<DrillItem>, EngineError> {
    let Some(trap) = p.moves.first() else { return Ok(None) };
    let mut item = defend_at(pool, &p.fen, trap, Some(&p.moves), technique, "bank").await?;
    if let Some(i) = item.as_mut() {
        i.rating = Some(p.rating.max(0) as u32);
    }
    Ok(item)
}

/// Up to `n` transformed copies of the position where the player walked into
/// the technique, as defence items; their own move is the trap.
pub async fn own_defend_items(pool: &EnginePool, seed: &Seed, technique: &str, n: usize) -> Result<Vec<DrillItem>, EngineError> {
    let mut out = vec![];
    for v in all_variants(&seed.fen, &seed.solution_uci) {
        if out.len() >= n {
            break;
        }
        if let Some(item) = defend_at(pool, &v.fen, &v.solution[0], None, technique, "variant").await? {
            out.push(item);
        }
    }
    Ok(out)
}

/// "The best move is shown: what idea does it use?"
pub fn idea_quiz(p: &BankPuzzle, technique: &str, rng: u64) -> Option<DrillItem> {
    let (fen, solution) = p.solving()?;
    let pos = parse_fen(&fen).ok()?;
    let first = solution.first()?;
    let found = solver_motifs(&pos, &solution);
    // Distractors: other techniques the solution doesn't use.
    let mut others: Vec<&str> = TECHNIQUES
        .iter()
        .map(|(t, _)| *t)
        .filter(|t| *t != technique && !matches!(*t, "endgame_technique" | "king_safety"))
        .filter(|t| motif_for(t).is_some_and(|m| !found.contains(&m)))
        .collect();
    others.sort_by_key(|t| mix(rng ^ t.len() as u64 ^ t.as_bytes()[0] as u64));
    let mut choices: Vec<String> = others.into_iter().take(3).map(label).collect();
    let at = (mix(rng) % (choices.len() as u64 + 1)) as usize;
    choices.insert(at, label(technique));
    let san = uci_to_san(&pos, first)?;
    let explanation = motif_text(&pos, first, technique).unwrap_or_else(|| format!("{san}: {}.", label(technique).to_lowercase()));
    let mut item = blank(DrillKind::Spot, "bank", fen, format!("{} to move. The best move is shown.", side_name(&pos)));
    item.quiz = Some(DrillQuiz {
        question: "What idea does it use?".into(),
        choices,
        answer_choice: Some(at as u32),
        answer_squares: vec![],
        arrow_uci: Some(first.clone()),
        explanation,
    });
    item.rating = Some(p.rating.max(0) as u32);
    Some(item)
}

/// "Tap the piece that starts it" (or, for loose pieces, "tap a piece you can win").
pub fn piece_quiz(p: &BankPuzzle, technique: &str) -> Option<DrillItem> {
    let (fen, solution) = p.solving()?;
    let pos = parse_fen(&fen).ok()?;
    let first = solution.first()?;
    let m = uci_to_move(&pos, first)?;
    let san = uci_to_san(&pos, first)?;
    let (question, answers) = if technique == "hanging_piece" {
        let loot: Vec<String> = hanging_pieces(&pos, !pos.turn()).into_iter().map(|(sq, _)| sq.to_string()).collect();
        ("Tap an enemy piece you can win.".to_string(), loot)
    } else {
        (
            format!("Tap the piece that starts the {}.", label(technique).to_lowercase()),
            vec![m.from()?.to_string()],
        )
    };
    if answers.is_empty() {
        return None;
    }
    let explanation = motif_text(&pos, first, technique).unwrap_or_else(|| format!("The move is {san}."));
    let mut item = blank(DrillKind::Spot, "bank", fen, format!("{} to move.", side_name(&pos)));
    item.quiz = Some(DrillQuiz {
        question,
        choices: vec![],
        answer_choice: None,
        answer_squares: answers,
        arrow_uci: None,
        explanation,
    });
    item.rating = Some(p.rating.max(0) as u32);
    Some(item)
}

/// Own moves in a play-out.
pub const PLAYOUT_MOVES: u32 = 8;

/// Where a bank puzzle ends up once solved, with the opponent's best reply
/// played: the solver is winning and to move. Convert it against the bot.
pub async fn playout_item(pool: &EnginePool, p: &BankPuzzle, technique: &str) -> Result<Option<DrillItem>, EngineError> {
    let Some((fen, solution)) = p.solving() else { return Ok(None) };
    let Ok(mut pos) = parse_fen(&fen) else { return Ok(None) };
    let solver = pos.turn();
    for u in &solution {
        let Some(m) = uci_to_move(&pos, u) else { return Ok(None) };
        pos.play_unchecked(m);
    }
    if pos.is_game_over() || pos.turn() == solver {
        return Ok(None);
    }
    let fen_after = chess_core::position::to_fen(&pos);
    let a = pool
        .analyse(AnalysisRequest { fen: fen_after, multipv: 1, limit: Limit::depth(DEPTH - 2), priority: Priority::Interactive })
        .await?;
    let Some(best) = a.lines.first() else { return Ok(None) };
    let Some(reply) = best.pv.first().and_then(|u| uci_to_move(&pos, u)) else { return Ok(None) };
    let start = win_percent_for(best.score, solver);
    pos.play_unchecked(reply);
    // Already over, or not clearly winning: nothing to convert.
    if pos.is_game_over() || start < 75.0 || pos.legal_moves().len() < 2 {
        return Ok(None);
    }
    let min = (start - 20.0).max(60.0).round();
    let mut item = blank(
        DrillKind::Playout,
        "bank",
        chess_core::position::to_fen(&pos),
        format!(
            "The {} has worked: {} is winning. Convert it against the bot. Play {PLAYOUT_MOVES} moves and keep your winning chances above {min}%.",
            label(technique).to_lowercase(),
            side_name(&pos)
        ),
    );
    item.goal = Some(PlayoutGoal { moves: PLAYOUT_MOVES, min_win_pct: min, start_win_pct: start.round() });
    item.hints = vec![
        "Trade pieces when you're ahead: every trade makes the extra material count more.".into(),
        "Check every move for your opponent's checks, captures and threats first.".into(),
    ];
    item.rating = Some(p.rating.max(0) as u32);
    Ok(Some(item))
}

/// Most items a set grows to with easier follow-ups.
pub const MAX_ITEMS: usize = 12;

/// After a missed find: an easier bank position on the same idea, `step`
/// rating points below the one missed.
pub fn easier_find(bank: &Bank, technique: &str, missed_rating: i32, exclude: &HashSet<String>, rng: u64) -> Option<ItemDraft> {
    let themes = themes_for(technique)?;
    let p = bank.pick(themes, missed_rating - 250, exclude, rng)?;
    let mut item = find_item(p, technique)?;
    item.prompt = format!("One more, a little easier. {}", item.prompt);
    Some(ItemDraft { item, bank_id: Some(p.id.clone()) })
}

/// Build a drill set: 2 recognition questions, 4 positions to find it in, 2
/// to defend against it, and 1 to play out. Up to 2 of those come from the player's own position:
/// finds when they missed the idea, defences when they walked into it.
pub async fn compose(pool: &EnginePool, bank: &Bank, req: &DrillRequest) -> Result<Vec<ItemDraft>, EngineError> {
    let Some(themes) = themes_for(&req.technique) else { return Ok(vec![]) };
    let t = req.technique.as_str();
    let r = req.rating as i32 + PUZZLE_RATING_OFFSET;
    let mut used = req.exclude.clone();
    let mut rng = req.rng;
    let mut next = |used: &HashSet<String>, rating: i32| -> Option<BankPuzzle> {
        rng = mix(rng);
        bank.pick(themes, rating, used, rng).cloned()
    };
    let mut out: Vec<ItemDraft> = vec![];
    let mut push = |item: DrillItem, p: Option<&BankPuzzle>, used: &mut HashSet<String>| {
        if let Some(p) = p {
            used.insert(p.id.clone());
        }
        out.push(ItemDraft { item, bank_id: p.map(|p| p.id.clone()) });
    };

    // Recognise it: easier positions, no move to find.
    for (i, rating) in [r - 250, r - 150].into_iter().enumerate() {
        for _ in 0..4 {
            let Some(p) = next(&used, rating) else { break };
            let q = if i == 0 { piece_quiz(&p, t) } else { idea_quiz(&p, t, req.rng) };
            used.insert(p.id.clone());
            if let Some(q) = q {
                push(q, Some(&p), &mut used);
                break;
            }
        }
    }

    // Find it: the player's own position redrawn, then fresh positions on a ladder.
    let own = match &req.seed {
        Some(seed) if !seed.allowed => variant_items(pool, seed, t, 2).await?,
        _ => vec![],
    };
    let ladder: &[i32] = match own.len() {
        0 => &[-100, 0, 100, 200],
        1 => &[-50, 50, 150],
        _ => &[0, 150],
    };
    let mut finds: Vec<(DrillItem, Option<BankPuzzle>)> = own.into_iter().map(|i| (i, None)).collect();
    for d in ladder {
        if let Some(p) = next(&used, r + d) {
            used.insert(p.id.clone());
            if let Some(item) = find_item(&p, t) {
                finds.push((item, Some(p)));
            }
        }
    }
    // Own positions first (familiar), then bank puzzles easiest first.
    for (item, p) in finds {
        push(item, p.as_ref(), &mut used);
    }

    // Stop it: the player's own position redrawn (when they walked into it),
    // then positions where other players did.
    let mut defended = 0;
    if let Some(seed) = req.seed.as_ref().filter(|s| s.allowed) {
        for item in own_defend_items(pool, seed, t, 2).await? {
            push(item, None, &mut used);
            defended += 1;
        }
    }
    for _ in 0..6 {
        if defended >= 2 {
            break;
        }
        let Some(p) = next(&used, r - 50) else { break };
        used.insert(p.id.clone());
        if let Some(item) = defend_item(pool, &p, t).await? {
            push(item, Some(&p), &mut used);
            defended += 1;
        }
    }

    // Use it: convert a won position against the bot.
    for _ in 0..4 {
        let Some(p) = next(&used, r - 150) else { break };
        used.insert(p.id.clone());
        if let Some(item) = playout_item(pool, &p, t).await? {
            push(item, Some(&p), &mut used);
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fork_puzzle() -> BankPuzzle {
        // Opponent plays Kf8-e8?; Nc7+ then forks king and rook.
        BankPuzzle {
            id: "t1".into(),
            fen: "r4k2/8/8/3N4/8/8/8/4K3 b - - 0 1".into(),
            moves: vec!["f8e8".into(), "d5c7".into(), "e8d7".into(), "c7a8".into()],
            rating: 900,
            themes: vec!["fork".into()],
        }
    }

    #[test]
    fn find_item_has_solution_and_hints() {
        let item = find_item(&fork_puzzle(), "fork").unwrap();
        assert_eq!(item.solution_uci, vec!["d5c7", "e8d7", "c7a8"]);
        assert_eq!(item.line_san, vec!["Nc7+", "Kd7", "Nxa8"]);
        assert_eq!(item.hints, vec!["Look for a fork.", "Move your knight on d5.", "Play Nc7+."]);
        assert!(item.prompt.starts_with("White to move"), "{}", item.prompt);
    }

    #[test]
    fn idea_quiz_answer_is_the_technique() {
        for rng in 0..10 {
            let item = idea_quiz(&fork_puzzle(), "fork", rng).unwrap();
            let q = item.quiz.unwrap();
            assert_eq!(q.choices.len(), 4);
            assert_eq!(q.choices[q.answer_choice.unwrap() as usize], "Fork");
            assert_eq!(q.arrow_uci.as_deref(), Some("d5c7"));
            assert!(q.explanation.contains("forks"), "{}", q.explanation);
        }
    }

    #[test]
    fn piece_quiz_answers() {
        let q = piece_quiz(&fork_puzzle(), "fork").unwrap().quiz.unwrap();
        assert_eq!(q.answer_squares, vec!["d5"]);
        // A loose-piece question lists every enemy piece that can be won.
        let p = BankPuzzle {
            id: "t2".into(),
            fen: "4k3/8/8/3n4/8/8/8/3RK3 b - - 0 1".into(),
            moves: vec!["e8e7".into(), "d1d5".into()],
            rating: 600,
            themes: vec!["hangingPiece".into()],
        };
        let q = piece_quiz(&p, "hanging_piece").unwrap().quiz.unwrap();
        assert_eq!(q.answer_squares, vec!["d5"]);
    }

    #[test]
    fn every_technique_has_quizzes_from_the_bank() {
        let bank = Bank::builtin();
        for (tag, themes) in TECHNIQUES {
            let mut ok = 0;
            for seed in 0..10 {
                let Some(p) = bank.pick(themes, 1400, &HashSet::new(), seed) else { continue };
                ok += (idea_quiz(p, tag, seed).is_some() && find_item(p, tag).is_some()) as u32;
            }
            assert!(ok >= 8, "{tag}: {ok}/10");
        }
    }
}
