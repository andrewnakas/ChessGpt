use api_types::{Settings, SettingsInput};
use sqlx::Row;

use crate::{Db, Result, now_ms};

impl Db {
    pub async fn settings(&self) -> Result<Settings> {
        let r = sqlx::query(
            "SELECT elo, lichess_username, chesscom_username, explorer_enabled, lichess_token_enc
             FROM user_settings WHERE user_id = ?",
        )
        .bind(&self.user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(Settings {
            elo: r.try_get::<i64, _>("elo")? as u32,
            lichess_username: r.try_get("lichess_username")?,
            chesscom_username: r.try_get("chesscom_username")?,
            explorer_enabled: r.try_get::<i64, _>("explorer_enabled")? != 0,
            has_lichess_token: r.try_get::<Option<String>, _>("lichess_token_enc")?.is_some(),
        })
    }

    pub async fn put_settings(&self, s: &SettingsInput) -> Result<Settings> {
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
        if let Some(t) = s.lichess_token.as_deref().map(str::trim) {
            let enc = if t.is_empty() { None } else { Some(self.keyring.encrypt(t)?) };
            sqlx::query("UPDATE user_settings SET lichess_token_enc = ? WHERE user_id = ?")
                .bind(enc)
                .bind(&self.user_id)
                .execute(&self.pool)
                .await?;
        }
        self.settings().await
    }

    pub async fn lichess_token(&self) -> Result<Option<String>> {
        let enc: Option<String> = sqlx::query_scalar("SELECT lichess_token_enc FROM user_settings WHERE user_id = ?")
            .bind(&self.user_id)
            .fetch_one(&self.pool)
            .await?;
        enc.map(|e| self.keyring.decrypt(&e)).transpose()
    }

    pub async fn set_chesscom_username(&self, username: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE user_settings SET chesscom_username = ?, updated_at = ? WHERE user_id = ?")
            .bind(username)
            .bind(now_ms())
            .bind(&self.user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Users with a Lichess or Chess.com username to keep in sync.
    pub async fn linked_users(&self) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT user_id FROM user_settings WHERE COALESCE(lichess_username, '') != '' OR COALESCE(chesscom_username, '') != ''",
        )
        .fetch_all(&self.pool)
        .await?)
    }
}
