//! chessgpt HTTP server: REST + server-sent events over the engine pool,
//! database, importers and coach, plus the embedded web UI.

pub mod assets;
pub mod auth;
pub mod config;
pub mod error;
pub mod jobs;
pub mod managed;
pub mod mcp;
pub mod oauth;
pub mod relay;
pub mod routes;
pub mod state;
pub mod sync;

use std::sync::Arc;

use anyhow::Context;
use engine::{EngineConfig, EnginePool, find_stockfish};

use crate::config::{Config, search_roots};
use crate::state::AppState;

/// Open the database, start the engine pool, and build the shared state.
pub async fn build_state(config: Config) -> anyhow::Result<AppState> {
    let db = db::Db::open(&config.data_dir)
        .await
        .with_context(|| format!("opening database in {}", config.data_dir.display()))?;
    let stockfish = match &config.stockfish {
        Some(p) => p.clone(),
        None => search_roots().iter().find_map(|r| find_stockfish(r)).context(
            "Stockfish not found. Run `cargo xtask fetch-stockfish`, or set STOCKFISH_PATH to a Stockfish binary.",
        )?,
    };
    let mut ecfg = EngineConfig::with_defaults(stockfish.clone());
    if let Some(w) = config.engine_workers {
        ecfg.workers = w.max(1);
    }
    if let Some(t) = config.engine_threads {
        ecfg.threads = t.max(1);
    }
    if let Some(h) = config.engine_hash_mb {
        ecfg.hash_mb = h.max(16);
    }
    let store: Arc<dyn engine::AnalysisStore> = Arc::new(db.clone());
    let pool = EnginePool::start(ecfg, Some(store))
        .await
        .with_context(|| format!("starting Stockfish at {}", stockfish.display()))?;
    let managed = managed::ManagedLlm::from_env();
    match &managed {
        Some(m) => tracing::info!(
            "AI coach provided by the site: {} (daily token limit: {})",
            m.label(),
            m.daily_tokens.map(|t| t.to_string()).unwrap_or_else(|| "none".into())
        ),
        None if config.mode == "hosted" => {
            tracing::warn!("hosted mode without CHESSGPT_LLM_* or ANTHROPIC_API_KEY: users must bring their own keys")
        }
        None => {}
    }
    if let Some(m) = &managed
        && m.kind.needs_key()
        && m.api_key.is_none()
    {
        anyhow::bail!("CHESSGPT_LLM_PROVIDER={} needs CHESSGPT_LLM_API_KEY", m.kind.as_str());
    }
    Ok(AppState {
        db,
        pool,
        importers: importers::Importers::default(),
        jobs: Arc::new(jobs::Jobs::default()),
        config: Arc::new(config),
        pending: Arc::new(auth::PendingLogins::default()),
        managed: managed.map(Arc::new),
        budget: Arc::new(managed::Budget::default()),
        relay: Arc::new(relay::RelayHub::default()),
        user: None,
    })
}
