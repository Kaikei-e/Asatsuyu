//! CST to untyped AST conversion for the Asatsuyu language.
//!
//! Strips trivia from the lossless CST and produces a normalized abstract
//! syntax tree where every node carries a [`Span`](asatsuyu_syntax::Span).
//!
//! # Usage
//!
//! ```
//! use asatsuyu_ast::lower;
//! use asatsuyu_parser::parse;
//! use asatsuyu_syntax::FileId;
//!
//! let cst = parse(FileId(0), "pub fn main() { 42 }");
//! let result = lower(&cst, FileId(0));
//! assert!(!result.has_errors());
//! assert_eq!(result.module.definitions.len(), 1);
//! ```

mod lower;
mod types;

pub use types::{
    BinOp, CustomType, Definition, Expr, FnDef, Ident, Import, Literal, LiteralKind, MatchArm,
    Module, Param, Pattern, RecordField, TypeBody, TypeExpr, UnOp, Variant, VariantField,
    Visibility,
};

use asatsuyu_parser::ParseResult;
use asatsuyu_syntax::{Diagnostic, FileId, Severity};

/// The result of lowering a CST into an untyped AST.
#[derive(Debug)]
pub struct LowerResult {
    /// The lowered module.
    pub module: Module,
    /// Diagnostics collected during lowering.
    pub diagnostics: Vec<Diagnostic>,
}

impl LowerResult {
    /// Returns `true` if any error-level diagnostic was emitted during lowering.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.severity == Severity::Error)
    }
}

/// Lower a parsed CST into an untyped AST.
///
/// Strips trivia, normalizes the tree structure, and collects diagnostics
/// for malformed nodes. Always returns a [`Module`], even when errors are
/// present.
#[must_use]
pub fn lower(parse_result: &ParseResult, file_id: FileId) -> LowerResult {
    let mut ctx = lower::LowerCtx::new(file_id);
    let module = ctx.lower_source_file(&parse_result.syntax());
    LowerResult { module, diagnostics: ctx.into_diagnostics() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asatsuyu_parser::parse;

    const FID: FileId = FileId(0);

    /// Helper: parse + lower source code.
    fn lower_source(source: &str) -> LowerResult {
        let cst = parse(FID, source);
        lower(&cst, FID)
    }

    /// Helper: extract a function definition from module.definitions by index.
    fn get_fn(module: &Module, index: usize) -> &FnDef {
        match &module.definitions[index] {
            Definition::Function(f) => f,
            Definition::CustomType(ct) => {
                panic!("expected Function, got CustomType({:?})", ct.name)
            }
        }
    }

    /// Helper: extract type name from `TypeExpr`.
    fn type_name(te: &TypeExpr) -> &str {
        match te {
            TypeExpr::Named { name, .. } => name.name.as_str(),
        }
    }

    // ── 1. Empty source ─────────────────────────────────────────────

    #[test]
    fn lower_empty_source() {
        let result = lower_source("");
        assert!(!result.has_errors());
        assert!(result.module.imports.is_empty());
        assert!(result.module.definitions.is_empty());
    }

    // ── 2. Minimal function (DoD case) ──────────────────────────────

    #[test]
    fn lower_minimal_function() {
        let result = lower_source("pub fn main() { 42 }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert_eq!(result.module.definitions.len(), 1);

        let f = get_fn(&result.module, 0);
        assert_eq!(f.name.name.as_str(), "main");
        assert_eq!(f.visibility, Visibility::Public);
        assert!(f.params.is_empty());
        assert!(f.return_type.is_none());

        match &f.body {
            Expr::Block { exprs, .. } => {
                assert_eq!(exprs.len(), 1);
                match &exprs[0] {
                    Expr::Literal(lit) => {
                        assert_eq!(lit.kind, LiteralKind::Int);
                        assert_eq!(lit.value.as_str(), "42");
                    }
                    other => panic!("expected Literal, got {other:?}"),
                }
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn lower_async_fn_marks_flag() {
        let result = lower_source("pub async fn fetch() { 42 }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let f = get_fn(&result.module, 0);
        assert!(f.is_async, "async fn should set is_async on AST fn def");
    }

    #[test]
    fn lower_await_expr() {
        let result = lower_source("fn f() { await fetch() }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let f = get_fn(&result.module, 0);
        match &f.body {
            Expr::Block { exprs, .. } => match &exprs[0] {
                Expr::Await { expr, .. } => {
                    assert!(
                        matches!(expr.as_ref(), Expr::Call { .. }),
                        "await should wrap the call"
                    );
                }
                other => panic!("expected Await, got {other:?}"),
            },
            other => panic!("expected Block, got {other:?}"),
        }
    }

    // ── 3. Function without pub ─────────────────────────────────────

    #[test]
    fn lower_private_function() {
        let result = lower_source("fn main() { 42 }");
        assert!(!result.has_errors());

        let f = get_fn(&result.module, 0);
        assert_eq!(f.visibility, Visibility::Private);
    }

    // ── 4. Function with parameters ─────────────────────────────────

    #[test]
    fn lower_function_with_params() {
        let result = lower_source("fn add(x: Int, y: Int) { 1 }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let f = get_fn(&result.module, 0);
        assert_eq!(f.params.len(), 2);
        assert_eq!(f.params[0].name.name.as_str(), "x");
        assert_eq!(type_name(f.params[0].type_ann.as_ref().unwrap()), "Int");
        assert_eq!(f.params[1].name.name.as_str(), "y");
        assert_eq!(type_name(f.params[1].type_ann.as_ref().unwrap()), "Int");
    }

    // ── 5. Function with return type ────────────────────────────────

    #[test]
    fn lower_function_with_return_type() {
        let result = lower_source("fn id(x: Int) -> Int { x }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let f = get_fn(&result.module, 0);
        let rt = f.return_type.as_ref().expect("expected return type");
        assert_eq!(type_name(rt), "Int");

        match &f.body {
            Expr::Block { exprs, .. } => {
                assert_eq!(exprs.len(), 1);
                match &exprs[0] {
                    Expr::Variable(ident) => assert_eq!(ident.name.as_str(), "x"),
                    other => panic!("expected Variable, got {other:?}"),
                }
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    // ── 6. String literal ───────────────────────────────────────────

    #[test]
    fn lower_string_literal() {
        let result = lower_source(r#"fn greet() { "hello" }"#);
        assert!(!result.has_errors());

        let f = get_fn(&result.module, 0);
        match &f.body {
            Expr::Block { exprs, .. } => {
                assert_eq!(exprs.len(), 1);
                match &exprs[0] {
                    Expr::Literal(lit) => {
                        assert_eq!(lit.kind, LiteralKind::String);
                        assert_eq!(lit.value.as_str(), "\"hello\"");
                    }
                    other => panic!("expected Literal, got {other:?}"),
                }
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    // ── 7. Multiple functions ───────────────────────────────────────

    #[test]
    fn lower_multiple_functions() {
        let result = lower_source("fn a() { 1 }\nfn b() { 2 }");
        assert!(!result.has_errors());
        assert_eq!(result.module.definitions.len(), 2);

        let a = get_fn(&result.module, 0);
        let b = get_fn(&result.module, 1);
        assert_eq!(a.name.name.as_str(), "a");
        assert_eq!(b.name.name.as_str(), "b");
    }

    // ── 8. Block with multiple expressions ──────────────────────────

    #[test]
    fn lower_block_multiple_exprs() {
        let result = lower_source(r#"fn f() { 1 "hi" x }"#);
        assert!(!result.has_errors());

        let f = get_fn(&result.module, 0);
        match &f.body {
            Expr::Block { exprs, .. } => assert_eq!(exprs.len(), 3),
            other => panic!("expected Block, got {other:?}"),
        }
    }

    // ── 9. All AST nodes carry non-dummy Span ───────────────────────

    #[test]
    fn all_nodes_have_span() {
        let result = lower_source("pub fn add(x: Int) -> Int { x }");
        assert!(!result.has_errors());

        let f = get_fn(&result.module, 0);

        // Module span covers entire source
        assert!(!result.module.span.is_empty());

        // FnDef span
        assert!(!f.span.is_empty());
        assert_eq!(f.span.start, 0);

        // Name span
        assert!(!f.name.span.is_empty());

        // Param spans
        assert!(!f.params[0].span.is_empty());
        assert!(!f.params[0].name.span.is_empty());
        assert!(!f.params[0].type_ann.as_ref().unwrap().span().is_empty());

        // Return type span
        let rt = f.return_type.as_ref().unwrap();
        assert!(!rt.span().is_empty());

        // Body span
        assert!(!f.body.span().is_empty());
    }

    // ── 10. hello.asty lowering ─────────────────────────────────────

    #[test]
    fn lower_hello_asty() {
        let source = include_str!("../../../examples/hello.asty");
        let cst = parse(FID, source);
        let result = lower(&cst, FID);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert_eq!(result.module.definitions.len(), 1);

        let f = get_fn(&result.module, 0);
        assert_eq!(f.name.name.as_str(), "main");
        assert_eq!(f.visibility, Visibility::Public);
    }

    // ── 11. greet.asty lowering ─────────────────────────────────────

    #[test]
    fn lower_greet_asty() {
        let source = include_str!("../../../examples/greet.asty");
        let cst = parse(FID, source);
        let result = lower(&cst, FID);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert_eq!(result.module.definitions.len(), 2);

        let greet = get_fn(&result.module, 0);
        assert_eq!(greet.name.name.as_str(), "greet");
        assert_eq!(greet.visibility, Visibility::Public);
        assert_eq!(greet.params.len(), 1);
        assert!(greet.return_type.is_some());

        let main = get_fn(&result.module, 1);
        assert_eq!(main.name.name.as_str(), "main");
        assert_eq!(main.visibility, Visibility::Public);
        assert_eq!(main.params.len(), 0);
    }

    // ── 12. Error recovery input ────────────────────────────────────

    #[test]
    fn lower_with_error_recovery() {
        let result = lower_source("42 fn main() { 1 }");
        // Should have a diagnostic for the `42` error node
        assert!(result.has_errors(), "expected lowering diagnostics");
        // But should still recover the function
        assert_eq!(result.module.definitions.len(), 1);

        let f = get_fn(&result.module, 0);
        assert_eq!(f.name.name.as_str(), "main");
    }

    // ── 13. AST dump works (DoD) ────────────────────────────────────

    #[test]
    fn ast_dump() {
        let result = lower_source("pub fn main() { 42 }");
        let dump = format!("{:#?}", result.module);
        assert!(dump.contains("main"), "dump should contain function name:\n{dump}");
        assert!(dump.contains("Public"), "dump should contain visibility:\n{dump}");
        assert!(dump.contains("Int"), "dump should contain literal kind:\n{dump}");
    }

    // ── Snapshot tests (Issue 19) ───────────────────────────────────

    /// Helper: parse + lower, assert no errors, return module debug string.
    fn snapshot(source: &str) -> String {
        let result = lower_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        format!("{:#?}", result.module)
    }

    #[test]
    fn snap_custom_type_variant() {
        insta::assert_snapshot!(snapshot("type Option(a) {\n  Some(a)\n  None\n}"));
    }

    #[test]
    fn snap_custom_type_record() {
        insta::assert_snapshot!(snapshot("type User {\n  name: String\n  age: Int\n}"));
    }

    #[test]
    fn snap_custom_type_with_params() {
        insta::assert_snapshot!(snapshot("type Result(a, e) {\n  Ok(a)\n  Err(e)\n}"));
    }

    #[test]
    fn snap_import_basic() {
        insta::assert_snapshot!(snapshot("import io"));
    }

    #[test]
    fn snap_import_dotted() {
        insta::assert_snapshot!(snapshot("import gleam.io"));
    }

    #[test]
    fn snap_import_alias() {
        insta::assert_snapshot!(snapshot("import io as stdio"));
    }

    #[test]
    fn snap_match_expr() {
        insta::assert_snapshot!(snapshot(
            "fn f(x: Int) -> Int {\n  match x {\n    0 -> 0\n    _ -> 1\n  }\n}"
        ));
    }

    #[test]
    fn snap_match_constructor() {
        insta::assert_snapshot!(snapshot(
            "fn f(opt: Option) -> Int {\n  match opt {\n    Some(x) -> x\n    None -> 0\n  }\n}"
        ));
    }

    #[test]
    fn snap_if_else() {
        insta::assert_snapshot!(snapshot("fn f(x: Int) -> Int {\n  if x { 1 } else { 2 }\n}"));
    }

    #[test]
    fn snap_if_else_chain() {
        insta::assert_snapshot!(snapshot(
            "fn f(a: Int, b: Int) -> Int {\n  if a { 1 } else if b { 2 } else { 3 }\n}"
        ));
    }

    #[test]
    fn snap_pipeline() {
        insta::assert_snapshot!(snapshot("fn f(x: Int) -> Int {\n  x |> g\n}"));
    }

    #[test]
    fn snap_binary_ops() {
        insta::assert_snapshot!(snapshot("fn f() -> Int {\n  1 + 2 * 3\n}"));
    }

    #[test]
    fn snap_unary_ops() {
        insta::assert_snapshot!(snapshot("fn f(x: Int) -> Int {\n  -x\n}"));
    }

    #[test]
    fn snap_call_expr() {
        insta::assert_snapshot!(snapshot("fn f() -> Int {\n  g(1, 2)\n}"));
    }

    #[test]
    fn snap_pipeline_with_call() {
        insta::assert_snapshot!(snapshot("fn f(x: Int) -> Int {\n  x |> g(1)\n}"));
    }

    #[test]
    fn snap_match_basic_asty() {
        let source = include_str!("../../../examples/match_basic.asty");
        let result = lower_source(source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        insta::assert_snapshot!(format!("{:#?}", result.module));
    }

    #[test]
    fn snap_bool_literal() {
        insta::assert_snapshot!(snapshot("fn f() -> Bool {\n  True\n}"));
    }

    #[test]
    fn snap_float_literal() {
        insta::assert_snapshot!(snapshot("fn f() -> Float {\n  3.14\n}"));
    }

    #[test]
    fn snap_string_concat() {
        insta::assert_snapshot!(snapshot("fn f(name: String) -> String {\n  \"hello\" <> name\n}"));
    }
}
