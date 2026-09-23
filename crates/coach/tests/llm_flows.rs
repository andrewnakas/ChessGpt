//! Explanation, review and chat flows against a scripted mock model.

mod common;

use api_types::{Classification, EloTier, MoveEval, Phase, Score, Side, VerificationStatus};
use chess_core::pgn::parse_pgn;
use chess_core::position::{parse_fen, play_sans};
use coach::analysis::MomentContext;
use coach::chat::{TurnInput, run_turn};
use coach::explain::{explain_moment, review_game};
use coach::prompts::{self, GameContext};
use coach::tools::ToolEnv;
use common::{MockProvider, text_reply, tool_reply};
use serde_json::json;
use tokio::sync::mpsc;

/// Opera game, 9... b5 (ply 18), with hand-made but legal engine lines.
fn opera_moment() -> (GameContext, MoveEval, MomentContext) {
    let game = parse_pgn(&common::fixture("opera.pgn")).unwrap();
    let pm = game.moves[17].clone();
    assert_eq!(pm.san, "b5");
    let before = parse_fen(&pm.fen_before).unwrap();
    let after = parse_fen(&pm.fen_after).unwrap();
    let l1: Vec<String> = ["Na6", "O-O-O", "Nc5"].map(String::from).to_vec();
    let l2: Vec<String> = ["h6", "Bxf6", "Qxf6"].map(String::from).to_vec();
    let refu: Vec<String> = ["Nxb5", "cxb5", "Bxb5+", "Nbd7"].map(String::from).to_vec();
    play_sans(&before, &l1).unwrap();
    play_sans(&before, &l2).unwrap();
    play_sans(&after, &refu).unwrap();
    let ctx = GameContext::new(&game, Some(Side::Black), 1300, Some("Philidor Defense".into()));
    let m = MoveEval {
        ply: 18,
        mover: Side::Black,
        san: "b5".into(),
        uci: pm.uci.clone(),
        score: Score::Cp(520),
        depth: 20,
        best_uci: Some("b8a6".into()),
        best_san: Some("Na6".into()),
        best_line_san: l1.clone(),
        classification: Classification::Blunder,
        lichess_judgement: Some(api_types::Judgement::Blunder),
        win_before: 30.0,
        win_after: 6.0,
        delta_wc: 0.48,
        accuracy: 20.0,
        phase: Phase::Opening,
        is_key_moment: true,
    };
    let mc = MomentContext {
        ply: 18,
        fen_before: pm.fen_before,
        fen_after: pm.fen_after,
        lines_before: vec![(Score::Cp(210), l1), (Score::Cp(260), l2)],
        refutation: Some((Score::Cp(520), refu)),
    };
    (ctx, m, mc)
}

#[test]
fn prompt_snapshots() {
    let (ctx, m, mc) = opera_moment();
    insta::assert_snapshot!("explain_system_intermediate", prompts::explain_system(&ctx));
    insta::assert_snapshot!("explain_moment_b5", prompts::explain_moment(&ctx, &m, &mc));
    let mut beginner = ctx.clone();
    beginner.tier = EloTier::Beginner;
    beginner.elo = 900;
    assert!(prompts::explain_system(&beginner).contains("Define any chess term"));
    insta::assert_snapshot!("review_system", prompts::review_system(&ctx));
}

fn draft(why: &str, better: serde_json::Value) -> String {
    json!({
        "headline": "b5 drops a pawn and opens lines",
        "why_it_matters": why,
        "better_move": better,
        "concept_tags": ["development", "king_safety", "not_a_tag"],
        "takeaway": "Finish development before pawn moves on the side where you are behind.",
        "mentioned_moves": ["b5", "Nxb5", "Na6"]
    })
    .to_string()
}

#[tokio::test]
async fn illegal_move_triggers_one_correction() {
    let (ctx, m, mc) = opera_moment();
    let bad = draft("After b5 White wins with Qxf7# at once.", json!(null));
    let good = draft(
        "After b5 White plays Nxb5 and after cxb5 Bxb5+ Black's king is stuck in the centre.",
        json!({"san": "Na6", "line_san": ["Na6"], "reason": "It develops and guards c5."}),
    );
    let p = MockProvider::new(vec![text_reply(&bad), text_reply(&good)]);
    let out = explain_moment(&p, &ctx, &m, &mc).await.unwrap();
    let e = out.explanation;
    assert_eq!(e.verification.status, VerificationStatus::Ok, "{:?}", e.verification);
    assert_eq!(p.requests.lock().unwrap().len(), 2);
    let second = &p.requests.lock().unwrap()[1];
    assert!(second.messages.last().unwrap().text().contains("Qxf7#"));
    let bm = e.better_move.unwrap();
    assert_eq!(bm.line_san, vec!["Na6", "O-O-O"], "line replaced with the engine's own line");
    assert_eq!(e.concept_tags, vec!["development", "king_safety"], "unknown tags dropped");
    assert_eq!(out.usage.input_tokens, 200);
}

#[tokio::test]
async fn persistent_hallucination_is_stripped() {
    let (ctx, m, mc) = opera_moment();
    let bad = draft("b5 weakens c6. Then Qxf7# follows. Develop first.", json!({"san": "Ke6", "line_san": [], "reason": "x"}));
    let p = MockProvider::new(vec![text_reply(&bad), text_reply(&bad)]);
    let e = explain_moment(&p, &ctx, &m, &mc).await.unwrap().explanation;
    assert_eq!(e.verification.status, VerificationStatus::Partial);
    assert_eq!(e.why_it_matters, "b5 weakens c6. Develop first.");
    assert!(e.better_move.is_none(), "illegal better move removed");
}

#[tokio::test]
async fn json_repair_path_for_local_models() {
    let (ctx, m, mc) = opera_moment();
    let mut p = MockProvider::new(vec![
        text_reply("I think b5 was bad."),
        text_reply(&format!("```json\n{}\n```", draft("Nxb5 wins a pawn.", json!(null)))),
    ]);
    p.json = false;
    let e = explain_moment(&p, &ctx, &m, &mc).await.unwrap().explanation;
    assert_eq!(e.verification.status, VerificationStatus::Ok);
    assert_eq!(p.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn review_strips_unbacked_moves() {
    let (ctx, m, _mc) = opera_moment();
    let e = api_types::Explanation {
        ply: 18,
        headline: "b5 lost".into(),
        why_it_matters: String::new(),
        better_move: Some(api_types::BetterMove { san: "Na6".into(), line_san: vec!["Na6".into(), "O-O-O".into()], reason: String::new() }),
        concept_tags: vec!["development".into()],
        takeaway: "develop".into(),
        mentioned_moves: vec!["b5".into(), "Nxb5".into()],
        verification: api_types::Verification { status: VerificationStatus::Ok, issues: vec![], unverified_moves: vec![], rejected_moves: vec![] },
        provider: "mock".into(),
        model: "mock-1".into(),
    };
    let reply = json!({
        "text": "Black fell behind in development. 9...b5 let Morphy open lines with 10. Nxb5. Instead Na6 was calmer. Another idea was Qh4 at once.",
        "themes": ["development", "bogus"]
    });
    let p = MockProvider::new(vec![text_reply(&reply.to_string())]);
    let (r, _) = review_game(&p, &ctx, std::slice::from_ref(&m), Some(90.0), Some(40.0), &[e]).await.unwrap();
    assert_eq!(r.themes, vec!["development"]);
    assert!(!r.text.contains("Qh4"), "{}", r.text);
    assert!(r.text.contains("Nxb5"));
    assert_eq!(r.verification.status, VerificationStatus::Partial);
}

#[tokio::test]
async fn chat_runs_tools_and_verifies() {
    let Some(pool) = common::deterministic_pool().await else { return };
    let (_, _, mc) = opera_moment();
    let env = ToolEnv { pool, tier: EloTier::Intermediate, explorer: None, game: None };
    let p = MockProvider::new(vec![
        tool_reply("t1", "legal_moves", json!({"fen": mc.fen_before})),
        tool_reply("t2", "play_line", json!({"fen": mc.fen_before, "moves": ["Na6", "O-O-O", "Ke2"]})),
        text_reply("Play Na6 to develop; b5 fails to Nxb5. A wild idea like Qxf2 does not work."),
    ]);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let r = run_turn(
        &p,
        &env,
        TurnInput {
            history: vec![],
            text: "What should Black play here?".into(),
            fen: mc.fen_before.clone(),
            ply: Some(17),
            move_path: vec![],
            elo: 1300,
            start_fen: None,
        },
        &tx,
    )
    .await
    .unwrap();
    drop(tx);
    assert_eq!(r.tool_calls.len(), 2);
    assert!(!r.tool_calls[0].is_error);
    assert!(r.tool_calls[1].is_error, "Ke2 is illegal: {}", r.tool_calls[1].summary);
    assert!(r.tool_calls[1].summary.contains("Ke2"));
    // user, assistant(tool), results, assistant(tool), results, assistant(final)
    assert_eq!(r.new_messages.len(), 6);
    assert!(r.new_messages[0].text().starts_with("[Board] FEN"));
    assert_eq!(r.verification.status, VerificationStatus::Partial, "Qxf2 is illegal here");
    assert!(r.verification.issues.iter().any(|i| i.contains("Qxf2")));
    let mut kinds = vec![];
    while let Some(e) = rx.recv().await {
        kinds.push(match e {
            api_types::ChatEvent::TextDelta { .. } => "text",
            api_types::ChatEvent::ToolCall { .. } => "call",
            api_types::ChatEvent::ToolResult { .. } => "result",
            api_types::ChatEvent::Verification { .. } => "verify",
            _ => "other",
        });
    }
    assert_eq!(kinds.iter().filter(|k| **k == "call").count(), 2);
    assert_eq!(kinds.last(), Some(&"verify"));
    // Second request replays the tool result for t1 as a user tool_result.
    let reqs = p.requests.lock().unwrap();
    assert_eq!(reqs.len(), 3);
    assert!(reqs[1].messages.iter().any(|m| m.parts.iter().any(|p| matches!(p, llm::Part::ToolResult { call_id, .. } if call_id == "t1"))));
    assert!(reqs[0].tools.iter().any(|t| t.name == "analyse_position"));
    assert!(!reqs[0].tools.iter().any(|t| t.name == "opening_lookup"), "explorer disabled");
}
