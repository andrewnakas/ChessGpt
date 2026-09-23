//! Storage for chessgpt's OAuth 2.1 authorization server (used by MCP
//! clients such as Claude and ChatGPT to act on a user's behalf).

use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::accounts::{hash_token, random_token};
use crate::{Db, DbError, Result, now_ms};

pub const ACCESS_TTL_MS: i64 = 60 * 60 * 1000;
pub const REFRESH_TTL_MS: i64 = 90 * 24 * 60 * 60 * 1000;
pub const CODE_TTL_MS: i64 = 10 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OAuthClient {
    pub client_id: String,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    pub metadata: serde_json::Value,
}

/// Who a token speaks for.
#[derive(Debug, Clone, PartialEq)]
pub struct OAuthGrant {
    pub user_id: String,
    pub client_id: String,
    pub scope: String,
    pub resource: Option<String>,
}

pub struct IssuedTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    pub scope: String,
}

impl Db {
    pub async fn register_client(
        &self,
        client_id: &str,
        name: &str,
        redirect_uris: &[String],
        metadata: &serde_json::Value,
    ) -> Result<OAuthClient> {
        sqlx::query(
            "INSERT INTO oauth_clients (client_id, client_name, redirect_uris, metadata_json, created_at) VALUES (?,?,?,?,?)
             ON CONFLICT(client_id) DO UPDATE SET client_name = excluded.client_name,
               redirect_uris = excluded.redirect_uris, metadata_json = excluded.metadata_json",
        )
        .bind(client_id)
        .bind(name)
        .bind(serde_json::to_string(redirect_uris)?)
        .bind(metadata.to_string())
        .bind(now_ms())
        .execute(&self.pool)
        .await?;
        self.client(client_id).await?.ok_or(DbError::NotFound)
    }

    pub async fn client(&self, client_id: &str) -> Result<Option<OAuthClient>> {
        let r = sqlx::query("SELECT * FROM oauth_clients WHERE client_id = ?")
            .bind(client_id)
            .fetch_optional(&self.pool)
            .await?;
        let Some(r) = r else { return Ok(None) };
        Ok(Some(OAuthClient {
            client_id: r.try_get("client_id")?,
            client_name: r.try_get("client_name")?,
            redirect_uris: serde_json::from_str(&r.try_get::<String, _>("redirect_uris")?)?,
            metadata: serde_json::from_str(&r.try_get::<String, _>("metadata_json")?)?,
        }))
    }

    pub async fn create_code(
        &self,
        client_id: &str,
        user_id: &str,
        redirect_uri: &str,
        code_challenge: &str,
        resource: Option<&str>,
        scope: &str,
    ) -> Result<String> {
        let code = random_token();
        sqlx::query(
            "INSERT INTO oauth_codes (code_hash, client_id, user_id, redirect_uri, code_challenge, resource, scope, expires_at)
             VALUES (?,?,?,?,?,?,?,?)",
        )
        .bind(hash_token(&code))
        .bind(client_id)
        .bind(user_id)
        .bind(redirect_uri)
        .bind(code_challenge)
        .bind(resource)
        .bind(scope)
        .bind(now_ms() + CODE_TTL_MS)
        .execute(&self.pool)
        .await?;
        Ok(code)
    }

    /// Redeem a code once. Returns (grant, redirect_uri, code_challenge).
    pub async fn take_code(&self, code: &str) -> Result<Option<(OAuthGrant, String, String)>> {
        let h = hash_token(code);
        let r = sqlx::query("DELETE FROM oauth_codes WHERE code_hash = ? RETURNING *")
            .bind(&h)
            .fetch_optional(&self.pool)
            .await?;
        let Some(r) = r else { return Ok(None) };
        if r.try_get::<i64, _>("expires_at")? < now_ms() {
            return Ok(None);
        }
        Ok(Some((
            OAuthGrant {
                user_id: r.try_get("user_id")?,
                client_id: r.try_get("client_id")?,
                scope: r.try_get("scope")?,
                resource: r.try_get("resource")?,
            },
            r.try_get("redirect_uri")?,
            r.try_get("code_challenge")?,
        )))
    }

    pub async fn issue_tokens(&self, g: &OAuthGrant) -> Result<IssuedTokens> {
        let access = random_token();
        let refresh = random_token();
        let now = now_ms();
        for (tok, kind, ttl) in [(&access, "access", ACCESS_TTL_MS), (&refresh, "refresh", REFRESH_TTL_MS)] {
            sqlx::query(
                "INSERT INTO oauth_tokens (token_hash, kind, client_id, user_id, resource, scope, expires_at, created_at)
                 VALUES (?,?,?,?,?,?,?,?)",
            )
            .bind(hash_token(tok))
            .bind(kind)
            .bind(&g.client_id)
            .bind(&g.user_id)
            .bind(&g.resource)
            .bind(&g.scope)
            .bind(now + ttl)
            .bind(now)
            .execute(&self.pool)
            .await?;
        }
        Ok(IssuedTokens { access_token: access, refresh_token: refresh, expires_in: ACCESS_TTL_MS / 1000, scope: g.scope.clone() })
    }

    async fn lookup_token(&self, token: &str, kind: &str) -> Result<Option<OAuthGrant>> {
        let r = sqlx::query("SELECT * FROM oauth_tokens WHERE token_hash = ? AND kind = ? AND expires_at > ?")
            .bind(hash_token(token))
            .bind(kind)
            .bind(now_ms())
            .fetch_optional(&self.pool)
            .await?;
        let Some(r) = r else { return Ok(None) };
        Ok(Some(OAuthGrant {
            user_id: r.try_get("user_id")?,
            client_id: r.try_get("client_id")?,
            scope: r.try_get("scope")?,
            resource: r.try_get("resource")?,
        }))
    }

    pub async fn access_grant(&self, token: &str) -> Result<Option<OAuthGrant>> {
        self.lookup_token(token, "access").await
    }

    /// Rotate a refresh token: the old one stops working.
    pub async fn refresh(&self, refresh_token: &str, client_id: &str) -> Result<Option<IssuedTokens>> {
        let Some(g) = self.lookup_token(refresh_token, "refresh").await? else { return Ok(None) };
        if g.client_id != client_id {
            return Ok(None);
        }
        sqlx::query("DELETE FROM oauth_tokens WHERE token_hash = ?")
            .bind(hash_token(refresh_token))
            .execute(&self.pool)
            .await?;
        Ok(Some(self.issue_tokens(&g).await?))
    }

    pub async fn revoke_token(&self, token: &str) -> Result<()> {
        sqlx::query("DELETE FROM oauth_tokens WHERE token_hash = ?").bind(hash_token(token)).execute(&self.pool).await?;
        Ok(())
    }

    /// Apps (MCP clients) this user has connected, for the settings page.
    pub async fn connected_clients(&self) -> Result<Vec<(String, String, i64)>> {
        let rows = sqlx::query(
            "SELECT c.client_id, c.client_name, MAX(t.created_at) AS last FROM oauth_tokens t
             JOIN oauth_clients c ON c.client_id = t.client_id
             WHERE t.user_id = ? AND t.expires_at > ? GROUP BY c.client_id ORDER BY last DESC",
        )
        .bind(&self.user_id)
        .bind(now_ms())
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| Ok((r.try_get("client_id")?, r.try_get("client_name")?, r.try_get("last")?)))
            .collect()
    }

    pub async fn disconnect_client(&self, client_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM oauth_tokens WHERE user_id = ? AND client_id = ?")
            .bind(&self.user_id)
            .bind(client_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
