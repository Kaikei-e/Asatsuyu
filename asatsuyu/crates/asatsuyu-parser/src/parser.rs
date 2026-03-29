//! Core parser infrastructure: token navigation, trivia handling, and error recovery.

use asatsuyu_lexer::Token;
use asatsuyu_syntax::{Diagnostic, FileId, Span, SyntaxKind};
use rowan::{GreenNode, GreenNodeBuilder};

use crate::language::raw;

// ── TokenSet ────────────────────────────────────────────────────────

/// A compact set of [`SyntaxKind`] values, implemented as a `u128` bitset.
///
/// Used to pass context-specific recovery / follow-token sets into grammar
/// rules without heap allocation. The `SyntaxKind` enum has ~80 variants,
/// well within the 128-bit capacity.
#[derive(Clone, Copy)]
pub(crate) struct TokenSet(u128);

impl TokenSet {
    pub(crate) const EMPTY: Self = Self(0);

    /// Build a set from a slice of kinds. Usable in `const` context.
    pub(crate) const fn new(kinds: &[SyntaxKind]) -> Self {
        let mut bits = 0u128;
        let mut i = 0;
        while i < kinds.len() {
            bits |= 1u128 << (kinds[i] as u16);
            i += 1;
        }
        Self(bits)
    }

    pub(crate) const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub(crate) const fn contains(self, kind: SyntaxKind) -> bool {
        self.0 & (1u128 << (kind as u16)) != 0
    }
}

/// Top-level sync points: items that start a new definition.
pub(crate) const TOP_LEVEL_RECOVERY: TokenSet = TokenSet::new(&[
    SyntaxKind::FnKw,
    SyntaxKind::PubKw,
    SyntaxKind::TypeKw,
    SyntaxKind::LetKw,
    SyntaxKind::ImportKw,
]);

/// Closing delimiters — **never** consumed during error recovery.
pub(crate) const CLOSING_DELIMITERS: TokenSet =
    TokenSet::new(&[SyntaxKind::RParen, SyntaxKind::RBrace, SyntaxKind::RBracket]);

/// Recursive descent parser that builds a rowan green tree from a token stream.
///
/// This struct is `pub(crate)` — the public entry point is [`crate::parse()`].
pub(crate) struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    builder: GreenNodeBuilder<'static>,
    diagnostics: Vec<Diagnostic>,
    file_id: FileId,
}

impl<'a> Parser<'a> {
    /// Create a new parser over the given token stream.
    pub(crate) fn new(tokens: &'a [Token], file_id: FileId) -> Self {
        Self { tokens, pos: 0, builder: GreenNodeBuilder::new(), diagnostics: Vec::new(), file_id }
    }

    // ── Peek / query ─────────────────────────────────────────────────

    /// Returns the kind of the current non-trivia token **without consuming** anything.
    pub(crate) fn current(&self) -> SyntaxKind {
        self.nth(0)
    }

    /// Returns the kind of the `n`-th non-trivia token ahead (0-based) without consuming.
    pub(crate) fn nth(&self, n: usize) -> SyntaxKind {
        let mut i = self.pos;
        let mut remaining = n;
        while i < self.tokens.len() {
            if !self.tokens[i].kind.is_trivia() {
                if remaining == 0 {
                    return self.tokens[i].kind;
                }
                remaining -= 1;
            }
            i += 1;
        }
        SyntaxKind::Eof
    }

    /// Returns `true` if the current non-trivia token is `kind`.
    pub(crate) fn at(&self, kind: SyntaxKind) -> bool {
        self.current() == kind
    }

    /// Returns `true` if the current token is in the given set.
    pub(crate) fn at_any(&self, set: TokenSet) -> bool {
        set.contains(self.current())
    }

    /// Returns `true` if the parser has reached the end of input.
    pub(crate) fn at_eof(&self) -> bool {
        self.current() == SyntaxKind::Eof
    }

    /// Returns the current raw token position. Used by callers to assert
    /// that parsing loops make progress.
    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    /// Returns the span of the current non-trivia token.
    pub(crate) fn current_span(&self) -> Span {
        let mut i = self.pos;
        while i < self.tokens.len() && self.tokens[i].kind.is_trivia() {
            i += 1;
        }
        if i < self.tokens.len() {
            self.tokens[i].span
        } else {
            // Past the end — return a zero-width span at file end.
            let last = self.tokens.last().expect("token stream always has Eof");
            Span::new(self.file_id, last.span.end, last.span.end)
        }
    }

    // ── Consume ──────────────────────────────────────────────────────

    /// Consume all consecutive trivia tokens at the current position,
    /// adding each to the green tree at the current nesting level.
    pub(crate) fn eat_trivia(&mut self) {
        while self.pos < self.tokens.len() && self.tokens[self.pos].kind.is_trivia() {
            let token = &self.tokens[self.pos];
            self.builder.token(raw(token.kind), &token.text);
            self.pos += 1;
        }
    }

    /// Consume the next non-trivia token: eat leading trivia, then add the
    /// token itself to the green tree.
    ///
    /// # Panics
    ///
    /// Panics if called at EOF.
    pub(crate) fn bump(&mut self) {
        self.eat_trivia();
        assert!(
            self.pos < self.tokens.len() && self.tokens[self.pos].kind != SyntaxKind::Eof,
            "bump() called at EOF",
        );
        let token = &self.tokens[self.pos];
        self.builder.token(raw(token.kind), &token.text);
        self.pos += 1;
    }

    /// If the current token matches `kind`, consume it and return `true`.
    /// Otherwise, emit a diagnostic and return `false` (the token is **not** consumed).
    pub(crate) fn expect(&mut self, kind: SyntaxKind) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            let span = self.current_span();
            self.diagnostics.push(
                Diagnostic::error(format!("expected {kind:?}"), span)
                    .with_label(span, format!("expected {kind:?}")),
            );
            false
        }
    }

    // ── Error recovery ───────────────────────────────────────────────

    /// Wrap the current token in a `NodeError` node and emit a diagnostic.
    pub(crate) fn error_and_bump(&mut self, message: &str) {
        let span = self.current_span();
        self.diagnostics.push(Diagnostic::error(message, span).with_label(span, message));
        self.builder.start_node(raw(SyntaxKind::NodeError));
        self.bump();
        self.builder.finish_node();
    }

    /// Skip tokens until a synchronization point (`fn`, `pub`, or EOF),
    /// wrapping everything skipped in a `NodeError` node.
    pub(crate) fn error_recover(&mut self, message: &str) {
        let span = self.current_span();
        self.diagnostics.push(Diagnostic::error(message, span).with_label(span, message));
        self.builder.start_node(raw(SyntaxKind::NodeError));
        while !self.at_eof() && !self.at_recovery_point() {
            self.bump();
        }
        self.eat_trivia();
        self.builder.finish_node();
    }

    /// Skip tokens until one from `recovery` (or a closing delimiter, or a
    /// top-level keyword, or EOF) is reached, wrapping skipped tokens in a
    /// `NodeError` node.
    ///
    /// If the current token is already in the combined stop set, the
    /// diagnostic is emitted but **nothing is consumed** — this lets the
    /// caller handle the token (e.g. a closing `}` that belongs to an
    /// outer scope).
    pub(crate) fn error_recover_until(&mut self, message: &str, recovery: TokenSet) {
        let span = self.current_span();
        self.diagnostics.push(Diagnostic::error(message, span).with_label(span, message));

        let stop = recovery.union(CLOSING_DELIMITERS).union(TOP_LEVEL_RECOVERY);

        // Already at a sync point — emit diagnostic only, no NodeError.
        if self.at_eof() || self.at_any(stop) {
            return;
        }

        self.builder.start_node(raw(SyntaxKind::NodeError));
        while !self.at_eof() && !self.at_any(stop) {
            self.bump();
        }
        self.eat_trivia();
        self.builder.finish_node();
    }

    /// Returns `true` if the current token is a recovery synchronization point.
    fn at_recovery_point(&self) -> bool {
        self.at_any(TOP_LEVEL_RECOVERY)
    }

    // ── Diagnostics access ─────────────────────────────────────────────

    /// Mutable access to the diagnostics list (for grammar rules that need
    /// to emit diagnostics directly).
    pub(crate) fn diagnostics_mut(&mut self) -> &mut Vec<Diagnostic> {
        &mut self.diagnostics
    }

    // ── Builder access ───────────────────────────────────────────────

    /// Start a new CST node of the given kind.
    ///
    /// Trivia is **not** consumed here — it is handled by [`bump()`](Self::bump),
    /// which eats leading trivia before each token. This ensures trivia
    /// attaches to the innermost node that contains the following token.
    pub(crate) fn start_node(&mut self, kind: SyntaxKind) {
        self.builder.start_node(raw(kind));
    }

    /// Finish the current CST node.
    pub(crate) fn finish_node(&mut self) {
        self.builder.finish_node();
    }

    /// Finalise parsing and return the green tree plus collected diagnostics.
    pub(crate) fn finish(self) -> (GreenNode, Vec<Diagnostic>) {
        (self.builder.finish(), self.diagnostics)
    }

    // ── Checkpoint ──────────────────────────────────────────────────

    /// Save the current position in the green tree builder.
    ///
    /// Returns a checkpoint that can later be passed to
    /// [`start_node_at()`](Self::start_node_at) to retroactively wrap
    /// previously emitted tokens/nodes in a new parent node. Essential
    /// for Pratt parsing where the left-hand side is parsed before the
    /// operator is known.
    pub(crate) fn checkpoint(&self) -> rowan::Checkpoint {
        self.builder.checkpoint()
    }

    /// Start a new CST node at the given checkpoint, retroactively
    /// wrapping everything emitted since the checkpoint under a node
    /// of the given `kind`.
    pub(crate) fn start_node_at(&mut self, checkpoint: rowan::Checkpoint, kind: SyntaxKind) {
        self.builder.start_node_at(checkpoint, raw(kind));
    }
}
