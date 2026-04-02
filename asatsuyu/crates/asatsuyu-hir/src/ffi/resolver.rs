//! FFI module resolver framework.
//!
//! Implements a chain-of-responsibility pattern following PEP 561 lookup order:
//! 1. `py.typed` (inline type info)
//! 2. Stub-only packages (`*-stubs`)
//! 3. Typeshed (bundled)
//! 4. Builtin (hand-crafted, MVP only)

use std::path::PathBuf;

use super::admissibility;
use super::builtins;
use super::model::FfiModule;

// ── Configuration ─────────────────────────────────────────────────

/// Configuration for FFI module resolution.
///
/// Controls which modules are resolvable and where to search for stub files.
#[derive(Debug, Clone, Default)]
pub struct FfiResolverConfig {
    /// When `true`, only stdlib modules (`pathlib`, `json`, `os`, `sys`) are
    /// resolvable. Third-party modules (e.g. `requests`) are rejected.
    pub stdlib_only: bool,
    /// Additional directories to search for `.pyi` stub files.
    /// Reserved for future use — no stub-file resolver is implemented yet.
    pub stub_paths: Vec<PathBuf>,
}

/// Standard library modules that are always allowed when `stdlib_only` is set.
const STDLIB_MODULES: &[&str] = &["pathlib", "json", "os", "sys", "asyncio"];

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
    config: FfiResolverConfig,
}

/// Known module names for the builtin resolver.
const KNOWN_MODULES: &[&str] = &["pathlib", "json", "os", "sys", "requests", "asyncio"];

impl ChainResolver {
    /// Create a new `ChainResolver` with the default MVP configuration.
    ///
    /// Currently only includes the [`BuiltinResolver`].
    /// Future resolvers (typeshed, stub packages) will be prepended.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(FfiResolverConfig::default())
    }

    /// Create a `ChainResolver` with the given configuration.
    ///
    /// `config.stdlib_only` restricts resolution to stdlib modules only.
    /// `config.stub_paths` is reserved for future stub-file resolvers.
    #[must_use]
    pub fn with_config(config: FfiResolverConfig) -> Self {
        Self { resolvers: vec![Box::new(BuiltinResolver)], config }
    }

    /// Resolve all known builtin modules and return them with trust levels applied.
    ///
    /// Used by the `verify-ffi` CLI command to generate a trust report.
    #[must_use]
    pub fn verify_all(&self) -> Vec<FfiModule> {
        KNOWN_MODULES.iter().filter_map(|name| self.resolve(name)).collect()
    }
}

impl Default for ChainResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl FfiModuleResolver for ChainResolver {
    fn resolve(&self, module_name: &str) -> Option<FfiModule> {
        if self.config.stdlib_only && !STDLIB_MODULES.contains(&module_name) {
            return None;
        }
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
            "requests" => Some(builtins::requests_module()),
            "asyncio" => Some(builtins::asyncio_module()),
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

    #[test]
    fn builtin_resolves_requests() {
        let resolver = BuiltinResolver;
        let module = resolver.resolve("requests").expect("requests should resolve");
        assert_eq!(module.name.as_str(), "requests");
    }

    #[test]
    fn chain_resolver_requests_is_checked() {
        let chain = ChainResolver::new();
        let module = chain.resolve("requests").expect("should resolve");
        assert_eq!(module.trust_level, FfiTrustLevel::Checked);
        // get/post/put/delete should be Checked (return Response which has .json() -> Any)
        let get_sym = module.symbols.iter().find(|s| s.name == "get").unwrap();
        assert_eq!(get_sym.trust_level, Some(FfiTrustLevel::Checked));
    }

    #[test]
    fn verify_all_returns_known_modules() {
        let chain = ChainResolver::new();
        let modules = chain.verify_all();
        let names: Vec<&str> = modules.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"pathlib"), "should contain pathlib: {names:?}");
        assert!(names.contains(&"json"), "should contain json: {names:?}");
        assert!(names.contains(&"os"), "should contain os: {names:?}");
        assert!(names.contains(&"sys"), "should contain sys: {names:?}");
        assert!(names.contains(&"requests"), "should contain requests: {names:?}");
        assert!(names.contains(&"asyncio"), "should contain asyncio: {names:?}");
        assert_eq!(modules.len(), 6);
    }

    // ── FfiResolverConfig tests ───────────────────────────────────

    #[test]
    fn stdlib_only_blocks_requests() {
        let config = FfiResolverConfig { stdlib_only: true, ..Default::default() };
        let chain = ChainResolver::with_config(config);
        assert!(chain.resolve("requests").is_none(), "requests should be blocked");
    }

    #[test]
    fn stdlib_only_allows_pathlib() {
        let config = FfiResolverConfig { stdlib_only: true, ..Default::default() };
        let chain = ChainResolver::with_config(config);
        let module = chain.resolve("pathlib").expect("pathlib should resolve with stdlib_only");
        assert_eq!(module.name.as_str(), "pathlib");
    }

    #[test]
    fn stdlib_only_allows_all_stdlib() {
        let config = FfiResolverConfig { stdlib_only: true, ..Default::default() };
        let chain = ChainResolver::with_config(config);
        for name in &["pathlib", "json", "os", "sys"] {
            assert!(chain.resolve(name).is_some(), "{name} should resolve with stdlib_only");
        }
    }

    #[test]
    fn default_config_allows_all() {
        let chain = ChainResolver::with_config(FfiResolverConfig::default());
        assert!(chain.resolve("pathlib").is_some());
        assert!(chain.resolve("requests").is_some());
    }
}
