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
///
/// **Priority rules** (logos resolves ambiguity automatically):
/// - `#[token]` (literal) beats `#[regex]` → keywords win over identifiers
/// - Longer match beats shorter → `|>` wins over `|`, `==` over `=`, etc.
/// - `#[token("_")]` beats `#[regex("[a-zA-Z_]...")]` for single `_`
#[derive(Logos, Debug, Clone, Copy, PartialEq)]
enum LexToken {
    // === Keywords ===
    #[token("fn")]
    FnKw,
    #[token("pub")]
    PubKw,
    #[token("let")]
    LetKw,
    #[token("type")]
    TypeKw,
    #[token("match")]
    MatchKw,
    #[token("if")]
    IfKw,
    #[token("else")]
    ElseKw,
    #[token("import")]
    ImportKw,
    #[token("from")]
    FromKw,
    #[token("python")]
    PythonKw,
    #[token("as")]
    AsKw,
    #[token("True")]
    TrueKw,
    #[token("False")]
    FalseKw,
    #[token("try")]
    TryKw,

    // === Literals ===
    // Float before Int in source for clarity; logos uses longest-match so
    // "3.14" matches FloatLit (5 chars) over IntLit (1 char).
    #[regex("[0-9]+\\.[0-9]+")]
    FloatLit,
    #[regex("[0-9]+")]
    IntLit,
    #[regex(r#""[^"]*""#)]
    StringLit,

    // === Identifiers ===
    #[regex("[a-zA-Z_][a-zA-Z0-9_]*")]
    Ident,

    // === Operators ===
    #[token("|>")]
    Pipe,
    #[token("=>")]
    FatArrow,
    #[token("<>")]
    StringConcat,
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("%")]
    Percent,
    #[token("==")]
    EqEq,
    #[token("!=")]
    BangEq,
    #[token("<=")]
    LtEq,
    #[token(">=")]
    GtEq,
    #[token("=")]
    Eq,
    #[token("<")]
    Lt,
    #[token(">")]
    Gt,
    #[token("!")]
    Bang,
    #[token("&")]
    Ampersand,
    #[token("|")]
    PipeSingle,
    #[token("&&")]
    AmpAmp,
    #[token("||")]
    PipePipe,

    // === Delimiters & Punctuation ===
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token(",")]
    Comma,
    #[token(":")]
    Colon,
    #[token(";")]
    Semicolon,
    #[token("..")]
    DotDot,
    #[token(".")]
    Dot,
    #[token("->")]
    Arrow,
    #[token("_", priority = 3)]
    Underscore,

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
            LexToken::LetKw => SyntaxKind::LetKw,
            LexToken::TypeKw => SyntaxKind::TypeKw,
            LexToken::MatchKw => SyntaxKind::MatchKw,
            LexToken::IfKw => SyntaxKind::IfKw,
            LexToken::ElseKw => SyntaxKind::ElseKw,
            LexToken::ImportKw => SyntaxKind::ImportKw,
            LexToken::FromKw => SyntaxKind::FromKw,
            LexToken::PythonKw => SyntaxKind::PythonKw,
            LexToken::AsKw => SyntaxKind::AsKw,
            LexToken::TrueKw => SyntaxKind::TrueKw,
            LexToken::FalseKw => SyntaxKind::FalseKw,
            LexToken::TryKw => SyntaxKind::TryKw,
            LexToken::IntLit => SyntaxKind::IntLit,
            LexToken::FloatLit => SyntaxKind::FloatLit,
            LexToken::StringLit => SyntaxKind::StringLit,
            LexToken::Ident => SyntaxKind::Ident,
            LexToken::Plus => SyntaxKind::Plus,
            LexToken::Minus => SyntaxKind::Minus,
            LexToken::Star => SyntaxKind::Star,
            LexToken::Slash => SyntaxKind::Slash,
            LexToken::Percent => SyntaxKind::Percent,
            LexToken::Eq => SyntaxKind::Eq,
            LexToken::EqEq => SyntaxKind::EqEq,
            LexToken::BangEq => SyntaxKind::BangEq,
            LexToken::Lt => SyntaxKind::Lt,
            LexToken::LtEq => SyntaxKind::LtEq,
            LexToken::Gt => SyntaxKind::Gt,
            LexToken::GtEq => SyntaxKind::GtEq,
            LexToken::Bang => SyntaxKind::Bang,
            LexToken::Ampersand => SyntaxKind::Ampersand,
            LexToken::PipeSingle => SyntaxKind::PipeSingle,
            LexToken::AmpAmp => SyntaxKind::AmpAmp,
            LexToken::PipePipe => SyntaxKind::PipePipe,
            LexToken::Pipe => SyntaxKind::Pipe,
            LexToken::FatArrow => SyntaxKind::FatArrow,
            LexToken::StringConcat => SyntaxKind::StringConcat,
            LexToken::LParen => SyntaxKind::LParen,
            LexToken::RParen => SyntaxKind::RParen,
            LexToken::LBrace => SyntaxKind::LBrace,
            LexToken::RBrace => SyntaxKind::RBrace,
            LexToken::LBracket => SyntaxKind::LBracket,
            LexToken::RBracket => SyntaxKind::RBracket,
            LexToken::Comma => SyntaxKind::Comma,
            LexToken::Colon => SyntaxKind::Colon,
            LexToken::Semicolon => SyntaxKind::Semicolon,
            LexToken::Dot => SyntaxKind::Dot,
            LexToken::DotDot => SyntaxKind::DotDot,
            LexToken::Arrow => SyntaxKind::Arrow,
            LexToken::Underscore => SyntaxKind::Underscore,
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

    /// Render tokens as a multi-line string for snapshot tests.
    fn snapshot_tokens(source: &str) -> String {
        let (tokens, _) = lex(source, FID);
        tokens
            .iter()
            .map(|t| format!("{:?} {:?} {}..{}", t.kind, t.text.as_str(), t.span.start, t.span.end))
            .collect::<Vec<_>>()
            .join("\n")
    }

    const FID: FileId = FileId(0);

    // =====================================================================
    // Assertion-based tests (retained from Issue 04)
    // =====================================================================

    #[test]
    fn lex_empty() {
        let (tokens, diags) = lex("", FID);
        assert!(diags.is_empty());
        assert_eq!(token_kinds(&tokens), vec![SyntaxKind::Eof]);
    }

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

    #[test]
    fn lex_identifier() {
        let (tokens, diags) = lex("main", FID);
        assert!(diags.is_empty());
        assert_eq!(token_kinds(&tokens), vec![SyntaxKind::Ident, SyntaxKind::Eof]);
        assert_eq!(tokens[0].text, "main");
    }

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

    #[test]
    fn lex_comment() {
        let (tokens, diags) = lex("// hello", FID);
        assert!(diags.is_empty());
        assert_eq!(token_kinds(&tokens), vec![SyntaxKind::Comment, SyntaxKind::Eof]);
        assert_eq!(tokens[0].text, "// hello");
    }

    #[test]
    fn lex_minimal_program() {
        let (tokens, diags) = lex("pub fn main() { 1 }", FID);
        assert!(diags.is_empty());
        assert_eq!(
            token_kinds(&tokens),
            vec![
                SyntaxKind::PubKw,
                SyntaxKind::Whitespace,
                SyntaxKind::FnKw,
                SyntaxKind::Whitespace,
                SyntaxKind::Ident,
                SyntaxKind::LParen,
                SyntaxKind::RParen,
                SyntaxKind::Whitespace,
                SyntaxKind::LBrace,
                SyntaxKind::Whitespace,
                SyntaxKind::IntLit,
                SyntaxKind::Whitespace,
                SyntaxKind::RBrace,
                SyntaxKind::Eof,
            ]
        );
    }

    #[test]
    fn lex_invalid_token() {
        let (tokens, diags) = lex("@", FID);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "unexpected character");
        assert_eq!(token_kinds(&tokens), vec![SyntaxKind::Error, SyntaxKind::Eof]);
        assert_eq!(tokens[0].text, "@");
    }

    #[test]
    fn lex_spans_correct() {
        let (tokens, _) = lex("fn main", FID);
        assert_eq!(tokens[0].span.start, 0);
        assert_eq!(tokens[0].span.end, 2);
        assert_eq!(tokens[1].span.start, 2);
        assert_eq!(tokens[1].span.end, 3);
        assert_eq!(tokens[2].span.start, 3);
        assert_eq!(tokens[2].span.end, 7);
        assert_eq!(tokens[3].span.start, 7);
        assert_eq!(tokens[3].span.end, 7);
    }

    #[test]
    fn lex_arrow() {
        let (tokens, diags) = lex("-> Int", FID);
        assert!(diags.is_empty());
        assert_eq!(
            token_kinds(&tokens),
            vec![SyntaxKind::Arrow, SyntaxKind::Whitespace, SyntaxKind::Ident, SyntaxKind::Eof]
        );
    }

    #[test]
    fn lex_keyword_prefix() {
        let (tokens, diags) = lex("fns", FID);
        assert!(diags.is_empty());
        assert_eq!(token_kinds(&tokens), vec![SyntaxKind::Ident, SyntaxKind::Eof]);
        assert_eq!(tokens[0].text, "fns");
    }

    // =====================================================================
    // Snapshot tests (Issue 12)
    // =====================================================================

    // --- Keywords ---

    #[test]
    fn snap_keyword_let() {
        insta::assert_snapshot!(snapshot_tokens("let"), @r#"
        LetKw "let" 0..3
        Eof "" 3..3
        "#);
    }

    #[test]
    fn snap_keyword_type() {
        insta::assert_snapshot!(snapshot_tokens("type"), @r#"
        TypeKw "type" 0..4
        Eof "" 4..4
        "#);
    }

    #[test]
    fn snap_keyword_match() {
        insta::assert_snapshot!(snapshot_tokens("match"), @r#"
        MatchKw "match" 0..5
        Eof "" 5..5
        "#);
    }

    #[test]
    fn snap_keyword_if() {
        insta::assert_snapshot!(snapshot_tokens("if"), @r#"
        IfKw "if" 0..2
        Eof "" 2..2
        "#);
    }

    #[test]
    fn snap_keyword_else() {
        insta::assert_snapshot!(snapshot_tokens("else"), @r#"
        ElseKw "else" 0..4
        Eof "" 4..4
        "#);
    }

    #[test]
    fn snap_keyword_import() {
        insta::assert_snapshot!(snapshot_tokens("import"), @r#"
        ImportKw "import" 0..6
        Eof "" 6..6
        "#);
    }

    #[test]
    fn snap_keyword_from() {
        insta::assert_snapshot!(snapshot_tokens("from"), @r#"
        FromKw "from" 0..4
        Eof "" 4..4
        "#);
    }

    #[test]
    fn snap_keyword_as() {
        insta::assert_snapshot!(snapshot_tokens("as"), @r#"
        AsKw "as" 0..2
        Eof "" 2..2
        "#);
    }

    #[test]
    fn snap_bool_true() {
        insta::assert_snapshot!(snapshot_tokens("True"), @r#"
        TrueKw "True" 0..4
        Eof "" 4..4
        "#);
    }

    #[test]
    fn snap_bool_false() {
        insta::assert_snapshot!(snapshot_tokens("False"), @r#"
        FalseKw "False" 0..5
        Eof "" 5..5
        "#);
    }

    // --- Literals ---

    #[test]
    fn snap_int_literal() {
        insta::assert_snapshot!(snapshot_tokens("42"), @r#"
        IntLit "42" 0..2
        Eof "" 2..2
        "#);
    }

    #[test]
    fn snap_float_literal() {
        insta::assert_snapshot!(snapshot_tokens("3.14"), @r#"
        FloatLit "3.14" 0..4
        Eof "" 4..4
        "#);
    }

    #[test]
    fn snap_float_vs_int() {
        insta::assert_snapshot!(snapshot_tokens("1 3.14 42"), @r#"
        IntLit "1" 0..1
        Whitespace " " 1..2
        FloatLit "3.14" 2..6
        Whitespace " " 6..7
        IntLit "42" 7..9
        Eof "" 9..9
        "#);
    }

    #[test]
    fn snap_string_literal() {
        insta::assert_snapshot!(snapshot_tokens(r#""hello world""#), @r#"
        StringLit "\"hello world\"" 0..13
        Eof "" 13..13
        "#);
    }

    // --- Identifiers & keywords ---

    #[test]
    fn snap_identifier() {
        insta::assert_snapshot!(snapshot_tokens("foo_bar"), @r#"
        Ident "foo_bar" 0..7
        Eof "" 7..7
        "#);
    }

    #[test]
    fn snap_keyword_prefix_not_keyword() {
        insta::assert_snapshot!(snapshot_tokens("letter"), @r#"
        Ident "letter" 0..6
        Eof "" 6..6
        "#);
    }

    #[test]
    fn snap_underscore_vs_ident() {
        insta::assert_snapshot!(snapshot_tokens("_ _foo"), @r#"
        Underscore "_" 0..1
        Whitespace " " 1..2
        Ident "_foo" 2..6
        Eof "" 6..6
        "#);
    }

    // --- Operators ---

    #[test]
    fn snap_all_operators() {
        insta::assert_snapshot!(snapshot_tokens("+ - * / % = == != < <= > >= ! & |"), @r#"
        Plus "+" 0..1
        Whitespace " " 1..2
        Minus "-" 2..3
        Whitespace " " 3..4
        Star "*" 4..5
        Whitespace " " 5..6
        Slash "/" 6..7
        Whitespace " " 7..8
        Percent "%" 8..9
        Whitespace " " 9..10
        Eq "=" 10..11
        Whitespace " " 11..12
        EqEq "==" 12..14
        Whitespace " " 14..15
        BangEq "!=" 15..17
        Whitespace " " 17..18
        Lt "<" 18..19
        Whitespace " " 19..20
        LtEq "<=" 20..22
        Whitespace " " 22..23
        Gt ">" 23..24
        Whitespace " " 24..25
        GtEq ">=" 25..27
        Whitespace " " 27..28
        Bang "!" 28..29
        Whitespace " " 29..30
        Ampersand "&" 30..31
        Whitespace " " 31..32
        PipeSingle "|" 32..33
        Eof "" 33..33
        "#);
    }

    #[test]
    fn snap_multi_char_operators() {
        insta::assert_snapshot!(snapshot_tokens("|> => <> .."), @r#"
        Pipe "|>" 0..2
        Whitespace " " 2..3
        FatArrow "=>" 3..5
        Whitespace " " 5..6
        StringConcat "<>" 6..8
        Whitespace " " 8..9
        DotDot ".." 9..11
        Eof "" 11..11
        "#);
    }

    // --- Delimiters ---

    #[test]
    fn snap_all_delimiters() {
        insta::assert_snapshot!(snapshot_tokens("( ) { } [ ] , : ; . ->"), @r#"
        LParen "(" 0..1
        Whitespace " " 1..2
        RParen ")" 2..3
        Whitespace " " 3..4
        LBrace "{" 4..5
        Whitespace " " 5..6
        RBrace "}" 6..7
        Whitespace " " 7..8
        LBracket "[" 8..9
        Whitespace " " 9..10
        RBracket "]" 10..11
        Whitespace " " 11..12
        Comma "," 12..13
        Whitespace " " 13..14
        Colon ":" 14..15
        Whitespace " " 15..16
        Semicolon ";" 16..17
        Whitespace " " 17..18
        Dot "." 18..19
        Whitespace " " 19..20
        Arrow "->" 20..22
        Eof "" 22..22
        "#);
    }

    // --- Disambiguation ---

    #[test]
    fn snap_arrow_vs_minus() {
        insta::assert_snapshot!(snapshot_tokens("-> -"), @r#"
        Arrow "->" 0..2
        Whitespace " " 2..3
        Minus "-" 3..4
        Eof "" 4..4
        "#);
    }

    #[test]
    fn snap_pipe_vs_pipe_single() {
        insta::assert_snapshot!(snapshot_tokens("|> |"), @r#"
        Pipe "|>" 0..2
        Whitespace " " 2..3
        PipeSingle "|" 3..4
        Eof "" 4..4
        "#);
    }

    #[test]
    fn snap_lt_vs_string_concat() {
        insta::assert_snapshot!(snapshot_tokens("< <>"), @r#"
        Lt "<" 0..1
        Whitespace " " 1..2
        StringConcat "<>" 2..4
        Eof "" 4..4
        "#);
    }

    #[test]
    fn snap_dot_vs_dotdot() {
        insta::assert_snapshot!(snapshot_tokens(". .."), @r#"
        Dot "." 0..1
        Whitespace " " 1..2
        DotDot ".." 2..4
        Eof "" 4..4
        "#);
    }

    #[test]
    fn snap_eq_vs_eqeq_vs_fat_arrow() {
        insta::assert_snapshot!(snapshot_tokens("= == =>"), @r#"
        Eq "=" 0..1
        Whitespace " " 1..2
        EqEq "==" 2..4
        Whitespace " " 4..5
        FatArrow "=>" 5..7
        Eof "" 7..7
        "#);
    }

    // --- Trivia ---

    #[test]
    fn snap_trivia_whitespace() {
        insta::assert_snapshot!(snapshot_tokens("fn  main"), @r#"
        FnKw "fn" 0..2
        Whitespace "  " 2..4
        Ident "main" 4..8
        Eof "" 8..8
        "#);
    }

    #[test]
    fn snap_trivia_newlines() {
        insta::assert_snapshot!(snapshot_tokens("a\nb\nc"), @r#"
        Ident "a" 0..1
        Newline "\n" 1..2
        Ident "b" 2..3
        Newline "\n" 3..4
        Ident "c" 4..5
        Eof "" 5..5
        "#);
    }

    #[test]
    fn snap_trivia_comment() {
        insta::assert_snapshot!(snapshot_tokens("// comment\nfn"), @r#"
        Comment "// comment" 0..10
        Newline "\n" 10..11
        FnKw "fn" 11..13
        Eof "" 13..13
        "#);
    }

    #[test]
    fn snap_comment_at_eof() {
        insta::assert_snapshot!(snapshot_tokens("fn // trailing"), @r#"
        FnKw "fn" 0..2
        Whitespace " " 2..3
        Comment "// trailing" 3..14
        Eof "" 14..14
        "#);
    }

    // --- Error recovery ---

    #[test]
    fn snap_invalid_token() {
        let (_, diags) = lex("@", FID);
        assert_eq!(diags.len(), 1);
        insta::assert_snapshot!(snapshot_tokens("@"), @r#"
        Error "@" 0..1
        Eof "" 1..1
        "#);
    }

    #[test]
    fn snap_invalid_mixed() {
        let (_, diags) = lex("let @ x", FID);
        assert_eq!(diags.len(), 1);
        insta::assert_snapshot!(snapshot_tokens("let @ x"), @r#"
        LetKw "let" 0..3
        Whitespace " " 3..4
        Error "@" 4..5
        Whitespace " " 5..6
        Ident "x" 6..7
        Eof "" 7..7
        "#);
    }

    // --- Realistic programs ---

    #[test]
    fn snap_let_binding() {
        insta::assert_snapshot!(snapshot_tokens("let x = 42"), @r#"
        LetKw "let" 0..3
        Whitespace " " 3..4
        Ident "x" 4..5
        Whitespace " " 5..6
        Eq "=" 6..7
        Whitespace " " 7..8
        IntLit "42" 8..10
        Eof "" 10..10
        "#);
    }

    #[test]
    fn snap_fn_definition_full() {
        insta::assert_snapshot!(snapshot_tokens("pub fn add(a: Int, b: Int) -> Int { a }"), @r#"
        PubKw "pub" 0..3
        Whitespace " " 3..4
        FnKw "fn" 4..6
        Whitespace " " 6..7
        Ident "add" 7..10
        LParen "(" 10..11
        Ident "a" 11..12
        Colon ":" 12..13
        Whitespace " " 13..14
        Ident "Int" 14..17
        Comma "," 17..18
        Whitespace " " 18..19
        Ident "b" 19..20
        Colon ":" 20..21
        Whitespace " " 21..22
        Ident "Int" 22..25
        RParen ")" 25..26
        Whitespace " " 26..27
        Arrow "->" 27..29
        Whitespace " " 29..30
        Ident "Int" 30..33
        Whitespace " " 33..34
        LBrace "{" 34..35
        Whitespace " " 35..36
        Ident "a" 36..37
        Whitespace " " 37..38
        RBrace "}" 38..39
        Eof "" 39..39
        "#);
    }

    #[test]
    fn snap_match_expression() {
        insta::assert_snapshot!(snapshot_tokens("match x { 1 => True }"), @r#"
        MatchKw "match" 0..5
        Whitespace " " 5..6
        Ident "x" 6..7
        Whitespace " " 7..8
        LBrace "{" 8..9
        Whitespace " " 9..10
        IntLit "1" 10..11
        Whitespace " " 11..12
        FatArrow "=>" 12..14
        Whitespace " " 14..15
        TrueKw "True" 15..19
        Whitespace " " 19..20
        RBrace "}" 20..21
        Eof "" 21..21
        "#);
    }

    #[test]
    fn snap_pipeline() {
        insta::assert_snapshot!(snapshot_tokens("x |> f |> g"), @r#"
        Ident "x" 0..1
        Whitespace " " 1..2
        Pipe "|>" 2..4
        Whitespace " " 4..5
        Ident "f" 5..6
        Whitespace " " 6..7
        Pipe "|>" 7..9
        Whitespace " " 9..10
        Ident "g" 10..11
        Eof "" 11..11
        "#);
    }

    #[test]
    fn snap_import_statement() {
        insta::assert_snapshot!(snapshot_tokens("import foo from bar as baz"), @r#"
        ImportKw "import" 0..6
        Whitespace " " 6..7
        Ident "foo" 7..10
        Whitespace " " 10..11
        FromKw "from" 11..15
        Whitespace " " 15..16
        Ident "bar" 16..19
        Whitespace " " 19..20
        AsKw "as" 20..22
        Whitespace " " 22..23
        Ident "baz" 23..26
        Eof "" 26..26
        "#);
    }

    #[test]
    fn snap_span_correctness() {
        insta::assert_snapshot!(snapshot_tokens("let x"), @r#"
        LetKw "let" 0..3
        Whitespace " " 3..4
        Ident "x" 4..5
        Eof "" 5..5
        "#);
    }

    #[test]
    fn snap_multiline_program() {
        let source = "pub fn main() -> Int {\n  let x = 1\n  x\n}";
        insta::assert_snapshot!(snapshot_tokens(source), @r#"
        PubKw "pub" 0..3
        Whitespace " " 3..4
        FnKw "fn" 4..6
        Whitespace " " 6..7
        Ident "main" 7..11
        LParen "(" 11..12
        RParen ")" 12..13
        Whitespace " " 13..14
        Arrow "->" 14..16
        Whitespace " " 16..17
        Ident "Int" 17..20
        Whitespace " " 20..21
        LBrace "{" 21..22
        Newline "\n" 22..23
        Whitespace "  " 23..25
        LetKw "let" 25..28
        Whitespace " " 28..29
        Ident "x" 29..30
        Whitespace " " 30..31
        Eq "=" 31..32
        Whitespace " " 32..33
        IntLit "1" 33..34
        Newline "\n" 34..35
        Whitespace "  " 35..37
        Ident "x" 37..38
        Newline "\n" 38..39
        RBrace "}" 39..40
        Eof "" 40..40
        "#);
    }

    #[test]
    fn snap_if_else() {
        insta::assert_snapshot!(snapshot_tokens("if x { True } else { False }"), @r#"
        IfKw "if" 0..2
        Whitespace " " 2..3
        Ident "x" 3..4
        Whitespace " " 4..5
        LBrace "{" 5..6
        Whitespace " " 6..7
        TrueKw "True" 7..11
        Whitespace " " 11..12
        RBrace "}" 12..13
        Whitespace " " 13..14
        ElseKw "else" 14..18
        Whitespace " " 18..19
        LBrace "{" 19..20
        Whitespace " " 20..21
        FalseKw "False" 21..26
        Whitespace " " 26..27
        RBrace "}" 27..28
        Eof "" 28..28
        "#);
    }

    #[test]
    fn snap_list_and_brackets() {
        insta::assert_snapshot!(snapshot_tokens("[1, 2, 3]"), @r#"
        LBracket "[" 0..1
        IntLit "1" 1..2
        Comma "," 2..3
        Whitespace " " 3..4
        IntLit "2" 4..5
        Comma "," 5..6
        Whitespace " " 6..7
        IntLit "3" 7..8
        RBracket "]" 8..9
        Eof "" 9..9
        "#);
    }

    #[test]
    fn snap_string_concat() {
        insta::assert_snapshot!(snapshot_tokens(r#""a" <> "b""#), @r#"
        StringLit "\"a\"" 0..3
        Whitespace " " 3..4
        StringConcat "<>" 4..6
        Whitespace " " 6..7
        StringLit "\"b\"" 7..10
        Eof "" 10..10
        "#);
    }

    #[test]
    fn snap_type_def() {
        insta::assert_snapshot!(snapshot_tokens("pub type Option { Some(a) None }"), @r#"
        PubKw "pub" 0..3
        Whitespace " " 3..4
        TypeKw "type" 4..8
        Whitespace " " 8..9
        Ident "Option" 9..15
        Whitespace " " 15..16
        LBrace "{" 16..17
        Whitespace " " 17..18
        Ident "Some" 18..22
        LParen "(" 22..23
        Ident "a" 23..24
        RParen ")" 24..25
        Whitespace " " 25..26
        Ident "None" 26..30
        Whitespace " " 30..31
        RBrace "}" 31..32
        Eof "" 32..32
        "#);
    }

    #[test]
    fn snap_logical_operators() {
        insta::assert_snapshot!(snapshot_tokens("&& ||"), @r#"
        AmpAmp "&&" 0..2
        Whitespace " " 2..3
        PipePipe "||" 3..5
        Eof "" 5..5
        "#);
    }

    #[test]
    fn snap_ampamp_vs_ampersand() {
        insta::assert_snapshot!(snapshot_tokens("&& &"), @r#"
        AmpAmp "&&" 0..2
        Whitespace " " 2..3
        Ampersand "&" 3..4
        Eof "" 4..4
        "#);
    }

    #[test]
    fn snap_pipepipe_vs_pipe() {
        insta::assert_snapshot!(snapshot_tokens("|| | |>"), @r#"
        PipePipe "||" 0..2
        Whitespace " " 2..3
        PipeSingle "|" 3..4
        Whitespace " " 4..5
        Pipe "|>" 5..7
        Eof "" 7..7
        "#);
    }
}
