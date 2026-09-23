use api_types::{
    EloTier, Explanation, GameAnalysis, GameReview, JobStatus, MoveEval, Score, Side,
};
use engine::{Analysis, AnalysisStore, PvLine};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::games::{parse_side, side_str};
use crate::{Db, DbError, Result, new_id, now_ms};

pub struct NewAnalysis {
    pub game_id: String,
    pub elo: u32,
    pub tier: EloTier,
    pub user_side: Option<Side>,
    pub engine: String,
}

fn tier_str(t: EloTier) -> &'static str {
    match t {
        EloTier::Beginner => "beginner",
        EloTier::Intermediate => "intermediate",
        EloTier::Advanced => "advanced",
        EloTier::Expert => "expert",
    }
}

fn parse_tier(s: &str) -> EloTier {
    match s {
        "beginner" => EloTier::Beginner,
        "advanced" => EloTier::Advanced,
        "expert" => EloTier::Expert,
        _ => EloTier::Intermediate,
    }
}

#[async_trait::async_trait]
impl AnalysisStore for Db {
    async fn get(&self, engine: &str, fen_key: &str, multipv: u32, min_depth: u32) -> Option<Analysis> {
        let r = sqlx::query(
            "SELECT a.multipv, a.depth, a.nodes, a.time_ms, a.best_move, a.lines_json
             FROM analyses a JOIN positions p ON p.id = a.position_id
             WHERE p.fen_key = ? AND a.engine = ? AND a.multipv >= ? AND a.depth >= ?
             ORDER BY a.depth DESC LIMIT 1",
        )
        .bind(fen_key)
        .bind(engine)
        .bind(multipv as i64)
        .bind(min_depth as i64)
        .fetch_optional(&self.pool)
        .await
        .ok()??;
        let lines: Vec<PvLine> = serde_json::from_str(r.try_get::<String, _>("lines_json").ok()?.as_str()).ok()?;
        Some(Analysis {
            fen: String::new(),
            depth: r.try_get::<i64, _>("depth").ok()? as u32,
            multipv: r.try_get::<i64, _>("multipv").ok()? as u32,
            lines,
            nodes: r.try_get::<i64, _>("nodes").ok()? as u64,
            nps: 0,
            time_ms: r.try_get::<i64, _>("time_ms").ok()? as u64,
            best_move: r.try_get("best_move").ok()?,
            terminal: None,
            done: true,
            engine: engine.to_string(),
        })
    }

    async fn put(&self, _fen_key: &str, a: &Analysis) {
        if a.lines.is_empty() || !a.done {
            return;
        }
        let res: Result<()> = async {
            let pid = self.position_id(&a.fen).await?;
            sqlx::query(
                "INSERT INTO analyses (position_id, engine, multipv, depth, nodes, time_ms, best_move, lines_json, created_at)
                 VALUES (?,?,?,?,?,?,?,?,?)
                 ON CONFLICT(position_id, engine, multipv) DO UPDATE SET
                   depth = excluded.depth, nodes = excluded.nodes, time_ms = excluded.time_ms,
                   best_move = excluded.best_move, lines_json = excluded.lines_json, created_at = excluded.created_at
                 WHERE excluded.depth > analyses.depth",
            )
            .bind(pid)
            .bind(&a.engine)
            .bind(a.multipv as i64)
            .bind(a.depth as i64)
            .bind(a.nodes as i64)
            .bind(a.time_ms as i64)
            .bind(&a.best_move)
            .bind(serde_json::to_string(&a.lines)?)
            .bind(now_ms())
            .execute(&self.pool)
            .await?;
            Ok(())
        }
        .await;
        if let Err(e) = res {
            tracing::warn!("analysis cache write failed: {e}");
        }
    }
}

impl Db {
    pub async fn create_analysis(&self, n: &NewAnalysis) -> Result<String> {
        let id = new_id();
        let now = now_ms();
        sqlx::query(
            "INSERT INTO game_analyses (id, game_id, status, elo, tier, user_side, engine, created_at, updated_at)
             VALUES (?, ?, 'queued', ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&n.game_id)
        .bind(n.elo as i64)
        .bind(tier_str(n.tier))
        .bind(n.user_side.map(side_str))
        .bind(&n.engine)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    pub async fn set_analysis_status(&self, id: &str, status: JobStatus, error: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE game_analyses SET status = ?, error = ?, updated_at = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(error)
            .bind(now_ms())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_start_score(&self, id: &str, score: Option<Score>) -> Result<()> {
        sqlx::query("UPDATE game_analyses SET start_score_json = ?, updated_at = ? WHERE id = ?")
            .bind(score.map(|s| serde_json::to_string(&s)).transpose()?)
            .bind(now_ms())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn put_move_eval(&self, id: &str, m: &MoveEval) -> Result<()> {
        sqlx::query(
            "INSERT INTO move_evals (analysis_id, ply, classification, lichess_judgement, delta_wc, phase, is_key_moment, data_json)
             VALUES (?,?,?,?,?,?,?,?)
             ON CONFLICT(analysis_id, ply) DO UPDATE SET classification = excluded.classification,
               lichess_judgement = excluded.lichess_judgement, delta_wc = excluded.delta_wc, phase = excluded.phase,
               is_key_moment = excluded.is_key_moment, data_json = excluded.data_json",
        )
        .bind(id)
        .bind(m.ply as i64)
        .bind(m.classification.as_str())
        .bind(m.lichess_judgement.map(|j| format!("{j:?}").to_lowercase()))
        .bind(m.delta_wc)
        .bind(m.phase.as_str())
        .bind(m.is_key_moment as i64)
        .bind(serde_json::to_string(m)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_accuracy(&self, id: &str, white: Option<f64>, black: Option<f64>) -> Result<()> {
        sqlx::query("UPDATE game_analyses SET white_accuracy = ?, black_accuracy = ?, updated_at = ? WHERE id = ?")
            .bind(white)
            .bind(black)
            .bind(now_ms())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_key_moments(&self, id: &str, plies: &[u32]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE game_analyses SET key_moments_json = ?, updated_at = ? WHERE id = ?")
            .bind(serde_json::to_string(plies)?)
            .bind(now_ms())
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE move_evals SET is_key_moment = 0, data_json = json_set(data_json, '$.is_key_moment', json('false')) WHERE analysis_id = ?",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        for p in plies {
            sqlx::query(
                "UPDATE move_evals SET is_key_moment = 1, data_json = json_set(data_json, '$.is_key_moment', json('true')) WHERE analysis_id = ? AND ply = ?",
            )
            .bind(id)
            .bind(*p as i64)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn put_explanation(
        &self,
        analysis_id: &str,
        e: &Explanation,
        prompt_version: &str,
        request_json: &serde_json::Value,
        response_json: &serde_json::Value,
        tokens_in: Option<u32>,
        tokens_out: Option<u32>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO explanations (id, analysis_id, ply, provider, model, prompt_version, request_json, response_json,
               concept_tags_json, verification_json, tokens_in, tokens_out, created_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)
             ON CONFLICT(analysis_id, ply) DO UPDATE SET provider = excluded.provider, model = excluded.model,
               prompt_version = excluded.prompt_version, request_json = excluded.request_json,
               response_json = excluded.response_json, concept_tags_json = excluded.concept_tags_json,
               verification_json = excluded.verification_json, tokens_in = excluded.tokens_in,
               tokens_out = excluded.tokens_out, created_at = excluded.created_at",
        )
        .bind(new_id())
        .bind(analysis_id)
        .bind(e.ply as i64)
        .bind(&e.provider)
        .bind(&e.model)
        .bind(prompt_version)
        .bind(request_json.to_string())
        .bind(serde_json::to_string(e)?)
        .bind(serde_json::to_string(&e.concept_tags)?)
        .bind(serde_json::to_string(&e.verification)?)
        .bind(tokens_in.map(|t| t as i64))
        .bind(tokens_out.map(|t| t as i64))
        .bind(now_ms())
        .execute(&self.pool)
        .await?;
        let _ = response_json;
        self.tag_mistake(analysis_id, e.ply, &e.concept_tags).await
    }

    pub async fn set_review(&self, id: &str, review: &GameReview) -> Result<()> {
        sqlx::query("UPDATE game_analyses SET review_json = ?, updated_at = ? WHERE id = ?")
            .bind(serde_json::to_string(review)?)
            .bind(now_ms())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn latest_analysis(&self, game_id: &str) -> Result<Option<GameAnalysis>> {
        let id: Option<String> = sqlx::query_scalar(
            "SELECT a.id FROM game_analyses a JOIN games g ON g.id = a.game_id
             WHERE a.game_id = ? AND g.user_id = ? ORDER BY a.created_at DESC LIMIT 1",
        )
        .bind(game_id)
        .bind(&self.user_id)
        .fetch_optional(&self.pool)
        .await?;
        match id {
            Some(id) => Ok(Some(self.analysis(&id).await?)),
            None => Ok(None),
        }
    }

    pub async fn analysis(&self, id: &str) -> Result<GameAnalysis> {
        let r = sqlx::query(
            "SELECT a.* FROM game_analyses a JOIN games g ON g.id = a.game_id WHERE a.id = ? AND g.user_id = ?",
        )
        .bind(id)
        .bind(&self.user_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(DbError::NotFound)?;
        self.analysis_from_row(&r).await
    }

    async fn analysis_from_row(&self, r: &SqliteRow) -> Result<GameAnalysis> {
        let id: String = r.try_get("id")?;
        let moves: Vec<MoveEval> = sqlx::query_scalar::<_, String>(
            "SELECT data_json FROM move_evals WHERE analysis_id = ? ORDER BY ply",
        )
        .bind(&id)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(|j| serde_json::from_str(j))
        .collect::<std::result::Result<_, _>>()?;
        let explanations: Vec<Explanation> = sqlx::query_scalar::<_, String>(
            "SELECT response_json FROM explanations WHERE analysis_id = ? ORDER BY ply",
        )
        .bind(&id)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .filter_map(|j| serde_json::from_str(j).ok())
        .collect();
        let start: Option<String> = r.try_get("start_score_json")?;
        let review: Option<String> = r.try_get("review_json")?;
        let side: Option<String> = r.try_get("user_side")?;
        Ok(GameAnalysis {
            game_id: r.try_get("game_id")?,
            status: JobStatus::parse(&r.try_get::<String, _>("status")?),
            elo: r.try_get::<i64, _>("elo")? as u32,
            tier: parse_tier(&r.try_get::<String, _>("tier")?),
            user_side: side.as_deref().and_then(parse_side),
            engine: r.try_get("engine")?,
            start_score: start.map(|s| serde_json::from_str(&s)).transpose()?,
            white_accuracy: r.try_get("white_accuracy")?,
            black_accuracy: r.try_get("black_accuracy")?,
            key_moments: serde_json::from_str(&r.try_get::<String, _>("key_moments_json")?)?,
            review: review.map(|s| serde_json::from_str(&s)).transpose()?,
            error: r.try_get("error")?,
            created_at: r.try_get("created_at")?,
            id,
            moves,
            explanations,
        })
    }

    /// Analyses left queued/running by a previous process.
    /// (analysis id, owner) of analyses a previous process left unfinished.
    pub async fn unfinished_analyses(&self) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query(
            "SELECT a.id, g.user_id FROM game_analyses a JOIN games g ON g.id = a.game_id
             WHERE a.status IN ('queued', 'running') ORDER BY a.created_at",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(|r| Ok((r.try_get("id")?, r.try_get("user_id")?))).collect()
    }

    /// Record the player's errors in the mistake index (untagged).
    pub async fn index_mistakes(&self, analysis_id: &str) -> Result<u32> {
        let a = self.analysis(analysis_id).await?;
        let game = self.game(&a.game_id).await?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM mistake_index WHERE analysis_id = ?")
            .bind(analysis_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        let mut n = 0;
        for m in &a.moves {
            if !m.classification.is_error() || a.user_side.is_some_and(|s| s != m.mover) {
                continue;
            }
            let Some(pm) = game.parsed.moves.get(m.ply as usize - 1) else { continue };
            let pid = self.position_id(&pm.fen_before).await?;
            sqlx::query(
                "INSERT OR REPLACE INTO mistake_index (user_id, game_id, analysis_id, ply, position_id, fen, side,
                   classification, phase, concept_tag, delta_wc, played_uci, best_uci, created_at)
                 VALUES (?,?,?,?,?,?,?,?,?,'',?,?,?,?)",
            )
            .bind(&self.user_id)
            .bind(&a.game_id)
            .bind(analysis_id)
            .bind(m.ply as i64)
            .bind(pid)
            .bind(&pm.fen_before)
            .bind(side_str(m.mover))
            .bind(m.classification.as_str())
            .bind(m.phase.as_str())
            .bind(m.delta_wc)
            .bind(&m.uci)
            .bind(&m.best_uci)
            .bind(now_ms())
            .execute(&self.pool)
            .await?;
            n += 1;
        }
        Ok(n)
    }

    /// Replace the untagged mistake row for a ply with one row per concept tag.
    async fn tag_mistake(&self, analysis_id: &str, ply: u32, tags: &[String]) -> Result<()> {
        if tags.is_empty() {
            return Ok(());
        }
        let base = sqlx::query("SELECT * FROM mistake_index WHERE analysis_id = ? AND ply = ? LIMIT 1")
            .bind(analysis_id)
            .bind(ply as i64)
            .fetch_optional(&self.pool)
            .await?;
        let Some(b) = base else { return Ok(()) };
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM mistake_index WHERE analysis_id = ? AND ply = ?")
            .bind(analysis_id)
            .bind(ply as i64)
            .execute(&mut *tx)
            .await?;
        for tag in tags {
            sqlx::query(
                "INSERT OR REPLACE INTO mistake_index (user_id, game_id, analysis_id, ply, position_id, fen, side,
                   classification, phase, concept_tag, delta_wc, played_uci, best_uci, created_at)
                 VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(b.try_get::<String, _>("user_id")?)
            .bind(b.try_get::<String, _>("game_id")?)
            .bind(analysis_id)
            .bind(ply as i64)
            .bind(b.try_get::<i64, _>("position_id")?)
            .bind(b.try_get::<String, _>("fen")?)
            .bind(b.try_get::<String, _>("side")?)
            .bind(b.try_get::<String, _>("classification")?)
            .bind(b.try_get::<String, _>("phase")?)
            .bind(tag)
            .bind(b.try_get::<f64, _>("delta_wc")?)
            .bind(b.try_get::<String, _>("played_uci")?)
            .bind(b.try_get::<Option<String>, _>("best_uci")?)
            .bind(now_ms())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Count of indexed mistakes per concept tag for the user (for later training).
    pub async fn mistake_counts_by_tag(&self) -> Result<Vec<(String, i64)>> {
        let rows = sqlx::query(
            "SELECT concept_tag, COUNT(*) AS n FROM mistake_index WHERE user_id = ? AND concept_tag != ''
             GROUP BY concept_tag ORDER BY n DESC",
        )
        .bind(&self.user_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| Ok((r.try_get("concept_tag")?, r.try_get("n")?)))
            .collect()
    }
}

impl Db {
    /// Count of the user's indexed mistakes per game phase.
    pub async fn mistake_counts_by_phase(&self) -> Result<Vec<(String, i64)>> {
        let rows = sqlx::query(
            "SELECT phase, COUNT(DISTINCT analysis_id || ':' || ply) AS n FROM mistake_index WHERE user_id = ?
             GROUP BY phase ORDER BY n DESC",
        )
        .bind(&self.user_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(|r| Ok((r.try_get("phase")?, r.try_get("n")?))).collect()
    }
}
