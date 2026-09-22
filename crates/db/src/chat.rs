use api_types::{ChatMessage, ChatRole, ChatThread, ChatThreadDetail};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::{Db, DbError, Result, new_id, now_ms};

fn row_to_thread(r: &SqliteRow) -> Result<ChatThread> {
    Ok(ChatThread {
        id: r.try_get("id")?,
        game_id: r.try_get("game_id")?,
        title: r.try_get("title")?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
    })
}

impl Db {
    pub async fn create_thread(&self, game_id: Option<&str>, title: &str) -> Result<ChatThread> {
        let id = new_id();
        let now = now_ms();
        sqlx::query("INSERT INTO chat_threads (id, user_id, game_id, title, created_at, updated_at) VALUES (?,?,?,?,?,?)")
            .bind(&id)
            .bind(&self.user_id)
            .bind(game_id)
            .bind(title)
            .bind(now)
            .bind(now)
            .execute(&self.pool)
            .await?;
        self.thread(&id).await
    }

    pub async fn thread(&self, id: &str) -> Result<ChatThread> {
        let r = sqlx::query("SELECT * FROM chat_threads WHERE user_id = ? AND id = ?")
            .bind(&self.user_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(DbError::NotFound)?;
        row_to_thread(&r)
    }

    pub async fn list_threads(&self, game_id: Option<&str>) -> Result<Vec<ChatThread>> {
        let rows = match game_id {
            Some(g) => {
                sqlx::query("SELECT * FROM chat_threads WHERE user_id = ? AND game_id = ? ORDER BY updated_at DESC")
                    .bind(&self.user_id)
                    .bind(g)
                    .fetch_all(&self.pool)
                    .await?
            }
            None => {
                sqlx::query("SELECT * FROM chat_threads WHERE user_id = ? ORDER BY updated_at DESC LIMIT 200")
                    .bind(&self.user_id)
                    .fetch_all(&self.pool)
                    .await?
            }
        };
        rows.iter().map(row_to_thread).collect()
    }

    pub async fn thread_detail(&self, id: &str) -> Result<ChatThreadDetail> {
        let thread = self.thread(id).await?;
        let messages = sqlx::query_scalar::<_, String>("SELECT view_json FROM chat_messages WHERE thread_id = ? ORDER BY seq")
            .bind(id)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|j| serde_json::from_str(j))
            .collect::<std::result::Result<Vec<ChatMessage>, _>>()?;
        Ok(ChatThreadDetail { thread, messages })
    }

    /// Append one message. `ir` holds the provider-neutral LLM messages this
    /// entry contributes to the conversation (replayed verbatim next turn).
    pub async fn append_message(
        &self,
        thread_id: &str,
        msg: &ChatMessage,
        ir: &serde_json::Value,
        tokens: (Option<u32>, Option<u32>),
    ) -> Result<()> {
        let seq: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) + 1 FROM chat_messages WHERE thread_id = ?")
            .bind(thread_id)
            .fetch_one(&self.pool)
            .await?;
        sqlx::query(
            "INSERT INTO chat_messages (id, thread_id, seq, role, view_json, ir_json, fen, ply, tokens_in, tokens_out, created_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&msg.id)
        .bind(thread_id)
        .bind(seq)
        .bind(match msg.role {
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
        })
        .bind(serde_json::to_string(msg)?)
        .bind(ir.to_string())
        .bind(&msg.fen)
        .bind(msg.ply.map(|p| p as i64))
        .bind(tokens.0.map(|t| t as i64))
        .bind(tokens.1.map(|t| t as i64))
        .bind(msg.created_at)
        .execute(&self.pool)
        .await?;
        sqlx::query("UPDATE chat_threads SET updated_at = ? WHERE id = ?")
            .bind(now_ms())
            .bind(thread_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Every stored IR entry for the thread, in order.
    pub async fn thread_ir(&self, thread_id: &str) -> Result<Vec<serde_json::Value>> {
        let rows = sqlx::query("SELECT ir_json FROM chat_messages WHERE thread_id = ? ORDER BY seq")
            .bind(thread_id)
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|r| Ok(serde_json::from_str(&r.try_get::<String, _>("ir_json")?)?))
            .collect()
    }

    pub async fn rename_thread(&self, id: &str, title: &str) -> Result<()> {
        sqlx::query("UPDATE chat_threads SET title = ? WHERE user_id = ? AND id = ?")
            .bind(title)
            .bind(&self.user_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_thread(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM chat_threads WHERE user_id = ? AND id = ?")
            .bind(&self.user_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
