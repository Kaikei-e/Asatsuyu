use crate::Span;

/// Severity level of a diagnostic message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// Visual style of a source label in diagnostic output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelStyle {
    /// The primary label that points to the main source of the diagnostic.
    Primary,
    /// A secondary label providing additional context.
    Secondary,
}

/// A labeled source span within a diagnostic.
#[derive(Debug, Clone)]
pub struct Label {
    pub span: Span,
    pub message: String,
    pub style: LabelStyle,
}

/// A compiler diagnostic (error or warning) with source location and context.
///
/// Designed to produce Gleam-quality error messages. The `miette` integration
/// for terminal rendering lives in `asatsuyu-cli`.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub span: Span,
    pub labels: Vec<Label>,
    pub hints: Vec<String>,
    pub notes: Vec<String>,
}

impl Diagnostic {
    /// Creates an error diagnostic at the given span.
    pub fn error(message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            span,
            labels: Vec::new(),
            hints: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// Creates a warning diagnostic at the given span.
    pub fn warning(message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
            span,
            labels: Vec::new(),
            hints: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// Adds a primary label at the given span.
    #[must_use]
    pub fn with_label(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(Label { span, message: message.into(), style: LabelStyle::Primary });
        self
    }

    /// Adds a secondary label at the given span.
    #[must_use]
    pub fn with_secondary_label(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(Label { span, message: message.into(), style: LabelStyle::Secondary });
        self
    }

    /// Adds a hint (suggested fix) to this diagnostic.
    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hints.push(hint.into());
        self
    }

    /// Adds a note (additional context) to this diagnostic.
    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FileId, Span};

    #[test]
    fn diagnostic_error() {
        let span = Span::new(FileId(0), 10, 20);
        let diag = Diagnostic::error("unexpected token", span);
        assert_eq!(diag.severity, Severity::Error);
        assert_eq!(diag.message, "unexpected token");
        assert_eq!(diag.span, span);
        assert!(diag.labels.is_empty());
        assert!(diag.hints.is_empty());
        assert!(diag.notes.is_empty());
    }

    #[test]
    fn diagnostic_warning() {
        let diag = Diagnostic::warning("unused variable", Span::dummy());
        assert_eq!(diag.severity, Severity::Warning);
    }

    #[test]
    fn diagnostic_builder() {
        let file = FileId(0);
        let diag = Diagnostic::error("type mismatch", Span::new(file, 10, 15))
            .with_label(Span::new(file, 10, 15), "expected Int")
            .with_secondary_label(Span::new(file, 30, 40), "this is String")
            .with_hint("try converting with `to_int`")
            .with_note("Int and String are not compatible");

        assert_eq!(diag.labels.len(), 2);
        assert_eq!(diag.labels[0].style, LabelStyle::Primary);
        assert_eq!(diag.labels[0].message, "expected Int");
        assert_eq!(diag.labels[1].style, LabelStyle::Secondary);
        assert_eq!(diag.hints.len(), 1);
        assert_eq!(diag.notes.len(), 1);
    }
}
