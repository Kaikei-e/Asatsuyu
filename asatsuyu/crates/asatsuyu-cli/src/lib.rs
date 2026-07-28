//! Command-line interface for the Asatsuyu compiler.
//!
//! Provides `check`, `build`, `run`, and `new` subcommands that drive the
//! compilation pipeline from `.asty` source to Python 3.12+ output.

mod diagnostic_report;
mod json_diagnostic;
mod lockfile;
mod lsp;
mod project;
mod python_env;
mod sync;
mod toml_edit_util;
mod watch;

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitCode};
use std::str::FromStr;

use asatsuyu_backend_python::{FfiRuntimeMode, GeneratedPackage, PackageConfig};
// PackageConfig is constructed via build_package_config() helper below.
use asatsuyu_hir::ffi::FfiResolverConfig;
use asatsuyu_hir::purity::{EffectSource, Purity, PurityReport};
use asatsuyu_syntax::{Diagnostic, FileId, LineIndex, Severity};
use asatsuyu_ty::ThirModule;
use clap::{Parser, Subcommand, ValueEnum};

use crate::diagnostic_report::SourceDiagnostic;

// ── CLI definition ─────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "asatsuyu", version, about = "The Asatsuyu compiler")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// FFI runtime inclusion mode.
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum FfiRuntime {
    /// Always include the `PyO3` runtime extension
    On,
    /// Never include the runtime (pure Python prelude shim only)
    Off,
    /// Auto-detect from code (include when Checked FFI is used)
    #[default]
    Auto,
}

/// Diagnostic output format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub(crate) enum ErrorFormat {
    /// Human-readable output with source context (miette)
    #[default]
    Human,
    /// Machine-readable JSON (one object per line, NDJSON)
    Json,
}

/// Arguments shared across check/build/run for diagnostic output control.
#[derive(clap::Args, Clone, Debug)]
struct OutputArgs {
    /// Diagnostic output format: human (default) or json
    #[arg(long, value_enum, default_value_t = ErrorFormat::Human)]
    error_format: ErrorFormat,
}

/// Arguments shared across check/build/run for FFI configuration.
#[derive(clap::Args, Clone, Debug)]
struct FfiArgs {
    /// Restrict FFI to stdlib modules only (pathlib, json, os, sys)
    #[arg(long)]
    ffi_stdlib_only: bool,
    /// Additional directories for .pyi stub files
    #[arg(long)]
    ffi_stub_path: Vec<PathBuf>,
}

/// Arguments shared across check/build/run for Python environment configuration.
#[derive(clap::Args, Clone, Debug)]
struct PythonArgs {
    /// Path to Python interpreter (overrides environment discovery)
    #[arg(long)]
    python_path: Option<PathBuf>,
}

// ── Exit codes ────────────────────────────────────────────────────
//
// Convention (following ruff/ty):
//   0 — success
//   1 — compilation or semantic errors found
//   2 — invalid configuration, CLI arguments, or I/O error

fn exit_compile_error() -> ExitCode {
    ExitCode::from(1)
}

fn exit_config_error() -> ExitCode {
    ExitCode::from(2)
}

#[derive(Subcommand)]
enum Commands {
    /// Type-check Asatsuyu source files without code generation
    Check {
        /// Paths to `.asty` source files (optional when inside a project)
        paths: Vec<PathBuf>,
        /// Watch for file changes and re-check automatically
        #[arg(long)]
        watch: bool,
        /// Print each function's purity and why it is effectful
        #[arg(long)]
        purity: bool,
        #[command(flatten)]
        output: OutputArgs,
        #[command(flatten)]
        ffi: FfiArgs,
        #[command(flatten)]
        python: PythonArgs,
    },
    /// Compile Asatsuyu source to a Python 3.12+ package
    Build {
        /// Path to the .asty source file
        path: PathBuf,
        /// Output directory
        #[arg(short, long, default_value = "dist")]
        output_dir: PathBuf,
        /// Add source-map comments (# asty:L<n>) to generated Python
        #[arg(long)]
        source_map: bool,
        /// Skip full package generation; emit only the .py module file
        #[arg(long)]
        no_emit_package: bool,
        /// Control FFI runtime inclusion: on, off, or auto
        #[arg(long, value_enum, default_value_t = FfiRuntime::Auto)]
        ffi_runtime: FfiRuntime,
        #[command(flatten)]
        output: OutputArgs,
        #[command(flatten)]
        ffi: FfiArgs,
        #[command(flatten)]
        python: PythonArgs,
    },
    /// Compile and execute an Asatsuyu source file with python3
    Run {
        /// Path to the .asty source file
        path: PathBuf,
        /// Add source-map comments (# asty:L<n>) to generated Python
        #[arg(long)]
        source_map: bool,
        /// Control FFI runtime inclusion: on, off, or auto
        #[arg(long, value_enum, default_value_t = FfiRuntime::Auto)]
        ffi_runtime: FfiRuntime,
        #[command(flatten)]
        output: OutputArgs,
        #[command(flatten)]
        ffi: FfiArgs,
        #[command(flatten)]
        python: PythonArgs,
    },
    /// Create a new Asatsuyu project
    New {
        /// Project name (used as directory name)
        name: String,
    },
    /// Generate a reproducible pylock.toml lockfile from declared dependencies
    Lock {
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Add a Python dependency to asatsuyu.toml and re-lock
    Add {
        /// Package name (e.g., "requests")
        package: String,
        /// PEP 440 version specifier (e.g., ">=2.31"). Defaults to ">=0" (any version).
        #[arg(default_value = ">=0")]
        specifier: String,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Remove a Python dependency from asatsuyu.toml and re-lock
    Remove {
        /// Package name to remove
        package: String,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Sync Python environment from pylock.toml
    Sync {
        #[command(flatten)]
        output: OutputArgs,
        #[command(flatten)]
        python: PythonArgs,
    },
    /// Start the Language Server Protocol server (stdio transport)
    Lsp,
    /// Format Asatsuyu source files
    Fmt {
        /// Paths to `.asty` source files (optional when inside a project)
        paths: Vec<PathBuf>,
        /// Check if files are already formatted (exit 1 if not, for CI)
        #[arg(long)]
        check: bool,
    },
    /// Show FFI trust report for all known Python modules
    #[command(name = "verify-ffi")]
    VerifyFfi,
}

// ── Entry point ────────────────────────────────────────────────────

/// Run the CLI, returning an appropriate exit code.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn run() -> ExitCode {
    // Configure miette for graphical diagnostic output.
    miette::set_hook(Box::new(|_| {
        Box::new(
            miette::MietteHandlerOpts::new()
                .terminal_links(false)
                .context_lines(2)
                .tab_width(2)
                .build(),
        )
    }))
    .ok(); // ok() — set_hook fails if called twice (e.g., in tests)

    let cli = Cli::parse();
    match cli.command {
        Commands::Check { paths, watch, purity, output, ffi, python } => {
            let ffi_config = match build_ffi_config(&ffi, &python) {
                Ok(config) => config,
                Err(err) => {
                    eprintln!("error: {err}");
                    return exit_config_error();
                }
            };
            let context = match resolve_check_context(&paths) {
                Ok(ctx) => ctx,
                Err(err) => {
                    eprintln!("error: {err}");
                    return exit_config_error();
                }
            };

            // Dependency check (project mode only, warnings).
            run_dependency_check(
                context.project.as_ref(),
                python.python_path.as_deref(),
                output.error_format,
                false,
            );
            check_lockfile_staleness(context.project.as_ref(), output.error_format);

            if watch {
                watch::run_watch(&context.paths, &ffi_config, output.error_format)
            } else {
                cmd_check(&context.paths, &ffi_config, output.error_format, purity)
            }
        }
        Commands::Build {
            path,
            output_dir,
            source_map,
            no_emit_package,
            ffi_runtime,
            output,
            ffi,
            python,
        } => {
            let ffi_config = match build_ffi_config(&ffi, &python) {
                Ok(config) => config,
                Err(err) => {
                    eprintln!("error: {err}");
                    return exit_config_error();
                }
            };

            // Attempt project discovery from file's parent for dep check.
            let discovered =
                path.parent().and_then(|dir| project::discover_project(dir).ok().flatten());
            run_dependency_check(
                discovered.as_ref(),
                python.python_path.as_deref(),
                output.error_format,
                false,
            );
            check_lockfile_staleness(discovered.as_ref(), output.error_format);

            let runtime_mode = convert_ffi_runtime(ffi_runtime);
            cmd_build(
                &path,
                &output_dir,
                source_map,
                no_emit_package,
                runtime_mode,
                &ffi_config,
                output.error_format,
                discovered.as_ref(),
            )
        }
        Commands::Run { path, source_map, ffi_runtime, output, ffi, python } => {
            let ffi_config = match build_ffi_config(&ffi, &python) {
                Ok(config) => config,
                Err(err) => {
                    eprintln!("error: {err}");
                    return exit_config_error();
                }
            };

            // Attempt project discovery from file's parent for dep check.
            // For `run`, missing deps are errors (exit code 2).
            let discovered =
                path.parent().and_then(|dir| project::discover_project(dir).ok().flatten());
            if run_dependency_check(
                discovered.as_ref(),
                python.python_path.as_deref(),
                output.error_format,
                true,
            ) {
                return exit_config_error();
            }
            check_lockfile_staleness(discovered.as_ref(), output.error_format);

            let runtime_mode = convert_ffi_runtime(ffi_runtime);
            cmd_run(
                &path,
                source_map,
                runtime_mode,
                &ffi_config,
                output.error_format,
                discovered.as_ref(),
            )
        }
        Commands::Lock { output } => cmd_lock(output.error_format),
        Commands::Add { package, specifier, output } => {
            cmd_add(&package, &specifier, output.error_format)
        }
        Commands::Remove { package, output } => cmd_remove(&package, output.error_format),
        Commands::Sync { output, python } => {
            cmd_sync(output.error_format, python.python_path.as_deref())
        }
        Commands::New { name } => cmd_new(&name),
        Commands::Lsp => {
            lsp::start_lsp();
            ExitCode::SUCCESS
        }
        Commands::Fmt { paths, check } => {
            let resolved = match resolve_fmt_paths(&paths) {
                Ok(p) => p,
                Err(err) => {
                    eprintln!("error: {err}");
                    return exit_config_error();
                }
            };
            cmd_fmt(&resolved, check)
        }
        Commands::VerifyFfi => cmd_verify_ffi(),
    }
}

// ── FFI config helpers ────────────────────────────────────────────

fn build_ffi_config(ffi: &FfiArgs, python: &PythonArgs) -> Result<FfiResolverConfig, CliError> {
    for path in &ffi.ffi_stub_path {
        if !path.exists() {
            return Err(CliError::InvalidFfiStubPath {
                path: path.clone(),
                reason: "directory does not exist",
            });
        }
        if !path.is_dir() {
            return Err(CliError::InvalidFfiStubPath {
                path: path.clone(),
                reason: "path is not a directory",
            });
        }
    }

    Ok(FfiResolverConfig {
        stdlib_only: ffi.ffi_stdlib_only,
        stub_paths: ffi.ffi_stub_path.clone(),
        typeshed_path: None,
        python_path: python.python_path.clone(),
    })
}

fn convert_ffi_runtime(runtime: FfiRuntime) -> FfiRuntimeMode {
    match runtime {
        FfiRuntime::On => FfiRuntimeMode::On,
        FfiRuntime::Off => FfiRuntimeMode::Off,
        FfiRuntime::Auto => FfiRuntimeMode::Auto,
    }
}

/// Build a `PackageConfig` from CLI arguments and optional project discovery.
///
/// When a project is discovered, uses its `name`, `version`, `python_version`,
/// and `python_dependencies`. Falls back to file stem and defaults otherwise.
fn build_package_config(
    file_stem: &str,
    source_map: bool,
    ffi_runtime: FfiRuntimeMode,
    project: Option<&project::Project>,
) -> PackageConfig {
    let (name, version, requires_python, dependencies) = if let Some(proj) = project {
        let cfg = &proj.config;
        (
            cfg.name().to_string(),
            cfg.version().to_string(),
            cfg.python_version().map(String::from),
            format_pep508_deps(cfg.python_dependencies()),
        )
    } else {
        (file_stem.to_string(), "0.1.0".into(), None, Vec::new())
    };

    PackageConfig { name, version, source_map, ffi_runtime, requires_python, dependencies }
}

/// Convert `asatsuyu.toml` dependencies (`name → specifier`) to PEP 508 strings.
///
/// E.g., `("requests", ">=2.31")` → `"requests>=2.31"`.
fn format_pep508_deps(deps: &BTreeMap<String, String>) -> Vec<String> {
    deps.iter().map(|(name, spec)| format!("{name}{spec}")).collect()
}

// ── Command handlers ───────────────────────────────────────────────

pub(crate) fn cmd_check(
    paths: &[PathBuf],
    ffi_config: &FfiResolverConfig,
    error_format: ErrorFormat,
    show_purity: bool,
) -> ExitCode {
    let mut compile_failed = false;
    let mut config_failed = false;
    let mut all_diags: Vec<Diagnostic> = Vec::new();

    for path in paths {
        let filename = path.display().to_string();
        match compile_with_source(path, ffi_config) {
            Ok(result) => {
                if show_purity {
                    emit_purity_report(&result.purity, &filename, error_format);
                }
                if !result.warnings.is_empty() {
                    report_diagnostics(&result.warnings, &result.source, &filename, error_format);
                    all_diags.extend(result.warnings);
                }
            }
            Err(CliError::CompileErrors { diagnostics, source }) => {
                report_diagnostics(&diagnostics, &source, &filename, error_format);
                all_diags.extend(diagnostics);
                compile_failed = true;
            }
            Err(err) => {
                eprintln!("error: {err}");
                config_failed = true;
            }
        }
    }

    emit_final_summary(&all_diags, error_format);

    if compile_failed {
        exit_compile_error()
    } else if config_failed {
        exit_config_error()
    } else {
        ExitCode::SUCCESS
    }
}

#[allow(clippy::too_many_arguments)]
fn cmd_build(
    path: &Path,
    output_dir: &Path,
    source_map: bool,
    no_emit_package: bool,
    ffi_runtime: FfiRuntimeMode,
    ffi_config: &FfiResolverConfig,
    error_format: ErrorFormat,
    discovered_project: Option<&project::Project>,
) -> ExitCode {
    let filename = path.display().to_string();
    let result = match compile_with_source(path, ffi_config) {
        Ok(result) => result,
        Err(CliError::CompileErrors { diagnostics, source }) => {
            report_diagnostics(&diagnostics, &source, &filename, error_format);
            emit_final_summary(&diagnostics, error_format);
            return exit_compile_error();
        }
        Err(err) => {
            eprintln!("error: {err}");
            return exit_config_error();
        }
    };

    if !result.warnings.is_empty() {
        report_diagnostics(&result.warnings, &result.source, &filename, error_format);
    }

    let stem = path.file_stem().unwrap_or_default().to_string_lossy();

    // --no-emit-package: emit only the .py module file, no package structure.
    if no_emit_package {
        let py = asatsuyu_backend_python::emit_module(&result.module);
        let out_path = output_dir.join(format!("{stem}.py"));
        if let Some(parent) = out_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&out_path, &py) {
            eprintln!("error: cannot write {}: {e}", out_path.display());
            return exit_config_error();
        }
        println!("{}", out_path.display());
        if matches!(error_format, ErrorFormat::Human) {
            eprintln!("  Compiled {stem} (module only) → {}", out_path.display());
        }
        emit_final_summary(&result.warnings, error_format);
        return ExitCode::SUCCESS;
    }

    let config = build_package_config(&stem, source_map, ffi_runtime, discovered_project);
    let package =
        asatsuyu_backend_python::emit_package(&result.module, &config, Some(&result.source));

    // Clean generated package directories before writing.
    let python_root = output_dir.join("python");
    let pkg_dir_name = asatsuyu_backend_python::python_package_name(&config.name);
    let legacy_pkg_dir = python_root.join(stem.as_ref());
    if legacy_pkg_dir.exists() {
        let _ = std::fs::remove_dir_all(&legacy_pkg_dir);
    }
    let pkg_dir = python_root.join(&pkg_dir_name);
    if pkg_dir.exists() {
        let _ = std::fs::remove_dir_all(&pkg_dir);
    }

    if let Err(msg) = write_package(&package, output_dir) {
        eprintln!("error: {msg}");
        return exit_config_error();
    }

    // stdout: output directory (for scripting).
    println!("{}", output_dir.display());
    if matches!(error_format, ErrorFormat::Human) {
        // stderr: human-readable progress.
        eprintln!("  Compiled {stem} ({} files) → {}", package.files.len(), output_dir.display());
    }
    emit_final_summary(&result.warnings, error_format);
    ExitCode::SUCCESS
}

fn cmd_run(
    path: &Path,
    source_map: bool,
    ffi_runtime: FfiRuntimeMode,
    ffi_config: &FfiResolverConfig,
    error_format: ErrorFormat,
    discovered_project: Option<&project::Project>,
) -> ExitCode {
    let filename = path.display().to_string();
    let result = match compile_with_source(path, ffi_config) {
        Ok(result) => result,
        Err(CliError::CompileErrors { diagnostics, source }) => {
            report_diagnostics(&diagnostics, &source, &filename, error_format);
            emit_final_summary(&diagnostics, error_format);
            return exit_compile_error();
        }
        Err(err) => {
            eprintln!("error: {err}");
            return exit_config_error();
        }
    };

    if !result.warnings.is_empty() {
        report_diagnostics(&result.warnings, &result.source, &filename, error_format);
    }

    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let output_dir = PathBuf::from("target/run");
    let config = build_package_config(&stem, source_map, ffi_runtime, discovered_project);
    let package =
        asatsuyu_backend_python::emit_package(&result.module, &config, Some(&result.source));

    // Clean generated package directories, but preserve helper stubs such as
    // `target/run/python/requests.py` used by tests and local workflows.
    let pkg_dir_name = asatsuyu_backend_python::python_package_name(&config.name);
    let python_root = output_dir.join("python");
    let legacy_pkg_dir = python_root.join(stem.as_ref());
    if legacy_pkg_dir.exists() {
        let _ = std::fs::remove_dir_all(&legacy_pkg_dir);
    }
    let pkg_dir = python_root.join(&pkg_dir_name);
    if pkg_dir.exists() {
        let _ = std::fs::remove_dir_all(&pkg_dir);
    }

    if let Err(msg) = write_package(&package, &output_dir) {
        eprintln!("error: {msg}");
        return exit_config_error();
    }

    // Emit diagnostic summary before executing Python.
    emit_final_summary(&result.warnings, error_format);

    // Execute with python3.
    let has_main = result
        .module
        .functions
        .iter()
        .any(|f| result.module.symbol_table.get(f.def_id).name.as_str() == "main");

    // Python source lives under python/ in the new layout.
    // When inside a project, the package name comes from the config (e.g., "myapp"),
    // not the file stem (e.g., "main"). Use pkg_dir_name for project mode.
    let python_dir = output_dir.join("python");
    let python_dir = match python_dir.canonicalize() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!(
                "error: cannot resolve generated python directory {}: {e}",
                python_dir.display()
            );
            return exit_config_error();
        }
    };
    let module_name = pkg_dir_name.clone();
    let status_result = if has_main {
        let python_path = match std::env::var_os("PYTHONPATH") {
            Some(existing) if !existing.is_empty() => {
                let mut joined = python_dir.into_os_string();
                joined.push(std::ffi::OsStr::new(":"));
                joined.push(existing);
                joined
            }
            _ => python_dir.into_os_string(),
        };
        Command::new("python3").env("PYTHONPATH", python_path).arg("-m").arg(&module_name).status()
    } else {
        let py_path = python_dir.join(format!("{module_name}/{module_name}.py"));
        Command::new("python3").arg(&py_path).status()
    };

    match status_result {
        Ok(status) => {
            if status.success() {
                ExitCode::SUCCESS
            } else {
                // Propagate python's exit code (clamped to u8 range).
                let code = status.code().unwrap_or(1).clamp(1, 255);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                ExitCode::from(code as u8)
            }
        }
        Err(e) => {
            eprintln!("error: cannot execute python3: {e}");
            exit_config_error()
        }
    }
}

// ── lock ─────────────────────────────────────────────────────────

fn cmd_lock(error_format: ErrorFormat) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(e) => {
            emit_lock_event(
                error_format,
                "error",
                serde_json::json!({ "message": format!("cannot determine working directory: {e}") }),
            );
            return exit_config_error();
        }
    };

    let project = match project::discover_project(&cwd) {
        Ok(Some(p)) => p,
        Ok(None) => {
            emit_lock_event(
                error_format,
                "error",
                serde_json::json!({ "message": "no asatsuyu.toml found (run from inside a project)" }),
            );
            return exit_config_error();
        }
        Err(e) => {
            emit_lock_event(error_format, "error", serde_json::json!({ "message": e.to_string() }));
            return exit_config_error();
        }
    };

    if project.config.python_dependencies().is_empty() {
        emit_lock_event(
            error_format,
            "skipped",
            serde_json::json!({ "reason": "no_dependencies", "message": "no [python-dependencies] declared; nothing to lock" }),
        );
        return ExitCode::SUCCESS;
    }

    let Some(tool) = lockfile::discover_lock_tool() else {
        emit_lock_event(
            error_format,
            "error",
            serde_json::json!({ "message": lockfile::LockError::ToolNotFound.to_string() }),
        );
        return exit_config_error();
    };

    let output_path = project.root.join("pylock.toml");
    let dep_count = project.config.python_dependencies().len();

    match lockfile::generate_lockfile(&project, &tool, &output_path) {
        Ok(()) => {
            emit_lock_event(
                error_format,
                "generated",
                serde_json::json!({
                    "path": output_path.display().to_string(),
                    "tool": tool.to_string(),
                    "dependency_count": dep_count,
                }),
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            emit_lock_event(error_format, "error", serde_json::json!({ "message": e.to_string() }));
            exit_config_error()
        }
    }
}

fn emit_lock_event(error_format: ErrorFormat, status: &str, extra: serde_json::Value) {
    match error_format {
        ErrorFormat::Human => match status {
            "generated" => {
                let dep_count =
                    extra.get("dependency_count").and_then(serde_json::Value::as_u64).unwrap_or(0);
                let tool = extra.get("tool").and_then(serde_json::Value::as_str).unwrap_or("tool");
                let path = extra.get("path").and_then(serde_json::Value::as_str).unwrap_or("");
                eprintln!(
                    "  Locked {dep_count} dependenc{} via {tool}",
                    if dep_count == 1 { "y" } else { "ies" }
                );
                eprintln!("  Output: {path}");
            }
            "skipped" => {
                let message = extra
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("lock skipped");
                eprintln!("warning: {message}");
            }
            _ => {
                let message = extra
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("lock failed");
                eprintln!("error: {message}");
            }
        },
        ErrorFormat::Json => {
            let mut obj = serde_json::Map::new();
            obj.insert("type".into(), serde_json::Value::String("lockfile".into()));
            obj.insert("status".into(), serde_json::Value::String(status.into()));
            if let serde_json::Value::Object(extra_obj) = extra {
                obj.extend(extra_obj);
            }
            println!("{}", serde_json::Value::Object(obj));
        }
    }
}

/// Emit a warning if `pylock.toml` is stale or missing.
///
/// Called from `cmd_check`, `cmd_build`, and `cmd_run` after dependency checks.
fn check_lockfile_staleness(project: Option<&project::Project>, error_format: ErrorFormat) {
    let Some(project) = project else { return };
    if project.config.python_dependencies().is_empty() {
        return;
    }

    let pylock_path = project.root.join("pylock.toml");
    match lockfile::check_staleness(project, &pylock_path) {
        lockfile::LockStaleness::Fresh => {}
        lockfile::LockStaleness::Missing => {
            if error_format == ErrorFormat::Human {
                eprintln!("warning: no pylock.toml found; run `asatsuyu lock` to create one");
            } else {
                let json = serde_json::json!({
                    "type": "lockfile",
                    "status": "missing",
                    "message": "no pylock.toml found; run `asatsuyu lock` to create one",
                });
                println!("{json}");
            }
        }
        lockfile::LockStaleness::Stale { reason } => {
            if error_format == ErrorFormat::Human {
                eprintln!("warning: pylock.toml may be stale: {reason}");
                eprintln!("  hint: run `asatsuyu lock` to update");
            } else {
                let json = serde_json::json!({
                    "type": "lockfile",
                    "status": "stale",
                    "reason": reason,
                });
                println!("{json}");
            }
        }
    }
}

// ── add ─────────────────────────────────────────────────────────

fn cmd_add(package: &str, specifier: &str, error_format: ErrorFormat) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(e) => {
            eprintln!("error: cannot determine working directory: {e}");
            return exit_config_error();
        }
    };

    let project = match project::discover_project(&cwd) {
        Ok(Some(p)) => p,
        Ok(None) => {
            eprintln!("error: no asatsuyu.toml found (run from inside a project)");
            return exit_config_error();
        }
        Err(e) => {
            eprintln!("error: {e}");
            return exit_config_error();
        }
    };

    // Validate PEP 440 specifier.
    if pep440_rs::VersionSpecifiers::from_str(specifier).is_err() {
        eprintln!("error: invalid PEP 440 specifier \"{specifier}\"");
        return exit_config_error();
    }

    let toml_path = project.root.join("asatsuyu.toml");
    match toml_edit_util::add_dependency(&toml_path, package, specifier) {
        Ok(prev) => {
            if error_format == ErrorFormat::Human {
                if let Some(old) = &prev {
                    eprintln!("  Updated {package} \"{old}\" → \"{specifier}\"");
                } else {
                    eprintln!("  Added {package} \"{specifier}\" to [python-dependencies]");
                }
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            return exit_config_error();
        }
    }

    // Validate the edited config still parses.
    let content = match std::fs::read_to_string(&toml_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: cannot re-read asatsuyu.toml: {e}");
            return exit_config_error();
        }
    };
    if let Err(e) = project::parse_config(&content) {
        eprintln!("error: asatsuyu.toml is invalid after edit: {e}");
        return exit_config_error();
    }

    // Re-lock (best-effort).
    attempt_relock(&project.root, error_format);

    ExitCode::SUCCESS
}

// ── remove ──────────────────────────────────────────────────────

fn cmd_remove(package: &str, error_format: ErrorFormat) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(e) => {
            eprintln!("error: cannot determine working directory: {e}");
            return exit_config_error();
        }
    };

    let project = match project::discover_project(&cwd) {
        Ok(Some(p)) => p,
        Ok(None) => {
            eprintln!("error: no asatsuyu.toml found (run from inside a project)");
            return exit_config_error();
        }
        Err(e) => {
            eprintln!("error: {e}");
            return exit_config_error();
        }
    };

    let toml_path = project.root.join("asatsuyu.toml");
    match toml_edit_util::remove_dependency(&toml_path, package) {
        Ok(Some(old_spec)) => {
            if error_format == ErrorFormat::Human {
                eprintln!("  Removed {package} \"{old_spec}\" from [python-dependencies]");
            }
        }
        Ok(None) => {
            eprintln!("error: {package} is not in [python-dependencies]");
            return exit_config_error();
        }
        Err(e) => {
            eprintln!("error: {e}");
            return exit_config_error();
        }
    }

    // Re-read config to check if deps remain.
    let content = match std::fs::read_to_string(&toml_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: cannot re-read asatsuyu.toml: {e}");
            return exit_config_error();
        }
    };
    let config = match project::parse_config(&content) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: asatsuyu.toml is invalid after edit: {e}");
            return exit_config_error();
        }
    };

    let pylock_path = project.root.join("pylock.toml");
    if config.python_dependencies().is_empty() {
        // No deps left — remove stale lockfile.
        if pylock_path.exists() {
            let _ = std::fs::remove_file(&pylock_path);
            if error_format == ErrorFormat::Human {
                eprintln!("  Removed pylock.toml (no dependencies remain)");
            }
        }
    } else {
        attempt_relock(&project.root, error_format);
    }

    ExitCode::SUCCESS
}

// ── sync ────────────────────────────────────────────────────────

fn cmd_sync(error_format: ErrorFormat, explicit_python_path: Option<&Path>) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(e) => {
            eprintln!("error: cannot determine working directory: {e}");
            return exit_config_error();
        }
    };

    let project = match project::discover_project(&cwd) {
        Ok(Some(p)) => p,
        Ok(None) => {
            eprintln!("error: no asatsuyu.toml found (run from inside a project)");
            return exit_config_error();
        }
        Err(e) => {
            eprintln!("error: {e}");
            return exit_config_error();
        }
    };

    let pylock_path = project.root.join("pylock.toml");
    if !pylock_path.exists() {
        eprintln!("error: pylock.toml not found");
        eprintln!("  hint: run `asatsuyu lock` to generate it");
        return exit_config_error();
    }

    // Warn if stale.
    check_lockfile_staleness(Some(&project), error_format);

    // Discover Python environment.
    let python_path = explicit_python_path.or_else(|| project.config.python_path());
    let Some(env) = python_env::discover_environment(&project.root, python_path) else {
        eprintln!("error: no Python environment found");
        eprintln!("  hint: create a venv with `python3 -m venv .venv`");
        return exit_config_error();
    };

    let Some(tool) = sync::discover_sync_tool() else {
        eprintln!("error: {}", sync::SyncError::ToolNotFound);
        return exit_config_error();
    };

    match sync::sync_environment(&pylock_path, &env, &tool) {
        Ok(report) => {
            if error_format == ErrorFormat::Human {
                eprintln!(
                    "  Synced {} package{} via {}",
                    report.packages_synced,
                    if report.packages_synced == 1 { "" } else { "s" },
                    report.tool_used,
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            exit_config_error()
        }
    }
}

/// Attempt to re-generate `pylock.toml` after editing `asatsuyu.toml`.
///
/// This is best-effort: if no lock tool is available or locking fails,
/// we emit a warning but do not fail the overall operation.
fn attempt_relock(project_root: &Path, error_format: ErrorFormat) {
    let toml_path = project_root.join("asatsuyu.toml");
    let Ok(content) = std::fs::read_to_string(&toml_path) else {
        return;
    };
    let Ok(config) = project::parse_config(&content) else {
        return;
    };

    if config.python_dependencies().is_empty() {
        return;
    }

    let project = project::Project { root: project_root.to_path_buf(), config };
    let Some(tool) = lockfile::discover_lock_tool() else {
        if error_format == ErrorFormat::Human {
            eprintln!(
                "  warning: could not re-lock (no lock tool found; install uv >= 0.6.15 or pip >= 25.1)"
            );
        }
        return;
    };

    let pylock_path = project_root.join("pylock.toml");
    match lockfile::generate_lockfile(&project, &tool, &pylock_path) {
        Ok(()) => {
            if error_format == ErrorFormat::Human {
                eprintln!(
                    "  Locked {} dependencies via {tool}",
                    project.config.python_dependencies().len()
                );
            }
        }
        Err(e) => {
            if error_format == ErrorFormat::Human {
                eprintln!("  warning: re-lock failed: {e}");
            }
        }
    }
}

// ── new ──────────────────────────────────────────────────────────

fn cmd_new(name: &str) -> ExitCode {
    // Validate project name.
    if name.is_empty() {
        eprintln!("error: project name cannot be empty");
        return exit_config_error();
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        eprintln!("error: project name must contain only ASCII letters, digits, and underscores");
        return exit_config_error();
    }

    let project_dir = Path::new(name);
    if project_dir.exists() {
        eprintln!("error: directory `{name}` already exists");
        return exit_config_error();
    }

    // Create directory structure.
    let src_dir = project_dir.join("src");
    if let Err(e) = std::fs::create_dir_all(&src_dir) {
        eprintln!("error: cannot create directory: {e}");
        return exit_config_error();
    }

    // src/main.asty
    let main_asty = "pub fn main() {\n  42\n}\n";
    if let Err(e) = std::fs::write(src_dir.join("main.asty"), main_asty) {
        eprintln!("error: cannot write main.asty: {e}");
        return exit_config_error();
    }

    // asatsuyu.toml
    let toml = format!(
        "schema_version = 1\n\n[project]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n[python]\nversion = \">=3.12\"\n",
    );
    if let Err(e) = std::fs::write(project_dir.join("asatsuyu.toml"), toml) {
        eprintln!("error: cannot write asatsuyu.toml: {e}");
        return exit_config_error();
    }

    // .gitignore
    let gitignore = "/dist/\n/target/\n__pycache__/\n*.pyc\n";
    if let Err(e) = std::fs::write(project_dir.join(".gitignore"), gitignore) {
        eprintln!("error: cannot write .gitignore: {e}");
        return exit_config_error();
    }

    eprintln!("  Created project `{name}` in ./{name}");
    eprintln!("  Run `asatsuyu run {name}/src/main.asty` to get started");
    ExitCode::SUCCESS
}

// ── fmt ──────────────────────────────────────────────────────────

/// Resolve paths for the `fmt` command, reusing the check-context pattern.
fn resolve_fmt_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, CliError> {
    if !paths.is_empty() {
        return Ok(paths.to_vec());
    }
    let cwd = std::env::current_dir().map_err(CliError::Io)?;
    let proj =
        project::discover_project(&cwd).map_err(CliError::Project)?.ok_or(CliError::NoProject)?;
    project::discover_sources(&proj.root).map_err(CliError::Project)
}

fn cmd_fmt(paths: &[PathBuf], check_mode: bool) -> ExitCode {
    let mut formatted_count: u32 = 0;
    let mut unchanged_count: u32 = 0;
    let mut error_paths: Vec<PathBuf> = Vec::new();

    for path in paths {
        let source = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(err) => {
                eprintln!("error: cannot read {}: {err}", path.display());
                return exit_config_error();
            }
        };

        let result = asatsuyu_parser::format_source(&source);

        if result.has_parse_errors {
            // Skip files with parse errors (print warning).
            eprintln!("warning: {} has parse errors, skipping", path.display());
            continue;
        }

        if result.formatted == source {
            unchanged_count += 1;
            continue;
        }

        if check_mode {
            error_paths.push(path.clone());
        } else {
            if let Err(err) = std::fs::write(path, &result.formatted) {
                eprintln!("error: cannot write {}: {err}", path.display());
                return exit_config_error();
            }
            formatted_count += 1;
        }
    }

    if check_mode {
        if error_paths.is_empty() {
            eprintln!("{unchanged_count} file(s) already formatted");
            ExitCode::SUCCESS
        } else {
            for p in &error_paths {
                eprintln!("would reformat: {}", p.display());
            }
            eprintln!(
                "{} file(s) would be reformatted, {} already formatted",
                error_paths.len(),
                unchanged_count
            );
            exit_compile_error()
        }
    } else {
        let total = formatted_count + unchanged_count;
        if formatted_count > 0 {
            eprintln!(
                "Formatted {formatted_count} file(s) ({unchanged_count} unchanged, {total} total)"
            );
        } else {
            eprintln!("{total} file(s) already formatted");
        }
        ExitCode::SUCCESS
    }
}

// ── verify-ffi ────────────────────────────────────────────────────

fn cmd_verify_ffi() -> ExitCode {
    use asatsuyu_hir::ffi::{ChainResolver, FfiSymbolKind, FfiTrustLevel};
    use std::fmt::Write;

    let resolver = ChainResolver::new();
    let modules = resolver.verify_all();

    let mut out = String::new();
    let mut verified_count = 0u32;
    let mut checked_count = 0u32;
    let mut unsafe_count = 0u32;

    let _ = writeln!(out, "FFI Trust Report");
    let _ = writeln!(out, "================\n");

    for module in &modules {
        let trust = module.trust_level;
        match trust {
            FfiTrustLevel::Verified => verified_count += 1,
            FfiTrustLevel::Checked => checked_count += 1,
            FfiTrustLevel::Unsafe => unsafe_count += 1,
        }

        let source = format!("{:?}", module.source);
        let _ = writeln!(out, "{} ({source}) ... {trust:?}", module.name);

        for sym in &module.symbols {
            let sym_trust = sym.trust_level.unwrap_or(trust);
            let kind_label = match &sym.kind {
                FfiSymbolKind::Function(_) => "Function",
                FfiSymbolKind::Class(_) => "Class",
                FfiSymbolKind::Constant(_) => "Constant",
            };
            let _ = writeln!(out, "  {} ({kind_label}) ... {sym_trust:?}", sym.name);

            // Show class members for detail.
            if let FfiSymbolKind::Class(cls) = &sym.kind {
                for (prop_name, prop_ty) in &cls.properties {
                    let _ = writeln!(out, "    {prop_name}: {prop_ty:?}");
                }
                for (method_name, sig) in &cls.methods {
                    let ret = format!("{:?}", sig.return_ty);
                    let _ = writeln!(out, "    {method_name}() -> {ret}");
                }
            }
        }
        out.push('\n');
    }

    let _ = writeln!(
        out,
        "Summary: {verified_count} Verified, {checked_count} Checked, {unsafe_count} Unsafe"
    );

    // stdout: structured report (for scripting).
    print!("{out}");
    ExitCode::SUCCESS
}

// ── Package writing ───────────────────────────────────────────────

fn write_package(package: &GeneratedPackage, output_dir: &Path) -> Result<(), String> {
    for file in &package.files {
        let out_path = output_dir.join(&file.path);
        if let Some(parent) = out_path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            return Err(format!("cannot create directory: {e}"));
        }
        let content = if file.path == Path::new("Cargo.toml") {
            materialize_runtime_crate_path(&file.content, output_dir)?
        } else {
            file.content.clone()
        };
        if let Err(e) = std::fs::write(&out_path, content) {
            return Err(format!("cannot write {}: {e}", out_path.display()));
        }
    }
    Ok(())
}

fn materialize_runtime_crate_path(content: &str, output_dir: &Path) -> Result<String, String> {
    const PLACEHOLDER: &str = "PATH_TO_RUNTIME";
    if !content.contains(PLACEHOLDER) {
        return Ok(content.to_string());
    }

    let workspace_root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().map_err(|e| {
            format!("cannot resolve Asatsuyu workspace root for generated Cargo.toml: {e}")
        })?;
    let runtime_crate = workspace_root.join("crates/asatsuyu-runtime-python");
    if !runtime_crate.exists() {
        return Err(format!(
            "cannot find runtime crate required for mixed layout: {}",
            runtime_crate.display()
        ));
    }

    let output_root = output_dir
        .canonicalize()
        .or_else(|_| output_dir.parent().map_or_else(std::env::current_dir, Path::canonicalize))
        .map_err(|e| format!("cannot resolve output directory for generated Cargo.toml: {e}"))?;
    let relative = diff_paths(&runtime_crate, &output_root).ok_or_else(|| {
        format!(
            "cannot compute relative path from {} to {}",
            output_root.display(),
            runtime_crate.display()
        )
    })?;
    let relative = relative.to_string_lossy().replace('\\', "/");

    Ok(content.replace(PLACEHOLDER, &relative))
}

fn diff_paths(path: &Path, base: &Path) -> Option<PathBuf> {
    let path_components: Vec<Component<'_>> = path.components().collect();
    let base_components: Vec<Component<'_>> = base.components().collect();

    if path_components.first().zip(base_components.first()).is_some_and(|(lhs, rhs)| lhs != rhs) {
        return None;
    }

    let mut shared = 0;
    while shared < path_components.len()
        && shared < base_components.len()
        && path_components[shared] == base_components[shared]
    {
        shared += 1;
    }

    let mut relative = PathBuf::new();
    for component in &base_components[shared..] {
        if matches!(component, Component::Normal(_)) {
            relative.push("..");
        }
    }
    for component in &path_components[shared..] {
        relative.push(component.as_os_str());
    }

    if relative.as_os_str().is_empty() {
        relative.push(OsStr::new("."));
    }

    Some(relative)
}

// ── Compilation pipeline ───────────────────────────────────────────

/// Result of a successful compilation, including any warnings.
struct CompileOutput {
    module: ThirModule,
    source: String,
    warnings: Vec<Diagnostic>,
    purity: PurityReport,
}

/// Compile a `.asty` file, returning the typed module, source, and warnings.
fn compile_with_source(
    path: &Path,
    ffi_config: &FfiResolverConfig,
) -> Result<CompileOutput, CliError> {
    let source = std::fs::read_to_string(path).map_err(CliError::Io)?;

    let mut all_diagnostics = Vec::new();

    // Parse
    let cst = asatsuyu_parser::parse(FileId(0), &source);
    all_diagnostics.extend(cst.diagnostics().iter().cloned());
    if cst.has_errors() {
        return Err(CliError::CompileErrors { diagnostics: all_diagnostics, source });
    }

    // AST
    let ast = asatsuyu_ast::lower(&cst, FileId(0));
    all_diagnostics.extend(ast.diagnostics.iter().cloned());
    if ast.has_errors() {
        return Err(CliError::CompileErrors { diagnostics: all_diagnostics, source });
    }

    // HIR
    let hir = asatsuyu_hir::lower_to_hir(&ast.module);
    all_diagnostics.extend(hir.diagnostics.iter().cloned());
    if hir.has_errors() {
        return Err(CliError::CompileErrors { diagnostics: all_diagnostics, source });
    }

    // Type check
    let thir = asatsuyu_ty::check_types_with_ffi_config(&hir.module, ffi_config);
    all_diagnostics.extend(thir.diagnostics.iter().cloned());
    if thir.has_errors() {
        return Err(CliError::CompileErrors { diagnostics: all_diagnostics, source });
    }

    // Collect non-error diagnostics (warnings) for display.
    let warnings = all_diagnostics.into_iter().filter(|d| d.severity != Severity::Error).collect();

    Ok(CompileOutput { module: thir.module, source, warnings, purity: hir.purity })
}

// ── Error type ─────────────────────────────────────────────────────

enum CliError {
    Io(std::io::Error),
    CompileErrors { diagnostics: Vec<Diagnostic>, source: String },
    InvalidFfiStubPath { path: PathBuf, reason: &'static str },
    Project(project::ProjectError),
    NoProject,
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::CompileErrors { diagnostics, .. } => {
                for d in diagnostics {
                    writeln!(f, "{}", d.message)?;
                }
                Ok(())
            }
            Self::InvalidFfiStubPath { path, reason } => {
                write!(f, "invalid --ffi-stub-path `{}`: {reason}", path.display())
            }
            Self::Project(e) => write!(f, "{e}"),
            Self::NoProject => write!(
                f,
                "no asatsuyu.toml found; specify files explicitly or run from within a project"
            ),
        }
    }
}

// ── Path resolution ──────────────────────────────────────────────────

/// Context returned by `resolve_check_context`, carrying both the files
/// to compile and (optionally) the discovered project for dependency checking.
struct CheckContext {
    paths: Vec<PathBuf>,
    project: Option<project::Project>,
}

/// Resolve the files to check and optionally discover the project.
///
/// If explicit paths are given, use them (no project discovery).
/// Otherwise, discover the project root from the current directory and
/// collect all `src/**/*.asty` files.
fn resolve_check_context(paths: &[PathBuf]) -> Result<CheckContext, CliError> {
    if !paths.is_empty() {
        let project = discover_project_for_explicit_paths(paths);
        return Ok(CheckContext { paths: paths.to_vec(), project });
    }

    let cwd = std::env::current_dir().map_err(CliError::Io)?;
    let project =
        project::discover_project(&cwd).map_err(CliError::Project)?.ok_or(CliError::NoProject)?;

    let sources = project::discover_sources(&project.root).map_err(CliError::Project)?;
    Ok(CheckContext { paths: sources, project: Some(project) })
}

/// Best-effort project discovery for explicit `check` paths.
///
/// `asatsuyu check src/main.asty` inside a project should still honor
/// `asatsuyu.toml` and run the Python dependency check introduced in Issue 57.
/// When multiple paths are given, use the first project we can discover.
fn discover_project_for_explicit_paths(paths: &[PathBuf]) -> Option<project::Project> {
    let cwd = std::env::current_dir().ok()?;

    for path in paths {
        let candidate = if path.is_absolute() { path.clone() } else { cwd.join(path) };
        let start_dir =
            if candidate.is_dir() { candidate } else { candidate.parent()?.to_path_buf() };

        if let Ok(Some(project)) = project::discover_project(&start_dir) {
            return Some(project);
        }
    }

    project::discover_project(&cwd).ok().flatten()
}

// ── Dependency checking ──────────────────────────────────────────────

/// Run dependency check against the Python environment.
///
/// Only runs when a project is discovered and `[python-dependencies]` is
/// non-empty. Returns `true` if there are issues and `error_on_missing` is set.
fn run_dependency_check(
    project: Option<&project::Project>,
    explicit_python_path: Option<&Path>,
    error_format: ErrorFormat,
    error_on_missing: bool,
) -> bool {
    let Some(project) = project else {
        return false;
    };
    let config = &project.config;

    if config.python_dependencies().is_empty() {
        return false;
    }

    let python_path = explicit_python_path.or_else(|| config.python_path());
    let env = python_env::discover_environment(&project.root, python_path);

    let Some(env) = env else {
        if matches!(error_format, ErrorFormat::Human) {
            eprintln!("warning: no Python environment found; dependency check skipped");
        }
        // Not finding an environment is not a blocking error.
        return false;
    };

    let installed = python_env::scan_installed_packages(&env.site_packages);
    let statuses = python_env::check_dependencies(config.python_dependencies(), &installed);

    report_dependency_issues(&statuses, error_format, error_on_missing)
}

/// Report dependency issues to stderr (human) or stdout (JSON/NDJSON).
///
/// Returns `true` if any issue was found and `error_on_missing` is set
/// (caller should abort with exit code 2).
fn report_dependency_issues(
    statuses: &BTreeMap<String, python_env::DependencyStatus>,
    error_format: ErrorFormat,
    error_on_missing: bool,
) -> bool {
    use python_env::DependencyStatus;

    let mut has_issues = false;

    for (name, status) in statuses {
        match status {
            DependencyStatus::Satisfied { .. } => {}
            DependencyStatus::Missing => {
                has_issues = true;
                let level = if error_on_missing { "error" } else { "warning" };
                match error_format {
                    ErrorFormat::Human => {
                        eprintln!("{level}: Python package `{name}` is not installed");
                        eprintln!("  hint: install it with `pip install {name}`");
                    }
                    ErrorFormat::Json => {
                        let json = serde_json::json!({
                            "type": "dependency",
                            "status": "missing",
                            "package": name,
                            "severity": level,
                        });
                        println!("{json}");
                    }
                }
            }
            DependencyStatus::VersionMismatch { installed, required } => {
                has_issues = true;
                let level = if error_on_missing { "error" } else { "warning" };
                match error_format {
                    ErrorFormat::Human => {
                        eprintln!(
                            "{level}: Python package `{name}` version {installed} \
                             does not satisfy {required}"
                        );
                        eprintln!("  hint: upgrade with `pip install \"{name}{required}\"`");
                    }
                    ErrorFormat::Json => {
                        let json = serde_json::json!({
                            "type": "dependency",
                            "status": "version_mismatch",
                            "package": name,
                            "installed": installed.to_string(),
                            "required": required,
                            "severity": level,
                        });
                        println!("{json}");
                    }
                }
            }
        }
    }

    has_issues && error_on_missing
}

// ── Diagnostic reporting (miette) ─────────────────────────────────

fn report_diagnostics(
    diagnostics: &[Diagnostic],
    source: &str,
    filename: &str,
    error_format: ErrorFormat,
) {
    match error_format {
        ErrorFormat::Human => {
            for d in diagnostics {
                let report = SourceDiagnostic::from_diagnostic(d, filename, source);
                eprintln!("{:?}", miette::Report::new(report));
            }
        }
        ErrorFormat::Json => {
            let line_index = LineIndex::new(source);
            for d in diagnostics {
                let json = json_diagnostic::diagnostic_to_json(d, filename, &line_index);
                json_diagnostic::emit_json_line(&json);
            }
        }
    }
}

/// Print what the compiler proved about each function's purity.
///
/// This is the requested output of `check --purity`, so it goes to stdout,
/// leaving stderr to the diagnostic stream. Functions the compiler could not
/// prove pure are separated from functions it proved effectful, because only
/// the second kind is a fact about the program.
fn emit_purity_report(report: &PurityReport, filename: &str, error_format: ErrorFormat) {
    match error_format {
        ErrorFormat::Human => {
            println!("{filename}");
            for func in &report.functions {
                let verdict = match func.purity {
                    Purity::Pure => "pure",
                    Purity::Effectful => "effectful",
                };
                match func.source() {
                    Some(source) => {
                        println!("  {verdict:<10} {:<24} {}", func.name, describe(source));
                    }
                    None => println!("  {verdict:<10} {}", func.name),
                }
            }
            println!("  -- {} pure / {} effectful", report.pure_count(), report.effectful_count());
        }
        ErrorFormat::Json => {
            for func in &report.functions {
                let line = serde_json::json!({
                    "type": "purity",
                    "file": filename,
                    "function": func.name.as_str(),
                    "purity": match func.purity {
                        Purity::Pure => "pure",
                        Purity::Effectful => "effectful",
                    },
                    "reason": func.source().map(describe),
                });
                println!("{line}");
            }
        }
    }
}

/// One-word explanation of why a function is effectful.
fn describe(source: EffectSource) -> &'static str {
    match source {
        EffectSource::Boundary => "crosses the Python boundary",
        EffectSource::Async => "async",
        EffectSource::Propagated => "calls an effectful function",
        EffectSource::Unresolved => "contains a call the compiler cannot resolve",
    }
}

/// Emit a single final summary after all diagnostics for a command invocation.
///
/// Called exactly once per command, on both success and failure paths.
///
/// In human mode:
///   - errors present: `error: aborting due to N error(s)[ and M warning(s)]`
///   - only warnings:  `warning: N warning(s) emitted`
///   - clean success:  no output
///
/// In JSON mode: always a `{"type":"summary",...}` NDJSON line.
fn emit_final_summary(all_diags: &[Diagnostic], error_format: ErrorFormat) {
    match error_format {
        ErrorFormat::Human => {
            let errors = all_diags.iter().filter(|d| d.severity == Severity::Error).count();
            let warnings = all_diags.iter().filter(|d| d.severity == Severity::Warning).count();

            if errors > 0 {
                let mut parts = Vec::new();
                if errors == 1 {
                    parts.push("1 error".to_string());
                } else {
                    parts.push(format!("{errors} errors"));
                }
                if warnings == 1 {
                    parts.push("1 warning".to_string());
                } else if warnings > 0 {
                    parts.push(format!("{warnings} warnings"));
                }
                eprintln!("error: aborting due to {}", parts.join(" and "));
            } else if warnings == 1 {
                eprintln!("warning: 1 warning emitted");
            } else if warnings > 0 {
                eprintln!("warning: {warnings} warnings emitted");
            }
        }
        ErrorFormat::Json => {
            let summary = json_diagnostic::summary_to_json(all_diags);
            json_diagnostic::emit_json_line(&summary);
        }
    }
}
