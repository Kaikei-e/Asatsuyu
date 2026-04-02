//! Crash safety tests — Issue 64.
//!
//! Verify that no input, however malformed, causes a panic in any pipeline stage.
//! Every input must either succeed or produce diagnostics — never crash.
//!
//! The corpus lives in `tests/corpus/`. To add a regression case, drop an `.asty`
//! file there with the prefix `crash-{stage}-{description}.asty`.

use std::panic::{self, AssertUnwindSafe};

use asatsuyu_hir::ffi::FfiResolverConfig;
use asatsuyu_syntax::Diagnostic;
use asatsuyu_syntax::FileId;

const FID: FileId = FileId(0);
const MALFORMED_CORPUS_CASES: &[&str] = &[
    "crash-hir-circular-name.asty",
    "crash-multi-errors-all-stages.asty",
    "crash-parse-fn-only.asty",
    "crash-parse-import-incomplete.asty",
    "crash-parse-unbalanced-parens.asty",
    "crash-ty-recursive-type.asty",
    "crash-ty-self-referencing.asty",
    "crash-ty-unknown-builtin-use.asty",
];

fn ffi_config() -> FfiResolverConfig {
    FfiResolverConfig::default()
}

/// Run the full pipeline with early-exit on errors (matches real CLI behavior).
fn compile_pipeline(source: &str) {
    let cst = asatsuyu_parser::parse(FID, source);
    if cst.has_errors() {
        return;
    }

    let ast = asatsuyu_ast::lower(&cst, FID);
    if ast.has_errors() {
        return;
    }

    let hir = asatsuyu_hir::lower_to_hir(&ast.module);
    if hir.has_errors() {
        return;
    }

    let ffi = ffi_config();
    let thir = asatsuyu_ty::check_types_with_ffi_config(&hir.module, &ffi);
    if thir.has_errors() {
        return;
    }

    let _py = asatsuyu_backend_python::emit_module(&thir.module);
}

/// Collect diagnostics across the full pipeline, stopping at the same
/// boundaries as the real CLI.
fn collect_diagnostics(source: &str) -> Vec<Diagnostic> {
    let mut all = Vec::new();

    let cst = asatsuyu_parser::parse(FID, source);
    all.extend(cst.diagnostics().iter().cloned());
    if cst.has_errors() {
        return all;
    }

    let ast = asatsuyu_ast::lower(&cst, FID);
    all.extend(ast.diagnostics.iter().cloned());
    if ast.has_errors() {
        return all;
    }

    let hir = asatsuyu_hir::lower_to_hir(&ast.module);
    all.extend(hir.diagnostics.iter().cloned());
    if hir.has_errors() {
        return all;
    }

    let ffi = ffi_config();
    let thir = asatsuyu_ty::check_types_with_ffi_config(&hir.module, &ffi);
    all.extend(thir.diagnostics.iter().cloned());
    all
}

/// Run the full pipeline forcing all stages regardless of errors.
/// This exercises later stages with potentially invalid input from earlier stages.
fn compile_pipeline_force_all(source: &str) {
    let cst = asatsuyu_parser::parse(FID, source);
    let ast = asatsuyu_ast::lower(&cst, FID);
    let hir = asatsuyu_hir::lower_to_hir(&ast.module);
    let ffi = ffi_config();
    let thir = asatsuyu_ty::check_types_with_ffi_config(&hir.module, &ffi);
    let _py = asatsuyu_backend_python::emit_module(&thir.module);
}

/// Run the pipeline inside `catch_unwind` and return `Err` if it panicked.
fn assert_no_panic(source: &str, force: bool) -> Result<(), String> {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        if force {
            compile_pipeline_force_all(source);
        } else {
            compile_pipeline(source);
        }
    }));
    result.map_err(|payload| {
        let msg = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("<unknown panic>");
        msg.to_owned()
    })
}

// ── Corpus-driven tests ──────────────────────────────────────────────

#[test]
fn crash_safety_corpus() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
    insta::glob!(base, "**/*.asty", |path| {
        let source = std::fs::read_to_string(path).expect("read corpus file");
        let result = assert_no_panic(&source, false);
        assert!(result.is_ok(), "PANIC on {}: {}", path.display(), result.unwrap_err());
    });
}

#[test]
fn crash_safety_force_all_stages() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
    insta::glob!(base, "**/*.asty", |path| {
        let source = std::fs::read_to_string(path).expect("read corpus file");
        let result = assert_no_panic(&source, true);
        assert!(result.is_ok(), "PANIC (forced) on {}: {}", path.display(), result.unwrap_err());
    });
}

#[test]
fn malformed_crash_corpus_produces_diagnostics() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
    for name in MALFORMED_CORPUS_CASES {
        let path = base.join(name);
        let name = path.file_name().and_then(|name| name.to_str()).expect("utf-8 corpus filename");
        assert!(path.is_file(), "missing malformed corpus file: {name}");
        let source = std::fs::read_to_string(&path).expect("read corpus file");
        let diagnostics = collect_diagnostics(&source);
        assert!(!diagnostics.is_empty(), "expected diagnostics for malformed corpus input: {name}");
    }
}

#[test]
fn crash_corpus_inventory_is_stable() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
    let mut files = std::fs::read_dir(&base)
        .expect("read corpus dir")
        .map(|entry| entry.expect("dir entry").path())
        .collect::<Vec<_>>();
    files.sort();

    assert_eq!(files.len(), 32, "unexpected corpus size");
    for path in files {
        let name = path.file_name().and_then(|name| name.to_str()).expect("utf-8 corpus filename");
        assert!(
            name.starts_with("crash-")
                && std::path::Path::new(name)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("asty")),
            "corpus file must follow crash-{{stage}}-{{description}}.asty: {name}"
        );
        assert!(
            name.matches('-').count() >= 2,
            "corpus file must include stage and description: {name}"
        );
    }
}

// ── Inline edge-case tests ───────────────────────────────────────────

#[test]
fn crash_empty_input() {
    assert!(assert_no_panic("", false).is_ok());
    assert!(assert_no_panic("", true).is_ok());
}

#[test]
fn crash_null_byte() {
    assert!(assert_no_panic("\0", false).is_ok());
    assert!(assert_no_panic("\0", true).is_ok());
}

#[test]
fn crash_only_whitespace() {
    assert!(assert_no_panic("   \n\t\n   ", false).is_ok());
    assert!(assert_no_panic("   \n\t\n   ", true).is_ok());
}

#[test]
fn crash_deep_nesting() {
    let depth = 500;
    let input = "fn main() { ".to_owned() + &"(".repeat(depth) + "1" + &")".repeat(depth) + " }";
    assert!(assert_no_panic(&input, false).is_ok());
    assert!(assert_no_panic(&input, true).is_ok());
}

#[test]
fn crash_huge_identifier() {
    let name = "a".repeat(100_000);
    let input = format!("fn {name}() {{ 42 }}");
    assert!(assert_no_panic(&input, false).is_ok());
    assert!(assert_no_panic(&input, true).is_ok());
}

#[test]
fn crash_many_newlines() {
    let input = "\n".repeat(10_000);
    assert!(assert_no_panic(&input, false).is_ok());
    assert!(assert_no_panic(&input, true).is_ok());
}

#[test]
fn crash_binary_garbage() {
    let input: String = (0u8..=255).map(|b| b as char).collect();
    assert!(assert_no_panic(&input, false).is_ok());
    assert!(assert_no_panic(&input, true).is_ok());
}

#[test]
fn crash_repeated_pipes() {
    let input = "fn f() { 1 |> |> |> |> |> |> |> |> |> |> 2 }";
    assert!(assert_no_panic(input, false).is_ok());
    assert!(assert_no_panic(input, true).is_ok());
}

#[test]
fn crash_string_concat_chain() {
    let input = "fn f() { \"a\" <> <> <> <> <> \"b\" }";
    assert!(assert_no_panic(input, false).is_ok());
    assert!(assert_no_panic(input, true).is_ok());
}

#[test]
fn crash_pub_repeated() {
    let input = "pub pub pub pub fn main() { 42 }";
    assert!(assert_no_panic(input, false).is_ok());
    assert!(assert_no_panic(input, true).is_ok());
}
