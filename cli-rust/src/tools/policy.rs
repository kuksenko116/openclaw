// Tool policy: controls which tools are available and which commands are allowed.
//
// Three profiles:
// - "full":    all tools allowed
// - "coding":  filesystem + bash tools only
// - "minimal": read-only tools only

/// Tool policy determining which tools are available.
#[derive(Debug, Clone)]
pub(crate) struct ToolPolicy {
    profile: String,
}

impl ToolPolicy {
    /// Create a policy from a profile name.
    pub fn from_profile(profile: &str) -> Self {
        Self {
            profile: profile.to_string(),
        }
    }

    /// Check if a tool is allowed under the current profile.
    pub fn is_tool_allowed(&self, name: &str) -> bool {
        match self.profile.as_str() {
            "full" => true,
            "coding" => matches!(
                name,
                "bash" | "read" | "write" | "edit" | "glob" | "grep" | "web_fetch"
            ),
            "minimal" => matches!(name, "read" | "glob" | "grep"),
            "none" => false,
            _ => true, // Unknown profile defaults to full.
        }
    }
}

/// Check if a command is allowed against an allowlist.
///
/// The security parameter controls the behavior:
/// - "full": always allowed
/// - "deny": never allowed
/// - "allowlist": check the first command token against the allowlist.
///   Commands containing shell chaining operators (;, &&, ||, |, $( ) are
///   rejected unless the pattern explicitly ends with a space (prefix mode).
pub(crate) fn is_command_allowed(command: &str, allowlist: &[String], security: &str) -> bool {
    match security {
        "full" => true,
        "deny" => false,
        "allowlist" => {
            // Reject commands with shell chaining/injection metacharacters.
            let has_chain = command.contains(';')
                || command.contains("&&")
                || command.contains('|')  // catches | and ||
                || command.contains('`')
                || command.contains("$(")
                || command.contains('\n')
                || command.contains('\r')
                || command.contains("<<")
                || command.contains("<(")
                || command.contains(">(");

            let trimmed = command.trim();
            // Extract the first token (command name)
            let first_token = trimmed.split_whitespace().next().unwrap_or("");
            // Get just the binary name (basename) in case of full path
            let bin_name = std::path::Path::new(first_token)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(first_token)
                .to_lowercase();

            allowlist.iter().any(|pattern| {
                let p = pattern.to_lowercase();
                if p.ends_with(' ') {
                    // Prefix pattern (e.g. "git "): full command must start with it
                    // and must not contain shell chaining
                    trimmed.to_lowercase().starts_with(&p) && !has_chain
                } else {
                    // Binary name pattern (e.g. "git"): match the command name only
                    // and must not contain shell chaining
                    bin_name == p && !has_chain
                }
            })
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_full_allows_everything() {
        let policy = ToolPolicy::from_profile("full");
        assert!(policy.is_tool_allowed("bash"));
        assert!(policy.is_tool_allowed("read"));
        assert!(policy.is_tool_allowed("write"));
        assert!(policy.is_tool_allowed("anything"));
    }

    #[test]
    fn test_coding_profile() {
        let policy = ToolPolicy::from_profile("coding");
        assert!(policy.is_tool_allowed("bash"));
        assert!(policy.is_tool_allowed("read"));
        assert!(policy.is_tool_allowed("write"));
        assert!(policy.is_tool_allowed("edit"));
        assert!(policy.is_tool_allowed("glob"));
        assert!(policy.is_tool_allowed("grep"));
        assert!(policy.is_tool_allowed("web_fetch"));
        assert!(!policy.is_tool_allowed("browser"));
    }

    #[test]
    fn test_minimal_profile() {
        let policy = ToolPolicy::from_profile("minimal");
        assert!(!policy.is_tool_allowed("bash"));
        assert!(policy.is_tool_allowed("read"));
        assert!(!policy.is_tool_allowed("write"));
        assert!(!policy.is_tool_allowed("edit"));
        assert!(policy.is_tool_allowed("glob"));
        assert!(policy.is_tool_allowed("grep"));
    }

    #[test]
    fn test_none_profile() {
        let policy = ToolPolicy::from_profile("none");
        assert!(!policy.is_tool_allowed("bash"));
        assert!(!policy.is_tool_allowed("read"));
        assert!(!policy.is_tool_allowed("write"));
        assert!(!policy.is_tool_allowed("glob"));
    }

    #[test]
    fn test_command_allowed_full() {
        assert!(is_command_allowed("rm -rf /", &[], "full"));
    }

    #[test]
    fn test_command_allowed_deny() {
        assert!(!is_command_allowed("ls", &["ls".to_string()], "deny"));
    }

    #[test]
    fn test_command_allowed_allowlist_prefix() {
        let allowlist = vec!["git ".to_string()];
        assert!(is_command_allowed("git status", &allowlist, "allowlist"));
        assert!(!is_command_allowed("rm -rf", &allowlist, "allowlist"));
    }

    #[test]
    fn test_command_allowed_allowlist_binary() {
        let allowlist = vec!["ls".to_string()];
        assert!(is_command_allowed("ls -la", &allowlist, "allowlist"));
        assert!(is_command_allowed("/bin/ls -la", &allowlist, "allowlist"));
    }

    #[test]
    fn test_command_rejects_shell_injection() {
        let allowlist = vec!["git".to_string()];
        assert!(!is_command_allowed(
            "git; rm -rf /",
            &allowlist,
            "allowlist"
        ));
        assert!(!is_command_allowed(
            "git && malicious",
            &allowlist,
            "allowlist"
        ));
        assert!(!is_command_allowed(
            "git || malicious",
            &allowlist,
            "allowlist"
        ));
        assert!(!is_command_allowed("git `whoami`", &allowlist, "allowlist"));
        assert!(!is_command_allowed(
            "git $(whoami)",
            &allowlist,
            "allowlist"
        ));
    }

    #[test]
    fn test_command_rejects_pipe() {
        let allowlist = vec!["git".to_string()];
        assert!(!is_command_allowed(
            "git log | rm -rf /",
            &allowlist,
            "allowlist"
        ));
    }

    #[test]
    fn test_command_rejects_newline_injection() {
        let allowlist = vec!["git".to_string()];
        assert!(!is_command_allowed(
            "git status\nrm -rf /",
            &allowlist,
            "allowlist"
        ));
        assert!(!is_command_allowed(
            "git status\rrm -rf /",
            &allowlist,
            "allowlist"
        ));
    }

    #[test]
    fn test_command_rejects_heredoc() {
        let allowlist = vec!["cat".to_string()];
        assert!(!is_command_allowed("cat << EOF", &allowlist, "allowlist"));
    }

    #[test]
    fn test_command_rejects_process_substitution() {
        let allowlist = vec!["diff".to_string()];
        assert!(!is_command_allowed(
            "diff <(cat /etc/shadow) /dev/null",
            &allowlist,
            "allowlist"
        ));
        assert!(!is_command_allowed(
            "diff >(tee /tmp/out) /dev/null",
            &allowlist,
            "allowlist"
        ));
    }

    #[test]
    fn test_command_rejects_non_matching() {
        let allowlist = vec!["git".to_string()];
        assert!(!is_command_allowed("grep foo", &allowlist, "allowlist"));
        assert!(!is_command_allowed("rm file", &allowlist, "allowlist"));
    }
}
