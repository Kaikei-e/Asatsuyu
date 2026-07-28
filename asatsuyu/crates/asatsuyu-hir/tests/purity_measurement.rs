//! Measurement of the purity analysis against the example corpus.
//!
//! The analysis treats an unresolvable call as an effect. These tests pin what
//! that costs for the programs that exist today, so a future change to the
//! analysis — or a new example that trips it — shows up as a diff.

use std::fmt::Write as _;

use asatsuyu_hir::lower_to_hir;
use asatsuyu_hir::purity::{EffectSource, PurityReport, UnresolvedCallKind, analyze};
use asatsuyu_parser::parse;
use asatsuyu_syntax::FileId;

const EXAMPLES: &[(&str, &str)] = &[
    ("hello", include_str!("../../../examples/hello.asty")),
    ("greet", include_str!("../../../examples/greet.asty")),
    ("match_basic", include_str!("../../../examples/match_basic.asty")),
    ("ffi_pathlib", include_str!("../../../examples/ffi_pathlib.asty")),
    ("ffi_json", include_str!("../../../examples/ffi_json.asty")),
    ("ffi_try", include_str!("../../../examples/ffi_try.asty")),
    ("http", include_str!("../../../examples/http.asty")),
    ("tutorial_file_inventory", include_str!("../../../examples/tutorial_file_inventory.asty")),
    ("purity", include_str!("../../../examples/purity.asty")),
];

fn report_for(source: &str) -> PurityReport {
    let cst = parse(FileId(0), source);
    let ast = asatsuyu_ast::lower(&cst, FileId(0));
    let hir = lower_to_hir(&ast.module);
    analyze(&hir.module)
}

/// Renders a report deterministically, for snapshot review.
fn render(name: &str, report: &PurityReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "== {name} ==");
    for func in &report.functions {
        let _ = write!(out, "  {:<24} {:?}", func.name.as_str(), func.purity);
        if let Some(source) = func.source() {
            let _ = write!(out, " ({source:?})");
        }
        if !func.unresolved.is_empty() {
            let mut kinds: Vec<String> =
                func.unresolved.iter().map(|c| format!("{:?}", c.kind)).collect();
            kinds.sort_unstable();
            let _ = write!(out, "  unresolved: [{}]", kinds.join(", "));
        }
        let _ = writeln!(out);
    }
    let _ = writeln!(
        out,
        "  -- pure {} / effectful {} / higher-order call sites {}",
        report.pure_count(),
        report.effectful_count(),
        report.higher_order_count()
    );
    out
}

#[test]
fn corpus_purity_snapshot() {
    let mut out = String::new();
    for (name, source) in EXAMPLES {
        out.push_str(&render(name, &report_for(source)));
    }
    insta::assert_snapshot!("corpus_purity", out);
}

/// What conservatism costs: a function reported effectful only because a call
/// could not be resolved is one the analysis may be wrong about. Zero of them
/// means treating unresolvable calls as effects loses no precision here.
#[test]
fn every_effectful_function_reaches_a_proven_effect() {
    let mut assumed = Vec::new();
    for (name, source) in EXAMPLES {
        let report = report_for(source);
        for func in &report.functions {
            let path = report.effect_path(func.def_id);
            if path.last().is_some_and(|step| !step.origin.source.is_proven()) {
                assumed.push(format!("{name}::{}", func.name));
            }
        }
    }

    assert!(assumed.is_empty(), "these functions are effectful only by assumption: {assumed:?}");
}

/// The effect chain a diagnostic prints must terminate. Every effectful
/// function in the corpus resolves to a boundary crossing or to `async`.
#[test]
fn every_effect_path_terminates_at_a_boundary_or_async() {
    for (name, source) in EXAMPLES {
        let report = report_for(source);
        for func in &report.functions {
            let path = report.effect_path(func.def_id);
            let Some(last) = path.last() else { continue };
            assert!(
                matches!(last.origin.source, EffectSource::Boundary | EffectSource::Async),
                "{name}::{} ends its effect path at {:?}",
                func.name,
                last.origin.source
            );
        }
    }
}

/// Higher-order use in the corpus goes through built-in list combinators with
/// literal lambdas, which the analysis resolves by walking the lambda body.
/// Nothing applies an opaque function-typed parameter.
#[test]
fn corpus_has_no_opaque_function_application() {
    for (name, source) in EXAMPLES {
        let report = report_for(source);
        let offenders: Vec<_> = report
            .functions
            .iter()
            .flat_map(|f| f.unresolved.iter().map(move |c| (f.name.as_str(), c.kind)))
            .filter(|(_, kind)| kind.is_higher_order())
            .collect();

        assert!(offenders.is_empty(), "{name} applies opaque functions: {offenders:?}");
    }
}

/// Unresolved call sites that remain are receiver-typed method calls and
/// cross-module members — both resolvable with information the compiler
/// already computes, unlike higher-order application.
#[test]
fn remaining_unresolved_calls_are_resolvable_with_more_context() {
    for (name, source) in EXAMPLES {
        let report = report_for(source);
        for func in &report.functions {
            for call in &func.unresolved {
                assert!(
                    matches!(
                        call.kind,
                        UnresolvedCallKind::ReceiverUntyped
                            | UnresolvedCallKind::ModuleMember
                            | UnresolvedCallKind::ComputedCallee
                    ),
                    "{name}::{} has an unexpected unresolved call: {:?}",
                    func.name,
                    call.kind
                );
            }
        }
    }
}
