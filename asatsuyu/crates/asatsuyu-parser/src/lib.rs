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

    #[test]
    fn parse_function_with_parameterized_types() {
        let source = "fn wrap(xs: List(Int)) -> Result(Int, String) { xs }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("ReturnType"), "tree should contain ReturnType:\n{tree}");
        assert!(tree.contains("TypeExpr"), "tree should contain TypeExpr:\n{tree}");
        assert!(tree.contains("\"List\""), "tree should contain List:\n{tree}");
        assert!(tree.contains("\"Result\""), "tree should contain Result:\n{tree}");
        assert!(tree.contains("\"String\""), "tree should contain String:\n{tree}");
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
            "fn f() { x |> g }",
            "fn f() { x |> f |> g }",
            "fn f() { x |> f(1, 2) }",
            "fn f() { a + b |> f }",
            "type User {\n  name: String\n  age: Int\n}",
            "type Option(a) {\n  Some(a)\n  None\n}",
            "type Result(a, e) {\n  Ok(a)\n  Error(e)\n}",
            "pub type User {\n  User(name: String, age: Int)\n}",
            "type Empty { }",
            "fn f() { match x { 1 -> 2\n_ -> 0 } }",
            "fn f() { match value { Some(x) -> x\nNone -> 0 } }",
            "fn f() { match items { [head, ..] -> head\n[] -> 0 } }",
            "fn f() { match x { Some(n) if n > 0 -> n\n_ -> 0 } }",
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

    // ── Pipeline expression tests ──────────────────────────────

    // ── 32. Simple pipeline ────────────────────────────────────

    #[test]
    fn parse_simple_pipeline() {
        let source = "fn f() { x |> g }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("PipelineExpr"), "tree should contain PipelineExpr:\n{tree}");
        assert!(tree.contains("Pipe"), "tree should contain Pipe token:\n{tree}");
    }

    // ── 33. Chained pipeline ───────────────────────────────────

    #[test]
    fn parse_chained_pipeline() {
        let source = "fn f() { x |> f |> g }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        let pipeline_count = tree.matches("PipelineExpr@").count();
        assert!(
            pipeline_count >= 2,
            "expected 2+ PipelineExpr for chained pipeline, got {pipeline_count}:\n{tree}"
        );
    }

    // ── 34. Pipeline with call ─────────────────────────────────

    #[test]
    fn parse_pipeline_with_call() {
        let source = "fn f() { x |> f(1, 2) }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("PipelineExpr"), "tree should contain PipelineExpr:\n{tree}");
        assert!(tree.contains("CallExpr"), "tree should contain CallExpr on RHS:\n{tree}");
    }

    // ── 35. Pipeline precedence vs addition ────────────────────

    #[test]
    fn parse_pipeline_precedence_add() {
        // |> and + have the same binding power (7, 8), left-associative
        // so `a + b |> f` parses as `(a + b) |> f`
        let source = "fn f() { a + b |> f }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        // Outer node should be PipelineExpr wrapping BinaryExpr
        assert!(tree.contains("PipelineExpr"), "tree should contain PipelineExpr:\n{tree}");
        assert!(tree.contains("BinaryExpr"), "tree should contain BinaryExpr:\n{tree}");
    }

    // ── 36. Pipeline precedence vs multiplication ──────────────

    #[test]
    fn parse_pipeline_precedence_mul() {
        // * has higher bp (9, 10) than |> (7, 8)
        // so `a * b |> f` parses as `(a * b) |> f`
        let source = "fn f() { a * b |> f }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("PipelineExpr"), "tree should contain PipelineExpr:\n{tree}");
        assert!(tree.contains("BinaryExpr"), "tree should contain BinaryExpr:\n{tree}");
    }

    // ── 37. Pipeline precedence vs comparison ──────────────────

    #[test]
    fn parse_pipeline_precedence_comparison() {
        // == has lower bp (5, 6) than |> (7, 8)
        // so `x |> f == y` parses as `(x |> f) == y`
        let source = "fn f() { x |> f == y }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        // Outer node should be BinaryExpr (==) wrapping PipelineExpr
        assert!(tree.contains("PipelineExpr"), "tree should contain PipelineExpr:\n{tree}");
        assert!(tree.contains("BinaryExpr"), "tree should contain BinaryExpr:\n{tree}");
        assert!(tree.contains("EqEq"), "tree should contain EqEq:\n{tree}");
    }

    // ── 38. Pipeline in if condition ───────────────────────────

    #[test]
    fn parse_pipeline_in_if() {
        let source = "fn f() { if x |> g { 1 } }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("IfExpr"), "tree should contain IfExpr:\n{tree}");
        assert!(tree.contains("PipelineExpr"), "tree should contain PipelineExpr:\n{tree}");
    }

    // ── 39. Pipeline multiline (lossless roundtrip) ────────────

    #[test]
    fn parse_pipeline_multiline() {
        let source = "fn f() { x\n|> f\n|> g }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        let pipeline_count = tree.matches("PipelineExpr@").count();
        assert!(
            pipeline_count >= 2,
            "expected 2+ PipelineExpr for multiline, got {pipeline_count}:\n{tree}"
        );
        // Lossless roundtrip
        assert_eq!(result.syntax().to_string(), source);
    }

    // ── 40. Pipeline three stages ──────────────────────────────

    #[test]
    fn parse_pipeline_three_stages() {
        let source = "fn f() { x |> a |> b |> c }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        let pipeline_count = tree.matches("PipelineExpr@").count();
        assert_eq!(pipeline_count, 3, "expected 3 PipelineExpr, got {pipeline_count}:\n{tree}");
    }

    // ── 41. Pipeline with call args and trailing comma ─────────

    #[test]
    fn parse_pipeline_call_trailing_comma() {
        let source = "fn f() { x |> g(1, 2,) }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());
    }

    // ── Type definition tests ─────────────────────────────────

    // ── 42. Record type (Go-style) ─────────────────────────────

    #[test]
    fn parse_record_type() {
        let source = "type User {\n  name: String\n  age: Int\n}";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("TypeDef"), "tree should contain TypeDef:\n{tree}");
        let field_count = tree.matches("Field@").count();
        assert_eq!(field_count, 2, "expected 2 Field nodes, got {field_count}:\n{tree}");
        assert!(tree.contains("TypeExpr"), "tree should contain TypeExpr:\n{tree}");
    }

    // ── 43. Pub record type ────────────────────────────────────

    #[test]
    fn parse_pub_record_type() {
        let source = "pub type User {\n  name: String\n  age: Int\n}";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("Visibility"), "tree should contain Visibility:\n{tree}");
        assert!(tree.contains("TypeDef"), "tree should contain TypeDef:\n{tree}");
    }

    // ── 44. ADT with no fields ─────────────────────────────────

    #[test]
    fn parse_adt_no_fields() {
        let source = "type Season {\n  Spring\n  Summer\n  Autumn\n  Winter\n}";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("TypeDef"), "tree should contain TypeDef:\n{tree}");
        let variant_count = tree.matches("Variant@").count();
        assert_eq!(variant_count, 4, "expected 4 Variant nodes, got {variant_count}:\n{tree}");
    }

    // ── 45. ADT Option ─────────────────────────────────────────

    #[test]
    fn parse_adt_option() {
        let source = "type Option(a) {\n  Some(a)\n  None\n}";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("TypeParam"), "tree should contain TypeParam:\n{tree}");
        let variant_count = tree.matches("Variant@").count();
        assert_eq!(variant_count, 2, "expected 2 Variant nodes, got {variant_count}:\n{tree}");
    }

    // ── 46. ADT Result ─────────────────────────────────────────

    #[test]
    fn parse_adt_result() {
        let source = "type Result(a, e) {\n  Ok(a)\n  Error(e)\n}";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        let type_param_count = tree.matches("TypeParam@").count();
        assert_eq!(type_param_count, 2, "expected 2 TypeParam, got {type_param_count}:\n{tree}");
        let variant_count = tree.matches("Variant@").count();
        assert_eq!(variant_count, 2, "expected 2 Variant, got {variant_count}:\n{tree}");
    }

    // ── 47. ADT with labelled fields ───────────────────────────

    #[test]
    fn parse_adt_labelled_fields() {
        let source = "type User {\n  User(name: String, age: Int)\n}";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("Variant"), "tree should contain Variant:\n{tree}");
        let field_count = tree.matches("Field@").count();
        assert!(field_count >= 2, "expected 2+ Field in variant, got {field_count}:\n{tree}");
    }

    // ── 48. Parameterized field type ───────────────────────────

    #[test]
    fn parse_type_parameterized_field() {
        let source = "type Wrapper {\n  items: List(Int)\n}";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        // TypeExpr for List(Int) should contain nested TypeExpr for Int
        let type_expr_count = tree.matches("TypeExpr@").count();
        assert!(
            type_expr_count >= 2,
            "expected 2+ TypeExpr (List and Int), got {type_expr_count}:\n{tree}"
        );
    }

    // ── 49. Trailing commas in type params and variant fields ──

    #[test]
    fn parse_type_trailing_commas() {
        let source = "type R(a,) {\n  Ok(a,)\n}";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());
    }

    // ── 50. Type definition alongside function ─────────────────

    #[test]
    fn parse_type_alongside_fn() {
        let source = "type Color {\n  Red\n  Blue\n}\nfn f() { 1 }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("TypeDef"), "tree should contain TypeDef:\n{tree}");
        assert!(tree.contains("FnDef"), "tree should contain FnDef:\n{tree}");
    }

    // ── 51. Empty type body ────────────────────────────────────

    #[test]
    fn parse_empty_type() {
        let source = "type Empty { }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("TypeDef"), "tree should contain TypeDef:\n{tree}");
    }

    // ── 52. Error: type missing brace ──────────────────────────

    #[test]
    fn error_type_missing_brace() {
        let source = "type X";
        let result = parse(FID, source);
        assert!(result.has_errors());
    }

    // ── 53. Pub type alongside pub fn ──────────────────────────

    #[test]
    fn parse_pub_type_and_pub_fn() {
        let source = "pub type Option(a) {\n  Some(a)\n  None\n}\npub fn main() { 1 }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("TypeDef"), "tree should contain TypeDef:\n{tree}");
        assert!(tree.contains("FnDef"), "tree should contain FnDef:\n{tree}");
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

    // ── Match expression tests ──────────────────────────────────────

    // ── 54. Match with constructor patterns (DoD case) ──────────────

    #[test]
    fn parse_match_constructor_patterns() {
        let source = "fn f() { match value { Some(x) -> x\nNone -> 0 } }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("MatchExpr"), "tree should contain MatchExpr:\n{tree}");
        let arm_count = tree.matches("MatchArm@").count();
        assert_eq!(arm_count, 2, "expected 2 MatchArm, got {arm_count}:\n{tree}");
        assert!(tree.contains("ConstructorPat"), "tree should contain ConstructorPat:\n{tree}");
        assert!(tree.contains("IdentPat"), "tree should contain IdentPat:\n{tree}");
    }

    // ── 55. Match with wildcard pattern ─────────────────────────────

    #[test]
    fn parse_match_wildcard() {
        let source = "fn f() { match b { 0 -> 1\n_ -> 2 } }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("MatchExpr"), "tree should contain MatchExpr:\n{tree}");
        assert!(tree.contains("WildcardPat"), "tree should contain WildcardPat:\n{tree}");
        assert!(tree.contains("LiteralPat"), "tree should contain LiteralPat:\n{tree}");
    }

    // ── 56. Match with multiple literal patterns ────────────────────

    #[test]
    fn parse_match_literal_patterns() {
        let source = "fn f() { match x { 1 -> 10\n2 -> 20\n_ -> 0 } }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        let arm_count = tree.matches("MatchArm@").count();
        assert_eq!(arm_count, 3, "expected 3 MatchArm, got {arm_count}:\n{tree}");
        let literal_pat_count = tree.matches("LiteralPat@").count();
        assert!(literal_pat_count >= 2, "expected 2+ LiteralPat, got {literal_pat_count}:\n{tree}");
    }

    // ── 57. Match with guard ────────────────────────────────────────

    #[test]
    fn parse_match_guard() {
        let source = "fn f() { match value { Some(n) if n > 100 -> n\nSome(n) -> n\nNone -> 0 } }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("Guard"), "tree should contain Guard:\n{tree}");
        let arm_count = tree.matches("MatchArm@").count();
        assert_eq!(arm_count, 3, "expected 3 MatchArm, got {arm_count}:\n{tree}");
    }

    // ── 58. Match with list patterns ────────────────────────────────

    #[test]
    fn parse_match_list_patterns() {
        let source = "fn f() { match items { [head, ..] -> head\n[] -> 0 } }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        let list_pat_count = tree.matches("ListPat@").count();
        assert_eq!(list_pat_count, 2, "expected 2 ListPat, got {list_pat_count}:\n{tree}");
    }

    // ── 59. Match with list rest binding ────────────────────────────

    #[test]
    fn parse_match_list_rest_binding() {
        let source = "fn f() { match items { [head, ..rest] -> head\n[] -> 0 } }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("ListPat"), "tree should contain ListPat:\n{tree}");
        assert!(tree.contains("DotDot"), "tree should contain DotDot for rest:\n{tree}");
    }

    // ── 60. Match with call expression in arm body ──────────────────

    #[test]
    fn parse_match_arm_call_body() {
        let source = r#"fn f() { match b { 0 -> Error("division by zero")
_ -> Ok(1) } }"#;
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        let call_count = tree.matches("CallExpr@").count();
        assert!(call_count >= 2, "expected 2+ CallExpr, got {call_count}:\n{tree}");
    }

    // ── 61. Match as expression in function body ────────────────────

    #[test]
    fn parse_match_as_expression() {
        let source = "fn f() { match x { 1 -> True\n_ -> False } }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("MatchExpr"), "tree should contain MatchExpr:\n{tree}");
        assert!(tree.contains("BlockExpr"), "MatchExpr should be inside BlockExpr:\n{tree}");
    }

    // ── 62. Empty match body ────────────────────────────────────────

    #[test]
    fn parse_match_empty_body() {
        let source = "fn f() { match x { } }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("MatchExpr"), "tree should contain MatchExpr:\n{tree}");
        let arm_count = tree.matches("MatchArm@").count();
        assert_eq!(arm_count, 0, "expected 0 MatchArm, got {arm_count}:\n{tree}");
    }

    // ── 63. Match with trailing comma in constructor ────────────────

    #[test]
    fn parse_match_constructor_trailing_comma() {
        let source = "fn f() { match x { Some(a,) -> a\nNone -> 0 } }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());
    }

    // ── 64. Nested match ────────────────────────────────────────────

    #[test]
    fn parse_nested_match() {
        let source =
            "fn f() { match x { Some(y) -> match y { 1 -> True\n_ -> False }\nNone -> False } }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        let match_count = tree.matches("MatchExpr@").count();
        assert_eq!(match_count, 2, "expected 2 MatchExpr, got {match_count}:\n{tree}");
    }

    // ── 65. Match with boolean literal patterns ─────────────────────

    #[test]
    fn parse_match_bool_patterns() {
        let source = "fn f() { match b { True -> 1\nFalse -> 0 } }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        let literal_pat_count = tree.matches("LiteralPat@").count();
        assert_eq!(literal_pat_count, 2, "expected 2 LiteralPat, got {literal_pat_count}:\n{tree}");
    }

    // ── 66. Match with string literal pattern ───────────────────────

    #[test]
    fn parse_match_string_pattern() {
        let source = r#"fn f() { match s { "hello" -> 1
_ -> 0 } }"#;
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("LiteralPat"), "tree should contain LiteralPat:\n{tree}");
        assert!(tree.contains("WildcardPat"), "tree should contain WildcardPat:\n{tree}");
    }

    // ── 67. Nested constructor patterns ─────────────────────────────

    #[test]
    fn parse_nested_constructor_patterns() {
        let source = "fn f() { match x { Ok(Some(n)) -> n\n_ -> 0 } }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        let constructor_count = tree.matches("ConstructorPat@").count();
        assert_eq!(
            constructor_count, 2,
            "expected 2 ConstructorPat (Ok + Some), got {constructor_count}:\n{tree}"
        );
    }

    // ── Match error recovery tests ──────────────────────────────────

    // ── 68. Error: missing `{` after match subject ──────────────────

    #[test]
    fn error_match_missing_lbrace() {
        let source = "fn f() { match x 1 -> 2 }";
        let result = parse(FID, source);
        assert!(result.has_errors());
        assert!(
            result.diagnostics().iter().any(|d| d.message.contains("block after match")),
            "should report missing block: {:?}",
            result.diagnostics()
        );
    }

    // ── 69. Error: missing `->` in arm ──────────────────────────────

    #[test]
    fn error_match_missing_arrow() {
        let source = "fn f() { match x { 1 2 } }";
        let result = parse(FID, source);
        assert!(result.has_errors());
        assert!(
            result
                .diagnostics()
                .iter()
                .any(|d| d.message.contains("->") || d.message.contains("Arrow")),
            "should report missing `->`: {:?}",
            result.diagnostics()
        );
    }

    // ── 70. Match with if-expression in arm body ────────────────────

    #[test]
    fn parse_match_arm_with_if_body() {
        let source = "fn f() { match x { 1 -> if y { 2 } else { 3 }\n_ -> 0 } }";
        let result = parse(FID, source);
        assert!(!result.has_errors(), "diagnostics: {:?}", result.diagnostics());

        let tree = debug_tree(source);
        assert!(tree.contains("MatchExpr"), "tree should contain MatchExpr:\n{tree}");
        assert!(tree.contains("IfExpr"), "tree should contain IfExpr in arm body:\n{tree}");
    }

    // ══════════════════════════════════════════════════════════════════
    // Issue 18 — Malformed input tests (error recovery)
    // ══════════════════════════════════════════════════════════════════

    // ── A. Top-level recovery ───────────────────────────────────────

    #[test]
    fn error_garbage_between_fns() {
        let source = "fn a() { 1 } @@@ fn b() { 2 }";
        let result = parse(FID, source);
        assert!(result.has_errors());
        let tree = debug_tree(source);
        let fn_count = tree.matches("FnDef@").count();
        assert!(fn_count >= 2, "expected 2 FnDef after recovery, got {fn_count}:\n{tree}");
        assert_eq!(result.syntax().to_string(), source, "lossless roundtrip");
    }

    #[test]
    fn error_multiple_garbage_top_level() {
        let source = "+ - * fn f() { 1 }";
        let result = parse(FID, source);
        assert!(result.has_errors());
        let tree = debug_tree(source);
        assert!(tree.contains("FnDef"), "should recover to parse FnDef:\n{tree}");
    }

    #[test]
    fn error_bare_number_then_type() {
        let source = "99 type X { A }";
        let result = parse(FID, source);
        assert!(result.has_errors());
        let tree = debug_tree(source);
        assert!(tree.contains("TypeDef"), "should recover to parse TypeDef:\n{tree}");
    }

    #[test]
    fn error_two_broken_items_recovery() {
        let source = "fn { fn a() { 1 } fn b() { 2 }";
        let result = parse(FID, source);
        assert!(result.has_errors());
        let tree = debug_tree(source);
        let fn_count = tree.matches("FnDef@").count();
        assert!(fn_count >= 2, "expected 2+ FnDef after recovery, got {fn_count}:\n{tree}");
    }

    // ── B. Function definition errors ───────────────────────────────

    #[test]
    fn error_fn_missing_name() {
        let source = "fn () { 1 }";
        let result = parse(FID, source);
        assert!(result.has_errors());
    }

    #[test]
    fn error_fn_missing_parens_and_body() {
        let source = "fn f";
        let result = parse(FID, source);
        assert!(result.has_errors());
    }

    #[test]
    fn error_fn_body_missing_rbrace_recovery() {
        let source = "fn a() { 1 fn b() { 2 }";
        let result = parse(FID, source);
        assert!(result.has_errors());
        let tree = debug_tree(source);
        // At least the second fn should be parsed via recovery
        assert!(tree.contains("FnDef"), "should contain FnDef:\n{tree}");
    }

    #[test]
    fn error_fn_return_type_missing_type() {
        let source = "fn f() -> { 1 }";
        let result = parse(FID, source);
        assert!(result.has_errors());
    }

    // ── C. Parameter errors ─────────────────────────────────────────

    #[test]
    fn error_param_missing_type() {
        let source = "fn f(x:) { 1 }";
        let result = parse(FID, source);
        assert!(result.has_errors());
    }

    #[test]
    fn error_param_extra_comma() {
        let source = "fn f(x: Int,, y: Int) { 1 }";
        let result = parse(FID, source);
        assert!(result.has_errors());
    }

    #[test]
    fn error_param_no_colon_no_type() {
        let source = "fn f(x y) { 1 }";
        let result = parse(FID, source);
        assert!(result.has_errors());
    }

    // ── D. Expression errors ────────────────────────────────────────

    #[test]
    fn error_unclosed_paren_expr() {
        let source = "fn f() { (1 + 2 }";
        let result = parse(FID, source);
        assert!(result.has_errors());
        assert!(
            result.diagnostics().iter().any(|d| d.message.contains("RParen")),
            "should report missing `)`: {:?}",
            result.diagnostics()
        );
    }

    #[test]
    fn error_double_operator() {
        let source = "fn f() { 1 + + 2 }";
        let result = parse(FID, source);
        // `+` is not a prefix operator, so second `+` triggers an error
        // Actually `-` and `!` are prefix, but `+` is not, so this should error
        // However the Pratt parser may parse `+ 2` as an error atom
        assert!(result.has_errors());
    }

    #[test]
    fn error_bare_operator_in_block() {
        let source = "fn f() { * }";
        let result = parse(FID, source);
        assert!(result.has_errors());
        let tree = debug_tree(source);
        assert!(tree.contains("NodeError"), "should contain NodeError:\n{tree}");
    }

    // ── E. Match expression errors ──────────────────────────────────

    #[test]
    fn error_match_arm_missing_body() {
        let source = "fn f() { match x { 1 -> } }";
        let result = parse(FID, source);
        assert!(result.has_errors());
    }

    #[test]
    fn error_match_multiple_bad_arms() {
        let source = "fn f() { match x { -> 1\n_ -> 0 } }";
        let result = parse(FID, source);
        assert!(result.has_errors());
    }

    #[test]
    fn error_match_unclosed_constructor() {
        let source = "fn f() { match x { Some(a -> 1 } }";
        let result = parse(FID, source);
        assert!(result.has_errors());
    }

    // ── F. Type definition errors ───────────────────────────────────

    #[test]
    fn error_type_missing_name() {
        let source = "type { A }";
        let result = parse(FID, source);
        assert!(result.has_errors());
    }

    #[test]
    fn error_type_unclosed_body_recovery() {
        let source = "type X { A\nfn f() { 1 }";
        let result = parse(FID, source);
        assert!(result.has_errors());
        let tree = debug_tree(source);
        // The fn should be parsed after type body recovery
        assert!(tree.contains("FnDef"), "should recover to parse FnDef:\n{tree}");
    }

    #[test]
    fn error_type_malformed_variant() {
        let source = "type X { 123 A B }";
        let result = parse(FID, source);
        assert!(result.has_errors());
        let tree = debug_tree(source);
        assert!(tree.contains("NodeError"), "should contain NodeError for 123:\n{tree}");
    }

    // ── G. Nested / consecutive errors ──────────────────────────────

    #[test]
    fn error_nested_bad_if_in_match() {
        let source = "fn f() { match x { 1 -> if y 2\n_ -> 0 } }";
        let result = parse(FID, source);
        assert!(result.has_errors());
    }

    #[test]
    fn error_nested_bad_call() {
        let source = "fn f() { g(1, , 3) }";
        let result = parse(FID, source);
        assert!(result.has_errors());
    }

    #[test]
    fn error_many_garbage_then_valid() {
        let source = "@@@ @@@ fn f() { 1 }";
        let result = parse(FID, source);
        assert!(result.has_errors());
        let tree = debug_tree(source);
        assert!(tree.contains("FnDef"), "should recover to parse FnDef:\n{tree}");
    }

    #[test]
    fn error_lossless_roundtrip_with_errors() {
        let source = "fn f() { * } fn g() { 2 }";
        let result = parse(FID, source);
        assert!(result.has_errors());
        assert_eq!(result.syntax().to_string(), source, "lossless roundtrip even with errors");
        let tree = debug_tree(source);
        let fn_count = tree.matches("FnDef@").count();
        assert_eq!(fn_count, 2, "both FnDefs should parse, got {fn_count}:\n{tree}");
    }
}
