// Anthropic Messages API provider.
//
// Streams responses via SSE from POST /v1/messages with stream: true.
// Implements the LlmProvider trait from agent/mod.rs.

use anyhow::Result;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::pin::Pin;
use tokio_stream::Stream;

use super::streaming::parse_sse_stream;
use crate::agent::types::{
    AgentEvent, ChatRequest, ContentBlock, ImageSource, Message, Role, StopReason, ToolDefinition,
};
use crate::agent::LlmProvider;

pub(crate) struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl AnthropicProvider {
    pub fn new(api_key: &str, base_url: &str) -> Result<Self> {
        let base_url = if base_url.is_empty() {
            "https://api.anthropic.com".to_string()
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

/// Build the request body for the Anthropic Messages API.
fn build_request_body(request: &ChatRequest) -> Value {
    let mut messages = convert_messages(&request.messages);
    let mut tools = convert_tools(&request.tools);

    let thinking_enabled = matches!(request.thinking_budget, Some(budget) if budget > 0);

    // Calculate max_tokens: when thinking is enabled, the budget counts toward max_tokens
    let max_tokens = if let Some(budget) = request.thinking_budget {
        if budget > 0 {
            budget + request.max_tokens
        } else {
            request.max_tokens
        }
    } else {
        request.max_tokens
    };

    // Add cache_control to the last tool definition (prompt caching breakpoint)
    if !tools.is_empty() {
        let last_idx = tools.len() - 1;
        tools[last_idx]["cache_control"] = json!({ "type": "ephemeral" });
    }

    // Add cache_control to the last user message (prompt caching breakpoint)
    if let Some(last_user_idx) = find_last_user_message_index(&messages) {
        if let Some(msg) = messages.get_mut(last_user_idx) {
            if let Some(content) = msg.get_mut("content") {
                if let Some(arr) = content.as_array_mut() {
                    if let Some(last_block) = arr.last_mut() {
                        last_block["cache_control"] = json!({ "type": "ephemeral" });
                    }
                }
            }
        }
    }

    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "max_tokens": max_tokens,
        "stream": true,
    });

    // Extended thinking support
    if thinking_enabled {
        body["thinking"] = json!({
            "type": "enabled",
            "budget_tokens": request.thinking_budget.unwrap()
        });
        // Anthropic doesn't allow temperature with thinking
    } else if let Some(temp) = request.temperature {
        body["temperature"] = json!(temp);
    }

    if !request.system_prompt.is_empty() {
        body["system"] = json!([{
            "type": "text",
            "text": request.system_prompt,
            "cache_control": { "type": "ephemeral" }
        }]);
    }

    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }

    body
}

/// Find the index of the last user message in the converted messages array.
fn find_last_user_message_index(messages: &[Value]) -> Option<usize> {
    messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, msg)| msg["role"].as_str() == Some("user"))
        .map(|(idx, _)| idx)
}

/// Convert internal messages to Anthropic format.
fn convert_messages(messages: &[Message]) -> Vec<Value> {
    let mut result = Vec::new();

    for msg in messages {
        match msg.role {
            Role::User => {
                let content = convert_user_content(&msg.content);
                result.push(json!({
                    "role": "user",
                    "content": content,
                }));
            }
            Role::Assistant => {
                let content = convert_assistant_content(&msg.content);
                result.push(json!({
                    "role": "assistant",
                    "content": content,
                }));
            }
        }
    }

    result
}

/// Convert user-side content blocks to Anthropic JSON.
fn convert_user_content(blocks: &[ContentBlock]) -> Vec<Value> {
    blocks
        .iter()
        .map(|block| match block {
            ContentBlock::Text { text } => json!({ "type": "text", "text": text }),
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                let mut v = json!({
                    "type": "tool_result",
                    "tool_use_id": tool_use_id,
                    "content": content,
                });
                if *is_error {
                    v["is_error"] = json!(true);
                }
                v
            }
            ContentBlock::ToolUse { id, name, input } => json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input,
            }),
            ContentBlock::Image { source } => convert_image_source(source),
        })
        .collect()
}

/// Convert assistant-side content blocks to Anthropic JSON.
fn convert_assistant_content(blocks: &[ContentBlock]) -> Vec<Value> {
    blocks
        .iter()
        .map(|block| match block {
            ContentBlock::Text { text } => json!({ "type": "text", "text": text }),
            ContentBlock::ToolUse { id, name, input } => json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input,
            }),
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } => json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": content,
            }),
            ContentBlock::Image { source } => convert_image_source(source),
        })
        .collect()
}

/// Convert an ImageSource to the Anthropic API JSON format.
fn convert_image_source(source: &ImageSource) -> Value {
    json!({
        "type": "image",
        "source": {
            "type": source.source_type,
            "media_type": source.media_type,
            "data": source.data,
        }
    })
}

/// Convert tool definitions to Anthropic format.
fn convert_tools(tools: &[ToolDefinition]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
            })
        })
        .collect()
}

/// Classify HTTP error status codes into meaningful error messages.
fn classify_http_error(status: u16, body: &str) -> anyhow::Error {
    match status {
        401 | 403 => anyhow::anyhow!("Anthropic auth error ({}): {}", status, body),
        429 => anyhow::anyhow!("Anthropic rate limit exceeded (429)"),
        529 => anyhow::anyhow!("Anthropic API overloaded (529). Retry later."),
        402 => anyhow::anyhow!("Anthropic billing error (402): {}", body),
        _ => anyhow::anyhow!("Anthropic API error ({}): {}", status, body),
    }
}

/// Parse the stop_reason string from the Anthropic API.
fn parse_stop_reason(reason: &str) -> StopReason {
    match reason {
        "end_turn" => StopReason::EndTurn,
        "tool_use" => StopReason::ToolUse,
        "max_tokens" => StopReason::MaxTokens,
        _ => StopReason::EndTurn,
    }
}

impl LlmProvider for AnthropicProvider {
    fn stream_chat(
        &self,
        request: ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<AgentEvent>> + Send>>> {
        let body = build_request_body(&request);
        let url = format!("{}/v1/messages", self.base_url);
        let api_key = self.api_key.clone();
        let client = self.client.clone();

        let stream = futures_util::stream::once(async move {
            let response = client
                .post(&url)
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2025-01-24")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("Anthropic request failed: {}", e))?;

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let body = response.text().await.unwrap_or_default();
                return Err(classify_http_error(status, &body));
            }

            Ok(response)
        })
        .flat_map(move |response_result| {
            match response_result {
                Err(e) => {
                    // Yield a single error and stop.
                    Box::pin(futures_util::stream::once(async move { Err(e) }))
                        as Pin<Box<dyn Stream<Item = Result<AgentEvent>> + Send>>
                }
                Ok(response) => {
                    let byte_stream = response.bytes_stream();
                    let sse_stream = parse_sse_stream(byte_stream);
                    Box::pin(AnthropicEventStream::new(sse_stream))
                }
            }
        });

        Ok(Box::pin(stream))
    }
}

/// Adapter that converts SSE events into AgentEvents, managing tool accumulation state.
struct AnthropicEventStream {
    inner: Pin<Box<dyn Stream<Item = Result<super::streaming::SseEvent>> + Send>>,
    current_tool_id: Option<String>,
    current_tool_name: Option<String>,
    current_tool_input_json: String,
    /// Whether the current content block is a thinking block.
    in_thinking_block: bool,
    pending: Vec<Result<AgentEvent>>,
    done: bool,
}

impl AnthropicEventStream {
    fn new(inner: Pin<Box<dyn Stream<Item = Result<super::streaming::SseEvent>> + Send>>) -> Self {
        Self {
            inner,
            current_tool_id: None,
            current_tool_name: None,
            current_tool_input_json: String::new(),
            in_thinking_block: false,
            pending: Vec::new(),
            done: false,
        }
    }

    fn process_sse_event(&mut self, sse_event: super::streaming::SseEvent) {
        let event_type = sse_event.event_type.as_deref().unwrap_or("");

        match event_type {
            "content_block_start" => {
                if let Ok(data) = serde_json::from_str::<Value>(&sse_event.data) {
                    let block = &data["content_block"];
                    match block["type"].as_str() {
                        Some("tool_use") => {
                            self.current_tool_id = block["id"].as_str().map(String::from);
                            self.current_tool_name = block["name"].as_str().map(String::from);
                            self.current_tool_input_json.clear();
                            self.in_thinking_block = false;
                        }
                        Some("thinking") => {
                            self.in_thinking_block = true;
                        }
                        _ => {
                            self.in_thinking_block = false;
                        }
                    }
                }
            }

            "content_block_delta" => {
                if let Ok(data) = serde_json::from_str::<Value>(&sse_event.data) {
                    let delta = &data["delta"];
                    match delta["type"].as_str() {
                        Some("text_delta") => {
                            if let Some(text) = delta["text"].as_str() {
                                self.pending
                                    .push(Ok(AgentEvent::TextDelta(text.to_string())));
                            }
                        }
                        Some("thinking_delta") => {
                            if let Some(text) = delta["thinking"].as_str() {
                                self.pending
                                    .push(Ok(AgentEvent::Thinking(text.to_string())));
                            }
                        }
                        Some("input_json_delta") => {
                            if let Some(json_chunk) = delta["partial_json"].as_str() {
                                self.current_tool_input_json.push_str(json_chunk);
                            }
                        }
                        _ => {}
                    }
                }
            }

            "content_block_stop" => {
                if self.in_thinking_block {
                    // Thinking block ended, clear state
                    self.in_thinking_block = false;
                } else if let (Some(id), Some(name)) =
                    (self.current_tool_id.take(), self.current_tool_name.take())
                {
                    let input: Value = serde_json::from_str(&self.current_tool_input_json)
                        .unwrap_or(Value::Object(Default::default()));
                    self.current_tool_input_json.clear();
                    self.pending
                        .push(Ok(AgentEvent::ToolUse { id, name, input }));
                }
            }

            "message_start" => {
                if let Ok(data) = serde_json::from_str::<Value>(&sse_event.data) {
                    let usage = &data["message"]["usage"];
                    let input_tokens = usage["input_tokens"].as_u64().unwrap_or(0);
                    let cache_creation = usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
                    let cache_read = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
                    if input_tokens > 0 || cache_creation > 0 || cache_read > 0 {
                        self.pending.push(Ok(AgentEvent::UsageUpdate {
                            input_tokens,
                            output_tokens: 0,
                            cache_creation_input_tokens: cache_creation,
                            cache_read_input_tokens: cache_read,
                        }));
                    }
                }
            }

            "message_delta" => {
                if let Ok(data) = serde_json::from_str::<Value>(&sse_event.data) {
                    if let Some(reason) = data["delta"]["stop_reason"].as_str() {
                        self.pending.push(Ok(AgentEvent::MessageEnd {
                            stop_reason: parse_stop_reason(reason),
                        }));
                    }
                    let output_tokens = data["usage"]["output_tokens"].as_u64().unwrap_or(0);
                    if output_tokens > 0 {
                        self.pending.push(Ok(AgentEvent::UsageUpdate {
                            input_tokens: 0,
                            output_tokens,
                            cache_creation_input_tokens: 0,
                            cache_read_input_tokens: 0,
                        }));
                    }
                }
            }

            "message_stop" => {
                self.pending.push(Ok(AgentEvent::MessageEnd {
                    stop_reason: StopReason::EndTurn,
                }));
            }

            "error" => {
                let message = serde_json::from_str::<Value>(&sse_event.data)
                    .ok()
                    .and_then(|d| d["error"]["message"].as_str().map(String::from))
                    .unwrap_or_else(|| "unknown error".to_string());
                self.pending
                    .push(Err(anyhow::anyhow!("Anthropic stream error: {}", message)));
            }

            _ => {} // Ignore ping, etc.
        }
    }
}

impl Stream for AnthropicEventStream {
    type Item = Result<AgentEvent>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use std::task::Poll;

        if self.done {
            return Poll::Ready(None);
        }

        // Return buffered events first.
        if !self.pending.is_empty() {
            let event = self.pending.remove(0);
            if event.is_err() {
                self.done = true;
            }
            return Poll::Ready(Some(event));
        }

        // Poll the inner SSE stream.
        loop {
            match self.inner.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(sse_event))) => {
                    self.process_sse_event(sse_event);
                    if !self.pending.is_empty() {
                        let event = self.pending.remove(0);
                        if event.is_err() {
                            self.done = true;
                        }
                        return Poll::Ready(Some(event));
                    }
                    // No events produced from this SSE event, keep polling.
                }
                Poll::Ready(Some(Err(e))) => {
                    self.done = true;
                    return Poll::Ready(Some(Err(e)));
                }
                Poll::Ready(None) => {
                    self.done = true;
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
    use crate::agent::types::*;

    fn simple_request() -> ChatRequest {
        ChatRequest {
            messages: vec![Message::user("hello")],
            system_prompt: "You are helpful.".to_string(),
            tools: vec![],
            model: "claude-sonnet-4-20250514".to_string(),
            max_tokens: 1024,
            temperature: None,
            thinking_budget: None,
        }
    }

    #[test]
    fn test_build_request_body_basic() {
        let body = build_request_body(&simple_request());

        assert_eq!(body["model"], "claude-sonnet-4-20250514");
        assert_eq!(body["max_tokens"], 1024);
        assert_eq!(body["stream"], true);
        assert!(body["system"].is_array());
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["messages"][0]["role"], "user");
        // No tools in request => tools key absent
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn test_build_request_body_with_tools() {
        let mut req = simple_request();
        req.tools = vec![ToolDefinition {
            name: "bash".to_string(),
            description: "Run a command".to_string(),
            input_schema: json!({"type": "object", "properties": {"command": {"type": "string"}}}),
        }];
        let body = build_request_body(&req);

        assert!(body["tools"].is_array());
        assert_eq!(body["tools"][0]["name"], "bash");
    }

    #[test]
    fn test_build_request_body_with_temperature() {
        let mut req = simple_request();
        req.temperature = Some(0.7);
        let body = build_request_body(&req);

        let temp = body["temperature"].as_f64().unwrap();
        assert!((temp - 0.7).abs() < 0.01);
    }

    #[test]
    fn test_build_request_body_no_system_prompt() {
        let mut req = simple_request();
        req.system_prompt = String::new();
        let body = build_request_body(&req);

        assert!(body.get("system").is_none());
    }

    #[test]
    fn test_parse_stop_reason() {
        assert_eq!(parse_stop_reason("end_turn"), StopReason::EndTurn);
        assert_eq!(parse_stop_reason("tool_use"), StopReason::ToolUse);
        assert_eq!(parse_stop_reason("max_tokens"), StopReason::MaxTokens);
        assert_eq!(parse_stop_reason("unknown"), StopReason::EndTurn);
    }

    #[test]
    fn test_process_text_delta() {
        let empty_stream = Box::pin(futures_util::stream::empty());
        let mut stream = AnthropicEventStream::new(empty_stream);

        stream.process_sse_event(super::super::streaming::SseEvent {
            event_type: Some("content_block_delta".to_string()),
            data: json!({"delta": {"type": "text_delta", "text": "Hello"}}).to_string(),
        });

        assert_eq!(stream.pending.len(), 1);
        match &stream.pending[0] {
            Ok(AgentEvent::TextDelta(t)) => assert_eq!(t, "Hello"),
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[test]
    fn test_process_tool_use() {
        let empty_stream = Box::pin(futures_util::stream::empty());
        let mut stream = AnthropicEventStream::new(empty_stream);

        // Start tool
        stream.process_sse_event(super::super::streaming::SseEvent {
            event_type: Some("content_block_start".to_string()),
            data: json!({"content_block": {"type": "tool_use", "id": "t1", "name": "bash"}})
                .to_string(),
        });

        // Input JSON delta
        stream.process_sse_event(super::super::streaming::SseEvent {
            event_type: Some("content_block_delta".to_string()),
            data: json!({"delta": {"type": "input_json_delta", "partial_json": "{\"command\":\"ls\"}"}}).to_string(),
        });

        // End tool block
        stream.process_sse_event(super::super::streaming::SseEvent {
            event_type: Some("content_block_stop".to_string()),
            data: "{}".to_string(),
        });

        assert_eq!(stream.pending.len(), 1);
        match &stream.pending[0] {
            Ok(AgentEvent::ToolUse { id, name, input }) => {
                assert_eq!(id, "t1");
                assert_eq!(name, "bash");
                assert_eq!(input["command"], "ls");
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[test]
    fn test_classify_http_error() {
        let err = classify_http_error(401, "unauthorized");
        assert!(err.to_string().contains("auth error"));

        let err = classify_http_error(429, "");
        assert!(err.to_string().contains("rate limit"));

        let err = classify_http_error(529, "");
        assert!(err.to_string().contains("overloaded"));
    }

    #[test]
    fn test_build_request_body_with_thinking() {
        let mut req = simple_request();
        req.thinking_budget = Some(10000);
        let body = build_request_body(&req);

        // Thinking should be enabled
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 10000);
        // max_tokens should be thinking_budget + max_tokens
        assert_eq!(body["max_tokens"], 10000 + 1024);
        // Temperature must not be present when thinking is enabled
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn test_build_request_body_thinking_zero_budget() {
        let mut req = simple_request();
        req.thinking_budget = Some(0);
        let body = build_request_body(&req);

        // Thinking should NOT be enabled with zero budget
        assert!(body.get("thinking").is_none());
        assert_eq!(body["max_tokens"], 1024);
    }

    #[test]
    fn test_build_request_body_thinking_strips_temperature() {
        let mut req = simple_request();
        req.thinking_budget = Some(5000);
        req.temperature = Some(0.7);
        let body = build_request_body(&req);

        // Temperature must be omitted when thinking is enabled
        assert!(body.get("temperature").is_none());
        assert_eq!(body["thinking"]["type"], "enabled");
    }

    #[test]
    fn test_build_request_body_cache_control_on_last_tool() {
        let mut req = simple_request();
        req.tools = vec![
            ToolDefinition {
                name: "bash".to_string(),
                description: "Run a command".to_string(),
                input_schema: json!({"type": "object"}),
            },
            ToolDefinition {
                name: "read".to_string(),
                description: "Read a file".to_string(),
                input_schema: json!({"type": "object"}),
            },
        ];
        let body = build_request_body(&req);

        // First tool should NOT have cache_control
        assert!(body["tools"][0].get("cache_control").is_none());
        // Last tool should have cache_control
        assert_eq!(body["tools"][1]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn test_build_request_body_cache_control_on_last_user_message() {
        let mut req = simple_request();
        req.messages = vec![
            Message::user("first message"),
            Message::assistant(vec![ContentBlock::Text {
                text: "response".to_string(),
            }]),
            Message::user("second message"),
        ];
        let body = build_request_body(&req);

        // Only the last user message's last content block should have cache_control
        assert!(body["messages"][0]["content"][0]
            .get("cache_control")
            .is_none());
        assert_eq!(
            body["messages"][2]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn test_process_thinking_delta() {
        let empty_stream = Box::pin(futures_util::stream::empty());
        let mut stream = AnthropicEventStream::new(empty_stream);

        // Start a thinking block
        stream.process_sse_event(super::super::streaming::SseEvent {
            event_type: Some("content_block_start".to_string()),
            data: json!({"content_block": {"type": "thinking"}}).to_string(),
        });

        assert!(stream.in_thinking_block);

        // Thinking delta
        stream.process_sse_event(super::super::streaming::SseEvent {
            event_type: Some("content_block_delta".to_string()),
            data: json!({"delta": {"type": "thinking_delta", "thinking": "Let me think..."}})
                .to_string(),
        });

        assert_eq!(stream.pending.len(), 1);
        match &stream.pending[0] {
            Ok(AgentEvent::Thinking(t)) => assert_eq!(t, "Let me think..."),
            other => panic!("unexpected event: {:?}", other),
        }

        // Stop thinking block
        stream.pending.clear();
        stream.process_sse_event(super::super::streaming::SseEvent {
            event_type: Some("content_block_stop".to_string()),
            data: "{}".to_string(),
        });

        assert!(!stream.in_thinking_block);
        // No ToolUse event should be emitted for thinking block stop
        assert!(stream.pending.is_empty());
    }

    #[test]
    fn test_process_message_start_with_cache_tokens() {
        let empty_stream = Box::pin(futures_util::stream::empty());
        let mut stream = AnthropicEventStream::new(empty_stream);

        stream.process_sse_event(super::super::streaming::SseEvent {
            event_type: Some("message_start".to_string()),
            data: json!({
                "message": {
                    "usage": {
                        "input_tokens": 100,
                        "cache_creation_input_tokens": 500,
                        "cache_read_input_tokens": 200
                    }
                }
            })
            .to_string(),
        });

        assert_eq!(stream.pending.len(), 1);
        match &stream.pending[0] {
            Ok(AgentEvent::UsageUpdate {
                input_tokens,
                output_tokens,
                cache_creation_input_tokens,
                cache_read_input_tokens,
            }) => {
                assert_eq!(*input_tokens, 100);
                assert_eq!(*output_tokens, 0);
                assert_eq!(*cache_creation_input_tokens, 500);
                assert_eq!(*cache_read_input_tokens, 200);
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }
}
