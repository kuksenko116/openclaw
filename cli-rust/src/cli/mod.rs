use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use crate::agent::{self, LlmProvider, ToolExecutor};
use crate::agent::session::Session;
use crate::config::Config;

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
    config: &Config,
) -> Result<()> {
    let mut rl = DefaultEditor::new()?;

    // Load history from disk (ignore errors — file may not exist yet)
    if let Some(ref path) = history_path() {
        let _ = rl.load_history(path);
    }

    eprintln!(
        "\x1b[1mopenclaw-cli\x1b[0m \x1b[2mv{}\x1b[0m  \x1b[2m({}:{})\x1b[0m",
        env!("CARGO_PKG_VERSION"),
        config.provider,
        config.model,
    );
    eprintln!("\x1b[2mType \"exit\" or Ctrl+D to quit.\x1b[0m\n");

    loop {
        let readline = rl.readline("\x1b[1;32m❯\x1b[0m ");
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
                    handle_slash_command(trimmed, config);
                    continue;
                }

                rl.add_history_entry(trimmed)?;

                session.add_user_message(trimmed);

                let result = agent::run_agent_loop(provider, session, tools, config).await;
                match result {
                    Ok(r) => {
                        if r.tool_calls > 0 || r.usage.input_tokens > 0 {
                            eprintln!(
                                "\x1b[2m({} tool call{}, {} in / {} out tokens)\x1b[0m",
                                r.tool_calls,
                                if r.tool_calls == 1 { "" } else { "s" },
                                r.usage.input_tokens,
                                r.usage.output_tokens,
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

fn handle_slash_command(cmd: &str, config: &Config) {
    match cmd {
        "/help" | "/h" => {
            eprintln!(
                "\n  Commands:\n\
                 \x20   /help, /h      Show this help\n\
                 \x20   /info          Show current provider and model\n\
                 \x20   exit, quit     Exit the REPL\n\
                 \x20   Ctrl+D         Exit (EOF)\n"
            );
        }
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
        _ => {
            eprintln!(
                "\x1b[33mUnknown command: {}. Type /help for available commands.\x1b[0m",
                cmd
            );
        }
    }
}
