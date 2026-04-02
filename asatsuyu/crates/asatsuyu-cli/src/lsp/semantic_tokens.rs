//! Semantic token generation from the THIR.
//!
//! Walks the typed IR to produce a sorted list of [`RawSemanticToken`]s,
//! then delta-encodes them into the LSP wire format.

use asatsuyu_hir::ffi::FfiTrustLevel;
use asatsuyu_hir::{DefKind, HirImportKind};
use asatsuyu_syntax::LineIndex;
use asatsuyu_ty::{ThirExpr, ThirModule, ThirPattern};
use tower_lsp::lsp_types::{SemanticToken, SemanticTokenModifier, SemanticTokenType};

// ── Token legend indices ───────────────────────────────────────────

// Token types — order must match TOKEN_TYPES in server.rs.
const TT_NAMESPACE: u32 = 0;
const TT_TYPE: u32 = 1;
const TT_ENUM_MEMBER: u32 = 2;
// Reserved for future type parameter support (not yet tracked in symbol table).
#[allow(dead_code)]
const TT_TYPE_PARAMETER: u32 = 3;
const TT_PARAMETER: u32 = 4;
const TT_VARIABLE: u32 = 5;
const TT_PROPERTY: u32 = 6;
const TT_FUNCTION: u32 = 7;

// Token modifiers — bitmask positions must match TOKEN_MODIFIERS in server.rs.
const TM_DECLARATION: u32 = 1 << 0;
const TM_READONLY: u32 = 1 << 1;
const TM_ASYNC: u32 = 1 << 2;
const TM_MODIFICATION: u32 = 1 << 3;
const TM_DEFAULT_LIBRARY: u32 = 1 << 4;
const TM_FFI: u32 = 1 << 5;
const TM_CHECKED: u32 = 1 << 6;
const TM_VERIFIED: u32 = 1 << 7;

/// The token legend types, in order. Index positions matter.
pub(super) fn token_types() -> Vec<SemanticTokenType> {
    vec![
        SemanticTokenType::NAMESPACE,      // 0
        SemanticTokenType::TYPE,           // 1
        SemanticTokenType::ENUM_MEMBER,    // 2
        SemanticTokenType::TYPE_PARAMETER, // 3
        SemanticTokenType::PARAMETER,      // 4
        SemanticTokenType::VARIABLE,       // 5
        SemanticTokenType::PROPERTY,       // 6
        SemanticTokenType::FUNCTION,       // 7
    ]
}

/// The token legend modifiers, in order. Bit positions matter.
pub(super) fn token_modifiers() -> Vec<SemanticTokenModifier> {
    vec![
        SemanticTokenModifier::DECLARATION,     // bit 0
        SemanticTokenModifier::READONLY,        // bit 1
        SemanticTokenModifier::ASYNC,           // bit 2
        SemanticTokenModifier::MODIFICATION,    // bit 3
        SemanticTokenModifier::DEFAULT_LIBRARY, // bit 4
        SemanticTokenModifier::new("ffi"),      // bit 5
        SemanticTokenModifier::new("checked"),  // bit 6
        SemanticTokenModifier::new("verified"), // bit 7
    ]
}

// ── Raw token ──────────────────────────────────────────────────────

/// A semantic token before delta-encoding.
struct RawToken {
    start: u32,
    length: u32,
    token_type: u32,
    modifiers: u32,
}

// ── Collection ─────────────────────────────────────────────────────

/// Collect semantic tokens from a type-checked module and encode them for LSP.
pub(super) fn collect_and_encode(
    module: &ThirModule,
    line_index: &LineIndex,
) -> Vec<SemanticToken> {
    let mut tokens = Vec::new();
    collect_module_tokens(module, &mut tokens);
    tokens.sort_by_key(|t| t.start);
    encode(&tokens, line_index)
}

fn collect_module_tokens(module: &ThirModule, out: &mut Vec<RawToken>) {
    // Import declarations.
    for imp in &module.imports {
        let def = module.symbol_table.get(imp.def_id);
        let mut modifiers = TM_DECLARATION;
        if let HirImportKind::Python { module_name } = &imp.kind {
            modifiers |= TM_FFI;
            if let Some(ffi_mod) = module.ffi_modules.get(module_name.as_str()) {
                modifiers |= trust_level_modifier(ffi_mod.trust_level);
            }
        }
        push_def_token(out, def, TT_NAMESPACE, modifiers);
    }

    // Custom type definitions.
    for ct in &module.custom_types {
        let def = module.symbol_table.get(ct.def_id);
        push_def_token(out, def, TT_TYPE, TM_DECLARATION);

        // Type parameters are not tracked in the symbol table as DefIds,
        // so we skip them for now (they're handled by TextMate grammar).

        // Constructors.
        for variant in &ct.variants {
            let ctor_def = module.symbol_table.get(variant.def_id);
            push_def_token(out, ctor_def, TT_ENUM_MEMBER, TM_DECLARATION);
        }
    }

    // Function definitions.
    for func in &module.functions {
        let def = module.symbol_table.get(func.def_id);
        let mut modifiers = TM_DECLARATION;
        if func.is_async {
            modifiers |= TM_ASYNC;
        }
        push_def_token(out, def, TT_FUNCTION, modifiers);

        // Parameters.
        for param in &func.params {
            let pdef = module.symbol_table.get(param.def_id);
            push_def_token(out, pdef, TT_PARAMETER, TM_DECLARATION);
        }

        // Body.
        collect_expr_tokens(&func.body, &module.symbol_table, &module.ffi_modules, out);
    }
}

fn collect_expr_tokens(
    expr: &ThirExpr,
    st: &asatsuyu_hir::SymbolTable,
    ffi_modules: &std::collections::HashMap<smol_str::SmolStr, asatsuyu_hir::ffi::FfiModule>,
    out: &mut Vec<RawToken>,
) {
    match expr {
        ThirExpr::Var { def_id, span, .. } => {
            let def = st.get(*def_id);
            let (tt, mods) = classify_def(def, ffi_modules);
            push_span_token(out, *span, tt, mods);
        }
        ThirExpr::Let { binding, value, is_mutable, .. } => {
            let def = st.get(*binding);
            let mut mods = TM_DECLARATION;
            if !is_mutable {
                mods |= TM_READONLY;
            }
            push_def_token(out, def, TT_VARIABLE, mods);
            collect_expr_tokens(value, st, ffi_modules, out);
        }
        ThirExpr::Assign { target, value, target_span, .. } => {
            // The assignment target gets `modification` modifier.
            let def = st.get(*target);
            let span =
                asatsuyu_syntax::Span::new(def.span.file_id, target_span.start, target_span.end);
            push_span_token(out, span, TT_VARIABLE, TM_MODIFICATION);
            collect_expr_tokens(value, st, ffi_modules, out);
        }
        ThirExpr::Call { func, args, .. } => {
            collect_expr_tokens(func, st, ffi_modules, out);
            for arg in args {
                collect_expr_tokens(arg, st, ffi_modules, out);
            }
        }
        ThirExpr::FieldAccess { receiver, field, span, .. } => {
            collect_expr_tokens(receiver, st, ffi_modules, out);
            // The field name span: receiver ends, then `.`, then field name.
            // We compute field span from the main span end minus field length.
            #[allow(clippy::cast_possible_truncation)]
            let field_len = field.len() as u32;
            let field_start = span.end.saturating_sub(field_len);
            if field_start < span.end {
                push_raw(out, field_start, field_len, TT_PROPERTY, 0);
            }
        }
        ThirExpr::Lambda { params, body, .. } => {
            for param in params {
                let pdef = st.get(param.def_id);
                push_def_token(out, pdef, TT_PARAMETER, TM_DECLARATION);
            }
            collect_expr_tokens(body, st, ffi_modules, out);
        }
        ThirExpr::Block { exprs, .. } => {
            for e in exprs {
                collect_expr_tokens(e, st, ffi_modules, out);
            }
        }
        ThirExpr::If { condition, then_body, else_body, .. } => {
            collect_expr_tokens(condition, st, ffi_modules, out);
            collect_expr_tokens(then_body, st, ffi_modules, out);
            if let Some(e) = else_body {
                collect_expr_tokens(e, st, ffi_modules, out);
            }
        }
        ThirExpr::Match { subject, arms, .. } => {
            collect_expr_tokens(subject, st, ffi_modules, out);
            for arm in arms {
                collect_pattern_tokens(&arm.pattern, st, out);
                collect_expr_tokens(&arm.body, st, ffi_modules, out);
            }
        }
        ThirExpr::BinaryOp { lhs, rhs, .. } => {
            collect_expr_tokens(lhs, st, ffi_modules, out);
            collect_expr_tokens(rhs, st, ffi_modules, out);
        }
        ThirExpr::UnaryOp { expr, .. }
        | ThirExpr::Try { expr, .. }
        | ThirExpr::Await { expr, .. } => {
            collect_expr_tokens(expr, st, ffi_modules, out);
        }
        ThirExpr::List { elements, .. } => {
            for e in elements {
                collect_expr_tokens(e, st, ffi_modules, out);
            }
        }
        ThirExpr::Literal(_) => {}
    }
}

fn collect_pattern_tokens(
    pattern: &ThirPattern,
    st: &asatsuyu_hir::SymbolTable,
    out: &mut Vec<RawToken>,
) {
    match pattern {
        ThirPattern::Variable { def_id, span, .. } => {
            let _def = st.get(*def_id);
            push_span_token(out, *span, TT_VARIABLE, TM_DECLARATION);
        }
        ThirPattern::Constructor { def_id, fields, .. } => {
            let ctor_def = st.get(*def_id);
            push_def_token(out, ctor_def, TT_ENUM_MEMBER, 0);
            for f in fields {
                collect_pattern_tokens(f, st, out);
            }
        }
        ThirPattern::Tuple { elements, .. } | ThirPattern::List { elements, .. } => {
            for e in elements {
                collect_pattern_tokens(e, st, out);
            }
            if let ThirPattern::List { rest: Some(r), .. } = pattern {
                collect_pattern_tokens(r, st, out);
            }
        }
        ThirPattern::Wildcard(_) | ThirPattern::Literal(_) => {}
    }
}

// ── Helpers ────────────────────────────────────────────────────────

/// Map a `DefKind` to a semantic token type and modifiers for a reference site.
fn classify_def(
    def: &asatsuyu_hir::DefData,
    ffi_modules: &std::collections::HashMap<smol_str::SmolStr, asatsuyu_hir::ffi::FfiModule>,
) -> (u32, u32) {
    match def.kind {
        DefKind::Function => (TT_FUNCTION, 0),
        DefKind::Parameter => (TT_PARAMETER, 0),
        DefKind::LocalBinding => {
            let mods = if def.is_mutable { 0 } else { TM_READONLY };
            (TT_VARIABLE, mods)
        }
        DefKind::Constructor => (TT_ENUM_MEMBER, 0),
        DefKind::Type => (TT_TYPE, 0),
        DefKind::Builtin => (TT_FUNCTION, TM_DEFAULT_LIBRARY),
        DefKind::Import => {
            // Check if this is an FFI import and add trust level.
            let mut mods = 0u32;
            if let Some(ffi_mod) = ffi_modules.get(def.name.as_str()) {
                mods |= TM_FFI | trust_level_modifier(ffi_mod.trust_level);
            }
            (TT_NAMESPACE, mods)
        }
    }
}

fn trust_level_modifier(level: FfiTrustLevel) -> u32 {
    match level {
        FfiTrustLevel::Verified => TM_VERIFIED,
        FfiTrustLevel::Checked => TM_CHECKED,
        FfiTrustLevel::Unsafe => 0,
    }
}

fn push_def_token(
    out: &mut Vec<RawToken>,
    def: &asatsuyu_hir::DefData,
    token_type: u32,
    modifiers: u32,
) {
    let len = def.span.end.saturating_sub(def.span.start);
    if len > 0 {
        out.push(RawToken { start: def.span.start, length: len, token_type, modifiers });
    }
}

fn push_span_token(
    out: &mut Vec<RawToken>,
    span: asatsuyu_syntax::Span,
    token_type: u32,
    modifiers: u32,
) {
    let len = span.end.saturating_sub(span.start);
    if len > 0 {
        out.push(RawToken { start: span.start, length: len, token_type, modifiers });
    }
}

fn push_raw(out: &mut Vec<RawToken>, start: u32, length: u32, token_type: u32, modifiers: u32) {
    if length > 0 {
        out.push(RawToken { start, length, token_type, modifiers });
    }
}

// ── Delta encoding ─────────────────────────────────────────────────

/// Convert sorted `RawToken`s to LSP delta-encoded `SemanticToken`s.
fn encode(raw: &[RawToken], line_index: &LineIndex) -> Vec<SemanticToken> {
    let mut prev_line = 0u32;
    let mut prev_start = 0u32;
    let mut result = Vec::with_capacity(raw.len());

    for token in raw {
        let Some(lc) = line_index.line_col(token.start) else {
            continue;
        };
        let line = lc.line.saturating_sub(1); // LSP is 0-based
        let col = lc.column.saturating_sub(1);

        let delta_line = line - prev_line;
        let delta_start = if delta_line == 0 { col - prev_start } else { col };

        result.push(SemanticToken {
            delta_line,
            delta_start,
            length: token.length,
            token_type: token.token_type,
            token_modifiers_bitset: token.modifiers,
        });

        prev_line = line;
        prev_start = col;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_line_index(src: &str) -> LineIndex {
        LineIndex::new(src)
    }

    #[test]
    fn encode_single_token() {
        let source = "fn main() {}";
        let li = make_line_index(source);
        let raw = vec![RawToken {
            start: 3,
            length: 4,
            token_type: TT_FUNCTION,
            modifiers: TM_DECLARATION,
        }];
        let encoded = encode(&raw, &li);
        assert_eq!(encoded.len(), 1);
        assert_eq!(encoded[0].delta_line, 0);
        assert_eq!(encoded[0].delta_start, 3);
        assert_eq!(encoded[0].length, 4);
        assert_eq!(encoded[0].token_type, TT_FUNCTION);
        assert_eq!(encoded[0].token_modifiers_bitset, TM_DECLARATION);
    }

    #[test]
    fn encode_two_tokens_same_line() {
        let source = "let x = y";
        let li = make_line_index(source);
        let raw = vec![
            RawToken { start: 4, length: 1, token_type: TT_VARIABLE, modifiers: TM_DECLARATION },
            RawToken { start: 8, length: 1, token_type: TT_VARIABLE, modifiers: 0 },
        ];
        let encoded = encode(&raw, &li);
        assert_eq!(encoded.len(), 2);
        // First token: line 0, col 4.
        assert_eq!(encoded[0].delta_line, 0);
        assert_eq!(encoded[0].delta_start, 4);
        // Second token: same line, delta_start = 8 - 4 = 4.
        assert_eq!(encoded[1].delta_line, 0);
        assert_eq!(encoded[1].delta_start, 4);
    }

    #[test]
    fn encode_tokens_across_lines() {
        let source = "let x = 1\nlet y = 2";
        let li = make_line_index(source);
        let raw = vec![
            RawToken { start: 4, length: 1, token_type: TT_VARIABLE, modifiers: 0 },
            RawToken { start: 14, length: 1, token_type: TT_VARIABLE, modifiers: 0 },
        ];
        let encoded = encode(&raw, &li);
        assert_eq!(encoded.len(), 2);
        assert_eq!(encoded[0].delta_line, 0);
        assert_eq!(encoded[0].delta_start, 4);
        // Second token on line 1 (delta_line = 1), col 4.
        assert_eq!(encoded[1].delta_line, 1);
        assert_eq!(encoded[1].delta_start, 4);
    }

    #[test]
    fn modifier_bitmask_combines_correctly() {
        let mods = TM_DECLARATION | TM_READONLY | TM_ASYNC;
        assert_eq!(mods & TM_DECLARATION, TM_DECLARATION);
        assert_eq!(mods & TM_READONLY, TM_READONLY);
        assert_eq!(mods & TM_ASYNC, TM_ASYNC);
        assert_eq!(mods & TM_MODIFICATION, 0);
    }

    #[test]
    fn trust_level_modifier_maps_correctly() {
        assert_eq!(trust_level_modifier(FfiTrustLevel::Verified), TM_VERIFIED);
        assert_eq!(trust_level_modifier(FfiTrustLevel::Checked), TM_CHECKED);
        assert_eq!(trust_level_modifier(FfiTrustLevel::Unsafe), 0);
    }
}
