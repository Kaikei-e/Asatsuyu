//! Hindley-Milner unification engine.
//!
//! Implements substitution-based type unification. Type variables ([`TyVarId`])
//! are bound in a substitution map and resolved lazily via [`InferCtx::resolve`].
//!
//! Occurs check is deferred to Issue 24.

use std::collections::HashMap;

use crate::types::{Ty, TyVarId};

// ── Unification error ──────────────────────────────────────────────

/// A unification failure: two types could not be made equal.
#[derive(Debug)]
pub(crate) struct UnifyError {
    pub expected: Ty,
    pub found: Ty,
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

            // Bind unbound variable to the other type.
            (Ty::Var(x), _) => {
                self.subst.insert(*x, b);
                Ok(())
            }
            (_, Ty::Var(y)) => {
                self.subst.insert(*y, a);
                Ok(())
            }

            // Primitives must match exactly.
            (Ty::Primitive(p), Ty::Primitive(q)) => {
                if p == q {
                    Ok(())
                } else {
                    Err(UnifyError { expected: a, found: b })
                }
            }

            // Function types: arity must match, then unify component-wise.
            (Ty::Function { params: ps1, ret: r1 }, Ty::Function { params: ps2, ret: r2 }) => {
                if ps1.len() != ps2.len() {
                    return Err(UnifyError { expected: a, found: b });
                }
                for (p1, p2) in ps1.iter().zip(ps2.iter()) {
                    self.unify(p1, p2)?;
                }
                self.unify(r1, r2)
            }

            // All other combinations are incompatible.
            _ => Err(UnifyError { expected: a, found: b }),
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
}
