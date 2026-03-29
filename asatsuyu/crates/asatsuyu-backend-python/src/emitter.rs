//! THIR → Python 3.12+ source code emitter.
//!
//! Walks the [`ThirModule`] tree and writes readable Python with type annotations.

use std::fmt::Write;

use asatsuyu_ast::{BinOp, UnOp};
use asatsuyu_ty::{PrimTy, ThirExpr, ThirFnDef, ThirModule, Ty};

/// 4-space indentation per PEP 8.
const INDENT: &str = "    ";

/// Emits Python source code from a typed HIR module.
pub(crate) struct Emitter<'a> {
    module: &'a ThirModule,
    output: String,
    indent: usize,
}

impl<'a> Emitter<'a> {
    pub(crate) fn new(module: &'a ThirModule) -> Self {
        Self { module, output: String::new(), indent: 0 }
    }

    pub(crate) fn emit(&mut self) {
        self.emit_module();
    }

    pub(crate) fn into_output(self) -> String {
        self.output
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
        for (i, fn_def) in self.module.functions.iter().enumerate() {
            if i > 0 {
                // PEP 8: two blank lines between top-level definitions.
                self.output.push('\n');
                self.output.push('\n');
            }
            self.emit_fn_def(fn_def);
        }
    }

    // ── Function ───────────────────────────────────────────────────

    fn emit_fn_def(&mut self, fn_def: &ThirFnDef) {
        // def name(params) -> return_ty:
        self.write_indent();
        let name = &self.module.symbol_table.get(fn_def.def_id).name;
        let _ = write!(self.output, "def {name}(");

        for (i, param) in fn_def.params.iter().enumerate() {
            if i > 0 {
                self.output.push_str(", ");
            }
            let param_name = &self.module.symbol_table.get(param.def_id).name;
            let _ = write!(self.output, "{param_name}: {}", ty_to_python(&param.ty));
        }

        let _ = writeln!(self.output, ") -> {}:", ty_to_python(&fn_def.return_ty));

        // Body.
        self.push_indent();
        self.emit_body(&fn_def.body);
        self.pop_indent();
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
        self.write_indent();
        self.emit_expr(expr);
        self.output.push('\n');
    }

    fn emit_return_stmt(&mut self, expr: &ThirExpr) {
        self.write_indent();
        self.output.push_str("return ");
        self.emit_expr(expr);
        self.output.push('\n');
    }

    // ── Expressions (inline, no newline) ───────────────────────────

    fn emit_expr(&mut self, expr: &ThirExpr) {
        match expr {
            ThirExpr::Literal(lit) => {
                self.output.push_str(lit.value.as_str());
            }
            ThirExpr::Var { def_id, .. } => {
                let name = &self.module.symbol_table.get(*def_id).name;
                self.output.push_str(name.as_str());
            }
            ThirExpr::Block { exprs, .. } => {
                // Nested block in expression position: emit the last expression.
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
            ThirExpr::Match { subject, arms, .. } => {
                // Placeholder: pattern typing is Issue 26+.
                // Emit a basic match/case structure.
                // In statement position this would be multi-line; in expression
                // position we fall back to emitting the first arm body.
                if let Some(first) = arms.first() {
                    self.emit_expr(&first.body);
                }
                let _ = (subject, arms);
            }
            ThirExpr::Let { binding, value, .. } => {
                let name = &self.module.symbol_table.get(*binding).name;
                self.output.push_str(name.as_str());
                self.output.push_str(" = ");
                self.emit_expr(value);
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
        }
    }

    fn emit_builtin_call(&mut self, func: &ThirExpr, args: &[ThirExpr]) -> bool {
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
        } else {
            false
        }
    }
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
fn ty_to_python(ty: &Ty) -> &'static str {
    match ty {
        Ty::Primitive(PrimTy::Int) => "int",
        Ty::Primitive(PrimTy::Float) => "float",
        Ty::Primitive(PrimTy::String) => "str",
        Ty::Primitive(PrimTy::Bool) => "bool",
        Ty::Primitive(PrimTy::None) => "None",
        Ty::Function { .. } | Ty::Var(_) | Ty::Error => "Any",
    }
}
