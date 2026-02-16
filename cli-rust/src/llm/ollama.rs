// Ollama native API provider.
//
// IMPORTANT: Ollama uses NDJSON streaming, NOT SSE.
// Each response line is a complete JSON object.
//
// CRITICAL: Tool calls appear in intermediate (done:false) chunks,
// NOT in the final done:true chunk. Must accumulate tool_calls across ALL chunks.
//
// Reference: src/agents/ollama-stream.ts in the TypeScript codebase.
// Implements the LlmProvider trait from agent/mod.rs.

use anyhow::Result;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::pin::Pin;
use tokio_stream::Stream;

use crate::agent::types::{
    AgentEvent, ChatRequest, ContentBlock, Message, Role, StopReason, ToolDefinition,
};
use crate::agent::LlmProvider;

pub(crate) struct OllamaProvider {
    client: reqwest::Client,
    base_url: String,
}

impl OllamaProvider {
    pub fn new(base_url: &str) -> Result<Self> {
        let base_url = if base_url.is_empty() {
            "http://127.0.0.1:11434".to_string()
        } else {
            base_url.trim_end_matches('/').to_string()
        };

        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(600)) // Ollama can be slow.
                .build()?,
            base_url,
        })
    }
}

/// Resolve the /api/chat URL from the configured base URL.
/// Users configure baseUrl with /v1 suffix for OpenAI compat; strip it for native API.
fn resolve_chat_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let base = base.strip_suffix("/v1").unwrap_or(base);
    format!("{}/api/chat", base)
}

/// A single chunk from the Ollama NDJSON stream.
#[derive(Debug, Deserialize)]
#[serde(default)]
struct OllamaChatChunk {
    message: OllamaMessage,
    done: bool,
    #[serde(default)]
    prompt_eval_count: u64,
    #[serde(default)]
    eval_count: u64,
}

impl Default for OllamaChatChunk {
    fn default() -> Self {
        Self {
            message: OllamaMessage::default(),
            done: false,
            prompt_eval_count: 0,
            eval_count: 0,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct OllamaMessage {
    content: String,
    tool_calls: Option<Vec<OllamaToolCall>>,
}

#[derive(Debug, Clone, Deserialize)]
struct OllamaToolCall {
    function: OllamaToolCallFunction,
}

#[derive(Debug, Clone, Deserialize)]
struct OllamaToolCallFunction {
    name: String,
    arguments: Value,
}

/// Build the request body for the Ollama /api/chat endpoint.
fn build_request_body(request: &ChatRequest) -> Value {
    let messages = convert_messages(&request.messages, &request.system_prompt);
    let tools = convert_tools(&request.tools);

    // Ollama defaults to num_ctx=4096, too small for system prompts + tools.
    let mut options = json!({ "num_ctx": 65536 });
    if let Some(temp) = request.temperature {
        options["temperature"] = json!(temp);
    }
    options["num_predict"] = json!(request.max_tokens);

    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "stream": true,
        "options": options,
    });

    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }

    body
}

/// Convert messages to Ollama format.
fn convert_messages(messages: &[Message], system_prompt: &str) -> Vec<Value> {
    let mut result = Vec::new();

    if !system_prompt.is_empty() {
        result.push(json!({
            "role": "system",
            "content": system_prompt,
        }));
    }

    for msg in messages {
        match msg.role {
            Role::User => {
                // Check for tool results.
                let tool_results: Vec<_> = msg
                    .content
                    .iter()
                    .filter(|b| matches!(b, ContentBlock::ToolResult { .. }))
                    .collect();

                if !tool_results.is_empty() {
                    for block in &tool_results {
                        if let ContentBlock::ToolResult { content, .. } = block {
                            result.push(json!({
                                "role": "tool",
                                "content": content,
                            }));
                        }
                    }
                } else {
                    result.push(json!({
                        "role": "user",
                        "content": extract_text(&msg.content),
                    }));
                }
            }
            Role::Assistant => {
                let text = extract_text(&msg.content);
                let tool_calls = extract_tool_calls(&msg.content);

                let mut m = json!({
                    "role": "assistant",
                    "content": text,
                });
                if !tool_calls.is_empty() {
                    m["tool_calls"] = json!(tool_calls);
                }
                result.push(m);
            }
        }
    }

    result
}

/// Extract concatenated text from content blocks.
fn extract_text(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Extract tool calls from content blocks in Ollama format.
fn extract_tool_calls(blocks: &[ContentBlock]) -> Vec<Value> {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolUse { name, input, .. } => Some(json!({
                "function": {
                    "name": name,
                    "arguments": input,
                }
            })),
            _ => None,
        })
        .collect()
}

/// Convert tool definitions to Ollama format.
fn convert_tools(tools: &[ToolDefinition]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| json!({
            "type": "function",
            "function": {
                "name": t.name,
                "description": t.description,
                "parameters": t.input_schema,
            }
        }))
        .collect()
}

impl LlmProvider for OllamaProvider {
    fn stream_chat(
        &self,
        request: ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<AgentEvent>> + Send>>> {
        let url = resolve_chat_url(&self.base_url);
        let body = build_request_body(&request);
        let client = self.client.clone();

        // Use unfold to create a stateful stream that:
        // 1. Sends the HTTP request
        // 2. Reads NDJSON lines
        // 3. Emits TextDelta for each content chunk
        // 4. Accumulates tool calls
        // 5. On done: emits all tool calls then MessageEnd
        let stream = futures_util::stream::once(async move {
            let response = client
                .post(&url)
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("Ollama request failed: {}", e))?;

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let body = response.text().await.unwrap_or_default();
                return Err(anyhow::anyhow!("Ollama API error ({}): {}", status, body));
            }

            Ok(response)
        })
        .flat_map(move |response_result| match response_result {
            Err(e) => Box::pin(futures_util::stream::once(async move { Err(e) }))
                as Pin<Box<dyn Stream<Item = Result<AgentEvent>> + Send>>,
            Ok(response) => Box::pin(OllamaNdjsonStream::new(response)),
        });

        Ok(Box::pin(stream))
    }
}

/// Stateful NDJSON stream that accumulates tool calls across chunks.
struct OllamaNdjsonStream {
    byte_stream: Pin<Box<dyn Stream<Item = reqwest::Result<bytes::Bytes>> + Send>>,
    ndjson_buffer: String,
    accumulated_tool_calls: Vec<OllamaToolCall>,
    pending: Vec<Result<AgentEvent>>,
    finished: bool,
}

impl OllamaNdjsonStream {
    fn new(response: reqwest::Response) -> Self {
        Self {
            byte_stream: Box::pin(response.bytes_stream()),
            ndjson_buffer: String::new(),
            accumulated_tool_calls: Vec::new(),
            pending: Vec::new(),
            finished: false,
        }
    }

    /// Parse complete lines from the buffer and process them.
    fn process_buffer(&mut self) {
        while let Some(newline_pos) = self.ndjson_buffer.find('\n') {
            let line = self.ndjson_buffer[..newline_pos].trim().to_string();
            self.ndjson_buffer = self.ndjson_buffer[newline_pos + 1..].to_string();

            if line.is_empty() {
                continue;
            }

            self.process_line(&line);

            if self.finished {
                break;
            }
        }
    }

    /// Process a single NDJSON line.
    fn process_line(&mut self, line: &str) {
        let chunk: OllamaChatChunk = match serde_json::from_str(line) {
            Ok(c) => c,
            Err(_) => {
                tracing::warn!(
                    "Skipping malformed NDJSON line: {}",
                    &line[..line.len().min(120)]
                );
                return;
            }
        };

        // Stream text deltas as they arrive.
        if !chunk.message.content.is_empty() {
            self.pending
                .push(Ok(AgentEvent::TextDelta(chunk.message.content.clone())));
        }

        // CRITICAL: Collect tool calls from intermediate chunks.
        if let Some(ref tcs) = chunk.message.tool_calls {
            self.accumulated_tool_calls.extend(tcs.iter().cloned());
        }

        if chunk.done {
            self.emit_final_events(chunk.prompt_eval_count, chunk.eval_count);
            self.finished = true;
        }
    }

    /// Emit accumulated tool calls and the final MessageEnd event.
    fn emit_final_events(&mut self, input_tokens: u64, output_tokens: u64) {
        let has_tool_calls = !self.accumulated_tool_calls.is_empty();

        for tc in &self.accumulated_tool_calls {
            self.pending.push(Ok(AgentEvent::ToolUse {
                id: format!("ollama_call_{}", uuid::Uuid::new_v4()),
                name: tc.function.name.clone(),
                input: tc.function.arguments.clone(),
            }));
        }
        self.accumulated_tool_calls.clear();

        if input_tokens > 0 || output_tokens > 0 {
            self.pending.push(Ok(AgentEvent::UsageUpdate {
                input_tokens,
                output_tokens,
            }));
        }

        let stop_reason = if has_tool_calls {
            StopReason::ToolUse
        } else {
            StopReason::EndTurn
        };
        self.pending
            .push(Ok(AgentEvent::MessageEnd { stop_reason }));
    }
}

impl Stream for OllamaNdjsonStream {
    type Item = Result<AgentEvent>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use std::task::Poll;

        // Return buffered events first.
        if !self.pending.is_empty() {
            return Poll::Ready(Some(self.pending.remove(0)));
        }

        if self.finished {
            return Poll::Ready(None);
        }

        // Poll for more bytes.
        loop {
            match self.byte_stream.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    let text = String::from_utf8_lossy(&bytes);
                    self.ndjson_buffer.push_str(&text);
                    self.process_buffer();

                    if !self.pending.is_empty() {
                        return Poll::Ready(Some(self.pending.remove(0)));
                    }
                    if self.finished {
                        return Poll::Ready(None);
                    }
                }
                Poll::Ready(Some(Err(e))) => {
                    self.finished = true;
                    return Poll::Ready(Some(Err(anyhow::anyhow!(
                        "Ollama stream read error: {}",
                        e
                    ))));
                }
                Poll::Ready(None) => {
                    // Stream ended. Check for trailing data.
                    let remaining = self.ndjson_buffer.trim().to_string();
                    if !remaining.is_empty() {
                        self.ndjson_buffer.clear();
                        self.process_line(&remaining);
                    }

                    if !self.finished {
                        // Stream ended without a done:true chunk.
                        self.pending.push(Err(anyhow::anyhow!(
                            "Ollama stream ended without final response"
                        )));
                        self.finished = true;
                    }

                    if !self.pending.is_empty() {
                        return Poll::Ready(Some(self.pending.remove(0)));
                    }
                    return Poll::Ready(None);
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_chat_url_plain() {
        assert_eq!(
            resolve_chat_url("http://localhost:11434"),
            "http://localhost:11434/api/chat"
        );
    }

    #[test]
    fn test_resolve_chat_url_with_v1() {
        assert_eq!(
            resolve_chat_url("http://localhost:11434/v1"),
            "http://localhost:11434/api/chat"
        );
    }

    #[test]
    fn test_resolve_chat_url_trailing_slash() {
        assert_eq!(
            resolve_chat_url("http://localhost:11434/v1/"),
            "http://localhost:11434/api/chat"
        );
    }
}
