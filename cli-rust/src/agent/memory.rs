use std::path::PathBuf;

use anyhow::{Context, Result};

/// Return the memory directory: `~/.openclaw-cli/memory/`.
pub(crate) fn memory_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    Ok(home.join(".openclaw-cli").join("memory"))
}

/// Read a file from the memory directory. Returns `Ok(None)` if the file
/// does not exist.
pub(crate) fn read_memory_file(name: &str) -> Result<Option<String>> {
    let path = memory_dir()?.join(name);
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("reading memory file {}", path.display()))?;
    Ok(Some(content))
}

/// Write content to a file in the memory directory, creating the directory
/// if it does not exist.
#[allow(dead_code)]
pub(crate) fn write_memory_file(name: &str, content: &str) -> Result<()> {
    let dir = memory_dir()?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating memory directory {}", dir.display()))?;
    let path = dir.join(name);
    std::fs::write(&path, content)
        .with_context(|| format!("writing memory file {}", path.display()))?;
    Ok(())
}

/// List all files in the memory directory. Returns an empty vec if the
/// directory does not exist.
#[allow(dead_code)]
pub(crate) fn list_memory_files() -> Result<Vec<String>> {
    let dir = memory_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .with_context(|| format!("reading memory directory {}", dir.display()))?
    {
        let entry = entry?;
        if let Some(name) = entry.file_name().to_str() {
            names.push(name.to_string());
        }
    }
    names.sort();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_dir_is_under_home() {
        let dir = memory_dir().unwrap();
        assert!(dir.ends_with(".openclaw-cli/memory"));
    }

    #[test]
    fn test_read_nonexistent_returns_none() {
        // Reading a file that almost certainly doesn't exist should return None.
        let result = read_memory_file("__test_nonexistent_file_abc123__").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_write_and_read_memory_file() {
        let test_name = "__test_memory_roundtrip__.txt";
        // Clean up from any prior run
        let _ = std::fs::remove_file(memory_dir().unwrap().join(test_name));

        write_memory_file(test_name, "hello memory").unwrap();
        let content = read_memory_file(test_name).unwrap();
        assert_eq!(content.as_deref(), Some("hello memory"));

        // Clean up
        let _ = std::fs::remove_file(memory_dir().unwrap().join(test_name));
    }

    #[test]
    fn test_list_memory_files_no_panic() {
        // Should not panic even if the directory does not exist (returns empty vec).
        let files = list_memory_files().unwrap();
        // We can't assert much about the contents, but it should be a valid vec.
        let _ = files;
    }
}
