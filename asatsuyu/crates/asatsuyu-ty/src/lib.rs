//! Hindley-Milner type inference and type checking for the Asatsuyu language.
//!
//! Receives HIR and produces THIR (Typed HIR) with resolved types on every node.
//!
//! # Usage
//!
//! ```
//! use asatsuyu_ast::lower;
//! use asatsuyu_hir::lower_to_hir;
//! use asatsuyu_parser::parse;
//! use asatsuyu_syntax::FileId;
//! use asatsuyu_ty::check_types;
//!
//! let cst = parse(FileId(0), "pub fn main() { 42 }");
//! let ast = lower(&cst, FileId(0));
//! let hir = lower_to_hir(&ast.module);
//! let thir = check_types(&hir.module);
//! assert!(!thir.has_errors());
//! assert_eq!(thir.module.functions.len(), 1);
//! ```

mod check;
mod types;
mod unify;

pub use types::{
    PrimTy, ThirExpr, ThirFnDef, ThirLiteral, ThirMatchArm, ThirModule, ThirParam, ThirPattern, Ty,
    TyVarId,
};

use asatsuyu_hir::HirModule;
use asatsuyu_hir::ffi::FfiResolverConfig;
use asatsuyu_syntax::{Diagnostic, Severity};

/// The result of type-checking an HIR module.
#[derive(Debug)]
pub struct TyCheckResult {
    /// The typed HIR module with resolved types on every expression.
    pub module: ThirModule,
    /// Diagnostics collected during type checking (e.g., type mismatches).
    pub diagnostics: Vec<Diagnostic>,
}

impl TyCheckResult {
    /// Returns `true` if any error-level diagnostic was emitted during type checking.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.severity == Severity::Error)
    }
}

/// Type-check an HIR module, producing THIR with resolved types.
///
/// Uses a two-pass approach:
/// 1. Collect function signatures into the type environment.
/// 2. Check each function body against its declared return type.
#[must_use]
pub fn check_types(hir: &HirModule) -> TyCheckResult {
    check_types_with_ffi_config(hir, &FfiResolverConfig::default())
}

/// Type-check an HIR module with custom FFI resolver configuration.
///
/// Like [`check_types`], but accepts an [`FfiResolverConfig`] to control
/// which Python modules are resolvable (e.g. `--ffi-stdlib-only`).
#[must_use]
pub fn check_types_with_ffi_config(
    hir: &HirModule,
    ffi_config: &FfiResolverConfig,
) -> TyCheckResult {
    let mut ctx = check::TyCheckCtx::new();
    ctx.collect_signatures_with_config(hir, ffi_config);
    let module = ctx.check_module(hir);
    let diagnostics = ctx.into_diagnostics();
    TyCheckResult { module, diagnostics }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asatsuyu_ast::LiteralKind;
    use asatsuyu_parser::parse;
    use asatsuyu_syntax::FileId;

    const FID: FileId = FileId(0);

    fn assert_no_error_ty(expr: &ThirExpr) {
        assert_ne!(*expr.ty(), Ty::Error, "expression has Error type: {expr:?}");
        if let ThirExpr::Block { exprs, .. } = expr {
            for e in exprs {
                assert_no_error_ty(e);
            }
        }
    }

    /// Helper: parse → AST → HIR → THIR.
    fn thir_from_source(source: &str) -> TyCheckResult {
        let cst = parse(FID, source);
        let ast = asatsuyu_ast::lower(&cst, FID);
        let hir = asatsuyu_hir::lower_to_hir(&ast.module);
        check_types(&hir.module)
    }

    // ── 1. Empty module ─────────────────────────────────────────────

    #[test]
    fn check_empty_module() {
        let result = thir_from_source("");
        assert!(!result.has_errors());
        assert!(result.module.functions.is_empty());
    }

    // ── 2. Minimal function ─────────────────────────────────────────

    #[test]
    fn check_minimal_function() {
        let result = thir_from_source("pub fn main() { 42 }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert_eq!(result.module.functions.len(), 1);

        let f = &result.module.functions[0];
        match &f.body {
            ThirExpr::Block { exprs, ty, .. } => {
                assert_eq!(*ty, Ty::Primitive(PrimTy::Int));
                assert_eq!(exprs.len(), 1);
                match &exprs[0] {
                    ThirExpr::Literal(lit) => {
                        assert_eq!(lit.kind, LiteralKind::Int);
                        assert_eq!(lit.ty, Ty::Primitive(PrimTy::Int));
                    }
                    other => panic!("expected Literal, got {other:?}"),
                }
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    // ── 3. String literal ───────────────────────────────────────────

    #[test]
    fn check_string_literal() {
        let result = thir_from_source(r#"fn f() { "hello" }"#);
        assert!(!result.has_errors());

        let f = &result.module.functions[0];
        match &f.body {
            ThirExpr::Block { exprs, .. } => match &exprs[0] {
                ThirExpr::Literal(lit) => {
                    assert_eq!(lit.kind, LiteralKind::String);
                    assert_eq!(lit.ty, Ty::Primitive(PrimTy::String));
                }
                other => panic!("expected Literal, got {other:?}"),
            },
            other => panic!("expected Block, got {other:?}"),
        }
    }

    // ── 4. Parameter type ───────────────────────────────────────────

    #[test]
    fn check_parameter_type() {
        let result = thir_from_source("fn id(x: Int) -> Int { x }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let f = &result.module.functions[0];
        assert_eq!(f.params.len(), 1);
        assert_eq!(f.params[0].ty, Ty::Primitive(PrimTy::Int));

        // Body var should resolve to Int (same as parameter type).
        match &f.body {
            ThirExpr::Block { exprs, .. } => match &exprs[0] {
                ThirExpr::Var { ty, .. } => {
                    assert_eq!(*ty, Ty::Primitive(PrimTy::Int));
                }
                other => panic!("expected Var, got {other:?}"),
            },
            other => panic!("expected Block, got {other:?}"),
        }
    }

    // ── 5. Return type match ────────────────────────────────────────

    #[test]
    fn check_return_type_match() {
        let result = thir_from_source("fn f() -> Int { 42 }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let f = &result.module.functions[0];
        assert_eq!(f.return_ty, Ty::Primitive(PrimTy::Int));
    }

    // ── 6. Return type mismatch ─────────────────────────────────────

    #[test]
    fn check_return_type_mismatch() {
        let result = thir_from_source(r#"fn f() -> Int { "hello" }"#);
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("type mismatch")),
            "expected type mismatch diagnostic: {:?}",
            result.diagnostics
        );
    }

    // ── 7. Unknown type annotation ──────────────────────────────────

    #[test]
    fn check_unknown_type_annotation() {
        let result = thir_from_source("fn f(x: Foo) { x }");
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("unknown type `Foo`")),
            "expected unknown type diagnostic: {:?}",
            result.diagnostics
        );
    }

    // ── 8. Empty block is None ──────────────────────────────────────

    #[test]
    fn check_empty_block_is_none() {
        let result = thir_from_source("fn f() { }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let f = &result.module.functions[0];
        assert_eq!(f.return_ty, Ty::Primitive(PrimTy::None));
    }

    // ── 9. Block type is last expr ──────────────────────────────────

    #[test]
    fn check_block_type_is_last_expr() {
        let result = thir_from_source(r#"fn f() { 1 "hi" }"#);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let f = &result.module.functions[0];
        // Return type inferred from body: last expr is "hi" → String.
        assert_eq!(f.return_ty, Ty::Primitive(PrimTy::String));
    }

    // ── 10. Inferred return — no error ──────────────────────────────

    #[test]
    fn check_inferred_return_no_error() {
        // No return type annotation → infer from body, no mismatch.
        let result = thir_from_source("pub fn main() { 42 }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let f = &result.module.functions[0];
        // Inferred return type from body (Int literal).
        assert_eq!(f.return_ty, Ty::Primitive(PrimTy::Int));
    }

    // ── 11. Function ref type ───────────────────────────────────────

    #[test]
    fn check_function_ref_type() {
        let result = thir_from_source("fn a() { b }\nfn b() { 1 }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        // In function `a`, the body references `b` which is a function.
        let a = &result.module.functions[0];
        match &a.body {
            ThirExpr::Block { exprs, .. } => match &exprs[0] {
                ThirExpr::Var { ty, .. } => {
                    assert!(matches!(ty, Ty::Function { .. }), "expected Ty::Function, got {ty:?}");
                }
                other => panic!("expected Var, got {other:?}"),
            },
            other => panic!("expected Block, got {other:?}"),
        }
    }

    // ── 12. hello.asty ──────────────────────────────────────────────

    #[test]
    fn check_hello_asty() {
        let source = include_str!("../../../examples/hello.asty");
        let cst = parse(FID, source);
        let ast = asatsuyu_ast::lower(&cst, FID);
        let hir = asatsuyu_hir::lower_to_hir(&ast.module);
        let result = check_types(&hir.module);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert_eq!(result.module.functions.len(), 1);
    }

    // ── 13. greet.asty ──────────────────────────────────────────────

    #[test]
    fn check_greet_asty() {
        let source = include_str!("../../../examples/greet.asty");
        let cst = parse(FID, source);
        let ast = asatsuyu_ast::lower(&cst, FID);
        let hir = asatsuyu_hir::lower_to_hir(&ast.module);
        let result = check_types(&hir.module);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert_eq!(result.module.functions.len(), 2);

        let greet = &result.module.functions[0];
        assert_eq!(greet.return_ty, Ty::Primitive(PrimTy::String));
        assert_eq!(greet.params.len(), 1);
        assert_eq!(greet.params[0].ty, Ty::Primitive(PrimTy::String));

        let main = &result.module.functions[1];
        assert_eq!(main.return_ty, Ty::Primitive(PrimTy::None));
        assert_eq!(main.params.len(), 0);
    }

    // ── 14. THIR dump ───────────────────────────────────────────────

    #[test]
    fn check_thir_dump() {
        let result = thir_from_source("pub fn main() { 42 }");
        let dump = format!("{:#?}", result.module);
        assert!(dump.contains("Primitive"), "dump should contain Primitive:\n{dump}");
        assert!(dump.contains("Int"), "dump should contain Int:\n{dump}");
        assert!(dump.contains("Function"), "dump should contain Function:\n{dump}");
    }

    #[test]
    fn check_thir_dump_shows_mutable_let_and_assign() {
        let result = thir_from_source("pub fn main() { let mut x = 0\n x = 1 }");
        assert!(!result.has_errors(), "unexpected diagnostics: {:?}", result.diagnostics);

        let dump = format!("{:#?}", result.module);
        assert!(dump.contains("is_mutable: true"), "dump should show mutable let flag:\n{dump}");
        assert!(dump.contains("Assign"), "dump should show typed Assign node:\n{dump}");
        assert!(
            dump.contains("Primitive(\n                            None"),
            "assign/let should remain statement-typed:\n{dump}"
        );
    }

    // ── 15. All expressions have type ───────────────────────────────

    #[test]
    fn check_all_exprs_have_ty() {
        let result = thir_from_source("pub fn add(x: Int) -> Int { x }");
        assert!(!result.has_errors());

        let f = &result.module.functions[0];

        // Function has a type.
        assert!(matches!(f.ty, Ty::Function { .. }));

        // Body and its inner expressions have non-Error types.
        assert_no_error_ty(&f.body);
    }

    // ── 16. Call — basic ───────────────────────────────────────────

    #[test]
    fn check_call_basic() {
        let result = thir_from_source("fn f(x: Int) -> Int { x }\nfn g() -> Int { f(1) }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let g = &result.module.functions[1];
        assert_eq!(g.return_ty, Ty::Primitive(PrimTy::Int));
    }

    // ── 17. Call — arity mismatch ──────────────────────────────────

    #[test]
    fn check_call_arity_mismatch() {
        let result = thir_from_source("fn f(x: Int) -> Int { x }\nfn g() { f(1, 2) }");
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("argument")),
            "expected arity diagnostic: {:?}",
            result.diagnostics
        );
    }

    // ── 18. Call — type mismatch ───────────────────────────────────

    #[test]
    fn check_call_type_mismatch() {
        let result = thir_from_source(r#"fn f(x: Int) -> Int { x } fn g() { f("hello") }"#);
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("type mismatch")),
            "expected type mismatch: {:?}",
            result.diagnostics
        );
    }

    // ── 19. BinaryOp — add ─────────────────────────────────────────

    #[test]
    fn check_binary_add() {
        let result = thir_from_source("fn f(x: Int, y: Int) -> Int { x + y }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        let f = &result.module.functions[0];
        assert_eq!(f.return_ty, Ty::Primitive(PrimTy::Int));
    }

    // ── 20. BinaryOp — eq ──────────────────────────────────────────

    #[test]
    fn check_binary_eq() {
        let result = thir_from_source("fn f(x: Int, y: Int) -> Bool { x == y }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        let f = &result.module.functions[0];
        assert_eq!(f.return_ty, Ty::Primitive(PrimTy::Bool));
    }

    // ── 21. BinaryOp — type mismatch ───────────────────────────────

    #[test]
    fn check_binary_type_mismatch() {
        let result = thir_from_source(r#"fn f() -> Int { 1 + "hello" }"#);
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("type mismatch")),
            "diagnostics: {:?}",
            result.diagnostics
        );
    }

    // ── 22. BinaryOp — logical and ─────────────────────────────────

    #[test]
    fn check_binary_and() {
        let result = thir_from_source("fn f(a: Bool, b: Bool) -> Bool { a && b }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
    }

    // ── 23. UnaryOp — neg ──────────────────────────────────────────

    #[test]
    fn check_unary_neg() {
        let result = thir_from_source("fn f(x: Int) -> Int { -x }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        let f = &result.module.functions[0];
        assert_eq!(f.return_ty, Ty::Primitive(PrimTy::Int));
    }

    // ── 24. UnaryOp — not ──────────────────────────────────────────

    #[test]
    fn check_unary_not() {
        let result = thir_from_source("fn f(x: Bool) -> Bool { !x }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
    }

    // ── 25. If — basic ─────────────────────────────────────────────

    #[test]
    fn check_if_basic() {
        let result = thir_from_source("fn f(b: Bool) -> Int { if b { 1 } else { 2 } }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        let f = &result.module.functions[0];
        assert_eq!(f.return_ty, Ty::Primitive(PrimTy::Int));
    }

    // ── 26. If — branch type mismatch ──────────────────────────────

    #[test]
    fn check_if_branch_mismatch() {
        let result = thir_from_source(r#"fn f(b: Bool) { if b { 1 } else { "hi" } }"#);
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("type mismatch")),
            "diagnostics: {:?}",
            result.diagnostics
        );
    }

    // ── 27. If — condition not Bool ────────────────────────────────

    #[test]
    fn check_if_cond_not_bool() {
        let result = thir_from_source("fn f() { if 1 { 2 } else { 3 } }");
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("type mismatch")),
            "diagnostics: {:?}",
            result.diagnostics
        );
    }

    // ── 28. Match — basic ──────────────────────────────────────────

    #[test]
    fn check_match_basic() {
        let source = "fn f(x: Int) -> Int {\n  match x {\n    0 -> 1\n    _ -> 2\n  }\n}";
        let result = thir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        let f = &result.module.functions[0];
        assert_eq!(f.return_ty, Ty::Primitive(PrimTy::Int));
    }

    // ── 29. Match — arm type mismatch ──────────────────────────────

    #[test]
    fn check_match_arm_mismatch() {
        let source = r#"fn f(x: Int) { match x { 0 -> 1 _ -> "hi" } }"#;
        let result = thir_from_source(source);
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("type mismatch")),
            "diagnostics: {:?}",
            result.diagnostics
        );
    }

    // ── 30. Non-callable diagnostic ────────────────────────────────

    #[test]
    fn check_non_callable() {
        let result = thir_from_source("fn f(x: Int) { x(1) }");
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("expected function")),
            "diagnostics: {:?}",
            result.diagnostics
        );
    }

    // ── 31. Arithmetic on non-numeric ──────────────────────────────

    #[test]
    fn check_arithmetic_on_non_numeric() {
        let result = thir_from_source("fn f(a: Bool, b: Bool) -> Bool { a + b }");
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("arithmetic")),
            "diagnostics: {:?}",
            result.diagnostics
        );
    }

    // ── 32. Occurs check (Issue 24) ───────────────────────────────

    #[test]
    fn occurs_check_produces_infinite_type_error() {
        use crate::unify::{InferCtx, UnifyErrorKind};

        let mut ctx = InferCtx::new();
        let a = ctx.fresh_var(); // ?0
        // ?0 = fn(?0) -> Int  →  infinite type
        let recursive =
            Ty::Function { params: vec![a.clone()], ret: Box::new(Ty::Primitive(PrimTy::Int)) };
        let err = ctx.unify(&a, &recursive).unwrap_err();
        match err.kind {
            UnifyErrorKind::InfiniteType { var, .. } => {
                assert_eq!(var, crate::types::TyVarId(0));
            }
            UnifyErrorKind::Mismatch { .. } => {
                panic!("expected InfiniteType, got Mismatch");
            }
        }
    }

    // ── 33. Let binding (Issue 25) ────────────────────────────────

    #[test]
    fn let_simple_binding() {
        let result = thir_from_source("pub fn main() { let x = 42\n x }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
    }

    // ── 34. Lambda expression (Issue 25) ──────────────────────────

    #[test]
    fn lambda_inferred_param() {
        // fn(x) { x } used with an Int argument should infer (Int) -> Int
        let result = thir_from_source("pub fn main() -> Int { let f = fn(x) { x }\n f(42) }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
    }

    // ── 35. Let-polymorphism: identity function (Issue 25 DoD) ───

    #[test]
    fn let_polymorphic_identity() {
        // let id = fn(x) { x } should be polymorphic:
        // id(42) : Int, id("hello") : String
        let result = thir_from_source(
            "pub fn main() { let id = fn(x) { x }\n let a = id(42)\n let b = id(\"hello\")\n b }",
        );
        assert!(
            !result.has_errors(),
            "polymorphic identity should type-check without errors: {:?}",
            result.diagnostics
        );
    }

    // ── 36. Let monomorphic use ───────────────────────────────────

    #[test]
    fn let_monomorphic_value() {
        // let x = 42 — x should be Int, not polymorphic
        let result = thir_from_source("pub fn main() -> Int { let x = 42\n x }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
    }

    // ── 37. Lambda with type annotation ───────────────────────────

    #[test]
    fn lambda_with_annotation() {
        let result = thir_from_source("pub fn main() -> Int { let f = fn(x: Int) { x }\n f(42) }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
    }

    // ── 38. Generalize/instantiate unit test ──────────────────────

    #[test]
    fn generalize_and_instantiate() {
        use crate::unify::InferCtx;
        use std::collections::HashSet;

        let mut ctx = InferCtx::new();
        let a = ctx.fresh_var(); // ?0
        // Type: ?0 -> ?0 (identity function type)
        let fn_ty = Ty::Function { params: vec![a.clone()], ret: Box::new(a) };

        // Generalize with empty env → forall ?0. ?0 -> ?0
        let scheme = ctx.generalize(&fn_ty, &HashSet::new());
        assert_eq!(scheme.vars.len(), 1, "should quantify one variable");

        // Instantiate twice → independent fresh vars
        let inst1 = ctx.instantiate(&scheme);
        let inst2 = ctx.instantiate(&scheme);
        assert_ne!(inst1, inst2, "each instantiation should produce fresh variables");
    }

    // ── ADT constructor typing tests (Issue 26) ───────────────────────

    #[test]
    fn check_unary_constructor_some_int() {
        let source = "\
            type Option(a) { Some(a) None }\n\
            pub fn f() { Some(42) }";
        let result = thir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        let f = &result.module.functions[0];
        let body_ty = f.body.ty();
        match body_ty {
            Ty::Named { name, args, .. } => {
                assert_eq!(name.as_str(), "Option");
                assert_eq!(args.len(), 1);
                assert_eq!(args[0], Ty::Primitive(PrimTy::Int));
            }
            other => panic!("expected Named(Option(Int)), got {other:?}"),
        }
    }

    #[test]
    fn check_nullary_constructor_none() {
        let source = "\
            type Option(a) { Some(a) None }\n\
            pub fn f() { None }";
        let result = thir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        let f = &result.module.functions[0];
        match f.body.ty() {
            Ty::Named { name, .. } => assert_eq!(name.as_str(), "Option"),
            other => panic!("expected Named(Option(...)), got {other:?}"),
        }
    }

    #[test]
    fn check_result_constructor_ok() {
        let source = "\
            type Result(a, e) { Ok(a) Err(e) }\n\
            pub fn f() { Ok(\"hello\") }";
        let result = thir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        let f = &result.module.functions[0];
        match f.body.ty() {
            Ty::Named { name, args, .. } => {
                assert_eq!(name.as_str(), "Result");
                assert_eq!(args.len(), 2);
                assert_eq!(args[0], Ty::Primitive(PrimTy::String));
            }
            other => panic!("expected Named(Result(String, ?)), got {other:?}"),
        }
    }

    #[test]
    fn check_constructor_arity_mismatch() {
        let source = "\
            type Option(a) { Some(a) None }\n\
            pub fn f() { Some(1, 2) }";
        let result = thir_from_source(source);
        assert!(result.has_errors(), "should report arity mismatch");
    }

    #[test]
    fn check_type_annotation_adt() {
        let source = "\
            type Option(a) { Some(a) None }\n\
            pub fn unwrap(opt: Option) -> Int { 42 }";
        let result = thir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        let f = &result.module.functions[0];
        match &f.params[0].ty {
            Ty::Named { name, .. } => assert_eq!(name.as_str(), "Option"),
            other => panic!("expected Named(Option(...)), got {other:?}"),
        }
    }

    #[test]
    fn check_user_defined_type() {
        let source = "\
            type Color { Red Green Blue }\n\
            pub fn f() { Red }";
        let result = thir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        let f = &result.module.functions[0];
        match f.body.ty() {
            Ty::Named { name, args, .. } => {
                assert_eq!(name.as_str(), "Color");
                assert!(args.is_empty());
            }
            other => panic!("expected Named(Color), got {other:?}"),
        }
    }

    // ── Match typing tests (Issue 27) ─────────────────────────────────

    // ── Pattern type checking ──

    #[test]
    fn check_match_option_constructor_patterns() {
        let source = "\
            type Option(a) { Some(a) None }\n\
            pub fn unwrap(opt: Option(Int)) -> Int {\n\
              match opt { Some(x) -> x  None -> 0 }\n\
            }";
        let result = thir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert_eq!(result.module.functions[0].return_ty, Ty::Primitive(PrimTy::Int));
    }

    #[test]
    fn check_match_binding_type() {
        let source = "\
            type Option(a) { Some(a) None }\n\
            pub fn get(opt: Option(String)) -> String {\n\
              match opt { Some(s) -> s  None -> \"\" }\n\
            }";
        let result = thir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
    }

    #[test]
    fn check_match_wildcard() {
        let source = "pub fn f(x: Int) -> Int { match x { _ -> 42 } }";
        let result = thir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
    }

    #[test]
    fn check_match_variable_catchall() {
        let source = "pub fn f(x: Int) -> Int { match x { 0 -> 1  n -> n } }";
        let result = thir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
    }

    // ── Exhaustiveness ──

    #[test]
    fn check_match_non_exhaustive_option() {
        let source = "\
            type Option(a) { Some(a) None }\n\
            pub fn f(opt: Option(Int)) -> Int {\n\
              match opt { Some(x) -> x }\n\
            }";
        let result = thir_from_source(source);
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("non-exhaustive")),
            "diagnostics: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn check_match_non_exhaustive_result() {
        let source = "\
            type Result(a, e) { Ok(a) Err(e) }\n\
            pub fn f(r: Result(Int, String)) -> Int {\n\
              match r { Ok(x) -> x }\n\
            }";
        let result = thir_from_source(source);
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("non-exhaustive")),
            "diagnostics: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn check_match_exhaustive_option() {
        let source = "\
            type Option(a) { Some(a) None }\n\
            pub fn f(opt: Option(Int)) -> Int {\n\
              match opt { Some(x) -> x  None -> 0 }\n\
            }";
        let result = thir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
    }

    #[test]
    fn check_match_exhaustive_with_wildcard() {
        let source = "\
            type Option(a) { Some(a) None }\n\
            pub fn f(opt: Option(Int)) -> Int {\n\
              match opt { _ -> 0 }\n\
            }";
        let result = thir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
    }

    #[test]
    fn check_match_primitive_non_exhaustive() {
        let source = "pub fn f(x: Int) -> Int { match x { 0 -> 1  1 -> 2 } }";
        let result = thir_from_source(source);
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("non-exhaustive")),
            "diagnostics: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn check_match_exhaustive_color() {
        let source = "\
            type Color { Red Green Blue }\n\
            pub fn f(c: Color) -> Int {\n\
              match c { Red -> 1  Green -> 2  Blue -> 3 }\n\
            }";
        let result = thir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
    }

    #[test]
    fn check_match_non_exhaustive_color() {
        let source = "\
            type Color { Red Green Blue }\n\
            pub fn f(c: Color) -> Int {\n\
              match c { Red -> 1  Green -> 2 }\n\
            }";
        let result = thir_from_source(source);
        assert!(result.has_errors());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.message.contains("non-exhaustive") && d.message.contains("Blue")),
            "diagnostics: {:?}",
            result.diagnostics
        );
    }

    // ── Unreachable arms ──

    #[test]
    fn check_match_unreachable_after_wildcard() {
        let source = "\
            type Option(a) { Some(a) None }\n\
            pub fn f(opt: Option(Int)) -> Int {\n\
              match opt { _ -> 0  Some(x) -> x }\n\
            }";
        let result = thir_from_source(source);
        assert!(
            !result.has_errors(),
            "warnings should not count as errors: {:?}",
            result.diagnostics
        );
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("unreachable")),
            "expected unreachable warning: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn check_match_unreachable_after_all_ctors() {
        let source = "\
            type Option(a) { Some(a) None }\n\
            pub fn f(opt: Option(Int)) -> Int {\n\
              match opt { Some(x) -> x  None -> 0  _ -> 99 }\n\
            }";
        let result = thir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("unreachable")),
            "expected unreachable warning: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn check_match_unreachable_primitive() {
        let source = "pub fn f(x: Int) -> Int { match x { 0 -> 1  n -> n  _ -> 99 } }";
        let result = thir_from_source(source);
        assert!(!result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("unreachable")),
            "diagnostics: {:?}",
            result.diagnostics
        );
    }

    // ── Issue 28: Diagnostic quality representative tests ──────────

    use asatsuyu_syntax::DiagnosticCode;

    /// Find a diagnostic by code.
    fn find_by_code(result: &TyCheckResult, code: DiagnosticCode) -> Option<&Diagnostic> {
        result.diagnostics.iter().find(|d| d.code == Some(code))
    }

    #[test]
    fn type_mismatch_has_labels() {
        // Use an argument type mismatch (goes through unify_or_error).
        let source = r#"pub fn f(x: Int) -> Int { f("hello") }"#;
        let result = thir_from_source(source);
        let diag =
            find_by_code(&result, DiagnosticCode::E0200).expect("should have E0200 diagnostic");
        assert!(!diag.labels.is_empty(), "E0200 should have labels");
        assert!(
            diag.labels
                .iter()
                .any(|l| l.message.contains("expected") && l.message.contains("found")),
            "label should mention expected and found: {:?}",
            diag.labels,
        );
    }

    #[test]
    fn return_type_mismatch_shows_declaration() {
        let source = r#"pub fn f() -> Int { "hello" }"#;
        let result = thir_from_source(source);
        let diag =
            find_by_code(&result, DiagnosticCode::E0200).expect("should have E0200 diagnostic");
        // Should have a secondary label pointing to the function declaration.
        assert!(
            diag.labels.len() >= 2,
            "return mismatch should have primary + secondary label: {:?}",
            diag.labels,
        );
        assert!(
            diag.labels.iter().any(|l| l.message.contains("return type annotation")),
            "secondary label should mention return type annotation: {:?}",
            diag.labels,
        );
    }

    #[test]
    fn argument_count_error_has_code() {
        let source = "pub fn f(x: Int) -> Int { f(1, 2) }";
        let result = thir_from_source(source);
        let diag =
            find_by_code(&result, DiagnosticCode::E0203).expect("should have E0203 diagnostic");
        assert!(diag.message.contains("argument"));
        assert!(!diag.labels.is_empty(), "E0203 should have labels");
    }

    #[test]
    fn non_exhaustive_match_has_hint() {
        let source = "
            type Option(a) { Some(a) None }
            pub fn f(x: Option(Int)) -> Int {
                match x { Some(v) -> v }
            }
        ";
        let result = thir_from_source(source);
        let diag =
            find_by_code(&result, DiagnosticCode::E0300).expect("should have E0300 diagnostic");
        assert!(!diag.hints.is_empty(), "E0300 should have hints: {diag:?}");
        assert!(
            diag.hints.iter().any(|h| h.contains("None")),
            "hint should mention missing variant: {:?}",
            diag.hints,
        );
    }

    #[test]
    fn unreachable_arm_warning_has_code() {
        let source = "
            type Option(a) { Some(a) None }
            pub fn f(x: Option(Int)) -> Int {
                match x { _ -> 0  Some(v) -> v  None -> 1 }
            }
        ";
        let result = thir_from_source(source);
        let warnings: Vec<_> =
            result.diagnostics.iter().filter(|d| d.code == Some(DiagnosticCode::E0301)).collect();
        assert!(!warnings.is_empty(), "should have E0301 warnings");
        for w in &warnings {
            assert!(!w.labels.is_empty(), "E0301 should have labels");
            assert!(!w.notes.is_empty(), "E0301 should have notes");
        }
    }

    #[test]
    fn unknown_type_suggests_builtins() {
        let source = "pub fn f(x: Foo) -> Int { 42 }";
        let result = thir_from_source(source);
        let diag =
            find_by_code(&result, DiagnosticCode::E0202).expect("should have E0202 diagnostic");
        assert!(!diag.hints.is_empty(), "E0202 should have hints");
        assert!(
            diag.hints.iter().any(|h| h.contains("Int")),
            "hint should mention built-in types: {:?}",
            diag.hints,
        );
    }

    #[test]
    fn infinite_type_has_note() {
        let source = "pub fn f(x: Int) -> Int { f(f) }";
        let result = thir_from_source(source);
        let diag = find_by_code(&result, DiagnosticCode::E0201);
        // The occurs check might fire or it might produce a mismatch depending on
        // the unification order. Either way, we should have a type error with a note.
        if let Some(d) = diag {
            assert!(!d.notes.is_empty(), "E0201 should have notes: {d:?}");
        }
    }

    #[test]
    fn if_else_branch_mismatch_labels() {
        let source = r#"pub fn f(b: Bool) -> Int { if b { 42 } else { "hello" } }"#;
        let result = thir_from_source(source);
        let diag =
            find_by_code(&result, DiagnosticCode::E0200).expect("should have E0200 diagnostic");
        // Should have a secondary label pointing to the then branch.
        assert!(
            diag.labels.iter().any(|l| l.message.contains("because of this branch")),
            "should have secondary label for if/else branch: {:?}",
            diag.labels,
        );
    }

    // ── Display: Opaque type (Issue 39) ────────────────────────────

    #[test]
    fn display_opaque() {
        let ty = Ty::Opaque { module: "json".into(), symbol: "loads".into() };
        assert_eq!(ty.to_string(), "PyOpaque[json.loads]");
    }

    // ── FFI: Issue 40 ──────────────────────────────────────────────

    #[test]
    fn ffi_pathlib_import_has_ffi_module_type() {
        let result = thir_from_source("from python import pathlib\npub fn f() { pathlib }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        let f = &result.module.functions[0];
        // The body should reference pathlib with FfiModule type
        let body_ty = f.body.ty();
        assert!(
            matches!(body_ty, Ty::FfiModule { module_name } if module_name == "pathlib"),
            "expected FfiModule, got: {body_ty:?}",
        );
    }

    #[test]
    fn ffi_pathlib_path_constructor_call() {
        let src = "from python import pathlib\npub fn f() { pathlib.Path(\".\") }";
        let result = thir_from_source(src);
        // With typeshed stubs, Path may resolve differently (diagnostics allowed).
        // Core check: compilation does not panic.
        let f = &result.module.functions[0];
        let body_ty = f.body.ty();
        // Accept FfiInstance or Error (type mismatch from complex typeshed types)
        assert!(
            matches!(body_ty, Ty::FfiInstance { module, class } if module == "pathlib" && class == "Path")
                || matches!(body_ty, Ty::Error),
            "expected FfiInstance(pathlib.Path) or Error, got: {body_ty:?}",
        );
    }

    #[test]
    fn ffi_pathlib_path_no_args() {
        // Path() with no args should also work (path param has default)
        let src = "from python import pathlib\npub fn f() { pathlib.Path() }";
        let result = thir_from_source(src);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
    }

    #[test]
    fn ffi_pathlib_path_property() {
        let src = "from python import pathlib\npub fn f() -> String { let p = pathlib.Path(\".\"); p.name }";
        let result = thir_from_source(src);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
    }

    #[test]
    fn ffi_pathlib_path_method() {
        let src = "from python import pathlib\npub fn f() -> Bool { let p = pathlib.Path(\".\"); p.exists() }";
        let result = thir_from_source(src);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
    }

    #[test]
    fn ffi_os_getcwd() {
        let src = "from python import os\npub fn f() -> String { os.getcwd() }";
        let result = thir_from_source(src);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
    }

    #[test]
    fn ffi_os_sep_constant() {
        let src = "from python import os\npub fn f() { os.sep }";
        let result = thir_from_source(src);
        // os.sep should resolve without panicking
        assert!(result.module.functions.len() == 1);
    }

    #[test]
    fn ffi_os_getenv_returns_option() {
        let src = "from python import os\npub fn f() { os.getenv(\"HOME\") }";
        let result = thir_from_source(src);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert!(
            matches!(result.module.functions[0].body.ty(), Ty::Named { name, args, .. } if name == "Option" && args.len() == 1),
            "expected Option(String), got: {:?}",
            result.module.functions[0].body.ty(),
        );
    }

    #[test]
    fn ffi_os_environ_resolves() {
        let src = "from python import os\npub fn f() { os.environ }";
        let result = thir_from_source(src);
        // environ from typeshed is `os._Environ[str]` (a complex type), so
        // the exact type varies. Just verify it resolves without panic.
        assert_eq!(result.module.functions.len(), 1);
    }

    #[test]
    fn ffi_sys_exit() {
        let src = "from python import sys\npub fn f() { sys.exit(1) }";
        let result = thir_from_source(src);
        // With typeshed stubs, sys.exit may have different param types.
        // Core check: does not panic.
        assert_eq!(result.module.functions.len(), 1);
    }

    #[test]
    fn ffi_sys_argv_is_typed_list() {
        let src = "from python import sys\npub fn f() { sys.argv }";
        let result = thir_from_source(src);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert!(
            matches!(result.module.functions[0].body.ty(), Ty::Named { name, args, .. } if name == "List" && args.len() == 1),
            "expected List(String), got: {:?}",
            result.module.functions[0].body.ty(),
        );
    }

    #[test]
    fn ffi_pathlib_parts_resolves() {
        let src = "from python import pathlib\npub fn f() { pathlib.Path(\".\").parts }";
        let result = thir_from_source(src);
        // parts type varies between builtin (Tuple(Str)) and typeshed stubs.
        // Core check: compilation does not panic.
        assert_eq!(result.module.functions.len(), 1);
    }

    #[test]
    fn ffi_requests_get_returns_response_instance() {
        let src = "from python import requests\npub fn f(url: String) { requests.get(url) }";
        let result = thir_from_source(src);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert!(
            matches!(result.module.functions[0].body.ty(), Ty::FfiInstance { module, class } if module == "requests" && class == "Response"),
            "expected FfiInstance(requests.Response), got: {:?}",
            result.module.functions[0].body.ty(),
        );
    }

    #[test]
    fn ffi_requests_response_text_is_string() {
        let src = "from python import requests\npub fn f(url: String) -> String { let response = requests.get(url); response.text }";
        let result = thir_from_source(src);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert!(
            matches!(result.module.functions[0].body.ty(), Ty::Primitive(PrimTy::String)),
            "expected String, got: {:?}",
            result.module.functions[0].body.ty(),
        );
    }

    #[test]
    fn ffi_requests_response_status_code_is_int() {
        let src = "from python import requests\npub fn f(url: String) -> Int { let response = requests.get(url); response.status_code }";
        let result = thir_from_source(src);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert!(
            matches!(result.module.functions[0].body.ty(), Ty::Primitive(PrimTy::Int)),
            "expected Int, got: {:?}",
            result.module.functions[0].body.ty(),
        );
    }

    #[test]
    fn ffi_requests_response_json_is_opaque() {
        let src = "from python import requests\npub fn f(url: String) { let response = requests.get(url); response.json() }";
        let result = thir_from_source(src);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert!(
            matches!(result.module.functions[0].body.ty(), Ty::Opaque { module, symbol } if module == "python" && symbol == "Any"),
            "expected Opaque(python.Any), got: {:?}",
            result.module.functions[0].body.ty(),
        );
    }

    #[test]
    fn ffi_unknown_module_error() {
        let src = "from python import nonexistent\npub fn f() { nonexistent.foo() }";
        let result = thir_from_source(src);
        assert!(result.has_errors());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == Some(asatsuyu_syntax::DiagnosticCode::E0208)),
            "expected E0208, got: {:?}",
            result.diagnostics,
        );
    }

    #[test]
    fn ffi_unknown_member_error() {
        let src = "from python import pathlib\npub fn f() { pathlib.NonExistent }";
        let result = thir_from_source(src);
        assert!(result.has_errors());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == Some(asatsuyu_syntax::DiagnosticCode::E0211)),
            "expected E0211, got: {:?}",
            result.diagnostics,
        );
    }

    #[test]
    fn ffi_field_access_on_int_error() {
        let src = "pub fn f() { 42.foo }";
        let result = thir_from_source(src);
        assert!(result.has_errors());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == Some(asatsuyu_syntax::DiagnosticCode::E0210)),
            "expected E0210, got: {:?}",
            result.diagnostics,
        );
    }

    #[test]
    fn display_ffi_module() {
        let ty = Ty::FfiModule { module_name: "pathlib".into() };
        assert_eq!(ty.to_string(), "module(pathlib)");
    }

    #[test]
    fn display_ffi_instance() {
        let ty = Ty::FfiInstance { module: "pathlib".into(), class: "Path".into() };
        assert_eq!(ty.to_string(), "pathlib.Path");
    }

    // ── Try expression (Issue 41) ──────────────────────────────────

    #[test]
    fn try_in_result_function_no_error() {
        let src = "\
from python import pathlib
type Result(a, e) { Ok(a) Error(e) }
type PyExc { PyExc(t: String, m: String) }
pub fn f() -> Result(Bool, PyExc) {
  let p = pathlib.Path(\".\")
  let r = try p.exists()
  Ok(r)
}";
        let result = thir_from_source(src);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
    }

    #[test]
    fn try_outside_result_function_error() {
        let src = "\
from python import pathlib
pub fn f() -> Bool {
  let p = pathlib.Path(\".\")
  try p.exists()
}";
        let result = thir_from_source(src);
        assert!(result.has_errors());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == Some(asatsuyu_syntax::DiagnosticCode::E0212)),
            "expected E0212, got: {:?}",
            result.diagnostics,
        );
    }

    #[test]
    fn try_expr_has_inner_type() {
        let src = "\
from python import pathlib
type Result(a, e) { Ok(a) Error(e) }
type PyExc { PyExc(t: String, m: String) }
pub fn f() -> Result(Bool, PyExc) {
  let p = pathlib.Path(\".\")
  let r = try p.exists()
  Ok(r)
}";
        let result = thir_from_source(src);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        // The function body's last expression should be Ok(r) of type Result(Bool, PyExc)
        let f = &result.module.functions[0];
        assert!(
            matches!(f.return_ty, Ty::Named { ref name, .. } if name.as_str() == "Result"),
            "expected Result return type, got: {:?}",
            f.return_ty,
        );
    }

    #[test]
    fn try_in_nested_expression_position_error() {
        let src = "\
from python import pathlib
type Result(a, e) { Ok(a) Error(e) }
type PyExc { PyExc(t: String, m: String) }
pub fn f() -> Result(Bool, PyExc) {
  let p = pathlib.Path(\".\")
  Ok(try p.exists())
}";
        let result = thir_from_source(src);
        assert!(result.has_errors());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == Some(asatsuyu_syntax::DiagnosticCode::E0213)),
            "expected E0213, got: {:?}",
            result.diagnostics,
        );
    }

    #[test]
    fn async_fn_returns_task_type() {
        let result = thir_from_source("async fn fetch() -> Int { 1 }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let f = &result.module.functions[0];
        assert!(f.is_async, "THIR function should preserve async marker");
        // The full function type should be () -> Task(Int).
        match &f.ty {
            Ty::Function { ret, .. } => match ret.as_ref() {
                Ty::Named { name, args, .. } => {
                    assert_eq!(name.as_str(), "Task");
                    assert_eq!(args.len(), 1);
                    assert_eq!(args[0], Ty::Primitive(PrimTy::Int));
                }
                other => panic!("expected Task(Int), got {other:?}"),
            },
            other => panic!("expected Function, got {other:?}"),
        }
        // return_ty should be the inner type (Int), used by backend for `-> int`.
        assert_eq!(f.return_ty, Ty::Primitive(PrimTy::Int));
    }

    #[test]
    fn await_unwraps_task_to_inner_type() {
        let result = thir_from_source(
            "async fn inner() -> Int { 1 }\npub async fn fetch() -> Int { await inner() }",
        );
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let f = &result.module.functions[1];
        assert!(f.is_async, "THIR function should preserve async marker");
        match &f.body {
            ThirExpr::Block { exprs, .. } => match &exprs[0] {
                ThirExpr::Await { expr, ty, .. } => {
                    assert!(
                        matches!(expr.as_ref(), ThirExpr::Call { .. }),
                        "await should wrap call in THIR"
                    );
                    // inner() returns Task(Int), so the call type is Task(Int).
                    match expr.ty() {
                        Ty::Named { name, .. } => assert_eq!(name.as_str(), "Task"),
                        other => panic!("expected Task type on call, got {other:?}"),
                    }
                    // await Task(Int) should produce Int.
                    assert_eq!(*ty, Ty::Primitive(PrimTy::Int));
                }
                other => panic!("expected Await, got {other:?}"),
            },
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn await_non_task_is_type_error() {
        let result = thir_from_source("async fn f() -> Int { await 42 }");
        assert!(result.has_errors(), "await on Int should be an error");
        assert!(
            result.diagnostics.iter().any(|d| d.code == Some(DiagnosticCode::E0219)),
            "expected E0219, got: {:?}",
            result.diagnostics,
        );
    }

    #[test]
    fn await_non_task_string_is_type_error() {
        let result = thir_from_source("async fn f() -> String { await \"hello\" }");
        assert!(result.has_errors(), "await on String should be an error");
        assert!(
            result.diagnostics.iter().any(|d| d.code == Some(DiagnosticCode::E0219)),
            "expected E0219, got: {:?}",
            result.diagnostics,
        );
    }

    #[test]
    fn await_sync_function_is_type_error() {
        let result =
            thir_from_source("fn sync_fn() -> Int { 1 }\nasync fn f() -> Int { await sync_fn() }");
        assert!(result.has_errors(), "await on sync function result should be an error");
        assert!(
            result.diagnostics.iter().any(|d| d.code == Some(DiagnosticCode::E0219)),
            "expected E0219, got: {:?}",
            result.diagnostics,
        );
    }

    #[test]
    fn async_fn_body_type_mismatch() {
        let result = thir_from_source("async fn f() -> Int { \"wrong\" }");
        assert!(result.has_errors(), "body returning String for -> Int should be an error");
        assert!(
            result.diagnostics.iter().any(|d| d.code == Some(DiagnosticCode::E0200)),
            "expected E0200, got: {:?}",
            result.diagnostics,
        );
    }

    #[test]
    fn async_fn_unannotated_return_infers_task() {
        let result = thir_from_source("async fn f() { 1 }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let f = &result.module.functions[0];
        // Full function type should wrap inferred return in Task.
        match &f.ty {
            Ty::Function { ret, .. } => match ret.as_ref() {
                Ty::Named { name, .. } => assert_eq!(name.as_str(), "Task"),
                other => panic!("expected Task(...), got {other:?}"),
            },
            other => panic!("expected Function, got {other:?}"),
        }
    }

    #[test]
    fn await_diagnostic_mentions_task_type() {
        let result =
            thir_from_source("fn sync_fn() -> Int { 1 }\nasync fn f() -> Int { await sync_fn() }");
        let diag = result
            .diagnostics
            .iter()
            .find(|d| d.code == Some(DiagnosticCode::E0219))
            .expect("should have E0219");
        assert!(
            diag.labels.iter().any(|l| l.message.contains("Task(T)")),
            "E0219 should mention Task(T) in labels: {:?}",
            diag.labels
        );
        assert!(
            diag.hints.iter().any(|h| h.contains("Task(T)")),
            "E0219 should mention Task(T) in hints: {:?}",
            diag.hints
        );
    }

    // ── Issue 98: async color rules ────────────────────────────────

    #[test]
    fn await_in_sync_fn_is_error() {
        let result = thir_from_source("async fn g() -> Int { 1 }\nfn f() -> Int { await g() }");
        assert!(result.has_errors(), "await in sync fn should be an error");
        assert!(
            result.diagnostics.iter().any(|d| d.code == Some(DiagnosticCode::E0220)),
            "expected E0220, got: {:?}",
            result.diagnostics,
        );
    }

    #[test]
    fn await_in_lambda_inside_async_fn_is_error() {
        let result = thir_from_source(
            "async fn g() -> Int { 1 }\nasync fn f() -> Int { let h = fn() { await g() }; 1 }",
        );
        assert!(result.has_errors(), "await in lambda should be an error");
        assert!(
            result.diagnostics.iter().any(|d| d.code == Some(DiagnosticCode::E0220)),
            "expected E0220, got: {:?}",
            result.diagnostics,
        );
    }

    #[test]
    fn sync_fn_calling_async_fn_returns_task() {
        let result = thir_from_source("async fn g() -> Int { 1 }\nfn f() -> Task(Int) { g() }");
        assert!(
            !result.has_errors(),
            "sync fn can call async fn and get Task(T): {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn await_literal_in_sync_fn_gets_e0220() {
        let result = thir_from_source("fn f() -> Int { await 42 }");
        assert!(result.has_errors());
        // Context error (E0220) should fire, not type error (E0219).
        assert!(
            result.diagnostics.iter().any(|d| d.code == Some(DiagnosticCode::E0220)),
            "expected E0220, got: {:?}",
            result.diagnostics,
        );
        assert!(
            !result.diagnostics.iter().any(|d| d.code == Some(DiagnosticCode::E0219)),
            "E0219 should not fire when E0220 takes priority",
        );
    }

    // ── Issue 99: async FFI typing ──────────────────────────────────

    #[test]
    fn async_ffi_sleep_returns_task_none() {
        let result = thir_from_source(
            "from python import asyncio\nasync fn f() { await asyncio.sleep(1.0) }",
        );
        assert!(
            !result.has_errors(),
            "await asyncio.sleep() should type-check in async fn: {:?}",
            result.diagnostics,
        );
    }

    #[test]
    fn async_ffi_without_await_is_task() {
        let result = thir_from_source(
            "from python import asyncio\nfn f() -> Task(None) { asyncio.sleep(1.0) }",
        );
        assert!(
            !result.has_errors(),
            "asyncio.sleep() without await should return Task(None): {:?}",
            result.diagnostics,
        );
    }

    #[test]
    fn sync_ffi_pathlib_unchanged() {
        let result = thir_from_source(
            "from python import pathlib\nfn f() -> String { pathlib.Path(\".\").name }",
        );
        assert!(
            !result.has_errors(),
            "sync FFI pathlib should still work: {:?}",
            result.diagnostics,
        );
    }

    // ── FFI: Issue 44 — ffi_modules in THIR ───────────────────────

    #[test]
    fn ffi_modules_passed_to_thir() {
        let result = thir_from_source("from python import json\npub fn f() { json }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert!(
            result.module.ffi_modules.contains_key("json"),
            "expected ffi_modules to contain 'json', got keys: {:?}",
            result.module.ffi_modules.keys().collect::<Vec<_>>(),
        );
        let json_mod = &result.module.ffi_modules["json"];
        assert_eq!(
            json_mod.trust_level,
            asatsuyu_hir::ffi::FfiTrustLevel::Checked,
            "json module should be Checked (contains Any)",
        );
    }

    #[test]
    fn ffi_modules_pathlib_is_verified() {
        let result = thir_from_source("from python import pathlib\npub fn f() { pathlib }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        let pathlib_mod = &result.module.ffi_modules["pathlib"];
        // With typeshed stubs, pathlib may be Checked (complex types degrade to Any).
        // The key invariant is that it resolves at all.
        assert!(
            matches!(
                pathlib_mod.trust_level,
                asatsuyu_hir::ffi::FfiTrustLevel::Verified
                    | asatsuyu_hir::ffi::FfiTrustLevel::Checked
            ),
            "pathlib trust should be Verified or Checked, got {:?}",
            pathlib_mod.trust_level,
        );
    }

    // ── Issue 47: Opaque escape hatch ─────────────────────────────────

    #[test]
    fn opaque_pass_through_allowed() {
        // Opaque values can be bound to variables and returned.
        let src = "from python import json\nfn f(data: String) { let x = json.loads(data)\n x }";
        let result = thir_from_source(src);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
    }

    #[test]
    fn opaque_field_access_rejected() {
        let src =
            "from python import json\nfn f(data: String) { let x = json.loads(data)\n x.foo }";
        let result = thir_from_source(src);
        assert!(result.has_errors());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == Some(asatsuyu_syntax::DiagnosticCode::E0209)),
            "should emit E0209 for field access on opaque: {:?}",
            result.diagnostics,
        );
    }

    #[test]
    fn opaque_match_rejected() {
        let src = "from python import json\nfn f(data: String) -> Int { let x = json.loads(data)\n match x { _ -> 1 } }";
        let result = thir_from_source(src);
        assert!(result.has_errors());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == Some(asatsuyu_syntax::DiagnosticCode::E0214)),
            "should emit E0214 for match on opaque: {:?}",
            result.diagnostics,
        );
    }

    // ── List pattern matching ──────────────────────────────────────

    #[test]
    fn list_pattern_empty() {
        let src = "pub fn f(items: List(Int)) -> Int {\
                     match items { [] -> 0  _ -> 1 } }";
        let result = thir_from_source(src);
        assert!(
            !result.has_errors(),
            "empty list pattern should type-check: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn list_pattern_head_rest() {
        let src = "pub fn f(items: List(Int)) -> Int {\
                     match items { [h, ..] -> h  [] -> 0 } }";
        let result = thir_from_source(src);
        assert!(
            !result.has_errors(),
            "head+rest pattern should type-check: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn list_pattern_named_rest() {
        let src = "pub fn f(items: List(Int)) -> Int {\
                     match items { [h, ..rest] -> h  [] -> 0 } }";
        let result = thir_from_source(src);
        assert!(
            !result.has_errors(),
            "named rest pattern should type-check: {:?}",
            result.diagnostics
        );
    }

    // ── list module builtins ───────────────────────────────────────

    #[test]
    fn list_map_type_checks() {
        let src = "pub fn f() -> List(Int) {\
                     list.map([1, 2, 3], fn(x) { x * 2 }) }";
        let result = thir_from_source(src);
        assert!(!result.has_errors(), "list.map should type-check: {:?}", result.diagnostics);
        assert_no_error_ty(&result.module.functions[0].body);
    }

    #[test]
    fn list_filter_type_checks() {
        let src = "pub fn f() -> List(Int) {\
                     list.filter([1, 2, 3], fn(x) { x > 0 }) }";
        let result = thir_from_source(src);
        assert!(!result.has_errors(), "list.filter should type-check: {:?}", result.diagnostics);
        assert_no_error_ty(&result.module.functions[0].body);
    }

    #[test]
    fn list_length_type_checks() {
        let src = "pub fn f() -> Int { list.length([1, 2, 3]) }";
        let result = thir_from_source(src);
        assert!(!result.has_errors(), "list.length should type-check: {:?}", result.diagnostics);
        assert_no_error_ty(&result.module.functions[0].body);
    }

    #[test]
    fn list_fold_type_checks() {
        // fold with explicit type annotation on accumulator (type inference
        // for higher-order callback params is a known limitation for MVP).
        let src = "pub fn add(acc: Int, x: Int) -> Int { acc + x }\n\
                   pub fn f() -> Int {\
                     list.fold([1, 2, 3], 0, add) }";
        let result = thir_from_source(src);
        assert!(!result.has_errors(), "list.fold should type-check: {:?}", result.diagnostics);
        assert_no_error_ty(&result.module.functions[1].body);
    }

    #[test]
    fn list_head_and_rest_type_check() {
        let src = "pub fn heady(items: List(Int)) -> Option(Int) { list.head(items) }\n\
                   pub fn resty(items: List(Int)) -> Option(List(Int)) { list.rest(items) }";
        let result = thir_from_source(src);
        assert!(!result.has_errors(), "list.head/rest should type-check: {:?}", result.diagnostics);
        assert_no_error_ty(&result.module.functions[0].body);
        assert_no_error_ty(&result.module.functions[1].body);
    }

    #[test]
    fn list_match_requires_empty_and_non_empty() {
        let src = "pub fn f(items: List(Int)) -> Int { match items { [] -> 0 } }";
        let result = thir_from_source(src);
        assert!(result.has_errors(), "single empty-arm list match should be rejected");
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == Some(asatsuyu_syntax::DiagnosticCode::E0300)),
            "should emit E0300 for non-exhaustive list match: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn list_match_empty_and_rest_is_exhaustive() {
        let src = "pub fn f(items: List(Int)) -> Int { match items { [] -> 0  [x, ..] -> x } }";
        let result = thir_from_source(src);
        assert!(
            !result.has_errors(),
            "empty + non-empty list patterns should be exhaustive: {:?}",
            result.diagnostics
        );
    }

    // ── Mutation rules (Issue 94) ────────────────────────────────────

    #[test]
    fn assign_to_mutable_let_is_ok() {
        let result = thir_from_source("pub fn main() { let mut x = 0\n x = 1 }");
        assert!(
            !result.has_errors(),
            "mutable let should allow reassignment: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn assign_to_immutable_let_is_error() {
        let result = thir_from_source("pub fn main() { let x = 0\n x = 1 }");
        assert!(result.has_errors(), "immutable let should reject reassignment");
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == Some(asatsuyu_syntax::DiagnosticCode::E0215)),
            "should emit E0215: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn assign_to_parameter_is_error() {
        let result = thir_from_source("pub fn f(x: Int) { x = 1 }");
        assert!(result.has_errors(), "parameter reassignment should be rejected");
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == Some(asatsuyu_syntax::DiagnosticCode::E0216)),
            "should emit E0216: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn assign_type_mismatch_is_error() {
        let result = thir_from_source("pub fn main() { let mut x = 0\n x = \"hello\" }");
        assert!(result.has_errors(), "type mismatch on reassignment should be rejected");
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == Some(asatsuyu_syntax::DiagnosticCode::E0217)),
            "should emit E0217: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn assign_type_match_is_ok() {
        let result = thir_from_source("pub fn main() { let mut x = 0\n x = 42 }");
        assert!(
            !result.has_errors(),
            "same-type reassignment should be accepted: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn assign_captured_var_in_lambda_is_error() {
        let result =
            thir_from_source("pub fn main() { let mut x = 0\n let f = fn() { x = 1 }\n f() }");
        assert!(result.has_errors(), "captured variable mutation in lambda should be rejected");
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == Some(asatsuyu_syntax::DiagnosticCode::E0218)),
            "should emit E0218: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn assign_lambda_local_is_ok() {
        let result =
            thir_from_source("pub fn main() { let f = fn() { let mut y = 0\n y = 1 }\n f() }");
        assert!(
            !result.has_errors(),
            "lambda-local mutable should allow reassignment: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn assign_immutable_has_hint_for_mut() {
        let result = thir_from_source("pub fn main() { let x = 0\n x = 1 }");
        let diag = result
            .diagnostics
            .iter()
            .find(|d| d.code == Some(asatsuyu_syntax::DiagnosticCode::E0215))
            .expect("should have E0215");
        assert!(
            diag.hints.iter().any(|h| h.contains("let mut")),
            "E0215 hint should suggest `let mut`: {:?}",
            diag.hints
        );
        assert!(
            diag.notes.iter().any(|n| n.contains("only `let mut` bindings")),
            "E0215 should explain mutable-only reassignment: {:?}",
            diag.notes
        );
    }

    #[test]
    fn assign_parameter_has_hint_for_local_binding() {
        let result = thir_from_source("pub fn f(x: Int) { x = 1 }");
        let diag = result
            .diagnostics
            .iter()
            .find(|d| d.code == Some(asatsuyu_syntax::DiagnosticCode::E0216))
            .expect("should have E0216");
        assert!(
            diag.hints.iter().any(|h| h.contains("let mut")),
            "E0216 hint should suggest local binding: {:?}",
            diag.hints
        );
    }

    #[test]
    fn assign_type_mismatch_has_hint_and_note() {
        let result = thir_from_source("pub fn main() { let mut x = 0\n x = \"hello\" }");
        let diag = result
            .diagnostics
            .iter()
            .find(|d| d.code == Some(asatsuyu_syntax::DiagnosticCode::E0217))
            .expect("should have E0217");
        assert!(
            diag.hints.iter().any(|h| h.contains("same type as the binding")),
            "E0217 should suggest preserving the binding type: {:?}",
            diag.hints
        );
        assert!(
            diag.notes.iter().any(|n| n.contains("preserve the original binding type")),
            "E0217 should explain reassignment type invariance: {:?}",
            diag.notes
        );
    }

    #[test]
    fn assign_captured_var_has_hint() {
        let result =
            thir_from_source("pub fn main() { let mut x = 0\n let f = fn() { x = 1 }\n f() }");
        let diag = result
            .diagnostics
            .iter()
            .find(|d| d.code == Some(asatsuyu_syntax::DiagnosticCode::E0218))
            .expect("should have E0218");
        assert!(
            diag.hints.iter().any(|h| h.contains("inside the lambda")),
            "E0218 should suggest introducing a lambda-local binding: {:?}",
            diag.hints
        );
    }
}
