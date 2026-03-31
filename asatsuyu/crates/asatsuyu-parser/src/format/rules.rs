//! Per-node formatting rules for each `SyntaxKind`.
//!
//! Each `format_*` function receives a `&mut Formatter` and a `&SyntaxNode`
//! and appends formatted text to the output buffer.

use asatsuyu_syntax::SyntaxKind;
use rowan::NodeOrToken;

use crate::language::{SyntaxNode, SyntaxToken};

use super::formatter::Formatter;

// ── Helpers ─────────────────────────────────────────────────────

/// Get all non-trivia children (nodes and tokens) from a syntax node.
fn non_trivia_children(node: &SyntaxNode) -> Vec<NodeOrToken<SyntaxNode, SyntaxToken>> {
    node.children_with_tokens()
        .filter(|elem| match elem {
            NodeOrToken::Token(t) => !t.kind().is_trivia(),
            NodeOrToken::Node(_) => true,
        })
        .collect()
}

/// Get child nodes of a specific kind.
fn child_nodes(node: &SyntaxNode, kind: SyntaxKind) -> Vec<SyntaxNode> {
    node.children().filter(|n| n.kind() == kind).collect()
}

/// Get the first child node of a specific kind.
fn first_child_node(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxNode> {
    node.children().find(|n| n.kind() == kind)
}

/// Get the first non-trivia token of a specific kind.
fn first_token(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxToken> {
    node.children_with_tokens().find_map(|elem| match elem {
        NodeOrToken::Token(t) if t.kind() == kind => Some(t),
        _ => None,
    })
}

/// Check if a node contains a child of a specific kind.
fn has_child(node: &SyntaxNode, kind: SyntaxKind) -> bool {
    node.children_with_tokens().any(|elem| elem.kind() == kind)
}

/// Collect comment tokens from the leading trivia attached to `node`.
fn collect_leading_comments(node: &SyntaxNode) -> Vec<String> {
    let mut comments = Vec::new();
    let mut current = node.clone();

    loop {
        let mut descended = false;

        for elem in current.children_with_tokens() {
            match elem {
                NodeOrToken::Token(ref t) if t.kind() == SyntaxKind::Comment => {
                    comments.push(t.text().to_string());
                }
                NodeOrToken::Token(ref t) if t.kind().is_trivia() => {}
                NodeOrToken::Node(n) => {
                    current = n;
                    descended = true;
                    break;
                }
                NodeOrToken::Token(_) => return comments,
            }
        }

        if !descended {
            return comments;
        }
    }
}

/// Collect trailing comment tokens that remain after the last non-trivia child.
fn collect_trailing_comments(node: &SyntaxNode) -> Vec<String> {
    let mut current_comments = Vec::new();

    for elem in node.children_with_tokens() {
        match elem {
            NodeOrToken::Token(ref t) if t.kind() == SyntaxKind::Comment => {
                current_comments.push(t.text().to_string());
            }
            NodeOrToken::Token(ref t) if t.kind().is_trivia() => {}
            _ => current_comments.clear(),
        }
    }

    current_comments
}

/// Collect comment tokens immediately before a closing delimiter.
fn collect_comments_before_closer(node: &SyntaxNode, closing_kind: SyntaxKind) -> Vec<String> {
    let mut current_comments = Vec::new();

    for elem in node.children_with_tokens() {
        match elem {
            NodeOrToken::Token(ref t) if t.kind() == closing_kind => return current_comments,
            NodeOrToken::Token(ref t) if t.kind() == SyntaxKind::Comment => {
                current_comments.push(t.text().to_string());
            }
            NodeOrToken::Token(ref t) if t.kind().is_trivia() => {}
            _ => current_comments.clear(),
        }
    }

    Vec::new()
}

// ── SourceFile ──────────────────────────────────────────────────

pub(super) fn format_source_file(f: &mut Formatter, node: &SyntaxNode) {
    let items: Vec<SyntaxNode> =
        node.children().filter(|n| !matches!(n.kind(), SyntaxKind::NodeError)).collect();

    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            f.write_blank_line();
        }

        for c in collect_leading_comments(item) {
            f.write_str(&c);
            f.write_newline();
        }

        format_node(f, item);
    }

    // Trailing comments after last item.
    let trailing_comments = collect_trailing_comments(node);
    if !trailing_comments.is_empty() {
        if !items.is_empty() {
            f.write_blank_line();
        }
        for c in trailing_comments {
            f.write_str(&c);
            f.write_newline();
        }
    }
}

// ── Dispatch ────────────────────────────────────────────────────

pub(crate) fn format_node(f: &mut Formatter, node: &SyntaxNode) {
    match node.kind() {
        SyntaxKind::FnDef => format_fn_def(f, node),
        SyntaxKind::TypeDef => format_type_def(f, node),
        SyntaxKind::ImportStmt => format_import_stmt(f, node),
        SyntaxKind::FromPythonImportStmt => format_from_python_import(f, node),
        SyntaxKind::LetStmt => format_let_stmt(f, node),
        SyntaxKind::BlockExpr => format_block_expr(f, node),
        SyntaxKind::LiteralExpr => format_literal_expr(f, node),
        SyntaxKind::IdentExpr => format_ident_expr(f, node),
        SyntaxKind::CallExpr => format_call_expr(f, node),
        SyntaxKind::PipelineExpr => format_binary_like(f, node, "|>"),
        SyntaxKind::BinaryExpr => format_binary_expr(f, node),
        SyntaxKind::UnaryExpr => format_unary_expr(f, node),
        SyntaxKind::FieldAccessExpr => format_field_access_expr(f, node),
        SyntaxKind::IfExpr => format_if_expr(f, node),
        SyntaxKind::MatchExpr => format_match_expr(f, node),
        SyntaxKind::MatchArm => format_match_arm(f, node),
        SyntaxKind::LambdaExpr => format_lambda_expr(f, node),
        SyntaxKind::ParenExpr => format_paren_expr(f, node),
        SyntaxKind::TryExpr => format_try_expr(f, node),
        SyntaxKind::ListExpr => format_list_expr(f, node),
        SyntaxKind::TupleExpr => format_tuple_expr(f, node),
        SyntaxKind::RecordExpr => format_record_expr(f, node),
        SyntaxKind::StringConcat => format_binary_like(f, node, "<>"),
        // Patterns
        SyntaxKind::WildcardPat => format_wildcard_pat(f, node),
        SyntaxKind::IdentPat => format_ident_pat(f, node),
        SyntaxKind::LiteralPat => format_literal_pat(f, node),
        SyntaxKind::ConstructorPat => format_constructor_pat(f, node),
        SyntaxKind::ListPat => format_list_pat(f, node),
        SyntaxKind::TuplePat => format_tuple_pat(f, node),
        // Everything else (sub-nodes, error recovery): emit verbatim
        _ => format_verbatim(f, node),
    }
}

/// Fallback: emit all non-trivia tokens in order with original spacing.
fn format_verbatim(f: &mut Formatter, node: &SyntaxNode) {
    for elem in non_trivia_children(node) {
        match elem {
            NodeOrToken::Token(t) => f.write_token(&t),
            NodeOrToken::Node(n) => format_node(f, &n),
        }
    }
}

// ── Top-level definitions ───────────────────────────────────────

fn format_fn_def(f: &mut Formatter, node: &SyntaxNode) {
    // pub fn name(params) -> ReturnType { body }
    let children = non_trivia_children(node);

    for (i, elem) in children.iter().enumerate() {
        match elem {
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::Visibility => {
                f.write_str("pub");
                f.write_space();
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::FnKw => {
                f.write_str("fn");
                f.write_space();
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::Ident => {
                f.write_token(t);
            }
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::ParamList => {
                format_param_list(f, n);
            }
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::ReturnType => {
                f.write_space();
                format_return_type(f, n);
            }
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::BlockExpr => {
                f.write_space();
                format_block_expr(f, n);
            }
            _ => {
                // Unexpected child — emit as-is
                format_element(f, elem, i > 0);
            }
        }
    }
}

fn format_param_list(f: &mut Formatter, node: &SyntaxNode) {
    f.write_str("(");

    let params: Vec<SyntaxNode> = child_nodes(node, SyntaxKind::Param);
    for (i, param) in params.iter().enumerate() {
        if i > 0 {
            f.write_str(",");
            f.write_space();
        }
        format_param(f, param);
    }

    f.write_str(")");
}

fn format_param(f: &mut Formatter, node: &SyntaxNode) {
    let children = non_trivia_children(node);
    let mut wrote_name = false;

    for elem in &children {
        match elem {
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::Ident && !wrote_name => {
                f.write_token(t);
                wrote_name = true;
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::Colon => {
                f.write_str(":");
                f.write_space();
            }
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::TypeExpr => {
                format_type_expr(f, n);
            }
            _ => {}
        }
    }
}

fn format_return_type(f: &mut Formatter, node: &SyntaxNode) {
    f.write_str("->");
    f.write_space();
    if let Some(ty) = first_child_node(node, SyntaxKind::TypeExpr) {
        format_type_expr(f, &ty);
    }
}

fn format_type_expr(f: &mut Formatter, node: &SyntaxNode) {
    let children = non_trivia_children(node);
    let mut wrote_name = false;

    for elem in &children {
        match elem {
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::Ident && !wrote_name => {
                f.write_token(t);
                wrote_name = true;
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::LParen => {
                f.write_str("(");
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::RParen => {
                f.write_str(")");
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::Comma => {
                f.write_str(",");
                f.write_space();
            }
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::TypeExpr => {
                format_type_expr(f, n);
            }
            _ => {}
        }
    }
}

// ── Type definitions ────────────────────────────────────────────

fn format_type_def(f: &mut Formatter, node: &SyntaxNode) {
    let children = non_trivia_children(node);

    // Detect if it's a record type (fields with Colon) or ADT (variants).
    let variants = child_nodes(node, SyntaxKind::Variant);
    let fields = child_nodes(node, SyntaxKind::Field);
    let is_record = variants.is_empty() && !fields.is_empty();

    // Header: [pub] type Name[(params)]
    for elem in &children {
        match elem {
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::Visibility => {
                f.write_str("pub");
                f.write_space();
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::TypeKw => {
                f.write_str("type");
                f.write_space();
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::Ident => {
                f.write_token(t);
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::LParen => {
                // Type parameters
                f.write_str("(");
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::RParen => {
                f.write_str(")");
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::Comma => {
                f.write_str(",");
                f.write_space();
            }
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::TypeParam => {
                format_type_param(f, n);
            }
            _ => {} // handled below
        }
    }

    // Body: { ... }
    if has_child(node, SyntaxKind::LBrace) {
        f.write_space();
        f.write_str("{");
        f.write_newline();
        f.indent();

        if is_record {
            for field in &fields {
                f.write_indent();
                format_record_field(f, field);
                f.write_newline();
            }
        } else {
            for variant in &variants {
                f.write_indent();
                format_variant(f, variant);
                f.write_newline();
            }
        }

        f.dedent();
        f.write_indent();
        f.write_str("}");
    }
}

fn format_type_param(f: &mut Formatter, node: &SyntaxNode) {
    if let Some(t) = first_token(node, SyntaxKind::Ident) {
        f.write_token(&t);
    }
}

fn format_variant(f: &mut Formatter, node: &SyntaxNode) {
    let children = non_trivia_children(node);
    let fields = child_nodes(node, SyntaxKind::Field);
    let mut wrote_name = false;

    for elem in &children {
        match elem {
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::Ident && !wrote_name => {
                f.write_token(t);
                wrote_name = true;
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::LParen => {
                f.write_str("(");
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",");
                        f.write_space();
                    }
                    format_variant_field(f, field);
                }
                // Don't write `(` again, just capture it
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::RParen => {
                f.write_str(")");
            }
            _ => {} // fields handled inline above
        }
    }

    // If no paren tokens were encountered but there are fields, this is handled
    // by the paren token arms above.
}

fn format_variant_field(f: &mut Formatter, node: &SyntaxNode) {
    let children = non_trivia_children(node);
    let has_label = children.iter().any(|e| e.kind() == SyntaxKind::Colon);

    if has_label {
        // Labelled field: `name: Type`
        let mut wrote_name = false;
        for elem in &children {
            match elem {
                NodeOrToken::Token(t) if t.kind() == SyntaxKind::Ident && !wrote_name => {
                    f.write_token(t);
                    wrote_name = true;
                }
                NodeOrToken::Token(t) if t.kind() == SyntaxKind::Colon => {
                    f.write_str(":");
                    f.write_space();
                }
                NodeOrToken::Node(n) if n.kind() == SyntaxKind::TypeExpr => {
                    format_type_expr(f, n);
                }
                _ => {}
            }
        }
    } else {
        // Positional field: just a type
        for elem in &children {
            if let NodeOrToken::Node(n) = elem
                && n.kind() == SyntaxKind::TypeExpr
            {
                format_type_expr(f, n);
            }
        }
    }
}

fn format_record_field(f: &mut Formatter, node: &SyntaxNode) {
    // name: Type
    let children = non_trivia_children(node);
    let mut wrote_name = false;

    for elem in &children {
        match elem {
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::Ident && !wrote_name => {
                f.write_token(t);
                wrote_name = true;
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::Colon => {
                f.write_str(":");
                f.write_space();
            }
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::TypeExpr => {
                format_type_expr(f, n);
            }
            _ => {}
        }
    }
}

// ── Import statements ───────────────────────────────────────────

fn format_import_stmt(f: &mut Formatter, node: &SyntaxNode) {
    let children = non_trivia_children(node);
    let mut first = true;

    for elem in &children {
        match elem {
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::ImportKw => {
                f.write_str("import");
                f.write_space();
                first = false;
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::Ident => {
                f.write_token(t);
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::Dot => {
                f.write_str(".");
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::AsKw => {
                f.write_space();
                f.write_str("as");
                f.write_space();
            }
            _ => {
                if first {
                    first = false;
                }
            }
        }
    }
}

fn format_from_python_import(f: &mut Formatter, node: &SyntaxNode) {
    let children = non_trivia_children(node);

    for elem in &children {
        match elem {
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::FromKw => {
                f.write_str("from");
                f.write_space();
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::PythonKw => {
                f.write_str("python");
                f.write_space();
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::ImportKw => {
                f.write_str("import");
                f.write_space();
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::Ident => {
                f.write_token(t);
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::AsKw => {
                f.write_space();
                f.write_str("as");
                f.write_space();
            }
            _ => {}
        }
    }
}

// ── Expressions ─────────────────────────────────────────────────

fn format_block_expr(f: &mut Formatter, node: &SyntaxNode) {
    f.write_str("{");
    f.write_newline();
    f.indent();

    // Collect body items (non-trivia child nodes, excluding braces).
    let body_items: Vec<SyntaxNode> = node.children().collect();

    for item in &body_items {
        for c in collect_leading_comments(item) {
            f.write_indent();
            f.write_str(&c);
            f.write_newline();
        }

        f.write_indent();
        format_node(f, item);
        f.write_newline();
    }

    // Trailing comments.
    for c in collect_comments_before_closer(node, SyntaxKind::RBrace) {
        f.write_indent();
        f.write_str(&c);
        f.write_newline();
    }

    f.dedent();
    f.write_indent();
    f.write_str("}");
}

fn format_let_stmt(f: &mut Formatter, node: &SyntaxNode) {
    let children = non_trivia_children(node);

    for elem in &children {
        match elem {
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::LetKw => {
                f.write_str("let");
                f.write_space();
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::Ident => {
                f.write_token(t);
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::Eq => {
                f.write_space();
                f.write_str("=");
                f.write_space();
            }
            NodeOrToken::Node(n) => {
                format_node(f, n);
            }
            NodeOrToken::Token(_) => {}
        }
    }
}

fn format_literal_expr(f: &mut Formatter, node: &SyntaxNode) {
    for elem in non_trivia_children(node) {
        if let NodeOrToken::Token(t) = elem {
            f.write_token(&t);
        }
    }
}

fn format_ident_expr(f: &mut Formatter, node: &SyntaxNode) {
    if let Some(t) = first_token(node, SyntaxKind::Ident) {
        f.write_token(&t);
    }
}

fn format_call_expr(f: &mut Formatter, node: &SyntaxNode) {
    // CallExpr wraps: <callee_expr> ArgList
    // The callee can be any expression (IdentExpr, FieldAccessExpr, etc.)
    let children = non_trivia_children(node);

    for elem in &children {
        match elem {
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::ArgList => {
                format_arg_list(f, n);
            }
            NodeOrToken::Node(n) => {
                format_node(f, n);
            }
            NodeOrToken::Token(t) => {
                f.write_token(t);
            }
        }
    }
}

fn format_arg_list(f: &mut Formatter, node: &SyntaxNode) {
    f.write_str("(");

    // Collect all expression children (arguments).
    let args: Vec<SyntaxNode> = node.children().collect();
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            f.write_str(",");
            f.write_space();
        }
        format_node(f, arg);
    }

    f.write_str(")");
}

fn format_binary_expr(f: &mut Formatter, node: &SyntaxNode) {
    let children = non_trivia_children(node);

    for (i, elem) in children.iter().enumerate() {
        match elem {
            NodeOrToken::Token(t) if is_binary_operator(t.kind()) => {
                f.write_space();
                f.write_token(t);
                f.write_space();
            }
            NodeOrToken::Node(n) => {
                format_node(f, n);
            }
            NodeOrToken::Token(t) => {
                if i > 0 {
                    f.write_space();
                }
                f.write_token(t);
            }
        }
    }
}

fn format_binary_like(f: &mut Formatter, node: &SyntaxNode, op_text: &str) {
    let children = non_trivia_children(node);

    for elem in &children {
        match elem {
            NodeOrToken::Token(t)
                if t.kind() == SyntaxKind::Pipe || t.kind() == SyntaxKind::StringConcat =>
            {
                f.write_space();
                f.write_str(op_text);
                f.write_space();
            }
            NodeOrToken::Node(n) => {
                format_node(f, n);
            }
            NodeOrToken::Token(t) => {
                f.write_token(t);
            }
        }
    }
}

fn is_binary_operator(kind: SyntaxKind) -> bool {
    matches!(
        kind,
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
            | SyntaxKind::Pipe
            | SyntaxKind::StringConcat
    )
}

fn format_unary_expr(f: &mut Formatter, node: &SyntaxNode) {
    let children = non_trivia_children(node);

    for elem in &children {
        match elem {
            NodeOrToken::Token(t)
                if t.kind() == SyntaxKind::Minus || t.kind() == SyntaxKind::Bang =>
            {
                f.write_token(t);
                // No space after unary operator.
            }
            NodeOrToken::Node(n) => {
                format_node(f, n);
            }
            NodeOrToken::Token(t) => {
                f.write_token(t);
            }
        }
    }
}

fn format_field_access_expr(f: &mut Formatter, node: &SyntaxNode) {
    let children = non_trivia_children(node);

    for elem in &children {
        match elem {
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::Dot => {
                f.write_str(".");
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::Ident => {
                f.write_token(t);
            }
            NodeOrToken::Node(n) => {
                format_node(f, n);
            }
            NodeOrToken::Token(_) => {}
        }
    }
}

fn format_if_expr(f: &mut Formatter, node: &SyntaxNode) {
    let children = non_trivia_children(node);

    for elem in &children {
        match elem {
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::IfKw => {
                f.write_str("if");
                f.write_space();
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::ElseKw => {
                f.write_space();
                f.write_str("else");
                f.write_space();
            }
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::BlockExpr => {
                f.write_space();
                format_block_expr(f, n);
            }
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::IfExpr => {
                // else if chain — no extra space (already handled by `else` above)
                format_if_expr(f, n);
            }
            NodeOrToken::Node(n) => {
                // condition expression
                format_node(f, n);
            }
            NodeOrToken::Token(_) => {}
        }
    }
}

fn format_match_expr(f: &mut Formatter, node: &SyntaxNode) {
    let children = non_trivia_children(node);
    let arms = child_nodes(node, SyntaxKind::MatchArm);

    for elem in &children {
        match elem {
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::MatchKw => {
                f.write_str("match");
                f.write_space();
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::LBrace => {
                f.write_space();
                f.write_str("{");
                f.write_newline();
                f.indent();

                for arm in &arms {
                    for c in collect_leading_comments(arm) {
                        f.write_indent();
                        f.write_str(&c);
                        f.write_newline();
                    }
                    f.write_indent();
                    format_match_arm(f, arm);
                    f.write_newline();
                }

                // Trailing comments.
                for c in collect_comments_before_closer(node, SyntaxKind::RBrace) {
                    f.write_indent();
                    f.write_str(&c);
                    f.write_newline();
                }

                f.dedent();
                f.write_indent();
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::RBrace => {
                f.write_str("}");
            }
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::MatchArm => {
                // arms already handled above
            }
            NodeOrToken::Node(n) => {
                // subject expression
                format_node(f, n);
            }
            NodeOrToken::Token(_) => {}
        }
    }
}

fn format_match_arm(f: &mut Formatter, node: &SyntaxNode) {
    let children = non_trivia_children(node);

    for elem in &children {
        match elem {
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::Arrow => {
                f.write_space();
                f.write_str("->");
                f.write_space();
            }
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::Guard => {
                f.write_space();
                format_guard(f, n);
            }
            NodeOrToken::Node(n) => {
                format_node(f, n);
            }
            NodeOrToken::Token(t) => {
                f.write_token(t);
            }
        }
    }
}

fn format_guard(f: &mut Formatter, node: &SyntaxNode) {
    let children = non_trivia_children(node);

    for elem in &children {
        match elem {
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::IfKw => {
                f.write_str("if");
                f.write_space();
            }
            NodeOrToken::Node(n) => {
                format_node(f, n);
            }
            NodeOrToken::Token(t) => {
                f.write_token(t);
            }
        }
    }
}

fn format_lambda_expr(f: &mut Formatter, node: &SyntaxNode) {
    let children = non_trivia_children(node);

    for elem in &children {
        match elem {
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::FnKw => {
                f.write_str("fn");
            }
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::ParamList => {
                format_param_list(f, n);
            }
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::ReturnType => {
                f.write_space();
                format_return_type(f, n);
            }
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::BlockExpr => {
                f.write_space();
                format_block_expr(f, n);
            }
            _ => {}
        }
    }
}

fn format_paren_expr(f: &mut Formatter, node: &SyntaxNode) {
    f.write_str("(");
    for child in node.children() {
        format_node(f, &child);
    }
    f.write_str(")");
}

fn format_try_expr(f: &mut Formatter, node: &SyntaxNode) {
    f.write_str("try");
    f.write_space();
    for child in node.children() {
        format_node(f, &child);
    }
}

fn format_list_expr(f: &mut Formatter, node: &SyntaxNode) {
    f.write_str("[");
    let items: Vec<SyntaxNode> = node.children().collect();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            f.write_str(",");
            f.write_space();
        }
        format_node(f, item);
    }
    f.write_str("]");
}

fn format_tuple_expr(f: &mut Formatter, node: &SyntaxNode) {
    f.write_str("#(");
    let items: Vec<SyntaxNode> = node.children().collect();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            f.write_str(",");
            f.write_space();
        }
        format_node(f, item);
    }
    f.write_str(")");
}

fn format_record_expr(f: &mut Formatter, node: &SyntaxNode) {
    // Record constructor: Name(field1, field2)
    // This is essentially a call-like syntax
    let children = non_trivia_children(node);
    for elem in &children {
        match elem {
            NodeOrToken::Node(n) => format_node(f, n),
            NodeOrToken::Token(t) => f.write_token(t),
        }
    }
}

// ── Patterns ────────────────────────────────────────────────────

fn format_wildcard_pat(f: &mut Formatter, _node: &SyntaxNode) {
    f.write_str("_");
}

fn format_ident_pat(f: &mut Formatter, node: &SyntaxNode) {
    if let Some(t) = first_token(node, SyntaxKind::Ident) {
        f.write_token(&t);
    }
}

fn format_literal_pat(f: &mut Formatter, node: &SyntaxNode) {
    for elem in non_trivia_children(node) {
        if let NodeOrToken::Token(t) = elem {
            f.write_token(&t);
        }
    }
}

fn format_constructor_pat(f: &mut Formatter, node: &SyntaxNode) {
    let children = non_trivia_children(node);
    let sub_patterns: Vec<SyntaxNode> = node
        .children()
        .filter(|n| {
            matches!(
                n.kind(),
                SyntaxKind::WildcardPat
                    | SyntaxKind::IdentPat
                    | SyntaxKind::LiteralPat
                    | SyntaxKind::ConstructorPat
                    | SyntaxKind::ListPat
                    | SyntaxKind::TuplePat
            )
        })
        .collect();

    let mut wrote_name = false;
    let has_parens = children.iter().any(|e| e.kind() == SyntaxKind::LParen);

    for elem in &children {
        match elem {
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::Ident && !wrote_name => {
                f.write_token(t);
                wrote_name = true;
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::LParen => {
                f.write_str("(");
                for (i, pat) in sub_patterns.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",");
                        f.write_space();
                    }
                    format_node(f, pat);
                }
                // Don't emit patterns again
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::RParen => {
                f.write_str(")");
            }
            NodeOrToken::Node(_) if has_parens => {
                // Already handled inside LParen arm
            }
            NodeOrToken::Node(n) => {
                format_node(f, n);
            }
            NodeOrToken::Token(_) => {}
        }
    }
}

fn format_list_pat(f: &mut Formatter, node: &SyntaxNode) {
    f.write_str("[");

    let children = non_trivia_children(node);
    let mut first = true;

    for elem in &children {
        match elem {
            NodeOrToken::Token(t)
                if t.kind() == SyntaxKind::LBracket || t.kind() == SyntaxKind::RBracket => {}
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::Comma => {
                f.write_str(",");
                f.write_space();
                first = false;
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::DotDot => {
                if !first {
                    f.write_str(",");
                    f.write_space();
                }
                f.write_str("..");
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::Ident => {
                // rest binding name after `..`
                f.write_token(t);
            }
            NodeOrToken::Node(n) => {
                if !first {
                    // comma already written
                }
                format_node(f, n);
                first = false;
            }
            NodeOrToken::Token(_) => {}
        }
    }

    f.write_str("]");
}

fn format_tuple_pat(f: &mut Formatter, node: &SyntaxNode) {
    f.write_str("#(");
    let items: Vec<SyntaxNode> = node.children().collect();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            f.write_str(",");
            f.write_space();
        }
        format_node(f, item);
    }
    f.write_str(")");
}

// ── Utility ─────────────────────────────────────────────────────

fn format_element(
    f: &mut Formatter,
    elem: &NodeOrToken<SyntaxNode, SyntaxToken>,
    _needs_space: bool,
) {
    match elem {
        NodeOrToken::Token(t) => f.write_token(t),
        NodeOrToken::Node(n) => format_node(f, n),
    }
}
