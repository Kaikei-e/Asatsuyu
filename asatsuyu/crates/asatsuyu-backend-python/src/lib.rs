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
mod runtime_shim;

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

/// Controls whether the `PyO3` runtime extension is included in the generated package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FfiRuntimeMode {
    /// Always include the runtime extension files (maturin layout).
    On,
    /// Never include the runtime extension (pure Python prelude shim only).
    Off,
    /// Auto-detect from code: include only when Checked FFI calls are present.
    #[default]
    Auto,
}

/// Configuration for Python package generation.
pub struct PackageConfig {
    /// Package name (used in `pyproject.toml` `[project] name`).
    pub name: String,
    /// Package version.
    pub version: String,
    /// Whether to include source-map comments (`# asty:L<n>`).
    pub source_map: bool,
    /// Controls whether the `PyO3` runtime extension is emitted.
    pub ffi_runtime: FfiRuntimeMode,
    /// Python version constraint (PEP 440, e.g., `">=3.12"`). Defaults to `">=3.12"`.
    pub requires_python: Option<String>,
    /// Python dependency specifiers (PEP 508, e.g., `["requests>=2.31"]`).
    pub dependencies: Vec<String>,
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

/// Convert a project name to a valid Python package directory name.
///
/// Replaces hyphens and dots with underscores and lowercases.
/// E.g., `"my-app"` → `"my_app"`, `"hello"` → `"hello"`.
fn python_package_name(name: &str) -> String {
    name.chars().map(|c| if c == '-' || c == '.' { '_' } else { c.to_ascii_lowercase() }).collect()
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
    let needs_runtime_shim = match config.ffi_runtime {
        FfiRuntimeMode::On => true,
        FfiRuntimeMode::Off => false,
        FfiRuntimeMode::Auto => em.has_checked_ffi,
    };
    let needs_prelude = em.has_try || needs_runtime_shim;
    let module_py = em.into_output();

    let pkg_dir = python_package_name(&config.name);
    let mut files = Vec::new();

    // All Python source lives under python/{pkg_dir}/ (maturin best practice).
    // __init__.py
    files.push(GeneratedFile {
        path: PathBuf::from(format!("python/{pkg_dir}/__init__.py")),
        content: String::new(),
    });

    // Main module
    files.push(GeneratedFile {
        path: PathBuf::from(format!("python/{pkg_dir}/{pkg_dir}.py")),
        content: module_py,
    });

    // py.typed marker (PEP 561) — always emitted.
    files.push(GeneratedFile {
        path: PathBuf::from(format!("python/{pkg_dir}/py.typed")),
        content: String::new(),
    });

    // Prelude is emitted when the generated code uses `try` expressions.
    if needs_prelude {
        files.push(GeneratedFile {
            path: PathBuf::from(format!("python/{pkg_dir}/asatsuyu_prelude.py")),
            content: prelude::PRELUDE_PY.to_string(),
        });
    }

    // Pure-Python runtime shim (fallback when native extension is not built).
    if needs_runtime_shim {
        files.push(GeneratedFile {
            path: PathBuf::from(format!("python/{pkg_dir}/_asatsuyu_runtime.py")),
            content: runtime_shim::RUNTIME_SHIM_PY.to_string(),
        });
        // Type stubs for the native extension (IDE support).
        files.push(GeneratedFile {
            path: PathBuf::from(format!("python/{pkg_dir}/_asatsuyu_runtime.pyi")),
            content: runtime_shim::RUNTIME_STUB_PYI.to_string(),
        });
        // Maturin wrapper crate files.
        files.push(GeneratedFile {
            path: PathBuf::from("src/lib.rs"),
            content: runtime_shim::MATURIN_LIB_RS.to_string(),
        });
        files.push(GeneratedFile {
            path: PathBuf::from("Cargo.toml"),
            content: runtime_shim::maturin_cargo_toml(&pkg_dir),
        });
    }

    // __main__.py if a `main` function exists
    let has_main =
        module.functions.iter().any(|f| module.symbol_table.get(f.def_id).name.as_str() == "main");
    if has_main {
        files.push(GeneratedFile {
            path: PathBuf::from(format!("python/{pkg_dir}/__main__.py")),
            content: format!(
                "from .{pkg_dir} import main\n\nif __name__ == \"__main__\":\n    main()\n"
            ),
        });
    }

    // pyproject.toml
    let pyproject = generate_pyproject_toml(config, &pkg_dir, has_main, needs_runtime_shim);
    files.push(GeneratedFile { path: PathBuf::from("pyproject.toml"), content: pyproject });

    GeneratedPackage { files }
}

/// Generate `pyproject.toml` content from package configuration.
///
/// Produces a standards-compliant file with `[build-system]`, `[project]`
/// (including optional `dependencies` and `scripts`), and tool-specific
/// sections.
fn generate_pyproject_toml(
    config: &PackageConfig,
    pkg_dir: &str,
    has_main: bool,
    needs_runtime: bool,
) -> String {
    use std::fmt::Write;

    let mut out = String::with_capacity(512);

    // ── [build-system] ────────────────────────────────────────────
    if needs_runtime {
        out.push_str(
            "[build-system]\nrequires = [\"maturin>=1.9,<2\"]\nbuild-backend = \"maturin\"\n",
        );
    } else {
        out.push_str(
            "[build-system]\nrequires = [\"setuptools>=75\"]\nbuild-backend = \"setuptools.build_meta\"\n",
        );
    }

    // ── [project] ─────────────────────────────────────────────────
    let requires_python = config.requires_python.as_deref().unwrap_or(">=3.12");
    let _ = write!(
        out,
        "\n[project]\nname = \"{name}\"\nversion = \"{ver}\"\nrequires-python = \"{requires_python}\"\n",
        name = config.name,
        ver = config.version,
    );

    // dependencies (only if non-empty)
    if !config.dependencies.is_empty() {
        out.push_str("dependencies = [\n");
        for dep in &config.dependencies {
            let _ = writeln!(out, "    \"{dep}\",");
        }
        out.push_str("]\n");
    }

    // [project.scripts] (only if main() exists)
    if has_main {
        let _ = write!(
            out,
            "\n[project.scripts]\n{name} = \"{pkg_dir}.{pkg_dir}:main\"\n",
            name = config.name,
        );
    }

    // ── Tool-specific sections ────────────────────────────────────
    if needs_runtime {
        let _ = write!(
            out,
            "\n[tool.maturin]\npython-source = \"python\"\nmodule-name = \"{pkg_dir}._asatsuyu_runtime\"\n",
        );
    } else {
        out.push_str("\n[tool.setuptools.packages.find]\nwhere = [\"python\"]\n");
    }

    // [tool.asatsuyu] — compiler metadata
    let _ =
        write!(out, "\n[tool.asatsuyu]\ncompiler-version = \"{}\"\n", env!("CARGO_PKG_VERSION"),);

    out
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
        assert!(py.contains("def main() -> None:"), "main fn: {py}");
        assert!(py.contains("print(greet(\"world\"))"), "println call: {py}");
    }

    #[test]
    fn emit_async_def_and_await() {
        let source = "\
async fn inner() -> Int { 1 }
pub async fn fetch() -> Int { await inner() }";
        let py = python_from_source(source);
        assert!(py.contains("async def fetch() -> int:"), "should emit async def: {py}");
        assert!(
            py.contains("return await inner()"),
            "should emit await expression in Python: {py}"
        );
    }

    #[test]
    fn emit_task_type_as_coroutine_annotation() {
        let source = "fn schedule(task: Task(Int)) -> Task(Int) { task }";
        let py = python_from_source(source);
        assert!(
            py.contains("from collections.abc import Coroutine"),
            "Task types should trigger Coroutine import: {py}"
        );
        assert!(py.contains("from typing import Any"), "Task types should import Any: {py}");
        assert!(
            py.contains("task: Coroutine[Any, Any, int]"),
            "Task(Int) parameter should map to Coroutine[Any, Any, int]: {py}"
        );
        assert!(
            py.contains("-> Coroutine[Any, Any, int]:"),
            "Task(Int) return type should map to Coroutine[Any, Any, int]: {py}"
        );
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
    fn prelude_classifies_only_keyerror_as_keyerror() {
        let content = crate::prelude::PRELUDE_PY;
        assert!(content.contains("if isinstance(e, KeyError):"), "KeyError branch: {content}");
        assert!(!content.contains("LookupError"), "should not collapse all LookupError: {content}");
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
            ffi_runtime: super::FfiRuntimeMode::Auto,
            requires_python: None,
            dependencies: Vec::new(),
        };
        super::emit_package(&thir.module, &config, None)
    }

    #[test]
    fn package_contains_expected_files() {
        let pkg = package_from_source("pub fn main() { 42 }", "hello");
        let paths: Vec<String> = pkg.files.iter().map(|f| f.path.display().to_string()).collect();
        assert!(paths.contains(&"python/hello/__init__.py".to_string()), "init: {paths:?}");
        assert!(paths.contains(&"python/hello/hello.py".to_string()), "module: {paths:?}");
        assert!(paths.contains(&"python/hello/__main__.py".to_string()), "main: {paths:?}");
        assert!(paths.contains(&"pyproject.toml".to_string()), "pyproject: {paths:?}");
    }

    #[test]
    fn package_omits_prelude_when_unused() {
        let pkg = package_from_source("pub fn main() { 42 }", "hello");
        let paths: Vec<String> = pkg.files.iter().map(|f| f.path.display().to_string()).collect();
        assert!(
            !paths.contains(&"python/hello/asatsuyu_prelude.py".to_string()),
            "prelude should be omitted when unused: {paths:?}",
        );
    }

    #[test]
    fn package_emits_runtime_shim_for_checked_ffi() {
        let pkg = package_from_source(
            "from python import json\npub fn main(data: String) { json.loads(data) }",
            "hello",
        );
        let paths: Vec<String> = pkg.files.iter().map(|f| f.path.display().to_string()).collect();
        assert!(
            paths.contains(&"python/hello/_asatsuyu_runtime.py".to_string()),
            "runtime shim should be emitted for Checked FFI: {paths:?}",
        );
    }

    #[test]
    fn package_no_main_without_main_fn() {
        let pkg = package_from_source("fn add(x: Int, y: Int) -> Int { x }", "lib");
        let paths: Vec<String> = pkg.files.iter().map(|f| f.path.display().to_string()).collect();
        assert!(!paths.contains(&"python/lib/__main__.py".to_string()), "no __main__: {paths:?}");
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
            .find(|f| f.path.display().to_string() == "python/hello/__main__.py")
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

    // ── Issue 46: Mixed layout tests ────────────────────────────────────

    #[test]
    fn package_contains_py_typed() {
        let pkg = package_from_source("fn f() { 1 }", "mylib");
        let paths: Vec<String> = pkg.files.iter().map(|f| f.path.display().to_string()).collect();
        assert!(
            paths.contains(&"python/mylib/py.typed".to_string()),
            "py.typed should always be emitted: {paths:?}",
        );
    }

    #[test]
    fn package_maturin_layout_for_checked_ffi() {
        let pkg = package_from_source(
            "from python import json\npub fn main(data: String) { json.loads(data) }",
            "myapp",
        );
        let paths: Vec<String> = pkg.files.iter().map(|f| f.path.display().to_string()).collect();
        assert!(paths.contains(&"Cargo.toml".to_string()), "Cargo.toml: {paths:?}");
        assert!(paths.contains(&"src/lib.rs".to_string()), "src/lib.rs: {paths:?}");
        assert!(
            paths.contains(&"python/myapp/_asatsuyu_runtime.pyi".to_string()),
            ".pyi stub: {paths:?}",
        );

        let pyproject =
            pkg.files.iter().find(|f| f.path.display().to_string() == "pyproject.toml").unwrap();
        assert!(pyproject.content.contains("maturin"), "should use maturin: {}", pyproject.content);
        assert!(
            pyproject.content.contains("module-name = \"myapp._asatsuyu_runtime\""),
            "module-name: {}",
            pyproject.content,
        );
        assert!(
            pyproject.content.contains("python-source = \"python\""),
            "python-source: {}",
            pyproject.content,
        );
    }

    #[test]
    fn package_setuptools_for_non_ffi() {
        let pkg = package_from_source("pub fn main() { 42 }", "hello");
        let paths: Vec<String> = pkg.files.iter().map(|f| f.path.display().to_string()).collect();
        assert!(!paths.contains(&"Cargo.toml".to_string()), "no Cargo.toml: {paths:?}");
        assert!(!paths.contains(&"src/lib.rs".to_string()), "no src/lib.rs: {paths:?}");

        let pyproject =
            pkg.files.iter().find(|f| f.path.display().to_string() == "pyproject.toml").unwrap();
        assert!(pyproject.content.contains("setuptools"), "should use setuptools");
        assert!(
            pyproject.content.contains("where = [\"python\"]"),
            "setuptools package find: {}",
            pyproject.content,
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
            ffi_runtime: super::FfiRuntimeMode::Auto,
            requires_python: None,
            dependencies: Vec::new(),
        };
        let pkg = super::emit_package(&thir.module, &config, Some(source));
        pkg.files
            .into_iter()
            .find(|f| f.path.display().to_string() == "python/test/test.py")
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

    // ── Snapshot tests: prelude + try codegen (Issue 42) ───────────

    #[test]
    fn snap_prelude_content() {
        insta::assert_snapshot!(crate::prelude::PRELUDE_PY);
    }

    #[test]
    fn snap_try_let_codegen() {
        let source = "\
from python import pathlib
type Result(a, e) { Ok(a) Error(e) }
type PyException { PyExc(kind: String, exception_type: String, message: String, module: String, traceback_summary: String) }
pub fn f() -> Result(Bool, PyException) {
  let p = pathlib.Path(\".\")
  let r = try p.exists()
  Ok(r)
}";
        let py = python_from_source(source);
        insta::assert_snapshot!(py);
    }

    #[test]
    fn snap_try_return_codegen() {
        let source = "\
from python import pathlib
type Result(a, e) { Ok(a) Error(e) }
type PyException { PyExc(kind: String, exception_type: String, message: String, module: String, traceback_summary: String) }
pub fn f() -> Result(Bool, PyException) {
  let p = pathlib.Path(\".\")
  try p.exists()
}";
        let py = python_from_source(source);
        insta::assert_snapshot!(py);
    }

    // ── Issue 44: Checked FFI ──────────────────────────────────────

    #[test]
    fn emit_checked_ffi_json_loads() {
        let source = "\
from python import json
pub fn f(data: String) -> Int {
  let result = json.loads(data)
  42
}";
        let py = python_from_source(source);
        assert!(py.contains("try:"), "should contain try block: {py}");
        assert!(
            py.contains("_checked_runtime_json = _asatsuyu_runtime.import_module(\"json\")"),
            "should import runtime module: {py}"
        );
        assert!(
            py.contains("_checked_0 = _asatsuyu_runtime.call_function(_checked_runtime_json, \"loads\", data)"),
            "should route Checked call through runtime: {py}"
        );
        assert!(py.contains("except Exception as _e:"), "should have except: {py}");
        assert!(py.contains("isinstance"), "should have isinstance check: {py}");
        assert!(py.contains("AsatsuyuError"), "should raise AsatsuyuError: {py}");
    }

    #[test]
    fn emit_checked_ffi_json_dumps_return() {
        let source = "\
from python import json
pub fn f(data: String) { json.dumps(data) }";
        let py = python_from_source(source);
        assert!(py.contains("try:"), "should contain try block: {py}");
        assert!(
            py.contains("_asatsuyu_runtime.call_function(_checked_runtime_json, \"dumps\", data)"),
            "should route return-position Checked call through runtime: {py}"
        );
        assert!(py.contains("isinstance(_checked_0, str)"), "should check str return: {py}");
    }

    #[test]
    fn emit_verified_ffi_unchanged() {
        let source = "\
from python import pathlib
pub fn f() { pathlib.Path(\".\") }";
        let py = python_from_source(source);
        // Verified FFI should NOT have checked wrappers
        assert!(!py.contains("_checked_"), "should not wrap Verified calls: {py}");
        assert!(!py.contains("AsatsuyuError"), "no AsatsuyuError for Verified: {py}");
    }

    #[test]
    fn emit_checked_ffi_hasattr_guard() {
        let source = "\
from python import json
pub fn f(data: String) { json.loads(data) }";
        let py = python_from_source(source);
        assert!(
            py.contains("if not _asatsuyu_runtime.ffi_available():"),
            "should check runtime capability: {py}"
        );
        assert!(
            py.contains("hasattr(_checked_runtime_json, 'loads')"),
            "should check loads on runtime module: {py}"
        );
        assert!(
            py.contains("hasattr(_checked_runtime_json, 'dumps')"),
            "should check dumps on runtime module: {py}"
        );
    }

    #[test]
    fn emit_checked_triggers_prelude() {
        let source = "\
from python import json
pub fn f(data: String) { json.loads(data) }";
        let py = python_from_source(source);
        assert!(
            py.contains("from .asatsuyu_prelude import PyException, AsatsuyuError"),
            "should import AsatsuyuError: {py}"
        );
        assert!(
            py.contains("from . import _asatsuyu_runtime"),
            "should import runtime helper: {py}"
        );
    }

    #[test]
    fn snap_checked_ffi_codegen() {
        let source = "\
from python import json
pub fn f(data: String) -> Int {
  let result = json.loads(data)
  42
}";
        let py = python_from_source(source);
        insta::assert_snapshot!(py);
    }

    // ── Issue 45: requests as Checked FFI target ─────────────────────

    #[test]
    fn emit_requests_get_checked() {
        let source = "\
from python import requests
pub fn download(url: String) -> String {
  let response = requests.get(url)
  response.text
}";
        let py = python_from_source(source);
        assert!(py.contains("import requests"), "should import requests: {py}");
        assert!(
            py.contains(
                "_checked_runtime_requests = _asatsuyu_runtime.import_module(\"requests\")"
            ),
            "should bind runtime module: {py}"
        );
        assert!(
            py.contains("_asatsuyu_runtime.call_function(_checked_runtime_requests, \"get\", url)"),
            "should call get via runtime: {py}"
        );
        assert!(py.contains("response.text"), "should access text directly: {py}");
    }

    #[test]
    fn emit_requests_response_json_checked() {
        let source = "\
from python import requests
pub fn get_data(url: String) -> Int {
  let response = requests.get(url)
  let data = response.json()
  42
}";
        let py = python_from_source(source);
        assert!(
            py.contains("_asatsuyu_runtime.call_method(response, \"json\")"),
            "should call json via call_method: {py}"
        );
        assert!(py.contains("isinstance(_checked_1"), "should validate json return: {py}");
    }

    #[test]
    fn emit_requests_text_not_wrapped() {
        // response.text is a clean str property — should NOT be wrapped
        let source = "\
from python import requests
pub fn get_text(url: String) -> String {
  let response = requests.get(url)
  response.text
}";
        let py = python_from_source(source);
        assert!(py.contains("return response.text"), "text should be direct access: {py}");
        // call_method should NOT appear for property access
        assert!(
            !py.contains("call_method(response, \"text\")"),
            "text should not use call_method: {py}"
        );
    }

    #[test]
    fn snap_requests_get_codegen() {
        let source = "\
from python import requests
pub fn download(url: String) -> String {
  let response = requests.get(url)
  response.text
}";
        let py = python_from_source(source);
        insta::assert_snapshot!(py);
    }

    #[test]
    fn snap_requests_json_codegen() {
        let source = "\
from python import requests
pub fn get_data(url: String) -> Int {
  let response = requests.get(url)
  let data = response.json()
  42
}";
        let py = python_from_source(source);
        insta::assert_snapshot!(py);
    }

    // ── Issue 95: mutable locals and assignment emission ───────────

    #[test]
    fn snap_mutable_locals_codegen() {
        let source = "\
pub fn accumulate() -> Int {
  let mut sum = 0
  sum = sum + 10
  sum = sum + 20
  sum
}";
        let py = python_from_source(source);
        insta::assert_snapshot!(py);
    }

    #[test]
    fn mutable_let_emits_same_as_immutable_let() {
        let mutable_source = "pub fn f() -> Int { let mut x = 42\n x }";
        let immutable_source = "pub fn f() -> Int { let x = 42\n x }";
        let mutable_py = python_from_source(mutable_source);
        let immutable_py = python_from_source(immutable_source);
        assert_eq!(mutable_py, immutable_py, "Python output should be identical for mut/non-mut");
    }

    #[test]
    fn assign_emits_python_assignment() {
        let source = "pub fn f() -> Int { let mut x = 0\n x = 1\n x }";
        let py = python_from_source(source);
        assert!(py.contains("x = 0"), "should emit initial binding: {py}");
        assert!(py.contains("x = 1"), "should emit reassignment: {py}");
        assert!(py.contains("return x"), "should return the variable: {py}");
    }

    #[test]
    fn trailing_assign_emits_statement_then_return_none() {
        let source = "pub fn f() { let mut x = 0\n x = 1 }";
        let py = python_from_source(source);
        assert!(py.contains("x = 1"), "should emit trailing assignment as a statement: {py}");
        assert!(
            py.contains("return None"),
            "statement-typed trailing assign should return None: {py}"
        );
        assert!(
            !py.contains("return x = 1"),
            "must not emit invalid Python assignment in return position: {py}"
        );
    }

    #[test]
    fn trailing_let_emits_statement_then_return_none() {
        let source = "pub fn f() { let x = 1 }";
        let py = python_from_source(source);
        assert!(py.contains("x = 1"), "should emit trailing let as a statement: {py}");
        assert!(
            py.contains("return None"),
            "statement-typed trailing let should return None: {py}"
        );
        assert!(
            !py.contains("return x = 1"),
            "must not emit invalid Python assignment in return position: {py}"
        );
    }

    #[test]
    fn assign_checked_ffi_uses_statement_routing() {
        let source = "\
from python import requests
pub fn refresh(url: String) -> String {
  let mut response = requests.get(url)
  response = requests.get(url)
  response.text
}";
        let py = python_from_source(source);
        assert!(
            py.contains("_asatsuyu_runtime.call_function(_checked_runtime_requests, \"get\", url)"),
            "should route reassignment through checked runtime calls: {py}"
        );
        assert!(
            py.contains("response = _checked_"),
            "should assign validated checked value back to response: {py}"
        );
        assert!(py.contains("try:"), "checked reassignment should stay in try/except form: {py}");
    }

    #[test]
    fn assign_list_fold_uses_statement_routing() {
        let source = "\
pub fn add(acc: Int, x: Int) -> Int { acc + x }
pub fn accumulate() -> Int {
  let mut total = 0
  total = list.fold([1, 2, 3], total, add)
  total
}";
        let py = python_from_source(source);
        assert!(py.contains("total = 0"), "should seed the accumulator variable: {py}");
        assert!(
            py.contains("for _fold_item_"),
            "list.fold reassignment should lower to a for loop: {py}"
        );
        assert!(
            py.contains("total = add(total, _fold_item_"),
            "loop body should reassign the target variable: {py}"
        );
    }

    // ── Issue 50: golden emission tests (auto-discovered) ─────────

    #[test]
    fn golden_emission() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test-cases");
        insta::glob!(base, "*/input.asty", |path| {
            let source = std::fs::read_to_string(path).unwrap();
            let py = python_from_source(&source);
            insta::assert_snapshot!(py);
        });
    }

    // ── Issue 58: Standards-compliant pyproject.toml tests ─────────

    fn package_with_config(source: &str, config: &super::PackageConfig) -> super::GeneratedPackage {
        let cst = parse(FID, source);
        let ast = asatsuyu_ast::lower(&cst, FID);
        let hir = asatsuyu_hir::lower_to_hir(&ast.module);
        let thir = asatsuyu_ty::check_types(&hir.module);
        super::emit_package(&thir.module, config, None)
    }

    fn get_pyproject(pkg: &super::GeneratedPackage) -> &str {
        &pkg.files
            .iter()
            .find(|f| f.path.display().to_string() == "pyproject.toml")
            .expect("pyproject.toml should exist")
            .content
    }

    #[test]
    fn pyproject_includes_dependencies() {
        let config = super::PackageConfig {
            name: "myapp".into(),
            version: "0.2.0".into(),
            source_map: false,
            ffi_runtime: super::FfiRuntimeMode::Auto,
            requires_python: None,
            dependencies: vec!["requests>=2.31".into(), "flask>=3.0".into()],
        };
        let pkg = package_with_config("pub fn main() { 42 }", &config);
        let pyproject = get_pyproject(&pkg);
        assert!(pyproject.contains("dependencies = ["), "deps section: {pyproject}");
        assert!(pyproject.contains("\"requests>=2.31\""), "requests dep: {pyproject}",);
        assert!(pyproject.contains("\"flask>=3.0\""), "flask dep: {pyproject}");
    }

    #[test]
    fn pyproject_no_dependencies_when_empty() {
        let pkg = package_from_source("pub fn main() { 42 }", "myapp");
        let pyproject = get_pyproject(&pkg);
        assert!(!pyproject.contains("dependencies"), "no dependencies when empty: {pyproject}",);
    }

    #[test]
    fn pyproject_custom_requires_python() {
        let config = super::PackageConfig {
            name: "myapp".into(),
            version: "0.1.0".into(),
            source_map: false,
            ffi_runtime: super::FfiRuntimeMode::Auto,
            requires_python: Some(">=3.13".into()),
            dependencies: Vec::new(),
        };
        let pkg = package_with_config("pub fn main() { 42 }", &config);
        let pyproject = get_pyproject(&pkg);
        assert!(
            pyproject.contains("requires-python = \">=3.13\""),
            "custom requires-python: {pyproject}",
        );
    }

    #[test]
    fn pyproject_default_requires_python() {
        let pkg = package_from_source("pub fn main() { 42 }", "myapp");
        let pyproject = get_pyproject(&pkg);
        assert!(
            pyproject.contains("requires-python = \">=3.12\""),
            "default requires-python: {pyproject}",
        );
    }

    #[test]
    fn pyproject_scripts_with_main() {
        let pkg = package_from_source("pub fn main() { 42 }", "myapp");
        let pyproject = get_pyproject(&pkg);
        assert!(pyproject.contains("[project.scripts]"), "scripts section: {pyproject}",);
        assert!(pyproject.contains("myapp = \"myapp.myapp:main\""), "entry point: {pyproject}",);
    }

    #[test]
    fn pyproject_no_scripts_without_main() {
        let pkg = package_from_source("fn add(x: Int, y: Int) -> Int { x }", "lib");
        let pyproject = get_pyproject(&pkg);
        assert!(!pyproject.contains("[project.scripts]"), "no scripts without main: {pyproject}",);
    }

    #[test]
    fn pyproject_tool_asatsuyu_metadata() {
        let pkg = package_from_source("pub fn main() { 42 }", "myapp");
        let pyproject = get_pyproject(&pkg);
        assert!(pyproject.contains("[tool.asatsuyu]"), "tool.asatsuyu section: {pyproject}",);
        assert!(pyproject.contains("compiler-version = \""), "compiler version: {pyproject}",);
    }

    #[test]
    fn package_name_normalization() {
        let config = super::PackageConfig {
            name: "my-app".into(),
            version: "0.1.0".into(),
            source_map: false,
            ffi_runtime: super::FfiRuntimeMode::Auto,
            requires_python: None,
            dependencies: Vec::new(),
        };
        let pkg = package_with_config("pub fn main() { 42 }", &config);
        let paths: Vec<String> = pkg.files.iter().map(|f| f.path.display().to_string()).collect();
        // Directory should use normalized name (underscores).
        assert!(
            paths.contains(&"python/my_app/__init__.py".to_string()),
            "normalized dir: {paths:?}",
        );
        assert!(
            paths.contains(&"python/my_app/my_app.py".to_string()),
            "normalized module: {paths:?}",
        );
        // pyproject.toml should use original name.
        let pyproject = get_pyproject(&pkg);
        assert!(pyproject.contains("name = \"my-app\""), "original name in pyproject: {pyproject}",);
    }

    // ── PEP 695 generic functions ─────────────────────────────────

    #[test]
    fn emit_generic_fn_pep695() {
        let py = python_from_source(
            "type Option(a) { Some(a) None }\n\
             pub fn unwrap_or(opt: Option, default: String) -> String {\n\
               match opt { Some(x) -> x  None -> default }\n\
             }",
        );
        assert!(py.contains("def unwrap_or[T]"), "should emit PEP 695 type param: {py}");
        assert!(py.contains("Option[T]"), "should use T not Any: {py}");
        assert!(!py.contains("Any"), "must not contain Any: {py}");
    }

    #[test]
    fn emit_multi_generic_fn_pep695() {
        let py = python_from_source(
            "type Result(a, e) { Ok(a) Error(e) }\n\
             pub fn is_ok(r: Result) -> Bool {\n\
               match r { Ok(_) -> True  Error(_) -> False }\n\
             }",
        );
        assert!(py.contains("def is_ok[T, U]"), "should emit multiple type params: {py}");
        assert!(py.contains("Result[T, U]"), "should use T, U not Any: {py}");
        assert!(!py.contains("Any"), "must not contain Any: {py}");
    }

    // ── list module → list comprehension ──────────────────────────

    #[test]
    fn emit_list_map_as_comprehension() {
        let py =
            python_from_source("pub fn f() -> List(Int) { list.map([1, 2, 3], fn(x) { x * 2 }) }");
        assert!(
            py.contains("[x * 2 for x in") || py.contains("[(x * 2) for x in"),
            "list.map should emit list comprehension: {py}",
        );
    }

    #[test]
    fn emit_list_filter_as_comprehension() {
        let py = python_from_source(
            "pub fn f() -> List(Int) { list.filter([1, 2, 3], fn(x) { x > 0 }) }",
        );
        assert!(
            py.contains("[x for x in") && py.contains("if (x > 0)"),
            "list.filter should emit list comprehension: {py}",
        );
    }

    #[test]
    fn emit_list_length() {
        let py = python_from_source("pub fn f() -> Int { list.length([1, 2, 3]) }");
        assert!(py.contains("len("), "list.length should emit len(): {py}");
    }

    #[test]
    fn emit_list_fold_as_loop_in_return_position() {
        let py = python_from_source(
            "pub fn add(acc: Int, x: Int) -> Int { acc + x }\n\
             pub fn f() -> Int { list.fold([1, 2, 3], 0, add) }",
        );
        assert!(py.contains("for _fold_item_"), "list.fold should emit a for loop: {py}");
        assert!(py.contains("return _fold_acc_"), "list.fold should return the accumulator: {py}");
    }

    #[test]
    fn emit_list_head_and_rest() {
        let py = python_from_source(
            "pub fn heady(items: List(Int)) -> Option(Int) { list.head(items) }\n\
             pub fn resty(items: List(Int)) -> Option(List(Int)) { list.rest(items) }",
        );
        assert!(
            py.contains("[0] if items else None"),
            "list.head should emit conditional indexing: {py}"
        );
        assert!(
            py.contains("[1:] if items else None"),
            "list.rest should emit conditional slicing: {py}"
        );
    }
}
