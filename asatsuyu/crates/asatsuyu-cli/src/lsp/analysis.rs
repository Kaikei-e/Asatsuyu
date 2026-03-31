//! Semantic analysis helpers for LSP features.
//!
//! Provides position-to-node lookup in the THIR for hover and go-to-definition.

use asatsuyu_hir::{DefId, DefKind};
use asatsuyu_syntax::Span;
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
        | ThirExpr::Try { expr: inner, .. } => find_in_expr(inner, offset),
        ThirExpr::If { condition, then_body, else_body, .. } => find_in_expr(condition, offset)
            .or_else(|| find_in_expr(then_body, offset))
            .or_else(|| else_body.as_ref().and_then(|e| find_in_expr(e, offset))),
        ThirExpr::Match { subject, arms, .. } => find_in_expr(subject, offset)
            .or_else(|| arms.iter().find_map(|arm| find_in_expr(&arm.body, offset))),
        ThirExpr::Let { value, .. } | ThirExpr::Lambda { body: value, .. } => {
            find_in_expr(value, offset)
        }
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
            Some(format!("{}: {ty}", def.name))
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
        | ThirExpr::Try { expr: inner, .. } => {
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

/// A completion candidate with metadata.
pub(super) struct CompletionEntry {
    pub name: SmolStr,
    pub kind: DefKind,
    pub ty: Option<Ty>,
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
                    entries.push(CompletionEntry { name: def.name.clone(), kind: def.kind, ty });
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
                    kind: DefKind::Parameter,
                    ty: Some(param.ty.clone()),
                });
            }
        }
        // Collect let bindings and pattern variables before the offset.
        collect_locals_in_expr(&func.body, offset, &module.symbol_table, &mut entries, &mut seen);
        break;
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
                    kind: DefKind::LocalBinding,
                    ty: Some(ty.clone()),
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
                        kind: DefKind::Parameter,
                        ty: Some(p.ty.clone()),
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
                    kind: DefKind::LocalBinding,
                    ty: Some(ty.clone()),
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
