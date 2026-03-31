//! Python environment discovery, installed package scanning, and dependency checking.
//!
//! Discovers the user's Python environment (venv, system Python) and checks whether
//! declared `[python-dependencies]` are installed at compatible versions. This is a
//! pre-compilation step that runs only in project mode (when `asatsuyu.toml` is present).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use pep440_rs::{Version, VersionSpecifiers};

// ── Types ────────────────────────────────────────────────────────────

/// A discovered Python environment.
#[derive(Debug)]
pub(crate) struct PythonEnvironment {
    /// Path to the Python interpreter (used by `cmd_run` to execute Python).
    #[allow(dead_code)]
    pub(crate) python_path: PathBuf,
    /// Path to the `site-packages` directory.
    pub(crate) site_packages: PathBuf,
    /// Python version (parsed from `pyvenv.cfg` or interpreter output).
    #[allow(dead_code)]
    pub(crate) version: Option<Version>,
    /// Whether this is a virtual environment.
    #[allow(dead_code)]
    pub(crate) is_venv: bool,
}

/// A Python package installed in the environment.
#[derive(Debug)]
pub(crate) struct InstalledPackage {
    /// Normalized package name (PEP 503).
    pub(crate) name: String,
    /// Installed version.
    pub(crate) version: Version,
}

/// Result of checking a single declared dependency.
#[derive(Debug)]
pub(crate) enum DependencyStatus {
    /// Package found and version satisfies specifier.
    Satisfied {
        #[allow(dead_code)]
        installed: Version,
    },
    /// Package found but version does not match specifier.
    VersionMismatch { installed: Version, required: String },
    /// Package not found in the environment.
    Missing,
}

// ── Name normalization (PEP 503) ─────────────────────────────────────

/// Normalize a package name per PEP 503.
///
/// Lowercases and replaces sequences of `[-_.]` with a single hyphen.
pub(crate) fn normalize_name(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let mut prev_was_separator = false;

    for ch in name.chars() {
        if matches!(ch, '-' | '_' | '.') {
            if !prev_was_separator && !result.is_empty() {
                result.push('-');
            }
            prev_was_separator = true;
        } else {
            prev_was_separator = false;
            result.push(ch.to_ascii_lowercase());
        }
    }

    // Trim trailing hyphen if the name ended with separators.
    if result.ends_with('-') {
        result.pop();
    }

    result
}

// ── METADATA parsing ─────────────────────────────────────────────────

/// Parse `Name` and `Version` from a `METADATA` file content (RFC 5322 headers).
///
/// Returns `None` if either field is missing or the version cannot be parsed.
fn parse_metadata(content: &str) -> Option<(String, Version)> {
    let mut name: Option<String> = None;
    let mut version: Option<Version> = None;

    for line in content.lines() {
        // Stop at the first blank line (end of headers).
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Name:") {
            name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("Version:") {
            version = Version::from_str(value.trim()).ok();
        }
        // Short-circuit once both are found.
        if name.is_some() && version.is_some() {
            break;
        }
    }

    Some((name?, version?))
}

// ── Package scanning ─────────────────────────────────────────────────

/// Scan installed packages from a `site-packages` directory.
///
/// Reads `*.dist-info/METADATA` files and extracts `Name` + `Version`.
/// Unreadable entries are silently skipped.
pub(crate) fn scan_installed_packages(site_packages: &Path) -> Vec<InstalledPackage> {
    let Ok(entries) = std::fs::read_dir(site_packages) else {
        return Vec::new();
    };

    let mut packages = Vec::new();

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        if !dir_name.ends_with(".dist-info") || !path.is_dir() {
            continue;
        }

        let metadata_path = path.join("METADATA");
        let Ok(content) = std::fs::read_to_string(&metadata_path) else {
            continue;
        };

        if let Some((raw_name, version)) = parse_metadata(&content) {
            packages.push(InstalledPackage { name: normalize_name(&raw_name), version });
        }
    }

    packages
}

// ── Dependency checking ──────────────────────────────────────────────

/// Check declared dependencies against installed packages.
///
/// Returns a map of package name → status for every entry in `[python-dependencies]`.
/// The specifiers are assumed to be valid PEP 440 (validated at config parse time).
pub(crate) fn check_dependencies(
    declared: &BTreeMap<String, String>,
    installed: &[InstalledPackage],
) -> BTreeMap<String, DependencyStatus> {
    let mut results = BTreeMap::new();

    for (name, specifier_str) in declared {
        let normalized = normalize_name(name);

        let found = installed.iter().find(|pkg| pkg.name == normalized);

        let status = match found {
            Some(pkg) => {
                // Specifier is pre-validated, but parse defensively.
                match VersionSpecifiers::from_str(specifier_str) {
                    Ok(specs) if specs.contains(&pkg.version) => {
                        DependencyStatus::Satisfied { installed: pkg.version.clone() }
                    }
                    Ok(_) => DependencyStatus::VersionMismatch {
                        installed: pkg.version.clone(),
                        required: specifier_str.clone(),
                    },
                    // Should not happen (validated at config load), treat as satisfied.
                    Err(_) => DependencyStatus::Satisfied { installed: pkg.version.clone() },
                }
            }
            None => DependencyStatus::Missing,
        };

        results.insert(name.clone(), status);
    }

    results
}

// ── Environment discovery ────────────────────────────────────────────

/// Find the `site-packages` directory for a venv root.
///
/// Tries POSIX layout (`lib/python{X.Y}/site-packages/`) first, then Windows
/// layout (`Lib/site-packages/`).
fn find_venv_site_packages(venv_root: &Path) -> Option<PathBuf> {
    // POSIX: lib/python{X.Y}/site-packages/
    let lib_dir = venv_root.join("lib");
    if lib_dir.is_dir()
        && let Ok(entries) = std::fs::read_dir(&lib_dir)
    {
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("python") {
                let sp = entry.path().join("site-packages");
                if sp.is_dir() {
                    return Some(sp);
                }
            }
        }
    }

    // Windows: Lib/site-packages/
    let win_sp = venv_root.join("Lib").join("site-packages");
    if win_sp.is_dir() {
        return Some(win_sp);
    }

    None
}

/// Query a Python interpreter for its site-packages path.
///
/// Runs: `python3 -c "import site; print(site.getsitepackages()[0])"`
fn query_site_packages(python_path: &Path) -> Option<PathBuf> {
    let output = Command::new(python_path)
        .args(["-c", "import site; print(site.getsitepackages()[0])"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let path_str = String::from_utf8_lossy(&output.stdout);
    let path = PathBuf::from(path_str.trim());
    if path.is_dir() { Some(path) } else { None }
}

/// Parse `pyvenv.cfg` for the Python version.
fn parse_pyvenv_version(venv_root: &Path) -> Option<Version> {
    let cfg_path = venv_root.join("pyvenv.cfg");
    let content = std::fs::read_to_string(cfg_path).ok()?;

    for line in content.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("version") {
            // Handle `version = 3.12.4` and `version=3.12.4`.
            let value = value.trim_start_matches([' ', '=']).trim();
            return Version::from_str(value).ok();
        }
    }

    None
}

/// Discover the active Python environment.
///
/// Search order:
/// 1. Explicit path (from `--python-path` or `[python] path` in config)
/// 2. `VIRTUAL_ENV` environment variable
/// 3. `.venv/` directory relative to project root
/// 4. `python3` on PATH
///
/// Returns `None` if no usable environment is found.
pub(crate) fn discover_environment(
    project_root: &Path,
    explicit_path: Option<&Path>,
) -> Option<PythonEnvironment> {
    // 1. Explicit path.
    if let Some(path) = explicit_path {
        return try_python_interpreter(path);
    }

    // 2. VIRTUAL_ENV environment variable.
    if let Ok(venv) = std::env::var("VIRTUAL_ENV") {
        let venv_path = PathBuf::from(&venv);
        if let Some(env) = try_venv_root(&venv_path) {
            return Some(env);
        }
    }

    // 3. .venv/ relative to project root.
    let dot_venv = project_root.join(".venv");
    if dot_venv.is_dir()
        && let Some(env) = try_venv_root(&dot_venv)
    {
        return Some(env);
    }

    // 4. python3 on PATH.
    try_python_on_path()
}

/// Try to construct a `PythonEnvironment` from a venv root directory.
fn try_venv_root(venv_root: &Path) -> Option<PythonEnvironment> {
    let site_packages = find_venv_site_packages(venv_root)?;
    let version = parse_pyvenv_version(venv_root);

    // Find the interpreter inside the venv.
    let python_path = if cfg!(windows) {
        venv_root.join("Scripts").join("python.exe")
    } else {
        venv_root.join("bin").join("python3")
    };

    if !python_path.is_file() {
        // Try plain `python` as fallback.
        let alt = if cfg!(windows) {
            venv_root.join("Scripts").join("python.exe")
        } else {
            venv_root.join("bin").join("python")
        };
        if !alt.is_file() {
            return None;
        }
        return Some(PythonEnvironment { python_path: alt, site_packages, version, is_venv: true });
    }

    Some(PythonEnvironment { python_path, site_packages, version, is_venv: true })
}

/// Try to construct a `PythonEnvironment` from an explicit interpreter path.
fn try_python_interpreter(python_path: &Path) -> Option<PythonEnvironment> {
    if !python_path.is_file() {
        return None;
    }
    let site_packages = query_site_packages(python_path)?;
    Some(PythonEnvironment {
        python_path: python_path.to_path_buf(),
        site_packages,
        version: None,
        is_venv: false,
    })
}

/// Try to find `python3` (or `python` on Windows) on PATH and build an environment.
fn try_python_on_path() -> Option<PythonEnvironment> {
    let cmd = if cfg!(windows) { "python" } else { "python3" };

    // Use `which`-style lookup via Command.
    let output = Command::new(cmd).args(["--version"]).output().ok()?;

    if !output.status.success() {
        return None;
    }

    // Resolve the full path.
    let full_path_output =
        Command::new(cmd).args(["-c", "import sys; print(sys.executable)"]).output().ok()?;

    if !full_path_output.status.success() {
        return None;
    }

    let python_path = PathBuf::from(String::from_utf8_lossy(&full_path_output.stdout).trim());
    let site_packages = query_site_packages(&python_path)?;

    Some(PythonEnvironment { python_path, site_packages, version: None, is_venv: false })
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── normalize_name ────────────────────────────────────────────

    #[test]
    fn normalize_name_lowercase() {
        assert_eq!(normalize_name("Foo-Bar"), "foo-bar");
    }

    #[test]
    fn normalize_name_underscores() {
        assert_eq!(normalize_name("foo_bar"), "foo-bar");
    }

    #[test]
    fn normalize_name_dots() {
        assert_eq!(normalize_name("FOO.BAR"), "foo-bar");
    }

    #[test]
    fn normalize_name_mixed_separators() {
        assert_eq!(normalize_name("My_Package.Name-V2"), "my-package-name-v2");
    }

    #[test]
    fn normalize_name_consecutive_separators() {
        assert_eq!(normalize_name("a__b..c--d"), "a-b-c-d");
    }

    #[test]
    fn normalize_name_simple() {
        assert_eq!(normalize_name("requests"), "requests");
    }

    // ── parse_metadata ────────────────────────────────────────────

    #[test]
    fn parse_metadata_valid() {
        let content = "Metadata-Version: 2.1\nName: requests\nVersion: 2.31.0\nSummary: HTTP\n";
        let (name, version) = parse_metadata(content).unwrap();
        assert_eq!(name, "requests");
        assert_eq!(version.to_string(), "2.31.0");
    }

    #[test]
    fn parse_metadata_missing_name() {
        let content = "Metadata-Version: 2.1\nVersion: 1.0\n";
        assert!(parse_metadata(content).is_none());
    }

    #[test]
    fn parse_metadata_missing_version() {
        let content = "Metadata-Version: 2.1\nName: foo\n";
        assert!(parse_metadata(content).is_none());
    }

    #[test]
    fn parse_metadata_stops_at_blank_line() {
        let content = "Name: foo\nVersion: 1.0\n\nName: bar\nVersion: 2.0\n";
        let (name, _) = parse_metadata(content).unwrap();
        assert_eq!(name, "foo");
    }

    // ── check_dependencies ────────────────────────────────────────

    fn make_pkg(name: &str, version: &str) -> InstalledPackage {
        InstalledPackage {
            name: normalize_name(name),
            version: Version::from_str(version).unwrap(),
        }
    }

    #[test]
    fn check_deps_all_satisfied() {
        let installed = vec![make_pkg("requests", "2.31.0"), make_pkg("flask", "3.0.1")];

        let mut declared = BTreeMap::new();
        declared.insert("requests".into(), ">=2.31".into());
        declared.insert("flask".into(), ">=3.0".into());

        let statuses = check_dependencies(&declared, &installed);
        assert!(matches!(statuses["requests"], DependencyStatus::Satisfied { .. }));
        assert!(matches!(statuses["flask"], DependencyStatus::Satisfied { .. }));
    }

    #[test]
    fn check_deps_version_mismatch() {
        let installed = vec![make_pkg("flask", "2.3.0")];

        let mut declared = BTreeMap::new();
        declared.insert("flask".into(), ">=3.0".into());

        let statuses = check_dependencies(&declared, &installed);
        assert!(matches!(statuses["flask"], DependencyStatus::VersionMismatch { .. }));
    }

    #[test]
    fn check_deps_missing() {
        let installed = vec![make_pkg("requests", "2.31.0")];

        let mut declared = BTreeMap::new();
        declared.insert("numpy".into(), ">=1.26".into());

        let statuses = check_dependencies(&declared, &installed);
        assert!(matches!(statuses["numpy"], DependencyStatus::Missing));
    }

    #[test]
    fn check_deps_name_normalization() {
        // Declared with underscore, installed with hyphen normalization.
        let installed = vec![make_pkg("Foo-Bar", "1.0.0")];

        let mut declared = BTreeMap::new();
        declared.insert("foo_bar".into(), ">=1.0".into());

        let statuses = check_dependencies(&declared, &installed);
        assert!(matches!(statuses["foo_bar"], DependencyStatus::Satisfied { .. }));
    }

    // ── scan_installed_packages ───────────────────────────────────

    fn create_temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("asatsuyu-pyenv-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn scan_installed_packages_with_dist_info() {
        let dir = create_temp_dir("scan-pkgs");

        // Create a dist-info directory with METADATA.
        let dist_info = dir.join("requests-2.31.0.dist-info");
        std::fs::create_dir_all(&dist_info).unwrap();
        std::fs::write(
            dist_info.join("METADATA"),
            "Metadata-Version: 2.1\nName: requests\nVersion: 2.31.0\n",
        )
        .unwrap();

        let packages = scan_installed_packages(&dir);
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "requests");
        assert_eq!(packages[0].version.to_string(), "2.31.0");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_installed_packages_empty_dir() {
        let dir = create_temp_dir("scan-empty");
        let packages = scan_installed_packages(&dir);
        assert!(packages.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_installed_packages_nonexistent_dir() {
        let packages = scan_installed_packages(Path::new("/nonexistent/path"));
        assert!(packages.is_empty());
    }

    #[test]
    fn scan_installed_packages_skips_non_dist_info() {
        let dir = create_temp_dir("scan-skip");

        // Regular directory (not .dist-info).
        let other = dir.join("some-package");
        std::fs::create_dir_all(&other).unwrap();

        // File that looks like dist-info but is not a directory.
        std::fs::write(dir.join("fake-1.0.dist-info"), "not a dir").unwrap();

        let packages = scan_installed_packages(&dir);
        assert!(packages.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
