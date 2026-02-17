use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use crate::agent::context;
use crate::agent::images;
use crate::agent::session::Session;
use crate::agent::types::{ContentBlock, Message, Role, Usage};
use crate::agent::{self, LlmProvider, ToolExecutor};
use crate::config::Config;
use crate::llm::models;

/// Resolve the history file path (~/.openclaw-cli/history).
fn history_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".openclaw-cli").join("history"))
}

/// Run the interactive REPL loop.
///
/// Uses rustyline for line editing (arrow keys, history, Ctrl+A/E, etc.).
/// History is persisted to ~/.openclaw-cli/history across sessions.
pub(crate) async fn run_repl(
    provider: &dyn LlmProvider,
    session: &mut Session,
    tools: &dyn ToolExecutor,
    config: &mut Config,
) -> Result<()> {
    let mut rl = DefaultEditor::new()?;

    // Cumulative token usage across the entire REPL session
    let mut total_usage = Usage::default();

    // Load history from disk (ignore errors -- file may not exist yet)
    if let Some(ref path) = history_path() {
        let _ = rl.load_history(path);
    }

    eprintln!(
        "\x1b[1mopenclaw-cli\x1b[0m \x1b[2mv{}\x1b[0m  \x1b[2m({}:{})\x1b[0m",
        env!("CARGO_PKG_VERSION"),
        config.provider,
        config.model,
    );
    eprintln!("\x1b[2mType \"exit\" or Ctrl+D to quit. Type /help for commands.\x1b[0m\n");

    loop {
        let readline = rl.readline("\x1b[1;32m>\x1b[0m ");
        match readline {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if trimmed == "exit" || trimmed == "quit" {
                    break;
                }

                // Handle slash commands
                if trimmed.starts_with('/') {
                    rl.add_history_entry(trimmed)?;
                    handle_slash_command(trimmed, session, config, provider, &mut total_usage)
                        .await;
                    continue;
                }

                rl.add_history_entry(trimmed)?;

                // Detect image paths in the user's input and attach them
                let image_paths = images::detect_image_paths(trimmed);
                if image_paths.is_empty() {
                    session.add_user_message(trimmed);
                } else {
                    let mut blocks = vec![ContentBlock::Text {
                        text: trimmed.to_string(),
                    }];
                    for img_path in &image_paths {
                        match images::load_image_from_path(img_path).await {
                            Ok(block) => {
                                eprintln!("\x1b[2m  Attached image: {}\x1b[0m", img_path);
                                blocks.push(block);
                            }
                            Err(e) => {
                                eprintln!(
                                    "\x1b[33m  Warning: could not load image '{}': {}\x1b[0m",
                                    img_path, e
                                );
                            }
                        }
                    }
                    session.push_message(Message {
                        role: Role::User,
                        content: blocks,
                    });
                }

                let result = agent::run_agent_loop(provider, session, tools, config).await;
                match result {
                    Ok(r) => {
                        // Accumulate usage
                        total_usage.input_tokens += r.usage.input_tokens;
                        total_usage.output_tokens += r.usage.output_tokens;
                        total_usage.cache_creation_input_tokens +=
                            r.usage.cache_creation_input_tokens;
                        total_usage.cache_read_input_tokens += r.usage.cache_read_input_tokens;

                        if r.tool_calls > 0 || r.usage.input_tokens > 0 {
                            let cost_str = models::estimate_cost(
                                &config.model,
                                r.usage.input_tokens,
                                r.usage.output_tokens,
                            )
                            .map(|c| format!(" ~{}", models::format_cost(c)))
                            .unwrap_or_default();

                            eprintln!(
                                "\x1b[2m({} tool call{}, {} in / {} out tokens{})\x1b[0m",
                                r.tool_calls,
                                if r.tool_calls == 1 { "" } else { "s" },
                                r.usage.input_tokens,
                                r.usage.output_tokens,
                                cost_str,
                            );
                        }
                    }
                    Err(e) => {
                        eprintln!("\x1b[31mError: {e:#}\x1b[0m");
                    }
                }

                session.save()?;
            }
            Err(ReadlineError::Interrupted) => {
                // Ctrl+C: cancel current input, don't exit
                continue;
            }
            Err(ReadlineError::Eof) => {
                // Ctrl+D: exit
                println!();
                break;
            }
            Err(e) => {
                eprintln!("\x1b[31mInput error: {e}\x1b[0m");
                break;
            }
        }
    }

    // Save history
    if let Some(ref path) = history_path() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let _ = rl.save_history(path);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Model aliases (delegates to llm::models)
// ---------------------------------------------------------------------------

/// Resolve a model alias to the full model identifier.
/// Returns the input unchanged if it is not a known alias.
fn resolve_model_alias(name: &str) -> &str {
    models::resolve_model_alias(name)
}

/// Try to read the current git branch name.
fn current_git_branch() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if branch.is_empty() {
            None
        } else {
            Some(branch)
        }
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Thinking level helpers
// ---------------------------------------------------------------------------

/// Map a thinking level name to a token budget.
fn thinking_level_to_budget(level: &str) -> Option<u32> {
    match level {
        "off" | "none" => None,
        "low" => Some(1024),
        "medium" | "med" => Some(4096),
        "high" => Some(16384),
        _ => None, // unknown level
    }
}

/// Map a thinking budget back to a human-readable level name.
fn budget_to_thinking_level(budget: Option<u32>) -> &'static str {
    match budget {
        None | Some(0) => "off",
        Some(b) if b <= 1024 => "low",
        Some(b) if b <= 4096 => "medium",
        _ => "high",
    }
}

// ---------------------------------------------------------------------------
// Slash command handler (async because /compact calls the LLM)
// ---------------------------------------------------------------------------

async fn handle_slash_command(
    cmd: &str,
    session: &mut Session,
    config: &mut Config,
    provider: &dyn LlmProvider,
    total_usage: &mut Usage,
) {
    // Split command and arguments
    let mut parts = cmd.splitn(2, char::is_whitespace);
    let command = parts.next().unwrap_or("");
    let args = parts.next().unwrap_or("").trim();

    match command {
        // -----------------------------------------------------------------
        "/help" | "/h" => {
            eprintln!(
                "\n\x1b[1m  Commands:\x1b[0m\n\
                 \x20   /help, /h                Show this help\n\
                 \x20   /new [model]             Reset session, optionally switch model\n\
                 \x20   /reset                   Clear all messages in the session\n\
                 \x20   /status                  Show session status and statistics\n\
                 \x20   /compact [instructions]   Compact context (summarize old messages)\n\
                 \x20   /model [name]            Show or switch model (supports aliases)\n\
                 \x20   /usage                   Show token usage and estimated cost\n\
                 \x20   /think [level]           Set thinking mode: off, low, medium, high\n\
                 \x20   /verbose [on|off]        Toggle verbose mode\n\
                 \x20   /info                    Show current provider and model\n\
                 \x20   exit, quit               Exit the REPL\n\
                 \x20   Ctrl+D                   Exit (EOF)\n\
                 \n\x1b[2m  Model aliases: sonnet, opus, haiku, gpt4o, gpt4o-mini\x1b[0m\n"
            );
        }

        // -----------------------------------------------------------------
        "/new" => {
            // Optionally switch model
            if !args.is_empty() {
                let resolved = resolve_model_alias(args);
                config.model = resolved.to_string();
                eprintln!("  Model switched to: {}", config.model);
            }
            session.clear_messages();
            eprintln!("  Session reset. Starting fresh.");
        }

        // -----------------------------------------------------------------
        "/reset" => {
            session.clear_messages();
            eprintln!("  Session reset.");
        }

        // -----------------------------------------------------------------
        "/status" => {
            let msg_count = session.messages().len();
            let est_tokens = context::estimate_messages_tokens(session.messages());
            let ctx_limit = context::context_limit_for_model(&config.model);

            eprintln!("  \x1b[1mSession Status\x1b[0m");
            eprintln!("  Provider:          {}", config.provider);
            eprintln!("  Model:             {}", config.model);
            eprintln!("  Messages:          {}", msg_count);
            eprintln!(
                "  Est. tokens:       {} / {} ({:.1}%)",
                est_tokens,
                ctx_limit,
                if ctx_limit > 0 {
                    (est_tokens as f64 / ctx_limit as f64) * 100.0
                } else {
                    0.0
                }
            );
            eprintln!("  Session file:      {}", session.path().display());
            eprintln!(
                "  Thinking:          {}",
                budget_to_thinking_level(config.thinking_budget)
            );
            eprintln!(
                "  Verbose:           {}",
                if config.verbose { "on" } else { "off" }
            );

            if let Some(branch) = current_git_branch() {
                eprintln!("  Git branch:        {}", branch);
            }
        }

        // -----------------------------------------------------------------
        "/compact" => {
            eprintln!("  Compacting context...");
            let system_prompt = if args.is_empty() {
                "You are an AI assistant.".to_string()
            } else {
                format!("You are an AI assistant. Additional instructions: {}", args)
            };

            let before_count = session.messages().len();
            let before_tokens = context::estimate_messages_tokens(session.messages());

            match context::compact_messages(
                provider,
                session.messages(),
                &config.model,
                &system_prompt,
            )
            .await
            {
                Ok(compacted) => {
                    let after_count = compacted.len();
                    let after_tokens = context::estimate_messages_tokens(&compacted);
                    session.replace_messages(compacted);
                    eprintln!(
                        "  Compacted: {} -> {} messages, ~{} -> ~{} tokens",
                        before_count, after_count, before_tokens, after_tokens,
                    );
                    if let Err(e) = session.save() {
                        eprintln!("\x1b[31m  Error saving session: {e}\x1b[0m");
                    }
                }
                Err(e) => {
                    eprintln!("\x1b[31m  Compaction failed: {e:#}\x1b[0m");
                }
            }
        }

        // -----------------------------------------------------------------
        "/model" => {
            if args.is_empty() {
                eprintln!("  Current model: {}", config.model);
                eprintln!("\x1b[2m  Aliases: sonnet, opus, haiku, gpt4o, gpt4o-mini\x1b[0m");
            } else {
                let resolved = resolve_model_alias(args);
                config.model = resolved.to_string();
                eprintln!("  Model switched to: {}", config.model);
            }
        }

        // -----------------------------------------------------------------
        "/usage" => {
            eprintln!("  \x1b[1mSession Usage\x1b[0m");
            eprintln!("  Model:             {}", config.model);
            eprintln!("  Input tokens:      {}", total_usage.input_tokens);
            eprintln!("  Output tokens:     {}", total_usage.output_tokens);
            if total_usage.cache_creation_input_tokens > 0
                || total_usage.cache_read_input_tokens > 0
            {
                eprintln!(
                    "  Cache write:       {}",
                    total_usage.cache_creation_input_tokens
                );
                eprintln!(
                    "  Cache read:        {}",
                    total_usage.cache_read_input_tokens
                );
            }
            eprintln!(
                "  Total tokens:      {}",
                total_usage.input_tokens + total_usage.output_tokens
            );
            match models::estimate_cost(
                &config.model,
                total_usage.input_tokens,
                total_usage.output_tokens,
            ) {
                Some(cost) => {
                    eprintln!("  Est. cost:         {}", models::format_cost(cost));
                    if let Some(info) = models::get_model_info(&config.model) {
                        eprintln!(
                            "\x1b[2m  (${}/M in, ${}/M out)\x1b[0m",
                            info.input_price_per_mtok, info.output_price_per_mtok
                        );
                    }
                }
                None => {
                    eprintln!("  Est. cost:         (unknown model pricing)");
                }
            }
        }

        // -----------------------------------------------------------------
        "/think" => {
            if args.is_empty() {
                let level = budget_to_thinking_level(config.thinking_budget);
                eprintln!("  Thinking mode: {}", level);
                if let Some(budget) = config.thinking_budget {
                    eprintln!("  Budget: {} tokens", budget);
                }
            } else {
                let level = args.to_lowercase();
                match level.as_str() {
                    "off" | "none" | "low" | "medium" | "med" | "high" => {
                        config.thinking_budget = thinking_level_to_budget(&level);
                        let display = budget_to_thinking_level(config.thinking_budget);
                        eprintln!("  Thinking mode set to: {}", display);
                        if let Some(budget) = config.thinking_budget {
                            eprintln!("  Budget: {} tokens", budget);
                        }
                    }
                    _ => {
                        eprintln!(
                            "\x1b[33m  Unknown thinking level: '{}'. Use: off, low, medium, high\x1b[0m",
                            args
                        );
                    }
                }
            }
        }

        // -----------------------------------------------------------------
        "/verbose" => {
            if args.is_empty() {
                config.verbose = !config.verbose;
                eprintln!(
                    "  Verbose mode: {}",
                    if config.verbose { "on" } else { "off" }
                );
            } else {
                match args.to_lowercase().as_str() {
                    "on" | "true" | "1" | "yes" => {
                        config.verbose = true;
                        eprintln!("  Verbose mode: on");
                    }
                    "off" | "false" | "0" | "no" => {
                        config.verbose = false;
                        eprintln!("  Verbose mode: off");
                    }
                    _ => {
                        eprintln!("\x1b[33m  Unknown value: '{}'. Use: on, off\x1b[0m", args);
                    }
                }
            }
        }

        // -----------------------------------------------------------------
        "/info" => {
            eprintln!("  Provider: {}", config.provider);
            eprintln!("  Model: {}", config.model);
            if let Some(ref url) = config.base_url {
                eprintln!("  Base URL: {}", url);
            }
            if let Some(max_tokens) = config.max_tokens {
                eprintln!("  Max tokens: {}", max_tokens);
            }
        }

        // -----------------------------------------------------------------
        _ => {
            eprintln!(
                "\x1b[33mUnknown command: {}. Type /help for available commands.\x1b[0m",
                command
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_model_alias_sonnet() {
        assert_eq!(resolve_model_alias("sonnet"), "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_resolve_model_alias_opus() {
        assert_eq!(resolve_model_alias("opus"), "claude-opus-4-20250514");
    }

    #[test]
    fn test_resolve_model_alias_haiku() {
        assert_eq!(resolve_model_alias("haiku"), "claude-haiku-3-20250307");
    }

    #[test]
    fn test_resolve_model_alias_gpt4o() {
        assert_eq!(resolve_model_alias("gpt4o"), "gpt-4o");
    }

    #[test]
    fn test_resolve_model_alias_gpt4o_mini() {
        assert_eq!(resolve_model_alias("gpt4o-mini"), "gpt-4o-mini");
    }

    #[test]
    fn test_resolve_model_alias_passthrough() {
        assert_eq!(resolve_model_alias("custom-model-v1"), "custom-model-v1");
    }

    #[test]
    fn test_thinking_level_to_budget() {
        assert_eq!(thinking_level_to_budget("off"), None);
        assert_eq!(thinking_level_to_budget("none"), None);
        assert_eq!(thinking_level_to_budget("low"), Some(1024));
        assert_eq!(thinking_level_to_budget("medium"), Some(4096));
        assert_eq!(thinking_level_to_budget("med"), Some(4096));
        assert_eq!(thinking_level_to_budget("high"), Some(16384));
        assert_eq!(thinking_level_to_budget("invalid"), None);
    }

    #[test]
    fn test_budget_to_thinking_level() {
        assert_eq!(budget_to_thinking_level(None), "off");
        assert_eq!(budget_to_thinking_level(Some(0)), "off");
        assert_eq!(budget_to_thinking_level(Some(512)), "low");
        assert_eq!(budget_to_thinking_level(Some(1024)), "low");
        assert_eq!(budget_to_thinking_level(Some(2048)), "medium");
        assert_eq!(budget_to_thinking_level(Some(4096)), "medium");
        assert_eq!(budget_to_thinking_level(Some(8192)), "high");
        assert_eq!(budget_to_thinking_level(Some(16384)), "high");
    }
}
