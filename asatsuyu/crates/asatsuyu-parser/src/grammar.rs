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

/// ```text
/// Expr = LiteralExpr | IdentExpr
/// ```
fn parse_expr(p: &mut Parser<'_>) {
    match p.current() {
        SyntaxKind::IntLit | SyntaxKind::StringLit => parse_literal_expr(p),
        SyntaxKind::Ident => parse_ident_expr(p),
        _ => p.error_and_bump("expected expression"),
    }
}

/// ```text
/// LiteralExpr = INT_LIT | STRING_LIT
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
