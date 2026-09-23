use std::sync::Arc;

use db::Db;
use engine::EnginePool;
use importers::Importers;

use crate::config::Config;
use crate::error::{ApiResult, AppError};
use crate::jobs::Jobs;
use crate::managed::{Budget, Budgeted, ManagedLlm};
use crate::relay::{Relay, RelayHub};

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
    /// Browser tabs running the coach model, by user.
    pub relay: Arc<RelayHub>,
    /// The signed-in account this state is scoped to (None in local mode).
    pub user: Option<String>,
}

impl AppState {
    /// Hosted mode has accounts; local mode is one user with no login.
    pub fn accounts_enabled(&self) -> bool {
        self.config.mode == "hosted"
    }

    /// This state with the database scoped to `user_id`.
    pub fn scoped(&self, user_id: &str) -> AppState {
        AppState { db: self.db.for_user(user_id), user: Some(user_id.to_string()), ..self.clone() }
    }

    /// Key for per-user relay state.
    pub fn user_key(&self) -> &str {
        self.user.as_deref().unwrap_or("local")
    }

    /// The LLM to use: the model in the user's browser when a tab has one
    /// loaded (falling back to the server's), else the server's model.
    pub async fn provider(&self) -> ApiResult<Arc<dyn llm::Provider>> {
        let server = self.server_provider().await;
        if let Some(model) = self.relay.device_model(self.user_key()) {
            return Ok(Arc::new(Relay {
                hub: self.relay.clone(),
                user: self.user_key().to_string(),
                model,
                fallback: server.ok(),
                dead: Default::default(),
            }));
        }
        server
    }

    /// The operator's managed model when configured, otherwise the user's
    /// default provider from Settings.
    pub async fn server_provider(&self) -> ApiResult<Arc<dyn llm::Provider>> {
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
