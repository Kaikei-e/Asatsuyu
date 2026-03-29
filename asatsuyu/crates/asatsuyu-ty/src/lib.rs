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
    PrimTy, ThirExpr, ThirFnDef, ThirLiteral, ThirMatchArm, ThirModule, ThirParam, Ty, TyVarId,
};

use asatsuyu_hir::HirModule;
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
    let mut ctx = check::TyCheckCtx::new();
    ctx.collect_signatures(hir);
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

        let add = &result.module.functions[1];
        assert_eq!(add.return_ty, Ty::Primitive(PrimTy::Int));
        assert_eq!(add.params.len(), 2);
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
}
