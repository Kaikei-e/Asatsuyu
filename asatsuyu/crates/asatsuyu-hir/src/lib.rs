//! High-level intermediate representation (HIR) for the Asatsuyu language.
//!
//! Performs name resolution: variable references are resolved to [`DefId`]s
//! via a [`SymbolTable`] and a lexical scope stack. Desugaring of pipeline
//! and string concatenation operators will be added in Issue 21.
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

pub mod ffi;
mod lower;
mod types;

pub use types::{
    DefData, DefId, DefKind, HirCustomType, HirExpr, HirFieldType, HirFnDef, HirImport,
    HirImportKind, HirLiteral, HirMatchArm, HirModule, HirParam, HirPattern, HirTypeExpr,
    HirVariant, SymbolTable,
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
/// references to [`DefId`]s using lexical scoping. Unresolved names and
/// duplicate top-level definitions produce diagnostics but lowering
/// continues.
#[must_use]
pub fn lower_to_hir(ast: &Module) -> HirLowerResult {
    let mut ctx = lower::HirLowerCtx::new();
    let module = ctx.lower_module(ast);
    let diagnostics = ctx.into_diagnostics();
    HirLowerResult { module, diagnostics }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asatsuyu_ast::{LiteralKind, Visibility};
    use asatsuyu_parser::parse;
    use asatsuyu_syntax::FileId;
    use smol_str::SmolStr;

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
        // Symbol table contains built-in definitions (string_concat, println, list, option).
        assert_eq!(result.module.symbol_table.len(), 4);
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
        assert_eq!(f.params[0].type_ann.as_ref().unwrap().name.as_str(), "Int");
        assert_eq!(f.params[1].type_ann.as_ref().unwrap().name.as_str(), "Int");

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
        assert_eq!(result.module.functions[0].return_type.as_ref().unwrap().name.as_str(), "Int");
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

    #[test]
    fn duplicate_parameter_binding_diagnostic() {
        let result = hir_from_source("fn f(x: Int, x: Int) { x }");
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("duplicate binding `x`")),
            "expected duplicate binding diagnostic: {:?}",
            result.diagnostics
        );
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

        let main = &result.module.functions[1];
        assert_eq!(main.visibility, Visibility::Public);
        assert_eq!(main.params.len(), 0);
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

    #[test]
    fn hir_dump_shows_mutable_binding_and_assign() {
        let result = hir_from_source("pub fn main() { let mut x = 0\n x = 1 }");
        assert!(!result.has_errors(), "unexpected diagnostics: {:?}", result.diagnostics);

        let dump = format!("{:#?}", result.module);
        assert!(dump.contains("is_mutable: true"), "dump should show mutable binding:\n{dump}");
        assert!(dump.contains("Assign"), "dump should show Assign node:\n{dump}");

        let binding = result
            .module
            .symbol_table
            .iter()
            .find(|(_, def)| def.name.as_str() == "x")
            .map(|(_, def)| def)
            .expect("expected local binding x");
        assert!(binding.is_mutable, "symbol table should record mutable binding");
    }

    #[test]
    fn hir_propagates_async_fn_and_await() {
        let result = hir_from_source(
            "fn inner() -> Int { 1 }\npub async fn fetch() -> Int { await inner() }",
        );
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let func = &result.module.functions[1];
        assert!(func.is_async, "HIR function should preserve async marker");

        match &func.body {
            HirExpr::Block { exprs, .. } => match &exprs[0] {
                HirExpr::Await { expr, .. } => {
                    assert!(
                        matches!(expr.as_ref(), HirExpr::Call { .. }),
                        "await should wrap call in HIR"
                    );
                }
                other => panic!("expected Await, got {other:?}"),
            },
            other => panic!("expected Block, got {other:?}"),
        }
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

    // ═══════════════════════════════════════════════════════════════
    // Issue 20: New tests for scoping, patterns, and expression lowering
    // ═══════════════════════════════════════════════════════════════

    // ── 15. Parameter shadows function name ─────────────────────────

    #[test]
    fn scope_parameter_shadows_function() {
        // Parameter `f` should shadow the function `f` in the body.
        let result = hir_from_source("fn f(f: Int) -> Int { f }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let func = &result.module.functions[0];
        let param_def_id = func.params[0].def_id;

        match &func.body {
            HirExpr::Block { exprs, .. } => match &exprs[0] {
                HirExpr::Var(def_id, _) => {
                    assert_eq!(
                        *def_id, param_def_id,
                        "body `f` should resolve to parameter, not function"
                    );
                }
                other => panic!("expected Var, got {other:?}"),
            },
            other => panic!("expected Block, got {other:?}"),
        }
    }

    // ── 16. Call expression lowering ────────────────────────────────

    #[test]
    fn lower_call_expr() {
        let result = hir_from_source("fn f(x: Int) -> Int { f(1) }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let func = &result.module.functions[0];
        match &func.body {
            HirExpr::Block { exprs, .. } => match &exprs[0] {
                HirExpr::Call { func: callee, args, .. } => {
                    assert!(matches!(callee.as_ref(), HirExpr::Var(..)));
                    assert_eq!(args.len(), 1);
                }
                other => panic!("expected Call, got {other:?}"),
            },
            other => panic!("expected Block, got {other:?}"),
        }
    }

    // ── 17. Binary operation lowering ───────────────────────────────

    #[test]
    fn lower_binary_op() {
        let result = hir_from_source("fn f() -> Int { 1 + 2 }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let func = &result.module.functions[0];
        match &func.body {
            HirExpr::Block { exprs, .. } => {
                assert!(matches!(&exprs[0], HirExpr::BinaryOp { .. }));
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    // ── 18. Unary operation lowering ────────────────────────────────

    #[test]
    fn lower_unary_op() {
        let result = hir_from_source("fn f(x: Int) -> Int { -x }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let func = &result.module.functions[0];
        match &func.body {
            HirExpr::Block { exprs, .. } => {
                assert!(matches!(&exprs[0], HirExpr::UnaryOp { .. }));
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    // ── 19. If expression lowering ──────────────────────────────────

    #[test]
    fn lower_if_expr() {
        let result = hir_from_source("fn f(x: Int) -> Int { if x { 1 } else { 2 } }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let func = &result.module.functions[0];
        match &func.body {
            HirExpr::Block { exprs, .. } => match &exprs[0] {
                HirExpr::If { else_body, .. } => {
                    assert!(else_body.is_some(), "else branch should exist");
                }
                other => panic!("expected If, got {other:?}"),
            },
            other => panic!("expected Block, got {other:?}"),
        }
    }

    // ── 20. Pipeline lowering ───────────────────────────────────────

    #[test]
    fn desugar_pipeline_bare() {
        // x |> f → f(x)
        let result = hir_from_source("fn f(x: Int) -> Int { x |> f }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let func = &result.module.functions[0];
        match &func.body {
            HirExpr::Block { exprs, .. } => match &exprs[0] {
                HirExpr::Call { func: callee, args, .. } => {
                    assert!(matches!(callee.as_ref(), HirExpr::Var(..)), "callee should be Var");
                    assert_eq!(args.len(), 1, "pipeline bare: f(x) has 1 arg");
                    assert!(matches!(&args[0], HirExpr::Var(..)));
                }
                other => panic!("expected Call (desugared pipeline), got {other:?}"),
            },
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn desugar_pipeline_with_args() {
        // x |> f(1) → f(x, 1)
        let result = hir_from_source("fn f(x: Int) -> Int { x |> f(1) }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let func = &result.module.functions[0];
        match &func.body {
            HirExpr::Block { exprs, .. } => match &exprs[0] {
                HirExpr::Call { args, .. } => {
                    assert_eq!(args.len(), 2, "pipeline with args: f(x, 1) has 2 args");
                }
                other => panic!("expected Call (desugared pipeline), got {other:?}"),
            },
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn desugar_pipeline_chain() {
        // x |> f |> g → g(f(x))
        let result = hir_from_source("fn test(x: Int) -> Int { x |> f |> g }");
        // f and g are unresolved, but structure should be correct
        let func = &result.module.functions[0];
        match &func.body {
            HirExpr::Block { exprs, .. } => match &exprs[0] {
                HirExpr::Call { func: outer_callee, args: outer_args, .. } => {
                    // Outer: g(...)
                    assert!(matches!(outer_callee.as_ref(), HirExpr::Var(..)));
                    assert_eq!(outer_args.len(), 1);
                    // Inner: f(x)
                    match &outer_args[0] {
                        HirExpr::Call { args: inner_args, .. } => {
                            assert_eq!(inner_args.len(), 1);
                        }
                        other => panic!("expected inner Call, got {other:?}"),
                    }
                }
                other => panic!("expected Call (desugared chain), got {other:?}"),
            },
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn desugar_string_concat() {
        // "a" <> "b" → string_concat("a", "b")
        let result = hir_from_source("fn f() -> String { \"a\" <> \"b\" }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let func = &result.module.functions[0];
        match &func.body {
            HirExpr::Block { exprs, .. } => match &exprs[0] {
                HirExpr::Call { func: callee, args, .. } => {
                    // Callee should be the built-in string_concat
                    if let HirExpr::Var(def_id, _) = callee.as_ref() {
                        let data = result.module.symbol_table.get(*def_id);
                        assert_eq!(data.name.as_str(), "string_concat");
                        assert_eq!(data.kind, DefKind::Builtin);
                    } else {
                        panic!("expected Var callee, got {callee:?}");
                    }
                    assert_eq!(args.len(), 2);
                }
                other => panic!("expected Call (desugared string concat), got {other:?}"),
            },
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn desugar_string_concat_chain() {
        // "a" <> b <> "c" → string_concat(string_concat("a", b), "c")
        let result = hir_from_source("fn f(b: String) -> String { \"a\" <> b <> \"c\" }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let func = &result.module.functions[0];
        match &func.body {
            HirExpr::Block { exprs, .. } => match &exprs[0] {
                HirExpr::Call { args, .. } => {
                    assert_eq!(args.len(), 2);
                    // First arg should be another Call (nested string_concat)
                    assert!(matches!(&args[0], HirExpr::Call { .. }), "nested concat");
                }
                other => panic!("expected Call, got {other:?}"),
            },
            other => panic!("expected Block, got {other:?}"),
        }
    }

    // ── 21. Match with pattern binding ──────────────────────────────

    #[test]
    fn match_pattern_binding() {
        let source = "type Option(a) {\n  Some(a)\n  None\n}\n\
                       fn f(x: Int) -> Int {\n  match x {\n    0 -> 0\n    y -> y\n  }\n}";
        let result = hir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let func = &result.module.functions[0];
        match &func.body {
            HirExpr::Block { exprs, .. } => match &exprs[0] {
                HirExpr::Match { arms, .. } => {
                    assert_eq!(arms.len(), 2);
                    // Second arm binds `y` and uses it
                    match &arms[1].pattern {
                        HirPattern::Variable(def_id, _) => {
                            let data = result.module.symbol_table.get(*def_id);
                            assert_eq!(data.name.as_str(), "y");
                            assert_eq!(data.kind, DefKind::LocalBinding);
                        }
                        other => panic!("expected Variable pattern, got {other:?}"),
                    }
                    // Body should reference the same DefId
                    match &arms[1].body {
                        HirExpr::Var(body_def_id, _) => {
                            if let HirPattern::Variable(pat_def_id, _) = &arms[1].pattern {
                                assert_eq!(body_def_id, pat_def_id);
                            }
                        }
                        other => panic!("expected Var, got {other:?}"),
                    }
                }
                other => panic!("expected Match, got {other:?}"),
            },
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_pattern_binding_diagnostic() {
        let result = hir_from_source("fn f(xs: Int) { match xs { [x, ..x] -> x } }");
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("duplicate binding `x`")),
            "expected duplicate binding diagnostic: {:?}",
            result.diagnostics
        );
    }

    // ── 22. Match arm scope isolation ────────────────────────────────

    #[test]
    fn match_arm_scope_isolation() {
        // Each arm's binding should have a distinct DefId even if same name.
        let source = "fn f(x: Int) -> Int {\n  match x {\n    y -> y\n    y -> y\n  }\n}";
        let result = hir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let func = &result.module.functions[0];
        if let HirExpr::Block { exprs, .. } = &func.body
            && let HirExpr::Match { arms, .. } = &exprs[0]
            && let (HirPattern::Variable(id1, _), HirPattern::Variable(id2, _)) =
                (&arms[0].pattern, &arms[1].pattern)
        {
            assert_ne!(id1, id2, "arm bindings should have distinct DefIds");
        }
    }

    // ── 23. Wildcard pattern ────────────────────────────────────────

    #[test]
    fn match_wildcard_pattern() {
        let source = "fn f(x: Int) -> Int {\n  match x {\n    _ -> 0\n  }\n}";
        let result = hir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let func = &result.module.functions[0];
        if let HirExpr::Block { exprs, .. } = &func.body
            && let HirExpr::Match { arms, .. } = &exprs[0]
        {
            assert!(matches!(&arms[0].pattern, HirPattern::Wildcard(_)));
        }
    }

    // ── 24. Literal pattern ─────────────────────────────────────────

    #[test]
    fn match_literal_pattern() {
        let source = "fn f(x: Int) -> Int {\n  match x {\n    42 -> 1\n    _ -> 0\n  }\n}";
        let result = hir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let func = &result.module.functions[0];
        if let HirExpr::Block { exprs, .. } = &func.body
            && let HirExpr::Match { arms, .. } = &exprs[0]
        {
            assert!(matches!(&arms[0].pattern, HirPattern::Literal(_)));
        }
    }

    // ── 25. Constructor pattern ─────────────────────────────────────

    #[test]
    fn match_constructor_pattern() {
        let source = "type Option(a) {\n  Some(a)\n  None\n}\n\
                       fn f(opt: Option) -> Int {\n  match opt {\n    Some(x) -> x\n    None -> 0\n  }\n}";
        let result = hir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let func = &result.module.functions[0];
        if let HirExpr::Block { exprs, .. } = &func.body
            && let HirExpr::Match { arms, .. } = &exprs[0]
        {
            match &arms[0].pattern {
                HirPattern::Constructor { def_id, fields, .. } => {
                    let data = result.module.symbol_table.get(*def_id);
                    assert_eq!(data.name.as_str(), "Some");
                    assert_eq!(data.kind, DefKind::Constructor);
                    assert_eq!(fields.len(), 1);
                    assert!(matches!(&fields[0], HirPattern::Variable(..)));
                }
                other => panic!("expected Constructor, got {other:?}"),
            }
        }
    }

    // ── 26. List pattern with rest ──────────────────────────────────

    #[test]
    fn match_list_pattern() {
        let source =
            "fn f(items: List) -> Int {\n  match items {\n    [h, ..t] -> h\n    [] -> 0\n  }\n}";
        let result = hir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let func = &result.module.functions[0];
        if let HirExpr::Block { exprs, .. } = &func.body
            && let HirExpr::Match { arms, .. } = &exprs[0]
        {
            match &arms[0].pattern {
                HirPattern::List { elements, rest, .. } => {
                    assert_eq!(elements.len(), 1);
                    assert!(rest.is_some(), "rest binding should exist");
                    // h should be resolvable in body
                    match &arms[0].body {
                        HirExpr::Var(def_id, _) => {
                            let data = result.module.symbol_table.get(*def_id);
                            assert_eq!(data.name.as_str(), "h");
                        }
                        other => panic!("expected Var, got {other:?}"),
                    }
                }
                other => panic!("expected List, got {other:?}"),
            }
        }
    }

    // ── 27. Duplicate function diagnostic ───────────────────────────

    #[test]
    fn duplicate_function_diagnostic() {
        let result = hir_from_source("fn f() { 1 }\nfn f() { 2 }");
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("duplicate definition `f`")),
            "expected duplicate diagnostic: {:?}",
            result.diagnostics
        );
    }

    // ── 28. Constructor resolves in expression ──────────────────────

    #[test]
    fn constructor_resolves_in_expression() {
        let source = "type Option(a) {\n  Some(a)\n  None\n}\n\
                       fn f() -> Int { None }";
        let result = hir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        // `None` should resolve to the constructor DefId
        let func = &result.module.functions[0];
        if let HirExpr::Block { exprs, .. } = &func.body {
            match &exprs[0] {
                HirExpr::Var(def_id, _) => {
                    let data = result.module.symbol_table.get(*def_id);
                    assert_eq!(data.name.as_str(), "None");
                    assert_eq!(data.kind, DefKind::Constructor);
                }
                other => panic!("expected Var, got {other:?}"),
            }
        }
    }

    // ── 29. match_basic.asty full e2e ───────────────────────────────

    #[test]
    fn lower_match_basic_asty() {
        let source = include_str!("../../../examples/match_basic.asty");
        let cst = parse(FID, source);
        let ast = asatsuyu_ast::lower(&cst, FID);
        let result = lower_to_hir(&ast.module);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        // match_basic.asty has 1 type + 3 functions
        assert_eq!(result.module.custom_types.len(), 1);
        assert_eq!(result.module.functions.len(), 3);
    }

    // ── 30. Constructor pattern resolves DefId ─────────────────────

    #[test]
    fn constructor_pattern_resolves_def_id() {
        let source = "type Option(a) {\n  Some(a)\n  None\n}\n\
                       fn f(opt: Option) -> Int {\n  match opt {\n    Some(x) -> x\n    None -> 0\n  }\n}";
        let result = hir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        // Find the constructor DefId for "Some" from the type registration
        let some_id = result
            .module
            .symbol_table
            .iter()
            .find(|(_, d)| d.name == "Some" && d.kind == DefKind::Constructor)
            .map(|(id, _)| id)
            .expect("Some constructor not found");

        let func = &result.module.functions[0];
        if let HirExpr::Block { exprs, .. } = &func.body
            && let HirExpr::Match { arms, .. } = &exprs[0]
        {
            match &arms[0].pattern {
                HirPattern::Constructor { def_id, fields, .. } => {
                    assert_eq!(*def_id, some_id);
                    assert_eq!(fields.len(), 1);
                }
                other => panic!("expected Constructor, got {other:?}"),
            }
        } else {
            panic!("unexpected body structure");
        }
    }

    // ── 31. Unresolved constructor pattern produces diagnostic ─────

    #[test]
    fn unresolved_constructor_pattern_diagnostic() {
        let source = "fn f(x: Int) -> Int {\n  match x {\n    Unknown(y) -> y\n    _ -> 0\n  }\n}";
        let result = hir_from_source(source);
        assert!(result.has_errors());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.message.contains("unresolved constructor `Unknown`")),
            "diagnostics: {:?}",
            result.diagnostics
        );
    }

    // ── 32. Import registers name in scope ─────────────────────────

    #[test]
    fn import_registers_name() {
        let result = hir_from_source("import io\nfn f() { io }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert_eq!(result.module.imports.len(), 1);

        let func = &result.module.functions[0];
        if let HirExpr::Block { exprs, .. } = &func.body {
            match &exprs[0] {
                HirExpr::Var(def_id, _) => {
                    let data = result.module.symbol_table.get(*def_id);
                    assert_eq!(data.name.as_str(), "io");
                    assert_eq!(data.kind, DefKind::Import);
                }
                other => panic!("expected Var, got {other:?}"),
            }
        }
    }

    // ── 33. Dotted import binds last segment ───────────────────────

    #[test]
    fn dotted_import_binds_last_segment() {
        let result = hir_from_source("import gleam.io\nfn f() { io }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert_eq!(result.module.imports.len(), 1);
        let imp = &result.module.imports[0];
        match &imp.kind {
            HirImportKind::Module { module_path } => {
                assert_eq!(module_path, &vec![SmolStr::from("gleam"), SmolStr::from("io")]);
            }
            other @ HirImportKind::Python { .. } => panic!("expected Module import, got {other:?}"),
        }

        let func = &result.module.functions[0];
        if let HirExpr::Block { exprs, .. } = &func.body {
            match &exprs[0] {
                HirExpr::Var(def_id, _) => {
                    let data = result.module.symbol_table.get(*def_id);
                    assert_eq!(data.name.as_str(), "io");
                    assert_eq!(data.kind, DefKind::Import);
                }
                other => panic!("expected Var, got {other:?}"),
            }
        }
    }

    // ── 34. Import alias binds alias name ──────────────────────────

    #[test]
    fn import_alias_binds_alias() {
        let result = hir_from_source("import gleam.io as stdio\nfn f() { stdio }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let func = &result.module.functions[0];
        if let HirExpr::Block { exprs, .. } = &func.body {
            match &exprs[0] {
                HirExpr::Var(def_id, _) => {
                    let data = result.module.symbol_table.get(*def_id);
                    assert_eq!(data.name.as_str(), "stdio");
                    assert_eq!(data.kind, DefKind::Import);
                }
                other => panic!("expected Var, got {other:?}"),
            }
        }
    }

    // ── 35. Import alias: original name is unresolved ──────────────

    #[test]
    fn import_alias_original_name_unresolved() {
        let result = hir_from_source("import gleam.io as stdio\nfn f() { io }");
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("unresolved name `io`")),
            "diagnostics: {:?}",
            result.diagnostics
        );
    }

    // ── 36. No imports produces empty vec ──────────────────────────

    #[test]
    fn no_imports_empty_vec() {
        let result = hir_from_source("fn f() { 1 }");
        assert!(result.module.imports.is_empty());
    }

    // ── 37. Duplicate import diagnostic ────────────────────────────

    #[test]
    fn duplicate_import_diagnostic() {
        let result = hir_from_source("import io\nimport io");
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("duplicate definition `io`")),
            "diagnostics: {:?}",
            result.diagnostics
        );
    }

    // ── 38. Import clashes with function name ──────────────────────

    #[test]
    fn import_clashes_with_function() {
        let result = hir_from_source("import io\nfn io() { 1 }");
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("duplicate definition `io`")),
            "diagnostics: {:?}",
            result.diagnostics
        );
    }

    // ── 39. Type name resolves in expression ───────────────────────

    #[test]
    fn type_name_resolves_in_expression() {
        let source = "type Option(a) {\n  Some(a)\n  None\n}\nfn f() { Option }";
        let result = hir_from_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let func = &result.module.functions[0];
        if let HirExpr::Block { exprs, .. } = &func.body {
            match &exprs[0] {
                HirExpr::Var(def_id, _) => {
                    let data = result.module.symbol_table.get(*def_id);
                    assert_eq!(data.name.as_str(), "Option");
                    assert_eq!(data.kind, DefKind::Type);
                }
                other => panic!("expected Var, got {other:?}"),
            }
        }
    }

    // ── 40. Type name clashes with function name ───────────────────

    #[test]
    fn type_name_clashes_with_function() {
        let result = hir_from_source("type Foo {\n  Bar\n}\nfn Foo() { 1 }");
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("duplicate definition `Foo`")),
            "diagnostics: {:?}",
            result.diagnostics
        );
    }

    // ── 41. Import clashes with constructor ─────────────────────────

    #[test]
    fn import_clashes_with_constructor() {
        let result = hir_from_source("import io\ntype T {\n  io\n}");
        assert!(result.has_errors());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains("duplicate definition `io`")),
            "diagnostics: {:?}",
            result.diagnostics
        );
    }

    // ── Python FFI import tests ───────────────────────────────────

    #[test]
    fn python_import_registers_name() {
        let result = hir_from_source("from python import pathlib\nfn f() { pathlib }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert_eq!(result.module.imports.len(), 1);

        let imp = &result.module.imports[0];
        match &imp.kind {
            HirImportKind::Python { module_name } => {
                assert_eq!(module_name.as_str(), "pathlib");
            }
            other @ HirImportKind::Module { .. } => panic!("expected Python import, got {other:?}"),
        }

        // The name `pathlib` should resolve in the function body.
        let func = &result.module.functions[0];
        if let HirExpr::Block { exprs, .. } = &func.body {
            match &exprs[0] {
                HirExpr::Var(def_id, _) => {
                    let data = result.module.symbol_table.get(*def_id);
                    assert_eq!(data.name.as_str(), "pathlib");
                    assert_eq!(data.kind, DefKind::Import);
                }
                other => panic!("expected Var, got {other:?}"),
            }
        }
    }

    #[test]
    fn python_import_with_alias() {
        let result = hir_from_source("from python import pathlib as pl\nfn f() { pl }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert_eq!(result.module.imports.len(), 1);

        let imp = &result.module.imports[0];
        match &imp.kind {
            HirImportKind::Python { module_name } => {
                assert_eq!(module_name.as_str(), "pathlib");
            }
            other @ HirImportKind::Module { .. } => panic!("expected Python import, got {other:?}"),
        }

        let data = result.module.symbol_table.get(imp.def_id);
        assert_eq!(data.name.as_str(), "pl");
    }

    #[test]
    fn python_import_alias_hides_original() {
        let result = hir_from_source("from python import json as j\nfn f() { json }");
        assert!(result.has_errors(), "expected unresolved `json`");
    }
}
