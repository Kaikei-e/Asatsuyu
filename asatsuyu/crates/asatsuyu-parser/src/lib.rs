//! Recursive descent parser for the Asatsuyu language.
//!
//! Produces a lossless concrete syntax tree (CST) with error recovery.
//!
//! # Usage
//!
//! ```
//! use asatsuyu_parser::parse;
//! use asatsuyu_syntax::FileId;
//!
//! let result = parse(FileId(0), "pub fn main() { 42 }");
//! assert!(!result.has_errors());
//! assert_eq!(result.syntax().to_string(), "pub fn main() { 42 }");
//! ```

mod grammar;
mod language;
mod parser;

pub use language::{AsatsuyuLanguage, SyntaxElement, SyntaxNode, SyntaxToken};

use asatsuyu_syntax::{Diagnostic, FileId, Severity};
use rowan::GreenNode;

/// The result of parsing source code into a lossless CST.
///
/// Always contains a tree — even when errors are present, the tree is as
/// complete as possible with `NodeError` nodes marking malformed regions.
#[derive(Debug)]
pub struct ParseResult {
    green: GreenNode,
    diagnostics: Vec<Diagnostic>,
}

impl ParseResult {
    /// Returns the root green node of the CST.
    #[must_use]
    pub fn green(&self) -> &GreenNode {
        &self.green
    }

    /// Returns the typed root syntax node for tree traversal.
    #[must_use]
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    /// Returns all diagnostics (lexer + parser) collected during parsing.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Returns `true` if any error-level diagnostic was emitted.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.severity == Severity::Error)
    }
}

/// Parse source code into a lossless concrete syntax tree.
///
/// Runs the lexer internally, then parses the token stream. Lexer and parser
/// diagnostics are merged into a single list on the returned [`ParseResult`].
#[must_use]
pub fn parse(file_id: FileId, source: &str) -> ParseResult {
    let (tokens, mut diagnostics) = asatsuyu_lexer::lex(source, file_id);
    let mut p = parser::Parser::new(&tokens, file_id);
    grammar::parse_source_file(&mut p);
    let (green, parse_diagnostics) = p.finish();
    diagnostics.extend(parse_diagnostics);
    ParseResult { green, diagnostics }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asatsuyu_syntax::FileId;

    const FID: FileId = FileId(0);

    /// Helper: parse source and return the debug representation of the syntax tree.
    fn debug_tree(source: &str) -> String {
        let result = parse(FID, source);
        format!("{:#?}", result.syntax())
    }

    // ── 1. Empty source ──────────────────────────────────────────────

    #[test]
    fn parse_empty_source() {
        use rowan::Language;

        let result = parse(FID, "");
        assert!(!result.has_errors());
        assert_eq!(result.syntax().to_string(), "");
        // The root node should be SourceFile.
        let expected = AsatsuyuLanguage::kind_to_raw(asatsuyu_syntax::SyntaxKind::SourceFile);
        assert_eq!(result.syntax().kind(), AsatsuyuLanguage::kind_from_raw(expected));
    }

    // ── 2. Minimal function (DoD case) ───────────────────────────────

    #[test]
    fn parse_minimal_function() {
        let source = "pub fn main() { 42 }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("FnDef"), "tree should contain FnDef:\n{tree}");
        assert!(tree.contains("Visibility"), "tree should contain Visibility:\n{tree}");
        assert!(tree.contains("ParamList"), "tree should contain ParamList:\n{tree}");
        assert!(tree.contains("BlockExpr"), "tree should contain BlockExpr:\n{tree}");
        assert!(tree.contains("LiteralExpr"), "tree should contain LiteralExpr:\n{tree}");
    }

    // ── 3. Function without pub ──────────────────────────────────────

    #[test]
    fn parse_function_without_pub() {
        let source = "fn main() { 42 }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(!tree.contains("Visibility"), "tree should NOT contain Visibility:\n{tree}");
        assert!(tree.contains("FnDef"), "tree should contain FnDef:\n{tree}");
    }

    // ── 4. Function with parameters ──────────────────────────────────

    #[test]
    fn parse_function_with_params() {
        let source = "fn add(x: Int, y: Int) { 1 }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        // Should have two Param nodes
        let param_count = tree.matches("Param@").count();
        assert!(param_count >= 2, "expected 2+ Param nodes, got {param_count}:\n{tree}");
    }

    // ── 5. Function with return type ─────────────────────────────────

    #[test]
    fn parse_function_with_return_type() {
        let source = "fn id(x: Int) -> Int { x }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("ReturnType"), "tree should contain ReturnType:\n{tree}");
        assert!(tree.contains("IdentExpr"), "tree should contain IdentExpr:\n{tree}");
    }

    // ── 6. String literal ────────────────────────────────────────────

    #[test]
    fn parse_string_literal() {
        let source = r#"fn greet() { "hello" }"#;
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("LiteralExpr"), "tree should contain LiteralExpr:\n{tree}");
        assert!(tree.contains("StringLit"), "tree should contain StringLit:\n{tree}");
    }

    // ── 7. Multiple functions ────────────────────────────────────────

    #[test]
    fn parse_multiple_functions() {
        let source = "fn a() { 1 }\nfn b() { 2 }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        let fn_count = tree.matches("FnDef@").count();
        assert_eq!(fn_count, 2, "expected 2 FnDef nodes:\n{tree}");
    }

    // ── 8. Trailing comma in params ──────────────────────────────────

    #[test]
    fn parse_trailing_comma_in_params() {
        let source = "fn f(x: Int,) { 1 }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());
    }

    // ── 9. Lossless roundtrip ────────────────────────────────────────

    #[test]
    fn lossless_roundtrip() {
        let sources = &[
            "",
            "pub fn main() { 42 }",
            "fn add(x: Int, y: Int) { 1 }",
            "fn id(x: Int) -> Int { x }",
            "fn a() { 1 }\nfn b() { 2 }",
            "  \n  pub fn main() { 42 }  \n  ",
            r#"fn greet() { "hello" }"#,
            "fn f(x: Int,) { 1 }",
            "fn f() { a + b * c }",
            "fn f() { g(1, 2) }",
            "fn f() { if x { 1 } else { 2 } }",
            "fn f() { (a + b) }",
            "fn f() { -x }",
        ];
        for &source in sources {
            let result = parse(FID, source);
            assert_eq!(
                result.syntax().to_string(),
                source,
                "lossless roundtrip failed for: {source:?}"
            );
        }
    }

    // ── 10. Binary addition ───────────────────────────────────────

    #[test]
    fn parse_binary_addition() {
        let source = "fn f() { a + b }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("BinaryExpr"), "tree should contain BinaryExpr:\n{tree}");
    }

    // ── 11. Binary precedence ───────────────────────────────────

    #[test]
    fn parse_binary_precedence() {
        let source = "fn f() { a + b * c }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        // Should have nested BinaryExpr: outer (+) wrapping inner (*)
        let binary_count = tree.matches("BinaryExpr@").count();
        assert!(
            binary_count >= 2,
            "expected 2+ BinaryExpr for precedence, got {binary_count}:\n{tree}"
        );
    }

    // ── 12. Comparison ──────────────────────────────────────────

    #[test]
    fn parse_comparison() {
        let source = "fn f() { x == y }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("BinaryExpr"), "tree should contain BinaryExpr:\n{tree}");
        assert!(tree.contains("EqEq"), "tree should contain EqEq:\n{tree}");
    }

    // ── 13. Call expression (no args) ───────────────────────────

    #[test]
    fn parse_call_no_args() {
        let source = "fn f() { g() }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("CallExpr"), "tree should contain CallExpr:\n{tree}");
        assert!(tree.contains("ArgList"), "tree should contain ArgList:\n{tree}");
    }

    // ── 14. Call expression (with args) ─────────────────────────

    #[test]
    fn parse_call_with_args() {
        let source = "fn f() { g(1, 2) }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("CallExpr"), "tree should contain CallExpr:\n{tree}");
        assert!(tree.contains("ArgList"), "tree should contain ArgList:\n{tree}");
    }

    // ── 15. Call expression (trailing comma) ────────────────────

    #[test]
    fn parse_call_trailing_comma() {
        let source = "fn f() { g(1,) }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());
    }

    // ── 16. Unary minus ─────────────────────────────────────────

    #[test]
    fn parse_unary_minus() {
        let source = "fn f() { -x }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("UnaryExpr"), "tree should contain UnaryExpr:\n{tree}");
    }

    // ── 17. Unary not ───────────────────────────────────────────

    #[test]
    fn parse_unary_not() {
        let source = "fn f() { !x }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("UnaryExpr"), "tree should contain UnaryExpr:\n{tree}");
    }

    // ── 18. Parenthesized expression ────────────────────────────

    #[test]
    fn parse_paren_expr() {
        let source = "fn f() { (a + b) * c }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("ParenExpr"), "tree should contain ParenExpr:\n{tree}");
    }

    // ── 19. Float literal ───────────────────────────────────────

    #[test]
    fn parse_float_literal() {
        let source = "fn f() { 3.14 }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("FloatLit"), "tree should contain FloatLit:\n{tree}");
    }

    // ── 20. Bool literal ────────────────────────────────────────

    #[test]
    fn parse_bool_literal() {
        let source = "fn f() { True }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("TrueKw"), "tree should contain TrueKw:\n{tree}");
    }

    // ── 21. Chained calls ───────────────────────────────────────

    #[test]
    fn parse_chained_calls() {
        let source = "fn f() { g(1)(2) }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        let call_count = tree.matches("CallExpr@").count();
        assert!(call_count >= 2, "expected 2+ CallExpr, got {call_count}:\n{tree}");
    }

    // ── 22. Complex expression ──────────────────────────────────

    #[test]
    fn parse_complex_expr() {
        let source = "fn f() { f(a + b, c * d) }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());
    }

    // ── 23. If expression (simple) ──────────────────────────────

    #[test]
    fn parse_if_simple() {
        let source = "fn f() { if x { 1 } }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("IfExpr"), "tree should contain IfExpr:\n{tree}");
    }

    // ── 24. If-else expression ──────────────────────────────────

    #[test]
    fn parse_if_else() {
        let source = "fn f() { if x { 1 } else { 2 } }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("IfExpr"), "tree should contain IfExpr:\n{tree}");
        assert!(tree.contains("ElseKw"), "tree should contain ElseKw:\n{tree}");
    }

    // ── 25. If-else-if chain ────────────────────────────────────

    #[test]
    fn parse_if_else_if() {
        let source = "fn f() { if x { 1 } else if y { 2 } else { 3 } }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        let if_count = tree.matches("IfExpr@").count();
        assert!(if_count >= 2, "expected 2+ IfExpr for else-if chain, got {if_count}:\n{tree}");
    }

    // ── 26. If with comparison condition ────────────────────────

    #[test]
    fn parse_if_with_condition() {
        let source = "fn f() { if x == 1 { True } }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("IfExpr"), "tree should contain IfExpr:\n{tree}");
        assert!(tree.contains("BinaryExpr"), "tree should contain BinaryExpr:\n{tree}");
    }

    // ── 27. If with logical && ──────────────────────────────────

    #[test]
    fn parse_if_logical_and() {
        let source = "fn f() { if a && b { 1 } }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("AmpAmp"), "tree should contain AmpAmp:\n{tree}");
    }

    // ── 28. If with logical || ──────────────────────────────────

    #[test]
    fn parse_if_logical_or() {
        let source = "fn f() { if a || b { 1 } }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("PipePipe"), "tree should contain PipePipe:\n{tree}");
    }

    // ── 29. Error: missing call RParen ──────────────────────────

    #[test]
    fn error_missing_call_rparen() {
        let source = "fn f() { g(1 }";
        let result = parse(FID, source);
        assert!(result.has_errors());
        assert!(
            result.diagnostics().iter().any(|d| d.message.contains("RParen")),
            "should report missing `)`: {:?}",
            result.diagnostics()
        );
    }

    // ── 30. Error: malformed if (no block) ──────────────────────

    #[test]
    fn error_malformed_if_no_block() {
        let source = "fn f() { if x 1 }";
        let result = parse(FID, source);
        assert!(result.has_errors());
        assert!(
            result.diagnostics().iter().any(|d| d.message.contains("expected block")),
            "should report expected block: {:?}",
            result.diagnostics()
        );
    }

    // ── 31. Error: missing else block ───────────────────────────

    #[test]
    fn error_missing_else_block() {
        let source = "fn f() { if x { 1 } else 2 }";
        let result = parse(FID, source);
        assert!(result.has_errors());
        assert!(
            result.diagnostics().iter().any(|d| d.message.contains("expected block or `if`")),
            "should report expected block or if: {:?}",
            result.diagnostics()
        );
    }

    // ── Error tests (original) ──────────────────────────────────

    // ── 10. Error: unexpected top-level token ────────────────────────

    #[test]
    fn error_unexpected_top_level() {
        let source = "42 fn main() { 1 }";
        let result = parse(FID, source);
        assert!(result.has_errors());

        let tree = debug_tree(source);
        // Should have a NodeError wrapping the `42`, then recover to parse fn
        assert!(tree.contains("NodeError"), "should have NodeError:\n{tree}");
        assert!(tree.contains("FnDef"), "should recover and parse FnDef:\n{tree}");

        // Roundtrip still works
        assert_eq!(result.syntax().to_string(), source);
    }

    // ── 11. Error: missing closing brace ─────────────────────────────

    #[test]
    fn error_missing_brace() {
        let source = "fn main() { 42";
        let result = parse(FID, source);
        assert!(result.has_errors());
        assert!(
            result.diagnostics().iter().any(|d| d.message.contains("RBrace")),
            "should report missing `}}`: {:?}",
            result.diagnostics()
        );
        // Roundtrip still works
        assert_eq!(result.syntax().to_string(), source);
    }

    // ── 12. Error: missing param colon ───────────────────────────────

    #[test]
    fn error_missing_param_colon() {
        let source = "fn f(x) { 1 }";
        let result = parse(FID, source);
        assert!(result.has_errors());
        assert!(
            result.diagnostics().iter().any(|d| d.message.contains("Colon")),
            "should report missing `:`: {:?}",
            result.diagnostics()
        );
    }
}
