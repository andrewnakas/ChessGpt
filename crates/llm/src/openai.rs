//! OpenAI-compatible Chat Completions with streaming: OpenAI, OpenRouter,
//! Ollama, LM Studio, llama.cpp server, vLLM.

use std::collections::BTreeMap;

use api_types::ProviderKind;
use futures::StreamExt;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::ir::*;
use crate::{Caps, LlmError, Provider, ProviderConfig};

pub struct OpenAiCompat {
    client: reqwest::Client,
    cfg: ProviderConfig,
}

impl OpenAiCompat {
    pub fn new(client: reqwest::Client, cfg: ProviderConfig) -> OpenAiCompat {
        OpenAiCompat { client, cfg }
    }

    fn name(&self) -> &'static str {
        self.cfg.kind.as_str()
    }

    fn hosted(&self) -> bool {
        matches!(self.cfg.kind, ProviderKind::Openai | ProviderKind::Openrouter)
    }

    pub fn body(&self, req: &ChatRequest) -> Value {
        let mut system = req.system.clone();
        if let Some(s) = &req.json_schema
            && !self.hosted()
        {
            // Local servers: ask for JSON in the prompt and parse leniently.
            system.push_str(&format!(
                "\n\nReply with a single JSON object and nothing else. It must match this JSON Schema:\n{}",
                s.schema
            ));
        }
        let mut messages = vec![json!({"role": "system", "content": system})];
        messages.extend(to_wire(&req.messages));
        let mut body = json!({"model": self.cfg.model, "stream": true, "messages": messages});
        if self.cfg.kind == ProviderKind::Openai {
            body["max_completion_tokens"] = json!(req.max_tokens);
        } else {
            body["max_tokens"] = json!(req.max_tokens);
        }
        if self.hosted() {
            body["stream_options"] = json!({"include_usage": true});
        }
        if !req.tools.is_empty() {
            body["tools"] = req
                .tools
                .iter()
                .map(|t| {
                    json!({"type": "function", "function": {
                        "name": t.name, "description": t.description, "parameters": t.input_schema
                    }})
                })
                .collect();
        }
        if let Some(s) = &req.json_schema
            && self.hosted()
        {
            body["response_format"] = json!({"type": "json_schema", "json_schema": {
                "name": s.name, "schema": s.schema, "strict": true
            }});
        }
        if let Some(e) = req.effort
            && self.cfg.kind == ProviderKind::Openai
            && (self.cfg.model.starts_with('o') || self.cfg.model.starts_with("gpt-5"))
        {
            body["reasoning_effort"] = json!(match e {
                Effort::Low => "low",
                Effort::Medium => "medium",
                _ => "high",
            });
        }
        body
    }
}

pub fn to_wire(messages: &[Message]) -> Vec<Value> {
    let mut out = vec![];
    for m in messages {
        match m.role {
            Role::User => {
                let mut text = String::new();
                for p in &m.parts {
                    match p {
                        Part::Text { text: t } => {
                            if !text.is_empty() {
                                text.push_str("\n\n");
                            }
                            text.push_str(t);
                        }
                        Part::ToolResult { call_id, content, is_error } => {
                            let content = if *is_error { format!("ERROR: {content}") } else { content.clone() };
                            out.push(json!({"role": "tool", "tool_call_id": call_id, "content": content}));
                        }
                        _ => {}
                    }
                }
                if !text.is_empty() {
                    out.push(json!({"role": "user", "content": text}));
                }
            }
            Role::Assistant => {
                let text = m.text();
                let calls: Vec<Value> = m
                    .parts
                    .iter()
                    .filter_map(|p| match p {
                        Part::ToolCall { id, name, input } => {
                            let args = if invalid_json_input(input).is_some() { "{}".to_string() } else { input.to_string() };
                            Some(json!({"id": id, "type": "function", "function": {"name": name, "arguments": args}}))
                        }
                        _ => None,
                    })
                    .collect();
                let mut msg = json!({"role": "assistant", "content": if text.is_empty() { Value::Null } else { json!(text) }});
                if !calls.is_empty() {
                    msg["tool_calls"] = json!(calls);
                }
                if text.is_empty() && calls.is_empty() {
                    continue;
                }
                out.push(msg);
            }
        }
    }
    out
}

#[async_trait::async_trait]
impl Provider for OpenAiCompat {
    fn kind(&self) -> &str {
        self.name()
    }

    fn model(&self) -> &str {
        &self.cfg.model
    }

    fn caps(&self) -> Caps {
        Caps { tools: true, json_schema: self.hosted() }
    }

    async fn send(
        &self,
        req: &ChatRequest,
        events: Option<&mpsc::UnboundedSender<StreamEvent>>,
    ) -> Result<Completion, LlmError> {
        let provider = self.name().to_string();
        let mut rb = self
            .client
            .post(format!("{}/chat/completions", self.cfg.base_url.trim_end_matches('/')))
            .json(&self.body(req));
        if let Some(k) = self.cfg.api_key.as_deref().filter(|k| !k.is_empty()) {
            rb = rb.bearer_auth(k);
        }
        if self.cfg.kind == ProviderKind::Openrouter {
            rb = rb.header("HTTP-Referer", "https://chessgpt.com").header("X-Title", "chessgpt");
        }
        let resp = rb
            .send()
            .await
            .map_err(|e| LlmError::Network { provider: provider.clone(), message: e.to_string() })?;
        let status = resp.status().as_u16();
        if status != 200 {
            let text = resp.text().await.unwrap_or_default();
            let message = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| {
                    v["error"]["message"].as_str().or(v["error"].as_str()).map(String::from)
                })
                .unwrap_or(text);
            return Err(LlmError::Api { provider, status, message });
        }

        let mut stream = eventsource_stream::EventStream::new(resp.bytes_stream());
        let mut text = String::new();
        let mut calls: BTreeMap<u64, (String, String, String)> = BTreeMap::new();
        let mut finish: Option<String> = None;
        let mut usage = Usage::default();
        let mut model = self.cfg.model.clone();
        while let Some(ev) = stream.next().await {
            let ev = ev.map_err(|e| LlmError::Stream { provider: provider.clone(), message: e.to_string() })?;
            if ev.data.trim() == "[DONE]" {
                break;
            }
            let Ok(data) = serde_json::from_str::<Value>(&ev.data) else { continue };
            if let Some(err) = data.get("error") {
                return Err(LlmError::Api {
                    provider,
                    status: 500,
                    message: err["message"].as_str().unwrap_or("stream error").to_string(),
                });
            }
            if let Some(m) = data["model"].as_str() {
                model = m.to_string();
            }
            if let Some(u) = data.get("usage").filter(|u| u.is_object()) {
                usage.input_tokens = u["prompt_tokens"].as_u64().unwrap_or(0) as u32;
                usage.output_tokens = u["completion_tokens"].as_u64().unwrap_or(0) as u32;
                usage.cache_read_tokens =
                    u["prompt_tokens_details"]["cached_tokens"].as_u64().unwrap_or(0) as u32;
            }
            let Some(choice) = data["choices"].get(0) else { continue };
            let d = &choice["delta"];
            if let Some(s) = d["content"].as_str().filter(|s| !s.is_empty()) {
                text.push_str(s);
                if let Some(tx) = events {
                    let _ = tx.send(StreamEvent::TextDelta(s.to_string()));
                }
            }
            if let Some(s) = d["reasoning_content"].as_str().or(d["reasoning"].as_str()).filter(|s| !s.is_empty())
                && let Some(tx) = events
            {
                let _ = tx.send(StreamEvent::ThinkingDelta(s.to_string()));
            }
            if let Some(tcs) = d["tool_calls"].as_array() {
                for (n, tc) in tcs.iter().enumerate() {
                    let idx = tc["index"].as_u64().unwrap_or(n as u64);
                    let entry = calls.entry(idx).or_default();
                    if let Some(id) = tc["id"].as_str().filter(|s| !s.is_empty()) {
                        entry.0 = id.to_string();
                    }
                    if let Some(name) = tc["function"]["name"].as_str().filter(|s| !s.is_empty()) {
                        entry.1.push_str(name);
                        if let Some(tx) = events {
                            let _ = tx.send(StreamEvent::ToolCallStart { id: entry.0.clone(), name: entry.1.clone() });
                        }
                    }
                    if let Some(a) = tc["function"]["arguments"].as_str() {
                        entry.2.push_str(a);
                    }
                }
            }
            if let Some(f) = choice["finish_reason"].as_str() {
                finish = Some(f.to_string());
            }
        }

        let mut parts = vec![];
        if !text.is_empty() {
            parts.push(Part::Text { text });
        }
        for (i, (id, name, args)) in calls {
            let src = if args.trim().is_empty() { "{}" } else { args.as_str() };
            let input = match serde_json::from_str::<Value>(src) {
                Ok(v @ Value::Object(_)) => v,
                _ => json!({ INVALID_JSON_KEY: args }),
            };
            let id = if id.is_empty() { format!("call_{i}") } else { id };
            parts.push(Part::ToolCall { id, name, input });
        }
        let has_calls = parts.iter().any(|p| matches!(p, Part::ToolCall { .. }));
        let stop = match finish.as_deref() {
            Some("tool_calls") | Some("function_call") => StopReason::ToolUse,
            Some("length") => StopReason::MaxTokens,
            Some("content_filter") => StopReason::Refusal(Some("content_filter".into())),
            _ if has_calls => StopReason::ToolUse,
            Some("stop") | None => StopReason::EndTurn,
            Some(other) => StopReason::Other(other.into()),
        };
        Ok(Completion { message: Message { role: Role::Assistant, parts }, stop, usage, model })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn chunks(cs: &[Value]) -> String {
        let mut s: String = cs.iter().map(|c| format!("data: {c}\n\n")).collect();
        s.push_str("data: [DONE]\n\n");
        s
    }

    fn p(kind: ProviderKind, base: &str) -> OpenAiCompat {
        OpenAiCompat::new(
            crate::http_client(),
            ProviderConfig { kind, base_url: base.into(), model: "m".into(), api_key: Some("k".into()) },
        )
    }

    #[tokio::test]
    async fn merges_streamed_tool_calls_by_index() {
        let server = MockServer::start().await;
        let body = chunks(&[
            json!({"model": "gpt-5", "choices": [{"delta": {"role": "assistant", "content": "Checking"}}]}),
            json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "call_a", "function": {"name": "legal_moves", "arguments": ""}}]}}]}),
            json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": "{\"fen\":"}}]}}]}),
            json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": "\"x\"}"}}, {"index": 1, "id": "call_b", "function": {"name": "play_line", "arguments": "{}"}}]}}]}),
            json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]}),
            json!({"choices": [], "usage": {"prompt_tokens": 50, "completion_tokens": 7, "prompt_tokens_details": {"cached_tokens": 10}}}),
        ]);
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer k"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .mount(&server)
            .await;
        let c = p(ProviderKind::Openai, &format!("{}/v1", server.uri()))
            .send(&ChatRequest::new("s", vec![Message::user("x")]), None)
            .await
            .unwrap();
        assert_eq!(c.stop, StopReason::ToolUse);
        assert_eq!(c.model, "gpt-5");
        assert_eq!(c.message.text(), "Checking");
        let calls = c.message.tool_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "call_a");
        assert_eq!(calls[0].2["fen"], "x");
        assert_eq!(calls[1].1, "play_line");
        assert_eq!(c.usage.input_tokens, 50);
        assert_eq!(c.usage.cache_read_tokens, 10);
        let reqs = server.received_requests().await.unwrap();
        let b: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert!(b.get("max_completion_tokens").is_some());
    }

    #[test]
    fn wire_and_body_per_kind() {
        let msgs = vec![
            Message::user("q"),
            Message {
                role: Role::Assistant,
                parts: vec![
                    Part::Opaque { provider: "anthropic".into(), block: json!({"type": "thinking"}) },
                    Part::ToolCall { id: "c1".into(), name: "legal_moves".into(), input: json!({"fen": "f"}) },
                ],
            },
            Message { role: Role::User, parts: vec![Part::ToolResult { call_id: "c1".into(), content: "e4".into(), is_error: false }] },
        ];
        let w = to_wire(&msgs);
        assert_eq!(w.len(), 3);
        assert_eq!(w[1]["tool_calls"][0]["function"]["arguments"], "{\"fen\":\"f\"}");
        assert!(w[1]["content"].is_null());
        assert_eq!(w[2]["role"], "tool");

        let mut req = ChatRequest::new("sys", msgs);
        req.json_schema = Some(JsonSchema { name: "x".into(), schema: json!({"type": "object"}) });
        let ollama = p(ProviderKind::Ollama, "http://localhost:11434/v1").body(&req);
        assert!(ollama.get("response_format").is_none());
        assert!(ollama["messages"][0]["content"].as_str().unwrap().contains("JSON Schema"));
        assert!(ollama.get("max_tokens").is_some());
        let or = p(ProviderKind::Openrouter, "https://openrouter.ai/api/v1").body(&req);
        assert_eq!(or["response_format"]["json_schema"]["strict"], true);
    }

    #[tokio::test]
    async fn openrouter_headers_and_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("X-Title", "chessgpt"))
            .respond_with(ResponseTemplate::new(402).set_body_json(json!({"error": {"message": "Insufficient credits"}})))
            .mount(&server)
            .await;
        let e = p(ProviderKind::Openrouter, &server.uri())
            .send(&ChatRequest::new("s", vec![Message::user("x")]), None)
            .await
            .unwrap_err();
        assert!(e.to_string().contains("Insufficient credits"), "{e}");
    }
}
