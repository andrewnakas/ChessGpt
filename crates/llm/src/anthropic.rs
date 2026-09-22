//! Anthropic Messages API over raw HTTP with SSE streaming.
//!
//! Request conventions (see the Claude API docs):
//! - adaptive thinking with summarized display, `output_config.effort`
//! - `output_config.format` for structured JSON replies
//! - client tools streamed with `eager_input_streaming`, parsed strictly at
//!   `content_block_stop`; unparseable input is surfaced, never executed
//! - server-side refusal fallbacks (`fallbacks: "default"`) on models that
//!   support them, retried without if the API rejects the parameter
//! - thinking / redacted / fallback blocks kept as opaque parts and echoed back

use futures::StreamExt;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::ir::*;
use crate::{Caps, LlmError, Provider, ProviderConfig};

const VERSION: &str = "2023-06-01";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
const PROVIDER: &str = "anthropic";

pub struct Anthropic {
    client: reqwest::Client,
    cfg: ProviderConfig,
}

impl Anthropic {
    pub fn new(client: reqwest::Client, cfg: ProviderConfig) -> Anthropic {
        Anthropic { client, cfg }
    }

    fn supports_adaptive_thinking(&self) -> bool {
        let m = &self.cfg.model;
        !(m.contains("haiku") || m.starts_with("claude-3") || m.contains("-4-5") || m.contains("-4-1") || m.contains("-4-0"))
    }

    fn supports_fallbacks(&self) -> bool {
        let m = &self.cfg.model;
        m.starts_with("claude-opus-5") || m.starts_with("claude-fable-5") || m.starts_with("claude-mythos-5")
    }

    pub fn body(&self, req: &ChatRequest, with_fallbacks: bool) -> Value {
        let mut body = json!({
            "model": self.cfg.model,
            "max_tokens": req.max_tokens,
            "stream": true,
            "system": [{"type": "text", "text": req.system, "cache_control": {"type": "ephemeral"}}],
            "messages": to_wire(&req.messages),
        });
        if !req.tools.is_empty() {
            body["tools"] = req
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.input_schema,
                        "strict": true,
                        "eager_input_streaming": true,
                    })
                })
                .collect();
        }
        if self.supports_adaptive_thinking() {
            body["thinking"] = json!({"type": "adaptive", "display": "summarized"});
        }
        let mut oc = serde_json::Map::new();
        if let Some(e) = req.effort
            && self.supports_adaptive_thinking()
        {
            oc.insert("effort".into(), serde_json::to_value(e).unwrap());
        }
        if let Some(s) = &req.json_schema {
            oc.insert("format".into(), json!({"type": "json_schema", "schema": s.schema}));
        }
        if !oc.is_empty() {
            body["output_config"] = Value::Object(oc);
        }
        if with_fallbacks {
            body["fallbacks"] = json!("default");
        }
        body
    }

    async fn post(&self, body: &Value, with_fallbacks: bool) -> Result<reqwest::Response, LlmError> {
        let key = self.cfg.api_key.clone().unwrap_or_default();
        let mut rb = self
            .client
            .post(format!("{}/v1/messages", self.cfg.base_url.trim_end_matches('/')))
            .header("x-api-key", key)
            .header("anthropic-version", VERSION)
            .header("content-type", "application/json")
            .json(body);
        if with_fallbacks {
            rb = rb.header("anthropic-beta", FALLBACK_BETA);
        }
        rb.send()
            .await
            .map_err(|e| LlmError::Network { provider: PROVIDER.into(), message: e.to_string() })
    }
}

/// Convert IR messages to the Messages API shape. Consecutive same-role
/// messages are merged; opaque blocks from other providers are dropped; after
/// a mid-output fallback, model-internal blocks before the last `fallback`
/// marker are omitted as the API requires.
pub fn to_wire(messages: &[Message]) -> Vec<Value> {
    let mut out: Vec<Value> = vec![];
    for m in messages {
        let role = match m.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        let last_fallback = m.parts.iter().rposition(|p| {
            matches!(p, Part::Opaque { provider, block } if provider == PROVIDER && block["type"] == "fallback")
        });
        let mut blocks = vec![];
        for (i, p) in m.parts.iter().enumerate() {
            let before_fallback = last_fallback.is_some_and(|f| i < f);
            match p {
                Part::Text { text } if !text.is_empty() => blocks.push(json!({"type": "text", "text": text})),
                Part::Text { .. } => {}
                Part::ToolCall { id, name, input } => {
                    if !before_fallback {
                        let input = if invalid_json_input(input).is_some() { json!({}) } else { input.clone() };
                        blocks.push(json!({"type": "tool_use", "id": id, "name": name, "input": input}));
                    }
                }
                Part::ToolResult { call_id, content, is_error } => blocks.push(json!({
                    "type": "tool_result", "tool_use_id": call_id, "content": content, "is_error": is_error
                })),
                Part::Opaque { provider, block } if provider == PROVIDER => {
                    let t = block["type"].as_str().unwrap_or("");
                    if before_fallback && t != "text" {
                        continue;
                    }
                    blocks.push(block.clone());
                }
                Part::Opaque { .. } => {}
            }
        }
        if blocks.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(prev) if prev["role"] == role => {
                prev["content"].as_array_mut().unwrap().extend(blocks);
            }
            _ => out.push(json!({"role": role, "content": blocks})),
        }
    }
    out
}

enum Acc {
    Text(String),
    Thinking { thinking: String, signature: String },
    Tool { id: String, name: String, json: String },
    Other(Value),
}

#[async_trait::async_trait]
impl Provider for Anthropic {
    fn kind(&self) -> &str {
        PROVIDER
    }

    fn model(&self) -> &str {
        &self.cfg.model
    }

    fn caps(&self) -> Caps {
        Caps { tools: true, json_schema: true }
    }

    async fn send(
        &self,
        req: &ChatRequest,
        events: Option<&mpsc::UnboundedSender<StreamEvent>>,
    ) -> Result<Completion, LlmError> {
        let mut with_fallbacks = self.supports_fallbacks();
        let resp = loop {
            let body = self.body(req, with_fallbacks);
            let resp = self.post(&body, with_fallbacks).await?;
            let status = resp.status().as_u16();
            if status == 200 {
                break resp;
            }
            let text = resp.text().await.unwrap_or_default();
            if status == 400 && with_fallbacks && text.contains("fallback") {
                tracing::warn!("anthropic rejected fallbacks, retrying without: {text}");
                with_fallbacks = false;
                continue;
            }
            let message = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(String::from))
                .unwrap_or(text);
            return Err(LlmError::Api { provider: PROVIDER.into(), status, message });
        };

        let mut stream = eventsource_stream::EventStream::new(resp.bytes_stream());
        let mut blocks: Vec<(usize, Acc)> = vec![];
        let mut usage = Usage::default();
        let mut model = self.cfg.model.clone();
        let mut stop: Option<StopReason> = None;
        let stream_err = |m: String| LlmError::Stream { provider: PROVIDER.into(), message: m };

        while let Some(ev) = stream.next().await {
            let ev = ev.map_err(|e| stream_err(e.to_string()))?;
            if ev.data.is_empty() {
                continue;
            }
            let data: Value = serde_json::from_str(&ev.data).map_err(|e| stream_err(format!("bad event json: {e}")))?;
            match data["type"].as_str().unwrap_or(&ev.event) {
                "message_start" => {
                    let m = &data["message"];
                    if let Some(s) = m["model"].as_str() {
                        model = s.to_string();
                    }
                    read_usage(&m["usage"], &mut usage);
                }
                "content_block_start" => {
                    let idx = data["index"].as_u64().unwrap_or(0) as usize;
                    let cb = &data["content_block"];
                    let acc = match cb["type"].as_str().unwrap_or("") {
                        "text" => Acc::Text(cb["text"].as_str().unwrap_or("").to_string()),
                        "thinking" => Acc::Thinking {
                            thinking: cb["thinking"].as_str().unwrap_or("").to_string(),
                            signature: cb["signature"].as_str().unwrap_or("").to_string(),
                        },
                        "tool_use" => {
                            let id = cb["id"].as_str().unwrap_or("").to_string();
                            let name = cb["name"].as_str().unwrap_or("").to_string();
                            if let Some(tx) = events {
                                let _ = tx.send(StreamEvent::ToolCallStart { id: id.clone(), name: name.clone() });
                            }
                            Acc::Tool { id, name, json: String::new() }
                        }
                        _ => Acc::Other(cb.clone()),
                    };
                    blocks.push((idx, acc));
                }
                "content_block_delta" => {
                    let idx = data["index"].as_u64().unwrap_or(0) as usize;
                    let d = &data["delta"];
                    let Some((_, acc)) = blocks.iter_mut().rev().find(|(i, _)| *i == idx) else { continue };
                    match (d["type"].as_str().unwrap_or(""), acc) {
                        ("text_delta", Acc::Text(t)) => {
                            let s = d["text"].as_str().unwrap_or("");
                            t.push_str(s);
                            if let Some(tx) = events {
                                let _ = tx.send(StreamEvent::TextDelta(s.to_string()));
                            }
                        }
                        ("thinking_delta", Acc::Thinking { thinking, .. }) => {
                            let s = d["thinking"].as_str().unwrap_or("");
                            thinking.push_str(s);
                            if let Some(tx) = events {
                                let _ = tx.send(StreamEvent::ThinkingDelta(s.to_string()));
                            }
                        }
                        ("signature_delta", Acc::Thinking { signature, .. }) => {
                            signature.push_str(d["signature"].as_str().unwrap_or(""));
                        }
                        ("input_json_delta", Acc::Tool { json, .. }) => {
                            json.push_str(d["partial_json"].as_str().unwrap_or(""));
                        }
                        _ => {}
                    }
                }
                "message_delta" => {
                    let d = &data["delta"];
                    if let Some(r) = d["stop_reason"].as_str() {
                        stop = Some(match r {
                            "end_turn" | "stop_sequence" => StopReason::EndTurn,
                            "tool_use" => StopReason::ToolUse,
                            "max_tokens" => StopReason::MaxTokens,
                            "refusal" => StopReason::Refusal(
                                d["stop_details"]["category"]
                                    .as_str()
                                    .or(data["stop_details"]["category"].as_str())
                                    .map(String::from),
                            ),
                            other => StopReason::Other(other.to_string()),
                        });
                    }
                    read_usage(&data["usage"], &mut usage);
                }
                "error" => {
                    let e = &data["error"];
                    let status = match e["type"].as_str() {
                        Some("overloaded_error") => 529,
                        Some("rate_limit_error") => 429,
                        Some("api_error") => 500,
                        _ => 400,
                    };
                    return Err(LlmError::Api {
                        provider: PROVIDER.into(),
                        status,
                        message: e["message"].as_str().unwrap_or("stream error").to_string(),
                    });
                }
                "message_stop" => break,
                _ => {}
            }
        }

        blocks.sort_by_key(|(i, _)| *i);
        let parts = blocks
            .into_iter()
            .map(|(_, acc)| match acc {
                Acc::Text(text) => Part::Text { text },
                Acc::Thinking { thinking, signature } => Part::Opaque {
                    provider: PROVIDER.into(),
                    block: json!({"type": "thinking", "thinking": thinking, "signature": signature}),
                },
                Acc::Tool { id, name, json } => {
                    let src = if json.trim().is_empty() { "{}" } else { json.as_str() };
                    let input = match serde_json::from_str::<Value>(src) {
                        Ok(v @ Value::Object(_)) => v,
                        _ => json!({ INVALID_JSON_KEY: json }),
                    };
                    Part::ToolCall { id, name, input }
                }
                Acc::Other(block) => Part::Opaque { provider: PROVIDER.into(), block },
            })
            .collect();
        Ok(Completion {
            message: Message { role: Role::Assistant, parts },
            stop: stop.ok_or_else(|| stream_err("stream ended without a stop reason".into()))?,
            usage,
            model,
        })
    }
}

fn read_usage(u: &Value, usage: &mut Usage) {
    let get = |k: &str| u[k].as_u64().map(|v| v as u32);
    if let Some(v) = get("input_tokens") {
        usage.input_tokens = v;
    }
    if let Some(v) = get("output_tokens") {
        usage.output_tokens = v;
    }
    if let Some(v) = get("cache_read_input_tokens") {
        usage.cache_read_tokens = v;
    }
    if let Some(v) = get("cache_creation_input_tokens") {
        usage.cache_write_tokens = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api_types::ProviderKind;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sse(events: &[(&str, Value)]) -> String {
        events.iter().map(|(e, d)| format!("event: {e}\ndata: {d}\n\n")).collect()
    }

    fn provider(base: &str, model: &str) -> Anthropic {
        Anthropic::new(
            crate::http_client(),
            ProviderConfig {
                kind: ProviderKind::Anthropic,
                base_url: base.into(),
                model: model.into(),
                api_key: Some("sk-test".into()),
            },
        )
    }

    fn tool_turn() -> String {
        sse(&[
            ("message_start", json!({"type": "message_start", "message": {"id": "msg_1", "model": "claude-opus-5", "usage": {"input_tokens": 120, "cache_read_input_tokens": 100, "output_tokens": 1}}})),
            ("content_block_start", json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": "", "signature": ""}})),
            ("content_block_delta", json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "Need the engine."}})),
            ("content_block_delta", json!({"type": "content_block_delta", "index": 0, "delta": {"type": "signature_delta", "signature": "sig123"}})),
            ("content_block_stop", json!({"type": "content_block_stop", "index": 0})),
            ("content_block_start", json!({"type": "content_block_start", "index": 1, "content_block": {"type": "text", "text": ""}})),
            ("content_block_delta", json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "Let me check."}})),
            ("content_block_stop", json!({"type": "content_block_stop", "index": 1})),
            ("content_block_start", json!({"type": "content_block_start", "index": 2, "content_block": {"type": "tool_use", "id": "toolu_1", "name": "analyse_position", "input": {}}})),
            ("content_block_delta", json!({"type": "content_block_delta", "index": 2, "delta": {"type": "input_json_delta", "partial_json": "{\"fen\": \"8/8"}})),
            ("content_block_delta", json!({"type": "content_block_delta", "index": 2, "delta": {"type": "input_json_delta", "partial_json": "/8/8/8/8/8/8 w - - 0 1\"}"}})),
            ("content_block_stop", json!({"type": "content_block_stop", "index": 2})),
            ("message_delta", json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 42}})),
            ("message_stop", json!({"type": "message_stop"})),
        ])
    }

    #[tokio::test]
    async fn streams_thinking_text_and_tool_call() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "sk-test"))
            .and(header("anthropic-version", VERSION))
            .and(header("anthropic-beta", FALLBACK_BETA))
            .respond_with(ResponseTemplate::new(200).set_body_raw(tool_turn(), "text/event-stream"))
            .mount(&server)
            .await;
        let p = provider(&server.uri(), "claude-opus-5");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let c = p.send(&ChatRequest::new("sys", vec![Message::user("hi")]), Some(&tx)).await.unwrap();
        assert_eq!(c.stop, StopReason::ToolUse);
        assert_eq!(c.usage.input_tokens, 120);
        assert_eq!(c.usage.cache_read_tokens, 100);
        assert_eq!(c.usage.output_tokens, 42);
        assert_eq!(c.message.parts.len(), 3);
        assert!(matches!(&c.message.parts[0], Part::Opaque { block, .. } if block["signature"] == "sig123"));
        assert_eq!(c.message.text(), "Let me check.");
        let calls = c.message.tool_calls();
        assert_eq!(calls[0].1, "analyse_position");
        assert_eq!(calls[0].2["fen"], "8/8/8/8/8/8/8/8 w - - 0 1");
        drop(tx);
        let mut evs = vec![];
        while let Some(e) = rx.recv().await {
            evs.push(e);
        }
        assert!(evs.contains(&StreamEvent::TextDelta("Let me check.".into())));
        assert!(evs.iter().any(|e| matches!(e, StreamEvent::ToolCallStart { name, .. } if name == "analyse_position")));

        // The request body carried the documented fields.
        let reqs = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["fallbacks"], "default");
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(body["stream"], true);
    }

    #[tokio::test]
    async fn invalid_tool_json_is_flagged_not_parsed() {
        let server = MockServer::start().await;
        let body = sse(&[
            ("message_start", json!({"type": "message_start", "message": {"model": "claude-opus-5", "usage": {"input_tokens": 1}}})),
            ("content_block_start", json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": "t", "name": "play_line"}})),
            ("content_block_delta", json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": "{\"moves\": [\"e4\""}})),
            ("content_block_stop", json!({"type": "content_block_stop", "index": 0})),
            ("message_delta", json!({"type": "message_delta", "delta": {"stop_reason": "max_tokens"}, "usage": {"output_tokens": 5}})),
            ("message_stop", json!({"type": "message_stop"})),
        ]);
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .mount(&server)
            .await;
        let c = provider(&server.uri(), "claude-opus-5")
            .send(&ChatRequest::new("s", vec![Message::user("x")]), None)
            .await
            .unwrap();
        assert_eq!(c.stop, StopReason::MaxTokens);
        let (_, _, input) = c.message.tool_calls()[0];
        assert!(invalid_json_input(input).is_some());
    }

    #[tokio::test]
    async fn refusal_category_and_fallback_retry() {
        let server = MockServer::start().await;
        // First call rejects the fallbacks parameter, second succeeds with a refusal.
        Mock::given(method("POST"))
            .and(header("anthropic-beta", FALLBACK_BETA))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({"type": "error", "error": {"type": "invalid_request_error", "message": "fallbacks: not supported for this model"}})))
            .mount(&server)
            .await;
        let body = sse(&[
            ("message_start", json!({"type": "message_start", "message": {"model": "claude-opus-5", "usage": {"input_tokens": 0}}})),
            ("message_delta", json!({"type": "message_delta", "delta": {"stop_reason": "refusal", "stop_details": {"type": "refusal", "category": "cyber"}}, "usage": {"output_tokens": 0}})),
            ("message_stop", json!({"type": "message_stop"})),
        ]);
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .mount(&server)
            .await;
        let c = provider(&server.uri(), "claude-opus-5")
            .send(&ChatRequest::new("s", vec![Message::user("x")]), None)
            .await
            .unwrap();
        assert_eq!(c.stop, StopReason::Refusal(Some("cyber".into())));
    }

    #[tokio::test]
    async fn api_errors_surface_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({"type": "error", "error": {"type": "authentication_error", "message": "invalid x-api-key"}})))
            .mount(&server)
            .await;
        let e = provider(&server.uri(), "claude-sonnet-5")
            .send(&ChatRequest::new("s", vec![Message::user("x")]), None)
            .await
            .unwrap_err();
        assert!(e.to_string().contains("invalid x-api-key"), "{e}");
        assert!(!e.is_retryable());
    }

    #[test]
    fn wire_conversion_rules() {
        let msgs = vec![
            Message::user("q"),
            Message {
                role: Role::Assistant,
                parts: vec![
                    Part::Opaque { provider: "anthropic".into(), block: json!({"type": "thinking", "thinking": "a", "signature": "s"}) },
                    Part::Opaque { provider: "openai".into(), block: json!({"x": 1}) },
                    Part::Text { text: "partial".into() },
                    Part::ToolCall { id: "t0".into(), name: "legal_moves".into(), input: json!({}) },
                    Part::Opaque { provider: "anthropic".into(), block: json!({"type": "fallback", "from": {"model": "a"}, "to": {"model": "b"}}) },
                    Part::Text { text: "rest".into() },
                    Part::ToolCall { id: "t1".into(), name: "legal_moves".into(), input: json!({"fen": "x"}) },
                ],
            },
            Message { role: Role::User, parts: vec![Part::ToolResult { call_id: "t1".into(), content: "ok".into(), is_error: false }] },
            Message::user("follow-up in the same user turn"),
        ];
        let w = to_wire(&msgs);
        assert_eq!(w.len(), 3, "consecutive user messages merge");
        let a = w[1]["content"].as_array().unwrap();
        let types: Vec<&str> = a.iter().map(|b| b["type"].as_str().unwrap()).collect();
        assert_eq!(types, vec!["text", "fallback", "text", "tool_use"], "internal blocks before the fallback marker are dropped");
        assert_eq!(w[2]["content"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn body_shapes() {
        let p = provider("http://x", "claude-opus-5");
        let mut req = ChatRequest::new("sys", vec![Message::user("hi")]);
        req.tools = vec![ToolDef { name: "t".into(), description: "d".into(), input_schema: json!({"type": "object", "properties": {}, "required": [], "additionalProperties": false}) }];
        req.effort = Some(Effort::High);
        req.json_schema = Some(JsonSchema { name: "x".into(), schema: json!({"type": "object"}) });
        let b = p.body(&req, true);
        assert_eq!(b["tools"][0]["strict"], true);
        assert_eq!(b["tools"][0]["eager_input_streaming"], true);
        assert_eq!(b["output_config"]["effort"], "high");
        assert_eq!(b["output_config"]["format"]["type"], "json_schema");
        let haiku = provider("http://x", "claude-haiku-4-5");
        let b = haiku.body(&req, false);
        assert!(b.get("thinking").is_none());
        assert!(b["output_config"].get("effort").is_none());
        assert!(b.get("fallbacks").is_none());
    }
}
