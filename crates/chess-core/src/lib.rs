//! Pure chess logic for chessgpt: scores, win%, Lichess-exact move judgements,
//! accuracy, PGN parsing, opening book, and position helpers. No I/O.

pub mod accuracy;
pub mod book;
pub mod classify;
pub mod motifs;
pub mod pgn;
pub mod position;
pub mod rating;
pub mod score;
pub mod srs;
pub mod winpct;

pub use classify::{Classification, Judgement};
pub use score::Score;
