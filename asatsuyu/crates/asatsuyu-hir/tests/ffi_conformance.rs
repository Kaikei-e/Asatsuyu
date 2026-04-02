//! FFI conformance tests — CI gate for Verified/Checked trust levels.
//!
//! These tests ensure that:
//! 1. The FFI surface is snapshot-tracked (any change requires explicit review)
//! 2. Trust level invariants are maintained (Verified modules stay Verified)
//! 3. Symbol counts don't silently regress
//! 4. Known `Any`-bearing symbols are explicitly tracked

use std::fmt::Write;

use asatsuyu_hir::ffi::{
    ChainResolver, FfiModule, FfiModuleResolver, FfiSymbolKind, FfiTrustLevel,
};

// ── Helpers ──────────────────────────────────────────────────────────

/// Format an `FfiModule` into a deterministic, human-readable string for snapshots.
fn format_module(module: &FfiModule) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "module: {}", module.name);
    let _ = writeln!(out, "source: {:?}", module.source);
    let _ = writeln!(out, "trust:  {:?}", module.trust_level);
    let _ = writeln!(out, "symbols ({}):", module.symbols.len());

    for sym in &module.symbols {
        let trust = sym.trust_level.map_or("?".to_string(), |t| format!("{t:?}"));
        match &sym.kind {
            FfiSymbolKind::Function(sig) => {
                let params: Vec<String> =
                    sig.params.iter().map(|p| format!("{}: {:?}", p.name, p.ty)).collect();
                let _ = writeln!(
                    out,
                    "  fn {}({}) -> {:?}  [{}]",
                    sym.name,
                    params.join(", "),
                    sig.return_ty,
                    trust,
                );
            }
            FfiSymbolKind::Class(cls) => {
                let _ = writeln!(out, "  class {}  [{}]", sym.name, trust);
                if let Some(ctor) = &cls.constructor {
                    let params: Vec<String> =
                        ctor.params.iter().map(|p| format!("{}: {:?}", p.name, p.ty)).collect();
                    let _ = writeln!(out, "    __init__({})", params.join(", "));
                }
                for (name, sig) in &cls.methods {
                    let _ = writeln!(out, "    method {}() -> {:?}", name, sig.return_ty);
                }
                for (name, ty) in &cls.properties {
                    let _ = writeln!(out, "    prop {name}: {ty:?}");
                }
            }
            FfiSymbolKind::Constant(ty) => {
                let _ = writeln!(out, "  const {}: {:?}  [{}]", sym.name, ty, trust);
            }
        }
    }
    out
}

/// Format all modules from `verify_all()` into a single snapshot string.
fn format_all_modules(modules: &[FfiModule]) -> String {
    let mut out = String::new();
    for (i, module) in modules.iter().enumerate() {
        if i > 0 {
            out.push_str("---\n");
        }
        out.push_str(&format_module(module));
    }
    out
}

// ── Snapshot: complete FFI surface ──────────────────────────────────

#[test]
fn ffi_surface_snapshot() {
    let chain = ChainResolver::new();
    let modules = chain.verify_all();
    let formatted = format_all_modules(&modules);
    insta::assert_snapshot!("ffi_surface", formatted);
}

// ── Trust level invariants ──────────────────────────────────────────

/// Verified stdlib modules must stay Verified.
#[test]
fn verified_modules_stay_verified() {
    let chain = ChainResolver::new();
    for name in &["pathlib", "os", "sys"] {
        let module = chain.resolve(name).unwrap_or_else(|| panic!("{name} should resolve"));
        assert_eq!(
            module.trust_level,
            FfiTrustLevel::Verified,
            "{name} must be Verified, got {:?}",
            module.trust_level,
        );
        // Every symbol in a Verified module must also be Verified.
        for sym in &module.symbols {
            assert_eq!(
                sym.trust_level,
                Some(FfiTrustLevel::Verified),
                "{name}.{} must be Verified",
                sym.name,
            );
        }
    }
}

/// json module must be Checked (loads returns Any, dumps accepts Any).
#[test]
fn json_is_checked() {
    let chain = ChainResolver::new();
    let module = chain.resolve("json").expect("json should resolve");
    assert_eq!(module.trust_level, FfiTrustLevel::Checked);
}

/// requests module must be Checked (`Response.json()` returns Any).
#[test]
fn requests_is_checked() {
    let chain = ChainResolver::new();
    let module = chain.resolve("requests").expect("requests should resolve");
    assert_eq!(module.trust_level, FfiTrustLevel::Checked);
}

// ── Symbol count regression guards ──────────────────────────────────

#[test]
fn pathlib_symbol_count() {
    let chain = ChainResolver::new();
    let module = chain.resolve("pathlib").unwrap();
    // pathlib exposes 1 symbol: the Path class (with methods/properties inside)
    assert_eq!(module.symbols.len(), 1, "pathlib symbol count changed");
    // Path class method + property count
    if let FfiSymbolKind::Class(cls) = &module.symbols[0].kind {
        assert!(
            cls.methods.len() >= 7,
            "pathlib.Path should have >=7 methods, got {}",
            cls.methods.len()
        );
        assert!(
            cls.properties.len() >= 5,
            "pathlib.Path should have >=5 properties, got {}",
            cls.properties.len()
        );
    } else {
        panic!("pathlib.Path should be a Class");
    }
}

#[test]
fn os_symbol_count() {
    let chain = ChainResolver::new();
    let module = chain.resolve("os").unwrap();
    assert!(module.symbols.len() >= 5, "os should have >=5 symbols, got {}", module.symbols.len());
}

#[test]
fn sys_symbol_count() {
    let chain = ChainResolver::new();
    let module = chain.resolve("sys").unwrap();
    assert!(module.symbols.len() >= 4, "sys should have >=4 symbols, got {}", module.symbols.len());
}

#[test]
fn json_symbol_count() {
    let chain = ChainResolver::new();
    let module = chain.resolve("json").unwrap();
    assert!(
        module.symbols.len() >= 2,
        "json should have >=2 symbols, got {}",
        module.symbols.len()
    );
}

#[test]
fn requests_symbol_count() {
    let chain = ChainResolver::new();
    let module = chain.resolve("requests").unwrap();
    assert!(
        module.symbols.len() >= 5,
        "requests should have >=5 symbols, got {}",
        module.symbols.len()
    );
}

// ── Known Any-bearing symbols ───────────────────────────────────────

/// Track exactly which symbols contain Any (the Checked surface).
/// If this list changes, it means the FFI boundary has shifted.
#[test]
fn known_any_bearing_symbols() {
    let chain = ChainResolver::new();

    // json: loads (return Any), dumps (param Any)
    let json = chain.resolve("json").unwrap();
    for sym in &json.symbols {
        assert_eq!(
            sym.trust_level,
            Some(FfiTrustLevel::Checked),
            "json.{} should be Checked",
            sym.name,
        );
    }

    // requests: all symbols are Checked (Response.json() -> Any propagates)
    let requests = chain.resolve("requests").unwrap();
    for sym in &requests.symbols {
        assert_eq!(
            sym.trust_level,
            Some(FfiTrustLevel::Checked),
            "requests.{} should be Checked",
            sym.name,
        );
    }
}

/// No Any should appear in Verified module type surfaces.
#[test]
fn verified_modules_have_no_any() {
    let chain = ChainResolver::new();
    for name in &["pathlib", "os", "sys"] {
        let module = chain.resolve(name).unwrap();
        for sym in &module.symbols {
            match &sym.kind {
                FfiSymbolKind::Function(sig) => {
                    assert!(
                        !sig.return_ty.contains_any(),
                        "{name}.{} return type contains Any",
                        sym.name,
                    );
                    for p in &sig.params {
                        assert!(
                            !p.ty.contains_any(),
                            "{name}.{} param {} contains Any",
                            sym.name,
                            p.name,
                        );
                    }
                }
                FfiSymbolKind::Class(cls) => {
                    for (mname, sig) in &cls.methods {
                        assert!(
                            !sig.return_ty.contains_any(),
                            "{name}.{}.{mname} return type contains Any",
                            sym.name,
                        );
                    }
                    for (pname, ty) in &cls.properties {
                        assert!(
                            !ty.contains_any(),
                            "{name}.{}.{pname} property contains Any",
                            sym.name,
                        );
                    }
                }
                FfiSymbolKind::Constant(ty) => {
                    assert!(!ty.contains_any(), "{name}.{} constant contains Any", sym.name,);
                }
            }
        }
    }
}

// ── All known modules resolve ───────────────────────────────────────

#[test]
fn all_known_modules_resolve() {
    let chain = ChainResolver::new();
    let modules = chain.verify_all();
    let names: Vec<&str> = modules.iter().map(|m| m.name.as_str()).collect();
    assert!(names.contains(&"pathlib"), "pathlib missing");
    assert!(names.contains(&"json"), "json missing");
    assert!(names.contains(&"os"), "os missing");
    assert!(names.contains(&"sys"), "sys missing");
    assert!(names.contains(&"requests"), "requests missing");
    assert!(names.contains(&"asyncio"), "asyncio missing");
    assert_eq!(modules.len(), 6, "expected 6 known modules");
}
