//! LLM providers behind one streaming interface: Anthropic (Messages API over
//! raw HTTP + SSE; there is no official Rust SDK) and any OpenAI-compatible
//! server (OpenAI, OpenRouter, Ollama, LM Studio, llama.cpp, vLLM).

pub mod anthropic;
pub mod ir;
pub mod json;
pub mod openai;
pub mod prompted;

use std::sync::Arc;
use std::time::Duration;

use api_types::ProviderKind;
use tokio::sync::mpsc;

pub use ir::*;

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("{provider} API error ({status}): {message}")]
    Api { provider: String, status: u16, message: String },
    #[error("could not reach {provider}: {message}")]
    Network { provider: String, message: String },
    #[error("{provider} stream error: {message}")]
    Stream { provider: String, message: String },
    #[error("missing API key for {0}")]
    MissingKey(String),
}

impl LlmError {
    pub fn is_retryable(&self) -> bool {
        match self {
            LlmError::Api { status, .. } => matches!(status, 408 | 409 | 429 | 500..=599),
            LlmError::Network { .. } => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub tools: bool,
    /// Server-enforced JSON schema output.
    pub json_schema: bool,
}

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    /// Provider kind, e.g. "anthropic", "openai".
    fn kind(&self) -> &str;
    fn model(&self) -> &str;
    fn caps(&self) -> Caps;
    /// Run one request, streaming deltas to `events` (if given) and returning
    /// the complete assistant message.
    async fn send(
        &self,
        req: &ChatRequest,
        events: Option<&mpsc::UnboundedSender<StreamEvent>>,
    ) -> Result<Completion, LlmError>;
}

/// Everything needed to build a provider client.
#[derive(Clone)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
}

pub fn build(cfg: ProviderConfig) -> Result<Arc<dyn Provider>, LlmError> {
    if cfg.kind.needs_key() && cfg.api_key.as_deref().is_none_or(str::is_empty) {
        return Err(LlmError::MissingKey(cfg.kind.as_str().into()));
    }
    let client = http_client();
    Ok(match cfg.kind {
        ProviderKind::Anthropic => Arc::new(anthropic::Anthropic::new(client, cfg)),
        _ => Arc::new(openai::OpenAiCompat::new(client, cfg)),
    })
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        // Long turns on large models can stream for minutes.
        .read_timeout(Duration::from_secs(600))
        .user_agent(concat!("chessgpt/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("http client")
}

/// Send with retries on 429/5xx/network errors (exponential backoff).
pub async fn send_with_retry(
    p: &dyn Provider,
    req: &ChatRequest,
    events: Option<&mpsc::UnboundedSender<StreamEvent>>,
) -> Result<Completion, LlmError> {
    let mut delay = Duration::from_millis(800);
    let mut attempt = 0;
    loop {
        match p.send(req, events).await {
            Err(e) if e.is_retryable() && attempt < 3 => {
                tracing::warn!("{} request failed ({e}); retrying in {:?}", p.kind(), delay);
                tokio::time::sleep(delay).await;
                delay *= 2;
                attempt += 1;
            }
            r => return r,
        }
    }
}

/// Cheap round trip used by the settings "Test connection" button.
pub async fn ping(p: &dyn Provider) -> Result<String, LlmError> {
    let mut req = ChatRequest::new(
        "You are a connectivity check. Reply with exactly: OK",
        vec![Message::user("Say OK.")],
    );
    req.max_tokens = 512;
    req.effort = Some(Effort::Low);
    let c = p.send(&req, None).await?;
    Ok(format!("{} replied: {}", c.model, c.message.text().trim()))
}
