//! Transformations of a puzzle (position plus UCI solution) that keep the idea
//! but change how it looks: colours swapped, board mirrored, pieces shifted,
//! bystanders removed. Results are only legal-checked here; whether the idea
//! still works is for the engine and the motif detectors to confirm.

use shakmaty::fen::Fen;
use shakmaty::{Bitboard, CastlingMode, Chess, EnPassantMode, Position, Role, Setup, Square};

use crate::position::{parse_fen, uci_to_move};

/// A position and the solver's moves from it (UCI, alternating sides).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    pub fen: String,
    pub solution: Vec<String>,
}

fn setup_of(fen: &str) -> Option<Setup> {
    let pos = parse_fen(fen).ok()?;
    Some(pos.to_setup(EnPassantMode::Legal))
}

fn map_uci(uci: &str, f: &impl Fn(Square) -> Option<Square>) -> Option<String> {
    if uci.len() < 4 || !uci.is_ascii() {
        return None;
    }
    let from: Square = uci[0..2].parse().ok()?;
    let to: Square = uci[2..4].parse().ok()?;
    Some(format!("{}{}{}", f(from)?, f(to)?, &uci[4..]))
}

/// Build the variant, checking the position and every solution move are legal.
fn finish(setup: Setup, solution: &[String], f: impl Fn(Square) -> Option<Square>) -> Option<Variant> {
    let pos: Chess = setup.position(CastlingMode::Standard).ok()?;
    let fen = Fen::from_position(&pos, EnPassantMode::Legal).to_string();
    let solution: Vec<String> = solution.iter().map(|u| map_uci(u, &f)).collect::<Option<_>>()?;
    let mut p = pos;
    for u in &solution {
        let m = uci_to_move(&p, u)?;
        p.play_unchecked(m);
    }
    Some(Variant { fen, solution })
}

/// Same position with colours swapped and ranks mirrored: White's idea becomes
/// Black's, played from the other side of the board.
pub fn color_flip(fen: &str, solution: &[String]) -> Option<Variant> {
    finish(setup_of(fen)?.into_mirrored(), solution, |s| Some(s.flip_vertical()))
}

/// Mirror a-file to h-file. Only without castling rights (castling is not
/// symmetric).
pub fn file_mirror(fen: &str, solution: &[String]) -> Option<Variant> {
    let mut s = setup_of(fen)?;
    if s.castling_rights.any() {
        return None;
    }
    s.board.flip_horizontal();
    s.ep_square = s.ep_square.map(Square::flip_horizontal);
    finish(s, solution, |sq| Some(sq.flip_horizontal()))
}

fn offset(sq: Square, df: i32, dr: i32) -> Option<Square> {
    let f = i32::from(sq.file()) + df;
    let r = i32::from(sq.rank()) + dr;
    if !(0..8).contains(&f) || !(0..8).contains(&r) {
        return None;
    }
    Some(Square::from_coords(shakmaty::File::new(f as u32), shakmaty::Rank::new(r as u32)))
}

/// Translate every piece by `df` files and `dr` ranks. Fails when a piece would
/// leave the board, a pawn would reach its first or last rank, or the position
/// has castling rights.
pub fn shift(fen: &str, solution: &[String], df: i32, dr: i32) -> Option<Variant> {
    if df == 0 && dr == 0 {
        return None;
    }
    let s = setup_of(fen)?;
    if s.castling_rights.any() {
        return None;
    }
    let mut out = s.clone();
    out.board = shakmaty::Board::empty();
    out.ep_square = None;
    for (sq, piece) in s.board.clone() {
        let to = offset(sq, df, dr)?;
        if piece.role == Role::Pawn && matches!(to.rank(), shakmaty::Rank::First | shakmaty::Rank::Eighth) {
            return None;
        }
        out.board.set_piece_at(to, piece);
    }
    finish(out, solution, |sq| offset(sq, df, dr))
}

/// Squares that matter to the solution: every from/to square of its moves plus
/// the squares of pieces attacking or defending them, and both kings with
/// their neighbourhoods.
fn relevant_squares(pos: &Chess, solution: &[String]) -> Option<Bitboard> {
    let mut touched = Bitboard::EMPTY;
    let mut p = pos.clone();
    for u in solution {
        let m = uci_to_move(&p, u)?;
        if let Some(from) = m.from() {
            touched.add(from);
        }
        touched.add(m.to());
        p.play_unchecked(m);
    }
    let b = pos.board();
    let occ = b.occupied();
    let mut keep = touched;
    for sq in touched {
        keep |= b.attacks_to(sq, shakmaty::Color::White, occ);
        keep |= b.attacks_to(sq, shakmaty::Color::Black, occ);
    }
    for k in b.kings() {
        keep.add(k);
        keep |= shakmaty::attacks::king_attacks(k);
    }
    Some(keep)
}

/// Remove pieces that take no part in the solution: not moved, captured, or
/// guarding a square the solution uses, and not next to a king. Returns `None`
/// when nothing would be removed.
pub fn prune_bystanders(fen: &str, solution: &[String]) -> Option<Variant> {
    let pos = parse_fen(fen).ok()?;
    let keep = relevant_squares(&pos, solution)?;
    let mut s = pos.to_setup(EnPassantMode::Legal);
    let removable = s.board.occupied() & !keep;
    if removable.is_empty() {
        return None;
    }
    for sq in removable {
        s.board.discard_piece_at(sq);
    }
    // Castling rights need the rooks they name.
    s.castling_rights &= s.board.rooks();
    finish(s, solution, Some)
}

/// Every distinct variant of a puzzle, cheapest transformations first.
pub fn all_variants(fen: &str, solution: &[String]) -> Vec<Variant> {
    let mut out: Vec<Variant> = vec![];
    let mut push = |v: Option<Variant>| {
        if let Some(v) = v {
            if v.fen != fen && !out.iter().any(|o| o.fen == v.fen) {
                out.push(v);
            }
        }
    };
    push(color_flip(fen, solution));
    push(file_mirror(fen, solution));
    push(prune_bystanders(fen, solution));
    for (df, dr) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, -1)] {
        push(shift(fen, solution, df, dr));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motifs::{Motif, move_motifs};

    fn sol(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    fn first_move_motifs(v: &Variant) -> Vec<Motif> {
        let p = parse_fen(&v.fen).unwrap();
        let m = uci_to_move(&p, &v.solution[0]).unwrap();
        move_motifs(&p, m).into_iter().map(|h| h.motif).collect()
    }

    const FORK: &str = "r3k3/8/8/3N4/8/8/8/4K3 w - - 0 1";

    #[test]
    fn color_flip_keeps_the_fork() {
        let v = color_flip(FORK, &sol(&["d5c7"])).unwrap();
        assert_eq!(v.fen, "4k3/8/8/8/3n4/8/8/R3K3 b - - 0 1");
        assert_eq!(v.solution, sol(&["d4c2"]));
        assert!(first_move_motifs(&v).contains(&Motif::Fork));
    }

    #[test]
    fn file_mirror_keeps_the_fork() {
        let v = file_mirror(FORK, &sol(&["d5c7"])).unwrap();
        assert_eq!(v.solution, sol(&["e5f7"]));
        assert!(first_move_motifs(&v).contains(&Motif::Fork));
        let castles = "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1";
        assert!(file_mirror(castles, &sol(&["e1g1"])).is_none());
    }

    #[test]
    fn shift_moves_everything_or_fails() {
        let v = shift(FORK, &sol(&["d5c7"]), 1, 0).unwrap();
        assert_eq!(v.solution, sol(&["e5d7"]));
        assert!(first_move_motifs(&v).contains(&Motif::Fork));
        // The rook on a8 cannot move one file left.
        assert!(shift(FORK, &sol(&["d5c7"]), -1, 0).is_none());
        // A pawn pushed onto the first rank is rejected.
        assert!(shift("4k3/8/8/8/8/8/P7/4K3 w - - 0 1", &sol(&["a2a3"]), 1, -1).is_none());
    }

    #[test]
    fn back_rank_mate_survives_the_flip() {
        let v = color_flip("6k1/5ppp/8/8/8/8/8/3R2K1 w - - 0 1", &sol(&["d1d8"])).unwrap();
        assert_eq!(first_move_motifs(&v), vec![Motif::BackRank]);
    }

    #[test]
    fn prune_drops_only_bystanders() {
        // The pawns on a2/h7 play no part in Nc7+ forking king and rook.
        let fen = "r3k3/7p/8/3N4/8/8/P7/4K3 w - - 0 1";
        let v = prune_bystanders(fen, &sol(&["d5c7", "e8d7", "c7a8"])).unwrap();
        assert_eq!(v.fen, "r3k3/8/8/3N4/8/8/8/4K3 w - - 0 1");
        assert!(prune_bystanders(&v.fen, &v.solution).is_none());
    }

    #[test]
    fn illegal_solution_is_rejected() {
        assert!(color_flip(FORK, &sol(&["e1e3"])).is_none());
    }

    #[test]
    fn all_variants_are_distinct_and_legal() {
        let vs = all_variants(FORK, &sol(&["d5c7", "e8d7", "c7a8"]));
        assert!(vs.len() >= 3, "{vs:?}");
        for v in &vs {
            assert!(first_move_motifs(v).contains(&Motif::Fork), "{v:?}");
        }
    }

    /// Motifs the solver's moves create along a solution.
    fn solver_motifs(v: &Variant) -> std::collections::BTreeSet<Motif> {
        let mut p = parse_fen(&v.fen).unwrap();
        let mut out = std::collections::BTreeSet::new();
        for (i, u) in v.solution.iter().enumerate() {
            let m = uci_to_move(&p, u).unwrap();
            if i % 2 == 0 {
                out.extend(move_motifs(&p, m).into_iter().map(|h| h.motif));
            }
            p.play_unchecked(m);
        }
        out
    }

    /// Colour flips and mirrors are exact symmetries: across the Lichess sample
    /// every puzzle keeps its motifs.
    #[test]
    fn symmetries_keep_motifs_on_lichess_sample() {
        let data = include_str!("../../../fixtures/puzzles/sample.csv");
        let (mut n, mut mirrored) = (0, 0);
        for line in data.lines().skip(1) {
            let cols: Vec<&str> = line.split(',').collect();
            let mut p = parse_fen(cols[1]).unwrap();
            let mut moves = cols[2].split_whitespace();
            p.play_unchecked(uci_to_move(&p, moves.next().unwrap()).unwrap());
            let start = Variant {
                fen: crate::position::to_fen(&p),
                solution: moves.map(String::from).collect(),
            };
            let want = solver_motifs(&start);
            let flip = color_flip(&start.fen, &start.solution).expect(line);
            assert_eq!(solver_motifs(&flip), want, "{line}");
            if let Some(m) = file_mirror(&start.fen, &start.solution) {
                assert_eq!(solver_motifs(&m), want, "{line}");
                mirrored += 1;
            }
            n += 1;
        }
        assert!(n > 1000 && mirrored > n / 2, "{n} {mirrored}");
    }
}
