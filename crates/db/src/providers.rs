use api_types::{Provider, ProviderInput, ProviderKind};
use sqlx::Row;
use sqlx::AssertSqlSafe;
use sqlx::sqlite::SqliteRow;

use crate::{Db, DbError, Result, new_id, now_ms};

/// A provider with its decrypted key, for server-side use only.
#[derive(Clone)]
pub struct ProviderRecord {
    pub provider: Provider,
    pub api_key: Option<String>,
}

impl std::fmt::Debug for ProviderRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderRecord")
            .field("provider", &self.provider)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

fn row_to_provider(r: &SqliteRow) -> Result<(Provider, Option<String>)> {
    let kind: String = r.try_get("kind")?;
    let enc: Option<String> = r.try_get("api_key_enc")?;
    Ok((
        Provider {
            id: r.try_get("id")?,
            kind: ProviderKind::parse(&kind).ok_or_else(|| DbError::Invalid(format!("bad provider kind {kind}")))?,
            label: r.try_get("label")?,
            base_url: r.try_get("base_url")?,
            model: r.try_get("model")?,
            has_key: enc.is_some(),
            is_default: r.try_get::<i64, _>("is_default")? != 0,
        },
        enc,
    ))
}

const COLS: &str = "id, kind, label, base_url, model, api_key_enc, is_default";

impl Db {
    pub async fn list_providers(&self) -> Result<Vec<Provider>> {
        let rows = sqlx::query(AssertSqlSafe(format!(
            "SELECT {COLS} FROM providers WHERE user_id = ? ORDER BY is_default DESC, created_at"
        )))
        .bind(&self.user_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(|r| row_to_provider(r).map(|(p, _)| p)).collect()
    }

    pub async fn provider(&self, id: &str) -> Result<ProviderRecord> {
        let r = sqlx::query(AssertSqlSafe(format!("SELECT {COLS} FROM providers WHERE user_id = ? AND id = ?")))
            .bind(&self.user_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(DbError::NotFound)?;
        let (provider, enc) = row_to_provider(&r)?;
        let api_key = enc.map(|e| self.keyring.decrypt(&e)).transpose()?;
        Ok(ProviderRecord { provider, api_key })
    }

    /// The default provider, or the first one configured.
    pub async fn default_provider(&self) -> Result<Option<ProviderRecord>> {
        let id: Option<String> = sqlx::query_scalar(
            "SELECT id FROM providers WHERE user_id = ? ORDER BY is_default DESC, created_at LIMIT 1",
        )
        .bind(&self.user_id)
        .fetch_optional(&self.pool)
        .await?;
        match id {
            Some(id) => Ok(Some(self.provider(&id).await?)),
            None => Ok(None),
        }
    }

    pub async fn create_provider(&self, input: &ProviderInput) -> Result<Provider> {
        let id = new_id();
        let enc = match input.api_key.as_deref().map(str::trim) {
            Some(k) if !k.is_empty() => Some(self.keyring.encrypt(k)?),
            _ => None,
        };
        let first = self.list_providers().await?.is_empty();
        let mut tx = self.pool.begin().await?;
        if input.is_default || first {
            sqlx::query("UPDATE providers SET is_default = 0 WHERE user_id = ?")
                .bind(&self.user_id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query(
            "INSERT INTO providers (id, user_id, kind, label, base_url, model, api_key_enc, is_default, created_at) VALUES (?,?,?,?,?,?,?,?,?)",
        )
        .bind(&id)
        .bind(&self.user_id)
        .bind(input.kind.as_str())
        .bind(label(input))
        .bind(base_url(input))
        .bind(model(input))
        .bind(enc)
        .bind((input.is_default || first) as i64)
        .bind(now_ms())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(self.provider(&id).await?.provider)
    }

    pub async fn update_provider(&self, id: &str, input: &ProviderInput) -> Result<Provider> {
        let existing = self.provider(id).await?;
        let mut tx = self.pool.begin().await?;
        if input.is_default {
            sqlx::query("UPDATE providers SET is_default = 0 WHERE user_id = ?")
                .bind(&self.user_id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("UPDATE providers SET kind = ?, label = ?, base_url = ?, model = ?, is_default = ? WHERE user_id = ? AND id = ?")
            .bind(input.kind.as_str())
            .bind(label(input))
            .bind(base_url(input))
            .bind(model(input))
            .bind((input.is_default || existing.provider.is_default) as i64)
            .bind(&self.user_id)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        if let Some(k) = input.api_key.as_deref().map(str::trim) {
            let enc = if k.is_empty() { None } else { Some(self.keyring.encrypt(k)?) };
            sqlx::query("UPDATE providers SET api_key_enc = ? WHERE user_id = ? AND id = ?")
                .bind(enc)
                .bind(&self.user_id)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(self.provider(id).await?.provider)
    }

    pub async fn delete_provider(&self, id: &str) -> Result<()> {
        let n = sqlx::query("DELETE FROM providers WHERE user_id = ? AND id = ?")
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

fn label(i: &ProviderInput) -> String {
    i.label
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            match i.kind {
                ProviderKind::Anthropic => "Claude",
                ProviderKind::Openai => "OpenAI",
                ProviderKind::Openrouter => "OpenRouter",
                ProviderKind::Ollama => "Ollama (local)",
                ProviderKind::OpenaiCompatible => "Local server",
            }
            .to_string()
        })
}

fn base_url(i: &ProviderInput) -> String {
    i.base_url
        .as_ref()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| i.kind.default_base_url().to_string())
}

fn model(i: &ProviderInput) -> String {
    i.model
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| i.kind.default_model().to_string())
}
