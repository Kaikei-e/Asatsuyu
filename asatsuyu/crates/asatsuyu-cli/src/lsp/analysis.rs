//! Semantic analysis helpers for LSP features.
//!
//! Provides position-to-node lookup in the THIR for hover and go-to-definition.

use asatsuyu_hir::DefId;
use asatsuyu_syntax::Span;
use asatsuyu_ty::{ThirExpr, ThirModule, Ty};

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
