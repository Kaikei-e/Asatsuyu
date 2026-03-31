//! Core formatter that walks a rowan CST and emits formatted text.

use asatsuyu_syntax::SyntaxKind;

use crate::language::{SyntaxNode, SyntaxToken};

use super::rules;

/// Indentation unit: 2 spaces per level (matching existing fixture conventions).
const INDENT: &str = "  ";

pub(super) struct Formatter {
    output: String,
    indent_level: usize,
    /// Tracks whether we've already written a newline at the end of output,
    /// so we can avoid double blank lines.
    trailing_newlines: usize,
}

impl Formatter {
    pub(super) fn new() -> Self {
        Self { output: String::new(), indent_level: 0, trailing_newlines: 0 }
    }

    /// Entry point: format the root `SourceFile` node and return the output.
    pub(super) fn format_node(mut self, root: &SyntaxNode) -> String {
        debug_assert_eq!(root.kind(), SyntaxKind::SourceFile);
        rules::format_source_file(&mut self, root);
        // Ensure exactly one trailing newline.
        self.ensure_trailing_newline();
        self.output
    }

    // ── Output helpers ──────────────────────────────────────────────

    pub(crate) fn write_str(&mut self, s: &str) {
        if !s.is_empty() {
            self.output.push_str(s);
            self.trailing_newlines = 0;
        }
    }

    pub(crate) fn write_token(&mut self, token: &SyntaxToken) {
        self.write_str(token.text());
    }

    pub(crate) fn write_space(&mut self) {
        self.output.push(' ');
        self.trailing_newlines = 0;
    }

    pub(crate) fn write_newline(&mut self) {
        self.output.push('\n');
        self.trailing_newlines += 1;
    }

    /// Write a blank line (two newlines), but only if there isn't one already.
    pub(crate) fn write_blank_line(&mut self) {
        while self.trailing_newlines < 2 {
            self.write_newline();
        }
    }

    pub(crate) fn write_indent(&mut self) {
        for _ in 0..self.indent_level {
            self.output.push_str(INDENT);
        }
        if self.indent_level > 0 {
            self.trailing_newlines = 0;
        }
    }

    pub(crate) fn indent(&mut self) {
        self.indent_level += 1;
    }

    pub(crate) fn dedent(&mut self) {
        self.indent_level = self.indent_level.saturating_sub(1);
    }

    fn ensure_trailing_newline(&mut self) {
        if self.trailing_newlines == 0 {
            self.write_newline();
        }
    }
}
