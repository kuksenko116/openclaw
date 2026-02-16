// Tool system: registry, dispatch, and built-in tool implementations.
//
// Implements the ToolExecutor trait from agent/mod.rs.
//
// Dependencies needed in Cargo.toml:
//   glob = "0.3"

pub(crate) mod bash;
pub(crate) mod file_ops;
pub(crate) mod policy;
pub(crate) mod web_fetch;

use anyhow::Result;
use serde_json::Value;

use crate::agent::types::{ToolDefinition, ToolResult};
use crate::agent::ToolExecutor;
use crate::config::schema::ExecConfig;
use policy::ToolPolicy;

/// Registry of available tools implementing ToolExecutor.
pub(crate) struct ToolRegistry {
    policy: ToolPolicy,
    exec_config: ExecConfig,
}

impl ToolRegistry {
    /// Create a new tool registry with the given policy and exec config.
    pub fn new(policy: ToolPolicy, exec_config: ExecConfig) -> Self {
        Self {
            policy,
            exec_config,
        }
    }
}

#[async_trait::async_trait]
impl ToolExecutor for ToolRegistry {
    /// Execute a tool by name with the given arguments.
    async fn execute(&self, name: &str, args: Value) -> Result<ToolResult> {
        if !self.policy.is_tool_allowed(name) {
            return Ok(ToolResult {
                content: format!("Tool '{}' is not allowed by the current policy.", name),
                is_error: true,
            });
        }

        let result = match name {
            "bash" => {
                // Enforce command allowlist before execution
                if let Some(cmd) = args["command"].as_str() {
                    if !policy::is_command_allowed(
                        cmd,
                        &self.exec_config.allowlist,
                        &self.exec_config.security,
                    ) {
                        return Ok(ToolResult {
                            content: format!(
                                "Command not allowed by exec policy (security={:?}): {}",
                                self.exec_config.security,
                                cmd.chars().take(200).collect::<String>(),
                            ),
                            is_error: true,
                        });
                    }
                }
                bash::execute(args).await
            }
            "read" | "write" | "edit" => {
                if let Some(err_msg) = check_file_arg(&args) {
                    return Ok(ToolResult {
                        content: err_msg,
                        is_error: true,
                    });
                }
                match name {
                    "read" => file_ops::read_file(args).await,
                    "write" => file_ops::write_file(args).await,
                    "edit" => file_ops::edit_file(args).await,
                    _ => unreachable!(),
                }
            }
            "glob" => file_ops::glob_files(args).await,
            "grep" => file_ops::grep_files(args).await,
            "web_fetch" => web_fetch::execute(args).await,
            _ => Ok(ToolResult {
                content: format!("Unknown tool: '{}'", name),
                is_error: true,
            }),
        }?;

        Ok(truncate_result(result))
    }

    /// Return tool definitions for all allowed tools (sent to the LLM).
    fn definitions(&self) -> Vec<ToolDefinition> {
        let all_defs = vec![
            bash::definition(),
            file_ops::read_definition(),
            file_ops::write_definition(),
            file_ops::edit_definition(),
            file_ops::glob_definition(),
            file_ops::grep_definition(),
            web_fetch::definition(),
        ];

        all_defs
            .into_iter()
            .filter(|d| self.policy.is_tool_allowed(&d.name))
            .collect()
    }
}

/// Sensitive path prefixes and filenames that tools should not access.
const DENIED_PATHS: &[&str] = &[
    "/etc/shadow",
    "/etc/gshadow",
    "/etc/master.passwd",
    "/.ssh/",
    "/.gnupg/",
    "/.aws/credentials",
    "/.config/gcloud/",
    "/.docker/config.json",
];

/// Validate that a file path doesn't target known sensitive locations.
/// Canonicalizes the path to resolve `.`, `..`, and `//` before checking.
fn validate_file_path(path: &str) -> Result<(), String> {
    // Lexically normalize the path (resolve . and .. components, collapse //)
    use std::path::PathBuf;
    let mut normalized = PathBuf::new();
    for component in std::path::Path::new(path).components() {
        normalized.push(component);
    }
    let normalized_str = normalized.to_string_lossy();

    for denied in DENIED_PATHS {
        if normalized_str.contains(denied) {
            return Err(format!("Access denied: path matches sensitive pattern '{denied}'"));
        }
    }
    Ok(())
}

/// Extract the file_path argument from tool args and validate it.
fn check_file_arg(args: &Value) -> Option<String> {
    let path = args["file_path"].as_str()?;
    if let Err(msg) = validate_file_path(path) {
        return Some(msg);
    }
    None
}

const MAX_RESULT_CHARS: usize = 30_000;

/// Find the byte offset of the n-th character boundary in a string.
fn char_boundary(s: &str, n: usize) -> usize {
    s.char_indices()
        .nth(n)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

/// Truncate tool output that exceeds the character limit.
fn truncate_result(result: ToolResult) -> ToolResult {
    let char_count = result.content.chars().count();
    if char_count <= MAX_RESULT_CHARS {
        return result;
    }

    let keep_start = MAX_RESULT_CHARS * 2 / 3;
    let keep_end = MAX_RESULT_CHARS / 3 - 60;
    let omitted = char_count - keep_start - keep_end;

    let start_end = char_boundary(&result.content, keep_start);
    let tail_start = char_boundary(&result.content, char_count - keep_end);

    let truncated = format!(
        "{}\n\n... [truncated {} characters] ...\n\n{}",
        &result.content[..start_end],
        omitted,
        &result.content[tail_start..],
    );

    ToolResult {
        content: truncated,
        is_error: result.is_error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_short() {
        let result = ToolResult {
            content: "hello".to_string(),
            is_error: false,
        };
        let t = truncate_result(result);
        assert_eq!(t.content, "hello");
    }

    #[test]
    fn test_truncate_long() {
        let content = "x".repeat(MAX_RESULT_CHARS + 1000);
        let result = ToolResult {
            content,
            is_error: false,
        };
        let t = truncate_result(result);
        assert!(t.content.len() < MAX_RESULT_CHARS + 100);
        assert!(t.content.contains("[truncated"));
    }
}
