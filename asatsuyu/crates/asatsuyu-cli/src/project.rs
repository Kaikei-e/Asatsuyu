//! Project root discovery and source file enumeration.
//!
//! Walks up from the current directory looking for `asatsuyu.toml` to locate the
//! project root. Discovers `.asty` source files under `src/`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Marker file that identifies an Asatsuyu project root.
const PROJECT_MARKER: &str = "asatsuyu.toml";

// ── Project types ────────────────────────────────────────────────────

/// A discovered project root with its configuration.
#[derive(Debug)]
pub(crate) struct Project {
    /// Absolute path to the project root (the directory containing `asatsuyu.toml`).
    pub(crate) root: PathBuf,
    /// Parsed project configuration.
    #[allow(dead_code)]
    pub(crate) config: ProjectConfig,
}

/// Parsed `asatsuyu.toml`.
#[derive(Debug, Deserialize)]
pub(crate) struct ProjectConfig {
    #[allow(dead_code)]
    pub(crate) project: ProjectMeta,
}

/// `[project]` section of `asatsuyu.toml`.
#[derive(Debug, Deserialize)]
pub(crate) struct ProjectMeta {
    #[allow(dead_code)]
    pub(crate) name: String,
    #[allow(dead_code)]
    pub(crate) version: String,
}

// ── Error type ───────────────────────────────────────────────────────

/// Errors that can occur during project discovery.
#[derive(Debug)]
pub(crate) enum ProjectError {
    /// Failed to read `asatsuyu.toml`.
    ReadConfig(std::io::Error),
    /// Failed to parse `asatsuyu.toml`.
    ParseConfig(toml::de::Error),
    /// `src/` directory does not exist.
    NoSrcDir(PathBuf),
    /// No `.asty` files found under `src/`.
    NoSourceFiles(PathBuf),
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadConfig(e) => write!(f, "cannot read {PROJECT_MARKER}: {e}"),
            Self::ParseConfig(e) => write!(f, "failed to parse {PROJECT_MARKER}: {e}"),
            Self::NoSrcDir(root) => {
                write!(f, "no `src/` directory found in {}", root.display())
            }
            Self::NoSourceFiles(src_dir) => {
                write!(f, "no `.asty` files found in {}", src_dir.display())
            }
        }
    }
}

// ── Discovery functions ──────────────────────────────────────────────

/// Discover the project root by walking up from `start_dir`.
///
/// Returns `Ok(None)` if no `asatsuyu.toml` is found before reaching the
/// filesystem root.
pub(crate) fn discover_project(start_dir: &Path) -> Result<Option<Project>, ProjectError> {
    let start = start_dir.canonicalize().map_err(ProjectError::ReadConfig)?;

    let mut current = start.as_path();
    loop {
        let marker = current.join(PROJECT_MARKER);
        if marker.is_file() {
            let content = std::fs::read_to_string(&marker).map_err(ProjectError::ReadConfig)?;
            let config: ProjectConfig =
                toml::from_str(&content).map_err(ProjectError::ParseConfig)?;
            return Ok(Some(Project { root: current.to_path_buf(), config }));
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return Ok(None),
        }
    }
}

/// Collect all `.asty` files under `project_root/src/`, sorted for deterministic
/// output.
pub(crate) fn discover_sources(project_root: &Path) -> Result<Vec<PathBuf>, ProjectError> {
    let src_dir = project_root.join("src");
    if !src_dir.is_dir() {
        return Err(ProjectError::NoSrcDir(project_root.to_path_buf()));
    }

    let mut files = Vec::new();
    collect_asty_files(&src_dir, &mut files).map_err(ProjectError::ReadConfig)?;
    files.sort();

    if files.is_empty() {
        return Err(ProjectError::NoSourceFiles(src_dir));
    }

    Ok(files)
}

/// Recursively collect `.asty` files under `dir`.
fn collect_asty_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_asty_files(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "asty") {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("asatsuyu-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn discover_project_finds_marker() {
        let dir = create_temp_dir("find-marker");
        let toml_content = "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n";
        std::fs::write(dir.join("asatsuyu.toml"), toml_content).unwrap();

        let result = discover_project(&dir).unwrap();
        assert!(result.is_some());
        let project = result.unwrap();
        assert_eq!(project.config.project.name, "demo");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_project_walks_up() {
        let dir = create_temp_dir("walk-up");
        let child = dir.join("src");
        std::fs::create_dir_all(&child).unwrap();
        let toml_content = "[project]\nname = \"parent\"\nversion = \"0.1.0\"\n";
        std::fs::write(dir.join("asatsuyu.toml"), toml_content).unwrap();

        let result = discover_project(&child).unwrap();
        assert!(result.is_some());
        let project = result.unwrap();
        assert_eq!(project.config.project.name, "parent");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_project_none_without_marker() {
        let dir = create_temp_dir("no-marker");
        let result = discover_project(&dir).unwrap();
        assert!(result.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_sources_finds_asty_files() {
        let dir = create_temp_dir("find-sources");
        let src = dir.join("src");
        let sub = src.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(src.join("main.asty"), "pub fn main() { 42 }").unwrap();
        std::fs::write(sub.join("lib.asty"), "pub fn add(a: Int, b: Int) -> Int { a }").unwrap();
        // Non-.asty file should be ignored.
        std::fs::write(src.join("notes.txt"), "not a source file").unwrap();

        let files = discover_sources(&dir).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|p| p.ends_with("main.asty")));
        assert!(files.iter().any(|p| p.ends_with("lib.asty")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_sources_errors_on_missing_src() {
        let dir = create_temp_dir("no-src");
        let result = discover_sources(&dir);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("no `src/` directory"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
