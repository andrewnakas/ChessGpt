//! The fixed vocabulary of concept tags. Explanations may only use these, so
//! the mistake index can be queried by theme ("show me my fork mistakes").

pub const CONCEPT_TAGS: &[(&str, &str)] = &[
    // tactics
    ("hanging_piece", "a piece left undefended or insufficiently defended"),
    ("fork", "one piece attacks two or more targets"),
    ("pin", "a piece cannot move without exposing something more valuable"),
    ("skewer", "a valuable piece is attacked and must move, exposing another"),
    ("discovered_attack", "moving one piece unveils an attack by another"),
    ("double_attack", "two threats at once"),
    ("back_rank", "mate or threats on the first/last rank"),
    ("mating_attack", "a direct attack on the king leading to mate"),
    ("overloaded_defender", "a defender with too many duties"),
    ("deflection", "luring a defender away from its task"),
    ("decoy", "luring a piece onto a bad square"),
    ("removing_the_defender", "capturing or chasing away a key defender"),
    ("trapped_piece", "a piece with no safe squares"),
    ("zwischenzug", "an in-between move before the expected recapture"),
    ("x_ray", "attacking or defending through another piece"),
    ("promotion", "pawn promotion tactics"),
    ("missed_tactic", "a tactical opportunity that was not taken"),
    ("calculation", "a concrete line was miscalculated"),
    // strategy
    ("king_safety", "exposed king, weakened shelter"),
    ("pawn_structure", "pawn weaknesses, chains, islands, breaks"),
    ("passed_pawn", "creating, supporting or stopping a passed pawn"),
    ("weak_squares", "holes and outposts"),
    ("open_file", "control of open or half-open files"),
    ("piece_activity", "active versus passive pieces"),
    ("bishop_pair", "the two bishops"),
    ("bad_bishop", "a bishop blocked by its own pawns"),
    ("space", "space advantage and cramped positions"),
    ("initiative", "keeping or losing the initiative"),
    ("prophylaxis", "stopping the opponent's plan before it starts"),
    ("counterplay", "creating or allowing counter-chances"),
    ("material_imbalance", "unequal material, exchanges"),
    ("simplification", "trading into a won or drawn ending"),
    // opening
    ("development", "bringing pieces into play"),
    ("center_control", "fighting for the central squares"),
    ("castling", "castling timing and safety"),
    ("opening_principles", "general opening rules"),
    ("tempo", "gaining or losing time"),
    // endgame
    ("endgame_technique", "converting or holding an ending"),
    ("king_activity", "using the king in the endgame"),
    ("opposition", "king opposition and key squares"),
    // practical
    ("greed", "grabbing material at too high a cost"),
    ("time_management", "clock handling"),
];

pub fn tag_names() -> Vec<&'static str> {
    CONCEPT_TAGS.iter().map(|(t, _)| *t).collect()
}

pub fn is_tag(s: &str) -> bool {
    CONCEPT_TAGS.iter().any(|(t, _)| *t == s)
}

/// Human label for a tag, e.g. "hanging_piece" -> "Hanging piece".
pub fn label(tag: &str) -> String {
    let s = tag.replace('_', " ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => s,
    }
}
