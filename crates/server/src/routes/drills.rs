//! Technique drills: sets that train one idea several ways.

use std::collections::BTreeMap;

use api_types::{DailyStatus, DrillAttempt, DrillKind, DrillOverview, DrillSet, DrillSource, MistakeMotif, TechniqueMastery};
use axum::Json;
use axum::extract::Path;
use coach::bank::{Bank, TECHNIQUES};
use coach::drills::{
    DrillRequest, MAX_ITEMS, Seed, compose, daily, daily_technique, date_of, day_of, day_of_date, drillable, easier_find, streak,
};

use crate::auth::UserState;
use crate::error::{ApiResult, AppError};
use crate::state::AppState;

/// Drill sets a week the plan asks for (about 5 minutes each).
const WEEK_GOAL: u32 = 5;

/// The rating to pitch drills at: the rating estimate once there are 5+
/// analysed games (as on the progress page), else the rating in settings.
async fn player_rating(state: &AppState) -> ApiResult<u32> {
    let games = state.db.progress_games().await?;
    let estimates: Vec<u32> = games.iter().filter_map(|g| g.estimate).collect();
    if estimates.len() >= 5
        && let Some((r, _)) = chess_core::rating::rolling(&estimates)
    {
        return Ok(r);
    }
    Ok(state.db.settings().await?.elo)
}

/// Drillable techniques, most frequent in the user's mistakes first, with
/// ones already drilled well (80%+ clean over 10+ items) moved to the back.
async fn focus(state: &AppState, rating: u32) -> ApiResult<Vec<String>> {
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    for (tag, _, n) in state.db.mistake_motif_counts().await? {
        if drillable(&tag) {
            *counts.entry(tag).or_default() += n;
        }
    }
    let mastery = state.db.technique_mastery().await?;
    let mastered = |t: &str| mastery.get(t).is_some_and(|m| m.attempted >= 10 && m.clean * 5 >= m.attempted * 4);
    let mut tags: Vec<(String, i64)> = counts.into_iter().collect();
    tags.sort_by_key(|(t, n)| (mastered(t), std::cmp::Reverse(*n)));
    let mut out: Vec<String> = tags.into_iter().map(|(t, _)| t).collect();
    // Nothing analysed yet: the basics for the rating.
    let defaults: &[&str] =
        if rating < 1200 { &["hanging_piece", "fork", "back_rank"] } else { &["fork", "pin", "discovered_attack"] };
    for d in defaults {
        if out.len() >= 2 {
            break;
        }
        if !out.iter().any(|t| t == d) {
            out.push(d.to_string());
        }
    }
    Ok(out)
}

/// Today's daily drill for the user: started?, finished?, and the streak.
async fn daily_status(state: &AppState) -> ApiResult<DailyStatus> {
    let today = day_of(db::now_ms());
    let date = date_of(today);
    let sets = state.db.daily_sets().await?;
    let done: std::collections::HashSet<i64> =
        sets.iter().filter(|(_, _, f)| *f).filter_map(|(d, _, _)| day_of_date(d)).collect();
    let mine = sets.iter().find(|(d, _, _)| *d == date);
    let technique = daily_technique(today);
    Ok(DailyStatus {
        label: coach::tags::label(technique),
        technique: technique.into(),
        set_id: mine.map(|(_, id, _)| id.clone()),
        done: mine.is_some_and(|(_, _, f)| *f),
        streak: streak(&done, today),
        date,
    })
}

/// Today's daily drill (the same for everyone), for guests to play in the browser.
pub async fn daily_public() -> ApiResult<Json<DrillSet>> {
    let today = day_of(db::now_ms());
    let date = date_of(today);
    let technique = daily_technique(today);
    let items = daily(Bank::builtin(), today)
        .into_iter()
        .enumerate()
        .map(|(n, d)| api_types::DrillItem { id: format!("g{n}"), ..d.item })
        .collect();
    Ok(Json(DrillSet {
        id: format!("daily-{date}"),
        label: coach::tags::label(technique),
        technique: technique.into(),
        source: DrillSource::Daily { date },
        rating: 1500,
        items,
        created_at: db::now_ms(),
        completed_at: None,
    }))
}

pub async fn overview(UserState(state, _): UserState) -> ApiResult<Json<DrillOverview>> {
    let rating = player_rating(&state).await?;
    let mastery = state.db.technique_mastery().await?;
    let techniques = TECHNIQUES
        .iter()
        .map(|(tag, _)| {
            let m = mastery.get(*tag).cloned().unwrap_or_default();
            TechniqueMastery {
                tag: tag.to_string(),
                label: coach::tags::label(tag),
                attempted: m.attempted,
                solved: m.solved,
                clean: m.clean,
                median_ms: m.median_ms,
            }
        })
        .collect();
    Ok(Json(DrillOverview {
        techniques,
        focus: focus(&state, rating).await?,
        week_done: state.db.drill_sets_finished_this_week().await?,
        week_goal: WEEK_GOAL,
        daily: daily_status(&state).await?,
        recent: state.db.recent_drill_sets(10).await?,
    }))
}

pub async fn create(UserState(state, _): UserState, Json(source): Json<DrillSource>) -> ApiResult<Json<DrillSet>> {
    if let DrillSource::Daily { date } = &source {
        let today = day_of(db::now_ms());
        if *date != date_of(today) {
            return Err(AppError::bad_request("only today's daily drill can be started"));
        }
        // One daily set per day: starting it again resumes it.
        if let Some((_, id, _)) = state.db.daily_sets().await?.into_iter().find(|(d, _, _)| d == date) {
            return Ok(Json(state.db.drill_set(&id).await?));
        }
        let items = daily(Bank::builtin(), today).into_iter().map(|d| (d.item, d.bank_id)).collect();
        return Ok(Json(state.db.create_drill_set(daily_technique(today), &source, 1500, items).await?));
    }
    let rating = player_rating(&state).await?;
    let (technique, seed) = match &source {
        DrillSource::Mistake { analysis_id, ply } => {
            let m = state.db.mistake_seed(analysis_id, *ply).await?.ok_or_else(|| AppError::not_found("no such mistake"))?;
            let (tag, kind) = m
                .tags
                .iter()
                .find(|(t, _)| drillable(t))
                .ok_or_else(|| AppError::bad_request("this mistake has no technique to drill"))?;
            // The player's own position, redrawn: find the idea they missed, or
            // hold where they walked into it (their move is the trap).
            let seed = match kind.as_str() {
                "missed" if !m.best_uci.is_empty() => {
                    Some(Seed { fen: m.fen.clone(), solution_uci: vec![m.best_uci.clone()], allowed: false })
                }
                "allowed" if !m.played_uci.is_empty() => {
                    Some(Seed { fen: m.fen.clone(), solution_uci: vec![m.played_uci.clone()], allowed: true })
                }
                _ => None,
            };
            (tag.clone(), seed)
        }
        DrillSource::Puzzle { id } => {
            let p = state.db.puzzle(id).await?;
            let tag = p
                .themes
                .iter()
                .find(|t| drillable(t))
                .ok_or_else(|| AppError::bad_request("this puzzle has no technique to drill"))?;
            let seed = p
                .solution_uci
                .first()
                .map(|u| Seed { fen: p.fen.clone(), solution_uci: vec![u.clone()], allowed: false });
            (tag.clone(), seed)
        }
        DrillSource::Theme { tag } => {
            if !drillable(tag) {
                return Err(AppError::bad_request(format!("{tag} can't be drilled")));
            }
            (tag.clone(), None)
        }
        DrillSource::Daily { .. } => unreachable!("handled above"),
        DrillSource::Weakest => {
            let tag = focus(&state, rating).await?.into_iter().next().unwrap_or_else(|| "fork".into());
            (tag, None)
        }
    };
    let req = DrillRequest {
        technique: technique.clone(),
        rating,
        seed,
        exclude: state.db.recent_bank_ids().await?,
        rng: db::now_ms() as u64,
    };
    let items = compose(&state.pool, Bank::builtin(), &req).await?;
    if items.is_empty() {
        return Err(AppError::unavailable("could not build a drill set"));
    }
    let items = items.into_iter().map(|d| (d.item, d.bank_id)).collect();
    Ok(Json(state.db.create_drill_set(&technique, &source, rating, items).await?))
}

/// Guest drills: build a set on a technique without an account. Nothing is
/// stored; the browser keeps the set and grades it. Engine time is shared, so
/// each address gets a few sets an hour and only two build at once.
pub async fn try_set(
    axum::extract::State(state): axum::extract::State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<GuestDrillRequest>,
) -> ApiResult<Json<DrillSet>> {
    if !drillable(&req.tag) {
        return Err(AppError::bad_request(format!("{} can't be drilled", req.tag)));
    }
    let ip = headers
        .get("cf-connecting-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').next().unwrap_or("").trim().to_string())
        .unwrap_or_default();
    if !guest_allowance(&ip) {
        return Err(AppError(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "That's the guest limit for this hour. Create a free account to keep drilling.".into(),
        ));
    }
    let _permit = guest_slots().acquire().await.map_err(|_| AppError::unavailable("busy"))?;
    let rating = req.rating.unwrap_or(1200).clamp(400, 2800);
    let draft = DrillRequest { technique: req.tag.clone(), rating, seed: None, exclude: Default::default(), rng: db::now_ms() as u64 };
    let items = compose(&state.pool, Bank::builtin(), &draft).await?;
    if items.is_empty() {
        return Err(AppError::unavailable("could not build a drill set"));
    }
    let items = items
        .into_iter()
        .enumerate()
        .map(|(n, d)| api_types::DrillItem { id: format!("g{n}"), ..d.item })
        .collect();
    Ok(Json(DrillSet {
        id: format!("guest-{}", db::now_ms()),
        label: coach::tags::label(&req.tag),
        technique: req.tag.clone(),
        source: DrillSource::Theme { tag: req.tag },
        rating,
        items,
        created_at: db::now_ms(),
        completed_at: None,
    }))
}

#[derive(serde::Deserialize)]
pub struct GuestDrillRequest {
    tag: String,
    rating: Option<u32>,
}

/// Guest sets per address per hour.
const GUEST_PER_HOUR: usize = 6;

fn guest_allowance(ip: &str) -> bool {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    static SEEN: OnceLock<Mutex<HashMap<String, Vec<Instant>>>> = OnceLock::new();
    let mut seen = SEEN.get_or_init(Default::default).lock().unwrap();
    let hour = Duration::from_secs(3600);
    seen.retain(|_, v| {
        v.retain(|t| t.elapsed() < hour);
        !v.is_empty()
    });
    let v = seen.entry(ip.to_string()).or_default();
    if v.len() >= GUEST_PER_HOUR {
        return false;
    }
    v.push(Instant::now());
    true
}

fn guest_slots() -> &'static tokio::sync::Semaphore {
    static SLOTS: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    SLOTS.get_or_init(|| tokio::sync::Semaphore::new(2))
}

pub async fn get(UserState(state, _): UserState, Path(id): Path<String>) -> ApiResult<Json<DrillSet>> {
    Ok(Json(state.db.drill_set(&id).await?))
}

pub async fn attempt(
    UserState(state, _): UserState,
    Path((id, item)): Path<(String, String)>,
    Json(a): Json<DrillAttempt>,
) -> ApiResult<Json<DrillSet>> {
    let before = state.db.drill_set(&id).await?;
    let first_try = before.items.iter().find(|i| i.id == item).is_some_and(|i| i.solved.is_none());
    let set = state.db.record_drill_item(&id, &item, &a).await?;
    // A missed find gets an easier follow-up on the same idea, straight after.
    let missed = set.items.iter().find(|i| i.id == item).filter(|i| i.kind == DrillKind::Find);
    if let Some(m) = missed.filter(|_| first_try && !a.solved && set.items.len() < MAX_ITEMS) {
        let rating = m.rating.map(|r| r as i32).unwrap_or(set.rating as i32 + coach::bank::PUZZLE_RATING_OFFSET);
        let mut exclude = state.db.recent_bank_ids().await?;
        exclude.extend(state.db.drill_bank_ids(&id).await?);
        if let Some(d) = easier_find(Bank::builtin(), &set.technique, rating, &exclude, db::now_ms() as u64) {
            return Ok(Json(state.db.insert_drill_item_after(&id, &item, d.item, d.bank_id).await?));
        }
    }
    Ok(Json(set))
}

/// The motifs behind the user's mistakes in an analysis (for "Drill this").
pub async fn analysis_motifs(UserState(state, _): UserState, Path(id): Path<String>) -> ApiResult<Json<Vec<MistakeMotif>>> {
    state.db.analysis(&id).await?; // ownership check
    Ok(Json(state.db.mistake_motifs(&id).await?))
}
