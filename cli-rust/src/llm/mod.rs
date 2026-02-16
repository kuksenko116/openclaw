// LLM provider implementations.
//
// Dependencies needed in Cargo.toml:
//   futures-util = "0.3"
//   bytes = "1"
//
// Each provider implements the LlmProvider trait defined in agent/mod.rs.

pub(crate) mod anthropic;
pub(crate) mod ollama;
pub(crate) mod openai;
pub(crate) mod streaming;

use anyhow::Result;
use crate::agent::LlmProvider;
use crate::config::Config;

/// Create a provider from the application configuration.
pub(crate) fn create_provider(config: &Config) -> Result<Box<dyn LlmProvider>> {
    let api_key = config.api_key.clone().unwrap_or_default();
    let base_url = config.base_url.clone().unwrap_or_default();

    match config.provider.as_str() {
        "anthropic" => Ok(Box::new(anthropic::AnthropicProvider::new(
            &api_key, &base_url,
        )?)),
        "openai" | "openrouter" | "together" | "google" | "gemini" => {
            Ok(Box::new(openai::OpenAiProvider::new(&api_key, &base_url)?))
        }
        "ollama" => Ok(Box::new(ollama::OllamaProvider::new(&base_url)?)),
        unknown => {
            // Treat unknown providers as OpenAI-compatible (most are).
            tracing::warn!(
                "Unknown provider '{}', using OpenAI-compatible client",
                unknown
            );
            Ok(Box::new(openai::OpenAiProvider::new(&api_key, &base_url)?))
        }
    }
}
