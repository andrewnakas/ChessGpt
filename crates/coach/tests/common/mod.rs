#![allow(dead_code)]

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use engine::{EngineConfig, EnginePool, find_stockfish};
use llm::{Caps, ChatRequest, Completion, LlmError, Message, Part, Provider, Role, StopReason, StreamEvent, Usage};
use tokio::sync::mpsc;

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap().to_path_buf()
}

pub fn fixture(name: &str) -> String {
    std::fs::read_to_string(root().join("fixtures/games").join(name)).unwrap()
}

/// One worker, one thread: deterministic depth-limited searches.
pub async fn deterministic_pool() -> Option<EnginePool> {
    let Some(path) = find_stockfish(&root()) else {
        eprintln!("SKIP: no Stockfish binary");
        return None;
    };
    let mut cfg = EngineConfig::with_defaults(path);
    cfg.workers = 1;
    cfg.threads = 1;
    cfg.hash_mb = 32;
    Some(EnginePool::start(cfg, None).await.unwrap())
}

/// Scripted provider: returns queued completions in order and records requests.
pub struct MockProvider {
    pub replies: Mutex<VecDeque<Completion>>,
    pub requests: Mutex<Vec<ChatRequest>>,
    pub json: bool,
}

impl MockProvider {
    pub fn new(replies: Vec<Completion>) -> MockProvider {
        MockProvider { replies: Mutex::new(replies.into()), requests: Mutex::new(vec![]), json: true }
    }
}

pub fn text_reply(text: &str) -> Completion {
    Completion {
        message: Message { role: Role::Assistant, parts: vec![Part::Text { text: text.into() }] },
        stop: StopReason::EndTurn,
        usage: Usage { input_tokens: 100, output_tokens: 50, ..Default::default() },
        model: "mock-1".into(),
    }
}

pub fn tool_reply(id: &str, name: &str, input: serde_json::Value) -> Completion {
    Completion {
        message: Message {
            role: Role::Assistant,
            parts: vec![
                Part::Text { text: "Let me check.".into() },
                Part::ToolCall { id: id.into(), name: name.into(), input },
            ],
        },
        stop: StopReason::ToolUse,
        usage: Usage::default(),
        model: "mock-1".into(),
    }
}

#[async_trait::async_trait]
impl Provider for MockProvider {
    fn kind(&self) -> &str {
        "mock"
    }
    fn model(&self) -> &str {
        "mock-1"
    }
    fn caps(&self) -> Caps {
        Caps { tools: true, json_schema: self.json }
    }
    async fn send(
        &self,
        req: &ChatRequest,
        events: Option<&mpsc::UnboundedSender<StreamEvent>>,
    ) -> Result<Completion, LlmError> {
        self.requests.lock().unwrap().push(req.clone());
        let c = self.replies.lock().unwrap().pop_front().expect("mock ran out of replies");
        if let Some(tx) = events {
            let t = c.message.text();
            if !t.is_empty() {
                let _ = tx.send(StreamEvent::TextDelta(t));
            }
        }
        Ok(c)
    }
}
