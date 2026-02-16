pub(crate) mod context;
pub(crate) mod images;
pub(crate) mod memory;
pub(crate) mod prompt;
pub(crate) mod session;
pub(crate) mod types;

use std::io::Write;
use std::pin::Pin;

use anyhow::Result;
use async_trait::async_trait;
use tokio_stream::{Stream, StreamExt};
use tokio::time::{sleep, Duration};

use crate::config::Config;

use self::session::Session;
use self::types::{
    AgentEvent, AgentResult, ChatRequest, ContentBlock, Message, Role, StopReason, ToolDefinition,
    ToolResult, Usage,
};

// ---------------------------------------------------------------------------
// Traits -- providers and tools implement these
// ---------------------------------------------------------------------------

/// Streams chat completions from an LLM.
pub(crate) trait LlmProvider: Send + Sync {
    /// Start a streaming chat completion. Returns a stream of events.
    fn stream_chat(
        &self,
        request: ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<AgentEvent>> + Send>>>;
}

/// Executes tools on behalf of the agent.
#[async_trait]
pub(crate) trait ToolExecutor: Send + Sync {
    /// Execute a tool by name with the given arguments.
    async fn execute(&self, name: &str, args: serde_json::Value) -> Result<ToolResult>;

    /// Return the tool definitions to present to the LLM.
    fn definitions(&self) -> Vec<ToolDefinition>;
}

// ---------------------------------------------------------------------------
// Agent loop
// ---------------------------------------------------------------------------

const MAX_AGENT_TURNS: usize = 20;
const MAX_RETRIES: usize = 2;

/// Check if an error is retryable (rate limit or server overload).
fn is_retryable_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string();
    msg.contains("rate limit")
        || msg.contains("429")
        || msg.contains("overloaded")
        || msg.contains("529")
        || msg.contains("503")
}

/// Run the agent loop: send messages to LLM, execute tools, repeat.
///
/// Streams text deltas to stdout as they arrive. Loops until the LLM
/// signals `EndTurn` or `MaxTokens` (i.e., no more tool calls), or
/// until `MAX_AGENT_TURNS` iterations are reached.
pub(crate) async fn run_agent_loop(
    provider: &dyn LlmProvider,
    session: &mut Session,
    tools: &dyn ToolExecutor,
    config: &Config,
) -> Result<AgentResult> {
    let mut total_tool_calls: usize = 0;
    let mut usage = Usage::default();

    for _turn in 0..MAX_AGENT_TURNS {
        // Auto-compact when approaching context window limit.
        let system_prompt = prompt::build_system_prompt(config, tools);
        let tool_defs_tokens = prompt::estimate_tool_definitions_tokens(tools);
        if let Some((compacted, _stats)) = context::maybe_compact(
            provider,
            session.messages(),
            &config.model,
            &system_prompt,
            tool_defs_tokens,
        )
        .await?
        {
            session.replace_messages(compacted);
        }

        let request = build_chat_request(session, tools, config);

        if config.verbose {
            eprintln!(
                "\x1b[2m[verbose] Turn {} | model={} | max_tokens={} | messages={} | tools={} | thinking={:?}\x1b[0m",
                _turn + 1,
                request.model,
                request.max_tokens,
                request.messages.len(),
                request.tools.len(),
                request.thinking_budget,
            );
        }

        let mut stream = {
            let mut last_err = None;
            let mut acquired = false;
            for attempt in 0..=MAX_RETRIES {
                match provider.stream_chat(request.clone()) {
                    Ok(s) => {
                        acquired = true;
                        last_err = Some(Ok(s));
                        break;
                    }
                    Err(e) => {
                        if attempt < MAX_RETRIES && is_retryable_error(&e) {
                            let delay = Duration::from_secs(1 << attempt);
                            eprintln!(
                                "\x1b[33mRetryable error (attempt {}/{}): {}. Retrying in {}s…\x1b[0m",
                                attempt + 1, MAX_RETRIES, e, delay.as_secs()
                            );
                            sleep(delay).await;
                        } else {
                            last_err = Some(Err(e));
                            break;
                        }
                    }
                }
            }
            match last_err {
                Some(Ok(s)) => s,
                Some(Err(e)) => return Err(e),
                None if !acquired => return Err(anyhow::anyhow!("stream_chat failed after retries")),
                None => unreachable!(),
            }
        };

        let mut text_buf = String::new();
        let mut tool_calls: Vec<PendingToolCall> = Vec::new();
        let mut stop_reason = StopReason::EndTurn;

        let stream_timeout = std::time::Duration::from_secs(300);
        loop {
            let event = match tokio::time::timeout(stream_timeout, stream.next()).await {
                Ok(Some(event)) => event?,
                Ok(None) => break,          // stream ended
                Err(_) => {
                    eprintln!("\x1b[33mWarning: stream timed out after {}s\x1b[0m", stream_timeout.as_secs());
                    break;
                }
            };
            match event {
                AgentEvent::TextDelta(delta) => {
                    print!("{}", delta);
                    std::io::stdout().flush().ok();
                    text_buf.push_str(&delta);
                }
                AgentEvent::Thinking(text) => {
                    // Print thinking text to stderr in dim/italic, don't add to text_buf
                    eprint!("\x1b[2;3m{}\x1b[0m", text);
                    std::io::stderr().flush().ok();
                }
                AgentEvent::ToolUse { id, name, input } => {
                    // Print tool call header
                    eprintln!(
                        "\n\x1b[36m⚙ Tool: {}\x1b[0m \x1b[2m({})\x1b[0m",
                        name, id
                    );
                    tool_calls.push(PendingToolCall { id, name, input });
                }
                AgentEvent::MessageEnd {
                    stop_reason: reason,
                } => {
                    stop_reason = reason;
                }
                AgentEvent::UsageUpdate {
                    input_tokens,
                    output_tokens,
                    cache_creation_input_tokens,
                    cache_read_input_tokens,
                } => {
                    usage.input_tokens += input_tokens;
                    usage.output_tokens += output_tokens;
                    usage.cache_creation_input_tokens += cache_creation_input_tokens;
                    usage.cache_read_input_tokens += cache_read_input_tokens;
                }
            }
        }

        // Newline after streamed text
        if !text_buf.is_empty() {
            println!();
        }

        // Build the assistant message content blocks
        let mut content: Vec<ContentBlock> = Vec::new();
        if !text_buf.is_empty() {
            content.push(ContentBlock::Text {
                text: text_buf.clone(),
            });
        }
        for tc in &tool_calls {
            content.push(ContentBlock::ToolUse {
                id: tc.id.clone(),
                name: tc.name.clone(),
                input: tc.input.clone(),
            });
        }
        session.add_assistant_message(content);

        // If no tool calls or model says end_turn / max_tokens, we're done
        if tool_calls.is_empty() || stop_reason != StopReason::ToolUse {
            return Ok(AgentResult {
                text: text_buf,
                tool_calls: total_tool_calls,
                usage,
            });
        }

        // Execute each tool call and feed results back
        for tc in &tool_calls {
            total_tool_calls += 1;
            tracing::info!(tool = %tc.name, id = %tc.id, "executing tool");

            eprintln!("\x1b[2m  Running {}…\x1b[0m", tc.name);

            let tool_start = std::time::Instant::now();
            let result = tools.execute(&tc.name, tc.input.clone()).await;
            let tool_elapsed = tool_start.elapsed();
            let tool_result = match result {
                Ok(r) => {
                    let preview = truncate_preview(&r.content, 200);
                    if config.verbose {
                        eprintln!(
                            "\x1b[32m  ✓\x1b[0m \x1b[2m({:.1}ms) {}\x1b[0m",
                            tool_elapsed.as_secs_f64() * 1000.0,
                            preview,
                        );
                    } else {
                        eprintln!("\x1b[32m  ✓\x1b[0m \x1b[2m{}\x1b[0m", preview);
                    }
                    r
                }
                Err(e) => {
                    eprintln!("\x1b[31m  ✗ Error: {e}\x1b[0m");
                    ToolResult {
                        content: format!("Error: {e}"),
                        is_error: true,
                    }
                }
            };

            // Add tool result as a message so the LLM can see it
            let result_content = vec![ContentBlock::ToolResult {
                tool_use_id: tc.id.clone(),
                content: tool_result.content,
                is_error: tool_result.is_error,
            }];
            session.push_message(Message {
                role: Role::User,
                content: result_content,
            });
        }

        // Loop back to send the updated messages to the LLM
    }

    // Exhausted all turns without the LLM signaling end_turn
    eprintln!(
        "\x1b[33mWarning: reached max agent turns ({}).\x1b[0m",
        MAX_AGENT_TURNS,
    );
    Ok(AgentResult {
        text: String::new(),
        tool_calls: total_tool_calls,
        usage,
    })
}

/// A tool call waiting to be executed.
struct PendingToolCall {
    id: String,
    name: String,
    input: serde_json::Value,
}

/// Build a ChatRequest from the current session state.
fn build_chat_request(
    session: &Session,
    tools: &dyn ToolExecutor,
    config: &Config,
) -> ChatRequest {
    let default_max = crate::llm::models::max_output_tokens(&config.model) as u32;
    ChatRequest {
        messages: session.messages().to_vec(),
        system_prompt: prompt::build_system_prompt(config, tools),
        tools: tools.definitions(),
        model: config.model.clone(),
        max_tokens: config.max_tokens.unwrap_or(default_max),
        temperature: config.temperature,
        thinking_budget: config.thinking_budget,
    }
}

/// Truncate a string to `max_len` chars for preview display.
fn truncate_preview(s: &str, max_len: usize) -> String {
    let single_line = s.replace('\n', " ");
    if single_line.chars().count() <= max_len {
        single_line
    } else {
        let truncated: String = single_line.chars().take(max_len).collect();
        format!("{truncated}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- Mock provider that returns canned events --

    struct MockProvider {
        /// Each inner Vec is one stream_chat response (one turn).
        responses: std::sync::Mutex<Vec<Vec<AgentEvent>>>,
    }

    impl MockProvider {
        fn new(responses: Vec<Vec<AgentEvent>>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses),
            }
        }
    }

    impl LlmProvider for MockProvider {
        fn stream_chat(
            &self,
            _request: ChatRequest,
        ) -> Result<Pin<Box<dyn Stream<Item = Result<AgentEvent>> + Send>>> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                anyhow::bail!("MockProvider: no more canned responses");
            }
            let events = responses.remove(0);
            let stream = tokio_stream::iter(events.into_iter().map(Ok));
            Ok(Box::pin(stream))
        }
    }

    // -- Mock tool executor --

    struct MockTools {
        /// Map of tool name -> (content, is_error)
        results: std::collections::HashMap<String, (String, bool)>,
    }

    impl MockTools {
        fn new() -> Self {
            Self {
                results: std::collections::HashMap::new(),
            }
        }
        fn add(mut self, name: &str, content: &str) -> Self {
            self.results
                .insert(name.to_string(), (content.to_string(), false));
            self
        }
    }

    #[async_trait]
    impl ToolExecutor for MockTools {
        async fn execute(&self, name: &str, _args: serde_json::Value) -> Result<ToolResult> {
            match self.results.get(name) {
                Some((content, is_error)) => Ok(ToolResult {
                    content: content.clone(),
                    is_error: *is_error,
                }),
                None => Ok(ToolResult {
                    content: format!("unknown tool: {name}"),
                    is_error: true,
                }),
            }
        }

        fn definitions(&self) -> Vec<ToolDefinition> {
            self.results
                .keys()
                .map(|name| ToolDefinition {
                    name: name.clone(),
                    description: format!("mock {name}"),
                    input_schema: json!({"type": "object"}),
                })
                .collect()
        }
    }

    fn test_config() -> Config {
        Config {
            model: "test-model".to_string(),
            provider: "test".to_string(),
            ..Config::default()
        }
    }

    #[tokio::test]
    async fn test_agent_loop_text_only() {
        let provider = MockProvider::new(vec![vec![
            AgentEvent::TextDelta("Hello ".to_string()),
            AgentEvent::TextDelta("world!".to_string()),
            AgentEvent::MessageEnd {
                stop_reason: StopReason::EndTurn,
            },
        ]]);
        let tools = MockTools::new();
        let config = test_config();
        let mut session = Session::new(std::path::PathBuf::from("/tmp/test-session.json"));
        session.add_user_message("hi");

        let result = run_agent_loop(&provider, &mut session, &tools, &config)
            .await
            .unwrap();

        assert_eq!(result.text, "Hello world!");
        assert_eq!(result.tool_calls, 0);
    }

    #[tokio::test]
    async fn test_agent_loop_with_tool_call() {
        let provider = MockProvider::new(vec![
            // Turn 1: LLM calls a tool
            vec![
                AgentEvent::ToolUse {
                    id: "t1".to_string(),
                    name: "bash".to_string(),
                    input: json!({"command": "echo hello"}),
                },
                AgentEvent::MessageEnd {
                    stop_reason: StopReason::ToolUse,
                },
            ],
            // Turn 2: LLM responds with text after seeing tool result
            vec![
                AgentEvent::TextDelta("The command output: hello".to_string()),
                AgentEvent::MessageEnd {
                    stop_reason: StopReason::EndTurn,
                },
            ],
        ]);
        let tools = MockTools::new().add("bash", "hello\n");
        let config = test_config();
        let mut session = Session::new(std::path::PathBuf::from("/tmp/test-session.json"));
        session.add_user_message("run echo hello");

        let result = run_agent_loop(&provider, &mut session, &tools, &config)
            .await
            .unwrap();

        assert_eq!(result.tool_calls, 1);
        assert_eq!(result.text, "The command output: hello");
        // Session should have: user, assistant (tool_use), user (tool_result), assistant (text)
        assert_eq!(session.messages().len(), 4);
    }

    #[tokio::test]
    async fn test_agent_loop_usage_tracking() {
        let provider = MockProvider::new(vec![vec![
            AgentEvent::UsageUpdate {
                input_tokens: 100,
                output_tokens: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
            AgentEvent::TextDelta("hi".to_string()),
            AgentEvent::UsageUpdate {
                input_tokens: 0,
                output_tokens: 25,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
            AgentEvent::MessageEnd {
                stop_reason: StopReason::EndTurn,
            },
        ]]);
        let tools = MockTools::new();
        let config = test_config();
        let mut session = Session::new(std::path::PathBuf::from("/tmp/test-session.json"));
        session.add_user_message("test");

        let result = run_agent_loop(&provider, &mut session, &tools, &config)
            .await
            .unwrap();

        assert_eq!(result.usage.input_tokens, 100);
        assert_eq!(result.usage.output_tokens, 25);
    }

    #[test]
    fn test_truncate_preview_short() {
        assert_eq!(truncate_preview("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_preview_long() {
        let result = truncate_preview("hello world this is long", 10);
        assert!(result.ends_with('…'));
        assert!(result.chars().count() <= 11); // 10 + ellipsis
    }

    #[test]
    fn test_truncate_preview_multiline() {
        assert_eq!(truncate_preview("line1\nline2", 20), "line1 line2");
    }
}
