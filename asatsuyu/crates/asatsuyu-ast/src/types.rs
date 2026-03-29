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
    String,
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

// ── Module (root) ───────────────────────────────────────────────────

/// The root AST node representing a single source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub definitions: Vec<Definition>,
    pub span: Span,
}

// ── Definition ──────────────────────────────────────────────────────

/// A top-level definition within a module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Definition {
    Function(FnDef),
}

// ── Function definition ─────────────────────────────────────────────

/// A function definition: `pub fn name(params) -> ReturnType { body }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnDef {
    pub name: Ident,
    pub visibility: Visibility,
    pub params: Vec<Param>,
    /// Return type annotation. `None` when omitted.
    pub return_type: Option<Ident>,
    pub body: Expr,
    pub span: Span,
}

// ── Parameter ───────────────────────────────────────────────────────

/// A function parameter: `name: Type`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub name: Ident,
    /// Type annotation (currently always a simple identifier).
    pub type_ann: Ident,
    pub span: Span,
}

// ── Expression ──────────────────────────────────────────────────────

/// An expression node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// A literal value: `42`, `"hello"`.
    Literal(Literal),
    /// A variable reference: `foo`.
    Variable(Ident),
    /// A block expression: `{ expr1; expr2 }`.
    Block { exprs: Vec<Expr>, span: Span },
}

impl Expr {
    /// Returns the span of this expression.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Literal(lit) => lit.span,
            Self::Variable(ident) => ident.span,
            Self::Block { span, .. } => *span,
        }
    }
}
