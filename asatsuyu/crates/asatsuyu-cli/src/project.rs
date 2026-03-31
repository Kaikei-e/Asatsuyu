//! Project root discovery, configuration schema, and source file enumeration.
//!
//! Walks up from the current directory looking for `asatsuyu.toml` to locate the
//! project root. Parses and validates the project configuration. Discovers `.asty`
//! source files under `src/`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use pep440_rs::VersionSpecifiers;
use serde::Deserialize;

/// Marker file that identifies an Asatsuyu project root.
const PROJECT_MARKER: &str = "asatsuyu.toml";

/// The highest schema version this compiler supports.
const SUPPORTED_SCHEMA_VERSION: u32 = 1;

// ── Project types ────────────────────────────────────────────────────

/// A discovered project root with its configuration.
#[derive(Debug)]
pub(crate) struct Project {
    /// Absolute path to the project root (the directory containing `asatsuyu.toml`).
    pub(crate) root: PathBuf,
    /// Parsed and validated project configuration.
    /// Currently consumed by project discovery; downstream use expands in Issues 57–60.
    #[allow(dead_code)]
    pub(crate) config: ProjectConfig,
}

/// Parsed `asatsuyu.toml`.
///
/// Unknown keys at any level are rejected (`deny_unknown_fields`) to catch
/// typos early. The only exception is `[tool]`, which acts as a passthrough
/// namespace for external tools (following the pyproject.toml `[tool.*]`
/// convention).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectConfig {
    /// Schema version (default: 1). Allows future breaking changes.
    #[serde(default = "default_schema_version")]
    schema_version: u32,

    /// `[project]` — required project metadata.
    pub(crate) project: ProjectMeta,

    /// `[python]` — Python environment constraints.
    /// Consumed starting in Issue 57.
    #[serde(default)]
    #[allow(dead_code)]
    python: Option<PythonConfig>,

    /// `[python-dependencies]` — Python package dependencies.
    /// Consumed starting in Issue 57.
    #[serde(default, rename = "python-dependencies")]
    #[allow(dead_code)]
    python_dependencies: BTreeMap<String, String>,

    /// `[ffi]` — FFI resolution configuration.
    /// Consumed starting in Issue 57.
    #[serde(default)]
    #[allow(dead_code)]
    ffi: Option<FfiConfig>,

    /// `[tool]` — passthrough namespace for external tool configuration.
    /// Not validated by the compiler.
    #[serde(default)]
    #[allow(dead_code)]
    tool: Option<toml::Table>,
}

fn default_schema_version() -> u32 {
    1
}

/// `[project]` section of `asatsuyu.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectMeta {
    /// Project name (used as package name in generated Python output).
    pub(crate) name: String,
    /// Project version (semver recommended).
    #[serde(default = "default_version")]
    #[allow(dead_code)]
    pub(crate) version: String,
}

fn default_version() -> String {
    "0.1.0".to_string()
}

/// `[python]` section of `asatsuyu.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
pub(crate) struct PythonConfig {
    /// Python version constraint (e.g. `">=3.12"`).
    #[serde(default)]
    pub(crate) version: Option<String>,
    /// Explicit path to Python interpreter (overrides environment discovery).
    #[serde(default)]
    pub(crate) path: Option<PathBuf>,
}

/// `[ffi]` section of `asatsuyu.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[allow(dead_code)]
pub(crate) struct FfiConfig {
    /// Restrict FFI to stdlib modules only (pathlib, json, os, sys).
    #[serde(default)]
    pub(crate) stdlib_only: bool,
    /// Additional directories for `.pyi` stub files.
    #[serde(default)]
    pub(crate) stub_paths: Vec<PathBuf>,
}

// ── Config accessors ────────────────────────────────────────────────

#[allow(dead_code)]
impl ProjectConfig {
    pub(crate) fn name(&self) -> &str {
        &self.project.name
    }

    pub(crate) fn version(&self) -> &str {
        &self.project.version
    }

    pub(crate) fn python_version(&self) -> Option<&str> {
        self.python.as_ref().and_then(|p| p.version.as_deref())
    }

    pub(crate) fn python_dependencies(&self) -> &BTreeMap<String, String> {
        &self.python_dependencies
    }

    pub(crate) fn ffi_config(&self) -> Option<&FfiConfig> {
        self.ffi.as_ref()
    }

    pub(crate) fn python_path(&self) -> Option<&Path> {
        self.python.as_ref().and_then(|p| p.path.as_deref())
    }
}

// ── Validation ──────────────────────────────────────────────────────

impl ProjectConfig {
    /// Validate semantic constraints that cannot be expressed via serde attributes.
    pub(crate) fn validate(&self) -> Result<(), ProjectError> {
        if self.schema_version > SUPPORTED_SCHEMA_VERSION {
            return Err(ProjectError::UnsupportedSchema {
                found: self.schema_version,
                max: SUPPORTED_SCHEMA_VERSION,
            });
        }

        if self.project.name.is_empty() {
            return Err(ProjectError::InvalidConfig("project.name cannot be empty".into()));
        }

        if !self.project.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(ProjectError::InvalidConfig(
                "project.name must contain only ASCII letters, digits, underscores, and hyphens"
                    .into(),
            ));
        }

        self.validate_python_specifiers()?;

        Ok(())
    }

    /// Validate PEP 440 version specifiers in `[python-dependencies]` and `[python] version`.
    fn validate_python_specifiers(&self) -> Result<(), ProjectError> {
        // Validate [python] version constraint.
        if let Some(version_str) = self.python_version() {
            VersionSpecifiers::from_str(version_str).map_err(|e| {
                ProjectError::InvalidConfig(format!(
                    "python.version: invalid PEP 440 specifier \"{version_str}\": {e}"
                ))
            })?;
        }

        // Validate each [python-dependencies] specifier.
        for (name, specifier) in &self.python_dependencies {
            VersionSpecifiers::from_str(specifier).map_err(|e| {
                ProjectError::InvalidConfig(format!(
                    "python-dependencies.{name}: invalid PEP 440 specifier \"{specifier}\": {e}"
                ))
            })?;
        }

        Ok(())
    }
}

// ── Error type ───────────────────────────────────────────────────────

/// Errors that can occur during project discovery and configuration.
#[derive(Debug)]
pub(crate) enum ProjectError {
    /// Failed to read `asatsuyu.toml`.
    ReadConfig(std::io::Error),
    /// Failed to parse `asatsuyu.toml` (syntax or unknown field).
    ParseConfig(toml::de::Error),
    /// `src/` directory does not exist.
    NoSrcDir(PathBuf),
    /// No `.asty` files found under `src/`.
    NoSourceFiles(PathBuf),
    /// Schema version is newer than what this compiler supports.
    UnsupportedSchema { found: u32, max: u32 },
    /// Semantic validation failure.
    InvalidConfig(String),
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
            Self::UnsupportedSchema { found, max } => {
                write!(
                    f,
                    "{PROJECT_MARKER} schema_version {found} is not supported \
                     (max: {max}); upgrade your compiler"
                )
            }
            Self::InvalidConfig(msg) => {
                write!(f, "invalid {PROJECT_MARKER}: {msg}")
            }
        }
    }
}

// ── Discovery functions ──────────────────────────────────────────────

/// Discover the project root by walking up from `start_dir`.
///
/// Returns `Ok(None)` if no `asatsuyu.toml` is found before reaching the
/// filesystem root. Validates the configuration after parsing.
pub(crate) fn discover_project(start_dir: &Path) -> Result<Option<Project>, ProjectError> {
    let start = start_dir.canonicalize().map_err(ProjectError::ReadConfig)?;

    let mut current = start.as_path();
    loop {
        let marker = current.join(PROJECT_MARKER);
        if marker.is_file() {
            let content = std::fs::read_to_string(&marker).map_err(ProjectError::ReadConfig)?;
            let config: ProjectConfig =
                toml::from_str(&content).map_err(ProjectError::ParseConfig)?;
            config.validate()?;
            return Ok(Some(Project { root: current.to_path_buf(), config }));
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return Ok(None),
        }
    }
}

/// Parse and validate `asatsuyu.toml` content directly (for testing and
/// programmatic use).
#[allow(dead_code)]
pub(crate) fn parse_config(content: &str) -> Result<ProjectConfig, ProjectError> {
    let config: ProjectConfig = toml::from_str(content).map_err(ProjectError::ParseConfig)?;
    config.validate()?;
    Ok(config)
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

    // ── Parsing tests ───────────────────────────────────────────────

    #[test]
    fn parse_full_config() {
        let toml = r#"
schema_version = 1

[project]
name = "my-app"
version = "0.2.0"

[python]
version = ">=3.12"

[python-dependencies]
requests = ">=2.31"
flask = ">=3.0"

[ffi]
stdlib-only = true
stub-paths = ["stubs/", "vendor/stubs/"]

[tool.asatsuyu]
custom-key = "value"
"#;
        let config = parse_config(toml).unwrap();
        assert_eq!(config.name(), "my-app");
        assert_eq!(config.version(), "0.2.0");
        assert_eq!(config.python_version(), Some(">=3.12"));
        assert_eq!(config.python_dependencies().len(), 2);
        assert_eq!(config.python_dependencies()["requests"], ">=2.31");
        assert_eq!(config.python_dependencies()["flask"], ">=3.0");
        let ffi = config.ffi_config().unwrap();
        assert!(ffi.stdlib_only);
        assert_eq!(ffi.stub_paths.len(), 2);
    }

    #[test]
    fn parse_minimal_config() {
        let toml = "[project]\nname = \"demo\"\n";
        let config = parse_config(toml).unwrap();
        assert_eq!(config.name(), "demo");
        assert_eq!(config.version(), "0.1.0"); // default
        assert_eq!(config.schema_version, 1); // default
        assert!(config.python_version().is_none());
        assert!(config.python_dependencies().is_empty());
        assert!(config.ffi_config().is_none());
    }

    #[test]
    fn parse_config_generated_by_cmd_new() {
        let toml = "schema_version = 1\n\n[project]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[python]\nversion = \">=3.12\"\n";
        let config = parse_config(toml).unwrap();
        assert_eq!(config.name(), "demo");
        assert_eq!(config.schema_version, 1);
        assert_eq!(config.python_version(), Some(">=3.12"));
    }

    // ── Unknown field rejection ────────────────────────────────────

    #[test]
    fn reject_unknown_top_level_key() {
        let toml = "[project]\nname = \"demo\"\ntypo = true\n";
        let err = parse_config(toml).unwrap_err();
        assert!(
            err.to_string().contains("unknown field"),
            "should reject unknown top-level key: {err}"
        );
    }

    #[test]
    fn reject_unknown_section() {
        let toml = "[project]\nname = \"demo\"\n\n[unknown]\nfoo = 1\n";
        let err = parse_config(toml).unwrap_err();
        assert!(err.to_string().contains("unknown field"), "should reject unknown section: {err}");
    }

    #[test]
    fn reject_unknown_project_key() {
        let toml = "[project]\nname = \"demo\"\ndescription = \"oops\"\n";
        let err = parse_config(toml).unwrap_err();
        assert!(
            err.to_string().contains("unknown field"),
            "should reject unknown key in [project]: {err}"
        );
    }

    #[test]
    fn reject_unknown_ffi_key() {
        let toml = "[project]\nname = \"demo\"\n\n[ffi]\ntypo = true\n";
        let err = parse_config(toml).unwrap_err();
        assert!(
            err.to_string().contains("unknown field"),
            "should reject unknown key in [ffi]: {err}"
        );
    }

    // ── Tool section passthrough ───────────────────────────────────

    #[test]
    fn tool_section_allows_arbitrary_keys() {
        let toml = r#"
[project]
name = "demo"

[tool.asatsuyu]
custom = "value"

[tool.other-tool]
nested = { key = 42 }
"#;
        let config = parse_config(toml).unwrap();
        assert_eq!(config.name(), "demo");
    }

    // ── Schema version validation ──────────────────────────────────

    #[test]
    fn reject_unsupported_schema_version() {
        let toml = "schema_version = 99\n\n[project]\nname = \"demo\"\n";
        let err = parse_config(toml).unwrap_err();
        assert!(
            err.to_string().contains("schema_version 99 is not supported"),
            "should reject unsupported schema version: {err}"
        );
    }

    #[test]
    fn schema_version_defaults_to_1() {
        let toml = "[project]\nname = \"demo\"\n";
        let config = parse_config(toml).unwrap();
        assert_eq!(config.schema_version, 1);
    }

    // ── Project name validation ────────────────────────────────────

    #[test]
    fn reject_empty_project_name() {
        let toml = "[project]\nname = \"\"\n";
        let err = parse_config(toml).unwrap_err();
        assert!(err.to_string().contains("cannot be empty"), "should reject empty name: {err}");
    }

    #[test]
    fn reject_invalid_project_name() {
        let toml = "[project]\nname = \"bad name!\"\n";
        let err = parse_config(toml).unwrap_err();
        assert!(err.to_string().contains("must contain only"), "should reject invalid name: {err}");
    }

    #[test]
    fn accept_hyphenated_project_name() {
        let toml = "[project]\nname = \"my-app\"\n";
        let config = parse_config(toml).unwrap();
        assert_eq!(config.name(), "my-app");
    }

    // ── Default values ─────────────────────────────────────────────

    #[test]
    fn version_defaults_to_0_1_0() {
        let toml = "[project]\nname = \"demo\"\n";
        let config = parse_config(toml).unwrap();
        assert_eq!(config.version(), "0.1.0");
    }

    // ── FFI config ─────────────────────────────────────────────────

    #[test]
    fn parse_ffi_config() {
        let toml =
            "[project]\nname = \"demo\"\n\n[ffi]\nstdlib-only = true\nstub-paths = [\"stubs/\"]\n";
        let config = parse_config(toml).unwrap();
        let ffi = config.ffi_config().unwrap();
        assert!(ffi.stdlib_only);
        assert_eq!(ffi.stub_paths, vec![PathBuf::from("stubs/")]);
    }

    #[test]
    fn ffi_config_defaults() {
        let toml = "[project]\nname = \"demo\"\n\n[ffi]\n";
        let config = parse_config(toml).unwrap();
        let ffi = config.ffi_config().unwrap();
        assert!(!ffi.stdlib_only);
        assert!(ffi.stub_paths.is_empty());
    }

    // ── Python dependencies ────────────────────────────────────────

    #[test]
    fn parse_python_dependencies() {
        let toml = "[project]\nname = \"demo\"\n\n[python-dependencies]\nrequests = \">=2.31\"\nnumpy = \">=1.26\"\n";
        let config = parse_config(toml).unwrap();
        let deps = config.python_dependencies();
        assert_eq!(deps.len(), 2);
        assert_eq!(deps["requests"], ">=2.31");
        assert_eq!(deps["numpy"], ">=1.26");
    }

    // ── PEP 440 validation ──────────────────────────────────────────

    #[test]
    fn valid_pep440_specifiers() {
        let cases = [
            (">=2.31", "simple >="),
            (">=1.0,<2.0", "range"),
            ("==3.12.*", "wildcard"),
            ("~=1.0", "compatible release"),
            ("!=1.5", "exclusion"),
            (">=1.0,!=1.3.*,<2.0", "complex"),
        ];
        for (spec, label) in cases {
            let toml =
                format!("[project]\nname = \"demo\"\n\n[python-dependencies]\npkg = \"{spec}\"\n");
            assert!(parse_config(&toml).is_ok(), "should accept {label}: {spec}");
        }
    }

    #[test]
    fn reject_invalid_pep440_specifier() {
        let cases = ["not a version", "abc", ">>>1.0", ">=,<2"];
        for spec in cases {
            let toml =
                format!("[project]\nname = \"demo\"\n\n[python-dependencies]\npkg = \"{spec}\"\n");
            let err = parse_config(&toml).unwrap_err();
            assert!(
                err.to_string().contains("invalid PEP 440 specifier"),
                "should reject \"{spec}\": {err}"
            );
        }
    }

    #[test]
    fn validate_python_version_specifier_valid() {
        let toml = "[project]\nname = \"demo\"\n\n[python]\nversion = \">=3.12\"\n";
        assert!(parse_config(toml).is_ok());
    }

    #[test]
    fn validate_python_version_specifier_invalid() {
        let toml = "[project]\nname = \"demo\"\n\n[python]\nversion = \"bad\"\n";
        let err = parse_config(toml).unwrap_err();
        assert!(err.to_string().contains("python.version"), "should mention python.version: {err}");
    }

    // ── Python path config ────────────────────────────────────────────

    #[test]
    fn python_path_config_parses() {
        let toml = "[project]\nname = \"demo\"\n\n[python]\nversion = \">=3.12\"\npath = \"/usr/bin/python3\"\n";
        let config = parse_config(toml).unwrap();
        assert_eq!(config.python_path(), Some(Path::new("/usr/bin/python3")));
    }

    #[test]
    fn python_path_defaults_to_none() {
        let toml = "[project]\nname = \"demo\"\n\n[python]\nversion = \">=3.12\"\n";
        let config = parse_config(toml).unwrap();
        assert!(config.python_path().is_none());
    }

    // ── Discovery tests ────────────────────────────────────────────

    #[test]
    fn discover_project_finds_marker() {
        let dir = create_temp_dir("find-marker");
        let toml_content = "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n";
        std::fs::write(dir.join("asatsuyu.toml"), toml_content).unwrap();

        let result = discover_project(&dir).unwrap();
        assert!(result.is_some());
        let project = result.unwrap();
        assert_eq!(project.config.name(), "demo");

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
        assert_eq!(project.config.name(), "parent");

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
