//! Recursive descent grammar rules for the Asatsuyu language.
//!
//! Each `parse_*` function corresponds to a grammar production and builds
//! a subtree in the rowan green tree via the [`Parser`](crate::parser::Parser).

use asatsuyu_syntax::SyntaxKind;

use crate::parser::{Parser, TOP_LEVEL_RECOVERY, TokenSet};

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
        SyntaxKind::FnKw => parse_fn_def(p),
        SyntaxKind::TypeKw => parse_type_def(p),
        SyntaxKind::PubKw => match p.nth(1) {
            SyntaxKind::TypeKw => parse_type_def(p),
            _ => parse_fn_def(p),
        },
        SyntaxKind::ImportKw => parse_import(p),
        SyntaxKind::FromKw => parse_from_python_import(p),
        SyntaxKind::LetKw => {
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

    // Function name — recover towards `(` if missing
    if p.at(SyntaxKind::Ident) {
        p.bump();
    } else {
        p.error_recover_until(
            "expected function name",
            TokenSet::new(&[SyntaxKind::LParen, SyntaxKind::LBrace, SyntaxKind::Arrow]),
        );
    }

    // Parameter list — recover towards `{` or `->` if `(` missing
    if p.at(SyntaxKind::LParen) {
        parse_param_list(p);
    } else {
        p.error_recover_until(
            "expected parameter list",
            TokenSet::new(&[SyntaxKind::LBrace, SyntaxKind::Arrow]),
        );
    }

    // Optional return type: `-> Type`
    if p.at(SyntaxKind::Arrow) {
        parse_return_type(p);
    }

    // Body block — recover towards next top-level item if `{` missing
    if p.at(SyntaxKind::LBrace) {
        parse_block_expr(p);
    } else {
        p.error_recover_until("expected function body", TokenSet::EMPTY);
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

// ── Type definition parsing ──────────────────────────────────────

/// ```text
/// TypeDef = Visibility? 'type' IDENT TypeParams? '{' TypeBody '}'
/// ```
fn parse_type_def(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::TypeDef);

    // Optional visibility: `pub`
    if p.at(SyntaxKind::PubKw) {
        parse_visibility(p);
    }

    // `type` keyword
    p.expect(SyntaxKind::TypeKw);

    // Type name
    p.expect(SyntaxKind::Ident);

    // Optional type parameters: `(a, b)`
    if p.at(SyntaxKind::LParen) {
        parse_type_param_list(p);
    }

    // Body: `{ ... }`
    if p.at(SyntaxKind::LBrace) {
        p.bump(); // consume `{`
        parse_type_body(p);
        p.expect(SyntaxKind::RBrace);
    } else {
        p.error_recover_until("expected type body", TokenSet::EMPTY);
    }

    p.finish_node();
}

/// ```text
/// TypeParams = '(' TypeParam (',' TypeParam)* ','? ')'
/// ```
fn parse_type_param_list(p: &mut Parser<'_>) {
    p.bump(); // consume `(`

    if !p.at(SyntaxKind::RParen) && !p.at_eof() {
        parse_type_param(p);
        while p.at(SyntaxKind::Comma) {
            p.bump(); // consume `,`
            if p.at(SyntaxKind::RParen) {
                break;
            }
            parse_type_param(p);
        }
    }

    p.expect(SyntaxKind::RParen);
}

/// ```text
/// TypeParam = IDENT
/// ```
fn parse_type_param(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::TypeParam);
    p.expect(SyntaxKind::Ident);
    p.finish_node();
}

/// Dispatch type body as record fields or ADT variants.
///
/// Detection: if the first non-trivia token after `{` is `Ident` followed
/// by `Colon`, treat as Go-style record fields. Otherwise, ADT variants.
fn parse_type_body(p: &mut Parser<'_>) {
    if p.at(SyntaxKind::RBrace) {
        return; // empty type body
    }

    let is_record = p.at(SyntaxKind::Ident) && p.nth(1) == SyntaxKind::Colon;

    if is_record {
        while !p.at(SyntaxKind::RBrace) && !p.at_eof() && !p.at_any(TOP_LEVEL_RECOVERY) {
            let prev = p.pos();
            if p.at(SyntaxKind::Ident) {
                parse_record_field(p);
            } else {
                p.error_and_bump("expected field definition");
            }
            debug_assert!(p.pos() > prev, "parse_type_body: no progress (record)");
        }
    } else {
        while !p.at(SyntaxKind::RBrace) && !p.at_eof() && !p.at_any(TOP_LEVEL_RECOVERY) {
            let prev = p.pos();
            if p.at(SyntaxKind::Ident) {
                parse_variant(p);
            } else {
                p.error_and_bump("expected variant definition");
            }
            debug_assert!(p.pos() > prev, "parse_type_body: no progress (variant)");
        }
    }
}

/// ```text
/// Field = IDENT ':' TypeExpr
/// ```
fn parse_record_field(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::Field);
    p.expect(SyntaxKind::Ident); // field name
    p.expect(SyntaxKind::Colon);
    parse_type_expr(p);
    p.finish_node();
}

/// ```text
/// Variant = IDENT VariantArgs?
/// VariantArgs = '(' VarField (',' VarField)* ','? ')'
/// ```
fn parse_variant(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::Variant);
    p.expect(SyntaxKind::Ident); // variant name

    // Optional fields: `(field, field, ...)`
    if p.at(SyntaxKind::LParen) {
        p.bump(); // consume `(`
        if !p.at(SyntaxKind::RParen) && !p.at_eof() {
            parse_variant_field(p);
            while p.at(SyntaxKind::Comma) {
                p.bump(); // consume `,`
                if p.at(SyntaxKind::RParen) {
                    break;
                }
                parse_variant_field(p);
            }
        }
        p.expect(SyntaxKind::RParen);
    }

    p.finish_node();
}

/// ```text
/// VarField = (IDENT ':')? TypeExpr
/// ```
fn parse_variant_field(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::Field);
    // Lookahead: Ident + Colon → labelled field
    if p.at(SyntaxKind::Ident) && p.nth(1) == SyntaxKind::Colon {
        p.bump(); // consume label
        p.bump(); // consume `:`
    }
    parse_type_expr(p);
    p.finish_node();
}

/// ```text
/// TypeExpr = IDENT TypeArgs?
/// TypeArgs = '(' TypeExpr (',' TypeExpr)* ','? ')'
/// ```
fn parse_type_expr(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::TypeExpr);
    p.expect(SyntaxKind::Ident); // type name

    // Optional type arguments: `(Type, Type, ...)`
    if p.at(SyntaxKind::LParen) {
        p.bump(); // consume `(`
        if !p.at(SyntaxKind::RParen) && !p.at_eof() {
            parse_type_expr(p); // recursive
            while p.at(SyntaxKind::Comma) {
                p.bump(); // consume `,`
                if p.at(SyntaxKind::RParen) {
                    break;
                }
                parse_type_expr(p);
            }
        }
        p.expect(SyntaxKind::RParen);
    }

    p.finish_node();
}

// ── Import statement parsing ────────────────────────────────────

/// ```text
/// ImportStmt = 'import' IDENT ('.' IDENT)* ('as' IDENT)?
/// ```
fn parse_import(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::ImportStmt);
    p.bump(); // consume `import`

    // Module path: at least one identifier
    if !p.expect(SyntaxKind::Ident) {
        p.error_recover_until("expected module name", TokenSet::EMPTY);
        p.finish_node();
        return;
    }

    // Additional path segments: `.ident`
    while p.at(SyntaxKind::Dot) {
        p.bump(); // consume `.`
        if !p.expect(SyntaxKind::Ident) {
            p.error_recover_until("expected module name after `.`", TokenSet::EMPTY);
            p.finish_node();
            return;
        }
    }

    // Optional alias: `as name`
    if p.at(SyntaxKind::AsKw) {
        p.bump(); // consume `as`
        if !p.expect(SyntaxKind::Ident) {
            p.error_recover_until("expected alias name after `as`", TokenSet::EMPTY);
        }
    }

    p.finish_node();
}

// ── Python FFI import parsing ──────────────────────────────────

/// ```text
/// FromPythonImportStmt = 'from' 'python' 'import' IDENT ('as' IDENT)?
/// ```
fn parse_from_python_import(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::FromPythonImportStmt);
    p.bump(); // consume `from`

    // Expect `python` keyword
    if !p.at(SyntaxKind::PythonKw) {
        p.error_recover_until("expected `python` after `from`", TokenSet::EMPTY);
        p.finish_node();
        return;
    }
    p.bump(); // consume `python`

    // Expect `import` keyword
    if !p.expect(SyntaxKind::ImportKw) {
        p.error_recover_until("expected `import` after `from python`", TokenSet::EMPTY);
        p.finish_node();
        return;
    }

    // Module name: a single identifier
    if !p.expect(SyntaxKind::Ident) {
        p.error_recover_until("expected module name after `import`", TokenSet::EMPTY);
        p.finish_node();
        return;
    }

    // Optional alias: `as name`
    if p.at(SyntaxKind::AsKw) {
        p.bump(); // consume `as`
        if !p.expect(SyntaxKind::Ident) {
            p.error_recover_until("expected alias name after `as`", TokenSet::EMPTY);
        }
    }

    p.finish_node();
}

// ── Function definition helpers ─────────────────────────────────

/// ```text
/// ParamList = '(' (Param (',' Param)* ','?)? ')'
/// ```
fn parse_param_list(p: &mut Parser<'_>) {
    parse_param_list_with_mode(p, true);
}

fn parse_lambda_param_list(p: &mut Parser<'_>) {
    parse_param_list_with_mode(p, false);
}

fn parse_param_list_with_mode(p: &mut Parser<'_>, require_type_ann: bool) {
    p.start_node(SyntaxKind::ParamList);
    p.bump(); // consume `(`

    while !p.at(SyntaxKind::RParen) && !p.at_eof() {
        let prev = p.pos();
        parse_param(p, require_type_ann);
        // Consume comma separator, or break if none
        if p.at(SyntaxKind::Comma) {
            p.bump();
        } else if !p.at(SyntaxKind::RParen) {
            // Not comma, not `)` — stuck
            if p.pos() == prev {
                p.error_and_bump("unexpected token in parameter list");
            }
            break;
        }
    }

    p.expect(SyntaxKind::RParen);
    p.finish_node();
}

/// ```text
/// Param = IDENT (':' TypeExpr)?
/// ```
fn parse_param(p: &mut Parser<'_>, require_type_ann: bool) {
    const PARAM_FOLLOW: TokenSet =
        TokenSet::new(&[SyntaxKind::Comma, SyntaxKind::RParen, SyntaxKind::LBrace]);

    p.start_node(SyntaxKind::Param);

    // Parameter name
    if !p.expect(SyntaxKind::Ident) {
        p.error_recover_until("expected parameter name", PARAM_FOLLOW);
        p.finish_node();
        return;
    }

    // Optional type annotation: `: Type`
    if p.at(SyntaxKind::Colon) {
        p.bump(); // consume `:`
        if p.at(SyntaxKind::Ident) {
            parse_type_expr(p);
        } else {
            p.error_recover_until("expected parameter type", PARAM_FOLLOW);
        }
    } else if require_type_ann {
        p.error_recover_until("expected `:` after parameter name", PARAM_FOLLOW);
    }

    p.finish_node();
}

/// ```text
/// ReturnType = '->' TypeExpr
/// ```
fn parse_return_type(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::ReturnType);
    p.bump(); // consume `->`
    if p.at(SyntaxKind::Ident) {
        parse_type_expr(p);
    } else {
        p.error_recover_until("expected return type", TokenSet::new(&[SyntaxKind::LBrace]));
    }
    p.finish_node();
}

/// ```text
/// BlockExpr = '{' (LetStmt | Expr)* '}'
/// ```
fn parse_block_expr(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::BlockExpr);
    p.bump(); // consume `{`

    while !p.at(SyntaxKind::RBrace) && !p.at_eof() {
        let prev = p.pos();
        if p.at(SyntaxKind::LetKw) {
            parse_let_stmt(p);
        } else {
            parse_expr(p);
        }
        if p.pos() == prev {
            p.error_and_bump("unexpected token in block");
        }
    }

    p.expect(SyntaxKind::RBrace);
    p.finish_node();
}

/// ```text
/// LetStmt = 'let' IDENT '=' Expr
/// ```
fn parse_let_stmt(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::LetStmt);
    p.bump(); // consume `let`

    if p.at(SyntaxKind::Ident) {
        p.bump(); // binding name
    } else {
        p.error_recover_until(
            "expected binding name",
            TokenSet::new(&[SyntaxKind::Eq, SyntaxKind::RBrace]),
        );
    }

    p.expect(SyntaxKind::Eq);
    parse_expr(p);

    p.finish_node();
}

/// ```text
/// LambdaExpr = 'fn' ParamList ReturnType? BlockExpr
/// ```
fn parse_lambda_expr(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::LambdaExpr);
    p.bump(); // consume `fn`

    if p.at(SyntaxKind::LParen) {
        parse_lambda_param_list(p);
    } else {
        p.error_recover_until(
            "expected parameter list",
            TokenSet::new(&[SyntaxKind::LBrace, SyntaxKind::Arrow]),
        );
    }

    if p.at(SyntaxKind::Arrow) {
        parse_return_type(p);
    }

    if p.at(SyntaxKind::LBrace) {
        parse_block_expr(p);
    } else {
        p.error_recover_until("expected lambda body", TokenSet::EMPTY);
    }

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
        SyntaxKind::Pipe | SyntaxKind::Plus | SyntaxKind::Minus | SyntaxKind::StringConcat => {
            Some((7, 8))
        }
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

/// Binding power for postfix operators (call and field access).
///
/// Returns `Some(left_bp)` for postfix operators, `None` otherwise.
/// Dot (field access) binds tighter than `LParen` (call) so that
/// `pathlib.Path("x")` parses as `Call(FieldAccess(pathlib, Path), args)`.
fn postfix_binding_power(op: SyntaxKind) -> Option<u8> {
    match op {
        SyntaxKind::LParen => Some(13),
        SyntaxKind::Dot => Some(15),
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

        // Match expression
        SyntaxKind::MatchKw => {
            parse_match_expr(p);
        }

        // Lambda expression: `fn(params) { body }`
        SyntaxKind::FnKw => {
            parse_lambda_expr(p);
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

        // Postfix: call expression `expr(args)` or field access `expr.field`
        if let Some(left_bp) = postfix_binding_power(op) {
            if left_bp < min_bp {
                break;
            }
            match op {
                SyntaxKind::Dot => {
                    p.start_node_at(checkpoint, SyntaxKind::FieldAccessExpr);
                    p.bump(); // consume `.`
                    p.expect(SyntaxKind::Ident);
                    p.finish_node();
                }
                SyntaxKind::LParen => {
                    p.start_node_at(checkpoint, SyntaxKind::CallExpr);
                    parse_arg_list(p);
                    p.finish_node();
                }
                _ => break,
            }
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

    while !p.at(SyntaxKind::RParen) && !p.at_eof() {
        let prev = p.pos();
        parse_expr_bp(p, 0);
        if p.at(SyntaxKind::Comma) {
            p.bump();
        } else if !p.at(SyntaxKind::RParen) {
            if p.pos() == prev {
                p.error_and_bump("unexpected token in argument list");
            }
            break;
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
        p.error_recover_until(
            "expected block after `if` condition",
            TokenSet::new(&[SyntaxKind::ElseKw]),
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
            p.error_recover_until("expected block or `if` after `else`", TokenSet::EMPTY);
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

// ── Match expression and pattern parsing ────────────────────────

/// Returns `true` if the current token can start a pattern.
fn at_pattern_start(p: &Parser<'_>) -> bool {
    matches!(
        p.current(),
        SyntaxKind::Underscore
            | SyntaxKind::IntLit
            | SyntaxKind::FloatLit
            | SyntaxKind::StringLit
            | SyntaxKind::TrueKw
            | SyntaxKind::FalseKw
            | SyntaxKind::Ident
            | SyntaxKind::LBracket
    )
}

/// ```text
/// Pattern = WildcardPat | LiteralPat | ConstructorPat | IdentPat | ListPat
/// ```
fn parse_pattern(p: &mut Parser<'_>) {
    match p.current() {
        SyntaxKind::Underscore => parse_wildcard_pat(p),

        SyntaxKind::IntLit
        | SyntaxKind::FloatLit
        | SyntaxKind::StringLit
        | SyntaxKind::TrueKw
        | SyntaxKind::FalseKw => parse_literal_pat(p),

        SyntaxKind::Ident => {
            if p.nth(1) == SyntaxKind::LParen {
                parse_constructor_pat(p);
            } else if p.current_text().starts_with(char::is_uppercase) {
                // Nullary constructor: e.g., `None`, `Nil`
                p.start_node(SyntaxKind::ConstructorPat);
                p.bump();
                p.finish_node();
            } else {
                parse_ident_pat(p);
            }
        }

        SyntaxKind::LBracket => parse_list_pat(p),

        _ => p.error_and_bump("expected pattern"),
    }
}

/// ```text
/// WildcardPat = '_'
/// ```
fn parse_wildcard_pat(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::WildcardPat);
    p.bump(); // consume `_`
    p.finish_node();
}

/// ```text
/// LiteralPat = INT_LIT | FLOAT_LIT | STRING_LIT | TRUE | FALSE
/// ```
fn parse_literal_pat(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::LiteralPat);
    p.bump(); // consume the literal token
    p.finish_node();
}

/// ```text
/// IdentPat = IDENT
/// ```
fn parse_ident_pat(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::IdentPat);
    p.bump(); // consume the identifier
    p.finish_node();
}

/// ```text
/// ConstructorPat = IDENT '(' (Pattern (',' Pattern)* ','?)? ')'
/// ```
fn parse_constructor_pat(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::ConstructorPat);
    p.bump(); // consume constructor name
    p.bump(); // consume `(`

    while !p.at(SyntaxKind::RParen) && !p.at_eof() {
        let prev = p.pos();
        parse_pattern(p);
        if p.at(SyntaxKind::Comma) {
            p.bump();
        } else if !p.at(SyntaxKind::RParen) {
            if p.pos() == prev {
                p.error_and_bump("unexpected token in constructor pattern");
            }
            break;
        }
    }

    p.expect(SyntaxKind::RParen);
    p.finish_node();
}

/// ```text
/// ListPat = '[' (Pattern (',' Pattern)* ','?)? ('..' IDENT?)? ']'
/// ```
fn parse_list_pat(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::ListPat);
    p.bump(); // consume `[`

    while !p.at(SyntaxKind::RBracket) && !p.at(SyntaxKind::DotDot) && !p.at_eof() {
        let prev = p.pos();
        parse_pattern(p);
        if p.at(SyntaxKind::Comma) {
            p.bump();
        } else if !p.at(SyntaxKind::RBracket) && !p.at(SyntaxKind::DotDot) {
            if p.pos() == prev {
                p.error_and_bump("unexpected token in list pattern");
            }
            break;
        }
    }

    // Optional rest pattern: `..` or `..rest`
    if p.at(SyntaxKind::DotDot) {
        p.bump(); // consume `..`
        if p.at(SyntaxKind::Ident) {
            p.bump(); // consume optional rest binding name
        }
    }

    p.expect(SyntaxKind::RBracket);
    p.finish_node();
}

/// ```text
/// Guard = 'if' Expr
/// ```
fn parse_guard(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::Guard);
    p.bump(); // consume `if`
    parse_expr_bp(p, 0); // guard condition; stops at `->` naturally
    p.finish_node();
}

/// ```text
/// MatchArm = Pattern Guard? '->' Expr
/// ```
fn parse_match_arm(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::MatchArm);
    parse_pattern(p);

    // Optional guard: `if condition`
    if p.at(SyntaxKind::IfKw) {
        parse_guard(p);
    }

    // Arrow separating pattern from body — recover towards `->` if missing
    let found_arrow = if p.at(SyntaxKind::Arrow) {
        p.bump();
        true
    } else {
        p.error_recover_until("expected `->` after pattern", TokenSet::new(&[SyntaxKind::Arrow]));
        // If we recovered to `->`, consume it
        if p.at(SyntaxKind::Arrow) {
            p.bump();
            true
        } else {
            false
        }
    };

    // Arm body expression — only parse if arrow was found
    if found_arrow {
        parse_expr_bp(p, 0);
    }

    p.finish_node();
}

/// ```text
/// MatchExpr = 'match' Expr '{' MatchArm* '}'
/// ```
fn parse_match_expr(p: &mut Parser<'_>) {
    p.start_node(SyntaxKind::MatchExpr);
    p.bump(); // consume `match`

    // Subject expression; stops at `{` naturally (no infix bp)
    parse_expr_bp(p, 0);

    // Arm list: `{ arm* }`
    if p.at(SyntaxKind::LBrace) {
        p.bump(); // consume `{`
    } else {
        let span = p.current_span();
        p.diagnostics_mut().push(
            asatsuyu_syntax::Diagnostic::error("expected block after match subject", span)
                .with_label(span, "expected `{`"),
        );
    }

    while !p.at(SyntaxKind::RBrace) && !p.at_eof() {
        let prev = p.pos();
        if at_pattern_start(p) {
            parse_match_arm(p);
        } else {
            p.error_and_bump("expected pattern");
        }
        if p.pos() == prev {
            p.error_and_bump("unexpected token in match");
            break;
        }
    }

    p.expect(SyntaxKind::RBrace);
    p.finish_node();
}
