//! Format-preserving TOML editing for `asatsuyu.toml`.
//!
//! Uses `toml_edit` to modify `[python-dependencies]` while preserving
//! comments, formatting, and key ordering in the file.

use std::fmt;
use std::path::Path;

use toml_edit::DocumentMut;

// ── Error type ──────────────────────────────────────────────────────

/// Errors during TOML editing.
#[derive(Debug)]
pub(crate) enum TomlEditError {
    /// I/O error reading or writing the file.
    Io(std::io::Error),
    /// Failed to parse TOML content.
    Parse(toml_edit::TomlError),
    /// The TOML structure is unexpected (e.g., `python-dependencies` is not a table).
    InvalidStructure(String),
}

impl fmt::Display for TomlEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::Parse(err) => write!(f, "TOML parse error: {err}"),
            Self::InvalidStructure(msg) => write!(f, "invalid asatsuyu.toml structure: {msg}"),
        }
    }
}

impl From<std::io::Error> for TomlEditError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<toml_edit::TomlError> for TomlEditError {
    fn from(err: toml_edit::TomlError) -> Self {
        Self::Parse(err)
    }
}

// ── Public API ──────────────────────────────────────────────────────

const DEPS_KEY: &str = "python-dependencies";

/// Add a dependency to `asatsuyu.toml`.
///
/// If `[python-dependencies]` does not exist, creates it.
/// If the package already exists, updates the specifier and returns the old one.
pub(crate) fn add_dependency(
    toml_path: &Path,
    package: &str,
    specifier: &str,
) -> Result<Option<String>, TomlEditError> {
    let content = std::fs::read_to_string(toml_path)?;
    let mut doc: DocumentMut = content.parse()?;

    // Ensure [python-dependencies] table exists.
    if !doc.contains_key(DEPS_KEY) {
        doc[DEPS_KEY] = toml_edit::table();
    }

    let table = doc[DEPS_KEY]
        .as_table_mut()
        .ok_or_else(|| TomlEditError::InvalidStructure(format!("[{DEPS_KEY}] is not a table")))?;

    let previous = table.get(package).and_then(toml_edit::Item::as_str).map(String::from);

    table[package] = toml_edit::value(specifier);

    std::fs::write(toml_path, doc.to_string())?;
    Ok(previous)
}

/// Remove a dependency from `asatsuyu.toml`.
///
/// Returns the removed specifier, or `None` if the package was not present.
pub(crate) fn remove_dependency(
    toml_path: &Path,
    package: &str,
) -> Result<Option<String>, TomlEditError> {
    let content = std::fs::read_to_string(toml_path)?;
    let mut doc: DocumentMut = content.parse()?;

    let Some(table) = doc.get_mut(DEPS_KEY).and_then(toml_edit::Item::as_table_mut) else {
        return Ok(None);
    };

    let previous = table.get(package).and_then(toml_edit::Item::as_str).map(String::from);

    if previous.is_some() {
        table.remove(package);
    }

    std::fs::write(toml_path, doc.to_string())?;
    Ok(previous)
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn write_temp_toml(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("asatsuyu.toml");
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[test]
    fn add_to_empty_config() {
        let (_dir, path) = write_temp_toml("[project]\nname = \"test\"\n");
        let prev = add_dependency(&path, "requests", ">=2.31").unwrap();
        assert!(prev.is_none());

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("[python-dependencies]"));
        assert!(content.contains("requests = \">=2.31\""));
    }

    #[test]
    fn add_to_existing_section() {
        let (_dir, path) = write_temp_toml(
            "[project]\nname = \"test\"\n\n[python-dependencies]\nflask = \">=3.0\"\n",
        );
        let prev = add_dependency(&path, "requests", ">=2.31").unwrap();
        assert!(prev.is_none());

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("flask = \">=3.0\""));
        assert!(content.contains("requests = \">=2.31\""));
    }

    #[test]
    fn add_updates_existing() {
        let (_dir, path) = write_temp_toml(
            "[project]\nname = \"test\"\n\n[python-dependencies]\nrequests = \">=2.0\"\n",
        );
        let prev = add_dependency(&path, "requests", ">=2.31").unwrap();
        assert_eq!(prev, Some(">=2.0".into()));

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("requests = \">=2.31\""));
        assert!(!content.contains(">=2.0"));
    }

    #[test]
    fn add_preserves_comments() {
        let (_dir, path) = write_temp_toml(
            "# My project config\n[project]\nname = \"test\" # project name\n\n[python-dependencies]\n# HTTP library\nrequests = \">=2.0\"\n",
        );
        add_dependency(&path, "flask", ">=3.0").unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("# My project config"));
        assert!(content.contains("# project name"));
        assert!(content.contains("# HTTP library"));
    }

    #[test]
    fn remove_existing() {
        let (_dir, path) = write_temp_toml(
            "[project]\nname = \"test\"\n\n[python-dependencies]\nrequests = \">=2.31\"\nflask = \">=3.0\"\n",
        );
        let prev = remove_dependency(&path, "requests").unwrap();
        assert_eq!(prev, Some(">=2.31".into()));

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.contains("requests"));
        assert!(content.contains("flask = \">=3.0\""));
    }

    #[test]
    fn remove_nonexistent() {
        let (_dir, path) = write_temp_toml(
            "[project]\nname = \"test\"\n\n[python-dependencies]\nflask = \">=3.0\"\n",
        );
        let prev = remove_dependency(&path, "requests").unwrap();
        assert!(prev.is_none());
    }

    #[test]
    fn remove_last_dep() {
        let (_dir, path) = write_temp_toml(
            "[project]\nname = \"test\"\n\n[python-dependencies]\nrequests = \">=2.31\"\n",
        );
        let prev = remove_dependency(&path, "requests").unwrap();
        assert_eq!(prev, Some(">=2.31".into()));

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.contains("requests"));
    }

    #[test]
    fn remove_from_no_section() {
        let (_dir, path) = write_temp_toml("[project]\nname = \"test\"\n");
        let prev = remove_dependency(&path, "requests").unwrap();
        assert!(prev.is_none());
    }
}
