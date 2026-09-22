//! Integration tests against a real Stockfish. Skipped (with a message) when
//! no binary is found; run `cargo xtask fetch-stockfish` first.

use std::path::Path;
use std::time::{Duration, Instant};

use chess_core::Score;
use engine::{AnalysisRequest, EngineConfig, EngineError, EnginePool, Limit, Priority, Terminal, find_stockfish};

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap()
}

async fn pool(workers: usize) -> Option<EnginePool> {
    let Some(path) = find_stockfish(root()) else {
        eprintln!("SKIP: no Stockfish binary found");
        return None;
    };
    let mut cfg = EngineConfig::with_defaults(path);
    cfg.workers = workers;
    cfg.threads = 2;
    cfg.hash_mb = 64;
    Some(EnginePool::start(cfg, None).await.expect("pool starts"))
}

fn req(fen: &str, multipv: u32, limit: Limit) -> AnalysisRequest {
    AnalysisRequest { fen: fen.into(), multipv, limit, priority: Priority::Interactive }
}

const MATE_IN_2: &str = "r2qkb1r/pp2nppp/3p4/2pNN1B1/2BnP3/3P4/PPP2PPP/R2bK2R w KQkq - 1 10";

#[tokio::test]
async fn mate_in_two() {
    let Some(p) = pool(1).await else { return };
    assert!(p.engine_name().starts_with("Stockfish"), "{}", p.engine_name());
    let a = p.analyse(req(MATE_IN_2, 1, Limit::depth(14))).await.unwrap();
    assert_eq!(a.score(), Some(Score::Mate(2)));
    assert_eq!(a.lines[0].pv[0], "d5f6");
    assert_eq!(a.best_move.as_deref(), Some("d5f6"));
    assert!(a.done);
}

#[tokio::test]
async fn mate_in_one_and_black_pov() {
    let Some(p) = pool(1).await else { return };
    let a = p.analyse(req("6k1/5ppp/8/8/8/8/5PPP/3R2K1 w - - 0 1", 1, Limit::depth(8))).await.unwrap();
    assert_eq!(a.score(), Some(Score::Mate(1)));
    assert_eq!(a.lines[0].pv[0], "d1d8");
    // Same idea with Black to move: Black mates, so White-POV score is negative.
    let a = p.analyse(req("3r2k1/5ppp/8/8/8/8/5PPP/6K1 b - - 0 1", 1, Limit::depth(8))).await.unwrap();
    assert_eq!(a.score(), Some(Score::Mate(-1)));
    let wdl = a.lines[0].wdl.unwrap();
    assert!(wdl[2] > 900, "black winning from white POV means loss share high: {wdl:?}");
}

#[tokio::test]
async fn multipv_lines_are_ordered() {
    let Some(p) = pool(1).await else { return };
    let a = p
        .analyse(req("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1", 3, Limit::depth(12)))
        .await
        .unwrap();
    assert_eq!(a.lines.len(), 3);
    assert_eq!(a.lines.iter().map(|l| l.rank).collect::<Vec<_>>(), vec![1, 2, 3]);
    let first = engine::process::score_order(a.lines[0].score);
    let third = engine::process::score_order(a.lines[2].score);
    assert!(first >= third - 30, "{:?}", a.lines);
}

#[tokio::test]
async fn terminal_positions_skip_engine() {
    let Some(p) = pool(1).await else { return };
    // Fool's mate final position: White is checkmated.
    let a = p
        .analyse(req("rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3", 1, Limit::depth(10)))
        .await
        .unwrap();
    assert_eq!(a.terminal, Some(Terminal::Checkmate));
    assert!(a.lines.is_empty());
}

#[tokio::test]
async fn cancellation_leaves_worker_reusable() {
    let Some(p) = pool(1).await else { return };
    let handle = p
        .analyse_stream(req("r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3", 2, Limit::depth(60)))
        .await
        .unwrap();
    let mut handle = handle;
    // Wait for at least one partial result, then cancel by dropping.
    let first = tokio::time::timeout(Duration::from_secs(10), handle.updates.recv()).await.unwrap();
    assert!(first.is_some());
    let started = Instant::now();
    drop(handle);
    // The single worker must pick up the next job promptly.
    let a = p.analyse(req(MATE_IN_2, 1, Limit::depth(10))).await.unwrap();
    assert_eq!(a.score(), Some(Score::Mate(2)));
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
}

#[tokio::test]
async fn cache_hit_is_instant() {
    let Some(p) = pool(1).await else { return };
    let fen = "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4";
    let a = p.analyse(req(fen, 2, Limit::depth(16))).await.unwrap();
    let t = Instant::now();
    let b = p.analyse(req(fen, 1, Limit::depth(12))).await.unwrap();
    assert!(t.elapsed() < Duration::from_millis(20));
    assert_eq!(b.lines.len(), 1);
    assert_eq!(a.lines[0], b.lines[0]);
    // Scholar's mate threat: Qxf7#
    assert_eq!(a.score(), Some(Score::Mate(1)));
}

#[tokio::test]
async fn bad_fen_is_rejected() {
    let Some(p) = pool(1).await else { return };
    let e = p.analyse(req("not a fen", 1, Limit::depth(5))).await.unwrap_err();
    assert!(matches!(e, EngineError::BadFen(_)));
}
