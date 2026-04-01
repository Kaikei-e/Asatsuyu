//! Type definitions for the Asatsuyu type system and THIR (Typed HIR).
//!
//! Every [`ThirExpr`] carries a resolved [`Ty`], enabling type-directed code
//! generation in the Python backend.

use std::collections::HashMap;
use std::fmt;

use asatsuyu_ast::{BinOp, LiteralKind, UnOp, Visibility};
use asatsuyu_hir::ffi::FfiModule;
use asatsuyu_hir::{DefId, HirCustomType, HirImport, SymbolTable};
use asatsuyu_syntax::Span;
use smol_str::SmolStr;

// ── Type variable ──────────────────────────────────────────────────

/// A unique identifier for a type variable during inference.
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    /// A primitive type: `Int`, `Float`, `String`, `Bool`, `None`.
    Primitive(PrimTy),
    /// A function type: `(params) -> ret`.
    Function { params: Vec<Ty>, ret: Box<Ty> },
    /// A named (ADT) type: `Option(Int)`, `Result(String, Error)`.
    Named { def_id: DefId, name: SmolStr, args: Vec<Ty> },
    /// An FFI module namespace (e.g., `pathlib`, `os`).
    ///
    /// Only field access is permitted on this type — it represents the
    /// module itself, not a value within it.
    FfiModule { module_name: SmolStr },
    /// An instance of an FFI class (e.g., a `pathlib.Path` value).
    ///
    /// Field access resolves to class properties and methods via the FFI model.
    FfiInstance { module: SmolStr, class: SmolStr },
    /// An opaque FFI type from an `Unsafe` symbol.
    ///
    /// Cannot be destructured, pattern-matched, or field-accessed.
    /// Can only be passed to other FFI calls that accept the same opaque type.
    Opaque { module: SmolStr, symbol: SmolStr },
    /// An inference variable.
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
            Self::Named { name, args, .. } => {
                write!(f, "{name}")?;
                if !args.is_empty() {
                    f.write_str("(")?;
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            f.write_str(", ")?;
                        }
                        write!(f, "{a}")?;
                    }
                    f.write_str(")")?;
                }
                Ok(())
            }
            Self::FfiModule { module_name } => write!(f, "module({module_name})"),
            Self::FfiInstance { module, class } => write!(f, "{module}.{class}"),
            Self::Opaque { module, symbol } => write!(f, "PyOpaque[{module}.{symbol}]"),
            Self::Var(id) => write!(f, "?{}", id.0),
            Self::Error => f.write_str("<error>"),
        }
    }
}

// ── Type Scheme (let-polymorphism, Issue 25) ──────────────────────

/// A polymorphic type: universally quantified type variables + monotype.
///
/// Monomorphic types have `vars: vec![]`. Let-bound polymorphic types
/// have the generalized variables listed in `vars`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TypeScheme {
    /// Quantified type variables (empty for monomorphic types).
    pub vars: Vec<TyVarId>,
    /// The underlying monotype.
    pub ty: Ty,
}

impl TypeScheme {
    /// Create a monomorphic scheme (no quantified variables).
    pub fn mono(ty: Ty) -> Self {
        Self { vars: vec![], ty }
    }
}

// ── THIR Module ────────────────────────────────────────────────────

/// The root THIR node: a type-checked module.
#[derive(Debug)]
pub struct ThirModule {
    pub functions: Vec<ThirFnDef>,
    /// Custom type definitions passed through from HIR for backend use.
    pub custom_types: Vec<HirCustomType>,
    /// Import declarations passed through from HIR for backend use.
    pub imports: Vec<HirImport>,
    /// Re-exported from HIR for downstream convenience.
    pub symbol_table: SymbolTable,
    /// FFI module metadata (trust levels, signatures) for backend code generation.
    /// Populated by the type checker from resolved Python imports.
    pub ffi_modules: HashMap<SmolStr, FfiModule>,
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

// ── THIR Pattern ──────────────────────────────────────────────────

/// A type-checked pattern used in match arms.
#[derive(Debug)]
pub enum ThirPattern {
    /// `_` — matches anything, binds nothing.
    Wildcard(Span),
    /// A variable binding with resolved type.
    Variable { def_id: DefId, ty: Ty, span: Span },
    /// A literal pattern: `42`, `"hello"`, `True`.
    Literal(ThirLiteral),
    /// A constructor pattern: `Some(x)`, `None`, `Ok(value)`.
    Constructor { def_id: DefId, fields: Vec<ThirPattern>, ty: Ty, span: Span },
    /// A tuple pattern: `(a, b, c)`.
    Tuple { elements: Vec<ThirPattern>, ty: Ty, span: Span },
    /// A list pattern: `[head, ..rest]`, `[]`.
    List { elements: Vec<ThirPattern>, rest: Option<Box<ThirPattern>>, ty: Ty, span: Span },
}

impl ThirPattern {
    /// Returns the span of this pattern.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Wildcard(span)
            | Self::Variable { span, .. }
            | Self::Constructor { span, .. }
            | Self::Tuple { span, .. }
            | Self::List { span, .. } => *span,
            Self::Literal(lit) => lit.span,
        }
    }
}

// ── THIR Match Arm ─────────────────────────────────────────────────

/// A type-checked match arm with a typed pattern.
#[derive(Debug)]
pub struct ThirMatchArm {
    pub pattern: ThirPattern,
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
    /// A let binding: `let x = expr` or `let mut x = expr`.
    Let { binding: DefId, value: Box<ThirExpr>, ty: Ty, span: Span },
    /// A reassignment: `x = expr`. Enforcement rules are in Issue 94.
    Assign { target: DefId, value: Box<ThirExpr>, ty: Ty, span: Span },
    /// An anonymous function: `fn(params) { body }`.
    Lambda { params: Vec<ThirParam>, body: Box<ThirExpr>, ty: Ty, span: Span },
    /// A field access: `expr.field`.
    FieldAccess { receiver: Box<ThirExpr>, field: SmolStr, ty: Ty, span: Span },
    /// A try expression: `try expr`. The type is the success type (unwrapped).
    Try { expr: Box<ThirExpr>, ty: Ty, span: Span },
    /// A list literal: `[1, 2, 3]`.
    List { elements: Vec<ThirExpr>, ty: Ty, span: Span },
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
            | Self::Match { ty, .. }
            | Self::Let { ty, .. }
            | Self::Assign { ty, .. }
            | Self::Lambda { ty, .. }
            | Self::FieldAccess { ty, .. }
            | Self::Try { ty, .. }
            | Self::List { ty, .. } => ty,
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
