//! User accounts (email + password, or Lichess), browser sessions, and
//! share links.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use rand::RngCore;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::{Db, DbError, Result, new_id, now_ms};

pub const SESSION_DAYS: i64 = 30;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Account {
    pub id: String,
    pub display_name: String,
    pub email: Option<String>,
    pub lichess_username: Option<String>,
    pub kind: String,
}

/// 32 random bytes, hex encoded.
pub fn random_token() -> String {
    let mut b = [0u8; 32];
    rand::rng().fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Tokens are stored only as SHA-256 hashes.
pub fn hash_token(t: &str) -> String {
    Sha256::digest(t.as_bytes()).iter().map(|x| format!("{x:02x}")).collect()
}

fn row_to_account(r: &SqliteRow) -> Result<Account> {
    Ok(Account {
        id: r.try_get("id")?,
        display_name: r.try_get("display_name")?,
        email: r.try_get("email")?,
        lichess_username: r.try_get("lichess_username")?,
        kind: r.try_get("kind")?,
    })
}

fn normalize_email(e: &str) -> Result<String> {
    let e = e.trim().to_lowercase();
    let ok = e.len() <= 254 && e.split_once('@').is_some_and(|(a, d)| !a.is_empty() && d.contains('.'));
    if !ok {
        return Err(DbError::Invalid("enter a valid email address".into()));
    }
    Ok(e)
}

impl Db {
    async fn insert_account(
        &self,
        display_name: &str,
        email: Option<&str>,
        password_hash: Option<&str>,
        lichess: Option<(&str, &str)>,
    ) -> Result<Account> {
        let id = new_id();
        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO users (id, kind, display_name, created_at, email, password_hash, lichess_id, lichess_username)
             VALUES (?, 'account', ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(display_name)
        .bind(now)
        .bind(email)
        .bind(password_hash)
        .bind(lichess.map(|l| l.0))
        .bind(lichess.map(|l| l.1))
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO user_settings (user_id, lichess_username, updated_at) VALUES (?, ?, ?)")
            .bind(&id)
            .bind(lichess.map(|l| l.1))
            .bind(now)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.account(&id).await
    }

    pub async fn account(&self, id: &str) -> Result<Account> {
        let r = sqlx::query("SELECT * FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(DbError::NotFound)?;
        row_to_account(&r)
    }

    pub async fn register(&self, email: &str, password: &str, display_name: Option<&str>) -> Result<Account> {
        let email = normalize_email(email)?;
        if password.chars().count() < 8 {
            return Err(DbError::Invalid("use at least 8 characters for the password".into()));
        }
        let exists: Option<String> = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
            .bind(&email)
            .fetch_optional(&self.pool)
            .await?;
        if exists.is_some() {
            return Err(DbError::Invalid("an account with that email already exists".into()));
        }
        let salt = SaltString::encode_b64(&{
            let mut b = [0u8; 16];
            rand::rng().fill_bytes(&mut b);
            b
        })
        .map_err(|e| DbError::Crypto(e.to_string()))?;
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| DbError::Crypto(e.to_string()))?
            .to_string();
        let name = display_name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(String::from)
            .unwrap_or_else(|| email.split('@').next().unwrap_or("player").to_string());
        self.insert_account(&name, Some(&email), Some(&hash), None).await
    }

    /// Check email + password. Returns `NotFound` on any mismatch.
    pub async fn verify_login(&self, email: &str, password: &str) -> Result<Account> {
        let email = normalize_email(email).map_err(|_| DbError::NotFound)?;
        let r = sqlx::query("SELECT * FROM users WHERE email = ?")
            .bind(&email)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(DbError::NotFound)?;
        let hash: Option<String> = r.try_get("password_hash")?;
        let hash = hash.ok_or(DbError::NotFound)?;
        let parsed = PasswordHash::new(&hash).map_err(|e| DbError::Crypto(e.to_string()))?;
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .map_err(|_| DbError::NotFound)?;
        row_to_account(&r)
    }

    /// Find or create the account for a Lichess user, and store their token
    /// (it also unlocks username import and the opening explorer).
    pub async fn lichess_login(&self, lichess_id: &str, username: &str, token: &str) -> Result<Account> {
        let existing: Option<String> = sqlx::query_scalar("SELECT id FROM users WHERE lichess_id = ?")
            .bind(lichess_id)
            .fetch_optional(&self.pool)
            .await?;
        let acct = match existing {
            Some(id) => {
                sqlx::query("UPDATE users SET lichess_username = ? WHERE id = ?")
                    .bind(username)
                    .bind(&id)
                    .execute(&self.pool)
                    .await?;
                self.account(&id).await?
            }
            None => self.insert_account(username, None, None, Some((lichess_id, username))).await?,
        };
        let enc = self.keyring.encrypt(token)?;
        sqlx::query(
            "UPDATE user_settings SET lichess_token_enc = ?, lichess_username = COALESCE(lichess_username, ?) WHERE user_id = ?",
        )
        .bind(enc)
        .bind(username)
        .bind(&acct.id)
        .execute(&self.pool)
        .await?;
        Ok(acct)
    }

    pub async fn create_session(&self, user_id: &str) -> Result<String> {
        let token = random_token();
        let now = now_ms();
        sqlx::query("INSERT INTO sessions (token_hash, user_id, created_at, expires_at) VALUES (?, ?, ?, ?)")
            .bind(hash_token(&token))
            .bind(user_id)
            .bind(now)
            .bind(now + SESSION_DAYS * 86_400_000)
            .execute(&self.pool)
            .await?;
        Ok(token)
    }

    pub async fn session_user(&self, token: &str) -> Result<Option<Account>> {
        let id: Option<String> = sqlx::query_scalar("SELECT user_id FROM sessions WHERE token_hash = ? AND expires_at > ?")
            .bind(hash_token(token))
            .bind(now_ms())
            .fetch_optional(&self.pool)
            .await?;
        match id {
            Some(id) => Ok(Some(self.account(&id).await?)),
            None => Ok(None),
        }
    }

    pub async fn delete_session(&self, token: &str) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?").bind(hash_token(token)).execute(&self.pool).await?;
        Ok(())
    }

    /// Share token for one of this user's games, created on first use.
    pub async fn share_token(&self, game_id: &str) -> Result<String> {
        let current: Option<Option<String>> = sqlx::query_scalar("SELECT share_token FROM games WHERE user_id = ? AND id = ?")
            .bind(&self.user_id)
            .bind(game_id)
            .fetch_optional(&self.pool)
            .await?;
        match current {
            None => Err(DbError::NotFound),
            Some(Some(t)) => Ok(t),
            Some(None) => {
                let t = random_token()[..24].to_string();
                sqlx::query("UPDATE games SET share_token = ? WHERE id = ?").bind(&t).bind(game_id).execute(&self.pool).await?;
                Ok(t)
            }
        }
    }

    /// Resolve a share token to (owner, game id).
    pub async fn shared_game(&self, token: &str) -> Result<(String, String)> {
        let r = sqlx::query("SELECT user_id, id FROM games WHERE share_token = ?")
            .bind(token)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(DbError::NotFound)?;
        Ok((r.try_get("user_id")?, r.try_get("id")?))
    }

    /// Copy a shared game into this user's library (analysis is re-run).
    pub async fn claim_shared(&self, token: &str) -> Result<String> {
        let r = sqlx::query("SELECT * FROM games WHERE share_token = ?")
            .bind(token)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(DbError::NotFound)?;
        let owner: String = r.try_get("user_id")?;
        if owner == self.user_id {
            return Ok(r.try_get("id")?);
        }
        let pgn: String = r.try_get("pgn")?;
        let parsed = chess_core::pgn::parse_pgn(&pgn).map_err(|e| DbError::Invalid(e.to_string()))?;
        let source = api_types::GameSource::parse(&r.try_get::<String, _>("source")?);
        let source_id: Option<String> = r.try_get("source_id")?;
        let (g, _) = self.insert_game(crate::NewGame { source, source_id, pgn, parsed }).await?;
        Ok(g.id)
    }
}
