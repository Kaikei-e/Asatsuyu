//! Integration test: parse actual `.asty` files from the examples directory.

use asatsuyu_parser::parse;
use asatsuyu_syntax::FileId;

const HELLO: &str = include_str!("../../../examples/hello.asty");
const GREET: &str = include_str!("../../../examples/greet.asty");
const MATCH_BASIC: &str = include_str!("../../../examples/match_basic.asty");

#[test]
fn parse_hello_asty_no_errors() {
    let result = parse(FileId(0), HELLO);
    assert!(!result.has_errors(), "hello.asty produced errors: {:?}", result.diagnostics());
}

#[test]
fn parse_hello_asty_lossless_roundtrip() {
    let result = parse(FileId(0), HELLO);
    assert_eq!(result.syntax().to_string(), HELLO, "roundtrip mismatch for hello.asty");
}

#[test]
fn parse_hello_asty_tree_shape() {
    let result = parse(FileId(0), HELLO);
    let tree = format!("{:#?}", result.syntax());

    // Print tree for manual inspection
    eprintln!("=== CST for hello.asty ===\n{tree}");

    assert!(tree.contains("SourceFile"), "missing SourceFile root");
    assert!(tree.contains("FnDef"), "missing FnDef");
    assert!(tree.contains("Visibility"), "missing Visibility (pub)");
    assert!(tree.contains("ParamList"), "missing ParamList");
    assert!(tree.contains("BlockExpr"), "missing BlockExpr");
    assert!(tree.contains("LiteralExpr"), "missing LiteralExpr (42)");
}

#[test]
fn parse_greet_asty_no_errors() {
    let result = parse(FileId(0), GREET);
    assert!(!result.has_errors(), "greet.asty produced errors: {:?}", result.diagnostics());
}

#[test]
fn parse_greet_asty_tree_shape() {
    let result = parse(FileId(0), GREET);
    let tree = format!("{:#?}", result.syntax());
    eprintln!("=== CST for greet.asty ===\n{tree}");

    // Two function definitions
    let fn_count = tree.matches("FnDef@").count();
    assert_eq!(fn_count, 2, "expected 2 FnDef, got {fn_count}");

    // First fn has Visibility (pub), second doesn't
    assert!(tree.contains("Visibility"), "missing Visibility for pub fn");
    assert!(tree.contains("ReturnType"), "missing ReturnType");
    assert!(tree.contains("Param@"), "missing Param");
}

#[test]
fn parse_greet_asty_lossless_roundtrip() {
    let result = parse(FileId(0), GREET);
    assert_eq!(result.syntax().to_string(), GREET, "roundtrip mismatch for greet.asty");
}

#[test]
fn parse_match_basic_asty_no_errors() {
    let result = parse(FileId(0), MATCH_BASIC);
    assert!(!result.has_errors(), "match_basic.asty produced errors: {:?}", result.diagnostics());
}

#[test]
fn parse_match_basic_asty_tree_shape() {
    let result = parse(FileId(0), MATCH_BASIC);
    let tree = format!("{:#?}", result.syntax());
    eprintln!("=== CST for match_basic.asty ===\n{tree}");

    // Three function definitions + one type definition
    let fn_count = tree.matches("FnDef@").count();
    assert_eq!(fn_count, 3, "expected 3 FnDef, got {fn_count}");
    assert!(tree.contains("TypeDef"), "missing TypeDef");

    // Three match expressions
    let match_count = tree.matches("MatchExpr@").count();
    assert_eq!(match_count, 3, "expected 3 MatchExpr, got {match_count}");

    // Pattern types present
    assert!(tree.contains("WildcardPat"), "missing WildcardPat");
    assert!(tree.contains("LiteralPat"), "missing LiteralPat");
    assert!(tree.contains("ConstructorPat"), "missing ConstructorPat");
    assert!(tree.contains("IdentPat"), "missing IdentPat");
    assert!(tree.contains("ListPat"), "missing ListPat");
}

#[test]
fn parse_match_basic_asty_lossless_roundtrip() {
    let result = parse(FileId(0), MATCH_BASIC);
    assert_eq!(result.syntax().to_string(), MATCH_BASIC, "roundtrip mismatch for match_basic.asty");
}
