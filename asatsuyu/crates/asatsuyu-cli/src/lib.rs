//! Command-line interface for the Asatsuyu compiler.
//!
//! Provides `check`, `build`, and `run` subcommands that drive the
//! compilation pipeline from `.asty` source to Python 3.12+ output.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use asatsuyu_backend_python::PackageConfig;
use asatsuyu_syntax::{Diagnostic, FileId, LabelStyle, Severity};
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
}

// ── Entry point ────────────────────────────────────────────────────

/// Run the CLI, returning an appropriate exit code.
#[must_use]
pub fn run() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Commands::Check { path } => cmd_check(&path),
        Commands::Build { path, output, source_map } => cmd_build(&path, &output, source_map),
        Commands::Run { path, source_map } => cmd_run(&path, source_map),
    }
}

// ── Command handlers ───────────────────────────────────────────────

fn cmd_check(path: &Path) -> ExitCode {
    match compile(path) {
        Ok(_) => ExitCode::SUCCESS,
        Err(CliError::CompileErrors { diagnostics, source }) => {
            report_diagnostics(&diagnostics, &source);
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_build(path: &Path, output_dir: &Path, source_map: bool) -> ExitCode {
    let (thir, source) = match compile_with_source(path) {
        Ok(result) => result,
        Err(CliError::CompileErrors { diagnostics, source }) => {
            report_diagnostics(&diagnostics, &source);
            return ExitCode::FAILURE;
        }
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::FAILURE;
        }
    };

    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let config = PackageConfig { name: stem.to_string(), version: "0.1.0".into(), source_map };
    let package = asatsuyu_backend_python::emit_package(&thir, &config, Some(&source));

    for file in &package.files {
        let out_path = output_dir.join(&file.path);
        if let Some(parent) = out_path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            eprintln!("error: cannot create directory: {e}");
            return ExitCode::FAILURE;
        }
        if let Err(e) = std::fs::write(&out_path, &file.content) {
            eprintln!("error: cannot write {}: {e}", out_path.display());
            return ExitCode::FAILURE;
        }
    }

    println!("{}", output_dir.display());
    ExitCode::SUCCESS
}

fn cmd_run(path: &Path, source_map: bool) -> ExitCode {
    let (thir, source) = match compile_with_source(path) {
        Ok(result) => result,
        Err(CliError::CompileErrors { diagnostics, source }) => {
            report_diagnostics(&diagnostics, &source);
            return ExitCode::FAILURE;
        }
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::FAILURE;
        }
    };

    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let output_dir = Path::new("dist");
    let config = PackageConfig { name: stem.to_string(), version: "0.1.0".into(), source_map };
    let package = asatsuyu_backend_python::emit_package(&thir, &config, Some(&source));

    for file in &package.files {
        let out_path = output_dir.join(&file.path);
        if let Some(parent) = out_path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            eprintln!("error: cannot create directory: {e}");
            return ExitCode::FAILURE;
        }
        if let Err(e) = std::fs::write(&out_path, &file.content) {
            eprintln!("error: cannot write {}: {e}", out_path.display());
            return ExitCode::FAILURE;
        }
    }

    // Execute with python3 -m <package> if main exists, else just the module.
    let has_main =
        thir.functions.iter().any(|f| thir.symbol_table.get(f.def_id).name.as_str() == "main");

    let status_result = if has_main {
        Command::new("python3").arg("-m").arg(stem.as_ref()).current_dir(output_dir).status()
    } else {
        let py_path = output_dir.join(format!("{stem}/{stem}.py"));
        Command::new("python3").arg(&py_path).status()
    };

    match status_result {
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

/// Compile a `.asty` file, returning the typed module and the original source.
fn compile_with_source(path: &Path) -> Result<(ThirModule, String), CliError> {
    let source = std::fs::read_to_string(path).map_err(CliError::Io)?;
    let module = compile_source(&source)?;
    Ok((module, source))
}

/// Compile a `.asty` file through the full pipeline, returning the typed module.
fn compile(path: &Path) -> Result<ThirModule, CliError> {
    let source = std::fs::read_to_string(path).map_err(CliError::Io)?;
    compile_source(&source)
}

/// Compile source text through the full pipeline.
fn compile_source(source: &str) -> Result<ThirModule, CliError> {
    let mut all_diagnostics = Vec::new();

    // Parse
    let cst = asatsuyu_parser::parse(FileId(0), source);
    all_diagnostics.extend(cst.diagnostics().iter().cloned());
    if cst.has_errors() {
        return Err(CliError::CompileErrors {
            diagnostics: all_diagnostics,
            source: source.to_string(),
        });
    }

    // AST
    let ast = asatsuyu_ast::lower(&cst, FileId(0));
    all_diagnostics.extend(ast.diagnostics.iter().cloned());
    if ast.has_errors() {
        return Err(CliError::CompileErrors {
            diagnostics: all_diagnostics,
            source: source.to_string(),
        });
    }

    // HIR
    let hir = asatsuyu_hir::lower_to_hir(&ast.module);
    all_diagnostics.extend(hir.diagnostics.iter().cloned());
    if hir.has_errors() {
        return Err(CliError::CompileErrors {
            diagnostics: all_diagnostics,
            source: source.to_string(),
        });
    }

    // Type check
    let thir = asatsuyu_ty::check_types(&hir.module);
    all_diagnostics.extend(thir.diagnostics.iter().cloned());
    if thir.has_errors() {
        return Err(CliError::CompileErrors {
            diagnostics: all_diagnostics,
            source: source.to_string(),
        });
    }

    Ok(thir.module)
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

// ── Diagnostic reporting ───────────────────────────────────────────

fn report_diagnostics(diagnostics: &[Diagnostic], source: &str) {
    for d in diagnostics {
        let prefix = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let code_str = d.code.map_or(String::new(), |c| format!("[{c}] "));
        eprintln!("{prefix}: {code_str}{}", d.message);

        // Labels with source location.
        for label in &d.labels {
            let marker = match label.style {
                LabelStyle::Primary => "-->",
                LabelStyle::Secondary => "  =",
            };
            let (line, col) = offset_to_line_col(source, label.span.start);
            eprintln!("  {marker} {line}:{col}: {}", label.message);
        }

        // Notes.
        for note in &d.notes {
            eprintln!("  = note: {note}");
        }

        // Hints.
        for hint in &d.hints {
            eprintln!("  = hint: {hint}");
        }

        eprintln!();
    }
}

/// Convert a byte offset to a 1-based (line, column) pair.
fn offset_to_line_col(source: &str, offset: u32) -> (usize, usize) {
    let offset = offset as usize;
    let mut line = 1;
    let mut col = 1;
    for (i, ch) in source.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}
