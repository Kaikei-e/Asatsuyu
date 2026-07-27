//! Purity analysis over the HIR call graph.
//!
//! Every function is classified as [`Purity::Pure`] or [`Purity::Effectful`].
//! Effects are seeded where the program reaches the Python boundary and then
//! propagated backwards along static call edges to a least fixpoint. The
//! lattice is `Pure ⊑ Effectful`, so a worklist over the reverse call graph
//! converges in `O(V + E)`.
//!
//! # What this analysis cannot decide
//!
//! A call whose callee is not a statically known function — applying a
//! function-typed parameter, or invoking a method on a value whose type is
//! only known after inference — is recorded in [`FunctionPurity::unresolved`]
//! rather than folded into the result. Those call sites are the open question:
//! resolving them either requires effect polymorphism or a conservative
//! approximation that pushes the enclosing function to `Effectful`.
//!
//! Deliberate approximations, both of which over-report effects:
//!
//! - A lambda's body is attributed to the function that *creates* it, not to
//!   whoever calls it. A function that merely builds an effectful closure is
//!   therefore reported as effectful.
//! - Reassignment of a `let mut` binding is not an effect. Local mutation
//!   cannot escape its scope, so it is not observable to a caller.

use std::collections::{HashMap, HashSet};

use asatsuyu_syntax::Span;
use smol_str::SmolStr;

use crate::types::{DefId, DefKind, HirExpr, HirImportKind, HirModule};

/// Built-in functions whose call produces observable output.
const EFFECTFUL_BUILTINS: &[&str] = &["println"];

// ── Lattice ─────────────────────────────────────────────────────────

/// Where a function sits on the purity lattice.
///
/// The lattice has height 1, which is what makes the fixpoint linear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Purity {
    /// No observable effect is reachable from this function.
    Pure,
    /// An observable effect is reachable from this function.
    Effectful,
}

/// Why a function was classified as [`Purity::Effectful`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectSource {
    /// The body reaches the Python boundary: an FFI call, an attribute read on
    /// an FFI module, a `try` expression, or an effectful built-in.
    Boundary,
    /// The function is declared `async`, or its body awaits a `Task`.
    Async,
    /// A callee is effectful.
    Propagated,
}

// ── Unresolved call sites ───────────────────────────────────────────

/// A call site this analysis could not attribute to a known function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnresolvedCall {
    /// Span of the call expression.
    pub span: Span,
    /// Why the callee could not be resolved.
    pub kind: UnresolvedCallKind,
}

/// Why a callee could not be resolved to a definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnresolvedCallKind {
    /// Applying a function-typed parameter: `f(x)` where `f` is a parameter.
    ///
    /// Genuinely higher-order — no amount of extra context resolves it, only
    /// effect polymorphism or a conservative approximation does.
    ParameterApplied,
    /// Applying a value bound by `let` or a match arm.
    ///
    /// Also genuinely higher-order.
    LocalApplied,
    /// A method call on a local or parameter: `p.exists()`.
    ///
    /// Not higher-order. Deciding it needs the receiver's inferred type, which
    /// exists only after `asatsuyu-ty` runs.
    ReceiverUntyped,
    /// A member of an internal module import: `output.write(x)`.
    ///
    /// Needs cross-module resolution, which the compiler does not yet perform.
    ModuleMember,
    /// The callee is an arbitrary expression rather than a name.
    ComputedCallee,
}

impl UnresolvedCallKind {
    /// Returns `true` when resolving this call site requires reasoning about
    /// effects that flow through function values.
    #[must_use]
    pub fn is_higher_order(self) -> bool {
        matches!(self, Self::ParameterApplied | Self::LocalApplied)
    }
}

// ── Per-function result ─────────────────────────────────────────────

/// The analysis result for a single function.
#[derive(Debug, Clone)]
pub struct FunctionPurity {
    /// The function's definition.
    pub def_id: DefId,
    /// The function's name, for reporting.
    pub name: SmolStr,
    /// Computed purity.
    pub purity: Purity,
    /// Why it is effectful. `None` when pure.
    pub source: Option<EffectSource>,
    /// Call sites whose callee could not be resolved.
    pub unresolved: Vec<UnresolvedCall>,
}

impl FunctionPurity {
    /// Returns `true` when this function is pure only because unresolved
    /// higher-order call sites were assumed pure.
    ///
    /// These are exactly the functions whose classification would flip under a
    /// conservative approximation.
    #[must_use]
    pub fn is_undecided(&self) -> bool {
        self.purity == Purity::Pure
            && self.unresolved.iter().any(|call| call.kind.is_higher_order())
    }
}

// ── Report ──────────────────────────────────────────────────────────

/// Purity classification for every function in a module.
///
/// Functions appear in source order, so the report is deterministic.
#[derive(Debug, Clone)]
pub struct PurityReport {
    /// One entry per top-level function, in source order.
    pub functions: Vec<FunctionPurity>,
}

impl PurityReport {
    /// Looks up the purity of a function by its definition.
    #[must_use]
    pub fn purity_of(&self, def_id: DefId) -> Option<Purity> {
        self.functions.iter().find(|f| f.def_id == def_id).map(|f| f.purity)
    }

    /// Number of functions classified as [`Purity::Pure`].
    #[must_use]
    pub fn pure_count(&self) -> usize {
        self.functions.iter().filter(|f| f.purity == Purity::Pure).count()
    }

    /// Number of functions classified as [`Purity::Effectful`].
    #[must_use]
    pub fn effectful_count(&self) -> usize {
        self.functions.iter().filter(|f| f.purity == Purity::Effectful).count()
    }

    /// Total unresolved call sites across the module.
    #[must_use]
    pub fn unresolved_count(&self) -> usize {
        self.functions.iter().map(|f| f.unresolved.len()).sum()
    }

    /// Unresolved call sites that are genuinely higher-order.
    #[must_use]
    pub fn higher_order_count(&self) -> usize {
        self.functions
            .iter()
            .flat_map(|f| &f.unresolved)
            .filter(|call| call.kind.is_higher_order())
            .count()
    }

    /// Functions whose classification would flip under a conservative
    /// approximation of higher-order calls.
    pub fn undecided(&self) -> impl Iterator<Item = &FunctionPurity> {
        self.functions.iter().filter(|f| f.is_undecided())
    }
}

// ── Entry point ─────────────────────────────────────────────────────

/// Classifies every function in `module` as pure or effectful.
#[must_use]
pub fn analyze(module: &HirModule) -> PurityReport {
    let python_imports: HashSet<DefId> = module
        .imports
        .iter()
        .filter(|import| matches!(import.kind, HirImportKind::Python { .. }))
        .map(|import| import.def_id)
        .collect();

    let mut functions = Vec::with_capacity(module.functions.len());
    let mut edges = Vec::with_capacity(module.functions.len());

    for func in &module.functions {
        let mut scan = Scan {
            module,
            python_imports: &python_imports,
            effect: if func.is_async { Some(EffectSource::Async) } else { None },
            callees: Vec::new(),
            unresolved: Vec::new(),
        };
        scan.visit(&func.body);

        let purity = if scan.effect.is_some() { Purity::Effectful } else { Purity::Pure };
        functions.push(FunctionPurity {
            def_id: func.def_id,
            name: module.symbol_table.get(func.def_id).name.clone(),
            purity,
            source: scan.effect,
            unresolved: scan.unresolved,
        });
        edges.push(scan.callees);
    }

    propagate(&mut functions, &edges);
    PurityReport { functions }
}

/// Propagates effects backwards along call edges until a fixpoint is reached.
fn propagate(functions: &mut [FunctionPurity], edges: &[Vec<DefId>]) {
    let index: HashMap<DefId, usize> =
        functions.iter().enumerate().map(|(i, f)| (f.def_id, i)).collect();

    // Reverse the call graph: callee -> callers.
    let mut callers: HashMap<usize, Vec<usize>> = HashMap::new();
    for (caller, callees) in edges.iter().enumerate() {
        for callee in callees {
            if let Some(&target) = index.get(callee) {
                callers.entry(target).or_default().push(caller);
            }
        }
    }

    let mut worklist: Vec<usize> = functions
        .iter()
        .enumerate()
        .filter(|(_, f)| f.purity == Purity::Effectful)
        .map(|(i, _)| i)
        .collect();

    while let Some(current) = worklist.pop() {
        let Some(incoming) = callers.get(&current) else { continue };
        for &caller in incoming {
            if functions[caller].purity == Purity::Pure {
                functions[caller].purity = Purity::Effectful;
                functions[caller].source = Some(EffectSource::Propagated);
                worklist.push(caller);
            }
        }
    }
}

// ── Body scan ───────────────────────────────────────────────────────

/// Collects direct effects, static call edges, and unresolved call sites for
/// one function body.
struct Scan<'a> {
    module: &'a HirModule,
    python_imports: &'a HashSet<DefId>,
    effect: Option<EffectSource>,
    callees: Vec<DefId>,
    unresolved: Vec<UnresolvedCall>,
}

impl Scan<'_> {
    /// Records a direct effect. The first one found in source order wins.
    fn mark(&mut self, source: EffectSource) {
        if self.effect.is_none() {
            self.effect = Some(source);
        }
    }

    fn unresolved(&mut self, span: Span, kind: UnresolvedCallKind) {
        self.unresolved.push(UnresolvedCall { span, kind });
    }

    fn visit(&mut self, expr: &HirExpr) {
        match expr {
            HirExpr::Literal(_) | HirExpr::Var(..) => {}
            HirExpr::Call { func, args, span } => {
                self.visit_callee(func, *span);
                for arg in args {
                    self.visit(arg);
                }
            }
            HirExpr::FieldAccess { receiver, .. } => {
                // A bare attribute read on an FFI module is already a boundary
                // crossing: `sys.platform` runs Python code.
                if let Some(root) = root_def(receiver)
                    && self.python_imports.contains(&root)
                {
                    self.mark(EffectSource::Boundary);
                }
                self.visit(receiver);
            }
            // `try` exists only to absorb Python exceptions, so it always
            // marks a boundary crossing.
            HirExpr::Try { expr, .. } => {
                self.mark(EffectSource::Boundary);
                self.visit(expr);
            }
            HirExpr::Await { expr, .. } => {
                self.mark(EffectSource::Async);
                self.visit(expr);
            }
            HirExpr::Block { exprs: items, .. } | HirExpr::List { elements: items, .. } => {
                for item in items {
                    self.visit(item);
                }
            }
            HirExpr::BinaryOp { lhs, rhs, .. } => {
                self.visit(lhs);
                self.visit(rhs);
            }
            HirExpr::UnaryOp { expr, .. } | HirExpr::Lambda { body: expr, .. } => self.visit(expr),
            HirExpr::If { condition, then_body, else_body, .. } => {
                self.visit(condition);
                self.visit(then_body);
                if let Some(alt) = else_body {
                    self.visit(alt);
                }
            }
            HirExpr::Match { subject, arms, .. } => {
                self.visit(subject);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.visit(guard);
                    }
                    self.visit(&arm.body);
                }
            }
            // Reassigning a `let mut` binding is not observable outside the
            // function, so it is not an effect.
            HirExpr::Let { value, .. } | HirExpr::Assign { value, .. } => self.visit(value),
        }
    }

    fn visit_callee(&mut self, func: &HirExpr, span: Span) {
        match func {
            HirExpr::Var(def_id, _) => self.classify_applied(*def_id, span),
            HirExpr::FieldAccess { receiver, .. } => {
                if let Some(root) = root_def(receiver) {
                    self.classify_member(root, span);
                } else {
                    self.unresolved(span, UnresolvedCallKind::ComputedCallee);
                    self.visit(receiver);
                }
            }
            other => {
                self.unresolved(span, UnresolvedCallKind::ComputedCallee);
                self.visit(other);
            }
        }
    }

    /// Classifies `name(..)` where `name` resolves to `def_id`.
    fn classify_applied(&mut self, def_id: DefId, span: Span) {
        let data = self.module.symbol_table.get(def_id);
        match data.kind {
            DefKind::Function => self.callees.push(def_id),
            DefKind::Builtin => {
                if EFFECTFUL_BUILTINS.contains(&data.name.as_str()) {
                    self.mark(EffectSource::Boundary);
                }
            }
            DefKind::Constructor | DefKind::Type => {}
            DefKind::Parameter => self.unresolved(span, UnresolvedCallKind::ParameterApplied),
            DefKind::LocalBinding => self.unresolved(span, UnresolvedCallKind::LocalApplied),
            DefKind::Import => self.classify_import(def_id, span),
        }
    }

    /// Classifies `root.member(..)` where `root` resolves to `def_id`.
    fn classify_member(&mut self, def_id: DefId, span: Span) {
        match self.module.symbol_table.get(def_id).kind {
            DefKind::Import => self.classify_import(def_id, span),
            // `list.map(f)` and friends: purity flows through the lambda
            // arguments, which the caller visits separately.
            DefKind::Builtin | DefKind::Constructor | DefKind::Type => {}
            DefKind::Parameter | DefKind::LocalBinding | DefKind::Function => {
                self.unresolved(span, UnresolvedCallKind::ReceiverUntyped);
            }
        }
    }

    fn classify_import(&mut self, def_id: DefId, span: Span) {
        if self.python_imports.contains(&def_id) {
            self.mark(EffectSource::Boundary);
        } else {
            self.unresolved(span, UnresolvedCallKind::ModuleMember);
        }
    }
}

/// Walks a field-access chain down to the name it is rooted at.
fn root_def(expr: &HirExpr) -> Option<DefId> {
    match expr {
        HirExpr::Var(def_id, _) => Some(*def_id),
        HirExpr::FieldAccess { receiver, .. } => root_def(receiver),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lower_to_hir;
    use asatsuyu_parser::parse;
    use asatsuyu_syntax::FileId;

    fn report_for(source: &str) -> PurityReport {
        let cst = parse(FileId(0), source);
        let ast = asatsuyu_ast::lower(&cst, FileId(0));
        let hir = lower_to_hir(&ast.module);
        assert!(!hir.has_errors(), "lowering failed: {:?}", hir.diagnostics);
        analyze(&hir.module)
    }

    fn find<'a>(report: &'a PurityReport, name: &str) -> &'a FunctionPurity {
        report.functions.iter().find(|f| f.name == name).expect("function not found")
    }

    #[test]
    fn arithmetic_function_is_pure() {
        let report = report_for("pub fn double(x: Int) -> Int { x * 2 }");
        assert_eq!(find(&report, "double").purity, Purity::Pure);
        assert_eq!(report.unresolved_count(), 0);
    }

    #[test]
    fn println_is_a_boundary_effect() {
        let report = report_for(r#"pub fn shout() { println("hi") }"#);
        let shout = find(&report, "shout");
        assert_eq!(shout.purity, Purity::Effectful);
        assert_eq!(shout.source, Some(EffectSource::Boundary));
    }

    #[test]
    fn ffi_call_is_a_boundary_effect() {
        let report = report_for("from python import os\n\npub fn cwd() -> String { os.getcwd() }");
        assert_eq!(find(&report, "cwd").source, Some(EffectSource::Boundary));
    }

    #[test]
    fn ffi_attribute_read_is_a_boundary_effect() {
        let report =
            report_for("from python import sys\n\npub fn platform() -> String { sys.platform }");
        assert_eq!(find(&report, "platform").source, Some(EffectSource::Boundary));
    }

    #[test]
    fn try_marks_a_boundary_crossing() {
        let report =
            report_for("from python import os\n\npub fn cwd() { let c = try os.getcwd()\n  c }");
        assert_eq!(find(&report, "cwd").source, Some(EffectSource::Boundary));
    }

    #[test]
    fn async_function_is_effectful() {
        let report = report_for("pub async fn fetch() -> Int { 1 }");
        assert_eq!(find(&report, "fetch").source, Some(EffectSource::Async));
    }

    #[test]
    fn effects_propagate_through_the_call_graph() {
        let report = report_for(
            r#"
fn leaf() { println("x") }

fn middle() { leaf() }

pub fn top() { middle() }
"#,
        );
        assert_eq!(find(&report, "leaf").source, Some(EffectSource::Boundary));
        assert_eq!(find(&report, "middle").source, Some(EffectSource::Propagated));
        assert_eq!(find(&report, "top").source, Some(EffectSource::Propagated));
    }

    #[test]
    fn mutual_recursion_reaches_a_fixpoint() {
        let report = report_for(
            r"
fn ping(n: Int) -> Int { pong(n) }

fn pong(n: Int) -> Int { ping(n) }

pub fn start(n: Int) -> Int { ping(n) }
",
        );
        assert_eq!(report.effectful_count(), 0);
        assert_eq!(report.pure_count(), 3);
    }

    #[test]
    fn local_mutation_is_not_an_effect() {
        let report = report_for(
            r"
pub fn total(n: Int) -> Int {
  let mut acc = 0
  acc = acc + n
  acc
}
",
        );
        assert_eq!(find(&report, "total").purity, Purity::Pure);
    }

    #[test]
    fn applying_a_parameter_is_higher_order_and_undecided() {
        let report = report_for("pub fn apply(f: Int, x: Int) -> Int { f(x) }");
        let apply = find(&report, "apply");
        assert_eq!(apply.purity, Purity::Pure);
        assert_eq!(apply.unresolved.len(), 1);
        assert_eq!(apply.unresolved[0].kind, UnresolvedCallKind::ParameterApplied);
        assert!(apply.is_undecided());
        assert_eq!(report.higher_order_count(), 1);
    }

    #[test]
    fn lambda_effects_are_attributed_to_the_enclosing_function() {
        let report = report_for(
            r#"pub fn build() { let f = fn(x: Int) { println("side") }
  f(1) }"#,
        );
        let build = find(&report, "build");
        assert_eq!(build.source, Some(EffectSource::Boundary));
    }

    #[test]
    fn method_call_on_a_local_needs_type_information() {
        let report = report_for(
            "from python import pathlib\n\npub fn check(path: String) -> Bool {\n  let p = pathlib.Path(path)\n  p.exists()\n}",
        );
        let check = find(&report, "check");
        assert_eq!(check.source, Some(EffectSource::Boundary));
        assert!(
            check.unresolved.iter().any(|c| c.kind == UnresolvedCallKind::ReceiverUntyped),
            "expected a receiver-untyped call site: {:?}",
            check.unresolved
        );
        assert_eq!(report.higher_order_count(), 0);
    }
}
