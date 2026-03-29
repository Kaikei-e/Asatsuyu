//! Hindley-Milner unification engine.
//!
//! Implements substitution-based type unification. Type variables ([`TyVarId`])
//! are bound in a substitution map and resolved lazily via [`InferCtx::resolve`].
//!
//! Includes an occurs check to prevent infinite recursive types (Issue 24).

use std::collections::HashMap;

use crate::types::{Ty, TyVarId};

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
    #[allow(dead_code)] // Used in Issue 25 (let-polymorphism)
    next_var: u32,
    subst: HashMap<TyVarId, Ty>,
}

impl InferCtx {
    pub(crate) fn new() -> Self {
        Self { next_var: 0, subst: HashMap::new() }
    }

    /// Allocate a fresh type variable.
    #[allow(dead_code)] // Used in Issue 25 (let-polymorphism)
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
            Ty::Primitive(_) | Ty::Error => false,
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
            Ty::Primitive(_) | Ty::Error => ty.clone(),
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
}
