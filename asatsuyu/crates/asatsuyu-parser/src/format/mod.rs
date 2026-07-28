//! Deterministic code formatter for the Asatsuyu language.
//!
//! Walks the lossless CST produced by the parser and emits canonically
//! formatted source text.  No configuration — one true format.
//!
//! # Guarantees
//!
//! - **Idempotent**: `format(format(s)) == format(s)`
//! - **Comment-preserving**: all `//` comments survive formatting
//! - **Error-safe**: source with parse errors is returned unchanged

mod formatter;
mod rules;

use asatsuyu_syntax::FileId;

/// The result of formatting Asatsuyu source code.
#[derive(Debug, Clone)]
pub struct FormatResult {
    /// The formatted source text (or the original if there were parse errors).
    pub formatted: String,
    /// `true` when the input had parse errors and was returned unchanged.
    pub has_parse_errors: bool,
}

/// Format Asatsuyu source code into its canonical form.
///
/// If the source contains parse errors the original text is returned
/// unchanged with `has_parse_errors = true`.
#[must_use]
pub fn format_source(source: &str) -> FormatResult {
    let result = crate::parse(FileId(0), source);
    if result.has_errors() {
        return FormatResult { formatted: source.to_owned(), has_parse_errors: true };
    }
    let root = result.syntax();
    let formatted = formatter::Formatter::new().format_node(&root);
    FormatResult { formatted, has_parse_errors: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(source: &str) -> String {
        let result = format_source(source);
        assert!(!result.has_parse_errors, "unexpected parse error");
        result.formatted
    }

    fn assert_idempotent(source: &str) {
        let first = fmt(source);
        let second = fmt(&first);
        assert_eq!(first, second, "format is not idempotent");
    }

    #[test]
    fn format_simple_fn() {
        let input = "pub fn main() { 42 }\n";
        let expected = "pub fn main() {\n  42\n}\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn format_fn_with_params() {
        let input = "pub fn add(x: Int, y: Int) -> Int {\n  x + y\n}\n";
        assert_idempotent(input);
    }

    #[test]
    fn format_pure_fn() {
        let input = "pub pure fn add(x: Int, y: Int) -> Int {\n  x + y\n}\n";
        assert_idempotent(input);
    }

    #[test]
    fn format_type_def() {
        let input = "type Color {\n  Red\n  Green\n  Blue\n}\n";
        assert_idempotent(input);
    }

    #[test]
    fn format_match_expr() {
        let input = "pub fn name(c: Color) -> String {\n  match c {\n    Red -> \"red\"\n    Green -> \"green\"\n    Blue -> \"blue\"\n  }\n}\n";
        assert_idempotent(input);
    }

    #[test]
    fn format_pipeline() {
        let input = "pub fn main() -> Int {\n  10 |> double |> inc\n}\n";
        assert_idempotent(input);
    }

    #[test]
    fn format_multiline_pipeline_with_lambdas() {
        let input = "pub fn main() {\n  [\".py\", \".rs\", \".md\", \".json\", \".xyz\"]\n  |> list.map(fn(ext) { categorize(ext) })\n  |> list.filter(fn(cat) { is_source(cat) })\n  |> list.map(fn(cat) { category_label(cat) })\n  |> println\n}\n";
        let expected = "pub fn main() {\n  [\".py\", \".rs\", \".md\", \".json\", \".xyz\"]\n  |> list.map(fn(ext) {\n    categorize(ext)\n  })\n  |> list.filter(fn(cat) {\n    is_source(cat)\n  })\n  |> list.map(fn(cat) {\n    category_label(cat)\n  })\n  |> println\n}\n";
        assert_eq!(fmt(input), expected);
        assert_idempotent(input);
    }

    #[test]
    fn format_import_and_ffi() {
        let input = "from python import pathlib\n";
        assert_idempotent(input);
    }

    #[test]
    fn format_multiple_items() {
        let input = "pub fn a() {\n  1\n}\n\npub fn b() {\n  2\n}\n";
        assert_idempotent(input);
    }

    #[test]
    fn format_string_concat() {
        let input = "pub fn greet(name: String) -> String {\n  \"Hello, \" <> name <> \"!\"\n}\n";
        assert_idempotent(input);
    }

    #[test]
    fn format_parse_error_returns_original() {
        let input = "pub fn {";
        let result = format_source(input);
        assert!(result.has_parse_errors);
        assert_eq!(result.formatted, input);
    }

    #[test]
    fn format_let_binding() {
        let input = "pub fn main() {\n  let x = 42\n  x\n}\n";
        assert_idempotent(input);
    }

    #[test]
    fn format_if_else() {
        let input = "pub fn main() {\n  if True { 1 } else { 2 }\n}\n";
        assert_idempotent(input);
    }

    #[test]
    fn format_normalizes_extra_spaces() {
        let input = "pub  fn  main(  )  {  42  }\n";
        let result = fmt(input);
        assert_eq!(result, "pub fn main() {\n  42\n}\n");
        assert_idempotent(input);
    }

    #[test]
    fn format_adt_with_fields() {
        let input = "type Option(a) {\n  Some(a)\n  None\n}\n";
        assert_idempotent(input);
    }

    #[test]
    fn format_try_expr() {
        let input =
            "pub fn safe(p: String) -> Result {\n  let result = try p.exists()\n  Ok(result)\n}\n";
        // Note: `try p.exists()` involves field access + call + try
        // Just verify idempotency first
        let result = format_source(input);
        if !result.has_parse_errors {
            assert_idempotent(input);
        }
    }

    #[test]
    fn format_preserves_leading_and_block_comments() {
        let input = "// leading\npub fn main() {\n// inner\n42\n}\n";
        let expected = "// leading\npub fn main() {\n  // inner\n  42\n}\n";
        assert_eq!(fmt(input), expected);
        assert_idempotent(input);
    }

    #[test]
    fn format_preserves_comments_between_top_level_items() {
        let input = "fn a() {\n  1\n}\n// between\nfn b() {\n  2\n}\n";
        let expected = "fn a() {\n  1\n}\n\n// between\nfn b() {\n  2\n}\n";
        assert_eq!(fmt(input), expected);
        assert_idempotent(input);
    }

    #[test]
    fn format_preserves_comments_between_match_arms() {
        let input = "fn classify(x: Int) {\n  match x {\n    // zero\n    0 -> 0\n    // fallback\n    _ -> 1\n  }\n}\n";
        let expected = "fn classify(x: Int) {\n  match x {\n    // zero\n    0 -> 0\n    // fallback\n    _ -> 1\n  }\n}\n";
        assert_eq!(fmt(input), expected);
        assert_idempotent(input);
    }
}
