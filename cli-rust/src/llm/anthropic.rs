// Anthropic Messages API provider.
//
// Streams responses via SSE from POST /v1/messages with stream: true.
// Implements the LlmProvider trait from agent/mod.rs.

use anyhow::Result;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::pin::Pin;
use tokio_stream::Stream;

use crate::agent::types::{
    AgentEvent, ChatRequest, ContentBlock, Message, Role, StopReason, ToolDefinition,
};
use crate::agent::LlmProvider;
use super::streaming::parse_sse_stream;

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
    let messages = convert_messages(&request.messages);
    let tools = convert_tools(&request.tools);

    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "max_tokens": request.max_tokens,
        "stream": true,
    });

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

    if let Some(temp) = request.temperature {
        body["temperature"] = json!(temp);
    }

    body
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
        })
        .collect()
}

/// Convert tool definitions to Anthropic format.
fn convert_tools(tools: &[ToolDefinition]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| json!({
            "name": t.name,
            "description": t.description,
            "input_schema": t.input_schema,
        }))
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
                .header("anthropic-version", "2023-06-01")
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
    pending: Vec<Result<AgentEvent>>,
    done: bool,
}

impl AnthropicEventStream {
    fn new(
        inner: Pin<Box<dyn Stream<Item = Result<super::streaming::SseEvent>> + Send>>,
    ) -> Self {
        Self {
            inner,
            current_tool_id: None,
            current_tool_name: None,
            current_tool_input_json: String::new(),
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
                    if block["type"].as_str() == Some("tool_use") {
                        self.current_tool_id = block["id"].as_str().map(String::from);
                        self.current_tool_name = block["name"].as_str().map(String::from);
                        self.current_tool_input_json.clear();
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
                if let (Some(id), Some(name)) =
                    (self.current_tool_id.take(), self.current_tool_name.take())
                {
                    let input: Value =
                        serde_json::from_str(&self.current_tool_input_json)
                            .unwrap_or(Value::Object(Default::default()));
                    self.current_tool_input_json.clear();
                    self.pending
                        .push(Ok(AgentEvent::ToolUse { id, name, input }));
                }
            }

            "message_start" => {
                if let Ok(data) = serde_json::from_str::<Value>(&sse_event.data) {
                    let input_tokens = data["message"]["usage"]["input_tokens"].as_u64().unwrap_or(0);
                    if input_tokens > 0 {
                        self.pending.push(Ok(AgentEvent::UsageUpdate {
                            input_tokens,
                            output_tokens: 0,
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

            _ => {} // Ignore ping, message_start, etc.
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
            data: json!({"content_block": {"type": "tool_use", "id": "t1", "name": "bash"}}).to_string(),
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
}
