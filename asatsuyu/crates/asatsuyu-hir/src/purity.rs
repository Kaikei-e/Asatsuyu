//! Purity analysis over the HIR call graph.
//!
//! Every function is classified as [`Purity::Pure`] or [`Purity::Effectful`].
//! Effects are seeded where the program reaches the Python boundary and then
//! propagated backwards along static call edges to a least fixpoint. The
//! lattice is `Pure ⊑ Effectful`, so a worklist over the reverse call graph
//! converges in `O(V + E)`.
//!
//! # Soundness
//!
//! The analysis is conservative: a call it cannot attribute to a known
//! function pushes the enclosing function to [`Purity::Effectful`] with
//! [`EffectSource::Unresolved`]. `Pure` therefore means "proven to have no
//! reachable effect", which is what makes a `pure` declaration worth checking.
//! The unattributable call sites are still listed in
//! [`FunctionPurity::unresolved`], so a later pass with more information —
//! inferred receiver types, cross-module resolution — can narrow them without
//! changing what `Pure` means.
//!
//! Deliberate approximations, both of which over-report effects:
//!
//! - A lambda's body is attributed to the function that *creates* it, not to
//!   whoever calls it. A function that merely builds an effectful closure is
//!   therefore reported as effectful.
//! - Reassignment of a `let mut` binding is not an effect. Local mutation
//!   cannot escape its scope, so it is not observable to a caller.

use std::collections::{HashMap, HashSet};

use asatsuyu_syntax::{Diagnostic, DiagnosticCode, Span};
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
    /// A call site could not be attributed to a known function, so the effect
    /// is assumed rather than proven.
    Unresolved,
}

impl EffectSource {
    /// Returns `true` when the effect was observed rather than assumed.
    #[must_use]
    pub fn is_proven(self) -> bool {
        !matches!(self, Self::Unresolved)
    }
}

/// Where a function's effect enters, and what to blame for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectOrigin {
    /// What kind of effect this is.
    pub source: EffectSource,
    /// The expression that introduces the effect into this function.
    pub span: Span,
    /// The effectful callee, for [`EffectSource::Propagated`].
    pub callee: Option<DefId>,
}

/// One hop along the chain from a function to the effect it reaches.
#[derive(Debug, Clone)]
pub struct EffectStep {
    /// The function this step describes.
    pub def_id: DefId,
    /// Its name, for reporting.
    pub name: SmolStr,
    /// Where its effect enters.
    pub origin: EffectOrigin,
    /// Name of the callee blamed by a [`EffectSource::Propagated`] origin.
    pub callee_name: Option<SmolStr>,
}

impl EffectStep {
    /// Renders this step as a diagnostic label.
    #[must_use]
    pub fn label(&self) -> String {
        match self.origin.source {
            EffectSource::Boundary => "this crosses the Python boundary".to_string(),
            EffectSource::Async => format!("`{}` is async, which is an effect", self.name),
            EffectSource::Propagated => match &self.callee_name {
                Some(callee) => format!("`{}` calls `{callee}` here", self.name),
                None => format!("`{}` calls an effectful function here", self.name),
            },
            EffectSource::Unresolved => {
                "this call cannot be resolved, so it counts as an effect".to_string()
            }
        }
    }
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
    /// Where its effect enters. `None` when pure.
    pub origin: Option<EffectOrigin>,
    /// Call sites whose callee could not be resolved.
    pub unresolved: Vec<UnresolvedCall>,
}

impl FunctionPurity {
    /// Why this function is effectful. `None` when pure.
    #[must_use]
    pub fn source(&self) -> Option<EffectSource> {
        self.origin.map(|origin| origin.source)
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

    /// The chain from `def_id` to the effect that makes it effectful.
    ///
    /// The first step is `def_id` itself and the last step holds the effect
    /// that is not merely propagated. Returns an empty vector when the
    /// function is pure or unknown. Recursion through effectful callees is
    /// cut off rather than followed twice.
    #[must_use]
    pub fn effect_path(&self, def_id: DefId) -> Vec<EffectStep> {
        let mut path = Vec::new();
        let mut seen = HashSet::new();
        let mut current = def_id;

        while seen.insert(current) {
            let Some(func) = self.functions.iter().find(|f| f.def_id == current) else { break };
            let Some(origin) = func.origin else { break };

            let callee_name = origin
                .callee
                .and_then(|callee| self.functions.iter().find(|f| f.def_id == callee))
                .map(|f| f.name.clone());

            path.push(EffectStep { def_id: current, name: func.name.clone(), origin, callee_name });

            match origin.callee {
                Some(callee) => current = callee,
                None => break,
            }
        }

        path
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
            proven: (func.is_async).then_some(EffectOrigin {
                source: EffectSource::Async,
                span: func.span,
                callee: None,
            }),
            assumed: None,
            callees: Vec::new(),
            unresolved: Vec::new(),
        };
        scan.visit(&func.body);

        // A proven effect makes a better diagnostic than an assumed one, so it
        // wins even when the assumed one appears earlier in the body.
        let origin = scan.proven.or(scan.assumed);
        let purity = if origin.is_some() { Purity::Effectful } else { Purity::Pure };
        functions.push(FunctionPurity {
            def_id: func.def_id,
            name: module.symbol_table.get(func.def_id).name.clone(),
            purity,
            origin,
            unresolved: scan.unresolved,
        });
        edges.push(scan.callees);
    }

    propagate(&mut functions, &edges);
    PurityReport { functions }
}

/// Propagates effects backwards along call edges until a fixpoint is reached.
fn propagate(functions: &mut [FunctionPurity], edges: &[Vec<(DefId, Span)>]) {
    let index: HashMap<DefId, usize> =
        functions.iter().enumerate().map(|(i, f)| (f.def_id, i)).collect();

    // Reverse the call graph: callee -> (caller, call span).
    let mut callers: HashMap<usize, Vec<(usize, Span)>> = HashMap::new();
    for (caller, callees) in edges.iter().enumerate() {
        for &(callee, span) in callees {
            if let Some(&target) = index.get(&callee) {
                callers.entry(target).or_default().push((caller, span));
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
        let Some(incoming) = callers.get(&current).cloned() else { continue };
        let callee = functions[current].def_id;
        for (caller, span) in incoming {
            if functions[caller].purity == Purity::Pure {
                functions[caller].purity = Purity::Effectful;
                functions[caller].origin = Some(EffectOrigin {
                    source: EffectSource::Propagated,
                    span,
                    callee: Some(callee),
                });
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
    proven: Option<EffectOrigin>,
    assumed: Option<EffectOrigin>,
    callees: Vec<(DefId, Span)>,
    unresolved: Vec<UnresolvedCall>,
}

impl Scan<'_> {
    /// Records an observed effect. The first one in source order wins.
    fn mark(&mut self, source: EffectSource, span: Span) {
        if self.proven.is_none() {
            self.proven = Some(EffectOrigin { source, span, callee: None });
        }
    }

    /// Records a call site that cannot be attributed to a known function.
    ///
    /// Such a call counts as an effect, so that `Pure` stays a proof rather
    /// than an optimistic guess.
    fn unresolved(&mut self, span: Span, kind: UnresolvedCallKind) {
        self.unresolved.push(UnresolvedCall { span, kind });
        if self.assumed.is_none() {
            self.assumed =
                Some(EffectOrigin { source: EffectSource::Unresolved, span, callee: None });
        }
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
            HirExpr::FieldAccess { receiver, span, .. } => {
                // A bare attribute read on an FFI module is already a boundary
                // crossing: `sys.platform` runs Python code.
                if let Some(root) = root_def(receiver)
                    && self.python_imports.contains(&root)
                {
                    self.mark(EffectSource::Boundary, *span);
                }
                self.visit(receiver);
            }
            // `try` exists only to absorb Python exceptions, so it always
            // marks a boundary crossing.
            HirExpr::Try { expr, span } => {
                self.mark(EffectSource::Boundary, *span);
                self.visit(expr);
            }
            HirExpr::Await { expr, span } => {
                self.mark(EffectSource::Async, *span);
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
            DefKind::Function => self.callees.push((def_id, span)),
            DefKind::Builtin => {
                if EFFECTFUL_BUILTINS.contains(&data.name.as_str()) {
                    self.mark(EffectSource::Boundary, span);
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
            self.mark(EffectSource::Boundary, span);
        } else {
            self.unresolved(span, UnresolvedCallKind::ModuleMember);
        }
    }
}

// ── Checking declarations against the analysis ──────────────────────

/// Checks every `pure` declaration in `module` against `report`.
///
/// Inference alone never contradicts itself, so this is where a purity
/// mistake becomes an error: the author asserts `pure`, and the analysis
/// either agrees or produces the chain that refutes it.
#[must_use]
pub fn check(module: &HirModule, report: &PurityReport) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for func in &module.functions {
        let Some(pure_span) = func.pure_span else { continue };
        let name = &module.symbol_table.get(func.def_id).name;

        if func.is_async {
            diagnostics.push(
                Diagnostic::error(format!("`{name}` is both `pure` and `async`"), pure_span)
                    .with_code(DiagnosticCode::E0155)
                    .with_label(pure_span, "declared pure here")
                    .with_hint(format!("drop `pure` from `{name}`"))
                    .with_note("an `async fn` returns `Task(T)`, which is itself an effect"),
            );
            continue;
        }

        if report.purity_of(func.def_id) != Some(Purity::Effectful) {
            continue;
        }

        let path = report.effect_path(func.def_id);
        let mut diagnostic = Diagnostic::error(
            format!("`{name}` is declared `pure` but reaches an effect"),
            pure_span,
        )
        .with_code(DiagnosticCode::E0154)
        .with_label(pure_span, "declared pure here");

        for step in &path {
            diagnostic = diagnostic.with_secondary_label(step.origin.span, step.label());
        }

        diagnostic = diagnostic
            .with_hint(format!("drop `pure` from `{name}`, or move the effect into its caller"));

        if path.last().is_some_and(|step| !step.origin.source.is_proven()) {
            diagnostic = diagnostic.with_note(
                "a call the compiler cannot resolve counts as an effect, so that `pure` stays a proof",
            );
        }

        diagnostics.push(diagnostic);
    }

    diagnostics
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
        assert_eq!(shout.source(), Some(EffectSource::Boundary));
    }

    #[test]
    fn ffi_call_is_a_boundary_effect() {
        let report = report_for("from python import os\n\npub fn cwd() -> String { os.getcwd() }");
        assert_eq!(find(&report, "cwd").source(), Some(EffectSource::Boundary));
    }

    #[test]
    fn ffi_attribute_read_is_a_boundary_effect() {
        let report =
            report_for("from python import sys\n\npub fn platform() -> String { sys.platform }");
        assert_eq!(find(&report, "platform").source(), Some(EffectSource::Boundary));
    }

    #[test]
    fn try_marks_a_boundary_crossing() {
        let report =
            report_for("from python import os\n\npub fn cwd() { let c = try os.getcwd()\n  c }");
        assert_eq!(find(&report, "cwd").source(), Some(EffectSource::Boundary));
    }

    #[test]
    fn async_function_is_effectful() {
        let report = report_for("pub async fn fetch() -> Int { 1 }");
        assert_eq!(find(&report, "fetch").source(), Some(EffectSource::Async));
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
        assert_eq!(find(&report, "leaf").source(), Some(EffectSource::Boundary));
        assert_eq!(find(&report, "middle").source(), Some(EffectSource::Propagated));
        assert_eq!(find(&report, "top").source(), Some(EffectSource::Propagated));
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
    fn applying_a_parameter_is_higher_order_and_assumed_effectful() {
        let report = report_for("pub fn apply(f: Int, x: Int) -> Int { f(x) }");
        let apply = find(&report, "apply");
        assert_eq!(apply.purity, Purity::Effectful);
        assert_eq!(apply.source(), Some(EffectSource::Unresolved));
        assert_eq!(apply.unresolved.len(), 1);
        assert_eq!(apply.unresolved[0].kind, UnresolvedCallKind::ParameterApplied);
        assert_eq!(report.higher_order_count(), 1);
    }

    #[test]
    fn a_proven_effect_outranks_an_assumed_one() {
        // The unresolved call comes first in source order, but the boundary
        // crossing is the better explanation, so it wins.
        let report = report_for(
            r"from python import os

pub fn probe(f: Int) -> String {
  let ignored = f(1)
  os.getcwd()
}",
        );
        let probe = find(&report, "probe");
        assert_eq!(probe.source(), Some(EffectSource::Boundary));
        assert_eq!(probe.unresolved.len(), 1);
    }

    #[test]
    fn effect_path_reaches_the_boundary_through_the_call_graph() {
        let report = report_for(
            r#"
fn leaf() { println("x") }

fn middle() { leaf() }

pub fn top() { middle() }
"#,
        );
        let top = find(&report, "top");
        let path = report.effect_path(top.def_id);

        let names: Vec<&str> = path.iter().map(|step| step.name.as_str()).collect();
        assert_eq!(names, ["top", "middle", "leaf"]);
        assert_eq!(path[0].callee_name.as_deref(), Some("middle"));
        assert_eq!(path[2].origin.source, EffectSource::Boundary);
    }

    #[test]
    fn effect_path_terminates_on_recursive_cycles() {
        let report = report_for(
            r#"
fn ping(n: Int) -> Int {
  println("p")
  pong(n)
}

fn pong(n: Int) -> Int { ping(n) }
"#,
        );
        let pong = find(&report, "pong");
        assert!(!report.effect_path(pong.def_id).is_empty());
    }

    #[test]
    fn lambda_effects_are_attributed_to_the_enclosing_function() {
        let report = report_for(
            r#"pub fn build() { let f = fn(x: Int) { println("side") }
  f(1) }"#,
        );
        let build = find(&report, "build");
        assert_eq!(build.source(), Some(EffectSource::Boundary));
    }

    #[test]
    fn method_call_on_a_local_needs_type_information() {
        let report = report_for(
            "from python import pathlib\n\npub fn check(path: String) -> Bool {\n  let p = pathlib.Path(path)\n  p.exists()\n}",
        );
        let check = find(&report, "check");
        assert_eq!(check.source(), Some(EffectSource::Boundary));
        assert!(
            check.unresolved.iter().any(|c| c.kind == UnresolvedCallKind::ReceiverUntyped),
            "expected a receiver-untyped call site: {:?}",
            check.unresolved
        );
        assert_eq!(report.higher_order_count(), 0);
    }

    // ── `pure` declarations ─────────────────────────────────────────

    fn check_for(source: &str) -> Vec<Diagnostic> {
        let cst = parse(FileId(0), source);
        let ast = asatsuyu_ast::lower(&cst, FileId(0));
        let hir = lower_to_hir(&ast.module);
        let report = analyze(&hir.module);
        check(&hir.module, &report)
    }

    #[test]
    fn a_truthful_pure_declaration_is_accepted() {
        assert!(check_for("pub pure fn double(x: Int) -> Int { x * 2 }").is_empty());
    }

    #[test]
    fn a_pure_function_reaching_the_boundary_is_rejected() {
        let diagnostics = check_for(r#"pub pure fn shout() { println("hi") }"#);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, Some(DiagnosticCode::E0154));
    }

    #[test]
    fn the_diagnostic_names_every_hop_to_the_effect() {
        let diagnostics = check_for(
            r#"
fn leaf() { println("x") }

fn middle() { leaf() }

pub pure fn top() { middle() }
"#,
        );
        assert_eq!(diagnostics.len(), 1);
        // One label for the `pure` keyword, plus one per hop: top, middle, leaf.
        assert_eq!(diagnostics[0].labels.len(), 4);
    }

    #[test]
    fn pure_and_async_together_are_rejected() {
        let diagnostics = check_for("pub pure async fn fetch() -> Int { 1 }");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, Some(DiagnosticCode::E0155));
    }

    #[test]
    fn modifier_order_does_not_change_the_verdict() {
        let diagnostics = check_for("pub async pure fn fetch() -> Int { 1 }");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, Some(DiagnosticCode::E0155));
    }

    #[test]
    fn an_unresolvable_call_refutes_a_pure_declaration() {
        let diagnostics = check_for("pub pure fn apply(f: Int, x: Int) -> Int { f(x) }");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, Some(DiagnosticCode::E0154));
        assert!(
            diagnostics[0].notes.iter().any(|note| note.contains("cannot resolve")),
            "expected a note explaining the assumption: {:?}",
            diagnostics[0].notes
        );
    }

    #[test]
    fn an_undeclared_effectful_function_is_not_an_error() {
        assert!(check_for(r#"pub fn shout() { println("hi") }"#).is_empty());
    }
}
