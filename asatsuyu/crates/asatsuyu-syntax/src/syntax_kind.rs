/// All syntactic element kinds for the Asatsuyu language.
///
/// This enum covers both tokens (lexical elements) and nodes (syntax tree elements).
/// It maps 1:1 to rowan's `SyntaxKind(u16)` via the `#[repr(u16)]` layout.
///
/// The rowan `Language` impl lives in `asatsuyu-parser` to keep this crate
/// dependency-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum SyntaxKind {
    // === Tokens: Keywords ===
    FnKw,
    PubKw,

    // === Tokens: Literals ===
    IntLit,
    StringLit,

    // === Tokens: Identifiers ===
    Ident,

    // === Tokens: Delimiters & Punctuation ===
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `{`
    LBrace,
    /// `}`
    RBrace,
    /// `,`
    Comma,
    /// `:`
    Colon,
    /// `->`
    Arrow,

    // === Tokens: Trivia ===
    Whitespace,
    Newline,
    Comment,

    // === Tokens: Special ===
    /// Invalid or unrecognized token.
    Error,
    /// End of file.
    Eof,

    // === Nodes ===
    /// Root node of a source file.
    SourceFile,
    /// Function definition: `pub fn name(params) -> ReturnType { body }`
    FnDef,
    /// Literal expression: `42`, `"hello"`
    LiteralExpr,
    /// Identifier expression: `foo`
    IdentExpr,
    /// Block expression: `{ expr1; expr2 }`
    BlockExpr,
    /// Function parameter: `name: Type`
    Param,
    /// Parameter list: `(param1, param2)`
    ParamList,
    /// Return type annotation: `-> Type`
    ReturnType,
    /// Visibility modifier: `pub`
    Visibility,
    /// Error node inserted by parser recovery.
    NodeError,

    #[doc(hidden)]
    __LAST,
}

impl SyntaxKind {
    /// Returns `true` if this kind represents trivia (whitespace, newlines, comments).
    #[inline]
    pub fn is_trivia(self) -> bool {
        matches!(self, Self::Whitespace | Self::Newline | Self::Comment)
    }

    /// Returns `true` if this kind represents a keyword token.
    #[inline]
    pub fn is_keyword(self) -> bool {
        matches!(self, Self::FnKw | Self::PubKw)
    }

    /// Returns `true` if this kind represents a token (as opposed to a node).
    #[inline]
    pub fn is_token(self) -> bool {
        (self as u16) <= (Self::Eof as u16)
    }

    /// Returns `true` if this kind represents a syntax tree node.
    #[inline]
    pub fn is_node(self) -> bool {
        !self.is_token() && self != Self::__LAST
    }
}

impl From<SyntaxKind> for u16 {
    #[inline]
    fn from(kind: SyntaxKind) -> u16 {
        kind as u16
    }
}

impl From<u16> for SyntaxKind {
    #[inline]
    #[allow(unsafe_code)]
    fn from(raw: u16) -> SyntaxKind {
        assert!(raw < SyntaxKind::__LAST as u16, "invalid SyntaxKind value: {raw}");
        // SAFETY: SyntaxKind is `#[repr(u16)]` and we verified `raw < __LAST`.
        // This is the same pattern used by rust-analyzer.
        unsafe { std::mem::transmute::<u16, SyntaxKind>(raw) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syntax_kind_u16_roundtrip() {
        for raw in 0..SyntaxKind::__LAST as u16 {
            let kind = SyntaxKind::from(raw);
            assert_eq!(u16::from(kind), raw);
        }
    }

    #[test]
    #[should_panic(expected = "invalid SyntaxKind value")]
    fn syntax_kind_out_of_range() {
        let _ = SyntaxKind::from(u16::MAX);
    }

    #[test]
    fn trivia_classification() {
        assert!(SyntaxKind::Whitespace.is_trivia());
        assert!(SyntaxKind::Newline.is_trivia());
        assert!(SyntaxKind::Comment.is_trivia());
        assert!(!SyntaxKind::Ident.is_trivia());
        assert!(!SyntaxKind::FnKw.is_trivia());
    }

    #[test]
    fn keyword_classification() {
        assert!(SyntaxKind::FnKw.is_keyword());
        assert!(SyntaxKind::PubKw.is_keyword());
        assert!(!SyntaxKind::Ident.is_keyword());
        assert!(!SyntaxKind::LParen.is_keyword());
    }

    #[test]
    fn token_vs_node_classification() {
        // Tokens
        assert!(SyntaxKind::FnKw.is_token());
        assert!(SyntaxKind::Ident.is_token());
        assert!(SyntaxKind::LParen.is_token());
        assert!(SyntaxKind::Eof.is_token());
        assert!(SyntaxKind::Error.is_token());
        assert!(!SyntaxKind::FnKw.is_node());

        // Nodes
        assert!(SyntaxKind::SourceFile.is_node());
        assert!(SyntaxKind::FnDef.is_node());
        assert!(SyntaxKind::BlockExpr.is_node());
        assert!(SyntaxKind::NodeError.is_node());
        assert!(!SyntaxKind::SourceFile.is_token());

        // __LAST is neither
        assert!(!SyntaxKind::__LAST.is_node());
    }
}
