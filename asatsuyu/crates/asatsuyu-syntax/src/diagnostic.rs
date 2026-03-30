use std::fmt;

use crate::Span;

/// Machine-readable diagnostic codes.
///
/// Convention: `E` + 4-digit number, grouped by phase.
/// - E0001–E0099: syntax / parse errors (reserved)
/// - E0100–E0199: name resolution / HIR errors (reserved)
/// - E0200–E0299: type checking errors
/// - E0300–E0399: match / pattern errors
/// - E0400–E0499: backend errors (reserved)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticCode {
    // ── Type checking ──────────────────────────────────────────────
    /// Type mismatch: expected vs found.
    E0200,
    /// Infinite type (occurs check failure).
    E0201,
    /// Unknown type annotation.
    E0202,
    /// Argument count mismatch.
    E0203,
    /// Expected function, found non-callable type.
    E0204,
    /// Arithmetic operator requires numeric type.
    E0205,
    /// Negation requires numeric type.
    E0206,
    /// Logical operator requires Bool.
    E0207,
    /// Unknown Python module in FFI import.
    E0208,
    /// Field access on opaque FFI type.
    E0209,
    /// Field access on type that does not support it.
    E0210,
    /// Unknown member on FFI module or class.
    E0211,
    /// `try` expression outside a function returning `Result`.
    E0212,

    // ── Match / pattern ────────────────────────────────────────────
    /// Non-exhaustive match.
    E0300,
    /// Unreachable match arm.
    E0301,
    /// Unknown constructor in pattern.
    E0302,
    /// Constructor pattern field count mismatch.
    E0303,
    /// Unsupported pattern kind.
    E0304,
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::E0200 => write!(f, "E0200"),
            Self::E0201 => write!(f, "E0201"),
            Self::E0202 => write!(f, "E0202"),
            Self::E0203 => write!(f, "E0203"),
            Self::E0204 => write!(f, "E0204"),
            Self::E0205 => write!(f, "E0205"),
            Self::E0206 => write!(f, "E0206"),
            Self::E0207 => write!(f, "E0207"),
            Self::E0208 => write!(f, "E0208"),
            Self::E0209 => write!(f, "E0209"),
            Self::E0210 => write!(f, "E0210"),
            Self::E0211 => write!(f, "E0211"),
            Self::E0212 => write!(f, "E0212"),
            Self::E0300 => write!(f, "E0300"),
            Self::E0301 => write!(f, "E0301"),
            Self::E0302 => write!(f, "E0302"),
            Self::E0303 => write!(f, "E0303"),
            Self::E0304 => write!(f, "E0304"),
        }
    }
}

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
    pub code: Option<DiagnosticCode>,
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
            code: None,
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
            code: None,
            message: message.into(),
            span,
            labels: Vec::new(),
            hints: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// Sets the diagnostic code.
    #[must_use]
    pub fn with_code(mut self, code: DiagnosticCode) -> Self {
        self.code = Some(code);
        self
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
        assert_eq!(diag.code, None);
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
            .with_code(DiagnosticCode::E0200)
            .with_label(Span::new(file, 10, 15), "expected Int")
            .with_secondary_label(Span::new(file, 30, 40), "this is String")
            .with_hint("try converting with `to_int`")
            .with_note("Int and String are not compatible");

        assert_eq!(diag.code, Some(DiagnosticCode::E0200));
        assert_eq!(diag.labels.len(), 2);
        assert_eq!(diag.labels[0].style, LabelStyle::Primary);
        assert_eq!(diag.labels[0].message, "expected Int");
        assert_eq!(diag.labels[1].style, LabelStyle::Secondary);
        assert_eq!(diag.hints.len(), 1);
        assert_eq!(diag.notes.len(), 1);
    }

    #[test]
    fn diagnostic_code_display() {
        assert_eq!(DiagnosticCode::E0200.to_string(), "E0200");
        assert_eq!(DiagnosticCode::E0300.to_string(), "E0300");
    }
}
