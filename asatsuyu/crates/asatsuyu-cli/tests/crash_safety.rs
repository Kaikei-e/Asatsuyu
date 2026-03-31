//! Crash safety tests — Issue 64.
//!
//! Verify that no input, however malformed, causes a panic in any pipeline stage.
//! Every input must either succeed or produce diagnostics — never crash.
//!
//! The corpus lives in `tests/corpus/`. To add a regression case, drop an `.asty`
//! file there with the prefix `crash-{stage}-{description}.asty`.

use std::panic::{self, AssertUnwindSafe};

use asatsuyu_hir::ffi::FfiResolverConfig;
use asatsuyu_syntax::FileId;

const FID: FileId = FileId(0);

fn ffi_config() -> FfiResolverConfig {
    FfiResolverConfig { stdlib_only: false, stub_paths: vec![] }
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
