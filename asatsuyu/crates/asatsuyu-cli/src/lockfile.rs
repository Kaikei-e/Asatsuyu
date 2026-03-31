//! Lockfile generation and staleness detection for `pylock.toml` (PEP 751).
//!
//! Delegates dependency resolution to external tools (`uv` or `pip`). Asatsuyu
//! does NOT implement its own resolver — this module orchestrates existing Python
//! packaging tools to produce a spec-compliant `pylock.toml`.

use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::project::Project;
use crate::python_env;

// ── Types ────────────────────────────────────────────────────────────

/// An external tool capable of generating `pylock.toml`.
#[derive(Debug)]
pub(crate) enum LockTool {
    /// uv >= 0.6.15 — preferred (faster, native pylock.toml support).
    Uv { path: PathBuf },
    /// pip >= 25.1 — fallback.
    Pip { path: PathBuf },
}

impl fmt::Display for LockTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uv { path } => write!(f, "uv ({})", path.display()),
            Self::Pip { path } => write!(f, "pip ({})", path.display()),
        }
    }
}

/// Result of checking lock freshness against declared dependencies.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LockStaleness {
    /// Lock file is consistent with declared dependencies.
    Fresh,
    /// Lock file does not exist.
    Missing,
    /// Lock file exists but may be outdated.
    Stale { reason: String },
}

/// Errors during lock operations.
#[derive(Debug)]
pub(crate) enum LockError {
    /// No suitable lock tool (uv or pip) found on PATH.
    ToolNotFound,
    /// Lock tool invocation failed.
    ToolFailed { tool: String, stderr: String },
    /// Generated lockfile is not valid pylock.toml.
    InvalidLockfile(String),
    /// I/O error (temp dir, file read/write).
    Io(std::io::Error),
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ToolNotFound => {
                write!(f, "no lock tool found; install uv >= 0.6.15 or pip >= 25.1")
            }
            Self::ToolFailed { tool, stderr } => {
                write!(f, "{tool} failed:\n{stderr}")
            }
            Self::InvalidLockfile(reason) => write!(f, "invalid pylock.toml: {reason}"),
            Self::Io(err) => write!(f, "I/O error: {err}"),
        }
    }
}

impl From<std::io::Error> for LockError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

// ── Minimal pylock.toml schema for parsing ──────────────────────────

/// Minimal parse of `pylock.toml` — only fields needed for validation and
/// staleness detection. The full spec has many more fields that we ignore.
#[derive(Debug, Deserialize)]
struct PylockToml {
    #[serde(rename = "lock-version")]
    lock_version: String,
    #[serde(rename = "created-by")]
    #[allow(dead_code)]
    created_by: String,
    #[serde(default)]
    packages: Vec<PylockPackage>,
}

#[derive(Debug, Deserialize)]
struct PylockPackage {
    name: String,
    #[allow(dead_code)]
    version: Option<String>,
}

// ── Tool discovery ──────────────────────────────────────────────────

/// Discover a lock tool on PATH. Prefers uv (faster), falls back to pip.
pub(crate) fn discover_lock_tool() -> Option<LockTool> {
    if let Some(tool) = discover_uv() {
        return Some(tool);
    }
    discover_pip()
}

fn discover_uv() -> Option<LockTool> {
    let output = Command::new("uv").arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = parse_uv_version(&stdout)?;
    if version_at_least(&version, &[0, 6, 15]) {
        Some(LockTool::Uv { path: which_tool("uv")? })
    } else {
        None
    }
}

fn discover_pip() -> Option<LockTool> {
    let output = Command::new("pip").arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = parse_pip_version(&stdout)?;
    if version_at_least(&version, &[25, 1]) {
        Some(LockTool::Pip { path: which_tool("pip")? })
    } else {
        None
    }
}

/// Locate a tool on PATH.
fn which_tool(name: &str) -> Option<PathBuf> {
    let output = Command::new("which").arg(name).output().ok()?;
    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Some(PathBuf::from(path))
    } else {
        None
    }
}

/// Parse version from `uv X.Y.Z (...)` output.
fn parse_uv_version(stdout: &str) -> Option<Vec<u32>> {
    // "uv 0.11.0 (x86_64-unknown-linux-gnu)\n"
    let line = stdout.lines().next()?;
    let version_str = line.strip_prefix("uv ")?;
    let version_str = version_str.split_whitespace().next()?;
    parse_dotted_version(version_str)
}

/// Parse version from `pip X.Y.Z from ...` output.
fn parse_pip_version(stdout: &str) -> Option<Vec<u32>> {
    // "pip 25.1 from /path/to/pip (python 3.13)\n"
    let line = stdout.lines().next()?;
    let version_str = line.strip_prefix("pip ")?;
    let version_str = version_str.split_whitespace().next()?;
    parse_dotted_version(version_str)
}

/// Parse a dotted version string like "1.2.3" into [1, 2, 3].
fn parse_dotted_version(s: &str) -> Option<Vec<u32>> {
    let parts: Option<Vec<u32>> = s.split('.').map(|p| p.parse().ok()).collect();
    let parts = parts?;
    if parts.is_empty() { None } else { Some(parts) }
}

/// Check if a parsed version is >= the minimum required.
fn version_at_least(actual: &[u32], minimum: &[u32]) -> bool {
    for (i, &min) in minimum.iter().enumerate() {
        let act = actual.get(i).copied().unwrap_or(0);
        match act.cmp(&min) {
            std::cmp::Ordering::Greater => return true,
            std::cmp::Ordering::Less => return false,
            std::cmp::Ordering::Equal => {}
        }
    }
    true
}

// ── Lockfile generation ─────────────────────────────────────────────

/// Generate `pylock.toml` by delegating to an external tool.
///
/// Creates a temporary `pyproject.toml` from the project's declared dependencies,
/// invokes the lock tool, and writes the result to `output_path`.
pub(crate) fn generate_lockfile(
    project: &Project,
    tool: &LockTool,
    output_path: &Path,
) -> Result<(), LockError> {
    let temp_dir = tempfile::tempdir()?;
    let temp_pyproject = temp_dir.path().join("pyproject.toml");

    let pyproject_content = build_temp_pyproject(project);
    std::fs::write(&temp_pyproject, &pyproject_content)?;

    match tool {
        LockTool::Uv { path } => run_uv_lock(path, &temp_pyproject, output_path),
        LockTool::Pip { path } => run_pip_lock(path, temp_dir.path(), output_path),
    }?;

    // Validate the generated file.
    let content = std::fs::read_to_string(output_path)?;
    validate_pylock(&content)?;

    Ok(())
}

/// Build a minimal `pyproject.toml` for lock resolution.
///
/// Only includes `[project]` with name, version, requires-python, and
/// dependencies — the minimum pip/uv need for resolution.
fn build_temp_pyproject(project: &Project) -> String {
    let cfg = &project.config;
    let name = cfg.name();
    let version = cfg.version();

    let mut content = format!("[project]\nname = \"{name}\"\nversion = \"{version}\"\n");

    if let Some(python_version) = cfg.python_version() {
        writeln!(content, "requires-python = \"{python_version}\"").unwrap();
    }

    let deps = cfg.python_dependencies();
    if !deps.is_empty() {
        content.push_str("dependencies = [\n");
        for (dep_name, specifier) in deps {
            writeln!(content, "    \"{dep_name}{specifier}\",").unwrap();
        }
        content.push_str("]\n");
    }

    content
}

fn run_uv_lock(uv_path: &Path, pyproject_path: &Path, output_path: &Path) -> Result<(), LockError> {
    let output = Command::new(uv_path)
        .args([
            "pip",
            "compile",
            &pyproject_path.to_string_lossy(),
            "--format",
            "pylock.toml",
            "-o",
            &output_path.to_string_lossy(),
        ])
        .output()?;

    if output.status.success() {
        Ok(())
    } else {
        Err(LockError::ToolFailed {
            tool: "uv pip compile".into(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

fn run_pip_lock(pip_path: &Path, working_dir: &Path, output_path: &Path) -> Result<(), LockError> {
    let output = Command::new(pip_path)
        .args(["lock", "-o", &output_path.to_string_lossy()])
        .current_dir(working_dir)
        .output()?;

    if output.status.success() {
        Ok(())
    } else {
        Err(LockError::ToolFailed {
            tool: "pip lock".into(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Validate that a file is a spec-compliant `pylock.toml`.
fn validate_pylock(content: &str) -> Result<(), LockError> {
    let pylock: PylockToml = toml::from_str(content)
        .map_err(|e| LockError::InvalidLockfile(format!("failed to parse pylock.toml: {e}")))?;

    if pylock.lock_version != "1.0" {
        return Err(LockError::InvalidLockfile(format!(
            "unsupported lock-version \"{}\", expected \"1.0\"",
            pylock.lock_version
        )));
    }

    Ok(())
}

// ── Staleness detection ─────────────────────────────────────────────

/// Check whether `pylock.toml` is consistent with `asatsuyu.toml` declarations.
///
/// Returns `Fresh` when the lock is up to date, `Missing` when no lock file
/// exists, or `Stale` with a reason when dependencies have diverged.
pub(crate) fn check_staleness(project: &Project, pylock_path: &Path) -> LockStaleness {
    if !pylock_path.exists() {
        return LockStaleness::Missing;
    }

    // Fast heuristic: mtime comparison.
    if let Some(staleness) = check_mtime_staleness(project, pylock_path) {
        return staleness;
    }

    // Content comparison: declared dep names vs locked package names.
    check_content_staleness(project, pylock_path)
}

fn check_mtime_staleness(project: &Project, pylock_path: &Path) -> Option<LockStaleness> {
    let config_path = project.root.join("asatsuyu.toml");
    let config_mtime = std::fs::metadata(&config_path).ok()?.modified().ok()?;
    let lock_mtime = std::fs::metadata(pylock_path).ok()?.modified().ok()?;

    if config_mtime > lock_mtime {
        Some(LockStaleness::Stale { reason: "asatsuyu.toml is newer than pylock.toml".into() })
    } else {
        None
    }
}

fn check_content_staleness(project: &Project, pylock_path: &Path) -> LockStaleness {
    let Ok(content) = std::fs::read_to_string(pylock_path) else {
        return LockStaleness::Stale { reason: "failed to read pylock.toml".into() };
    };

    let Ok(pylock): Result<PylockToml, _> = toml::from_str(&content) else {
        return LockStaleness::Stale { reason: "pylock.toml is not valid TOML".into() };
    };

    let declared: BTreeSet<String> = project
        .config
        .python_dependencies()
        .keys()
        .map(|name| python_env::normalize_name(name))
        .collect();

    let locked: BTreeSet<String> =
        pylock.packages.iter().map(|pkg| python_env::normalize_name(&pkg.name)).collect();

    // Check for declared deps missing from the lock.
    let missing: Vec<&String> = declared.difference(&locked).collect();
    if !missing.is_empty() {
        return LockStaleness::Stale {
            reason: format!(
                "dependencies not in lock: {}",
                missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
            ),
        };
    }

    LockStaleness::Fresh
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Version parsing ────────────────────────────────────────────

    #[test]
    fn parse_uv_version_standard() {
        let v = parse_uv_version("uv 0.11.0 (x86_64-unknown-linux-gnu)\n");
        assert_eq!(v, Some(vec![0, 11, 0]));
    }

    #[test]
    fn parse_uv_version_minimal() {
        let v = parse_uv_version("uv 0.6.15\n");
        assert_eq!(v, Some(vec![0, 6, 15]));
    }

    #[test]
    fn parse_pip_version_standard() {
        let v = parse_pip_version(
            "pip 25.1 from /usr/lib/python3.13/site-packages/pip (python 3.13)\n",
        );
        assert_eq!(v, Some(vec![25, 1]));
    }

    #[test]
    fn parse_pip_version_three_part() {
        let v = parse_pip_version("pip 25.1.1 from /path (python 3.12)\n");
        assert_eq!(v, Some(vec![25, 1, 1]));
    }

    #[test]
    fn parse_uv_version_garbage() {
        assert_eq!(parse_uv_version("not uv output"), None);
    }

    #[test]
    fn parse_pip_version_garbage() {
        assert_eq!(parse_pip_version("not pip output"), None);
    }

    // ── Version comparison ────────────────────────────────────────

    #[test]
    fn version_at_least_equal() {
        assert!(version_at_least(&[0, 6, 15], &[0, 6, 15]));
    }

    #[test]
    fn version_at_least_greater_major() {
        assert!(version_at_least(&[1, 0, 0], &[0, 6, 15]));
    }

    #[test]
    fn version_at_least_less() {
        assert!(!version_at_least(&[0, 6, 14], &[0, 6, 15]));
    }

    #[test]
    fn version_at_least_shorter_actual() {
        assert!(version_at_least(&[25, 1], &[25, 1]));
        assert!(!version_at_least(&[25, 0], &[25, 1]));
    }

    // ── pylock.toml validation ────────────────────────────────────

    #[test]
    fn validate_pylock_valid() {
        let content = r#"
lock-version = "1.0"
created-by = "uv"

[[packages]]
name = "requests"
version = "2.31.0"
"#;
        assert!(validate_pylock(content).is_ok());
    }

    #[test]
    fn validate_pylock_missing_lock_version() {
        let content = r#"
created-by = "uv"

[[packages]]
name = "requests"
"#;
        let err = validate_pylock(content);
        assert!(err.is_err());
    }

    #[test]
    fn validate_pylock_wrong_version() {
        let content = r#"
lock-version = "2.0"
created-by = "uv"

[[packages]]
name = "requests"
"#;
        let err = validate_pylock(content).unwrap_err();
        match err {
            LockError::InvalidLockfile(msg) => {
                assert!(msg.contains("2.0"), "error should mention the version: {msg}");
            }
            _ => panic!("expected InvalidLockfile, got: {err}"),
        }
    }

    // ── Temp pyproject.toml generation ────────────────────────────

    #[test]
    fn temp_pyproject_with_deps() {
        let config = crate::project::parse_config(
            r#"
[project]
name = "my-app"
version = "1.0.0"

[python]
version = ">=3.12"

[python-dependencies]
requests = ">=2.31"
flask = ">=3.0"
"#,
        )
        .unwrap();

        let project = Project { root: PathBuf::from("/tmp/fake"), config };

        let content = build_temp_pyproject(&project);
        assert!(content.contains("name = \"my-app\""));
        assert!(content.contains("version = \"1.0.0\""));
        assert!(content.contains("requires-python = \">=3.12\""));
        assert!(content.contains("\"flask>=3.0\""));
        assert!(content.contains("\"requests>=2.31\""));
    }

    #[test]
    fn temp_pyproject_no_deps() {
        let config = crate::project::parse_config(
            r#"
[project]
name = "simple"
"#,
        )
        .unwrap();

        let project = Project { root: PathBuf::from("/tmp/fake"), config };

        let content = build_temp_pyproject(&project);
        assert!(content.contains("name = \"simple\""));
        assert!(!content.contains("dependencies"));
    }

    #[test]
    fn temp_pyproject_no_python_version() {
        let config = crate::project::parse_config(
            r#"
[project]
name = "no-py"

[python-dependencies]
click = ">=8.0"
"#,
        )
        .unwrap();

        let project = Project { root: PathBuf::from("/tmp/fake"), config };

        let content = build_temp_pyproject(&project);
        assert!(!content.contains("requires-python"));
        assert!(content.contains("\"click>=8.0\""));
    }

    // ── Staleness detection ──────────────────────────────────────

    #[test]
    fn staleness_missing_lockfile() {
        let config = crate::project::parse_config(
            r#"
[project]
name = "test"

[python-dependencies]
requests = ">=2.31"
"#,
        )
        .unwrap();

        let project = Project { root: PathBuf::from("/tmp/nonexistent-project"), config };

        let result = check_staleness(&project, Path::new("/tmp/nonexistent/pylock.toml"));
        assert_eq!(result, LockStaleness::Missing);
    }

    #[test]
    fn content_staleness_fresh() {
        let config = crate::project::parse_config(
            r#"
[project]
name = "test"

[python-dependencies]
requests = ">=2.31"
"#,
        )
        .unwrap();

        let project = Project { root: PathBuf::from("/tmp/fake"), config };

        let temp_dir = tempfile::tempdir().unwrap();
        let pylock_path = temp_dir.path().join("pylock.toml");
        std::fs::write(
            &pylock_path,
            r#"
lock-version = "1.0"
created-by = "uv"

[[packages]]
name = "requests"
version = "2.31.0"

[[packages]]
name = "urllib3"
version = "2.2.0"
"#,
        )
        .unwrap();

        let result = check_content_staleness(&project, &pylock_path);
        assert_eq!(result, LockStaleness::Fresh);
    }

    #[test]
    fn content_staleness_new_dep() {
        let config = crate::project::parse_config(
            r#"
[project]
name = "test"

[python-dependencies]
requests = ">=2.31"
flask = ">=3.0"
"#,
        )
        .unwrap();

        let project = Project { root: PathBuf::from("/tmp/fake"), config };

        let temp_dir = tempfile::tempdir().unwrap();
        let pylock_path = temp_dir.path().join("pylock.toml");
        std::fs::write(
            &pylock_path,
            r#"
lock-version = "1.0"
created-by = "uv"

[[packages]]
name = "requests"
version = "2.31.0"
"#,
        )
        .unwrap();

        let result = check_content_staleness(&project, &pylock_path);
        assert!(matches!(result, LockStaleness::Stale { .. }));
        if let LockStaleness::Stale { reason } = result {
            assert!(reason.contains("flask"), "reason should mention flask: {reason}");
        }
    }

    #[test]
    fn content_staleness_no_declared_deps() {
        let config = crate::project::parse_config(
            r#"
[project]
name = "test"
"#,
        )
        .unwrap();

        let project = Project { root: PathBuf::from("/tmp/fake"), config };

        let temp_dir = tempfile::tempdir().unwrap();
        let pylock_path = temp_dir.path().join("pylock.toml");
        std::fs::write(
            &pylock_path,
            r#"
lock-version = "1.0"
created-by = "uv"

[[packages]]
name = "leftover"
version = "1.0.0"
"#,
        )
        .unwrap();

        // No declared deps → lock has extra packages → still Fresh
        // (transitive deps in lock are expected even if not directly declared).
        let result = check_content_staleness(&project, &pylock_path);
        assert_eq!(result, LockStaleness::Fresh);
    }
}
