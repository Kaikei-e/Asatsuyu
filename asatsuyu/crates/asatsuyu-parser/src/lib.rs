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
