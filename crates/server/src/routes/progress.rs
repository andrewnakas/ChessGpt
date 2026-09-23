//! Progress page data: rating trend and what the user's mistakes are made of.

use std::collections::BTreeMap;

use api_types::{MotifRate, PhaseRate, Progress};
use axum::Json;

use crate::auth::UserState;
use crate::error::ApiResult;

fn per_100(n: u32, moves: u32) -> f64 {
    if moves == 0 { 0.0 } else { (n as f64 * 1000.0 / moves as f64).round() / 10.0 }
}

pub async fn get(UserState(state, _): UserState) -> ApiResult<Json<Progress>> {
    let db = &state.db;
    let games = db.progress_games().await?;
    let estimates: Vec<u32> = games.iter().filter_map(|g| g.estimate).collect();
    let rated: Vec<(u32, f64)> = games.iter().filter_map(|g| Some((g.opponent_elo?, g.score?))).collect();
    let recent = &rated[rated.len().saturating_sub(20)..];

    let phases_raw = db.phase_counts().await?;
    let user_moves: u32 = phases_raw.iter().map(|(_, m, _)| m).sum();
    let phases = phases_raw
        .into_iter()
        .map(|(phase, moves, errors)| PhaseRate { phase, moves, errors, per_100_moves: per_100(errors, moves) })
        .collect();

    let mut by_tag: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    for (tag, kind, n) in db.mistake_motif_counts().await? {
        let e = by_tag.entry(tag).or_default();
        match kind.as_str() {
            "allowed" => e.1 += n as u32,
            _ => e.0 += n as u32,
        }
    }
    let mut motifs: Vec<MotifRate> = by_tag
        .into_iter()
        .map(|(tag, (missed, allowed))| MotifRate {
            label: coach::tags::label(&tag),
            per_100_moves: per_100(missed + allowed, user_moves),
            baseline_per_100: None,
            tag,
            missed,
            allowed,
        })
        .collect();
    motifs.sort_by_key(|m| std::cmp::Reverse(m.missed + m.allowed));

    let rolling = chess_core::rating::rolling(&estimates);
    Ok(Json(Progress {
        rolling_estimate: rolling.map(|r| r.0),
        estimate_margin: rolling.map(|r| r.1),
        performance: chess_core::rating::performance(recent),
        games,
        user_moves,
        motifs,
        phases,
    }))
}
