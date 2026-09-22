//! Opening book built from lichess-org/chess-openings (CC0). Every position
//! reached along a named line is "book"; the final position of each line
//! carries its ECO code and name.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use shakmaty::{Chess, Position};

use crate::position::{fen_key, san_to_move};

const TSV: [&str; 5] = [
    include_str!("../data/openings/a.tsv"),
    include_str!("../data/openings/b.tsv"),
    include_str!("../data/openings/c.tsv"),
    include_str!("../data/openings/d.tsv"),
    include_str!("../data/openings/e.tsv"),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Opening {
    pub eco: String,
    pub name: String,
}

pub struct Book {
    positions: HashSet<String>,
    names: HashMap<String, Opening>,
}

pub static BOOK: LazyLock<Book> = LazyLock::new(Book::build);

impl Book {
    fn build() -> Book {
        let mut positions = HashSet::new();
        let mut names = HashMap::new();
        for file in TSV {
            for line in file.lines().skip(1) {
                let mut cols = line.split('\t');
                let (Some(eco), Some(name), Some(pgn)) = (cols.next(), cols.next(), cols.next()) else {
                    continue;
                };
                let mut pos = Chess::default();
                let mut ok = true;
                for tok in pgn.split_whitespace() {
                    if tok.ends_with('.') || tok.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                        continue;
                    }
                    match san_to_move(&pos, tok) {
                        Some(m) => {
                            pos.play_unchecked(m);
                            positions.insert(fen_key(&pos));
                        }
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    // Later (more specific) entries for the same position win.
                    names.insert(fen_key(&pos), Opening { eco: eco.into(), name: name.into() });
                }
            }
        }
        Book { positions, names }
    }

    pub fn is_book(&self, key: &str) -> bool {
        self.positions.contains(key)
    }

    pub fn name(&self, key: &str) -> Option<&Opening> {
        self.names.get(key)
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// For a sequence of position keys after each ply, returns which plies are
/// book (a ply is book only while every earlier ply was book too) and the
/// deepest named opening reached.
pub fn book_prefix(keys_after: &[String]) -> (Vec<bool>, Option<Opening>) {
    let mut flags = Vec::with_capacity(keys_after.len());
    let mut still = true;
    let mut opening = None;
    for k in keys_after {
        still = still && BOOK.is_book(k);
        flags.push(still);
        if let Some(o) = BOOK.name(k) {
            opening = Some(o.clone());
        }
    }
    (flags, opening)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::{START_FEN, parse_fen, play_sans};

    #[test]
    fn loads_all_lines() {
        assert!(BOOK.len() > 3000, "{}", BOOK.len());
    }

    #[test]
    fn ruy_lopez() {
        let start = parse_fen(START_FEN).unwrap();
        let mut keys = vec![];
        let mut pos = start.clone();
        for s in ["e4", "e5", "Nf3", "Nc6", "Bb5", "a6", "Qe2"] {
            let (p, _) = play_sans(&pos, &[s]).unwrap();
            keys.push(fen_key(&p));
            pos = p;
        }
        let (flags, opening) = book_prefix(&keys);
        assert!(flags[..6].iter().all(|b| *b));
        let o = opening.unwrap();
        assert!(o.name.starts_with("Ruy Lopez"), "{o:?}");
        assert!(o.eco.starts_with('C'));
    }
}
