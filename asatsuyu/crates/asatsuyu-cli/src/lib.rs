//! Command-line interface for the Asatsuyu compiler.
//!
//! Provides `check`, `build`, `run`, and `new` subcommands that drive the
//! compilation pipeline from `.asty` source to Python 3.12+ output.

mod diagnostic_report;
mod json_diagnostic;
mod project;
mod watch;

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitCode};

use asatsuyu_backend_python::{FfiRuntimeMode, GeneratedPackage, PackageConfig};
use asatsuyu_hir::ffi::FfiResolverConfig;
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

#[derive(Subcommand)]
enum Commands {
    /// Type-check without code generation
    Check {
        /// Paths to `.asty` source files (optional when inside a project)
        paths: Vec<PathBuf>,
        /// Watch for file changes and re-check automatically
        #[arg(long)]
        watch: bool,
        #[command(flatten)]
        output: OutputArgs,
        /// Restrict FFI to stdlib modules only (pathlib, json, os, sys)
        #[arg(long)]
        ffi_stdlib_only: bool,
        /// Additional directories for .pyi stub files
        #[arg(long)]
        ffi_stub_path: Vec<PathBuf>,
    },
    /// Compile .asty to Python
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
        /// Restrict FFI to stdlib modules only (pathlib, json, os, sys)
        #[arg(long)]
        ffi_stdlib_only: bool,
        /// Additional directories for .pyi stub files
        #[arg(long)]
        ffi_stub_path: Vec<PathBuf>,
    },
    /// Compile and execute with python3
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
        /// Restrict FFI to stdlib modules only (pathlib, json, os, sys)
        #[arg(long)]
        ffi_stdlib_only: bool,
        /// Additional directories for .pyi stub files
        #[arg(long)]
        ffi_stub_path: Vec<PathBuf>,
    },
    /// Create a new Asatsuyu project
    New {
        /// Project name (used as directory name)
        name: String,
    },
    /// Show FFI trust report for all known Python modules
    #[command(name = "verify-ffi")]
    VerifyFfi,
}

// ── Entry point ────────────────────────────────────────────────────

/// Run the CLI, returning an appropriate exit code.
#[must_use]
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
        Commands::Check { paths, watch, output, ffi_stdlib_only, ffi_stub_path } => {
            let ffi_config = match build_ffi_config(ffi_stdlib_only, &ffi_stub_path) {
                Ok(config) => config,
                Err(err) => {
                    eprintln!("error: {err}");
                    return ExitCode::FAILURE;
                }
            };
            let resolved = match resolve_check_paths(&paths) {
                Ok(p) => p,
                Err(err) => {
                    eprintln!("error: {err}");
                    return ExitCode::FAILURE;
                }
            };
            if watch {
                watch::run_watch(&resolved, &ffi_config, output.error_format)
            } else {
                cmd_check(&resolved, &ffi_config, output.error_format)
            }
        }
        Commands::Build {
            path,
            output_dir,
            source_map,
            no_emit_package,
            ffi_runtime,
            output,
            ffi_stdlib_only,
            ffi_stub_path,
        } => {
            let ffi_config = match build_ffi_config(ffi_stdlib_only, &ffi_stub_path) {
                Ok(config) => config,
                Err(err) => {
                    eprintln!("error: {err}");
                    return ExitCode::FAILURE;
                }
            };
            let runtime_mode = convert_ffi_runtime(ffi_runtime);
            cmd_build(
                &path,
                &output_dir,
                source_map,
                no_emit_package,
                runtime_mode,
                &ffi_config,
                output.error_format,
            )
        }
        Commands::Run { path, source_map, ffi_runtime, output, ffi_stdlib_only, ffi_stub_path } => {
            let ffi_config = match build_ffi_config(ffi_stdlib_only, &ffi_stub_path) {
                Ok(config) => config,
                Err(err) => {
                    eprintln!("error: {err}");
                    return ExitCode::FAILURE;
                }
            };
            let runtime_mode = convert_ffi_runtime(ffi_runtime);
            cmd_run(&path, source_map, runtime_mode, &ffi_config, output.error_format)
        }
        Commands::New { name } => cmd_new(&name),
        Commands::VerifyFfi => cmd_verify_ffi(),
    }
}

// ── FFI config helpers ────────────────────────────────────────────

fn build_ffi_config(
    ffi_stdlib_only: bool,
    ffi_stub_path: &[PathBuf],
) -> Result<FfiResolverConfig, CliError> {
    for path in ffi_stub_path {
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

    Ok(FfiResolverConfig { stdlib_only: ffi_stdlib_only, stub_paths: ffi_stub_path.to_vec() })
}

fn convert_ffi_runtime(runtime: FfiRuntime) -> FfiRuntimeMode {
    match runtime {
        FfiRuntime::On => FfiRuntimeMode::On,
        FfiRuntime::Off => FfiRuntimeMode::Off,
        FfiRuntime::Auto => FfiRuntimeMode::Auto,
    }
}

// ── Command handlers ───────────────────────────────────────────────

pub(crate) fn cmd_check(
    paths: &[PathBuf],
    ffi_config: &FfiResolverConfig,
    error_format: ErrorFormat,
) -> ExitCode {
    let mut failed = false;
    let mut all_diags: Vec<Diagnostic> = Vec::new();

    for path in paths {
        let filename = path.display().to_string();
        match compile_with_source(path, ffi_config) {
            Ok(result) => {
                if !result.warnings.is_empty() {
                    report_diagnostics(&result.warnings, &result.source, &filename, error_format);
                    all_diags.extend(result.warnings);
                }
            }
            Err(CliError::CompileErrors { diagnostics, source }) => {
                report_diagnostics(&diagnostics, &source, &filename, error_format);
                // In human mode, show per-file error summary immediately.
                // In JSON mode, summary is emitted once at the end.
                if matches!(error_format, ErrorFormat::Human) {
                    report_error_summary(&diagnostics, error_format);
                }
                all_diags.extend(diagnostics);
                failed = true;
            }
            Err(err) => {
                eprintln!("error: {err}");
                failed = true;
            }
        }
    }

    // JSON mode: always emit a summary as the final line.
    if matches!(error_format, ErrorFormat::Json) {
        let summary = json_diagnostic::summary_to_json(&all_diags);
        json_diagnostic::emit_json_line(&summary);
    }

    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

fn cmd_build(
    path: &Path,
    output_dir: &Path,
    source_map: bool,
    no_emit_package: bool,
    ffi_runtime: FfiRuntimeMode,
    ffi_config: &FfiResolverConfig,
    error_format: ErrorFormat,
) -> ExitCode {
    let filename = path.display().to_string();
    let result = match compile_with_source(path, ffi_config) {
        Ok(result) => result,
        Err(CliError::CompileErrors { diagnostics, source }) => {
            report_diagnostics(&diagnostics, &source, &filename, error_format);
            report_error_summary(&diagnostics, error_format);
            return ExitCode::FAILURE;
        }
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::FAILURE;
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
            return ExitCode::FAILURE;
        }
        println!("{}", out_path.display());
        if matches!(error_format, ErrorFormat::Human) {
            eprintln!("  Compiled {stem} (module only) → {}", out_path.display());
        } else {
            let summary = json_diagnostic::summary_to_json(&result.warnings);
            json_diagnostic::emit_json_line(&summary);
        }
        return ExitCode::SUCCESS;
    }

    let config =
        PackageConfig { name: stem.to_string(), version: "0.1.0".into(), source_map, ffi_runtime };
    let package =
        asatsuyu_backend_python::emit_package(&result.module, &config, Some(&result.source));

    // Clean the package subdirectory before writing.
    let pkg_dir = output_dir.join("python").join(stem.as_ref());
    if pkg_dir.exists() {
        let _ = std::fs::remove_dir_all(&pkg_dir);
    }

    if let Err(msg) = write_package(&package, output_dir) {
        eprintln!("error: {msg}");
        return ExitCode::FAILURE;
    }

    // stdout: output directory (for scripting).
    println!("{}", output_dir.display());
    if matches!(error_format, ErrorFormat::Human) {
        // stderr: human-readable summary.
        eprintln!("  Compiled {stem} ({} files) → {}", package.files.len(), output_dir.display());
    } else {
        let summary = json_diagnostic::summary_to_json(&result.warnings);
        json_diagnostic::emit_json_line(&summary);
    }
    ExitCode::SUCCESS
}

fn cmd_run(
    path: &Path,
    source_map: bool,
    ffi_runtime: FfiRuntimeMode,
    ffi_config: &FfiResolverConfig,
    error_format: ErrorFormat,
) -> ExitCode {
    let filename = path.display().to_string();
    let result = match compile_with_source(path, ffi_config) {
        Ok(result) => result,
        Err(CliError::CompileErrors { diagnostics, source }) => {
            report_diagnostics(&diagnostics, &source, &filename, error_format);
            report_error_summary(&diagnostics, error_format);
            return ExitCode::FAILURE;
        }
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::FAILURE;
        }
    };

    if !result.warnings.is_empty() {
        report_diagnostics(&result.warnings, &result.source, &filename, error_format);
    }

    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let output_dir = PathBuf::from("target/run");
    let config =
        PackageConfig { name: stem.to_string(), version: "0.1.0".into(), source_map, ffi_runtime };
    let package =
        asatsuyu_backend_python::emit_package(&result.module, &config, Some(&result.source));

    // Always clean run output to avoid stale files.
    let pkg_dir = output_dir.join("python").join(stem.as_ref());
    if pkg_dir.exists() {
        let _ = std::fs::remove_dir_all(&pkg_dir);
    }

    if let Err(msg) = write_package(&package, &output_dir) {
        eprintln!("error: {msg}");
        return ExitCode::FAILURE;
    }

    // Execute with python3.
    let has_main = result
        .module
        .functions
        .iter()
        .any(|f| result.module.symbol_table.get(f.def_id).name.as_str() == "main");

    // Python source lives under python/ in the new layout.
    let python_dir = output_dir.join("python");
    let status_result = if has_main {
        Command::new("python3").arg("-m").arg(stem.as_ref()).current_dir(&python_dir).status()
    } else {
        let py_path = python_dir.join(format!("{stem}/{stem}.py"));
        Command::new("python3").arg(&py_path).status()
    };

    match status_result {
        Ok(status) => {
            if matches!(error_format, ErrorFormat::Json) {
                let summary = json_diagnostic::summary_to_json(&result.warnings);
                json_diagnostic::emit_json_line(&summary);
            }
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
            ExitCode::FAILURE
        }
    }
}

fn cmd_new(name: &str) -> ExitCode {
    // Validate project name.
    if name.is_empty() {
        eprintln!("error: project name cannot be empty");
        return ExitCode::FAILURE;
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        eprintln!("error: project name must contain only ASCII letters, digits, and underscores");
        return ExitCode::FAILURE;
    }

    let project_dir = Path::new(name);
    if project_dir.exists() {
        eprintln!("error: directory `{name}` already exists");
        return ExitCode::FAILURE;
    }

    // Create directory structure.
    let src_dir = project_dir.join("src");
    if let Err(e) = std::fs::create_dir_all(&src_dir) {
        eprintln!("error: cannot create directory: {e}");
        return ExitCode::FAILURE;
    }

    // src/main.asty
    let main_asty = "pub fn main() {\n  42\n}\n";
    if let Err(e) = std::fs::write(src_dir.join("main.asty"), main_asty) {
        eprintln!("error: cannot write main.asty: {e}");
        return ExitCode::FAILURE;
    }

    // asatsuyu.toml
    let toml = format!(
        "[project]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n[python]\nversion = \">=3.12\"\n",
    );
    if let Err(e) = std::fs::write(project_dir.join("asatsuyu.toml"), toml) {
        eprintln!("error: cannot write asatsuyu.toml: {e}");
        return ExitCode::FAILURE;
    }

    // .gitignore
    let gitignore = "/dist/\n/target/\n__pycache__/\n*.pyc\n";
    if let Err(e) = std::fs::write(project_dir.join(".gitignore"), gitignore) {
        eprintln!("error: cannot write .gitignore: {e}");
        return ExitCode::FAILURE;
    }

    eprintln!("  Created project `{name}` in ./{name}");
    eprintln!("  Run `asatsuyu run {name}/src/main.asty` to get started");
    ExitCode::SUCCESS
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

    Ok(CompileOutput { module: thir.module, source, warnings })
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

/// Resolve the files to check.
///
/// If explicit paths are given, use them. Otherwise, discover the project root
/// from the current directory and collect all `src/**/*.asty` files.
fn resolve_check_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, CliError> {
    if !paths.is_empty() {
        return Ok(paths.to_vec());
    }

    let cwd = std::env::current_dir().map_err(CliError::Io)?;
    let project =
        project::discover_project(&cwd).map_err(CliError::Project)?.ok_or(CliError::NoProject)?;

    project::discover_sources(&project.root).map_err(CliError::Project)
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

/// Print a summary after error diagnostics.
///
/// In human mode: `error: aborting due to 2 errors and 1 warning`
/// In JSON mode: a `{"type":"summary",...}` NDJSON line.
fn report_error_summary(diagnostics: &[Diagnostic], error_format: ErrorFormat) {
    match error_format {
        ErrorFormat::Human => {
            let errors = diagnostics.iter().filter(|d| d.severity == Severity::Error).count();
            let warnings = diagnostics.iter().filter(|d| d.severity == Severity::Warning).count();

            if errors == 0 {
                return;
            }

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
        }
        ErrorFormat::Json => {
            let summary = json_diagnostic::summary_to_json(diagnostics);
            json_diagnostic::emit_json_line(&summary);
        }
    }
}
