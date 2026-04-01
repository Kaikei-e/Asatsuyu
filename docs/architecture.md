# Asatsuyu -- Compiler Architecture Design Notes v2

## Concept

```
Asatsuyu source code (.asty)
  -> [Rust compiler: CST -> AST -> HIR -> THIR -> Python emitter]
Python 3.12+ source code (.py)
  -> [CPython 3.12+]
Execution (Python ecosystem assets available with tiered trust levels)
```

Asatsuyu does for Python what TypeScript did for JavaScript.
The compiler infrastructure is built in Rust. The target runtime is fixed at Python 3.12+,
leveraging modern features such as PEP 695 (type parameter syntax) and PEP 634
(structural pattern matching) to maximize the clarity and runtime performance of generated code.

Asatsuyu is not about "inventing a language." The immediate priority is
**shipping a single end-to-end type-safe Python frontend**.

---

## Fixed Policies

### In Scope

1. **The backend is fixed to Python source generation**
   - Debuggability, observability, and CPython compatibility are top priorities
   - Direct `.pyc` generation is relegated to the research track
2. **Runtime target is Python 3.12+**
   - PEP 695 (`type` statement, type parameter syntax) and PEP 634 (`match/case`) are assumed
3. **FFI consumes `.pyi` / typeshed / PEP 561**
   - No bespoke declaration format is invented upfront
4. **Language design: functional, explicit, small**
   - Curly braces, explicit blocks, pipelines, ADTs, exhaustive match, Result/Option
   - Exceptions are not a language feature
   - Surface syntax draws from the ML/Gleam family, but the design center is Python interop — not Erlang, BEAM, or any other runtime
5. **Exceptions are banished from the language interior and absorbed only at the Python boundary**
   - Exceptions thrown by Python APIs are wrapped into `Result` at the boundary layer

### Non-Goals (First Year)

- Mainlining direct `.pyc` generation
- JIT / native optimization
- Classes / inheritance / trait-style abstraction
- Full native async/await design
- Effect system / macro system
- Package registry
- Multi-backend support
- Dependent types / refinement types
- Mutable variables beyond scoped locals (Phase 3-1 introduces `let mut` for local bindings)

---

## Crate Layout and Data Flow

```
Source code (.asty)
  |
  v
+-------------------+
| asatsuyu-lexer    |  Fast lexical analysis via logos DFA
+--------+----------+
         v
+-------------------+
| asatsuyu-parser   |  Hand-written recursive descent -> lossless CST (rowan)
+--------+----------+
         v
+-------------------+
| asatsuyu-ast      |  CST -> untyped AST conversion
+--------+----------+
         v
+-------------------+
| asatsuyu-hir      |  Name resolution, desugaring, FFI resolution
| (includes builtin |  (builtin type surfaces for Python FFI)
|  FFI type surfaces)|
+--------+----------+
         v
+-------------------+
| asatsuyu-ty       |  HM type inference & checking -> THIR (Typed HIR)
+--------+----------+
         v
+---------------------------+
| asatsuyu-backend-python   |  THIR -> Python 3.12+ source generation
+--------+------------------+
         v
      Output (.py + pyproject.toml)
```

Cross-cutting crates:
```
asatsuyu-syntax           Shared type definitions used by all crates (token kinds, CST node kinds, Span, Diagnostic)
asatsuyu-runtime-python   PyO3 runtime boundary for Checked FFI validation (independent of compiler crates)
asatsuyu-cli              Entry point (check / build / run / fmt / test)
asatsuyu-parser::format   CST-based code formatter (implemented as a module within the parser crate)
asatsuyu-cli::lsp         Language Server Protocol implementation (tower-lsp, implemented as a module within the CLI crate)
```

The pipeline has **5 stages: CST -> AST -> HIR -> THIR -> Python emitter**.
Because Asatsuyu is a type-safe frontend language rather than a natively optimized one,
additional IRs such as MIR or optimization passes are deliberately avoided.

---

## Role of Each Crate

### asatsuyu-syntax

The **lowest-level shared definitions crate** depended upon by all other crates. Targets zero external dependencies.

```rust
/// Syntax element kinds (tokens and nodes in a single enum, rust-analyzer pattern)
///
/// Maps 1:1 to rowan's `SyntaxKind(u16)`.
/// Layout invariant: token kinds precede `Eof`; node kinds follow `Eof`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum SyntaxKind {
    // === Tokens: Keywords (14) ===
    FnKw, PubKw, LetKw, TypeKw, MatchKw,
    IfKw, ElseKw,
    ImportKw, FromKw, PythonKw, AsKw,
    TrueKw, FalseKw,
    TryKw,
    MutKw,                             // hard keyword (Phase 3-1)
    AsyncKw, AwaitKw,                  // reserved (Phase 3-2)

    // === Tokens: Literals ===
    IntLit, FloatLit, StringLit,

    // === Tokens: Identifiers ===
    Ident,

    // === Tokens: Operators ===
    Plus, Minus, Star, Slash, Percent,   // arithmetic
    Eq, EqEq, BangEq,                   // assignment / comparison
    Lt, LtEq, Gt, GtEq,                 // comparison
    Bang, Ampersand, PipeSingle,         // unary / bitwise
    AmpAmp, PipePipe,                    // logical && , ||
    Pipe,                                // |> pipeline
    StringConcat,                        // <>

    // === Tokens: Delimiters / Punctuation ===
    LParen, RParen, LBrace, RBrace, LBracket, RBracket,
    Comma, Colon, Semicolon, Dot, DotDot,
    Arrow,                               // ->
    Underscore,                          // _

    // === Tokens: Trivia ===
    Whitespace, Newline, Comment,

    // === Tokens: Special ===
    Error, Eof,

    // --- Node kinds (after Eof) ---------------------------------

    // === Nodes: Top-level ===
    SourceFile, FnDef, TypeDef,
    ImportStmt, FromPythonImportStmt, LetStmt, AssignStmt,

    // === Nodes: Expressions ===
    LiteralExpr, IdentExpr, CallExpr, PipelineExpr,
    MatchExpr, IfExpr, LambdaExpr, BlockExpr,
    BinaryExpr, UnaryExpr, FieldAccessExpr,
    ListExpr, TupleExpr, RecordExpr, ParenExpr, TryExpr,

    // === Nodes: Patterns ===
    WildcardPat, IdentPat, LiteralPat,
    ConstructorPat, ListPat, TuplePat,

    // === Nodes: Types ===
    TypeExpr, TypeParam,

    // === Nodes: ADT ===
    Variant, Field,

    // === Nodes: Match ===
    MatchArm, Guard,

    // === Nodes: Parameters ===
    Param, ParamList, ArgList,

    // === Nodes: Other ===
    ReturnType, Visibility, Path, NodeError,
}

/// Source location (u32-based, same approach as Ruff)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub file_id: FileId,
    pub start: u32,
    pub end: u32,
}

/// Diagnostic message (aiming for rich, helpful error messages with fix suggestions)
#[derive(Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: DiagnosticCode,
    pub message: String,
    pub span: Span,
    pub labels: Vec<Label>,    // point to multiple locations
    pub hints: Vec<String>,    // suggested fixes
    pub notes: Vec<String>,    // additional explanation
}

#[derive(Debug)]
pub struct Label {
    pub span: Span,
    pub message: String,
    pub style: LabelStyle,  // Primary / Secondary
}
```

Each `Diagnostic` carries a `code` (e.g. `E0001`) to enable future linking to
documentation pages. Pretty terminal rendering via the miette crate is handled
in asatsuyu-cli.

---

### asatsuyu-lexer

Fast lexical analysis powered by `logos`. Because logos generates a deterministic finite
automaton at compile time, it achieves performance on par with a hand-written state machine.

```rust
use logos::Logos;
use asatsuyu_syntax::{Diagnostic, DiagnosticCode, FileId, Span, SyntaxKind};

/// Internal logos token enum. Unified via From conversion to SyntaxKind.
/// Priority rules: #[token] > #[regex]; longer match > shorter match
#[derive(Logos, Debug, Clone, Copy, PartialEq)]
enum LexToken {
    // === Keywords (14) ===
    #[token("fn")]     FnKw,
    #[token("pub")]    PubKw,
    #[token("let")]    LetKw,
    #[token("type")]   TypeKw,
    #[token("match")]  MatchKw,
    #[token("if")]     IfKw,
    #[token("else")]   ElseKw,
    #[token("import")] ImportKw,
    #[token("from")]   FromKw,
    #[token("python")] PythonKw,
    #[token("as")]     AsKw,
    #[token("True")]   TrueKw,
    #[token("False")]  FalseKw,
    #[token("try")]    TryKw,
    #[token("mut")]    MutKw,     // hard keyword (Phase 3-1)
    #[token("async")]  AsyncKw,   // reserved
    #[token("await")]  AwaitKw,   // reserved

    // === Literals ===
    #[regex("[0-9]+\\.[0-9]+")]  FloatLit,
    #[regex("[0-9]+")]           IntLit,
    #[regex(r#""[^"]*""#)]       StringLit,

    // === Identifiers ===
    #[regex("[a-zA-Z_][a-zA-Z0-9_]*")]  Ident,

    // === Operators ===
    #[token("|>")]  Pipe,
    #[token("<>")]  StringConcat,
    #[token("&&")]  AmpAmp,
    #[token("||")]  PipePipe,
    #[token("==")]  EqEq,
    #[token("!=")]  BangEq,
    #[token("<=")]  LtEq,
    #[token(">=")]  GtEq,
    // Single-character operators...
    #[token("+")]  Plus,
    #[token("-")]  Minus,
    #[token("*")]  Star,
    #[token("/")]  Slash,
    #[token("%")]  Percent,
    #[token("=")]  Eq,
    #[token("<")]  Lt,
    #[token(">")]  Gt,
    #[token("!")]  Bang,
    #[token("&")]  Ampersand,
    #[token("|")]  PipeSingle,

    // === Delimiters / Punctuation ===
    #[token("(")]  LParen,  #[token(")")]  RParen,
    #[token("{")]  LBrace,  #[token("}")]  RBrace,
    #[token("[")]  LBracket, #[token("]")] RBracket,
    #[token(",")]  Comma,   #[token(":")]  Colon,
    #[token(";")]  Semicolon,
    #[token("..")]  DotDot,  #[token(".")]  Dot,
    #[token("->")]  Arrow,
    #[token("_", priority = 3)]  Underscore,

    // === Trivia (not skipped -- required for lossless CST) ===
    #[regex(r"[ \t\r]+")]          Whitespace,
    #[token("\n")]                 Newline,
    #[regex("//[^\n]*")]           Comment,
}

pub struct Token {
    pub kind: SyntaxKind,
    pub span: Span,
    pub text: SmolStr,  // interned string
}

pub fn lex(source: &str, file_id: FileId) -> (Vec<Token>, Vec<Diagnostic>) {
    let mut tokens = Vec::with_capacity(source.len() / 3 + 1);
    let mut diagnostics = Vec::new();
    let mut lexer = LexToken::lexer(source);
    // Convert each logos token to SyntaxKind, collect errors into diagnostics
    // Append an Eof token at the end
    (tokens, diagnostics)
}
```

Completion criteria:
- 30+ `lexer_snapshot_tests` pass
- Spans are never corrupted
- Newlines and comments are retained as trivia

---

### asatsuyu-parser

**Hand-written recursive descent parser**. This follows the precedent set by Ruff,
which switched from a generated parser to hand-written in v0.4.0, achieving over 2x
speedup and dramatically improved error diagnostics.

Output is a **rowan-based lossless CST**. Rowan is a general-purpose lossless
syntax tree library that retains comments, whitespace, and broken tokens. This allows
the formatter to fully reconstruct source from the CST.

```rust
use rowan::{GreenNode, GreenNodeBuilder};
use asatsuyu_syntax::{DiagnosticCode, SyntaxKind};

pub struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    builder: GreenNodeBuilder<'static>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Parser<'a> {
    pub fn parse(tokens: &'a [Token]) -> (GreenNode, Vec<Diagnostic>) {
        let mut parser = Parser::new(tokens);
        parser.parse_source_file();
        (parser.builder.finish(), parser.diagnostics)
    }

    fn parse_source_file(&mut self) {
        self.start_node(SyntaxKind::SourceFile);
        while !self.at_eof() {
            self.parse_top_level();
        }
        self.eat_trivia();
        self.finish_node();
    }

    fn parse_top_level(&mut self) {
        match self.current() {
            SyntaxKind::FnKw => self.parse_fn_def(),
            SyntaxKind::TypeKw => self.parse_type_def(),
            SyntaxKind::PubKw => match self.nth(1) {
                SyntaxKind::TypeKw => self.parse_type_def(),
                _ => self.parse_fn_def(),
            },
            SyntaxKind::ImportKw => self.parse_import(),
            SyntaxKind::FromKw => self.parse_from_python_import(),
            SyntaxKind::LetKw => {
                self.error_recover("not yet implemented", DiagnosticCode::E0062);
            }
            _ => self.error_recover("expected item definition", DiagnosticCode::E0051),
        }
    }

    fn parse_expr(&mut self) {
        self.parse_expr_bp(0)  // Pratt parsing (operator precedence parsing)
    }
}
```

Design decisions:
- **Pratt parsing** for expression operator precedence (same approach as rustc)
- **Error recovery**: synchronization points at keywords like `fn`, `type`, `let`
  prevent a single syntax error from halting the entire parse
- **Lossless CST**: parse -> print -> parse round-trips without structural loss
- Separating CST from AST lets the formatter and compiler share the same syntactic foundation

Completion criteria:
- First error position is correct even on broken input
- 20+ malformed input tests pass

---

### asatsuyu-ast

**Untyped AST** converted from the CST. Trivia (whitespace and comments) is stripped,
and the tree is normalized into a structure convenient for subsequent compiler phases.

```rust
pub struct Module {
    pub imports: Vec<Import>,
    pub definitions: Vec<Definition>,
    pub span: Span,
}

pub enum Import {
    /// Asatsuyu module import
    Asatsuyu { path: Vec<Ident>, items: Vec<ImportItem>, span: Span },
    /// Python module import: from python import numpy as np
    Python { module: String, alias: Option<Ident>, span: Span },
}

pub enum Definition {
    Function(FnDef),
    TypeAlias(TypeAlias),
    CustomType(CustomType),
    Constant(ConstDef),
}

pub struct FnDef {
    pub name: Ident,
    pub visibility: Visibility,
    pub params: Vec<Param>,
    pub return_type: Option<TypeExpr>,
    pub body: Expr,
    pub span: Span,
}

pub enum Expr {
    Literal(Literal, Span),
    Variable(Ident, Span),
    Call { func: Box<Expr>, args: Vec<Expr>, span: Span },
    Lambda { params: Vec<Param>, body: Box<Expr>, span: Span },
    Let { pattern: Pattern, value: Box<Expr>, body: Box<Expr>, span: Span },
    Match { subject: Box<Expr>, arms: Vec<MatchArm>, span: Span },
    If { cond: Box<Expr>, then_: Box<Expr>, else_: Box<Expr>, span: Span },
    Pipeline { left: Box<Expr>, right: Box<Expr>, span: Span },
    List(Vec<Expr>, Span),
    Tuple(Vec<Expr>, Span),
    Record { fields: Vec<(Ident, Expr)>, span: Span },
    FieldAccess { expr: Box<Expr>, field: Ident, span: Span },
    Block(Vec<Expr>, Span),
    StringConcat { left: Box<Expr>, right: Box<Expr>, span: Span },
    BinaryOp { op: BinOp, left: Box<Expr>, right: Box<Expr>, span: Span },
    UnaryOp { op: UnOp, expr: Box<Expr>, span: Span },
}

pub enum Pattern {
    Wildcard(Span),
    Variable(Ident, Span),
    Literal(Literal, Span),
    Constructor { name: Path, fields: Vec<Pattern>, span: Span },
    List { head: Vec<Pattern>, tail: Option<Box<Pattern>>, span: Span },
    Tuple(Vec<Pattern>, Span),
}

pub struct CustomType {
    pub name: Ident,
    pub visibility: Visibility,
    pub type_params: Vec<Ident>,
    pub variants: Vec<Variant>,
    pub span: Span,
}
```

Every AST node retains a `Span`, ensuring that error messages in later phases
can point to accurate source locations.

---

### asatsuyu-hir

**High-level Intermediate Representation**. Performs name resolution and desugaring
so that the type inference phase operates without knowledge of syntactic conveniences.

```rust
pub struct HirModule {
    pub definitions: Vec<HirDef>,
    pub imports: Vec<ResolvedImport>,
    pub symbol_table: SymbolTable,
}

pub enum ResolvedImport {
    Asatsuyu { module: ModuleId, items: Vec<ImportItem> },
    Python { module: String, alias: Option<String>, type_info: PythonModuleType },
}

/// All expressions are name-resolved via DefId
pub enum HirExpr {
    Literal(Literal, Span),
    Var(DefId, Span),
    Call { func: Box<HirExpr>, args: Vec<HirExpr>, span: Span },
    Lambda { params: Vec<HirParam>, body: Box<HirExpr>, span: Span },
    Let { binding: DefId, value: Box<HirExpr>, body: Box<HirExpr>, span: Span },
    Match { subject: Box<HirExpr>, arms: Vec<HirMatchArm>, span: Span },
    If { cond: Box<HirExpr>, then_: Box<HirExpr>, else_: Box<HirExpr>, span: Span },
    // Pipeline |> is desugared (converted to plain function application)
    // StringConcat <> is also desugared
    Block(Vec<HirExpr>, Span),
}
```

Desugaring examples:

```
// Asatsuyu source
xs |> list.filter(fn(x) { x > 0 }) |> list.map(fn(x) { x * 2 })

// HIR (after desugaring)
list.map(list.filter(xs, fn(x) { x > 0 }), fn(x) { x * 2 })
```

```
// Asatsuyu source
"Hello, " <> name <> "!"

// HIR (after desugaring)
string.concat(string.concat("Hello, ", name), "!")
```

This phase performs:
- Name resolution (scope table construction, Symbol -> DefId)
- Pipeline `|>` -> function application desugaring
- String concatenation `<>` -> `string.concat` desugaring
- `if` expression chain normalization
- Python import resolution (via builtin type surfaces in `asatsuyu-hir/src/ffi/`)
- Diagnostics for unused variables and unresolved identifiers

Completion criteria:
- HIR retains no syntactic sugar
- Type inference operates on HIR alone

---

### asatsuyu-ty

Type inference and type checking engine. **The heart of the compiler**.
Accepts HIR and returns THIR (Typed HIR).

```rust
pub enum Type {
    Primitive(PrimType),                          // Int, Float, String, Bool
    Function { params: Vec<Type>, ret: Box<Type> },
    Named { id: TypeId, args: Vec<Type> },        // ADT: Option(a), Result(a, e)
    Var(TypeVarId),                               // type variable during inference
    Tuple(Vec<Type>),
    List(Box<Type>),
    Record { fields: Vec<(String, Type)> },
    Python { module: String, name: String, args: Vec<Type> }, // from typeshed
}

pub enum Constraint {
    Eq(Type, Type, Span),
    Instantiate(Type, TypeScheme, Span),
}

pub struct TypeChecker {
    constraints: Vec<Constraint>,
    substitution: Substitution,
    type_env: TypeEnv,
}

impl TypeChecker {
    pub fn check(&mut self, module: &HirModule) -> Result<TypedModule, Vec<Diagnostic>> {
        // 1. Constraint generation (HIR walk -> collect Constraints)
        // 2. Constraint solving (Unification + occurs check)
        // 3. Generalization / instantiation (let-polymorphism)
        // 4. Type concretization (type variables -> resolved types)
        // 5. Exhaustiveness checking (pattern coverage in match expressions)
        // 6. THIR construction (HIR with resolved types on every node)
    }
}
```

**In scope:**
- Type variables, unification, occurs check
- let-polymorphism (`let id = fn(x) { x }` -> `forall a. a -> a`)
- Function types, ADTs, tuples, lists
- Explicit type annotation override
- Exhaustiveness checking (including unreachable arm detection)
- Type errors that show "expected type / actual type"

**Not in scope (first year):**
- Dependent types / refinement types
- Higher-kinded type system
- Effect row polymorphism
- Type class-style abstraction

Python type boundary mapping (3.12+ assumed):

| Asatsuyu | Python 3.12+ |
|---|---|
| `Int` | `int` |
| `Float` | `float` |
| `String` | `str` |
| `Bool` | `bool` |
| `List(T)` | `list[T]` |
| `Dict(K, V)` | `dict[K, V]` |
| `Option(T)` | `T \| None` |
| `Result(T, E)` | `Ok[T] \| Error[E]` |
| `Tuple(A, B)` | `tuple[A, B]` |
| Python dynamic / unknown | `PyOpaque[module.Symbol]` |

Completion criteria:
- 100-200 type inference tests are stable
- Type errors report "what conflicted with what"
- Generic functions can be inferred without type annotations

---

### FFI Architecture

Asatsuyu's FFI bridges the gap to the Python ecosystem. Its role is not to treat
arbitrary Python libraries as "fully sound." Asatsuyu maintains a
**sound core + verified boundary** guarantee. PEP 561 provides a distribution
convention for type information but does not guarantee the correctness of stubs
themselves. The FFI layer therefore does not simply read `.pyi` files -- it
**evaluates admissibility and assigns a trust level** to each imported symbol.

The current implementation uses hand-crafted builtin type surfaces within
`asatsuyu-hir/src/ffi/` rather than a standalone crate. The design principles below
(Verified/Checked/Unsafe tiers, admissibility checking) are real and implemented.

```rust
pub enum FfiTrustLevel {
    /// Static contract can flow directly into THIR
    Verified,
    /// Accepted only with a runtime wrapper / validator
    Checked,
    /// Isolated as an opaque value outside the sound world
    Unsafe,
}

pub struct PythonSymbolType {
    pub module: SmolStr,
    pub symbol: SmolStr,
    pub ty: ForeignType,
    pub trust: FfiTrustLevel,
}

pub enum ForeignType {
    Direct(Type),
    Opaque { module: SmolStr, symbol: SmolStr },
}

pub fn load_python_module_type(module: &str) -> Result<PythonModuleType, Error> {
    // Resolution priority:
    // 1. Inline type info (py.typed marker)
    // 2. Stub-only package (*-stubs)
    // 3. Bundled typeshed
    // 4. Project-local supplement metadata (Unsafe only)
}
```

#### FFI Soundness Model

1. **Verified FFI**
   - Type information can be resolved from `py.typed` / stub package / typeshed
   - No `Any`, bare generic, or partial-stub-derived unknown remains in the exported surface
   - Type completeness can be checked in CI via pyright `--verifytypes`; stub/runtime divergence can be audited via mypy `stubtest`
   - Treated as ordinary types on the Asatsuyu side

2. **Checked FFI**
   - Statically accepted but not treated as sound without validation
   - The compiler generates Python wrappers that check arguments, return values, and exceptions at the boundary
   - APIs with dynamic shape (JSON, dict, third-party response objects) are placed here
   - `requests` enters through this tier in the MVP

3. **Unsafe / Opaque FFI**
   - `Any`, dynamic attributes, `__getattr__`, and monkey-patch-dependent surfaces are isolated here
   - Visible from Asatsuyu only as `PyOpaque[module.Symbol]`
   - Field access, pattern matching, and implicit conversions are prohibited
   - The only permitted operations are passing the opaque value to another foreign call that accepts the same opaque type, and explicit checked conversion via a boundary function

#### FFI Admissibility Rules

- **Accepted types (MVP)**
  - `int`, `float`, `str`, `bool`, `None`
  - `list[T]`, `tuple[...]`, `dict[K, V]`, `Literal`, `Union`, `Optional`, `TypedDict`
  - Fully-known generic classes
  - Narrowly-scoped `Protocol`
- **Rejected for Verified tier**
  - `Any`
  - Bare generic (no type arguments)
  - Partial stub packages
  - Surfaces dependent on `__getattr__`
  - APIs that require a plugin to close their types

#### Implementation Strategy

- Read `.pyi` / typeshed and normalize Python typing into Asatsuyu's FFI IR
- The importer evaluates admissibility and assigns `Verified / Checked / Unsafe`
- `Checked` symbols receive generated wrappers with `try/except` and runtime validators
- In the MVP, validators leverage existing Python typing assets for rapid bootstrapping; a zero-dependency generated validator can replace them later
- Prefer `Protocol` and `TypedDict` over trusting entire large concrete classes

#### Coverage Phases

- **Phase 1 (Verified)**: `pathlib`, `os`, `sys`
- **Phase 2 (Checked)**: `json`, `requests`
- **Phase 3 (Checked / Opaque-first)**: Basic `numpy` APIs
- **Phase 4 (Opaque-first)**: Core `pandas` and `torch` APIs

---

### asatsuyu-backend-python

Transforms THIR (type-checked HIR) into **readable Python 3.12+ source code**.
Generated code targets a quality level suitable for human review — the output
should look like code a Python developer could have written by hand.

```rust
pub fn generate(module: &TypedModule, config: &CodegenConfig) -> GeneratedPackage {
    let mut emitter = PythonEmitter::new(config);
    emitter.emit_module(module);
    emitter.finish()
}

pub struct GeneratedPackage {
    pub files: Vec<(PathBuf, String)>,  // each .py file
    pub pyproject_toml: String,         // package metadata
    pub prelude: String,               // asatsuyu_prelude.py
}

pub struct CodegenConfig {
    pub source_map: bool,           // emit # asty:L12 comments
    pub emit_type_annotations: bool, // include type annotations in generated code
}
```

#### Generation Rules

- 1 Asatsuyu module -> 1 `.py` file
- Imports map to Python-side imports
- Runtime helpers are isolated in a single `asatsuyu_prelude.py`
- Name mangling (`_asty_` prefix) only when collisions occur
- Source maps: each line ends with a comment `# asty:L12`
- Output includes a complete package tree with `pyproject.toml`

#### Example 1: ADT and Pattern Matching

```
// hello.asty
pub type Result(value, error) {
  Ok(value)
  Error(error)
}

pub fn divide(a: Int, b: Int) -> Result(Float, String) {
  match b {
    0 -> Error("division by zero")
    _ -> Ok(float(a) / float(b))
  }
}

pub fn main() {
  divide(10, 3)
  |> result.map(fn(x) { x * 2.0 })
  |> io.println
}
```

```python
# Generated by Asatsuyu 0.1.0 -- do not edit  # asty:L1
from dataclasses import dataclass            # asty:L2

@dataclass(frozen=True, slots=True)          # asty:L3
class Ok[T]:                                 # asty:L4
    value: T                                 # asty:L5

@dataclass(frozen=True, slots=True)          # asty:L7
class Error[E]:                              # asty:L8
    error: E                                 # asty:L9

type Result[T, E] = Ok[T] | Error[E]        # asty:L2

def divide(a: int, b: int) -> Result[float, str]:  # asty:L13
    match b:                                 # asty:L14
        case 0:                              # asty:L15
            return Error("division by zero") # asty:L15
        case _:                              # asty:L16
            return Ok(float(a) / float(b))   # asty:L16

def main() -> None:                          # asty:L19
    _pipe0 = divide(10, 3)                   # asty:L20
    _pipe1 = result_map(_pipe0, lambda x: x * 2.0)  # asty:L21
    print(_pipe1)                            # asty:L22
```

#### Example 2: Generic Functions

```
// list_utils.asty
pub fn first(items: List(a)) -> Option(a) {
  match items {
    [head, ..] -> Some(head)
    [] -> None
  }
}
```

```python
def first[T](items: list[T]) -> T | None:
    match items:
        case [head, *_]:
            return head
        case []:
            return None
```

PEP 695 allows writing `[T]` directly in function signatures, so the Asatsuyu
source and generated Python correspond nearly 1:1.

#### Example 3: Checked FFI and Exception Boundary

```
// downloader.asty
from python import pathlib
from python import requests

pub type DownloadResult {
  DownloadResult(text: String)
}

pub fn download(url: String, output: String) -> Result(DownloadResult, PyException) {
  let response = try requests.get(url)
  let path = pathlib.Path(output)
  let _ = try path.write_text(response.text)
  Ok(DownloadResult(response.text))
}
```

```python
import pathlib
import requests
from asatsuyu_prelude import Ok, Error, PyException

def download(url: str, output: str) -> Ok[DownloadResult] | Error[PyException]:
    try:
        response = requests.get(url)
    except Exception as e:
        return Error(PyException.from_exception(e))

    text = response.text

    try:
        path = pathlib.Path(output)
        path.write_text(text)
    except Exception as e:
        return Error(PyException.from_exception(e))

    return Ok(DownloadResult(text=text))
```

`try` is a **boundary operator** that automatically converts Python exceptions
into `Result`. Checked FFI sources like `requests` enter the Asatsuyu pure domain
accompanied by compiler-generated wrappers and validators as needed. Failures
propagate into Asatsuyu only as `Result` values, never as exceptions.

#### Example 4: Record Types

```
// user.asty
pub type User {
  User(name: String, age: Int, email: String)
}

pub fn greet(user: User) -> String {
  "Hello, " <> user.name <> "!"
}
```

```python
@dataclass(frozen=True, slots=True)
class User:
    name: str
    age: int
    email: str

def greet(user: User) -> str:
    return "Hello, " + user.name + "!"
```

#### Code Generation Summary

| Asatsuyu Syntax | Python 3.12+ Output |
|---|---|
| ADT `type` definition | `@dataclass(frozen=True, slots=True)` + PEP 695 `type` statement |
| Generics | PEP 695 bracket syntax `class Ok[T]:` / `def f[T]():` |
| `Option(T)` | `T \| None` |
| `Result(T, E)` | `Ok[T] \| Error[E]` (provided by prelude) |
| `match` | Python `match/case` (PEP 634) |
| `\|>` pipeline | Temporary variables `_pipe0`, `_pipe1`, ... |
| `<>` string concat | `+` operator |
| `try expr` | `try/except` -> `Result` wrap |
| `from python import` | Python `import` statement |

---

### asatsuyu-runtime-python

PyO3 runtime boundary layer for Checked FFI validation. This crate is independent of
the compiler pipeline and provides runtime type checking and exception wrapping for
Python interop.

> **Note:** This crate has `unsafe_code = "allow"` because PyO3 macros internally
> generate unsafe code. This is the sole exception to the workspace-wide `unsafe_code = "deny"` policy.

---

### asatsuyu-cli

```
asatsuyu new <name>    # Create a new project
asatsuyu check         # Type check only (fast, incremental)
asatsuyu build         # Generate Python package
asatsuyu run           # build + run with Python
asatsuyu fmt           # Format code
asatsuyu test          # Run tests
```

`asatsuyu check` is optimized to be the fastest command. As demonstrated by Astral's ty,
a Rust-based incremental type checker is central to the developer experience.

Project configuration:

```toml
# asatsuyu.toml
[project]
name = "my-app"
version = "0.1.0"

[python]
version = ">=3.12"

[python-dependencies]
requests = ">=2.31"
```

---

### asatsuyu-parser::format (formerly planned as asatsuyu-format)

**CST-based** code formatter. Fully preserves comments and whitespace.
Enforces **a single canonical format** with no configuration options (opinionated,
zero-config, similar to `gofmt`).

> **Implementation note:** Implemented as the `format` module within `asatsuyu-parser`
> rather than as a standalone crate. Because the parser owns the CST types (`SyntaxNode`, rowan),
> the `parse() -> format()` round-trip completes within a single crate. Extraction
> to a separate crate is a mechanical refactor if needed in the future.

---

### asatsuyu-cli::lsp (formerly planned as asatsuyu-lsp)

Language Server Protocol implementation built on `tower-lsp`.

> **Implementation note:** Implemented as the `lsp` module within `asatsuyu-cli`
> rather than as a standalone crate. The server is started via the `asatsuyu lsp`
> subcommand over a stdio transport. The LSP is integrated into the compiler
> binary rather than shipped as a separate process.

Implemented features:
- diagnostics (on-save + debounced on-change with 200ms delay)
- hover (type information display)
- go to definition (DefId -> jump to definition site)
- document formatting (CST-based formatter integration)
- completion (keyword-aware + symbol completion with lightweight context classification)
- rename (bulk rename of all references via DefId)
- find references (find all references via DefId)
- document symbols (list of functions and types)

The design supports HIR/type information caching from Phases 2-3 for incremental
analysis. Adoption of Salsa (rust-analyzer's incremental computation framework) is
under consideration for the future.

---

## Cargo.toml (Workspace Root)

```toml
[workspace]
resolver = "3"
members = ["crates/*"]

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "MIT OR Apache-2.0"
repository = "https://github.com/kaikei/asatsuyu"

[workspace.dependencies]
# Internal crates
asatsuyu-syntax         = { path = "crates/asatsuyu-syntax" }
asatsuyu-lexer          = { path = "crates/asatsuyu-lexer" }
asatsuyu-parser         = { path = "crates/asatsuyu-parser" }
asatsuyu-ast            = { path = "crates/asatsuyu-ast" }
asatsuyu-hir            = { path = "crates/asatsuyu-hir" }
asatsuyu-ty             = { path = "crates/asatsuyu-ty" }
asatsuyu-backend-python = { path = "crates/asatsuyu-backend-python" }
asatsuyu-runtime-python = { path = "crates/asatsuyu-runtime-python" }

# Lexer
logos = "0.16"

# CST (lossless syntax tree)
rowan = "0.16"

# CLI
clap = { version = "4.6", features = ["derive"] }

# Diagnostics
miette = { version = "7.6", features = ["fancy"] }

# String interning
smol_str = "0.3"

# Arena allocation
la-arena = "0.3"

# PyO3 (runtime crate only)
pyo3 = { version = "0.28" }

# Testing
insta = { version = "1.47", features = ["glob"] }

# Serialization
serde = { version = "1", features = ["derive"] }

# Project config
toml = "0.8"

# LSP
tower-lsp = "0.20"
```

---

## Crate Dependency Graph

```
asatsuyu-syntax  <-- depended upon by all crates (lowest layer, zero external deps)
     ^
asatsuyu-lexer  (+ logos)
     ^
asatsuyu-parser  (+ rowan)
     ^
asatsuyu-ast
     ^
asatsuyu-hir  (includes FFI resolution via builtin type surfaces)
     ^
asatsuyu-ty
     ^
asatsuyu-backend-python
     ^
asatsuyu-cli --> asatsuyu-parser::format (module within parser)
 (+ clap,   --> asatsuyu-cli::lsp (+ tower-lsp, module within CLI)
    miette)

asatsuyu-runtime-python  (standalone, PyO3; independent of the compiler pipeline)
```

---

## Language Syntax Sketch (.asty)

```
// --- Type definitions ---
pub type Option(a) {
  Some(a)
  None
}

pub type Result(a, e) {
  Ok(a)
  Error(e)
}

pub type User {
  User(name: String, age: Int, email: String)
}

// --- Function definitions ---
pub fn greet(user: User) -> String {
  "Hello, " <> user.name <> "!"
}

// --- Pipeline ---
pub fn process(data: List(Int)) -> List(Int) {
  data
  |> list.filter(fn(x) { x > 0 })
  |> list.map(fn(x) { x * 2 })
  |> list.sort
}

// --- Pattern matching ---
pub fn describe(value: Option(Int)) -> String {
  match value {
    Some(n) if n > 100 -> "large: " <> int.to_string(n)
    Some(n) -> "small: " <> int.to_string(n)
    None -> "nothing"
  }
}

// --- Using Python libraries ---
from python import pathlib
from python import requests

pub fn download(url: String, output: String) -> Result(String, PyException) {
  let response = try requests.get(url)
  let path = pathlib.Path(output)
  let _ = try path.write_text(response.text)
  Ok(output)
}
```

---

## Technical Choices (Fixed)

| Area | Choice | Rationale |
|---|---|---|
| Lexer | `logos` | Compile-time DFA generation, fast |
| Parser | Hand-written recursive descent | Ruff's proven track record, error quality |
| CST | `rowan` | Lossless, rust-analyzer track record |
| Error display | `miette` | Beautiful terminal diagnostics |
| CLI | `clap` | De facto standard for Rust CLIs |
| LSP | `tower-lsp` | De facto standard for Rust LSP servers |
| FFI type info | `typeshed` + PEP 561 | Official Python type distribution infrastructure |
| Strings | `smol_str` | Small string interning |

---

## Research Tracks (Not Part of Mainline)

**Research A: Direct Python Bytecode Generation**
- `.pyc` output using RustPython's `rustpython-compiler`
- High maintenance cost due to CPython version differences; unsuitable for the MVP mainline

**Research B: Automatic Stub Generation**
- Auto-generate Asatsuyu external declarations from Python modules and `.pyi` files
- Realistic given typeshed and typing spec, but deferred

**Research C: Rust Native Extensions**
- PyO3 is positioned not as part of Asatsuyu itself, but as a path for providing
  fast native extensions from Asatsuyu-authored code

---

## Initial Planning Issues

> **Note:** The list below was the initial planning backlog. Many of these items have
> been completed. Detailed implementation tracking is maintained in a separate internal
> document (`IMPL_PHASES.md`).

1. Initialize Rust workspace with 9 crates
2. `asatsuyu-syntax`: define `SyntaxKind` enum (tokens)
3. `asatsuyu-syntax`: define `SyntaxKind` enum (nodes)
4. `asatsuyu-syntax`: define `Span`, `Diagnostic`, `Label`
5. `asatsuyu-lexer`: implement lexer with `logos`
6. `asatsuyu-lexer`: add 30+ snapshot tests
7. `asatsuyu-parser`: scaffold recursive descent parser
8. `asatsuyu-parser`: parse literals and identifiers
9. `asatsuyu-parser`: parse `fn` definitions
10. `asatsuyu-parser`: parse call expressions
11. `asatsuyu-parser`: parse `|>` pipeline
12. `asatsuyu-parser`: parse `if` expressions
13. `asatsuyu-parser`: parse `type` (ADT) definitions
14. `asatsuyu-parser`: parse `match` expressions
15. `asatsuyu-parser`: implement error recovery
16. `asatsuyu-parser`: add 20+ malformed input tests
17. `asatsuyu-ast`: build AST from CST
18. `asatsuyu-hir`: implement symbol table and scopes
19. `asatsuyu-hir`: implement pipeline desugaring
20. `asatsuyu-hir`: implement name resolution
21. `asatsuyu-ty`: implement HM unification core
22. `asatsuyu-ty`: implement occurs check
23. `asatsuyu-ty`: implement let-polymorphism
24. `asatsuyu-ty`: ADT constructor typing
25. `asatsuyu-ty`: match exhaustiveness check
26. `asatsuyu-backend-python`: Python dataclass emitter
27. `asatsuyu-backend-python`: Python match/case emitter
28. `asatsuyu-backend-python`: `asatsuyu_prelude.py` generation
29. `asatsuyu-cli`: implement `asatsuyu check`
30. `asatsuyu-cli`: implement `asatsuyu build` + `asatsuyu run`

---

## 6-Month Success Criteria

**Write a 300-500 line CLI application in Asatsuyu using `Result` / `Option` /
`match` / ADT, call `pathlib` as Verified FFI and `requests` as Checked FFI, and
generate a readable Python 3.12+ package.**

The MVP evaluation criteria are not about speed. They are:
- Quality of type error messages
- Readability of generated code
- Low friction when leveraging Python assets
