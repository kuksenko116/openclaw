mod agent;
mod cli;
mod config;
mod llm;
mod tools;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::agent::images;
use crate::agent::session::{load_or_create_session, resolve_sessions_dir};
use crate::agent::types::{ContentBlock, Message, Role};

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "openclaw-cli",
    about = "AI agent CLI — chat with LLMs and use tools",
    version = env!("CARGO_PKG_VERSION"),
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Send a prompt (single turn or interactive)
    Chat {
        /// The prompt to send
        prompt: Option<String>,

        /// Session name to resume
        #[arg(short, long)]
        session: Option<String>,

        /// Interactive REPL mode
        #[arg(short, long)]
        interactive: bool,

        /// Override the model (e.g. claude-sonnet-4-20250514, gpt-4o)
        #[arg(short, long)]
        model: Option<String>,

        /// Override the provider (anthropic, openai, ollama)
        #[arg(short, long)]
        provider: Option<String>,

        /// Override the API key
        #[arg(long)]
        api_key: Option<String>,

        /// Override the system prompt
        #[arg(long)]
        system_prompt: Option<String>,

        /// Override the base URL for the provider API
        #[arg(long)]
        base_url: Option<String>,

        /// Disable tool use
        #[arg(long)]
        no_tools: bool,

        /// Max tokens for the response
        #[arg(long)]
        max_tokens: Option<u32>,
    },
    /// Session management
    Sessions {
        #[command(subcommand)]
        action: SessionsAction,
    },
    /// Provider management
    Providers {
        #[command(subcommand)]
        action: ProvidersAction,
    },
}

#[derive(Subcommand)]
enum SessionsAction {
    /// List saved sessions
    List,
}

#[derive(Subcommand)]
enum ProvidersAction {
    /// List configured providers
    List,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    // Ctrl+C: reset terminal style and exit cleanly.
    // The first Ctrl+C is handled gracefully (prints newline, resets style).
    // A second Ctrl+C forces immediate exit for stuck processes.
    let ctrl_c_count = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0));
    let ctrl_c_count2 = ctrl_c_count.clone();
    ctrlc::set_handler(move || {
        let count = ctrl_c_count2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        eprint!("\x1b[0m");
        if count >= 1 {
            // Second Ctrl+C: force exit
            std::process::exit(130);
        }
        eprintln!("\nInterrupted. Press Ctrl+C again to force quit.");
    })
    .ok();
    // Suppress unused variable warning (used to keep Arc alive)
    let _ = &ctrl_c_count;

    let cli = Cli::parse();
    let mut config = config::load_config()?;

    match cli.command {
        Commands::Chat {
            prompt,
            session,
            interactive,
            model,
            provider,
            api_key,
            system_prompt,
            base_url,
            no_tools,
            max_tokens,
        } => {
            // Apply CLI overrides to config
            if let Some(m) = model {
                config.model = m;
            }
            if let Some(p) = provider {
                config.provider = p;
            }
            if let Some(k) = api_key {
                config.api_key = Some(k);
            }
            if let Some(s) = system_prompt {
                config.system_prompt = Some(s);
            }
            if let Some(u) = base_url {
                config.base_url = Some(u);
            }
            if let Some(t) = max_tokens {
                config.max_tokens = Some(t);
            }

            // Resolve model aliases (e.g. "sonnet" -> "claude-sonnet-4-20250514")
            config.model = llm::models::resolve_model_alias(&config.model).to_string();

            // Resolve API key from provider-specific env vars if not set
            if config.api_key.as_deref().unwrap_or("").is_empty() {
                let env_var = match config.provider.as_str() {
                    "anthropic" => "ANTHROPIC_API_KEY",
                    "openai" | "openrouter" | "together" => "OPENAI_API_KEY",
                    "google" | "gemini" => "GOOGLE_API_KEY",
                    _ => "OPENAI_API_KEY", // OpenAI-compatible fallback
                };
                if let Ok(val) = std::env::var(env_var) {
                    if !val.is_empty() {
                        config.api_key = Some(val);
                    }
                }
            }

            // Validate API key for providers that need one
            if config.provider != "ollama" {
                let key = config.api_key.as_deref().unwrap_or("");
                if key.is_empty() {
                    anyhow::bail!(
                        "No API key configured for provider '{}'. \
                         Set {} environment variable, \
                         use --api-key, or add api_key to ~/.openclaw-cli/config.yaml",
                        config.provider,
                        match config.provider.as_str() {
                            "anthropic" => "ANTHROPIC_API_KEY",
                            "openai" | "openrouter" | "together" => "OPENAI_API_KEY",
                            "google" | "gemini" => "GOOGLE_API_KEY",
                            _ => "OPENAI_API_KEY",
                        }
                    );
                }
            }

            config.validate()?;
            cmd_chat(config, prompt, session, interactive, no_tools).await
        }
        Commands::Sessions { action } => match action {
            SessionsAction::List => cmd_sessions_list(&config),
        },
        Commands::Providers { action } => match action {
            ProvidersAction::List => cmd_providers_list(&config),
        },
    }
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("openclaw_cli=info".parse().unwrap()),
        )
        .with_target(false)
        .init();
}

// ---------------------------------------------------------------------------
// Subcommands
// ---------------------------------------------------------------------------

async fn cmd_chat(
    mut config: config::Config,
    prompt: Option<String>,
    session_name: Option<String>,
    interactive: bool,
    no_tools: bool,
) -> Result<()> {
    let sessions_dir = resolve_sessions_dir(config.sessions_dir.as_deref())?;

    let name = session_name.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let mut session = load_or_create_session(&sessions_dir, &name)?;

    let provider = llm::create_provider(&config)?;

    let tools_profile = if no_tools {
        // Empty registry with no tools
        tools::policy::ToolPolicy::from_profile("none")
    } else {
        tools::policy::ToolPolicy::from_profile(&config.tools.profile)
    };
    let tools = tools::ToolRegistry::new(tools_profile, config.tools.exec.clone());

    if interactive {
        cli::run_repl(provider.as_ref(), &mut session, &tools, &mut config).await?;
    } else {
        let prompt_text = prompt
            .context("prompt is required in non-interactive mode (use -i for interactive)")?;

        // Detect image paths in the prompt and attach them
        let image_paths = images::detect_image_paths(&prompt_text);
        if image_paths.is_empty() {
            session.add_user_message(&prompt_text);
        } else {
            let mut blocks = vec![ContentBlock::Text {
                text: prompt_text.clone(),
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

        let result =
            agent::run_agent_loop(provider.as_ref(), &mut session, &tools, &config).await?;

        // Print usage stats
        if result.tool_calls > 0 || result.usage.input_tokens > 0 {
            let cost_str = llm::models::estimate_cost(
                &config.model,
                result.usage.input_tokens,
                result.usage.output_tokens,
            )
            .map(|c| format!(" ~{}", llm::models::format_cost(c)))
            .unwrap_or_default();

            eprintln!(
                "\n\x1b[2m({} tool call{}, {} in / {} out tokens{})\x1b[0m",
                result.tool_calls,
                if result.tool_calls == 1 { "" } else { "s" },
                result.usage.input_tokens,
                result.usage.output_tokens,
                cost_str,
            );
        }

        session.save()?;
    }

    Ok(())
}

fn cmd_sessions_list(config: &config::Config) -> Result<()> {
    let sessions_dir = resolve_sessions_dir(config.sessions_dir.as_deref())?;

    if !sessions_dir.exists() {
        println!("No sessions found.");
        return Ok(());
    }

    let mut entries: Vec<(String, std::time::SystemTime)> = Vec::new();
    for entry in std::fs::read_dir(&sessions_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            let modified = entry.metadata()?.modified()?;
            entries.push((name, modified));
        }
    }

    entries.sort_by(|a, b| b.1.cmp(&a.1));

    if entries.is_empty() {
        println!("No sessions found.");
    } else {
        for (name, _modified) in &entries {
            println!("{}", name);
        }
    }

    Ok(())
}

fn cmd_providers_list(config: &config::Config) -> Result<()> {
    println!("Configured provider: {}", config.provider);
    println!("Model: {}", config.model);
    if let Some(ref url) = config.base_url {
        println!("Base URL: {}", url);
    }
    let has_key = config.api_key.as_ref().is_some_and(|k| !k.is_empty());
    println!("API key: {}", if has_key { "set" } else { "not set" });
    Ok(())
}
