//! Recursive descent grammar rules for the Asatsuyu language.
//!
//! Each `parse_*` function corresponds to a grammar production and builds
//! a subtree in the rowan green tree via the [`Parser`](crate::parser::Parser).

use asatsuyu_syntax::SyntaxKind;

use crate::parser::Parser;

/// ```text
/// SourceFile = TopLevel*
/// ```
pub(crate) fn parse_source_file(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::SourceFile);
    while !p.at_eof() {
        parse_top_level(p);
    }
    // Trailing trivia (whitespace / newlines at file end) belongs to SourceFile.
    p.eat_trivia();
    p.finish_node();
}

/// Dispatch a top-level item.
fn parse_top_level(p: &mut Parser<'_>) {
    match p.current() {
        SyntaxKind::PubKw | SyntaxKind::FnKw => parse_fn_def(p),
        SyntaxKind::LetKw | SyntaxKind::TypeKw | SyntaxKind::ImportKw => {
            p.error_recover("not yet implemented");
        }
        _ => p.error_recover("expected item definition"),
    }
}

/// ```text
/// FnDef = Visibility? 'fn' IDENT ParamList ReturnType? BlockExpr
/// ```
fn parse_fn_def(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::FnDef);

    // Optional visibility: `pub`
    if p.at(SyntaxKind::PubKw) {
        parse_visibility(p);
    }

    // `fn` keyword
    p.expect(SyntaxKind::FnKw);

    // Function name
    p.expect(SyntaxKind::Ident);

    // Parameter list
    if p.at(SyntaxKind::LParen) {
        parse_param_list(p);
    } else {
        let span = p.current_span();
        p.diagnostics_mut().push(
            asatsuyu_syntax::Diagnostic::error("expected parameter list", span)
                .with_label(span, "expected `(`"),
        );
    }

    // Optional return type: `-> Type`
    if p.at(SyntaxKind::Arrow) {
        parse_return_type(p);
    }

    // Body block
    if p.at(SyntaxKind::LBrace) {
        parse_block_expr(p);
    } else {
        let span = p.current_span();
        p.diagnostics_mut().push(
            asatsuyu_syntax::Diagnostic::error("expected function body", span)
                .with_label(span, "expected `{`"),
        );
    }

    p.finish_node();
}

/// ```text
/// Visibility = 'pub'
/// ```
fn parse_visibility(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::Visibility);
    p.bump(); // consume `pub`
    p.finish_node();
}

/// ```text
/// ParamList = '(' (Param (',' Param)* ','?)? ')'
/// ```
fn parse_param_list(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::ParamList);
    p.bump(); // consume `(`

    if !p.at(SyntaxKind::RParen) && !p.at_eof() {
        parse_param(p);
        while p.at(SyntaxKind::Comma) {
            p.bump(); // consume `,`
            // Allow trailing comma: stop if `)` follows
            if p.at(SyntaxKind::RParen) {
                break;
            }
            parse_param(p);
        }
    }

    p.expect(SyntaxKind::RParen);
    p.finish_node();
}

/// ```text
/// Param = IDENT ':' IDENT
/// ```
fn parse_param(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::Param);
    p.expect(SyntaxKind::Ident); // parameter name
    p.expect(SyntaxKind::Colon);
    p.expect(SyntaxKind::Ident); // type (just an identifier for now)
    p.finish_node();
}

/// ```text
/// ReturnType = '->' IDENT
/// ```
fn parse_return_type(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::ReturnType);
    p.bump(); // consume `->`
    p.expect(SyntaxKind::Ident); // return type
    p.finish_node();
}

/// ```text
/// BlockExpr = '{' Expr* '}'
/// ```
fn parse_block_expr(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::BlockExpr);
    p.bump(); // consume `{`

    while !p.at(SyntaxKind::RBrace) && !p.at_eof() {
        parse_expr(p);
    }

    p.expect(SyntaxKind::RBrace);
    p.finish_node();
}

// ── Expression parsing (Pratt parser) ───────────────────────────

/// ```text
/// Expr = Atom | PrefixExpr | InfixExpr | PostfixExpr | IfExpr
/// ```
fn parse_expr(p: &mut Parser<'_>) {
    parse_expr_bp(p, 0);
}

/// Binding power for infix binary operators.
///
/// Returns `Some((left_bp, right_bp))` for infix operators, `None` otherwise.
/// Left-associative operators have `right_bp = left_bp + 1`.
fn infix_binding_power(op: SyntaxKind) -> Option<(u8, u8)> {
    match op {
        SyntaxKind::PipePipe => Some((1, 2)),
        SyntaxKind::AmpAmp => Some((3, 4)),
        SyntaxKind::EqEq
        | SyntaxKind::BangEq
        | SyntaxKind::Lt
        | SyntaxKind::LtEq
        | SyntaxKind::Gt
        | SyntaxKind::GtEq => Some((5, 6)),
        SyntaxKind::Pipe | SyntaxKind::Plus | SyntaxKind::Minus => Some((7, 8)),
        SyntaxKind::Star | SyntaxKind::Slash | SyntaxKind::Percent => Some((9, 10)),
        _ => None,
    }
}

/// Binding power for prefix unary operators.
///
/// Returns `Some(right_bp)` for prefix operators, `None` otherwise.
fn prefix_binding_power(op: SyntaxKind) -> Option<u8> {
    match op {
        SyntaxKind::Minus | SyntaxKind::Bang => Some(11),
        _ => None,
    }
}

/// Binding power for postfix operators (call expressions).
///
/// Returns `Some(left_bp)` for postfix operators, `None` otherwise.
fn postfix_binding_power(op: SyntaxKind) -> Option<u8> {
    match op {
        SyntaxKind::LParen => Some(13),
        _ => None,
    }
}

/// Parse an expression with a minimum binding power.
///
/// This is the heart of the Pratt parser. It first parses an atom (primary)
/// or prefix expression, then loops consuming infix and postfix operators
/// as long as their binding power exceeds `min_bp`.
fn parse_expr_bp(p: &mut Parser<'_>, min_bp: u8) {
    let checkpoint = p.checkpoint();

    // ── Prefix / Atom ──
    match p.current() {
        // Prefix unary operators: `-expr`, `!expr`
        kind if prefix_binding_power(kind).is_some() => {
            let right_bp = prefix_binding_power(kind).expect("checked above");
            p.start_node(SyntaxKind::UnaryExpr);
            p.bump(); // consume operator
            parse_expr_bp(p, right_bp);
            p.finish_node();
        }

        // Parenthesized expression: `(expr)`
        SyntaxKind::LParen => {
            p.start_node(SyntaxKind::ParenExpr);
            p.bump(); // consume `(`
            parse_expr_bp(p, 0); // reset binding power inside parens
            p.expect(SyntaxKind::RParen);
            p.finish_node();
        }

        // If expression
        SyntaxKind::IfKw => {
            parse_if_expr(p);
        }

        // Literal atoms
        SyntaxKind::IntLit
        | SyntaxKind::FloatLit
        | SyntaxKind::StringLit
        | SyntaxKind::TrueKw
        | SyntaxKind::FalseKw => {
            parse_literal_expr(p);
        }

        // Identifier atom
        SyntaxKind::Ident => {
            parse_ident_expr(p);
        }

        // Error: not a valid expression start
        _ => {
            p.error_and_bump("expected expression");
            return;
        }
    }

    // ── Postfix and Infix loop ──
    loop {
        let op = p.current();

        // Postfix: call expression `expr(args)`
        if let Some(left_bp) = postfix_binding_power(op) {
            if left_bp < min_bp {
                break;
            }
            p.start_node_at(checkpoint, SyntaxKind::CallExpr);
            parse_arg_list(p);
            p.finish_node();
            continue;
        }

        // Infix: binary expression `expr op expr` or pipeline `expr |> expr`
        if let Some((left_bp, right_bp)) = infix_binding_power(op) {
            if left_bp < min_bp {
                break;
            }
            let node_kind = if op == SyntaxKind::Pipe {
                SyntaxKind::PipelineExpr
            } else {
                SyntaxKind::BinaryExpr
            };
            p.start_node_at(checkpoint, node_kind);
            p.bump(); // consume the operator token
            parse_expr_bp(p, right_bp);
            p.finish_node();
            continue;
        }

        // Not a postfix or infix operator — stop
        break;
    }
}

/// ```text
/// ArgList = '(' (Expr (',' Expr)* ','?)? ')'
/// ```
fn parse_arg_list(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::ArgList);
    p.bump(); // consume `(`

    if !p.at(SyntaxKind::RParen) && !p.at_eof() {
        parse_expr_bp(p, 0);
        while p.at(SyntaxKind::Comma) {
            p.bump(); // consume `,`
            // Allow trailing comma: stop if `)` follows
            if p.at(SyntaxKind::RParen) {
                break;
            }
            parse_expr_bp(p, 0);
        }
    }

    p.expect(SyntaxKind::RParen);
    p.finish_node();
}

/// ```text
/// IfExpr = 'if' Expr BlockExpr ('else' (IfExpr | BlockExpr))?
/// ```
fn parse_if_expr(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::IfExpr);
    p.bump(); // consume `if`

    // Condition: any expression (LBrace has no bp, so Pratt loop stops naturally)
    parse_expr_bp(p, 0);

    // Then-body: must be a block
    if p.at(SyntaxKind::LBrace) {
        parse_block_expr(p);
    } else {
        let span = p.current_span();
        p.diagnostics_mut().push(
            asatsuyu_syntax::Diagnostic::error("expected block after `if` condition", span)
                .with_label(span, "expected `{`"),
        );
    }

    // Optional else clause
    if p.at(SyntaxKind::ElseKw) {
        p.bump(); // consume `else`
        if p.at(SyntaxKind::IfKw) {
            // else-if chain
            parse_if_expr(p);
        } else if p.at(SyntaxKind::LBrace) {
            parse_block_expr(p);
        } else {
            let span = p.current_span();
            p.diagnostics_mut().push(
                asatsuyu_syntax::Diagnostic::error("expected block or `if` after `else`", span)
                    .with_label(span, "expected `{` or `if`"),
            );
        }
    }

    p.finish_node();
}

/// ```text
/// LiteralExpr = INT_LIT | FLOAT_LIT | STRING_LIT | TRUE | FALSE
/// ```
fn parse_literal_expr(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::LiteralExpr);
    p.bump();
    p.finish_node();
}

/// ```text
/// IdentExpr = IDENT
/// ```
fn parse_ident_expr(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::IdentExpr);
    p.bump();
    p.finish_node();
}
