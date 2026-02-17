// SSE (Server-Sent Events) parser for Anthropic and OpenAI streaming.
//
// SSE format:
//   event: event_name\n
//   data: json_payload\n
//   \n
//
// Rules:
// - Lines starting with ':' are comments (ignored).
// - Empty line is the event boundary.
// - data fields can span multiple lines (joined with '\n').
// - event field sets the event type for the next dispatched event.

use anyhow::Result;
use bytes::Bytes;
use futures_util::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};

/// A parsed SSE event.
#[derive(Debug, Clone)]
pub(crate) struct SseEvent {
    pub event_type: Option<String>,
    pub data: String,
}

/// Parse a byte stream into SSE events.
pub(crate) fn parse_sse_stream(
    byte_stream: impl Stream<Item = reqwest::Result<Bytes>> + Unpin + Send + 'static,
) -> Pin<Box<dyn Stream<Item = Result<SseEvent>> + Send>> {
    Box::pin(SseStream::new(byte_stream))
}

struct SseStream<S> {
    inner: S,
    buffer: String,
    current_event_type: Option<String>,
    current_data: Vec<String>,
    pending_events: Vec<SseEvent>,
}

impl<S> SseStream<S>
where
    S: Stream<Item = reqwest::Result<Bytes>> + Unpin,
{
    fn new(inner: S) -> Self {
        Self {
            inner,
            buffer: String::new(),
            current_event_type: None,
            current_data: Vec::new(),
            pending_events: Vec::new(),
        }
    }

    /// Process a single line and optionally produce an event.
    fn process_line(&mut self, line: &str) {
        if line.is_empty() {
            // Empty line = event boundary.
            if !self.current_data.is_empty() {
                let event = SseEvent {
                    event_type: self.current_event_type.take(),
                    data: self.current_data.join("\n"),
                };
                self.current_data.clear();
                self.pending_events.push(event);
            }
            return;
        }

        // Comment line.
        if line.starts_with(':') {
            return;
        }

        let (field, value) = match line.find(':') {
            Some(pos) => {
                let value = &line[pos + 1..];
                // Strip single leading space per SSE spec.
                let value = value.strip_prefix(' ').unwrap_or(value);
                (&line[..pos], value)
            }
            None => (line, ""),
        };

        match field {
            "event" => self.current_event_type = Some(value.to_string()),
            "data" => self.current_data.push(value.to_string()),
            "id" | "retry" => {} // Ignored for our use case.
            _ => {}
        }
    }

    /// Consume bytes from the inner stream, parse lines, and produce events.
    fn process_bytes(&mut self, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        self.buffer.push_str(&text);

        // Process complete lines.
        while let Some(newline_pos) = self.buffer.find('\n') {
            let line = self.buffer[..newline_pos]
                .trim_end_matches('\r')
                .to_string();
            self.buffer = self.buffer[newline_pos + 1..].to_string();
            self.process_line(&line);
        }
    }
}

impl<S> Stream for SseStream<S>
where
    S: Stream<Item = reqwest::Result<Bytes>> + Unpin,
{
    type Item = Result<SseEvent>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;

        // Return any buffered events first.
        if !this.pending_events.is_empty() {
            return Poll::Ready(Some(Ok(this.pending_events.remove(0))));
        }

        // Poll the inner stream for more bytes.
        loop {
            match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    this.process_bytes(&bytes);
                    if !this.pending_events.is_empty() {
                        return Poll::Ready(Some(Ok(this.pending_events.remove(0))));
                    }
                    // Keep polling for more data.
                }
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(anyhow::anyhow!("SSE stream error: {}", e))));
                }
                Poll::Ready(None) => {
                    // Stream ended. Flush any remaining data as a final event.
                    if !this.current_data.is_empty() {
                        let event = SseEvent {
                            event_type: this.current_event_type.take(),
                            data: this.current_data.join("\n"),
                        };
                        this.current_data.clear();
                        return Poll::Ready(Some(Ok(event)));
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
    use futures_util::stream;
    use tokio_stream::StreamExt;

    /// Helper to create a byte stream from raw strings for testing.
    fn test_stream(
        items: Vec<&'static str>,
    ) -> impl Stream<Item = reqwest::Result<Bytes>> + Unpin + Send + 'static {
        stream::iter(
            items
                .into_iter()
                .map(|s| Ok(Bytes::from(s)) as reqwest::Result<Bytes>)
                .collect::<Vec<_>>(),
        )
    }

    #[tokio::test]
    async fn test_parse_simple_event() {
        let mut sse = parse_sse_stream(test_stream(vec![
            "event: message\ndata: {\"text\":\"hello\"}\n\n",
        ]));

        let event = sse.next().await.unwrap().unwrap();
        assert_eq!(event.event_type.as_deref(), Some("message"));
        assert_eq!(event.data, "{\"text\":\"hello\"}");
    }

    #[tokio::test]
    async fn test_parse_multiline_data() {
        let mut sse = parse_sse_stream(test_stream(vec!["data: line1\ndata: line2\n\n"]));

        let event = sse.next().await.unwrap().unwrap();
        assert_eq!(event.data, "line1\nline2");
    }

    #[tokio::test]
    async fn test_ignore_comments() {
        let mut sse = parse_sse_stream(test_stream(vec![
            ": this is a comment\nevent: test\ndata: hello\n\n",
        ]));

        let event = sse.next().await.unwrap().unwrap();
        assert_eq!(event.event_type.as_deref(), Some("test"));
        assert_eq!(event.data, "hello");
    }

    #[tokio::test]
    async fn test_chunked_delivery() {
        // Data split across multiple byte chunks.
        let mut sse = parse_sse_stream(test_stream(vec!["event: te", "st\ndata: hel", "lo\n\n"]));

        let event = sse.next().await.unwrap().unwrap();
        assert_eq!(event.event_type.as_deref(), Some("test"));
        assert_eq!(event.data, "hello");
    }
}
