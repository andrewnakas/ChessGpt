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
        verification: Verification { status: VerificationStatus::Ok, issues: vec![], unverified_moves: vec![], rejected_moves: vec![] },
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

    // Progress: the game counts once the user's side is known.
    assert!(db.progress_games().await.unwrap().is_empty());
    db.set_user_side(&g.id, Some(Side::Black)).await.unwrap();
    db.set_estimates(&id, Some(2100), Some(1250)).await.unwrap();
    let p = db.progress_games().await.unwrap();
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].user_side, Side::Black);
    assert_eq!(p[0].estimate, Some(1250));
    assert_eq!(p[0].score, Some(0.0));
    assert_eq!((p[0].moves, p[0].errors), (16, 1));
    let phases = db.phase_counts().await.unwrap();
    assert_eq!(phases, vec![(Phase::Opening, 16, 1)]);
    let kinds = db.mistake_motif_counts().await.unwrap();
    assert!(kinds.iter().all(|(_, k, _)| k == "coach"), "{kinds:?}");
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

#[tokio::test]
async fn accounts_sessions_and_isolation() {
    let db = Db::open_memory().await.unwrap();
    let a = db.register("Ann@Example.com", "correct horse", None).await.unwrap();
    assert_eq!(a.email.as_deref(), Some("ann@example.com"));
    assert!(db.register("ann@example.com", "another pass", None).await.is_err());
    assert!(db.register("bob@example.com", "short", None).await.is_err());
    assert!(db.verify_login("ann@example.com", "wrong pass").await.is_err());
    assert_eq!(db.verify_login("ANN@example.com", "correct horse").await.unwrap().id, a.id);
    let t = db.create_session(&a.id).await.unwrap();
    assert_eq!(db.session_user(&t).await.unwrap().unwrap().id, a.id);
    db.delete_session(&t).await.unwrap();
    assert!(db.session_user(&t).await.unwrap().is_none());

    // Ann's games are invisible to Bob; sharing lets Bob copy one.
    let ann = db.for_user(&a.id);
    let g = opera(&ann).await;
    let b = db.register("bob@example.com", "password1", Some("Bob")).await.unwrap();
    let bob = db.for_user(&b.id);
    assert!(bob.list_games(10, 0).await.unwrap().is_empty());
    assert!(bob.game(&g.id).await.is_err());
    let share = ann.share_token(&g.id).await.unwrap();
    assert_eq!(ann.share_token(&g.id).await.unwrap(), share, "stable");
    assert_eq!(db.shared_game(&share).await.unwrap(), (a.id.clone(), g.id.clone()));
    let copy = bob.claim_shared(&share).await.unwrap();
    assert_ne!(copy, g.id);
    assert_eq!(bob.list_games(10, 0).await.unwrap().len(), 1);

    // Lichess login creates once, then finds the same account.
    let l1 = db.lichess_login("drnykterstein", "DrNykterstein", "lip_abc").await.unwrap();
    let l2 = db.lichess_login("drnykterstein", "DrNykterstein", "lip_def").await.unwrap();
    assert_eq!(l1.id, l2.id);
    assert_eq!(db.for_user(&l1.id).lichess_token().await.unwrap().as_deref(), Some("lip_def"));
}

#[tokio::test]
async fn oauth_code_and_token_lifecycle() {
    let db = Db::open_memory().await.unwrap();
    let u = db.register("c@example.com", "password1", None).await.unwrap();
    db.register_client("cid", "Claude", &["https://claude.ai/api/mcp/auth_callback".into()], &serde_json::json!({}))
        .await
        .unwrap();
    let code = db.create_code("cid", &u.id, "https://claude.ai/api/mcp/auth_callback", "chal", Some("https://x/mcp"), "chess")
        .await
        .unwrap();
    let (g, redirect, chal) = db.take_code(&code).await.unwrap().unwrap();
    assert_eq!((redirect.as_str(), chal.as_str()), ("https://claude.ai/api/mcp/auth_callback", "chal"));
    assert!(db.take_code(&code).await.unwrap().is_none(), "codes are single use");
    let t = db.issue_tokens(&g).await.unwrap();
    assert_eq!(db.access_grant(&t.access_token).await.unwrap().unwrap().user_id, u.id);
    assert!(db.access_grant(&t.refresh_token).await.unwrap().is_none(), "refresh token is not an access token");
    assert!(db.refresh(&t.refresh_token, "other").await.unwrap().is_none());
    let t2 = db.refresh(&t.refresh_token, "cid").await.unwrap().unwrap();
    assert!(db.refresh(&t.refresh_token, "cid").await.unwrap().is_none(), "rotated");
    assert_eq!(db.for_user(&u.id).connected_clients().await.unwrap().len(), 1);
    db.for_user(&u.id).disconnect_client("cid").await.unwrap();
    assert!(db.access_grant(&t2.access_token).await.unwrap().is_none());
}

#[tokio::test]
async fn puzzles_are_scheduled() {
    let db = Db::open_memory().await.unwrap();
    let g = opera(&db).await;
    let p = crate::NewPuzzle {
        game_id: g.id.clone(),
        analysis_id: "a1".into(),
        ply: 8,
        fen: chess_core::position::START_FEN.into(),
        solution_uci: vec!["e2e4".into()],
        line_san: vec!["e4".into(), "e5".into()],
        themes: vec!["fork".into()],
    };
    assert!(db.add_puzzle(&p).await.unwrap());
    assert!(!db.add_puzzle(&p).await.unwrap(), "once per game and ply");
    let due = db.due_puzzles(10).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].themes, vec!["fork"]);
    let after = db.record_attempt(&due[0].id, true).await.unwrap();
    assert_eq!(after.reps, 1);
    assert!(db.due_puzzles(10).await.unwrap().is_empty(), "solved: due tomorrow");
    assert_eq!(db.puzzle_counts().await.unwrap(), (1, 0, 0));
    let missed = db.record_attempt(&due[0].id, false).await.unwrap();
    assert_eq!((missed.reps, missed.lapses), (0, 1));
}

#[tokio::test]
async fn linking_a_username_marks_sides() {
    let db = Db::open_memory().await.unwrap();
    let g = opera(&db).await;
    assert_eq!(g.user_side, None);
    assert!(db.linked_users().await.unwrap().is_empty());
    db.set_chesscom_username(Some("PaulMorphy")).await.unwrap();
    assert_eq!(db.linked_users().await.unwrap().len(), 1);
    // Opera game: White is "Paul Morphy"; only an exact (case-insensitive) name counts.
    let white = db.game_summary(&g.id).await.unwrap().white;
    assert_eq!(db.mark_side_by_name("nobody").await.unwrap(), 0);
    assert_eq!(db.mark_side_by_name(&white.to_uppercase()).await.unwrap(), 1);
    assert_eq!(db.game_summary(&g.id).await.unwrap().user_side, Some(Side::White));
    assert_eq!(db.mark_side_by_name(&white).await.unwrap(), 0, "already marked");
}

fn drill_item(kind: api_types::DrillKind, fen: &str) -> api_types::DrillItem {
    api_types::DrillItem {
        id: String::new(),
        kind,
        origin: "bank".into(),
        fen: fen.into(),
        prompt: "p".into(),
        solution_uci: vec!["e2e4".into()],
        accept_uci: if kind == api_types::DrillKind::Defend { vec!["d2d4".into(), "g1f3".into()] } else { vec![] },
        trap_san: vec![],
        line_san: vec!["e4".into()],
        quiz: None,
        goal: None,
        hints: vec![],
        rating: Some(1500),
        solved: None,
    }
}

#[tokio::test]
async fn drill_sets_record_results_and_schedule_misses() {
    use api_types::{DrillAttempt, DrillKind, DrillSource};
    let db = Db::open_memory().await.unwrap();
    let f1 = "4k3/8/8/8/8/8/4P3/4K3 w - - 0 1";
    let f2 = "4k3/8/8/8/8/8/3P4/4K3 w - - 0 1";
    let items = vec![
        (drill_item(DrillKind::Spot, f1), Some("b1".to_string())),
        (drill_item(DrillKind::Find, f1), Some("b2".to_string())),
        (drill_item(DrillKind::Defend, f2), None),
    ];
    let set = db.create_drill_set("fork", &DrillSource::Theme { tag: "fork".into() }, 1200, items).await.unwrap();
    assert_eq!(set.label, "Fork");
    assert_eq!(set.items.len(), 3);
    assert!(set.items.iter().all(|i| !i.id.is_empty() && i.solved.is_none()));
    assert_eq!(db.recent_bank_ids().await.unwrap(), ["b1".to_string(), "b2".to_string()].into());

    let att = |solved, hints_used| DrillAttempt { solved, ms: 4000, hints_used };
    // A missed spot question is not a puzzle; a missed find is, once.
    db.record_drill_item(&set.id, &set.items[0].id, &att(false, 0)).await.unwrap();
    db.record_drill_item(&set.id, &set.items[1].id, &att(false, 1)).await.unwrap();
    let again = db.record_drill_item(&set.id, &set.items[1].id, &att(true, 0)).await.unwrap();
    assert_eq!(again.items[1].solved, Some(false), "first attempt counts");
    assert_eq!(db.due_puzzles(10).await.unwrap().len(), 1);
    // A solved defence finishes the set and schedules nothing.
    let done = db.record_drill_item(&set.id, &set.items[2].id, &att(true, 0)).await.unwrap();
    assert!(done.completed_at.is_some());
    let due = db.due_puzzles(10).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!((due[0].source.as_str(), due[0].themes.clone()), ("drill", vec!["fork".to_string()]));

    let m = db.technique_mastery().await.unwrap();
    let fork = &m["fork"];
    assert_eq!((fork.attempted, fork.solved, fork.clean, fork.median_ms), (3, 1, 1, Some(4000)));
    let recent = db.recent_drill_sets(5).await.unwrap();
    assert_eq!((recent[0].items, recent[0].attempted, recent[0].solved), (3, 3, 1));

    // A missed defence becomes a puzzle that accepts every holding move.
    let set2 = db
        .create_drill_set("fork", &DrillSource::Weakest, 1200, vec![(drill_item(DrillKind::Defend, f2), None)])
        .await
        .unwrap();
    db.record_drill_item(&set2.id, &set2.items[0].id, &att(false, 0)).await.unwrap();
    let p = db.due_puzzles(10).await.unwrap().into_iter().find(|p| p.fen == f2).unwrap();
    assert_eq!(p.accept_uci, vec!["d2d4", "g1f3"]);
    assert_eq!(p.solution_uci, vec!["d2d4"]);
}

#[tokio::test]
async fn drill_follow_up_goes_right_after_the_miss() {
    use api_types::{DrillAttempt, DrillKind, DrillSource};
    let db = Db::open_memory().await.unwrap();
    let f = "4k3/8/8/8/8/8/4P3/4K3 w - - 0 1";
    let items = vec![(drill_item(DrillKind::Find, f), Some("a".to_string())), (drill_item(DrillKind::Spot, f), None)];
    let set = db.create_drill_set("pin", &DrillSource::Weakest, 1500, items).await.unwrap();
    let first = set.items[0].id.clone();
    db.record_drill_item(&set.id, &first, &DrillAttempt { solved: false, ms: 1, hints_used: 0 }).await.unwrap();
    let mut extra = drill_item(DrillKind::Find, f);
    extra.prompt = "easier".into();
    let set = db.insert_drill_item_after(&set.id, &first, extra, Some("b".into())).await.unwrap();
    assert_eq!(set.items.iter().map(|i| i.prompt.as_str()).collect::<Vec<_>>(), ["p", "easier", "p"]);
    assert_eq!(set.items[1].kind, DrillKind::Find);
    assert_eq!(set.items[2].kind, DrillKind::Spot);
    assert_eq!(db.drill_bank_ids(&set.id).await.unwrap(), ["a".to_string(), "b".to_string()].into());
}
