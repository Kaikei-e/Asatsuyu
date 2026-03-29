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
mod prelude;

use std::path::PathBuf;

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

// ── Package generation (Issue 32–33) ──────────────────────────────

/// Configuration for Python package generation.
pub struct PackageConfig {
    /// Package name (used in directory name and `pyproject.toml`).
    pub name: String,
    /// Package version.
    pub version: String,
    /// Whether to include source-map comments (`# asty:L<n>`).
    pub source_map: bool,
}

/// A single file in the generated Python package.
pub struct GeneratedFile {
    /// Relative path within the output directory (e.g., `my_app/main.py`).
    pub path: PathBuf,
    /// File content.
    pub content: String,
}

/// A complete generated Python package.
pub struct GeneratedPackage {
    /// All files to write.
    pub files: Vec<GeneratedFile>,
}

/// Generate a Python package from a type-checked module.
///
/// Produces a `GeneratedPackage` containing the module source, prelude
/// (if needed), `__init__.py`, `__main__.py` (if `main` exists),
/// and `pyproject.toml`.
#[must_use]
pub fn emit_package(
    module: &ThirModule,
    config: &PackageConfig,
    source: Option<&str>,
) -> GeneratedPackage {
    let mut em = if config.source_map {
        if let Some(src) = source {
            emitter::Emitter::with_source_map(module, src)
        } else {
            emitter::Emitter::new(module)
        }
    } else {
        emitter::Emitter::new(module)
    };
    em.emit();
    let module_py = em.into_output();

    let pkg = &config.name;
    let mut files = Vec::new();

    // __init__.py
    files.push(GeneratedFile {
        path: PathBuf::from(format!("{pkg}/__init__.py")),
        content: String::new(),
    });

    // Main module
    files
        .push(GeneratedFile { path: PathBuf::from(format!("{pkg}/{pkg}.py")), content: module_py });

    // Prelude is emitted only when the generated package explicitly depends on it.
    if package_needs_prelude(module) {
        files.push(GeneratedFile {
            path: PathBuf::from(format!("{pkg}/asatsuyu_prelude.py")),
            content: prelude::PRELUDE_PY.to_string(),
        });
    }

    // __main__.py if a `main` function exists
    let has_main =
        module.functions.iter().any(|f| module.symbol_table.get(f.def_id).name.as_str() == "main");
    if has_main {
        files.push(GeneratedFile {
            path: PathBuf::from(format!("{pkg}/__main__.py")),
            content: format!(
                "from .{pkg} import main\n\nif __name__ == \"__main__\":\n    main()\n"
            ),
        });
    }

    // pyproject.toml
    files.push(GeneratedFile {
        path: PathBuf::from("pyproject.toml"),
        content: format!(
            "[build-system]\nrequires = [\"setuptools\"]\nbuild-backend = \"setuptools.build_meta\"\n\n[project]\nname = \"{}\"\nversion = \"{}\"\nrequires-python = \">=3.12\"\n",
            config.name, config.version,
        ),
    });

    GeneratedPackage { files }
}

fn package_needs_prelude(_module: &ThirModule) -> bool {
    // Prelude-backed builtins are not wired into the language surface yet.
    // Keep package output minimal until the compiler can reference them.
    false
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
        assert!(py.contains("def main() -> int:"), "output: {py}");
        assert!(py.contains("return 42"), "output: {py}");
        assert!(py.starts_with("# Generated by Asatsuyu"), "header: {py}");
    }

    // ── 3. greet.asty ───────────────────────────────────────────────

    #[test]
    fn emit_greet_asty() {
        let source = include_str!("../../../examples/greet.asty");
        let py = python_from_source(source);
        assert!(py.starts_with("# Generated by Asatsuyu"), "header: {py}");
        assert!(py.contains("def greet(name: str) -> str:"), "greet fn: {py}");
        assert!(py.contains("def add(x: int, y: int) -> int:"), "add fn: {py}");
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
        // def should not be indented (after the header).
        assert!(py.contains("\ndef f("), "no indent on def: {py}");
    }

    // ── 11. Output looks like valid Python ──────────────────────────

    #[test]
    fn emit_output_is_valid_python() {
        let py = python_from_source("pub fn main() { 42 }");
        // Basic structural checks.
        assert!(py.starts_with("# Generated by Asatsuyu"), "starts with header");
        assert!(py.contains("def main("), "has def");
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

    // ── Issue 29: Module structure ─────────────────────────────────

    #[test]
    fn emit_module_header() {
        let py = python_from_source("fn f() { 42 }");
        assert!(
            py.starts_with("# Generated by Asatsuyu"),
            "non-empty module should have header: {py}",
        );
    }

    #[test]
    fn emit_dataclass_import() {
        let py = python_from_source("type Color { Red Green Blue }\nfn f() { 1 }");
        assert!(
            py.contains("from dataclasses import dataclass"),
            "ADT module should import dataclass: {py}",
        );
    }

    #[test]
    fn emit_no_dataclass_import_without_adt() {
        let py = python_from_source("fn f() { 1 }");
        assert!(
            !py.contains("from dataclasses import dataclass"),
            "no ADT → no dataclass import: {py}",
        );
    }

    #[test]
    fn emit_plain_imports() {
        let py = python_from_source("import gleam.io\nfn f() { io }");
        assert!(py.contains("import gleam.io\n"), "plain import: {py}");
    }

    #[test]
    fn emit_import_aliases() {
        let py = python_from_source("import gleam.io as stdio\nfn f() { stdio }");
        assert!(py.contains("import gleam.io as stdio\n"), "aliased import: {py}");
    }

    // ── Issue 30: ADT as dataclasses ───────────────────────────────

    #[test]
    fn emit_record_type() {
        // Go-style record type: labels are preserved.
        let py = python_from_source("type User { name: String  age: Int }");
        assert!(py.contains("@dataclass(frozen=True, slots=True)"), "decorator: {py}");
        assert!(py.contains("class User:"), "class name: {py}");
        assert!(py.contains("    name: str"), "field name: {py}");
        assert!(py.contains("    age: int"), "field age: {py}");
        // Single-variant record should not have a type alias.
        assert!(!py.contains("type User ="), "no type alias for record: {py}");
    }

    #[test]
    fn emit_single_variant_adt() {
        // Variant-style single constructor: positional fields.
        let py = python_from_source("type Wrapper { Wrapper(String, Int) }");
        assert!(py.contains("class Wrapper:"), "class: {py}");
        assert!(py.contains("    _0: str"), "field 0: {py}");
        assert!(py.contains("    _1: int"), "field 1: {py}");
    }

    #[test]
    fn emit_option_adt() {
        let py = python_from_source("type Option(a) { Some(a) None }");
        // Two dataclasses.
        assert!(py.contains("class Some[T]:"), "Some class: {py}");
        assert!(py.contains("class None_:"), "None class (sanitized): {py}");
        assert!(py.contains("    _0: T"), "Some field: {py}");
        assert!(py.contains("    pass"), "None pass: {py}");
        // Type alias.
        assert!(py.contains("type Option[T] = Some[T] | None_"), "type alias: {py}");
    }

    #[test]
    fn emit_result_adt() {
        let py = python_from_source("type Result(a, e) { Ok(a) Error(e) }");
        assert!(py.contains("class Ok[T]:"), "Ok class: {py}");
        assert!(py.contains("class Error[U]:"), "Error class: {py}");
        assert!(py.contains("type Result[T, U] = Ok[T] | Error[U]"), "type alias: {py}");
    }

    #[test]
    fn emit_nullary_constructor() {
        let py = python_from_source("type Color { Red Green Blue }");
        assert!(py.contains("class Red:\n    pass"), "Red: {py}");
        assert!(py.contains("class Green:\n    pass"), "Green: {py}");
        assert!(py.contains("class Blue:\n    pass"), "Blue: {py}");
        assert!(py.contains("type Color = Red | Green | Blue"), "type alias: {py}");
    }

    #[test]
    fn emit_adt_before_functions() {
        let source = "type Color { Red Blue }\nfn f() { 1 }";
        let py = python_from_source(source);
        let class_pos = py.find("class Red").expect("Red class");
        let fn_pos = py.find("def f(").expect("f function");
        assert!(class_pos < fn_pos, "ADT should appear before functions: {py}");
    }

    #[test]
    fn emit_constructor_call_in_expression() {
        let source = "
            type Option(a) { Some(a) None }
            pub fn wrap(x: Int) -> Option(Int) { Some(x) }
        ";
        let py = python_from_source(source);
        assert!(py.contains("return Some(x)"), "constructor call: {py}");
    }

    #[test]
    fn emit_adt_with_match() {
        let source = "
            type Option(a) { Some(a) None }
            pub fn unwrap_or(opt: Option(Int), default: Int) -> Int {
                match opt {
                    Some(x) -> x
                    None -> default
                }
            }
        ";
        let py = python_from_source(source);
        assert!(py.contains("case Some(x):"), "Some pattern: {py}");
        assert!(py.contains("case None_():"), "None pattern (sanitized): {py}");
    }

    // ── Issue 31: Match/case emission tests ───────────────────────────

    #[test]
    fn emit_match_literal_int() {
        let py = python_from_source("pub fn f(x: Int) -> Int { match x { 0 -> 1  _ -> 2 } }");
        assert!(py.contains("match x:"), "match statement: {py}");
        assert!(py.contains("case 0:"), "literal 0 case: {py}");
        assert!(py.contains("case _:"), "wildcard case: {py}");
        assert!(py.contains("return 1"), "return in first arm: {py}");
        assert!(py.contains("return 2"), "return in wildcard arm: {py}");
    }

    #[test]
    fn emit_match_variable_binding() {
        let py = python_from_source("pub fn f(x: Int) -> Int { match x { n -> n } }");
        assert!(py.contains("case n:"), "variable binding: {py}");
        assert!(py.contains("return n"), "return bound var: {py}");
    }

    #[test]
    fn emit_match_constructor_return_position() {
        let source = "
            type Result(a, e) { Ok(a) Error(e) }
            pub fn check(r: Result(Int, String)) -> Int {
                match r {
                    Ok(v) -> v
                    Error(msg) -> 0
                }
            }
        ";
        let py = python_from_source(source);
        assert!(py.contains("case Ok(v):"), "Ok pattern: {py}");
        assert!(py.contains("case Error(msg):"), "Error pattern: {py}");
        assert!(py.contains("return v"), "return from Ok: {py}");
        assert!(py.contains("return 0"), "return from Error: {py}");
    }

    #[test]
    fn emit_match_nested_constructor() {
        let source = "
            type Option(a) { Some(a) None }
            type Result(a, e) { Ok(a) Error(e) }
            pub fn deep(r: Result(Option(Int), String)) -> Int {
                match r {
                    Ok(Some(x)) -> x
                    Ok(None) -> 0
                    Error(msg) -> 0
                }
            }
        ";
        let py = python_from_source(source);
        assert!(py.contains("case Ok(Some(x)):"), "nested Ok(Some(x)): {py}");
        assert!(py.contains("case Ok(None_()):"), "nested Ok(None): {py}");
    }

    #[test]
    fn emit_match_statement_not_return() {
        let source = "
            pub fn f(x: Int) -> Int {
                match x {
                    0 -> 0
                    _ -> 1
                }
                42
            }
        ";
        let py = python_from_source(source);
        // match is NOT in return position; the `42` at the end is the return.
        assert!(py.contains("match x:"), "match statement: {py}");
        assert!(py.contains("return 42"), "return 42 at end: {py}");
        // Arms in statement-position match should NOT have `return`.
        let match_section = &py[py.find("match x:").unwrap()..py.find("return 42").unwrap()];
        assert!(!match_section.contains("return"), "no return inside statement match: {py}");
    }

    #[test]
    fn emit_match_multiple_arms() {
        let source = "
            type Color { Red Green Blue }
            pub fn code(c: Color) -> Int {
                match c {
                    Red -> 1
                    Green -> 2
                    Blue -> 3
                }
            }
        ";
        let py = python_from_source(source);
        assert!(py.contains("case Red():"), "Red: {py}");
        assert!(py.contains("case Green():"), "Green: {py}");
        assert!(py.contains("case Blue():"), "Blue: {py}");
        assert!(py.contains("return 1"), "return 1: {py}");
        assert!(py.contains("return 2"), "return 2: {py}");
        assert!(py.contains("return 3"), "return 3: {py}");
    }

    // ── Issue 32: Prelude generation tests ────────────────────────────

    #[test]
    fn prelude_contains_ok_error() {
        let content = crate::prelude::PRELUDE_PY;
        assert!(content.contains("class Ok[T]:"), "Ok class: {content}");
        assert!(content.contains("class Error[E]:"), "Error class: {content}");
        assert!(
            content.contains("type Result[T, E] = Ok[T] | Error[E]"),
            "Result alias: {content}",
        );
    }

    #[test]
    fn prelude_uses_frozen_dataclass() {
        let content = crate::prelude::PRELUDE_PY;
        assert!(
            content.contains("@dataclass(frozen=True, slots=True)"),
            "frozen dataclass: {content}",
        );
    }

    #[test]
    fn prelude_is_valid_module() {
        let content = crate::prelude::PRELUDE_PY;
        assert!(content.contains("from dataclasses import dataclass"), "import: {content}");
        assert!(content.ends_with('\n'), "trailing newline");
    }

    // ── Issue 33: Package generation tests ────────────────────────────

    fn package_from_source(source: &str, name: &str) -> super::GeneratedPackage {
        let cst = parse(FID, source);
        let ast = asatsuyu_ast::lower(&cst, FID);
        let hir = asatsuyu_hir::lower_to_hir(&ast.module);
        let thir = asatsuyu_ty::check_types(&hir.module);
        let config = super::PackageConfig {
            name: name.to_string(),
            version: "0.1.0".into(),
            source_map: false,
        };
        super::emit_package(&thir.module, &config, None)
    }

    #[test]
    fn package_contains_expected_files() {
        let pkg = package_from_source("pub fn main() { 42 }", "hello");
        let paths: Vec<String> = pkg.files.iter().map(|f| f.path.display().to_string()).collect();
        assert!(paths.contains(&"hello/__init__.py".to_string()), "init: {paths:?}");
        assert!(paths.contains(&"hello/hello.py".to_string()), "module: {paths:?}");
        assert!(paths.contains(&"hello/__main__.py".to_string()), "main: {paths:?}");
        assert!(paths.contains(&"pyproject.toml".to_string()), "pyproject: {paths:?}");
    }

    #[test]
    fn package_omits_prelude_when_unused() {
        let pkg = package_from_source("pub fn main() { 42 }", "hello");
        let paths: Vec<String> = pkg.files.iter().map(|f| f.path.display().to_string()).collect();
        assert!(
            !paths.contains(&"hello/asatsuyu_prelude.py".to_string()),
            "prelude should be omitted when unused: {paths:?}",
        );
    }

    #[test]
    fn package_no_main_without_main_fn() {
        let pkg = package_from_source("fn add(x: Int, y: Int) -> Int { x }", "lib");
        let paths: Vec<String> = pkg.files.iter().map(|f| f.path.display().to_string()).collect();
        assert!(!paths.contains(&"lib/__main__.py".to_string()), "no __main__: {paths:?}");
    }

    #[test]
    fn package_pyproject_content() {
        let pkg = package_from_source("pub fn main() { 42 }", "myapp");
        let pyproject = pkg
            .files
            .iter()
            .find(|f| f.path.display().to_string() == "pyproject.toml")
            .expect("pyproject.toml");
        assert!(pyproject.content.contains("name = \"myapp\""), "name: {}", pyproject.content);
        assert!(
            pyproject.content.contains("version = \"0.1.0\""),
            "version: {}",
            pyproject.content,
        );
        assert!(
            pyproject.content.contains("requires-python = \">=3.12\""),
            "python: {}",
            pyproject.content,
        );
    }

    #[test]
    fn package_main_py_content() {
        let pkg = package_from_source("pub fn main() { 42 }", "hello");
        let main_py = pkg
            .files
            .iter()
            .find(|f| f.path.display().to_string() == "hello/__main__.py")
            .expect("__main__.py");
        assert!(
            main_py.content.contains("from .hello import main"),
            "import main: {}",
            main_py.content,
        );
        assert!(
            main_py.content.contains("if __name__ == \"__main__\":"),
            "guard: {}",
            main_py.content,
        );
    }

    // ── Issue 34: Source-map comment tests ─────────────────────────────

    fn python_from_source_with_sourcemap(source: &str) -> String {
        let cst = parse(FID, source);
        let ast = asatsuyu_ast::lower(&cst, FID);
        let hir = asatsuyu_hir::lower_to_hir(&ast.module);
        let thir = asatsuyu_ty::check_types(&hir.module);
        let config = super::PackageConfig {
            name: "test".to_string(),
            version: "0.1.0".into(),
            source_map: true,
        };
        let pkg = super::emit_package(&thir.module, &config, Some(source));
        pkg.files
            .into_iter()
            .find(|f| f.path.display().to_string() == "test/test.py")
            .expect("module file")
            .content
    }

    #[test]
    fn sourcemap_comments_present() {
        let py = python_from_source_with_sourcemap("pub fn main() { 42 }");
        assert!(py.contains("# asty:L"), "source-map comment present: {py}");
    }

    #[test]
    fn sourcemap_comments_absent_when_disabled() {
        let py = python_from_source("pub fn main() { 42 }");
        assert!(!py.contains("# asty:L"), "no source-map in plain emit: {py}");
    }

    #[test]
    fn sourcemap_line_numbers_correct() {
        let source = "pub fn main() -> Int {\n  42\n}";
        let py = python_from_source_with_sourcemap(source);
        // `def main()` maps to line 1
        assert!(py.contains("# asty:L1"), "fn def at L1: {py}");
    }

    #[test]
    fn sourcemap_annotates_let_bindings() {
        let source = "pub fn main() -> Int {\n  let x = 42\n  x\n}";
        let py = python_from_source_with_sourcemap(source);
        assert!(py.contains("x = 42  # asty:L2"), "let binding should carry source-map: {py}");
    }
}
