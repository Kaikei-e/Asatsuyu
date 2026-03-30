//! Hindley-Milner unification engine.
//!
//! Implements substitution-based type unification. Type variables ([`TyVarId`])
//! are bound in a substitution map and resolved lazily via [`InferCtx::resolve`].
//!
//! Includes an occurs check to prevent infinite recursive types (Issue 24).

use std::collections::{HashMap, HashSet};

use crate::types::{Ty, TyVarId, TypeScheme};

// ── Unification error ──────────────────────────────────────────────

/// The kind of unification failure.
#[derive(Debug)]
pub(crate) enum UnifyErrorKind {
    /// Two concrete types could not be made equal.
    Mismatch { expected: Ty, found: Ty },
    /// A type variable would have to equal a type that contains itself,
    /// producing an infinite type (e.g. `?0 = fn(?0) -> Int`).
    InfiniteType { var: TyVarId, ty: Ty },
}

/// A unification failure.
#[derive(Debug)]
pub(crate) struct UnifyError {
    pub kind: UnifyErrorKind,
}

// ── Inference context ──────────────────────────────────────────────

/// Accumulates type variable bindings during inference.
pub(crate) struct InferCtx {
    next_var: u32,
    subst: HashMap<TyVarId, Ty>,
}

impl InferCtx {
    pub(crate) fn new() -> Self {
        Self { next_var: 0, subst: HashMap::new() }
    }

    /// Allocate a fresh type variable.
    pub(crate) fn fresh_var(&mut self) -> Ty {
        let id = TyVarId(self.next_var);
        self.next_var += 1;
        Ty::Var(id)
    }

    // ── Resolution ─────────────────────────────────────────────────

    /// Resolve one level: if `ty` is a bound variable, return its binding.
    fn shallow_resolve(&self, ty: &Ty) -> Ty {
        match ty {
            Ty::Var(id) => match self.subst.get(id) {
                Some(bound) => self.shallow_resolve(bound),
                None => ty.clone(),
            },
            _ => ty.clone(),
        }
    }

    /// Returns `true` if `var` occurs anywhere inside `ty` (after shallow resolution).
    ///
    /// Prevents infinite types such as `?0 = fn(?0) -> Int`.
    fn occurs_in(&self, var: TyVarId, ty: &Ty) -> bool {
        match self.shallow_resolve(ty) {
            Ty::Var(id) => id == var,
            Ty::Function { params, ret } => {
                params.iter().any(|p| self.occurs_in(var, p)) || self.occurs_in(var, &ret)
            }
            Ty::Named { args, .. } => args.iter().any(|a| self.occurs_in(var, a)),
            Ty::Primitive(_)
            | Ty::FfiModule { .. }
            | Ty::FfiInstance { .. }
            | Ty::Opaque { .. }
            | Ty::Error => false,
        }
    }

    /// Fully resolve a type by recursively applying the substitution.
    pub(crate) fn resolve(&self, ty: &Ty) -> Ty {
        match ty {
            Ty::Var(id) => match self.subst.get(id) {
                Some(bound) => self.resolve(bound),
                None => ty.clone(),
            },
            Ty::Function { params, ret } => {
                let params = params.iter().map(|p| self.resolve(p)).collect();
                let ret = Box::new(self.resolve(ret));
                Ty::Function { params, ret }
            }
            Ty::Named { def_id, name, args } => {
                let args = args.iter().map(|a| self.resolve(a)).collect();
                Ty::Named { def_id: *def_id, name: name.clone(), args }
            }
            Ty::Primitive(_)
            | Ty::FfiModule { .. }
            | Ty::FfiInstance { .. }
            | Ty::Opaque { .. }
            | Ty::Error => ty.clone(),
        }
    }

    // ── Polymorphism (Issue 25: let-polymorphism) ───────────────────

    /// Collect free (unbound) type variables in a type.
    pub(crate) fn free_vars(&self, ty: &Ty) -> HashSet<TyVarId> {
        match self.shallow_resolve(ty) {
            Ty::Var(id) => {
                let mut s = HashSet::new();
                s.insert(id);
                s
            }
            Ty::Function { params, ret } => {
                let mut s = HashSet::new();
                for p in &params {
                    s.extend(self.free_vars(p));
                }
                s.extend(self.free_vars(&ret));
                s
            }
            Ty::Named { args, .. } => {
                let mut s = HashSet::new();
                for a in &args {
                    s.extend(self.free_vars(a));
                }
                s
            }
            Ty::Primitive(_)
            | Ty::FfiModule { .. }
            | Ty::FfiInstance { .. }
            | Ty::Opaque { .. }
            | Ty::Error => HashSet::new(),
        }
    }

    /// Generalize a type: quantify free vars not appearing in the environment.
    pub(crate) fn generalize(&self, ty: &Ty, env_fvs: &HashSet<TyVarId>) -> TypeScheme {
        let resolved = self.resolve(ty);
        let ty_fvs = self.free_vars(&resolved);
        let mut vars: Vec<TyVarId> = ty_fvs.difference(env_fvs).copied().collect();
        vars.sort_by_key(|v| v.0); // deterministic ordering
        TypeScheme { vars, ty: resolved }
    }

    /// Instantiate a type scheme: replace quantified vars with fresh vars.
    pub(crate) fn instantiate(&mut self, scheme: &TypeScheme) -> Ty {
        if scheme.vars.is_empty() {
            return scheme.ty.clone(); // Monomorphic fast path
        }
        let mapping: HashMap<TyVarId, Ty> =
            scheme.vars.iter().map(|&v| (v, self.fresh_var())).collect();
        Self::apply_mapping(&mapping, &scheme.ty)
    }

    /// Apply a variable mapping to a type (for instantiation).
    fn apply_mapping(mapping: &HashMap<TyVarId, Ty>, ty: &Ty) -> Ty {
        match ty {
            Ty::Var(id) => mapping.get(id).cloned().unwrap_or_else(|| ty.clone()),
            Ty::Function { params, ret } => Ty::Function {
                params: params.iter().map(|p| Self::apply_mapping(mapping, p)).collect(),
                ret: Box::new(Self::apply_mapping(mapping, ret)),
            },
            Ty::Named { def_id, name, args } => Ty::Named {
                def_id: *def_id,
                name: name.clone(),
                args: args.iter().map(|a| Self::apply_mapping(mapping, a)).collect(),
            },
            Ty::Primitive(_)
            | Ty::FfiModule { .. }
            | Ty::FfiInstance { .. }
            | Ty::Opaque { .. }
            | Ty::Error => ty.clone(),
        }
    }

    // ── Unification ────────────────────────────────────────────────

    /// Unify two types: make them equal by binding type variables.
    ///
    /// Returns `Ok(())` on success, `Err(UnifyError)` if the types are
    /// incompatible.
    pub(crate) fn unify(&mut self, a: &Ty, b: &Ty) -> Result<(), UnifyError> {
        let a = self.shallow_resolve(a);
        let b = self.shallow_resolve(b);

        match (&a, &b) {
            // Error absorbs everything — prevents cascading diagnostics.
            (Ty::Error, _) | (_, Ty::Error) => Ok(()),

            // Same variable — nothing to do.
            (Ty::Var(x), Ty::Var(y)) if x == y => Ok(()),

            // Bind unbound variable to the other type (with occurs check).
            (Ty::Var(x), _) => {
                if self.occurs_in(*x, &b) {
                    return Err(UnifyError {
                        kind: UnifyErrorKind::InfiniteType { var: *x, ty: b },
                    });
                }
                self.subst.insert(*x, b);
                Ok(())
            }
            (_, Ty::Var(y)) => {
                if self.occurs_in(*y, &a) {
                    return Err(UnifyError {
                        kind: UnifyErrorKind::InfiniteType { var: *y, ty: a },
                    });
                }
                self.subst.insert(*y, a);
                Ok(())
            }

            // Primitives must match exactly.
            (Ty::Primitive(p), Ty::Primitive(q)) => {
                if p == q {
                    Ok(())
                } else {
                    Err(UnifyError { kind: UnifyErrorKind::Mismatch { expected: a, found: b } })
                }
            }

            // Function types: arity must match, then unify component-wise.
            (Ty::Function { params: ps1, ret: r1 }, Ty::Function { params: ps2, ret: r2 }) => {
                if ps1.len() != ps2.len() {
                    return Err(UnifyError {
                        kind: UnifyErrorKind::Mismatch { expected: a, found: b },
                    });
                }
                for (p1, p2) in ps1.iter().zip(ps2.iter()) {
                    self.unify(p1, p2)?;
                }
                self.unify(r1, r2)
            }

            // Named (ADT) types: same def_id, then unify args pairwise.
            (Ty::Named { def_id: d1, args: a1, .. }, Ty::Named { def_id: d2, args: a2, .. }) => {
                if d1 != d2 || a1.len() != a2.len() {
                    return Err(UnifyError {
                        kind: UnifyErrorKind::Mismatch { expected: a, found: b },
                    });
                }
                for (x, y) in a1.iter().zip(a2.iter()) {
                    self.unify(x, y)?;
                }
                Ok(())
            }

            // FFI module types: match only if module names are identical.
            (Ty::FfiModule { module_name: m1 }, Ty::FfiModule { module_name: m2 }) => {
                if m1 == m2 {
                    Ok(())
                } else {
                    Err(UnifyError { kind: UnifyErrorKind::Mismatch { expected: a, found: b } })
                }
            }

            // FFI instance types: match only if module and class are identical.
            (
                Ty::FfiInstance { module: m1, class: c1 },
                Ty::FfiInstance { module: m2, class: c2 },
            ) => {
                if m1 == m2 && c1 == c2 {
                    Ok(())
                } else {
                    Err(UnifyError { kind: UnifyErrorKind::Mismatch { expected: a, found: b } })
                }
            }

            // Opaque FFI types: match only if module and symbol are identical.
            (Ty::Opaque { module: m1, symbol: s1 }, Ty::Opaque { module: m2, symbol: s2 }) => {
                if m1 == m2 && s1 == s2 {
                    Ok(())
                } else {
                    Err(UnifyError { kind: UnifyErrorKind::Mismatch { expected: a, found: b } })
                }
            }

            // All other combinations are incompatible.
            _ => Err(UnifyError { kind: UnifyErrorKind::Mismatch { expected: a, found: b } }),
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PrimTy;

    fn int() -> Ty {
        Ty::Primitive(PrimTy::Int)
    }

    fn string() -> Ty {
        Ty::Primitive(PrimTy::String)
    }

    fn bool_ty() -> Ty {
        Ty::Primitive(PrimTy::Bool)
    }

    fn fun(params: Vec<Ty>, ret: Ty) -> Ty {
        Ty::Function { params, ret: Box::new(ret) }
    }

    #[test]
    fn unify_same_primitive() {
        let mut ctx = InferCtx::new();
        assert!(ctx.unify(&int(), &int()).is_ok());
    }

    #[test]
    fn unify_different_primitive() {
        let mut ctx = InferCtx::new();
        assert!(ctx.unify(&int(), &string()).is_err());
    }

    #[test]
    fn unify_var_with_primitive() {
        let mut ctx = InferCtx::new();
        let var = ctx.fresh_var();
        assert!(ctx.unify(&var, &int()).is_ok());
        assert_eq!(ctx.resolve(&var), int());
    }

    #[test]
    fn unify_primitive_with_var() {
        let mut ctx = InferCtx::new();
        let var = ctx.fresh_var();
        assert!(ctx.unify(&int(), &var).is_ok());
        assert_eq!(ctx.resolve(&var), int());
    }

    #[test]
    fn unify_two_vars() {
        let mut ctx = InferCtx::new();
        let a = ctx.fresh_var();
        let b = ctx.fresh_var();
        assert!(ctx.unify(&a, &b).is_ok());
        // Bind both to a concrete type via one of them.
        assert!(ctx.unify(&a, &int()).is_ok());
        assert_eq!(ctx.resolve(&a), int());
        assert_eq!(ctx.resolve(&b), int());
    }

    #[test]
    fn unify_function_types() {
        let mut ctx = InferCtx::new();
        let f1 = fun(vec![int()], int());
        let f2 = fun(vec![int()], int());
        assert!(ctx.unify(&f1, &f2).is_ok());
    }

    #[test]
    fn unify_function_arity_mismatch() {
        let mut ctx = InferCtx::new();
        let f1 = fun(vec![int()], int());
        let f2 = fun(vec![int(), int()], int());
        assert!(ctx.unify(&f1, &f2).is_err());
    }

    #[test]
    fn unify_function_param_mismatch() {
        let mut ctx = InferCtx::new();
        let f1 = fun(vec![int()], int());
        let f2 = fun(vec![string()], int());
        assert!(ctx.unify(&f1, &f2).is_err());
    }

    #[test]
    fn unify_var_in_function() {
        let mut ctx = InferCtx::new();
        let a = ctx.fresh_var();
        let b = ctx.fresh_var();
        let f1 = fun(vec![a.clone()], b.clone());
        let f2 = fun(vec![int()], string());
        assert!(ctx.unify(&f1, &f2).is_ok());
        assert_eq!(ctx.resolve(&a), int());
        assert_eq!(ctx.resolve(&b), string());
    }

    #[test]
    fn unify_error_is_absorbing() {
        let mut ctx = InferCtx::new();
        assert!(ctx.unify(&Ty::Error, &int()).is_ok());
        assert!(ctx.unify(&string(), &Ty::Error).is_ok());
    }

    #[test]
    fn resolve_chain() {
        let mut ctx = InferCtx::new();
        let a = ctx.fresh_var(); // ?0
        let b = ctx.fresh_var(); // ?1
        // ?0 → ?1 → Int
        assert!(ctx.unify(&a, &b).is_ok());
        assert!(ctx.unify(&b, &int()).is_ok());
        assert_eq!(ctx.resolve(&a), int());
    }

    #[test]
    fn unify_bool_types() {
        let mut ctx = InferCtx::new();
        assert!(ctx.unify(&bool_ty(), &bool_ty()).is_ok());
        assert!(ctx.unify(&bool_ty(), &int()).is_err());
    }

    // ── Occurs check tests (Issue 24) ─────────────────────────────

    #[test]
    fn occurs_check_var_in_function_param() {
        let mut ctx = InferCtx::new();
        let a = ctx.fresh_var(); // ?0
        // ?0 = fn(?0) -> Int  →  infinite type
        let recursive = fun(vec![a.clone()], int());
        let err = ctx.unify(&a, &recursive).unwrap_err();
        assert!(matches!(err.kind, UnifyErrorKind::InfiniteType { .. }));
    }

    #[test]
    fn occurs_check_var_in_function_return() {
        let mut ctx = InferCtx::new();
        let a = ctx.fresh_var(); // ?0
        // ?0 = fn(Int) -> ?0  →  infinite type
        let recursive = fun(vec![int()], a.clone());
        let err = ctx.unify(&a, &recursive).unwrap_err();
        assert!(matches!(err.kind, UnifyErrorKind::InfiniteType { .. }));
    }

    #[test]
    fn occurs_check_transitive() {
        let mut ctx = InferCtx::new();
        let a = ctx.fresh_var(); // ?0
        let b = ctx.fresh_var(); // ?1
        // ?0 → ?1, then ?1 = fn(?0) -> Int  →  infinite type via chain
        assert!(ctx.unify(&a, &b).is_ok());
        let recursive = fun(vec![a.clone()], int());
        let err = ctx.unify(&b, &recursive).unwrap_err();
        assert!(matches!(err.kind, UnifyErrorKind::InfiniteType { .. }));
    }

    #[test]
    fn occurs_check_same_var_identity() {
        let mut ctx = InferCtx::new();
        let a = ctx.fresh_var(); // ?0
        // ?0 = ?0  →  identity, handled before occurs check
        assert!(ctx.unify(&a, &a).is_ok());
    }

    // ── Named (ADT) type tests (Issue 26) ────────────────────────────

    use asatsuyu_hir::SymbolTable;

    /// Helper: allocate a fresh type `DefId` in a symbol table.
    fn alloc_type(st: &mut SymbolTable, name: &str) -> asatsuyu_hir::DefId {
        st.alloc(asatsuyu_hir::DefData {
            name: smol_str::SmolStr::from(name),
            kind: asatsuyu_hir::DefKind::Type,
            span: asatsuyu_syntax::Span::dummy(),
        })
    }

    #[test]
    fn unify_named_same_type() {
        let mut ctx = InferCtx::new();
        let mut st = SymbolTable::new();
        let id = alloc_type(&mut st, "Option");
        let a = Ty::Named { def_id: id, name: "Option".into(), args: vec![int()] };
        let b = Ty::Named { def_id: id, name: "Option".into(), args: vec![int()] };
        assert!(ctx.unify(&a, &b).is_ok());
    }

    #[test]
    fn unify_named_different_type() {
        let mut ctx = InferCtx::new();
        let mut st = SymbolTable::new();
        let id1 = alloc_type(&mut st, "Option");
        let id2 = alloc_type(&mut st, "Result");
        let a = Ty::Named { def_id: id1, name: "Option".into(), args: vec![int()] };
        let b = Ty::Named { def_id: id2, name: "Result".into(), args: vec![int()] };
        assert!(ctx.unify(&a, &b).is_err());
    }

    #[test]
    fn unify_named_arg_mismatch() {
        let mut ctx = InferCtx::new();
        let mut st = SymbolTable::new();
        let id = alloc_type(&mut st, "Option");
        let a = Ty::Named { def_id: id, name: "Option".into(), args: vec![int()] };
        let b = Ty::Named { def_id: id, name: "Option".into(), args: vec![string()] };
        assert!(ctx.unify(&a, &b).is_err());
    }

    #[test]
    fn unify_named_with_var() {
        let mut ctx = InferCtx::new();
        let mut st = SymbolTable::new();
        let id = alloc_type(&mut st, "Option");
        let var = ctx.fresh_var();
        let n = Ty::Named { def_id: id, name: "Option".into(), args: vec![int()] };
        assert!(ctx.unify(&var, &n).is_ok());
        assert_eq!(ctx.resolve(&var), n);
    }

    #[test]
    fn occurs_check_named() {
        let mut ctx = InferCtx::new();
        let mut st = SymbolTable::new();
        let id = alloc_type(&mut st, "Option");
        let a = ctx.fresh_var(); // ?0
        // ?0 = Option(?0) → infinite type
        let recursive = Ty::Named { def_id: id, name: "Option".into(), args: vec![a.clone()] };
        let err = ctx.unify(&a, &recursive).unwrap_err();
        assert!(matches!(err.kind, UnifyErrorKind::InfiniteType { .. }));
    }

    #[test]
    fn resolve_named() {
        let mut ctx = InferCtx::new();
        let mut st = SymbolTable::new();
        let id = alloc_type(&mut st, "Option");
        let var = ctx.fresh_var(); // ?0
        assert!(ctx.unify(&var, &int()).is_ok());
        let n = Ty::Named { def_id: id, name: "Option".into(), args: vec![var] };
        let resolved = ctx.resolve(&n);
        match resolved {
            Ty::Named { args, .. } => assert_eq!(args[0], int()),
            other => panic!("expected Named, got {other:?}"),
        }
    }

    #[test]
    fn free_vars_named() {
        let mut ctx = InferCtx::new();
        let mut st = SymbolTable::new();
        let id = alloc_type(&mut st, "Option");
        let var = ctx.fresh_var(); // ?0
        let n = Ty::Named { def_id: id, name: "Option".into(), args: vec![var.clone()] };
        let fvs = ctx.free_vars(&n);
        assert_eq!(fvs.len(), 1);
        match var {
            Ty::Var(id) => assert!(fvs.contains(&id)),
            _ => unreachable!(),
        }
        let _ = st; // keep alive
    }

    // ── Opaque type tests (Issue 39) ─────────────────────────────

    fn opaque(module: &str, symbol: &str) -> Ty {
        Ty::Opaque { module: module.into(), symbol: symbol.into() }
    }

    #[test]
    fn unify_same_opaque() {
        let mut ctx = InferCtx::new();
        assert!(ctx.unify(&opaque("json", "loads"), &opaque("json", "loads")).is_ok());
    }

    #[test]
    fn unify_different_opaque() {
        let mut ctx = InferCtx::new();
        assert!(ctx.unify(&opaque("json", "loads"), &opaque("os", "getenv")).is_err());
    }

    #[test]
    fn unify_opaque_with_primitive() {
        let mut ctx = InferCtx::new();
        assert!(ctx.unify(&opaque("json", "loads"), &int()).is_err());
    }

    #[test]
    fn unify_var_with_opaque() {
        let mut ctx = InferCtx::new();
        let var = ctx.fresh_var();
        assert!(ctx.unify(&var, &opaque("json", "loads")).is_ok());
        assert_eq!(ctx.resolve(&var), opaque("json", "loads"));
    }

    #[test]
    fn resolve_opaque() {
        let ctx = InferCtx::new();
        let o = opaque("json", "loads");
        assert_eq!(ctx.resolve(&o), o);
    }

    #[test]
    fn free_vars_opaque() {
        let ctx = InferCtx::new();
        let fvs = ctx.free_vars(&opaque("json", "loads"));
        assert!(fvs.is_empty());
    }

    #[test]
    fn occurs_check_opaque() {
        let ctx = InferCtx::new();
        let var = TyVarId(99);
        assert!(!ctx.occurs_in(var, &opaque("json", "loads")));
    }

    // ── FfiModule type tests (Issue 40) ──────────────────────────

    fn ffi_module(name: &str) -> Ty {
        Ty::FfiModule { module_name: name.into() }
    }

    fn ffi_instance(module: &str, class: &str) -> Ty {
        Ty::FfiInstance { module: module.into(), class: class.into() }
    }

    #[test]
    fn unify_same_ffi_module() {
        let mut ctx = InferCtx::new();
        assert!(ctx.unify(&ffi_module("pathlib"), &ffi_module("pathlib")).is_ok());
    }

    #[test]
    fn unify_different_ffi_module() {
        let mut ctx = InferCtx::new();
        assert!(ctx.unify(&ffi_module("pathlib"), &ffi_module("os")).is_err());
    }

    #[test]
    fn unify_ffi_module_with_primitive() {
        let mut ctx = InferCtx::new();
        assert!(ctx.unify(&ffi_module("pathlib"), &int()).is_err());
    }

    #[test]
    fn unify_var_with_ffi_module() {
        let mut ctx = InferCtx::new();
        let var = ctx.fresh_var();
        assert!(ctx.unify(&var, &ffi_module("pathlib")).is_ok());
        assert_eq!(ctx.resolve(&var), ffi_module("pathlib"));
    }

    #[test]
    fn unify_same_ffi_instance() {
        let mut ctx = InferCtx::new();
        assert!(
            ctx.unify(&ffi_instance("pathlib", "Path"), &ffi_instance("pathlib", "Path")).is_ok()
        );
    }

    #[test]
    fn unify_different_ffi_instance() {
        let mut ctx = InferCtx::new();
        assert!(
            ctx.unify(&ffi_instance("pathlib", "Path"), &ffi_instance("os", "DirEntry")).is_err()
        );
    }

    #[test]
    fn unify_ffi_instance_with_primitive() {
        let mut ctx = InferCtx::new();
        assert!(ctx.unify(&ffi_instance("pathlib", "Path"), &string()).is_err());
    }

    #[test]
    fn unify_var_with_ffi_instance() {
        let mut ctx = InferCtx::new();
        let var = ctx.fresh_var();
        assert!(ctx.unify(&var, &ffi_instance("pathlib", "Path")).is_ok());
        assert_eq!(ctx.resolve(&var), ffi_instance("pathlib", "Path"));
    }

    #[test]
    fn free_vars_ffi_module() {
        let ctx = InferCtx::new();
        assert!(ctx.free_vars(&ffi_module("pathlib")).is_empty());
    }

    #[test]
    fn free_vars_ffi_instance() {
        let ctx = InferCtx::new();
        assert!(ctx.free_vars(&ffi_instance("pathlib", "Path")).is_empty());
    }
}
