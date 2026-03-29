//! High-level intermediate representation (HIR) for the Asatsuyu language.
//!
//! Performs name resolution: variable references are resolved to [`DefId`]s
//! via a [`SymbolTable`]. Desugaring of pipeline and string concatenation
//! operators will be added in Issue 21.
//!
//! # Usage
//!
//! ```
//! use asatsuyu_ast::lower;
//! use asatsuyu_hir::lower_to_hir;
//! use asatsuyu_parser::parse;
//! use asatsuyu_syntax::FileId;
//!
//! let cst = parse(FileId(0), "pub fn main() { 42 }");
//! let ast = lower(&cst, FileId(0));
//! let hir = lower_to_hir(&ast.module);
//! assert!(!hir.has_errors());
//! assert_eq!(hir.module.functions.len(), 1);
//! ```

mod lower;
mod types;

pub use types::{
    DefData, DefId, DefKind, HirExpr, HirFnDef, HirLiteral, HirModule, HirParam, SymbolTable,
};

use asatsuyu_ast::Module;
use asatsuyu_syntax::{Diagnostic, Severity};

/// The result of lowering an AST into HIR.
#[derive(Debug)]
pub struct HirLowerResult {
    /// The lowered HIR module with name resolution.
    pub module: HirModule,
    /// Diagnostics collected during lowering (e.g., unresolved names).
    pub diagnostics: Vec<Diagnostic>,
}

impl HirLowerResult {
    /// Returns `true` if any error-level diagnostic was emitted during HIR lowering.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.severity == Severity::Error)
    }
}

/// Lower an AST module into HIR with name resolution.
///
/// Registers all definitions in a [`SymbolTable`] and resolves variable
/// references to [`DefId`]s. Unresolved names produce diagnostics but
/// lowering continues with dummy definitions.
#[must_use]
pub fn lower_to_hir(ast: &Module) -> HirLowerResult {
    let mut ctx = lower::HirLowerCtx::new();
    let mut module = ctx.lower_module(ast);
    let (symbol_table, diagnostics) = ctx.into_parts();
    module.symbol_table = symbol_table;
    HirLowerResult { module, diagnostics }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asatsuyu_ast::{LiteralKind, Visibility};
    use asatsuyu_parser::parse;
    use asatsuyu_syntax::FileId;

    const FID: FileId = FileId(0);

    /// Helper: parse + AST lower + HIR lower.
    fn hir_from_source(source: &str) -> HirLowerResult {
        let cst = parse(FID, source);
        let ast = asatsuyu_ast::lower(&cst, FID);
        lower_to_hir(&ast.module)
    }

    // ── 1. Empty module ─────────────────────────────────────────────

    #[test]
    fn lower_empty_module() {
        let result = hir_from_source("");
        assert!(!result.has_errors());
        assert!(result.module.functions.is_empty());
        assert!(result.module.symbol_table.is_empty());
    }

    // ── 2. Minimal function ─────────────────────────────────────────

    #[test]
    fn lower_minimal_function() {
        let result = hir_from_source("pub fn main() { 42 }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert_eq!(result.module.functions.len(), 1);

        let f = &result.module.functions[0];
        assert_eq!(result.module.symbol_table.get(f.def_id).name.as_str(), "main");
        assert_eq!(result.module.symbol_table.get(f.def_id).kind, DefKind::Function);
        assert_eq!(f.visibility, Visibility::Public);
        assert!(f.params.is_empty());

        match &f.body {
            HirExpr::Block { exprs, .. } => {
                assert_eq!(exprs.len(), 1);
                match &exprs[0] {
                    HirExpr::Literal(lit) => {
                        assert_eq!(lit.kind, LiteralKind::Int);
                        assert_eq!(lit.value.as_str(), "42");
                    }
                    other => panic!("expected Literal, got {other:?}"),
                }
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    // ── 3. Private function ─────────────────────────────────────────

    #[test]
    fn lower_private_function() {
        let result = hir_from_source("fn main() { 42 }");
        assert!(!result.has_errors());
        assert_eq!(result.module.functions[0].visibility, Visibility::Private);
    }

    // ── 4. Parameter reference resolves ─────────────────────────────

    #[test]
    fn lower_parameter_reference() {
        let result = hir_from_source("fn id(x: Int) -> Int { x }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let f = &result.module.functions[0];
        assert_eq!(f.params.len(), 1);

        let param_def_id = f.params[0].def_id;
        assert_eq!(result.module.symbol_table.get(param_def_id).name.as_str(), "x");
        assert_eq!(result.module.symbol_table.get(param_def_id).kind, DefKind::Parameter);

        // Body should reference the same DefId as the parameter.
        match &f.body {
            HirExpr::Block { exprs, .. } => match &exprs[0] {
                HirExpr::Var(def_id, _) => {
                    assert_eq!(*def_id, param_def_id, "variable should resolve to parameter DefId");
                }
                other => panic!("expected Var, got {other:?}"),
            },
            other => panic!("expected Block, got {other:?}"),
        }
    }

    // ── 5. Multiple parameters ──────────────────────────────────────

    #[test]
    fn lower_multiple_params() {
        let result = hir_from_source("fn add(x: Int, y: Int) { x }");
        assert!(!result.has_errors());

        let f = &result.module.functions[0];
        assert_eq!(f.params.len(), 2);
        assert_eq!(f.params[0].type_ann.as_str(), "Int");
        assert_eq!(f.params[1].type_ann.as_str(), "Int");

        // x should resolve to the first parameter.
        let x_def_id = f.params[0].def_id;
        match &f.body {
            HirExpr::Block { exprs, .. } => match &exprs[0] {
                HirExpr::Var(def_id, _) => assert_eq!(*def_id, x_def_id),
                other => panic!("expected Var, got {other:?}"),
            },
            other => panic!("expected Block, got {other:?}"),
        }
    }

    // ── 6. Return type preserved ────────────────────────────────────

    #[test]
    fn lower_return_type() {
        let result = hir_from_source("fn id(x: Int) -> Int { x }");
        assert!(!result.has_errors());
        assert_eq!(result.module.functions[0].return_type.as_deref(), Some("Int"));
    }

    // ── 7. Cross-function reference ─────────────────────────────────

    #[test]
    fn lower_cross_function_reference() {
        let result = hir_from_source("fn a() { b }\nfn b() { 1 }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert_eq!(result.module.functions.len(), 2);

        let b_fn_def_id = result.module.functions[1].def_id;

        // In function `a`, the body references `b` which should resolve to function b's DefId.
        let a = &result.module.functions[0];
        match &a.body {
            HirExpr::Block { exprs, .. } => match &exprs[0] {
                HirExpr::Var(def_id, _) => {
                    assert_eq!(*def_id, b_fn_def_id, "`b` should resolve to function b's DefId");
                }
                other => panic!("expected Var, got {other:?}"),
            },
            other => panic!("expected Block, got {other:?}"),
        }
    }

    // ── 8. Unresolved name produces diagnostic ──────────────────────

    #[test]
    fn lower_unresolved_name() {
        let result = hir_from_source("fn f() { unknown }");
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("unresolved name `unknown`")),
            "expected unresolved name diagnostic: {:?}",
            result.diagnostics
        );
        // Should still produce the function (with a dummy DefId for `unknown`).
        assert_eq!(result.module.functions.len(), 1);
    }

    // ── 9. String literal ───────────────────────────────────────────

    #[test]
    fn lower_string_literal() {
        let result = hir_from_source(r#"fn greet() { "hello" }"#);
        assert!(!result.has_errors());

        let f = &result.module.functions[0];
        match &f.body {
            HirExpr::Block { exprs, .. } => match &exprs[0] {
                HirExpr::Literal(lit) => {
                    assert_eq!(lit.kind, LiteralKind::String);
                    assert_eq!(lit.value.as_str(), "\"hello\"");
                }
                other => panic!("expected Literal, got {other:?}"),
            },
            other => panic!("expected Block, got {other:?}"),
        }
    }

    // ── 10. All nodes have non-empty Span ───────────────────────────

    #[test]
    fn all_nodes_have_span() {
        let result = hir_from_source("pub fn add(x: Int) -> Int { x }");
        assert!(!result.has_errors());

        let f = &result.module.functions[0];

        assert!(!result.module.span.is_empty());
        assert!(!f.span.is_empty());
        assert!(!f.params[0].span.is_empty());
        assert!(!f.body.span().is_empty());
    }

    // ── 11. hello.asty ──────────────────────────────────────────────

    #[test]
    fn lower_hello_asty() {
        let source = include_str!("../../../examples/hello.asty");
        let cst = parse(FID, source);
        let ast = asatsuyu_ast::lower(&cst, FID);
        let result = lower_to_hir(&ast.module);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert_eq!(result.module.functions.len(), 1);
        assert_eq!(
            result.module.symbol_table.get(result.module.functions[0].def_id).name.as_str(),
            "main"
        );
    }

    // ── 12. greet.asty ──────────────────────────────────────────────

    #[test]
    fn lower_greet_asty() {
        let source = include_str!("../../../examples/greet.asty");
        let cst = parse(FID, source);
        let ast = asatsuyu_ast::lower(&cst, FID);
        let result = lower_to_hir(&ast.module);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert_eq!(result.module.functions.len(), 2);

        let greet = &result.module.functions[0];
        assert_eq!(greet.visibility, Visibility::Public);
        assert_eq!(greet.params.len(), 1);
        assert!(greet.return_type.is_some());

        let add = &result.module.functions[1];
        assert_eq!(add.visibility, Visibility::Private);
        assert_eq!(add.params.len(), 2);
    }

    // ── 13. HIR dump (DoD) ──────────────────────────────────────────

    #[test]
    fn hir_dump() {
        let result = hir_from_source("pub fn main() { 42 }");
        let dump = format!("{:#?}", result.module);
        assert!(dump.contains("main"), "dump should contain function name:\n{dump}");
        assert!(dump.contains("Function"), "dump should contain DefKind::Function:\n{dump}");
        assert!(dump.contains("Int"), "dump should contain literal kind:\n{dump}");
    }

    // ── 14. DefId uniqueness ────────────────────────────────────────

    #[test]
    fn def_id_uniqueness() {
        // Two functions with same-named parameters should get different DefIds.
        let result = hir_from_source("fn f(x: Int) { x }\nfn g(x: Int) { x }");
        assert!(!result.has_errors());

        let f_param = result.module.functions[0].params[0].def_id;
        let g_param = result.module.functions[1].params[0].def_id;
        assert_ne!(
            f_param, g_param,
            "same-named params in different functions should have different DefIds"
        );

        // Both functions should also have different DefIds.
        let f_fn = result.module.functions[0].def_id;
        let g_fn = result.module.functions[1].def_id;
        assert_ne!(f_fn, g_fn);
    }
}
