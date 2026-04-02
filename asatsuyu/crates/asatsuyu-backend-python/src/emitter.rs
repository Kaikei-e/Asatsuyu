//! THIR → Python 3.12+ source code emitter.
//!
//! Walks the [`ThirModule`] tree and writes readable Python with type annotations.

use std::fmt::Write;

use asatsuyu_ast::{BinOp, UnOp};
use asatsuyu_hir::ffi::{FfiSymbolKind, FfiTrustLevel, FfiType};
use asatsuyu_hir::{DefId, DefKind, HirCustomType, HirFieldType, HirTypeExpr, HirVariant};
use asatsuyu_syntax::Span;
use asatsuyu_ty::{
    PrimTy, ThirExpr, ThirFnDef, ThirMatchArm, ThirModule, ThirPattern, Ty, TyVarId,
};
use smol_str::SmolStr;

/// 4-space indentation per PEP 8.
const INDENT: &str = "    ";

/// Precomputed line-start byte offsets for source-map comment generation.
pub(crate) struct LineOffsets {
    offsets: Vec<u32>,
}

impl LineOffsets {
    /// Build a line offset table from source text.
    pub(crate) fn from_source(source: &str) -> Self {
        let mut offsets = vec![0u32];
        for (i, ch) in source.char_indices() {
            if ch == '\n' {
                // Source files are capped well below u32::MAX.
                #[allow(clippy::cast_possible_truncation)]
                offsets.push((i + 1) as u32);
            }
        }
        Self { offsets }
    }

    /// Convert a byte offset to a 1-based line number.
    fn line_number(&self, offset: u32) -> usize {
        match self.offsets.binary_search(&offset) {
            Ok(i) => i + 1,
            Err(i) => i,
        }
    }
}

/// Emits Python source code from a typed HIR module.
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct Emitter<'a> {
    module: &'a ThirModule,
    output: String,
    indent: usize,
    /// Source-map line offsets. `Some` enables `# asty:L<n>` comments.
    line_offsets: Option<LineOffsets>,
    /// Counter for generating unique `try` temporary variable names.
    try_counter: usize,
    /// Whether any `try` expression was emitted (to decide prelude import).
    pub(crate) has_try: bool,
    /// Counter for generating unique Checked FFI temporary variable names.
    checked_counter: usize,
    /// Whether any Checked FFI wrapper was emitted (to decide prelude import).
    pub(crate) has_checked_ffi: bool,
    /// Whether `functools` is needed for expression-position `list.fold`.
    has_functools: bool,
    /// Whether any `Task(T)` type annotation appears (for `Coroutine` import).
    has_task_type: bool,
    /// Whether the currently emitted function returns `Result(_, _)`.
    current_fn_returns_result: bool,
}

/// Metadata for a Checked FFI call target.
struct CheckedFfiInfo {
    module_name: SmolStr,
    symbol_name: SmolStr,
    return_ty: Option<FfiType>,
    /// `true` for instance method calls (`call_method`),
    /// `false` for module-level function calls (`call_function`).
    is_method: bool,
}

impl<'a> Emitter<'a> {
    pub(crate) fn new(module: &'a ThirModule) -> Self {
        Self {
            module,
            output: String::new(),
            indent: 0,
            line_offsets: None,
            try_counter: 0,
            has_try: false,
            checked_counter: 0,
            has_checked_ffi: false,
            has_functools: false,
            has_task_type: false,
            current_fn_returns_result: false,
        }
    }

    /// Create an emitter with source-map comment generation enabled.
    pub(crate) fn with_source_map(module: &'a ThirModule, source: &str) -> Self {
        Self {
            module,
            output: String::new(),
            indent: 0,
            line_offsets: Some(LineOffsets::from_source(source)),
            try_counter: 0,
            has_try: false,
            checked_counter: 0,
            has_checked_ffi: false,
            has_functools: false,
            has_task_type: false,
            current_fn_returns_result: false,
        }
    }

    pub(crate) fn emit(&mut self) {
        // Pre-scan for try expressions to decide prelude imports.
        self.has_try = self.module.functions.iter().any(|f| expr_contains_try(&f.body));
        // Pre-scan for Checked FFI calls.
        self.has_checked_ffi = self.scan_for_checked_ffi();
        self.has_functools =
            self.module.functions.iter().any(|f| self.expr_contains_list_fold(&f.body));
        // Pre-scan for Task(T) types in parameter or return-type positions.
        // Async fn return types are stored as the inner type T (not Task(T)),
        // so they won't match. But sync fns forwarding Task values need the import.
        self.has_task_type = self.module.functions.iter().any(|f| {
            f.params.iter().any(|p| ty_contains_task(&p.ty)) || ty_contains_task(&f.return_ty)
        });
        if self.has_checked_ffi {
            self.has_try = true; // Checked FFI wrappers use PyException
        }
        self.emit_module();
    }

    pub(crate) fn into_output(self) -> String {
        self.output
    }

    // ── Source-map helpers ─────────────────────────────────────────

    /// Append a `# asty:L<n>` comment before the trailing newline on the
    /// current output line. No-op when source-map is disabled.
    fn write_source_comment(&mut self, span: Span) {
        if let Some(ref lo) = self.line_offsets {
            let line = lo.line_number(span.start);
            let _ = write!(self.output, "  # asty:L{line}");
        }
    }

    // ── Indent helpers ─────────────────────────────────────────────

    fn write_indent(&mut self) {
        for _ in 0..self.indent {
            self.output.push_str(INDENT);
        }
    }

    fn push_indent(&mut self) {
        self.indent += 1;
    }

    fn pop_indent(&mut self) {
        self.indent -= 1;
    }

    // ── Module ─────────────────────────────────────────────────────

    fn emit_module(&mut self) {
        let has_custom_types = !self.module.custom_types.is_empty();
        let has_functions = !self.module.functions.is_empty();
        let has_imports = !self.module.imports.is_empty();

        if !has_custom_types && !has_functions && !has_imports {
            return;
        }

        // 1. Header comment and postponed annotation evaluation.
        self.output.push_str("# Generated by Asatsuyu \u{2014} do not edit\n");
        self.output.push_str("from __future__ import annotations\n");

        // 2. Imports.
        for import in &self.module.imports {
            let bound_name = &self.module.symbol_table.get(import.def_id).name;
            match &import.kind {
                asatsuyu_hir::HirImportKind::Module { module_path } => {
                    let module_name = module_path
                        .iter()
                        .map(smol_str::SmolStr::as_str)
                        .collect::<Vec<_>>()
                        .join(".");
                    let needs_alias = module_path.last().is_none_or(|last| last != bound_name);
                    if needs_alias {
                        let _ = writeln!(self.output, "import {module_name} as {bound_name}");
                    } else {
                        let _ = writeln!(self.output, "import {module_name}");
                    }
                }
                asatsuyu_hir::HirImportKind::Python { module_name } => {
                    if module_name.as_str() == bound_name.as_str() {
                        let _ = writeln!(self.output, "import {module_name}");
                    } else {
                        let _ = writeln!(self.output, "import {module_name} as {bound_name}");
                    }
                }
            }
        }
        if has_custom_types {
            self.output.push_str("from dataclasses import dataclass\n");
        }
        if self.has_functools {
            self.output.push_str("import functools\n");
        }
        if self.has_task_type {
            self.output.push_str("from collections.abc import Coroutine\nfrom typing import Any\n");
        }
        if self.has_try || self.has_checked_ffi {
            if self.has_checked_ffi {
                self.output.push_str(
                    "from .asatsuyu_prelude import PyException, AsatsuyuError\nfrom . import _asatsuyu_runtime\n",
                );
            } else {
                self.output.push_str("from .asatsuyu_prelude import PyException\n");
            }
        }
        // hasattr guards for Checked FFI symbols (after prelude import).
        if self.has_checked_ffi {
            self.output.push_str("if not _asatsuyu_runtime.ffi_available():\n");
            self.output.push_str(
                "    raise AsatsuyuError(\"_asatsuyu_runtime is not available for Checked FFI\")\n",
            );
            for import in &self.module.imports {
                if let asatsuyu_hir::HirImportKind::Python { module_name } = &import.kind {
                    let runtime_binding = checked_runtime_binding(module_name);
                    let _ = writeln!(
                        self.output,
                        "{runtime_binding} = _asatsuyu_runtime.import_module(\"{module_name}\")"
                    );
                    self.emit_hasattr_guards(module_name);
                }
            }
        }

        // 3. ADT definitions.
        for ct in &self.module.custom_types {
            self.output.push_str("\n\n");
            self.emit_custom_type(ct);
        }

        // 4. Function definitions.
        for fn_def in &self.module.functions {
            self.output.push_str("\n\n");
            self.emit_fn_def(fn_def);
        }
    }

    // ── Custom type (ADT → dataclass) ─────────────────────────────

    fn emit_custom_type(&mut self, ct: &HirCustomType) {
        let type_name = self.module.symbol_table.get(ct.def_id).name.clone();
        let param_map = build_type_param_map(&ct.type_params);

        // Emit each variant as a @dataclass.
        for (i, variant) in ct.variants.iter().enumerate() {
            if i > 0 {
                self.output.push_str("\n\n");
            }
            self.emit_variant(variant, &param_map);
        }

        // Emit PEP 695 type alias for multi-variant ADTs.
        if ct.variants.len() > 1 {
            self.output.push_str("\n\n");
            self.emit_type_alias(ct, &type_name, &param_map);
        }
    }

    fn emit_variant(&mut self, variant: &HirVariant, param_map: &[(SmolStr, String)]) {
        let name = &self.module.symbol_table.get(variant.def_id).name;
        let py_name = sanitize_python_name(name);

        self.output.push_str("@dataclass(frozen=True, slots=True)\n");
        let _ = write!(self.output, "class {py_name}");

        // PEP 695 type parameters.
        let used_params = collect_used_params(&variant.fields, param_map);
        if !used_params.is_empty() {
            let _ = write!(self.output, "[{}]", used_params.join(", "));
        }

        self.output.push_str(":\n");

        if variant.fields.is_empty() {
            self.output.push_str("    pass\n");
        } else {
            for (i, field) in variant.fields.iter().enumerate() {
                let field_name = field
                    .label
                    .as_ref()
                    .map_or_else(|| format!("_{i}"), |l| sanitize_python_name(l));
                let py_type = hir_type_expr_to_python(&field.type_expr, param_map);
                let _ = writeln!(self.output, "    {field_name}: {py_type}");
            }
        }
    }

    fn emit_type_alias(
        &mut self,
        ct: &HirCustomType,
        type_name: &str,
        param_map: &[(SmolStr, String)],
    ) {
        let _ = write!(self.output, "type {type_name}");
        if !param_map.is_empty() {
            let params: Vec<&str> = param_map.iter().map(|(_, p)| p.as_str()).collect();
            let _ = write!(self.output, "[{}]", params.join(", "));
        }
        self.output.push_str(" = ");

        for (i, variant) in ct.variants.iter().enumerate() {
            if i > 0 {
                self.output.push_str(" | ");
            }
            let name = &self.module.symbol_table.get(variant.def_id).name;
            let py_name = sanitize_python_name(name);
            self.output.push_str(&py_name);

            let used = collect_used_params(&variant.fields, param_map);
            if !used.is_empty() {
                let _ = write!(self.output, "[{}]", used.join(", "));
            }
        }
        self.output.push('\n');
    }

    // ── Function ───────────────────────────────────────────────────

    fn emit_fn_def(&mut self, fn_def: &ThirFnDef) {
        // Collect type variables from the signature for PEP 695 generics.
        let mut var_ids = Vec::new();
        for param in &fn_def.params {
            collect_type_vars(&param.ty, &mut var_ids);
        }
        collect_type_vars(&fn_def.return_ty, &mut var_ids);
        let var_map = build_fn_type_param_map(&var_ids);

        // async def name[T, U](params) -> return_ty:
        self.write_indent();
        if fn_def.is_async {
            self.output.push_str("async ");
        }
        let name = &self.module.symbol_table.get(fn_def.def_id).name;
        let _ = write!(self.output, "def {name}");

        if !var_map.is_empty() {
            let params: Vec<&str> = var_map.iter().map(|(_, n)| n.as_str()).collect();
            let _ = write!(self.output, "[{}]", params.join(", "));
        }

        self.output.push('(');
        for (i, param) in fn_def.params.iter().enumerate() {
            if i > 0 {
                self.output.push_str(", ");
            }
            let param_name = &self.module.symbol_table.get(param.def_id).name;
            let _ = write!(self.output, "{param_name}: {}", ty_to_python(&param.ty, &var_map));
        }

        let _ = write!(self.output, ") -> {}:", ty_to_python(&fn_def.return_ty, &var_map));
        self.write_source_comment(fn_def.span);
        self.output.push('\n');

        // Body.
        let previous_returns_result = self.current_fn_returns_result;
        self.current_fn_returns_result = is_result_ty(&fn_def.return_ty);
        self.push_indent();
        self.emit_body(&fn_def.body);
        self.pop_indent();
        self.current_fn_returns_result = previous_returns_result;
    }

    // ── Body (block handling) ──────────────────────────────────────

    fn emit_body(&mut self, body: &ThirExpr) {
        match body {
            ThirExpr::Block { exprs, .. } if exprs.is_empty() => {
                self.write_indent();
                self.output.push_str("pass\n");
            }
            ThirExpr::Block { exprs, .. } => {
                let (statements, last) = exprs.split_at(exprs.len() - 1);
                for stmt in statements {
                    self.emit_stmt(stmt);
                }
                self.emit_return_stmt(&last[0]);
            }
            // Non-block body (single expression).
            other => {
                self.emit_return_stmt(other);
            }
        }
    }

    // ── Statements ─────────────────────────────────────────────────

    fn emit_stmt(&mut self, expr: &ThirExpr) {
        if let ThirExpr::Match { subject, arms, .. } = expr {
            self.emit_match_stmt(subject, arms, false);
            return;
        }
        if let ThirExpr::Let { binding, value, .. } = expr
            && let ThirExpr::Call { func, args, .. } = value.as_ref()
            && let Some(method) = self.list_module_method(func)
            && method == "fold"
            && args.len() == 3
        {
            self.emit_list_fold_let_stmt(*binding, args, expr.span());
            return;
        }
        // `x = list.fold(...)` (reassignment to mutable local)
        if let ThirExpr::Assign { target, value, .. } = expr
            && let ThirExpr::Call { func, args, .. } = value.as_ref()
            && let Some(method) = self.list_module_method(func)
            && method == "fold"
            && args.len() == 3
        {
            self.emit_list_fold_let_stmt(*target, args, expr.span());
            return;
        }
        // `let x = try expr` → try/except block
        if let ThirExpr::Let { binding, value, .. } = expr
            && let ThirExpr::Try { expr: inner, .. } = value.as_ref()
        {
            self.emit_try_let_stmt(*binding, inner, expr.span());
            return;
        }
        // `let x = <checked_ffi_call>` → try/except + validator
        if let ThirExpr::Let { binding, value, .. } = expr
            && let ThirExpr::Call { func, args, .. } = value.as_ref()
            && let Some(info) = self.checked_ffi_target(func)
        {
            self.emit_checked_ffi_let_stmt(*binding, func, args, &info, expr.span());
            return;
        }
        // `x = <checked_ffi_call>` (reassignment to mutable local)
        if let ThirExpr::Assign { target, value, .. } = expr
            && let ThirExpr::Call { func, args, .. } = value.as_ref()
            && let Some(info) = self.checked_ffi_target(func)
        {
            self.emit_checked_ffi_let_stmt(*target, func, args, &info, expr.span());
            return;
        }
        // bare `try expr` as a statement
        if let ThirExpr::Try { expr: inner, .. } = expr {
            self.emit_try_bare_stmt(inner, expr.span());
            return;
        }
        // bare `<checked_ffi_call>` as a statement
        if let ThirExpr::Call { func, args, .. } = expr
            && let Some(info) = self.checked_ffi_target(func)
        {
            self.emit_checked_ffi_bare_stmt(func, args, &info, expr.span());
            return;
        }
        self.write_indent();
        self.emit_expr(expr);
        if matches!(expr, ThirExpr::Let { .. } | ThirExpr::Assign { .. }) {
            self.write_source_comment(expr.span());
        }
        self.output.push('\n');
    }

    fn emit_return_stmt(&mut self, expr: &ThirExpr) {
        if let ThirExpr::Match { subject, arms, .. } = expr {
            self.emit_match_stmt(subject, arms, true);
            return;
        }
        if matches!(expr, ThirExpr::Let { .. } | ThirExpr::Assign { .. }) {
            self.emit_stmt(expr);
            self.write_indent();
            self.output.push_str("return None\n");
            return;
        }
        if let ThirExpr::Call { func, args, .. } = expr
            && let Some(method) = self.list_module_method(func)
            && method == "fold"
            && args.len() == 3
        {
            self.emit_list_fold_return_stmt(args, expr.span());
            return;
        }
        // `try expr` in return position → try/except, then return Ok(value)
        if let ThirExpr::Try { expr: inner, .. } = expr {
            self.emit_try_return_stmt(inner, expr.span());
            return;
        }
        // Checked FFI call in return position → try/except + validator + return
        if let ThirExpr::Call { func, args, .. } = expr
            && let Some(info) = self.checked_ffi_target(func)
        {
            self.emit_checked_ffi_return_stmt(func, args, &info, expr.span());
            return;
        }
        self.write_indent();
        self.output.push_str("return ");
        self.emit_expr(expr);
        self.write_source_comment(expr.span());
        self.output.push('\n');
    }

    // ── Expressions (inline, no newline) ───────────────────────────

    #[allow(clippy::too_many_lines)]
    fn emit_expr(&mut self, expr: &ThirExpr) {
        match expr {
            ThirExpr::Literal(lit) => self.output.push_str(lit.value.as_str()),
            ThirExpr::Var { def_id, .. } => {
                let def = self.module.symbol_table.get(*def_id);
                if def.kind == DefKind::Constructor {
                    self.output.push_str(&sanitize_python_name(&def.name));
                } else {
                    self.output.push_str(def.name.as_str());
                }
            }
            // Nested block in expression position: emit the last expression.
            ThirExpr::Block { exprs, .. } => {
                if let Some(last) = exprs.last() {
                    self.emit_expr(last);
                }
            }
            ThirExpr::Call { func, args, .. } => {
                if self.emit_builtin_call(func, args) {
                    return;
                }
                self.emit_expr(func);
                self.output.push('(');
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        self.output.push_str(", ");
                    }
                    self.emit_expr(arg);
                }
                self.output.push(')');
            }
            ThirExpr::BinaryOp { op, lhs, rhs, .. } => {
                self.output.push('(');
                self.emit_expr(lhs);
                self.output.push_str(binop_to_python(*op));
                self.emit_expr(rhs);
                self.output.push(')');
            }
            ThirExpr::UnaryOp { op, expr, .. } => {
                self.output.push('(');
                self.output.push_str(unop_to_python(*op));
                self.emit_expr(expr);
                self.output.push(')');
            }
            ThirExpr::If { condition, then_body, else_body, .. } => {
                // Python ternary: (then if cond else else_)
                self.output.push('(');
                self.emit_expr(then_body);
                self.output.push_str(" if ");
                self.emit_expr(condition);
                self.output.push_str(" else ");
                if let Some(else_expr) = else_body {
                    self.emit_expr(else_expr);
                } else {
                    self.output.push_str("None");
                }
                self.output.push(')');
            }
            // Match in inline expression position — full match/case in emit_stmt.
            ThirExpr::Match { arms, .. } => {
                if let Some(first) = arms.first() {
                    self.emit_expr(&first.body);
                }
            }
            ThirExpr::Let { binding: def_id, value, .. }
            | ThirExpr::Assign { target: def_id, value, .. } => {
                let name = &self.module.symbol_table.get(*def_id).name;
                self.output.push_str(name.as_str());
                self.output.push_str(" = ");
                self.emit_expr(value);
            }
            ThirExpr::Await { expr, .. } => {
                self.output.push_str("await ");
                self.emit_expr(expr);
            }
            ThirExpr::Lambda { params, body, .. } => {
                self.output.push_str("lambda ");
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        self.output.push_str(", ");
                    }
                    let name = &self.module.symbol_table.get(p.def_id).name;
                    self.output.push_str(name.as_str());
                }
                self.output.push_str(": ");
                self.emit_expr(body);
            }
            ThirExpr::FieldAccess { receiver, field, .. } => {
                self.emit_expr(receiver);
                self.output.push('.');
                self.output.push_str(field.as_str());
            }
            ThirExpr::Try { expr, .. } => {
                // Fallback for try in pure expression position.
                // Statement-level try is handled in emit_stmt/emit_try_stmt.
                self.emit_expr(expr);
            }
            ThirExpr::List { elements, .. } => {
                self.output.push('[');
                for (i, element) in elements.iter().enumerate() {
                    if i > 0 {
                        self.output.push_str(", ");
                    }
                    self.emit_expr(element);
                }
                self.output.push(']');
            }
        }
    }

    fn emit_builtin_call(&mut self, func: &ThirExpr, args: &[ThirExpr]) -> bool {
        // Check for list.* module calls (field access on "list" identifier).
        if let Some(method) = self.list_module_method(func) {
            return self.emit_list_call(&method, args);
        }

        let ThirExpr::Var { def_id, .. } = func else {
            return false;
        };
        let name = self.module.symbol_table.get(*def_id).name.as_str();
        if name == "string_concat" && args.len() == 2 {
            self.output.push('(');
            self.emit_expr(&args[0]);
            self.output.push_str(" + ");
            self.emit_expr(&args[1]);
            self.output.push(')');
            true
        } else if name == "println" && args.len() == 1 {
            self.output.push_str("print(");
            self.emit_expr(&args[0]);
            self.output.push(')');
            true
        } else {
            false
        }
    }

    /// If `func` is `list.<method>`, return the method name.
    fn list_module_method(&self, func: &ThirExpr) -> Option<SmolStr> {
        let ThirExpr::FieldAccess { receiver, field, .. } = func else {
            return None;
        };
        let ThirExpr::Var { def_id, .. } = receiver.as_ref() else {
            return None;
        };
        let name = self.module.symbol_table.get(*def_id).name.as_str();
        if name == "list" { Some(field.clone()) } else { None }
    }

    /// Emit a `list.*` call as idiomatic Python.
    fn emit_list_call(&mut self, method: &str, args: &[ThirExpr]) -> bool {
        match method {
            // list.map(items, fn(x) { expr }) → [expr for x in items]
            "map" if args.len() == 2 => self.emit_list_map_call(args),
            // list.filter(items, fn(x) { cond }) → [x for x in items if cond]
            "filter" if args.len() == 2 => self.emit_list_filter_call(args),
            // list.length(items) → len(items)
            "length" if args.len() == 1 => {
                self.output.push_str("len(");
                self.emit_expr(&args[0]);
                self.output.push(')');
                true
            }
            // list.reverse(items) → list(reversed(items))
            "reverse" if args.len() == 1 => {
                self.output.push_str("list(reversed(");
                self.emit_expr(&args[0]);
                self.output.push_str("))");
                true
            }
            // list.append(a, b) → a + b
            "append" if args.len() == 2 => {
                self.output.push('(');
                self.emit_expr(&args[0]);
                self.output.push_str(" + ");
                self.emit_expr(&args[1]);
                self.output.push(')');
                true
            }
            // list.is_empty(items) → (len(items) == 0)
            "is_empty" if args.len() == 1 => {
                self.output.push_str("(len(");
                self.emit_expr(&args[0]);
                self.output.push_str(") == 0)");
                true
            }
            // list.contains(items, x) → (x in items)
            "contains" if args.len() == 2 => {
                self.output.push('(');
                self.emit_expr(&args[1]);
                self.output.push_str(" in ");
                self.emit_expr(&args[0]);
                self.output.push(')');
                true
            }
            // list.fold(items, init, f) → functools.reduce-style loop
            "fold" if args.len() == 3 => self.emit_list_fold_expr(args),
            // list.head(items) → (items[0] if items else None)
            "head" if args.len() == 1 => self.emit_list_head_expr(args),
            // list.rest(items) → (items[1:] if items else None)
            "rest" if args.len() == 1 => self.emit_list_rest_expr(args),
            _ => false,
        }
    }

    fn emit_list_map_call(&mut self, args: &[ThirExpr]) -> bool {
        if let Some((param, body)) = self.extract_lambda(&args[1]) {
            self.output.push('[');
            self.emit_expr(body);
            self.output.push_str(" for ");
            self.output.push_str(&param);
            self.output.push_str(" in ");
            self.emit_expr(&args[0]);
            self.output.push(']');
        } else {
            self.output.push('[');
            self.emit_expr(&args[1]);
            self.output.push_str("(_x) for _x in ");
            self.emit_expr(&args[0]);
            self.output.push(']');
        }
        true
    }

    fn emit_list_filter_call(&mut self, args: &[ThirExpr]) -> bool {
        if let Some((param, body)) = self.extract_lambda(&args[1]) {
            self.output.push('[');
            self.output.push_str(&param);
            self.output.push_str(" for ");
            self.output.push_str(&param);
            self.output.push_str(" in ");
            self.emit_expr(&args[0]);
            self.output.push_str(" if ");
            self.emit_expr(body);
            self.output.push(']');
        } else {
            self.output.push_str("[_x for _x in ");
            self.emit_expr(&args[0]);
            self.output.push_str(" if ");
            self.emit_expr(&args[1]);
            self.output.push_str("(_x)]");
        }
        true
    }

    fn emit_list_fold_expr(&mut self, args: &[ThirExpr]) -> bool {
        self.output.push_str("functools.reduce(");
        self.emit_expr(&args[2]);
        self.output.push_str(", ");
        self.emit_expr(&args[0]);
        self.output.push_str(", ");
        self.emit_expr(&args[1]);
        self.output.push(')');
        true
    }

    fn emit_list_head_expr(&mut self, args: &[ThirExpr]) -> bool {
        self.output.push('(');
        self.emit_expr(&args[0]);
        self.output.push_str("[0] if ");
        self.emit_expr(&args[0]);
        self.output.push_str(" else None)");
        true
    }

    fn emit_list_rest_expr(&mut self, args: &[ThirExpr]) -> bool {
        self.output.push('(');
        self.emit_expr(&args[0]);
        self.output.push_str("[1:] if ");
        self.emit_expr(&args[0]);
        self.output.push_str(" else None)");
        true
    }

    fn emit_list_fold_let_stmt(&mut self, binding: DefId, args: &[ThirExpr], span: Span) {
        let name = self.module.symbol_table.get(binding).name.clone();
        let item_tmp = format!("_fold_item_{}", self.try_counter);
        self.try_counter += 1;

        self.write_indent();
        let _ = write!(self.output, "{name} = ");
        self.emit_expr(&args[1]);
        self.write_source_comment(span);
        self.output.push('\n');
        self.write_indent();
        let _ = write!(self.output, "for {item_tmp} in ");
        self.emit_expr(&args[0]);
        self.output.push_str(":\n");
        self.push_indent();
        self.write_indent();
        let _ = write!(self.output, "{name} = ");
        self.emit_expr(&args[2]);
        self.output.push('(');
        self.output.push_str(name.as_str());
        self.output.push_str(", ");
        self.output.push_str(&item_tmp);
        self.output.push_str(")\n");
        self.pop_indent();
    }

    fn emit_list_fold_return_stmt(&mut self, args: &[ThirExpr], span: Span) {
        let acc_tmp = format!("_fold_acc_{}", self.try_counter);
        let item_tmp = format!("_fold_item_{}", self.try_counter);
        self.try_counter += 1;

        self.write_indent();
        let _ = write!(self.output, "{acc_tmp} = ");
        self.emit_expr(&args[1]);
        self.write_source_comment(span);
        self.output.push('\n');
        self.write_indent();
        let _ = write!(self.output, "for {item_tmp} in ");
        self.emit_expr(&args[0]);
        self.output.push_str(":\n");
        self.push_indent();
        self.write_indent();
        let _ = write!(self.output, "{acc_tmp} = ");
        self.emit_expr(&args[2]);
        self.output.push('(');
        self.output.push_str(&acc_tmp);
        self.output.push_str(", ");
        self.output.push_str(&item_tmp);
        self.output.push_str(")\n");
        self.pop_indent();
        self.write_indent();
        let _ = writeln!(self.output, "return {acc_tmp}");
    }

    /// Extract parameter name and body from a single-parameter lambda expression.
    fn extract_lambda<'b>(&self, expr: &'b ThirExpr) -> Option<(String, &'b ThirExpr)> {
        let ThirExpr::Lambda { params, body, .. } = expr else {
            return None;
        };
        if params.len() != 1 {
            return None;
        }
        let param_name = self.module.symbol_table.get(params[0].def_id).name.to_string();
        Some((param_name, body.as_ref()))
    }

    // ── Match statement emission ──────────────────────────────────

    // ── Try/except generation ───────────────────────────────────────

    /// Emit `let x = try expr` as:
    /// ```python
    /// try:
    ///     x = <expr>
    /// except Exception as _e:
    ///     return Error(PyException.from_exception(_e))
    /// ```
    fn emit_try_let_stmt(&mut self, binding: DefId, inner: &ThirExpr, span: Span) {
        self.has_try = true;
        let name = self.module.symbol_table.get(binding).name.clone();
        self.write_indent();
        self.output.push_str("try:");
        self.write_source_comment(span);
        self.output.push('\n');
        self.push_indent();
        self.write_indent();
        let _ = write!(self.output, "{name} = ");
        self.emit_expr(inner);
        self.output.push('\n');
        self.pop_indent();
        self.emit_except_block();
    }

    /// Emit bare `try expr` as a statement:
    /// ```python
    /// try:
    ///     <expr>
    /// except Exception as _e:
    ///     return Error(PyException.from_exception(_e))
    /// ```
    fn emit_try_bare_stmt(&mut self, inner: &ThirExpr, span: Span) {
        self.has_try = true;
        self.write_indent();
        self.output.push_str("try:");
        self.write_source_comment(span);
        self.output.push('\n');
        self.push_indent();
        self.write_indent();
        self.emit_expr(inner);
        self.output.push('\n');
        self.pop_indent();
        self.emit_except_block();
    }

    /// Emit `try expr` in return position:
    /// ```python
    /// try:
    ///     _try_N = <expr>
    /// except Exception as _e:
    ///     return Error(PyException.from_exception(_e))
    /// return Ok(_try_N)
    /// ```
    fn emit_try_return_stmt(&mut self, inner: &ThirExpr, span: Span) {
        self.has_try = true;
        let tmp = format!("_try_{}", self.try_counter);
        self.try_counter += 1;
        self.write_indent();
        self.output.push_str("try:");
        self.write_source_comment(span);
        self.output.push('\n');
        self.push_indent();
        self.write_indent();
        let _ = write!(self.output, "{tmp} = ");
        self.emit_expr(inner);
        self.output.push('\n');
        self.pop_indent();
        self.emit_except_block();
        self.write_indent();
        let _ = writeln!(self.output, "return Ok({tmp})");
    }

    /// Emit the common `except Exception as _e: return Error(...)` block.
    fn emit_except_block(&mut self) {
        self.write_indent();
        self.output.push_str("except Exception as _e:\n");
        self.push_indent();
        self.write_indent();
        self.output.push_str("return Error(PyException.from_exception(_e))\n");
        self.pop_indent();
    }

    // ── Checked FFI wrapper generation ─────────────────────────────

    /// Determine whether a call targets a Checked FFI symbol.
    fn checked_ffi_target(&self, func: &ThirExpr) -> Option<CheckedFfiInfo> {
        let ThirExpr::FieldAccess { receiver, field, .. } = func else {
            return None;
        };

        match receiver.ty() {
            // Case 1: Module-level function — e.g., `requests.get(url)`
            Ty::FfiModule { module_name } => {
                let ffi_mod = self.module.ffi_modules.get(module_name)?;
                let sym = ffi_mod.symbols.iter().find(|s| s.name == *field)?;
                if sym.trust_level != Some(FfiTrustLevel::Checked) {
                    return None;
                }
                let return_ty = match &sym.kind {
                    FfiSymbolKind::Function(sig) => Some(sig.return_ty.clone()),
                    _ => None,
                };
                Some(CheckedFfiInfo {
                    module_name: module_name.clone(),
                    symbol_name: field.clone(),
                    return_ty,
                    is_method: false,
                })
            }

            // Case 2: Instance method — e.g., `response.json()`
            Ty::FfiInstance { module, class } => {
                let ffi_mod = self.module.ffi_modules.get(module)?;
                let cls = ffi_mod.symbols.iter().find_map(|s| {
                    if s.name == *class {
                        if let FfiSymbolKind::Class(c) = &s.kind { Some(c) } else { None }
                    } else {
                        None
                    }
                })?;
                let (_, method_sig) = cls.methods.iter().find(|(n, _)| n == field)?;
                // Only wrap methods whose return type contains Any.
                if !method_sig.return_ty.contains_any() {
                    return None;
                }
                Some(CheckedFfiInfo {
                    module_name: module.clone(),
                    symbol_name: field.clone(),
                    return_ty: Some(method_sig.return_ty.clone()),
                    is_method: true,
                })
            }

            _ => None,
        }
    }

    /// Pre-scan: does the module contain any Checked FFI calls?
    fn scan_for_checked_ffi(&self) -> bool {
        self.module.functions.iter().any(|f| self.expr_contains_checked_ffi(&f.body))
    }

    fn expr_contains_checked_ffi(&self, expr: &ThirExpr) -> bool {
        match expr {
            ThirExpr::Call { func, args, .. } => {
                if self.checked_ffi_target(func).is_some() {
                    return true;
                }
                self.expr_contains_checked_ffi(func)
                    || args.iter().any(|a| self.expr_contains_checked_ffi(a))
            }
            ThirExpr::Block { exprs, .. } => {
                exprs.iter().any(|e| self.expr_contains_checked_ffi(e))
            }
            ThirExpr::Let { value, .. } | ThirExpr::Assign { value, .. } => {
                self.expr_contains_checked_ffi(value)
            }
            ThirExpr::If { condition, then_body, else_body, .. } => {
                self.expr_contains_checked_ffi(condition)
                    || self.expr_contains_checked_ffi(then_body)
                    || else_body.as_ref().is_some_and(|e| self.expr_contains_checked_ffi(e))
            }
            ThirExpr::Match { subject, arms, .. } => {
                self.expr_contains_checked_ffi(subject)
                    || arms.iter().any(|a| self.expr_contains_checked_ffi(&a.body))
            }
            ThirExpr::BinaryOp { lhs, rhs, .. } => {
                self.expr_contains_checked_ffi(lhs) || self.expr_contains_checked_ffi(rhs)
            }
            ThirExpr::UnaryOp { expr, .. }
            | ThirExpr::Lambda { body: expr, .. }
            | ThirExpr::Try { expr, .. }
            | ThirExpr::Await { expr, .. } => self.expr_contains_checked_ffi(expr),
            ThirExpr::FieldAccess { receiver, .. } => self.expr_contains_checked_ffi(receiver),
            ThirExpr::List { elements, .. } => {
                elements.iter().any(|e| self.expr_contains_checked_ffi(e))
            }
            ThirExpr::Literal(_) | ThirExpr::Var { .. } => false,
        }
    }

    fn expr_contains_list_fold(&self, expr: &ThirExpr) -> bool {
        match expr {
            ThirExpr::Call { func, args, .. } => {
                self.list_module_method(func).is_some_and(|m| m == "fold")
                    || self.expr_contains_list_fold(func)
                    || args.iter().any(|a| self.expr_contains_list_fold(a))
            }
            ThirExpr::Block { exprs, .. } => exprs.iter().any(|e| self.expr_contains_list_fold(e)),
            ThirExpr::Let { value, .. } | ThirExpr::Assign { value, .. } => {
                self.expr_contains_list_fold(value)
            }
            ThirExpr::If { condition, then_body, else_body, .. } => {
                self.expr_contains_list_fold(condition)
                    || self.expr_contains_list_fold(then_body)
                    || else_body.as_ref().is_some_and(|e| self.expr_contains_list_fold(e))
            }
            ThirExpr::Match { subject, arms, .. } => {
                self.expr_contains_list_fold(subject)
                    || arms.iter().any(|a| self.expr_contains_list_fold(&a.body))
            }
            ThirExpr::BinaryOp { lhs, rhs, .. } => {
                self.expr_contains_list_fold(lhs) || self.expr_contains_list_fold(rhs)
            }
            ThirExpr::UnaryOp { expr, .. }
            | ThirExpr::Lambda { body: expr, .. }
            | ThirExpr::Try { expr, .. }
            | ThirExpr::Await { expr, .. } => self.expr_contains_list_fold(expr),
            ThirExpr::FieldAccess { receiver, .. } => self.expr_contains_list_fold(receiver),
            ThirExpr::List { elements, .. } => {
                elements.iter().any(|e| self.expr_contains_list_fold(e))
            }
            ThirExpr::Literal(_) | ThirExpr::Var { .. } => false,
        }
    }

    /// Emit the runtime call expression: `_checked_N = _asatsuyu_runtime.call_*(...)`.
    ///
    /// For module-level functions (`is_method == false`):
    ///   `_asatsuyu_runtime.call_function(_checked_runtime_mod, "func", args...)`
    /// For instance methods (`is_method == true`):
    ///   `_asatsuyu_runtime.call_method(receiver, "method", args...)`
    fn emit_checked_ffi_call_expr(
        &mut self,
        tmp: &str,
        func: &ThirExpr,
        args: &[ThirExpr],
        info: &CheckedFfiInfo,
    ) {
        self.write_indent();
        if info.is_method {
            let ThirExpr::FieldAccess { receiver, .. } = func else {
                unreachable!("Checked FFI method must be a field access")
            };
            let _ = write!(self.output, "{tmp} = _asatsuyu_runtime.call_method(");
            self.emit_expr(receiver);
            let _ = write!(self.output, ", \"{}\"", info.symbol_name);
        } else {
            let runtime_binding = checked_runtime_binding(&info.module_name);
            let _ = write!(
                self.output,
                "{tmp} = _asatsuyu_runtime.call_function({runtime_binding}, \"{}\"",
                info.symbol_name
            );
        }
        self.emit_runtime_args(args);
        self.output.push_str(")\n");
    }

    /// Emit `let x = <checked_ffi_call>` as try/except + validator + assignment.
    fn emit_checked_ffi_let_stmt(
        &mut self,
        binding: DefId,
        func: &ThirExpr,
        args: &[ThirExpr],
        info: &CheckedFfiInfo,
        span: Span,
    ) {
        self.has_checked_ffi = true;
        let tmp = format!("_checked_{}", self.checked_counter);
        self.checked_counter += 1;
        let name = self.module.symbol_table.get(binding).name.clone();

        self.write_indent();
        self.output.push_str("try:");
        self.write_source_comment(span);
        self.output.push('\n');
        self.push_indent();
        self.emit_checked_ffi_call_expr(&tmp, func, args, info);
        self.pop_indent();
        self.emit_checked_except_block();
        self.emit_validator(&tmp, info);
        // x = _checked_N
        self.write_indent();
        let _ = writeln!(self.output, "{name} = {tmp}");
    }

    /// Emit bare `<checked_ffi_call>` as a statement.
    fn emit_checked_ffi_bare_stmt(
        &mut self,
        func: &ThirExpr,
        args: &[ThirExpr],
        info: &CheckedFfiInfo,
        span: Span,
    ) {
        self.has_checked_ffi = true;
        let tmp = format!("_checked_{}", self.checked_counter);
        self.checked_counter += 1;

        self.write_indent();
        self.output.push_str("try:");
        self.write_source_comment(span);
        self.output.push('\n');
        self.push_indent();
        self.emit_checked_ffi_call_expr(&tmp, func, args, info);
        self.pop_indent();
        self.emit_checked_except_block();
        self.emit_validator(&tmp, info);
    }

    /// Emit Checked FFI call in return position.
    fn emit_checked_ffi_return_stmt(
        &mut self,
        func: &ThirExpr,
        args: &[ThirExpr],
        info: &CheckedFfiInfo,
        span: Span,
    ) {
        self.has_checked_ffi = true;
        let tmp = format!("_checked_{}", self.checked_counter);
        self.checked_counter += 1;

        self.write_indent();
        self.output.push_str("try:");
        self.write_source_comment(span);
        self.output.push('\n');
        self.push_indent();
        self.emit_checked_ffi_call_expr(&tmp, func, args, info);
        self.pop_indent();
        self.emit_checked_except_block();
        self.emit_validator(&tmp, info);
        self.write_indent();
        let _ = writeln!(self.output, "return {tmp}");
    }

    /// Emit the common Checked FFI exception block using runtime normalization.
    fn emit_checked_except_block(&mut self) {
        self.write_indent();
        self.output.push_str("except Exception as _e:\n");
        self.push_indent();
        self.write_indent();
        if self.current_fn_returns_result {
            self.output.push_str(
                "return Error(PyException(**_asatsuyu_runtime.normalize_exception(_e)))\n",
            );
        } else {
            self.output.push_str("_asatsuyu_exc = _asatsuyu_runtime.normalize_exception(_e)\n");
            self.write_indent();
            self.output.push_str(
                "raise AsatsuyuError(f\"{_asatsuyu_exc['exception_type']}: {_asatsuyu_exc['message']}\")\n",
            );
        }
        self.pop_indent();
    }

    /// Emit isinstance validator for a Checked FFI return value.
    fn emit_validator(&mut self, tmp: &str, info: &CheckedFfiInfo) {
        let Some(ref return_ty) = info.return_ty else {
            return;
        };
        let Some(check) = ffi_type_to_isinstance(return_ty) else {
            return;
        };
        let qual = format!("{}.{}", info.module_name, info.symbol_name);
        self.write_indent();
        let _ = writeln!(self.output, "if not isinstance({tmp}, {check}):");
        self.push_indent();
        self.write_indent();
        let _ = writeln!(
            self.output,
            "raise AsatsuyuError(f\"{qual} returned unexpected type: {{type({tmp}).__name__}}\")"
        );
        self.pop_indent();
    }

    /// Emit hasattr guards for Checked FFI symbols after a module import.
    fn emit_hasattr_guards(&mut self, module_name: &SmolStr) {
        let Some(ffi_mod) = self.module.ffi_modules.get(module_name.as_str()) else {
            return;
        };
        let runtime_binding = checked_runtime_binding(module_name);
        for sym in &ffi_mod.symbols {
            if sym.trust_level == Some(FfiTrustLevel::Checked) {
                let sym_name = &sym.name;
                let _ = writeln!(self.output, "if not hasattr({runtime_binding}, '{sym_name}'):");
                let _ = writeln!(
                    self.output,
                    "    raise AsatsuyuError(\"{module_name}.{sym_name}: symbol not available at runtime (stub/runtime mismatch)\")"
                );
            }
        }
    }

    /// Emit runtime helper arguments after `call_function(..., "name"`.
    fn emit_runtime_args(&mut self, args: &[ThirExpr]) {
        for arg in args {
            self.output.push_str(", ");
            self.emit_expr(arg);
        }
    }

    fn emit_match_stmt(&mut self, subject: &ThirExpr, arms: &[ThirMatchArm], is_return: bool) {
        self.write_indent();
        self.output.push_str("match ");
        self.emit_expr(subject);
        self.output.push(':');
        self.write_source_comment(subject.span());
        self.output.push('\n');
        self.push_indent();
        for arm in arms {
            self.write_indent();
            self.output.push_str("case ");
            self.emit_pattern(&arm.pattern);
            self.output.push(':');
            self.write_source_comment(arm.span);
            self.output.push('\n');
            self.push_indent();
            if is_return {
                self.emit_return_stmt(&arm.body);
            } else {
                self.emit_stmt(&arm.body);
            }
            self.pop_indent();
        }
        self.pop_indent();
    }

    fn emit_pattern(&mut self, pattern: &ThirPattern) {
        match pattern {
            ThirPattern::Wildcard(_) => {
                self.output.push('_');
            }
            ThirPattern::Variable { def_id, .. } => {
                let name = &self.module.symbol_table.get(*def_id).name;
                self.output.push_str(name.as_str());
            }
            ThirPattern::Literal(lit) => {
                self.output.push_str(lit.value.as_str());
            }
            ThirPattern::Constructor { def_id, fields, .. } => {
                let name = &self.module.symbol_table.get(*def_id).name;
                let py_name = sanitize_python_name(name);
                self.output.push_str(&py_name);
                if fields.is_empty() {
                    // Nullary constructors need `()` in patterns to match as class.
                    self.output.push_str("()");
                } else {
                    self.output.push('(');
                    for (i, field) in fields.iter().enumerate() {
                        if i > 0 {
                            self.output.push_str(", ");
                        }
                        self.emit_pattern(field);
                    }
                    self.output.push(')');
                }
            }
            ThirPattern::Tuple { elements, .. } => {
                self.output.push('(');
                for (i, elem) in elements.iter().enumerate() {
                    if i > 0 {
                        self.output.push_str(", ");
                    }
                    self.emit_pattern(elem);
                }
                // Single-element tuples need a trailing comma.
                if elements.len() == 1 {
                    self.output.push(',');
                }
                self.output.push(')');
            }
            ThirPattern::List { elements, rest, .. } => {
                self.output.push('[');
                for (i, elem) in elements.iter().enumerate() {
                    if i > 0 {
                        self.output.push_str(", ");
                    }
                    self.emit_pattern(elem);
                }
                if let Some(rest_pat) = rest {
                    if !elements.is_empty() {
                        self.output.push_str(", ");
                    }
                    self.output.push('*');
                    self.emit_pattern(rest_pat);
                }
                self.output.push(']');
            }
        }
    }
}

fn is_result_ty(ty: &Ty) -> bool {
    matches!(ty, Ty::Named { name, .. } if name.as_str() == "Result")
}

// ── Type mapping ───────────────────────────────────────────────────

/// Map a binary operator to its Python syntax (with surrounding spaces).
fn binop_to_python(op: BinOp) -> &'static str {
    match op {
        BinOp::Add | BinOp::StringConcat => " + ",
        BinOp::Sub => " - ",
        BinOp::Mul => " * ",
        BinOp::Div => " / ",
        BinOp::Mod => " % ",
        BinOp::Eq => " == ",
        BinOp::NotEq => " != ",
        BinOp::Lt => " < ",
        BinOp::LtEq => " <= ",
        BinOp::Gt => " > ",
        BinOp::GtEq => " >= ",
        BinOp::And => " and ",
        BinOp::Or => " or ",
    }
}

/// Map a unary operator to its Python syntax.
fn unop_to_python(op: UnOp) -> &'static str {
    match op {
        UnOp::Neg => "-",
        UnOp::Not => "not ",
    }
}

/// Map an Asatsuyu [`Ty`] to its Python type annotation string.
///
/// `var_map` maps unresolved type variable IDs to PEP 695 parameter names
/// (e.g. `T`, `U`). When empty, any remaining `Ty::Var` falls back to `object`.
fn ty_to_python(ty: &Ty, var_map: &[(TyVarId, String)]) -> String {
    match ty {
        Ty::Primitive(PrimTy::Int) => "int".into(),
        Ty::Primitive(PrimTy::Float) => "float".into(),
        Ty::Primitive(PrimTy::String) => "str".into(),
        Ty::Primitive(PrimTy::Bool) => "bool".into(),
        Ty::Primitive(PrimTy::None) => "None".into(),
        Ty::Named { name, args, .. } => {
            // Map Asatsuyu builtin types to Python builtin names.
            if name.as_str() == "Task" {
                // Task(T) → Coroutine[Any, Any, T]
                let inner =
                    args.first().map_or_else(|| "object".into(), |a| ty_to_python(a, var_map));
                return format!("Coroutine[Any, Any, {inner}]");
            }
            let py_name = match name.as_str() {
                "List" => "list",
                "Dict" => "dict",
                _ => name.as_str(),
            };
            if args.is_empty() {
                py_name.to_string()
            } else {
                let arg_strs: Vec<String> = args.iter().map(|a| ty_to_python(a, var_map)).collect();
                format!("{py_name}[{}]", arg_strs.join(", "))
            }
        }
        Ty::FfiModule { module_name } => module_name.to_string(),
        Ty::FfiInstance { module, class } => format!("{module}.{class}"),
        Ty::Opaque { module, symbol } => format!("\"{module}.{symbol}\""),
        Ty::Var(id) => var_map
            .iter()
            .find(|(vid, _)| vid == id)
            .map_or_else(|| "object".into(), |(_, name)| name.clone()),
        Ty::Function { .. } | Ty::Error => "object".into(),
    }
}

/// Collect unique [`TyVarId`]s from a type, preserving insertion order.
fn collect_type_vars(ty: &Ty, vars: &mut Vec<TyVarId>) {
    match ty {
        Ty::Var(id) => {
            if !vars.contains(id) {
                vars.push(*id);
            }
        }
        Ty::Named { args, .. } => {
            for arg in args {
                collect_type_vars(arg, vars);
            }
        }
        Ty::Function { params, ret } => {
            for p in params {
                collect_type_vars(p, vars);
            }
            collect_type_vars(ret, vars);
        }
        _ => {}
    }
}

/// Map type variable IDs to PEP 695 parameter names (`T`, `U`, `V`, `W`, `T4`, …).
fn build_fn_type_param_map(var_ids: &[TyVarId]) -> Vec<(TyVarId, String)> {
    const NAMES: &[&str] = &["T", "U", "V", "W"];
    var_ids
        .iter()
        .enumerate()
        .map(|(i, &id)| {
            let name = if i < NAMES.len() { NAMES[i].to_string() } else { format!("T{i}") };
            (id, name)
        })
        .collect()
}

// ── ADT helpers ───────────────────────────────────────────────────

/// Python keywords and builtins that must be suffixed with `_` when used as names.
const PYTHON_RESERVED: &[&str] = &[
    "None", "True", "False", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

/// Append `_` to names that collide with Python keywords.
fn sanitize_python_name(name: &str) -> String {
    if PYTHON_RESERVED.contains(&name) { format!("{name}_") } else { name.to_string() }
}

/// Map Asatsuyu type parameter names to PEP 695 convention.
///
/// `["a"]` → `[("a", "T")]`, `["a", "b"]` → `[("a", "T"), ("b", "E")]`.
fn build_type_param_map(params: &[SmolStr]) -> Vec<(SmolStr, String)> {
    const NAMES: &[&str] = &["T", "U", "V", "W"];
    params
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let py = if i < NAMES.len() { NAMES[i].to_string() } else { format!("T{i}") };
            (name.clone(), py)
        })
        .collect()
}

/// Convert an [`HirTypeExpr`] to a Python type annotation string.
///
/// Applies the type parameter mapping (e.g., `a` → `T`) and converts
/// primitive type names to Python equivalents.
fn hir_type_expr_to_python(te: &HirTypeExpr, param_map: &[(SmolStr, String)]) -> String {
    // Check if it's a mapped type parameter.
    if let Some((_, py_name)) = param_map.iter().find(|(asty, _)| *asty == te.name) {
        return py_name.clone();
    }

    // Primitive type names.
    let base = match te.name.as_str() {
        "Int" => "int",
        "Float" => "float",
        "String" => "str",
        "Bool" => "bool",
        "None" => "None",
        _ => te.name.as_str(),
    };

    if te.args.is_empty() {
        base.to_string()
    } else {
        let args: Vec<String> =
            te.args.iter().map(|a| hir_type_expr_to_python(a, param_map)).collect();
        format!("{base}[{}]", args.join(", "))
    }
}

/// Collect type parameter names actually used in a variant's fields.
fn collect_used_params(fields: &[HirFieldType], param_map: &[(SmolStr, String)]) -> Vec<String> {
    let mut used = Vec::new();
    for (asty_name, py_name) in param_map {
        if fields.iter().any(|f| type_expr_mentions(asty_name, &f.type_expr)) {
            used.push(py_name.clone());
        }
    }
    used
}

/// Check if a type expression mentions a given name (type parameter).
fn type_expr_mentions(name: &SmolStr, te: &HirTypeExpr) -> bool {
    if te.name == *name {
        return true;
    }
    te.args.iter().any(|a| type_expr_mentions(name, a))
}

/// Recursively check whether a THIR expression contains a `Try` node.
fn expr_contains_try(expr: &ThirExpr) -> bool {
    match expr {
        ThirExpr::Try { .. } => true,
        ThirExpr::Block { exprs, .. } => exprs.iter().any(expr_contains_try),
        ThirExpr::Call { func, args, .. } => {
            expr_contains_try(func) || args.iter().any(expr_contains_try)
        }
        ThirExpr::Let { value, .. } | ThirExpr::Assign { value, .. } => expr_contains_try(value),
        ThirExpr::If { condition, then_body, else_body, .. } => {
            expr_contains_try(condition)
                || expr_contains_try(then_body)
                || else_body.as_ref().is_some_and(|e| expr_contains_try(e))
        }
        ThirExpr::Match { subject, arms, .. } => {
            expr_contains_try(subject) || arms.iter().any(|a| expr_contains_try(&a.body))
        }
        ThirExpr::BinaryOp { lhs, rhs, .. } => expr_contains_try(lhs) || expr_contains_try(rhs),
        ThirExpr::UnaryOp { expr, .. } | ThirExpr::Await { expr, .. } => expr_contains_try(expr),
        ThirExpr::Lambda { body, .. } => expr_contains_try(body),
        ThirExpr::FieldAccess { receiver, .. } => expr_contains_try(receiver),
        ThirExpr::List { elements, .. } => elements.iter().any(expr_contains_try),
        ThirExpr::Literal(_) | ThirExpr::Var { .. } => false,
    }
}

/// Returns `true` if the type contains `Task(T)` anywhere.
fn ty_contains_task(ty: &Ty) -> bool {
    match ty {
        Ty::Named { name, args, .. } => {
            name.as_str() == "Task" || args.iter().any(ty_contains_task)
        }
        Ty::Function { params, ret } => {
            params.iter().any(ty_contains_task) || ty_contains_task(ret)
        }
        _ => false,
    }
}

/// Map an [`FfiType`] to its Python `isinstance` check expression.
///
/// Returns `None` for complex types where MVP skips validation.
fn ffi_type_to_isinstance(ty: &FfiType) -> Option<&'static str> {
    match ty {
        FfiType::Any => Some("(dict, list, str, int, float, bool, type(None))"),
        FfiType::Int => Some("int"),
        FfiType::Float => Some("(int, float)"),
        FfiType::Str => Some("str"),
        FfiType::Bool => Some("bool"),
        FfiType::NoneType => Some("type(None)"),
        FfiType::Bytes => Some("bytes"),
        FfiType::List(_) => Some("list"),
        FfiType::Dict(_, _) => Some("dict"),
        FfiType::Tuple(_) => Some("tuple"),
        FfiType::Optional(_) | FfiType::Union(_) | FfiType::Named { .. } => None,
    }
}

fn checked_runtime_binding(module_name: &str) -> String {
    let suffix: String =
        module_name.chars().map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' }).collect();
    format!("_checked_runtime_{suffix}")
}
