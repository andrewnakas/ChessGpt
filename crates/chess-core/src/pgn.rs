//! PGN parsing into a flat list of plies with FENs, clocks, and any embedded
//! Lichess `[%eval]` annotations and NAGs (used by regression tests).

use std::io::Cursor;
use std::ops::ControlFlow;

use pgn_reader::{Nag, RawComment, RawTag, Reader, SanPlus, Visitor};
use serde::{Deserialize, Serialize};
use shakmaty::{CastlingMode, Chess, Position, fen::Fen};

use crate::position::{ChessError, Side, move_to_uci, to_fen};
use crate::score::Score;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PlyMove {
    /// 1-based ply index within the game.
    pub ply: u32,
    pub mover: Side,
    pub san: String,
    pub uci: String,
    pub fen_before: String,
    pub fen_after: String,
    pub clock_ms: Option<u32>,
    /// White-POV `[%eval]` found in the PGN after this move (Lichess exports).
    pub pgn_eval: Option<Score>,
    /// First move-assessment NAG (1..=6) attached to this move.
    pub nag: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ParsedGame {
    pub tags: Vec<(String, String)>,
    pub start_fen: String,
    pub moves: Vec<PlyMove>,
}

impl ParsedGame {
    pub fn tag(&self, name: &str) -> Option<&str> {
        self.tags
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty() && *v != "?")
    }

    pub fn elo(&self, side: Side) -> Option<u32> {
        let t = match side {
            Side::White => "WhiteElo",
            Side::Black => "BlackElo",
        };
        self.tag(t).and_then(|v| v.parse().ok())
    }

    pub fn white_starts(&self) -> bool {
        self.moves.first().is_none_or(|m| m.mover == Side::White)
    }

    /// A game with no moves from a FEN.
    pub fn from_fen(fen: &str) -> Result<ParsedGame, ChessError> {
        let pos = crate::position::parse_fen(fen)?;
        Ok(ParsedGame {
            tags: vec![("FEN".into(), to_fen(&pos)), ("SetUp".into(), "1".into())],
            start_fen: to_fen(&pos),
            moves: vec![],
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PgnError {
    #[error("no game found in PGN")]
    Empty,
    #[error("game {game}: {msg}")]
    Invalid { game: usize, msg: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

struct Collector {
    game_index: usize,
}

struct Tags {
    tags: Vec<(String, String)>,
    start: Option<Chess>,
    error: Option<String>,
}

struct Movetext {
    tags: Vec<(String, String)>,
    start_fen: String,
    pos: Chess,
    moves: Vec<PlyMove>,
    depth: u32,
    error: Option<String>,
}

impl Visitor for Collector {
    type Tags = Tags;
    type Movetext = Movetext;
    type Output = Result<ParsedGame, String>;

    fn begin_tags(&mut self) -> ControlFlow<Self::Output, Self::Tags> {
        self.game_index += 1;
        ControlFlow::Continue(Tags { tags: vec![], start: None, error: None })
    }

    fn tag(&mut self, tags: &mut Tags, name: &[u8], value: RawTag<'_>) -> ControlFlow<Self::Output> {
        let name = String::from_utf8_lossy(name).to_string();
        let value = value.decode_utf8_lossy().to_string();
        if name.eq_ignore_ascii_case("FEN") {
            match Fen::from_ascii(value.as_bytes())
                .map_err(|e| e.to_string())
                .and_then(|f| f.into_position::<Chess>(CastlingMode::Standard).map_err(|e| e.to_string()))
            {
                Ok(p) => tags.start = Some(p),
                Err(e) => tags.error = Some(format!("bad FEN tag: {e}")),
            }
        }
        tags.tags.push((name, value));
        ControlFlow::Continue(())
    }

    fn begin_movetext(&mut self, tags: Tags) -> ControlFlow<Self::Output, Self::Movetext> {
        if let Some(e) = tags.error {
            return ControlFlow::Break(Err(e));
        }
        let pos = tags.start.unwrap_or_default();
        ControlFlow::Continue(Movetext {
            tags: tags.tags,
            start_fen: to_fen(&pos),
            pos,
            moves: vec![],
            depth: 0,
            error: None,
        })
    }

    fn san(&mut self, mt: &mut Movetext, san_plus: SanPlus) -> ControlFlow<Self::Output> {
        if mt.depth > 0 || mt.error.is_some() {
            return ControlFlow::Continue(());
        }
        match san_plus.san.to_move(&mt.pos) {
            Ok(m) => {
                let fen_before = to_fen(&mt.pos);
                let mover = Side::from(mt.pos.turn());
                let san = crate::position::move_to_san(&mt.pos, m);
                mt.pos.play_unchecked(m);
                let ply = mt.moves.len() as u32 + 1;
                mt.moves.push(PlyMove {
                    ply,
                    mover,
                    san,
                    uci: move_to_uci(m),
                    fen_before,
                    fen_after: to_fen(&mt.pos),
                    clock_ms: None,
                    pgn_eval: None,
                    nag: None,
                });
            }
            Err(e) => {
                mt.error = Some(format!("illegal move {san_plus} at ply {}: {e}", mt.moves.len() + 1));
            }
        }
        ControlFlow::Continue(())
    }

    fn nag(&mut self, mt: &mut Movetext, nag: Nag) -> ControlFlow<Self::Output> {
        if mt.depth == 0
            && let Some(last) = mt.moves.last_mut()
            && last.nag.is_none()
            && (1..=6).contains(&nag.0)
        {
            last.nag = Some(nag.0);
        }
        ControlFlow::Continue(())
    }

    fn comment(&mut self, mt: &mut Movetext, comment: RawComment<'_>) -> ControlFlow<Self::Output> {
        if mt.depth > 0 {
            return ControlFlow::Continue(());
        }
        let text = String::from_utf8_lossy(comment.as_bytes());
        if let Some(last) = mt.moves.last_mut() {
            if let Some(c) = parse_command(&text, "%clk").and_then(|s| parse_clock(&s)) {
                last.clock_ms = Some(c);
            }
            if let Some(e) = parse_command(&text, "%eval").and_then(|s| parse_eval(&s)) {
                last.pgn_eval = Some(e);
            }
        }
        ControlFlow::Continue(())
    }

    fn begin_variation(&mut self, mt: &mut Movetext) -> ControlFlow<Self::Output, pgn_reader::Skip> {
        mt.depth += 1;
        ControlFlow::Continue(pgn_reader::Skip(true))
    }

    fn end_variation(&mut self, mt: &mut Movetext) -> ControlFlow<Self::Output> {
        mt.depth = mt.depth.saturating_sub(1);
        ControlFlow::Continue(())
    }

    fn end_game(&mut self, mt: Movetext) -> Self::Output {
        if let Some(e) = mt.error {
            return Err(e);
        }
        Ok(ParsedGame { tags: mt.tags, start_fen: mt.start_fen, moves: mt.moves })
    }
}

fn parse_command(text: &str, cmd: &str) -> Option<String> {
    let start = text.find(&format!("[{cmd} "))? + cmd.len() + 2;
    let end = text[start..].find(']')? + start;
    Some(text[start..end].trim().to_string())
}

fn parse_clock(s: &str) -> Option<u32> {
    let parts: Vec<&str> = s.split(':').collect();
    let (h, m, sec) = match parts.as_slice() {
        [h, m, s] => (h.parse::<f64>().ok()?, m.parse::<f64>().ok()?, s.parse::<f64>().ok()?),
        [m, s] => (0.0, m.parse::<f64>().ok()?, s.parse::<f64>().ok()?),
        _ => return None,
    };
    Some(((h * 3600.0 + m * 60.0 + sec) * 1000.0).round() as u32)
}

/// `0.17`, `-1.5`, `#3`, `#-2`, optionally followed by `,depth`.
fn parse_eval(s: &str) -> Option<Score> {
    let v = s.split(',').next()?.trim();
    if let Some(m) = v.strip_prefix('#') {
        return m.parse().ok().map(Score::Mate);
    }
    let pawns: f64 = v.parse().ok()?;
    Some(Score::Cp((pawns * 100.0).round() as i32))
}

/// Parse every game in a PGN string.
pub fn parse_pgn_many(pgn: &str) -> Result<Vec<ParsedGame>, PgnError> {
    let mut reader = Reader::new(Cursor::new(pgn.as_bytes()));
    let mut v = Collector { game_index: 0 };
    let mut out = vec![];
    while let Some(res) = reader.read_game(&mut v)? {
        match res {
            Ok(g) => out.push(g),
            Err(msg) => return Err(PgnError::Invalid { game: v.game_index, msg }),
        }
    }
    if out.is_empty() {
        return Err(PgnError::Empty);
    }
    Ok(out)
}

/// Parse the first game in a PGN string.
pub fn parse_pgn(pgn: &str) -> Result<ParsedGame, PgnError> {
    parse_pgn_many(pgn)?.into_iter().next().ok_or(PgnError::Empty)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPERA: &str = r#"[Event "Paris"]
[Site "Paris FRA"]
[Date "1858.??.??"]
[White "Paul Morphy"]
[Black "Duke Karl / Count Isouard"]
[Result "1-0"]

1. e4 e5 2. Nf3 d6 3. d4 Bg4 4. dxe5 Bxf3 5. Qxf3 dxe5 6. Bc4 Nf6 7. Qb3 Qe7
8. Nc3 c6 9. Bg5 b5 10. Nxb5 cxb5 11. Bxb5+ Nbd7 12. O-O-O Rd8 13. Rxd7 Rxd7
14. Rd1 Qe6 15. Bxd7+ Nxd7 16. Qb8+ Nxb8 17. Rd8# 1-0"#;

    #[test]
    fn opera_game() {
        let g = parse_pgn(OPERA).unwrap();
        assert_eq!(g.moves.len(), 33);
        assert_eq!(g.tag("White"), Some("Paul Morphy"));
        assert_eq!(g.moves[0].uci, "e2e4");
        assert_eq!(g.moves[22].san, "O-O-O");
        assert_eq!(g.moves[22].uci, "e1c1");
        assert_eq!(g.moves[32].san, "Rd8#");
        assert!(g.white_starts());
    }

    #[test]
    fn lichess_annotations() {
        let pgn = r#"[Event "x"]
[WhiteElo "1500"]

1. e4 { [%eval 0.36] [%clk 0:03:00] } 1... e5?! { [%eval 0.9] [%clk 0:02:59.5] } 2. Qh5 { [%eval #-3] } ( 2. Nf3 Nc6 ) 2... Nc6 *"#;
        let g = parse_pgn(pgn).unwrap();
        assert_eq!(g.moves.len(), 4);
        assert_eq!(g.moves[0].pgn_eval, Some(Score::Cp(36)));
        assert_eq!(g.moves[0].clock_ms, Some(180_000));
        assert_eq!(g.moves[1].clock_ms, Some(179_500));
        assert_eq!(g.moves[1].nag, Some(6));
        assert_eq!(g.moves[2].pgn_eval, Some(Score::Mate(-3)));
        assert_eq!(g.moves[3].san, "Nc6");
        assert_eq!(g.elo(Side::White), Some(1500));
    }

    #[test]
    fn illegal_move_reports_ply() {
        let err = parse_pgn("1. e4 e5 2. Ke3 *").unwrap_err();
        assert!(err.to_string().contains("ply 3"), "{err}");
    }

    #[test]
    fn fen_start() {
        let pgn = "[FEN \"6k1/5ppp/8/8/8/8/5PPP/3R2K1 w - - 0 1\"]\n[SetUp \"1\"]\n\n1. Rd8# 1-0";
        let g = parse_pgn(pgn).unwrap();
        assert_eq!(g.moves[0].uci, "d1d8");
    }
}
