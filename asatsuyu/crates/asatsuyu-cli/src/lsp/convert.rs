//! Conversion between Asatsuyu compiler types and LSP types.

use asatsuyu_syntax::{Diagnostic, LineIndex, Severity, Span};
use tower_lsp::lsp_types;

/// Convert an Asatsuyu `Span` to an LSP `Range` using a `LineIndex`.
///
/// LSP positions are 0-based; Asatsuyu `LineCol` is 1-based.
/// MVP: treats byte offsets as character offsets (correct for ASCII).
pub(super) fn span_to_range(span: Span, line_index: &LineIndex) -> lsp_types::Range {
    let start =
        line_index.line_col(span.start).unwrap_or(asatsuyu_syntax::LineCol { line: 1, column: 1 });
    let end =
        line_index.line_col(span.end).unwrap_or(asatsuyu_syntax::LineCol { line: 1, column: 1 });
    lsp_types::Range {
        start: lsp_types::Position {
            line: start.line.saturating_sub(1),
            character: start.column.saturating_sub(1),
        },
        end: lsp_types::Position {
            line: end.line.saturating_sub(1),
            character: end.column.saturating_sub(1),
        },
    }
}

/// Convert an Asatsuyu `Diagnostic` to an LSP `Diagnostic`.
pub(super) fn to_lsp_diagnostic(
    diag: &Diagnostic,
    line_index: &LineIndex,
    uri: &lsp_types::Url,
) -> lsp_types::Diagnostic {
    let range = span_to_range(diag.span, line_index);

    let severity = Some(match diag.severity {
        Severity::Error => lsp_types::DiagnosticSeverity::ERROR,
        Severity::Warning => lsp_types::DiagnosticSeverity::WARNING,
        Severity::Note => lsp_types::DiagnosticSeverity::INFORMATION,
    });

    let code = diag.code.map(|c| lsp_types::NumberOrString::String(format!("{c:?}")));

    // Append hints to message for visibility.
    let mut message = diag.message.clone();
    for hint in &diag.hints {
        message.push_str("\nhint: ");
        message.push_str(hint);
    }
    for note in &diag.notes {
        message.push_str("\nnote: ");
        message.push_str(note);
    }

    // Convert secondary labels to related information.
    let related_information = if diag.labels.len() > 1 {
        Some(
            diag.labels[1..]
                .iter()
                .map(|label| lsp_types::DiagnosticRelatedInformation {
                    location: lsp_types::Location {
                        // Same file for now (single-file analysis).
                        uri: uri.clone(),
                        range: span_to_range(label.span, line_index),
                    },
                    message: label.message.clone(),
                })
                .collect(),
        )
    } else {
        None
    };

    lsp_types::Diagnostic {
        range,
        severity,
        code,
        code_description: None,
        source: Some("asatsuyu".to_owned()),
        message,
        related_information,
        tags: None,
        data: None,
    }
}

/// Convert multiple Asatsuyu diagnostics to LSP diagnostics.
pub(super) fn to_lsp_diagnostics(
    diags: &[Diagnostic],
    line_index: &LineIndex,
    uri: &lsp_types::Url,
) -> Vec<lsp_types::Diagnostic> {
    diags.iter().map(|d| to_lsp_diagnostic(d, line_index, uri)).collect()
}

#[cfg(test)]
mod tests {
    use asatsuyu_syntax::{DiagnosticCode, FileId, LineCol};

    use super::*;

    #[test]
    fn span_to_range_is_zero_based() {
        let index = LineIndex::new("ab\ncd\n");
        let range = span_to_range(Span::new(FileId(0), 3, 5), &index);
        assert_eq!(range.start.line, 1);
        assert_eq!(range.start.character, 0);
        assert_eq!(range.end.line, 1);
        assert_eq!(range.end.character, 2);
        assert_eq!(index.line_col(3), Some(LineCol { line: 2, column: 1 }));
    }

    #[test]
    fn related_information_uses_document_uri() {
        let uri = lsp_types::Url::parse("file:///tmp/example.asty").expect("valid file uri");
        let diag = Diagnostic::error("boom", Span::new(FileId(0), 0, 1))
            .with_code(DiagnosticCode::E0001)
            .with_label(Span::new(FileId(0), 0, 1), "primary")
            .with_secondary_label(Span::new(FileId(0), 2, 3), "secondary");
        let index = LineIndex::new("abc");

        let lsp = to_lsp_diagnostic(&diag, &index, &uri);
        let related = lsp.related_information.expect("related information");
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].location.uri, uri);
        assert_eq!(related[0].message, "secondary");
    }
}
