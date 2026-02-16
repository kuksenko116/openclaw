// File operation tools: read, write, edit, glob, grep.
//
// Each function does one thing and returns a ToolResult.

use anyhow::Result;
use serde_json::{json, Value};

use crate::agent::types::{ToolDefinition, ToolResult};

const DEFAULT_READ_LIMIT: usize = 2000;

// ── Read ────────────────────────────────────────────────────────────────────

pub(crate) fn read_definition() -> ToolDefinition {
    ToolDefinition {
        name: "read".to_string(),
        description: "Read a file from the filesystem with line numbers.".to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["file_path"],
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to read"
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (1-based)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read"
                }
            }
        }),
    }
}

pub(crate) async fn read_file(args: Value) -> Result<ToolResult> {
    let file_path = args["file_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing 'file_path' parameter"))?;

    let offset = args["offset"].as_u64().unwrap_or(0) as usize;
    let limit = args["limit"].as_u64().unwrap_or(DEFAULT_READ_LIMIT as u64) as usize;

    match tokio::fs::read_to_string(file_path).await {
        Ok(content) => {
            let lines: Vec<&str> = content.lines().collect();
            let start = if offset > 0 { offset.saturating_sub(1) } else { 0 };
            let end = (start + limit).min(lines.len());

            let mut result = String::new();
            for (i, line) in lines[start..end].iter().enumerate() {
                let line_num = start + i + 1;
                // cat -n style: right-aligned line number, tab, content.
                result.push_str(&format!("{:>6}\t{}\n", line_num, line));
            }

            if result.is_empty() {
                result = "(empty file)".to_string();
            }

            Ok(ToolResult {
                content: result,
                is_error: false,
            })
        }
        Err(e) => Ok(ToolResult {
            content: format!("Failed to read {}: {}", file_path, e),
            is_error: true,
        }),
    }
}

// ── Write ───────────────────────────────────────────────────────────────────

pub(crate) fn write_definition() -> ToolDefinition {
    ToolDefinition {
        name: "write".to_string(),
        description: "Write content to a file, creating parent directories if needed.".to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["file_path", "content"],
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            }
        }),
    }
}

pub(crate) async fn write_file(args: Value) -> Result<ToolResult> {
    let file_path = args["file_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing 'file_path' parameter"))?;
    let content = args["content"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing 'content' parameter"))?;

    // Create parent directories if needed.
    if let Some(parent) = std::path::Path::new(file_path).parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            return Ok(ToolResult {
                content: format!("Failed to create directory {}: {}", parent.display(), e),
                is_error: true,
            });
        }
    }

    match tokio::fs::write(file_path, content).await {
        Ok(()) => Ok(ToolResult {
            content: format!("Successfully wrote to {}", file_path),
            is_error: false,
        }),
        Err(e) => Ok(ToolResult {
            content: format!("Failed to write {}: {}", file_path, e),
            is_error: true,
        }),
    }
}

// ── Edit ────────────────────────────────────────────────────────────────────

pub(crate) fn edit_definition() -> ToolDefinition {
    ToolDefinition {
        name: "edit".to_string(),
        description: "Perform exact string replacement in a file.".to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["file_path", "old_string", "new_string"],
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to modify"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact text to replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The replacement text"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences (default: false)"
                }
            }
        }),
    }
}

pub(crate) async fn edit_file(args: Value) -> Result<ToolResult> {
    let file_path = args["file_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing 'file_path' parameter"))?;
    let old_string = args["old_string"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing 'old_string' parameter"))?;
    let new_string = args["new_string"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing 'new_string' parameter"))?;
    let replace_all = args["replace_all"].as_bool().unwrap_or(false);

    if old_string == new_string {
        return Ok(ToolResult {
            content: "old_string and new_string are identical.".to_string(),
            is_error: true,
        });
    }

    let content = match tokio::fs::read_to_string(file_path).await {
        Ok(c) => c,
        Err(e) => {
            return Ok(ToolResult {
                content: format!("Failed to read {}: {}", file_path, e),
                is_error: true,
            });
        }
    };

    let occurrences = content.matches(old_string).count();

    if occurrences == 0 {
        return Ok(ToolResult {
            content: format!(
                "Error: old_string not found in {}. Ensure it matches exactly.",
                file_path
            ),
            is_error: true,
        });
    }

    if !replace_all && occurrences > 1 {
        return Ok(ToolResult {
            content: format!(
                "Error: old_string found {} times in {}. \
                 Provide more context to make it unique, or use replace_all.",
                occurrences, file_path
            ),
            is_error: true,
        });
    }

    let new_content = if replace_all {
        content.replace(old_string, new_string)
    } else {
        content.replacen(old_string, new_string, 1)
    };

    // Write atomically: temp file then rename.
    match atomic_write(file_path, &new_content).await {
        Ok(()) => {
            let replaced = if replace_all { occurrences } else { 1 };
            Ok(ToolResult {
                content: format!(
                    "Replaced {} occurrence(s) in {}",
                    replaced, file_path
                ),
                is_error: false,
            })
        }
        Err(e) => Ok(ToolResult {
            content: format!("Failed to write {}: {}", file_path, e),
            is_error: true,
        }),
    }
}

/// Atomic file write: write to a temp file, then rename.
async fn atomic_write(path: &str, content: &str) -> Result<()> {
    let path = std::path::Path::new(path);
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    let temp_name = format!(
        ".{}.{}.tmp",
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy(),
        std::process::id()
    );
    let temp_path = dir.join(temp_name);

    tokio::fs::write(&temp_path, content).await?;

    if let Err(e) = tokio::fs::rename(&temp_path, path).await {
        // Clean up temp file on rename failure.
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(e.into());
    }

    Ok(())
}

// ── Glob ────────────────────────────────────────────────────────────────────

pub(crate) fn glob_definition() -> ToolDefinition {
    ToolDefinition {
        name: "glob".to_string(),
        description: "Find files matching a glob pattern.".to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern (e.g. '**/*.rs')"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (defaults to cwd)"
                }
            }
        }),
    }
}

pub(crate) async fn glob_files(args: Value) -> Result<ToolResult> {
    let pattern = args["pattern"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing 'pattern' parameter"))?;

    let base_dir = args["path"]
        .as_str()
        .map(|p| std::path::PathBuf::from(p))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    let full_pattern = base_dir.join(pattern).to_string_lossy().to_string();

    const MAX_GLOB_RESULTS: usize = 10_000;

    // Run glob in a blocking task (filesystem traversal).
    let matches = tokio::task::spawn_blocking(move || -> Result<(Vec<String>, bool)> {
        let mut results = Vec::new();
        let entries = glob::glob(&full_pattern)
            .map_err(|e| anyhow::anyhow!("invalid glob pattern: {}", e))?;
        let mut truncated = false;
        for entry in entries {
            if results.len() >= MAX_GLOB_RESULTS {
                truncated = true;
                break;
            }
            match entry {
                Ok(path) => results.push(path.display().to_string()),
                Err(e) => {
                    tracing::warn!("glob error: {}", e);
                }
            }
        }
        Ok((results, truncated))
    })
    .await??;

    let (results, truncated) = matches;
    let mut content = if results.is_empty() {
        "No files found matching the pattern.".to_string()
    } else {
        results.join("\n")
    };
    if truncated {
        content.push_str(&format!(
            "\n\n[truncated: showing first {} results]",
            MAX_GLOB_RESULTS
        ));
    }

    Ok(ToolResult {
        content,
        is_error: false,
    })
}

// ── Grep ────────────────────────────────────────────────────────────────────

pub(crate) fn grep_definition() -> ToolDefinition {
    ToolDefinition {
        name: "grep".to_string(),
        description: "Search file contents using regular expressions.".to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search in"
                },
                "include": {
                    "type": "string",
                    "description": "File pattern filter (e.g. '*.rs')"
                }
            }
        }),
    }
}

pub(crate) async fn grep_files(args: Value) -> Result<ToolResult> {
    let pattern = args["pattern"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing 'pattern' parameter"))?;

    let search_path = args["path"]
        .as_str()
        .unwrap_or(".");
    let include = args["include"].as_str();

    // Shell out to rg (ripgrep) for performance, fall back to grep.
    let result = run_grep(pattern, search_path, include).await;

    match result {
        Ok(output) => {
            let content = if output.is_empty() {
                "No matches found.".to_string()
            } else {
                output
            };
            Ok(ToolResult {
                content,
                is_error: false,
            })
        }
        Err(e) => Ok(ToolResult {
            content: format!("grep failed: {}", e),
            is_error: true,
        }),
    }
}

/// Run ripgrep (rg) with the given pattern and path. Falls back to grep.
async fn run_grep(pattern: &str, path: &str, include: Option<&str>) -> Result<String> {
    // Try ripgrep first.
    let mut cmd_args = vec![
        "--no-heading".to_string(),
        "-n".to_string(),
        pattern.to_string(),
        path.to_string(),
    ];
    if let Some(glob) = include {
        // Insert --glob before pattern.
        cmd_args.insert(0, format!("--glob={}", glob));
    }

    let rg_result = tokio::process::Command::new("rg")
        .args(&cmd_args)
        .output()
        .await;

    match rg_result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            Ok(stdout)
        }
        Err(_) => {
            // Fall back to grep -rn.
            let mut grep_args = vec!["-rn".to_string()];
            if let Some(glob) = include {
                grep_args.push(format!("--include={}", glob));
            }
            grep_args.push(pattern.to_string());
            grep_args.push(path.to_string());

            let output = tokio::process::Command::new("grep")
                .args(&grep_args)
                .output()
                .await
                .map_err(|e| anyhow::anyhow!("neither rg nor grep available: {}", e))?;

            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_read_nonexistent() {
        let args = json!({ "file_path": "/tmp/nonexistent_test_file_xyz" });
        let result = read_file(args).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Failed to read"));
    }

    #[tokio::test]
    async fn test_write_and_read() {
        let tmp = format!("/tmp/openclaw_test_{}", uuid::Uuid::new_v4());
        let write_args = json!({ "file_path": &tmp, "content": "line1\nline2\nline3" });
        let result = write_file(write_args).await.unwrap();
        assert!(!result.is_error);

        let read_args = json!({ "file_path": &tmp });
        let result = read_file(read_args).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("line1"));
        assert!(result.content.contains("     1\t"));

        let _ = tokio::fs::remove_file(&tmp).await;
    }

    #[tokio::test]
    async fn test_edit_not_found() {
        let tmp = format!("/tmp/openclaw_test_{}", uuid::Uuid::new_v4());
        tokio::fs::write(&tmp, "hello world").await.unwrap();

        let args = json!({
            "file_path": &tmp,
            "old_string": "missing text",
            "new_string": "replacement"
        });
        let result = edit_file(args).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found"));

        let _ = tokio::fs::remove_file(&tmp).await;
    }

    #[tokio::test]
    async fn test_edit_success() {
        let tmp = format!("/tmp/openclaw_test_{}", uuid::Uuid::new_v4());
        tokio::fs::write(&tmp, "hello world").await.unwrap();

        let args = json!({
            "file_path": &tmp,
            "old_string": "hello",
            "new_string": "goodbye"
        });
        let result = edit_file(args).await.unwrap();
        assert!(!result.is_error);

        let content = tokio::fs::read_to_string(&tmp).await.unwrap();
        assert_eq!(content, "goodbye world");

        let _ = tokio::fs::remove_file(&tmp).await;
    }

    #[tokio::test]
    async fn test_edit_not_unique() {
        let tmp = format!("/tmp/openclaw_test_{}", uuid::Uuid::new_v4());
        tokio::fs::write(&tmp, "aaa bbb aaa").await.unwrap();

        let args = json!({
            "file_path": &tmp,
            "old_string": "aaa",
            "new_string": "ccc"
        });
        let result = edit_file(args).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("2 times"));

        let _ = tokio::fs::remove_file(&tmp).await;
    }
}
