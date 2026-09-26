//! Drill sets against real Stockfish (skipped without a binary).

mod common;

use std::collections::HashSet;

use api_types::DrillKind;
use chess_core::position::{parse_fen, uci_to_move};
use coach::bank::Bank;
use coach::drills::{DrillRequest, Seed, compose};
use shakmaty::Position;

#[tokio::test]
async fn fork_set_from_own_position() {
    let Some(pool) = common::deterministic_pool().await else { return };
    let req = DrillRequest {
        technique: "fork".into(),
        rating: 1200,
        seed: Some(Seed { fen: "r3k3/8/8/3N4/8/8/5PP1/6K1 w - - 0 1".into(), solution_uci: vec!["d5c7".into()], allowed: false }),
        exclude: HashSet::new(),
        rng: 7,
    };
    let items = compose(&pool, Bank::builtin(), &req).await.unwrap();
    let kinds: Vec<DrillKind> = items.iter().map(|d| d.item.kind).collect();
    eprintln!("{kinds:?}");
    let count = |k| kinds.iter().filter(|x| **x == k).count();
    assert_eq!(count(DrillKind::Spot), 2);
    assert_eq!(count(DrillKind::Find), 4);
    assert_eq!(count(DrillKind::Defend), 2);
    assert_eq!(count(DrillKind::Playout), 1);
    let po = items.iter().find(|d| d.item.kind == DrillKind::Playout).unwrap();
    let goal = po.item.goal.as_ref().unwrap();
    assert!(goal.start_win_pct >= 75.0 && goal.min_win_pct < goal.start_win_pct, "{goal:?}");
    let variants: Vec<_> = items.iter().filter(|d| d.item.origin == "variant").collect();
    assert_eq!(variants.len(), 2);
    for v in &variants {
        assert_ne!(v.item.fen, req.seed.as_ref().unwrap().fen);
        assert!(v.item.line_san[0].starts_with('N'), "{:?}", v.item.line_san);
    }
    let ids: HashSet<_> = items.iter().filter_map(|d| d.bank_id.clone()).collect();
    assert_eq!(ids.len(), items.iter().filter(|d| d.bank_id.is_some()).count(), "a bank puzzle is used twice");
    for d in items.iter().filter(|d| d.item.kind == DrillKind::Defend) {
        let pos = parse_fen(&d.item.fen).unwrap();
        assert!(!d.item.accept_uci.is_empty());
        for u in &d.item.accept_uci {
            assert!(uci_to_move(&pos, u).is_some(), "{u} illegal in {}", d.item.fen);
        }
        assert!(d.item.accept_uci.len() < pos.legal_moves().len());
        assert!(d.item.trap_san.len() >= 2);
    }
}

#[tokio::test]
async fn technique_without_seed_uses_the_bank() {
    let Some(pool) = common::deterministic_pool().await else { return };
    let req = DrillRequest { technique: "back_rank".into(), rating: 900, seed: None, exclude: HashSet::new(), rng: 1 };
    let items = compose(&pool, Bank::builtin(), &req).await.unwrap();
    assert!(items.len() >= 7, "{}", items.len());
    assert!(items.iter().all(|d| d.bank_id.is_some()));
}

#[tokio::test]
async fn walked_into_it_becomes_own_defence() {
    let Some(pool) = common::deterministic_pool().await else { return };
    // Black played ...Qd7??, walking into Nf6+ forking king and queen (…Qxd5 wins a knight).
    let req = DrillRequest {
        technique: "fork".into(),
        rating: 1200,
        seed: Some(Seed {
            fen: "3qk3/8/8/3N4/8/8/5PP1/6K1 b - - 0 1".into(),
            solution_uci: vec!["d8d7".into()],
            allowed: true,
        }),
        exclude: HashSet::new(),
        rng: 3,
    };
    let items = compose(&pool, Bank::builtin(), &req).await.unwrap();
    let own: Vec<_> = items.iter().filter(|d| d.item.origin == "variant").collect();
    assert!(!own.is_empty(), "{:?}", items.iter().map(|d| (&d.item.kind, &d.item.origin)).collect::<Vec<_>>());
    for d in own {
        assert_eq!(d.item.kind, DrillKind::Defend);
        assert!(d.item.prompt.starts_with("Your own position"));
        assert!(d.item.trap_san.len() >= 2, "{:?}", d.item.trap_san);
    }
}
