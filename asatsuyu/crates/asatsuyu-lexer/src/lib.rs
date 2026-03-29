//! Lexer for the Asatsuyu language.
//!
//! Transforms source text into a stream of tokens with span information.
//! Uses [`logos`] for fast DFA-based tokenization.
//!
//! Trivia tokens (whitespace, newlines, comments) are **not** skipped —
//! they are preserved so the parser can build a lossless CST with rowan.

use logos::Logos;
use smol_str::SmolStr;

use asatsuyu_syntax::{Diagnostic, FileId, Span, SyntaxKind};

/// Internal token enum for the `logos` lexer.
///
/// This maps 1:1 to the token subset of [`SyntaxKind`] via the [`From`] impl.
/// It exists as a separate type because `SyntaxKind` also contains node kinds
/// that logos cannot derive.
#[derive(Logos, Debug, Clone, Copy, PartialEq)]
enum LexToken {
    // === Keywords ===
    // `#[token]` has higher default priority than `#[regex]`, so keywords
    // naturally win over the identifier pattern.
    #[token("fn")]
    FnKw,
    #[token("pub")]
    PubKw,

    // === Literals ===
    #[regex("[0-9]+")]
    IntLit,
    #[regex(r#""[^"]*""#)]
    StringLit,

    // === Identifiers ===
    #[regex("[a-zA-Z_][a-zA-Z0-9_]*")]
    Ident,

    // === Delimiters & Punctuation ===
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token(",")]
    Comma,
    #[token(":")]
    Colon,
    #[token("->")]
    Arrow,

    // === Trivia (NOT skipped — needed for lossless CST) ===
    #[regex(r"[ \t\r]+")]
    Whitespace,
    #[token("\n")]
    Newline,
    #[regex("//[^\n]*", allow_greedy = true)]
    Comment,
}

impl From<LexToken> for SyntaxKind {
    fn from(token: LexToken) -> SyntaxKind {
        match token {
            LexToken::FnKw => SyntaxKind::FnKw,
            LexToken::PubKw => SyntaxKind::PubKw,
            LexToken::IntLit => SyntaxKind::IntLit,
            LexToken::StringLit => SyntaxKind::StringLit,
            LexToken::Ident => SyntaxKind::Ident,
            LexToken::LParen => SyntaxKind::LParen,
            LexToken::RParen => SyntaxKind::RParen,
            LexToken::LBrace => SyntaxKind::LBrace,
            LexToken::RBrace => SyntaxKind::RBrace,
            LexToken::Comma => SyntaxKind::Comma,
            LexToken::Colon => SyntaxKind::Colon,
            LexToken::Arrow => SyntaxKind::Arrow,
            LexToken::Whitespace => SyntaxKind::Whitespace,
            LexToken::Newline => SyntaxKind::Newline,
            LexToken::Comment => SyntaxKind::Comment,
        }
    }
}

/// A single token produced by the lexer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: SyntaxKind,
    pub span: Span,
    pub text: SmolStr,
}

/// Tokenize `source` into a list of tokens and any diagnostics for invalid input.
///
/// All trivia (whitespace, newlines, comments) is preserved as explicit tokens.
/// An [`SyntaxKind::Eof`] token is always appended at the end.
#[must_use]
pub fn lex(source: &str, file_id: FileId) -> (Vec<Token>, Vec<Diagnostic>) {
    // Rough estimate: average token length ~3 bytes (keywords, delimiters, whitespace).
    let mut tokens = Vec::with_capacity(source.len() / 3 + 1);
    let mut diagnostics = Vec::new();
    let mut lexer = LexToken::lexer(source);

    while let Some(result) = lexer.next() {
        let range = lexer.span();
        let text = SmolStr::from(lexer.slice());
        #[allow(clippy::cast_possible_truncation)] // Span uses u32; files > 4 GiB are unsupported.
        let span = Span::new(file_id, range.start as u32, range.end as u32);

        if let Ok(lex_token) = result {
            tokens.push(Token { kind: SyntaxKind::from(lex_token), span, text });
        } else {
            diagnostics.push(
                Diagnostic::error("unexpected character", span).with_label(span, "invalid token"),
            );
            tokens.push(Token { kind: SyntaxKind::Error, span, text });
        }
    }

    // Append EOF token.
    #[allow(clippy::cast_possible_truncation)]
    let eof_offset = source.len() as u32;
    tokens.push(Token {
        kind: SyntaxKind::Eof,
        span: Span::new(file_id, eof_offset, eof_offset),
        text: SmolStr::default(),
    });

    (tokens, diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use asatsuyu_syntax::FileId;

    /// Extract just the `SyntaxKind` sequence from tokens (convenience for assertions).
    fn token_kinds(tokens: &[Token]) -> Vec<SyntaxKind> {
        tokens.iter().map(|t| t.kind).collect()
    }

    const FID: FileId = FileId(0);

    // --- 1. Empty input ---

    #[test]
    fn lex_empty() {
        let (tokens, diags) = lex("", FID);
        assert!(diags.is_empty());
        assert_eq!(token_kinds(&tokens), vec![SyntaxKind::Eof]);
    }

    // --- 2–3. Keywords ---

    #[test]
    fn lex_fn_keyword() {
        let (tokens, diags) = lex("fn", FID);
        assert!(diags.is_empty());
        assert_eq!(token_kinds(&tokens), vec![SyntaxKind::FnKw, SyntaxKind::Eof]);
    }

    #[test]
    fn lex_pub_keyword() {
        let (tokens, diags) = lex("pub", FID);
        assert!(diags.is_empty());
        assert_eq!(token_kinds(&tokens), vec![SyntaxKind::PubKw, SyntaxKind::Eof]);
    }

    // --- 4. Identifiers ---

    #[test]
    fn lex_identifier() {
        let (tokens, diags) = lex("main", FID);
        assert!(diags.is_empty());
        assert_eq!(token_kinds(&tokens), vec![SyntaxKind::Ident, SyntaxKind::Eof]);
        assert_eq!(tokens[0].text, "main");
    }

    // --- 5–6. Literals ---

    #[test]
    fn lex_int_literal() {
        let (tokens, diags) = lex("42", FID);
        assert!(diags.is_empty());
        assert_eq!(token_kinds(&tokens), vec![SyntaxKind::IntLit, SyntaxKind::Eof]);
        assert_eq!(tokens[0].text, "42");
    }

    #[test]
    fn lex_string_literal() {
        let (tokens, diags) = lex(r#""hello""#, FID);
        assert!(diags.is_empty());
        assert_eq!(token_kinds(&tokens), vec![SyntaxKind::StringLit, SyntaxKind::Eof]);
        assert_eq!(tokens[0].text, r#""hello""#);
    }

    // --- 7. Delimiters ---

    #[test]
    fn lex_delimiters() {
        let (tokens, diags) = lex("(){},:->", FID);
        assert!(diags.is_empty());
        assert_eq!(
            token_kinds(&tokens),
            vec![
                SyntaxKind::LParen,
                SyntaxKind::RParen,
                SyntaxKind::LBrace,
                SyntaxKind::RBrace,
                SyntaxKind::Comma,
                SyntaxKind::Colon,
                SyntaxKind::Arrow,
                SyntaxKind::Eof,
            ]
        );
    }

    // --- 8–9. Trivia preservation ---

    #[test]
    fn lex_whitespace_preserved() {
        let (tokens, diags) = lex("fn main", FID);
        assert!(diags.is_empty());
        assert_eq!(
            token_kinds(&tokens),
            vec![SyntaxKind::FnKw, SyntaxKind::Whitespace, SyntaxKind::Ident, SyntaxKind::Eof]
        );
    }

    #[test]
    fn lex_newline_preserved() {
        let (tokens, diags) = lex("a\nb", FID);
        assert!(diags.is_empty());
        assert_eq!(
            token_kinds(&tokens),
            vec![SyntaxKind::Ident, SyntaxKind::Newline, SyntaxKind::Ident, SyntaxKind::Eof]
        );
    }

    // --- 10. Comments ---

    #[test]
    fn lex_comment() {
        let (tokens, diags) = lex("// hello", FID);
        assert!(diags.is_empty());
        assert_eq!(token_kinds(&tokens), vec![SyntaxKind::Comment, SyntaxKind::Eof]);
        assert_eq!(tokens[0].text, "// hello");
    }

    // --- 11. DoD: minimal program ---

    #[test]
    fn lex_minimal_program() {
        let (tokens, diags) = lex("pub fn main() { 1 }", FID);
        assert!(diags.is_empty());
        assert_eq!(
            token_kinds(&tokens),
            vec![
                SyntaxKind::PubKw,      // pub
                SyntaxKind::Whitespace, // " "
                SyntaxKind::FnKw,       // fn
                SyntaxKind::Whitespace, // " "
                SyntaxKind::Ident,      // main
                SyntaxKind::LParen,     // (
                SyntaxKind::RParen,     // )
                SyntaxKind::Whitespace, // " "
                SyntaxKind::LBrace,     // {
                SyntaxKind::Whitespace, // " "
                SyntaxKind::IntLit,     // 1
                SyntaxKind::Whitespace, // " "
                SyntaxKind::RBrace,     // }
                SyntaxKind::Eof,
            ]
        );
    }

    // --- 12. Invalid token ---

    #[test]
    fn lex_invalid_token() {
        let (tokens, diags) = lex("@", FID);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "unexpected character");
        assert_eq!(token_kinds(&tokens), vec![SyntaxKind::Error, SyntaxKind::Eof]);
        assert_eq!(tokens[0].text, "@");
    }

    // --- 13. Span correctness ---

    #[test]
    fn lex_spans_correct() {
        let (tokens, _) = lex("fn main", FID);
        // "fn" at 0..2
        assert_eq!(tokens[0].span.start, 0);
        assert_eq!(tokens[0].span.end, 2);
        // " " at 2..3
        assert_eq!(tokens[1].span.start, 2);
        assert_eq!(tokens[1].span.end, 3);
        // "main" at 3..7
        assert_eq!(tokens[2].span.start, 3);
        assert_eq!(tokens[2].span.end, 7);
        // Eof at 7..7
        assert_eq!(tokens[3].span.start, 7);
        assert_eq!(tokens[3].span.end, 7);
    }

    // --- 14. Arrow ---

    #[test]
    fn lex_arrow() {
        let (tokens, diags) = lex("-> Int", FID);
        assert!(diags.is_empty());
        assert_eq!(
            token_kinds(&tokens),
            vec![SyntaxKind::Arrow, SyntaxKind::Whitespace, SyntaxKind::Ident, SyntaxKind::Eof]
        );
    }

    // --- 15. Keyword prefix is not a keyword ---

    #[test]
    fn lex_keyword_prefix() {
        let (tokens, diags) = lex("fns", FID);
        assert!(diags.is_empty());
        assert_eq!(token_kinds(&tokens), vec![SyntaxKind::Ident, SyntaxKind::Eof]);
        assert_eq!(tokens[0].text, "fns");
    }
}
