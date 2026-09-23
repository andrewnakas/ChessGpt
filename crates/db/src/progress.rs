//! The user's analysed games and mistake rates over time (progress page).

use api_types::{Phase, ProgressGame, Side};
use sqlx::Row;

use crate::games::{parse_side, side_str};
use crate::{Db, Result};

/// Latest finished analysis of each of the user's games with a known side.
const LATEST_DONE: &str = "SELECT g.id AS gid, g.white, g.black, g.white_elo, g.black_elo, g.result, g.date,
        g.time_control, g.user_side, g.imported_at, a.id AS aid, a.white_accuracy, a.black_accuracy,
        a.white_estimate, a.black_estimate
    FROM games g
    JOIN game_analyses a ON a.id = (
        SELECT id FROM game_analyses WHERE game_id = g.id AND status = 'done' ORDER BY created_at DESC LIMIT 1)
    WHERE g.user_id = ? AND g.user_side IS NOT NULL";

fn user_score(result: &str, side: Side) -> Option<f64> {
    let white = match result {
        "1-0" => 1.0,
        "0-1" => 0.0,
        "1/2-1/2" => 0.5,
        _ => return None,
    };
    Some(if side == Side::White { white } else { 1.0 - white })
}

impl Db {
    /// Analysed games where the user's side is known, oldest first.
    pub async fn progress_games(&self) -> Result<Vec<ProgressGame>> {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{LATEST_DONE} ORDER BY COALESCE(g.date, ''), g.imported_at"
        )))
        .bind(&self.user_id)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let Some(side) = r.try_get::<Option<String>, _>("user_side")?.as_deref().and_then(parse_side) else {
                continue;
            };
            let aid: String = r.try_get("aid")?;
            let (moves, errors): (i64, i64) = sqlx::query_as(
                "SELECT COUNT(*), COALESCE(SUM(classification IN ('mistake', 'blunder', 'missed_win')), 0)
                 FROM move_evals WHERE analysis_id = ? AND json_extract(data_json, '$.mover') = ?",
            )
            .bind(&aid)
            .bind(side_str(side))
            .fetch_one(&self.pool)
            .await?;
            let white = side == Side::White;
            let pick = |w: &str, b: &str| if white { w.to_string() } else { b.to_string() };
            let elo = |c: &str| -> Result<Option<u32>> { Ok(r.try_get::<Option<i64>, _>(c)?.map(|v| v as u32)) };
            out.push(ProgressGame {
                game_id: r.try_get("gid")?,
                date: r.try_get("date")?,
                imported_at: r.try_get("imported_at")?,
                user_side: side,
                opponent: r.try_get(pick("black", "white").as_str())?,
                user_elo: elo(&pick("white_elo", "black_elo"))?,
                opponent_elo: elo(&pick("black_elo", "white_elo"))?,
                score: user_score(&r.try_get::<String, _>("result")?, side),
                time_control: r.try_get("time_control")?,
                accuracy: r.try_get(pick("white_accuracy", "black_accuracy").as_str())?,
                estimate: elo(&pick("white_estimate", "black_estimate"))?,
                moves: moves as u32,
                errors: errors as u32,
            });
        }
        Ok(out)
    }

    /// Mistakes per motif tag and how they were found: (tag, kind, count).
    pub async fn mistake_motif_counts(&self) -> Result<Vec<(String, String, i64)>> {
        // Only each game's latest finished analysis counts, and only games
        // whose side is known (like the rest of the progress data).
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT mi.concept_tag, mi.motif_kind, COUNT(DISTINCT mi.analysis_id || ':' || mi.ply) AS n
             FROM mistake_index mi JOIN ({LATEST_DONE}) x ON mi.analysis_id = x.aid
             WHERE mi.user_id = ? AND mi.concept_tag != '' GROUP BY mi.concept_tag, mi.motif_kind ORDER BY n DESC"
        )))
        .bind(&self.user_id)
        .bind(&self.user_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(|r| Ok((r.try_get(0)?, r.try_get(1)?, r.try_get("n")?))).collect()
    }

    /// The user's moves and errors per game phase, over their analysed games.
    pub async fn phase_counts(&self) -> Result<Vec<(Phase, u32, u32)>> {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT me.phase AS phase, COUNT(*) AS moves,
                    COALESCE(SUM(me.classification IN ('mistake', 'blunder', 'missed_win')), 0) AS errors
             FROM move_evals me JOIN ({LATEST_DONE}) x ON me.analysis_id = x.aid
             WHERE json_extract(me.data_json, '$.mover') = x.user_side
             GROUP BY me.phase"
        )))
        .bind(&self.user_id)
        .fetch_all(&self.pool)
        .await?;
        let mut out = vec![];
        for r in rows {
            let phase = match r.try_get::<String, _>("phase")?.as_str() {
                "opening" => Phase::Opening,
                "middlegame" => Phase::Middlegame,
                _ => Phase::Endgame,
            };
            out.push((phase, r.try_get::<i64, _>("moves")? as u32, r.try_get::<i64, _>("errors")? as u32));
        }
        Ok(out)
    }
}
