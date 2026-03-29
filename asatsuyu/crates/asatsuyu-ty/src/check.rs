//! HIR → THIR type checking with Hindley-Milner unification.
//!
//! Walks the HIR, resolves type annotations, and attaches a [`Ty`] to every
//! expression node. Uses a two-pass approach:
//! 1. Collect function signatures into the type environment.
//! 2. Check each function body, comparing inferred types against annotations.

use std::collections::{HashMap, HashSet};

use asatsuyu_ast::{BinOp, LiteralKind, UnOp};
use asatsuyu_hir::{DefId, HirExpr, HirFnDef, HirModule, SymbolTable};
use asatsuyu_syntax::{Diagnostic, Span};

use crate::types::{
    PrimTy, ThirExpr, ThirFnDef, ThirLiteral, ThirMatchArm, ThirModule, ThirParam, Ty, TyVarId,
    TypeScheme,
};
use crate::unify::{InferCtx, UnifyErrorKind};

// ── Context ────────────────────────────────────────────────────────

/// Accumulates state during HIR → THIR type checking.
pub(crate) struct TyCheckCtx {
    /// Maps each `DefId` to its type scheme (monomorphic or polymorphic).
    type_env: HashMap<DefId, TypeScheme>,
    /// Functions whose return type was not explicitly annotated.
    unannotated_returns: HashSet<DefId>,
    diagnostics: Vec<Diagnostic>,
    /// Hindley-Milner inference state.
    infer: InferCtx,
}

impl TyCheckCtx {
    pub(crate) fn new() -> Self {
        Self {
            type_env: HashMap::new(),
            unannotated_returns: HashSet::new(),
            diagnostics: Vec::new(),
            infer: InferCtx::new(),
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

    /// Unify two types, emitting a diagnostic on failure.
    fn unify_or_error(&mut self, expected: &Ty, found: &Ty, span: Span) {
        if let Err(err) = self.infer.unify(expected, found) {
            match err.kind {
                UnifyErrorKind::Mismatch { expected, found } => {
                    let exp = self.infer.resolve(&expected);
                    let fnd = self.infer.resolve(&found);
                    self.push_error(
                        format!("type mismatch: expected `{exp}`, found `{fnd}`"),
                        span,
                    );
                }
                UnifyErrorKind::InfiniteType { var, ty } => {
                    let resolved = self.infer.resolve(&ty);
                    self.push_error(
                        format!("infinite type: type variable `?{}` occurs in `{resolved}`", var.0),
                        span,
                    );
                }
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
                self.type_env.insert(p.def_id, TypeScheme::mono(ty.clone()));
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
        self.type_env.insert(fn_def.def_id, TypeScheme::mono(fn_ty));
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
                let ty = self.type_env.get(&p.def_id).map_or(Ty::Error, |s| s.ty.clone());
                ThirParam { def_id: p.def_id, ty, span: p.span }
            })
            .collect();

        // Check the body.
        let body = self.check_expr(&fn_def.body);
        let body_ty = self.infer.resolve(body.ty());

        // Extract the declared return type from the function signature.
        let fn_scheme = self.type_env.get(&fn_def.def_id).cloned();
        let fn_ty = fn_scheme.map_or(Ty::Error, |s| s.ty);
        let declared_ret = match &fn_ty {
            Ty::Function { ret, .. } => *ret.clone(),
            _ => Ty::Error,
        };

        // Determine the actual return type.
        let is_unannotated = self.unannotated_returns.contains(&fn_def.def_id);
        let return_ty = if is_unannotated {
            // Infer return type from body.
            body_ty
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
                    LiteralKind::Float => Ty::Primitive(PrimTy::Float),
                    LiteralKind::String => Ty::Primitive(PrimTy::String),
                    LiteralKind::Bool => Ty::Primitive(PrimTy::Bool),
                };
                ThirExpr::Literal(ThirLiteral {
                    kind: lit.kind,
                    value: lit.value.clone(),
                    ty,
                    span: lit.span,
                })
            }

            HirExpr::Var(def_id, span) => {
                let ty = match self.type_env.get(def_id) {
                    Some(scheme) => self.infer.instantiate(scheme),
                    None => Ty::Error,
                };
                ThirExpr::Var { def_id: *def_id, ty, span: *span }
            }

            HirExpr::Block { exprs, span } => {
                let checked: Vec<ThirExpr> = exprs.iter().map(|e| self.check_expr(e)).collect();
                let ty = checked
                    .last()
                    .map_or(Ty::Primitive(PrimTy::None), |e| self.infer.resolve(e.ty()));
                ThirExpr::Block { exprs: checked, ty, span: *span }
            }

            HirExpr::Call { func, args, span } => self.check_call(func, args, *span),

            HirExpr::BinaryOp { op, lhs, rhs, span } => self.check_binary_op(*op, lhs, rhs, *span),

            HirExpr::UnaryOp { op, expr, span } => self.check_unary_op(*op, expr, *span),

            HirExpr::If { condition, then_body, else_body, span } => {
                self.check_if(condition, then_body, else_body.as_deref(), *span)
            }

            HirExpr::Match { subject, arms, span } => self.check_match(subject, arms, *span),

            HirExpr::Let { binding, value, span } => {
                let checked_value = self.check_expr(value);
                let value_ty = self.infer.resolve(checked_value.ty());
                let env_fvs = self.env_free_vars();
                let scheme = self.infer.generalize(&value_ty, &env_fvs);
                self.type_env.insert(*binding, scheme);
                ThirExpr::Let {
                    binding: *binding,
                    value: Box::new(checked_value),
                    ty: Ty::Primitive(PrimTy::None),
                    span: *span,
                }
            }

            HirExpr::Lambda { params, return_type, body, span } => {
                self.check_lambda(params, return_type.as_deref(), body, *span)
            }
        }
    }

    // ── Lambda ─────────────────────────────────────────────────────

    fn check_lambda(
        &mut self,
        params: &[asatsuyu_hir::HirParam],
        return_type: Option<&str>,
        body: &HirExpr,
        span: Span,
    ) -> ThirExpr {
        // Assign types to parameters: annotated → resolve, unannotated → fresh var.
        // Track param DefIds so we can remove them from env after checking.
        let param_def_ids: Vec<DefId> = params.iter().map(|p| p.def_id).collect();
        let thir_params: Vec<ThirParam> = params
            .iter()
            .map(|p| {
                let ty = if p.type_ann.is_empty() {
                    self.infer.fresh_var()
                } else {
                    self.resolve_type_name(&p.type_ann, p.span)
                };
                self.type_env.insert(p.def_id, TypeScheme::mono(ty.clone()));
                ThirParam { def_id: p.def_id, ty, span: p.span }
            })
            .collect();

        let checked_body = self.check_expr(body);
        let body_ty = self.infer.resolve(checked_body.ty());

        // Remove lambda params from type_env to prevent them from polluting
        // the environment during generalization of let-bound values.
        for def_id in &param_def_ids {
            self.type_env.remove(def_id);
        }

        let ret_ty = if let Some(ret_name) = return_type {
            let declared = self.resolve_type_name(ret_name, span);
            self.unify_or_error(&declared, &body_ty, body.span());
            declared
        } else {
            body_ty
        };

        let param_tys: Vec<Ty> = thir_params.iter().map(|p| self.infer.resolve(&p.ty)).collect();
        let fn_ty = Ty::Function { params: param_tys, ret: Box::new(self.infer.resolve(&ret_ty)) };

        ThirExpr::Lambda { params: thir_params, body: Box::new(checked_body), ty: fn_ty, span }
    }

    /// Collect free type variables across the entire type environment.
    fn env_free_vars(&self) -> HashSet<TyVarId> {
        let mut fvs = HashSet::new();
        for scheme in self.type_env.values() {
            let ty_fvs = self.infer.free_vars(&scheme.ty);
            let quantified: HashSet<_> = scheme.vars.iter().copied().collect();
            for v in ty_fvs {
                if !quantified.contains(&v) {
                    fvs.insert(v);
                }
            }
        }
        fvs
    }

    // ── Call ────────────────────────────────────────────────────────

    fn check_call(&mut self, func: &HirExpr, args: &[HirExpr], span: Span) -> ThirExpr {
        let checked_func = self.check_expr(func);
        let func_ty = self.infer.resolve(checked_func.ty());

        let checked_args: Vec<ThirExpr> = args.iter().map(|a| self.check_expr(a)).collect();

        match func_ty {
            Ty::Function { params, ret } => {
                if args.len() == params.len() {
                    for (arg, param_ty) in checked_args.iter().zip(params.iter()) {
                        let arg_ty = self.infer.resolve(arg.ty());
                        self.unify_or_error(param_ty, &arg_ty, arg.span());
                    }
                } else {
                    self.push_error(
                        format!(
                            "function expects {} argument(s), but {} were given",
                            params.len(),
                            args.len()
                        ),
                        span,
                    );
                }
                let ty = self.infer.resolve(&ret);
                ThirExpr::Call { func: Box::new(checked_func), args: checked_args, ty, span }
            }
            Ty::Error => ThirExpr::Call {
                func: Box::new(checked_func),
                args: checked_args,
                ty: Ty::Error,
                span,
            },
            _ => {
                self.push_error(format!("expected function, found `{func_ty}`"), func.span());
                ThirExpr::Call {
                    func: Box::new(checked_func),
                    args: checked_args,
                    ty: Ty::Error,
                    span,
                }
            }
        }
    }

    // ── BinaryOp ───────────────────────────────────────────────────

    fn check_binary_op(&mut self, op: BinOp, lhs: &HirExpr, rhs: &HirExpr, span: Span) -> ThirExpr {
        let checked_lhs = self.check_expr(lhs);
        let checked_rhs = self.check_expr(rhs);
        let lhs_ty = self.infer.resolve(checked_lhs.ty());
        let rhs_ty = self.infer.resolve(checked_rhs.ty());

        let ty = match op {
            // Arithmetic: both must be same numeric type.
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                self.unify_or_error(&lhs_ty, &rhs_ty, span);
                let unified = self.infer.resolve(&lhs_ty);
                if !is_numeric(&unified) && unified != Ty::Error {
                    self.push_error(
                        format!("arithmetic operator requires numeric type, found `{unified}`"),
                        span,
                    );
                    Ty::Error
                } else {
                    unified
                }
            }
            // Comparison: both must be same type, result is Bool.
            BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::LtEq | BinOp::Gt | BinOp::GtEq => {
                self.unify_or_error(&lhs_ty, &rhs_ty, span);
                Ty::Primitive(PrimTy::Bool)
            }
            // Logical: both must be Bool.
            BinOp::And | BinOp::Or => {
                self.unify_or_error(&Ty::Primitive(PrimTy::Bool), &lhs_ty, checked_lhs.span());
                self.unify_or_error(&Ty::Primitive(PrimTy::Bool), &rhs_ty, checked_rhs.span());
                Ty::Primitive(PrimTy::Bool)
            }
            // StringConcat: desugared in HIR, but handle defensively.
            BinOp::StringConcat => {
                self.unify_or_error(&Ty::Primitive(PrimTy::String), &lhs_ty, checked_lhs.span());
                self.unify_or_error(&Ty::Primitive(PrimTy::String), &rhs_ty, checked_rhs.span());
                Ty::Primitive(PrimTy::String)
            }
        };

        ThirExpr::BinaryOp { op, lhs: Box::new(checked_lhs), rhs: Box::new(checked_rhs), ty, span }
    }

    // ── UnaryOp ────────────────────────────────────────────────────

    fn check_unary_op(&mut self, op: UnOp, expr: &HirExpr, span: Span) -> ThirExpr {
        let checked = self.check_expr(expr);
        let expr_ty = self.infer.resolve(checked.ty());

        let ty = match op {
            UnOp::Neg => {
                if !is_numeric(&expr_ty) && expr_ty != Ty::Error {
                    self.push_error(
                        format!("negation requires numeric type, found `{expr_ty}`"),
                        span,
                    );
                    Ty::Error
                } else {
                    expr_ty
                }
            }
            UnOp::Not => {
                self.unify_or_error(&Ty::Primitive(PrimTy::Bool), &expr_ty, span);
                Ty::Primitive(PrimTy::Bool)
            }
        };

        ThirExpr::UnaryOp { op, expr: Box::new(checked), ty, span }
    }

    // ── If ──────────────────────────────────────────────────────────

    fn check_if(
        &mut self,
        condition: &HirExpr,
        then_body: &HirExpr,
        else_body: Option<&HirExpr>,
        span: Span,
    ) -> ThirExpr {
        let checked_cond = self.check_expr(condition);
        let cond_ty = self.infer.resolve(checked_cond.ty());
        self.unify_or_error(&Ty::Primitive(PrimTy::Bool), &cond_ty, checked_cond.span());

        let checked_then = self.check_expr(then_body);
        let then_ty = self.infer.resolve(checked_then.ty());

        let (checked_else, ty) = if let Some(else_expr) = else_body {
            let checked = self.check_expr(else_expr);
            let else_ty = self.infer.resolve(checked.ty());
            self.unify_or_error(&then_ty, &else_ty, span);
            let ty = self.infer.resolve(&then_ty);
            (Some(Box::new(checked)), ty)
        } else {
            (None, then_ty)
        };

        ThirExpr::If {
            condition: Box::new(checked_cond),
            then_body: Box::new(checked_then),
            else_body: checked_else,
            ty,
            span,
        }
    }

    // ── Match ──────────────────────────────────────────────────────

    fn check_match(
        &mut self,
        subject: &HirExpr,
        arms: &[asatsuyu_hir::HirMatchArm],
        span: Span,
    ) -> ThirExpr {
        let checked_subject = self.check_expr(subject);

        // Pattern typing deferred to Issue 26+. Only check arm bodies.
        let mut checked_arms = Vec::with_capacity(arms.len());
        let mut result_ty: Option<Ty> = None;

        for arm in arms {
            let checked_body = self.check_expr(&arm.body);
            let arm_ty = self.infer.resolve(checked_body.ty());

            if let Some(ref prev_ty) = result_ty {
                self.unify_or_error(prev_ty, &arm_ty, arm.span);
            } else {
                result_ty = Some(arm_ty);
            }

            checked_arms.push(ThirMatchArm { body: checked_body, span: arm.span });
        }

        let ty = result_ty.map_or(Ty::Primitive(PrimTy::None), |t| self.infer.resolve(&t));

        ThirExpr::Match { subject: Box::new(checked_subject), arms: checked_arms, ty, span }
    }
}

// ── Helpers ────────────────────────────────────────────────────────

fn is_numeric(ty: &Ty) -> bool {
    matches!(ty, Ty::Primitive(PrimTy::Int | PrimTy::Float))
}

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
