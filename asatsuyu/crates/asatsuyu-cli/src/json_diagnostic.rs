//! JSON diagnostic output for machine consumption (NDJSON format).
//!
//! Each diagnostic is emitted as a single JSON line to stderr, followed by a
//! summary line. This format is selected via `--error-format json`.
//!
//! The schema is stable: field additions are allowed, field removals are not.

use asatsuyu_syntax::{Diagnostic, LabelStyle, LineCol, LineIndex, Severity};
use serde::Serialize;

// ── JSON schema structs ──────────────────────────────────────────────

/// Top-level NDJSON envelope, discriminated by `type`.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum JsonOutput {
    Diagnostic(JsonDiagnostic),
    Summary(JsonSummary),
}

/// A single compiler diagnostic in JSON form.
#[derive(Serialize)]
pub(crate) struct JsonDiagnostic {
    pub severity: &'static str,
    pub code: Option<String>,
    pub message: String,
    pub file: String,
    pub span: JsonSpan,
    pub labels: Vec<JsonLabel>,
    pub hints: Vec<String>,
    pub notes: Vec<String>,
}

/// A source span with both line/column (1-based) and byte offset.
#[derive(Serialize)]
pub(crate) struct JsonSpan {
    pub start: JsonPosition,
    pub end: JsonPosition,
}

/// A position in a source file.
#[derive(Serialize)]
pub(crate) struct JsonPosition {
    /// 1-based line number.
    pub line: u32,
    /// 1-based column (byte offset from line start, matching rustc).
    pub column: u32,
    /// 0-based byte offset from file start.
    pub offset: u32,
}

/// A labeled span within a diagnostic.
#[derive(Serialize)]
pub(crate) struct JsonLabel {
    pub style: &'static str,
    pub message: String,
    pub span: JsonSpan,
}

/// Diagnostic count summary (always the last NDJSON line).
#[derive(Serialize)]
#[allow(clippy::struct_field_names)] // JSON schema requires `_count` suffix
pub(crate) struct JsonSummary {
    pub error_count: usize,
    pub warning_count: usize,
    pub note_count: usize,
}

// ── Conversion ───────────────────────────────────────────────────────

fn offset_to_position(offset: u32, line_index: &LineIndex) -> JsonPosition {
    let lc = line_index.line_col(offset).unwrap_or(LineCol { line: 1, column: 1 });
    JsonPosition { line: lc.line, column: lc.column, offset }
}

fn span_to_json(span: &asatsuyu_syntax::Span, line_index: &LineIndex) -> JsonSpan {
    JsonSpan {
        start: offset_to_position(span.start, line_index),
        end: offset_to_position(span.end, line_index),
    }
}

/// Convert a compiler diagnostic to a JSON-serializable struct.
pub(crate) fn diagnostic_to_json(
    d: &Diagnostic,
    filename: &str,
    line_index: &LineIndex,
) -> JsonOutput {
    let severity = match d.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Note => "note",
    };

    let labels = d
        .labels
        .iter()
        .map(|l| JsonLabel {
            style: match l.style {
                LabelStyle::Primary => "primary",
                LabelStyle::Secondary => "secondary",
            },
            message: l.message.clone(),
            span: span_to_json(&l.span, line_index),
        })
        .collect();

    JsonOutput::Diagnostic(JsonDiagnostic {
        severity,
        code: d.code.map(|c| c.to_string()),
        message: d.message.clone(),
        file: filename.to_string(),
        span: span_to_json(&d.span, line_index),
        labels,
        hints: d.hints.clone(),
        notes: d.notes.clone(),
    })
}

/// Build a summary JSON object from a slice of diagnostics.
pub(crate) fn summary_to_json(diagnostics: &[Diagnostic]) -> JsonOutput {
    let mut error_count = 0;
    let mut warning_count = 0;
    let mut note_count = 0;
    for d in diagnostics {
        match d.severity {
            Severity::Error => error_count += 1,
            Severity::Warning => warning_count += 1,
            Severity::Note => note_count += 1,
        }
    }
    JsonOutput::Summary(JsonSummary { error_count, warning_count, note_count })
}

// ── Output ───────────────────────────────────────────────────────────

/// Emit a single NDJSON line to stderr.
pub(crate) fn emit_json_line(output: &JsonOutput) {
    let json = serde_json::to_string(output).expect("diagnostic serialization should not fail");
    eprintln!("{json}");
}

#[cfg(test)]
mod tests {
    use asatsuyu_syntax::{DiagnosticCode, FileId, Label, Span};

    use super::*;

    #[test]
    fn diagnostic_json_round_trip() {
        let d = Diagnostic {
            severity: Severity::Error,
            code: Some(DiagnosticCode::E0200),
            message: "type mismatch".into(),
            span: Span::new(FileId(0), 10, 20),
            labels: vec![Label {
                span: Span::new(FileId(0), 10, 20),
                message: "expected Int".into(),
                style: LabelStyle::Primary,
            }],
            hints: vec!["add a type annotation".into()],
            notes: vec!["Int and String are not compatible".into()],
        };
        let line_index = LineIndex::new("0123456789abcdefghij");
        let json_out = diagnostic_to_json(&d, "test.asty", &line_index);

        let json_str = serde_json::to_string(&json_out).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(value["type"], "diagnostic");
        assert_eq!(value["severity"], "error");
        assert_eq!(value["code"], "E0200");
        assert_eq!(value["message"], "type mismatch");
        assert_eq!(value["file"], "test.asty");
        assert_eq!(value["span"]["start"]["line"], 1);
        assert_eq!(value["span"]["start"]["column"], 11);
        assert_eq!(value["span"]["start"]["offset"], 10);
        assert_eq!(value["labels"][0]["style"], "primary");
        assert_eq!(value["hints"][0], "add a type annotation");
        assert_eq!(value["notes"][0], "Int and String are not compatible");
    }

    #[test]
    fn summary_json_counts() {
        let diags = vec![
            Diagnostic::error("e1", Span::dummy()),
            Diagnostic::error("e2", Span::dummy()),
            Diagnostic::warning("w1", Span::dummy()),
        ];
        let json_out = summary_to_json(&diags);
        let json_str = serde_json::to_string(&json_out).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(value["type"], "summary");
        assert_eq!(value["error_count"], 2);
        assert_eq!(value["warning_count"], 1);
        assert_eq!(value["note_count"], 0);
    }

    #[test]
    fn summary_json_empty() {
        let json_out = summary_to_json(&[]);
        let json_str = serde_json::to_string(&json_out).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(value["error_count"], 0);
        assert_eq!(value["warning_count"], 0);
        assert_eq!(value["note_count"], 0);
    }

    #[test]
    fn diagnostic_without_code() {
        let d = Diagnostic::error("no code", Span::new(FileId(0), 0, 5));
        let line_index = LineIndex::new("hello");
        let json_out = diagnostic_to_json(&d, "test.asty", &line_index);
        let json_str = serde_json::to_string(&json_out).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert!(value["code"].is_null());
    }
}
