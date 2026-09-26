//! Technique drill sets, their results, and the puzzles that drill misses
//! turn into.

use std::collections::{BTreeMap, HashSet};

use api_types::{DrillAttempt, DrillItem, DrillKind, DrillSet, DrillSetSummary, DrillSource, MistakeMotif};
use sqlx::Row;

use crate::{Db, DbError, Result, new_id, now_ms};

const MONTH_MS: i64 = 30 * 86_400_000;

/// A mistake in an analysed game, as a place to start drilling.
#[derive(Debug, Clone, PartialEq)]
pub struct MistakeSeed {
    pub fen: String,
    pub best_uci: String,
    pub played_uci: String,
    /// (concept tag, "missed" | "allowed" | "coach").
    pub tags: Vec<(String, String)>,
}

/// Per technique over the last 30 days.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MasteryRow {
    pub attempted: u32,
    pub solved: u32,
    pub clean: u32,
    pub median_ms: Option<u32>,
}

fn kind_str(k: DrillKind) -> &'static str {
    match k {
        DrillKind::Spot => "spot",
        DrillKind::Find => "find",
        DrillKind::Defend => "defend",
        DrillKind::Playout => "playout",
    }
}

impl Db {
    /// The position before a mistake, the better move, and the motifs tagged on it.
    pub async fn mistake_seed(&self, analysis_id: &str, ply: u32) -> Result<Option<MistakeSeed>> {
        let rows = sqlx::query(
            "SELECT fen, best_uci, played_uci, concept_tag, motif_kind FROM mistake_index
             WHERE user_id = ? AND analysis_id = ? AND ply = ? ORDER BY motif_kind = 'missed' DESC, concept_tag",
        )
        .bind(&self.user_id)
        .bind(analysis_id)
        .bind(ply as i64)
        .fetch_all(&self.pool)
        .await?;
        let Some(first) = rows.first() else { return Ok(None) };
        let mut tags = vec![];
        for r in &rows {
            let tag: String = r.try_get("concept_tag")?;
            if !tag.is_empty() {
                tags.push((tag, r.try_get::<Option<String>, _>("motif_kind")?.unwrap_or_default()));
            }
        }
        Ok(Some(MistakeSeed {
            fen: first.try_get("fen")?,
            best_uci: first.try_get::<Option<String>, _>("best_uci")?.unwrap_or_default(),
            played_uci: first.try_get::<Option<String>, _>("played_uci")?.unwrap_or_default(),
            tags,
        }))
    }

    /// Every tagged mistake in an analysis, by ply.
    pub async fn mistake_motifs(&self, analysis_id: &str) -> Result<Vec<MistakeMotif>> {
        let rows = sqlx::query(
            "SELECT ply, concept_tag, motif_kind FROM mistake_index
             WHERE user_id = ? AND analysis_id = ? AND concept_tag != '' ORDER BY ply, motif_kind = 'missed' DESC",
        )
        .bind(&self.user_id)
        .bind(analysis_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(MistakeMotif {
                    ply: r.try_get::<i64, _>("ply")? as u32,
                    tag: r.try_get("concept_tag")?,
                    kind: r.try_get::<Option<String>, _>("motif_kind")?.unwrap_or_default(),
                })
            })
            .collect()
    }

    /// Lichess puzzles drilled in the last 30 days, so new sets pick others.
    pub async fn recent_bank_ids(&self) -> Result<HashSet<String>> {
        let rows = sqlx::query(
            "SELECT i.bank_id FROM drill_items i JOIN drill_sets s ON s.id = i.set_id
             WHERE i.user_id = ? AND i.bank_id IS NOT NULL AND s.created_at >= ?",
        )
        .bind(&self.user_id)
        .bind(now_ms() - MONTH_MS)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(|r| Ok(r.try_get::<String, _>(0)?)).collect()
    }

    /// Store a new set; items get ids in order.
    pub async fn create_drill_set(
        &self,
        technique: &str,
        source: &DrillSource,
        rating: u32,
        items: Vec<(DrillItem, Option<String>)>,
    ) -> Result<DrillSet> {
        let id = new_id();
        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT INTO drill_sets (id, user_id, technique, source_json, rating, created_at) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(&id)
            .bind(&self.user_id)
            .bind(technique)
            .bind(serde_json::to_string(source)?)
            .bind(rating as i64)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        for (pos, (mut item, bank_id)) in items.into_iter().enumerate() {
            item.id = new_id();
            item.solved = None;
            sqlx::query(
                "INSERT INTO drill_items (id, set_id, user_id, pos, kind, bank_id, item_json) VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&item.id)
            .bind(&id)
            .bind(&self.user_id)
            .bind(pos as i64)
            .bind(kind_str(item.kind))
            .bind(bank_id)
            .bind(serde_json::to_string(&item)?)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        self.drill_set(&id).await
    }

    /// Insert an item right after `after_id` (the adaptive follow-up to a miss).
    pub async fn insert_drill_item_after(
        &self,
        set_id: &str,
        after_id: &str,
        mut item: DrillItem,
        bank_id: Option<String>,
    ) -> Result<DrillSet> {
        let pos: i64 = sqlx::query_scalar("SELECT pos FROM drill_items WHERE user_id = ? AND set_id = ? AND id = ?")
            .bind(&self.user_id)
            .bind(set_id)
            .bind(after_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(DbError::NotFound)?;
        item.id = new_id();
        item.solved = None;
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE drill_items SET pos = pos + 1 WHERE set_id = ? AND pos > ?")
            .bind(set_id)
            .bind(pos)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO drill_items (id, set_id, user_id, pos, kind, bank_id, item_json) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&item.id)
        .bind(set_id)
        .bind(&self.user_id)
        .bind(pos + 1)
        .bind(kind_str(item.kind))
        .bind(bank_id)
        .bind(serde_json::to_string(&item)?)
        .execute(&mut *tx)
        .await?;
        // The set is not finished while the new item is open.
        sqlx::query("UPDATE drill_sets SET completed_at = NULL WHERE id = ?").bind(set_id).execute(&mut *tx).await?;
        tx.commit().await?;
        self.drill_set(set_id).await
    }

    /// Bank puzzle ids used in a set.
    pub async fn drill_bank_ids(&self, set_id: &str) -> Result<HashSet<String>> {
        let rows = sqlx::query("SELECT bank_id FROM drill_items WHERE user_id = ? AND set_id = ? AND bank_id IS NOT NULL")
            .bind(&self.user_id)
            .bind(set_id)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(|r| Ok(r.try_get::<String, _>(0)?)).collect()
    }

    pub async fn drill_set(&self, id: &str) -> Result<DrillSet> {
        let s = sqlx::query("SELECT * FROM drill_sets WHERE user_id = ? AND id = ?")
            .bind(&self.user_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(DbError::NotFound)?;
        let rows = sqlx::query("SELECT item_json, solved FROM drill_items WHERE set_id = ? ORDER BY pos")
            .bind(id)
            .fetch_all(&self.pool)
            .await?;
        let mut items = vec![];
        for r in rows {
            let mut item: DrillItem = serde_json::from_str(&r.try_get::<String, _>("item_json")?)?;
            item.solved = r.try_get::<Option<i64>, _>("solved")?.map(|v| v != 0);
            items.push(item);
        }
        let technique: String = s.try_get("technique")?;
        Ok(DrillSet {
            id: s.try_get("id")?,
            label: coach_label(&technique),
            technique,
            source: serde_json::from_str(&s.try_get::<String, _>("source_json")?)?,
            rating: s.try_get::<i64, _>("rating")? as u32,
            items,
            created_at: s.try_get("created_at")?,
            completed_at: s.try_get("completed_at")?,
        })
    }

    /// Record the first attempt at an item (later attempts only return it).
    /// A missed find or defend item becomes a review puzzle.
    pub async fn record_drill_item(&self, set_id: &str, item_id: &str, a: &DrillAttempt) -> Result<DrillSet> {
        let r = sqlx::query("SELECT item_json, solved FROM drill_items WHERE user_id = ? AND set_id = ? AND id = ?")
            .bind(&self.user_id)
            .bind(set_id)
            .bind(item_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(DbError::NotFound)?;
        if r.try_get::<Option<i64>, _>("solved")?.is_none() {
            let now = now_ms();
            sqlx::query("UPDATE drill_items SET solved = ?, ms = ?, hints_used = ?, attempted_at = ? WHERE id = ?")
                .bind(a.solved)
                .bind(a.ms as i64)
                .bind(a.hints_used as i64)
                .bind(now)
                .bind(item_id)
                .execute(&self.pool)
                .await?;
            let item: DrillItem = serde_json::from_str(&r.try_get::<String, _>("item_json")?)?;
            if !a.solved && matches!(item.kind, DrillKind::Find | DrillKind::Defend) {
                let set = self.drill_set(set_id).await?;
                self.add_drill_puzzle(&item, &set.technique).await?;
            }
            sqlx::query(
                "UPDATE drill_sets SET completed_at = ? WHERE id = ? AND completed_at IS NULL
                   AND NOT EXISTS (SELECT 1 FROM drill_items WHERE set_id = ? AND solved IS NULL)",
            )
            .bind(now)
            .bind(set_id)
            .bind(set_id)
            .execute(&self.pool)
            .await?;
        }
        self.drill_set(set_id).await
    }

    async fn add_drill_puzzle(&self, item: &DrillItem, technique: &str) -> Result<()> {
        let now = now_ms();
        let (solution, accept) = match item.kind {
            DrillKind::Defend => (item.accept_uci.iter().take(1).cloned().collect(), item.accept_uci.clone()),
            _ => (item.solution_uci.clone(), vec![]),
        };
        sqlx::query(
            "INSERT OR IGNORE INTO puzzles (id, user_id, source, fen, solution_json, line_json, themes_json,
               accept_json, due_at, created_at)
             VALUES (?, ?, 'drill', ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(new_id())
        .bind(&self.user_id)
        .bind(&item.fen)
        .bind(serde_json::to_string(&solution)?)
        .bind(serde_json::to_string(&item.line_san)?)
        .bind(serde_json::to_string(&[technique])?)
        .bind(if accept.is_empty() { None } else { Some(serde_json::to_string(&accept)?) })
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Drill results per technique over the last 30 days.
    pub async fn technique_mastery(&self) -> Result<BTreeMap<String, MasteryRow>> {
        let rows = sqlx::query(
            "SELECT s.technique, i.solved, i.hints_used, i.ms FROM drill_items i JOIN drill_sets s ON s.id = i.set_id
             WHERE i.user_id = ? AND i.solved IS NOT NULL AND i.attempted_at >= ?",
        )
        .bind(&self.user_id)
        .bind(now_ms() - MONTH_MS)
        .fetch_all(&self.pool)
        .await?;
        let mut out: BTreeMap<String, MasteryRow> = BTreeMap::new();
        let mut times: BTreeMap<String, Vec<u32>> = BTreeMap::new();
        for r in rows {
            let tag: String = r.try_get("technique")?;
            let solved = r.try_get::<i64, _>("solved")? != 0;
            let e = out.entry(tag.clone()).or_default();
            e.attempted += 1;
            e.solved += solved as u32;
            if solved && r.try_get::<Option<i64>, _>("hints_used")?.unwrap_or(0) == 0 {
                e.clean += 1;
                if let Some(ms) = r.try_get::<Option<i64>, _>("ms")? {
                    times.entry(tag).or_default().push(ms as u32);
                }
            }
        }
        for (tag, mut t) in times {
            t.sort_unstable();
            if let Some(e) = out.get_mut(&tag) {
                e.median_ms = t.get(t.len() / 2).copied();
            }
        }
        Ok(out)
    }

    /// Sets finished in the last 7 days.
    pub async fn drill_sets_finished_this_week(&self) -> Result<u32> {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM drill_sets WHERE user_id = ? AND completed_at >= ?")
            .bind(&self.user_id)
            .bind(now_ms() - 7 * 86_400_000)
            .fetch_one(&self.pool)
            .await?;
        Ok(n as u32)
    }

    /// The user's latest drill sets, newest first.
    pub async fn recent_drill_sets(&self, limit: u32) -> Result<Vec<DrillSetSummary>> {
        let rows = sqlx::query(
            "SELECT s.id, s.technique, s.created_at, s.completed_at, COUNT(i.id) AS items,
                    COALESCE(SUM(i.solved IS NOT NULL), 0) AS attempted, COALESCE(SUM(i.solved = 1), 0) AS solved
             FROM drill_sets s LEFT JOIN drill_items i ON i.set_id = s.id
             WHERE s.user_id = ? GROUP BY s.id ORDER BY s.created_at DESC LIMIT ?",
        )
        .bind(&self.user_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                let technique: String = r.try_get("technique")?;
                Ok(DrillSetSummary {
                    id: r.try_get("id")?,
                    label: coach_label(&technique),
                    technique,
                    items: r.try_get::<i64, _>("items")? as u32,
                    attempted: r.try_get::<i64, _>("attempted")? as u32,
                    solved: r.try_get::<i64, _>("solved")? as u32,
                    created_at: r.try_get("created_at")?,
                    completed_at: r.try_get("completed_at")?,
                })
            })
            .collect()
    }
}

/// "hanging_piece" -> "Hanging piece" (the coach crate's label, which db
/// doesn't depend on).
fn coach_label(tag: &str) -> String {
    let s = tag.replace('_', " ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => s,
    }
}
