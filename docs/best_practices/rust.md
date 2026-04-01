# Rust Best Practices — Asatsuyu

These conventions govern the Rust implementation of the Asatsuyu compiler and CLI.
Design assumptions follow the [compiler architecture](../architecture.md) and
[roadmap](../roadmap.md).

## 1. Scope

- Asatsuyu implements a **statically typed Python frontend** in Rust
- The workspace maintains a unidirectional pipeline: `lexer -> parser -> ast -> hir -> ty -> backend-python`
- `asatsuyu-cli` is the user entry point; `asatsuyu-syntax` is the lowest shared crate
- Do not introduce unnecessary responsibilities into the compiler core or CLI
- Implementation order is vertical-slice-first; the initial goal is `hello.asty -> hello.py -> run`
- Do not over-engineer the MVP for LSP / formatter / multi-backend concerns

## 2. Edition and Workspace

- Use `edition = "2024"`
- Manage shared dependencies and lints at the workspace level
- Keep each crate's responsibilities narrow; `lib.rs` is the public boundary, `main.rs` is a thin entry point
- Default to `pub(crate)`; use `pub` only for public API

```toml
[workspace.package]
edition = "2024"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
module_name_repetitions = "allow"
missing_panics_doc = "allow"
missing_errors_doc = "allow"
```

```rust
// crates/asatsuyu-cli/src/main.rs
fn main() -> std::process::ExitCode {
    asatsuyu_cli::run()
}
```

## 3. Crate Boundaries

- `asatsuyu-syntax`: shared definitions — token kinds, span, file id, diagnostics
- `asatsuyu-lexer`: lexical analysis only; no syntax trees or semantic analysis
- `asatsuyu-parser`: focused on building a lossless CST
- `asatsuyu-ast`: reshaping the CST into a meaningful AST
- `asatsuyu-hir`: name resolution and desugaring
- `asatsuyu-ty`: type inference and type checking
- `asatsuyu-backend-python`: THIR to Python 3.12+ generation
- `asatsuyu-cli`: I/O, diagnostic formatting, subcommand dispatch

```rust
// ✅ Keep dependency direction unidirectional
asatsuyu_cli -> asatsuyu_ty -> asatsuyu_hir -> asatsuyu_ast -> asatsuyu_parser
```

- Lower crates must not depend on upper crates
- Only `cli` knows about terminal display and exit codes
- Analysis crates must not call `std::process::exit`, `println!`, or `eprintln!`

## 4. Data and Ownership

- Pass syntax and type information as value objects; localize side effects
- Retain source mapping (`Span`, `FileId`, `TextRange`) from the start
- Make explicit what is discarded and what is preserved across AST/HIR/THIR conversions
- Avoid overusing `String`; consider dedicated types for identifiers and paths

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub file_id: FileId,
    pub start: u32,
    pub end: u32,
}
```

## 5. Errors and Diagnostics

- Separate internal error types from user-facing diagnostics
- Use `thiserror` domain errors within crates; convert to `miette` etc. in the CLI for final display
- `panic!` is for bugs only. User input errors must always ride on `Result` / `Diagnostic`
- Diagnostics must carry not just a message but `code`, `span`, `labels`, `hints`, and `notes`

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("unexpected token")]
    UnexpectedToken { span: Span },
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
    pub span: Span,
    pub hints: Vec<String>,
}
```

```rust
// ❌ Do not exit directly from library crates
if has_errors {
    std::process::exit(1);
}
```

## 6. API Design

- Name public APIs to directly reflect the crate's responsibility
- Use function names that reveal the pipeline stage: `parse`, `lower_to_hir`, `infer_types`, `emit_python`
- Do not collapse stages with convenience functions; prioritize debuggability
- Prefer straightforward functions over builders when builders are unnecessary

```rust
pub fn parse(file_id: FileId, source: &str) -> ParseResult<Cst>;
pub fn lower(cst: &Cst) -> Ast;
pub fn lower_to_hir(ast: &Ast, db: &mut HirDb) -> Result<Hir, HirError>;
pub fn infer_program(hir: &Hir, db: &mut TyDb) -> Result<Thir, TyError>;
pub fn emit_module(thir: &Thir) -> String;
```

## 7. Pattern Matching and Enums

- Prefer enums for fixed-set branching
- Avoid string-based kind discrimination
- Write `match` exhaustively; do not discard too much information with `_`
- Prefer enum + exhaustive match over small-object polymorphism

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Let,
    Fn,
    Type,
    Ident,
}

match token.kind {
    TokenKind::Let => parse_let(p),
    TokenKind::Fn => parse_fn(p),
    TokenKind::Type => parse_type(p),
    TokenKind::Ident => parse_expr_stmt(p),
}
```

## 8. Async and Concurrency

- The MVP compiler core defaults to synchronous processing
- Use `async` only at boundaries that require external I/O
- Do not put CPU-bound analysis work on `tokio` without justification
- Introduce parallelization only after profiling demonstrates the need

```rust
// ✅ Default to synchronous APIs
pub fn check_file(path: &Path) -> Result<Vec<Diagnostic>, CliError> {
    let source = std::fs::read_to_string(path)?;
    let cst = asatsuyu_parser::parse(FileId(0), &source)?;
    // ...
    Ok(vec![])
}
```

## 9. User Output, Logging, and Tracing

- Do not blanket-ban `println!`
- **Canonical CLI output** goes to `stdout`: success results, generated code, machine-readable JSON
- **Diagnostics, warnings, and progress** go to `stderr`: `miette` output and `eprintln!`
- **Internal instrumentation and debug events** use `tracing`
- Library crates must not produce user-facing output; output policy is centralized in `asatsuyu-cli`
- Fix the stdout/stderr contract for each command: `build`, `run`, `check`, `fmt`, `test`

```rust
// ✅ User-facing CLI result
println!("{python_source}");

// ✅ CLI diagnostics and errors
eprintln!("{report:?}");

// ✅ Internal instrumentation
tracing::debug!(tokens = tokens.len(), "lex finished");
```

```rust
// ❌ Do not produce terminal output in library crates
pub fn infer_program(hir: &Hir) -> Result<Thir, TyError> {
    println!("type inference started");
    todo!()
}
```

```rust
// ✅ Wire the subscriber in the CLI
pub fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,asatsuyu=info"));

    fmt().with_env_filter(filter).without_time().init();
}
```

## 10. CLI Conventions

- `asatsuyu-cli` has an exit code contract per subcommand
- Keep human-readable success messages brief
- If introducing `--json`, do not mix it with human-readable output
- Aggregate error counts before display
- Confine colors and decoration to the CLI boundary
- The first important commands are `check`, `build`, `run`; conventions prioritize these three

Output contracts:

- `check`: exit silently or with a brief success message on success; diagnostics go to `stderr` on failure
- `build`: output paths and results go to `stdout`; progress and warnings go to `stderr`
- `run`: compiler diagnostics go to `stderr`; the generated Python program's stdout passes through to `stdout`

```rust
pub fn run() -> std::process::ExitCode {
    match try_run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            std::process::ExitCode::FAILURE
        }
    }
}
```

## 11. Testing

- Place unit tests close to each crate
- Actively use snapshot tests for parser / formatter / backend
- Use golden tests to pin token sequences and tree shapes in the lexer/parser
- Verify the type checker with diagnostic codes and spans
- Use integration tests for the CLI, checking stdout, stderr, and exit code separately
- Add e2e tests aligned with the roadmap's definition of done

```rust
#[test]
fn parses_let_binding() {
    let cst = parse(FileId(0), "let x = 1");
    assert!(cst.is_ok());
}
```

```rust
#[test]
fn check_command_writes_diagnostics_to_stderr() {
    let output = run_cli(["check", "examples/type_error.asty"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}
```

## 12. Performance

- Measure before optimizing: `cargo bench`, `criterion`, `hyperfine`
- Avoid unnecessary allocations in the lexer/parser
- Do not casually add `clone()` on hot paths
- Prefer span + source slice over string concatenation
- Consider incremental / arena / interning for large inputs, but avoid premature abstraction

## 13. Lints and Formatting

- The CI baseline is `cargo fmt`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`
- Enable pedantic lints but explicitly `allow` noisy ones at the workspace level
- Do not use `unwrap()` carelessly outside tests/prototypes
- Consolidate new lint exceptions in workspace configuration, not in code

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## 14. Unsafe, Panics, and Stability

- `unsafe` is prohibited by default (`unsafe_code = "deny"` workspace-wide). If needed, attach `// SAFETY:` comments
- Use `expect` only in "this is a bug if it fails" contexts
- Never panic on user input or file contents
- Stabilize the error surface of public APIs

## 15. Dependency Policy

- Only add a dependency if you can explain why the crate is necessary
- Prioritize lightweight, debuggable dependencies for the analysis infrastructure
- Use established dependencies (`logos`, `rowan`, `clap`, `miette`, `smol_str`, `la-arena`, `insta`) as the baseline for additions and removals
- Confirm necessity before introducing large dependencies like `syn`, `serde`, or `tokio`
- Keep feature flags minimal to reduce the cognitive cost for users

## 16. Review Checklist

- Has the crate's scope grown too large?
- Is a lower crate taking on CLI or display responsibilities?
- Do diagnostics include span, code, and hints?
- Are `stdout` and `stderr` being mixed?
- Is `tracing` used only for internal instrumentation, not for user-facing messages?
- Does the change violate the pipeline direction from the [architecture document](../architecture.md)?
- Does the change unnecessarily detour from the current roadmap milestone?

## References

- Asatsuyu design: [architecture.md](../architecture.md)
- Implementation roadmap: [roadmap.md](../roadmap.md)
- Rust Book: separating stdout/stderr
- Rust std: `println!`, `eprintln!`
- `tracing` / `tracing-subscriber`
- `miette`
