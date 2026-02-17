pub(crate) mod schema;

pub(crate) use schema::Config;

use std::path::PathBuf;

use anyhow::{Context, Result};

/// Load configuration, checking (in order):
/// 1. `$OPENCLAW_CLI_CONFIG` env var
/// 2. `~/.openclaw-cli/config.yaml`
/// 3. Built-in defaults
pub(crate) fn load_config() -> Result<Config> {
    let path = resolve_config_path();

    let config = match path {
        Some(p) if p.exists() => {
            tracing::info!(path = %p.display(), "loading config");
            let raw = std::fs::read_to_string(&p)
                .with_context(|| format!("reading config from {}", p.display()))?;
            let mut cfg: Config = serde_yaml::from_str(&raw)
                .with_context(|| format!("parsing config from {}", p.display()))?;
            resolve_env_vars(&mut cfg);
            cfg
        }
        _ => {
            tracing::debug!("no config file found, using defaults");
            // Even with defaults, try to pick up an API key from the environment
            let mut cfg = Config {
                api_key: Some("${ANTHROPIC_API_KEY}".to_string()),
                ..Config::default()
            };
            resolve_env_vars(&mut cfg);
            cfg
        }
    };

    Ok(config)
}

/// Determine the config file path.
fn resolve_config_path() -> Option<PathBuf> {
    // Check env var first
    if let Ok(path) = std::env::var("OPENCLAW_CLI_CONFIG") {
        let p = PathBuf::from(path);
        if !p.as_os_str().is_empty() {
            return Some(p);
        }
    }

    // Default location
    dirs::home_dir().map(|h| h.join(".openclaw-cli").join("config.yaml"))
}

/// Resolve `${ENV_VAR}` references in the api_key field.
fn resolve_env_vars(config: &mut Config) {
    if let Some(ref key) = config.api_key {
        config.api_key = Some(substitute_env_vars(key));
    }
}

/// Substitute `${VAR}` patterns with environment variable values.
/// Returns the original string unchanged if the variable is not set.
fn substitute_env_vars(input: &str) -> String {
    let mut result = input.to_string();
    // Simple pattern: the whole value is ${VAR}
    if let Some(inner) = extract_env_ref(&result) {
        if let Ok(val) = std::env::var(inner) {
            return val;
        }
    }
    // Inline ${VAR} substitution within a larger string
    while let Some(start) = result.find("${") {
        let rest = &result[start + 2..];
        if let Some(end) = rest.find('}') {
            let var_name = &rest[..end];
            let replacement = std::env::var(var_name).unwrap_or_default();
            result = format!("{}{}{}", &result[..start], replacement, &rest[end + 1..]);
        } else {
            break;
        }
    }
    result
}

/// If the entire string is `${VAR}`, return the variable name.
fn extract_env_ref(s: &str) -> Option<&str> {
    let trimmed = s.trim();
    if trimmed.starts_with("${") && trimmed.ends_with('}') && trimmed.len() > 3 {
        let inner = &trimmed[2..trimmed.len() - 1];
        // Ensure there are no nested braces
        if !inner.contains('{') && !inner.contains('}') {
            return Some(inner);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        let config = Config::default();
        assert_eq!(config.provider, "anthropic");
        assert_eq!(config.model, "claude-sonnet-4-20250514");
        assert!(config.api_key.is_none());
        assert!(config.base_url.is_none());
        assert_eq!(config.tools.profile, "full");
        assert_eq!(config.tools.exec.security, "full");
    }

    #[test]
    fn test_validate_valid() {
        let config = Config::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_empty_provider() {
        let mut config = Config::default();
        config.provider = String::new();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_empty_model() {
        let mut config = Config::default();
        config.model = String::new();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_temperature_out_of_range() {
        let mut config = Config::default();
        config.temperature = Some(3.0);
        assert!(config.validate().is_err());

        config.temperature = Some(-1.0);
        assert!(config.validate().is_err());

        config.temperature = Some(1.0);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_max_tokens_zero() {
        let mut config = Config::default();
        config.max_tokens = Some(0);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_bad_tools_profile() {
        let mut config = Config::default();
        config.tools.profile = "invalid".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_bad_exec_security() {
        let mut config = Config::default();
        config.tools.exec.security = "invalid".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_extract_env_ref() {
        assert_eq!(extract_env_ref("${HOME}"), Some("HOME"));
        assert_eq!(extract_env_ref("plain"), None);
        assert_eq!(extract_env_ref("${A}extra"), None);
        assert_eq!(extract_env_ref("${}"), None);
    }

    #[test]
    fn test_substitute_env_vars_passthrough() {
        let result = substitute_env_vars("plain-key-123");
        assert_eq!(result, "plain-key-123");
    }

    #[test]
    fn test_substitute_env_vars_with_home() {
        // HOME is always set in test environment
        let result = substitute_env_vars("${HOME}");
        assert!(!result.is_empty());
        assert!(!result.contains("${"));
    }

    #[test]
    fn test_parse_yaml_config() {
        let yaml = r#"
provider: openai
model: gpt-4
max_tokens: 2048
tools:
  profile: coding
  exec:
    security: deny
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.provider, "openai");
        assert_eq!(config.model, "gpt-4");
        assert_eq!(config.max_tokens, Some(2048));
        assert_eq!(config.tools.profile, "coding");
        assert_eq!(config.tools.exec.security, "deny");
    }

    #[test]
    fn test_parse_empty_yaml() {
        let config: Config = serde_yaml::from_str("{}").unwrap();
        assert_eq!(config.provider, "anthropic");
        assert_eq!(config.model, "claude-sonnet-4-20250514");
    }
}
