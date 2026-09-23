//! Anti-hallucination checks. Every move an LLM writes is parsed and checked
//! against the engine lines it was given, the game's own moves, and the legal
//! moves of the positions in question. Lines it proposes are replaced by the
//! engine's own line whenever possible.

use std::collections::HashSet;
use std::sync::LazyLock;

use api_types::{BetterMove, Verification, VerificationStatus};
use chess_core::position::{legal_sans, play_sans};
use regex::Regex;
use shakmaty::Chess;

/// SAN-looking tokens in prose. Bare pawn moves like "e4" are only taken when
/// preceded by a move number ("12. e4", "12...e5") or followed by +/#, because
/// "the e4 square" is not a move.
static SAN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        (?:^|[^A-Za-z0-9])
        (
            O-O-O[+\#]? | O-O[+\#]? | 0-0-0[+\#]? | 0-0[+\#]?
          | [KQRBN][a-h]?[1-8]?x?[a-h][1-8][+\#]?
          | [a-h]x[a-h][1-8](?:=[QRBN])?[+\#]?
          | [a-h][1-8]=[QRBN][+\#]?
          | [a-h][1-8][+\#]
        )",
    )
    .unwrap()
});

static NUMBERED_PAWN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\d+\.(?:\.\.)?\s*([a-h][1-8])\b").unwrap());

/// Normalise SAN for comparison: drop check/annotation suffixes, zero-castling.
pub fn norm(san: &str) -> String {
    san.trim()
        .trim_end_matches(['+', '#', '!', '?'])
        .replace("0-0-0", "O-O-O")
        .replace("0-0", "O-O")
}

pub fn san_tokens(text: &str) -> Vec<String> {
    let mut out: Vec<String> = SAN_RE.captures_iter(text).map(|c| c[1].to_string()).collect();
    out.extend(NUMBERED_PAWN_RE.captures_iter(text).map(|c| c[1].to_string()));
    let mut seen = HashSet::new();
    out.retain(|t| seen.insert(norm(t)));
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grounded {
    /// Appears in an engine line or among the game's moves.
    Engine,
    /// Legal in a relevant position but not backed by any engine line.
    LegalOnly,
    /// Neither: illegal or invented.
    Unknown,
}

/// What an LLM answer may refer to.
#[derive(Debug, Clone, Default)]
pub struct Grounding {
    lines: Vec<Vec<String>>,
    line_moves: HashSet<String>,
    legal: HashSet<String>,
    history: HashSet<String>,
}

impl Grounding {
    pub fn new() -> Grounding {
        Grounding::default()
    }

    /// Engine-backed SAN line (from any position).
    pub fn add_line(&mut self, sans: &[String]) {
        for s in sans {
            self.line_moves.insert(norm(s));
        }
        if !sans.is_empty() {
            self.lines.push(sans.to_vec());
        }
    }

    /// Positions whose legal moves may be mentioned.
    pub fn add_position(&mut self, pos: &Chess) {
        for s in legal_sans(pos) {
            self.legal.insert(norm(&s));
        }
    }

    /// Moves actually played in the game.
    pub fn add_history(&mut self, sans: &[String]) {
        for s in sans {
            self.history.insert(norm(s));
        }
    }

    pub fn check(&self, san: &str) -> Grounded {
        let n = norm(san);
        if self.line_moves.contains(&n) || self.history.contains(&n) {
            Grounded::Engine
        } else if self.legal.contains(&n) {
            Grounded::LegalOnly
        } else {
            Grounded::Unknown
        }
    }

    /// The engine line starting with `first` (normalised match).
    pub fn line_starting_with(&self, first: &str) -> Option<&Vec<String>> {
        let f = norm(first);
        self.lines.iter().find(|l| l.first().is_some_and(|m| norm(m) == f))
    }
}

#[derive(Debug, Default, Clone)]
pub struct Findings {
    /// Illegal or invented moves (normalised SAN).
    pub hard: Vec<String>,
    /// Legal moves without engine backing.
    pub unverified: Vec<String>,
    pub issues: Vec<String>,
}

impl Findings {
    fn add_token(&mut self, g: &Grounding, t: &str) {
        match g.check(t) {
            Grounded::Engine => {}
            Grounded::LegalOnly => {
                if !self.unverified.iter().any(|u| norm(u) == norm(t)) {
                    self.unverified.push(t.to_string());
                }
            }
            Grounded::Unknown => {
                if !self.hard.iter().any(|u| norm(u) == norm(t)) {
                    self.hard.push(t.to_string());
                }
            }
        }
    }

    pub fn scan_text(&mut self, g: &Grounding, text: &str) {
        for t in san_tokens(text) {
            self.add_token(g, &t);
        }
    }

    pub fn scan_moves(&mut self, g: &Grounding, moves: &[String]) {
        for m in moves {
            self.add_token(g, m);
        }
    }

    pub fn to_verification(&self, stripped: bool) -> Verification {
        let mut issues = self.issues.clone();
        if !self.hard.is_empty() {
            issues.push(format!("moves not supported by the engine or illegal: {}", self.hard.join(", ")));
        }
        let status = if stripped || !self.hard.is_empty() {
            VerificationStatus::Partial
        } else {
            VerificationStatus::Ok
        };
        Verification { status, issues, unverified_moves: self.unverified.clone(), rejected_moves: self.hard.clone() }
    }
}

/// Check the suggested better move and replace its line with the engine's.
/// `before` is the position the move is played from.
pub fn repair_better_move(
    g: &Grounding,
    before: &Chess,
    candidates: &[String],
    bm: &mut BetterMove,
    max_plies: usize,
    f: &mut Findings,
) {
    let legal = play_sans(before, &[bm.san.as_str()]).is_ok();
    if !legal {
        f.hard.push(bm.san.clone());
        return;
    }
    if !candidates.iter().any(|c| norm(c) == norm(&bm.san)) {
        f.issues.push(format!("{} is legal but not among the engine's top moves", bm.san));
        if !f.unverified.iter().any(|u| norm(u) == norm(&bm.san)) {
            f.unverified.push(bm.san.clone());
        }
    }
    if let Some(line) = g.line_starting_with(&bm.san) {
        let n = bm.line_san.len().clamp(2, max_plies).min(line.len());
        bm.line_san = line[..n].to_vec();
        bm.san = line[0].clone();
        return;
    }
    // No engine line: keep only the legal prefix of the model's line.
    let mut keep = vec![];
    for i in 0..bm.line_san.len() {
        if play_sans(before, &bm.line_san[..=i]).is_ok() {
            keep.push(bm.line_san[i].clone());
        } else {
            f.issues.push(format!("line after {} was cut: {} is illegal", keep.join(" "), bm.line_san[i]));
            break;
        }
    }
    if keep.is_empty() || norm(&keep[0]) != norm(&bm.san) {
        keep = vec![bm.san.clone()];
    }
    bm.line_san = keep;
}

/// Remove sentences that contain any of `bad` tokens. Returns (text, changed).
pub fn strip_sentences(text: &str, bad: &[String]) -> (String, bool) {
    if bad.is_empty() {
        return (text.to_string(), false);
    }
    let bad: HashSet<String> = bad.iter().map(|b| norm(b)).collect();
    let mut out = String::new();
    let mut changed = false;
    let mut start = 0;
    let bytes: Vec<char> = text.chars().collect();
    let mut sentences = vec![];
    for (i, c) in bytes.iter().enumerate() {
        if matches!(c, '.' | '!' | '?') && bytes.get(i + 1).is_none_or(|n| n.is_whitespace()) {
            // Don't split after a move number ("12." / "12..."): digits that
            // are not the end of a square like "f6".
            let mut j = i;
            while j > 0 && bytes[j - 1].is_ascii_digit() {
                j -= 1;
            }
            let move_number = j < i && (j == 0 || !bytes[j - 1].is_ascii_alphabetic());
            if !move_number {
                sentences.push(bytes[start..=i].iter().collect::<String>());
                start = i + 1;
            }
        }
    }
    if start < bytes.len() {
        sentences.push(bytes[start..].iter().collect::<String>());
    }
    for s in sentences {
        if san_tokens(&s).iter().any(|t| bad.contains(&norm(t))) {
            changed = true;
        } else {
            out.push_str(&s);
        }
    }
    (out.trim().to_string(), changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_core::position::parse_fen;

    const ITALIAN: &str = "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3";

    fn grounding() -> Grounding {
        let pos = parse_fen(ITALIAN).unwrap();
        let mut g = Grounding::new();
        g.add_position(&pos);
        g.add_line(&["Nf6".into(), "Ng5".into(), "d5".into(), "exd5".into(), "Na5".into()]);
        g.add_line(&["Bc5".into(), "c3".into(), "Nf6".into(), "d4".into()]);
        g.add_history(&["e4".into(), "e5".into(), "Nf3".into(), "Nc6".into(), "Bc4".into()]);
        g
    }

    #[test]
    fn tokens_from_prose() {
        let t = san_tokens("After 3...Nf6 4. Ng5 d5! White plays exd5 and Black has Na5; the e4 square matters. O-O is fine, Qxf7# ends it.");
        let n: Vec<String> = t.iter().map(|s| norm(s)).collect();
        assert!(n.contains(&"Nf6".into()));
        assert!(n.contains(&"Ng5".into()));
        assert!(n.contains(&"exd5".into()));
        assert!(n.contains(&"O-O".into()));
        assert!(n.contains(&"Qxf7".into()));
        assert!(!n.contains(&"e4".into()), "bare squares are not moves: {n:?}");
    }

    #[test]
    fn grounding_levels() {
        let g = grounding();
        assert_eq!(g.check("Nf6"), Grounded::Engine);
        assert_eq!(g.check("exd5"), Grounded::Engine);
        assert_eq!(g.check("Bc4"), Grounded::Engine, "game history");
        assert_eq!(g.check("h6"), Grounded::LegalOnly);
        assert_eq!(g.check("Qxf7#"), Grounded::Unknown);
    }

    #[test]
    fn better_move_line_is_replaced_by_engine_line() {
        let g = grounding();
        let pos = parse_fen(ITALIAN).unwrap();
        let mut f = Findings::default();
        let mut bm = BetterMove { san: "Nf6".into(), line_san: vec!["Nf6".into(), "Ng5".into(), "Bc5".into()], reason: "x".into() };
        repair_better_move(&g, &pos, &["Nf6".into(), "Bc5".into()], &mut bm, 8, &mut f);
        assert_eq!(bm.line_san, vec!["Nf6", "Ng5", "d5"]);
        assert!(f.hard.is_empty() && f.issues.is_empty());

        let mut bm = BetterMove { san: "h6".into(), line_san: vec!["h6".into(), "Qxf7".into()], reason: "x".into() };
        let mut f = Findings::default();
        repair_better_move(&g, &pos, &["Nf6".into(), "Bc5".into()], &mut bm, 8, &mut f);
        assert_eq!(bm.line_san, vec!["h6"]);
        assert_eq!(f.unverified, vec!["h6"]);
        assert_eq!(f.issues.len(), 2, "{:?}", f.issues);

        let mut bm = BetterMove { san: "Kd7".into(), line_san: vec![], reason: "x".into() };
        let mut f = Findings::default();
        repair_better_move(&g, &pos, &["Nf6".into()], &mut bm, 8, &mut f);
        assert_eq!(f.hard, vec!["Kd7"]);
    }

    #[test]
    fn stripping_removes_only_bad_sentences() {
        let text = "Black should develop with 3...Nf6. The flashy Qxf7# is not available. After 4. Ng5 d5 it is sharp. Keep the king safe.";
        let (out, changed) = strip_sentences(text, &["Qxf7#".into()]);
        assert!(changed);
        assert_eq!(out, "Black should develop with 3...Nf6. After 4. Ng5 d5 it is sharp. Keep the king safe.");
    }

    #[test]
    fn findings_to_verification() {
        let g = grounding();
        let mut f = Findings::default();
        f.scan_text(&g, "Play 3...Nf6, not 3...h6 or Qxf7#.");
        assert_eq!(f.hard, vec!["Qxf7#"]);
        assert_eq!(f.unverified, vec!["h6"]);
        assert_eq!(f.to_verification(false).status, VerificationStatus::Partial);
        let mut ok = Findings::default();
        ok.scan_text(&g, "3...Nf6 4. Ng5 d5 5. exd5 Na5.");
        assert_eq!(ok.to_verification(false).status, VerificationStatus::Ok);
    }
}
