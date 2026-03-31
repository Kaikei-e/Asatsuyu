//! Adapter from Asatsuyu's [`Diagnostic`] to miette's [`Diagnostic`](miette::Diagnostic) trait.
//!
//! Keeps `asatsuyu-syntax` free of external dependencies by performing the
//! conversion only in the CLI crate.

use std::fmt;

use asatsuyu_syntax::{Diagnostic, LabelStyle, Severity};
use miette::{LabeledSpan, MietteDiagnostic, NamedSource, Severity as MietteSeverity};

/// A miette-compatible diagnostic that wraps source code for pretty rendering.
///
/// Bundles a [`MietteDiagnostic`] with a [`NamedSource`] so that miette can
/// render source code snippets alongside error messages.
#[derive(Debug)]
pub(crate) struct SourceDiagnostic {
    source_code: NamedSource<String>,
    inner: MietteDiagnostic,
}

impl SourceDiagnostic {
    /// Convert an Asatsuyu compiler diagnostic into a miette-renderable report.
    pub(crate) fn from_diagnostic(d: &Diagnostic, filename: &str, source: &str) -> Self {
        let severity = match d.severity {
            Severity::Error => MietteSeverity::Error,
            Severity::Warning => MietteSeverity::Warning,
            Severity::Note => MietteSeverity::Advice,
        };

        let mut diag = MietteDiagnostic::new(&d.message).with_severity(severity);

        // Diagnostic code (e.g., "E0200").
        if let Some(code) = d.code {
            diag = diag.with_code(code.to_string());
        }

        // Labels → LabeledSpan.
        for label in &d.labels {
            let offset = label.span.start as usize;
            let len = (label.span.end - label.span.start) as usize;
            let labeled = match label.style {
                LabelStyle::Primary => LabeledSpan::at(offset..offset + len, &label.message),
                LabelStyle::Secondary => LabeledSpan::new(Some(label.message.clone()), offset, len),
            };
            diag = diag.and_label(labeled);
        }

        // If no labels were added but we have a span, add a primary label
        // pointing to the diagnostic's own span.
        if d.labels.is_empty() && !d.span.is_empty() {
            let offset = d.span.start as usize;
            let len = d.span.len() as usize;
            diag = diag.and_label(LabeledSpan::at(offset..offset + len, "here"));
        }

        // Combine hints and notes into help text.
        let mut help_parts: Vec<&str> = Vec::new();
        for hint in &d.hints {
            help_parts.push(hint);
        }
        for note in &d.notes {
            help_parts.push(note);
        }
        if !help_parts.is_empty() {
            diag = diag.with_help(help_parts.join("\n"));
        }

        Self { source_code: NamedSource::new(filename, source.to_string()), inner: diag }
    }
}

impl fmt::Display for SourceDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner)
    }
}

impl std::error::Error for SourceDiagnostic {}

impl miette::Diagnostic for SourceDiagnostic {
    fn code<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        self.inner.code()
    }

    fn severity(&self) -> Option<MietteSeverity> {
        self.inner.severity()
    }

    fn help<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        self.inner.help()
    }

    fn url<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        self.inner.url()
    }

    fn source_code(&self) -> Option<&dyn miette::SourceCode> {
        Some(&self.source_code)
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = LabeledSpan> + '_>> {
        self.inner.labels()
    }

    fn related<'a>(&'a self) -> Option<Box<dyn Iterator<Item = &'a dyn miette::Diagnostic> + 'a>> {
        None
    }
}
