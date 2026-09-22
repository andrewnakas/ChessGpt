//! Position helpers on top of shakmaty: FEN/EPD, SAN/UCI conversion, line
//! validation, game phase, and deterministic tactical features for prompts.

use serde::{Deserialize, Serialize};
use shakmaty::fen::{Epd, Fen};
use shakmaty::san::{San, SanPlus};
use shakmaty::uci::UciMove;
use shakmaty::{Bitboard, CastlingMode, Chess, Color, EnPassantMode, Move, Position, Role, Square};

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ChessError {
    #[error("invalid FEN: {0}")]
    Fen(String),
    #[error("illegal or unparseable move {mv:?} at index {index}")]
    IllegalMove { index: usize, mv: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Side {
    White,
    Black,
}

impl From<Color> for Side {
    fn from(c: Color) -> Self {
        match c {
            Color::White => Side::White,
            Color::Black => Side::Black,
        }
    }
}

impl From<Side> for Color {
    fn from(s: Side) -> Self {
        match s {
            Side::White => Color::White,
            Side::Black => Color::Black,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Phase {
    Opening,
    Middlegame,
    Endgame,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Opening => "opening",
            Phase::Middlegame => "middlegame",
            Phase::Endgame => "endgame",
        }
    }
}

pub const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

pub fn parse_fen(fen: &str) -> Result<Chess, ChessError> {
    let f = Fen::from_ascii(fen.trim().as_bytes()).map_err(|e| ChessError::Fen(e.to_string()))?;
    f.into_position(CastlingMode::Standard)
        .map_err(|e| ChessError::Fen(e.to_string()))
}

pub fn to_fen(pos: &Chess) -> String {
    Fen::from_position(pos, EnPassantMode::Legal).to_string()
}

/// FEN without move counters, with the e.p. square only when capture is legal.
/// Positions that are identical for play share a key.
pub fn fen_key(pos: &Chess) -> String {
    Epd::from_position(pos, EnPassantMode::Legal).to_string()
}

pub fn fen_key_of(fen: &str) -> Result<String, ChessError> {
    Ok(fen_key(&parse_fen(fen)?))
}

pub fn move_to_uci(m: Move) -> String {
    UciMove::from_move(m, CastlingMode::Standard).to_string()
}

pub fn uci_to_move(pos: &Chess, uci: &str) -> Option<Move> {
    let u: UciMove = uci.parse().ok()?;
    u.to_move(pos).ok()
}

pub fn san_to_move(pos: &Chess, san: &str) -> Option<Move> {
    let cleaned = san.trim().trim_end_matches(['!', '?']);
    let s = San::from_ascii(cleaned.as_bytes())
        .or_else(|_| SanPlus::from_ascii(cleaned.as_bytes()).map(|sp| sp.san))
        .ok()?;
    s.to_move(pos).ok()
}

/// SAN with check/mate suffix.
pub fn move_to_san(pos: &Chess, m: Move) -> String {
    SanPlus::from_move(pos.clone(), m).to_string()
}

pub fn uci_to_san(pos: &Chess, uci: &str) -> Option<String> {
    uci_to_move(pos, uci).map(|m| move_to_san(pos, m))
}

pub fn legal_sans(pos: &Chess) -> Vec<String> {
    pos.legal_moves().into_iter().map(|m| move_to_san(pos, m)).collect()
}

/// A line of moves played out from a position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PlayedLine {
    pub san: Vec<String>,
    pub uci: Vec<String>,
    pub final_fen: String,
}

/// Play SAN moves one by one; error names the first illegal move.
pub fn play_sans(pos: &Chess, sans: &[impl AsRef<str>]) -> Result<(Chess, PlayedLine), ChessError> {
    let mut p = pos.clone();
    let mut line = PlayedLine { san: vec![], uci: vec![], final_fen: String::new() };
    for (i, s) in sans.iter().enumerate() {
        let m = san_to_move(&p, s.as_ref())
            .ok_or_else(|| ChessError::IllegalMove { index: i, mv: s.as_ref().to_string() })?;
        line.san.push(move_to_san(&p, m));
        line.uci.push(move_to_uci(m));
        p.play_unchecked(m);
    }
    line.final_fen = to_fen(&p);
    Ok((p, line))
}

/// Convert a UCI principal variation to SAN, stopping at the first illegal move.
pub fn pv_to_san(pos: &Chess, pv: &[impl AsRef<str>]) -> Vec<String> {
    let mut p = pos.clone();
    let mut out = Vec::with_capacity(pv.len());
    for u in pv {
        let Some(m) = uci_to_move(&p, u.as_ref()) else { break };
        out.push(move_to_san(&p, m));
        p.play_unchecked(m);
    }
    out
}

/// Numbered SAN line text, e.g. "12. Nf3 Nc6 13. Bb5" or "12... Nc6 13. Bb5".
pub fn numbered_line(pos: &Chess, sans: &[String]) -> String {
    let mut n = pos.fullmoves().get();
    let mut white = pos.turn() == Color::White;
    let mut out = String::new();
    for (i, s) in sans.iter().enumerate() {
        if white {
            out.push_str(&format!("{n}. "));
        } else if i == 0 {
            out.push_str(&format!("{n}... "));
        }
        out.push_str(s);
        out.push(' ');
        if !white {
            n += 1;
        }
        white = !white;
    }
    out.trim_end().to_string()
}

fn minors_and_majors(pos: &Chess) -> u32 {
    let b = pos.board();
    (b.occupied() & !b.by_role(Role::Pawn) & !b.by_role(Role::King)).count() as u32
}

/// Approximate game phase, modelled loosely on lichess' Divider: endgame when
/// at most 6 minor/major pieces remain; middlegame when at most 10 remain or a
/// back rank has thinned out; otherwise opening.
pub fn phase(pos: &Chess) -> Phase {
    let pieces = minors_and_majors(pos);
    if pieces <= 6 {
        return Phase::Endgame;
    }
    let b = pos.board();
    let white_back = (b.by_color(Color::White) & Bitboard::from_rank(shakmaty::Rank::First)).count();
    let black_back = (b.by_color(Color::Black) & Bitboard::from_rank(shakmaty::Rank::Eighth)).count();
    if pieces <= 10 || white_back < 4 || black_back < 4 {
        Phase::Middlegame
    } else {
        Phase::Opening
    }
}

pub fn role_value(r: Role) -> i32 {
    match r {
        Role::Pawn => 1,
        Role::Knight | Role::Bishop => 3,
        Role::Rook => 5,
        Role::Queen => 9,
        Role::King => 0,
    }
}

pub fn role_name(r: Role) -> &'static str {
    match r {
        Role::Pawn => "pawn",
        Role::Knight => "knight",
        Role::Bishop => "bishop",
        Role::Rook => "rook",
        Role::Queen => "queen",
        Role::King => "king",
    }
}

/// Material balance in pawns, White minus Black.
pub fn material_balance(pos: &Chess) -> i32 {
    let b = pos.board();
    let mut total = 0;
    for sq in b.occupied() {
        if let Some(p) = b.piece_at(sq) {
            let v = role_value(p.role);
            total += if p.color == Color::White { v } else { -v };
        }
    }
    total
}

/// Pieces of `color` that are attacked and either undefended or attacked by a
/// cheaper piece. Kings excluded.
pub fn hanging_pieces(pos: &Chess, color: Color) -> Vec<(Square, Role)> {
    let b = pos.board();
    let occ = b.occupied();
    let mut out = vec![];
    for sq in b.by_color(color) {
        let Some(role) = b.role_at(sq) else { continue };
        if role == Role::King {
            continue;
        }
        let attackers = b.attacks_to(sq, !color, occ);
        if attackers.is_empty() {
            continue;
        }
        let defenders = b.attacks_to(sq, color, occ);
        let cheapest_attacker = attackers
            .into_iter()
            .filter_map(|a| b.role_at(a))
            .map(role_value)
            .min()
            .unwrap_or(99);
        if defenders.is_empty() || cheapest_attacker < role_value(role) {
            out.push((sq, role));
        }
    }
    out
}

/// Short, deterministic facts about a position for LLM prompts. Every fact is
/// computed, never guessed.
pub fn features(pos: &Chess) -> Vec<String> {
    let mut f = vec![];
    let turn = pos.turn();
    let side = |c: Color| if c == Color::White { "White" } else { "Black" };
    if pos.is_checkmate() {
        f.push(format!("{} is checkmated", side(turn)));
        return f;
    }
    if pos.is_stalemate() {
        f.push("stalemate".into());
        return f;
    }
    if pos.is_check() {
        f.push(format!("{} to move is in check", side(turn)));
    }
    let bal = material_balance(pos);
    f.push(match bal {
        0 => "material is level".to_string(),
        b if b > 0 => format!("White is up {b} pawn(s) of material"),
        b => format!("Black is up {} pawn(s) of material", -b),
    });
    for c in [turn, !turn] {
        let h = hanging_pieces(pos, c);
        if !h.is_empty() {
            let list: Vec<String> =
                h.iter().map(|(sq, r)| format!("{} on {}", role_name(*r), sq)).collect();
            f.push(format!("{} has loose/attacked pieces: {}", side(c), list.join(", ")));
        }
    }
    let moves = pos.legal_moves();
    let checks: Vec<String> = moves
        .iter()
        .filter(|m| {
            let mut after = pos.clone();
            after.play_unchecked(**m);
            after.is_check()
        })
        .map(|m| move_to_san(pos, *m))
        .collect();
    if !checks.is_empty() {
        f.push(format!("checks available to {}: {}", side(turn), checks.join(", ")));
    }
    let captures: Vec<String> = pos.capture_moves().iter().map(|m| move_to_san(pos, *m)).collect();
    if !captures.is_empty() {
        f.push(format!("captures available to {}: {}", side(turn), captures.join(", ")));
    }
    f.push(format!("phase: {}", phase(pos).as_str()));
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_start() {
        let p = parse_fen(START_FEN).unwrap();
        assert_eq!(to_fen(&p), START_FEN);
        assert_eq!(fen_key(&p), "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq -");
        assert_eq!(legal_sans(&p).len(), 20);
    }

    #[test]
    fn play_and_convert() {
        let p = parse_fen(START_FEN).unwrap();
        let (_, line) = play_sans(&p, &["e4", "e5", "Nf3", "Nc6", "Bb5"]).unwrap();
        assert_eq!(line.uci, vec!["e2e4", "e7e5", "g1f3", "b8c6", "f1b5"]);
        assert_eq!(pv_to_san(&p, &line.uci), line.san);
        assert_eq!(numbered_line(&p, &line.san), "1. e4 e5 2. Nf3 Nc6 3. Bb5");
        let err = play_sans(&p, &["e4", "Ke2"]).unwrap_err();
        assert_eq!(err, ChessError::IllegalMove { index: 1, mv: "Ke2".into() });
    }

    #[test]
    fn mate_in_two_features() {
        let p = parse_fen("r2qkb1r/pp2nppp/3p4/2pNN1B1/2BnP3/3P4/PPP2PPP/R2bK2R w KQkq - 1 10").unwrap();
        assert_eq!(uci_to_san(&p, "d5f6").as_deref(), Some("Nf6+"));
        let f = features(&p);
        assert!(f.iter().any(|s| s.starts_with("checks available to White")), "{f:?}");
        assert!(f.iter().any(|s| s.contains("Black is up")), "{f:?}");
    }

    #[test]
    fn castling_uci_standard() {
        let p = parse_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
        let m = san_to_move(&p, "O-O").unwrap();
        assert_eq!(move_to_uci(m), "e1g1");
    }
}
