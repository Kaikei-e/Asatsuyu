//! Python environment synchronization from `pylock.toml`.
//!
//! Delegates installation to `uv pip sync` (preferred) or falls back to
//! package-by-package `pip install` via pip.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::lockfile;
use crate::python_env::PythonEnvironment;

// ── Types ────────────────────────────────────────────────────────────

/// An external tool capable of syncing from `pylock.toml`.
#[derive(Debug)]
pub(crate) enum SyncTool {
    /// uv >= 0.6.15 — supports `uv pip sync pylock.toml`.
    Uv { path: PathBuf },
    /// pip (any modern version) — fallback: individual `pip install`.
    Pip { path: PathBuf },
}

impl fmt::Display for SyncTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uv { path } => write!(f, "uv ({})", path.display()),
            Self::Pip { path } => write!(f, "pip ({})", path.display()),
        }
    }
}

/// Report of a successful sync operation.
pub(crate) struct SyncReport {
    pub(crate) packages_synced: usize,
    pub(crate) tool_used: String,
}

/// Errors during sync.
#[derive(Debug)]
pub(crate) enum SyncError {
    /// No suitable sync tool found on PATH.
    ToolNotFound,
    /// Sync tool invocation failed.
    ToolFailed { tool: String, stderr: String },
    /// Lock file is invalid.
    InvalidLockfile(String),
    /// I/O error.
    Io(std::io::Error),
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ToolNotFound => {
                write!(f, "no sync tool found; install uv >= 0.6.15 or pip")
            }
            Self::ToolFailed { tool, stderr } => write!(f, "{tool} failed:\n{stderr}"),
            Self::InvalidLockfile(reason) => write!(f, "invalid pylock.toml: {reason}"),
            Self::Io(err) => write!(f, "I/O error: {err}"),
        }
    }
}

impl From<std::io::Error> for SyncError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

// ── Tool discovery ──────────────────────────────────────────────────

/// Discover a sync tool on PATH. Prefers uv (exact sync), falls back to pip.
pub(crate) fn discover_sync_tool() -> Option<SyncTool> {
    // Try uv first — supports exact sync via `uv pip sync`.
    if let Some(tool) = try_discover_uv_sync() {
        return Some(tool);
    }

    // Fall back to pip (any version — individual `pip install` per package).
    lockfile::which_tool("pip").map(|path| SyncTool::Pip { path })
}

fn try_discover_uv_sync() -> Option<SyncTool> {
    let uv_output = Command::new("uv").arg("--version").output().ok()?;
    if !uv_output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&uv_output.stdout);
    let version = lockfile::parse_uv_version(&stdout)?;
    if !lockfile::version_at_least(&version, &[0, 6, 15]) {
        return None;
    }
    let path = lockfile::which_tool("uv")?;
    Some(SyncTool::Uv { path })
}

// ── Sync implementation ─────────────────────────────────────────────

/// Sync the Python environment from `pylock.toml`.
///
/// With uv: runs `uv pip sync pylock.toml` for exact reproducibility.
/// With pip: falls back to `pip install <name>==<version>` per package (additive only).
pub(crate) fn sync_environment(
    pylock_path: &Path,
    env: &PythonEnvironment,
    tool: &SyncTool,
) -> Result<SyncReport, SyncError> {
    match tool {
        SyncTool::Uv { path } => sync_with_uv(path, pylock_path, env),
        SyncTool::Pip { path } => sync_with_pip(path, pylock_path, env),
    }
}

fn sync_with_uv(
    uv_path: &Path,
    pylock_path: &Path,
    env: &PythonEnvironment,
) -> Result<SyncReport, SyncError> {
    let output = Command::new(uv_path)
        .args([
            "pip",
            "sync",
            &pylock_path.to_string_lossy(),
            "--python",
            &env.python_path.to_string_lossy(),
        ])
        .output()?;

    if output.status.success() {
        // Count packages from the lock file for the report.
        let packages = lockfile::parse_pylock_packages(pylock_path)
            .map_err(|e| SyncError::InvalidLockfile(format!("failed to count packages: {e}")))?;
        Ok(SyncReport { packages_synced: packages.len(), tool_used: "uv pip sync".into() })
    } else {
        Err(SyncError::ToolFailed {
            tool: "uv pip sync".into(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

fn sync_with_pip(
    pip_path: &Path,
    pylock_path: &Path,
    env: &PythonEnvironment,
) -> Result<SyncReport, SyncError> {
    let packages = lockfile::parse_pylock_packages(pylock_path)
        .map_err(|e| SyncError::InvalidLockfile(format!("failed to parse pylock.toml: {e}")))?;

    let count = packages.len();

    for (name, version) in &packages {
        let spec = format!("{name}=={version}");
        let output = Command::new(pip_path)
            .args(["install", &spec, "--python", &env.python_path.to_string_lossy()])
            .output()?;

        if !output.status.success() {
            return Err(SyncError::ToolFailed {
                tool: format!("pip install {spec}"),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
    }

    Ok(SyncReport { packages_synced: count, tool_used: "pip install (per-package)".into() })
}
