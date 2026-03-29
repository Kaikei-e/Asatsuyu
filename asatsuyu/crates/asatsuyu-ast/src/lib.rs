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

pub use types::{Definition, Expr, FnDef, Ident, Literal, LiteralKind, Module, Param, Visibility};

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

    // ── 1. Empty source ─────────────────────────────────────────────

    #[test]
    fn lower_empty_source() {
        let result = lower_source("");
        assert!(!result.has_errors());
        assert!(result.module.definitions.is_empty());
    }

    // ── 2. Minimal function (DoD case) ──────────────────────────────

    #[test]
    fn lower_minimal_function() {
        let result = lower_source("pub fn main() { 42 }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);
        assert_eq!(result.module.definitions.len(), 1);

        let Definition::Function(ref f) = result.module.definitions[0];
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

    // ── 3. Function without pub ─────────────────────────────────────

    #[test]
    fn lower_private_function() {
        let result = lower_source("fn main() { 42 }");
        assert!(!result.has_errors());

        let Definition::Function(ref f) = result.module.definitions[0];
        assert_eq!(f.visibility, Visibility::Private);
    }

    // ── 4. Function with parameters ─────────────────────────────────

    #[test]
    fn lower_function_with_params() {
        let result = lower_source("fn add(x: Int, y: Int) { 1 }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let Definition::Function(ref f) = result.module.definitions[0];
        assert_eq!(f.params.len(), 2);
        assert_eq!(f.params[0].name.name.as_str(), "x");
        assert_eq!(f.params[0].type_ann.name.as_str(), "Int");
        assert_eq!(f.params[1].name.name.as_str(), "y");
        assert_eq!(f.params[1].type_ann.name.as_str(), "Int");
    }

    // ── 5. Function with return type ────────────────────────────────

    #[test]
    fn lower_function_with_return_type() {
        let result = lower_source("fn id(x: Int) -> Int { x }");
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics);

        let Definition::Function(ref f) = result.module.definitions[0];
        let rt = f.return_type.as_ref().expect("expected return type");
        assert_eq!(rt.name.as_str(), "Int");

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

        let Definition::Function(ref f) = result.module.definitions[0];
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

        let Definition::Function(ref a) = result.module.definitions[0];
        let Definition::Function(ref b) = result.module.definitions[1];
        assert_eq!(a.name.name.as_str(), "a");
        assert_eq!(b.name.name.as_str(), "b");
    }

    // ── 8. Block with multiple expressions ──────────────────────────

    #[test]
    fn lower_block_multiple_exprs() {
        let result = lower_source(r#"fn f() { 1 "hi" x }"#);
        assert!(!result.has_errors());

        let Definition::Function(ref f) = result.module.definitions[0];
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

        let Definition::Function(ref f) = result.module.definitions[0];

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
        assert!(!f.params[0].type_ann.span.is_empty());

        // Return type span
        let rt = f.return_type.as_ref().unwrap();
        assert!(!rt.span.is_empty());

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

        let Definition::Function(ref f) = result.module.definitions[0];
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

        let Definition::Function(ref greet) = result.module.definitions[0];
        assert_eq!(greet.name.name.as_str(), "greet");
        assert_eq!(greet.visibility, Visibility::Public);
        assert_eq!(greet.params.len(), 1);
        assert!(greet.return_type.is_some());

        let Definition::Function(ref add) = result.module.definitions[1];
        assert_eq!(add.name.name.as_str(), "add");
        assert_eq!(add.visibility, Visibility::Private);
        assert_eq!(add.params.len(), 2);
    }

    // ── 12. Error recovery input ────────────────────────────────────

    #[test]
    fn lower_with_error_recovery() {
        let result = lower_source("42 fn main() { 1 }");
        // Should have a diagnostic for the `42` error node
        assert!(result.has_errors(), "expected lowering diagnostics");
        // But should still recover the function
        assert_eq!(result.module.definitions.len(), 1);

        let Definition::Function(ref f) = result.module.definitions[0];
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
}
