//! SQLite persistence (sqlx). Single local user by default; every table is
//! keyed by user so hosted multi-user mode is a later addition, not a rewrite.

mod accounts;
mod analysis;
mod chat;
mod oauth;
mod games;
mod keyring;
mod providers;
mod settings;

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

pub use accounts::{Account, hash_token, random_token};

pub fn accounts_session_seconds() -> i64 {
    accounts::SESSION_DAYS * 86_400
}
pub use analysis::NewAnalysis;
pub use oauth::{OAuthClient, OAuthGrant};
pub use games::{GameRecord, NewGame};
pub use keyring::KeyRing;
pub use providers::ProviderRecord;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error(transparent)]
    Sql(#[from] sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Invalid(String),
    #[error("key storage: {0}")]
    Crypto(String),
}

pub type Result<T> = std::result::Result<T, DbError>;

#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
    keyring: KeyRing,
    user_id: String,
}

pub const LOCAL_USER: &str = "local";

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

impl Db {
    /// Open (creating if needed) `chessgpt.db` and `keyring.key` in `data_dir`.
    pub async fn open(data_dir: &Path) -> Result<Db> {
        std::fs::create_dir_all(data_dir)?;
        let keyring = KeyRing::load_or_create(&data_dir.join("keyring.key"))?;
        let url = format!("sqlite://{}", data_dir.join("chessgpt.db").display());
        Self::open_url(&url, keyring).await
    }

    /// In-memory database for tests.
    pub async fn open_memory() -> Result<Db> {
        Self::open_url("sqlite::memory:", KeyRing::ephemeral()).await
    }

    async fn open_url(url: &str, keyring: KeyRing) -> Result<Db> {
        let opts = SqliteConnectOptions::from_str(url)?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(10));
        let max = if url.contains(":memory:") { 1 } else { 8 };
        let pool = SqlitePoolOptions::new().max_connections(max).connect_with(opts).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        let db = Db { pool, keyring, user_id: LOCAL_USER.to_string() };
        db.ensure_local_user().await?;
        Ok(db)
    }

    async fn ensure_local_user(&self) -> Result<()> {
        let now = now_ms();
        sqlx::query("INSERT OR IGNORE INTO users (id, kind, display_name, created_at) VALUES (?, 'local', 'You', ?)")
            .bind(&self.user_id)
            .bind(now)
            .execute(&self.pool)
            .await?;
        sqlx::query("INSERT OR IGNORE INTO user_settings (user_id, updated_at) VALUES (?, ?)")
            .bind(&self.user_id)
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    /// The same database, scoped to another user. Every per-user query goes
    /// through `self.user_id`, so this is how requests are isolated.
    pub fn for_user(&self, user_id: &str) -> Db {
        Db { pool: self.pool.clone(), keyring: self.keyring.clone(), user_id: user_id.to_string() }
    }

    pub fn keyring(&self) -> &KeyRing {
        &self.keyring
    }
}

#[cfg(test)]
mod tests;
