//! System prompt assembly.
//!
//! Builds the full system prompt from multiple sources: config, tool
//! descriptions, project instructions (CLAUDE.md), persistent memory,
//! and workspace metadata.

use std::path::Path;

use crate::agent::ToolExecutor;
use crate::config::Config;

use super::memory;

/// Default system prompt used when no custom prompt is configured.
const DEFAULT_SYSTEM_PROMPT: &str =
    "You are an AI assistant with access to tools for reading files, writing code, and running commands.";

/// Build the full system prompt by assembling sections from configuration,
/// tool definitions, project instructions, memory, and workspace metadata.
pub(crate) fn build_system_prompt(config: &Config, tools: &dyn ToolExecutor) -> String {
    let mut parts: Vec<String> = Vec::new();

    // 1. Base prompt (config override or default).
    let base = config
        .system_prompt
        .as_deref()
        .unwrap_or(DEFAULT_SYSTEM_PROMPT);
    parts.push(base.to_string());

    // 2. Tool descriptions section.
    let defs = tools.definitions();
    if !defs.is_empty() {
        let mut section = String::from("\n\n## Available Tools\n");
        for def in &defs {
            section.push_str(&format!("\n- **{}**: {}", def.name, def.description));
        }
        parts.push(section);
    }

    // 3. Project instructions (CLAUDE.md in current working directory).
    if let Ok(cwd) = std::env::current_dir() {
        let claude_md = cwd.join("CLAUDE.md");
        if let Ok(content) = std::fs::read_to_string(&claude_md) {
            if !content.trim().is_empty() {
                parts.push(format!("\n\n## Project Instructions\n\n{}", content.trim()));
            }
        }
    }

    // 4. Persistent memory (MEMORY.md from ~/.openclaw-cli/memory/).
    match memory::read_memory_file("MEMORY.md") {
        Ok(Some(content)) if !content.trim().is_empty() => {
            parts.push(format!("\n\n## Memory\n\n{}", content.trim()));
        }
        _ => {}
    }

    // 5. Workspace metadata.
    let mut meta = String::from("\n\n## Workspace\n");
    if let Ok(cwd) = std::env::current_dir() {
        meta.push_str(&format!("\n- Working directory: {}", cwd.display()));
    }
    meta.push_str(&format!(
        "\n- Date: {}",
        chrono::Local::now().format("%Y-%m-%d"),
    ));
    if let Some(branch) = detect_git_branch() {
        meta.push_str(&format!("\n- Git branch: {}", branch));
    }
    parts.push(meta);

    parts.join("")
}

/// Estimate the token cost of tool definitions for context window management.
///
/// Uses the same 4-chars-per-token heuristic as [`super::context::estimate_tokens`].
pub(crate) fn estimate_tool_definitions_tokens(tools: &dyn ToolExecutor) -> usize {
    let defs = tools.definitions();
    let mut chars = 0;
    for def in &defs {
        chars += def.name.len() + def.description.len() + def.input_schema.to_string().len();
    }
    (chars + 3) / 4
}

// ---------------------------------------------------------------------------
// Git branch detection
// ---------------------------------------------------------------------------

/// Try to detect the current git branch by reading `.git/HEAD`.
///
/// Walks up from the current working directory until it finds a `.git`
/// directory. Returns `None` if not in a git repo or if HEAD cannot be read.
fn detect_git_branch() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let mut dir = cwd.as_path();
    loop {
        let git_head = dir.join(".git").join("HEAD");
        if git_head.exists() {
            return parse_git_head(&git_head);
        }
        dir = dir.parent()?;
    }
}

/// Parse a `.git/HEAD` file to extract the branch name.
///
/// The file typically contains either:
///   - `ref: refs/heads/<branch-name>\n`   (normal branch)
///   - A raw commit SHA (detached HEAD)
fn parse_git_head(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let trimmed = content.trim();
    if let Some(ref_path) = trimmed.strip_prefix("ref: ") {
        // Extract branch name from e.g. "refs/heads/main"
        ref_path
            .strip_prefix("refs/heads/")
            .map(|b| b.to_string())
            .or_else(|| Some(ref_path.to_string()))
    } else {
        // Detached HEAD -- return a short SHA prefix.
        Some(trimmed.chars().take(12).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::{ToolDefinition, ToolResult};
    use serde_json::json;

    // -- Minimal mock tool executor for testing ---

    struct DummyTools;

    #[async_trait::async_trait]
    impl ToolExecutor for DummyTools {
        async fn execute(
            &self,
            _name: &str,
            _args: serde_json::Value,
        ) -> anyhow::Result<ToolResult> {
            Ok(ToolResult {
                content: String::new(),
                is_error: false,
            })
        }

        fn definitions(&self) -> Vec<ToolDefinition> {
            vec![
                ToolDefinition {
                    name: "bash".to_string(),
                    description: "Run shell commands".to_string(),
                    input_schema: json!({"type": "object"}),
                },
                ToolDefinition {
                    name: "read".to_string(),
                    description: "Read file contents".to_string(),
                    input_schema: json!({"type": "object"}),
                },
            ]
        }
    }

    struct NoTools;

    #[async_trait::async_trait]
    impl ToolExecutor for NoTools {
        async fn execute(
            &self,
            _name: &str,
            _args: serde_json::Value,
        ) -> anyhow::Result<ToolResult> {
            unreachable!()
        }
        fn definitions(&self) -> Vec<ToolDefinition> {
            Vec::new()
        }
    }

    // -- Tests ---

    #[test]
    fn test_build_prompt_includes_default_base() {
        let config = Config::default();
        let prompt = build_system_prompt(&config, &DummyTools);
        assert!(prompt.contains(DEFAULT_SYSTEM_PROMPT));
    }

    #[test]
    fn test_build_prompt_custom_base() {
        let config = Config {
            system_prompt: Some("You are a coding assistant.".to_string()),
            ..Config::default()
        };
        let prompt = build_system_prompt(&config, &DummyTools);
        assert!(prompt.starts_with("You are a coding assistant."));
        assert!(!prompt.contains(DEFAULT_SYSTEM_PROMPT));
    }

    #[test]
    fn test_build_prompt_includes_tool_descriptions() {
        let config = Config::default();
        let prompt = build_system_prompt(&config, &DummyTools);
        assert!(prompt.contains("## Available Tools"));
        assert!(prompt.contains("**bash**"));
        assert!(prompt.contains("Run shell commands"));
        assert!(prompt.contains("**read**"));
    }

    #[test]
    fn test_build_prompt_no_tools_section_when_empty() {
        let config = Config::default();
        let prompt = build_system_prompt(&config, &NoTools);
        assert!(!prompt.contains("## Available Tools"));
    }

    #[test]
    fn test_build_prompt_includes_workspace_metadata() {
        let config = Config::default();
        let prompt = build_system_prompt(&config, &DummyTools);
        assert!(prompt.contains("## Workspace"));
        assert!(prompt.contains("Working directory:"));
        assert!(prompt.contains("Date:"));
    }

    #[test]
    fn test_parse_git_head_branch() {
        let dir = std::env::temp_dir().join("test_parse_git_head_prompt");
        let _ = std::fs::create_dir_all(&dir);
        let head_path = dir.join("HEAD");
        std::fs::write(&head_path, "ref: refs/heads/feature-branch\n").unwrap();
        assert_eq!(
            parse_git_head(&head_path),
            Some("feature-branch".to_string()),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_git_head_detached() {
        let dir = std::env::temp_dir().join("test_parse_git_head_detached_prompt");
        let _ = std::fs::create_dir_all(&dir);
        let head_path = dir.join("HEAD");
        std::fs::write(&head_path, "abc123def456789\n").unwrap();
        let result = parse_git_head(&head_path);
        assert_eq!(result, Some("abc123def456".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_git_head_nonexistent() {
        let path = Path::new("/tmp/nonexistent_git_head_test_xyz");
        assert_eq!(parse_git_head(path), None);
    }

    #[test]
    fn test_estimate_tool_definitions_tokens() {
        let tokens = estimate_tool_definitions_tokens(&DummyTools);
        assert!(tokens > 0);
    }

    #[test]
    fn test_estimate_tool_definitions_tokens_empty() {
        let tokens = estimate_tool_definitions_tokens(&NoTools);
        assert_eq!(tokens, 0);
    }
}
