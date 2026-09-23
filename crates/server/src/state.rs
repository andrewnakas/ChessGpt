use std::sync::Arc;

use db::Db;
use engine::EnginePool;
use importers::Importers;

use crate::config::Config;
use crate::error::{ApiResult, AppError};
use crate::jobs::Jobs;
use crate::managed::{Budget, Budgeted, ManagedLlm};

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub pool: EnginePool,
    pub importers: Importers,
    pub jobs: Arc<Jobs>,
    pub config: Arc<Config>,
    /// In-flight "Sign in with Lichess" attempts.
    pub pending: Arc<crate::auth::PendingLogins>,
    /// The operator's own model, shared by all users (no user keys).
    pub managed: Option<Arc<ManagedLlm>>,
    pub budget: Arc<Budget>,
}

impl AppState {
    /// Hosted mode has accounts; local mode is one user with no login.
    pub fn accounts_enabled(&self) -> bool {
        self.config.mode == "hosted"
    }

    /// This state with the database scoped to `user_id`.
    pub fn scoped(&self, user_id: &str) -> AppState {
        AppState { db: self.db.for_user(user_id), ..self.clone() }
    }

    /// The LLM to use: the operator's managed model when configured,
    /// otherwise the user's default provider from Settings.
    pub async fn provider(&self) -> ApiResult<Arc<dyn llm::Provider>> {
        if let Some(m) = &self.managed {
            let inner = llm::build(llm::ProviderConfig {
                kind: m.kind,
                base_url: m.base_url.clone(),
                model: m.model.clone(),
                api_key: m.api_key.clone(),
            })?;
            return Ok(Arc::new(Budgeted { inner, budget: self.budget.clone(), limit: m.daily_tokens }));
        }
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

    /// Users may manage their own providers only when the site supplies none.
    pub fn user_providers_allowed(&self) -> bool {
        self.managed.is_none()
    }
}
