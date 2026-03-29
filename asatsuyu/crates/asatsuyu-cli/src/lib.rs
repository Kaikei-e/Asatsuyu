//! Command-line interface for the Asatsuyu compiler.
//!
//! Provides `check`, `build`, and `run` subcommands that drive the
//! compilation pipeline from `.asty` source to Python 3.12+ output.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use asatsuyu_syntax::{Diagnostic, FileId, Severity};
use asatsuyu_ty::ThirModule;
use clap::{Parser, Subcommand};

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
    },
    /// Compile and execute with python3
    Run {
        /// Path to the .asty source file
        path: PathBuf,
    },
}

// ── Entry point ────────────────────────────────────────────────────

/// Run the CLI, returning an appropriate exit code.
#[must_use]
pub fn run() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Commands::Check { path } => cmd_check(&path),
        Commands::Build { path, output } => cmd_build(&path, &output),
        Commands::Run { path } => cmd_run(&path),
    }
}

// ── Command handlers ───────────────────────────────────────────────

fn cmd_check(path: &Path) -> ExitCode {
    match compile(path) {
        Ok(_) => ExitCode::SUCCESS,
        Err(CliError::CompileErrors(diagnostics)) => {
            report_diagnostics(&diagnostics);
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_build(path: &Path, output_dir: &Path) -> ExitCode {
    let thir = match compile(path) {
        Ok(result) => result,
        Err(CliError::CompileErrors(diagnostics)) => {
            report_diagnostics(&diagnostics);
            return ExitCode::FAILURE;
        }
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::FAILURE;
        }
    };

    let python = asatsuyu_backend_python::emit_module(&thir);

    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let py_path = output_dir.join(format!("{stem}.py"));

    if let Err(e) = std::fs::create_dir_all(output_dir) {
        eprintln!("error: cannot create output directory: {e}");
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(&py_path, &python) {
        eprintln!("error: cannot write {}: {e}", py_path.display());
        return ExitCode::FAILURE;
    }

    println!("{}", py_path.display());
    ExitCode::SUCCESS
}

fn cmd_run(path: &Path) -> ExitCode {
    let thir = match compile(path) {
        Ok(result) => result,
        Err(CliError::CompileErrors(diagnostics)) => {
            report_diagnostics(&diagnostics);
            return ExitCode::FAILURE;
        }
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::FAILURE;
        }
    };

    let mut python = asatsuyu_backend_python::emit_module(&thir);

    // Append a __main__ guard that calls main() if it exists.
    if thir.functions.iter().any(|f| thir.symbol_table.get(f.def_id).name.as_str() == "main") {
        python.push_str("\n\nif __name__ == \"__main__\":\n    main()\n");
    }

    // Write to dist/ so the user can inspect generated code.
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let output_dir = Path::new("dist");
    let py_path = output_dir.join(format!("{stem}.py"));

    if let Err(e) = std::fs::create_dir_all(output_dir) {
        eprintln!("error: cannot create output directory: {e}");
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(&py_path, &python) {
        eprintln!("error: cannot write {}: {e}", py_path.display());
        return ExitCode::FAILURE;
    }

    // Execute with python3, passing stdout/stderr through.
    match Command::new("python3").arg(&py_path).status() {
        Ok(status) => {
            if status.success() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            }
        }
        Err(e) => {
            eprintln!("error: cannot execute python3: {e}");
            ExitCode::FAILURE
        }
    }
}

// ── Compilation pipeline ───────────────────────────────────────────

/// Compile a `.asty` file through the full pipeline, returning the typed module.
fn compile(path: &Path) -> Result<ThirModule, CliError> {
    let source = std::fs::read_to_string(path).map_err(CliError::Io)?;

    let mut all_diagnostics = Vec::new();

    // Parse
    let cst = asatsuyu_parser::parse(FileId(0), &source);
    all_diagnostics.extend(cst.diagnostics().iter().cloned());
    if cst.has_errors() {
        return Err(CliError::CompileErrors(all_diagnostics));
    }

    // AST
    let ast = asatsuyu_ast::lower(&cst, FileId(0));
    all_diagnostics.extend(ast.diagnostics.iter().cloned());
    if ast.has_errors() {
        return Err(CliError::CompileErrors(all_diagnostics));
    }

    // HIR
    let hir = asatsuyu_hir::lower_to_hir(&ast.module);
    all_diagnostics.extend(hir.diagnostics.iter().cloned());
    if hir.has_errors() {
        return Err(CliError::CompileErrors(all_diagnostics));
    }

    // Type check
    let thir = asatsuyu_ty::check_types(&hir.module);
    all_diagnostics.extend(thir.diagnostics.iter().cloned());
    if thir.has_errors() {
        return Err(CliError::CompileErrors(all_diagnostics));
    }

    Ok(thir.module)
}

// ── Error type ─────────────────────────────────────────────────────

enum CliError {
    Io(std::io::Error),
    CompileErrors(Vec<Diagnostic>),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::CompileErrors(diags) => {
                for d in diags {
                    writeln!(f, "{}", d.message)?;
                }
                Ok(())
            }
        }
    }
}

// ── Diagnostic reporting ───────────────────────────────────────────

fn report_diagnostics(diagnostics: &[Diagnostic]) {
    for d in diagnostics {
        let prefix = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        eprintln!("{prefix}: {}", d.message);
    }
}
