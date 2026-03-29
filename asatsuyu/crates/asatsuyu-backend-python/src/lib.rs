//! Python 3.12+ code generation for the Asatsuyu language.
//!
//! Transforms THIR into readable Python source code with type annotations.
//!
//! # Usage
//!
//! ```
//! use asatsuyu_ast::lower;
//! use asatsuyu_hir::lower_to_hir;
//! use asatsuyu_parser::parse;
//! use asatsuyu_syntax::FileId;
//! use asatsuyu_ty::check_types;
//! use asatsuyu_backend_python::emit_module;
//!
//! let cst = parse(FileId(0), "pub fn main() { 42 }");
//! let ast = lower(&cst, FileId(0));
//! let hir = lower_to_hir(&ast.module);
//! let thir = check_types(&hir.module);
//! let python = emit_module(&thir.module);
//! assert!(python.contains("def main()"));
//! assert!(python.contains("return 42"));
//! ```

mod emitter;

use asatsuyu_ty::ThirModule;

/// Emit Python 3.12+ source code from a type-checked module.
///
/// Produces readable Python with type annotations. Functions are separated
/// by two blank lines per PEP 8.
#[must_use]
pub fn emit_module(module: &ThirModule) -> String {
    let mut em = emitter::Emitter::new(module);
    em.emit();
    em.into_output()
}

#[cfg(test)]
mod tests {
    use super::*;
    use asatsuyu_parser::parse;
    use asatsuyu_syntax::FileId;

    const FID: FileId = FileId(0);

    /// Helper: source → CST → AST → HIR → THIR → Python.
    fn python_from_source(source: &str) -> String {
        let cst = parse(FID, source);
        let ast = asatsuyu_ast::lower(&cst, FID);
        let hir = asatsuyu_hir::lower_to_hir(&ast.module);
        let thir = asatsuyu_ty::check_types(&hir.module);
        emit_module(&thir.module)
    }

    // ── 1. Empty module ─────────────────────────────────────────────

    #[test]
    fn emit_empty_module() {
        let py = python_from_source("");
        assert_eq!(py, "");
    }

    // ── 2. hello.asty ───────────────────────────────────────────────

    #[test]
    fn emit_hello_asty() {
        let source = include_str!("../../../examples/hello.asty");
        let py = python_from_source(source);
        assert_eq!(py, "def main() -> int:\n    return 42\n");
    }

    // ── 3. greet.asty ───────────────────────────────────────────────

    #[test]
    fn emit_greet_asty() {
        let source = include_str!("../../../examples/greet.asty");
        let py = python_from_source(source);
        let expected = "\
def greet(name: str) -> str:
    return \"hello\"


def add(x: int, y: int) -> int:
    return x
";
        assert_eq!(py, expected);
    }

    // ── 4. String literal ───────────────────────────────────────────

    #[test]
    fn emit_string_literal() {
        let py = python_from_source(r#"fn f() { "hello" }"#);
        assert!(py.contains("return \"hello\""), "output: {py}");
    }

    // ── 5. Param types ──────────────────────────────────────────────

    #[test]
    fn emit_param_types() {
        let py = python_from_source("fn add(x: Int, y: Int) -> Int { x }");
        assert!(py.contains("x: int, y: int"), "output: {py}");
        assert!(py.contains("-> int:"), "output: {py}");
    }

    // ── 6. None return + empty body ─────────────────────────────────

    #[test]
    fn emit_none_return() {
        let py = python_from_source("fn f() { }");
        assert!(py.contains("-> None:"), "output: {py}");
        assert!(py.contains("pass"), "output: {py}");
    }

    // ── 7. Multi-expression block ───────────────────────────────────

    #[test]
    fn emit_multi_expr_block() {
        let py = python_from_source(r#"fn f() { 1 "hi" }"#);
        // First expr as statement, last as return.
        assert!(py.contains("    1\n"), "first expr as stmt: {py}");
        assert!(py.contains("    return \"hi\"\n"), "last expr as return: {py}");
    }

    // ── 8. Variable reference ───────────────────────────────────────

    #[test]
    fn emit_variable_ref() {
        let py = python_from_source("fn id(x: Int) -> Int { x }");
        assert!(py.contains("return x"), "output: {py}");
    }

    // ── 9. Two blank lines between functions ────────────────────────

    #[test]
    fn emit_two_blank_lines() {
        let py = python_from_source("fn a() { 1 }\nfn b() { 2 }");
        assert!(py.contains(":\n    return 1\n\n\ndef b("), "two blank lines: {py}");
    }

    // ── 10. Indentation ─────────────────────────────────────────────

    #[test]
    fn emit_indentation() {
        let py = python_from_source("fn f() { 42 }");
        // Body should be indented with 4 spaces.
        assert!(py.contains("    return 42"), "4-space indent: {py}");
        // def should not be indented.
        assert!(py.starts_with("def "), "no indent on def: {py}");
    }

    // ── 11. Output looks like valid Python ──────────────────────────

    #[test]
    fn emit_output_is_valid_python() {
        let py = python_from_source("pub fn main() { 42 }");
        // Basic structural checks.
        assert!(py.starts_with("def "), "starts with def");
        assert!(py.contains(":\n"), "has colon + newline");
        assert!(py.ends_with('\n'), "ends with newline");
    }

    // ── 12. No error types in valid output ──────────────────────────

    #[test]
    fn emit_readable() {
        let py = python_from_source(
            "pub fn greet(name: String) -> String { \"hello\" }\nfn add(x: Int, y: Int) -> Int { x }",
        );
        assert!(!py.contains("Any"), "no Any in valid output: {py}");
        assert!(!py.contains("<error>"), "no <error> in valid output: {py}");
    }

    #[test]
    fn emit_string_concat_as_python_plus() {
        let py = python_from_source(r#"fn f(name: String) -> String { "Hello, " <> name }"#);
        assert!(py.contains("return (\"Hello, \" + name)"), "output: {py}");
        assert!(!py.contains("string_concat("), "output: {py}");
    }
}
