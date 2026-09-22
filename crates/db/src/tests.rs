use api_types::*;
use chess_core::pgn::parse_pgn;
use engine::{Analysis, AnalysisStore, PvLine};

use crate::{Db, NewAnalysis, NewGame, now_ms};

const OPERA: &str = "[White \"Paul Morphy\"]\n[Black \"Duke Karl / Count Isouard\"]\n[Result \"1-0\"]\n\n1. e4 e5 2. Nf3 d6 3. d4 Bg4 4. dxe5 Bxf3 5. Qxf3 dxe5 6. Bc4 Nf6 7. Qb3 Qe7 8. Nc3 c6 9. Bg5 b5 10. Nxb5 cxb5 11. Bxb5+ Nbd7 12. O-O-O Rd8 13. Rxd7 Rxd7 14. Rd1 Qe6 15. Bxd7+ Nxd7 16. Qb8+ Nxb8 17. Rd8# 1-0";

async fn opera(db: &Db) -> GameSummary {
    let parsed = parse_pgn(OPERA).unwrap();
    let (s, inserted) = db
        .insert_game(NewGame { source: GameSource::Pgn, source_id: Some("opera".into()), pgn: OPERA.into(), parsed })
        .await
        .unwrap();
    assert!(inserted);
    s
}

#[tokio::test]
async fn settings_roundtrip() {
    let db = Db::open_memory().await.unwrap();
    let s = db.settings().await.unwrap();
    assert_eq!(s.elo, 1500);
    let s = db
        .put_settings(&SettingsInput { elo: 1850, lichess_username: Some(" Morph ".into()), chesscom_username: None, explorer_enabled: false, lichess_token: Some("lip_secret".into()) })
        .await
        .unwrap();
    assert_eq!(s.elo, 1850);
    assert!(s.has_lichess_token);
    assert_eq!(s.lichess_username.as_deref(), Some("Morph"));
    assert_eq!(db.lichess_token().await.unwrap().as_deref(), Some("lip_secret"));
    let s = db
        .put_settings(&SettingsInput { elo: 1850, lichess_username: None, chesscom_username: None, explorer_enabled: false, lichess_token: None })
        .await
        .unwrap();
    assert!(s.has_lichess_token, "None keeps the token");
    assert!(!s.explorer_enabled);
    assert_eq!(s.lichess_username, None);
}

#[tokio::test]
async fn providers_encrypt_keys() {
    let db = Db::open_memory().await.unwrap();
    let p = db
        .create_provider(&ProviderInput {
            kind: ProviderKind::Anthropic,
            label: None,
            base_url: None,
            model: None,
            api_key: Some("sk-ant-test".into()),
            is_default: false,
        })
        .await
        .unwrap();
    assert!(p.is_default, "first provider becomes default");
    assert!(p.has_key);
    assert_eq!(p.model, "claude-opus-5");
    let raw: String = sqlx::query_scalar("SELECT api_key_enc FROM providers").fetch_one(db.pool()).await.unwrap();
    assert!(!raw.contains("sk-ant"));
    assert_eq!(db.provider(&p.id).await.unwrap().api_key.as_deref(), Some("sk-ant-test"));
    let dbg = format!("{:?}", db.provider(&p.id).await.unwrap());
    assert!(!dbg.contains("sk-ant"));

    let o = db
        .create_provider(&ProviderInput {
            kind: ProviderKind::Ollama,
            label: None,
            base_url: None,
            model: Some("llama3.1:8b".into()),
            api_key: None,
            is_default: true,
        })
        .await
        .unwrap();
    assert!(o.is_default);
    assert_eq!(db.default_provider().await.unwrap().unwrap().provider.id, o.id);
    let updated = db
        .update_provider(&p.id, &ProviderInput { api_key: None, ..ProviderInput {
            kind: ProviderKind::Anthropic, label: Some("Claude".into()), base_url: None, model: Some("claude-sonnet-5".into()), api_key: None, is_default: false } })
        .await
        .unwrap();
    assert!(updated.has_key, "None keeps the stored key");
    assert_eq!(updated.model, "claude-sonnet-5");
}

#[tokio::test]
async fn games_import_is_idempotent() {
    let db = Db::open_memory().await.unwrap();
    db.put_settings(&SettingsInput { elo: 1500, lichess_username: Some("paul morphy".into()), chesscom_username: None, explorer_enabled: true, lichess_token: None })
        .await
        .unwrap();
    let s = opera(&db).await;
    assert_eq!(s.ply_count, 33);
    assert_eq!(s.user_side, Some(Side::White));
    assert_eq!(s.result, "1-0");
    assert!(s.opening.as_deref().unwrap_or("").contains("Philidor"), "{:?}", s.opening);
    let parsed = parse_pgn(OPERA).unwrap();
    let (again, inserted) = db
        .insert_game(NewGame { source: GameSource::Pgn, source_id: Some("opera".into()), pgn: OPERA.into(), parsed })
        .await
        .unwrap();
    assert!(!inserted);
    assert_eq!(again.id, s.id);
    let g = db.game(&s.id).await.unwrap();
    assert_eq!(g.parsed.moves.len(), 33);
    assert_eq!(db.list_games(10, 0).await.unwrap().len(), 1);
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM game_moves").fetch_one(db.pool()).await.unwrap();
    assert_eq!(n, 33);
}

#[tokio::test]
async fn engine_cache_keeps_deepest() {
    let db = Db::open_memory().await.unwrap();
    let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
    let key = chess_core::position::fen_key_of(fen).unwrap();
    let mk = |depth| Analysis {
        fen: fen.into(),
        depth,
        multipv: 2,
        lines: vec![PvLine { rank: 1, depth, score: Score::Cp(30), wdl: None, pv: vec!["e7e5".into()] }],
        nodes: 1,
        nps: 1,
        time_ms: 1,
        best_move: Some("e7e5".into()),
        terminal: None,
        done: true,
        engine: "Stockfish 19".into(),
    };
    db.put(&key, &mk(20)).await;
    db.put(&key, &mk(12)).await;
    let got = db.get("Stockfish 19", &key, 1, 18).await.unwrap();
    assert_eq!(got.depth, 20);
    assert!(db.get("Stockfish 19", &key, 3, 10).await.is_none(), "multipv 3 not stored");
    assert!(db.get("Stockfish 18", &key, 1, 10).await.is_none());
}

#[tokio::test]
async fn analysis_lifecycle_and_mistake_index() {
    let db = Db::open_memory().await.unwrap();
    let g = opera(&db).await;
    let id = db
        .create_analysis(&NewAnalysis {
            game_id: g.id.clone(),
            elo: 1400,
            tier: EloTier::Intermediate,
            user_side: Some(Side::Black),
            engine: "Stockfish 19".into(),
        })
        .await
        .unwrap();
    db.set_analysis_status(&id, JobStatus::Running, None).await.unwrap();
    let game = db.game(&g.id).await.unwrap();
    for pm in &game.parsed.moves {
        let bad = pm.ply == 8; // 4... Bxf3 by Black
        db.put_move_eval(
            &id,
            &MoveEval {
                ply: pm.ply,
                mover: pm.mover,
                san: pm.san.clone(),
                uci: pm.uci.clone(),
                score: Score::Cp(50),
                depth: 18,
                best_uci: None,
                best_san: None,
                best_line_san: vec![],
                classification: if bad { Classification::Mistake } else { Classification::Good },
                lichess_judgement: bad.then_some(Judgement::Mistake),
                win_before: 50.0,
                win_after: 45.0,
                delta_wc: if bad { 0.25 } else { 0.0 },
                accuracy: 90.0,
                phase: Phase::Opening,
                is_key_moment: false,
            },
        )
        .await
        .unwrap();
    }
    db.set_key_moments(&id, &[8, 20]).await.unwrap();
    db.set_accuracy(&id, Some(95.0), Some(60.0)).await.unwrap();
    assert_eq!(db.index_mistakes(&id).await.unwrap(), 1);
    let e = Explanation {
        ply: 8,
        headline: "Giving up the bishop pair".into(),
        why_it_matters: "x".into(),
        better_move: None,
        concept_tags: vec!["bishop_pair".into(), "development".into()],
        takeaway: "x".into(),
        mentioned_moves: vec![],
        verification: Verification { status: VerificationStatus::Ok, issues: vec![], unverified_moves: vec![] },
        provider: "anthropic".into(),
        model: "claude-opus-5".into(),
    };
    db.put_explanation(&id, &e, "explain_move.v1", &serde_json::json!({}), &serde_json::json!({}), Some(10), Some(5))
        .await
        .unwrap();
    db.set_analysis_status(&id, JobStatus::Done, None).await.unwrap();

    let a = db.latest_analysis(&g.id).await.unwrap().unwrap();
    assert_eq!(a.status, JobStatus::Done);
    assert_eq!(a.moves.len(), 33);
    assert_eq!(a.key_moments, vec![8, 20]);
    assert!(a.moves[7].is_key_moment && a.moves[19].is_key_moment && !a.moves[0].is_key_moment);
    assert_eq!(a.explanations.len(), 1);
    assert_eq!(a.black_accuracy, Some(60.0));
    let tags = db.mistake_counts_by_tag().await.unwrap();
    assert_eq!(tags.len(), 2);
    let summary = db.game_summary(&g.id).await.unwrap();
    assert_eq!(summary.analysis_status, Some(JobStatus::Done));
    assert_eq!(summary.white_accuracy, Some(95.0));
}

#[tokio::test]
async fn chat_threads_store_ir() {
    let db = Db::open_memory().await.unwrap();
    let t = db.create_thread(None, "Italian ideas").await.unwrap();
    let msg = ChatMessage {
        id: crate::new_id(),
        role: ChatRole::User,
        text: "What is the plan?".into(),
        tool_calls: vec![],
        verification: None,
        fen: Some(chess_core::position::START_FEN.into()),
        ply: None,
        created_at: now_ms(),
    };
    db.append_message(&t.id, &msg, &serde_json::json!([{"role": "user"}]), (None, None)).await.unwrap();
    let d = db.thread_detail(&t.id).await.unwrap();
    assert_eq!(d.messages.len(), 1);
    assert_eq!(db.thread_ir(&t.id).await.unwrap().len(), 1);
    assert_eq!(db.list_threads(None).await.unwrap().len(), 1);
}

#[tokio::test]
async fn keyring_persists_across_opens() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path()).await.unwrap();
    let p = db
        .create_provider(&ProviderInput {
            kind: ProviderKind::Openai,
            label: None,
            base_url: None,
            model: None,
            api_key: Some("sk-openai".into()),
            is_default: true,
        })
        .await
        .unwrap();
    db.pool().close().await;
    let db2 = Db::open(dir.path()).await.unwrap();
    assert_eq!(db2.provider(&p.id).await.unwrap().api_key.as_deref(), Some("sk-openai"));
}
