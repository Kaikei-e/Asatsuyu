//! Semantic analysis helpers for LSP features.
//!
//! Provides position-to-node lookup in the THIR for hover and go-to-definition.

use asatsuyu_hir::{DefId, DefKind};
use asatsuyu_syntax::Span;
use asatsuyu_syntax::{KEYWORDS, KeywordClass, KeywordSpec};
use asatsuyu_ty::{ThirExpr, ThirModule, ThirPattern, Ty};
use smol_str::SmolStr;

/// Information about a node at a specific position.
pub(super) enum NodeInfo<'a> {
    /// An expression with its type.
    Expr { ty: &'a Ty },
    /// A variable reference with its type and definition ID.
    Var { ty: &'a Ty, def_id: DefId },
    /// A function definition.
    FnDef { ty: &'a Ty, def_id: DefId },
}

/// Find the most specific THIR node at a byte offset.
pub(super) fn find_node_at_offset(module: &ThirModule, offset: u32) -> Option<NodeInfo<'_>> {
    for func in &module.functions {
        if func.span.contains(offset) {
            let body_span = func.body.span();
            if !body_span.contains(offset) {
                return Some(NodeInfo::FnDef { ty: &func.ty, def_id: func.def_id });
            }
            if let Some(info) = find_in_expr(&func.body, offset) {
                return Some(info);
            }
            return Some(NodeInfo::FnDef { ty: &func.ty, def_id: func.def_id });
        }
    }
    None
}

/// Recursively find the most specific expression at `offset`.
fn find_in_expr(expr: &ThirExpr, offset: u32) -> Option<NodeInfo<'_>> {
    let span = expr.span();
    if !span.contains(offset) {
        return None;
    }

    // Try children first (most specific wins).
    let child_result = match expr {
        ThirExpr::Block { exprs, .. } => exprs.iter().find_map(|e| find_in_expr(e, offset)),
        ThirExpr::Call { func, args, .. } => {
            find_in_expr(func, offset).or_else(|| args.iter().find_map(|a| find_in_expr(a, offset)))
        }
        ThirExpr::BinaryOp { lhs, rhs, .. } => {
            find_in_expr(lhs, offset).or_else(|| find_in_expr(rhs, offset))
        }
        ThirExpr::UnaryOp { expr: inner, .. }
        | ThirExpr::FieldAccess { receiver: inner, .. }
        | ThirExpr::Try { expr: inner, .. }
        | ThirExpr::Await { expr: inner, .. } => find_in_expr(inner, offset),
        ThirExpr::If { condition, then_body, else_body, .. } => find_in_expr(condition, offset)
            .or_else(|| find_in_expr(then_body, offset))
            .or_else(|| else_body.as_ref().and_then(|e| find_in_expr(e, offset))),
        ThirExpr::Match { subject, arms, .. } => find_in_expr(subject, offset)
            .or_else(|| arms.iter().find_map(|arm| find_in_expr(&arm.body, offset))),
        ThirExpr::Let { value, .. }
        | ThirExpr::Assign { value, .. }
        | ThirExpr::Lambda { body: value, .. } => find_in_expr(value, offset),
        ThirExpr::List { elements, .. } => elements.iter().find_map(|e| find_in_expr(e, offset)),
        ThirExpr::Literal(_) | ThirExpr::Var { .. } => None,
    };

    if child_result.is_some() {
        return child_result;
    }

    // No child matched more specifically — return this node.
    match expr {
        ThirExpr::Var { def_id, ty, .. } => Some(NodeInfo::Var { ty, def_id: *def_id }),
        _ => Some(NodeInfo::Expr { ty: expr.ty() }),
    }
}

/// Find the definition span for a node at the given offset.
pub(super) fn find_definition_at_offset(module: &ThirModule, offset: u32) -> Option<Span> {
    let info = find_node_at_offset(module, offset)?;
    match info {
        NodeInfo::Var { def_id, .. } | NodeInfo::FnDef { def_id, .. } => {
            let def = module.symbol_table.get(def_id);
            Some(def.span)
        }
        NodeInfo::Expr { .. } => None,
    }
}

/// Get hover text for a node at the given offset.
pub(super) fn hover_at_offset(module: &ThirModule, offset: u32) -> Option<String> {
    let info = find_node_at_offset(module, offset)?;
    match info {
        NodeInfo::Var { ty, def_id } => {
            let def = module.symbol_table.get(def_id);
            let prefix = if def.is_mutable { "mut " } else { "" };
            Some(format!("{prefix}{}: {ty}", def.name))
        }
        NodeInfo::FnDef { ty, def_id } => {
            let def = module.symbol_table.get(def_id);
            Some(format!("fn {}: {ty}", def.name))
        }
        NodeInfo::Expr { ty } => Some(format!("{ty}")),
    }
}

// ── Find all references ─────────────────────────────────────────

/// Collect all spans that reference or define the given `DefId`.
pub(super) fn find_all_references(module: &ThirModule, target: DefId) -> Vec<Span> {
    let mut spans = Vec::new();

    // Include the definition itself.
    let def = module.symbol_table.get(target);
    spans.push(def.span);

    // Search all function bodies.
    for func in &module.functions {
        // Check if this function's def_id matches.
        if func.def_id == target {
            // Name span is already in the definition span above.
        }
        // Check parameters.
        for param in &func.params {
            if param.def_id == target {
                spans.push(param.span);
            }
        }
        collect_refs_in_expr(&func.body, target, &mut spans);
    }

    spans
}

fn collect_refs_in_expr(expr: &ThirExpr, target: DefId, spans: &mut Vec<Span>) {
    match expr {
        ThirExpr::Var { def_id, span, .. } if *def_id == target => {
            spans.push(*span);
        }
        ThirExpr::Block { exprs, .. } => {
            for e in exprs {
                collect_refs_in_expr(e, target, spans);
            }
        }
        ThirExpr::Call { func, args, .. } => {
            collect_refs_in_expr(func, target, spans);
            for a in args {
                collect_refs_in_expr(a, target, spans);
            }
        }
        ThirExpr::BinaryOp { lhs, rhs, .. } => {
            collect_refs_in_expr(lhs, target, spans);
            collect_refs_in_expr(rhs, target, spans);
        }
        ThirExpr::UnaryOp { expr: inner, .. }
        | ThirExpr::FieldAccess { receiver: inner, .. }
        | ThirExpr::Try { expr: inner, .. }
        | ThirExpr::Await { expr: inner, .. } => {
            collect_refs_in_expr(inner, target, spans);
        }
        ThirExpr::If { condition, then_body, else_body, .. } => {
            collect_refs_in_expr(condition, target, spans);
            collect_refs_in_expr(then_body, target, spans);
            if let Some(e) = else_body {
                collect_refs_in_expr(e, target, spans);
            }
        }
        ThirExpr::Match { subject, arms, .. } => {
            collect_refs_in_expr(subject, target, spans);
            for arm in arms {
                collect_refs_in_pattern(&arm.pattern, target, spans);
                collect_refs_in_expr(&arm.body, target, spans);
            }
        }
        ThirExpr::Let { binding, value, .. } => {
            if *binding == target {
                // The let binding itself — definition span is already included.
            }
            collect_refs_in_expr(value, target, spans);
        }
        ThirExpr::Assign { target: t, value, span, .. } => {
            if *t == target {
                spans.push(*span);
            }
            collect_refs_in_expr(value, target, spans);
        }
        ThirExpr::Lambda { params, body, .. } => {
            for p in params {
                if p.def_id == target {
                    spans.push(p.span);
                }
            }
            collect_refs_in_expr(body, target, spans);
        }
        ThirExpr::List { elements, .. } => {
            for e in elements {
                collect_refs_in_expr(e, target, spans);
            }
        }
        ThirExpr::Literal(_) | ThirExpr::Var { .. } => {}
    }
}

fn collect_refs_in_pattern(pattern: &ThirPattern, target: DefId, spans: &mut Vec<Span>) {
    match pattern {
        ThirPattern::Variable { def_id, span, .. } if *def_id == target => {
            spans.push(*span);
        }
        ThirPattern::Constructor { fields, .. } => {
            for f in fields {
                collect_refs_in_pattern(f, target, spans);
            }
        }
        ThirPattern::List { elements, rest, .. } => {
            for e in elements {
                collect_refs_in_pattern(e, target, spans);
            }
            if let Some(r) = rest {
                collect_refs_in_pattern(r, target, spans);
            }
        }
        ThirPattern::Tuple { elements, .. } => {
            for e in elements {
                collect_refs_in_pattern(e, target, spans);
            }
        }
        _ => {}
    }
}

// ── Completion ──────────────────────────────────────────────────

/// Distinguishes symbol completions from keyword completions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CompletionEntryKind {
    Symbol(DefKind),
    Keyword,
}

/// A completion candidate with metadata.
pub(super) struct CompletionEntry {
    pub name: SmolStr,
    pub kind: CompletionEntryKind,
    pub ty: Option<Ty>,
    pub insert_text: Option<SmolStr>,
}

/// The syntactic context at the completion cursor position.
///
/// Determines which keywords are valid completions. Classified from source
/// text alone (no THIR required) so it works during editing with parse errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CompletionContext {
    /// Outside any function/type body. Offer item-level keywords.
    TopLevel,
    /// Inside a block expression (function body, if/match/lambda body).
    Block,
    /// In an expression position within a block.
    Expr,
    /// Inside an import statement line. Suppress keyword completions.
    Import,
}

/// Collect completion candidates visible at the given byte offset.
pub(super) fn collect_completions(module: &ThirModule, offset: u32) -> Vec<CompletionEntry> {
    let mut entries = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // 1. Module-level definitions (always visible).
    for (def_id, def) in module.symbol_table.iter() {
        match def.kind {
            DefKind::Function
            | DefKind::Type
            | DefKind::Constructor
            | DefKind::Import
            | DefKind::Builtin => {
                if seen.insert(def.name.clone()) {
                    let ty = find_type_for_def(module, def_id);
                    entries.push(CompletionEntry {
                        name: def.name.clone(),
                        kind: CompletionEntryKind::Symbol(def.kind),
                        ty,
                        insert_text: None,
                    });
                }
            }
            DefKind::Parameter | DefKind::LocalBinding => {
                // Handled below via scope analysis.
            }
        }
    }

    // 2. Find the enclosing function and add locals visible at offset.
    for func in &module.functions {
        if !func.span.contains(offset) {
            continue;
        }
        // Add parameters.
        for param in &func.params {
            let def = module.symbol_table.get(param.def_id);
            if seen.insert(def.name.clone()) {
                entries.push(CompletionEntry {
                    name: def.name.clone(),
                    kind: CompletionEntryKind::Symbol(DefKind::Parameter),
                    ty: Some(param.ty.clone()),
                    insert_text: None,
                });
            }
        }
        // Collect let bindings and pattern variables before the offset.
        collect_locals_in_expr(&func.body, offset, &module.symbol_table, &mut entries, &mut seen);
        break;
    }

    entries
}

/// Shared keyword vocabulary used as the basis for LSP keyword-aware features.
///
/// Issue 89 freezes the keyword taxonomy in `asatsuyu-syntax`; Issue 90 builds
/// completion items directly from this table.
pub(super) fn completion_keyword_specs() -> impl Iterator<Item = &'static KeywordSpec> {
    KEYWORDS.iter().filter(|spec| {
        matches!(spec.class, KeywordClass::Hard | KeywordClass::Literal | KeywordClass::Contextual)
    })
}

/// Classify the completion context at `offset` using source text heuristics.
///
/// Scans backwards from the cursor to determine whether we are at the top
/// level, inside a block, or on an import line. This is intentionally
/// lightweight — it uses brace counting rather than full parsing.
pub(super) fn classify_context(source: &str, offset: u32) -> CompletionContext {
    let offset = (offset as usize).min(source.len());
    let before_cursor = &source[..offset];

    // Check if current line starts with import/from keywords.
    let line_start = before_cursor.rfind('\n').map_or(0, |i| i + 1);
    let line_prefix = before_cursor[line_start..].trim_start();
    if line_prefix.starts_with("import ") || line_prefix.starts_with("from ") {
        return CompletionContext::Import;
    }

    // Count unmatched braces scanning backwards.
    let mut brace_depth: i32 = 0;
    for byte in before_cursor.bytes().rev() {
        match byte {
            b'}' => brace_depth += 1,
            b'{' => {
                brace_depth -= 1;
                if brace_depth < 0 {
                    return classify_in_block(line_prefix);
                }
            }
            _ => {}
        }
    }

    CompletionContext::TopLevel
}

fn classify_in_block(line_prefix: &str) -> CompletionContext {
    let trimmed = line_prefix.trim_end();
    if trimmed.is_empty() {
        return CompletionContext::Block;
    }

    if trimmed.ends_with('=') || trimmed.ends_with("->") || trimmed.ends_with('(') {
        return CompletionContext::Expr;
    }

    if trimmed.ends_with("let") || trimmed.ends_with("let ") || trimmed.starts_with("let ") {
        return CompletionContext::Expr;
    }

    CompletionContext::Block
}

/// Collect keyword completion entries appropriate for the given context.
fn collect_keyword_completions(ctx: CompletionContext) -> Vec<CompletionEntry> {
    let mut entries = Vec::new();

    let allowed: &[&str] = match ctx {
        CompletionContext::TopLevel => &["fn", "type", "import", "from", "pub"],
        CompletionContext::Block => &["let", "if", "match", "try", "fn", "True", "False"],
        CompletionContext::Expr => &["if", "match", "try", "fn", "True", "False", "await"],
        CompletionContext::Import => &[],
    };

    for spec in completion_keyword_specs().filter(|spec| allowed.contains(&spec.text)) {
        entries.push(CompletionEntry {
            name: SmolStr::new(spec.text),
            kind: CompletionEntryKind::Keyword,
            ty: None,
            insert_text: None,
        });
    }

    match ctx {
        CompletionContext::TopLevel => {
            entries.push(keyword_snippet("async fn", "async fn "));
        }
        CompletionContext::Block => {
            entries.push(keyword_snippet("let mut", "let mut "));
            entries.push(keyword_snippet("await", "await "));
        }
        CompletionContext::Expr => {
            entries.push(keyword_snippet("await", "await "));
            entries.push(keyword_snippet("mut", "mut "));
        }
        CompletionContext::Import => {}
    }

    entries
}

fn keyword_snippet(label: &'static str, insert_text: &'static str) -> CompletionEntry {
    CompletionEntry {
        name: SmolStr::new(label),
        kind: CompletionEntryKind::Keyword,
        ty: None,
        insert_text: Some(SmolStr::new(insert_text)),
    }
}

/// Collect all completion candidates (keywords + symbols) at the given offset.
///
/// Works even when THIR is unavailable (keyword completions only in that case).
pub(super) fn collect_all_completions(
    thir: Option<&ThirModule>,
    source: &str,
    offset: u32,
) -> Vec<CompletionEntry> {
    let ctx = classify_context(source, offset);
    let mut entries = collect_keyword_completions(ctx);

    if let Some(module) = thir {
        entries.extend(collect_completions(module, offset));
    }

    entries
}

/// Walk an expression tree and collect let-bound names and pattern variables
/// that are defined before `offset`.
fn collect_locals_in_expr(
    expr: &ThirExpr,
    offset: u32,
    st: &asatsuyu_hir::SymbolTable,
    entries: &mut Vec<CompletionEntry>,
    seen: &mut std::collections::HashSet<SmolStr>,
) {
    match expr {
        ThirExpr::Block { exprs, .. } => {
            for e in exprs {
                // Only include bindings defined before the cursor.
                if e.span().start >= offset {
                    break;
                }
                collect_locals_in_expr(e, offset, st, entries, seen);
            }
        }
        ThirExpr::Let { binding, value, ty, .. } => {
            let def = st.get(*binding);
            if def.span.start < offset && seen.insert(def.name.clone()) {
                entries.push(CompletionEntry {
                    name: def.name.clone(),
                    kind: CompletionEntryKind::Symbol(DefKind::LocalBinding),
                    ty: Some(ty.clone()),
                    insert_text: None,
                });
            }
            collect_locals_in_expr(value, offset, st, entries, seen);
        }
        ThirExpr::Match { arms, .. } => {
            for arm in arms {
                if arm.span.contains(offset) {
                    collect_pattern_bindings(&arm.pattern, st, entries, seen);
                    collect_locals_in_expr(&arm.body, offset, st, entries, seen);
                }
            }
        }
        ThirExpr::Lambda { params, body, .. } if expr.span().contains(offset) => {
            for p in params {
                let def = st.get(p.def_id);
                if seen.insert(def.name.clone()) {
                    entries.push(CompletionEntry {
                        name: def.name.clone(),
                        kind: CompletionEntryKind::Symbol(DefKind::Parameter),
                        ty: Some(p.ty.clone()),
                        insert_text: None,
                    });
                }
            }
            collect_locals_in_expr(body, offset, st, entries, seen);
        }
        ThirExpr::If { condition, then_body, else_body, .. } => {
            if condition.span().contains(offset) {
                collect_locals_in_expr(condition, offset, st, entries, seen);
            } else if then_body.span().contains(offset) {
                collect_locals_in_expr(then_body, offset, st, entries, seen);
            } else if let Some(e) = else_body
                && e.span().contains(offset)
            {
                collect_locals_in_expr(e, offset, st, entries, seen);
            }
        }
        _ => {}
    }
}

fn collect_pattern_bindings(
    pattern: &ThirPattern,
    st: &asatsuyu_hir::SymbolTable,
    entries: &mut Vec<CompletionEntry>,
    seen: &mut std::collections::HashSet<SmolStr>,
) {
    match pattern {
        ThirPattern::Variable { def_id, ty, .. } => {
            let def = st.get(*def_id);
            if seen.insert(def.name.clone()) {
                entries.push(CompletionEntry {
                    name: def.name.clone(),
                    kind: CompletionEntryKind::Symbol(DefKind::LocalBinding),
                    ty: Some(ty.clone()),
                    insert_text: None,
                });
            }
        }
        ThirPattern::Constructor { fields, .. } => {
            for f in fields {
                collect_pattern_bindings(f, st, entries, seen);
            }
        }
        ThirPattern::List { elements, rest, .. } => {
            for e in elements {
                collect_pattern_bindings(e, st, entries, seen);
            }
            if let Some(r) = rest {
                collect_pattern_bindings(r, st, entries, seen);
            }
        }
        ThirPattern::Tuple { elements, .. } => {
            for e in elements {
                collect_pattern_bindings(e, st, entries, seen);
            }
        }
        ThirPattern::Wildcard(_) | ThirPattern::Literal(_) => {}
    }
}

/// Find the type of a definition by searching the THIR module.
fn find_type_for_def(module: &ThirModule, target: DefId) -> Option<Ty> {
    for func in &module.functions {
        if func.def_id == target {
            return Some(func.ty.clone());
        }
    }
    None
}

// ── Document Symbols ────────────────────────────────────────────

/// A document symbol entry for the outline view.
pub(super) struct DocumentSymbolEntry {
    pub name: SmolStr,
    pub kind: DefKind,
    pub span: Span,
}

/// Collect top-level symbols for the document outline.
pub(super) fn collect_document_symbols(module: &ThirModule) -> Vec<DocumentSymbolEntry> {
    let mut symbols = Vec::new();

    // Functions.
    for func in &module.functions {
        let def = module.symbol_table.get(func.def_id);
        symbols.push(DocumentSymbolEntry {
            name: def.name.clone(),
            kind: def.kind,
            span: func.span,
        });
    }

    // Custom types and their constructors.
    for ct in &module.custom_types {
        let def = module.symbol_table.get(ct.def_id);
        symbols.push(DocumentSymbolEntry { name: def.name.clone(), kind: def.kind, span: ct.span });
    }

    symbols
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsp_keyword_specs_share_syntax_table() {
        let keywords: Vec<_> = completion_keyword_specs().collect();
        assert!(keywords.iter().any(|spec| spec.text == "fn"));
        assert!(keywords.iter().any(|spec| spec.text == "python"));
        assert!(keywords.iter().any(|spec| spec.text == "as"));
        // `mut` is a hard keyword (Phase 3-1) but filtered by allowed lists in completions
        assert!(keywords.iter().any(|spec| spec.text == "mut"));
        // `async`/`await` promoted to hard keywords in Phase 3-2
        assert!(keywords.iter().any(|spec| spec.text == "async"));
        assert!(keywords.iter().any(|spec| spec.text == "await"));
    }

    // ── Context classification ──────────────────────────────────

    #[test]
    fn classify_context_empty_file() {
        assert_eq!(classify_context("", 0), CompletionContext::TopLevel);
    }

    #[test]
    fn classify_context_top_level_after_fn() {
        assert_eq!(classify_context("fn main() {}\n", 14), CompletionContext::TopLevel,);
    }

    #[test]
    fn classify_context_block_inside_fn() {
        let source = "fn main() {\n  \n}";
        assert_eq!(classify_context(source, 14), CompletionContext::Block);
    }

    #[test]
    fn classify_context_nested_block() {
        let source = "fn main() {\n  if True {\n    \n  }\n}";
        assert_eq!(classify_context(source, 27), CompletionContext::Block);
    }

    #[test]
    fn classify_context_expr_after_equals() {
        let source = "fn main() {\n  let x = \n}";
        assert_eq!(classify_context(source, 22), CompletionContext::Expr);
    }

    #[test]
    fn classify_context_expr_after_let_prefix() {
        let source = "fn main() {\n  let \n}";
        assert_eq!(classify_context(source, 18), CompletionContext::Expr);
    }

    #[test]
    fn classify_context_import_line() {
        assert_eq!(classify_context("import ", 7), CompletionContext::Import);
        assert_eq!(classify_context("from python import ", 19), CompletionContext::Import,);
    }

    #[test]
    #[allow(clippy::cast_possible_truncation)]
    fn classify_context_after_closed_braces() {
        let source = "fn a() {}\nfn b() {}\n";
        assert_eq!(classify_context(source, source.len() as u32), CompletionContext::TopLevel,);
    }

    // ── Keyword completions ─────────────────────────────────────

    #[test]
    fn keyword_completions_top_level() {
        let entries = collect_keyword_completions(CompletionContext::TopLevel);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"fn"));
        assert!(names.contains(&"type"));
        assert!(names.contains(&"import"));
        assert!(names.contains(&"from"));
        assert!(names.contains(&"pub"));
        assert!(!names.contains(&"let"));
        assert!(!names.contains(&"match"));
    }

    #[test]
    fn keyword_completions_block() {
        let entries = collect_keyword_completions(CompletionContext::Block);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"let"));
        assert!(names.contains(&"if"));
        assert!(names.contains(&"match"));
        assert!(names.contains(&"try"));
        assert!(names.contains(&"fn"));
        assert!(names.contains(&"let mut"));
        assert!(names.contains(&"await"));
        assert!(names.contains(&"True"));
        assert!(names.contains(&"False"));
        assert!(!names.contains(&"type"));
        assert!(!names.contains(&"import"));
        assert!(!names.contains(&"pub"));
    }

    #[test]
    fn keyword_completions_expr() {
        let entries = collect_keyword_completions(CompletionContext::Expr);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"if"));
        assert!(names.contains(&"match"));
        assert!(names.contains(&"try"));
        assert!(names.contains(&"fn"));
        assert!(names.contains(&"await"));
        assert!(names.contains(&"mut"));
        assert!(!names.contains(&"let"));
        assert!(!names.contains(&"type"));
    }

    #[test]
    fn keyword_completions_import_is_empty() {
        let entries = collect_keyword_completions(CompletionContext::Import);
        assert!(entries.is_empty());
    }

    #[test]
    fn keyword_entries_are_marked_as_keyword_kind() {
        let entries = collect_keyword_completions(CompletionContext::TopLevel);
        for entry in &entries {
            assert_eq!(entry.kind, CompletionEntryKind::Keyword);
            assert!(entry.ty.is_none());
        }
    }

    #[test]
    fn keyword_snippets_have_insert_text() {
        let entries = collect_keyword_completions(CompletionContext::TopLevel);
        let async_fn = entries.iter().find(|e| e.name == "async fn").expect("missing async fn");
        assert_eq!(async_fn.insert_text.as_deref(), Some("async fn "));

        let block_entries = collect_keyword_completions(CompletionContext::Block);
        let let_mut = block_entries.iter().find(|e| e.name == "let mut").expect("missing let mut");
        assert_eq!(let_mut.insert_text.as_deref(), Some("let mut "));
    }

    // ── Unified completions ─────────────────────────────────────

    #[test]
    fn all_completions_works_without_thir() {
        let source = "fn main() {\n  \n}";
        let entries = collect_all_completions(None, source, 14);
        assert!(!entries.is_empty());
        assert!(entries.iter().all(|e| e.kind == CompletionEntryKind::Keyword));
    }

    #[test]
    fn all_completions_top_level_without_thir() {
        let entries = collect_all_completions(None, "", 0);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"fn"));
        assert!(names.contains(&"type"));
    }
}
