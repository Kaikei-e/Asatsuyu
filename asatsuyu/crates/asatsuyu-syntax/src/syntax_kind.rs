/// All syntactic element kinds for the Asatsuyu language.
///
/// This enum covers both tokens (lexical elements) and nodes (syntax tree elements).
/// It maps 1:1 to rowan's `SyntaxKind(u16)` via the `#[repr(u16)]` layout.
///
/// The rowan `Language` impl lives in `asatsuyu-parser` to keep this crate
/// dependency-free.
///
/// **Layout invariant**: all token kinds must appear before [`Eof`](Self::Eof),
/// and all node kinds after it. [`is_token()`](Self::is_token) relies on this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum SyntaxKind {
    // === Tokens: Keywords ===
    FnKw,
    PubKw,
    LetKw,
    TypeKw,
    MatchKw,
    IfKw,
    ElseKw,
    ImportKw,
    FromKw,
    PythonKw,
    AsKw,
    TrueKw,
    FalseKw,
    TryKw,
    /// Mutable binding modifier (Phase 3-1: scoped mutability).
    MutKw,
    /// Keyword: `async` (async function modifier).
    AsyncKw,
    /// Keyword: `await` (await expression prefix).
    AwaitKw,

    // === Tokens: Literals ===
    IntLit,
    FloatLit,
    StringLit,

    // === Tokens: Identifiers ===
    Ident,

    // === Tokens: Operators ===
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `*`
    Star,
    /// `/`
    Slash,
    /// `%`
    Percent,
    /// `=`
    Eq,
    /// `==`
    EqEq,
    /// `!=`
    BangEq,
    /// `<`
    Lt,
    /// `<=`
    LtEq,
    /// `>`
    Gt,
    /// `>=`
    GtEq,
    /// `!`
    Bang,
    /// `&`
    Ampersand,
    /// `|`
    PipeSingle,
    /// `&&`
    AmpAmp,
    /// `||`
    PipePipe,
    /// `|>`
    Pipe,
    /// `<>`
    StringConcat,

    // === Tokens: Delimiters & Punctuation ===
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `{`
    LBrace,
    /// `}`
    RBrace,
    /// `[`
    LBracket,
    /// `]`
    RBracket,
    /// `,`
    Comma,
    /// `:`
    Colon,
    /// `;`
    Semicolon,
    /// `.`
    Dot,
    /// `..`
    DotDot,
    /// `->`
    Arrow,
    /// `_`
    Underscore,

    // === Tokens: Trivia ===
    Whitespace,
    Newline,
    Comment,

    // === Tokens: Special ===
    /// Invalid or unrecognized token.
    Error,
    /// End of file.
    Eof,

    // ─── Node kinds (after Eof) ─────────────────────────────────────

    // === Nodes: Top-level ===
    /// Root node of a source file.
    SourceFile,
    /// Function definition: `pub fn name(params) -> ReturnType { body }`
    FnDef,
    /// Type definition: `type Name { Variant1, Variant2 }`
    TypeDef,
    /// Import statement: `import foo.bar`
    ImportStmt,
    /// Python FFI import: `from python import pathlib`
    FromPythonImportStmt,
    /// Let binding: `let x = expr` or `let mut x = expr`
    LetStmt,
    /// Assignment statement: `x = expr` (reassignment of mutable binding)
    AssignStmt,

    // === Nodes: Expressions ===
    /// Literal expression: `42`, `"hello"`, `True`
    LiteralExpr,
    /// Identifier expression: `foo`
    IdentExpr,
    /// Function call: `f(a, b)`
    CallExpr,
    /// Pipeline expression: `x |> f |> g`
    PipelineExpr,
    /// Match expression: `match x { ... }`
    MatchExpr,
    /// If expression: `if cond { ... } else { ... }`
    IfExpr,
    /// Lambda expression: `fn(x) { x + 1 }`
    LambdaExpr,
    /// Block expression: `{ expr1; expr2 }`
    BlockExpr,
    /// Binary expression: `a + b`
    BinaryExpr,
    /// Unary expression: `!x`
    UnaryExpr,
    /// Field access: `record.field`
    FieldAccessExpr,
    /// List literal: `[1, 2, 3]`
    ListExpr,
    /// Tuple literal: `#(1, 2)`
    TupleExpr,
    /// Record literal: `Name { field: value }`
    RecordExpr,
    /// Parenthesized expression: `(expr)`
    ParenExpr,
    /// Try expression: `try expr`
    TryExpr,
    /// Await expression: `await expr`
    AwaitExpr,

    // === Nodes: Patterns ===
    /// Wildcard pattern: `_`
    WildcardPat,
    /// Identifier pattern: `x`
    IdentPat,
    /// Literal pattern: `42`, `"hello"`
    LiteralPat,
    /// Constructor pattern: `Some(x)`
    ConstructorPat,
    /// List pattern: `[x, ..rest]`
    ListPat,
    /// Tuple pattern: `#(a, b)`
    TuplePat,

    // === Nodes: Types ===
    /// Type expression: `Int`, `List(Int)`
    TypeExpr,
    /// Type parameter: `a` in `type Box(a)`
    TypeParam,

    // === Nodes: ADT ===
    /// ADT variant: `Some(Int)`
    Variant,
    /// Record field: `name: Type`
    Field,

    // === Nodes: Match ===
    /// Match arm: `pattern -> expr`
    MatchArm,
    /// Guard clause: `if condition`
    Guard,

    // === Nodes: Parameters ===
    /// Function parameter: `name: Type`
    Param,
    /// Parameter list: `(param1, param2)`
    ParamList,
    /// Argument list: `(arg1, arg2)`
    ArgList,

    // === Nodes: Other ===
    /// Return type annotation: `-> Type`
    ReturnType,
    /// Visibility modifier: `pub`
    Visibility,
    /// Qualified path: `module.name`
    Path,
    /// Error node inserted by parser recovery.
    NodeError,

    #[doc(hidden)]
    __LAST,
}

impl SyntaxKind {
    /// Returns `true` if this kind represents trivia (whitespace, newlines, comments).
    #[inline]
    #[must_use]
    pub fn is_trivia(self) -> bool {
        matches!(self, Self::Whitespace | Self::Newline | Self::Comment)
    }

    /// Returns `true` if this kind represents a keyword token.
    #[inline]
    #[must_use]
    pub fn is_keyword(self) -> bool {
        crate::keyword::class_of(self).is_some()
    }

    /// Returns `true` if this is a hard keyword (always reserved, never an identifier).
    #[inline]
    #[must_use]
    pub fn is_hard_keyword(self) -> bool {
        matches!(crate::keyword::class_of(self), Some(crate::keyword::KeywordClass::Hard))
    }

    /// Returns `true` if this is a literal keyword (`True`, `False`).
    #[inline]
    #[must_use]
    pub fn is_literal_keyword(self) -> bool {
        matches!(crate::keyword::class_of(self), Some(crate::keyword::KeywordClass::Literal))
    }

    /// Returns `true` if this is a contextual keyword (`python`, `as`).
    #[inline]
    #[must_use]
    pub fn is_contextual_keyword(self) -> bool {
        matches!(crate::keyword::class_of(self), Some(crate::keyword::KeywordClass::Contextual))
    }

    /// Returns `true` if this is a reserved keyword (`mut`, `async`, `await`).
    #[inline]
    #[must_use]
    pub fn is_reserved_keyword(self) -> bool {
        matches!(crate::keyword::class_of(self), Some(crate::keyword::KeywordClass::Reserved))
    }

    /// Returns the source text for this keyword kind, or `None` if not a keyword.
    #[inline]
    #[must_use]
    pub fn keyword_text(self) -> Option<&'static str> {
        crate::keyword::text_of(self)
    }

    /// Returns `true` if this kind represents a token (as opposed to a node).
    #[inline]
    #[must_use]
    pub fn is_token(self) -> bool {
        (self as u16) <= (Self::Eof as u16)
    }

    /// Returns `true` if this kind represents a syntax tree node.
    #[inline]
    #[must_use]
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
        let keywords = [
            SyntaxKind::FnKw,
            SyntaxKind::PubKw,
            SyntaxKind::LetKw,
            SyntaxKind::TypeKw,
            SyntaxKind::MatchKw,
            SyntaxKind::IfKw,
            SyntaxKind::ElseKw,
            SyntaxKind::ImportKw,
            SyntaxKind::FromKw,
            SyntaxKind::PythonKw,
            SyntaxKind::AsKw,
            SyntaxKind::TrueKw,
            SyntaxKind::FalseKw,
            SyntaxKind::TryKw,
            SyntaxKind::MutKw,
            SyntaxKind::AsyncKw,
            SyntaxKind::AwaitKw,
        ];
        for kw in keywords {
            assert!(kw.is_keyword(), "{kw:?} should be a keyword");
        }
        assert!(!SyntaxKind::Ident.is_keyword());
        assert!(!SyntaxKind::LParen.is_keyword());
        assert!(!SyntaxKind::Plus.is_keyword());
    }

    #[test]
    fn token_vs_node_classification() {
        // Tokens
        assert!(SyntaxKind::FnKw.is_token());
        assert!(SyntaxKind::LetKw.is_token());
        assert!(SyntaxKind::Ident.is_token());
        assert!(SyntaxKind::LParen.is_token());
        assert!(SyntaxKind::Plus.is_token());
        assert!(SyntaxKind::Pipe.is_token());
        assert!(SyntaxKind::AmpAmp.is_token());
        assert!(SyntaxKind::PipePipe.is_token());
        assert!(SyntaxKind::FloatLit.is_token());
        assert!(SyntaxKind::Eof.is_token());
        assert!(SyntaxKind::Error.is_token());
        assert!(!SyntaxKind::FnKw.is_node());

        // Nodes
        assert!(SyntaxKind::SourceFile.is_node());
        assert!(SyntaxKind::FnDef.is_node());
        assert!(SyntaxKind::TypeDef.is_node());
        assert!(SyntaxKind::MatchExpr.is_node());
        assert!(SyntaxKind::BlockExpr.is_node());
        assert!(SyntaxKind::WildcardPat.is_node());
        assert!(SyntaxKind::MatchArm.is_node());
        assert!(SyntaxKind::ParenExpr.is_node());
        assert!(SyntaxKind::NodeError.is_node());
        assert!(!SyntaxKind::SourceFile.is_token());

        // __LAST is neither
        assert!(!SyntaxKind::__LAST.is_node());
    }

    #[test]
    fn keyword_class_methods() {
        // Hard
        assert!(SyntaxKind::FnKw.is_hard_keyword());
        assert!(SyntaxKind::LetKw.is_hard_keyword());
        assert!(SyntaxKind::TryKw.is_hard_keyword());
        assert!(!SyntaxKind::TrueKw.is_hard_keyword());
        assert!(!SyntaxKind::AsKw.is_hard_keyword());
        assert!(SyntaxKind::MutKw.is_hard_keyword());

        // Literal
        assert!(SyntaxKind::TrueKw.is_literal_keyword());
        assert!(SyntaxKind::FalseKw.is_literal_keyword());
        assert!(!SyntaxKind::FnKw.is_literal_keyword());

        // Contextual
        assert!(SyntaxKind::PythonKw.is_contextual_keyword());
        assert!(SyntaxKind::AsKw.is_contextual_keyword());
        assert!(!SyntaxKind::FnKw.is_contextual_keyword());

        // Reserved (none remaining — async/await promoted to Hard in Phase 3-2)
        assert!(!SyntaxKind::MutKw.is_reserved_keyword());
        assert!(!SyntaxKind::AsyncKw.is_reserved_keyword());
        assert!(!SyntaxKind::AwaitKw.is_reserved_keyword());
        assert!(!SyntaxKind::FnKw.is_reserved_keyword());

        // Non-keywords
        assert!(!SyntaxKind::Ident.is_hard_keyword());
        assert!(!SyntaxKind::Ident.is_literal_keyword());
        assert!(!SyntaxKind::Ident.is_contextual_keyword());
        assert!(!SyntaxKind::Ident.is_reserved_keyword());
    }

    #[test]
    fn keyword_text_method() {
        assert_eq!(SyntaxKind::FnKw.keyword_text(), Some("fn"));
        assert_eq!(SyntaxKind::TrueKw.keyword_text(), Some("True"));
        assert_eq!(SyntaxKind::PythonKw.keyword_text(), Some("python"));
        assert_eq!(SyntaxKind::MutKw.keyword_text(), Some("mut"));
        assert_eq!(SyntaxKind::Ident.keyword_text(), None);
        assert_eq!(SyntaxKind::Plus.keyword_text(), None);
    }

    #[test]
    fn is_keyword_consistent_with_table() {
        // Every SyntaxKind that is_keyword() returns true for must appear
        // in the KEYWORDS table, and vice versa.
        for raw in 0..SyntaxKind::__LAST as u16 {
            let kind = SyntaxKind::from(raw);
            let in_table = crate::keyword::class_of(kind).is_some();
            assert_eq!(
                kind.is_keyword(),
                in_table,
                "{kind:?}: is_keyword()={} but in_table={in_table}",
                kind.is_keyword()
            );
        }
    }
}
