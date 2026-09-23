//! Tactical and structural motifs, computed from the board. Prompts state them
//! as facts (so a small model narrates instead of calculating), and the mistake
//! index groups errors by them. Names match the coach's concept tags.

use serde::{Deserialize, Serialize};
use shakmaty::{Bitboard, Chess, Color, File, Move, Piece, Position, Rank, Role, Square, attacks};

use crate::position::{Side, move_to_san, numbered_line, role_name, san_to_move};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Motif {
    HangingPiece,
    /// An undefended pawn picked up: a fact for explanations, too small to
    /// count as a player's weakness.
    LoosePawn,
    Fork,
    Pin,
    Skewer,
    DiscoveredAttack,
    BackRank,
    MatingAttack,
    TrappedPiece,
    Promotion,
    KingSafety,
    PassedPawn,
    PawnStructure,
}

impl Motif {
    /// The concept tag this motif maps to.
    pub fn tag(self) -> &'static str {
        match self {
            Motif::HangingPiece | Motif::LoosePawn => "hanging_piece",
            Motif::Fork => "fork",
            Motif::Pin => "pin",
            Motif::Skewer => "skewer",
            Motif::DiscoveredAttack => "discovered_attack",
            Motif::BackRank => "back_rank",
            Motif::MatingAttack => "mating_attack",
            Motif::TrappedPiece => "trapped_piece",
            Motif::Promotion => "promotion",
            Motif::KingSafety => "king_safety",
            Motif::PassedPawn => "passed_pawn",
            Motif::PawnStructure => "pawn_structure",
        }
    }
}

/// One detected motif: who it favours and a one-line statement of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MotifHit {
    pub motif: Motif,
    pub by: Side,
    pub text: String,
}

fn side_name(c: Color) -> &'static str {
    if c == Color::White { "White" } else { "Black" }
}

/// Piece value for exchange logic; the king counts as priceless.
fn val(r: Role) -> i32 {
    match r {
        Role::Pawn => 1,
        Role::Knight | Role::Bishop => 3,
        Role::Rook => 5,
        Role::Queen => 9,
        Role::King => 100,
    }
}

fn piece_str(r: Role, sq: Square) -> String {
    format!("{} on {}", role_name(r), sq)
}

fn cheapest(pos: &Chess, set: Bitboard) -> i32 {
    set.into_iter()
        .filter_map(|s| pos.board().role_at(s))
        .map(val)
        .min()
        .unwrap_or(1000)
}

/// Can the piece on `sq` be won: attacked and undefended, or attacked by
/// something cheaper.
fn en_prise(pos: &Chess, sq: Square) -> bool {
    let b = pos.board();
    let Some(p) = b.piece_at(sq) else {
        return false;
    };
    let occ = b.occupied();
    let attackers = b.attacks_to(sq, !p.color, occ);
    if attackers.is_empty() {
        return false;
    }
    let defenders = b.attacks_to(sq, p.color, occ);
    defenders.is_empty() || cheapest(pos, attackers) < val(p.role)
}

/// Is `target` worth attacking with a piece of value `attacker`: the king, a
/// more valuable piece, or an undefended non-pawn.
fn worthy_target(pos: &Chess, target: Square, attacker: i32) -> bool {
    let b = pos.board();
    let Some(p) = b.piece_at(target) else {
        return false;
    };
    if p.role == Role::King || val(p.role) > attacker {
        return true;
    }
    p.role != Role::Pawn && b.attacks_to(target, p.color, b.occupied()).is_empty()
}

fn slider_attacks(sq: Square, piece: Piece, occ: Bitboard) -> Bitboard {
    attacks::attacks(sq, piece, occ)
}

/// Line relationships of `color`'s sliders against the enemy: (pin?, slider,
/// front piece, back piece). Pin when the back piece is worth more than the
/// front one; skewer when the front piece is the king or a major piece worth
/// more than the back one.
fn line_tactics(pos: &Chess, color: Color) -> Vec<(bool, Square, Square, Square)> {
    let b = pos.board();
    let occ = b.occupied();
    let enemy = b.by_color(!color);
    let mut out = vec![];
    for s in b.by_color(color) & b.sliders() {
        let Some(sp) = b.piece_at(s) else { continue };
        let first = slider_attacks(s, sp, occ);
        for front in first & enemy {
            let Some(fr) = b.role_at(front) else { continue };
            let through = slider_attacks(s, sp, occ.without(front)) & !first & enemy;
            for back in through {
                if !attacks::between(s, back).contains(front) {
                    continue;
                }
                let Some(br) = b.role_at(back) else { continue };
                if fr != Role::King
                    && val(br) > val(fr)
                    && (br == Role::King
                        || val(br) > val(sp.role)
                        || b.attacks_to(back, !color, occ.without(front)).is_empty())
                {
                    out.push((true, s, front, back));
                } else if (fr == Role::King || val(fr) >= 5)
                    && val(fr) > val(br)
                    && br != Role::Pawn
                    && (val(br) > val(sp.role)
                        || b.attacks_to(back, !color, occ.without(front)).is_empty())
                {
                    out.push((false, s, front, back));
                }
            }
        }
    }
    out
}

fn line_hit(
    pos: &Chess,
    color: Color,
    (pin, s, front, back): (bool, Square, Square, Square),
) -> MotifHit {
    let b = pos.board();
    let role = |sq| b.role_at(sq).unwrap_or(Role::Pawn);
    let slider = piece_str(role(s), s);
    let (f, bk) = (piece_str(role(front), front), piece_str(role(back), back));
    if pin {
        let kind = if role(back) == Role::King {
            "cannot legally move"
        } else {
            "cannot move without exposing it"
        };
        MotifHit {
            motif: Motif::Pin,
            by: color.into(),
            text: format!("the {slider} pins the {f} to the {bk}; it {kind}"),
        }
    } else {
        MotifHit {
            motif: Motif::Skewer,
            by: color.into(),
            text: format!("the {slider} skewers the {f}; when it moves, the {bk} behind it falls"),
        }
    }
}

/// A non-pawn, non-king piece of `color` that is attacked and has no safe
/// square to go to.
fn trapped_pieces(pos: &Chess, color: Color) -> Vec<Square> {
    let b = pos.board();
    let occ = b.occupied();
    let mut out = vec![];
    for sq in b.by_color(color) & !b.pawns() & !b.kings() {
        if !en_prise(pos, sq) {
            continue;
        }
        let Some(p) = b.piece_at(sq) else { continue };
        let dests = b.attacks_from(sq) & !b.by_color(color);
        let escapes = dests.into_iter().any(|d| {
            if b.role_at(d).is_some_and(|r| val(r) >= val(p.role)) {
                return true; // trades itself off at least evenly
            }
            let occ2 = occ.without(sq).with(d);
            let att = b.attacks_to(d, !color, occ2) & !Bitboard::from(d);
            if att.is_empty() {
                return true;
            }
            let def = b.attacks_to(d, color, occ2.without(d)) & !Bitboard::from(sq);
            !def.is_empty() && cheapest(pos, att) >= val(p.role)
        });
        if !escapes {
            out.push(sq);
        }
    }
    out
}

/// Mates in one available to `color` if it were their move.
fn mate_threats(pos: &Chess, color: Color) -> Vec<Move> {
    let p = if pos.turn() == color {
        pos.clone()
    } else {
        if pos.is_check() {
            return vec![];
        }
        match pos.clone().swap_turn() {
            Ok(p) => p,
            Err(_) => return vec![],
        }
    };
    p.legal_moves()
        .into_iter()
        .filter(|m| {
            let mut a = p.clone();
            a.play_unchecked(*m);
            a.is_checkmate()
        })
        .collect()
}

fn is_back_rank_mate(after: &Chess, m: Move) -> bool {
    let loser = after.turn();
    let Some(k) = after.board().king_of(loser) else {
        return false;
    };
    k.rank() == loser.backrank()
        && matches!(m.role(), Role::Rook | Role::Queen)
        && m.to().rank() == loser.backrank()
}

/// Motifs created by playing `m` in `pos`, from the mover's point of view.
pub fn move_motifs(pos: &Chess, m: Move) -> Vec<MotifHit> {
    let mover = pos.turn();
    let by: Side = mover.into();
    let san = numbered_line(pos, &[move_to_san(pos, m)]);
    let mut after = pos.clone();
    after.play_unchecked(m);
    let ab = after.board();
    let mut out = vec![];
    let hit = |motif, text: String| MotifHit { motif, by, text };

    if after.is_checkmate() {
        if is_back_rank_mate(&after, m) {
            out.push(hit(Motif::BackRank, format!("{san} is a back-rank mate")));
        } else {
            out.push(hit(Motif::MatingAttack, format!("{san} is checkmate")));
        }
        if let Some(r) = m.promotion() {
            out.push(hit(
                Motif::Promotion,
                format!("{san} promotes to a {} with mate", role_name(r)),
            ));
        }
        return out;
    }

    // Winning an undefended or under-defended piece.
    if let Some(cap) = m.capture() {
        let sq = if m.is_en_passant() {
            None
        } else {
            Some(m.to())
        };
        if cap != Role::Pawn && sq.is_some_and(|s| en_prise(pos, s)) {
            out.push(hit(
                Motif::HangingPiece,
                format!("{san} wins the loose {}", piece_str(cap, m.to())),
            ));
        } else if cap == Role::Pawn
            && sq.is_some_and(|s| {
                pos.board()
                    .attacks_to(s, !mover, pos.board().occupied())
                    .is_empty()
            })
        {
            out.push(hit(
                Motif::LoosePawn,
                format!("{san} picks up an undefended pawn"),
            ));
        }
    }

    // Fork: the moved piece attacks two worthwhile targets and is not simply lost.
    let to = m.to();
    if let Some(mp) = ab.piece_at(to) {
        let targets: Vec<Square> = (ab.attacks_from(to) & ab.by_color(!mover))
            .into_iter()
            .filter(|t| worthy_target(&after, *t, val(mp.role)))
            .collect();
        if targets.len() >= 2 && !en_prise(&after, to) {
            let list: Vec<String> = targets
                .iter()
                .filter_map(|t| ab.role_at(*t).map(|r| piece_str(r, *t)))
                .collect();
            out.push(hit(
                Motif::Fork,
                format!(
                    "{san} forks: the {} attacks the {}",
                    role_name(mp.role),
                    list.join(" and the ")
                ),
            ));
        }
    }

    // Discovered attack: another piece's line opens through the vacated square.
    if let Some(from) = m.from() {
        let occ_before = pos.board().occupied();
        for s in ab.by_color(mover) & ab.sliders() {
            if s == to {
                continue;
            }
            let Some(sp) = ab.piece_at(s) else { continue };
            let before = slider_attacks(s, sp, occ_before);
            let now = slider_attacks(s, sp, ab.occupied()) & ab.by_color(!mover) & !before;
            for t in now {
                if attacks::between(s, t).contains(from) && worthy_target(&after, t, val(sp.role)) {
                    let tr = ab.role_at(t).unwrap_or(Role::Pawn);
                    let what = if tr == Role::King {
                        "discovered check"
                    } else {
                        "discovered attack"
                    };
                    out.push(hit(
                        Motif::DiscoveredAttack,
                        format!(
                            "{san} unmasks a {what}: the {} now hits the {}",
                            piece_str(sp.role, s),
                            piece_str(tr, t)
                        ),
                    ));
                }
            }
        }
    }

    // New pins and skewers.
    let before_lines = line_tactics(pos, mover);
    for lt in line_tactics(&after, mover) {
        let key = |x: &(bool, Square, Square, Square)| (x.0, x.2, x.3);
        if !before_lines.iter().any(|b| key(b) == key(&lt)) {
            out.push(line_hit(&after, mover, lt));
        }
    }

    // Trapping a piece.
    let trapped_before = trapped_pieces(pos, !mover);
    for sq in trapped_pieces(&after, !mover) {
        if !trapped_before.contains(&sq) {
            let r = ab.role_at(sq).unwrap_or(Role::Pawn);
            out.push(hit(
                Motif::TrappedPiece,
                format!("after {san} the {} is trapped", piece_str(r, sq)),
            ));
        }
    }

    // Promotion, or a pawn arriving on the seventh.
    if let Some(r) = m.promotion() {
        out.push(hit(
            Motif::Promotion,
            format!("{san} promotes to a {}", role_name(r)),
        ));
    } else if m.role() == Role::Pawn && to.rank() == mover.relative_rank(Rank::Seventh) {
        out.push(hit(
            Motif::Promotion,
            format!("{san} puts a pawn one step from promotion"),
        ));
    }

    // Mate threat created.
    if mate_threats(pos, mover).is_empty()
        && let Some(t) = mate_threats(&after, mover).first()
    {
        let threat_pos = if after.turn() == mover {
            after.clone()
        } else {
            after.clone().swap_turn().unwrap_or(after.clone())
        };
        let t_san = move_to_san(&threat_pos, *t);
        out.push(hit(
            Motif::MatingAttack,
            format!("{san} threatens mate with {t_san}"),
        ));
    }
    out
}

fn pawn_files(pos: &Chess, color: Color) -> [u8; 8] {
    let mut f = [0u8; 8];
    for sq in pos.board().by_piece(Piece {
        color,
        role: Role::Pawn,
    }) {
        f[sq.file() as usize] += 1;
    }
    f
}

fn is_passed(pos: &Chess, sq: Square, color: Color) -> bool {
    let enemy = pos.board().by_piece(Piece {
        color: !color,
        role: Role::Pawn,
    });
    let file = sq.file() as i32;
    enemy.into_iter().all(|e| {
        let ahead = if color == Color::White {
            e.rank() > sq.rank()
        } else {
            e.rank() < sq.rank()
        };
        !(ahead && (e.file() as i32 - file).abs() <= 1)
    })
}

/// Static motifs in a position: pins and skewers on the board, trapped pieces,
/// mate threats, back-rank and king-shelter weaknesses, pawn structure.
pub fn position_motifs(pos: &Chess) -> Vec<MotifHit> {
    let b = pos.board();
    let mut out = vec![];
    for color in [pos.turn(), !pos.turn()] {
        let by: Side = color.into();
        let name = side_name(color);
        for lt in line_tactics(pos, color) {
            out.push(line_hit(pos, color, lt));
        }
        for sq in trapped_pieces(pos, !color) {
            let r = b.role_at(sq).unwrap_or(Role::Pawn);
            out.push(MotifHit {
                motif: Motif::TrappedPiece,
                by,
                text: format!("{name} has trapped the {}", piece_str(r, sq)),
            });
        }
        if let Some(m) = mate_threats(pos, color).first() {
            let p = if pos.turn() == color {
                pos.clone()
            } else {
                pos.clone().swap_turn().unwrap_or(pos.clone())
            };
            let verb = if pos.turn() == color {
                "has"
            } else {
                "threatens"
            };
            out.push(MotifHit {
                motif: Motif::MatingAttack,
                by,
                text: format!("{name} {verb} mate in one: {}", move_to_san(&p, *m)),
            });
        }
        // Weaknesses of `color`, favouring the other side.
        let opp: Side = (!color).into();
        if let Some(k) = b.king_of(color) {
            let own = b.by_color(color);
            let forward = attacks::king_attacks(k) & !Bitboard::from_rank(k.rank());
            let luft = (attacks::king_attacks(k) & !own)
                .into_iter()
                .any(|s| b.attacks_to(s, !color, b.occupied()).is_empty() && s.rank() != k.rank());
            if k.rank() == color.backrank()
                && forward.any()
                && (forward & own) == forward
                && !luft
                && (b.by_color(!color) & b.rooks_and_queens()).any()
            {
                out.push(MotifHit {
                    motif: Motif::BackRank,
                    by: opp,
                    text: format!("{name}'s king has no escape square off the back rank"),
                });
            }
            let wing = matches!(
                k.file(),
                File::A | File::B | File::C | File::F | File::G | File::H
            );
            if wing && k.rank() == color.backrank() {
                let shield = forward
                    & b.by_piece(Piece {
                        color,
                        role: Role::Pawn,
                    });
                let two = forward
                    .into_iter()
                    .filter_map(|s| s.offset(if color == Color::White { 8 } else { -8 }))
                    .collect::<Bitboard>()
                    & b.by_piece(Piece {
                        color,
                        role: Role::Pawn,
                    });
                if shield.count() + two.count() < 2 {
                    out.push(MotifHit {
                        motif: Motif::KingSafety,
                        by: opp,
                        text: format!(
                            "{name}'s king shelter is thin (fewer than two pawns in front)"
                        ),
                    });
                }
            }
        }
        let files = pawn_files(pos, color);
        for sq in b.by_piece(Piece {
            color,
            role: Role::Pawn,
        }) {
            if is_passed(pos, sq, color) {
                out.push(MotifHit {
                    motif: Motif::PassedPawn,
                    by,
                    text: format!("{name} has a passed pawn on {sq}"),
                });
            }
        }
        let doubled: Vec<String> = (0..8)
            .filter(|f| files[*f] > 1)
            .map(|f| File::new(f as u32).char().to_string())
            .collect();
        let isolated: Vec<String> = (0..8)
            .filter(|f| {
                files[*f] > 0 && (*f == 0 || files[f - 1] == 0) && (*f == 7 || files[f + 1] == 0)
            })
            .map(|f| File::new(f as u32).char().to_string())
            .collect();
        if !doubled.is_empty() {
            out.push(MotifHit {
                motif: Motif::PawnStructure,
                by: opp,
                text: format!("{name} has doubled pawns on the {}-file", doubled.join("/")),
            });
        }
        if !isolated.is_empty() {
            out.push(MotifHit {
                motif: Motif::PawnStructure,
                by: opp,
                text: format!(
                    "{name} has an isolated pawn on the {}-file",
                    isolated.join("/")
                ),
            });
        }
    }
    out
}

/// Motifs created by each move of a SAN line from `pos`, at most `max_plies`.
pub fn line_motifs(pos: &Chess, san_line: &[impl AsRef<str>], max_plies: usize) -> Vec<MotifHit> {
    let mut p = pos.clone();
    let mut out = vec![];
    for s in san_line.iter().take(max_plies) {
        let Some(m) = san_to_move(&p, s.as_ref()) else { break };
        out.extend(move_motifs(&p, m));
        p.play_unchecked(m);
        if p.is_game_over() {
            break;
        }
    }
    out
}

/// The motifs behind a mistake by the side to move in `before`: those the
/// engine's better line would have used (missed) and those the opponent's best
/// reply uses (allowed). Lines are SAN; the refutation starts after `played`.
pub fn mistake_motifs(
    before: &Chess,
    played: Move,
    best_line: &[impl AsRef<str>],
    refutation: &[impl AsRef<str>],
) -> (Vec<Motif>, Vec<Motif>) {
    let mover: Side = before.turn().into();
    let mut after = before.clone();
    after.play_unchecked(played);
    let pick = |hits: Vec<MotifHit>, by: Side| {
        let mut v: Vec<Motif> =
            hits.into_iter().filter(|h| h.by == by && h.motif != Motif::LoosePawn).map(|h| h.motif).collect();
        v.sort();
        v.dedup();
        v
    };
    let missed = pick(line_motifs(before, best_line, 5), mover);
    let opp = if mover == Side::White { Side::Black } else { Side::White };
    // The opponent's reply and their next move: deeper, the line has left the mistake behind.
    let allowed = pick(line_motifs(&after, refutation, 3), opp);
    (missed, allowed)
}

/// Plain-language facts about an engine line (SAN moves from `pos`): the
/// motifs each move creates, then the material outcome. At most `max_plies`.
pub fn line_story(pos: &Chess, san_line: &[impl AsRef<str>], max_plies: usize) -> Vec<String> {
    let start = pos.turn();
    let mut p = pos.clone();
    let mut out = vec![];
    let mut won = [0i32; 2]; // material captured by [start side, other side]
    let mut taken: [Vec<Role>; 2] = [vec![], vec![]];
    for s in san_line.iter().take(max_plies) {
        let Some(m) = san_to_move(&p, s.as_ref()) else {
            break;
        };
        for h in move_motifs(&p, m) {
            out.push(h.text);
        }
        let who = usize::from(p.turn() != start);
        if let Some(c) = m.capture() {
            won[who] += val(c);
            taken[who].push(c);
        }
        if let Some(r) = m.promotion() {
            won[who] += val(r) - 1;
        }
        p.play_unchecked(m);
        if p.is_game_over() {
            break;
        }
    }
    if !p.is_checkmate() {
        let net = won[0] - won[1];
        let names = |v: &[Role]| {
            let mut parts = vec![];
            for r in [Role::Queen, Role::Rook, Role::Bishop, Role::Knight, Role::Pawn] {
                let n = v.iter().filter(|x| **x == r).count();
                match n {
                    0 => {}
                    1 => parts.push(format!("a {}", role_name(r))),
                    n => parts.push(format!("{n} {}s", role_name(r))),
                }
            }
            parts.join(" and ")
        };
        if net != 0 && !(taken[0].is_empty() && taken[1].is_empty()) {
            let (w, l) = if net > 0 { (start, 0) } else { (!start, 1) };
            let gets = names(&taken[l]);
            let gives = names(&taken[1 - l]);
            let tail = if gives.is_empty() {
                String::new()
            } else {
                format!(" for {gives}")
            };
            out.push(format!(
                "net result of the line: {} comes out {} pawn(s) of material ahead (takes {gets}{tail})",
                side_name(w),
                net.abs()
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::{parse_fen, san_to_move, uci_to_move};
    use std::collections::{BTreeMap, BTreeSet};

    fn motifs_of(fen: &str, san: &str) -> Vec<Motif> {
        let p = parse_fen(fen).unwrap();
        let m = san_to_move(&p, san).unwrap();
        move_motifs(&p, m).into_iter().map(|h| h.motif).collect()
    }

    #[test]
    fn knight_fork() {
        // Nc7+ forks king on e8 and rook on a8.
        let m = motifs_of("r3k3/8/8/3N4/8/8/8/4K3 w - - 0 1", "Nc7+");
        assert!(m.contains(&Motif::Fork), "{m:?}");
    }

    #[test]
    fn absolute_pin_and_skewer() {
        let m = motifs_of("4k3/8/2n5/8/8/8/8/4K1B1 w - - 0 1", "Bh2");
        assert!(!m.contains(&Motif::Pin), "{m:?}");
        let m = motifs_of("4k3/3n4/8/8/8/8/8/4KB2 w - - 0 1", "Bb5");
        assert!(m.contains(&Motif::Pin), "{m:?}");
        let m = motifs_of("8/8/8/1q2k3/8/8/8/K5R1 w - - 0 1", "Rg5+");
        assert!(m.contains(&Motif::Skewer), "{m:?}");
    }

    #[test]
    fn back_rank_mate() {
        let m = motifs_of("6k1/5ppp/8/8/8/8/8/3R2K1 w - - 0 1", "Rd8#");
        assert_eq!(m, vec![Motif::BackRank]);
        let p = parse_fen("6k1/5ppp/8/8/8/8/8/3R2K1 b - - 0 1").unwrap();
        assert!(
            position_motifs(&p)
                .iter()
                .any(|h| h.motif == Motif::BackRank)
        );
    }

    #[test]
    fn mistake_motifs_split_missed_and_allowed() {
        // White played Kf2 instead of the fork Nc7+; Black's reply skewers nothing
        // but the rook on a8 escapes. Missed: fork.
        let p = parse_fen("r3k3/8/8/3N4/8/8/8/4K3 w - - 0 1").unwrap();
        let played = san_to_move(&p, "Kf2").unwrap();
        let (missed, allowed) = mistake_motifs(&p, played, &["Nc7+", "Kd7", "Nxa8"], &["Ra5"]);
        assert!(missed.contains(&Motif::Fork), "{missed:?}");
        assert!(allowed.is_empty(), "{allowed:?}");
    }

    #[test]
    fn story_counts_material() {
        let p = parse_fen("r3k3/8/8/3N4/8/8/8/4K3 w - - 0 1").unwrap();
        let s = line_story(&p, &["Nc7+", "Kd7", "Nxa8"], 8);
        assert!(s.iter().any(|t| t.contains("forks")), "{s:?}");
        assert!(
            s.iter().any(|t| t.contains("White comes out 5 pawn(s) of material ahead (takes a rook)")),
            "{s:?}"
        );
    }

    /// Solver-side motifs along a Lichess puzzle's solution.
    fn detected(fen: &str, moves: &str) -> BTreeSet<Motif> {
        let mut p = parse_fen(fen).unwrap();
        let mut ucis = moves.split_whitespace();
        let first = uci_to_move(&p, ucis.next().unwrap()).unwrap();
        p.play_unchecked(first);
        let solver = p.turn();
        let mut set = BTreeSet::new();
        for u in ucis {
            let Some(m) = uci_to_move(&p, u) else { break };
            if p.turn() == solver {
                for h in position_motifs(&p) {
                    if h.by == solver.into() && matches!(h.motif, Motif::Pin | Motif::TrappedPiece)
                    {
                        set.insert(h.motif);
                    }
                }
                set.extend(move_motifs(&p, m).into_iter().map(|h| h.motif));
            }
            p.play_unchecked(m);
        }
        set
    }

    /// Recall against Lichess puzzle themes (the themes are incomplete, so
    /// precision is only reported, not asserted).
    #[test]
    fn lichess_puzzle_recall() {
        let data = include_str!("../../../fixtures/puzzles/sample.csv");
        let map: &[(&str, Motif, f64)] = &[
            ("fork", Motif::Fork, 0.75),
            ("pin", Motif::Pin, 0.75),
            ("skewer", Motif::Skewer, 0.75),
            ("discoveredAttack", Motif::DiscoveredAttack, 0.7),
            ("backRankMate", Motif::BackRank, 0.9),
            ("hangingPiece", Motif::HangingPiece, 0.75),
            ("trappedPiece", Motif::TrappedPiece, 0.6),
            ("promotion", Motif::Promotion, 0.9),
            ("mateIn1", Motif::MatingAttack, 0.9),
        ];
        let mut stats: BTreeMap<&str, (u32, u32, u32)> = BTreeMap::new(); // (tagged, hit, predicted)
        for line in data.lines().skip(1) {
            let cols: Vec<&str> = line.split(',').collect();
            let (fen, moves, themes) = (cols[1], cols[2], cols[4]);
            let themes: BTreeSet<&str> = themes.split_whitespace().collect();
            let got = detected(fen, moves);
            for (theme, motif, _) in map {
                let e = stats.entry(theme).or_default();
                let tagged = themes.contains(theme);
                let found = got.contains(motif)
                    || (*motif == Motif::MatingAttack && got.contains(&Motif::BackRank));
                e.0 += tagged as u32;
                e.1 += (tagged && found) as u32;
                e.2 += found as u32;
            }
        }
        let mut failures = vec![];
        for (theme, _, min) in map {
            let (t, h, p) = stats[theme];
            let recall = h as f64 / t.max(1) as f64;
            let precision = h as f64 / p.max(1) as f64;
            eprintln!("{theme:18} tagged {t:4} recall {recall:.2} precision {precision:.2}");
            if recall < *min {
                failures.push(format!("{theme}: recall {recall:.2} < {min}"));
            }
        }
        assert!(failures.is_empty(), "{failures:?}");
    }
}
