//! Snapshot tests for the compiler diagnostic contract.
//!
//! Each `.asty` fixture in `tests/fixtures/diag/` is compiled through the full
//! pipeline and the resulting diagnostics are serialized into a stable text
//! format. Any change in diagnostic code, severity, span, labels, hints, or
//! notes will cause a snapshot mismatch that must be reviewed via
//! `cargo insta review`.

use std::fmt::Write;
use std::path::Path;

use asatsuyu_hir::ffi::FfiResolverConfig;
use asatsuyu_syntax::{Diagnostic, FileId, LabelStyle, Severity};

/// Compile a source string through the full pipeline, collecting all diagnostics.
fn collect_diagnostics(source: &str, ffi_config: &FfiResolverConfig) -> Vec<Diagnostic> {
    let mut all = Vec::new();

    let cst = asatsuyu_parser::parse(FileId(0), source);
    all.extend(cst.diagnostics().iter().cloned());
    if cst.has_errors() {
        return all;
    }

    let ast = asatsuyu_ast::lower(&cst, FileId(0));
    all.extend(ast.diagnostics.iter().cloned());
    if ast.has_errors() {
        return all;
    }

    let hir = asatsuyu_hir::lower_to_hir(&ast.module);
    all.extend(hir.diagnostics.iter().cloned());
    if hir.has_errors() {
        return all;
    }

    let thir = asatsuyu_ty::check_types_with_ffi_config(&hir.module, ffi_config);
    all.extend(thir.diagnostics.iter().cloned());
    all
}

/// Serialize diagnostics into a stable, human-readable format for snapshotting.
fn format_diagnostics(diags: &[Diagnostic]) -> String {
    if diags.is_empty() {
        return String::from("(no diagnostics)\n");
    }

    let mut out = String::new();
    for (i, d) in diags.iter().enumerate() {
        if i > 0 {
            out.push_str("---\n");
        }
        // Code + severity + message
        if let Some(code) = d.code {
            let _ = write!(out, "[{code}] ");
        }
        let severity = match d.severity {
            Severity::Error => "Error",
            Severity::Warning => "Warning",
            Severity::Note => "Note",
        };
        let _ = writeln!(out, "{severity}: {}", d.message);
        // Span
        let _ = writeln!(out, "  span: {}..{}", d.span.start, d.span.end);
        // Labels
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
        // Hints
        for hint in &d.hints {
            let _ = writeln!(out, "  hint: {hint}");
        }
        // Notes
        for note in &d.notes {
            let _ = writeln!(out, "  note: {note}");
        }
    }
    out
}

fn snapshot_fixture(name: &str) {
    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/diag");
    let path = fixture_dir.join(format!("{name}.asty"));
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", path.display()));
    let ffi_config = FfiResolverConfig { stdlib_only: true, stub_paths: vec![] };
    let diags = collect_diagnostics(&source, &ffi_config);
    let text = format_diagnostics(&diags);
    insta::assert_snapshot!(name, text);
}

// ── Lexer ─────────────────────────────────────────────────────────

#[test]
fn lex_unexpected_char() {
    snapshot_fixture("lex-unexpected-char");
}

#[test]
fn lex_multiple_errors() {
    snapshot_fixture("lex-multiple-errors");
}

#[test]
fn lex_error_in_context() {
    snapshot_fixture("lex-error-in-context");
}

// ── Parser ────────────────────────────────────────────────────────

#[test]
fn parse_expected_item() {
    snapshot_fixture("parse-expected-item");
}

#[test]
fn parse_missing_fn_name() {
    snapshot_fixture("parse-missing-fn-name");
}

#[test]
fn parse_missing_param_list() {
    snapshot_fixture("parse-missing-param-list");
}

#[test]
fn parse_missing_fn_body() {
    snapshot_fixture("parse-missing-fn-body");
}

#[test]
fn parse_missing_rparen() {
    snapshot_fixture("parse-missing-rparen");
}

#[test]
fn parse_missing_rbrace() {
    snapshot_fixture("parse-missing-rbrace");
}

#[test]
fn parse_expected_expr() {
    snapshot_fixture("parse-expected-expr");
}

#[test]
fn parse_expected_pattern() {
    snapshot_fixture("parse-expected-pattern");
}

#[test]
fn parse_unexpected_in_block() {
    snapshot_fixture("parse-unexpected-in-block");
}

#[test]
fn parse_expected_type_body() {
    snapshot_fixture("parse-expected-type-body");
}

#[test]
fn parse_expected_block_if() {
    snapshot_fixture("parse-expected-block-if");
}

#[test]
fn parse_expected_import() {
    snapshot_fixture("parse-expected-import");
}

// ── HIR ───────────────────────────────────────────────────────────

#[test]
fn hir_duplicate_binding() {
    snapshot_fixture("hir-duplicate-binding");
}

#[test]
fn hir_duplicate_def() {
    snapshot_fixture("hir-duplicate-def");
}

#[test]
fn hir_unresolved_name() {
    snapshot_fixture("hir-unresolved-name");
}

#[test]
fn hir_unresolved_ctor() {
    snapshot_fixture("hir-unresolved-ctor");
}

// ── Type checker ──────────────────────────────────────────────────

#[test]
fn ty_mismatch() {
    snapshot_fixture("ty-mismatch");
}

#[test]
fn ty_unknown_type() {
    snapshot_fixture("ty-unknown-type");
}

#[test]
fn ty_arg_count() {
    snapshot_fixture("ty-arg-count");
}

#[test]
fn ty_not_callable() {
    snapshot_fixture("ty-not-callable");
}

#[test]
fn ty_arithmetic_type() {
    snapshot_fixture("ty-arithmetic-type");
}

#[test]
fn ty_negation_type() {
    snapshot_fixture("ty-negation-type");
}

#[test]
fn ty_ffi_unknown_module() {
    snapshot_fixture("ty-ffi-unknown-module");
}

#[test]
fn ty_non_exhaustive() {
    snapshot_fixture("ty-non-exhaustive");
}

#[test]
fn ty_unreachable_arm() {
    snapshot_fixture("ty-unreachable-arm");
}

#[test]
fn ty_unknown_ctor_pat() {
    snapshot_fixture("ty-unknown-ctor-pat");
}

#[test]
fn ty_field_count_pat() {
    snapshot_fixture("ty-field-count-pat");
}

// ── Cross-layer ───────────────────────────────────────────────────

#[test]
fn multi_parse_and_recover() {
    snapshot_fixture("multi-parse-and-recover");
}

#[test]
fn multi_hir_and_type() {
    snapshot_fixture("multi-hir-and-type");
}
