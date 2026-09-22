use api_types::Settings;
use sqlx::Row;

use crate::{Db, Result, now_ms};

impl Db {
    pub async fn settings(&self) -> Result<Settings> {
        let r = sqlx::query(
            "SELECT elo, lichess_username, chesscom_username, explorer_enabled FROM user_settings WHERE user_id = ?",
        )
        .bind(&self.user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(Settings {
            elo: r.try_get::<i64, _>("elo")? as u32,
            lichess_username: r.try_get("lichess_username")?,
            chesscom_username: r.try_get("chesscom_username")?,
            explorer_enabled: r.try_get::<i64, _>("explorer_enabled")? != 0,
        })
    }

    pub async fn put_settings(&self, s: &Settings) -> Result<Settings> {
        let clean = |v: &Option<String>| v.as_ref().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        sqlx::query(
            "UPDATE user_settings SET elo = ?, lichess_username = ?, chesscom_username = ?, explorer_enabled = ?, updated_at = ? WHERE user_id = ?",
        )
        .bind(s.elo.clamp(100, 3500) as i64)
        .bind(clean(&s.lichess_username))
        .bind(clean(&s.chesscom_username))
        .bind(s.explorer_enabled as i64)
        .bind(now_ms())
        .bind(&self.user_id)
        .execute(&self.pool)
        .await?;
        self.settings().await
    }
}
