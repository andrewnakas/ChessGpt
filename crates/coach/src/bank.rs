//! A curated slice of the Lichess puzzle database (CC0), grouped by theme and
//! rating: the drill sets' source of fresh positions. `chessgpt-lab bank`
//! builds `data/drill_bank.csv` from the full database.

use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

use chess_core::motifs::Motif;
use chess_core::position::{parse_fen, to_fen, uci_to_move};
use shakmaty::Position;

/// Lichess themes kept in the bank, with the motif the detectors must find
/// along the solution for a puzzle to count (none: trust the Lichess tag).
pub const BANK_THEMES: &[(&str, Option<Motif>)] = &[
    ("hangingPiece", Some(Motif::HangingPiece)),
    ("fork", Some(Motif::Fork)),
    ("pin", Some(Motif::Pin)),
    ("skewer", Some(Motif::Skewer)),
    ("discoveredAttack", Some(Motif::DiscoveredAttack)),
    ("backRankMate", Some(Motif::BackRank)),
    ("mateIn1", Some(Motif::MatingAttack)),
    ("mateIn2", Some(Motif::MatingAttack)),
    ("trappedPiece", Some(Motif::TrappedPiece)),
    ("promotion", Some(Motif::Promotion)),
    ("exposedKing", None),
    ("kingsideAttack", None),
    ("advancedPawn", None),
    ("deflection", None),
    ("attraction", None),
    ("defensiveMove", None),
    ("rookEndgame", None),
    ("pawnEndgame", None),
];

/// Concept tags that can be drilled, and the Lichess themes that train them.
pub const TECHNIQUES: &[(&str, &[&str])] = &[
    ("hanging_piece", &["hangingPiece"]),
    ("fork", &["fork"]),
    ("pin", &["pin"]),
    ("skewer", &["skewer"]),
    ("discovered_attack", &["discoveredAttack"]),
    ("back_rank", &["backRankMate"]),
    ("mating_attack", &["mateIn2", "mateIn1"]),
    ("trapped_piece", &["trappedPiece"]),
    ("promotion", &["promotion"]),
    ("king_safety", &["exposedKing", "kingsideAttack"]),
    ("passed_pawn", &["advancedPawn"]),
    ("deflection", &["deflection"]),
    ("decoy", &["attraction"]),
    ("endgame_technique", &["rookEndgame", "pawnEndgame"]),
];

/// Lichess themes for a concept tag, if it can be drilled.
pub fn themes_for(tag: &str) -> Option<&'static [&'static str]> {
    TECHNIQUES.iter().find(|(t, _)| *t == tag).map(|(_, th)| *th)
}

/// The motif a technique is about, when the detectors know it.
pub fn motif_for(tag: &str) -> Option<Motif> {
    let themes = themes_for(tag)?;
    BANK_THEMES.iter().find(|(t, _)| *t == themes[0]).and_then(|(_, m)| *m)
}

/// Do the motifs found along a solution back up a Lichess theme? Themes the
/// detectors don't cover are taken on trust.
pub fn confirms(theme: &str, found: &[Motif]) -> bool {
    match BANK_THEMES.iter().find(|(t, _)| *t == theme) {
        None => false,
        Some((_, None)) => true,
        Some((_, Some(m))) => {
            found.contains(m) || (*m == Motif::MatingAttack && found.contains(&Motif::BackRank))
        }
    }
}

/// Lichess puzzle ratings run above game ratings for the same player.
pub const PUZZLE_RATING_OFFSET: i32 = 200;

#[derive(Debug, Clone, PartialEq)]
pub struct BankPuzzle {
    pub id: String,
    /// The position before the opponent's move that sets up the tactic.
    pub fen: String,
    /// The opponent's move, then the solution (UCI).
    pub moves: Vec<String>,
    pub rating: i32,
    pub themes: Vec<String>,
}

impl BankPuzzle {
    /// The position the solver faces and the solution from it.
    pub fn solving(&self) -> Option<(String, Vec<String>)> {
        let mut p = parse_fen(&self.fen).ok()?;
        let m = uci_to_move(&p, self.moves.first()?)?;
        p.play_unchecked(m);
        Some((to_fen(&p), self.moves[1..].to_vec()))
    }

    /// One CSV row: `PuzzleId,FEN,Moves,Rating,Themes`.
    pub fn parse_row(line: &str) -> Option<BankPuzzle> {
        let c: Vec<&str> = line.trim_end().split(',').collect();
        if c.len() < 5 {
            return None;
        }
        Some(BankPuzzle {
            id: c[0].into(),
            fen: c[1].into(),
            moves: c[2].split_whitespace().map(String::from).collect(),
            rating: c[3].parse().ok()?,
            themes: c[4].split_whitespace().map(String::from).collect(),
        })
    }

    pub fn to_row(&self) -> String {
        format!("{},{},{},{},{}", self.id, self.fen, self.moves.join(" "), self.rating, self.themes.join(" "))
    }
}

pub struct Bank {
    /// Lichess theme -> puzzles sorted by rating.
    by_theme: BTreeMap<String, Vec<BankPuzzle>>,
}

impl Bank {
    pub fn parse(csv: &str) -> Bank {
        let mut by_theme: BTreeMap<String, Vec<BankPuzzle>> = BTreeMap::new();
        for p in csv.lines().skip(1).filter_map(BankPuzzle::parse_row) {
            for t in &p.themes {
                if BANK_THEMES.iter().any(|(b, _)| b == t) {
                    by_theme.entry(t.clone()).or_default().push(p.clone());
                }
            }
        }
        for v in by_theme.values_mut() {
            v.sort_by_key(|p| p.rating);
        }
        Bank { by_theme }
    }

    /// The bank shipped with the binary.
    pub fn builtin() -> &'static Bank {
        static BANK: OnceLock<Bank> = OnceLock::new();
        BANK.get_or_init(|| Bank::parse(include_str!("../data/drill_bank.csv")))
    }

    pub fn len(&self, theme: &str) -> usize {
        self.by_theme.get(theme).map_or(0, Vec::len)
    }

    /// A puzzle on any of `themes` near `rating` (puzzle scale), skipping
    /// `exclude`d ids. `seed` picks among the ~8 closest so repeated sets vary.
    pub fn pick(&self, themes: &[&str], rating: i32, exclude: &HashSet<String>, seed: u64) -> Option<&BankPuzzle> {
        let mut near: Vec<&BankPuzzle> = themes
            .iter()
            .filter_map(|t| self.by_theme.get(*t))
            .flatten()
            .filter(|p| !exclude.contains(&p.id))
            .collect();
        near.sort_by_key(|p| ((p.rating - rating).abs(), p.id.clone()));
        near.dedup_by(|a, b| a.id == b.id);
        near.truncate(8);
        if near.is_empty() {
            return None;
        }
        Some(near[(mix(seed) % near.len() as u64) as usize])
    }
}

/// SplitMix64: cheap, deterministic scrambling of a seed.
pub fn mix(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_bank_covers_every_technique() {
        let bank = Bank::builtin();
        for (tag, themes) in TECHNIQUES {
            let n: usize = themes.iter().map(|t| bank.len(t)).sum();
            assert!(n >= 50, "{tag}: only {n} puzzles");
        }
    }

    #[test]
    fn rows_round_trip_and_solve() {
        let row = "00008,r6k/pp2r2p/4Rp1Q/3p4/8/1N1P2R1/PqP2bPP/7K b - - 0 24,f2g3 e6e7 b2b1 b3c1 b1c1 h6c1,1797,crushing hangingPiece";
        let p = BankPuzzle::parse_row(row).unwrap();
        assert_eq!(p.to_row(), row);
        let (fen, sol) = p.solving().unwrap();
        assert!(fen.starts_with("r6k/pp2r2p/4Rp1Q/3p4/8/1N1P2b1/PqP3PP/7K w"), "{fen}");
        assert_eq!(sol[0], "e6e7");
    }

    #[test]
    fn pick_prefers_near_ratings_and_skips_excluded() {
        let bank = Bank::builtin();
        let p = bank.pick(&["fork"], 1500, &HashSet::new(), 1).unwrap();
        assert!((p.rating - 1500).abs() < 300, "{}", p.rating);
        let ex: HashSet<String> = [p.id.clone()].into();
        for seed in 0..20 {
            assert_ne!(bank.pick(&["fork"], 1500, &ex, seed).unwrap().id, p.id);
        }
    }

    #[test]
    fn motif_for_known_techniques() {
        assert_eq!(motif_for("fork"), Some(Motif::Fork));
        assert_eq!(motif_for("mating_attack"), Some(Motif::MatingAttack));
        assert_eq!(motif_for("king_safety"), None);
        assert_eq!(motif_for("tempo"), None);
    }
}
