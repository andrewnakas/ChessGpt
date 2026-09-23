use api_types::{GameSource, GameSummary, JobStatus, Side};
use chess_core::book::book_prefix;
use chess_core::pgn::ParsedGame;
use chess_core::position::{fen_key, parse_fen, phase};
use shakmaty::Position;
use sqlx::AssertSqlSafe;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, Transaction};

use crate::{Db, DbError, Result, new_id, now_ms};

pub struct NewGame {
    pub source: GameSource,
    /// Stable id at the source (lichess game id, chess.com URL). PGN pastes
    /// use a hash of the movetext so re-pasting is idempotent.
    pub source_id: Option<String>,
    pub pgn: String,
    pub parsed: ParsedGame,
}

#[derive(Debug, Clone)]
pub struct GameRecord {
    pub summary: GameSummary,
    pub pgn: String,
    pub parsed: ParsedGame,
}

const SUMMARY_SQL: &str = "SELECT g.*, a.status AS a_status, a.white_accuracy AS a_wacc, a.black_accuracy AS a_bacc
    FROM games g
    LEFT JOIN game_analyses a ON a.id = (
        SELECT id FROM game_analyses WHERE game_id = g.id ORDER BY created_at DESC LIMIT 1)";

fn row_to_summary(r: &SqliteRow) -> Result<GameSummary> {
    let side: Option<String> = r.try_get("user_side")?;
    let status: Option<String> = r.try_get("a_status")?;
    Ok(GameSummary {
        id: r.try_get("id")?,
        source: GameSource::parse(&r.try_get::<String, _>("source")?),
        source_id: r.try_get("source_id")?,
        white: r.try_get("white")?,
        black: r.try_get("black")?,
        white_elo: r.try_get::<Option<i64>, _>("white_elo")?.map(|v| v as u32),
        black_elo: r.try_get::<Option<i64>, _>("black_elo")?.map(|v| v as u32),
        result: r.try_get("result")?,
        date: r.try_get("date")?,
        time_control: r.try_get("time_control")?,
        eco: r.try_get("eco")?,
        opening: r.try_get("opening_name")?,
        user_side: side.as_deref().and_then(parse_side),
        ply_count: r.try_get::<i64, _>("ply_count")? as u32,
        analysis_status: status.as_deref().map(JobStatus::parse),
        white_accuracy: r.try_get("a_wacc")?,
        black_accuracy: r.try_get("a_bacc")?,
        imported_at: r.try_get("imported_at")?,
    })
}

pub(crate) fn parse_side(s: &str) -> Option<Side> {
    match s {
        "white" => Some(Side::White),
        "black" => Some(Side::Black),
        _ => None,
    }
}

pub(crate) fn side_str(s: Side) -> &'static str {
    match s {
        Side::White => "white",
        Side::Black => "black",
    }
}

async fn upsert_position(tx: &mut Transaction<'_, Sqlite>, fen: &str) -> Result<i64> {
    let pos = parse_fen(fen).map_err(|e| DbError::Invalid(e.to_string()))?;
    let pieces = pos.board().occupied().count() as i64;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO positions (fen_key, phase, piece_count) VALUES (?, ?, ?)
         ON CONFLICT(fen_key) DO UPDATE SET piece_count = excluded.piece_count RETURNING id",
    )
    .bind(fen_key(&pos))
    .bind(phase(&pos).as_str())
    .bind(pieces)
    .fetch_one(&mut **tx)
    .await?;
    Ok(id)
}

impl Db {
    pub async fn position_id(&self, fen: &str) -> Result<i64> {
        let mut tx = self.pool.begin().await?;
        let id = upsert_position(&mut tx, fen).await?;
        tx.commit().await?;
        Ok(id)
    }

    /// Insert a game. Returns `(summary, inserted)`; `inserted` is false when
    /// the same source game already exists (the existing row is returned).
    pub async fn insert_game(&self, g: NewGame) -> Result<(GameSummary, bool)> {
        if let Some(sid) = &g.source_id {
            let existing: Option<String> = sqlx::query_scalar(
                "SELECT id FROM games WHERE user_id = ? AND source = ? AND source_id = ?",
            )
            .bind(&self.user_id)
            .bind(g.source.as_str())
            .bind(sid)
            .fetch_optional(&self.pool)
            .await?;
            if let Some(id) = existing {
                return Ok((self.game_summary(&id).await?, false));
            }
        }
        let settings = self.settings().await?;
        let p = &g.parsed;
        let name = |t: &str| p.tag(t).unwrap_or("?").to_string();
        let (white, black) = (name("White"), name("Black"));
        let matches = |n: &str| {
            [&settings.lichess_username, &settings.chesscom_username]
                .iter()
                .any(|u| u.as_deref().is_some_and(|u| u.eq_ignore_ascii_case(n)))
        };
        let user_side = if matches(&white) {
            Some(Side::White)
        } else if matches(&black) {
            Some(Side::Black)
        } else {
            None
        };
        let keys: Vec<String> = p
            .moves
            .iter()
            .filter_map(|m| parse_fen(&m.fen_after).ok().map(|pos| fen_key(&pos)))
            .collect();
        let (_, book_opening) = book_prefix(&keys);
        let eco = book_opening.as_ref().map(|o| o.eco.clone()).or_else(|| p.tag("ECO").map(String::from));
        let opening = book_opening
            .as_ref()
            .map(|o| o.name.clone())
            .or_else(|| p.tag("Opening").map(String::from));

        let id = new_id();
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO games (id, user_id, source, source_id, pgn, start_fen, white, black, white_elo, black_elo,
              result, date, time_control, eco, opening_name, user_side, ply_count, imported_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&id)
        .bind(&self.user_id)
        .bind(g.source.as_str())
        .bind(&g.source_id)
        .bind(&g.pgn)
        .bind(&p.start_fen)
        .bind(&white)
        .bind(&black)
        .bind(p.elo(Side::White).map(|v| v as i64))
        .bind(p.elo(Side::Black).map(|v| v as i64))
        .bind(p.tag("Result").unwrap_or("*"))
        .bind(p.tag("UTCDate").or(p.tag("Date")))
        .bind(p.tag("TimeControl"))
        .bind(eco)
        .bind(opening)
        .bind(user_side.map(side_str))
        .bind(p.moves.len() as i64)
        .bind(now_ms())
        .execute(&mut *tx)
        .await?;
        for m in &p.moves {
            let before = upsert_position(&mut tx, &m.fen_before).await?;
            let after = upsert_position(&mut tx, &m.fen_after).await?;
            sqlx::query(
                "INSERT INTO game_moves (game_id, ply, san, uci, position_before_id, position_after_id, clock_ms) VALUES (?,?,?,?,?,?,?)",
            )
            .bind(&id)
            .bind(m.ply as i64)
            .bind(&m.san)
            .bind(&m.uci)
            .bind(before)
            .bind(after)
            .bind(m.clock_ms.map(|c| c as i64))
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok((self.game_summary(&id).await?, true))
    }

    pub async fn game_summary(&self, id: &str) -> Result<GameSummary> {
        let r = sqlx::query(AssertSqlSafe(format!("{SUMMARY_SQL} WHERE g.user_id = ? AND g.id = ?")))
            .bind(&self.user_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(DbError::NotFound)?;
        row_to_summary(&r)
    }

    pub async fn list_games(&self, limit: u32, offset: u32) -> Result<Vec<GameSummary>> {
        let rows = sqlx::query(AssertSqlSafe(format!(
            "{SUMMARY_SQL} WHERE g.user_id = ? ORDER BY g.imported_at DESC, g.id DESC LIMIT ? OFFSET ?"
        )))
        .bind(&self.user_id)
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_summary).collect()
    }

    pub async fn game(&self, id: &str) -> Result<GameRecord> {
        let summary = self.game_summary(id).await?;
        let pgn: String = sqlx::query_scalar("SELECT pgn FROM games WHERE user_id = ? AND id = ?")
            .bind(&self.user_id)
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let parsed = chess_core::pgn::parse_pgn(&pgn).map_err(|e| DbError::Invalid(e.to_string()))?;
        Ok(GameRecord { summary, pgn, parsed })
    }

    /// Mark the user's side on games that don't have one yet, where a player
    /// name matches `username` (case-insensitive). Returns how many changed.
    pub async fn mark_side_by_name(&self, username: &str) -> Result<u64> {
        let mut n = 0;
        for (col, side) in [("white", "white"), ("black", "black")] {
            n += sqlx::query(sqlx::AssertSqlSafe(format!(
                "UPDATE games SET user_side = ? WHERE user_id = ? AND user_side IS NULL AND lower({col}) = lower(?)"
            )))
            .bind(side)
            .bind(&self.user_id)
            .bind(username)
            .execute(&self.pool)
            .await?
            .rows_affected();
        }
        Ok(n)
    }

    pub async fn set_user_side(&self, id: &str, side: Option<Side>) -> Result<()> {
        sqlx::query("UPDATE games SET user_side = ? WHERE user_id = ? AND id = ?")
            .bind(side.map(side_str))
            .bind(&self.user_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_game(&self, id: &str) -> Result<()> {
        let n = sqlx::query("DELETE FROM games WHERE user_id = ? AND id = ?")
            .bind(&self.user_id)
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if n == 0 {
            return Err(DbError::NotFound);
        }
        Ok(())
    }
}
