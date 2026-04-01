use std::fmt;

use crate::Span;

/// Machine-readable diagnostic codes.
///
/// Convention: `E` + 4-digit number, grouped by compiler phase.
/// - E0001–E0049: lexer errors
/// - E0050–E0099: parser errors
/// - E0100–E0149: AST lowering errors
/// - E0150–E0199: name resolution / HIR errors
/// - E0200–E0299: type checking errors
/// - E0300–E0399: match / pattern errors
/// - E0400–E0499: backend errors (reserved)
///
/// Codes are stable: once assigned, the meaning of a code never changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum DiagnosticCode {
    // ── Lexer ─────────────────────────────────────────────────────────
    /// Unexpected character.
    E0001 = 1,

    // ── Parser ────────────────────────────────────────────────────────
    /// Expected a specific token (generic `expect()` failure).
    E0050 = 50,
    /// Expected an item definition at top level.
    E0051 = 51,
    /// Expected function name.
    E0052 = 52,
    /// Expected parameter (name, list, or type).
    E0053 = 53,
    /// Expected function or lambda body.
    E0054 = 54,
    /// Expected expression.
    E0055 = 55,
    /// Expected pattern.
    E0056 = 56,
    /// Unexpected token in context (block, parameter list, argument list, etc.).
    E0057 = 57,
    /// Expected type body.
    E0058 = 58,
    /// Expected return type.
    E0059 = 59,
    /// Expected block after condition or match subject.
    E0060 = 60,
    /// Expected import path component.
    E0061 = 61,
    /// Feature not yet implemented (e.g. top-level `let`).
    E0062 = 62,
    /// Expected assignment value (right-hand side of `=`).
    E0063 = 63,

    // ── AST lowering ──────────────────────────────────────────────────
    /// Unexpected syntax (`NodeError` encountered during CST → AST lowering).
    E0100 = 100,
    /// Empty import path.
    E0101 = 101,
    /// Missing module name in Python import.
    E0102 = 102,
    /// Missing function body.
    E0103 = 103,
    /// Incomplete binary expression.
    E0104 = 104,
    /// Incomplete pipeline expression.
    E0105 = 105,
    /// Unsupported pattern kind.
    E0106 = 106,

    // ── HIR / name resolution ─────────────────────────────────────────
    /// Duplicate binding in the same scope.
    E0150 = 150,
    /// Duplicate definition at module level.
    E0151 = 151,
    /// Unresolved name.
    E0152 = 152,
    /// Unresolved constructor.
    E0153 = 153,

    // ── Type checking ─────────────────────────────────────────────────
    /// Type mismatch: expected vs found.
    E0200 = 200,
    /// Infinite type (occurs check failure).
    E0201 = 201,
    /// Unknown type annotation.
    E0202 = 202,
    /// Argument count mismatch.
    E0203 = 203,
    /// Expected function, found non-callable type.
    E0204 = 204,
    /// Arithmetic operator requires numeric type.
    E0205 = 205,
    /// Negation requires numeric type.
    E0206 = 206,
    /// Logical operator requires Bool.
    E0207 = 207,
    /// Unknown Python module in FFI import.
    E0208 = 208,
    /// Field access on opaque FFI type.
    E0209 = 209,
    /// Field access on type that does not support it.
    E0210 = 210,
    /// Unknown member on FFI module or class.
    E0211 = 211,
    /// `try` expression outside a function returning `Result`.
    E0212 = 212,
    /// `try` expression in a position the backend cannot lower safely.
    E0213 = 213,
    /// `match` subject is an opaque FFI type.
    E0214 = 214,
    /// Cannot assign to immutable binding (missing `mut`).
    E0215 = 215,
    /// Cannot reassign a function parameter.
    E0216 = 216,
    /// Assignment type mismatch: value type differs from binding type.
    E0217 = 217,
    /// Cannot assign to a variable captured from an outer scope inside a lambda.
    E0218 = 218,

    // ── Match / pattern ───────────────────────────────────────────────
    /// Non-exhaustive match.
    E0300 = 300,
    /// Unreachable match arm.
    E0301 = 301,
    /// Unknown constructor in pattern.
    E0302 = 302,
    /// Constructor pattern field count mismatch.
    E0303 = 303,
    /// Unsupported pattern kind.
    E0304 = 304,
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "E{:04}", *self as u16)
    }
}

/// Severity level of a diagnostic message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    /// Informational note, typically attached to a related diagnostic.
    Note,
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

/// A compiler diagnostic (error, warning, or note) with source location and context.
///
/// Designed to produce Gleam-quality error messages. The `miette` integration
/// for terminal rendering lives in `asatsuyu-cli`.
///
/// # Diagnostic contract (v1)
///
/// ## Codes
/// - Every diagnostic SHOULD have a [`DiagnosticCode`]. Codes are stable:
///   once assigned, the meaning of a code never changes.
/// - See [`DiagnosticCode`] for the range allocation by compiler phase.
///
/// ## Labels
/// - **Primary**: points to the exact source of the error. At most one per
///   diagnostic. Message should describe what was found vs. expected.
/// - **Secondary**: points to related context (e.g. "previously defined here").
///   Zero or more per diagnostic.
///
/// ## Hints
/// - Actionable suggestions the user can follow to fix the error.
/// - Use imperative mood: "add a type annotation", "rename the binding".
///
/// ## Notes
/// - Background information that explains *why* the error exists.
/// - Use declarative mood: "Int and String are not compatible".
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

    /// Creates an informational note diagnostic at the given span.
    pub fn note(message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Note,
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
    fn diagnostic_note() {
        let diag = Diagnostic::note("type was inferred here", Span::dummy());
        assert_eq!(diag.severity, Severity::Note);
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
        assert_eq!(DiagnosticCode::E0001.to_string(), "E0001");
        assert_eq!(DiagnosticCode::E0050.to_string(), "E0050");
        assert_eq!(DiagnosticCode::E0200.to_string(), "E0200");
        assert_eq!(DiagnosticCode::E0300.to_string(), "E0300");
    }

    #[test]
    fn diagnostic_code_repr_matches_display() {
        // Verify that repr(u16) discriminants produce the correct display strings.
        assert_eq!(DiagnosticCode::E0001 as u16, 1);
        assert_eq!(DiagnosticCode::E0200 as u16, 200);
        assert_eq!(DiagnosticCode::E0304 as u16, 304);
    }
}
