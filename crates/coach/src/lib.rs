//! The coach: whole-game engine analysis, key-moment selection, LLM
//! explanations with move verification, and the tool-using chat.

pub mod analysis;
pub mod bands;
pub mod chat;
pub mod explain;
pub mod key_moments;
pub mod prompts;
pub mod puzzles;
pub mod tags;
pub mod tools;
pub mod verify;

/// A move's inputs to the rating model.
pub fn rating_stat(m: &api_types::MoveEval) -> chess_core::rating::MoveStat {
    chess_core::rating::MoveStat {
        win_before: m.win_before,
        win_after: m.win_after,
        accuracy: m.accuracy,
        judgement: m.lichess_judgement,
        phase: m.phase,
    }
}
