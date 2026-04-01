//! AST type definitions for the Asatsuyu language.
//!
//! These types represent the **untyped** abstract syntax tree, produced by
//! lowering the lossless CST. Trivia (whitespace, comments) is stripped;
//! every node carries a [`Span`] for error reporting.

use asatsuyu_syntax::Span;
use smol_str::SmolStr;

// ── Identifier ──────────────────────────────────────────────────────

/// A name with its source location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ident {
    pub name: SmolStr,
    pub span: Span,
}

// ── Literal ─────────────────────────────────────────────────────────

/// The kind of a literal value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiteralKind {
    Int,
    Float,
    String,
    Bool,
}

/// A literal value with its source representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Literal {
    pub kind: LiteralKind,
    pub value: SmolStr,
    pub span: Span,
}

// ── Visibility ──────────────────────────────────────────────────────

/// Whether a definition is publicly exported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Private,
}

// ── Type expression ─────────────────────────────────────────────────

/// A type annotation: `Int`, `List(Int)`, `Result(String, Error)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeExpr {
    /// A named type with optional type arguments.
    Named { name: Ident, args: Vec<TypeExpr>, span: Span },
}

impl TypeExpr {
    /// Returns the span of this type expression.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Named { span, .. } => *span,
        }
    }
}

// ── Module (root) ───────────────────────────────────────────────────

/// The root AST node representing a single source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub imports: Vec<Import>,
    pub definitions: Vec<Definition>,
    pub span: Span,
}

// ── Import ──────────────────────────────────────────────────────────

/// An import statement, either an internal module import or a Python FFI import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Import {
    /// Internal module import: `import io`, `import gleam.io as io`.
    Module {
        /// Module path segments: `["gleam", "io"]` for `import gleam.io`.
        module: Vec<Ident>,
        /// Optional alias: `as alias`.
        alias: Option<Ident>,
        span: Span,
    },
    /// Python FFI import: `from python import pathlib`, `from python import pathlib as pl`.
    Python {
        /// The Python module name (e.g., `pathlib`, `json`).
        module_name: Ident,
        /// Optional alias: `as alias`.
        alias: Option<Ident>,
        span: Span,
    },
}

impl Import {
    /// Returns the span of this import.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Module { span, .. } | Self::Python { span, .. } => *span,
        }
    }
}

// ── Definition ──────────────────────────────────────────────────────

/// A top-level definition within a module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Definition {
    Function(FnDef),
    CustomType(CustomType),
}

// ── Function definition ─────────────────────────────────────────────

/// A function definition: `pub fn name(params) -> ReturnType { body }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnDef {
    pub name: Ident,
    pub visibility: Visibility,
    pub params: Vec<Param>,
    /// Return type annotation. `None` when omitted.
    pub return_type: Option<TypeExpr>,
    pub body: Expr,
    pub span: Span,
}

// ── Parameter ───────────────────────────────────────────────────────

/// A function parameter: `name: Type`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub name: Ident,
    /// Type annotation. `None` for lambda parameters without explicit types.
    pub type_ann: Option<TypeExpr>,
    pub span: Span,
}

// ── Custom type ─────────────────────────────────────────────────────

/// A custom type definition: `type Option(a) { Some(a) None }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomType {
    pub name: Ident,
    pub visibility: Visibility,
    pub type_params: Vec<Ident>,
    pub body: TypeBody,
    pub span: Span,
}

/// The body of a custom type: either Go-style record fields or Gleam ADT variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeBody {
    /// Go-style flat struct: `type User { name: String  age: Int }`.
    Record(Vec<RecordField>),
    /// Gleam-style ADT: `type Option(a) { Some(a) None }`.
    Variants(Vec<Variant>),
}

/// A record field: `name: Type`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordField {
    pub name: Ident,
    pub type_ann: TypeExpr,
    pub span: Span,
}

/// An ADT variant: `Some(Int)` or `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    pub name: Ident,
    pub fields: Vec<VariantField>,
    pub span: Span,
}

/// A variant field, optionally labelled: `Int` or `value: Int`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantField {
    pub label: Option<Ident>,
    pub type_ann: TypeExpr,
    pub span: Span,
}

// ── Pattern ─────────────────────────────────────────────────────────

/// A pattern used in match arms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    /// `_`
    Wildcard(Span),
    /// `x` — a variable binding (or nullary constructor, resolved in HIR).
    Variable(Ident),
    /// `42`, `"hello"`, `True`
    Literal(Literal),
    /// `Some(x)`, `Ok(value)`
    Constructor { name: Ident, fields: Vec<Pattern>, span: Span },
    /// `[head, ..rest]`, `[]`
    List {
        elements: Vec<Pattern>,
        /// Optional rest binding name in `[x, ..rest]`.
        rest: Option<Ident>,
        span: Span,
    },
}

impl Pattern {
    /// Returns the span of this pattern.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Variable(ident) => ident.span,
            Self::Literal(lit) => lit.span,
            Self::Wildcard(span) | Self::Constructor { span, .. } | Self::List { span, .. } => {
                *span
            }
        }
    }
}

// ── Match arm ───────────────────────────────────────────────────────

/// A single arm in a match expression: `pattern if guard -> body`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Box<Expr>>,
    pub body: Expr,
    pub span: Span,
}

// ── Binary / Unary operators ────────────────────────────────────────

/// Binary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
    StringConcat,
}

/// Unary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

// ── Expression ──────────────────────────────────────────────────────

/// An expression node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// A literal value: `42`, `"hello"`, `True`.
    Literal(Literal),
    /// A variable reference: `foo`.
    Variable(Ident),
    /// A block expression: `{ expr1; expr2 }`.
    Block { exprs: Vec<Expr>, span: Span },
    /// A function call: `f(a, b)`.
    Call { func: Box<Expr>, args: Vec<Expr>, span: Span },
    /// A binary operation: `a + b`, `x == y`.
    BinaryOp { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr>, span: Span },
    /// A unary operation: `-x`, `!flag`.
    UnaryOp { op: UnOp, expr: Box<Expr>, span: Span },
    /// An if expression: `if cond { a } else { b }`.
    If { condition: Box<Expr>, then_body: Box<Expr>, else_body: Option<Box<Expr>>, span: Span },
    /// A match expression: `match subject { pattern -> expr ... }`.
    Match { subject: Box<Expr>, arms: Vec<MatchArm>, span: Span },
    /// A pipeline expression: `x |> f`.
    Pipeline { left: Box<Expr>, right: Box<Expr>, span: Span },
    /// A let binding: `let x = expr` or `let mut x = expr`.
    Let { name: Ident, value: Box<Expr>, is_mutable: bool, span: Span },
    /// An assignment: `x = expr` (reassignment of mutable binding).
    Assign { target: Ident, value: Box<Expr>, span: Span },
    /// An anonymous function: `fn(params) { body }`.
    Lambda { params: Vec<Param>, return_type: Option<TypeExpr>, body: Box<Expr>, span: Span },
    /// A field access: `expr.field`.
    FieldAccess { receiver: Box<Expr>, field: Ident, span: Span },
    /// A try expression: `try expr`. Captures Python exceptions as Result errors.
    Try { expr: Box<Expr>, span: Span },
    /// A list literal: `[1, 2, 3]`.
    List { elements: Vec<Expr>, span: Span },
}

impl Expr {
    /// Returns the span of this expression.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Literal(lit) => lit.span,
            Self::Variable(ident) => ident.span,
            Self::Block { span, .. }
            | Self::Call { span, .. }
            | Self::BinaryOp { span, .. }
            | Self::UnaryOp { span, .. }
            | Self::If { span, .. }
            | Self::Match { span, .. }
            | Self::Pipeline { span, .. }
            | Self::Let { span, .. }
            | Self::Assign { span, .. }
            | Self::Lambda { span, .. }
            | Self::FieldAccess { span, .. }
            | Self::Try { span, .. }
            | Self::List { span, .. } => *span,
        }
    }
}
