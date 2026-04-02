//! FFI module resolver framework.
//!
//! Implements a chain-of-responsibility pattern following PEP 561 lookup order:
//! 1. Custom stub paths / site-packages / bundled typeshed (within `TypeshedResolver`)
//! 2. Builtin (hand-crafted, MVP only)

use std::path::PathBuf;

use std::cell::RefCell;
use std::process::Command;

use super::admissibility;
use super::builtins;
use super::cache::PythonApiIndexCache;
use super::index::PythonModuleInfo;
use super::model::{FfiModule, FfiTrustLevel};
use super::source::TypeSourceResolver;

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
    /// These take highest priority in the resolution chain (custom stubs).
    pub stub_paths: Vec<PathBuf>,
    /// Override path for typeshed stubs. When `None`, uses bundled typeshed.
    pub typeshed_path: Option<PathBuf>,
    /// Path to the Python interpreter, used to locate site-packages for
    /// stub package and `py.typed` package discovery.
    pub python_path: Option<PathBuf>,
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

    /// Downcast to [`TypeshedResolver`] for rich index access.
    /// Returns `None` for resolvers that don't provide `PythonModuleInfo`.
    fn as_typeshed_resolver(&self) -> Option<&TypeshedResolver> {
        None
    }
}

// ── ChainResolver ──────────────────────────────────────────────────

/// Composite resolver that tries each inner resolver in priority order.
///
/// The first resolver to return `Some` wins. Within [`TypeshedResolver`], the
/// PEP 561 order is custom stubs → typeshed → stub packages → `py.typed`.
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
    /// Resolution order:
    /// 1. `TypeshedResolver` — parses `.pyi` stubs from typeshed / custom paths
    /// 2. `BuiltinResolver` — hand-crafted fallback for modules not in typeshed
    #[must_use]
    pub fn with_config(config: FfiResolverConfig) -> Self {
        let resolvers: Vec<Box<dyn FfiModuleResolver>> =
            vec![Box::new(TypeshedResolver::new(&config)), Box::new(BuiltinResolver)];
        Self { resolvers, config }
    }

    /// Resolve all known builtin modules and return them with trust levels applied.
    ///
    /// Used by the `verify-ffi` CLI command to generate a trust report.
    #[must_use]
    pub fn verify_all(&self) -> Vec<FfiModule> {
        KNOWN_MODULES.iter().filter_map(|name| self.resolve(name)).collect()
    }

    /// Return the list of known module names available for completion.
    #[must_use]
    pub fn known_module_names(&self) -> &'static [&'static str] {
        KNOWN_MODULES
    }

    /// Resolve a module and also return its rich [`PythonModuleInfo`] for LSP
    /// completion, alongside the [`FfiModule`] used by the type checker.
    ///
    /// The `PythonModuleInfo` carries overload signatures, method classification,
    /// and property info that `FfiModule` flattens away.
    #[must_use]
    pub fn resolve_with_index(
        &self,
        module_name: &str,
    ) -> Option<(FfiModule, Option<PythonModuleInfo>)> {
        if self.config.stdlib_only && !STDLIB_MODULES.contains(&module_name) {
            return None;
        }

        // Try typeshed first for the rich index.
        let mut index_info: Option<PythonModuleInfo> = None;
        let mut ffi_module: Option<FfiModule> = None;

        for resolver in &self.resolvers {
            if let Some(module) = resolver.resolve(module_name) {
                ffi_module = Some(module);
                break;
            }
        }

        // Get the rich PythonModuleInfo from the TypeshedResolver.
        for resolver in &self.resolvers {
            if let Some(tsr) = (**resolver).as_typeshed_resolver() {
                index_info = tsr.resolve_index(module_name);
                break;
            }
        }

        let mut module = ffi_module?;
        let report = admissibility::check_module(&module);
        module.trust_level = std::cmp::min(module.trust_level, report.module_trust);
        for (sym, adm) in module.symbols.iter_mut().zip(&report.symbols) {
            let current = sym.trust_level.unwrap_or(FfiTrustLevel::Verified);
            sym.trust_level = Some(std::cmp::min(current, adm.trust_level));
        }
        Some((module, index_info))
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
        module.trust_level = std::cmp::min(module.trust_level, report.module_trust);
        for (sym, adm) in module.symbols.iter_mut().zip(&report.symbols) {
            let current = sym.trust_level.unwrap_or(FfiTrustLevel::Verified);
            sym.trust_level = Some(std::cmp::min(current, adm.trust_level));
        }
        Some(module)
    }
}

// ── TypeshedResolver ───────────────────────────────────────────────

/// Resolves Python modules by parsing `.pyi` stubs from typeshed and other sources.
///
/// Uses [`TypeSourceResolver`] to find stub files and [`PythonApiIndexCache`]
/// to avoid re-parsing stubs on every request. Results are converted to
/// [`FfiModule`] for consumption by the existing type checker.
pub struct TypeshedResolver {
    source_resolver: TypeSourceResolver,
    cache: RefCell<PythonApiIndexCache>,
}

impl TypeshedResolver {
    /// Create a new resolver from the given configuration.
    #[must_use]
    pub fn new(config: &FfiResolverConfig) -> Self {
        let source_resolver = TypeSourceResolver::new(
            config.stub_paths.clone(),
            config.typeshed_path.clone(),
            discover_site_packages_paths(config.python_path.as_deref()),
        );
        Self { source_resolver, cache: RefCell::new(PythonApiIndexCache::new()) }
    }

    /// Resolve a module to its rich [`PythonModuleInfo`] for LSP completion.
    ///
    /// Unlike [`FfiModuleResolver::resolve`], this returns the full index data
    /// including overloaded signatures, method classification, and properties.
    pub fn resolve_index(&self, module_name: &str) -> Option<PythonModuleInfo> {
        let mut cache = self.cache.borrow_mut();
        let info = cache.get_or_parse(module_name, &self.source_resolver)?;
        if info.symbols.is_empty() {
            return None;
        }
        Some(info.clone())
    }
}

impl FfiModuleResolver for TypeshedResolver {
    fn resolve(&self, module_name: &str) -> Option<FfiModule> {
        let mut cache = self.cache.borrow_mut();
        let info = cache.get_or_parse(module_name, &self.source_resolver)?;
        // If the parsed stub has no symbols (e.g., only re-exports), return
        // None to allow the BuiltinResolver fallback to handle it.
        if info.symbols.is_empty() {
            return None;
        }
        Some(info.to_ffi_module())
    }

    fn as_typeshed_resolver(&self) -> Option<&TypeshedResolver> {
        Some(self)
    }
}

fn discover_site_packages_paths(python_path: Option<&std::path::Path>) -> Vec<PathBuf> {
    let Some(python_path) = python_path else {
        return Vec::new();
    };

    if python_path.is_dir() {
        return vec![python_path.to_path_buf()];
    }

    let output = Command::new(python_path)
        .args([
            "-c",
            "import site; paths = site.getsitepackages(); user = site.getusersitepackages(); \
             print('\\n'.join(dict.fromkeys([*paths, user])))",
        ])
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };

    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .collect()
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
        // pathlib is now resolved from typeshed stubs (first priority)
        assert_eq!(module.source, FfiSource::Typeshed);
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
    fn chain_resolver_pathlib_resolves_with_path_class() {
        let chain = ChainResolver::new();
        let module = chain.resolve("pathlib").expect("pathlib should resolve");
        // pathlib should have a Path symbol
        let has_path = module.symbols.iter().any(|s| s.name == "Path");
        assert!(
            has_path,
            "pathlib should have Path: {:?}",
            module.symbols.iter().map(|s| s.name.as_str()).collect::<Vec<_>>()
        );
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
    fn stdlib_only_allows_asyncio() {
        let config = FfiResolverConfig { stdlib_only: true, ..Default::default() };
        let chain = ChainResolver::with_config(config);
        let module = chain.resolve("asyncio").expect("asyncio should resolve with stdlib_only");
        assert_eq!(module.name.as_str(), "asyncio");
        // asyncio trust level depends on parsed stubs; just verify it resolves
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

    #[test]
    fn python_path_directory_enables_site_packages_lookup() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let runtime_dir = dir.path().join("demo");
        std::fs::create_dir_all(&runtime_dir).expect("create runtime");
        std::fs::write(runtime_dir.join("__init__.py"), "def runtime_only() -> str: ...")
            .expect("write runtime");
        std::fs::write(runtime_dir.join("py.typed"), "").expect("write marker");

        let config =
            FfiResolverConfig { python_path: Some(dir.path().to_path_buf()), ..Default::default() };
        let chain = ChainResolver::with_config(config);
        let module = chain.resolve("demo").expect("py.typed package should resolve");
        assert_eq!(module.source, FfiSource::PyTyped);
        assert!(module.symbols.iter().any(|symbol| symbol.name == "runtime_only"));
    }

    #[test]
    fn partial_stub_packages_are_never_verified() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let stub_pkg_dir = dir.path().join("demo-stubs");
        let stub_module_dir = stub_pkg_dir.join("demo");
        std::fs::create_dir_all(&stub_module_dir).expect("create stub pkg");
        std::fs::write(stub_pkg_dir.join("py.typed"), "partial\n").expect("write marker");
        std::fs::write(stub_module_dir.join("__init__.pyi"), "def stubbed() -> int: ...")
            .expect("write stub");

        let runtime_dir = dir.path().join("demo");
        std::fs::create_dir_all(&runtime_dir).expect("create runtime");
        std::fs::write(runtime_dir.join("__init__.py"), "def runtime_only() -> str: ...")
            .expect("write runtime");

        let config =
            FfiResolverConfig { python_path: Some(dir.path().to_path_buf()), ..Default::default() };
        let chain = ChainResolver::with_config(config);
        let module = chain.resolve("demo").expect("partial stub package should resolve");
        assert_eq!(module.source, FfiSource::StubPackage);
        assert_eq!(module.trust_level, FfiTrustLevel::Checked);
    }
}
