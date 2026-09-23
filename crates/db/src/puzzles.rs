//! Training puzzles and their spaced-repetition schedule.

use api_types::Puzzle;
use chess_core::srs::{Card, due_ms, review};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::{Db, DbError, Result, new_id, now_ms};

pub struct NewPuzzle {
    pub game_id: String,
    pub analysis_id: String,
    pub ply: u32,
    pub fen: String,
    pub solution_uci: Vec<String>,
    pub line_san: Vec<String>,
    pub themes: Vec<String>,
}

fn row_to_puzzle(r: &SqliteRow) -> Result<Puzzle> {
    Ok(Puzzle {
        id: r.try_get("id")?,
        source: r.try_get("source")?,
        game_id: r.try_get("game_id")?,
        analysis_id: r.try_get("analysis_id")?,
        ply: r.try_get::<Option<i64>, _>("ply")?.map(|p| p as u32),
        fen: r.try_get("fen")?,
        solution_uci: serde_json::from_str(&r.try_get::<String, _>("solution_json")?)?,
        line_san: serde_json::from_str(&r.try_get::<String, _>("line_json")?)?,
        themes: serde_json::from_str(&r.try_get::<String, _>("themes_json")?)?,
        reps: r.try_get::<i64, _>("reps")? as u32,
        lapses: r.try_get::<i64, _>("lapses")? as u32,
        due_at: r.try_get("due_at")?,
    })
}

impl Db {
    /// Add a puzzle from one of the user's mistakes (once per game and ply);
    /// it is due at once.
    pub async fn add_puzzle(&self, p: &NewPuzzle) -> Result<bool> {
        let now = now_ms();
        let r = sqlx::query(
            "INSERT OR IGNORE INTO puzzles (id, user_id, source, game_id, analysis_id, ply, fen, solution_json,
               line_json, themes_json, due_at, created_at)
             VALUES (?, ?, 'mistake', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(new_id())
        .bind(&self.user_id)
        .bind(&p.game_id)
        .bind(&p.analysis_id)
        .bind(p.ply as i64)
        .bind(&p.fen)
        .bind(serde_json::to_string(&p.solution_uci)?)
        .bind(serde_json::to_string(&p.line_san)?)
        .bind(serde_json::to_string(&p.themes)?)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() == 1)
    }

    /// Puzzles due now, most overdue first.
    pub async fn due_puzzles(&self, limit: u32) -> Result<Vec<Puzzle>> {
        let rows = sqlx::query("SELECT * FROM puzzles WHERE user_id = ? AND due_at <= ? ORDER BY due_at LIMIT ?")
            .bind(&self.user_id)
            .bind(now_ms())
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(row_to_puzzle).collect()
    }

    /// (total, due now, learned) for the user's puzzles.
    pub async fn puzzle_counts(&self) -> Result<(u32, u32, u32)> {
        let (t, d, l): (i64, i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), COALESCE(SUM(due_at <= ?), 0), COALESCE(SUM(reps >= 2), 0) FROM puzzles WHERE user_id = ?",
        )
        .bind(now_ms())
        .bind(&self.user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok((t as u32, d as u32, l as u32))
    }

    /// Record a solve or a miss and reschedule the puzzle.
    pub async fn record_attempt(&self, id: &str, solved: bool) -> Result<Puzzle> {
        let r = sqlx::query("SELECT * FROM puzzles WHERE user_id = ? AND id = ?")
            .bind(&self.user_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(DbError::NotFound)?;
        let card = Card {
            interval_days: r.try_get("interval_days")?,
            ease: r.try_get("ease")?,
            reps: r.try_get::<i64, _>("reps")? as u32,
            lapses: r.try_get::<i64, _>("lapses")? as u32,
        };
        let next = review(card, solved);
        let now = now_ms();
        sqlx::query(
            "UPDATE puzzles SET interval_days = ?, ease = ?, reps = ?, lapses = ?, attempts = attempts + 1, due_at = ?
             WHERE user_id = ? AND id = ?",
        )
        .bind(next.interval_days)
        .bind(next.ease)
        .bind(next.reps as i64)
        .bind(next.lapses as i64)
        .bind(due_ms(&next, now))
        .bind(&self.user_id)
        .bind(id)
        .execute(&self.pool)
        .await?;
        let r = sqlx::query("SELECT * FROM puzzles WHERE id = ?").bind(id).fetch_one(&self.pool).await?;
        row_to_puzzle(&r)
    }
}
