//! Our Lichess judgements, fed Lichess's own evals, must reproduce Lichess's
//! own ?!/?/?? annotations exactly. Fixtures: `fixtures/lichess/*.pgn`,
//! exported with `?evals=true&literate=true` from analysed games.

use chess_core::classify::{Judgement, lichess_judgement};
use chess_core::pgn::parse_pgn;

fn nag_to_judgement(nag: Option<u8>) -> Option<Judgement> {
    match nag {
        Some(6) => Some(Judgement::Inaccuracy),
        Some(2) => Some(Judgement::Mistake),
        Some(4) => Some(Judgement::Blunder),
        _ => None,
    }
}

#[test]
fn judgements_match_lichess_annotations() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/lichess");
    let mut checked = 0;
    let mut annotated = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "pgn") {
            continue;
        }
        let game = parse_pgn(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for w in game.moves.windows(2) {
            let (prev, cur) = (&w[0], &w[1]);
            let (Some(pe), Some(ce)) = (prev.pgn_eval, cur.pgn_eval) else { continue };
            let ours = lichess_judgement(pe, ce, cur.mover.into());
            let theirs = nag_to_judgement(cur.nag);
            assert_eq!(ours, theirs, "{} ply {} {}: evals {:?} -> {:?}", path.display(), cur.ply, cur.san, pe, ce);
            checked += 1;
            annotated += theirs.is_some() as usize;
        }
    }
    assert!(checked > 50, "only {checked} moves checked");
    assert!(annotated >= 10, "only {annotated} annotated moves");
}
