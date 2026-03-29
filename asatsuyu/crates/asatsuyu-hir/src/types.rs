//! HIR type definitions for the Asatsuyu language.
//!
//! These types represent the **high-level intermediate representation**,
//! where variable references are resolved to [`DefId`]s via a [`SymbolTable`].
//! Every node carries a [`Span`] for error reporting.

use asatsuyu_ast::{LiteralKind, Visibility};
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
    pub span: Span,
}

/// What kind of thing a [`DefId`] refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefKind {
    Function,
    Parameter,
}

// ── Symbol Table ────────────────────────────────────────────────────

/// Provisional symbol table: an arena of definitions.
///
/// Stores all definitions (functions, parameters) for a module. Name lookup
/// is handled by the lowering context's scope maps, not by this struct.
///
/// Issue 20 will add lexical scopes, nested resolution, and shadowing.
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
    pub functions: Vec<HirFnDef>,
    pub symbol_table: SymbolTable,
    pub span: Span,
}

// ── HIR Function ────────────────────────────────────────────────────

/// A function definition in HIR, with a resolved [`DefId`].
#[derive(Debug)]
pub struct HirFnDef {
    pub def_id: DefId,
    pub visibility: Visibility,
    pub params: Vec<HirParam>,
    /// Return type name. `None` when omitted.
    pub return_type: Option<SmolStr>,
    pub body: HirExpr,
    pub span: Span,
}

// ── HIR Parameter ───────────────────────────────────────────────────

/// A function parameter in HIR, with a resolved [`DefId`].
#[derive(Debug)]
pub struct HirParam {
    pub def_id: DefId,
    /// Type annotation name (currently a simple identifier).
    pub type_ann: SmolStr,
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
}

impl HirExpr {
    /// Returns the span of this expression.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Literal(lit) => lit.span,
            Self::Var(_, span) | Self::Block { span, .. } => *span,
        }
    }
}
