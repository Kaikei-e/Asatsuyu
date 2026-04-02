//! Semantic analysis helpers for LSP features.
//!
//! Provides position-to-node lookup in the THIR for hover and go-to-definition.

use asatsuyu_hir::{DefId, DefKind};
use asatsuyu_syntax::Span;
use asatsuyu_syntax::{KEYWORDS, KeywordClass, KeywordSpec};
use asatsuyu_ty::{ThirExpr, ThirModule, ThirPattern, Ty};
use smol_str::SmolStr;

/// Information about a node at a specific position.
#[derive(Debug)]
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
            if let Some(info) = find_in_expr(&func.body, offset, &module.symbol_table) {
                return Some(info);
            }
            return Some(NodeInfo::FnDef { ty: &func.ty, def_id: func.def_id });
        }
    }
    None
}

/// Recursively find the most specific expression at `offset`.
///
/// The symbol table is threaded through so that assignment targets and let
/// binding names can be resolved to their `DefId` (Issue 104).
fn find_in_expr<'a>(
    expr: &'a ThirExpr,
    offset: u32,
    st: &asatsuyu_hir::SymbolTable,
) -> Option<NodeInfo<'a>> {
    let span = expr.span();
    if !span.contains(offset) {
        return None;
    }

    // Try children first (most specific wins).
    let child_result = match expr {
        ThirExpr::Block { exprs, .. } => exprs.iter().find_map(|e| find_in_expr(e, offset, st)),
        ThirExpr::Call { func, args, .. } => find_in_expr(func, offset, st)
            .or_else(|| args.iter().find_map(|a| find_in_expr(a, offset, st))),
        ThirExpr::BinaryOp { lhs, rhs, .. } => {
            find_in_expr(lhs, offset, st).or_else(|| find_in_expr(rhs, offset, st))
        }
        ThirExpr::UnaryOp { expr: inner, .. }
        | ThirExpr::FieldAccess { receiver: inner, .. }
        | ThirExpr::Try { expr: inner, .. }
        | ThirExpr::Await { expr: inner, .. } => find_in_expr(inner, offset, st),
        ThirExpr::If { condition, then_body, else_body, .. } => find_in_expr(condition, offset, st)
            .or_else(|| find_in_expr(then_body, offset, st))
            .or_else(|| else_body.as_ref().and_then(|e| find_in_expr(e, offset, st))),
        ThirExpr::Match { subject, arms, .. } => find_in_expr(subject, offset, st)
            .or_else(|| arms.iter().find_map(|arm| find_in_expr(&arm.body, offset, st))),
        ThirExpr::Let { binding, value, ty, .. } => {
            // Check if cursor is on the binding name itself.
            let def = st.get(*binding);
            if def.span.contains(offset) {
                return Some(NodeInfo::Var { ty, def_id: *binding });
            }
            find_in_expr(value, offset, st)
        }
        ThirExpr::Assign { target, value, target_span, ty, .. } => {
            // Check if cursor is on the assignment target identifier.
            if target_span.contains(offset) {
                return Some(NodeInfo::Var { ty, def_id: *target });
            }
            find_in_expr(value, offset, st)
        }
        ThirExpr::Lambda { body, .. } => find_in_expr(body, offset, st),
        ThirExpr::List { elements, .. } => {
            elements.iter().find_map(|e| find_in_expr(e, offset, st))
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

/// Return documentation for a keyword, or `None` if the word is not a keyword.
pub(super) fn keyword_hover(word: &str) -> Option<String> {
    let doc = match word {
        "fn" => {
            "\
**fn** — Declare a function

```asatsuyu
fn name(param: Type) -> ReturnType {
  body
}
```

Use `pub fn` to export. The last expression is the return value."
        }

        "let" => {
            "\
**let** — Create an immutable binding

```asatsuyu
let x = expr
```

Add a type annotation with `let x: Type = expr`."
        }

        "mut" => {
            "\
**mut** — Mark a binding as mutable

```asatsuyu
let mut x = 0
x = x + 1
```

Only local variables can be mutable."
        }

        "match" => {
            "\
**match** — Exhaustive pattern matching

```asatsuyu
match value {
  Pattern(x) -> expr
  _ -> default
}
```

The compiler ensures all variants are handled."
        }

        "try" => {
            "\
**try** — Convert a Python exception to Result

```asatsuyu
let result = try python_call()
```

Catches Python exceptions at the FFI boundary and wraps them as `Error`."
        }

        "async" => {
            "\
**async** — Declare an async function

```asatsuyu
async fn fetch(url: String) -> String {
  let data = await get(url)
  data
}
```

Async functions return `Task(T)`. Use `await` to unwrap."
        }

        "await" => {
            "\
**await** — Unwrap a Task value

```asatsuyu
let value = await async_call()
```

Only valid inside `async fn`."
        }

        "type" => {
            "\
**type** — Define an algebraic data type

```asatsuyu
type Option(a) {
  Some(a)
  None
}
```

Use `pub type` to export. Variants are constructors."
        }

        "if" => {
            "\
**if** — Conditional expression

```asatsuyu
if condition {
  then_branch
} else {
  else_branch
}
```

Both branches must have the same type."
        }

        _ => return None,
    };
    Some(doc.to_owned())
}

// ── Signature help ─────────────────────────────────────────────

/// Parameter information for signature help.
pub(super) struct ParamInfo {
    pub label: String,
}

/// Signature help information for a function call.
pub(super) struct SignatureHelpInfo {
    pub label: String,
    pub parameters: Vec<ParamInfo>,
    pub active_parameter: u32,
    /// Optional documentation shown alongside the signature.
    pub documentation: Option<String>,
}

/// Compute signature help at the given byte offset.
///
/// Returns `None` if the cursor is not inside a function call's argument list.
pub(super) fn signature_help_at_offset(
    module: &ThirModule,
    source: &str,
    offset: u32,
) -> Option<SignatureHelpInfo> {
    // Find the enclosing Call node.
    for func in &module.functions {
        if !func.span.contains(offset) {
            continue;
        }
        if let Some(info) = sig_help_in_expr(&func.body, source, offset, module) {
            return Some(info);
        }
    }
    None
}

/// Walk the THIR looking for the innermost Call whose argument list contains `offset`.
fn sig_help_in_expr(
    expr: &ThirExpr,
    source: &str,
    offset: u32,
    module: &ThirModule,
) -> Option<SignatureHelpInfo> {
    if !expr.span().contains(offset) {
        return None;
    }

    // Recurse into children first — innermost call wins.
    let child = match expr {
        ThirExpr::Block { exprs, .. } => {
            exprs.iter().find_map(|e| sig_help_in_expr(e, source, offset, module))
        }
        ThirExpr::Call { func, args, .. } => sig_help_in_expr(func, source, offset, module)
            .or_else(|| args.iter().find_map(|a| sig_help_in_expr(a, source, offset, module))),
        ThirExpr::BinaryOp { lhs, rhs, .. } => sig_help_in_expr(lhs, source, offset, module)
            .or_else(|| sig_help_in_expr(rhs, source, offset, module)),
        ThirExpr::UnaryOp { expr: inner, .. }
        | ThirExpr::FieldAccess { receiver: inner, .. }
        | ThirExpr::Try { expr: inner, .. }
        | ThirExpr::Await { expr: inner, .. } => sig_help_in_expr(inner, source, offset, module),
        ThirExpr::If { condition, then_body, else_body, .. } => {
            sig_help_in_expr(condition, source, offset, module)
                .or_else(|| sig_help_in_expr(then_body, source, offset, module))
                .or_else(|| {
                    else_body.as_ref().and_then(|e| sig_help_in_expr(e, source, offset, module))
                })
        }
        ThirExpr::Match { subject, arms, .. } => sig_help_in_expr(subject, source, offset, module)
            .or_else(|| {
                arms.iter().find_map(|arm| sig_help_in_expr(&arm.body, source, offset, module))
            }),
        ThirExpr::Let { value, .. } | ThirExpr::Assign { value, .. } => {
            sig_help_in_expr(value, source, offset, module)
        }
        ThirExpr::Lambda { body, .. } => sig_help_in_expr(body, source, offset, module),
        ThirExpr::List { elements, .. } => {
            elements.iter().find_map(|e| sig_help_in_expr(e, source, offset, module))
        }
        ThirExpr::Literal(_) | ThirExpr::Var { .. } => None,
    };

    if child.is_some() {
        return child;
    }

    // Check if *this* node is a Call and the cursor is inside the argument list.
    if let ThirExpr::Call { func, args, span, .. } = expr {
        return build_sig_help(func, args, *span, source, offset, module);
    }

    None
}

/// Build `SignatureHelpInfo` for a Call node, if the cursor is inside the parens.
fn build_sig_help(
    func: &ThirExpr,
    args: &[ThirExpr],
    call_span: Span,
    source: &str,
    offset: u32,
    module: &ThirModule,
) -> Option<SignatureHelpInfo> {
    // Find the opening `(` in the source after the callee expression.
    let func_end = func.span().end as usize;
    let call_end = call_span.end as usize;
    let call_text = source.get(func_end..call_end)?;
    let paren_offset_in_call = call_text.find('(')?;
    let paren_pos = func_end + paren_offset_in_call;

    // Cursor must be after `(` and within the call span.
    if (offset as usize) <= paren_pos {
        return None;
    }

    // Count active parameter by counting top-level commas before cursor.
    let active = count_active_parameter(source, paren_pos + 1, offset as usize);

    // Resolve the callee to get parameter names and types.
    let (label, params, documentation) = resolve_callee_signature(func, args, module)?;

    Some(SignatureHelpInfo { label, parameters: params, active_parameter: active, documentation })
}

/// Count commas at the top level (respecting nesting) between `start` and `cursor`.
fn count_active_parameter(source: &str, start: usize, cursor: usize) -> u32 {
    let mut count = 0u32;
    let mut depth = 0i32;
    let end = cursor.min(source.len());
    for byte in source.get(start..end).unwrap_or("").bytes() {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => count += 1,
            _ => {}
        }
    }
    count
}

/// Resolve a callee expression to its signature label, parameter list, and documentation.
fn resolve_callee_signature(
    func: &ThirExpr,
    _args: &[ThirExpr],
    module: &ThirModule,
) -> Option<(String, Vec<ParamInfo>, Option<String>)> {
    match func {
        // Asatsuyu function or constructor call: `f(...)` or `Some(...)`
        ThirExpr::Var { def_id, .. } => {
            let def = module.symbol_table.get(*def_id);
            match def.kind {
                DefKind::Function => {
                    let (label, params) = resolve_asatsuyu_fn(*def_id, module)?;
                    Some((label, params, None))
                }
                DefKind::Constructor => {
                    let (label, params) = resolve_constructor(*def_id, module)?;
                    Some((label, params, None))
                }
                _ => None,
            }
        }
        // FFI field access: `module.func(...)` or `instance.method(...)`
        ThirExpr::FieldAccess { receiver, field, .. } => {
            let receiver_ty = receiver.ty();
            match receiver_ty {
                Ty::FfiModule { module_name } => {
                    resolve_ffi_module_fn(module_name, field, &module.ffi_modules)
                }
                Ty::FfiInstance { module: mod_name, class } => {
                    resolve_ffi_instance_method(mod_name, class, field, &module.ffi_modules)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn resolve_asatsuyu_fn(def_id: DefId, module: &ThirModule) -> Option<(String, Vec<ParamInfo>)> {
    let fn_def = module.functions.iter().find(|f| f.def_id == def_id)?;
    let fn_name = module.symbol_table.get(def_id).name.clone();

    let params: Vec<ParamInfo> = fn_def
        .params
        .iter()
        .map(|p| {
            let name = module.symbol_table.get(p.def_id).name.clone();
            ParamInfo { label: format!("{name}: {}", p.ty) }
        })
        .collect();

    let param_labels: Vec<&str> = params.iter().map(|p| p.label.as_str()).collect();
    let label = format!("fn {}({}) -> {}", fn_name, param_labels.join(", "), fn_def.return_ty);

    Some((label, params))
}

fn resolve_constructor(
    ctor_def_id: DefId,
    module: &ThirModule,
) -> Option<(String, Vec<ParamInfo>)> {
    let ctor_name = module.symbol_table.get(ctor_def_id).name.clone();

    // Find the custom type that contains this constructor.
    for ct in &module.custom_types {
        for variant in &ct.variants {
            if variant.def_id == ctor_def_id {
                let params: Vec<ParamInfo> = variant
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(i, field)| {
                        let name = field.label.as_deref().unwrap_or(&format!("_{i}")).to_owned();
                        ParamInfo { label: format!("{name}: {}", field.type_expr.name) }
                    })
                    .collect();

                let param_labels: Vec<&str> = params.iter().map(|p| p.label.as_str()).collect();
                let label = format!("{}({})", ctor_name, param_labels.join(", "));
                return Some((label, params));
            }
        }
    }
    None
}

fn resolve_ffi_module_fn(
    module_name: &SmolStr,
    field: &SmolStr,
    ffi_modules: &std::collections::HashMap<SmolStr, asatsuyu_hir::ffi::FfiModule>,
) -> Option<(String, Vec<ParamInfo>, Option<String>)> {
    let ffi_mod = ffi_modules.get(module_name)?;
    let symbol = ffi_mod.symbols.iter().find(|s| s.name == *field)?;
    let trust_doc = ffi_trust_doc(ffi_mod.trust_level);
    match &symbol.kind {
        asatsuyu_hir::ffi::FfiSymbolKind::Function(sig) => {
            let (label, params) = build_ffi_sig_label(&format!("{module_name}.{field}"), sig);
            let doc = build_ffi_doc(trust_doc, sig.is_async);
            Some((label, params, Some(doc)))
        }
        asatsuyu_hir::ffi::FfiSymbolKind::Class(cls) => {
            let sig = cls.constructor.as_ref()?;
            let (label, params) = build_ffi_sig_label(&format!("{module_name}.{}", cls.name), sig);
            Some((label, params, Some(trust_doc.to_owned())))
        }
        asatsuyu_hir::ffi::FfiSymbolKind::Constant(_) => None,
    }
}

fn resolve_ffi_instance_method(
    module_name: &SmolStr,
    class_name: &SmolStr,
    method: &SmolStr,
    ffi_modules: &std::collections::HashMap<SmolStr, asatsuyu_hir::ffi::FfiModule>,
) -> Option<(String, Vec<ParamInfo>, Option<String>)> {
    let ffi_mod = ffi_modules.get(module_name)?;
    let class_symbol = ffi_mod.symbols.iter().find(|s| s.name == *class_name)?;
    let asatsuyu_hir::ffi::FfiSymbolKind::Class(cls) = &class_symbol.kind else {
        return None;
    };
    let (_, sig) = cls.methods.iter().find(|(name, _)| name == method)?;
    let trust_doc = ffi_trust_doc(ffi_mod.trust_level);
    let (label, params) = build_ffi_sig_label(&format!("{class_name}.{method}"), sig);
    let doc = build_ffi_doc(trust_doc, sig.is_async);
    Some((label, params, Some(doc)))
}

fn build_ffi_sig_label(
    name: &str,
    sig: &asatsuyu_hir::ffi::FfiSignature,
) -> (String, Vec<ParamInfo>) {
    let params: Vec<ParamInfo> = sig
        .params
        .iter()
        .map(|p| ParamInfo { label: format!("{}: {}", p.name, format_ffi_type(&p.ty)) })
        .collect();

    let param_labels: Vec<&str> = params.iter().map(|p| p.label.as_str()).collect();
    let ret = format_ffi_type(&sig.return_ty);
    let async_prefix = if sig.is_async { "async " } else { "" };
    let label = format!("{async_prefix}{}({}) -> {}", name, param_labels.join(", "), ret);

    (label, params)
}

/// Human-readable trust level label for FFI documentation.
fn ffi_trust_doc(level: asatsuyu_hir::ffi::FfiTrustLevel) -> &'static str {
    match level {
        asatsuyu_hir::ffi::FfiTrustLevel::Verified => "[Verified FFI]",
        asatsuyu_hir::ffi::FfiTrustLevel::Checked => "[Checked FFI]",
        asatsuyu_hir::ffi::FfiTrustLevel::Unsafe => "[Unsafe FFI]",
    }
}

/// Build documentation string for an FFI call, including trust level and async hint.
fn build_ffi_doc(trust_doc: &str, is_async: bool) -> String {
    if is_async {
        format!("{trust_doc} Returns `Task(T)` \u{2014} consider using `await`")
    } else {
        trust_doc.to_owned()
    }
}

fn format_ffi_type(ty: &asatsuyu_hir::ffi::FfiType) -> String {
    use asatsuyu_hir::ffi::FfiType;
    match ty {
        FfiType::Int => "Int".to_owned(),
        FfiType::Float => "Float".to_owned(),
        FfiType::Str => "String".to_owned(),
        FfiType::Bool => "Bool".to_owned(),
        FfiType::NoneType => "None".to_owned(),
        FfiType::Bytes => "Bytes".to_owned(),
        FfiType::List(inner) => format!("List({})", format_ffi_type(inner)),
        FfiType::Dict(k, v) => format!("Dict({}, {})", format_ffi_type(k), format_ffi_type(v)),
        FfiType::Tuple(elems) => {
            let inner: Vec<String> = elems.iter().map(format_ffi_type).collect();
            format!("Tuple({})", inner.join(", "))
        }
        FfiType::Optional(inner) => format!("Option({})", format_ffi_type(inner)),
        FfiType::Union(members) => {
            let inner: Vec<String> = members.iter().map(format_ffi_type).collect();
            inner.join(" | ")
        }
        FfiType::Named { module, name } => format!("{module}.{name}"),
        FfiType::Any => "Any".to_owned(),
    }
}

// ── Code actions ────────────────────────────────────────────────

/// What kind of code action this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CodeActionKindTag {
    QuickFix,
    Refactor,
}

/// A code action produced by analysis.
pub(super) struct CodeActionInfo {
    pub title: String,
    pub kind: CodeActionKindTag,
    /// Range in the source to replace (byte offsets).
    pub replace_start: u32,
    pub replace_end: u32,
    pub new_text: String,
}

/// Collect code actions relevant to the given diagnostics and cursor range.
///
/// Diagnostic-driven actions use the diagnostic message/hints text to extract
/// the fix. Refactor actions (e.g. add type annotation) are driven by the
/// cursor position in the THIR.
pub(super) fn collect_code_actions(
    thir: Option<&ThirModule>,
    source: &str,
    diagnostics: &[(String, String)],
    cursor_offset: u32,
) -> Vec<CodeActionInfo> {
    let mut actions = Vec::new();

    // Diagnostic-driven actions.
    for (code, message) in diagnostics {
        match code.as_str() {
            "E0300" => {
                if let Some(action) = action_add_missing_match_arms(source, message, cursor_offset)
                {
                    actions.push(action);
                }
            }
            "E0215" => {
                if let Some(action) = action_make_mutable(source, message) {
                    actions.push(action);
                }
            }
            "E0152" => {
                actions.extend(action_add_imports(source, message));
            }
            "E0208" => {
                actions.extend(action_generate_python_imports(source, message));
            }
            "E0220" => {
                if let Some(action) = action_make_fn_async(source, cursor_offset) {
                    actions.push(action);
                }
            }
            _ => {
                // E0200 with "consider adding `await`" hint.
                if code == "E0200" && message.contains("consider adding `await`") {
                    actions.push(action_add_await(source, message, cursor_offset));
                }
            }
        }
    }

    // Refactor: add type annotation for let binding at cursor.
    if let Some(module) = thir {
        if let Some(action) = action_add_type_annotation(module, source, cursor_offset) {
            actions.push(action);
        }
        // Refactor: convert let to let mut at cursor.
        if let Some(action) = action_let_to_let_mut(module, source, cursor_offset) {
            actions.push(action);
        }
    }

    actions
}

/// E0300: Add missing match arms.
///
/// Parses the hint `"add arms for: X, Y"` from the diagnostic message.
#[allow(clippy::items_after_statements)]
fn action_add_missing_match_arms(
    source: &str,
    message: &str,
    diag_offset: u32,
) -> Option<CodeActionInfo> {
    // Extract constructor names from "hint: add arms for: X, Y".
    let prefix = "hint: add arms for: ";
    let hint_start = message.find(prefix)?;
    let constructors_str = &message[hint_start + prefix.len()..];
    // The hint may be followed by more lines; take only the first line.
    let constructors_str = constructors_str.lines().next().unwrap_or(constructors_str);
    let constructors: Vec<&str> = constructors_str.split(", ").collect();

    if constructors.is_empty() {
        return None;
    }

    // Find the closing `}` of the match block after the diagnostic span.
    let search_start = diag_offset as usize;
    let match_end = find_match_closing_brace(source, search_start)?;

    // Build the new arms text.
    use std::fmt::Write;
    let mut arms_text = String::new();
    for ctor in &constructors {
        let _ = writeln!(arms_text, "    {ctor}(_) -> todo()");
    }

    #[allow(clippy::cast_possible_truncation)]
    let insert_pos = match_end as u32;
    Some(CodeActionInfo {
        title: format!("Add missing match arms: {}", constructors.join(", ")),
        kind: CodeActionKindTag::QuickFix,
        replace_start: insert_pos,
        replace_end: insert_pos,
        new_text: arms_text,
    })
}

/// Find the `}` that closes a match block, starting from `start`.
fn find_match_closing_brace(source: &str, start: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut seen_open = false;
    for (i, byte) in source[start..].bytes().enumerate() {
        match byte {
            b'{' => {
                depth += 1;
                seen_open = true;
            }
            b'}' => {
                if !seen_open {
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    return Some(start + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// E0215: Make binding mutable.
///
/// Extracts the binding name from the diagnostic message
/// and finds `let x` in source to insert `mut `.
fn action_make_mutable(source: &str, message: &str) -> Option<CodeActionInfo> {
    // Extract binding name from message.
    let prefix = "cannot assign to immutable binding `";
    let name_start = message.find(prefix)? + prefix.len();
    let name_end = message[name_start..].find('`')? + name_start;
    let binding_name = &message[name_start..name_end];

    // Find `let <name>` in source (first occurrence).
    let let_pattern = format!("let {binding_name}");
    let let_pos = source.find(&let_pattern)?;
    let insert_pos = let_pos + 4; // after "let "

    #[allow(clippy::cast_possible_truncation)]
    Some(CodeActionInfo {
        title: format!("Make `{binding_name}` mutable"),
        kind: CodeActionKindTag::QuickFix,
        replace_start: insert_pos as u32,
        replace_end: insert_pos as u32,
        new_text: "mut ".to_owned(),
    })
}

/// E0200 with Task hint: Add missing `await`.
fn action_add_await(_source: &str, _message: &str, diag_offset: u32) -> CodeActionInfo {
    // The diagnostic span points to the expression that has type Task(T).
    // Insert `await ` before the expression.
    CodeActionInfo {
        title: "Add `await` to unwrap Task".to_owned(),
        kind: CodeActionKindTag::QuickFix,
        replace_start: diag_offset,
        replace_end: diag_offset,
        new_text: "await ".to_owned(),
    }
}

/// E0152: Add missing import for an unresolved name.
///
/// Extracts the unresolved name from the diagnostic message and suggests
/// a `from python import <name>` statement at the import section.
fn action_add_imports(source: &str, message: &str) -> Vec<CodeActionInfo> {
    // Extract name from "unresolved name `foo`".
    let prefix = "unresolved name `";
    let Some(name_start) = message.find(prefix).map(|idx| idx + prefix.len()) else {
        return Vec::new();
    };
    let Some(name_end) = message[name_start..].find('`').map(|idx| idx + name_start) else {
        return Vec::new();
    };
    let name = &message[name_start..name_end];

    let insert_pos = find_import_insert_position(source);
    let alias = import_alias(name);
    let mut actions = Vec::with_capacity(2);

    #[allow(clippy::cast_possible_truncation)]
    actions.push(CodeActionInfo {
        title: format!("Add import: from python import {name}"),
        kind: CodeActionKindTag::QuickFix,
        replace_start: insert_pos,
        replace_end: insert_pos,
        new_text: format!("from python import {name}\n"),
    });
    #[allow(clippy::cast_possible_truncation)]
    actions.push(CodeActionInfo {
        title: format!("Add alias import: from python import {name} as {alias}"),
        kind: CodeActionKindTag::QuickFix,
        replace_start: insert_pos,
        replace_end: insert_pos,
        new_text: format!("from python import {name} as {alias}\n"),
    });
    actions
}

/// E0208: Generate `from python import` for an unknown module.
///
/// Extracts the module name from the diagnostic message.
fn action_generate_python_imports(source: &str, message: &str) -> Vec<CodeActionInfo> {
    // Extract module name from "unknown Python module `foo`".
    let prefix = "unknown Python module `";
    let Some(name_start) = message.find(prefix).map(|idx| idx + prefix.len()) else {
        return Vec::new();
    };
    let Some(name_end) = message[name_start..].find('`').map(|idx| idx + name_start) else {
        return Vec::new();
    };
    let module_name = &message[name_start..name_end];

    let insert_pos = find_import_insert_position(source);
    let alias = import_alias(module_name);
    let mut actions = Vec::with_capacity(2);

    #[allow(clippy::cast_possible_truncation)]
    actions.push(CodeActionInfo {
        title: format!("Add: from python import {module_name}"),
        kind: CodeActionKindTag::QuickFix,
        replace_start: insert_pos,
        replace_end: insert_pos,
        new_text: format!("from python import {module_name}\n"),
    });
    #[allow(clippy::cast_possible_truncation)]
    actions.push(CodeActionInfo {
        title: format!("Add alias import: from python import {module_name} as {alias}"),
        kind: CodeActionKindTag::QuickFix,
        replace_start: insert_pos,
        replace_end: insert_pos,
        new_text: format!("from python import {module_name} as {alias}\n"),
    });
    actions
}

fn import_alias(name: &str) -> String {
    let mut alias = String::from("py_");
    let mut prev_was_sep = false;
    for ch in name.chars() {
        let mapped = if ch.is_ascii_alphanumeric() || ch == '_' { ch } else { '_' };
        let mapped = mapped.to_ascii_lowercase();
        if mapped == '_' {
            if !prev_was_sep {
                alias.push('_');
            }
            prev_was_sep = true;
        } else {
            alias.push(mapped);
            prev_was_sep = false;
        }
    }
    while alias.ends_with('_') {
        alias.pop();
    }
    if alias == "py" { "py_alias".to_owned() } else { alias }
}

/// E0220: Make the enclosing function async.
///
/// Finds the `fn` keyword before the cursor and inserts `async ` before it.
fn action_make_fn_async(source: &str, cursor_offset: u32) -> Option<CodeActionInfo> {
    // Search backwards from cursor for `fn ` to find the enclosing function.
    let before = source.get(..cursor_offset as usize)?;
    let fn_pos = before.rfind("fn ")?;

    // Check this isn't already `async fn`.
    if fn_pos >= 6 && source.get(fn_pos - 6..fn_pos)?.trim_start().ends_with("async") {
        return None;
    }

    #[allow(clippy::cast_possible_truncation)]
    Some(CodeActionInfo {
        title: "Make function async".to_owned(),
        kind: CodeActionKindTag::QuickFix,
        replace_start: fn_pos as u32,
        replace_end: fn_pos as u32,
        new_text: "async ".to_owned(),
    })
}

/// Find the insertion position for a new import statement.
///
/// Returns the byte offset immediately after the last import/from line,
/// or 0 if there are no import statements.
fn find_import_insert_position(source: &str) -> u32 {
    let mut last_import_end = 0u32;
    let mut offset = 0u32;
    for line in source.lines() {
        let trimmed = line.trim();
        #[allow(clippy::cast_possible_truncation)]
        let line_end = offset + line.len() as u32 + 1; // +1 for newline
        if trimmed.starts_with("import ") || trimmed.starts_with("from ") {
            #[allow(clippy::cast_possible_truncation)]
            {
                last_import_end = line_end.min(source.len() as u32);
            }
        }
        offset = line_end;
    }
    last_import_end
}

/// Refactor: Convert `let` to `let mut` at cursor (no diagnostic needed).
fn action_let_to_let_mut(module: &ThirModule, source: &str, offset: u32) -> Option<CodeActionInfo> {
    for func in &module.functions {
        if !func.span.contains(offset) {
            continue;
        }
        if let Some(action) =
            find_let_for_mut_refactor(&func.body, source, offset, &module.symbol_table)
        {
            return Some(action);
        }
    }
    None
}

/// Walk the THIR looking for a `let` (immutable) binding at `offset` to convert to `let mut`.
fn find_let_for_mut_refactor(
    expr: &ThirExpr,
    source: &str,
    offset: u32,
    st: &asatsuyu_hir::SymbolTable,
) -> Option<CodeActionInfo> {
    if !expr.span().contains(offset) {
        return None;
    }
    match expr {
        ThirExpr::Let { binding, is_mutable, span, .. } if span.contains(offset) => {
            if *is_mutable {
                return None; // Already mutable.
            }
            let def = st.get(*binding);
            // Find `let name` in source and insert `mut `.
            let let_prefix = "let ";
            let search_start = span.start as usize;
            let fragment = source.get(search_start..span.end as usize)?;
            let let_offset = fragment.find(let_prefix)?;
            #[allow(clippy::cast_possible_truncation)]
            let insert_pos = (search_start + let_offset + let_prefix.len()) as u32;
            Some(CodeActionInfo {
                title: format!("Make `{}` mutable", def.name),
                kind: CodeActionKindTag::Refactor,
                replace_start: insert_pos,
                replace_end: insert_pos,
                new_text: "mut ".to_owned(),
            })
        }
        ThirExpr::Block { exprs, .. } => {
            exprs.iter().find_map(|e| find_let_for_mut_refactor(e, source, offset, st))
        }
        ThirExpr::If { condition, then_body, else_body, .. } => {
            find_let_for_mut_refactor(condition, source, offset, st)
                .or_else(|| find_let_for_mut_refactor(then_body, source, offset, st))
                .or_else(|| {
                    else_body
                        .as_ref()
                        .and_then(|e| find_let_for_mut_refactor(e, source, offset, st))
                })
        }
        ThirExpr::Match { arms, .. } => {
            arms.iter().find_map(|arm| find_let_for_mut_refactor(&arm.body, source, offset, st))
        }
        _ => None,
    }
}

/// Refactor: Add type annotation to a let binding.
fn action_add_type_annotation(
    module: &ThirModule,
    source: &str,
    offset: u32,
) -> Option<CodeActionInfo> {
    // Find a Let node at the cursor position.
    for func in &module.functions {
        if !func.span.contains(offset) {
            continue;
        }
        if let Some(action) =
            find_let_for_annotation(&func.body, source, offset, &module.symbol_table)
        {
            return Some(action);
        }
    }
    None
}

fn find_let_for_annotation(
    expr: &ThirExpr,
    source: &str,
    offset: u32,
    st: &asatsuyu_hir::SymbolTable,
) -> Option<CodeActionInfo> {
    if !expr.span().contains(offset) {
        return None;
    }
    match expr {
        ThirExpr::Let { binding, value, span, .. } if span.contains(offset) => {
            let def = st.get(*binding);
            let name_end = def.span.end;
            // Check if there's already a `:` between name and `=`.
            let between = source.get(name_end as usize..span.end as usize).unwrap_or("");
            if between.contains(':') {
                return None; // Already has annotation.
            }
            // Use the value's type (the binding's inferred type), not the
            // let expression's type (which is always `None`).
            let value_ty = value.ty();
            if matches!(value_ty, Ty::Error) {
                return None; // Don't suggest error types.
            }
            Some(CodeActionInfo {
                title: format!("Add type annotation: {value_ty}"),
                kind: CodeActionKindTag::Refactor,
                replace_start: name_end,
                replace_end: name_end,
                new_text: format!(": {value_ty}"),
            })
        }
        ThirExpr::Block { exprs, .. } => {
            exprs.iter().find_map(|e| find_let_for_annotation(e, source, offset, st))
        }
        ThirExpr::If { condition, then_body, else_body, .. } => {
            find_let_for_annotation(condition, source, offset, st)
                .or_else(|| find_let_for_annotation(then_body, source, offset, st))
                .or_else(|| {
                    else_body.as_ref().and_then(|e| find_let_for_annotation(e, source, offset, st))
                })
        }
        ThirExpr::Match { arms, .. } => {
            arms.iter().find_map(|arm| find_let_for_annotation(&arm.body, source, offset, st))
        }
        _ => None,
    }
}

// ── Find all references ─────────────────────────────────────────

/// Collect all spans that reference or define the given `DefId`.
pub(super) fn find_all_references(module: &ThirModule, target: DefId) -> Vec<Span> {
    let mut spans = Vec::new();
    let st = &module.symbol_table;

    // Include the definition itself.
    let def = st.get(target);
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
        collect_refs_in_expr(&func.body, target, &mut spans, st);
    }

    spans
}

#[allow(clippy::only_used_in_recursion)] // `st` is used in the Assign branch
fn collect_refs_in_expr(
    expr: &ThirExpr,
    target: DefId,
    spans: &mut Vec<Span>,
    st: &asatsuyu_hir::SymbolTable,
) {
    match expr {
        ThirExpr::Var { def_id, span, .. } if *def_id == target => {
            spans.push(*span);
        }
        ThirExpr::Block { exprs, .. } => {
            for e in exprs {
                collect_refs_in_expr(e, target, spans, st);
            }
        }
        ThirExpr::Call { func, args, .. } => {
            collect_refs_in_expr(func, target, spans, st);
            for a in args {
                collect_refs_in_expr(a, target, spans, st);
            }
        }
        ThirExpr::BinaryOp { lhs, rhs, .. } => {
            collect_refs_in_expr(lhs, target, spans, st);
            collect_refs_in_expr(rhs, target, spans, st);
        }
        ThirExpr::UnaryOp { expr: inner, .. }
        | ThirExpr::FieldAccess { receiver: inner, .. }
        | ThirExpr::Try { expr: inner, .. }
        | ThirExpr::Await { expr: inner, .. } => {
            collect_refs_in_expr(inner, target, spans, st);
        }
        ThirExpr::If { condition, then_body, else_body, .. } => {
            collect_refs_in_expr(condition, target, spans, st);
            collect_refs_in_expr(then_body, target, spans, st);
            if let Some(e) = else_body {
                collect_refs_in_expr(e, target, spans, st);
            }
        }
        ThirExpr::Match { subject, arms, .. } => {
            collect_refs_in_expr(subject, target, spans, st);
            for arm in arms {
                collect_refs_in_pattern(&arm.pattern, target, spans);
                collect_refs_in_expr(&arm.body, target, spans, st);
            }
        }
        ThirExpr::Let { binding, value, .. } => {
            if *binding == target {
                // The let binding itself — definition span is already included.
            }
            collect_refs_in_expr(value, target, spans, st);
        }
        ThirExpr::Assign { target: t, value, target_span, .. } => {
            if *t == target {
                // Use the target identifier span, not the full assignment
                // expression span (Issue 104).
                spans.push(*target_span);
            }
            collect_refs_in_expr(value, target, spans, st);
        }
        ThirExpr::Lambda { params, body, .. } => {
            for p in params {
                if p.def_id == target {
                    spans.push(p.span);
                }
            }
            collect_refs_in_expr(body, target, spans, st);
        }
        ThirExpr::List { elements, .. } => {
            for e in elements {
                collect_refs_in_expr(e, target, spans, st);
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CompletionEntryKind {
    Symbol(DefKind),
    Keyword,
    /// Python module-level function.
    FfiFunction,
    /// Python class.
    FfiClass,
    /// Python instance property.
    FfiProperty,
    /// Python instance method.
    FfiMethod,
    /// Python constant or module-level attribute.
    FfiConstant,
    /// Python module name (for import completion).
    FfiModule,
}

/// Whether the insert text should be treated as a snippet with placeholders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum InsertTextFormatTag {
    #[default]
    PlainText,
    Snippet,
}

/// A completion candidate with metadata.
pub(super) struct CompletionEntry {
    pub name: SmolStr,
    pub kind: CompletionEntryKind,
    pub ty: Option<Ty>,
    /// Optional detail string for FFI completions (signature, type info).
    /// Takes precedence over `ty` for display when set.
    pub detail: Option<String>,
    pub insert_text: Option<SmolStr>,
    pub insert_text_format: InsertTextFormatTag,
}

/// The syntactic context at the completion cursor position.
///
/// Determines which keywords are valid completions. Classified from source
/// text alone (no THIR required) so it works during editing with parse errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CompletionContext {
    /// Outside any function/type body. Offer item-level keywords.
    TopLevel,
    /// Inside a block expression (function body, if/match/lambda body).
    Block,
    /// In an expression position within a block.
    Expr,
    /// After `from`, where the FFI source keyword should be suggested.
    ImportFrom,
    /// After `from python import `, where Python module names are suggested.
    ImportPythonModule,
    /// Inside an import statement line. Suppress keyword completions.
    Import,
    /// After `module.` where `module` has type `Ty::FfiModule`.
    FfiModuleMember { module_name: SmolStr },
    /// After `instance.` where `instance` has type `Ty::FfiInstance`.
    FfiInstanceMember { module_name: SmolStr, class_name: SmolStr },
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
                        detail: None,
                        insert_text: None,
                        insert_text_format: InsertTextFormatTag::default(),
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
                    detail: None,
                    insert_text: None,
                    insert_text_format: InsertTextFormatTag::default(),
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

/// Classify the completion context at `offset` using source text heuristics
/// and optional THIR type information.
///
/// Scans backwards from the cursor to determine whether we are at the top
/// level, inside a block, on an import line, or after a dot on an FFI value.
pub(super) fn classify_context(
    source: &str,
    offset: u32,
    thir: Option<&ThirModule>,
) -> CompletionContext {
    let offset = (offset as usize).min(source.len());
    let before_cursor = &source[..offset];

    // Check if current line starts with import/from keywords.
    let line_start = before_cursor.rfind('\n').map_or(0, |i| i + 1);
    let line_prefix = before_cursor[line_start..].trim_start();

    // `from python import |` — module name completion
    if line_prefix.starts_with("from python import ") {
        return CompletionContext::ImportPythonModule;
    }
    if line_prefix.starts_with("from ") && !line_prefix.contains(" import") {
        return CompletionContext::ImportFrom;
    }
    if line_prefix.starts_with("import ") || line_prefix.starts_with("from ") {
        return CompletionContext::Import;
    }

    // Check for dot-access: `identifier.|`
    if let Some(ffi_ctx) = classify_dot_context(before_cursor, thir) {
        return ffi_ctx;
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
///
/// Keywords that represent block-level constructs are offered as snippet
/// completions with tab-stop placeholders so the user can fill in the
/// skeleton immediately.
#[allow(clippy::needless_pass_by_value)]
fn collect_keyword_completions(ctx: CompletionContext) -> Vec<CompletionEntry> {
    let mut entries = Vec::new();

    // Plain keyword completions (no snippet expansion).
    let allowed: &[&str] = match &ctx {
        CompletionContext::TopLevel => &["pub"],
        CompletionContext::Block => &["True", "False"],
        CompletionContext::Expr => &["True", "False", "await"],
        CompletionContext::ImportFrom
        | CompletionContext::ImportPythonModule
        | CompletionContext::Import
        | CompletionContext::FfiModuleMember { .. }
        | CompletionContext::FfiInstanceMember { .. } => &[],
    };

    for spec in completion_keyword_specs().filter(|spec| allowed.contains(&spec.text)) {
        entries.push(CompletionEntry {
            name: SmolStr::new(spec.text),
            kind: CompletionEntryKind::Keyword,
            ty: None,
            detail: None,
            insert_text: None,
            insert_text_format: InsertTextFormatTag::default(),
        });
    }

    // Snippet-backed completions with placeholders.
    match &ctx {
        CompletionContext::TopLevel => {
            entries.push(keyword_snippet_expand("fn", "fn ${1:name}(${2}) {\n\t$0\n}"));
            entries.push(keyword_snippet_expand("pub fn", "pub fn ${1:name}(${2}) {\n\t$0\n}"));
            entries.push(keyword_snippet_expand("async fn", "async fn ${1:name}(${2}) {\n\t$0\n}"));
            entries.push(keyword_snippet_expand("type", "type ${1:Name} {\n\t${2:Variant}($0)\n}"));
            entries.push(keyword_snippet_expand(
                "from python import",
                "from python import ${1:module}",
            ));
            entries.push(keyword_snippet("import", "import "));
        }
        CompletionContext::Block => {
            entries.push(keyword_snippet_expand("let", "let ${1:name} = ${0}"));
            entries.push(keyword_snippet_expand("let mut", "let mut ${1:name} = ${0}"));
            entries.push(keyword_snippet_expand("if", "if ${1:condition} {\n\t$0\n}"));
            entries.push(keyword_snippet_expand(
                "match",
                "match ${1:value} {\n\t${2:pattern} -> $0\n}",
            ));
            entries.push(keyword_snippet("try", "try "));
            entries.push(keyword_snippet_expand("fn", "fn ${1:name}(${2}) {\n\t$0\n}"));
            entries.push(keyword_snippet("await", "await "));
        }
        CompletionContext::Expr => {
            entries.push(keyword_snippet_expand("if", "if ${1:condition} {\n\t$0\n}"));
            entries.push(keyword_snippet_expand(
                "match",
                "match ${1:value} {\n\t${2:pattern} -> $0\n}",
            ));
            entries.push(keyword_snippet("try", "try "));
            entries.push(keyword_snippet_expand("fn", "fn(${1}) {\n\t$0\n}"));
            entries.push(keyword_snippet("mut", "mut "));
        }
        CompletionContext::ImportFrom => {
            entries.push(keyword_snippet("python", "python "));
        }
        CompletionContext::Import
        | CompletionContext::ImportPythonModule
        | CompletionContext::FfiModuleMember { .. }
        | CompletionContext::FfiInstanceMember { .. } => {}
    }

    entries
}

/// A keyword completion with plain text insertion (no placeholders).
fn keyword_snippet(label: &'static str, insert_text: &'static str) -> CompletionEntry {
    CompletionEntry {
        name: SmolStr::new(label),
        kind: CompletionEntryKind::Keyword,
        ty: None,
        detail: None,
        insert_text: Some(SmolStr::new(insert_text)),
        insert_text_format: InsertTextFormatTag::PlainText,
    }
}

/// A keyword completion with snippet expansion (tab-stop placeholders).
fn keyword_snippet_expand(label: &'static str, snippet: &'static str) -> CompletionEntry {
    CompletionEntry {
        name: SmolStr::new(label),
        kind: CompletionEntryKind::Keyword,
        ty: None,
        detail: None,
        insert_text: Some(SmolStr::new(snippet)),
        insert_text_format: InsertTextFormatTag::Snippet,
    }
}

// ── FFI dot-access context classification ─────────────────────────

/// Detect whether the cursor is immediately after `identifier.` and determine
/// the FFI context from the receiver's THIR type.
fn classify_dot_context(
    before_cursor: &str,
    thir: Option<&ThirModule>,
) -> Option<CompletionContext> {
    // The cursor must be right after a `.`
    let trimmed = before_cursor.trim_end();
    if !trimmed.ends_with('.') {
        return None;
    }
    // Extract the identifier before the dot.
    let before_dot = &trimmed[..trimmed.len() - 1];
    let ident = extract_trailing_identifier(before_dot)?;

    let module = thir?;

    // Walk the symbol table to find the identifier's type.
    let ty = resolve_identifier_type(ident, before_cursor, module)?;

    match ty {
        Ty::FfiModule { module_name } => {
            Some(CompletionContext::FfiModuleMember { module_name: module_name.clone() })
        }
        Ty::FfiInstance { module, class } => Some(CompletionContext::FfiInstanceMember {
            module_name: module.clone(),
            class_name: class.clone(),
        }),
        _ => None,
    }
}

/// Extract the trailing identifier from text (e.g., `"  pathlib"` → `"pathlib"`).
fn extract_trailing_identifier(text: &str) -> Option<&str> {
    let text = text.trim_end();
    if text.is_empty() {
        return None;
    }
    // Walk backwards to find identifier start.
    let start =
        text.bytes().rposition(|b| !b.is_ascii_alphanumeric() && b != b'_').map_or(0, |i| i + 1);
    let ident = &text[start..];
    if ident.is_empty() || ident.bytes().next().is_none_or(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(ident)
}

/// Resolve the type of a named identifier using the THIR symbol table and
/// scope-aware analysis (parameters, let bindings visible at the cursor).
fn resolve_identifier_type<'a>(
    ident: &str,
    source: &str,
    module: &'a ThirModule,
) -> Option<&'a Ty> {
    // Check module-level definitions first.
    for (def_id, def) in module.symbol_table.iter() {
        if def.name == ident {
            // For imports, find the type in the type environment.
            if def.kind == DefKind::Import {
                // Import variables are given Ty::FfiModule during type checking.
                // Find the corresponding ThirExpr::Var in function bodies.
                return find_var_type_in_module(def_id, module);
            }
        }
    }

    // Check function parameters and let bindings at the cursor position.
    #[allow(clippy::cast_possible_truncation)]
    let cursor_offset = source.len() as u32;
    for func in &module.functions {
        if !func.span.contains(cursor_offset) {
            continue;
        }
        // Check parameters.
        for param in &func.params {
            let def = module.symbol_table.get(param.def_id);
            if def.name == ident {
                return Some(&param.ty);
            }
        }
        // Walk the body for let bindings.
        if let Some(ty) =
            find_binding_type_in_expr(&func.body, ident, cursor_offset, &module.symbol_table)
        {
            return Some(ty);
        }
    }

    None
}

/// Find the type of a variable (by `DefId`) in the function bodies of the module.
fn find_var_type_in_module(def_id: DefId, module: &ThirModule) -> Option<&Ty> {
    for func in &module.functions {
        if let Some(ty) = find_var_type_in_expr(&func.body, def_id) {
            return Some(ty);
        }
    }
    None
}

/// Walk a THIR expression to find a Var referencing `target_def_id` and return its type.
fn find_var_type_in_expr(expr: &ThirExpr, target_def_id: DefId) -> Option<&Ty> {
    match expr {
        ThirExpr::Var { def_id, ty, .. } if *def_id == target_def_id => Some(ty),
        ThirExpr::Block { exprs, .. } => {
            exprs.iter().find_map(|e| find_var_type_in_expr(e, target_def_id))
        }
        ThirExpr::Let { value, .. } | ThirExpr::Assign { value, .. } => {
            find_var_type_in_expr(value, target_def_id)
        }
        ThirExpr::Call { func, args, .. } => find_var_type_in_expr(func, target_def_id)
            .or_else(|| args.iter().find_map(|a| find_var_type_in_expr(a, target_def_id))),
        ThirExpr::If { condition, then_body, else_body, .. } => {
            find_var_type_in_expr(condition, target_def_id)
                .or_else(|| find_var_type_in_expr(then_body, target_def_id))
                .or_else(|| {
                    else_body.as_ref().and_then(|e| find_var_type_in_expr(e, target_def_id))
                })
        }
        ThirExpr::FieldAccess { receiver, .. }
        | ThirExpr::Try { expr: receiver, .. }
        | ThirExpr::Await { expr: receiver, .. }
        | ThirExpr::UnaryOp { expr: receiver, .. } => {
            find_var_type_in_expr(receiver, target_def_id)
        }
        ThirExpr::BinaryOp { lhs, rhs, .. } => find_var_type_in_expr(lhs, target_def_id)
            .or_else(|| find_var_type_in_expr(rhs, target_def_id)),
        ThirExpr::Match { subject, arms, .. } => find_var_type_in_expr(subject, target_def_id)
            .or_else(|| {
                arms.iter().find_map(|arm| find_var_type_in_expr(&arm.body, target_def_id))
            }),
        ThirExpr::Lambda { body, .. } => find_var_type_in_expr(body, target_def_id),
        ThirExpr::List { elements, .. } => {
            elements.iter().find_map(|e| find_var_type_in_expr(e, target_def_id))
        }
        _ => None,
    }
}

/// Find the type of a let-bound variable by name within an expression tree.
fn find_binding_type_in_expr<'a>(
    expr: &'a ThirExpr,
    ident: &str,
    cursor_offset: u32,
    st: &asatsuyu_hir::SymbolTable,
) -> Option<&'a Ty> {
    match expr {
        ThirExpr::Block { exprs, .. } => {
            for e in exprs {
                if e.span().start >= cursor_offset {
                    break;
                }
                if let Some(ty) = find_binding_type_in_expr(e, ident, cursor_offset, st) {
                    return Some(ty);
                }
            }
            None
        }
        ThirExpr::Let { binding, ty, .. } => {
            let def = st.get(*binding);
            if def.name == ident && def.span.start < cursor_offset {
                return Some(ty);
            }
            None
        }
        _ => None,
    }
}

// ── FFI completion collectors ─────────────────────────────────────

/// Known Python module names available for `from python import` completion.
const KNOWN_PYTHON_MODULES: &[&str] = &["pathlib", "json", "os", "sys", "requests", "asyncio"];

/// Collect Python module name candidates for `from python import |`.
fn collect_ffi_import_module_completions(thir: Option<&ThirModule>) -> Vec<CompletionEntry> {
    let mut names: Vec<SmolStr> = KNOWN_PYTHON_MODULES.iter().map(|&n| SmolStr::new(n)).collect();

    // Also include any modules already resolved in the THIR (covers custom configs).
    if let Some(module) = thir {
        for key in module.ffi_modules.keys() {
            if !names.iter().any(|n| n == key) {
                names.push(key.clone());
            }
        }
    }

    names.sort();
    names
        .into_iter()
        .map(|name| CompletionEntry {
            name,
            kind: CompletionEntryKind::FfiModule,
            ty: None,
            detail: None,
            insert_text: None,
            insert_text_format: InsertTextFormatTag::default(),
        })
        .collect()
}

/// Collect module-level symbol completions for `module.|` using the rich
/// `PythonApiIndex`. Falls back to `FfiModule` if no index is available.
fn collect_ffi_module_member_completions(
    thir: &ThirModule,
    module_name: &str,
) -> Vec<CompletionEntry> {
    // Try the rich PythonApiIndex first.
    if let Some(index) = &thir.python_api_index
        && let Some(module_info) = index.get(module_name)
    {
        return module_info
            .symbols
            .iter()
            .map(|sym| {
                use asatsuyu_hir::ffi::PythonSymbolKind;
                let (kind, detail) = match &sym.kind {
                    PythonSymbolKind::Function(info) => {
                        let detail = info.signatures.first().map(format_ffi_signature_brief);
                        (CompletionEntryKind::FfiFunction, detail)
                    }
                    PythonSymbolKind::Class(_) => {
                        (CompletionEntryKind::FfiClass, Some("class".to_owned()))
                    }
                    PythonSymbolKind::Constant(ty) => {
                        (CompletionEntryKind::FfiConstant, Some(format_ffi_type(ty)))
                    }
                    PythonSymbolKind::Module => {
                        (CompletionEntryKind::FfiModule, Some("module".to_owned()))
                    }
                };
                CompletionEntry {
                    name: sym.name.clone(),
                    kind,
                    ty: None,
                    detail,
                    insert_text: None,
                    insert_text_format: InsertTextFormatTag::default(),
                }
            })
            .collect();
    }

    // Fallback: use the flattened FfiModule.
    collect_ffi_module_member_completions_from_ffi(thir, module_name)
}

/// Fallback: collect module member completions from `FfiModule`.
fn collect_ffi_module_member_completions_from_ffi(
    thir: &ThirModule,
    module_name: &str,
) -> Vec<CompletionEntry> {
    let Some(ffi_mod) = thir.ffi_modules.get(module_name) else {
        return Vec::new();
    };
    ffi_mod
        .symbols
        .iter()
        .map(|sym| {
            use asatsuyu_hir::ffi::FfiSymbolKind;
            let kind = match &sym.kind {
                FfiSymbolKind::Function(_) => CompletionEntryKind::FfiFunction,
                FfiSymbolKind::Class(_) => CompletionEntryKind::FfiClass,
                FfiSymbolKind::Constant(_) => CompletionEntryKind::FfiConstant,
            };
            CompletionEntry {
                name: sym.name.clone(),
                kind,
                ty: None,
                detail: None,
                insert_text: None,
                insert_text_format: InsertTextFormatTag::default(),
            }
        })
        .collect()
}

/// Collect instance member completions for `instance.|`.
fn collect_ffi_instance_member_completions(
    thir: &ThirModule,
    module_name: &str,
    class_name: &str,
) -> Vec<CompletionEntry> {
    // Try the rich PythonApiIndex first.
    if let Some(index) = &thir.python_api_index
        && let Some(module_info) = index.get(module_name)
    {
        for sym in &module_info.symbols {
            if let asatsuyu_hir::ffi::PythonSymbolKind::Class(cls) = &sym.kind
                && sym.name == class_name
            {
                return collect_instance_members_from_class(cls);
            }
        }
    }

    // Fallback: use FfiModule.
    collect_ffi_instance_member_completions_from_ffi(thir, module_name, class_name)
}

/// Build instance member completions from `PythonClassInfo`.
fn collect_instance_members_from_class(
    cls: &asatsuyu_hir::ffi::PythonClassInfo,
) -> Vec<CompletionEntry> {
    let mut entries = Vec::new();

    // Instance methods.
    for method in &cls.methods {
        let detail = method.signatures.first().map(format_ffi_signature_brief);
        entries.push(CompletionEntry {
            name: method.name.clone(),
            kind: CompletionEntryKind::FfiMethod,
            ty: None,
            detail: None,
            insert_text: None,
            insert_text_format: InsertTextFormatTag::default(),
        });
        // Store detail in a different way — for now, ty field is unused for FFI entries
        let _ = detail;
    }

    // Properties.
    for (name, _ty) in &cls.properties {
        entries.push(CompletionEntry {
            name: name.clone(),
            kind: CompletionEntryKind::FfiProperty,
            ty: None,
            detail: None,
            insert_text: None,
            insert_text_format: InsertTextFormatTag::default(),
        });
    }

    entries
}

/// Fallback: collect instance member completions from `FfiModule`.
fn collect_ffi_instance_member_completions_from_ffi(
    thir: &ThirModule,
    module_name: &str,
    class_name: &str,
) -> Vec<CompletionEntry> {
    let Some(ffi_mod) = thir.ffi_modules.get(module_name) else {
        return Vec::new();
    };
    let class_sym = ffi_mod.symbols.iter().find(|s| s.name == class_name);
    let Some(class_sym) = class_sym else {
        return Vec::new();
    };
    let asatsuyu_hir::ffi::FfiSymbolKind::Class(cls) = &class_sym.kind else {
        return Vec::new();
    };

    let mut entries = Vec::new();
    for (name, _sig) in &cls.methods {
        entries.push(CompletionEntry {
            name: name.clone(),
            kind: CompletionEntryKind::FfiMethod,
            ty: None,
            detail: None,
            insert_text: None,
            insert_text_format: InsertTextFormatTag::default(),
        });
    }
    for (name, _ty) in &cls.properties {
        entries.push(CompletionEntry {
            name: name.clone(),
            kind: CompletionEntryKind::FfiProperty,
            ty: None,
            detail: None,
            insert_text: None,
            insert_text_format: InsertTextFormatTag::default(),
        });
    }
    entries
}

/// Format an FFI signature briefly for completion detail (e.g. `"(path: String) -> Bool"`).
fn format_ffi_signature_brief(sig: &asatsuyu_hir::ffi::FfiSignature) -> String {
    let params: Vec<String> =
        sig.params.iter().map(|p| format!("{}: {}", p.name, format_ffi_type(&p.ty))).collect();
    let ret = format_ffi_type(&sig.return_ty);
    format!("({}) -> {}", params.join(", "), ret)
}

/// Collect all completion candidates (keywords + symbols + FFI) at the given offset.
///
/// Works even when THIR is unavailable (keyword completions only in that case).
pub(super) fn collect_all_completions(
    thir: Option<&ThirModule>,
    source: &str,
    offset: u32,
) -> Vec<CompletionEntry> {
    let ctx = classify_context(source, offset, thir);

    match &ctx {
        CompletionContext::ImportPythonModule => {
            return collect_ffi_import_module_completions(thir);
        }
        CompletionContext::FfiModuleMember { module_name } => {
            if let Some(module) = thir {
                return collect_ffi_module_member_completions(module, module_name);
            }
            return Vec::new();
        }
        CompletionContext::FfiInstanceMember { module_name, class_name } => {
            if let Some(module) = thir {
                return collect_ffi_instance_member_completions(module, module_name, class_name);
            }
            return Vec::new();
        }
        _ => {}
    }

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
                    detail: None,
                    insert_text: None,
                    insert_text_format: InsertTextFormatTag::default(),
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
                        detail: None,
                        insert_text: None,
                        insert_text_format: InsertTextFormatTag::default(),
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
                    detail: None,
                    insert_text: None,
                    insert_text_format: InsertTextFormatTag::default(),
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
        assert_eq!(classify_context("", 0, None), CompletionContext::TopLevel);
    }

    #[test]
    fn classify_context_top_level_after_fn() {
        assert_eq!(classify_context("fn main() {}\n", 14, None), CompletionContext::TopLevel,);
    }

    #[test]
    fn classify_context_block_inside_fn() {
        let source = "fn main() {\n  \n}";
        assert_eq!(classify_context(source, 14, None), CompletionContext::Block);
    }

    #[test]
    fn classify_context_nested_block() {
        let source = "fn main() {\n  if True {\n    \n  }\n}";
        assert_eq!(classify_context(source, 27, None), CompletionContext::Block);
    }

    #[test]
    fn classify_context_expr_after_equals() {
        let source = "fn main() {\n  let x = \n}";
        assert_eq!(classify_context(source, 22, None), CompletionContext::Expr);
    }

    #[test]
    fn classify_context_expr_after_let_prefix() {
        let source = "fn main() {\n  let \n}";
        assert_eq!(classify_context(source, 18, None), CompletionContext::Expr);
    }

    #[test]
    fn classify_context_import_line() {
        assert_eq!(classify_context("import ", 7, None), CompletionContext::Import);
        assert_eq!(
            classify_context("from python import ", 19, None),
            CompletionContext::ImportPythonModule,
        );
    }

    #[test]
    fn classify_context_after_from_keyword() {
        assert_eq!(classify_context("from ", 5, None), CompletionContext::ImportFrom);
        assert_eq!(classify_context("from path", 9, None), CompletionContext::ImportFrom);
    }

    #[test]
    #[allow(clippy::cast_possible_truncation)]
    fn classify_context_after_closed_braces() {
        let source = "fn a() {}\nfn b() {}\n";
        assert_eq!(
            classify_context(source, source.len() as u32, None),
            CompletionContext::TopLevel,
        );
    }

    // ── Keyword completions ─────────────────────────────────────

    #[test]
    fn keyword_completions_top_level() {
        let entries = collect_keyword_completions(CompletionContext::TopLevel);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"fn"));
        assert!(names.contains(&"type"));
        assert!(names.contains(&"import"));
        assert!(names.contains(&"from python import"));
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
    fn keyword_completions_import_from_suggests_python() {
        let entries = collect_keyword_completions(CompletionContext::ImportFrom);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["python"]);
        assert_eq!(entries[0].insert_text.as_deref(), Some("python "));
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
        assert!(async_fn.insert_text.is_some());
        assert_eq!(async_fn.insert_text_format, InsertTextFormatTag::Snippet);

        let block_entries = collect_keyword_completions(CompletionContext::Block);
        let let_mut = block_entries.iter().find(|e| e.name == "let mut").expect("missing let mut");
        assert!(let_mut.insert_text.is_some());
        assert_eq!(let_mut.insert_text_format, InsertTextFormatTag::Snippet);
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

    // ── Compilation helper for tests ───────────────────────────

    fn compile_to_thir(source: &str) -> Option<ThirModule> {
        use asatsuyu_hir::ffi::FfiResolverConfig;
        use asatsuyu_syntax::FileId;

        let parse = asatsuyu_parser::parse(FileId(0), source);
        if parse.has_errors() {
            return None;
        }
        let ast = asatsuyu_ast::lower(&parse, FileId(0));
        if ast.has_errors() {
            return None;
        }
        let hir = asatsuyu_hir::lower_to_hir(&ast.module);
        if hir.has_errors() {
            return None;
        }
        let ffi_config = FfiResolverConfig::default();
        let ty = asatsuyu_ty::check_types_with_ffi_config(&hir.module, &ffi_config);
        Some(ty.module)
    }

    // ── Signature help tests ───────────────────────────────────

    #[test]
    fn signature_help_asatsuyu_fn() {
        let source = "fn add(a: Int, b: Int) -> Int { a }\nfn main() { add(1, 2) }";
        let thir = compile_to_thir(source).expect("should compile");
        // Cursor after `add(` — offset points inside the argument list.
        let paren_pos = source.find("add(1").unwrap() + 4; // after '('
        #[allow(clippy::cast_possible_truncation)]
        let info =
            signature_help_at_offset(&thir, source, paren_pos as u32).expect("should have sig");
        assert!(info.label.contains("add"), "label should contain fn name: {}", info.label);
        assert_eq!(info.parameters.len(), 2);
        assert!(info.parameters[0].label.contains('a'));
        assert!(info.parameters[1].label.contains('b'));
        assert_eq!(info.active_parameter, 0);
    }

    #[test]
    fn signature_help_active_param_advances() {
        let source = "fn add(a: Int, b: Int) -> Int { a }\nfn main() { add(1, 2) }";
        let thir = compile_to_thir(source).expect("should compile");
        // Cursor after the comma: `add(1, |2)`
        let comma_pos = source.find("add(1, ").unwrap() + 7; // after ", "
        #[allow(clippy::cast_possible_truncation)]
        let info =
            signature_help_at_offset(&thir, source, comma_pos as u32).expect("should have sig");
        assert_eq!(info.active_parameter, 1);
    }

    #[test]
    fn signature_help_none_outside_parens() {
        let source = "fn add(a: Int, b: Int) -> Int { a }\nfn main() { add(1, 2) }";
        let thir = compile_to_thir(source).expect("should compile");
        // Cursor before `add(` — not in any call.
        let before_add = source.find("add(1").unwrap();
        #[allow(clippy::cast_possible_truncation)]
        let result = signature_help_at_offset(&thir, source, before_add as u32);
        assert!(result.is_none(), "should be None outside call parens");
    }

    #[test]
    fn signature_help_ffi_call() {
        // Use os.getenv which has a simple, well-defined signature
        let source = "from python import os\nfn main() { os.getenv(\"HOME\") }";
        let thir = compile_to_thir(source).expect("should compile");
        let paren_pos = source.find("getenv(\"").unwrap() + 7; // after '('
        #[allow(clippy::cast_possible_truncation)]
        let info =
            signature_help_at_offset(&thir, source, paren_pos as u32).expect("should have sig");
        assert!(!info.parameters.is_empty(), "should have params");
    }

    #[test]
    fn signature_help_async_ffi_call() {
        let source = "from python import asyncio\nasync fn main() { await asyncio.sleep(1.0) }";
        let thir = compile_to_thir(source).expect("should compile");
        let paren_pos = source.find("sleep(1.0").unwrap() + 6; // after '('
        #[allow(clippy::cast_possible_truncation)]
        let info =
            signature_help_at_offset(&thir, source, paren_pos as u32).expect("should have sig");
        assert!(info.label.contains("asyncio.sleep"), "label: {}", info.label);
        assert_eq!(info.parameters.len(), 1);
        assert_eq!(info.parameters[0].label, "delay: Float");
    }

    #[test]
    fn count_active_parameter_handles_nesting() {
        // `f(a, g(b, c), d)` — commas inside g() should not count.
        let source = "f(a, g(b, c), d)";
        assert_eq!(count_active_parameter(source, 2, 3), 0); // at `a`
        assert_eq!(count_active_parameter(source, 2, 5), 1); // after first `,`
        assert_eq!(count_active_parameter(source, 2, 14), 2); // after `g(b, c),`
    }

    // ── Issue 104: rename/references hardening tests ───────────

    #[test]
    fn cursor_on_let_binding_name_resolves() {
        let source = "fn main() { let x = 1\n  x }";
        let thir = compile_to_thir(source).expect("should compile");
        // Cursor on `x` in `let x = 1`
        let x_pos = source.find("let x").unwrap() + 4; // on `x`
        #[allow(clippy::cast_possible_truncation)]
        let info = find_node_at_offset(&thir, x_pos as u32);
        assert!(
            matches!(info, Some(NodeInfo::Var { .. })),
            "cursor on let binding name should resolve to Var"
        );
    }

    #[test]
    fn cursor_on_assign_target_resolves() {
        let source = "fn main() { let mut x = 0\n  x = 1\n  x }";
        let thir = compile_to_thir(source).expect("should compile");
        // Cursor on `x` in `x = 1`
        let assign_x_pos = source.find("x = 1").unwrap(); // on `x`
        #[allow(clippy::cast_possible_truncation)]
        let info = find_node_at_offset(&thir, assign_x_pos as u32);
        assert!(
            matches!(info, Some(NodeInfo::Var { .. })),
            "cursor on assign target should resolve to Var"
        );
    }

    #[test]
    fn references_include_assignment_target() {
        let source = "fn main() { let mut x = 0\n  x = 1\n  x }";
        let thir = compile_to_thir(source).expect("should compile");
        // Find def_id for `x` by looking at the let binding position.
        let x_pos = source.find("let mut x").unwrap() + 8; // on `x` in `let mut x`
        #[allow(clippy::cast_possible_truncation)]
        let info = find_node_at_offset(&thir, x_pos as u32);
        let def_id = match info {
            Some(NodeInfo::Var { def_id, .. }) => def_id,
            other => panic!("expected Var, got {other:?}"),
        };
        let refs = find_all_references(&thir, def_id);
        // Should have: definition (let x), assignment target (x = 1), and usage (x).
        assert!(
            refs.len() >= 3,
            "expected at least 3 references (def + assign + use), got {}",
            refs.len()
        );
    }

    // ── Issue 103: code action tests ───────────────────────────

    #[test]
    fn code_action_make_mutable() {
        let message =
            "cannot assign to immutable binding `x`\nhint: make this binding mutable: `let mut x`";
        let source = "fn main() { let x = 0\n  x = 1\n  x }";
        let action = action_make_mutable(source, message).expect("should produce action");
        assert_eq!(action.kind, CodeActionKindTag::QuickFix);
        assert_eq!(action.new_text, "mut ");
        // Insert position should be after "let " (4 bytes from the start of "let x").
        let let_pos = source.find("let x").unwrap();
        assert_eq!(action.replace_start as usize, let_pos + 4);
    }

    #[test]
    fn code_action_add_type_annotation() {
        let source = "fn main() { let x = 1\n  x }";
        let thir = compile_to_thir(source).expect("should compile");
        let let_pos = source.find("let x").unwrap();
        #[allow(clippy::cast_possible_truncation)]
        let action = action_add_type_annotation(&thir, source, let_pos as u32)
            .expect("should produce action");
        assert_eq!(action.kind, CodeActionKindTag::Refactor);
        assert!(
            action.new_text.contains(": Int"),
            "annotation should include type: {}",
            action.new_text
        );
    }

    #[test]
    fn code_action_none_without_actionable_diagnostics() {
        let actions = collect_code_actions(None, "", &[], 0);
        assert!(actions.is_empty());
    }

    #[test]
    fn code_action_add_missing_match_arms() {
        let message = "non-exhaustive match on `Color`: missing Blue\nhint: add arms for: Blue";
        // Source with a match block.
        let source = "type Color { Red Green Blue }\nfn f(c: Color) -> Int { match c { Red -> 1\n    Green -> 2\n  } }";
        // diag_offset should point inside the match.
        let match_pos = source.find("match c").unwrap();
        #[allow(clippy::cast_possible_truncation)]
        let action = action_add_missing_match_arms(source, message, match_pos as u32)
            .expect("should produce action");
        assert_eq!(action.kind, CodeActionKindTag::QuickFix);
        assert!(
            action.new_text.contains("Blue"),
            "should contain missing arm: {}",
            action.new_text
        );
        let expected_insert = source[match_pos..]
            .find('}')
            .map(|rel| match_pos + rel)
            .expect("match block closing brace");
        assert_eq!(action.replace_start as usize, expected_insert);
    }

    // ── Issue 105: LSP smoke tests for mutable/async ─────────

    #[test]
    fn lsp_smoke_mutable_hover() {
        let source = "fn main() -> Int { let mut x = 0\n  x = 1\n  x }";
        let thir = compile_to_thir(source).expect("mutable should compile for LSP");
        let x_pos = source.find("x = 1").unwrap();
        #[allow(clippy::cast_possible_truncation)]
        let info = find_node_at_offset(&thir, x_pos as u32);
        assert!(info.is_some(), "hover on mutable assign target should resolve");
    }

    #[test]
    fn lsp_smoke_mutable_completion() {
        let source = "fn main() -> Int {\n  let mut x = 0\n  \n}";
        let thir = compile_to_thir(source).expect("mutable should compile");
        #[allow(clippy::cast_possible_truncation)]
        let offset = source.rfind("  \n").unwrap() as u32 + 2;
        let entries = collect_all_completions(Some(&thir), source, offset);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"x"), "mutable binding should appear in completions");
    }

    #[test]
    fn lsp_smoke_async_hover() {
        let source = "async fn fetch() -> Int { 1 }\npub async fn main() -> Int { await fetch() }";
        let thir = compile_to_thir(source).expect("async should compile for LSP");
        let fetch_pos = source.rfind("fetch()").unwrap();
        #[allow(clippy::cast_possible_truncation)]
        let info = find_node_at_offset(&thir, fetch_pos as u32);
        assert!(info.is_some(), "hover on async function call should resolve");
    }

    #[test]
    fn lsp_smoke_async_completion() {
        let source = "async fn fetch() -> Int { 1 }\nasync fn main() -> Int {\n  \n}";
        let thir = compile_to_thir(source).expect("async should compile");
        #[allow(clippy::cast_possible_truncation)]
        let offset = source.rfind("  \n").unwrap() as u32 + 2;
        let entries = collect_all_completions(Some(&thir), source, offset);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"fetch"), "async function should appear in completions");
        assert!(names.contains(&"await"), "await keyword should appear in block completions");
    }

    // ── Issue 106: LSP regression tests ──────────────────────

    #[test]
    fn regression_rename_mutable_binding() {
        let source = "fn main() -> Int { let mut x = 0\n  x = 1\n  x = x + 1\n  x }";
        let thir = compile_to_thir(source).expect("should compile");
        let x_pos = source.find("let mut x").unwrap() + 8;
        #[allow(clippy::cast_possible_truncation)]
        let info = find_node_at_offset(&thir, x_pos as u32);
        let def_id = match info {
            Some(NodeInfo::Var { def_id, .. }) => def_id,
            other => panic!("expected Var, got {other:?}"),
        };
        let refs = find_all_references(&thir, def_id);
        // def + 2 assignments + read in `x + 1` + final read = at least 4
        assert!(
            refs.len() >= 4,
            "expected at least 4 references for mutable binding rename, got {}",
            refs.len(),
        );
    }

    #[test]
    fn regression_hover_await_expression() {
        let source = "async fn fetch() -> Int { 1 }\nasync fn main() -> Int { await fetch() }";
        let thir = compile_to_thir(source).expect("should compile");
        let await_pos = source.rfind("await").unwrap();
        #[allow(clippy::cast_possible_truncation)]
        let info = find_node_at_offset(&thir, await_pos as u32);
        assert!(info.is_some(), "hover on await expression should resolve");
    }

    #[test]
    fn regression_rename_async_fn() {
        let source = "async fn fetch() -> Int { 1 }\nasync fn main() -> Int { await fetch() }";
        let thir = compile_to_thir(source).expect("should compile");
        let fn_pos = source.find("fn fetch").unwrap() + 3; // on 'fetch'
        #[allow(clippy::cast_possible_truncation)]
        let info = find_node_at_offset(&thir, fn_pos as u32);
        assert!(
            matches!(info, Some(NodeInfo::FnDef { .. })),
            "cursor on async fn name should be FnDef, got {info:?}",
        );
    }

    #[test]
    fn latency_collect_all_completions() {
        let source = "fn add(a: Int, b: Int) -> Int { a + b }\nfn main() -> Int {\n  \n}";
        let thir = compile_to_thir(source).expect("should compile");

        #[allow(clippy::cast_possible_truncation)]
        let offset = source.rfind("  \n").unwrap() as u32 + 2;
        let start = std::time::Instant::now();
        for _ in 0..100 {
            let _ = collect_all_completions(Some(&thir), source, offset);
        }
        let elapsed = start.elapsed();
        let per_call = elapsed / 100;
        eprintln!("  completion per call: {per_call:?}");
        assert!(
            per_call < std::time::Duration::from_millis(10),
            "completion should be sub-10ms, got {per_call:?}",
        );
    }

    #[test]
    fn code_action_add_await() {
        let actions = collect_code_actions(
            None,
            "async fn main() {\n  fetch()\n}",
            &[(
                "E0200".to_owned(),
                "type mismatch: expected `Int`, found `Task(Int)`\nhint: consider adding `await` to unwrap the Task value"
                    .to_owned(),
            )],
            20,
        );
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, CodeActionKindTag::QuickFix);
        assert_eq!(actions[0].title, "Add `await` to unwrap Task");
        assert_eq!(actions[0].new_text, "await ");
    }

    #[test]
    fn code_action_add_imports_offer_alias_variant() {
        let actions = action_add_imports("fn main() { foo }\n", "unresolved name `foo`");
        assert_eq!(actions.len(), 2);
        assert!(actions.iter().any(|a| a.new_text == "from python import foo\n"));
        assert!(actions.iter().any(|a| a.new_text == "from python import foo as py_foo\n"));
    }

    #[test]
    fn code_action_generate_python_imports_offer_alias_variant() {
        let actions = action_generate_python_imports("", "unknown Python module `pathlib`");
        assert_eq!(actions.len(), 2);
        assert!(actions.iter().any(|a| a.new_text == "from python import pathlib\n"));
        assert!(actions.iter().any(|a| a.new_text == "from python import pathlib as py_pathlib\n"));
    }

    // ── Completion snapshot tests ──────────────────────────────

    fn completion_snapshot(source: &str, offset: u32) -> String {
        let thir = compile_to_thir(source);
        let entries = collect_all_completions(thir.as_ref(), source, offset);
        let mut lines: Vec<String> = entries
            .iter()
            .map(|e| {
                let kind = match &e.kind {
                    CompletionEntryKind::Keyword => "keyword".to_owned(),
                    CompletionEntryKind::Symbol(dk) => format!("symbol({dk:?})"),
                    CompletionEntryKind::FfiFunction => "ffi_function".to_owned(),
                    CompletionEntryKind::FfiClass => "ffi_class".to_owned(),
                    CompletionEntryKind::FfiProperty => "ffi_property".to_owned(),
                    CompletionEntryKind::FfiMethod => "ffi_method".to_owned(),
                    CompletionEntryKind::FfiConstant => "ffi_constant".to_owned(),
                    CompletionEntryKind::FfiModule => "ffi_module".to_owned(),
                };
                let fmt = match e.insert_text_format {
                    InsertTextFormatTag::PlainText => "",
                    InsertTextFormatTag::Snippet => " [snippet]",
                };
                format!("{:<25} {kind}{fmt}", e.name)
            })
            .collect();
        lines.sort();
        lines.join("\n")
    }

    #[test]
    fn completion_snapshot_top_level() {
        let snapshot = completion_snapshot("", 0);
        insta::assert_snapshot!("completion_top_level", snapshot);
    }

    #[test]
    fn completion_snapshot_block() {
        let source = "fn add(a: Int, b: Int) -> Int { a + b }\nfn main() {\n  \n}";
        #[allow(clippy::cast_possible_truncation)]
        let offset = source.rfind("  \n").unwrap() as u32 + 2;
        let snapshot = completion_snapshot(source, offset);
        insta::assert_snapshot!("completion_block", snapshot);
    }

    #[test]
    fn completion_snapshot_async_block() {
        let source = "async fn fetch() -> Int { 1 }\nasync fn main() {\n  \n}";
        #[allow(clippy::cast_possible_truncation)]
        let offset = source.rfind("  \n").unwrap() as u32 + 2;
        let snapshot = completion_snapshot(source, offset);
        insta::assert_snapshot!("completion_async_block", snapshot);
    }

    // ── Issue 125: FFI import + module member completion ────────

    #[test]
    fn classify_context_from_python_import() {
        assert_eq!(
            classify_context("from python import ", 19, None),
            CompletionContext::ImportPythonModule,
        );
        // Partially typed module name is still ImportPythonModule context.
        assert_eq!(
            classify_context("from python import path", 23, None),
            CompletionContext::ImportPythonModule,
        );
    }

    #[test]
    fn ffi_import_module_completions_no_thir() {
        let entries = collect_ffi_import_module_completions(None);
        assert!(!entries.is_empty(), "should offer known module names even without THIR");
        assert!(entries.iter().any(|e| e.name == "pathlib"));
        assert!(entries.iter().any(|e| e.name == "json"));
        assert!(entries.iter().all(|e| e.kind == CompletionEntryKind::FfiModule));
    }

    #[test]
    fn ffi_import_module_completions_with_thir() {
        let source = "from python import pathlib\nfn main() { pathlib }";
        let thir = compile_to_thir(source);
        let entries = collect_ffi_import_module_completions(thir.as_ref());
        assert!(entries.iter().any(|e| e.name == "pathlib"));
        // Known modules not yet imported should also appear.
        assert!(entries.iter().any(|e| e.name == "os"));
    }

    #[test]
    fn ffi_module_member_completion_pathlib() {
        // Use a valid source that imports pathlib (no parse errors).
        let source = "from python import pathlib\nfn main() { pathlib }";
        let thir = compile_to_thir(source);
        let thir = thir.as_ref().expect("THIR should compile");
        let entries = collect_ffi_module_member_completions(thir, "pathlib");
        assert!(!entries.is_empty(), "pathlib module should have member completions");
        // Path is a class in pathlib.
        assert!(entries.iter().any(|e| e.name == "Path"), "pathlib should offer Path class");
    }

    #[test]
    fn ffi_module_member_completion_unknown_module() {
        let source = "fn main() { 1 }";
        let thir = compile_to_thir(source);
        let thir = thir.as_ref().expect("THIR should compile");
        let entries = collect_ffi_module_member_completions(thir, "nonexistent");
        assert!(entries.is_empty(), "unknown module should return empty");
    }

    // ── Issue 126: receiver-based instance member completion ────

    #[test]
    fn ffi_instance_member_completion_path() {
        // Valid source with Path instance — no trailing dot parse error.
        let source =
            "from python import pathlib\nfn main() {\n  let p = pathlib.Path(\"x\")\n  p\n}";
        let thir = compile_to_thir(source);
        let thir = thir.as_ref().expect("THIR should compile");
        let entries = collect_ffi_instance_member_completions(thir, "pathlib", "Path");
        assert!(!entries.is_empty(), "Path instance should have member completions");
        // Should have methods like exists.
        let has_method = entries.iter().any(|e| e.kind == CompletionEntryKind::FfiMethod);
        assert!(has_method, "should have at least one method");
    }

    #[test]
    fn ffi_instance_member_completion_unknown_class() {
        let source = "from python import pathlib\nfn main() { pathlib }";
        let thir = compile_to_thir(source);
        let thir = thir.as_ref().expect("THIR should compile");
        let entries = collect_ffi_instance_member_completions(thir, "pathlib", "NonExistentClass");
        assert!(entries.is_empty(), "unknown class should return empty");
    }

    #[test]
    fn ffi_instance_no_class_methods_in_instance_completion() {
        let source = "from python import pathlib\nfn main() { pathlib }";
        let thir = compile_to_thir(source);
        let thir = thir.as_ref().expect("THIR should compile");
        let entries = collect_ffi_instance_member_completions(thir, "pathlib", "Path");
        // Instance completion should not include class_methods or static_methods.
        // All entries should be FfiMethod or FfiProperty.
        for entry in &entries {
            assert!(
                entry.kind == CompletionEntryKind::FfiMethod
                    || entry.kind == CompletionEntryKind::FfiProperty,
                "instance completion should only have methods and properties, got {:?}",
                entry.kind
            );
        }
    }

    #[test]
    fn collect_all_completions_from_python_import() {
        let source = "from python import ";
        #[allow(clippy::cast_possible_truncation)]
        let offset = source.len() as u32;
        let entries = collect_all_completions(None, source, offset);
        assert!(entries.iter().any(|e| e.name == "pathlib"));
        assert!(entries.iter().any(|e| e.kind == CompletionEntryKind::FfiModule));
        // No keyword completions should be mixed in.
        assert!(entries.iter().all(|e| e.kind == CompletionEntryKind::FfiModule));
    }

    #[test]
    fn extract_trailing_identifier_basic() {
        assert_eq!(extract_trailing_identifier("  pathlib"), Some("pathlib"));
        assert_eq!(extract_trailing_identifier("x.pathlib"), Some("pathlib"));
        assert_eq!(extract_trailing_identifier(""), None);
        assert_eq!(extract_trailing_identifier("  "), None);
        assert_eq!(extract_trailing_identifier("123"), None);
    }
}
