//! chessgpt HTTP server: REST + server-sent events over the engine pool,
//! database, importers and coach, plus the embedded web UI.

pub mod assets;
pub mod auth;
pub mod config;
pub mod error;
pub mod jobs;
pub mod routes;
pub mod state;

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
    let session_token = match &config.auth_password {
        Some(pw) if config.mode == "hosted" => {
            let salt = auth::load_salt(&config.data_dir)?;
            Some(Arc::new(auth::session_token(pw, &salt)))
        }
        _ => None,
    };
    if config.mode == "hosted" && session_token.is_none() {
        tracing::warn!("hosted mode without CHESSGPT_AUTH_PASSWORD: anyone who can reach this server can use it");
    }
    Ok(AppState {
        db,
        pool,
        importers: importers::Importers::default(),
        jobs: Arc::new(jobs::Jobs::default()),
        config: Arc::new(config),
        session_token,
    })
}
