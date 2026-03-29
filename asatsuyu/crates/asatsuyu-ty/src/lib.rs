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

pub use types::{PrimTy, ThirExpr, ThirFnDef, ThirLiteral, ThirModule, ThirParam, Ty, TyVarId};

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
}
