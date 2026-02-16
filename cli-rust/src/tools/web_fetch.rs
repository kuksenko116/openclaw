// Web fetch tool: fetch content from a URL and return it as text.
//
// Supports HTML (with tag stripping), JSON (pretty-printed), and plain text.

use anyhow::Result;
use serde_json::{json, Value};

use crate::agent::types::{ToolDefinition, ToolResult};

const MAX_OUTPUT_CHARS: usize = 50_000;
const USER_AGENT: &str = "openclaw-cli/0.1";
const FETCH_TIMEOUT_SECS: u64 = 30;

/// Return the tool definition for web_fetch.
pub(crate) fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "web_fetch".to_string(),
        description: "Fetch content from a URL. Returns the page content as text. \
            Supports HTML (converted to readable text), JSON, and plain text."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["url"],
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch"
                },
                "prompt": {
                    "type": "string",
                    "description": "Optional instruction for what to extract from the page"
                }
            }
        }),
    }
}

/// Execute the web_fetch tool.
pub(crate) async fn execute(args: Value) -> Result<ToolResult> {
    let url = match args["url"].as_str() {
        Some(u) => u,
        None => {
            return Ok(ToolResult {
                content: "Missing required 'url' parameter.".to_string(),
                is_error: true,
            });
        }
    };

    // Validate URL scheme
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Ok(ToolResult {
            content: format!("Invalid URL scheme. Only http:// and https:// are supported: {}", url),
            is_error: true,
        });
    }

    let prompt = args["prompt"].as_str();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {}", e))?;

    let response = match client
        .get(url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            return Ok(ToolResult {
                content: format!("Failed to fetch URL '{}': {}", url, e),
                is_error: true,
            });
        }
    };

    let status = response.status();
    if !status.is_success() {
        return Ok(ToolResult {
            content: format!(
                "HTTP error {} fetching '{}': {}",
                status.as_u16(),
                url,
                status.canonical_reason().unwrap_or("unknown"),
            ),
            is_error: true,
        });
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    let body = match response.text().await {
        Ok(b) => b,
        Err(e) => {
            return Ok(ToolResult {
                content: format!("Failed to read response body from '{}': {}", url, e),
                is_error: true,
            });
        }
    };

    let processed = if content_type.contains("text/html") {
        strip_html(&body)
    } else if content_type.contains("json") {
        // Try to pretty-print JSON
        match serde_json::from_str::<Value>(&body) {
            Ok(val) => serde_json::to_string_pretty(&val).unwrap_or(body),
            Err(_) => body,
        }
    } else {
        body
    };

    let truncated = truncate_output(&processed);

    let content = if let Some(p) = prompt {
        format!(
            "URL: {}\nPrompt: {}\n\n---\n\n{}",
            url, p, truncated
        )
    } else {
        format!("URL: {}\n\n---\n\n{}", url, truncated)
    };

    Ok(ToolResult {
        content,
        is_error: false,
    })
}

/// Strip HTML tags from content, removing script/style blocks first.
///
/// Uses a simple state-machine approach to avoid a regex dependency:
/// 1. Remove <script>...</script> and <style>...</style> blocks
/// 2. Strip all remaining HTML tags
/// 3. Collapse excessive whitespace
fn strip_html(html: &str) -> String {
    // Phase 1: Remove <script>...</script> and <style>...</style> blocks
    let without_scripts = remove_tag_blocks(html, "script");
    let without_styles = remove_tag_blocks(&without_scripts, "style");

    // Phase 2: Strip remaining HTML tags
    let stripped = strip_tags(&without_styles);

    // Phase 3: Decode common HTML entities
    let decoded = decode_html_entities(&stripped);

    // Phase 4: Collapse excessive whitespace
    collapse_whitespace(&decoded)
}

/// Remove all content between <tag...>...</tag> (case-insensitive).
fn remove_tag_blocks(input: &str, tag_name: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let lower = input.to_lowercase();
    let open_tag = format!("<{}", tag_name);
    let close_tag = format!("</{}>", tag_name);

    let mut pos = 0;
    while pos < input.len() {
        if let Some(start) = lower[pos..].find(&open_tag) {
            let abs_start = pos + start;
            // Copy everything before this tag
            result.push_str(&input[pos..abs_start]);
            // Find the closing tag
            if let Some(end) = lower[abs_start..].find(&close_tag) {
                pos = abs_start + end + close_tag.len();
            } else {
                // No closing tag found; skip to end
                pos = input.len();
            }
        } else {
            // No more occurrences; copy the rest
            result.push_str(&input[pos..]);
            break;
        }
    }

    result
}

/// Strip all HTML tags from the input using a simple state machine.
fn strip_tags(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut in_tag = false;

    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                if in_tag {
                    in_tag = false;
                    // Add a space to prevent words from merging
                    result.push(' ');
                } else {
                    result.push(ch);
                }
            }
            _ => {
                if !in_tag {
                    result.push(ch);
                }
            }
        }
    }

    result
}

/// Decode common HTML entities.
fn decode_html_entities(input: &str) -> String {
    input
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

/// Collapse runs of whitespace into single spaces/newlines.
fn collapse_whitespace(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut prev_was_newline = false;
    let mut prev_was_space = false;
    let mut consecutive_newlines = 0;

    for ch in input.chars() {
        if ch == '\n' || ch == '\r' {
            if ch == '\r' {
                continue; // Skip \r, handle \n
            }
            consecutive_newlines += 1;
            if consecutive_newlines <= 2 {
                result.push('\n');
            }
            prev_was_newline = true;
            prev_was_space = false;
        } else if ch.is_whitespace() {
            if !prev_was_space && !prev_was_newline {
                result.push(' ');
            }
            prev_was_space = true;
            consecutive_newlines = 0;
        } else {
            result.push(ch);
            prev_was_space = false;
            prev_was_newline = false;
            consecutive_newlines = 0;
        }
    }

    result.trim().to_string()
}

/// Truncate output to MAX_OUTPUT_CHARS.
fn truncate_output(content: &str) -> String {
    let char_count = content.chars().count();
    if char_count <= MAX_OUTPUT_CHARS {
        return content.to_string();
    }
    let truncated: String = content.chars().take(MAX_OUTPUT_CHARS).collect();
    format!("{}\n\n[truncated: {} characters total, showing first {}]", truncated, char_count, MAX_OUTPUT_CHARS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_tags_basic() {
        let result = strip_tags("<p>Hello <b>world</b></p>");
        assert!(result.contains("Hello"));
        assert!(result.contains("world"));
        assert!(!result.contains("<p>"));
        assert!(!result.contains("<b>"));
    }

    #[test]
    fn test_strip_html_with_script() {
        let html = "<html><head><script>var x = 1;</script></head><body><p>Hello</p></body></html>";
        let result = strip_html(html);
        assert!(result.contains("Hello"));
        assert!(!result.contains("var x"));
        assert!(!result.contains("<script>"));
    }

    #[test]
    fn test_strip_html_with_style() {
        let html = "<html><head><style>body { color: red; }</style></head><body><p>Content</p></body></html>";
        let result = strip_html(html);
        assert!(result.contains("Content"));
        assert!(!result.contains("color: red"));
    }

    #[test]
    fn test_remove_tag_blocks_nested() {
        let html = "<div>Before<script type=\"text/javascript\">alert('hi');</script>After</div>";
        let result = remove_tag_blocks(html, "script");
        assert!(result.contains("Before"));
        assert!(result.contains("After"));
        assert!(!result.contains("alert"));
    }

    #[test]
    fn test_collapse_whitespace() {
        let input = "Hello    world\n\n\n\n\nfoo  bar";
        let result = collapse_whitespace(input);
        assert_eq!(result, "Hello world\n\nfoo bar");
    }

    #[test]
    fn test_decode_html_entities() {
        assert_eq!(decode_html_entities("&amp; &lt; &gt;"), "& < >");
        assert_eq!(decode_html_entities("&quot;hello&quot;"), "\"hello\"");
        assert_eq!(decode_html_entities("it&#39;s"), "it's");
    }

    #[test]
    fn test_truncate_output_short() {
        let content = "hello world";
        assert_eq!(truncate_output(content), "hello world");
    }

    #[test]
    fn test_truncate_output_long() {
        let content = "x".repeat(MAX_OUTPUT_CHARS + 100);
        let result = truncate_output(&content);
        assert!(result.contains("[truncated:"));
        assert!(result.len() < content.len() + 100);
    }

    #[test]
    fn test_definition_has_required_fields() {
        let def = definition();
        assert_eq!(def.name, "web_fetch");
        assert!(!def.description.is_empty());
        assert!(def.input_schema["properties"]["url"].is_object());
    }

    #[tokio::test]
    async fn test_execute_missing_url() {
        let args = json!({});
        let result = execute(args).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Missing"));
    }

    #[tokio::test]
    async fn test_execute_invalid_scheme() {
        let args = json!({ "url": "ftp://example.com/file" });
        let result = execute(args).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid URL scheme"));
    }

    #[test]
    fn test_strip_html_full_page() {
        let html = r#"<!DOCTYPE html>
<html>
<head>
    <title>Test Page</title>
    <script>console.log("test");</script>
    <style>.foo { display: none; }</style>
</head>
<body>
    <h1>Hello World</h1>
    <p>This is a <strong>test</strong> paragraph.</p>
    <div>
        <a href="https://example.com">Link text</a>
    </div>
</body>
</html>"#;
        let result = strip_html(html);
        assert!(result.contains("Hello World"));
        assert!(result.contains("test"));
        assert!(result.contains("paragraph"));
        assert!(result.contains("Link text"));
        assert!(!result.contains("console.log"));
        assert!(!result.contains("display: none"));
        assert!(!result.contains("<h1>"));
        assert!(!result.contains("<script>"));
    }
}
