use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Top-level CLI configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Config {
    /// Provider name: "anthropic", "openai", "ollama".
    #[serde(default = "default_provider")]
    pub provider: String,

    /// Model identifier.
    #[serde(default = "default_model")]
    pub model: String,

    /// API key, supports `${ENV_VAR}` syntax.
    #[serde(default)]
    pub api_key: Option<String>,

    /// Base URL override for the provider API.
    #[serde(default)]
    pub base_url: Option<String>,

    /// Tools configuration.
    #[serde(default)]
    pub tools: ToolsConfig,

    /// Directory for session files.
    #[serde(default)]
    pub sessions_dir: Option<PathBuf>,

    /// System prompt override.
    #[serde(default)]
    pub system_prompt: Option<String>,

    /// Max tokens for LLM responses.
    #[serde(default)]
    pub max_tokens: Option<u32>,

    /// Temperature for LLM responses.
    #[serde(default)]
    pub temperature: Option<f32>,

    /// Extended thinking budget in tokens (Anthropic only).
    /// When set and > 0, enables extended thinking with this token budget.
    #[serde(default)]
    pub thinking_budget: Option<u32>,

    /// Verbose mode — show full request/response details at runtime.
    #[serde(default)]
    pub verbose: bool,
}

/// Tools section of the config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ToolsConfig {
    /// Tool profile: "full", "coding", "minimal".
    #[serde(default = "default_tools_profile")]
    pub profile: String,

    /// Execution security settings.
    #[serde(default)]
    pub exec: ExecConfig,
}

/// Execution security settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ExecConfig {
    /// Security mode: "deny", "allowlist", "full".
    #[serde(default = "default_exec_security")]
    pub security: String,

    /// Glob patterns for allowed commands (when security = "allowlist").
    #[serde(default)]
    pub allowlist: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            model: default_model(),
            api_key: None,
            base_url: None,
            tools: ToolsConfig::default(),
            sessions_dir: None,
            system_prompt: None,
            max_tokens: None,
            temperature: None,
            thinking_budget: None,
            verbose: false,
        }
    }
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            profile: default_tools_profile(),
            exec: ExecConfig::default(),
        }
    }
}

impl Default for ExecConfig {
    fn default() -> Self {
        Self {
            security: default_exec_security(),
            allowlist: Vec::new(),
        }
    }
}

fn default_provider() -> String {
    "anthropic".to_string()
}

fn default_model() -> String {
    "claude-sonnet-4-20250514".to_string()
}

fn default_tools_profile() -> String {
    "full".to_string()
}

fn default_exec_security() -> String {
    "full".to_string()
}

impl Config {
    /// Validate configuration values, returning an error with a helpful message
    /// if any value is out of range.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.provider.is_empty() {
            anyhow::bail!("provider cannot be empty");
        }
        if self.model.is_empty() {
            anyhow::bail!("model cannot be empty");
        }
        if let Some(t) = self.temperature {
            if !(0.0..=2.0).contains(&t) {
                anyhow::bail!("temperature must be between 0.0 and 2.0, got {t}");
            }
        }
        if let Some(t) = self.max_tokens {
            if t == 0 {
                anyhow::bail!("max_tokens must be greater than 0");
            }
        }
        let valid_profiles = ["full", "coding", "minimal", "none"];
        if !valid_profiles.contains(&self.tools.profile.as_str()) {
            anyhow::bail!(
                "unknown tools profile '{}', expected one of: {}",
                self.tools.profile,
                valid_profiles.join(", ")
            );
        }
        let valid_security = ["full", "deny", "allowlist"];
        if !valid_security.contains(&self.tools.exec.security.as_str()) {
            anyhow::bail!(
                "unknown exec security '{}', expected one of: {}",
                self.tools.exec.security,
                valid_security.join(", ")
            );
        }
        Ok(())
    }
}
