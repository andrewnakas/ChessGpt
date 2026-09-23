//! Tool calling for models without a native tool-call format (small local
//! models, the in-browser model): tools are described in the system prompt,
//! calls travel as one JSON object in the reply text, and results come back as
//! plain user text. [`Prompted`] wraps any provider this way.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::ir::*;
use crate::{Caps, LlmError, Provider};

fn tools_section(tools: &[ToolDef]) -> String {
    let mut s = String::from(
        "\n\nTOOLS\nYou can call these tools. To call one, reply with only this JSON object and nothing else:\n\
         {\"tool\": \"<name>\", \"arguments\": {...}}\n\
         You will get the result in the next message. Call one tool at a time. When you have what you need, answer normally in plain text (no JSON).\n",
    );
    for t in tools {
        s.push_str(&format!("\n- {}: {}\n  arguments schema: {}\n", t.name, t.description, t.input_schema));
    }
    s
}

/// Rewrite a request with tools into one without: tool definitions go into
/// the system prompt, past calls and results into plain text.
pub fn lower(req: &ChatRequest) -> ChatRequest {
    if req.tools.is_empty() {
        return req.clone();
    }
    let mut names: HashMap<String, String> = HashMap::new();
    let mut messages = Vec::with_capacity(req.messages.len());
    for m in &req.messages {
        let mut parts = vec![];
        for p in &m.parts {
            match p {
                Part::ToolCall { id, name, input } => {
                    names.insert(id.clone(), name.clone());
                    let call = json!({"tool": name, "arguments": input});
                    parts.push(Part::Text { text: call.to_string() });
                }
                Part::ToolResult { call_id, content, is_error } => {
                    let name = names.get(call_id).map(String::as_str).unwrap_or("tool");
                    let label = if *is_error { "TOOL ERROR" } else { "TOOL RESULT" };
                    parts.push(Part::Text { text: format!("{label} ({name}):\n{content}") });
                }
                other => parts.push(other.clone()),
            }
        }
        messages.push(Message { role: m.role, parts });
    }
    let mut out = req.clone();
    out.system.push_str(&tools_section(&req.tools));
    out.tools = vec![];
    out.messages = messages;
    out
}

/// Turn a reply that is exactly one `{"tool": ..., "arguments": ...}` object
/// naming a known tool into a tool call.
pub fn raise(mut c: Completion, tools: &[ToolDef], seq: usize) -> Completion {
    if tools.is_empty() || !c.message.tool_calls().is_empty() {
        return c;
    }
    let text = c.message.text();
    let t = text.trim();
    let looks_like_call = t.starts_with('{') || t.starts_with("```");
    let Some(v) = looks_like_call.then(|| crate::json::extract_object(t)).flatten() else { return c };
    let Some(name) = v.get("tool").and_then(Value::as_str) else { return c };
    if !tools.iter().any(|d| d.name == name) {
        return c;
    }
    let input = match v.get("arguments") {
        Some(a @ Value::Object(_)) => a.clone(),
        _ => json!({}),
    };
    c.message.parts = vec![Part::ToolCall { id: format!("call_p{seq}"), name: name.to_string(), input }];
    c.stop = StopReason::ToolUse;
    c
}

/// A provider that speaks tools through the prompt.
pub struct Prompted {
    pub inner: Arc<dyn Provider>,
}

#[async_trait::async_trait]
impl Provider for Prompted {
    fn kind(&self) -> &str {
        self.inner.kind()
    }

    fn model(&self) -> &str {
        self.inner.model()
    }

    fn caps(&self) -> Caps {
        Caps { tools: true, ..self.inner.caps() }
    }

    async fn send(
        &self,
        req: &ChatRequest,
        events: Option<&mpsc::UnboundedSender<StreamEvent>>,
    ) -> Result<Completion, LlmError> {
        // Streaming a JSON tool call as chat text would flash it at the user.
        let events = if req.tools.is_empty() { events } else { None };
        let c = self.inner.send(&lower(req), events).await?;
        let seq = req.messages.iter().map(|m| m.tool_calls().len()).sum();
        Ok(raise(c, &req.tools, seq))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> ToolDef {
        ToolDef {
            name: "legal_moves".into(),
            description: "List legal moves".into(),
            input_schema: json!({"type": "object", "properties": {"fen": {"type": "string"}}}),
        }
    }

    fn reply(text: &str) -> Completion {
        Completion {
            message: Message { role: Role::Assistant, parts: vec![Part::Text { text: text.into() }] },
            stop: StopReason::EndTurn,
            usage: Usage::default(),
            model: "m".into(),
        }
    }

    #[test]
    fn lowers_history_and_raises_calls() {
        let mut req = ChatRequest::new("sys", vec![
            Message::user("what can I play?"),
            Message {
                role: Role::Assistant,
                parts: vec![Part::ToolCall { id: "c1".into(), name: "legal_moves".into(), input: json!({"fen": "f"}) }],
            },
            Message { role: Role::User, parts: vec![Part::ToolResult { call_id: "c1".into(), content: "e4, d4".into(), is_error: false }] },
        ]);
        req.tools = vec![tool()];
        let low = lower(&req);
        assert!(low.tools.is_empty());
        assert!(low.system.contains("legal_moves: List legal moves"));
        assert_eq!(low.messages[1].text(), r#"{"arguments":{"fen":"f"},"tool":"legal_moves"}"#);
        assert!(low.messages[2].text().starts_with("TOOL RESULT (legal_moves):"));

        let c = raise(reply("```json\n{\"tool\": \"legal_moves\", \"arguments\": {\"fen\": \"x\"}}\n```"), &req.tools, 1);
        assert_eq!(c.stop, StopReason::ToolUse);
        assert_eq!(c.message.tool_calls()[0].2["fen"], "x");
        // Prose that merely contains JSON, or an unknown tool, stays text.
        assert_eq!(raise(reply("I would call {\"tool\": \"legal_moves\"}"), &req.tools, 1).stop, StopReason::EndTurn);
        assert_eq!(raise(reply("{\"tool\": \"nope\"}"), &req.tools, 1).stop, StopReason::EndTurn);
    }
}
