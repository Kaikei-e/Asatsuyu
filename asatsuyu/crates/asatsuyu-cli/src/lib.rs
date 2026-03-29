//! Command-line interface for the Asatsuyu compiler.
//!
//! Provides `check`, `build`, `run`, and `new` subcommands that drive the
//! compilation pipeline from `.asty` source to Python 3.12+ output.

mod diagnostic_report;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use asatsuyu_backend_python::{GeneratedPackage, PackageConfig};
use asatsuyu_syntax::{Diagnostic, FileId, Severity};
use asatsuyu_ty::ThirModule;
use clap::{Parser, Subcommand};

use crate::diagnostic_report::SourceDiagnostic;

// ── CLI definition ─────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "asatsuyu", version, about = "The Asatsuyu compiler")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Type-check without code generation
    Check {
        /// Path to the .asty source file
        path: PathBuf,
    },
    /// Compile .asty to Python
    Build {
        /// Path to the .asty source file
        path: PathBuf,
        /// Output directory
        #[arg(short, long, default_value = "dist")]
        output: PathBuf,
        /// Add source-map comments (# asty:L<n>) to generated Python
        #[arg(long)]
        source_map: bool,
    },
    /// Compile and execute with python3
    Run {
        /// Path to the .asty source file
        path: PathBuf,
        /// Add source-map comments (# asty:L<n>) to generated Python
        #[arg(long)]
        source_map: bool,
    },
    /// Create a new Asatsuyu project
    New {
        /// Project name (used as directory name)
        name: String,
    },
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
        Commands::Check { path } => cmd_check(&path),
        Commands::Build { path, output, source_map } => cmd_build(&path, &output, source_map),
        Commands::Run { path, source_map } => cmd_run(&path, source_map),
        Commands::New { name } => cmd_new(&name),
    }
}

// ── Command handlers ───────────────────────────────────────────────

fn cmd_check(path: &Path) -> ExitCode {
    let filename = path.display().to_string();
    match compile_with_source(path) {
        Ok(result) => {
            if !result.warnings.is_empty() {
                report_diagnostics(&result.warnings, &result.source, &filename);
            }
            ExitCode::SUCCESS
        }
        Err(CliError::CompileErrors { diagnostics, source }) => {
            report_diagnostics(&diagnostics, &source, &filename);
            report_error_summary(&diagnostics);
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_build(path: &Path, output_dir: &Path, source_map: bool) -> ExitCode {
    let filename = path.display().to_string();
    let result = match compile_with_source(path) {
        Ok(result) => result,
        Err(CliError::CompileErrors { diagnostics, source }) => {
            report_diagnostics(&diagnostics, &source, &filename);
            report_error_summary(&diagnostics);
            return ExitCode::FAILURE;
        }
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::FAILURE;
        }
    };

    if !result.warnings.is_empty() {
        report_diagnostics(&result.warnings, &result.source, &filename);
    }

    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let config = PackageConfig { name: stem.to_string(), version: "0.1.0".into(), source_map };
    let package =
        asatsuyu_backend_python::emit_package(&result.module, &config, Some(&result.source));

    // Clean the package subdirectory before writing.
    let pkg_dir = output_dir.join(stem.as_ref());
    if pkg_dir.exists() {
        let _ = std::fs::remove_dir_all(&pkg_dir);
    }

    if let Err(msg) = write_package(&package, output_dir) {
        eprintln!("error: {msg}");
        return ExitCode::FAILURE;
    }

    // stdout: output directory (for scripting).
    println!("{}", output_dir.display());
    // stderr: human-readable summary.
    eprintln!("  Compiled {stem} ({} files) → {}", package.files.len(), output_dir.display());
    ExitCode::SUCCESS
}

fn cmd_run(path: &Path, source_map: bool) -> ExitCode {
    let filename = path.display().to_string();
    let result = match compile_with_source(path) {
        Ok(result) => result,
        Err(CliError::CompileErrors { diagnostics, source }) => {
            report_diagnostics(&diagnostics, &source, &filename);
            report_error_summary(&diagnostics);
            return ExitCode::FAILURE;
        }
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::FAILURE;
        }
    };

    if !result.warnings.is_empty() {
        report_diagnostics(&result.warnings, &result.source, &filename);
    }

    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let output_dir = PathBuf::from("target/run");
    let config = PackageConfig { name: stem.to_string(), version: "0.1.0".into(), source_map };
    let package =
        asatsuyu_backend_python::emit_package(&result.module, &config, Some(&result.source));

    // Always clean run output to avoid stale files.
    let pkg_dir = output_dir.join(stem.as_ref());
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

    let status_result = if has_main {
        Command::new("python3").arg("-m").arg(stem.as_ref()).current_dir(&output_dir).status()
    } else {
        let py_path = output_dir.join(format!("{stem}/{stem}.py"));
        Command::new("python3").arg(&py_path).status()
    };

    match status_result {
        Ok(status) => {
            if status.success() {
                ExitCode::SUCCESS
            } else {
                // Propagate python's exit code.
                ExitCode::from(status.code().unwrap_or(1) as u8)
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
    let main_asty = format!("pub fn main() {{\n  42\n}}\n",);
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

// ── Package writing ───────────────────────────────────────────────

fn write_package(package: &GeneratedPackage, output_dir: &Path) -> Result<(), String> {
    for file in &package.files {
        let out_path = output_dir.join(&file.path);
        if let Some(parent) = out_path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            return Err(format!("cannot create directory: {e}"));
        }
        if let Err(e) = std::fs::write(&out_path, &file.content) {
            return Err(format!("cannot write {}: {e}", out_path.display()));
        }
    }
    Ok(())
}

// ── Compilation pipeline ───────────────────────────────────────────

/// Result of a successful compilation, including any warnings.
struct CompileOutput {
    module: ThirModule,
    source: String,
    warnings: Vec<Diagnostic>,
}

/// Compile a `.asty` file, returning the typed module, source, and warnings.
fn compile_with_source(path: &Path) -> Result<CompileOutput, CliError> {
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
    let thir = asatsuyu_ty::check_types(&hir.module);
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
        }
    }
}

// ── Diagnostic reporting (miette) ─────────────────────────────────

fn report_diagnostics(diagnostics: &[Diagnostic], source: &str, filename: &str) {
    for d in diagnostics {
        let report = SourceDiagnostic::from_diagnostic(d, filename, source);
        eprintln!("{:?}", miette::Report::new(report));
    }
}

/// Print a summary line after error diagnostics, e.g.:
/// `error: aborting due to 2 errors and 1 warning`
fn report_error_summary(diagnostics: &[Diagnostic]) {
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
