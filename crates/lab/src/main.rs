//! Offline tools for the coach model: build a fixed set of key moments with
//! Stockfish, then score any model on it (`cargo xtask eval`).
//!
//!   chessgpt-lab build-set --out FILE [--per-game N] [--depth D] [--pgn-evals] PGN...
//!   chessgpt-lab datagen --set FILE --out FILE [--limit N] [--jobs J] [--per-minute R] [--min-judge S]
//!   chessgpt-lab eval --set FILE --out FILE [--limit N] [--jobs J] [--per-minute R]
//!   chessgpt-lab fit-rating PGN...
//!   chessgpt-lab baseline --out FILE [--per-band N] PGN...   (Lichess games with [%eval])   (Lichess games with [%eval] on every move)
//!
//! The model under test comes from `LAB_LLM_PROVIDER` / `_MODEL` / `_BASE_URL`
//! / `_API_KEY`; an optional judge from `LAB_JUDGE_*` (same names).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use api_types::{MoveEval, ProviderKind, Side, VerificationStatus};
use chess_core::motifs::{Motif, line_motifs, move_motifs};
use chess_core::pgn::{ParsedGame, parse_pgn_many};
use chess_core::accuracy::move_accuracy;
use chess_core::classify::{MoveInput, judge};
use chess_core::position::{parse_fen, phase, san_to_move};
use chess_core::rating;
use chess_core::score::Score;
use coach::analysis::{MomentContext, deep_pass, engine_pass};
use coach::explain::explain_moment;
use coach::key_moments;
use coach::prompts::{self, GameContext};
use engine::{EngineConfig, EnginePool, Limit, find_stockfish};
use futures::StreamExt;
use llm::{ChatRequest, JsonSchema, Message, Provider, ProviderConfig, send_with_retry};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

/// Rating bands the eval set is balanced over.
const BANDS: &[(u32, &str)] =
    &[(0, "<1000"), (1000, "1000-1399"), (1400, "1400-1799"), (1800, "1800-2099"), (2100, "2100-2399"), (2400, "2400+")];

fn band(elo: u32) -> &'static str {
    BANDS.iter().rev().find(|(lo, _)| elo >= *lo).map(|(_, b)| *b).unwrap_or("<1000")
}

/// A game back to PGN (tags and SAN moves; comments dropped).
fn to_pgn(game: &ParsedGame) -> String {
    let mut s: String = game.tags.iter().map(|(k, v)| format!("[{k} \"{v}\"]\n")).collect();
    s.push('\n');
    for (i, m) in game.moves.iter().enumerate() {
        let n = m.ply.div_ceil(2);
        if m.mover == Side::White {
            s.push_str(&format!("{n}. "));
        } else if i == 0 {
            s.push_str(&format!("{n}... "));
        }
        s.push_str(&m.san);
        s.push(' ');
    }
    s.push_str(game.tag("Result").unwrap_or("*"));
    s.push('\n');
    s
}

impl Record {
    fn game(&self) -> Result<ParsedGame> {
        chess_core::pgn::parse_pgn(&self.pgn).map_err(|e| anyhow::anyhow!("{}: {e}", self.id))
    }
}

/// One key moment with everything needed to prompt for it offline.
#[derive(Serialize, Deserialize)]
struct Record {
    id: String,
    band: String,
    elo: u32,
    user_side: Side,
    /// The game as PGN (parsed on load).
    pgn: String,
    opening: Option<String>,
    eval: MoveEval,
    moment: MomentContext,
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap().to_path_buf()
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// Flags that take no value.
const SWITCHES: &[&str] = &["--pgn-evals"];

fn positional(args: &[String]) -> Vec<String> {
    let mut out = vec![];
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
        } else if SWITCHES.contains(&a.as_str()) {
        } else if a.starts_with("--") {
            skip = true;
        } else {
            out.push(a.clone());
        }
    }
    out
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("build-set") => build_set(&args[1..]).await,
        Some("eval") => eval(&args[1..]).await,
        Some("fit-rating") => fit_rating(&args[1..]),
        Some("datagen") => datagen(&args[1..]).await,
        Some("baseline") => baseline(&args[1..]).await,
        _ => {
            eprintln!("usage: chessgpt-lab <build-set|eval|fit-rating|datagen> ...  (see the source header)");
            std::process::exit(2);
        }
    }
}

// ---------------------------------------------------------------- build-set

async fn build_set(args: &[String]) -> Result<()> {
    let out = flag(args, "--out").context("--out FILE is required")?;
    let per_game: usize = flag(args, "--per-game").map(|v| v.parse()).transpose()?.unwrap_or(2);
    let depth: u32 = flag(args, "--depth").map(|v| v.parse()).transpose()?.unwrap_or(16);
    let max_per_band: usize = flag(args, "--per-band").map(|v| v.parse()).transpose()?.unwrap_or(50);
    // Use the games' own [%eval] comments for the whole-game pass (Lichess
    // exports); Stockfish then only runs the deep pass on the key moments.
    let pgn_evals = args.iter().any(|a| a == "--pgn-evals");
    let files = positional(args);
    if files.is_empty() {
        bail!("give one or more PGN files");
    }
    let path = find_stockfish(&root()).context("no Stockfish (run `cargo xtask fetch-stockfish`)")?;
    let pool = EnginePool::start(EngineConfig::with_defaults(path), None).await?;
    let cancel = CancellationToken::new();

    let mut per_band: BTreeMap<&str, usize> = BTreeMap::new();
    let mut lines = vec![];
    for f in files {
        let text = std::fs::read_to_string(&f).with_context(|| f.clone())?;
        for (gi, game) in parse_pgn_many(&text)?.into_iter().enumerate() {
            if game.moves.len() < 20 {
                continue;
            }
            // Alternate the coached side between games.
            let side = if gi % 2 == 0 { Side::White } else { Side::Black };
            let Some(elo) = game.elo(side) else { continue };
            let b = band(elo);
            if per_band.get(b).copied().unwrap_or(0) >= max_per_band {
                continue;
            }
            let moves = if pgn_evals {
                match pass_from_evals(&game) {
                    Some(m) => m,
                    None => continue,
                }
            } else {
                engine_pass(&pool, &game, Limit::depth(depth), &cancel, |_| {}).await?.moves
            };
            let result = game.tag("Result").unwrap_or("*").to_string();
            let keys = key_moments::select(&moves, Some(side), &result, per_game);
            let moments = deep_pass(&pool, &game, &keys, 3, Limit::depth(depth + 2), &cancel).await?;
            for mc in moments {
                let mut eval = moves[mc.ply as usize - 1].clone();
                if eval.best_line_san.is_empty()
                    && let Some((_, l)) = mc.lines_before.first()
                {
                    eval.best_san = l.first().cloned();
                    eval.best_line_san = l.iter().take(10).cloned().collect();
                }
                let id = format!("{}-{}", game.tag("Site").unwrap_or("game").rsplit('/').next().unwrap_or("g"), mc.ply);
                let rec = Record {
                    id,
                    band: b.into(),
                    elo,
                    user_side: side,
                    opening: game.tag("Opening").map(str::to_string),
                    pgn: to_pgn(&game),
                    eval,
                    moment: mc,
                };
                lines.push(serde_json::to_string(&rec)?);
                *per_band.entry(b).or_default() += 1;
            }
            eprintln!("{} moments {:?}", lines.len(), per_band);
        }
    }
    std::fs::write(&out, lines.join("\n") + "\n")?;
    eprintln!("wrote {} moments to {out}", lines.len());
    Ok(())
}

// ---------------------------------------------------------------- eval

fn provider_from_env(prefix: &str) -> Result<Option<Arc<dyn Provider>>> {
    let var = |k: &str| std::env::var(format!("{prefix}_{k}")).ok().filter(|v| !v.is_empty());
    let Some(kind) = var("PROVIDER") else { return Ok(None) };
    let kind = ProviderKind::parse(&kind).with_context(|| format!("unknown {prefix}_PROVIDER {kind}"))?;
    Ok(Some(llm::build(ProviderConfig {
        kind,
        base_url: var("BASE_URL").unwrap_or_else(|| kind.default_base_url().into()),
        model: var("MODEL").unwrap_or_else(|| kind.default_model().into()),
        api_key: var("API_KEY"),
    })?))
}

/// Motif tags a good explanation of this moment should name.
fn expected_motifs(r: &Record) -> Vec<Motif> {
    let mut v = vec![];
    let (Ok(before), Ok(after)) = (parse_fen(&r.moment.fen_before), parse_fen(&r.moment.fen_after)) else {
        return v;
    };
    if let Some(m) = san_to_move(&before, &r.eval.san) {
        v.extend(move_motifs(&before, m).into_iter().map(|h| h.motif));
    }
    if let Some((_, l)) = &r.moment.refutation {
        v.extend(line_motifs(&after, l, 6).into_iter().map(|h| h.motif));
    }
    if let Some((_, l)) = r.moment.lines_before.first() {
        v.extend(line_motifs(&before, l, 6).into_iter().map(|h| h.motif));
    }
    v.sort();
    v.dedup();
    v
}

fn mentions(text: &str, m: Motif) -> bool {
    let t = text.to_lowercase();
    let words: &[&str] = match m {
        Motif::HangingPiece | Motif::LoosePawn => &["hanging", "undefended", "loose", "unprotected"],
        Motif::Fork => &["fork"],
        Motif::Pin => &["pin"],
        Motif::Skewer => &["skewer"],
        Motif::DiscoveredAttack => &["discover"],
        Motif::BackRank => &["back rank", "back-rank"],
        Motif::MatingAttack => &["mate", "checkmate"],
        Motif::TrappedPiece => &["trap"],
        Motif::Promotion => &["promot", "queen"],
        Motif::KingSafety => &["king safety", "exposed king", "shelter"],
        Motif::PassedPawn => &["passed pawn", "passer"],
        Motif::PawnStructure => &["isolated", "doubled", "pawn structure"],
    };
    words.iter().any(|w| t.contains(w))
}

/// Run `f`, waiting out rate limits (HTTP 429) for up to a day: free tiers
/// cap tokens per minute and requests per day.
async fn patient<T, F, Fut>(f: F) -> Result<T, coach::explain::ExplainError>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, coach::explain::ExplainError>>,
{
    let started = Instant::now();
    loop {
        match f().await {
            Err(e) if e.to_string().contains("(429)") && started.elapsed().as_secs() < 86_400 => {
                eprintln!("rate limited; waiting a minute");
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
            other => return other,
        }
    }
}

const JUDGE_SYSTEM: &str = "You grade explanations written by a chess coaching model. You get the data the coach was given (engine lines and computed facts, which are correct) and the coach's JSON answer. Score each criterion from 1 (bad) to 5 (excellent):
- accuracy: every chess claim agrees with the engine data and facts; nothing invented.
- insight: names the real reason the move matters (the tactic, threat or plan the data shows), not a vague generality.
- level: language and depth suit the stated rating.
- takeaway: a practical, correct rule the student can reuse.
Be strict; a fluent answer with one false claim gets accuracy 2 or lower. Reply with JSON only.";

fn judge_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "accuracy": {"type": "integer", "minimum": 1, "maximum": 5},
            "insight": {"type": "integer", "minimum": 1, "maximum": 5},
            "level": {"type": "integer", "minimum": 1, "maximum": 5},
            "takeaway": {"type": "integer", "minimum": 1, "maximum": 5},
            "notes": {"type": "string"}
        },
        "required": ["accuracy", "insight", "level", "takeaway", "notes"],
        "additionalProperties": false
    })
}

async fn grade(j: &dyn Provider, elo: u32, moment_prompt: &str, answer: &Value) -> Option<Value> {
    let user = format!(
        "STUDENT RATING: {elo}\n\nDATA GIVEN TO THE COACH\n{moment_prompt}\n\nCOACH ANSWER\n{}",
        serde_json::to_string_pretty(answer).ok()?
    );
    let mut req = ChatRequest::new(JUDGE_SYSTEM.to_string(), vec![Message::user(user)]);
    req.json_schema = Some(JsonSchema { name: "grade".into(), schema: judge_schema() });
    for _ in 0..1440 {
        match send_with_retry(j, &req, None).await {
            Ok(c) => return llm::json::extract_object(&c.message.text()),
            Err(e) if e.to_string().contains("(429)") => tokio::time::sleep(std::time::Duration::from_secs(60)).await,
            Err(_) => return None,
        }
    }
    None
}

#[derive(Serialize, Default)]
struct Item {
    id: String,
    band: String,
    ok: bool,
    error: Option<String>,
    first_try: bool,
    verified: Option<String>,
    rejected_moves: usize,
    expected_motifs: Vec<&'static str>,
    motif_covered: Option<bool>,
    judge: Option<Value>,
    seconds: f64,
    output_tokens: u64,
    explanation: Option<Value>,
}

#[derive(Serialize, Default)]
struct Summary {
    n: usize,
    ok_rate: f64,
    first_try_rate: f64,
    verified_ok_rate: f64,
    motif_coverage: f64,
    judge_mean: Option<f64>,
    mean_seconds: f64,
}

fn summarize(items: &[&Item]) -> Summary {
    let n = items.len().max(1) as f64;
    let rate = |f: &dyn Fn(&Item) -> bool| items.iter().filter(|i| f(i)).count() as f64 / n;
    let with_motifs: Vec<_> = items.iter().filter_map(|i| i.motif_covered).collect();
    let judged: Vec<f64> = items
        .iter()
        .filter_map(|i| i.judge.as_ref())
        .filter_map(|j| {
            let ks = ["accuracy", "insight", "level", "takeaway"];
            let v: Vec<f64> = ks.iter().filter_map(|k| j.get(*k).and_then(Value::as_f64)).collect();
            (v.len() == ks.len()).then(|| v.iter().sum::<f64>() / v.len() as f64)
        })
        .collect();
    Summary {
        n: items.len(),
        ok_rate: rate(&|i| i.ok),
        first_try_rate: rate(&|i| i.first_try),
        verified_ok_rate: rate(&|i| i.verified.as_deref() == Some("ok")),
        motif_coverage: if with_motifs.is_empty() {
            0.0
        } else {
            with_motifs.iter().filter(|c| **c).count() as f64 / with_motifs.len() as f64
        },
        judge_mean: (!judged.is_empty()).then(|| judged.iter().sum::<f64>() / judged.len() as f64),
        mean_seconds: items.iter().map(|i| i.seconds).sum::<f64>() / n,
    }
}

async fn eval(args: &[String]) -> Result<()> {
    let set = flag(args, "--set").unwrap_or_else(|| root().join("fixtures/eval/moments.jsonl").display().to_string());
    let out = flag(args, "--out").context("--out FILE is required")?;
    let limit: usize = flag(args, "--limit").map(|v| v.parse()).transpose()?.unwrap_or(usize::MAX);
    let jobs: usize = flag(args, "--jobs").map(|v| v.parse()).transpose()?.unwrap_or(2);
    let per_minute: f64 = flag(args, "--per-minute").map(|v| v.parse()).transpose()?.unwrap_or(1000.0);
    let gap = std::time::Duration::from_secs_f64(60.0 / per_minute.max(0.1));
    let model = provider_from_env("LAB_LLM")?.context("set LAB_LLM_PROVIDER (and _MODEL, _BASE_URL, _API_KEY)")?;
    let judge_p = provider_from_env("LAB_JUDGE")?;

    let records: Vec<Record> = std::fs::read_to_string(&set)
        .with_context(|| set.clone())?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(limit)
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    eprintln!("evaluating {} on {} moments", model.model(), records.len());

    let items: Vec<Item> = futures::stream::iter(records.iter().map(|r| {
        let model = model.clone();
        let judge_p = judge_p.clone();
        async move {
            let game = match r.game() {
                Ok(g) => g,
                Err(e) => return Item { id: r.id.clone(), band: r.band.clone(), error: Some(e.to_string()), ..Default::default() },
            };
            let ctx = GameContext::new(&game, Some(r.user_side), r.elo, r.opening.clone());
            let expected = expected_motifs(r);
            let started = Instant::now();
            let res = patient(|| explain_moment(model.as_ref(), &ctx, &r.eval, &r.moment)).await;
            tokio::time::sleep(gap).await;
            let mut item = Item {
                id: r.id.clone(),
                band: r.band.clone(),
                expected_motifs: expected.iter().map(|m| m.tag()).collect(),
                seconds: started.elapsed().as_secs_f64(),
                ..Default::default()
            };
            match res {
                Ok(x) => {
                    let e = &x.explanation;
                    let text = format!("{} {} {}", e.headline, e.why_it_matters, e.takeaway);
                    item.ok = true;
                    item.first_try = x.response["texts"].as_array().is_some_and(|t| t.len() == 1);
                    item.verified = Some(
                        match e.verification.status {
                            VerificationStatus::Ok => "ok",
                            VerificationStatus::Partial => "partial",
                            VerificationStatus::Unverified => "unverified",
                        }
                        .into(),
                    );
                    item.rejected_moves = e.verification.rejected_moves.len();
                    item.output_tokens = u64::from(x.usage.output_tokens);
                    if !expected.is_empty() {
                        item.motif_covered = Some(
                            expected.iter().any(|m| e.concept_tags.iter().any(|t| t == m.tag()) || mentions(&text, *m)),
                        );
                    }
                    let answer = serde_json::to_value(e).unwrap_or(Value::Null);
                    if let Some(j) = &judge_p {
                        item.judge = grade(j.as_ref(), r.elo, &prompts::explain_moment(&ctx, &r.eval, &r.moment), &answer).await;
                    }
                    item.explanation = Some(answer);
                }
                Err(e) => item.error = Some(e.to_string()),
            }
            eprintln!("{} {} ok={} verified={:?}", item.id, item.band, item.ok, item.verified);
            item
        }
    }))
    .buffered(jobs)
    .collect()
    .await;

    let all: Vec<&Item> = items.iter().collect();
    let mut by_band = BTreeMap::new();
    for (_, b) in BANDS {
        let v: Vec<&Item> = items.iter().filter(|i| i.band == *b).collect();
        if !v.is_empty() {
            by_band.insert(*b, summarize(&v));
        }
    }
    let overall = summarize(&all);
    println!("{}", serde_json::to_string_pretty(&json!({"overall": overall, "by_band": by_band}))?);
    let report = json!({
        "model": model.model(),
        "judge": judge_p.as_ref().map(|j| j.model().to_string()),
        "prompt_version": prompts::EXPLAIN_VERSION,
        "set": set,
        "overall": overall,
        "by_band": by_band,
        "items": items,
    });
    if let Some(dir) = Path::new(&out).parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&out, serde_json::to_string_pretty(&report)?)?;
    eprintln!("wrote {out}");
    Ok(())
}

// ---------------------------------------------------------------- fit-rating

/// Move evaluations from a game's `[%eval]` comments (every move must have
/// one), classified like the engine pass; no best lines.
fn pass_from_evals(game: &ParsedGame) -> Option<Vec<MoveEval>> {
    let mut out = vec![];
    let mut prev = Score::Cp(15);
    for pm in &game.moves {
        let cur = pm.pgn_eval?;
        let v = judge(&MoveInput { mover: pm.mover.into(), prev, cur, best_uci: None, played_uci: &pm.uci, is_book: false });
        out.push(MoveEval {
            ply: pm.ply,
            mover: pm.mover,
            san: pm.san.clone(),
            uci: pm.uci.clone(),
            score: cur,
            depth: 0,
            best_uci: None,
            best_san: None,
            best_line_san: vec![],
            classification: v.classification,
            lichess_judgement: v.lichess_judgement,
            win_before: v.win_before,
            win_after: v.win_after,
            delta_wc: v.delta_wc,
            accuracy: move_accuracy(v.win_before, v.win_after),
            phase: phase(&parse_fen(&pm.fen_before).ok()?),
            is_key_moment: false,
        });
        prev = cur;
    }
    Some(out)
}

/// Per-side move stats from a game whose every move carries a PGN eval.
fn eval_game_stats(game: &ParsedGame) -> Option<[Vec<rating::MoveStat>; 2]> {
    let mut out: [Vec<rating::MoveStat>; 2] = [vec![], vec![]];
    let mut prev = Score::Cp(15);
    let last = game.moves.len().checked_sub(1)?;
    for (i, pm) in game.moves.iter().enumerate() {
        let cur = match pm.pgn_eval {
            Some(s) => s,
            None if i == last => break,
            None => return None,
        };
        let v = judge(&MoveInput {
            mover: pm.mover.into(),
            prev,
            cur,
            best_uci: None,
            played_uci: &pm.uci,
            is_book: false,
        });
        let phase = parse_fen(&pm.fen_before).map(|p| phase(&p)).ok()?;
        out[usize::from(pm.mover == Side::Black)].push(rating::MoveStat {
            win_before: v.win_before,
            win_after: v.win_after,
            accuracy: move_accuracy(v.win_before, v.win_after),
            judgement: v.lichess_judgement,
            phase,
        });
        prev = cur;
    }
    Some(out)
}

/// Solve `a x = b` (small, dense) by Gaussian elimination with pivoting.
fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Vec<f64> {
    let n = b.len();
    for c in 0..n {
        let p = (c..n).max_by(|i, j| a[*i][c].abs().total_cmp(&a[*j][c].abs())).unwrap();
        a.swap(c, p);
        b.swap(c, p);
        for r in c + 1..n {
            let f = a[r][c] / a[c][c];
            let pivot = a[c].clone();
            for (k, v) in a[r].iter_mut().enumerate().skip(c) {
                *v -= f * pivot[k];
            }
            b[r] -= f * b[c];
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        let s: f64 = (r + 1..n).map(|k| a[r][k] * x[k]).sum();
        x[r] = (b[r] - s) / a[r][r];
    }
    x
}

/// Ridge regression on standardized features; returns raw-scale coefficients.
fn ridge(xs: &[[f64; rating::FEATURES]], ys: &[f64], lambda: f64) -> [f64; rating::FEATURES] {
    const F: usize = rating::FEATURES;
    let n = xs.len() as f64;
    let mut mu = [0.0; F];
    let mut sd = [1.0; F];
    for j in 1..F {
        mu[j] = xs.iter().map(|x| x[j]).sum::<f64>() / n;
        let var = xs.iter().map(|x| (x[j] - mu[j]).powi(2)).sum::<f64>() / n;
        sd[j] = var.sqrt().max(1e-9);
    }
    let z = |x: &[f64; F]| -> [f64; F] {
        let mut o = [1.0; F];
        for j in 1..F {
            o[j] = (x[j] - mu[j]) / sd[j];
        }
        o
    };
    let mut a = vec![vec![0.0; F]; F];
    let mut b = vec![0.0; F];
    for (x, y) in xs.iter().zip(ys) {
        let zx = z(x);
        for i in 0..F {
            b[i] += zx[i] * y;
            for k in 0..F {
                a[i][k] += zx[i] * zx[k];
            }
        }
    }
    for (j, row) in a.iter_mut().enumerate().skip(1) {
        row[j] += lambda * n;
    }
    let beta = solve(a, b);
    let mut raw = [0.0; F];
    raw[0] = beta[0];
    for j in 1..F {
        raw[j] = beta[j] / sd[j];
        raw[0] -= beta[j] * mu[j] / sd[j];
    }
    raw
}

fn fit_rating(args: &[String]) -> Result<()> {
    let files = positional(args);
    if files.is_empty() {
        bail!("give one or more PGN files of Lichess games with [%eval] annotations");
    }
    let mut xs = vec![];
    let mut ys = vec![];
    for f in &files {
        let text = std::fs::read_to_string(f).with_context(|| f.clone())?;
        for game in parse_pgn_many(&text)? {
            let Some(stats) = eval_game_stats(&game) else { continue };
            let base = game.tag("TimeControl").and_then(rating::base_seconds);
            for (side, moves) in [Side::White, Side::Black].into_iter().zip(stats) {
                if let (Some(elo), Some(x)) = (game.elo(side), rating::features(&moves, base)) {
                    xs.push(x);
                    ys.push(elo as f64);
                }
            }
        }
    }
    if xs.len() < 100 {
        bail!("only {} rated sides with full evals; need more games", xs.len());
    }
    let test = |i: usize| i.is_multiple_of(5);
    let (tx, ty): (Vec<_>, Vec<_>) = xs.iter().zip(&ys).enumerate().filter(|(i, _)| !test(*i)).map(|(_, (x, y))| (*x, *y)).unzip();
    let coef = ridge(&tx, &ty, 0.01);
    let predict = |x: &[f64; rating::FEATURES]| x.iter().zip(coef.iter()).map(|(a, c)| a * c).sum::<f64>().clamp(400.0, 3000.0);
    let train_mean = ty.iter().sum::<f64>() / ty.len() as f64;
    let (mut mae, mut base_mae, mut n) = (0.0, 0.0, 0.0);
    for (i, (x, y)) in xs.iter().zip(&ys).enumerate() {
        if test(i) {
            mae += (predict(x) - y).abs();
            base_mae += (train_mean - y).abs();
            n += 1.0;
        }
    }
    eprintln!("{} sides ({} train, {} test)", xs.len(), tx.len(), n);
    eprintln!("test MAE {:.0} (predicting the mean: {:.0})", mae / n, base_mae / n);
    for (name, c) in rating::FEATURE_NAMES.iter().zip(coef.iter()) {
        eprintln!("  {name:20} {c:>12.4}");
    }
    // Calibration for averages: per-game predictions shrink toward the mean,
    // so fit prediction = a + b * rating and invert it when averaging games.
    let preds: Vec<f64> = xs.iter().map(predict).collect();
    let (my, mp) = (ys.iter().sum::<f64>() / ys.len() as f64, preds.iter().sum::<f64>() / preds.len() as f64);
    let b = ys.iter().zip(&preds).map(|(y, p)| (y - my) * (p - mp)).sum::<f64>() / ys.iter().map(|y| (y - my).powi(2)).sum::<f64>();
    let a = mp - b * my;
    let resid = (ys.iter().zip(&preds).map(|(y, p)| (p - a - b * y).powi(2)).sum::<f64>() / ys.len() as f64).sqrt();
    eprintln!("calibration: prediction = {a:.1} + {b:.3} x rating (residual sd {resid:.0}); 10-game average error ~ {:.0}", resid / b / 10f64.sqrt());
    eprintln!("pub const CALIBRATION: (f64, f64) = ({a:.3}, {b:.5});");
    let full = ridge(&xs, &ys, 0.01);
    let list: Vec<String> = full.iter().map(|c| format!("{c:.6}")).collect();
    println!("pub const COEF: [f64; FEATURES] = [{}];", list.join(", "));
    Ok(())
}

// ---------------------------------------------------------------- datagen

/// Distillation data: the teacher (`LAB_TEACHER_*`) explains each moment
/// through the production pipeline; answers that verify cleanly on the first
/// try, name the moment's motif, and (with `LAB_JUDGE_*`) grade well become
/// chat-format training examples. The messages are exactly what the
/// in-browser and llama.cpp models see (OpenAI-compatible body, schema in the
/// system prompt). Appends to `--out` and skips ids already done, so it can
/// be stopped and resumed across days of free-tier quota.
async fn datagen(args: &[String]) -> Result<()> {
    let set = flag(args, "--set").context("--set FILE is required")?;
    let out = flag(args, "--out").context("--out FILE is required")?;
    let limit: usize = flag(args, "--limit").map(|v| v.parse()).transpose()?.unwrap_or(usize::MAX);
    let jobs: usize = flag(args, "--jobs").map(|v| v.parse()).transpose()?.unwrap_or(1);
    let per_minute: f64 = flag(args, "--per-minute").map(|v| v.parse()).transpose()?.unwrap_or(20.0);
    let min_judge: f64 = flag(args, "--min-judge").map(|v| v.parse()).transpose()?.unwrap_or(4.0);
    let teacher = provider_from_env("LAB_TEACHER")?.context("set LAB_TEACHER_PROVIDER (and _MODEL, _BASE_URL, _API_KEY)")?;
    let judge_p = provider_from_env("LAB_JUDGE")?;
    let rejected_path = format!("{out}.rejected");
    let done: std::collections::HashSet<String> = [out.as_str(), rejected_path.as_str()]
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .flat_map(|t| t.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()?["id"].as_str().map(String::from)).collect::<Vec<_>>())
        .collect();
    let records: Vec<Record> = std::fs::read_to_string(&set)
        .with_context(|| set.clone())?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<Result<Vec<Record>, _>>()?
        .into_iter()
        .filter(|r| !done.contains(&r.id))
        .take(limit)
        .collect();
    eprintln!("{} moments to do ({} already done)", records.len(), done.len());
    let wire = llm::openai::OpenAiCompat::new(
        llm::http_client(),
        ProviderConfig { kind: ProviderKind::OpenaiCompatible, base_url: String::new(), model: "student".into(), api_key: None },
    );
    let gap = std::time::Duration::from_secs_f64(60.0 / per_minute.max(0.1));
    let (mut kept, mut dropped) = (0usize, 0usize);
    let mut stream = futures::stream::iter(records.iter().map(|r| {
        let teacher = teacher.clone();
        let judge_p = judge_p.clone();
        let wire = &wire;
        async move {
            let game = match r.game() {
                Ok(g) => g,
                Err(e) => return (r, Err(e.to_string())),
            };
            let mut ctx = GameContext::new(&game, Some(r.user_side), r.elo, r.opening.clone());
            ctx.recurring = vec![];
            let res = patient(|| explain_moment(teacher.as_ref(), &ctx, &r.eval, &r.moment)).await;
            let x = match res {
                Ok(x) => x,
                Err(e) => return (r, Err(format!("teacher: {e}"))),
            };
            let e = &x.explanation;
            if x.response["texts"].as_array().is_none_or(|t| t.len() != 1) {
                return (r, Err("needed a correction round".into()));
            }
            if e.verification.status != VerificationStatus::Ok {
                return (r, Err(format!("verification {:?}", e.verification.status)));
            }
            let expected = expected_motifs(r);
            let text = format!("{} {} {}", e.headline, e.why_it_matters, e.takeaway);
            if !expected.is_empty() && !expected.iter().any(|m| e.concept_tags.iter().any(|t| t == m.tag()) || mentions(&text, *m)) {
                return (r, Err("missed the motif".into()));
            }
            let answer = json!({
                "headline": e.headline,
                "why_it_matters": e.why_it_matters,
                "better_move": e.better_move,
                "concept_tags": e.concept_tags,
                "takeaway": e.takeaway,
                "mentioned_moves": e.mentioned_moves,
            });
            let prompt = prompts::explain_moment(&ctx, &r.eval, &r.moment);
            let mut score = None;
            if let Some(j) = &judge_p {
                let g = grade(j.as_ref(), r.elo, &prompt, &answer).await;
                let ks = ["accuracy", "insight", "level", "takeaway"];
                let v: Vec<f64> = ks.iter().filter_map(|k| g.as_ref()?.get(*k)?.as_f64()).collect();
                if v.len() != ks.len() {
                    return (r, Err("judge failed".into()));
                }
                let mean = v.iter().sum::<f64>() / v.len() as f64;
                if mean < min_judge || v[0] < 4.0 {
                    return (r, Err(format!("judge {mean:.1}")));
                }
                score = Some(mean);
            }
            let mut req = ChatRequest::new(prompts::explain_system(&ctx), vec![Message::user(prompt)]);
            req.json_schema = Some(JsonSchema { name: "move_explanation".into(), schema: prompts::explanation_schema() });
            let body = wire.body_with(&req, false);
            let mut messages = body["messages"].as_array().cloned().unwrap_or_default();
            messages.push(json!({"role": "assistant", "content": answer.to_string()}));
            let line = json!({
                "id": r.id,
                "messages": messages,
                "meta": {"band": r.band, "elo": r.elo, "teacher": teacher.model(), "judge": score, "prompt_version": prompts::EXPLAIN_VERSION},
            });
            (r, Ok(line))
        }
    }))
    .buffer_unordered(jobs);
    use std::io::Write as _;
    let mut ok_file = std::fs::OpenOptions::new().create(true).append(true).open(&out)?;
    let mut bad_file = std::fs::OpenOptions::new().create(true).append(true).open(&rejected_path)?;
    while let Some((r, res)) = stream.next().await {
        match res {
            Ok(line) => {
                writeln!(ok_file, "{line}")?;
                kept += 1;
            }
            Err(why) => {
                // Teacher errors (quota, network) are retried next run; quality rejects are final.
                if !why.starts_with("teacher:") {
                    writeln!(bad_file, "{}", json!({"id": r.id, "why": why}))?;
                }
                dropped += 1;
                eprintln!("{} rejected: {why}", r.id);
            }
        }
        eprintln!("kept {kept}, rejected {dropped}");
        tokio::time::sleep(gap).await;
    }
    Ok(())
}

// ---------------------------------------------------------------- baseline

/// Typical mistake-motif rates per rating band, for the progress page's
/// "compared with players at your level". Classification comes from the
/// games' `[%eval]` comments; Stockfish runs only on the error positions, to
/// get the better line and the refutation the motifs are read from.
async fn baseline(args: &[String]) -> Result<()> {
    let out = flag(args, "--out").context("--out FILE is required")?;
    let per_band: usize = flag(args, "--per-band").map(|v| v.parse()).transpose()?.unwrap_or(60);
    let files = positional(args);
    let path = find_stockfish(&root()).context("no Stockfish")?;
    let pool = EnginePool::start(EngineConfig::with_defaults(path), None).await?;
    let lim = Limit::depth(14);
    // band -> (games, moves, tag -> mistakes)
    let mut agg: BTreeMap<&str, (usize, u64, BTreeMap<String, u64>)> = BTreeMap::new();
    for f in &files {
        let text = std::fs::read_to_string(f).with_context(|| f.clone())?;
        for game in parse_pgn_many(&text)? {
            let Some(moves) = pass_from_evals(&game) else { continue };
            for side in [Side::White, Side::Black] {
                let Some(elo) = game.elo(side) else { continue };
                let b = band(elo);
                let e = agg.entry(b).or_default();
                if e.0 >= per_band {
                    continue;
                }
                e.0 += 1;
                e.1 += moves.iter().filter(|m| m.mover == side).count() as u64;
                for (i, m) in moves.iter().enumerate() {
                    if m.mover != side || !m.classification.is_error() || m.win_before < key_moments::ALREADY_LOST {
                        continue;
                    }
                    let pm = &game.moves[i];
                    let (Ok(before), Ok(after)) = (parse_fen(&pm.fen_before), parse_fen(&pm.fen_after)) else { continue };
                    let req = |fen: &str| engine::AnalysisRequest { fen: fen.into(), multipv: 1, limit: lim, priority: engine::Priority::Batch };
                    let (a, r) = tokio::join!(pool.analyse(req(&pm.fen_before)), pool.analyse(req(&pm.fen_after)));
                    let best = a.ok().and_then(|a| a.lines.first().map(|l| chess_core::position::pv_to_san(&before, &l.pv))).unwrap_or_default();
                    let refu = r.ok().and_then(|r| r.lines.first().map(|l| chess_core::position::pv_to_san(&after, &l.pv))).unwrap_or_default();
                    let Some(mv) = chess_core::position::uci_to_move(&before, &pm.uci) else { continue };
                    let (missed, allowed) = chess_core::motifs::mistake_motifs(&before, mv, &best, &refu);
                    let mut tags: Vec<&str> = missed.iter().chain(&allowed).map(|t| t.tag()).collect();
                    tags.sort();
                    tags.dedup();
                    let e = agg.get_mut(b).expect("present");
                    for t in tags {
                        *e.2.entry(t.to_string()).or_default() += 1;
                    }
                }
            }
            let counts: Vec<String> = agg.iter().map(|(b, (g, _, _))| format!("{b}: {g}")).collect();
            eprintln!("{}", counts.join(", "));
            if BANDS.iter().all(|(_, b)| agg.get(b).is_some_and(|e| e.0 >= per_band)) {
                break;
            }
        }
    }
    let json = json!(agg
        .iter()
        .map(|(b, (games, moves, tags))| (b.to_string(), json!({"games": games, "moves": moves, "mistakes_by_tag": tags})))
        .collect::<BTreeMap<_, _>>());
    std::fs::write(&out, serde_json::to_string_pretty(&json)?)?;
    eprintln!("wrote {out}");
    Ok(())
}
