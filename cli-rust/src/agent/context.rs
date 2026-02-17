//! Context window management: token estimation and automatic compaction.
//!
//! Provides heuristic token counting (4 chars/token) and a mechanism to
//! summarize older messages via the LLM when the conversation approaches the
//! model's context limit.

use anyhow::Result;
use tokio_stream::StreamExt;

use super::types::{AgentEvent, ChatRequest, ContentBlock, Message, Role};
use super::LlmProvider;

// ---------------------------------------------------------------------------
// Token estimation (heuristic: ~4 chars per token)
// ---------------------------------------------------------------------------

const CHARS_PER_TOKEN: usize = 4;

/// Estimate the number of tokens in a text string.
///
/// Uses the simple heuristic of ~4 characters per token. This is good enough
/// for context window management decisions but not for billing.
pub(crate) fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(CHARS_PER_TOKEN)
}

/// Estimate the token count for a single [`Message`].
pub(crate) fn estimate_message_tokens(msg: &Message) -> usize {
    // Each message carries a small overhead for role framing.
    let overhead = 4;
    let content_tokens: usize = msg
        .content
        .iter()
        .map(|block| match block {
            ContentBlock::Text { text } => estimate_tokens(text),
            ContentBlock::ToolUse { id, name, input } => {
                estimate_tokens(id) + estimate_tokens(name) + estimate_tokens(&input.to_string())
            }
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } => estimate_tokens(tool_use_id) + estimate_tokens(content),
            ContentBlock::Image { source } => {
                // Rough estimate: ~1 token per 750 base64 chars + fixed overhead.
                source.data.len() / 750 + 100
            }
        })
        .sum();
    overhead + content_tokens
}

/// Estimate the total token count for a slice of messages.
pub(crate) fn estimate_messages_tokens(msgs: &[Message]) -> usize {
    msgs.iter().map(estimate_message_tokens).sum()
}

// ---------------------------------------------------------------------------
// Model context limits (delegates to llm::models registry)
// ---------------------------------------------------------------------------

/// Return the context window limit (in tokens) for a given model name.
///
/// Delegates to the model registry; falls back to 128 000 for unknown models.
pub(crate) fn context_limit_for_model(model: &str) -> usize {
    crate::llm::models::context_limit(model)
}

// ---------------------------------------------------------------------------
// Compact (summarize) messages to fit context window
// ---------------------------------------------------------------------------

/// The number of recent messages to preserve verbatim during compaction.
const KEEP_RECENT_MESSAGES: usize = 6;

/// Compact messages by summarizing the middle portion via the LLM.
///
/// Returns a shorter message list with the layout:
///   `[first_user_msg, summary_as_user_msg, ...recent_messages]`
///
/// If the LLM summarization call fails, falls back to keeping only the first
/// user message and the most recent turns.
pub(crate) async fn compact_messages(
    provider: &dyn LlmProvider,
    messages: &[Message],
    model: &str,
    system_prompt: &str,
) -> Result<Vec<Message>> {
    // Nothing to compact if there aren't enough messages.
    if messages.len() <= KEEP_RECENT_MESSAGES + 1 {
        return Ok(messages.to_vec());
    }

    let first_msg = &messages[0];
    let split_point = messages.len().saturating_sub(KEEP_RECENT_MESSAGES);
    let middle = &messages[1..split_point];
    let recent = &messages[split_point..];

    if middle.is_empty() {
        return Ok(messages.to_vec());
    }

    // Build a text representation of the middle messages for summarization.
    let middle_text = render_messages_for_summary(middle);

    // Try to get a summary from the LLM; fall back to truncation on failure.
    match summarize_via_llm(provider, &middle_text, model, system_prompt).await {
        Ok(summary) => {
            let summary_msg = Message {
                role: Role::User,
                content: vec![ContentBlock::Text {
                    text: format!(
                        "[Conversation summary -- earlier messages were compacted to save context]\n\n{}",
                        summary,
                    ),
                }],
            };
            let mut result = vec![first_msg.clone(), summary_msg];
            result.extend_from_slice(recent);
            Ok(result)
        }
        Err(e) => {
            eprintln!(
                "\x1b[33mWarning: compaction summary failed ({}), keeping recent messages only\x1b[0m",
                e,
            );
            let mut result = vec![first_msg.clone()];
            result.extend_from_slice(recent);
            Ok(result)
        }
    }
}

/// Render a slice of messages into a human-readable text block suitable for
/// feeding into a summarization prompt.
fn render_messages_for_summary(messages: &[Message]) -> String {
    let mut buf = String::new();
    for msg in messages {
        let role_label = match msg.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
        };
        for block in &msg.content {
            match block {
                ContentBlock::Text { text } => {
                    buf.push_str(&format!("{}: {}\n", role_label, text));
                }
                ContentBlock::ToolUse { name, input, .. } => {
                    buf.push_str(&format!(
                        "{}: [called tool '{}' with {}]\n",
                        role_label, name, input,
                    ));
                }
                ContentBlock::ToolResult { content, .. } => {
                    // Truncate very long tool results to keep the summary prompt sane.
                    let preview: String = content.chars().take(500).collect();
                    buf.push_str(&format!("{}: [tool result: {}]\n", role_label, preview));
                }
                ContentBlock::Image { .. } => {
                    buf.push_str(&format!("{}: [image]\n", role_label));
                }
            }
        }
    }
    buf
}

/// Call the LLM to produce a concise summary of the conversation excerpt.
async fn summarize_via_llm(
    provider: &dyn LlmProvider,
    conversation_text: &str,
    model: &str,
    _system_prompt: &str,
) -> Result<String> {
    let user_prompt = format!(
        "Summarize the following conversation excerpt concisely. \
         Preserve key facts, decisions, file paths, code snippets, and tool results \
         that would be needed to continue the conversation. \
         Be brief but complete.\n\n---\n{}\n---",
        conversation_text,
    );

    let request = ChatRequest {
        messages: vec![Message::user(&user_prompt)],
        system_prompt: "You are a conversation summarizer. Produce a concise summary.".to_string(),
        tools: Vec::new(),
        model: model.to_string(),
        max_tokens: 1024,
        temperature: Some(0.0),
        thinking_budget: None,
    };

    let mut stream = provider.stream_chat(request)?;
    let mut summary = String::new();

    while let Some(event) = stream.next().await {
        match event {
            Ok(AgentEvent::TextDelta(delta)) => summary.push_str(&delta),
            Ok(AgentEvent::MessageEnd { .. }) => break,
            Ok(_) => {} // ignore usage updates, thinking, etc.
            Err(e) => return Err(e),
        }
    }

    if summary.is_empty() {
        anyhow::bail!("LLM returned empty summary");
    }

    Ok(summary)
}

/// Result of a compaction operation (used for reporting).
#[derive(Debug)]
pub(crate) struct CompactResult {
    pub messages_before: usize,
    pub messages_after: usize,
    pub tokens_before: usize,
    pub tokens_after: usize,
}

/// Check whether the conversation needs compaction and perform it if so.
///
/// Returns `Some((compacted_messages, stats))` when compaction happened,
/// `None` otherwise.
pub(crate) async fn maybe_compact(
    provider: &dyn LlmProvider,
    messages: &[Message],
    model: &str,
    system_prompt: &str,
    tool_definitions_token_estimate: usize,
) -> Result<Option<(Vec<Message>, CompactResult)>> {
    let limit = context_limit_for_model(model);
    let threshold = limit * 80 / 100;

    let system_tokens = estimate_tokens(system_prompt);
    let msg_tokens = estimate_messages_tokens(messages);
    let total = system_tokens + msg_tokens + tool_definitions_token_estimate;

    if total <= threshold {
        return Ok(None);
    }

    eprintln!(
        "\x1b[33mWarning: estimated {} tokens (~{}% of {} context limit for {}). \
         Compacting conversation...\x1b[0m",
        total,
        total * 100 / limit,
        limit,
        model,
    );

    let before_count = messages.len();
    let before_tokens = msg_tokens;

    let compacted = compact_messages(provider, messages, model, system_prompt).await?;

    let after_count = compacted.len();
    let after_tokens = estimate_messages_tokens(&compacted);

    let result = CompactResult {
        messages_before: before_count,
        messages_after: after_count,
        tokens_before: before_tokens,
        tokens_after: after_tokens,
    };

    eprintln!(
        "\x1b[33mCompacted: {} -> {} messages ({} -> ~{} tokens)\x1b[0m",
        result.messages_before, result.messages_after, result.tokens_before, result.tokens_after,
    );

    Ok(Some((compacted, result)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_estimate_tokens_empty() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn test_estimate_tokens_short() {
        // "hello" = 5 chars => ceil(5/4) = 2
        assert_eq!(estimate_tokens("hello"), 2);
    }

    #[test]
    fn test_estimate_tokens_exact_multiple() {
        // 8 chars => 8/4 = 2
        assert_eq!(estimate_tokens("12345678"), 2);
    }

    #[test]
    fn test_estimate_tokens_longer() {
        // 100 chars => 25 tokens
        let text = "a".repeat(100);
        assert_eq!(estimate_tokens(&text), 25);
    }

    #[test]
    fn test_estimate_message_tokens_text() {
        let msg = Message::user("Hello world");
        let tokens = estimate_message_tokens(&msg);
        // "Hello world" = 11 chars => ceil(11/4) = 3 tokens + 4 overhead = 7
        assert_eq!(tokens, 7);
    }

    #[test]
    fn test_estimate_message_tokens_tool_use() {
        let msg = Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "t1".to_string(),
                name: "bash".to_string(),
                input: json!({"command": "ls"}),
            }],
        };
        let tokens = estimate_message_tokens(&msg);
        assert!(tokens > 4); // overhead + some content
    }

    #[test]
    fn test_estimate_messages_tokens() {
        let msgs = vec![Message::user("Hello"), Message::user("World")];
        let total = estimate_messages_tokens(&msgs);
        // Each: ~2 content tokens + 4 overhead = 6; total = 12
        assert_eq!(total, 12);
    }

    #[test]
    fn test_context_limit_known_models() {
        assert_eq!(context_limit_for_model("claude-sonnet-4-20250514"), 200_000);
        assert_eq!(context_limit_for_model("claude-opus-4-20250514"), 200_000);
        assert_eq!(context_limit_for_model("claude-haiku-3-20250307"), 200_000);
        assert_eq!(context_limit_for_model("gpt-4o"), 128_000);
        assert_eq!(context_limit_for_model("gpt-4o-mini"), 128_000);
        assert_eq!(context_limit_for_model("gpt-4-turbo"), 128_000);
    }

    #[test]
    fn test_context_limit_unknown_model() {
        assert_eq!(context_limit_for_model("some-unknown-model"), 128_000);
    }

    #[test]
    fn test_render_messages_for_summary() {
        let msgs = vec![
            Message::user("Hello"),
            Message {
                role: Role::Assistant,
                content: vec![ContentBlock::Text {
                    text: "Hi there!".to_string(),
                }],
            },
        ];
        let rendered = render_messages_for_summary(&msgs);
        assert!(rendered.contains("User: Hello"));
        assert!(rendered.contains("Assistant: Hi there!"));
    }

    #[test]
    fn test_render_messages_with_tool_use() {
        let msgs = vec![Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "t1".to_string(),
                name: "bash".to_string(),
                input: json!({"command": "ls"}),
            }],
        }];
        let rendered = render_messages_for_summary(&msgs);
        assert!(rendered.contains("called tool 'bash'"));
    }

    #[test]
    fn test_render_messages_with_image() {
        use super::super::types::ImageSource;
        let msgs = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Image {
                source: ImageSource {
                    source_type: "base64".to_string(),
                    media_type: "image/png".to_string(),
                    data: "abc".to_string(),
                },
            }],
        }];
        let rendered = render_messages_for_summary(&msgs);
        assert!(rendered.contains("[image]"));
    }
}
