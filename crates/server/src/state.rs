use std::sync::Arc;

use db::Db;
use engine::EnginePool;
use importers::Importers;

use crate::config::Config;
use crate::error::{ApiResult, AppError};
use crate::jobs::Jobs;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub pool: EnginePool,
    pub importers: Importers,
    pub jobs: Arc<Jobs>,
    pub config: Arc<Config>,
    /// Expected session cookie value in hosted mode.
    pub session_token: Option<Arc<String>>,
}

impl AppState {
    /// The default LLM provider, built and ready.
    pub async fn provider(&self) -> ApiResult<Arc<dyn llm::Provider>> {
        let rec = self.db.default_provider().await?.ok_or_else(|| {
            AppError::bad_request("No AI provider configured. Add one in Settings (Claude, OpenAI, OpenRouter or a local model).")
        })?;
        Ok(llm::build(llm::ProviderConfig {
            kind: rec.provider.kind,
            base_url: rec.provider.base_url,
            model: rec.provider.model,
            api_key: rec.api_key,
        })?)
    }
}
