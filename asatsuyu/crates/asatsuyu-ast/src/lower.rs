//! CST → AST lowering.
//!
//! Walks the rowan syntax tree, strips trivia, and produces the typed AST
//! defined in [`crate::types`]. Diagnostics are collected for malformed nodes.

// `into_token()` lives on `rowan::NodeOrToken` which is not a direct dependency.
#![allow(clippy::redundant_closure_for_method_calls)]

use asatsuyu_parser::{SyntaxNode, SyntaxToken};
use asatsuyu_syntax::{Diagnostic, FileId, Span, SyntaxKind};
use smol_str::SmolStr;

use crate::types::{
    Definition, Expr, FnDef, Ident, Literal, LiteralKind, Module, Param, Visibility,
};

// ── Context ─────────────────────────────────────────────────────────

/// Accumulates diagnostics during CST → AST lowering.
pub(crate) struct LowerCtx {
    file_id: FileId,
    diagnostics: Vec<Diagnostic>,
}

impl LowerCtx {
    pub(crate) fn new(file_id: FileId) -> Self {
        Self { file_id, diagnostics: Vec::new() }
    }

    pub(crate) fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    fn push_error(&mut self, message: impl Into<String>, span: Span) {
        self.diagnostics.push(Diagnostic::error(message, span));
    }
}

// ── Span helpers ────────────────────────────────────────────────────

fn span_of(node: &SyntaxNode, file_id: FileId) -> Span {
    let range = node.text_range();
    Span::new(file_id, u32::from(range.start()), u32::from(range.end()))
}

fn span_of_token(token: &SyntaxToken, file_id: FileId) -> Span {
    let range = token.text_range();
    Span::new(file_id, u32::from(range.start()), u32::from(range.end()))
}

// ── Child helpers ───────────────────────────────────────────────────

/// Find the first child node with the given kind.
fn first_child_of_kind(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxNode> {
    node.children().find(|c| c.kind() == kind)
}

/// Find the first token (direct child) with the given kind.
fn first_token_of_kind(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxToken> {
    node.children_with_tokens().filter_map(|el| el.into_token()).find(|t| t.kind() == kind)
}

// ── Lowering ────────────────────────────────────────────────────────

impl LowerCtx {
    // ── SourceFile → Module ─────────────────────────────────────────

    pub(crate) fn lower_source_file(&mut self, root: &SyntaxNode) -> Module {
        debug_assert_eq!(root.kind(), SyntaxKind::SourceFile);

        let mut definitions = Vec::new();

        for child in root.children() {
            match child.kind() {
                SyntaxKind::FnDef => {
                    if let Some(fn_def) = self.lower_fn_def(&child) {
                        definitions.push(Definition::Function(fn_def));
                    }
                }
                SyntaxKind::NodeError => {
                    let span = span_of(&child, self.file_id);
                    self.push_error("unexpected syntax", span);
                }
                _ => {} // trivia / other tokens — skip
            }
        }

        Module { definitions, span: span_of(root, self.file_id) }
    }

    // ── FnDef ───────────────────────────────────────────────────────

    fn lower_fn_def(&mut self, node: &SyntaxNode) -> Option<FnDef> {
        debug_assert_eq!(node.kind(), SyntaxKind::FnDef);

        let visibility = if first_child_of_kind(node, SyntaxKind::Visibility).is_some() {
            Visibility::Public
        } else {
            Visibility::Private
        };

        // Function name: first Ident token that is a direct child of FnDef.
        let name_token = first_token_of_kind(node, SyntaxKind::Ident)?;
        let name = Ident {
            name: SmolStr::from(name_token.text()),
            span: span_of_token(&name_token, self.file_id),
        };

        let params = first_child_of_kind(node, SyntaxKind::ParamList)
            .map(|pl| self.lower_param_list(&pl))
            .unwrap_or_default();

        let return_type = first_child_of_kind(node, SyntaxKind::ReturnType)
            .and_then(|rt| self.lower_return_type(&rt));

        let body = if let Some(block) = first_child_of_kind(node, SyntaxKind::BlockExpr) {
            self.lower_block_expr(&block)
        } else {
            let span = span_of(node, self.file_id);
            self.push_error("missing function body", span);
            Expr::Block { exprs: Vec::new(), span }
        };

        Some(FnDef {
            name,
            visibility,
            params,
            return_type,
            body,
            span: span_of(node, self.file_id),
        })
    }

    // ── ParamList ───────────────────────────────────────────────────

    fn lower_param_list(&mut self, node: &SyntaxNode) -> Vec<Param> {
        debug_assert_eq!(node.kind(), SyntaxKind::ParamList);

        node.children()
            .filter(|c| c.kind() == SyntaxKind::Param)
            .filter_map(|c| self.lower_param(&c))
            .collect()
    }

    // ── Param ───────────────────────────────────────────────────────

    fn lower_param(&mut self, node: &SyntaxNode) -> Option<Param> {
        debug_assert_eq!(node.kind(), SyntaxKind::Param);

        // Collect all Ident tokens within the Param node.
        // Grammar: IDENT ':' IDENT  →  first = name, second = type.
        let idents: Vec<SyntaxToken> = node
            .children_with_tokens()
            .filter_map(|el| el.into_token())
            .filter(|t| t.kind() == SyntaxKind::Ident)
            .collect();

        if idents.len() < 2 {
            let span = span_of(node, self.file_id);
            self.push_error("incomplete parameter", span);
            return None;
        }

        let name = Ident {
            name: SmolStr::from(idents[0].text()),
            span: span_of_token(&idents[0], self.file_id),
        };
        let type_ann = Ident {
            name: SmolStr::from(idents[1].text()),
            span: span_of_token(&idents[1], self.file_id),
        };

        Some(Param { name, type_ann, span: span_of(node, self.file_id) })
    }

    // ── ReturnType ──────────────────────────────────────────────────

    fn lower_return_type(&mut self, node: &SyntaxNode) -> Option<Ident> {
        debug_assert_eq!(node.kind(), SyntaxKind::ReturnType);

        let token = first_token_of_kind(node, SyntaxKind::Ident)?;
        Some(Ident { name: SmolStr::from(token.text()), span: span_of_token(&token, self.file_id) })
    }

    // ── BlockExpr ───────────────────────────────────────────────────

    fn lower_block_expr(&mut self, node: &SyntaxNode) -> Expr {
        debug_assert_eq!(node.kind(), SyntaxKind::BlockExpr);

        let exprs: Vec<Expr> = node.children().filter_map(|c| self.lower_expr(&c)).collect();

        Expr::Block { exprs, span: span_of(node, self.file_id) }
    }

    // ── Expr dispatch ───────────────────────────────────────────────

    fn lower_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        match node.kind() {
            SyntaxKind::LiteralExpr => self.lower_literal_expr(node),
            SyntaxKind::IdentExpr => self.lower_ident_expr(node),
            SyntaxKind::BlockExpr => Some(self.lower_block_expr(node)),
            SyntaxKind::NodeError => {
                let span = span_of(node, self.file_id);
                self.push_error("unexpected syntax in expression position", span);
                None
            }
            _ => None, // skip unknown / trivia nodes
        }
    }

    // ── LiteralExpr ─────────────────────────────────────────────────

    fn lower_literal_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        debug_assert_eq!(node.kind(), SyntaxKind::LiteralExpr);

        let token = node
            .children_with_tokens()
            .filter_map(|el| el.into_token())
            .find(|t| matches!(t.kind(), SyntaxKind::IntLit | SyntaxKind::StringLit))?;

        let kind = match token.kind() {
            SyntaxKind::IntLit => LiteralKind::Int,
            SyntaxKind::StringLit => LiteralKind::String,
            _ => unreachable!(),
        };

        Some(Expr::Literal(Literal {
            kind,
            value: SmolStr::from(token.text()),
            span: span_of_token(&token, self.file_id),
        }))
    }

    // ── IdentExpr ───────────────────────────────────────────────────

    fn lower_ident_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        debug_assert_eq!(node.kind(), SyntaxKind::IdentExpr);

        let token = first_token_of_kind(node, SyntaxKind::Ident)?;
        Some(Expr::Variable(Ident {
            name: SmolStr::from(token.text()),
            span: span_of_token(&token, self.file_id),
        }))
    }
}
