//! Whole-game analysis against real Stockfish (skipped without a binary).

mod common;

use api_types::{Classification, Score, Side};
use chess_core::pgn::parse_pgn;
use coach::analysis::{deep_pass, engine_pass};
use coach::key_moments;
use engine::Limit;
use tokio_util::sync::CancellationToken;

fn table(moves: &[api_types::MoveEval]) -> String {
    moves
        .iter()
        .map(|m| format!("{:>3} {:<7} {:<11} {:?}", m.ply, m.san, format!("{:?}", m.classification), m.score))
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn opera_game() {
    let Some(pool) = common::deterministic_pool().await else { return };
    let game = parse_pgn(&common::fixture("opera.pgn")).unwrap();
    let mut streamed = 0;
    let pass = engine_pass(&pool, &game, Limit::depth(14), &CancellationToken::new(), |_| streamed += 1)
        .await
        .unwrap();
    assert_eq!(streamed, 33);
    assert_eq!(pass.moves.len(), 33);
    let last = pass.moves.last().unwrap();
    assert_eq!(last.san, "Rd8#");
    assert_eq!(last.score, Score::Mate(1), "checkmate is scored in White's favour");
    assert_eq!(last.classification, Classification::Best);
    assert!(pass.moves[0].classification == Classification::Book, "1. e4 is book");
    let (w, b) = (pass.white_accuracy.unwrap(), pass.black_accuracy.unwrap());
    assert!(w > b, "Morphy should be more accurate: {w:.1} vs {b:.1}");
    let black_errors = pass.moves.iter().filter(|m| m.mover == Side::Black && m.classification.is_error()).count();
    assert!(black_errors >= 2, "\n{}", table(&pass.moves));

    let keys = key_moments::select(&pass.moves, Some(Side::Black), "1-0", 6);
    assert!(!keys.is_empty() && keys.len() <= 6);
    let ctx = deep_pass(&pool, &game, &keys[..2.min(keys.len())], 3, Limit::depth(12), &CancellationToken::new())
        .await
        .unwrap();
    assert!(ctx.iter().all(|c| !c.lines_before.is_empty()));
    insta::assert_snapshot!("opera_classification", format!("{}\nkeys: {:?}\naccuracy: {w:.1} / {b:.1}", table(&pass.moves), keys));
}

#[tokio::test]
async fn kasparov_topalov() {
    let Some(pool) = common::deterministic_pool().await else { return };
    let game = parse_pgn(&common::fixture("kasparov_topalov_1999.pgn")).unwrap();
    assert_eq!(game.moves.len(), 87);
    let pass = engine_pass(&pool, &game, Limit::depth(14), &CancellationToken::new(), |_| {})
        .await
        .unwrap();
    let rxd4 = pass.moves.iter().find(|m| m.ply == 47).unwrap();
    assert_eq!(rxd4.san, "Rxd4");
    assert!(
        !matches!(rxd4.classification, Classification::Mistake | Classification::Blunder),
        "24. Rxd4 is Kasparov's famous winning sacrifice: {:?}",
        rxd4.classification
    );
    assert!(pass.moves.last().unwrap().score.cp().is_none_or(|c| c > 300), "White is winning at the end");
    insta::assert_snapshot!("kasparov_topalov_classification", table(&pass.moves));
}

#[tokio::test]
async fn cancellation_stops_the_pass() {
    let Some(pool) = common::deterministic_pool().await else { return };
    let game = parse_pgn(&common::fixture("kasparov_topalov_1999.pgn")).unwrap();
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        c2.cancel();
    });
    let r = engine_pass(&pool, &game, Limit::depth(30), &cancel, |_| {}).await;
    assert!(matches!(r, Err(coach::analysis::AnalysisError::Cancelled)));
}
