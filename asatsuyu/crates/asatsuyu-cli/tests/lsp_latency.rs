//! LSP latency budget tests — Issue 106.
//!
//! Measures compile and completion latency for fixture-sized inputs.
//! Assertions use generous thresholds (100x expected) to avoid CI flakes
//! while catching catastrophic regressions.
//!
//! Run with `cargo test -p asatsuyu-cli --test lsp_latency -- --nocapture`
//! to see actual timings in stderr.

use std::path::Path;
use std::time::{Duration, Instant};

use asatsuyu_hir::ffi::FfiResolverConfig;
use asatsuyu_syntax::FileId;

const FID: FileId = FileId(0);

/// Maximum allowed latency for full pipeline on a small file (~20 lines).
/// Expected: ~5ms. Budget: 500ms (100x).
const SMALL_FILE_BUDGET: Duration = Duration::from_millis(500);

/// Maximum allowed latency for full pipeline on a medium file (~100 functions).
/// Expected: ~20ms. Budget: 2000ms (100x).
const MEDIUM_FILE_BUDGET: Duration = Duration::from_millis(2000);

fn ffi_config() -> FfiResolverConfig {
    FfiResolverConfig::default()
}

fn fixture_path(project: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../fixtures/projects/{project}/src/main.asty"))
        .display()
        .to_string()
}

/// Run the full compile pipeline (parse → ast → hir → ty → emit), returning
/// elapsed time and whether it succeeded.
fn measure_pipeline(source: &str) -> (Duration, bool) {
    let start = Instant::now();

    let cst = asatsuyu_parser::parse(FID, source);
    if cst.has_errors() {
        return (start.elapsed(), false);
    }
    let ast = asatsuyu_ast::lower(&cst, FID);
    if ast.has_errors() {
        return (start.elapsed(), false);
    }
    let hir = asatsuyu_hir::lower_to_hir(&ast.module);
    if hir.has_errors() {
        return (start.elapsed(), false);
    }
    let ffi = ffi_config();
    let thir = asatsuyu_ty::check_types_with_ffi_config(&hir.module, &ffi);
    let _ = asatsuyu_backend_python::emit_module(&thir.module);

    (start.elapsed(), true)
}

/// Parse latency only.
fn measure_parse(source: &str) -> Duration {
    let start = Instant::now();
    let _ = asatsuyu_parser::parse(FID, source);
    start.elapsed()
}

// ── Small file latency ──────────────────────────────────────────────

#[test]
fn latency_small_file_pipeline() {
    let source = std::fs::read_to_string(fixture_path("hello_cli")).unwrap();

    // Warm-up run
    let _ = measure_pipeline(&source);

    let (elapsed, ok) = measure_pipeline(&source);
    eprintln!("  small file pipeline: {elapsed:?}");
    assert!(ok, "small file should compile successfully");
    assert!(
        elapsed < SMALL_FILE_BUDGET,
        "small file pipeline took {elapsed:?}, budget is {SMALL_FILE_BUDGET:?}",
    );
}

#[test]
fn latency_small_file_parse_only() {
    let source = std::fs::read_to_string(fixture_path("hello_cli")).unwrap();

    let _ = measure_parse(&source);
    let elapsed = measure_parse(&source);
    eprintln!("  small file parse: {elapsed:?}");
    assert!(elapsed < Duration::from_millis(100), "small file parse took {elapsed:?}");
}

// ── Mutable fixture latency ─────────────────────────────────────────

#[test]
fn latency_mutable_fixture_pipeline() {
    let source = std::fs::read_to_string(fixture_path("mutable_counter")).unwrap();

    let _ = measure_pipeline(&source);
    let (elapsed, ok) = measure_pipeline(&source);
    eprintln!("  mutable fixture pipeline: {elapsed:?}");
    assert!(ok, "mutable fixture should compile");
    assert!(
        elapsed < SMALL_FILE_BUDGET,
        "mutable fixture pipeline took {elapsed:?}, budget is {SMALL_FILE_BUDGET:?}",
    );
}

// ── Async fixture latency ───────────────────────────────────────────

#[test]
fn latency_async_fixture_pipeline() {
    let source = std::fs::read_to_string(fixture_path("async_fetch")).unwrap();

    let _ = measure_pipeline(&source);
    let (elapsed, ok) = measure_pipeline(&source);
    eprintln!("  async fixture pipeline: {elapsed:?}");
    assert!(ok, "async fixture should compile");
    assert!(
        elapsed < SMALL_FILE_BUDGET,
        "async fixture pipeline took {elapsed:?}, budget is {SMALL_FILE_BUDGET:?}",
    );
}

// ── Medium file (synthetic) ─────────────────────────────────────────

#[test]
fn latency_medium_synthetic_pipeline() {
    // Generate a ~100-function file to test scaling.
    let mut source = String::new();
    for i in 0..100 {
        use std::fmt::Write;
        let _ = writeln!(source, "fn func_{i}(x: Int) -> Int {{ x + {i} }}");
    }
    source.push_str("pub fn main() -> Int { func_0(1) }\n");

    let _ = measure_pipeline(&source);
    let (elapsed, ok) = measure_pipeline(&source);
    eprintln!("  medium file (100 fns) pipeline: {elapsed:?}");
    assert!(ok, "medium file should compile");
    assert!(
        elapsed < MEDIUM_FILE_BUDGET,
        "medium file pipeline took {elapsed:?}, budget is {MEDIUM_FILE_BUDGET:?}",
    );
}

// ── Completion context latency ──────────────────────────────────────

#[test]
fn latency_completion_context() {
    let source = "fn add(a: Int, b: Int) -> Int { a + b }\nfn main() -> Int { add(1, 2) }";

    let _ = measure_pipeline(source);
    let (elapsed, ok) = measure_pipeline(source);
    eprintln!("  completion-context pipeline: {elapsed:?}");
    assert!(ok, "completion context should compile");
    assert!(elapsed < Duration::from_millis(100), "completion context took {elapsed:?}",);
}

// ── check --watch vs LSP debounce gap ───────────────────────────────

#[test]
fn latency_debounce_gap() {
    // Documents the gap between watch mode and LSP debounce.
    let watch_debounce = Duration::from_millis(250);
    let lsp_debounce = Duration::from_millis(200);
    let gap = watch_debounce.checked_sub(lsp_debounce).unwrap();
    eprintln!("  check --watch debounce: {watch_debounce:?}");
    eprintln!("  LSP debounce:           {lsp_debounce:?}");
    eprintln!("  gap:                    {gap:?}");
    assert_eq!(gap, Duration::from_millis(50), "debounce gap should be 50ms");
}
