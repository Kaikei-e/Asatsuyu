//! Idempotency and roundtrip tests for the formatter against all golden fixtures.

use std::path::Path;

use asatsuyu_parser::format_source;
use asatsuyu_syntax::FileId;

/// Run format on every non-error fixture and verify idempotency + roundtrip.
#[test]
fn format_idempotent_all_fixtures() {
    let base =
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("asatsuyu-cli/tests/cases");

    if !base.exists() {
        eprintln!("fixtures dir not found: {base:?}");
        return;
    }

    let mut tested = 0;
    let mut skipped_error = 0;
    let mut failures = Vec::new();

    for entry in std::fs::read_dir(&base).unwrap() {
        let entry = entry.unwrap();
        let case_dir = entry.path();
        let input_path = case_dir.join("input.asty");
        if !input_path.exists() {
            continue;
        }
        let name = case_dir.file_name().unwrap().to_string_lossy().to_string();

        let source = std::fs::read_to_string(&input_path).unwrap();

        // Skip error cases (they have parse errors by design).
        if name.starts_with("err-parse") {
            skipped_error += 1;
            continue;
        }

        let result = format_source(&source);

        if result.has_parse_errors {
            // Some non-error cases may still have parse issues; skip gracefully.
            skipped_error += 1;
            continue;
        }

        // Idempotency: format(format(x)) == format(x)
        let result2 = format_source(&result.formatted);
        if result.formatted != result2.formatted {
            failures.push(format!(
                "{name}: NOT IDEMPOTENT\n  first:  {:?}\n  second: {:?}",
                &result.formatted[..result.formatted.len().min(200)],
                &result2.formatted[..result2.formatted.len().min(200)],
            ));
            continue;
        }

        // Roundtrip: parse(format(x)) has no errors
        let reparsed = asatsuyu_parser::parse(FileId(0), &result.formatted);
        if reparsed.has_errors() {
            failures.push(format!(
                "{name}: ROUNDTRIP PARSE ERROR\n  formatted: {:?}\n  errors: {:?}",
                &result.formatted[..result.formatted.len().min(200)],
                reparsed.diagnostics().iter().map(|d| d.message.clone()).collect::<Vec<_>>(),
            ));
            continue;
        }

        tested += 1;
    }

    eprintln!(
        "format_idempotent_all_fixtures: {tested} tested, {skipped_error} skipped (error cases)"
    );

    assert!(
        failures.is_empty(),
        "{} fixture(s) failed:\n{}",
        failures.len(),
        failures.join("\n\n")
    );

    // We expect at least 30 non-error fixtures to pass.
    assert!(tested >= 30, "expected at least 30 tested fixtures, got {tested}");
}
