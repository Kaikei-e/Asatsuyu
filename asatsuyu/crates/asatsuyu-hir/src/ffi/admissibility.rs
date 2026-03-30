//! FFI admissibility checker.
//!
//! Determines the trust level of each symbol in an [`FfiModule`] by inspecting
//! its type surface. A symbol is `Verified` only if its entire surface is free
//! of `Any`, bare generics, and partial-stub unknowns. Otherwise it is
//! downgraded to `Checked` (type info present but unsound) or `Unsafe`
//! (no type information at all).
//!
//! See the language charter (principles.md §4.1) for the full model.

use super::model::{
    AdmissibilityReason, AdmissibilityReport, FfiClass, FfiModule, FfiSignature, FfiSymbol,
    FfiSymbolKind, FfiTrustLevel, FfiType, SymbolAdmissibility,
};

// ── Type surface inspection ───────────────────────────────────────

/// Returns `true` if any parameter or the return type contains `Any`.
fn signature_contains_any(sig: &FfiSignature) -> bool {
    sig.params.iter().any(|p| p.ty.contains_any()) || sig.return_ty.contains_any()
}

/// Returns `true` if any part of a class surface contains `Any`.
fn class_contains_any(cls: &FfiClass) -> bool {
    cls.constructor.as_ref().is_some_and(signature_contains_any)
        || cls.methods.iter().any(|(_, sig)| signature_contains_any(sig))
        || cls.properties.iter().any(|(_, ty)| ty.contains_any())
}

// TODO: Implement when non-builtin resolvers produce bare generics.
// fn type_contains_bare_generic(ty: &FfiType) -> bool { false }

// ── Symbol-level check ────────────────────────────────────────────

/// Determine the trust level of a single FFI symbol.
///
/// `module_symbols` provides context so that `Named` return types can be
/// traced back to their class definition within the same module. A function
/// returning a class whose surface contains `Any` is downgraded to `Checked`.
fn check_symbol(symbol: &FfiSymbol, module_symbols: &[FfiSymbol]) -> SymbolAdmissibility {
    let has_any = match &symbol.kind {
        FfiSymbolKind::Function(sig) => {
            signature_contains_any(sig)
                || return_refers_to_any_bearing_class(&sig.return_ty, module_symbols)
        }
        FfiSymbolKind::Class(cls) => class_contains_any(cls),
        FfiSymbolKind::Constant(ty) => ty.contains_any(),
    };

    let (trust_level, reason) = if has_any {
        (FfiTrustLevel::Checked, AdmissibilityReason::ContainsAny)
    } else {
        (FfiTrustLevel::Verified, AdmissibilityReason::FullyTyped)
    };

    SymbolAdmissibility { name: symbol.name.clone(), trust_level, reason }
}

/// Returns `true` if the return type is a `Named` type whose class definition
/// (within the same module) contains `Any` in its surface.
fn return_refers_to_any_bearing_class(return_ty: &FfiType, symbols: &[FfiSymbol]) -> bool {
    let FfiType::Named { name, .. } = return_ty else {
        return false;
    };
    symbols.iter().any(|s| {
        s.name == *name && matches!(&s.kind, FfiSymbolKind::Class(cls) if class_contains_any(cls))
    })
}

// ── Module-level check ────────────────────────────────────────────

/// Check the admissibility of all symbols in an FFI module.
///
/// Returns a report with per-symbol trust levels and a module-level trust
/// that is the minimum across all symbols.
#[must_use]
pub fn check_module(module: &FfiModule) -> AdmissibilityReport {
    // If the module itself is declared Unsafe, all symbols inherit Unsafe
    // regardless of their type surface. This supports opaque isolation of
    // entirely untyped or dynamic Python surfaces.
    let force_unsafe = module.trust_level == FfiTrustLevel::Unsafe;

    let symbols: Vec<SymbolAdmissibility> = if force_unsafe {
        module
            .symbols
            .iter()
            .map(|s| SymbolAdmissibility {
                name: s.name.clone(),
                trust_level: FfiTrustLevel::Unsafe,
                reason: AdmissibilityReason::Untyped,
            })
            .collect()
    } else {
        module.symbols.iter().map(|s| check_symbol(s, &module.symbols)).collect()
    };

    let module_trust =
        symbols.iter().map(|s| s.trust_level).min().unwrap_or(FfiTrustLevel::Verified);

    AdmissibilityReport { module_trust, symbols }
}

#[cfg(test)]
mod tests {
    use smol_str::SmolStr;

    use super::*;
    use crate::ffi::builtins;
    use crate::ffi::model::{FfiParam, FfiSignature, FfiSymbol, FfiSymbolKind};

    // ── Real module tests ─────────────────────────────────────────

    #[test]
    fn pathlib_is_fully_verified() {
        let module = builtins::pathlib_module();
        let report = check_module(&module);
        assert_eq!(report.module_trust, FfiTrustLevel::Verified);
        for sym in &report.symbols {
            assert_eq!(sym.trust_level, FfiTrustLevel::Verified);
            assert_eq!(sym.reason, AdmissibilityReason::FullyTyped);
        }
    }

    #[test]
    fn json_loads_is_checked() {
        let module = builtins::json_module();
        let report = check_module(&module);
        let loads = report.symbols.iter().find(|s| s.name == "loads").unwrap();
        assert_eq!(loads.trust_level, FfiTrustLevel::Checked);
        assert_eq!(loads.reason, AdmissibilityReason::ContainsAny);
    }

    #[test]
    fn json_dumps_is_checked() {
        let module = builtins::json_module();
        let report = check_module(&module);
        let dumps = report.symbols.iter().find(|s| s.name == "dumps").unwrap();
        assert_eq!(dumps.trust_level, FfiTrustLevel::Checked);
        assert_eq!(dumps.reason, AdmissibilityReason::ContainsAny);
    }

    #[test]
    fn json_module_trust_is_checked() {
        let module = builtins::json_module();
        let report = check_module(&module);
        assert_eq!(report.module_trust, FfiTrustLevel::Checked);
    }

    #[test]
    fn os_is_fully_verified() {
        let module = builtins::os_module();
        let report = check_module(&module);
        assert_eq!(report.module_trust, FfiTrustLevel::Verified);
        for sym in &report.symbols {
            assert_eq!(sym.trust_level, FfiTrustLevel::Verified);
        }
    }

    #[test]
    fn sys_is_fully_verified() {
        let module = builtins::sys_module();
        let report = check_module(&module);
        assert_eq!(report.module_trust, FfiTrustLevel::Verified);
        for sym in &report.symbols {
            assert_eq!(sym.trust_level, FfiTrustLevel::Verified);
        }
    }

    // ── Synthetic type tests ──────────────────────────────────────

    fn make_func_symbol(name: &str, params: Vec<FfiType>, ret: FfiType) -> FfiSymbol {
        FfiSymbol {
            name: SmolStr::from(name),
            kind: FfiSymbolKind::Function(FfiSignature {
                params: params
                    .into_iter()
                    .map(|ty| FfiParam { name: SmolStr::from("x"), ty, has_default: false })
                    .collect(),
                return_ty: ret,
            }),
            trust_level: None,
        }
    }

    fn make_const_symbol(name: &str, ty: FfiType) -> FfiSymbol {
        FfiSymbol {
            name: SmolStr::from(name),
            kind: FfiSymbolKind::Constant(ty),
            trust_level: None,
        }
    }

    #[test]
    fn any_in_nested_type_detected() {
        // List(Any)
        assert!(FfiType::List(Box::new(FfiType::Any)).contains_any());
        // Optional(Any)
        assert!(FfiType::Optional(Box::new(FfiType::Any)).contains_any());
        // Dict(Str, Any)
        assert!(FfiType::Dict(Box::new(FfiType::Str), Box::new(FfiType::Any)).contains_any());
        // Tuple with Any
        assert!(FfiType::Tuple(vec![FfiType::Int, FfiType::Any]).contains_any());
        // Union with Any
        assert!(FfiType::Union(vec![FfiType::Str, FfiType::Any]).contains_any());
        // Clean types
        assert!(!FfiType::List(Box::new(FfiType::Int)).contains_any());
        assert!(!FfiType::Optional(Box::new(FfiType::Str)).contains_any());
    }

    #[test]
    fn any_in_class_method_detected() {
        let cls = FfiClass {
            name: SmolStr::from("Foo"),
            constructor: None,
            methods: vec![(
                SmolStr::from("bar"),
                FfiSignature { params: vec![], return_ty: FfiType::Any },
            )],
            properties: vec![],
        };
        assert!(class_contains_any(&cls));
    }

    #[test]
    fn any_in_class_property_detected() {
        let cls = FfiClass {
            name: SmolStr::from("Foo"),
            constructor: None,
            methods: vec![],
            properties: vec![(SmolStr::from("value"), FfiType::Any)],
        };
        assert!(class_contains_any(&cls));
    }

    #[test]
    fn any_in_constructor_detected() {
        let cls = FfiClass {
            name: SmolStr::from("Foo"),
            constructor: Some(FfiSignature {
                params: vec![FfiParam {
                    name: SmolStr::from("x"),
                    ty: FfiType::Any,
                    has_default: false,
                }],
                return_ty: FfiType::Named {
                    module: SmolStr::from("test"),
                    name: SmolStr::from("Foo"),
                },
            }),
            methods: vec![],
            properties: vec![],
        };
        assert!(class_contains_any(&cls));
    }

    #[test]
    fn trust_level_ordering() {
        assert!(FfiTrustLevel::Unsafe < FfiTrustLevel::Checked);
        assert!(FfiTrustLevel::Checked < FfiTrustLevel::Verified);
        // min() gives the least trusted
        let levels = vec![FfiTrustLevel::Verified, FfiTrustLevel::Checked];
        assert_eq!(levels.into_iter().min(), Some(FfiTrustLevel::Checked));
    }

    #[test]
    fn mixed_module_trust_is_minimum() {
        let module = FfiModule {
            name: SmolStr::from("mixed"),
            source: crate::ffi::model::FfiSource::Builtin,
            trust_level: FfiTrustLevel::Verified, // will be overridden
            symbols: vec![
                make_func_symbol("safe_fn", vec![FfiType::Int], FfiType::Str),
                make_func_symbol("unsafe_fn", vec![FfiType::Any], FfiType::Str),
                make_const_symbol("clean", FfiType::Int),
            ],
        };
        let report = check_module(&module);
        assert_eq!(report.module_trust, FfiTrustLevel::Checked);
        assert_eq!(report.symbols[0].trust_level, FfiTrustLevel::Verified);
        assert_eq!(report.symbols[1].trust_level, FfiTrustLevel::Checked);
        assert_eq!(report.symbols[2].trust_level, FfiTrustLevel::Verified);
    }

    // ── requests module tests ────────────────────────────────────────

    #[test]
    fn requests_get_is_checked_due_to_response_class() {
        let module = builtins::requests_module();
        let report = check_module(&module);
        assert_eq!(report.module_trust, FfiTrustLevel::Checked);
        let get_sym = report.symbols.iter().find(|s| s.name == "get").unwrap();
        assert_eq!(get_sym.trust_level, FfiTrustLevel::Checked);
        assert_eq!(get_sym.reason, AdmissibilityReason::ContainsAny);
    }

    #[test]
    fn requests_response_class_is_checked() {
        let module = builtins::requests_module();
        let report = check_module(&module);
        let resp = report.symbols.iter().find(|s| s.name == "Response").unwrap();
        assert_eq!(resp.trust_level, FfiTrustLevel::Checked);
    }

    #[test]
    fn pathlib_stays_verified_with_named_return() {
        // Regression: Named return types whose class has no Any must stay Verified.
        let module = builtins::pathlib_module();
        let report = check_module(&module);
        assert_eq!(report.module_trust, FfiTrustLevel::Verified);
        for sym in &report.symbols {
            assert_eq!(sym.trust_level, FfiTrustLevel::Verified);
        }
    }

    // ── Issue 47: Unsafe / Opaque ────────────────────────────────────

    #[test]
    fn unsafe_module_forces_all_symbols_unsafe() {
        let module = FfiModule {
            name: SmolStr::from("dynamic"),
            source: crate::ffi::model::FfiSource::Builtin,
            trust_level: FfiTrustLevel::Unsafe,
            symbols: vec![
                // Even a fully-typed symbol becomes Unsafe when the module is Unsafe.
                make_func_symbol("do_stuff", vec![FfiType::Str], FfiType::Str),
                make_const_symbol("version", FfiType::Str),
            ],
        };
        let report = check_module(&module);
        assert_eq!(report.module_trust, FfiTrustLevel::Unsafe);
        for sym in &report.symbols {
            assert_eq!(sym.trust_level, FfiTrustLevel::Unsafe);
            assert_eq!(sym.reason, AdmissibilityReason::Untyped);
        }
    }

    #[test]
    fn non_unsafe_module_is_not_forced() {
        // Verified module with clean symbols should remain Verified.
        let module = FfiModule {
            name: SmolStr::from("clean"),
            source: crate::ffi::model::FfiSource::Builtin,
            trust_level: FfiTrustLevel::Verified,
            symbols: vec![make_func_symbol("f", vec![FfiType::Str], FfiType::Int)],
        };
        let report = check_module(&module);
        assert_eq!(report.module_trust, FfiTrustLevel::Verified);
        assert_eq!(report.symbols[0].trust_level, FfiTrustLevel::Verified);
    }
}
