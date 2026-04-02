//! HIR → THIR type checking with Hindley-Milner unification.
//!
//! Walks the HIR, resolves type annotations, and attaches a [`Ty`] to every
//! expression node. Uses a two-pass approach:
//! 1. Collect function signatures into the type environment.
//! 2. Check each function body, comparing inferred types against annotations.

use std::collections::{HashMap, HashSet};

use asatsuyu_ast::{BinOp, LiteralKind, UnOp};
use asatsuyu_hir::ffi::{
    ChainResolver, FfiClass, FfiModule, FfiModuleResolver as _, FfiResolverConfig, FfiSignature,
    FfiSymbolKind, FfiTrustLevel, FfiType,
};
use asatsuyu_hir::{
    DefData, DefId, DefKind, HirExpr, HirFnDef, HirImportKind, HirModule, SymbolTable,
};
use asatsuyu_syntax::{Diagnostic, DiagnosticCode, Span};
use smol_str::SmolStr;

use crate::types::{
    PrimTy, ThirExpr, ThirFnDef, ThirLiteral, ThirMatchArm, ThirModule, ThirParam, ThirPattern, Ty,
    TyVarId, TypeScheme,
};
use crate::unify::{InferCtx, UnifyErrorKind};

// ── ADT Registry ──────────────────────────────────────────────────

/// An ADT definition for type checking.
struct AdtDef {
    /// Type parameter names in declaration order.
    type_params: Vec<SmolStr>,
    /// Variants with constructor info.
    variants: Vec<AdtVariant>,
}

/// A variant in an ADT for type checking.
struct AdtVariant {
    ctor_def_id: DefId,
    /// Constructor name for diagnostic messages.
    name: SmolStr,
    /// Pre-resolved field types (used by `check_pattern` for nested matching).
    #[allow(dead_code)]
    field_tys: Vec<Ty>,
}

// ── Diagnostic context ────────────────────────────────────────────

/// Additional context for type mismatch diagnostics.
///
/// Allows `unify_or_error` to attach secondary labels showing *why*
/// a particular type was expected.
#[derive(Clone, Copy)]
enum DiagnosticContext {
    /// No extra context.
    Simple,
    /// Function return type does not match body.
    ReturnType { fn_span: Span },
    /// Function argument type mismatch.
    Argument { param_index: usize, fn_span: Span },
    /// If/else branch types differ.
    IfElseBranch { then_span: Span },
    /// Match arm types differ.
    MatchArm { first_arm_span: Span },
    /// Assignment value type differs from binding type.
    Assignment { binding_span: Span },
}

#[derive(Clone, Copy)]
enum TryPosition {
    Statement,
    Return,
    Other,
}

// ── Context ────────────────────────────────────────────────────────

/// Accumulates state during HIR → THIR type checking.
pub(crate) struct TyCheckCtx {
    /// Maps each `DefId` to its type scheme (monomorphic or polymorphic).
    type_env: HashMap<DefId, TypeScheme>,
    /// Maps type `DefId` → ADT definition.
    adt_registry: HashMap<DefId, AdtDef>,
    /// Maps constructor `DefId` → parent type `DefId`.
    ctor_to_type: HashMap<DefId, DefId>,
    /// Maps type name → type `DefId` for annotation resolution.
    type_name_to_def_id: HashMap<SmolStr, DefId>,
    /// Functions whose return type was not explicitly annotated.
    unannotated_returns: HashSet<DefId>,
    /// Resolved FFI modules from Python imports.
    ffi_modules: HashMap<SmolStr, FfiModule>,
    /// Return type of the function currently being checked (for `try` validation).
    current_fn_return_ty: Option<Ty>,
    /// Module symbol table (cloned from HIR) for mutation rule lookups.
    module_symbols: SymbolTable,
    /// `DefId`s introduced in the current function/lambda scope.
    /// Used to detect assignment to captured variables inside lambdas.
    local_defs: HashSet<DefId>,
    /// Whether the current function being checked is `async`.
    /// Used to enforce that `await` only appears in async context (Issue 98).
    in_async_fn: bool,
    /// Synthetic symbol table used to allocate builtin collection type ids.
    builtin_types: SymbolTable,
    diagnostics: Vec<Diagnostic>,
    /// Hindley-Milner inference state.
    infer: InferCtx,
}

impl TyCheckCtx {
    pub(crate) fn new() -> Self {
        let mut ctx = Self {
            type_env: HashMap::new(),
            adt_registry: HashMap::new(),
            ctor_to_type: HashMap::new(),
            type_name_to_def_id: HashMap::new(),
            unannotated_returns: HashSet::new(),
            ffi_modules: HashMap::new(),
            current_fn_return_ty: None,
            module_symbols: SymbolTable::new(),
            local_defs: HashSet::new(),
            in_async_fn: false,
            builtin_types: SymbolTable::new(),
            diagnostics: Vec::new(),
            infer: InferCtx::new(),
        };
        ctx.register_builtin_collection_types();
        ctx
    }

    pub(crate) fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    fn push_diagnostic(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    fn register_builtin_collection_types(&mut self) {
        self.register_builtin_type("Option", &["a"]);
        self.register_builtin_type("List", &["a"]);
        self.register_builtin_type("Dict", &["k", "v"]);
        self.register_builtin_type("Tuple", &[]);
        self.register_builtin_type("Task", &["a"]);
    }

    fn register_builtin_type(&mut self, name: &str, type_params: &[&str]) {
        let def_id = self.builtin_types.alloc(DefData {
            name: SmolStr::from(name),
            kind: DefKind::Type,
            is_mutable: false,
            span: Span::dummy(),
        });
        self.type_name_to_def_id.insert(SmolStr::from(name), def_id);
        self.adt_registry.insert(
            def_id,
            AdtDef {
                type_params: type_params.iter().map(|param| SmolStr::from(*param)).collect(),
                variants: vec![],
            },
        );
    }

    /// Construct `Task(inner)` using the registered builtin `Task` type.
    fn make_task_ty(&self, inner: Ty) -> Ty {
        let &def_id =
            self.type_name_to_def_id.get("Task").expect("Task builtin type must be registered");
        Ty::Named { def_id, name: SmolStr::from("Task"), args: vec![inner] }
    }

    /// If `ty` is `Task(T)`, return `Some(T)`. Otherwise `None`.
    fn unwrap_task_ty(&self, ty: &Ty) -> Option<Ty> {
        let task_def_id = self.type_name_to_def_id.get("Task")?;
        match ty {
            Ty::Named { def_id, args, .. } if def_id == task_def_id && args.len() == 1 => {
                Some(args[0].clone())
            }
            _ => None,
        }
    }

    /// Register type schemes for built-in functions (e.g. `string_concat`, `println`).
    fn register_builtin_fn_types(&mut self, symbol_table: &SymbolTable) {
        for (def_id, data) in symbol_table.iter() {
            if data.kind != DefKind::Builtin {
                continue;
            }
            let scheme = match data.name.as_str() {
                "string_concat" => TypeScheme::mono(Ty::Function {
                    params: vec![Ty::Primitive(PrimTy::String), Ty::Primitive(PrimTy::String)],
                    ret: Box::new(Ty::Primitive(PrimTy::String)),
                }),
                "println" => {
                    let var = self.infer.fresh_var();
                    let Ty::Var(var_id) = var else { unreachable!() };
                    TypeScheme {
                        vars: vec![var_id],
                        ty: Ty::Function {
                            params: vec![Ty::Var(var_id)],
                            ret: Box::new(Ty::Primitive(PrimTy::None)),
                        },
                    }
                }
                // `list` is a builtin module namespace — field access resolves
                // individual list operations (map, filter, etc.) in the backend.
                "list" => TypeScheme::mono(Ty::FfiModule { module_name: SmolStr::from("list") }),
                _ => continue,
            };
            self.type_env.insert(def_id, scheme);
        }
    }

    /// Resolve an HIR type expression to a [`Ty`].
    fn resolve_type_expr(&mut self, te: &asatsuyu_hir::HirTypeExpr) -> Ty {
        self.resolve_type_expr_with_params(te, &HashMap::new())
    }

    /// Resolve an HIR type expression with a type parameter scope.
    fn resolve_type_expr_with_params(
        &mut self,
        te: &asatsuyu_hir::HirTypeExpr,
        type_params: &HashMap<SmolStr, Ty>,
    ) -> Ty {
        let name = te.name.as_str();

        // Check primitives first.
        match name {
            "Int" => return Ty::Primitive(PrimTy::Int),
            "Float" => return Ty::Primitive(PrimTy::Float),
            "String" => return Ty::Primitive(PrimTy::String),
            "Bool" => return Ty::Primitive(PrimTy::Bool),
            "None" => return Ty::Primitive(PrimTy::None),
            _ => {}
        }

        // Check type parameters.
        if let Some(ty) = type_params.get(te.name.as_str()) {
            return ty.clone();
        }

        // Look up in ADT registry by name.
        if let Some(&type_def_id) = self.type_name_to_def_id.get(te.name.as_str()) {
            let adt_param_count =
                self.adt_registry.get(&type_def_id).map_or(0, |a| a.type_params.len());
            let resolved_args: Vec<Ty> = te
                .args
                .iter()
                .map(|a| self.resolve_type_expr_with_params(a, type_params))
                .collect();
            // If no args given but type has params, use fresh vars.
            let args = if resolved_args.is_empty() && adt_param_count > 0 {
                (0..adt_param_count).map(|_| self.infer.fresh_var()).collect()
            } else {
                resolved_args
            };
            return Ty::Named { def_id: type_def_id, name: te.name.clone(), args };
        }

        // E0202: Unknown type annotation.
        self.push_diagnostic(
            Diagnostic::error(format!("unknown type `{}`", te.name), te.span)
                .with_code(DiagnosticCode::E0202)
                .with_label(te.span, "not found in this scope")
                .with_hint("built-in types: Int, Float, String, Bool, None"),
        );
        Ty::Error
    }

    /// Unify two types, emitting a diagnostic on failure.
    fn unify_or_error(
        &mut self,
        expected: &Ty,
        found: &Ty,
        span: Span,
        context: DiagnosticContext,
    ) {
        if let Err(err) = self.infer.unify(expected, found) {
            match err.kind {
                UnifyErrorKind::Mismatch { expected, found } => {
                    let exp = self.infer.resolve(&expected);
                    let fnd = self.infer.resolve(&found);
                    let mut diag = Diagnostic::error(
                        format!("type mismatch: expected `{exp}`, found `{fnd}`"),
                        span,
                    )
                    .with_code(DiagnosticCode::E0200)
                    .with_label(span, format!("expected `{exp}`, found `{fnd}`"));

                    match context {
                        DiagnosticContext::Simple => {}
                        DiagnosticContext::ReturnType { fn_span } => {
                            diag = diag.with_secondary_label(
                                fn_span,
                                format!("expected `{exp}` because of return type annotation"),
                            );
                        }
                        DiagnosticContext::Argument { param_index, fn_span } => {
                            diag = diag.with_secondary_label(
                                fn_span,
                                format!("parameter {} expects `{exp}`", param_index + 1,),
                            );
                        }
                        DiagnosticContext::IfElseBranch { then_span } => {
                            diag = diag.with_secondary_label(
                                then_span,
                                format!("expected `{exp}` because of this branch"),
                            );
                        }
                        DiagnosticContext::MatchArm { first_arm_span } => {
                            diag = diag.with_secondary_label(
                                first_arm_span,
                                format!("first arm has type `{exp}`"),
                            );
                        }
                        DiagnosticContext::Assignment { binding_span } => {
                            diag = diag
                                .with_code(DiagnosticCode::E0217)
                                .with_secondary_label(
                                    binding_span,
                                    format!("binding has type `{exp}`"),
                                )
                                .with_hint("assign a value with the same type as the binding")
                                .with_note("reassignment must preserve the original binding type");
                        }
                    }

                    self.push_diagnostic(diag);
                }
                UnifyErrorKind::InfiniteType { var, ty } => {
                    let resolved = self.infer.resolve(&ty);
                    self.push_diagnostic(
                        Diagnostic::error(
                            format!(
                                "infinite type: type variable `?{}` occurs in `{resolved}`",
                                var.0,
                            ),
                            span,
                        )
                        .with_code(DiagnosticCode::E0201)
                        .with_label(span, "this expression causes an infinite type")
                        .with_note("a type cannot contain itself recursively"),
                    );
                }
            }
        }
    }
}

// ── Pass 1: Collect signatures ─────────────────────────────────────

impl TyCheckCtx {
    pub(crate) fn collect_signatures_with_config(
        &mut self,
        module: &HirModule,
        ffi_config: &FfiResolverConfig,
    ) {
        // Register custom types and constructor signatures first,
        // so function signatures can reference ADT types.
        for ct in &module.custom_types {
            self.register_custom_type(ct, &module.symbol_table);
        }
        for fn_def in &module.functions {
            self.collect_fn_signature(fn_def);
        }

        // Register type schemes for built-in functions.
        self.register_builtin_fn_types(&module.symbol_table);

        // Register FFI module types for Python imports.
        let ffi_resolver = ChainResolver::with_config(ffi_config.clone());
        for import in &module.imports {
            if let HirImportKind::Python { module_name } = &import.kind {
                if let Some(ffi_module) = ffi_resolver.resolve(module_name.as_str()) {
                    let ty = Ty::FfiModule { module_name: module_name.clone() };
                    self.type_env.insert(import.def_id, TypeScheme::mono(ty));
                    self.ffi_modules.insert(module_name.clone(), ffi_module);
                } else {
                    self.push_diagnostic(
                        Diagnostic::error(
                            format!("unknown Python module `{module_name}`"),
                            import.span,
                        )
                        .with_code(DiagnosticCode::E0208)
                        .with_label(import.span, "not found in FFI registry"),
                    );
                }
            }
        }
    }

    /// Register a custom type in the ADT registry and create constructor type schemes.
    fn register_custom_type(
        &mut self,
        ct: &asatsuyu_hir::HirCustomType,
        symbol_table: &SymbolTable,
    ) {
        let type_name = symbol_table.get(ct.def_id).name.clone();

        // Allocate fresh type variables for each type parameter.
        let param_vars: Vec<(SmolStr, TyVarId)> = ct
            .type_params
            .iter()
            .map(|name| {
                let Ty::Var(var) = self.infer.fresh_var() else { unreachable!() };
                (name.clone(), var)
            })
            .collect();

        let type_param_scope: HashMap<SmolStr, Ty> =
            param_vars.iter().map(|(name, var_id)| (name.clone(), Ty::Var(*var_id))).collect();

        let quantified_vars: Vec<TyVarId> = param_vars.iter().map(|(_, v)| *v).collect();

        // The return type for all constructors: Named { type_def_id, [Var for each param] }
        let ret_ty = Ty::Named {
            def_id: ct.def_id,
            name: type_name.clone(),
            args: quantified_vars.iter().map(|v| Ty::Var(*v)).collect(),
        };

        let mut variants = Vec::new();

        for variant in &ct.variants {
            // Resolve field types in the ADT's type parameter context.
            let field_tys: Vec<Ty> = variant
                .fields
                .iter()
                .map(|f| self.resolve_type_expr_with_params(&f.type_expr, &type_param_scope))
                .collect();

            // Build the constructor's type scheme.
            let ctor_ty = if field_tys.is_empty() {
                // Nullary constructor: directly the ADT type.
                ret_ty.clone()
            } else {
                // Constructor with fields: function type.
                Ty::Function { params: field_tys.clone(), ret: Box::new(ret_ty.clone()) }
            };

            let scheme = TypeScheme { vars: quantified_vars.clone(), ty: ctor_ty };
            self.type_env.insert(variant.def_id, scheme);

            let ctor_name = symbol_table.get(variant.def_id).name.clone();
            self.ctor_to_type.insert(variant.def_id, ct.def_id);
            variants.push(AdtVariant { ctor_def_id: variant.def_id, name: ctor_name, field_tys });
        }

        self.adt_registry
            .insert(ct.def_id, AdtDef { type_params: ct.type_params.clone(), variants });
        self.type_name_to_def_id.insert(type_name, ct.def_id);
    }

    fn collect_fn_signature(&mut self, fn_def: &HirFnDef) {
        // Resolve parameter types.
        let param_tys: Vec<Ty> = fn_def
            .params
            .iter()
            .map(|p| {
                let ty = match &p.type_ann {
                    Some(te) => self.resolve_type_expr(te),
                    None => self.infer.fresh_var(),
                };
                self.type_env.insert(p.def_id, TypeScheme::mono(ty.clone()));
                ty
            })
            .collect();

        // Resolve return type: annotated or provisional None.
        let inner_ret_ty = if let Some(te) = &fn_def.return_type {
            self.resolve_type_expr(te)
        } else {
            self.unannotated_returns.insert(fn_def.def_id);
            Ty::Primitive(PrimTy::None) // provisional; replaced after body check
        };

        // async fn wraps the return type in Task(T).
        let ret_ty = if fn_def.is_async { self.make_task_ty(inner_ret_ty) } else { inner_ret_ty };

        let fn_ty = Ty::Function { params: param_tys, ret: Box::new(ret_ty) };
        self.type_env.insert(fn_def.def_id, TypeScheme::mono(fn_ty));
    }
}

// ── Pass 2: Check bodies ───────────────────────────────────────────

impl TyCheckCtx {
    /// Type-check the entire module, producing THIR.
    pub(crate) fn check_module(&mut self, module: &HirModule) -> ThirModule {
        // Store module symbol table for mutation rule lookups.
        self.module_symbols = clone_symbol_table(&module.symbol_table);
        let functions = module.functions.iter().map(|f| self.check_fn_def(f)).collect();
        let custom_types = module.custom_types.clone();
        let imports = module.imports.clone();
        let symbol_table = clone_symbol_table(&module.symbol_table);
        let ffi_modules = self.ffi_modules.clone();
        ThirModule {
            functions,
            custom_types,
            imports,
            symbol_table,
            ffi_modules,
            span: module.span,
        }
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

        // Extract the declared return type before checking the body (needed for `try` validation).
        let fn_scheme = self.type_env.get(&fn_def.def_id).cloned();
        let fn_ty = fn_scheme.map_or(Ty::Error, |s| s.ty);
        let declared_ret = match &fn_ty {
            Ty::Function { ret, .. } => *ret.clone(),
            _ => Ty::Error,
        };

        // For async functions, the declared return is Task(T) but the body produces T.
        // Unwrap to get the type the body should be checked against.
        let body_check_ty = if fn_def.is_async {
            self.unwrap_task_ty(&declared_ret).unwrap_or_else(|| declared_ret.clone())
        } else {
            declared_ret.clone()
        };

        // Set current function return type context for `try` expression validation.
        self.current_fn_return_ty = Some(body_check_ty.clone());

        // Set async context for `await` legality checking (Issue 98).
        let saved_in_async = self.in_async_fn;
        self.in_async_fn = fn_def.is_async;

        // Initialize local_defs with parameter DefIds for lambda capture tracking.
        let saved_local_defs = std::mem::take(&mut self.local_defs);
        for p in &fn_def.params {
            self.local_defs.insert(p.def_id);
        }

        // Check the body.
        let body = self.check_expr(&fn_def.body);
        let body_ty = self.infer.resolve(body.ty());

        // Restore local_defs, async context, and return type context.
        self.local_defs = saved_local_defs;
        self.in_async_fn = saved_in_async;
        self.current_fn_return_ty = None;

        // `try` lowering is currently only implemented for statement position,
        // `let x = try expr`, and direct final return expressions.
        self.validate_try_positions(&body, TryPosition::Return);

        // Determine the actual return type.
        let is_unannotated = self.unannotated_returns.contains(&fn_def.def_id);
        let return_ty = if is_unannotated {
            // Infer return type from body.
            if fn_def.is_async { self.make_task_ty(body_ty.clone()) } else { body_ty.clone() }
        } else {
            // Check body type against the expected inner type (T for async, declared for sync).
            self.unify_or_error(
                &body_check_ty,
                &body_ty,
                fn_def.body.span(),
                DiagnosticContext::ReturnType { fn_span: fn_def.span },
            );
            declared_ret
        };

        // ThirFnDef.return_ty is the *inner* type (what the user annotated / body produces).
        // For async fn this is T (used by backend for `async def f() -> T:`).
        // ThirFnDef.ty is the full function type including Task(T) wrapper.
        let inner_return_ty = if fn_def.is_async {
            self.unwrap_task_ty(&return_ty).unwrap_or_else(|| return_ty.clone())
        } else {
            return_ty.clone()
        };

        // Build the final function type with the resolved return type.
        let param_tys: Vec<Ty> = params.iter().map(|p| p.ty.clone()).collect();
        let final_fn_ty = Ty::Function { params: param_tys, ret: Box::new(return_ty) };

        ThirFnDef {
            def_id: fn_def.def_id,
            visibility: fn_def.visibility,
            is_async: fn_def.is_async,
            params,
            return_ty: inner_return_ty,
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

            HirExpr::Let { binding, value, is_mutable, span } => {
                let checked_value = self.check_expr(value);
                let value_ty = self.infer.resolve(checked_value.ty());
                let env_fvs = self.env_free_vars();
                let scheme = self.infer.generalize(&value_ty, &env_fvs);
                self.type_env.insert(*binding, scheme);
                self.local_defs.insert(*binding);
                ThirExpr::Let {
                    binding: *binding,
                    value: Box::new(checked_value),
                    is_mutable: *is_mutable,
                    ty: Ty::Primitive(PrimTy::None),
                    span: *span,
                }
            }

            HirExpr::Assign { target, value, span } => self.check_assign(*target, value, *span),

            HirExpr::Lambda { params, return_type, body, span } => {
                self.check_lambda(params, return_type.as_ref(), body, *span)
            }

            HirExpr::FieldAccess { receiver, field, span } => {
                self.check_field_access(receiver, field, *span)
            }

            HirExpr::Try { expr, span } => {
                let checked_inner = self.check_expr(expr);
                let inner_ty = self.infer.resolve(checked_inner.ty());

                // Validate: enclosing function must return a Result type.
                if let Some(ref ret_ty) = self.current_fn_return_ty {
                    let resolved_ret = self.infer.resolve(ret_ty);
                    if !matches!(&resolved_ret, Ty::Named { name, args, .. } if name.as_str() == "Result" && args.len() == 2)
                        && resolved_ret != Ty::Error
                    {
                        self.push_diagnostic(
                            Diagnostic::error(
                                "`try` requires enclosing function to return `Result(T, E)`",
                                *span,
                            )
                            .with_code(DiagnosticCode::E0212)
                            .with_label(*span, "`try` used here")
                            .with_hint("annotate the function with `-> Result(T, E)`"),
                        );
                    }
                }

                ThirExpr::Try { expr: Box::new(checked_inner), ty: inner_ty, span: *span }
            }

            HirExpr::Await { expr, span } => self.check_await(expr, *span),

            HirExpr::List { elements, span } => self.check_list_expr(elements, *span),
        }
    }

    fn check_await(&mut self, expr: &HirExpr, span: Span) -> ThirExpr {
        // E0220: await outside async context (Issue 98).
        if !self.in_async_fn {
            self.push_diagnostic(
                Diagnostic::error("`await` is only allowed inside `async fn`", span)
                    .with_code(DiagnosticCode::E0220)
                    .with_label(span, "`await` used outside async context")
                    .with_hint("move this into an `async fn`, or call the async function without `await` to get a `Task(T)` value"),
            );
            let checked_inner = self.check_expr(expr);
            return ThirExpr::Await { expr: Box::new(checked_inner), ty: Ty::Error, span };
        }

        let checked_inner = self.check_expr(expr);
        let inner_ty = self.infer.resolve(checked_inner.ty());

        let result_ty = if let Some(t) = self.unwrap_task_ty(&inner_ty) {
            t
        } else if matches!(inner_ty, Ty::Var(_)) {
            let fresh = self.infer.fresh_var();
            let task_ty = self.make_task_ty(fresh.clone());
            self.unify_or_error(&task_ty, &inner_ty, span, DiagnosticContext::Simple);
            self.infer.resolve(&fresh)
        } else if matches!(inner_ty, Ty::Error) {
            Ty::Error
        } else {
            self.push_diagnostic(
                Diagnostic::error(format!("cannot `await` a value of type `{inner_ty}`"), span)
                    .with_code(DiagnosticCode::E0219)
                    .with_label(span, format!("expected `Task(T)`, found `{inner_ty}`"))
                    .with_hint("only `Task(T)` values (from async functions) can be awaited"),
            );
            Ty::Error
        };

        ThirExpr::Await { expr: Box::new(checked_inner), ty: result_ty, span }
    }

    fn check_list_expr(&mut self, elements: &[HirExpr], span: Span) -> ThirExpr {
        let mut checked_elements = Vec::new();
        let element_ty = self.infer.fresh_var();

        for element in elements {
            let checked = self.check_expr(element);
            let curr_ty = self.infer.resolve(checked.ty());
            self.unify_or_error(&element_ty, &curr_ty, element.span(), DiagnosticContext::Simple);
            checked_elements.push(checked);
        }

        let list_def_id = self
            .type_name_to_def_id
            .get("List")
            .copied()
            .expect("List builtin type must be registered");
        let list_ty =
            Ty::Named { def_id: list_def_id, name: SmolStr::from("List"), args: vec![element_ty] };

        ThirExpr::List { elements: checked_elements, ty: list_ty, span }
    }

    fn validate_try_positions(&mut self, expr: &ThirExpr, position: TryPosition) {
        match expr {
            ThirExpr::Try { expr: inner, span, .. } => {
                if matches!(position, TryPosition::Other) {
                    self.push_diagnostic(
                        Diagnostic::error(
                            "`try` is only supported as `let x = try expr`, bare `try expr`, or a function's final expression",
                            *span,
                        )
                        .with_code(DiagnosticCode::E0213)
                        .with_label(*span, "`try` used in an unsupported expression position")
                        .with_hint("move `try` to a standalone statement or return position"),
                    );
                }
                self.validate_try_positions(inner, TryPosition::Other);
            }
            ThirExpr::Block { exprs, .. } => {
                if let Some((last, statements)) = exprs.split_last() {
                    for stmt in statements {
                        self.validate_try_positions(stmt, TryPosition::Statement);
                    }
                    self.validate_try_positions(last, TryPosition::Return);
                }
            }
            ThirExpr::Let { value, .. } => {
                if matches!(position, TryPosition::Statement)
                    && matches!(value.as_ref(), ThirExpr::Try { .. })
                {
                    if let ThirExpr::Try { expr: inner, .. } = value.as_ref() {
                        self.validate_try_positions(inner, TryPosition::Other);
                    }
                } else {
                    self.validate_try_positions(value, TryPosition::Other);
                }
            }
            ThirExpr::Match { subject, arms, .. } => {
                self.validate_try_positions(subject, TryPosition::Other);
                for arm in arms {
                    self.validate_try_positions(
                        &arm.body,
                        if matches!(position, TryPosition::Return) {
                            TryPosition::Return
                        } else {
                            TryPosition::Statement
                        },
                    );
                }
            }
            ThirExpr::If { condition, then_body, else_body, .. } => {
                self.validate_try_positions(condition, TryPosition::Other);
                self.validate_try_positions(then_body, TryPosition::Other);
                if let Some(else_expr) = else_body {
                    self.validate_try_positions(else_expr, TryPosition::Other);
                }
            }
            ThirExpr::Call { func, args, .. } => {
                self.validate_try_positions(func, TryPosition::Other);
                for arg in args {
                    self.validate_try_positions(arg, TryPosition::Other);
                }
            }
            ThirExpr::BinaryOp { lhs, rhs, .. } => {
                self.validate_try_positions(lhs, TryPosition::Other);
                self.validate_try_positions(rhs, TryPosition::Other);
            }
            ThirExpr::UnaryOp { expr, .. } | ThirExpr::FieldAccess { receiver: expr, .. } => {
                self.validate_try_positions(expr, TryPosition::Other);
            }
            ThirExpr::List { elements, .. } => {
                for element in elements {
                    self.validate_try_positions(element, TryPosition::Other);
                }
            }
            ThirExpr::Lambda { body, .. } => {
                self.validate_try_positions(body, TryPosition::Other);
            }
            ThirExpr::Assign { value, .. } | ThirExpr::Await { expr: value, .. } => {
                self.validate_try_positions(value, TryPosition::Other);
            }
            ThirExpr::Literal(_) | ThirExpr::Var { .. } => {}
        }
    }

    // ── FFI arity ──────────────────────────────────────────────────

    /// For an FFI call, return the minimum number of required arguments.
    ///
    /// Returns `None` for non-FFI calls (standard arity check applies).
    fn ffi_min_arity(&self, func: &HirExpr) -> Option<usize> {
        let HirExpr::FieldAccess { receiver, field, .. } = func else {
            return None;
        };
        // Need the receiver's type to determine the FFI module/class.
        // We check the HIR directly — resolve the receiver DefId if it's a Var.
        let (module_name, symbol_name) = self.ffi_call_target(receiver, field)?;
        let ffi_module = self.ffi_modules.get(&module_name)?;
        let symbol = ffi_module.symbols.iter().find(|s| s.name == *symbol_name)?;
        match &symbol.kind {
            FfiSymbolKind::Function(sig) => {
                Some(sig.params.iter().filter(|p| !p.has_default).count())
            }
            FfiSymbolKind::Class(cls) => cls
                .constructor
                .as_ref()
                .map(|sig| sig.params.iter().filter(|p| !p.has_default).count()),
            FfiSymbolKind::Constant(_) => None,
        }
    }

    /// Determine the FFI module name and symbol name for a field access call.
    fn ffi_call_target<'a>(
        &self,
        receiver: &'a HirExpr,
        field: &'a SmolStr,
    ) -> Option<(SmolStr, &'a SmolStr)> {
        // Direct module field access: pathlib.Path or os.getcwd
        if let HirExpr::Var(def_id, _) = receiver
            && let Some(scheme) = self.type_env.get(def_id)
            && let Ty::FfiModule { module_name } = &scheme.ty
        {
            return Some((module_name.clone(), field));
        }
        // Instance method call: path.exists — need to find the module/class
        // from the variable's type, which is not yet resolved at this HIR level.
        // For now, instance method min_arity is handled by letting the function
        // type include all params (methods already omit self in builtins.rs).
        // A more thorough approach would trace the receiver's type, but the
        // method signatures in builtins.rs mostly have 0 required params.
        None
    }

    // ── Field access ───────────────────────────────────────────────

    fn check_field_access(&mut self, receiver: &HirExpr, field: &SmolStr, span: Span) -> ThirExpr {
        let checked_receiver = self.check_expr(receiver);
        let receiver_ty = self.infer.resolve(checked_receiver.ty());

        let ty = match &receiver_ty {
            Ty::FfiModule { module_name } => {
                self.resolve_ffi_module_field(module_name, field, span)
            }
            Ty::FfiInstance { module, class } => {
                self.resolve_ffi_instance_field(module, class, field, span)
            }
            Ty::Opaque { .. } => {
                self.push_diagnostic(
                    Diagnostic::error(
                        format!("cannot access field `{field}` on opaque type `{receiver_ty}`"),
                        span,
                    )
                    .with_code(DiagnosticCode::E0209)
                    .with_label(span, "opaque types do not allow field access")
                    .with_note(
                        "opaque values can only be passed to foreign calls that accept the same type",
                    ),
                );
                Ty::Error
            }
            Ty::Error => Ty::Error,
            _ => {
                self.push_diagnostic(
                    Diagnostic::error(
                        format!("type `{receiver_ty}` does not support field access"),
                        span,
                    )
                    .with_code(DiagnosticCode::E0210)
                    .with_label(span, "field access not supported"),
                );
                Ty::Error
            }
        };

        ThirExpr::FieldAccess {
            receiver: Box::new(checked_receiver),
            field: field.clone(),
            ty,
            span,
        }
    }

    fn resolve_ffi_module_field(
        &mut self,
        module_name: &SmolStr,
        field: &SmolStr,
        span: Span,
    ) -> Ty {
        if module_name == "list" {
            return self.resolve_builtin_list_field(field, span);
        }

        let Some(ffi_module) = self.ffi_modules.get(module_name).cloned() else {
            return Ty::Error;
        };

        let Some(symbol) = ffi_module.symbols.iter().find(|s| s.name == *field) else {
            self.push_diagnostic(
                Diagnostic::error(format!("module `{module_name}` has no symbol `{field}`"), span)
                    .with_code(DiagnosticCode::E0211)
                    .with_label(span, "not found in this module"),
            );
            return Ty::Error;
        };

        // If symbol is Unsafe trust, return Opaque
        if symbol.trust_level == Some(FfiTrustLevel::Unsafe) {
            return Ty::Opaque { module: module_name.clone(), symbol: field.clone() };
        }

        match &symbol.kind {
            FfiSymbolKind::Function(sig) => self.ffi_signature_to_ty(sig),
            FfiSymbolKind::Class(cls) => self.ffi_class_constructor_to_ty(cls, module_name),
            FfiSymbolKind::Constant(ffi_ty) => self.ffi_type_to_ty(ffi_ty),
        }
    }

    fn resolve_builtin_list_field(&mut self, field: &SmolStr, span: Span) -> Ty {
        let list_def_id = self.type_name_to_def_id.get("List").copied().expect("builtin List type");
        let option_def_id =
            self.type_name_to_def_id.get("Option").copied().expect("builtin Option type");
        let elem_ty = self.infer.fresh_var();
        let ret_ty = self.infer.fresh_var();
        let list_of_elem = Ty::Named {
            def_id: list_def_id,
            name: SmolStr::from("List"),
            args: vec![elem_ty.clone()],
        };
        let list_of_elem_for_head = Ty::Named {
            def_id: list_def_id,
            name: SmolStr::from("List"),
            args: vec![elem_ty.clone()],
        };
        let list_of_elem_for_rest = Ty::Named {
            def_id: list_def_id,
            name: SmolStr::from("List"),
            args: vec![elem_ty.clone()],
        };
        let option_of_elem = Ty::Named {
            def_id: option_def_id,
            name: SmolStr::from("Option"),
            args: vec![elem_ty.clone()],
        };
        let option_of_list = Ty::Named {
            def_id: option_def_id,
            name: SmolStr::from("Option"),
            args: vec![list_of_elem.clone()],
        };

        match field.as_str() {
            "map" => Ty::Function {
                params: vec![
                    list_of_elem.clone(),
                    Ty::Function { params: vec![elem_ty.clone()], ret: Box::new(ret_ty.clone()) },
                ],
                ret: Box::new(Ty::Named {
                    def_id: list_def_id,
                    name: SmolStr::from("List"),
                    args: vec![ret_ty],
                }),
            },
            "filter" => Ty::Function {
                params: vec![
                    list_of_elem.clone(),
                    Ty::Function {
                        params: vec![elem_ty.clone()],
                        ret: Box::new(Ty::Primitive(PrimTy::Bool)),
                    },
                ],
                ret: Box::new(list_of_elem),
            },
            "fold" => Ty::Function {
                params: vec![
                    list_of_elem.clone(),
                    ret_ty.clone(),
                    Ty::Function {
                        params: vec![ret_ty.clone(), elem_ty.clone()],
                        ret: Box::new(ret_ty.clone()),
                    },
                ],
                ret: Box::new(ret_ty),
            },
            "length" => Ty::Function {
                params: vec![list_of_elem],
                ret: Box::new(Ty::Primitive(PrimTy::Int)),
            },
            "reverse" => {
                Ty::Function { params: vec![list_of_elem.clone()], ret: Box::new(list_of_elem) }
            }
            "append" => Ty::Function {
                params: vec![list_of_elem.clone(), list_of_elem.clone()],
                ret: Box::new(list_of_elem),
            },
            "is_empty" => Ty::Function {
                params: vec![list_of_elem.clone()],
                ret: Box::new(Ty::Primitive(PrimTy::Bool)),
            },
            "contains" => Ty::Function {
                params: vec![list_of_elem, elem_ty],
                ret: Box::new(Ty::Primitive(PrimTy::Bool)),
            },
            "head" => {
                Ty::Function { params: vec![list_of_elem_for_head], ret: Box::new(option_of_elem) }
            }
            "rest" => {
                Ty::Function { params: vec![list_of_elem_for_rest], ret: Box::new(option_of_list) }
            }
            _ => {
                self.push_diagnostic(
                    Diagnostic::error(format!("module `list` has no symbol `{field}`"), span)
                        .with_code(DiagnosticCode::E0211)
                        .with_label(span, "not found in this module"),
                );
                Ty::Error
            }
        }
    }

    fn resolve_ffi_instance_field(
        &mut self,
        module: &SmolStr,
        class: &SmolStr,
        field: &SmolStr,
        span: Span,
    ) -> Ty {
        let Some(ffi_module) = self.ffi_modules.get(module).cloned() else {
            return Ty::Error;
        };

        let Some(cls) = ffi_module.symbols.iter().find_map(|s| {
            if s.name == *class
                && let FfiSymbolKind::Class(cls) = &s.kind
            {
                Some(cls.clone())
            } else {
                None
            }
        }) else {
            return Ty::Error;
        };

        // Check properties first
        if let Some((_, prop_ty)) = cls.properties.iter().find(|(name, _)| name == field) {
            return self.ffi_type_to_ty(prop_ty);
        }

        // Then check methods
        if let Some((_, method_sig)) = cls.methods.iter().find(|(name, _)| name == field) {
            return self.ffi_signature_to_ty(method_sig);
        }

        self.push_diagnostic(
            Diagnostic::error(format!("`{module}.{class}` has no member `{field}`"), span)
                .with_code(DiagnosticCode::E0211)
                .with_label(span, "not found"),
        );
        Ty::Error
    }

    // ── Assign ─────────────────────────────────────────────────────

    fn check_assign(&mut self, target: DefId, value: &HirExpr, span: Span) -> ThirExpr {
        let checked_value = self.check_expr(value);
        let target_def = self.module_symbols.get(target);
        let target_name = target_def.name.clone();
        let target_span = target_def.span;
        let target_kind = target_def.kind;
        let target_mutable = target_def.is_mutable;

        // Rule 1: Cannot reassign function parameters.
        if target_kind == DefKind::Parameter {
            self.push_diagnostic(
                Diagnostic::error(format!("cannot assign to parameter `{target_name}`"), span)
                    .with_code(DiagnosticCode::E0216)
                    .with_label(span, "assignment to parameter")
                    .with_secondary_label(target_span, "parameter defined here")
                    .with_hint(format!(
                        "consider using a local binding: `let mut {target_name} = {target_name}`"
                    ))
                    .with_note("parameters cannot be reassigned in Asatsuyu"),
            );
        }
        // Rule 2: Cannot assign to immutable binding.
        else if !target_mutable {
            self.push_diagnostic(
                Diagnostic::error(
                    format!("cannot assign to immutable binding `{target_name}`"),
                    span,
                )
                .with_code(DiagnosticCode::E0215)
                .with_label(span, "assignment to immutable binding")
                .with_secondary_label(target_span, "defined as immutable here")
                .with_hint(format!("make this binding mutable: `let mut {target_name}`"))
                .with_note("only `let mut` bindings may be reassigned"),
            );
        }

        // Rule 3: Cannot assign to captured variable in lambda.
        if !self.local_defs.contains(&target) {
            self.push_diagnostic(
                Diagnostic::error(
                    format!("cannot assign to `{target_name}` captured from outer scope"),
                    span,
                )
                .with_code(DiagnosticCode::E0218)
                .with_label(span, "assignment to captured variable")
                .with_secondary_label(target_span, "defined in outer scope")
                .with_hint("introduce a new `let mut` binding inside the lambda instead")
                .with_note("closures cannot mutate variables captured from an enclosing scope"),
            );
        }

        // Rule 4: Assignment type must match binding type.
        if let Some(scheme) = self.type_env.get(&target) {
            let expected = self.infer.instantiate(scheme);
            let found = self.infer.resolve(checked_value.ty());
            self.unify_or_error(
                &expected,
                &found,
                span,
                DiagnosticContext::Assignment { binding_span: target_span },
            );
        }

        ThirExpr::Assign {
            target,
            value: Box::new(checked_value),
            ty: Ty::Primitive(PrimTy::None),
            span,
        }
    }

    // ── Lambda ─────────────────────────────────────────────────────

    fn check_lambda(
        &mut self,
        params: &[asatsuyu_hir::HirParam],
        return_type: Option<&asatsuyu_hir::HirTypeExpr>,
        body: &HirExpr,
        span: Span,
    ) -> ThirExpr {
        // Assign types to parameters: annotated → resolve, unannotated → fresh var.
        // Track param DefIds so we can remove them from env after checking.
        let param_def_ids: Vec<DefId> = params.iter().map(|p| p.def_id).collect();
        let thir_params: Vec<ThirParam> = params
            .iter()
            .map(|p| {
                let ty = match &p.type_ann {
                    Some(te) => self.resolve_type_expr(te),
                    None => self.infer.fresh_var(),
                };
                self.type_env.insert(p.def_id, TypeScheme::mono(ty.clone()));
                ThirParam { def_id: p.def_id, ty, span: p.span }
            })
            .collect();

        // Save and replace local_defs for lambda scope boundary.
        // Variables from outer scope are not in the new set, so assignments
        // to them will be detected as captured-variable mutations.
        let saved_local_defs = std::mem::take(&mut self.local_defs);
        for p in params {
            self.local_defs.insert(p.def_id);
        }

        // Lambdas are never async — reset async context so `await` inside is an error.
        let saved_in_async = self.in_async_fn;
        self.in_async_fn = false;

        let checked_body = self.check_expr(body);
        let body_ty = self.infer.resolve(checked_body.ty());

        // Restore outer scope's local_defs and async context.
        self.local_defs = saved_local_defs;
        self.in_async_fn = saved_in_async;

        // Remove lambda params from type_env to prevent them from polluting
        // the environment during generalization of let-bound values.
        for def_id in &param_def_ids {
            self.type_env.remove(def_id);
        }

        let ret_ty = if let Some(ret_te) = return_type {
            let declared = self.resolve_type_expr(ret_te);
            self.unify_or_error(
                &declared,
                &body_ty,
                body.span(),
                DiagnosticContext::ReturnType { fn_span: span },
            );
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

        // Compute minimum arity for FFI calls with default parameters.
        let min_arity = self.ffi_min_arity(func);

        match func_ty {
            Ty::Function { params, ret } => {
                let max_arity = params.len();
                let effective_min = min_arity.unwrap_or(max_arity);

                if args.len() >= effective_min && args.len() <= max_arity {
                    // Unify provided arguments with their corresponding param types.
                    for (i, (arg, param_ty)) in checked_args.iter().zip(params.iter()).enumerate() {
                        let arg_ty = self.infer.resolve(arg.ty());
                        self.unify_or_error(
                            param_ty,
                            &arg_ty,
                            arg.span(),
                            DiagnosticContext::Argument { param_index: i, fn_span: func.span() },
                        );
                    }
                } else {
                    // E0203: Argument count mismatch.
                    let arity_msg = if effective_min == max_arity {
                        format!("{max_arity}")
                    } else {
                        format!("{effective_min}..{max_arity}")
                    };
                    self.push_diagnostic(
                        Diagnostic::error(
                            format!(
                                "function expects {arity_msg} argument(s), but {} were given",
                                args.len(),
                            ),
                            span,
                        )
                        .with_code(DiagnosticCode::E0203)
                        .with_label(span, format!("{} argument(s) given here", args.len()))
                        .with_secondary_label(
                            func.span(),
                            format!("function expects {arity_msg} parameter(s)"),
                        ),
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
                // E0204: Expected function, found non-callable type.
                self.push_diagnostic(
                    Diagnostic::error(format!("expected function, found `{func_ty}`"), func.span())
                        .with_code(DiagnosticCode::E0204)
                        .with_label(func.span(), format!("this has type `{func_ty}`"))
                        .with_note("only functions can be called"),
                );
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
                self.unify_or_error(&lhs_ty, &rhs_ty, span, DiagnosticContext::Simple);
                let unified = self.infer.resolve(&lhs_ty);
                if !is_numeric(&unified) && unified != Ty::Error {
                    // E0205: Arithmetic operator requires numeric type.
                    self.push_diagnostic(
                        Diagnostic::error(
                            format!(
                                "arithmetic operator requires numeric type, found `{unified}`",
                            ),
                            span,
                        )
                        .with_code(DiagnosticCode::E0205)
                        .with_label(span, format!("operands have type `{unified}`"))
                        .with_note("arithmetic operators work with Int and Float"),
                    );
                    Ty::Error
                } else {
                    unified
                }
            }
            // Comparison: both must be same type, result is Bool.
            BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::LtEq | BinOp::Gt | BinOp::GtEq => {
                self.unify_or_error(&lhs_ty, &rhs_ty, span, DiagnosticContext::Simple);
                Ty::Primitive(PrimTy::Bool)
            }
            // Logical: both must be Bool.
            BinOp::And | BinOp::Or => {
                self.unify_or_error(
                    &Ty::Primitive(PrimTy::Bool),
                    &lhs_ty,
                    checked_lhs.span(),
                    DiagnosticContext::Simple,
                );
                self.unify_or_error(
                    &Ty::Primitive(PrimTy::Bool),
                    &rhs_ty,
                    checked_rhs.span(),
                    DiagnosticContext::Simple,
                );
                Ty::Primitive(PrimTy::Bool)
            }
            // StringConcat: desugared in HIR, but handle defensively.
            BinOp::StringConcat => {
                self.unify_or_error(
                    &Ty::Primitive(PrimTy::String),
                    &lhs_ty,
                    checked_lhs.span(),
                    DiagnosticContext::Simple,
                );
                self.unify_or_error(
                    &Ty::Primitive(PrimTy::String),
                    &rhs_ty,
                    checked_rhs.span(),
                    DiagnosticContext::Simple,
                );
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
                    // E0206: Negation requires numeric type.
                    self.push_diagnostic(
                        Diagnostic::error(
                            format!("negation requires numeric type, found `{expr_ty}`"),
                            span,
                        )
                        .with_code(DiagnosticCode::E0206)
                        .with_label(span, format!("this has type `{expr_ty}`"))
                        .with_note("negation works with Int and Float"),
                    );
                    Ty::Error
                } else {
                    expr_ty
                }
            }
            UnOp::Not => {
                self.unify_or_error(
                    &Ty::Primitive(PrimTy::Bool),
                    &expr_ty,
                    span,
                    DiagnosticContext::Simple,
                );
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
        self.unify_or_error(
            &Ty::Primitive(PrimTy::Bool),
            &cond_ty,
            checked_cond.span(),
            DiagnosticContext::Simple,
        );

        let checked_then = self.check_expr(then_body);
        let then_ty = self.infer.resolve(checked_then.ty());

        let (checked_else, ty) = if let Some(else_expr) = else_body {
            let checked = self.check_expr(else_expr);
            let else_ty = self.infer.resolve(checked.ty());
            self.unify_or_error(
                &then_ty,
                &else_ty,
                span,
                DiagnosticContext::IfElseBranch { then_span: then_body.span() },
            );
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
        let subject_ty = self.infer.resolve(checked_subject.ty());

        // Reject match on Opaque types — they cannot be destructured.
        if matches!(&subject_ty, Ty::Opaque { .. }) {
            self.push_diagnostic(
                Diagnostic::error(
                    format!("cannot match on opaque type `{subject_ty}`"),
                    span,
                )
                .with_code(DiagnosticCode::E0214)
                .with_label(span, "opaque types cannot be destructured")
                .with_note(
                    "opaque values can only be passed to foreign calls that accept the same type",
                ),
            );
        }

        let mut checked_arms = Vec::with_capacity(arms.len());
        let mut result_ty: Option<Ty> = None;
        let mut first_arm_span: Option<Span> = None;

        for arm in arms {
            let checked_pattern = self.check_pattern(&arm.pattern, &subject_ty);
            let checked_body = self.check_expr(&arm.body);
            let arm_ty = self.infer.resolve(checked_body.ty());

            if let Some(ref prev_ty) = result_ty {
                self.unify_or_error(
                    prev_ty,
                    &arm_ty,
                    arm.span,
                    DiagnosticContext::MatchArm {
                        first_arm_span: first_arm_span.unwrap_or(arm.span),
                    },
                );
            } else {
                result_ty = Some(arm_ty);
                first_arm_span = Some(arm.span);
            }

            checked_arms.push(ThirMatchArm {
                pattern: checked_pattern,
                body: checked_body,
                span: arm.span,
            });
        }

        self.check_match_coverage(&subject_ty, &checked_arms, span);

        let ty = result_ty.map_or(Ty::Primitive(PrimTy::None), |t| self.infer.resolve(&t));
        ThirExpr::Match { subject: Box::new(checked_subject), arms: checked_arms, ty, span }
    }

    // ── Pattern type checking ─────────────────────────────────────

    fn check_pattern(
        &mut self,
        pattern: &asatsuyu_hir::HirPattern,
        expected_ty: &Ty,
    ) -> ThirPattern {
        match pattern {
            asatsuyu_hir::HirPattern::Wildcard(span) => ThirPattern::Wildcard(*span),

            asatsuyu_hir::HirPattern::Variable(def_id, span) => {
                let ty = self.infer.resolve(expected_ty);
                self.type_env.insert(*def_id, TypeScheme::mono(ty.clone()));
                ThirPattern::Variable { def_id: *def_id, ty, span: *span }
            }

            asatsuyu_hir::HirPattern::Literal(lit) => {
                let lit_ty = match lit.kind {
                    LiteralKind::Int => Ty::Primitive(PrimTy::Int),
                    LiteralKind::Float => Ty::Primitive(PrimTy::Float),
                    LiteralKind::String => Ty::Primitive(PrimTy::String),
                    LiteralKind::Bool => Ty::Primitive(PrimTy::Bool),
                };
                self.unify_or_error(expected_ty, &lit_ty, lit.span, DiagnosticContext::Simple);
                ThirPattern::Literal(ThirLiteral {
                    kind: lit.kind,
                    value: lit.value.clone(),
                    ty: lit_ty,
                    span: lit.span,
                })
            }

            asatsuyu_hir::HirPattern::Constructor { def_id, fields, span } => {
                let Some(scheme) = self.type_env.get(def_id) else {
                    // E0302: Unknown constructor in pattern.
                    self.push_diagnostic(
                        Diagnostic::error("unknown constructor in pattern", *span)
                            .with_code(DiagnosticCode::E0302)
                            .with_label(*span, "not a known constructor"),
                    );
                    return ThirPattern::Wildcard(*span);
                };
                let ctor_ty = self.infer.instantiate(scheme);

                let (field_tys, ret_ty) = match &ctor_ty {
                    Ty::Function { params, ret } => (params.clone(), *ret.clone()),
                    ty @ Ty::Named { .. } => (vec![], ty.clone()),
                    _ => {
                        // E0302: Expected constructor type in pattern.
                        self.push_diagnostic(
                            Diagnostic::error("expected constructor type in pattern", *span)
                                .with_code(DiagnosticCode::E0302)
                                .with_label(*span, "not a constructor"),
                        );
                        return ThirPattern::Wildcard(*span);
                    }
                };

                self.unify_or_error(expected_ty, &ret_ty, *span, DiagnosticContext::Simple);

                if fields.len() != field_tys.len() {
                    // E0303: Constructor pattern field count mismatch.
                    self.push_diagnostic(
                        Diagnostic::error(
                            format!(
                                "constructor pattern expects {} field(s), but {} were given",
                                field_tys.len(),
                                fields.len(),
                            ),
                            *span,
                        )
                        .with_code(DiagnosticCode::E0303)
                        .with_label(*span, format!("{} field(s) here", fields.len())),
                    );
                }

                let checked_fields: Vec<ThirPattern> = fields
                    .iter()
                    .zip(field_tys.iter().chain(std::iter::repeat(&Ty::Error)))
                    .map(|(sub_pat, field_ty)| self.check_pattern(sub_pat, field_ty))
                    .collect();

                let resolved_ty = self.infer.resolve(&ret_ty);
                ThirPattern::Constructor {
                    def_id: *def_id,
                    fields: checked_fields,
                    ty: resolved_ty,
                    span: *span,
                }
            }

            asatsuyu_hir::HirPattern::List { elements, rest, span } => {
                self.check_list_pattern(elements, *rest, expected_ty, *span)
            }
        }
    }

    fn check_list_pattern(
        &mut self,
        elements: &[asatsuyu_hir::HirPattern],
        rest: Option<DefId>,
        expected_ty: &Ty,
        span: Span,
    ) -> ThirPattern {
        let elem_ty = self.infer.fresh_var();
        let list_def_id = self.type_name_to_def_id.get("List").copied().expect("builtin List type");
        let list_ty = Ty::Named {
            def_id: list_def_id,
            name: SmolStr::from("List"),
            args: vec![elem_ty.clone()],
        };
        self.unify_or_error(expected_ty, &list_ty, span, DiagnosticContext::Simple);

        let checked_elements: Vec<ThirPattern> =
            elements.iter().map(|p| self.check_pattern(p, &elem_ty)).collect();

        let checked_rest = rest.map(|rest_def_id| {
            let rest_ty = self.infer.resolve(&list_ty);
            self.type_env.insert(rest_def_id, TypeScheme::mono(rest_ty.clone()));
            Box::new(ThirPattern::Variable { def_id: rest_def_id, ty: rest_ty, span })
        });

        let resolved_ty = self.infer.resolve(&list_ty);
        ThirPattern::List { elements: checked_elements, rest: checked_rest, ty: resolved_ty, span }
    }

    // ── Exhaustiveness & reachability ─────────────────────────────

    fn check_match_coverage(&mut self, subject_ty: &Ty, arms: &[ThirMatchArm], match_span: Span) {
        let resolved = self.infer.resolve(subject_ty);
        match &resolved {
            Ty::Named { name, .. } if name == "List" => {
                self.check_list_coverage(arms, match_span);
            }
            Ty::Named { def_id, name, .. } => {
                self.check_adt_coverage(*def_id, name, arms, match_span);
            }
            Ty::Primitive(_) => {
                self.check_primitive_coverage(arms, match_span);
            }
            _ => {}
        }
    }

    fn check_list_coverage(&mut self, arms: &[ThirMatchArm], match_span: Span) {
        let mut empty_seen = false;
        let mut non_empty_seen = false;
        let mut catchall_seen = false;

        for arm in arms {
            let coverage_complete = catchall_seen || (empty_seen && non_empty_seen);
            let arm_is_reachable = !coverage_complete
                && match &arm.pattern {
                    ThirPattern::List { elements, rest, .. } => {
                        if elements.is_empty() && rest.is_none() {
                            !empty_seen
                        } else if elements.is_empty() && rest.is_some() {
                            true
                        } else {
                            !non_empty_seen
                        }
                    }
                    _ => true,
                };

            if !arm_is_reachable {
                self.push_unreachable_arm(arm.span);
                continue;
            }

            match &arm.pattern {
                ThirPattern::Wildcard(_) | ThirPattern::Variable { .. } => {
                    catchall_seen = true;
                }
                ThirPattern::List { elements, rest, .. } => {
                    if elements.is_empty() && rest.is_none() {
                        empty_seen = true;
                    } else if elements.is_empty() && rest.is_some() {
                        empty_seen = true;
                        non_empty_seen = true;
                    } else {
                        non_empty_seen = true;
                    }
                }
                _ => {}
            }
        }

        if !(catchall_seen || (empty_seen && non_empty_seen)) {
            self.push_diagnostic(
                Diagnostic::error(
                    "non-exhaustive list match: cover `[]` and a non-empty list pattern",
                    match_span,
                )
                .with_code(DiagnosticCode::E0300)
                .with_label(match_span, "not all list shapes are covered")
                .with_hint("add `[]`, `[x, ..]`, `[..]`, or `_` as needed"),
            );
        }
    }

    fn check_adt_coverage(
        &mut self,
        type_def_id: DefId,
        type_name: &SmolStr,
        arms: &[ThirMatchArm],
        match_span: Span,
    ) {
        let variant_ctor_ids: Vec<DefId> = match self.adt_registry.get(&type_def_id) {
            Some(adt_def) => adt_def.variants.iter().map(|v| v.ctor_def_id).collect(),
            None => return,
        };

        let mut remaining: HashSet<DefId> = variant_ctor_ids.iter().copied().collect();
        let mut wildcard_seen = false;

        for arm in arms {
            match &arm.pattern {
                ThirPattern::Constructor { def_id, .. } => {
                    if wildcard_seen {
                        self.push_unreachable_arm(arm.span);
                    } else {
                        remaining.remove(def_id);
                    }
                }
                ThirPattern::Wildcard(_) | ThirPattern::Variable { .. } => {
                    if wildcard_seen || remaining.is_empty() {
                        self.push_unreachable_arm(arm.span);
                    } else {
                        wildcard_seen = true;
                    }
                }
                ThirPattern::Literal(_) | ThirPattern::Tuple { .. } | ThirPattern::List { .. } => {
                    if wildcard_seen || remaining.is_empty() {
                        self.push_unreachable_arm(arm.span);
                    }
                }
            }
        }

        if !wildcard_seen && !remaining.is_empty() {
            let adt_def = &self.adt_registry[&type_def_id];
            let missing_names: Vec<&str> = adt_def
                .variants
                .iter()
                .filter(|v| remaining.contains(&v.ctor_def_id))
                .map(|v| v.name.as_str())
                .collect();
            let missing_list = missing_names.join(", ");
            // E0300: Non-exhaustive match (ADT).
            self.push_diagnostic(
                Diagnostic::error(
                    format!("non-exhaustive match on `{type_name}`: missing {missing_list}"),
                    match_span,
                )
                .with_code(DiagnosticCode::E0300)
                .with_label(match_span, "not all variants are covered")
                .with_hint(format!("add arms for: {missing_list}")),
            );
        }
    }

    fn check_primitive_coverage(&mut self, arms: &[ThirMatchArm], match_span: Span) {
        let mut catchall_seen = false;

        for arm in arms {
            match &arm.pattern {
                ThirPattern::Wildcard(_) | ThirPattern::Variable { .. } => {
                    if catchall_seen {
                        self.push_unreachable_arm(arm.span);
                    } else {
                        catchall_seen = true;
                    }
                }
                ThirPattern::Literal(_)
                | ThirPattern::Constructor { .. }
                | ThirPattern::Tuple { .. }
                | ThirPattern::List { .. } => {
                    if catchall_seen {
                        self.push_unreachable_arm(arm.span);
                    }
                }
            }
        }

        if !catchall_seen {
            // E0300: Non-exhaustive match (primitive).
            self.push_diagnostic(
                Diagnostic::error(
                    "non-exhaustive match: a wildcard `_` or variable pattern is required",
                    match_span,
                )
                .with_code(DiagnosticCode::E0300)
                .with_label(match_span, "not all values are covered")
                .with_hint("add a wildcard `_` or variable pattern to cover all values"),
            );
        }
    }

    /// Emit E0301 unreachable match arm warning.
    fn push_unreachable_arm(&mut self, span: Span) {
        self.push_diagnostic(
            Diagnostic::warning("unreachable match arm", span)
                .with_code(DiagnosticCode::E0301)
                .with_label(span, "this arm can never be reached")
                .with_note("a previous arm already covers all remaining patterns"),
        );
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

// ── FFI type conversion ──────────────────────────────────────────

impl TyCheckCtx {
    /// Convert an [`FfiType`] to an Asatsuyu [`Ty`].
    fn ffi_type_to_ty(&self, ffi_ty: &FfiType) -> Ty {
        match ffi_ty {
            FfiType::Int => Ty::Primitive(PrimTy::Int),
            FfiType::Float => Ty::Primitive(PrimTy::Float),
            FfiType::Str => Ty::Primitive(PrimTy::String),
            FfiType::Bool => Ty::Primitive(PrimTy::Bool),
            FfiType::NoneType => Ty::Primitive(PrimTy::None),
            FfiType::Named { module, name } => {
                Ty::FfiInstance { module: module.clone(), class: name.clone() }
            }
            FfiType::Any => {
                Ty::Opaque { module: SmolStr::from("python"), symbol: SmolStr::from("Any") }
            }
            FfiType::List(inner) => self.builtin_named_ty("List", vec![self.ffi_type_to_ty(inner)]),
            FfiType::Dict(key, value) => self.builtin_named_ty(
                "Dict",
                vec![self.ffi_type_to_ty(key), self.ffi_type_to_ty(value)],
            ),
            FfiType::Tuple(items) => self.builtin_named_ty(
                "Tuple",
                items.iter().map(|item| self.ffi_type_to_ty(item)).collect(),
            ),
            FfiType::Optional(inner) => {
                self.builtin_named_ty("Option", vec![self.ffi_type_to_ty(inner)])
            }
            // Deferred FFI surface: keep these explicit until the type system grows
            // a first-class representation for them.
            FfiType::Bytes | FfiType::Union(_) => Ty::Error,
        }
    }

    fn builtin_named_ty(&self, name: &str, args: Vec<Ty>) -> Ty {
        let Some(&def_id) = self.type_name_to_def_id.get(name) else {
            return Ty::Error;
        };
        Ty::Named { def_id, name: name.into(), args }
    }

    /// Convert an [`FfiSignature`] to a `Ty::Function`.
    ///
    /// Async FFI signatures (`is_async: true`) produce `Task(return_ty)`.
    fn ffi_signature_to_ty(&self, sig: &FfiSignature) -> Ty {
        let params: Vec<Ty> = sig.params.iter().map(|p| self.ffi_type_to_ty(&p.ty)).collect();
        let inner_ret = self.ffi_type_to_ty(&sig.return_ty);
        let ret =
            if sig.is_async { Box::new(self.make_task_ty(inner_ret)) } else { Box::new(inner_ret) };
        Ty::Function { params, ret }
    }

    /// Convert an [`FfiClass`] constructor to a `Ty::Function` returning `FfiInstance`.
    fn ffi_class_constructor_to_ty(&self, cls: &FfiClass, module: &SmolStr) -> Ty {
        match &cls.constructor {
            Some(sig) => {
                let params: Vec<Ty> =
                    sig.params.iter().map(|p| self.ffi_type_to_ty(&p.ty)).collect();
                let ret =
                    Box::new(Ty::FfiInstance { module: module.clone(), class: cls.name.clone() });
                Ty::Function { params, ret }
            }
            None => Ty::Error,
        }
    }
}

#[cfg(test)]
mod tests {
    use asatsuyu_hir::ffi::{
        FfiModule, FfiSource, FfiSymbol, FfiSymbolKind, FfiTrustLevel, FfiType,
    };
    use asatsuyu_syntax::Span;
    use smol_str::SmolStr;

    use super::TyCheckCtx;
    use crate::Ty;

    #[test]
    fn unsafe_symbol_resolves_to_symbol_scoped_opaque() {
        let mut ctx = TyCheckCtx::new();
        ctx.ffi_modules.insert(
            SmolStr::from("dynamic"),
            FfiModule {
                name: SmolStr::from("dynamic"),
                source: FfiSource::Builtin,
                trust_level: FfiTrustLevel::Unsafe,
                symbols: vec![FfiSymbol {
                    name: SmolStr::from("do_stuff"),
                    kind: FfiSymbolKind::Constant(FfiType::Str),
                    trust_level: Some(FfiTrustLevel::Unsafe),
                }],
            },
        );

        let ty = ctx.resolve_ffi_module_field(
            &SmolStr::from("dynamic"),
            &SmolStr::from("do_stuff"),
            Span::dummy(),
        );

        assert_eq!(
            ty,
            Ty::Opaque { module: SmolStr::from("dynamic"), symbol: SmolStr::from("do_stuff") }
        );
    }

    #[test]
    fn checked_any_still_resolves_to_python_any_opaque() {
        let mut ctx = TyCheckCtx::new();
        ctx.ffi_modules.insert(
            SmolStr::from("checked"),
            FfiModule {
                name: SmolStr::from("checked"),
                source: FfiSource::Builtin,
                trust_level: FfiTrustLevel::Checked,
                symbols: vec![FfiSymbol {
                    name: SmolStr::from("decode"),
                    kind: FfiSymbolKind::Constant(FfiType::Any),
                    trust_level: Some(FfiTrustLevel::Checked),
                }],
            },
        );

        let ty = ctx.resolve_ffi_module_field(
            &SmolStr::from("checked"),
            &SmolStr::from("decode"),
            Span::dummy(),
        );

        assert_eq!(
            ty,
            Ty::Opaque { module: SmolStr::from("python"), symbol: SmolStr::from("Any") }
        );
    }
}
