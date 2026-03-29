//! AST → HIR lowering with lexical scope resolution.
//!
//! Walks the AST, registers definitions in the [`SymbolTable`], and resolves
//! variable references to [`DefId`]s using a scope stack. Uses a two-pass
//! algorithm:
//! 1. Register all top-level names (functions, constructors) in module scope.
//! 2. Lower each definition body, resolving variables with lexical scoping.

use std::collections::HashMap;

use asatsuyu_ast::{self, Definition, Expr, Module, Pattern, TypeBody};
use asatsuyu_syntax::{Diagnostic, Span};
use smol_str::SmolStr;

use crate::types::{
    DefData, DefId, DefKind, HirCustomType, HirExpr, HirFnDef, HirImport, HirLiteral, HirMatchArm,
    HirModule, HirParam, HirPattern, SymbolTable,
};

// ── Scope Stack ─────────────────────────────────────────────────────

/// A stack of lexical scopes for name resolution.
///
/// Names are resolved by searching from the innermost scope outward.
/// Module scope is at index 0; function/block/arm scopes are pushed on top.
pub(crate) struct ScopeStack {
    scopes: Vec<HashMap<SmolStr, DefId>>,
}

impl ScopeStack {
    fn new() -> Self {
        Self { scopes: vec![HashMap::new()] }
    }

    /// Push a new empty scope (entering block, function, match arm).
    fn push(&mut self) {
        self.scopes.push(HashMap::new());
    }

    /// Pop the innermost scope (leaving block, function, match arm).
    fn pop(&mut self) {
        self.scopes.pop();
    }

    /// Define a name in the current (innermost) scope.
    /// Returns the previous `DefId` if the name was already defined in this scope.
    fn define(&mut self, name: SmolStr, id: DefId) -> Option<DefId> {
        self.scopes.last_mut().expect("scope stack empty").insert(name, id)
    }

    /// Resolve a name by searching from innermost to outermost scope.
    fn resolve(&self, name: &SmolStr) -> Option<DefId> {
        for scope in self.scopes.iter().rev() {
            if let Some(&id) = scope.get(name) {
                return Some(id);
            }
        }
        None
    }
}

// ── Context ─────────────────────────────────────────────────────────

/// Accumulates state during AST → HIR lowering.
pub(crate) struct HirLowerCtx {
    symbol_table: SymbolTable,
    diagnostics: Vec<Diagnostic>,
    scopes: ScopeStack,
    /// `DefId` for the built-in `string_concat` function.
    string_concat_id: DefId,
}

impl HirLowerCtx {
    pub(crate) fn new() -> Self {
        let mut symbol_table = SymbolTable::new();
        let string_concat_id = symbol_table.alloc(DefData {
            name: SmolStr::from("string_concat"),
            kind: DefKind::Builtin,
            span: Span::dummy(),
        });
        let mut scopes = ScopeStack::new();
        scopes.define(SmolStr::from("string_concat"), string_concat_id);
        Self { symbol_table, diagnostics: Vec::new(), scopes, string_concat_id }
    }

    pub(crate) fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    fn take_symbol_table(&mut self) -> SymbolTable {
        std::mem::take(&mut self.symbol_table)
    }

    fn push_error(&mut self, message: impl Into<String>, span: Span) {
        self.diagnostics.push(Diagnostic::error(message, span));
    }

    /// Define a name in the current scope. If a duplicate exists in the
    /// **module scope** (bottom of stack), emit a diagnostic.
    fn define_module_level(&mut self, name: &SmolStr, id: DefId, span: Span) {
        if let Some(prev_id) = self.scopes.define(name.clone(), id) {
            let prev = self.symbol_table.get(prev_id);
            self.diagnostics.push(
                Diagnostic::error(format!("duplicate definition `{name}`"), span)
                    .with_secondary_label(prev.span, "previously defined here"),
            );
        }
    }
}

// ── Module lowering ─────────────────────────────────────────────────

impl HirLowerCtx {
    /// Lower an AST module into HIR.
    pub(crate) fn lower_module(&mut self, ast: &Module) -> HirModule {
        // Pass 0: Register import bindings in module scope.
        let imports = self.register_imports(ast);

        // Pass 1: Register all top-level names in module scope.
        let (fn_entries, ct_entries) = self.register_top_level(ast);

        // Pass 2: Lower each function body with name resolution.
        let functions = fn_entries
            .into_iter()
            .map(|(fn_def, def_id)| self.lower_fn_def(fn_def, def_id))
            .collect();

        let custom_types = ct_entries
            .into_iter()
            .map(|(ct, def_id)| HirCustomType { def_id, visibility: ct.visibility, span: ct.span })
            .collect();

        HirModule {
            imports,
            functions,
            custom_types,
            symbol_table: self.take_symbol_table(),
            span: ast.span,
        }
    }

    /// Pass 0: Register import bindings in module scope.
    ///
    /// For `import gleam.io`, binds `io` in scope.
    /// For `import gleam.io as stdio`, binds `stdio` in scope.
    fn register_imports(&mut self, ast: &Module) -> Vec<HirImport> {
        ast.imports
            .iter()
            .filter_map(|imp| {
                let bound_name =
                    if let Some(alias) = &imp.alias { alias } else { imp.module.last()? };

                let def_id = self.symbol_table.alloc(DefData {
                    name: bound_name.name.clone(),
                    kind: DefKind::Import,
                    span: bound_name.span,
                });

                self.define_module_level(&bound_name.name, def_id, bound_name.span);

                let module_path = imp.module.iter().map(|seg| seg.name.clone()).collect();

                Some(HirImport { def_id, module_path, span: imp.span })
            })
            .collect()
    }

    /// Pass 1: Register all function names, type names, and constructors.
    #[allow(clippy::type_complexity)]
    fn register_top_level<'a>(
        &mut self,
        ast: &'a Module,
    ) -> (Vec<(&'a asatsuyu_ast::FnDef, DefId)>, Vec<(&'a asatsuyu_ast::CustomType, DefId)>) {
        let mut fn_entries = Vec::new();
        let mut ct_entries = Vec::new();

        for def in &ast.definitions {
            match def {
                Definition::Function(fn_def) => {
                    let def_id = self.symbol_table.alloc(DefData {
                        name: fn_def.name.name.clone(),
                        kind: DefKind::Function,
                        span: fn_def.name.span,
                    });
                    self.define_module_level(&fn_def.name.name, def_id, fn_def.name.span);
                    fn_entries.push((fn_def, def_id));
                }
                Definition::CustomType(ct) => {
                    let type_def_id = self.symbol_table.alloc(DefData {
                        name: ct.name.name.clone(),
                        kind: DefKind::Type,
                        span: ct.name.span,
                    });
                    self.define_module_level(&ct.name.name, type_def_id, ct.name.span);
                    ct_entries.push((ct, type_def_id));

                    // Register constructors in module scope.
                    if let TypeBody::Variants(variants) = &ct.body {
                        for variant in variants {
                            let ctor_id = self.symbol_table.alloc(DefData {
                                name: variant.name.name.clone(),
                                kind: DefKind::Constructor,
                                span: variant.name.span,
                            });
                            self.define_module_level(
                                &variant.name.name,
                                ctor_id,
                                variant.name.span,
                            );
                        }
                    }
                }
            }
        }

        (fn_entries, ct_entries)
    }

    // ── Function lowering ───────────────────────────────────────────

    fn lower_fn_def(&mut self, fn_def: &asatsuyu_ast::FnDef, def_id: DefId) -> HirFnDef {
        // Push function scope.
        self.scopes.push();

        // Register parameters in function scope.
        let params: Vec<HirParam> = fn_def
            .params
            .iter()
            .map(|p| {
                let param_def_id = self.symbol_table.alloc(DefData {
                    name: p.name.name.clone(),
                    kind: DefKind::Parameter,
                    span: p.name.span,
                });
                self.scopes.define(p.name.name.clone(), param_def_id);
                let type_name = match &p.type_ann {
                    asatsuyu_ast::TypeExpr::Named { name, .. } => name.name.clone(),
                };
                HirParam { def_id: param_def_id, type_ann: type_name, span: p.span }
            })
            .collect();

        let body = self.lower_expr(&fn_def.body);

        let return_type = fn_def.return_type.as_ref().map(|rt| match rt {
            asatsuyu_ast::TypeExpr::Named { name, .. } => name.name.clone(),
        });

        // Pop function scope.
        self.scopes.pop();

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
                if let Some(def_id) = self.scopes.resolve(&ident.name) {
                    HirExpr::Var(def_id, ident.span)
                } else {
                    self.push_error(format!("unresolved name `{}`", ident.name), ident.span);
                    // Allocate a dummy definition so downstream phases don't crash.
                    let dummy_id = self.symbol_table.alloc(DefData {
                        name: ident.name.clone(),
                        kind: DefKind::LocalBinding,
                        span: ident.span,
                    });
                    HirExpr::Var(dummy_id, ident.span)
                }
            }

            Expr::Block { exprs, span } => {
                self.scopes.push();
                let hir_exprs = exprs.iter().map(|e| self.lower_expr(e)).collect();
                self.scopes.pop();
                HirExpr::Block { exprs: hir_exprs, span: *span }
            }

            Expr::Call { func, args, span } => {
                let hir_func = self.lower_expr(func);
                let hir_args = args.iter().map(|a| self.lower_expr(a)).collect();
                HirExpr::Call { func: Box::new(hir_func), args: hir_args, span: *span }
            }

            Expr::BinaryOp { op: asatsuyu_ast::BinOp::StringConcat, lhs, rhs, span } => {
                // Desugar: "a" <> "b" → string_concat("a", "b")
                let hir_lhs = self.lower_expr(lhs);
                let hir_rhs = self.lower_expr(rhs);
                HirExpr::Call {
                    func: Box::new(HirExpr::Var(self.string_concat_id, *span)),
                    args: vec![hir_lhs, hir_rhs],
                    span: *span,
                }
            }

            Expr::BinaryOp { op, lhs, rhs, span } => {
                let hir_lhs = self.lower_expr(lhs);
                let hir_rhs = self.lower_expr(rhs);
                HirExpr::BinaryOp {
                    op: *op,
                    lhs: Box::new(hir_lhs),
                    rhs: Box::new(hir_rhs),
                    span: *span,
                }
            }

            Expr::UnaryOp { op, expr, span } => {
                let hir_expr = self.lower_expr(expr);
                HirExpr::UnaryOp { op: *op, expr: Box::new(hir_expr), span: *span }
            }

            Expr::If { condition, then_body, else_body, span } => {
                let hir_cond = self.lower_expr(condition);
                let hir_then = self.lower_expr(then_body);
                let hir_else = else_body.as_ref().map(|e| Box::new(self.lower_expr(e)));
                HirExpr::If {
                    condition: Box::new(hir_cond),
                    then_body: Box::new(hir_then),
                    else_body: hir_else,
                    span: *span,
                }
            }

            Expr::Match { subject, arms, span } => {
                let hir_subject = self.lower_expr(subject);
                let hir_arms = arms.iter().map(|arm| self.lower_match_arm(arm)).collect();
                HirExpr::Match { subject: Box::new(hir_subject), arms: hir_arms, span: *span }
            }

            Expr::Pipeline { left, right, span } => {
                // Desugar: x |> f(y) → f(x, y), x |> f → f(x)
                let hir_left = self.lower_expr(left);
                let hir_right = self.lower_expr(right);
                match hir_right {
                    HirExpr::Call { func, mut args, .. } => {
                        args.insert(0, hir_left);
                        HirExpr::Call { func, args, span: *span }
                    }
                    other => {
                        HirExpr::Call { func: Box::new(other), args: vec![hir_left], span: *span }
                    }
                }
            }
        }
    }

    // ── Match arm lowering ──────────────────────────────────────────

    fn lower_match_arm(&mut self, arm: &asatsuyu_ast::MatchArm) -> HirMatchArm {
        // Each arm gets its own scope for pattern bindings.
        self.scopes.push();

        let pattern = self.lower_pattern(&arm.pattern);
        let guard = arm.guard.as_ref().map(|g| Box::new(self.lower_expr(g)));
        let body = self.lower_expr(&arm.body);

        self.scopes.pop();

        HirMatchArm { pattern, guard, body, span: arm.span }
    }

    // ── Pattern lowering ────────────────────────────────────────────

    fn lower_pattern(&mut self, pat: &Pattern) -> HirPattern {
        match pat {
            Pattern::Wildcard(span) => HirPattern::Wildcard(*span),

            Pattern::Variable(ident) => {
                let def_id = self.symbol_table.alloc(DefData {
                    name: ident.name.clone(),
                    kind: DefKind::LocalBinding,
                    span: ident.span,
                });
                self.scopes.define(ident.name.clone(), def_id);
                HirPattern::Variable(def_id, ident.span)
            }

            Pattern::Literal(lit) => HirPattern::Literal(HirLiteral {
                kind: lit.kind,
                value: lit.value.clone(),
                span: lit.span,
            }),

            Pattern::Constructor { name, fields, span } => {
                let hir_fields = fields.iter().map(|f| self.lower_pattern(f)).collect();
                let def_id = if let Some(id) = self.scopes.resolve(&name.name) {
                    id
                } else {
                    self.push_error(format!("unresolved constructor `{}`", name.name), name.span);
                    self.symbol_table.alloc(DefData {
                        name: name.name.clone(),
                        kind: DefKind::Constructor,
                        span: name.span,
                    })
                };
                HirPattern::Constructor { def_id, fields: hir_fields, span: *span }
            }

            Pattern::List { elements, rest, span } => {
                let hir_elements = elements.iter().map(|e| self.lower_pattern(e)).collect();
                let hir_rest = rest.as_ref().map(|ident| {
                    let def_id = self.symbol_table.alloc(DefData {
                        name: ident.name.clone(),
                        kind: DefKind::LocalBinding,
                        span: ident.span,
                    });
                    self.scopes.define(ident.name.clone(), def_id);
                    def_id
                });
                HirPattern::List { elements: hir_elements, rest: hir_rest, span: *span }
            }
        }
    }
}
