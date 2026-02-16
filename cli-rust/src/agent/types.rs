use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Role in a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Role {
    User,
    Assistant,
}

/// A block of content within a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
}

/// A message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    /// Create a user message from plain text.
    pub fn user(text: &str) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
        }
    }

    /// Create an assistant message from content blocks.
    pub fn assistant(content: Vec<ContentBlock>) -> Self {
        Self {
            role: Role::Assistant,
            content,
        }
    }

}

/// Events streamed from the LLM provider during a response.
#[derive(Debug, Clone)]
pub(crate) enum AgentEvent {
    /// Incremental text from the assistant.
    TextDelta(String),
    /// The assistant wants to call a tool.
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// The response is complete.
    MessageEnd { stop_reason: StopReason },
    /// Token usage update (partial — fields may be zero).
    UsageUpdate { input_tokens: u64, output_tokens: u64 },
}

/// Why the LLM stopped generating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
}

/// A request to send to the LLM.
#[derive(Debug, Clone)]
pub(crate) struct ChatRequest {
    pub messages: Vec<Message>,
    pub system_prompt: String,
    pub tools: Vec<ToolDefinition>,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
}

/// A tool definition presented to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// The result of executing a tool.
#[derive(Debug, Clone)]
pub(crate) struct ToolResult {
    pub content: String,
    pub is_error: bool,
}

/// Summary of an agent loop run.
#[derive(Debug, Clone)]
pub(crate) struct AgentResult {
    /// The full response text (streamed to stdout, also captured here for callers).
    #[allow(dead_code)]
    pub text: String,
    pub tool_calls: usize,
    pub usage: Usage,
}

/// Token usage statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}
