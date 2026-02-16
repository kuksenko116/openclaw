// OpenAI Chat Completions API provider.
//
// Also works for OpenAI-compatible APIs (OpenRouter, Together, Gemini, etc.).
// Streams responses via SSE from POST /v1/chat/completions with stream: true.
// Implements the LlmProvider trait from agent/mod.rs.

use anyhow::Result;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::pin::Pin;
use tokio_stream::Stream;

use crate::agent::types::{
    AgentEvent, ChatRequest, ContentBlock, Message, Role, StopReason, ToolDefinition,
};
use crate::agent::LlmProvider;
use super::streaming::parse_sse_stream;

pub(crate) struct OpenAiProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl OpenAiProvider {
    pub fn new(api_key: &str, base_url: &str) -> Result<Self> {
        let base_url = if base_url.is_empty() {
            "https://api.openai.com/v1".to_string()
        } else {
            base_url.trim_end_matches('/').to_string()
        };

        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()?,
            api_key: api_key.to_string(),
            base_url,
        })
    }
}

/// Build the request body for the OpenAI Chat Completions API.
fn build_request_body(request: &ChatRequest) -> Value {
    let mut messages = Vec::new();

    if !request.system_prompt.is_empty() {
        messages.push(json!({
            "role": "system",
            "content": request.system_prompt,
        }));
    }

    for msg in &request.messages {
        messages.push(convert_message(msg));
    }

    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "max_tokens": request.max_tokens,
        "stream": true,
        "stream_options": { "include_usage": true },
    });

    if let Some(temp) = request.temperature {
        body["temperature"] = json!(temp);
    }

    if !request.tools.is_empty() {
        body["tools"] = json!(convert_tools(&request.tools));
    }

    body
}

/// Convert a single message to OpenAI format.
fn convert_message(msg: &Message) -> Value {
    match msg.role {
        Role::User => {
            // Check for tool results (Anthropic sends these as user messages).
            let tool_results: Vec<&ContentBlock> = msg
                .content
                .iter()
                .filter(|b| matches!(b, ContentBlock::ToolResult { .. }))
                .collect();

            if !tool_results.is_empty() {
                if let Some(ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                }) = tool_results.first()
                {
                    return json!({
                        "role": "tool",
                        "tool_call_id": tool_use_id,
                        "content": content,
                    });
                }
            }

            json!({
                "role": "user",
                "content": extract_text(&msg.content),
            })
        }
        Role::Assistant => {
            let tool_calls = extract_tool_calls(&msg.content);
            let text = extract_text(&msg.content);

            let mut result = json!({ "role": "assistant" });

            if !text.is_empty() {
                result["content"] = json!(text);
            }
            if !tool_calls.is_empty() {
                result["tool_calls"] = json!(tool_calls);
            }

            result
        }
    }
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

/// Extract tool calls from content blocks in OpenAI format.
fn extract_tool_calls(blocks: &[ContentBlock]) -> Vec<Value> {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, name, input } => Some(json!({
                "id": id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": input.to_string(),
                }
            })),
            _ => None,
        })
        .collect()
}

/// Convert tool definitions to OpenAI format.
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

/// Parse finish_reason string from OpenAI API.
fn parse_finish_reason(reason: &str) -> StopReason {
    match reason {
        "stop" => StopReason::EndTurn,
        "tool_calls" => StopReason::ToolUse,
        "length" => StopReason::MaxTokens,
        _ => StopReason::EndTurn,
    }
}

impl LlmProvider for OpenAiProvider {
    fn stream_chat(
        &self,
        request: ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<AgentEvent>> + Send>>> {
        let body = build_request_body(&request);
        let url = format!("{}/chat/completions", self.base_url);
        let api_key = self.api_key.clone();
        let client = self.client.clone();

        let stream = futures_util::stream::once(async move {
            let mut req = client
                .post(&url)
                .header("content-type", "application/json");

            if !api_key.is_empty() {
                req = req.header("authorization", format!("Bearer {}", api_key));
            }

            let response = req
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("OpenAI request failed: {}", e))?;

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let body = response.text().await.unwrap_or_default();
                return Err(anyhow::anyhow!("OpenAI API error ({}): {}", status, body));
            }

            Ok(response)
        })
        .flat_map(move |response_result| {
            match response_result {
                Err(e) => Box::pin(futures_util::stream::once(async move { Err(e) }))
                    as Pin<Box<dyn Stream<Item = Result<AgentEvent>> + Send>>,
                Ok(response) => {
                    let byte_stream = response.bytes_stream();
                    let sse_stream = parse_sse_stream(byte_stream);
                    Box::pin(OpenAiEventStream::new(sse_stream))
                }
            }
        });

        Ok(Box::pin(stream))
    }
}

/// Adapter that converts SSE events into AgentEvents for OpenAI.
struct OpenAiEventStream {
    inner: Pin<Box<dyn Stream<Item = Result<super::streaming::SseEvent>> + Send>>,
    tool_calls: HashMap<u32, (String, String, String)>, // index -> (id, name, args_json)
    pending: Vec<Result<AgentEvent>>,
    done: bool,
}

impl OpenAiEventStream {
    fn new(
        inner: Pin<Box<dyn Stream<Item = Result<super::streaming::SseEvent>> + Send>>,
    ) -> Self {
        Self {
            inner,
            tool_calls: HashMap::new(),
            pending: Vec::new(),
            done: false,
        }
    }

    fn process_sse_event(&mut self, sse_event: super::streaming::SseEvent) {
        // OpenAI terminates with "data: [DONE]".
        if sse_event.data.trim() == "[DONE]" {
            self.done = true;
            return;
        }

        let chunk: Value = match serde_json::from_str(&sse_event.data) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("Failed to parse OpenAI chunk: {}", e);
                return;
            }
        };

        // Process choices[0].delta.
        if let Some(delta) = chunk["choices"][0]["delta"].as_object() {
            // Text content.
            if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
                self.pending
                    .push(Ok(AgentEvent::TextDelta(text.to_string())));
            }

            // Tool calls (streamed incrementally by index).
            if let Some(tcs) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                for tc in tcs {
                    let index = tc["index"].as_u64().unwrap_or(0) as u32;
                    let entry = self
                        .tool_calls
                        .entry(index)
                        .or_insert_with(|| (String::new(), String::new(), String::new()));

                    if let Some(id) = tc["id"].as_str() {
                        entry.0 = id.to_string();
                    }
                    if let Some(name) = tc["function"]["name"].as_str() {
                        entry.1 = name.to_string();
                    }
                    if let Some(args) = tc["function"]["arguments"].as_str() {
                        entry.2.push_str(args);
                    }
                }
            }
        }

        // Usage (OpenAI sends it in the final chunk when stream_options.include_usage is set,
        // or in the chunk with finish_reason).
        if let Some(u) = chunk.get("usage").and_then(|v| v.as_object()) {
            let input = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            let output = u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            if input > 0 || output > 0 {
                self.pending.push(Ok(AgentEvent::UsageUpdate {
                    input_tokens: input,
                    output_tokens: output,
                }));
            }
        }

        // Check finish_reason.
        if let Some(reason) = chunk["choices"][0]["finish_reason"].as_str() {
            let stop_reason = parse_finish_reason(reason);

            // Emit accumulated tool calls before MessageEnd.
            if stop_reason == StopReason::ToolUse {
                let mut indices: Vec<u32> = self.tool_calls.keys().copied().collect();
                indices.sort();
                for idx in indices {
                    if let Some((id, name, args_json)) = self.tool_calls.remove(&idx) {
                        let input: Value = match serde_json::from_str(&args_json) {
                            Ok(v) => v,
                            Err(e) => {
                                tracing::warn!(
                                    tool = %name,
                                    error = %e,
                                    "Failed to parse tool call arguments, sending raw string"
                                );
                                // Pass the raw string so the error is visible
                                serde_json::json!({
                                    "_parse_error": e.to_string(),
                                    "_raw_args": args_json,
                                })
                            }
                        };
                        self.pending
                            .push(Ok(AgentEvent::ToolUse { id, name, input }));
                    }
                }
            }

            self.pending
                .push(Ok(AgentEvent::MessageEnd { stop_reason }));
        }
    }
}

impl Stream for OpenAiEventStream {
    type Item = Result<AgentEvent>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use std::task::Poll;

        if self.done && self.pending.is_empty() {
            return Poll::Ready(None);
        }

        // Return buffered events first.
        if !self.pending.is_empty() {
            let event = self.pending.remove(0);
            return Poll::Ready(Some(event));
        }

        // Poll the inner SSE stream.
        loop {
            match self.inner.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(sse_event))) => {
                    self.process_sse_event(sse_event);
                    if self.done || !self.pending.is_empty() {
                        if self.pending.is_empty() {
                            return Poll::Ready(None);
                        }
                        return Poll::Ready(Some(self.pending.remove(0)));
                    }
                }
                Poll::Ready(Some(Err(e))) => {
                    self.done = true;
                    return Poll::Ready(Some(Err(e)));
                }
                Poll::Ready(None) => {
                    self.done = true;
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
