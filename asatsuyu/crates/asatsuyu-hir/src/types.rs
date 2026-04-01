//! HIR type definitions for the Asatsuyu language.
//!
//! These types represent the **high-level intermediate representation**,
//! where variable references are resolved to [`DefId`]s via a [`SymbolTable`].
//! Every node carries a [`Span`] for error reporting.

use asatsuyu_ast::{BinOp, LiteralKind, UnOp, Visibility};
use asatsuyu_syntax::Span;
use la_arena::{Arena, Idx};
use smol_str::SmolStr;

// ── DefId ───────────────────────────────────────────────────────────

/// Arena index identifying a definition (function, parameter, etc.).
pub type DefId = Idx<DefData>;

/// Metadata for a definition registered in the symbol table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefData {
    pub name: SmolStr,
    pub kind: DefKind,
    /// Whether this binding was declared with `let mut`.
    pub is_mutable: bool,
    pub span: Span,
}

/// What kind of thing a [`DefId`] refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefKind {
    Function,
    Parameter,
    /// A binding introduced by a pattern (match arm, let binding).
    LocalBinding,
    /// An ADT constructor (e.g., `Some`, `None`, `Ok`, `Err`).
    Constructor,
    /// A type name (e.g., `Option`, `Result`).
    Type,
    /// A compiler built-in function (e.g., `string_concat`).
    Builtin,
    /// A name introduced by an import statement (e.g., `import io` binds `io`).
    Import,
}

// ── Symbol Table ────────────────────────────────────────────────────

/// Symbol table: an arena of definitions.
///
/// Stores all definitions (functions, parameters, constructors, local
/// bindings) for a module. Name lookup is handled by the lowering
/// context's [`ScopeStack`](super::lower::ScopeStack).
#[derive(Debug)]
pub struct SymbolTable {
    defs: Arena<DefData>,
}

impl SymbolTable {
    /// Creates an empty symbol table.
    #[must_use]
    pub fn new() -> Self {
        Self { defs: Arena::new() }
    }

    /// Register a new definition, returns its [`DefId`].
    pub fn alloc(&mut self, data: DefData) -> DefId {
        self.defs.alloc(data)
    }

    /// Look up definition metadata by [`DefId`].
    #[must_use]
    pub fn get(&self, id: DefId) -> &DefData {
        &self.defs[id]
    }

    /// Iterate all definitions.
    pub fn iter(&self) -> impl Iterator<Item = (DefId, &DefData)> {
        self.defs.iter()
    }

    /// Returns the number of registered definitions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.defs.len()
    }

    /// Returns `true` if no definitions are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.defs.len() == 0
    }
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}

// ── HIR Module ──────────────────────────────────────────────────────

/// The root HIR node representing a single source file after name resolution.
#[derive(Debug)]
pub struct HirModule {
    pub imports: Vec<HirImport>,
    pub functions: Vec<HirFnDef>,
    pub custom_types: Vec<HirCustomType>,
    pub symbol_table: SymbolTable,
    pub span: Span,
}

// ── HIR Import ─────────────────────────────────────────────────────

/// An import declaration in HIR, with a resolved [`DefId`] for the bound name.
#[derive(Debug, Clone)]
pub struct HirImport {
    pub def_id: DefId,
    /// Whether this is an internal module import or a Python FFI import.
    pub kind: HirImportKind,
    pub span: Span,
}

/// Distinguishes internal Asatsuyu imports from Python FFI imports.
#[derive(Debug, Clone)]
pub enum HirImportKind {
    /// Internal module import: `import io`, `import gleam.io as stdio`.
    Module {
        /// Module path segments: `["gleam", "io"]` for `import gleam.io`.
        module_path: Vec<SmolStr>,
    },
    /// Python FFI import: `from python import pathlib`.
    Python {
        /// The Python module name (e.g., `"pathlib"`, `"json"`).
        module_name: SmolStr,
    },
}

// ── HIR Type Expression ─────────────────────────────────────────────

/// A type expression in HIR, preserving structure from AST `TypeExpr`.
///
/// Example: `Option(Int)` → `HirTypeExpr { name: "Option", args: [HirTypeExpr { name: "Int", .. }], .. }`
#[derive(Debug, Clone)]
pub struct HirTypeExpr {
    pub name: SmolStr,
    pub args: Vec<HirTypeExpr>,
    pub span: Span,
}

// ── HIR Custom Type ─────────────────────────────────────────────────

/// A custom type definition in HIR, with resolved [`DefId`]s and variant info.
#[derive(Debug, Clone)]
pub struct HirCustomType {
    pub def_id: DefId,
    pub visibility: Visibility,
    /// Type parameter names (e.g., `["a"]` for `Option(a)`).
    pub type_params: Vec<SmolStr>,
    /// Variants of this ADT.
    pub variants: Vec<HirVariant>,
    pub span: Span,
}

/// A variant of a custom type in HIR.
#[derive(Debug, Clone)]
pub struct HirVariant {
    /// The [`DefId`] of this constructor (`DefKind::Constructor`).
    pub def_id: DefId,
    /// Field types for this constructor.
    pub fields: Vec<HirFieldType>,
    pub span: Span,
}

/// A field type in an ADT variant.
#[derive(Debug, Clone)]
pub struct HirFieldType {
    pub label: Option<SmolStr>,
    pub type_expr: HirTypeExpr,
    pub span: Span,
}

// ── HIR Function ────────────────────────────────────────────────────

/// A function definition in HIR, with a resolved [`DefId`].
#[derive(Debug)]
pub struct HirFnDef {
    pub def_id: DefId,
    pub visibility: Visibility,
    pub params: Vec<HirParam>,
    /// Return type annotation. `None` when omitted.
    pub return_type: Option<HirTypeExpr>,
    pub body: HirExpr,
    pub span: Span,
}

// ── HIR Parameter ───────────────────────────────────────────────────

/// A function parameter in HIR, with a resolved [`DefId`].
#[derive(Debug)]
pub struct HirParam {
    pub def_id: DefId,
    /// Type annotation. `None` when omitted (lambda params).
    pub type_ann: Option<HirTypeExpr>,
    pub span: Span,
}

// ── HIR Literal ─────────────────────────────────────────────────────

/// A literal value in HIR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HirLiteral {
    pub kind: LiteralKind,
    pub value: SmolStr,
    pub span: Span,
}

// ── HIR Pattern ─────────────────────────────────────────────────────

/// A pattern in HIR, with bindings resolved to [`DefId`]s.
#[derive(Debug)]
pub enum HirPattern {
    /// `_`
    Wildcard(Span),
    /// A variable binding resolved to a [`DefId`].
    Variable(DefId, Span),
    /// `42`, `"hello"`, `True`
    Literal(HirLiteral),
    /// `Some(x)`, `Ok(value)`, `None`
    Constructor { def_id: DefId, fields: Vec<HirPattern>, span: Span },
    /// `[head, ..rest]`, `[]`
    List { elements: Vec<HirPattern>, rest: Option<DefId>, span: Span },
}

impl HirPattern {
    /// Returns the span of this pattern.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Literal(lit) => lit.span,
            Self::Variable(_, span)
            | Self::Wildcard(span)
            | Self::Constructor { span, .. }
            | Self::List { span, .. } => *span,
        }
    }
}

// ── HIR Match Arm ───────────────────────────────────────────────────

/// A single arm in a match expression in HIR.
#[derive(Debug)]
pub struct HirMatchArm {
    pub pattern: HirPattern,
    pub guard: Option<Box<HirExpr>>,
    pub body: HirExpr,
    pub span: Span,
}

// ── HIR Expression ──────────────────────────────────────────────────

/// An expression node in HIR.
///
/// The key difference from AST: variable references are resolved to [`DefId`]s.
#[derive(Debug)]
pub enum HirExpr {
    /// A literal value: `42`, `"hello"`.
    Literal(HirLiteral),
    /// A name-resolved variable reference.
    Var(DefId, Span),
    /// A block expression: `{ expr1; expr2 }`.
    Block { exprs: Vec<HirExpr>, span: Span },
    /// A function call: `f(a, b)`.
    Call { func: Box<HirExpr>, args: Vec<HirExpr>, span: Span },
    /// A binary operation: `a + b`.
    BinaryOp { op: BinOp, lhs: Box<HirExpr>, rhs: Box<HirExpr>, span: Span },
    /// A unary operation: `-x`, `!flag`.
    UnaryOp { op: UnOp, expr: Box<HirExpr>, span: Span },
    /// An if expression: `if cond { a } else { b }`.
    If {
        condition: Box<HirExpr>,
        then_body: Box<HirExpr>,
        else_body: Option<Box<HirExpr>>,
        span: Span,
    },
    /// A match expression: `match subject { pattern -> expr ... }`.
    Match { subject: Box<HirExpr>, arms: Vec<HirMatchArm>, span: Span },
    /// A let binding: `let x = expr` or `let mut x = expr`.
    Let { binding: DefId, value: Box<HirExpr>, is_mutable: bool, span: Span },
    /// A reassignment: `x = expr`. Type-check enforcement is in Issue 94.
    Assign { target: DefId, value: Box<HirExpr>, span: Span },
    /// An anonymous function: `fn(params) { body }`.
    Lambda {
        params: Vec<HirParam>,
        return_type: Option<HirTypeExpr>,
        body: Box<HirExpr>,
        span: Span,
    },
    /// A field access: `expr.field`.
    FieldAccess { receiver: Box<HirExpr>, field: SmolStr, span: Span },
    /// A try expression: `try expr`. Wraps FFI calls in exception handling.
    Try { expr: Box<HirExpr>, span: Span },
    /// A list literal: `[1, 2, 3]`.
    List { elements: Vec<HirExpr>, span: Span },
}

impl HirExpr {
    /// Returns the span of this expression.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Literal(lit) => lit.span,
            Self::Var(_, span)
            | Self::Block { span, .. }
            | Self::Call { span, .. }
            | Self::BinaryOp { span, .. }
            | Self::UnaryOp { span, .. }
            | Self::If { span, .. }
            | Self::Match { span, .. }
            | Self::Let { span, .. }
            | Self::Assign { span, .. }
            | Self::Lambda { span, .. }
            | Self::FieldAccess { span, .. }
            | Self::Try { span, .. }
            | Self::List { span, .. } => *span,
        }
    }
}
