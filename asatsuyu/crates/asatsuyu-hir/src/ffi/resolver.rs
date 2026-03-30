//! FFI module resolver framework.
//!
//! Implements a chain-of-responsibility pattern following PEP 561 lookup order:
//! 1. `py.typed` (inline type info)
//! 2. Stub-only packages (`*-stubs`)
//! 3. Typeshed (bundled)
//! 4. Builtin (hand-crafted, MVP only)

use super::admissibility;
use super::builtins;
use super::model::FfiModule;

/// Trait for resolving Python module type information.
///
/// Implementors look up a module by name and return its FFI model,
/// or `None` if the module is not known to this resolver.
pub trait FfiModuleResolver {
    /// Attempt to resolve type information for the given Python module.
    fn resolve(&self, module_name: &str) -> Option<FfiModule>;
}

// ── ChainResolver ──────────────────────────────────────────────────

/// Composite resolver that tries each inner resolver in PEP 561 priority order.
///
/// The first resolver to return `Some` wins. This ensures that inline type
/// info (`py.typed`) takes precedence over stub packages, which take
/// precedence over typeshed, which takes precedence over builtins.
pub struct ChainResolver {
    resolvers: Vec<Box<dyn FfiModuleResolver>>,
}

impl ChainResolver {
    /// Create a new `ChainResolver` with the default MVP configuration.
    ///
    /// Currently only includes the [`BuiltinResolver`].
    /// Future resolvers (typeshed, stub packages) will be prepended.
    #[must_use]
    pub fn new() -> Self {
        Self { resolvers: vec![Box::new(BuiltinResolver)] }
    }
}

impl Default for ChainResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl FfiModuleResolver for ChainResolver {
    fn resolve(&self, module_name: &str) -> Option<FfiModule> {
        let mut module = self.resolvers.iter().find_map(|r| r.resolve(module_name))?;
        let report = admissibility::check_module(&module);
        module.trust_level = report.module_trust;
        for (sym, adm) in module.symbols.iter_mut().zip(&report.symbols) {
            sym.trust_level = Some(adm.trust_level);
        }
        Some(module)
    }
}

// ── BuiltinResolver ────────────────────────────────────────────────

/// Hand-crafted FFI signatures for Phase 1 stdlib modules.
///
/// Covers: `pathlib`, `json`, `os`, `sys`.
pub struct BuiltinResolver;

impl FfiModuleResolver for BuiltinResolver {
    fn resolve(&self, module_name: &str) -> Option<FfiModule> {
        match module_name {
            "pathlib" => Some(builtins::pathlib_module()),
            "json" => Some(builtins::json_module()),
            "os" => Some(builtins::os_module()),
            "sys" => Some(builtins::sys_module()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::model::{FfiSource, FfiTrustLevel};

    #[test]
    fn builtin_resolves_pathlib() {
        let resolver = BuiltinResolver;
        let module = resolver.resolve("pathlib").expect("pathlib should resolve");
        assert_eq!(module.name.as_str(), "pathlib");
        assert_eq!(module.source, FfiSource::Builtin);
        assert_eq!(module.trust_level, FfiTrustLevel::Verified);
        assert!(!module.symbols.is_empty());
    }

    #[test]
    fn builtin_resolves_json() {
        let resolver = BuiltinResolver;
        let module = resolver.resolve("json").expect("json should resolve");
        assert_eq!(module.name.as_str(), "json");
    }

    #[test]
    fn builtin_resolves_os() {
        let resolver = BuiltinResolver;
        let module = resolver.resolve("os").expect("os should resolve");
        assert_eq!(module.name.as_str(), "os");
    }

    #[test]
    fn builtin_resolves_sys() {
        let resolver = BuiltinResolver;
        let module = resolver.resolve("sys").expect("sys should resolve");
        assert_eq!(module.name.as_str(), "sys");
    }

    #[test]
    fn builtin_returns_none_for_unknown() {
        let resolver = BuiltinResolver;
        assert!(resolver.resolve("numpy").is_none());
    }

    #[test]
    fn chain_resolver_delegates() {
        let chain = ChainResolver::new();
        let module = chain.resolve("pathlib").expect("chain should resolve pathlib");
        assert_eq!(module.name.as_str(), "pathlib");
        assert!(chain.resolve("unknown_module").is_none());
    }

    #[test]
    fn chain_resolver_tracks_source() {
        let chain = ChainResolver::new();
        let module = chain.resolve("pathlib").unwrap();
        assert_eq!(module.source, FfiSource::Builtin);
    }

    #[test]
    fn chain_resolver_applies_admissibility_json() {
        let chain = ChainResolver::new();
        let module = chain.resolve("json").expect("json should resolve");
        // json has Any in loads/dumps, so module trust should be Checked.
        assert_eq!(module.trust_level, FfiTrustLevel::Checked);
        for sym in &module.symbols {
            assert!(sym.trust_level.is_some(), "trust_level should be set after resolve");
        }
    }

    #[test]
    fn chain_resolver_pathlib_stays_verified() {
        let chain = ChainResolver::new();
        let module = chain.resolve("pathlib").expect("pathlib should resolve");
        assert_eq!(module.trust_level, FfiTrustLevel::Verified);
        for sym in &module.symbols {
            assert_eq!(sym.trust_level, Some(FfiTrustLevel::Verified));
        }
    }
}
