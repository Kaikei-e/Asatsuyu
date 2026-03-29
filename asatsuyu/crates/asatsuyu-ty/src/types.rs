//! Type definitions for the Asatsuyu type system and THIR (Typed HIR).
//!
//! Every [`ThirExpr`] carries a resolved [`Ty`], enabling type-directed code
//! generation in the Python backend.

use std::fmt;

use asatsuyu_ast::{BinOp, LiteralKind, UnOp, Visibility};
use asatsuyu_hir::{DefId, SymbolTable};
use asatsuyu_syntax::Span;
use smol_str::SmolStr;

// ── Type variable ──────────────────────────────────────────────────

/// A unique identifier for a type variable during inference.
///
/// Placeholder for Issue 23 (HM unification). Currently unused by the
/// placeholder pass but defined so downstream code can pattern-match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TyVarId(pub u32);

// ── Primitive type ─────────────────────────────────────────────────

/// A built-in primitive type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimTy {
    Int,
    Float,
    String,
    Bool,
    None,
}

impl fmt::Display for PrimTy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int => f.write_str("Int"),
            Self::Float => f.write_str("Float"),
            Self::String => f.write_str("String"),
            Self::Bool => f.write_str("Bool"),
            Self::None => f.write_str("None"),
        }
    }
}

// ── Type ───────────────────────────────────────────────────────────

/// A resolved type in the Asatsuyu type system.
///
/// Currently covers primitives and function types. Will be extended in:
/// - Issue 23: `Var` used for HM inference
/// - Issue 25: let-polymorphism (type schemes)
/// - Issue 26: `Named` for ADT constructors
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    /// A primitive type: `Int`, `Float`, `String`, `Bool`, `None`.
    Primitive(PrimTy),
    /// A function type: `(params) -> ret`.
    Function { params: Vec<Ty>, ret: Box<Ty> },
    /// An inference variable (placeholder for Issue 23).
    Var(TyVarId),
    /// A type that failed to resolve. Allows checking to continue.
    Error,
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Primitive(p) => write!(f, "{p}"),
            Self::Function { params, ret } => {
                f.write_str("(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{p}")?;
                }
                write!(f, ") -> {ret}")
            }
            Self::Var(id) => write!(f, "?{}", id.0),
            Self::Error => f.write_str("<error>"),
        }
    }
}

// ── THIR Module ────────────────────────────────────────────────────

/// The root THIR node: a type-checked module.
#[derive(Debug)]
pub struct ThirModule {
    pub functions: Vec<ThirFnDef>,
    /// Re-exported from HIR for downstream convenience.
    pub symbol_table: SymbolTable,
    pub span: Span,
}

// ── THIR Function ──────────────────────────────────────────────────

/// A type-checked function definition.
#[derive(Debug)]
pub struct ThirFnDef {
    pub def_id: DefId,
    pub visibility: Visibility,
    pub params: Vec<ThirParam>,
    /// The resolved return type of this function.
    pub return_ty: Ty,
    pub body: ThirExpr,
    /// The full function type: `Ty::Function { params, ret }`.
    pub ty: Ty,
    pub span: Span,
}

// ── THIR Parameter ─────────────────────────────────────────────────

/// A type-checked function parameter.
#[derive(Debug)]
pub struct ThirParam {
    pub def_id: DefId,
    pub ty: Ty,
    pub span: Span,
}

// ── THIR Literal ───────────────────────────────────────────────────

/// A type-checked literal value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThirLiteral {
    pub kind: LiteralKind,
    pub value: SmolStr,
    pub ty: Ty,
    pub span: Span,
}

// ── THIR Match Arm ─────────────────────────────────────────────────

/// A type-checked match arm. Pattern typing is deferred to Issue 26+.
#[derive(Debug)]
pub struct ThirMatchArm {
    pub body: ThirExpr,
    pub span: Span,
}

// ── THIR Expression ────────────────────────────────────────────────

/// A type-checked expression. Every variant carries a [`Ty`].
#[derive(Debug)]
pub enum ThirExpr {
    /// A literal value: `42`, `"hello"`.
    Literal(ThirLiteral),
    /// A name-resolved, typed variable reference.
    Var { def_id: DefId, ty: Ty, span: Span },
    /// A block expression: `{ expr1; expr2 }`.
    Block { exprs: Vec<ThirExpr>, ty: Ty, span: Span },
    /// A function call: `f(a, b)`.
    Call { func: Box<ThirExpr>, args: Vec<ThirExpr>, ty: Ty, span: Span },
    /// A binary operation: `a + b`.
    BinaryOp { op: BinOp, lhs: Box<ThirExpr>, rhs: Box<ThirExpr>, ty: Ty, span: Span },
    /// A unary operation: `-x`, `!flag`.
    UnaryOp { op: UnOp, expr: Box<ThirExpr>, ty: Ty, span: Span },
    /// An if expression: `if cond { a } else { b }`.
    If {
        condition: Box<ThirExpr>,
        then_body: Box<ThirExpr>,
        else_body: Option<Box<ThirExpr>>,
        ty: Ty,
        span: Span,
    },
    /// A match expression: `match subject { pattern -> expr ... }`.
    Match { subject: Box<ThirExpr>, arms: Vec<ThirMatchArm>, ty: Ty, span: Span },
}

impl ThirExpr {
    /// Returns the type of this expression.
    #[must_use]
    pub fn ty(&self) -> &Ty {
        match self {
            Self::Literal(lit) => &lit.ty,
            Self::Var { ty, .. }
            | Self::Block { ty, .. }
            | Self::Call { ty, .. }
            | Self::BinaryOp { ty, .. }
            | Self::UnaryOp { ty, .. }
            | Self::If { ty, .. }
            | Self::Match { ty, .. } => ty,
        }
    }

    /// Returns the span of this expression.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Literal(lit) => lit.span,
            Self::Var { span, .. }
            | Self::Block { span, .. }
            | Self::Call { span, .. }
            | Self::BinaryOp { span, .. }
            | Self::UnaryOp { span, .. }
            | Self::If { span, .. }
            | Self::Match { span, .. } => *span,
        }
    }
}
