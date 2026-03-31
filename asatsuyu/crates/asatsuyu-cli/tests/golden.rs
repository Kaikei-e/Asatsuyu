//! Golden test suite — Issue 61.
//!
//! Each `tests/cases/<name>/input.asty` is compiled through the full pipeline
//! and snapshots are generated for every stage that succeeds:
//!
//! - **ast**: `{:#?}` dump of the untyped AST module
//! - **hir**: `{:#?}` dump of the HIR module (after name resolution & desugaring)
//! - **thir**: `{:#?}` dump of the typed HIR module
//! - **py**: generated Python 3.12+ source code
//! - **diag**: human-readable diagnostics (always present)
//!
//! Update snapshots with `cargo insta review`.

use std::fmt::Write;

use asatsuyu_hir::ffi::FfiResolverConfig;
use asatsuyu_syntax::{Diagnostic, FileId, LabelStyle, Severity};

const FID: FileId = FileId(0);

fn ffi_config() -> FfiResolverConfig {
    FfiResolverConfig { stdlib_only: false, stub_paths: vec![] }
}

// ── Pipeline output ────────────────────────────────────────────────

struct GoldenOutput {
    ast: Option<String>,
    hir: Option<String>,
    thir: Option<String>,
    py: Option<String>,
    diag: String,
}

fn compile_golden(source: &str) -> GoldenOutput {
    let mut all_diags: Vec<Diagnostic> = Vec::new();

    // 1. Parse
    let cst = asatsuyu_parser::parse(FID, source);
    all_diags.extend(cst.diagnostics().iter().cloned());
    if cst.has_errors() {
        return GoldenOutput {
            ast: None,
            hir: None,
            thir: None,
            py: None,
            diag: format_diagnostics(&all_diags),
        };
    }

    // 2. AST
    let ast = asatsuyu_ast::lower(&cst, FID);
    let ast_dump = format!("{:#?}", ast.module);
    all_diags.extend(ast.diagnostics.iter().cloned());
    if ast.has_errors() {
        return GoldenOutput {
            ast: Some(ast_dump),
            hir: None,
            thir: None,
            py: None,
            diag: format_diagnostics(&all_diags),
        };
    }

    // 3. HIR
    let hir = asatsuyu_hir::lower_to_hir(&ast.module);
    let hir_dump = format!("{:#?}", hir.module);
    all_diags.extend(hir.diagnostics.iter().cloned());
    if hir.has_errors() {
        return GoldenOutput {
            ast: Some(ast_dump),
            hir: Some(hir_dump),
            thir: None,
            py: None,
            diag: format_diagnostics(&all_diags),
        };
    }

    // 4. Type check
    let ffi = ffi_config();
    let thir = asatsuyu_ty::check_types_with_ffi_config(&hir.module, &ffi);
    let thir_dump = format!("{:#?}", thir.module);
    all_diags.extend(thir.diagnostics.iter().cloned());
    if thir.has_errors() {
        return GoldenOutput {
            ast: Some(ast_dump),
            hir: Some(hir_dump),
            thir: Some(thir_dump),
            py: None,
            diag: format_diagnostics(&all_diags),
        };
    }

    // 5. Python emission
    let py = asatsuyu_backend_python::emit_module(&thir.module);

    let diag = if all_diags.is_empty() {
        "(no diagnostics)\n".to_owned()
    } else {
        format_diagnostics(&all_diags)
    };

    GoldenOutput {
        ast: Some(ast_dump),
        hir: Some(hir_dump),
        thir: Some(thir_dump),
        py: Some(py),
        diag,
    }
}

// ── Diagnostic formatting (same contract as diagnostic_snapshots.rs) ──

fn format_diagnostics(diags: &[Diagnostic]) -> String {
    if diags.is_empty() {
        return String::from("(no diagnostics)\n");
    }

    let mut out = String::new();
    for (i, d) in diags.iter().enumerate() {
        if i > 0 {
            out.push_str("---\n");
        }
        if let Some(code) = d.code {
            let _ = write!(out, "[{code}] ");
        }
        let severity = match d.severity {
            Severity::Error => "Error",
            Severity::Warning => "Warning",
            Severity::Note => "Note",
        };
        let _ = writeln!(out, "{severity}: {}", d.message);
        let _ = writeln!(out, "  span: {}..{}", d.span.start, d.span.end);
        for label in &d.labels {
            let style = match label.style {
                LabelStyle::Primary => "primary",
                LabelStyle::Secondary => "secondary",
            };
            let _ = writeln!(
                out,
                "  {style} [{}..{}]: {}",
                label.span.start, label.span.end, label.message
            );
        }
        for hint in &d.hints {
            let _ = writeln!(out, "  hint: {hint}");
        }
        for note in &d.notes {
            let _ = writeln!(out, "  note: {note}");
        }
    }
    out
}

// ── Golden test entry point ────────────────────────────────────────

#[test]
fn golden() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cases");
    insta::glob!(base, "*/input.asty", |path| {
        let source = std::fs::read_to_string(path).unwrap();
        let out = compile_golden(&source);

        if let Some(ref ast) = out.ast {
            insta::assert_snapshot!("ast", ast);
        }
        if let Some(ref hir) = out.hir {
            insta::assert_snapshot!("hir", hir);
        }
        if let Some(ref thir) = out.thir {
            insta::assert_snapshot!("thir", thir);
        }
        if let Some(ref py) = out.py {
            insta::assert_snapshot!("py", py);
        }
        insta::assert_snapshot!("diag", out.diag);
    });
}
