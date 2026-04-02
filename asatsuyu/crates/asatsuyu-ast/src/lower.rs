//! CST → AST lowering.
//!
//! Walks the rowan syntax tree, strips trivia, and produces the typed AST
//! defined in [`crate::types`]. Diagnostics are collected for malformed nodes.

// `into_token()` lives on `rowan::NodeOrToken` which is not a direct dependency.
#![allow(clippy::redundant_closure_for_method_calls)]

use asatsuyu_parser::{SyntaxNode, SyntaxToken};
use asatsuyu_syntax::{Diagnostic, DiagnosticCode, FileId, Span, SyntaxKind};
use smol_str::SmolStr;

use crate::types::{
    BinOp, CustomType, Definition, Expr, FnDef, Ident, Import, Literal, LiteralKind, MatchArm,
    Module, Param, Pattern, RecordField, TypeBody, TypeExpr, UnOp, Variant, VariantField,
    Visibility,
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

    fn push_error(&mut self, message: impl Into<String>, span: Span, code: DiagnosticCode) {
        self.diagnostics
            .push(Diagnostic::error(message, span).with_code(code).with_label(span, "here"));
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

fn span_of_nontrivia(node: &SyntaxNode, file_id: FileId) -> Span {
    let mut tokens = node
        .descendants_with_tokens()
        .filter_map(|el| el.into_token())
        .filter(|tok| !tok.kind().is_trivia());

    let Some(first) = tokens.next() else {
        return span_of(node, file_id);
    };
    let last = tokens.last().unwrap_or_else(|| first.clone());
    Span::new(file_id, u32::from(first.text_range().start()), u32::from(last.text_range().end()))
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

/// Collect all child nodes with the given kind.
fn children_of_kind(node: &SyntaxNode, kind: SyntaxKind) -> Vec<SyntaxNode> {
    node.children().filter(|c| c.kind() == kind).collect()
}

// ── Lowering ────────────────────────────────────────────────────────

impl LowerCtx {
    // ── SourceFile → Module ─────────────────────────────────────────

    pub(crate) fn lower_source_file(&mut self, root: &SyntaxNode) -> Module {
        debug_assert_eq!(root.kind(), SyntaxKind::SourceFile);

        let mut imports = Vec::new();
        let mut definitions = Vec::new();

        for child in root.children() {
            match child.kind() {
                SyntaxKind::FnDef => {
                    if let Some(fn_def) = self.lower_fn_def(&child) {
                        definitions.push(Definition::Function(fn_def));
                    }
                }
                SyntaxKind::TypeDef => {
                    if let Some(ct) = self.lower_type_def(&child) {
                        definitions.push(Definition::CustomType(ct));
                    }
                }
                SyntaxKind::ImportStmt => {
                    if let Some(imp) = self.lower_import(&child) {
                        imports.push(imp);
                    }
                }
                SyntaxKind::FromPythonImportStmt => {
                    if let Some(imp) = self.lower_from_python_import(&child) {
                        imports.push(imp);
                    }
                }
                SyntaxKind::NodeError => {
                    let span = span_of(&child, self.file_id);
                    self.push_error("unexpected syntax", span, DiagnosticCode::E0100);
                }
                _ => {} // trivia / other tokens — skip
            }
        }

        Module { imports, definitions, span: span_of(root, self.file_id) }
    }

    // ── ImportStmt → Import ─────────────────────────────────────────

    fn lower_import(&mut self, node: &SyntaxNode) -> Option<Import> {
        debug_assert_eq!(node.kind(), SyntaxKind::ImportStmt);

        // Collect all Ident tokens — module path segments + optional alias.
        // The `as` keyword separates path from alias.
        let has_as = first_token_of_kind(node, SyntaxKind::AsKw).is_some();

        let idents: Vec<SyntaxToken> = node
            .children_with_tokens()
            .filter_map(|el| el.into_token())
            .filter(|t| t.kind() == SyntaxKind::Ident)
            .collect();

        if idents.is_empty() {
            let span = span_of(node, self.file_id);
            self.push_error("empty import path", span, DiagnosticCode::E0101);
            return None;
        }

        let (path_idents, alias) = if has_as && idents.len() >= 2 {
            let (path, alias_slice) = idents.split_at(idents.len() - 1);
            let alias_token = &alias_slice[0];
            (
                path,
                Some(Ident {
                    name: SmolStr::from(alias_token.text()),
                    span: span_of_token(alias_token, self.file_id),
                }),
            )
        } else {
            (idents.as_slice(), None)
        };

        let module: Vec<Ident> = path_idents
            .iter()
            .map(|t| Ident { name: SmolStr::from(t.text()), span: span_of_token(t, self.file_id) })
            .collect();

        Some(Import::Module { module, alias, span: span_of(node, self.file_id) })
    }

    // ── FromPythonImportStmt → Import::Python ──────────────────────

    fn lower_from_python_import(&mut self, node: &SyntaxNode) -> Option<Import> {
        debug_assert_eq!(node.kind(), SyntaxKind::FromPythonImportStmt);

        // Collect Ident tokens: first is the module name, second (if `as` present) is alias.
        let has_as = first_token_of_kind(node, SyntaxKind::AsKw).is_some();

        let idents: Vec<SyntaxToken> = node
            .children_with_tokens()
            .filter_map(|el| el.into_token())
            .filter(|t| t.kind() == SyntaxKind::Ident)
            .collect();

        if idents.is_empty() {
            let span = span_of(node, self.file_id);
            self.push_error("missing module name in Python import", span, DiagnosticCode::E0102);
            return None;
        }

        let module_name = Ident {
            name: SmolStr::from(idents[0].text()),
            span: span_of_token(&idents[0], self.file_id),
        };

        let alias = if has_as && idents.len() >= 2 {
            let alias_token = &idents[1];
            Some(Ident {
                name: SmolStr::from(alias_token.text()),
                span: span_of_token(alias_token, self.file_id),
            })
        } else {
            None
        };

        Some(Import::Python { module_name, alias, span: span_of(node, self.file_id) })
    }

    // ── FnDef ───────────────────────────────────────────────────────

    fn lower_fn_def(&mut self, node: &SyntaxNode) -> Option<FnDef> {
        debug_assert_eq!(node.kind(), SyntaxKind::FnDef);

        let visibility = if first_child_of_kind(node, SyntaxKind::Visibility).is_some() {
            Visibility::Public
        } else {
            Visibility::Private
        };

        let is_async = first_token_of_kind(node, SyntaxKind::AsyncKw).is_some();

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
            self.push_error("missing function body", span, DiagnosticCode::E0103);
            Expr::Block { exprs: Vec::new(), span }
        };

        Some(FnDef {
            name,
            visibility,
            is_async,
            params,
            return_type,
            body,
            span: span_of(node, self.file_id),
        })
    }

    // ── TypeDef → CustomType ────────────────────────────────────────

    fn lower_type_def(&mut self, node: &SyntaxNode) -> Option<CustomType> {
        debug_assert_eq!(node.kind(), SyntaxKind::TypeDef);

        let visibility = if first_child_of_kind(node, SyntaxKind::Visibility).is_some() {
            Visibility::Public
        } else {
            Visibility::Private
        };

        // Type name
        let name_token = first_token_of_kind(node, SyntaxKind::Ident)?;
        let name = Ident {
            name: SmolStr::from(name_token.text()),
            span: span_of_token(&name_token, self.file_id),
        };

        // Type parameters
        let type_params = self.lower_type_params(node);

        // Body: detect record vs variant by looking for Field vs Variant children
        let variants = children_of_kind(node, SyntaxKind::Variant);
        let fields = children_of_kind(node, SyntaxKind::Field);

        let body = if !fields.is_empty() && variants.is_empty() {
            // Record style
            let record_fields: Vec<RecordField> =
                fields.iter().filter_map(|f| self.lower_record_field(f)).collect();
            TypeBody::Record(record_fields)
        } else {
            // Variant style (or empty)
            let vs: Vec<Variant> = variants.iter().filter_map(|v| self.lower_variant(v)).collect();
            TypeBody::Variants(vs)
        };

        Some(CustomType { name, visibility, type_params, body, span: span_of(node, self.file_id) })
    }

    /// Extract type parameters from `TypeParam` children.
    fn lower_type_params(&mut self, node: &SyntaxNode) -> Vec<Ident> {
        children_of_kind(node, SyntaxKind::TypeParam)
            .iter()
            .filter_map(|tp| {
                let token = first_token_of_kind(tp, SyntaxKind::Ident)?;
                Some(Ident {
                    name: SmolStr::from(token.text()),
                    span: span_of_token(&token, self.file_id),
                })
            })
            .collect()
    }

    fn lower_record_field(&mut self, node: &SyntaxNode) -> Option<RecordField> {
        debug_assert_eq!(node.kind(), SyntaxKind::Field);

        let name_token = first_token_of_kind(node, SyntaxKind::Ident)?;
        let name = Ident {
            name: SmolStr::from(name_token.text()),
            span: span_of_token(&name_token, self.file_id),
        };

        let type_ann = first_child_of_kind(node, SyntaxKind::TypeExpr)
            .and_then(|te| self.lower_type_expr(&te))?;

        Some(RecordField { name, type_ann, span: span_of(node, self.file_id) })
    }

    fn lower_variant(&mut self, node: &SyntaxNode) -> Option<Variant> {
        debug_assert_eq!(node.kind(), SyntaxKind::Variant);

        let name_token = first_token_of_kind(node, SyntaxKind::Ident)?;
        let name = Ident {
            name: SmolStr::from(name_token.text()),
            span: span_of_token(&name_token, self.file_id),
        };

        let fields: Vec<VariantField> = children_of_kind(node, SyntaxKind::Field)
            .iter()
            .filter_map(|f| self.lower_variant_field(f))
            .collect();

        Some(Variant { name, fields, span: span_of(node, self.file_id) })
    }

    fn lower_variant_field(&mut self, node: &SyntaxNode) -> Option<VariantField> {
        debug_assert_eq!(node.kind(), SyntaxKind::Field);

        // Collect Ident tokens. If two exist + Colon → labelled; otherwise positional.
        let idents: Vec<SyntaxToken> = node
            .children_with_tokens()
            .filter_map(|el| el.into_token())
            .filter(|t| t.kind() == SyntaxKind::Ident)
            .collect();

        let has_colon = first_token_of_kind(node, SyntaxKind::Colon).is_some();

        let type_expr_node = first_child_of_kind(node, SyntaxKind::TypeExpr);

        if has_colon && idents.len() >= 2 {
            // Labelled field: `label: Type`
            let label = Some(Ident {
                name: SmolStr::from(idents[0].text()),
                span: span_of_token(&idents[0], self.file_id),
            });
            let type_ann = type_expr_node.and_then(|te| self.lower_type_expr(&te))?;
            Some(VariantField { label, type_ann, span: span_of(node, self.file_id) })
        } else {
            // Positional field: just a TypeExpr
            let type_ann = type_expr_node.and_then(|te| self.lower_type_expr(&te))?;
            Some(VariantField { label: None, type_ann, span: span_of(node, self.file_id) })
        }
    }

    // ── TypeExpr ────────────────────────────────────────────────────

    fn lower_type_expr(&mut self, node: &SyntaxNode) -> Option<TypeExpr> {
        debug_assert_eq!(node.kind(), SyntaxKind::TypeExpr);

        let name_token = first_token_of_kind(node, SyntaxKind::Ident)?;
        let name = Ident {
            name: SmolStr::from(name_token.text()),
            span: span_of_token(&name_token, self.file_id),
        };

        // Type arguments: nested TypeExpr children
        let args: Vec<TypeExpr> = children_of_kind(node, SyntaxKind::TypeExpr)
            .iter()
            .filter_map(|te| self.lower_type_expr(te))
            .collect();

        Some(TypeExpr::Named { name, args, span: span_of_nontrivia(node, self.file_id) })
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

        // Parameter name: first Ident token
        let name_token = node
            .children_with_tokens()
            .filter_map(|el| el.into_token())
            .find(|t| t.kind() == SyntaxKind::Ident)?;

        let name = Ident {
            name: SmolStr::from(name_token.text()),
            span: span_of_token(&name_token, self.file_id),
        };

        // Type annotation (optional): try TypeExpr child first, fall back to second Ident token
        let type_ann = if let Some(te) = first_child_of_kind(node, SyntaxKind::TypeExpr) {
            Some(self.lower_type_expr(&te)?)
        } else {
            // Fallback: second Ident token (simple `name: Type` without TypeExpr node)
            let idents: Vec<SyntaxToken> = node
                .children_with_tokens()
                .filter_map(|el| el.into_token())
                .filter(|t| t.kind() == SyntaxKind::Ident)
                .collect();

            if idents.len() >= 2 {
                let type_ident = Ident {
                    name: SmolStr::from(idents[1].text()),
                    span: span_of_token(&idents[1], self.file_id),
                };
                Some(TypeExpr::Named {
                    name: type_ident,
                    args: Vec::new(),
                    span: span_of_token(&idents[1], self.file_id),
                })
            } else {
                // No type annotation (valid for lambda parameters)
                None
            }
        };

        Some(Param { name, type_ann, span: span_of(node, self.file_id) })
    }

    // ── ReturnType ──────────────────────────────────────────────────

    fn lower_return_type(&mut self, node: &SyntaxNode) -> Option<TypeExpr> {
        debug_assert_eq!(node.kind(), SyntaxKind::ReturnType);

        // Try TypeExpr child first, fall back to bare Ident token
        if let Some(te) = first_child_of_kind(node, SyntaxKind::TypeExpr) {
            return self.lower_type_expr(&te);
        }

        let token = first_token_of_kind(node, SyntaxKind::Ident)?;
        let name =
            Ident { name: SmolStr::from(token.text()), span: span_of_token(&token, self.file_id) };
        Some(TypeExpr::Named { name, args: Vec::new(), span: span_of_token(&token, self.file_id) })
    }

    // ── BlockExpr ───────────────────────────────────────────────────

    fn lower_block_expr(&mut self, node: &SyntaxNode) -> Expr {
        debug_assert_eq!(node.kind(), SyntaxKind::BlockExpr);

        let exprs: Vec<Expr> = node
            .children()
            .filter_map(|c| match c.kind() {
                SyntaxKind::LetStmt => self.lower_let_stmt(&c),
                SyntaxKind::AssignStmt => self.lower_assign_stmt(&c),
                _ => self.lower_expr(&c),
            })
            .collect();

        Expr::Block { exprs, span: span_of(node, self.file_id) }
    }

    // ── LetStmt ─────────────────────────────────────────────────────

    fn lower_let_stmt(&mut self, node: &SyntaxNode) -> Option<Expr> {
        debug_assert_eq!(node.kind(), SyntaxKind::LetStmt);

        let is_mutable = node
            .children_with_tokens()
            .any(|c| c.as_token().is_some_and(|t| t.kind() == SyntaxKind::MutKw));

        let name_token = first_token_of_kind(node, SyntaxKind::Ident)?;
        let name = Ident {
            name: SmolStr::from(name_token.text()),
            span: span_of_token(&name_token, self.file_id),
        };

        // Value: the first child expression
        let value = node.children().find_map(|c| self.lower_expr(&c))?;

        Some(Expr::Let {
            name,
            value: Box::new(value),
            is_mutable,
            span: span_of_nontrivia(node, self.file_id),
        })
    }

    // ── AssignStmt ──────────────────────────────────────────────────

    fn lower_assign_stmt(&mut self, node: &SyntaxNode) -> Option<Expr> {
        debug_assert_eq!(node.kind(), SyntaxKind::AssignStmt);

        let name_token = first_token_of_kind(node, SyntaxKind::Ident)?;
        let target = Ident {
            name: SmolStr::from(name_token.text()),
            span: span_of_token(&name_token, self.file_id),
        };

        let value = node.children().find_map(|c| self.lower_expr(&c))?;

        Some(Expr::Assign {
            target,
            value: Box::new(value),
            span: span_of_nontrivia(node, self.file_id),
        })
    }

    // ── LambdaExpr ──────────────────────────────────────────────────

    fn lower_lambda_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        debug_assert_eq!(node.kind(), SyntaxKind::LambdaExpr);

        let params = if let Some(pl) = first_child_of_kind(node, SyntaxKind::ParamList) {
            self.lower_param_list(&pl)
        } else {
            Vec::new()
        };

        let return_type = first_child_of_kind(node, SyntaxKind::ReturnType)
            .and_then(|rt| self.lower_return_type(&rt));

        let body =
            first_child_of_kind(node, SyntaxKind::BlockExpr).map(|b| self.lower_block_expr(&b))?;

        Some(Expr::Lambda {
            params,
            return_type,
            body: Box::new(body),
            span: span_of(node, self.file_id),
        })
    }

    // ── Expr dispatch ───────────────────────────────────────────────

    fn lower_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        match node.kind() {
            SyntaxKind::LiteralExpr => self.lower_literal_expr(node),
            SyntaxKind::IdentExpr => self.lower_ident_expr(node),
            SyntaxKind::BlockExpr => Some(self.lower_block_expr(node)),
            SyntaxKind::CallExpr => self.lower_call_expr(node),
            SyntaxKind::BinaryExpr => self.lower_binary_expr(node),
            SyntaxKind::UnaryExpr => self.lower_unary_expr(node),
            SyntaxKind::PipelineExpr => self.lower_pipeline_expr(node),
            SyntaxKind::IfExpr => self.lower_if_expr(node),
            SyntaxKind::MatchExpr => self.lower_match_expr(node),
            SyntaxKind::LambdaExpr => self.lower_lambda_expr(node),
            SyntaxKind::FieldAccessExpr => self.lower_field_access_expr(node),
            SyntaxKind::TryExpr => self.lower_try_expr(node),
            SyntaxKind::AwaitExpr => self.lower_await_expr(node),
            SyntaxKind::ListExpr => Some(self.lower_list_expr(node)),
            SyntaxKind::ParenExpr => self.lower_paren_expr(node),
            SyntaxKind::NodeError => {
                let span = span_of(node, self.file_id);
                self.push_error(
                    "unexpected syntax in expression position",
                    span,
                    DiagnosticCode::E0100,
                );
                None
            }
            _ => None, // skip unknown / trivia nodes
        }
    }

    // ── ListExpr ────────────────────────────────────────────────────

    fn lower_list_expr(&mut self, node: &SyntaxNode) -> Expr {
        debug_assert_eq!(node.kind(), SyntaxKind::ListExpr);

        let elements = node.children().filter_map(|c| self.lower_expr(&c)).collect();

        Expr::List { elements, span: span_of(node, self.file_id) }
    }

    // ── LiteralExpr ─────────────────────────────────────────────────

    fn lower_literal_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        debug_assert_eq!(node.kind(), SyntaxKind::LiteralExpr);

        let token = node.children_with_tokens().filter_map(|el| el.into_token()).find(|t| {
            matches!(
                t.kind(),
                SyntaxKind::IntLit
                    | SyntaxKind::FloatLit
                    | SyntaxKind::StringLit
                    | SyntaxKind::TrueKw
                    | SyntaxKind::FalseKw
            )
        })?;

        let kind = match token.kind() {
            SyntaxKind::IntLit => LiteralKind::Int,
            SyntaxKind::FloatLit => LiteralKind::Float,
            SyntaxKind::StringLit => LiteralKind::String,
            SyntaxKind::TrueKw | SyntaxKind::FalseKw => LiteralKind::Bool,
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

    // ── CallExpr ────────────────────────────────────────────────────

    fn lower_call_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        debug_assert_eq!(node.kind(), SyntaxKind::CallExpr);

        // The first child is the callee expression, the ArgList contains arguments.
        let func = node
            .children()
            .find(|c| c.kind() != SyntaxKind::ArgList)
            .and_then(|c| self.lower_expr(&c))?;

        let args = first_child_of_kind(node, SyntaxKind::ArgList)
            .map(|al| self.lower_arg_list(&al))
            .unwrap_or_default();

        Some(Expr::Call { func: Box::new(func), args, span: span_of(node, self.file_id) })
    }

    fn lower_arg_list(&mut self, node: &SyntaxNode) -> Vec<Expr> {
        debug_assert_eq!(node.kind(), SyntaxKind::ArgList);
        node.children().filter_map(|c| self.lower_expr(&c)).collect()
    }

    // ── FieldAccessExpr ──────────────────────────────────────────────

    fn lower_field_access_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        debug_assert_eq!(node.kind(), SyntaxKind::FieldAccessExpr);

        // First child node is the receiver expression.
        let receiver = node
            .children()
            .find(|c| c.kind() != SyntaxKind::NodeError)
            .and_then(|c| self.lower_expr(&c))?;

        // The field name is the Ident token that is a direct child of this node
        // (not nested inside a child expression node). It appears after the Dot.
        let field_token = node
            .children_with_tokens()
            .filter_map(|el| el.into_token())
            .filter(|t| t.kind() == SyntaxKind::Ident)
            .last()?;

        let field = Ident {
            name: SmolStr::from(field_token.text()),
            span: span_of_token(&field_token, self.file_id),
        };

        Some(Expr::FieldAccess {
            receiver: Box::new(receiver),
            field,
            span: span_of(node, self.file_id),
        })
    }

    // ── TryExpr ─────────────────────────────────────────────────────

    fn lower_try_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        debug_assert_eq!(node.kind(), SyntaxKind::TryExpr);
        let inner = node.children().find_map(|c| self.lower_expr(&c))?;
        Some(Expr::Try { expr: Box::new(inner), span: span_of(node, self.file_id) })
    }

    // ── AwaitExpr ───────────────────────────────────────────────────

    fn lower_await_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        debug_assert_eq!(node.kind(), SyntaxKind::AwaitExpr);
        let inner = node.children().find_map(|c| self.lower_expr(&c))?;
        Some(Expr::Await { expr: Box::new(inner), span: span_of(node, self.file_id) })
    }

    // ── BinaryExpr ──────────────────────────────────────────────────

    fn lower_binary_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        debug_assert_eq!(node.kind(), SyntaxKind::BinaryExpr);

        let children: Vec<SyntaxNode> = node.children().collect();
        if children.len() < 2 {
            let span = span_of(node, self.file_id);
            self.push_error("incomplete binary expression", span, DiagnosticCode::E0104);
            return None;
        }

        let lhs = self.lower_expr(&children[0])?;
        let rhs = self.lower_expr(&children[1])?;

        // Find the operator token between children
        let op_token = node.children_with_tokens().filter_map(|el| el.into_token()).find(|t| {
            matches!(
                t.kind(),
                SyntaxKind::Plus
                    | SyntaxKind::Minus
                    | SyntaxKind::Star
                    | SyntaxKind::Slash
                    | SyntaxKind::Percent
                    | SyntaxKind::EqEq
                    | SyntaxKind::BangEq
                    | SyntaxKind::Lt
                    | SyntaxKind::LtEq
                    | SyntaxKind::Gt
                    | SyntaxKind::GtEq
                    | SyntaxKind::AmpAmp
                    | SyntaxKind::PipePipe
                    | SyntaxKind::StringConcat
            )
        })?;

        let op = match op_token.kind() {
            SyntaxKind::Plus => BinOp::Add,
            SyntaxKind::Minus => BinOp::Sub,
            SyntaxKind::Star => BinOp::Mul,
            SyntaxKind::Slash => BinOp::Div,
            SyntaxKind::Percent => BinOp::Mod,
            SyntaxKind::EqEq => BinOp::Eq,
            SyntaxKind::BangEq => BinOp::NotEq,
            SyntaxKind::Lt => BinOp::Lt,
            SyntaxKind::LtEq => BinOp::LtEq,
            SyntaxKind::Gt => BinOp::Gt,
            SyntaxKind::GtEq => BinOp::GtEq,
            SyntaxKind::AmpAmp => BinOp::And,
            SyntaxKind::PipePipe => BinOp::Or,
            SyntaxKind::StringConcat => BinOp::StringConcat,
            _ => unreachable!(),
        };

        Some(Expr::BinaryOp {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span: span_of(node, self.file_id),
        })
    }

    // ── UnaryExpr ───────────────────────────────────────────────────

    fn lower_unary_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        debug_assert_eq!(node.kind(), SyntaxKind::UnaryExpr);

        let op_token = node
            .children_with_tokens()
            .filter_map(|el| el.into_token())
            .find(|t| matches!(t.kind(), SyntaxKind::Minus | SyntaxKind::Bang))?;

        let op = match op_token.kind() {
            SyntaxKind::Minus => UnOp::Neg,
            SyntaxKind::Bang => UnOp::Not,
            _ => unreachable!(),
        };

        let operand = node.children().find_map(|c| self.lower_expr(&c))?;

        Some(Expr::UnaryOp { op, expr: Box::new(operand), span: span_of(node, self.file_id) })
    }

    // ── PipelineExpr ────────────────────────────────────────────────

    fn lower_pipeline_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        debug_assert_eq!(node.kind(), SyntaxKind::PipelineExpr);

        let children: Vec<SyntaxNode> = node.children().collect();
        if children.len() < 2 {
            let span = span_of(node, self.file_id);
            self.push_error("incomplete pipeline expression", span, DiagnosticCode::E0105);
            return None;
        }

        let left = self.lower_expr(&children[0])?;
        let right = self.lower_expr(&children[1])?;

        Some(Expr::Pipeline {
            left: Box::new(left),
            right: Box::new(right),
            span: span_of(node, self.file_id),
        })
    }

    // ── IfExpr ──────────────────────────────────────────────────────

    fn lower_if_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        debug_assert_eq!(node.kind(), SyntaxKind::IfExpr);

        let mut child_iter = node.children();

        // Condition: first non-block child expression
        let cond_node = child_iter.next()?;
        let condition = self.lower_expr(&cond_node)?;

        // Then body: first BlockExpr
        let then_node = child_iter.find(|c| c.kind() == SyntaxKind::BlockExpr)?;
        let then_body = self.lower_block_expr(&then_node);

        // Else body: optional — either IfExpr (else-if) or BlockExpr
        let else_body = child_iter
            .find(|c| matches!(c.kind(), SyntaxKind::IfExpr | SyntaxKind::BlockExpr))
            .and_then(|c| match c.kind() {
                SyntaxKind::IfExpr => self.lower_if_expr(&c),
                SyntaxKind::BlockExpr => Some(self.lower_block_expr(&c)),
                _ => None,
            });

        Some(Expr::If {
            condition: Box::new(condition),
            then_body: Box::new(then_body),
            else_body: else_body.map(Box::new),
            span: span_of(node, self.file_id),
        })
    }

    // ── MatchExpr ───────────────────────────────────────────────────

    fn lower_match_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        debug_assert_eq!(node.kind(), SyntaxKind::MatchExpr);

        // Subject: first non-MatchArm child expression
        let subject_node = node.children().find(|c| c.kind() != SyntaxKind::MatchArm)?;
        let subject = self.lower_expr(&subject_node)?;

        // Arms
        let arms: Vec<MatchArm> = children_of_kind(node, SyntaxKind::MatchArm)
            .iter()
            .filter_map(|arm| self.lower_match_arm(arm))
            .collect();

        Some(Expr::Match { subject: Box::new(subject), arms, span: span_of(node, self.file_id) })
    }

    fn lower_match_arm(&mut self, node: &SyntaxNode) -> Option<MatchArm> {
        debug_assert_eq!(node.kind(), SyntaxKind::MatchArm);

        // Pattern: first pattern-kind child
        let pat_node = node.children().find(|c| is_pattern_kind(c.kind()))?;
        let pattern = self.lower_pattern(&pat_node)?;

        // Optional guard
        let guard = first_child_of_kind(node, SyntaxKind::Guard)
            .and_then(|g| {
                // The guard's child expression (after `if` keyword)
                g.children().find_map(|c| self.lower_expr(&c))
            })
            .map(Box::new);

        // Body: expression after the arrow. It's the last non-pattern, non-guard child.
        let body = node
            .children()
            .filter(|c| !is_pattern_kind(c.kind()) && c.kind() != SyntaxKind::Guard)
            .find_map(|c| self.lower_expr(&c))?;

        Some(MatchArm { pattern, guard, body, span: span_of(node, self.file_id) })
    }

    // ── Pattern ─────────────────────────────────────────────────────

    fn lower_pattern(&mut self, node: &SyntaxNode) -> Option<Pattern> {
        match node.kind() {
            SyntaxKind::WildcardPat => Some(Pattern::Wildcard(span_of(node, self.file_id))),
            SyntaxKind::IdentPat => {
                let token = first_token_of_kind(node, SyntaxKind::Ident)?;
                Some(Pattern::Variable(Ident {
                    name: SmolStr::from(token.text()),
                    span: span_of_token(&token, self.file_id),
                }))
            }
            SyntaxKind::LiteralPat => {
                let token =
                    node.children_with_tokens().filter_map(|el| el.into_token()).find(|t| {
                        matches!(
                            t.kind(),
                            SyntaxKind::IntLit
                                | SyntaxKind::FloatLit
                                | SyntaxKind::StringLit
                                | SyntaxKind::TrueKw
                                | SyntaxKind::FalseKw
                        )
                    })?;
                let kind = match token.kind() {
                    SyntaxKind::IntLit => LiteralKind::Int,
                    SyntaxKind::FloatLit => LiteralKind::Float,
                    SyntaxKind::StringLit => LiteralKind::String,
                    SyntaxKind::TrueKw | SyntaxKind::FalseKw => LiteralKind::Bool,
                    _ => unreachable!(),
                };
                Some(Pattern::Literal(Literal {
                    kind,
                    value: SmolStr::from(token.text()),
                    span: span_of_token(&token, self.file_id),
                }))
            }
            SyntaxKind::ConstructorPat => {
                let name_token = first_token_of_kind(node, SyntaxKind::Ident)?;
                let name = Ident {
                    name: SmolStr::from(name_token.text()),
                    span: span_of_token(&name_token, self.file_id),
                };
                let fields: Vec<Pattern> = node
                    .children()
                    .filter(|c| is_pattern_kind(c.kind()))
                    .filter_map(|c| self.lower_pattern(&c))
                    .collect();
                Some(Pattern::Constructor { name, fields, span: span_of(node, self.file_id) })
            }
            SyntaxKind::ListPat => {
                let elements: Vec<Pattern> = node
                    .children()
                    .filter(|c| is_pattern_kind(c.kind()))
                    .filter_map(|c| self.lower_pattern(&c))
                    .collect();

                // Rest binding: `..rest` — look for DotDot + optional Ident after it
                let has_dot_dot = first_token_of_kind(node, SyntaxKind::DotDot).is_some();
                let rest = if has_dot_dot {
                    // Find Ident after DotDot
                    let mut found_dots = false;
                    node.children_with_tokens().filter_map(|el| el.into_token()).find_map(|t| {
                        if t.kind() == SyntaxKind::DotDot {
                            found_dots = true;
                            None
                        } else if found_dots && t.kind() == SyntaxKind::Ident {
                            Some(Ident {
                                name: SmolStr::from(t.text()),
                                span: span_of_token(&t, self.file_id),
                            })
                        } else {
                            None
                        }
                    })
                } else {
                    None
                };

                Some(Pattern::List { elements, rest, span: span_of(node, self.file_id) })
            }
            _ => {
                let span = span_of(node, self.file_id);
                self.push_error("unsupported pattern kind", span, DiagnosticCode::E0106);
                None
            }
        }
    }

    // ── ParenExpr ───────────────────────────────────────────────────

    fn lower_paren_expr(&mut self, node: &SyntaxNode) -> Option<Expr> {
        debug_assert_eq!(node.kind(), SyntaxKind::ParenExpr);
        // Transparently return the inner expression
        node.children().find_map(|c| self.lower_expr(&c))
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

fn is_pattern_kind(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::WildcardPat
            | SyntaxKind::IdentPat
            | SyntaxKind::LiteralPat
            | SyntaxKind::ConstructorPat
            | SyntaxKind::ListPat
            | SyntaxKind::TuplePat
    )
}
