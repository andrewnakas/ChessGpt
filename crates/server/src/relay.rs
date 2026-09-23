//! The coach model running in the user's browser (WebGPU). A tab with the
//! model loaded holds `GET /api/llm/device` open; while it does, the coach's
//! requests for that user go to the tab as OpenAI-style bodies and come back
//! through `POST /api/llm/relay/{id}`. The operator's model, if any, is the
//! fallback when the tab is gone or too slow.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use api_types::{LlmRelayRequest, ProviderKind};
use llm::openai::{OpenAiCompat, completion_from_response};
use llm::{Caps, ChatRequest, Completion, LlmError, Provider, ProviderConfig, StreamEvent, prompted};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

/// How long one request may take in the browser, queueing included.
const DEVICE_TIMEOUT: Duration = Duration::from_secs(180);

struct Device {
    conn: u64,
    model: String,
    tx: mpsc::UnboundedSender<LlmRelayRequest>,
}

#[derive(Default)]
pub struct RelayHub {
    next_conn: AtomicU64,
    devices: Mutex<HashMap<String, Device>>,
    pending: Mutex<HashMap<String, (String, oneshot::Sender<Value>)>>,
}

/// Unregisters the device when its event stream is dropped.
pub struct DeviceGuard {
    hub: Arc<RelayHub>,
    user: String,
    conn: u64,
}

impl Drop for DeviceGuard {
    fn drop(&mut self) {
        let mut d = self.hub.devices.lock().unwrap();
        if d.get(&self.user).is_some_and(|dev| dev.conn == self.conn) {
            d.remove(&self.user);
        }
    }
}

impl RelayHub {
    /// Register a tab as `user`'s device; the newest tab wins.
    pub fn connect(
        self: &Arc<Self>,
        user: &str,
        model: String,
    ) -> (DeviceGuard, mpsc::UnboundedReceiver<LlmRelayRequest>) {
        let conn = self.next_conn.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::unbounded_channel();
        self.devices.lock().unwrap().insert(user.to_string(), Device { conn, model, tx });
        (DeviceGuard { hub: self.clone(), user: user.to_string(), conn }, rx)
    }

    pub fn device_model(&self, user: &str) -> Option<String> {
        self.devices.lock().unwrap().get(user).filter(|d| !d.tx.is_closed()).map(|d| d.model.clone())
    }

    /// Deliver a browser's answer; false if the id is unknown or not `user`'s.
    pub fn reply(&self, user: &str, id: &str, v: Value) -> bool {
        let mut p = self.pending.lock().unwrap();
        match p.get(id) {
            Some((owner, _)) if owner == user => {
                let (_, tx) = p.remove(id).expect("present");
                tx.send(v).is_ok()
            }
            _ => false,
        }
    }

    async fn call(&self, user: &str, body: Value) -> Result<Value, String> {
        let id = db::new_id();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), (user.to_string(), tx));
        let sent = self
            .devices
            .lock()
            .unwrap()
            .get(user)
            .is_some_and(|d| d.tx.send(LlmRelayRequest { id: id.clone(), body }).is_ok());
        let res = if !sent {
            Err("the browser model is not connected".to_string())
        } else {
            match tokio::time::timeout(DEVICE_TIMEOUT, rx).await {
                Ok(Ok(v)) => Ok(v),
                Ok(Err(_)) => Err("the browser model went away".to_string()),
                Err(_) => Err("the browser model took too long".to_string()),
            }
        };
        self.pending.lock().unwrap().remove(&id);
        res
    }
}

/// A provider backed by the user's browser, with an optional fallback.
pub struct Relay {
    pub hub: Arc<RelayHub>,
    pub user: String,
    pub model: String,
    pub fallback: Option<Arc<dyn Provider>>,
    /// Set after the first failure so the rest of a job skips the browser.
    pub dead: AtomicBool,
}

impl Relay {
    fn wire(&self) -> OpenAiCompat {
        OpenAiCompat::new(
            llm::http_client(),
            ProviderConfig {
                kind: ProviderKind::OpenaiCompatible,
                base_url: String::new(),
                model: self.model.clone(),
                api_key: None,
            },
        )
    }

    async fn via_device(&self, req: &ChatRequest) -> Result<Completion, String> {
        let body = self.wire().body_with(&prompted::lower(req), false);
        let v = self.hub.call(&self.user, body).await?;
        let c = completion_from_response("browser", &v, &self.model).map_err(|e| e.to_string())?;
        let seq = req.messages.iter().map(|m| m.tool_calls().len()).sum();
        Ok(prompted::raise(c, &req.tools, seq))
    }
}

#[async_trait::async_trait]
impl Provider for Relay {
    fn kind(&self) -> &str {
        "browser"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn caps(&self) -> Caps {
        Caps { tools: true, json_schema: true }
    }

    async fn send(
        &self,
        req: &ChatRequest,
        events: Option<&mpsc::UnboundedSender<StreamEvent>>,
    ) -> Result<Completion, LlmError> {
        if !self.dead.load(Ordering::Relaxed) {
            match self.via_device(req).await {
                Ok(c) => {
                    if let Some(tx) = events
                        && req.tools.is_empty()
                    {
                        let text = c.message.text();
                        if !text.is_empty() {
                            let _ = tx.send(StreamEvent::TextDelta(text));
                        }
                    }
                    return Ok(c);
                }
                Err(e) => {
                    tracing::info!("browser model failed ({e}); using the fallback");
                    self.dead.store(true, Ordering::Relaxed);
                    if self.fallback.is_none() {
                        return Err(LlmError::Network { provider: "browser".into(), message: e });
                    }
                }
            }
        }
        match &self.fallback {
            Some(f) => f.send(req, events).await,
            None => Err(LlmError::Network {
                provider: "browser".into(),
                message: "the browser model is not available".into(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm::Message;
    use serde_json::json;

    #[tokio::test]
    async fn round_trip_and_owner_check() {
        let hub = Arc::new(RelayHub::default());
        let (_guard, mut rx) = hub.connect("u1", "qwen".into());
        let relay = Relay { hub: hub.clone(), user: "u1".into(), model: "qwen".into(), fallback: None, dead: AtomicBool::new(false) };
        let h2 = hub.clone();
        let answer = tokio::spawn(async move {
            let r = rx.recv().await.unwrap();
            assert_eq!(r.body["stream"], false);
            assert!(!h2.reply("someone-else", &r.id, json!({})));
            let reply = json!({"choices": [{"message": {"content": "hello"}, "finish_reason": "stop"}]});
            assert!(h2.reply("u1", &r.id, reply));
        });
        let c = relay.send(&ChatRequest::new("s", vec![Message::user("hi")]), None).await.unwrap();
        answer.await.unwrap();
        assert_eq!(c.message.text(), "hello");
    }

    #[tokio::test]
    async fn disconnected_device_fails_over() {
        let hub = Arc::new(RelayHub::default());
        let (guard, rx) = hub.connect("u1", "qwen".into());
        assert_eq!(hub.device_model("u1").as_deref(), Some("qwen"));
        drop(rx);
        drop(guard);
        assert!(hub.device_model("u1").is_none());
        let relay = Relay { hub, user: "u1".into(), model: "qwen".into(), fallback: None, dead: AtomicBool::new(false) };
        let e = relay.send(&ChatRequest::new("s", vec![Message::user("hi")]), None).await.unwrap_err();
        assert!(e.to_string().contains("not connected"), "{e}");
        assert!(relay.dead.load(Ordering::Relaxed));
    }
}
