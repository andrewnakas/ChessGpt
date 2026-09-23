//! The site operator's own model key. When configured, every user gets the
//! coach without supplying a key, and a daily token budget caps the bill.

use std::sync::{Arc, Mutex};

use api_types::ProviderKind;
use llm::{Caps, ChatRequest, Completion, LlmError, Provider, StreamEvent};
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct ManagedLlm {
    pub kind: ProviderKind,
    pub model: String,
    pub base_url: String,
    pub api_key: Option<String>,
    /// Input + output tokens allowed per UTC day; `None` = unlimited.
    pub daily_tokens: Option<u64>,
}

impl std::fmt::Debug for ManagedLlm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedLlm")
            .field("kind", &self.kind)
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("daily_tokens", &self.daily_tokens)
            .finish()
    }
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

impl ManagedLlm {
    /// `CHESSGPT_LLM_PROVIDER` / `_API_KEY` / `_MODEL` / `_BASE_URL`, falling
    /// back to a plain `ANTHROPIC_API_KEY`, `OPENAI_API_KEY` or `OPENROUTER_API_KEY`.
    pub fn from_env() -> Option<ManagedLlm> {
        let explicit = env("CHESSGPT_LLM_PROVIDER").and_then(|k| ProviderKind::parse(&k));
        let (kind, key) = match explicit {
            Some(kind) => (kind, env("CHESSGPT_LLM_API_KEY")),
            None => {
                if let Some(k) = env("ANTHROPIC_API_KEY") {
                    (ProviderKind::Anthropic, Some(k))
                } else if let Some(k) = env("OPENAI_API_KEY") {
                    (ProviderKind::Openai, Some(k))
                } else if let Some(k) = env("OPENROUTER_API_KEY") {
                    (ProviderKind::Openrouter, Some(k))
                } else {
                    return None;
                }
            }
        };
        let key = key.or_else(|| env("CHESSGPT_LLM_API_KEY"));
        Some(ManagedLlm {
            kind,
            model: env("CHESSGPT_LLM_MODEL").unwrap_or_else(|| kind.default_model().into()),
            base_url: env("CHESSGPT_LLM_BASE_URL").unwrap_or_else(|| kind.default_base_url().into()),
            api_key: key,
            daily_tokens: env("CHESSGPT_LLM_DAILY_TOKENS").and_then(|v| v.parse().ok()).filter(|v| *v > 0),
        })
    }

    pub fn label(&self) -> String {
        let name = match self.kind {
            ProviderKind::Anthropic => "Claude",
            ProviderKind::Openai => "OpenAI",
            ProviderKind::Openrouter => "OpenRouter",
            ProviderKind::Ollama => "Ollama",
            ProviderKind::OpenaiCompatible => "Local model",
        };
        format!("{name} ({})", self.model)
    }
}

/// Tokens spent today (UTC), shared by every request.
#[derive(Default)]
pub struct Budget {
    inner: Mutex<(u64, u64)>, // (day number, tokens)
}

fn today() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0)
}

impl Budget {
    pub fn used(&self) -> u64 {
        let mut g = self.inner.lock().unwrap();
        if g.0 != today() {
            *g = (today(), 0);
        }
        g.1
    }

    fn add(&self, n: u64) {
        let mut g = self.inner.lock().unwrap();
        if g.0 != today() {
            *g = (today(), 0);
        }
        g.1 += n;
    }
}

/// Wraps the operator's provider: refuses new requests once the daily budget
/// is spent and counts every request's tokens.
pub struct Budgeted {
    pub inner: Arc<dyn Provider>,
    pub budget: Arc<Budget>,
    pub limit: Option<u64>,
}

#[async_trait::async_trait]
impl Provider for Budgeted {
    fn kind(&self) -> &str {
        self.inner.kind()
    }

    fn model(&self) -> &str {
        self.inner.model()
    }

    fn caps(&self) -> Caps {
        self.inner.caps()
    }

    async fn send(
        &self,
        req: &ChatRequest,
        events: Option<&mpsc::UnboundedSender<StreamEvent>>,
    ) -> Result<Completion, LlmError> {
        if let Some(limit) = self.limit
            && self.budget.used() >= limit
        {
            return Err(LlmError::Api {
                provider: "chessgpt".into(),
                status: 429,
                message: "The coach has reached today's usage limit on this site. Engine analysis still works; the coach is back tomorrow.".into(),
            });
        }
        let c = self.inner.send(req, events).await?;
        self.budget.add(c.usage.input_tokens as u64 + c.usage.output_tokens as u64);
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm::{Message, Part, Role, StopReason, Usage};

    struct Fixed;
    #[async_trait::async_trait]
    impl Provider for Fixed {
        fn kind(&self) -> &str {
            "fixed"
        }
        fn model(&self) -> &str {
            "m"
        }
        fn caps(&self) -> Caps {
            Caps { tools: true, json_schema: true }
        }
        async fn send(&self, _: &ChatRequest, _: Option<&mpsc::UnboundedSender<StreamEvent>>) -> Result<Completion, LlmError> {
            Ok(Completion {
                message: Message { role: Role::Assistant, parts: vec![Part::Text { text: "ok".into() }] },
                stop: StopReason::EndTurn,
                usage: Usage { input_tokens: 600, output_tokens: 400, ..Default::default() },
                model: "m".into(),
            })
        }
    }

    #[tokio::test]
    async fn budget_stops_after_limit() {
        let b = Budgeted { inner: Arc::new(Fixed), budget: Arc::new(Budget::default()), limit: Some(1500) };
        let req = ChatRequest::new("s", vec![Message::user("x")]);
        assert!(b.send(&req, None).await.is_ok());
        assert!(b.send(&req, None).await.is_ok(), "1000 used, still under 1500");
        let e = b.send(&req, None).await.unwrap_err();
        assert!(e.to_string().contains("usage limit"), "{e}");
        assert_eq!(b.budget.used(), 2000);
    }
}
