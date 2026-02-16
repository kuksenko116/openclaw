use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use uuid::Uuid;

use super::types::{ContentBlock, Message};

/// A conversation session backed by a JSON file.
pub(crate) struct Session {
    path: PathBuf,
    messages: Vec<Message>,
}

impl Session {
    /// Create a new empty session at the given path.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            messages: Vec::new(),
        }
    }

    /// Load a session from a JSON file.
    pub fn load(path: &Path) -> Result<Self> {
        let data =
            std::fs::read_to_string(path).with_context(|| format!("reading session {}", path.display()))?;
        let messages: Vec<Message> =
            serde_json::from_str(&data).with_context(|| format!("parsing session {}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            messages,
        })
    }

    /// Save the session atomically (write to .tmp, then rename).
    pub fn save(&self) -> Result<()> {
        let dir = self
            .path
            .parent()
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating session directory {}", dir.display()))?;

        let tmp_path = dir.join(format!(".session-{}.tmp", Uuid::new_v4()));
        let data = serde_json::to_string_pretty(&self.messages)
            .context("serializing session")?;

        std::fs::write(&tmp_path, &data)
            .with_context(|| format!("writing temp session file {}", tmp_path.display()))?;
        std::fs::rename(&tmp_path, &self.path)
            .with_context(|| format!("renaming session file to {}", self.path.display()))?;

        Ok(())
    }

    /// Append a user message.
    pub fn add_user_message(&mut self, text: &str) {
        self.messages.push(Message::user(text));
    }

    /// Append an assistant message built from content blocks.
    pub fn add_assistant_message(&mut self, content: Vec<ContentBlock>) {
        self.messages.push(Message::assistant(content));
    }

    /// Return the message history.
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Append a raw message (used by the agent loop for tool results).
    pub fn push_message(&mut self, message: Message) {
        self.messages.push(message);
    }

    /// Clear all messages in the session.
    pub fn clear_messages(&mut self) {
        self.messages.clear();
    }

    /// Replace all messages with a new set (used by compaction).
    pub fn replace_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
    }

    /// Return the session file path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Resolve the sessions directory, defaulting to `~/.openclaw-cli/sessions/`.
pub(crate) fn resolve_sessions_dir(configured: Option<&Path>) -> Result<PathBuf> {
    if let Some(dir) = configured {
        return Ok(dir.to_path_buf());
    }
    let home = dirs::home_dir().context("could not determine home directory")?;
    Ok(home.join(".openclaw-cli").join("sessions"))
}

/// Sanitize a session name to prevent path traversal.
///
/// Strips path separators and ".." components, keeping only safe characters.
/// Returns an error if the name is empty or entirely invalid.
fn sanitize_session_name(name: &str) -> Result<String> {
    // Take only the final path component (strip any directory traversal)
    let basename = Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    // Strip any leading dots (prevents ".." and hidden files)
    let cleaned = basename.trim_start_matches('.');

    // Strip any .json extension the caller may have appended
    let cleaned = cleaned.strip_suffix(".json").unwrap_or(cleaned);

    anyhow::ensure!(!cleaned.is_empty(), "Invalid session name: '{name}'");

    Ok(cleaned.to_string())
}

/// Build a session file path from a session name.
pub(crate) fn session_path(sessions_dir: &Path, name: &str) -> Result<PathBuf> {
    let safe_name = sanitize_session_name(name)?;
    Ok(sessions_dir.join(format!("{safe_name}.json")))
}

/// Load an existing session or create a new one.
pub(crate) fn load_or_create_session(sessions_dir: &Path, name: &str) -> Result<Session> {
    let path = session_path(sessions_dir, name)?;
    if path.exists() {
        Session::load(&path)
    } else {
        Ok(Session::new(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_normal_name() {
        assert_eq!(sanitize_session_name("my-session").unwrap(), "my-session");
    }

    #[test]
    fn test_sanitize_strips_traversal() {
        assert_eq!(
            sanitize_session_name("../../etc/passwd").unwrap(),
            "passwd"
        );
    }

    #[test]
    fn test_sanitize_strips_leading_dots() {
        assert_eq!(sanitize_session_name(".hidden").unwrap(), "hidden");
    }

    #[test]
    fn test_sanitize_strips_json_extension() {
        assert_eq!(sanitize_session_name("test.json").unwrap(), "test");
    }

    #[test]
    fn test_sanitize_rejects_empty() {
        assert!(sanitize_session_name("").is_err());
    }

    #[test]
    fn test_sanitize_rejects_dots_only() {
        assert!(sanitize_session_name("..").is_err());
    }

    #[test]
    fn test_session_path_stays_within_dir() {
        let dir = Path::new("/home/user/sessions");
        let path = session_path(dir, "../../etc/passwd").unwrap();
        assert_eq!(path, PathBuf::from("/home/user/sessions/passwd.json"));
    }
}
