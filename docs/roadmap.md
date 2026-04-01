# Roadmap

## Overview

Asatsuyu is a statically typed language that compiles to Python 3.12+ source code,
aiming to bring type safety and algebraic data types to the Python ecosystem
while preserving access to Python libraries.
The relationship mirrors TypeScript to JavaScript.

**MVP goal**: Write a 300-500 line CLI application in Asatsuyu using `Result`,
`Option`, `match`, and ADTs, calling `pathlib` as Verified FFI and `requests` as
Checked FFI, and producing readable Python 3.12+ output.

**MVP success criteria** (speed is explicitly not a criterion):

- Type error quality
- Generated code readability
- Low friction when using Python libraries

## Current State

The compiler is implemented in Rust across 9 crates with a strictly
unidirectional dependency graph:

```
asatsuyu-syntax       Shared definitions (SyntaxKind, Span, Diagnostic). Zero external dependencies.
  -> asatsuyu-lexer          logos DFA tokenizer
    -> asatsuyu-parser       Hand-written recursive descent, lossless CST (rowan)
      -> asatsuyu-ast        CST to untyped AST
        -> asatsuyu-hir      Name resolution, desugaring, FFI module resolution
          -> asatsuyu-ty     Hindley-Milner type inference, exhaustiveness checking
            -> asatsuyu-backend-python   THIR to Python 3.12+ source generation
              -> asatsuyu-cli            check, build, run, fmt, new, lsp, lock, add, remove, sync
```

Additionally, `asatsuyu-runtime-python` provides a PyO3 boundary layer for
Checked FFI. It is independent of the compiler crates and does not introduce
`pyo3` into the compiler dependency tree.

**Current metrics**:

- 798+ tests passing
- Working `check`/`build`/`run`/`fmt`/`lsp` commands
- FFI support for `pathlib`/`os`/`sys` (Verified) and `requests`/`json` (Checked)
- Formatter: CST-based, deterministic, zero-config
- LSP: diagnostics, hover, go-to-definition, completion, rename, find-references, document symbols
- CI: 10 jobs (fmt, clippy, test, verify-ffi, maturin-build, pytest, package-install, docs-sync, release-gate)
- Python 3.12 / 3.13 / 3.14 cross-version test matrix

---

## Chapter 1: Core Compiler

Chapter 1 covers Issues 1-52, building the compiler from an empty workspace to
a functioning 5-stage pipeline (CST -> AST -> HIR -> THIR -> Python).

### Milestone 0: Repository Bootstrap (Complete)

**Issues 1-2.**

Established the Rust workspace under `asatsuyu/` with `edition = "2024"` and
`resolver = "3"`. Workspace-level lint configuration enforces `unsafe_code = "deny"`
and `clippy::pedantic` as warnings. GitHub Actions CI runs `fmt`, `clippy`, and
`test` as three parallel jobs.

Key dependencies pinned in `[workspace.dependencies]`:
`logos`, `rowan`, `clap`, `miette`, `smol_str`, `la-arena`, `insta`.

### Milestone 1: Minimal Vertical Slice (Complete)

**Issues 3-10.**

Cut the first end-to-end path: `.asty` source to running Python. This milestone
established the architecture by proving all 5 pipeline stages work together.

Deliverables: unified `SyntaxKind` enum (rust-analyzer pattern: tokens and nodes
in a single `#[repr(u16)]` enum), `Span`/`Diagnostic` types, minimal `logos`
lexer with trivia preservation, recursive descent parser producing lossless CST
via `rowan`, CST-to-AST lowering (separate AST types rather than rowan typed
wrappers), provisional HIR with `la-arena`-based `DefId`, placeholder THIR
type checker, Python emitter with PEP 8 formatting, and CLI `check`/`build`/`run`
commands. Proved with 6 end-to-end tests.

### Milestone 2: Lexer and Parser (Complete)

**Issues 11-18.**

Expanded the grammar to cover the full MVP surface.

Lexer: 48 token kinds (14 keywords, 3 literal types, 18 operators, 7 delimiter
pairs, trivia), 57+ snapshot tests. `logos` priority rules resolve ambiguities
(`|>` vs `|`, `FloatLit` vs `IntLit`, `_` vs `Ident`).

Parser: Pratt parsing for operator precedence (matklad approach), `if`/`else`
expressions, `|>` pipeline (left-associative), ADT type definitions (Go-style
flat records and named-constructor variants, auto-detected after `{`), `match` with 5
pattern kinds and guard expressions, `TokenSet`-based error recovery (rust-analyzer
`u128` bitset) with top-level synchronization. Progress guards on all loops
prevent infinite parsing. 33+ malformed input tests.

### Milestone 3: AST and HIR (Complete)

**Issues 19-22.**

AST expansion:

- `TypeExpr` with recursive type parameters (`List(Int)`, `Result(Float, String)`)
- `TypeBody` enum distinguishing records (`TypeBody::Record`) from variants (`TypeBody::Variants`)
- `Import` enum with `Module` and `Python` variants
- `Pattern` enum: Wildcard, Variable, Literal, Constructor, List
- Full expression coverage: Match, If, Pipeline, BinaryOp, UnaryOp, Call

HIR layer:

- `ScopeStack` (Vec of HashMaps) for lexical scope management with push/pop
- 7 `DefKind` variants: Function, Parameter, LocalBinding, Constructor, Type, Builtin, Import
- Name resolution assigns `DefId` to all references (variables, constructors, types, imports)
- 3-pass import resolution: imports first, then functions/types/constructors
- Duplicate definition detection with secondary labels pointing to prior definition
- Local shadowing is silent (same convention as Rust)

Desugaring (removed from HIR, invisible to type inference):

- Pipeline `|>`: `x |> f(y)` becomes `f(x, y)` (first-argument insertion, ML/Elixir convention)
- String concatenation `<>`: `"a" <> "b"` becomes `string_concat("a", "b")` via built-in function

### Milestone 4: Type Inference (Complete)

**Issues 23-28.**

The type inference engine is the compiler's core.

Unification: substitution-based (HashMap, not Union-Find), with `shallow_resolve`
and full `resolve`. Rules cover Var-Var, Var-concrete, Primitive identity,
Function structural recursion, Named (ADT) with pairwise argument unification,
and Error absorption. Occurs check prevents infinite types.

Let-polymorphism: `TypeScheme { vars, ty }` with `generalize` (ftv(ty) minus
ftv(env)) and `instantiate` (fresh variables for each quantified variable).
`let` bindings and lambda expressions added across all pipeline stages.

ADT typing: `Ty::Named { def_id, name, args }` for user-defined types.
Constructor type schemes follow standard ML convention: `Some(a)` gets
`forall a. a -> Option(a)`, `None` gets `forall a. Option(a)`.

Match typing: simplified Maranget exhaustiveness tracking remaining variants in
a `HashSet<DefId>`. Unreachable arm detection produces warnings. Pattern type
checking unifies constructor fields with subject type.

Diagnostics: `DiagnosticCode` enum (E0200-E0304), expected/actual type display,
`DiagnosticContext` enum for targeted labels, hints, and notes.

### Milestone 5: Python Backend (Complete)

**Issues 29-34.**

The Python emitter targets Python 3.12+ and produces human-readable output.

Code generation: ADTs emit as `@dataclass(frozen=True, slots=True)` with PEP 695
type parameters (`class Some[T]:`). Multi-variant types produce `type` aliases.
`Option(T)` maps to `T | None`. Match compiles to `match/case` (PEP 634).
Pipelines emit temporary variables (`_pipe0`, `_pipe1`). String concatenation
`<>` emits `+`. Python reserved words are sanitized (`None` -> `None_`).

Package output: complete package tree (`__init__.py`, `__main__.py`,
`asatsuyu_prelude.py` with `Ok`/`Error`/`Result`/`PyException` emitted only when
needed, `pyproject.toml`). Optional source-map comments (`# asty:L12`) via
`--source-map` flag.

### Milestone 6: CLI DX (Complete)

**Issues 35-37.**

- `asatsuyu check`: parse + typecheck without codegen, `miette` diagnostics with
  source context, labels, and colored output
- `asatsuyu build`: generates package tree to `dist/`, prints summary to stderr
  (`Compiled hello (4 files) -> dist/`), outputs directory path to stdout
- `asatsuyu run`: builds to `target/run/`, executes via `python3 -m <package>`,
  propagates Python exit code
- `asatsuyu new <name>`: project scaffolding with `src/main.asty`,
  `asatsuyu.toml`, `.gitignore`, name validation, and immediate `run` capability

### Milestone 7: FFI (Complete)

**Issues 38-51.**

The FFI system implements a three-tier trust model that separates type resolution
from soundness guarantees:

**Verified FFI** (pathlib, os, sys):

- Complete type information with no `Any` in exported surface
- Symbols flow into THIR as normal Asatsuyu types
- CI-verified with `pyright --verifytypes` and `mypy stubtest`
- Generated code uses standard Python imports (no runtime overhead)

**Checked FFI** (requests, json):

- Static type info exists but cannot be treated as sound
- Compiler generates Python wrappers with argument/return validation
- Exception-to-`Result` conversion via `try/except`
- Backed by PyO3 runtime extension (`_asatsuyu_runtime`) for dynamic dispatch
- `Response.json() -> Any` routes through runtime validator

**Unsafe/Opaque FFI**:

- Dynamic surfaces isolated as `PyOpaque[module.Symbol]` types
- Field access and pattern matching are prohibited (E0209, E0214 diagnostics)
- Values can only be passed to other foreign calls or explicitly converted

Key technical decisions:

- `from python import X` syntax with `python` as a reserved keyword
- Hand-written FFI signatures in `builtins.rs` (typeshed parsing deferred to post-MVP)
- `try` as a prefix operator converting Python exceptions to `Result` (Rust `?` semantics)
- `PyException` with 9-category classification (`IoError`, `ValueError`, `TypeError`,
  `KeyError`, `AttributeError`, `ImportError`, `ArithmeticError`, `RuntimeError`, `Other`)
  and `traceback.format_exception` for traceback preservation
- maturin mixed layout for packages containing Checked FFI
- `_asatsuyu_runtime` PyO3 crate with `import_module`, `call_function`, `call_method`,
  `normalize_exception`, and custom `AsatsuyuError` exception
- Compiler flags: `--ffi-stdlib-only`, `--ffi-runtime on|off|auto`,
  `--no-emit-package`, `--ffi-stub-path`
- Cross-version CI matrix: Python 3.12, 3.13, 3.14

### Milestone 8: MVP Validation (In Progress)

**Issue 52.**

The final validation milestone: build a 300-500 line CLI application in Asatsuyu
that exercises `Result`/`Option`/`match`/ADT with `pathlib` as Verified FFI and
`requests` as Checked FFI. This is a demonstration of the full pipeline,
not a new feature.

Success is measured against the three MVP criteria: type error quality,
generated code readability, and friction when using Python libraries.

---

## Chapter 2: DX and Practicality

Chapter 2 shifts focus from language features to developer experience, local
dependency management, regression prevention, and editor integration.

**In scope**: diagnostic stability, watch mode, project configuration,
dependency workflows, golden test suites, formatter, LSP.

**Out of scope**: package registry, async/await, JIT, native optimization,
multi-backend, independent environment management.

### Phase 2-1: Diagnostics and CLI DX (Complete)

**Issues 52-55.**

Diagnostic contract: 44 codes in a unified `E0xxx` scheme (E0001-E0049 Lexer,
E0050-E0099 Parser, E0100-E0149 AST, E0150-E0199 HIR, E0200-E0299 Type,
E0300-E0399 Match, E0400-E0499 Backend). Primary labels use "expected X, found Y"
format. Hints are imperative, notes declarative. 32 snapshot tests fix output.

Machine-readable output: `--error-format human|json` on `check`/`build`/`run`.
JSON mode emits NDJSON (rustc-compatible), always ending with `{"type":"summary"}`.
1-based line/column (matches rustc, Ruff, ESLint).

Watch mode: `asatsuyu check --watch` with `notify` v8 and 250ms debounce.
Project root discovery via `asatsuyu.toml`, automatic `src/**/*.asty` enumeration.

CLI normalization: exit codes 0/1/2 (ruff/ty convention), stdout/stderr separation,
unified `emit_final_summary` for error/warning counts.

### Phase 2-2: Dependency Management (Complete)

**Issues 56-60.**

Project configuration:

- `asatsuyu.toml` schema: `[project]` (required), `[python]`, `[python-dependencies]`,
  `[ffi]`, `[tool]` sections, plus optional `schema_version` top-level field
- Unknown keys rejected via `serde(deny_unknown_fields)` (except `[tool]`, which
  follows the pyproject.toml extensibility pattern)
- PEP 440 version specifier validation (`pep440_rs`) for dependencies and Python version

Environment resolution:

- Python discovery: explicit `--python-path` > `VIRTUAL_ENV` > `.venv/` > PATH `python3`
- Package scanning: `site-packages/*.dist-info/METADATA` filesystem scan (no subprocess)
- PEP 503 name normalization (lowercase, `[-_.]` to `-`)
- Missing dependency: warning on `check`, error on `run`

Package output:

- Standards-compliant `pyproject.toml` with `[build-system]`, `[project.dependencies]`,
  `[project.scripts]` (auto-generated when `main()` exists), `[tool.asatsuyu]`
- setuptools backend for pure Python; maturin for Checked FFI packages

Dependency workflows:

- `asatsuyu lock`: delegates to `uv pip compile` (preferred) or `pip lock` for
  PEP 751 `pylock.toml` generation. Staleness detection on check/build/run
- `asatsuyu add <pkg> [specifier]`: format-preserving TOML editing via `toml_edit`,
  automatic re-lock
- `asatsuyu remove <pkg>`: removes from `[python-dependencies]`, re-lock,
  cleans pylock.toml if no dependencies remain
- `asatsuyu sync`: installs from `pylock.toml` via `uv pip sync` or per-package
  `pip install` fallback

### Phase 2-3: Test Hardening (Complete)

**Issues 61-65.**

Golden test suite: 53 cases across all pipeline stages (AST, HIR, THIR, Python,
diagnostics) with `insta::glob!` auto-discovery. 240+ snapshots, updated only
via `cargo insta review`.

Executable fixtures: 5 projects under `fixtures/projects/` (`hello_cli`,
`pathlib_walk`, `stdlib_ffi`, `requests_client`, `build_install`) with 17 test
functions covering check/build/run. Includes `new -> populate -> build -> run`
end-to-end test.

FFI conformance CI: 3-layer gate with Rust unit tests (12 tests, insta snapshots),
Python runtime introspection (42 tests), and E2E trust summary assertions.

Crash safety: 30-file malformed input corpus with `catch_unwind` verification.
Edge cases include empty strings, null bytes, 500-deep nesting, 100K-character
identifiers. A parser infinite loop bug was discovered and fixed via this corpus.

Release gates: `release-gate` CI job aggregating all upstream jobs, `docs-sync`
checker validating code-documentation consistency, release checklist at
`docs/release-checklist.md`.

### Phase 2-4: Formatter (Complete)

**Issue 66.**

Implemented as `asatsuyu-parser::format` (same crate as parser, not a separate crate).

- Direct CST walker (no Wadler-Lindig Doc IR; line-width-based wrapping deferred)
- Zero configuration, single canonical format (opinionated, like `gofmt`)
- 2-space indent, single blank line between top-level definitions, trailing newline,
  no trailing whitespace
- Full comment preservation via `collect_inter_item_comments`,
  `collect_block_comments`, `collect_match_arm_comments`
- Parse errors: returns original text unchanged
- `asatsuyu fmt` with `--check` mode (exit 1 on diff, for CI)
- Idempotency and roundtrip stability verified against all 53 golden fixtures
- 14 unit tests + golden fixture verification

### Phase 2-5: LSP (Complete)

**Issues 67-68.**

Implemented as `asatsuyu-cli::lsp` using `tower-lsp` v0.20 and `tokio` v1,
launched via `asatsuyu lsp` (LSP integrated into the compiler binary).

Full-text document synchronization with re-analysis on `did_save` and `did_open`.
Per-file state holds source, line index, THIR, and symbol table.

Implemented features: diagnostics (severity, codes, labels, related information),
hover (type display for expressions and function signatures), go-to-definition
(`DefId` to source span), document formatting (CST formatter integration),
completion (scope reconstruction from SymbolTable + THIR, position-aware),
rename (`DefId`-based all-references rewrite with `prepare_rename` validation),
find-references, and document symbols.

A minimal VS Code extension is provided under `editors/vscode/`.

---

## What Comes Next

The following items are explicitly deferred beyond the current roadmap.
They may be revisited after the MVP is validated and the core is stable.

**Language features**:

- `async`/`await`
- Effect system / macro system
- Class / inheritance / trait abstractions
- Mutable variables
- Dependent types / refinement types

**Compiler infrastructure**:

- `.pyc` direct generation
- JIT / native optimization
- Multi-backend targets
- Incremental compilation (Salsa-based)
- Cross-module type checking

**Ecosystem**:

- Package registry
- Deep integration for `numpy`, `pandas`, `torch`
- `Any` parameter variance in FFI
- Typeshed parsing (replacing hand-written FFI signatures)
- Stub auto-generation from Python modules
