//! HIR → THIR type checking.
//!
//! Walks the HIR, resolves type annotations, and attaches a [`Ty`] to every
//! expression node. Uses a two-pass approach:
//! 1. Collect function signatures into the type environment.
//! 2. Check each function body, comparing inferred types against annotations.

use std::collections::{HashMap, HashSet};

use asatsuyu_ast::LiteralKind;
use asatsuyu_hir::{DefId, HirExpr, HirFnDef, HirModule, SymbolTable};
use asatsuyu_syntax::{Diagnostic, Span};

use crate::types::{PrimTy, ThirExpr, ThirFnDef, ThirLiteral, ThirModule, ThirParam, Ty};

// ── Context ────────────────────────────────────────────────────────

/// Accumulates state during HIR → THIR type checking.
pub(crate) struct TyCheckCtx {
    /// Maps each `DefId` to its resolved type.
    type_env: HashMap<DefId, Ty>,
    /// Functions whose return type was not explicitly annotated.
    unannotated_returns: HashSet<DefId>,
    diagnostics: Vec<Diagnostic>,
}

impl TyCheckCtx {
    pub(crate) fn new() -> Self {
        Self {
            type_env: HashMap::new(),
            unannotated_returns: HashSet::new(),
            diagnostics: Vec::new(),
        }
    }

    pub(crate) fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    fn push_error(&mut self, message: impl Into<String>, span: Span) {
        self.diagnostics.push(Diagnostic::error(message, span));
    }

    /// Resolve a type annotation name to a [`Ty`].
    fn resolve_type_name(&mut self, name: &str, span: Span) -> Ty {
        match name {
            "Int" => Ty::Primitive(PrimTy::Int),
            "Float" => Ty::Primitive(PrimTy::Float),
            "String" => Ty::Primitive(PrimTy::String),
            "Bool" => Ty::Primitive(PrimTy::Bool),
            "None" => Ty::Primitive(PrimTy::None),
            _ => {
                self.push_error(format!("unknown type `{name}`"), span);
                Ty::Error
            }
        }
    }
}

// ── Pass 1: Collect signatures ─────────────────────────────────────

impl TyCheckCtx {
    /// Register all function signatures in the type environment.
    pub(crate) fn collect_signatures(&mut self, module: &HirModule) {
        for fn_def in &module.functions {
            self.collect_fn_signature(fn_def);
        }
    }

    fn collect_fn_signature(&mut self, fn_def: &HirFnDef) {
        // Resolve parameter types.
        let param_tys: Vec<Ty> = fn_def
            .params
            .iter()
            .map(|p| {
                let ty = self.resolve_type_name(&p.type_ann, p.span);
                self.type_env.insert(p.def_id, ty.clone());
                ty
            })
            .collect();

        // Resolve return type: annotated or provisional None.
        let ret_ty = if let Some(name) = &fn_def.return_type {
            self.resolve_type_name(name, fn_def.span)
        } else {
            self.unannotated_returns.insert(fn_def.def_id);
            Ty::Primitive(PrimTy::None) // provisional; replaced after body check
        };

        let fn_ty = Ty::Function { params: param_tys, ret: Box::new(ret_ty) };
        self.type_env.insert(fn_def.def_id, fn_ty);
    }
}

// ── Pass 2: Check bodies ───────────────────────────────────────────

impl TyCheckCtx {
    /// Type-check the entire module, producing THIR.
    pub(crate) fn check_module(&mut self, module: &HirModule) -> ThirModule {
        let functions = module.functions.iter().map(|f| self.check_fn_def(f)).collect();
        let symbol_table = clone_symbol_table(&module.symbol_table);
        ThirModule { functions, symbol_table, span: module.span }
    }

    fn check_fn_def(&mut self, fn_def: &HirFnDef) -> ThirFnDef {
        // Build typed parameters.
        let params: Vec<ThirParam> = fn_def
            .params
            .iter()
            .map(|p| {
                let ty = self.type_env.get(&p.def_id).cloned().unwrap_or(Ty::Error);
                ThirParam { def_id: p.def_id, ty, span: p.span }
            })
            .collect();

        // Check the body.
        let body = self.check_expr(&fn_def.body);
        let body_ty = body.ty().clone();

        // Extract the declared return type from the function signature.
        let fn_ty = self.type_env.get(&fn_def.def_id).cloned().unwrap_or(Ty::Error);
        let declared_ret = match &fn_ty {
            Ty::Function { ret, .. } => *ret.clone(),
            _ => Ty::Error,
        };

        // Determine the actual return type.
        let is_unannotated = self.unannotated_returns.contains(&fn_def.def_id);
        let return_ty = if is_unannotated {
            // Infer return type from body.
            body_ty.clone()
        } else {
            // Check declared return type against body.
            if declared_ret != Ty::Error && body_ty != Ty::Error && declared_ret != body_ty {
                self.push_error(
                    format!("type mismatch: expected `{declared_ret}`, found `{body_ty}`"),
                    fn_def.body.span(),
                );
            }
            declared_ret
        };

        // Build the final function type with the resolved return type.
        let param_tys: Vec<Ty> = params.iter().map(|p| p.ty.clone()).collect();
        let final_fn_ty = Ty::Function { params: param_tys, ret: Box::new(return_ty.clone()) };

        ThirFnDef {
            def_id: fn_def.def_id,
            visibility: fn_def.visibility,
            params,
            return_ty,
            body,
            ty: final_fn_ty,
            span: fn_def.span,
        }
    }

    fn check_expr(&mut self, expr: &HirExpr) -> ThirExpr {
        match expr {
            HirExpr::Literal(lit) => {
                let ty = match lit.kind {
                    LiteralKind::Int => Ty::Primitive(PrimTy::Int),
                    LiteralKind::String => Ty::Primitive(PrimTy::String),
                };
                ThirExpr::Literal(ThirLiteral {
                    kind: lit.kind,
                    value: lit.value.clone(),
                    ty,
                    span: lit.span,
                })
            }

            HirExpr::Var(def_id, span) => {
                let ty = self.type_env.get(def_id).cloned().unwrap_or(Ty::Error);
                ThirExpr::Var { def_id: *def_id, ty, span: *span }
            }

            HirExpr::Block { exprs, span } => {
                let checked: Vec<ThirExpr> = exprs.iter().map(|e| self.check_expr(e)).collect();
                let ty = checked.last().map_or(Ty::Primitive(PrimTy::None), |e| e.ty().clone());
                ThirExpr::Block { exprs: checked, ty, span: *span }
            }
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────────

/// Rebuild a [`SymbolTable`] by iterating and re-allocating.
///
/// Preserves `DefId` ordering (arena indices are allocated sequentially).
fn clone_symbol_table(st: &SymbolTable) -> SymbolTable {
    let mut new = SymbolTable::new();
    for (_, data) in st.iter() {
        new.alloc(data.clone());
    }
    new
}
