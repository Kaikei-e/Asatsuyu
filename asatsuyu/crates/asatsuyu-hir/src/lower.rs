//! AST → HIR lowering with minimal name resolution.
//!
//! Walks the AST, registers definitions in the [`SymbolTable`], and resolves
//! variable references to [`DefId`]s. Uses a two-pass algorithm:
//! 1. Register all function names in module scope.
//! 2. Lower each function body, resolving variables against local then module scope.

use std::collections::HashMap;

use asatsuyu_ast::{self, Definition, Expr, Module};
use asatsuyu_syntax::{Diagnostic, Span};
use smol_str::SmolStr;

use crate::types::{
    DefData, DefId, DefKind, HirExpr, HirFnDef, HirLiteral, HirModule, HirParam, SymbolTable,
};

// ── Context ─────────────────────────────────────────────────────────

/// Accumulates state during AST → HIR lowering.
pub(crate) struct HirLowerCtx {
    symbol_table: SymbolTable,
    diagnostics: Vec<Diagnostic>,
    /// Module scope: function name → `DefId`.
    module_scope: HashMap<SmolStr, DefId>,
    /// Current function's local scope: parameter name → `DefId`.
    local_scope: HashMap<SmolStr, DefId>,
}

impl HirLowerCtx {
    pub(crate) fn new() -> Self {
        Self {
            symbol_table: SymbolTable::new(),
            diagnostics: Vec::new(),
            module_scope: HashMap::new(),
            local_scope: HashMap::new(),
        }
    }

    pub(crate) fn into_parts(self) -> (SymbolTable, Vec<Diagnostic>) {
        (self.symbol_table, self.diagnostics)
    }

    fn push_error(&mut self, message: impl Into<String>, span: Span) {
        self.diagnostics.push(Diagnostic::error(message, span));
    }

    /// Resolve a name: check local scope first, then module scope.
    fn resolve_name(&self, name: &SmolStr) -> Option<DefId> {
        self.local_scope.get(name).or_else(|| self.module_scope.get(name)).copied()
    }
}

// ── Module lowering ─────────────────────────────────────────────────

impl HirLowerCtx {
    /// Lower an AST module into HIR.
    pub(crate) fn lower_module(&mut self, ast: &Module) -> HirModule {
        // Pass 1: Register all function names in module scope.
        let fn_def_ids = self.register_functions(ast);

        // Pass 2: Lower each function body with name resolution.
        let functions = ast
            .definitions
            .iter()
            .zip(fn_def_ids)
            .map(|(def, def_id)| {
                let Definition::Function(ref fn_def) = *def;
                self.lower_fn_def(fn_def, def_id)
            })
            .collect();

        // symbol_table is transferred to HirModule by the public API via into_parts().
        HirModule { functions, symbol_table: SymbolTable::default(), span: ast.span }
    }

    /// Pass 1: Register all function definitions and return their `DefId`s.
    fn register_functions(&mut self, ast: &Module) -> Vec<DefId> {
        ast.definitions
            .iter()
            .map(|def| {
                let Definition::Function(ref fn_def) = *def;
                let def_id = self.symbol_table.alloc(DefData {
                    name: fn_def.name.name.clone(),
                    kind: DefKind::Function,
                    span: fn_def.name.span,
                });
                self.module_scope.insert(fn_def.name.name.clone(), def_id);
                def_id
            })
            .collect()
    }

    // ── Function lowering ───────────────────────────────────────────

    fn lower_fn_def(&mut self, fn_def: &asatsuyu_ast::FnDef, def_id: DefId) -> HirFnDef {
        // Clear local scope for this function.
        self.local_scope.clear();

        // Register parameters in local scope.
        let params: Vec<HirParam> = fn_def
            .params
            .iter()
            .map(|p| {
                let param_def_id = self.symbol_table.alloc(DefData {
                    name: p.name.name.clone(),
                    kind: DefKind::Parameter,
                    span: p.name.span,
                });
                self.local_scope.insert(p.name.name.clone(), param_def_id);
                HirParam { def_id: param_def_id, type_ann: p.type_ann.name.clone(), span: p.span }
            })
            .collect();

        let body = self.lower_expr(&fn_def.body);

        let return_type = fn_def.return_type.as_ref().map(|rt| rt.name.clone());

        HirFnDef {
            def_id,
            visibility: fn_def.visibility,
            params,
            return_type,
            body,
            span: fn_def.span,
        }
    }

    // ── Expression lowering ─────────────────────────────────────────

    fn lower_expr(&mut self, expr: &Expr) -> HirExpr {
        match expr {
            Expr::Literal(lit) => HirExpr::Literal(HirLiteral {
                kind: lit.kind,
                value: lit.value.clone(),
                span: lit.span,
            }),

            Expr::Variable(ident) => {
                if let Some(def_id) = self.resolve_name(&ident.name) {
                    HirExpr::Var(def_id, ident.span)
                } else {
                    self.push_error(format!("unresolved name `{}`", ident.name), ident.span);
                    // Allocate a dummy definition so downstream phases don't crash.
                    let dummy_id = self.symbol_table.alloc(DefData {
                        name: ident.name.clone(),
                        kind: DefKind::Parameter, // best guess
                        span: ident.span,
                    });
                    HirExpr::Var(dummy_id, ident.span)
                }
            }

            Expr::Block { exprs, span } => {
                let hir_exprs = exprs.iter().map(|e| self.lower_expr(e)).collect();
                HirExpr::Block { exprs: hir_exprs, span: *span }
            }
        }
    }
}
