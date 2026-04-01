//! Centralized keyword taxonomy for the Asatsuyu language.
//!
//! This module provides the single source of truth for all keyword definitions.
//! The [`KEYWORDS`] table is referenced by the lexer, parser, and LSP to derive
//! keyword knowledge, ensuring consistency across the compiler.

use crate::syntax_kind::SyntaxKind;

/// Classification of keywords in the Asatsuyu language.
///
/// This taxonomy determines how the keyword interacts with parsing,
/// error messages, and tooling (LSP completions, documentation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeywordClass {
    /// Always reserved. Cannot be used as identifiers in any context.
    /// Examples: `fn`, `let`, `match`, `if`
    Hard,
    /// Boolean literals that are keywords syntactically.
    /// Examples: `True`, `False`
    Literal,
    /// Only meaningful in specific syntactic positions.
    /// Examples: `python` (in `from python import`), `as` (in import aliases)
    Contextual,
    /// Reserved for future language features. Using them is an error today.
    /// Examples: `mut`, `async`, `await`
    Reserved,
}

/// A single keyword entry in the centralized keyword table.
///
/// Each entry pairs the source text with its [`SyntaxKind`] and
/// [`KeywordClass`]. The table is `const` and zero-allocation.
pub struct KeywordSpec {
    /// The source text that lexes to this keyword (e.g., `"fn"`, `"True"`).
    pub text: &'static str,
    /// The corresponding [`SyntaxKind`] token kind.
    pub kind: SyntaxKind,
    /// The keyword classification.
    pub class: KeywordClass,
}

/// Centralized keyword table — the single source of truth for all keyword
/// definitions in the Asatsuyu language.
///
/// Every keyword recognized by the lexer appears here exactly once.
/// The lexer, parser, and LSP all derive their keyword knowledge from
/// this table (or from the helper methods on [`SyntaxKind`] that
/// delegate to it).
///
/// Ordered by [`KeywordClass`]: Hard, Literal, Contextual, Reserved.
pub const KEYWORDS: &[KeywordSpec] = &[
    // Hard keywords (10)
    KeywordSpec { text: "fn", kind: SyntaxKind::FnKw, class: KeywordClass::Hard },
    KeywordSpec { text: "pub", kind: SyntaxKind::PubKw, class: KeywordClass::Hard },
    KeywordSpec { text: "let", kind: SyntaxKind::LetKw, class: KeywordClass::Hard },
    KeywordSpec { text: "type", kind: SyntaxKind::TypeKw, class: KeywordClass::Hard },
    KeywordSpec { text: "match", kind: SyntaxKind::MatchKw, class: KeywordClass::Hard },
    KeywordSpec { text: "if", kind: SyntaxKind::IfKw, class: KeywordClass::Hard },
    KeywordSpec { text: "else", kind: SyntaxKind::ElseKw, class: KeywordClass::Hard },
    KeywordSpec { text: "import", kind: SyntaxKind::ImportKw, class: KeywordClass::Hard },
    KeywordSpec { text: "from", kind: SyntaxKind::FromKw, class: KeywordClass::Hard },
    KeywordSpec { text: "try", kind: SyntaxKind::TryKw, class: KeywordClass::Hard },
    // Literal keywords (2)
    KeywordSpec { text: "True", kind: SyntaxKind::TrueKw, class: KeywordClass::Literal },
    KeywordSpec { text: "False", kind: SyntaxKind::FalseKw, class: KeywordClass::Literal },
    // Contextual keywords (2)
    KeywordSpec { text: "python", kind: SyntaxKind::PythonKw, class: KeywordClass::Contextual },
    KeywordSpec { text: "as", kind: SyntaxKind::AsKw, class: KeywordClass::Contextual },
    // Reserved keywords (3)
    KeywordSpec { text: "mut", kind: SyntaxKind::MutKw, class: KeywordClass::Reserved },
    KeywordSpec { text: "async", kind: SyntaxKind::AsyncKw, class: KeywordClass::Reserved },
    KeywordSpec { text: "await", kind: SyntaxKind::AwaitKw, class: KeywordClass::Reserved },
];

/// Look up the [`SyntaxKind`] for a keyword source text, or `None` if the text
/// is not a keyword.
#[must_use]
pub fn kind_of_text(text: &str) -> Option<SyntaxKind> {
    KEYWORDS.iter().find(|spec| spec.text == text).map(|spec| spec.kind)
}

/// Look up the [`KeywordClass`] for a [`SyntaxKind`], or `None` if it is
/// not a keyword.
#[must_use]
pub const fn class_of(kind: SyntaxKind) -> Option<KeywordClass> {
    let mut i = 0;
    while i < KEYWORDS.len() {
        if KEYWORDS[i].kind as u16 == kind as u16 {
            return Some(KEYWORDS[i].class);
        }
        i += 1;
    }
    None
}

/// Look up the source text for a keyword [`SyntaxKind`], or `None` if it is
/// not a keyword.
#[must_use]
pub const fn text_of(kind: SyntaxKind) -> Option<&'static str> {
    let mut i = 0;
    while i < KEYWORDS.len() {
        if KEYWORDS[i].kind as u16 == kind as u16 {
            return Some(KEYWORDS[i].text);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_table_has_17_entries() {
        assert_eq!(KEYWORDS.len(), 17);
    }

    #[test]
    fn all_keyword_kinds_are_keywords() {
        for spec in KEYWORDS {
            assert!(spec.kind.is_keyword(), "{:?} should be a keyword", spec.kind);
        }
    }

    #[test]
    fn no_duplicate_kinds() {
        for (i, a) in KEYWORDS.iter().enumerate() {
            for (j, b) in KEYWORDS.iter().enumerate() {
                if i != j {
                    assert_ne!(
                        a.kind, b.kind,
                        "duplicate SyntaxKind in KEYWORDS table: {:?}",
                        a.kind
                    );
                }
            }
        }
    }

    #[test]
    fn no_duplicate_texts() {
        for (i, a) in KEYWORDS.iter().enumerate() {
            for (j, b) in KEYWORDS.iter().enumerate() {
                if i != j {
                    assert_ne!(a.text, b.text, "duplicate text in KEYWORDS table: {:?}", a.text);
                }
            }
        }
    }

    #[test]
    fn class_of_hard_keywords() {
        assert_eq!(class_of(SyntaxKind::FnKw), Some(KeywordClass::Hard));
        assert_eq!(class_of(SyntaxKind::LetKw), Some(KeywordClass::Hard));
        assert_eq!(class_of(SyntaxKind::MatchKw), Some(KeywordClass::Hard));
        assert_eq!(class_of(SyntaxKind::IfKw), Some(KeywordClass::Hard));
        assert_eq!(class_of(SyntaxKind::TryKw), Some(KeywordClass::Hard));
    }

    #[test]
    fn class_of_literal_keywords() {
        assert_eq!(class_of(SyntaxKind::TrueKw), Some(KeywordClass::Literal));
        assert_eq!(class_of(SyntaxKind::FalseKw), Some(KeywordClass::Literal));
    }

    #[test]
    fn class_of_contextual_keywords() {
        assert_eq!(class_of(SyntaxKind::PythonKw), Some(KeywordClass::Contextual));
        assert_eq!(class_of(SyntaxKind::AsKw), Some(KeywordClass::Contextual));
    }

    #[test]
    fn class_of_reserved_keywords() {
        assert_eq!(class_of(SyntaxKind::MutKw), Some(KeywordClass::Reserved));
        assert_eq!(class_of(SyntaxKind::AsyncKw), Some(KeywordClass::Reserved));
        assert_eq!(class_of(SyntaxKind::AwaitKw), Some(KeywordClass::Reserved));
    }

    #[test]
    fn class_of_non_keyword_returns_none() {
        assert_eq!(class_of(SyntaxKind::Ident), None);
        assert_eq!(class_of(SyntaxKind::Plus), None);
        assert_eq!(class_of(SyntaxKind::LParen), None);
        assert_eq!(class_of(SyntaxKind::SourceFile), None);
    }

    #[test]
    fn text_of_keywords() {
        assert_eq!(text_of(SyntaxKind::FnKw), Some("fn"));
        assert_eq!(text_of(SyntaxKind::TrueKw), Some("True"));
        assert_eq!(text_of(SyntaxKind::PythonKw), Some("python"));
        assert_eq!(text_of(SyntaxKind::AsKw), Some("as"));
        assert_eq!(text_of(SyntaxKind::MutKw), Some("mut"));
        assert_eq!(text_of(SyntaxKind::AsyncKw), Some("async"));
        assert_eq!(text_of(SyntaxKind::AwaitKw), Some("await"));
    }

    #[test]
    fn text_of_non_keyword_returns_none() {
        assert_eq!(text_of(SyntaxKind::Ident), None);
        assert_eq!(text_of(SyntaxKind::Plus), None);
    }

    #[test]
    fn kind_of_text_keywords() {
        assert_eq!(kind_of_text("fn"), Some(SyntaxKind::FnKw));
        assert_eq!(kind_of_text("True"), Some(SyntaxKind::TrueKw));
        assert_eq!(kind_of_text("python"), Some(SyntaxKind::PythonKw));
        assert_eq!(kind_of_text("await"), Some(SyntaxKind::AwaitKw));
    }

    #[test]
    fn kind_of_text_non_keyword_returns_none() {
        assert_eq!(kind_of_text("main"), None);
        assert_eq!(kind_of_text("value"), None);
    }
}
