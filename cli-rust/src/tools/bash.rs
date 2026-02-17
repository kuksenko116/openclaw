// Bash command execution tool.
//
// Runs shell commands via /bin/bash -c with stdout/stderr capture and timeout.

use anyhow::Result;
use serde_json::{json, Value};

use crate::agent::types::{ToolDefinition, ToolResult};

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_TIMEOUT_MS: u64 = 600_000;
const MAX_OUTPUT_CHARS: usize = 30_000;

/// Return the tool definition for the bash tool.
pub(crate) fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "bash".to_string(),
        description: "Execute a bash command. Capture stdout and stderr.".to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["command"],
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (max 600000)"
                }
            }
        }),
    }
}

/// Execute a bash command and return the result.
pub(crate) async fn execute(args: Value) -> Result<ToolResult> {
    let command = args["command"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing 'command' parameter"))?;

    let timeout_ms = args["timeout"]
        .as_u64()
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .min(MAX_TIMEOUT_MS);

    run_command(command, timeout_ms).await
}

/// Run a shell command with timeout, capturing stdout and stderr.
async fn run_command(command: &str, timeout_ms: u64) -> Result<ToolResult> {
    let shell = resolve_shell();

    let child = tokio::process::Command::new(&shell)
        .arg("-c")
        .arg(command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn command: {}", e))?;

    let timeout = tokio::time::Duration::from_millis(timeout_ms);

    let output = tokio::select! {
        result = child.wait_with_output() => {
            result.map_err(|e| anyhow::anyhow!("command execution error: {}", e))?
        }
        _ = tokio::time::sleep(timeout) => {
            // kill_on_drop handles cleanup when `child` is dropped here
            return Ok(ToolResult {
                content: format!("Command timed out after {}ms", timeout_ms),
                is_error: true,
            });
        }
    };

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let content = format_output(&stdout, &stderr, exit_code);
    let content = truncate_output(&content);

    Ok(ToolResult {
        content,
        is_error: exit_code != 0,
    })
}

/// Resolve the shell binary to use.
fn resolve_shell() -> String {
    if let Ok(shell) = std::env::var("SHELL") {
        if std::path::Path::new(&shell).exists() {
            return shell;
        }
    }
    if std::path::Path::new("/bin/bash").exists() {
        return "/bin/bash".to_string();
    }
    "/bin/sh".to_string()
}

/// Format stdout, stderr, and exit code into a single output string.
fn format_output(stdout: &str, stderr: &str, exit_code: i32) -> String {
    let mut result = String::new();

    if !stdout.is_empty() {
        result.push_str(stdout);
    }
    if !stderr.is_empty() {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str("STDERR:\n");
        result.push_str(stderr);
    }
    if exit_code != 0 {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(&format!("Exit code: {}", exit_code));
    }
    if result.is_empty() {
        result.push_str("(no output)");
    }

    result
}

/// Find the byte offset of the n-th character boundary in a string.
fn char_boundary(s: &str, n: usize) -> usize {
    s.char_indices().nth(n).map(|(i, _)| i).unwrap_or(s.len())
}

/// Truncate output if it exceeds the character limit.
fn truncate_output(output: &str) -> String {
    let char_count = output.chars().count();
    if char_count <= MAX_OUTPUT_CHARS {
        return output.to_string();
    }

    let keep = MAX_OUTPUT_CHARS - 60;
    let half = keep / 2;
    let start_end = char_boundary(output, half);
    let tail_start = char_boundary(output, char_count - half);
    format!(
        "{}\n\n... [truncated {} chars] ...\n\n{}",
        &output[..start_end],
        char_count - keep,
        &output[tail_start..],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_shell() {
        let shell = resolve_shell();
        assert!(shell.contains("sh"));
    }

    #[test]
    fn test_format_output_success() {
        let result = format_output("hello\n", "", 0);
        assert_eq!(result, "hello\n");
    }

    #[test]
    fn test_format_output_with_stderr() {
        let result = format_output("out", "err", 1);
        assert!(result.contains("out"));
        assert!(result.contains("STDERR:"));
        assert!(result.contains("err"));
        assert!(result.contains("Exit code: 1"));
    }

    #[test]
    fn test_format_output_empty() {
        let result = format_output("", "", 0);
        assert_eq!(result, "(no output)");
    }

    #[test]
    fn test_truncate_short() {
        let result = truncate_output("short");
        assert_eq!(result, "short");
    }

    #[test]
    fn test_truncate_long() {
        let long = "x".repeat(MAX_OUTPUT_CHARS + 500);
        let result = truncate_output(&long);
        assert!(result.len() < MAX_OUTPUT_CHARS + 100);
        assert!(result.contains("[truncated"));
    }
}
