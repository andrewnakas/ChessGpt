//! Provider-neutral conversation representation. Stored verbatim in the
//! database and replayed append-only, so thinking blocks and other
//! provider-internal blocks survive across turns unchanged.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text {
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        call_id: String,
        content: String,
        is_error: bool,
    },
    /// A provider-internal block (Anthropic thinking with its signature,
    /// redacted thinking, fallback markers, ...). Echoed back unchanged to the
    /// same provider kind and dropped for any other.
    Opaque {
        provider: String,
        block: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub parts: Vec<Part>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Message {
        Message { role: Role::User, parts: vec![Part::Text { text: text.into() }] }
    }

    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    pub fn tool_calls(&self) -> Vec<(&str, &str, &Value)> {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::ToolCall { id, name, input } => Some((id.as_str(), name.as_str(), input)),
                _ => None,
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// JSON Schema. Keep it strict-friendly: every object has
    /// `additionalProperties: false` and lists all properties in `required`
    /// (use `["string", "null"]` style types for optional values).
    pub input_schema: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JsonSchema {
    pub name: String,
    /// Strict-friendly schema (see [`ToolDef::input_schema`]).
    pub schema: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    /// Stable instructions; cached where the provider supports it.
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,
    pub max_tokens: u32,
    /// Ask for a JSON reply matching this schema.
    pub json_schema: Option<JsonSchema>,
    pub effort: Option<Effort>,
}

impl ChatRequest {
    pub fn new(system: impl Into<String>, messages: Vec<Message>) -> ChatRequest {
        ChatRequest {
            system: system.into(),
            messages,
            tools: vec![],
            max_tokens: 16_000,
            json_schema: None,
            effort: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    /// Declined by the model or a safety classifier (category if given).
    Refusal(Option<String>),
    Other(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_write_tokens: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    /// The assistant message, including opaque blocks, ready to append.
    pub message: Message,
    pub stop: StopReason,
    pub usage: Usage,
    /// Model that actually served the reply (may differ after a fallback).
    pub model: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolCallStart { id: String, name: String },
}

/// Key used inside a tool call's `input` when the model's JSON could not be
/// parsed; callers must not run such a call.
pub const INVALID_JSON_KEY: &str = "__invalid_json__";

pub fn invalid_json_input(call_input: &Value) -> Option<&str> {
    call_input.get(INVALID_JSON_KEY).and_then(Value::as_str)
}
