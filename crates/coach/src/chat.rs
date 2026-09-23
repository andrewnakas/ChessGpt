//! Coach chat: a manual tool-use loop over the board position, streamed to the
//! UI, with every answer verified against what the tools returned.

use api_types::{ChatEvent, ToolCallView, Verification};
use chess_core::position::{numbered_line, parse_fen, START_FEN};
use llm::{ChatRequest, Message, Part, Provider, Role, StopReason, StreamEvent, Usage, send_with_retry};
use shakmaty::{Color, Position};
use tokio::sync::mpsc;

use crate::prompts::level_guidance;
use crate::tools::{ToolEnv, run_tool, tool_defs};
use crate::verify::{Findings, Grounding};

const CHAT_SYSTEM: &str = include_str!("../prompts/chat_system.v1.md");
pub const CHAT_VERSION: &str = "chat.v1";
pub const MAX_TOOL_ROUNDS: usize = 8;

pub struct TurnInput {
    /// Earlier conversation, replayed verbatim.
    pub history: Vec<Message>,
    pub text: String,
    pub fen: String,
    pub ply: Option<u32>,
    pub move_path: Vec<String>,
    pub elo: u32,
    /// Start position of `move_path` (the game's start).
    pub start_fen: Option<String>,
}

pub struct TurnResult {
    /// The user message plus everything the assistant produced this turn.
    pub new_messages: Vec<Message>,
    pub text: String,
    pub tool_calls: Vec<ToolCallView>,
    pub verification: Verification,
    pub usage: Usage,
    pub model: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ChatError {
    #[error(transparent)]
    Llm(#[from] llm::LlmError),
    #[error("invalid board position: {0}")]
    BadFen(String),
}

pub fn system_prompt(env: &ToolEnv, elo: u32) -> String {
    let explorer_line = if env.explorer.is_some() {
        "- For opening questions, call opening_lookup to see what strong players actually play.\n"
    } else {
        ""
    };
    let game_line = match &env.game {
        Some(g) => format!(
            "\nThe student is reviewing one of their games: {} Call game_context when the question is about the game.",
            g.header
        ),
        None => String::new(),
    };
    CHAT_SYSTEM
        .replace("{{elo}}", &elo.to_string())
        .replace("{{tier_label}}", env.tier.label())
        .replace("{{explorer_line}}", explorer_line)
        .replace("{{level_guidance}}", level_guidance(env.tier))
        .replace("{{game_line}}", &game_line)
}

/// The board context prepended to the user's question.
pub fn board_note(input: &TurnInput) -> Result<String, ChatError> {
    let pos = parse_fen(&input.fen).map_err(|e| ChatError::BadFen(e.to_string()))?;
    let side = if pos.turn() == Color::White { "White" } else { "Black" };
    let mut note = format!("[Board] FEN {} ({side} to move).", input.fen);
    if !input.move_path.is_empty() {
        let start = parse_fen(input.start_fen.as_deref().unwrap_or(START_FEN)).map_err(|e| ChatError::BadFen(e.to_string()))?;
        note.push_str(&format!(" Moves so far: {}.", numbered_line(&start, &input.move_path)));
    }
    if let Some(p) = input.ply {
        note.push_str(&format!(" This is ply {p} of the game."));
    }
    Ok(note)
}

pub async fn run_turn(
    provider: &dyn Provider,
    env: &ToolEnv,
    input: TurnInput,
    events: &mpsc::UnboundedSender<ChatEvent>,
) -> Result<TurnResult, ChatError> {
    let board = parse_fen(&input.fen).map_err(|e| ChatError::BadFen(e.to_string()))?;
    let note = board_note(&input)?;
    let user = Message {
        role: Role::User,
        parts: vec![Part::Text { text: format!("{note}\n\n{}", input.text.trim()) }],
    };
    let system = system_prompt(env, input.elo);
    let defs = tool_defs(env);

    let mut grounding = Grounding::new();
    grounding.add_position(&board);
    grounding.add_history(&input.move_path);
    if let Some(g) = &env.game {
        grounding.add_history(&g.moves_san);
    }

    let mut new_messages = vec![user];
    let mut tool_calls = vec![];
    let mut usage = Usage::default();
    let mut text = String::new();
    let mut model = provider.model().to_string();

    // Forward provider stream events to the chat event stream.
    let (stx, mut srx) = mpsc::unbounded_channel::<StreamEvent>();
    let fwd = events.clone();
    let forwarder = tokio::spawn(async move {
        while let Some(e) = srx.recv().await {
            let ev = match e {
                StreamEvent::TextDelta(t) => ChatEvent::TextDelta { text: t },
                StreamEvent::ThinkingDelta(t) => ChatEvent::Thinking { text: t },
                StreamEvent::ToolCallStart { .. } => continue,
            };
            if fwd.send(ev).is_err() {
                break;
            }
        }
    });

    for round in 0..MAX_TOOL_ROUNDS {
        let mut messages = input.history.clone();
        messages.extend(new_messages.iter().cloned());
        let mut req = ChatRequest::new(system.clone(), messages);
        req.tools = defs.clone();
        let c = send_with_retry(provider, &req, Some(&stx)).await?;
        model = c.model.clone();
        usage.input_tokens += c.usage.input_tokens;
        usage.output_tokens += c.usage.output_tokens;
        usage.cache_read_tokens += c.usage.cache_read_tokens;
        usage.cache_write_tokens += c.usage.cache_write_tokens;
        let turn_text = c.message.text();
        if !turn_text.is_empty() {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(&turn_text);
        }
        let calls: Vec<(String, String, serde_json::Value)> = c
            .message
            .tool_calls()
            .into_iter()
            .map(|(i, n, v)| (i.to_string(), n.to_string(), v.clone()))
            .collect();
        new_messages.push(c.message);
        match c.stop {
            StopReason::Refusal(cat) => {
                let msg = format!(
                    "The model declined to answer this{}.",
                    cat.map(|c| format!(" (category: {c})")).unwrap_or_default()
                );
                text.push_str(&format!("\n\n{msg}"));
                let _ = events.send(ChatEvent::TextDelta { text: format!("\n\n{msg}") });
                break;
            }
            // A truncated turn may carry a cut-off tool call: never run it.
            StopReason::MaxTokens => break,
            _ => {}
        }
        if calls.is_empty() {
            break;
        }
        let mut results = vec![];
        for (id, name, input_v) in calls {
            let _ = events.send(ChatEvent::ToolCall { id: id.clone(), name: name.clone(), input: input_v.clone() });
            let out = run_tool(env, &mut grounding, &id, &name, &input_v).await;
            let _ = events.send(ChatEvent::ToolResult { call: out.view.clone() });
            tool_calls.push(out.view.clone());
            results.push(Part::ToolResult { call_id: id, content: out.content, is_error: out.view.is_error });
        }
        if round + 2 == MAX_TOOL_ROUNDS {
            results.push(Part::Text {
                text: "Tool budget nearly used up: answer now from what you have, without further tool calls.".into(),
            });
        }
        new_messages.push(Message { role: Role::User, parts: results });
    }
    drop(stx);
    let _ = forwarder.await;

    let mut f = Findings::default();
    f.scan_text(&grounding, &text);
    let verification = f.to_verification(false);
    let _ = events.send(ChatEvent::Verification { verification: verification.clone() });
    Ok(TurnResult { new_messages, text, tool_calls, verification, usage, model })
}
