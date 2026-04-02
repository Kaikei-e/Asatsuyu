//! Module-level cache for parsed Python API indices.
//!
//! Avoids re-parsing `.pyi` stubs on every completion request. The cache
//! uses file mtime + size as a fingerprint for invalidation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use smol_str::SmolStr;

use super::index::PythonModuleInfo;
use super::source::{TypeSourceKind, TypeSourceResolver};
use super::stub_parser;

// ── Fingerprint ──────────────────────────────────────────────────

/// Lightweight file identity used for cache invalidation.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceFingerprint {
    /// Resolution source classification.
    source_kind: TypeSourceKind,
    /// Whether the resolved source is partial and requires runtime merge.
    is_partial: bool,
    /// Absolute or relative paths participating in the merged source set.
    paths: Vec<String>,
    /// File modification time (None for bundled/in-memory stubs).
    mtimes: Vec<Option<SystemTime>>,
    /// File sizes in bytes (0 for bundled stubs).
    sizes: Vec<u64>,
}

impl SourceFingerprint {
    /// Compute fingerprint for a resolved set of filesystem paths.
    fn from_paths(source_kind: TypeSourceKind, is_partial: bool, paths: &[&Path]) -> Self {
        let mut mtimes = Vec::with_capacity(paths.len());
        let mut sizes = Vec::with_capacity(paths.len());
        let mut path_strings = Vec::with_capacity(paths.len());

        for path in paths {
            let meta = std::fs::metadata(path).ok();
            mtimes.push(meta.as_ref().and_then(|m| m.modified().ok()));
            sizes.push(meta.map_or(0, |m| m.len()));
            path_strings.push(path.to_string_lossy().into_owned());
        }

        Self { source_kind, is_partial, paths: path_strings, mtimes, sizes }
    }

    /// Fingerprint for bundled (in-memory) stubs that never change.
    fn bundled(source_kind: TypeSourceKind, is_partial: bool) -> Self {
        Self { source_kind, is_partial, paths: Vec::new(), mtimes: Vec::new(), sizes: Vec::new() }
    }
}

// ── Cache entry ──────────────────────────────────────────────────

struct CacheEntry {
    module: PythonModuleInfo,
    fingerprint: SourceFingerprint,
}

// ── Cache ────────────────────────────────────────────────────────

/// Cache for parsed Python API indices.
///
/// Keyed by module name. On lookup, checks whether the source file has
/// changed (via mtime + size); if stale, re-parses the stub.
pub struct PythonApiIndexCache {
    entries: HashMap<SmolStr, CacheEntry>,
}

impl PythonApiIndexCache {
    /// Create an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self { entries: HashMap::new() }
    }

    /// Get a cached module, or parse and cache it if missing/stale.
    ///
    /// Returns `None` if the module cannot be found or parsed.
    pub fn get_or_parse(
        &mut self,
        module_name: &str,
        resolver: &TypeSourceResolver,
    ) -> Option<&PythonModuleInfo> {
        let key = SmolStr::from(module_name);
        let source_info = resolver.resolve(module_name)?;
        let current_fp = Self::compute_fingerprint(&source_info);

        // Check if we have a valid cached entry
        if let Some(entry) = self.entries.get(&key)
            && entry.fingerprint == current_fp
        {
            return self.entries.get(&key).map(|e| &e.module);
        }

        // Cache miss or stale — resolve and parse.
        let contents = resolver.read_stub_contents(&source_info)?;
        let mut parsed_modules = contents.into_iter().enumerate().map(|(idx, content)| {
            let mut parse_source = source_info.clone();
            parse_source.paths = source_info.paths.get(idx).cloned().into_iter().collect();
            if idx > 0 && source_info.source_kind == TypeSourceKind::StubPackage {
                parse_source.source_kind = TypeSourceKind::PyTypedInline;
                parse_source.is_partial = false;
            }
            stub_parser::parse_type_source(&parse_source, &content).ok()
        });

        let mut module_info = parsed_modules.next().flatten()?;
        for extra in parsed_modules.flatten() {
            module_info.merge_from(extra);
        }

        self.entries
            .insert(key.clone(), CacheEntry { module: module_info, fingerprint: current_fp });
        self.entries.get(&key).map(|e| &e.module)
    }

    /// Invalidate all cached entries.
    pub fn invalidate_all(&mut self) {
        self.entries.clear();
    }

    /// Invalidate a single module's cache entry.
    pub fn invalidate(&mut self, module_name: &str) {
        self.entries.remove(module_name);
    }

    /// Number of cached entries (for testing).
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn compute_fingerprint(source_info: &super::source::ResolvedTypeSource) -> SourceFingerprint {
        if source_info.paths.is_empty() {
            return SourceFingerprint::bundled(source_info.source_kind, source_info.is_partial);
        }

        let paths: Vec<&Path> = source_info.paths.iter().map(PathBuf::as_path).collect();
        SourceFingerprint::from_paths(source_info.source_kind, source_info.is_partial, &paths)
    }
}

impl Default for PythonApiIndexCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_starts_empty() {
        let cache = PythonApiIndexCache::new();
        assert!(cache.is_empty());
    }

    #[test]
    fn cache_hit_returns_same_result() {
        let resolver = TypeSourceResolver::bundled_only();
        let mut cache = PythonApiIndexCache::new();

        // First call: cache miss
        let first_name = cache.get_or_parse("pathlib", &resolver).map(|m| m.name.clone());
        assert!(first_name.is_some());
        assert_eq!(cache.len(), 1);

        // Second call: cache hit
        let second_name = cache.get_or_parse("pathlib", &resolver).map(|m| m.name.clone());
        assert!(second_name.is_some());
        assert_eq!(cache.len(), 1);

        // Both should return the same module name
        assert_eq!(first_name, second_name);
    }

    #[test]
    fn cache_returns_none_for_unknown() {
        let resolver = TypeSourceResolver::bundled_only();
        let mut cache = PythonApiIndexCache::new();
        assert!(cache.get_or_parse("nonexistent_xyz", &resolver).is_none());
    }

    #[test]
    fn invalidate_all_clears_cache() {
        let resolver = TypeSourceResolver::bundled_only();
        let mut cache = PythonApiIndexCache::new();

        cache.get_or_parse("pathlib", &resolver);
        cache.get_or_parse("os", &resolver);
        assert_eq!(cache.len(), 2);

        cache.invalidate_all();
        assert!(cache.is_empty());
    }

    #[test]
    fn invalidate_single_module() {
        let resolver = TypeSourceResolver::bundled_only();
        let mut cache = PythonApiIndexCache::new();

        cache.get_or_parse("pathlib", &resolver);
        cache.get_or_parse("os", &resolver);
        assert_eq!(cache.len(), 2);

        cache.invalidate("pathlib");
        assert_eq!(cache.len(), 1);
        assert!(cache.get_or_parse("os", &resolver).is_some());
    }

    #[test]
    fn cached_pathlib_has_path_class() {
        let resolver = TypeSourceResolver::bundled_only();
        let mut cache = PythonApiIndexCache::new();

        let info = cache.get_or_parse("pathlib", &resolver).expect("should parse pathlib");
        let has_path = info.symbols.iter().any(|s| s.name == "Path");
        assert!(has_path, "pathlib should have Path class");
    }

    #[test]
    fn cached_os_has_getcwd() {
        let resolver = TypeSourceResolver::bundled_only();
        let mut cache = PythonApiIndexCache::new();

        let info = cache.get_or_parse("os", &resolver).expect("should parse os");
        let has_getcwd = info.symbols.iter().any(|s| s.name == "getcwd");
        assert!(has_getcwd, "os should have getcwd");
    }

    #[test]
    fn multiple_modules_cached_independently() {
        let resolver = TypeSourceResolver::bundled_only();
        let mut cache = PythonApiIndexCache::new();

        cache.get_or_parse("pathlib", &resolver);
        cache.get_or_parse("os", &resolver);
        cache.get_or_parse("sys", &resolver);
        assert_eq!(cache.len(), 3);
    }

    #[test]
    fn partial_stub_runtime_change_invalidates_cache() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let stub_pkg_dir = dir.path().join("demo-stubs");
        let stub_module_dir = stub_pkg_dir.join("demo");
        std::fs::create_dir_all(&stub_module_dir).expect("create stub dir");
        std::fs::write(stub_pkg_dir.join("py.typed"), "partial\n").expect("write marker");
        std::fs::write(stub_module_dir.join("__init__.pyi"), "def stubbed() -> int: ...")
            .expect("write stub");

        let runtime_dir = dir.path().join("demo");
        std::fs::create_dir_all(&runtime_dir).expect("create runtime dir");
        let runtime_path = runtime_dir.join("__init__.py");
        std::fs::write(&runtime_path, "def runtime_one() -> str: ...").expect("write runtime");

        let resolver = TypeSourceResolver::new(Vec::new(), None, vec![dir.path().to_path_buf()]);
        let mut cache = PythonApiIndexCache::new();

        let first = cache.get_or_parse("demo", &resolver).expect("first parse");
        assert!(first.symbols.iter().any(|symbol| symbol.name == "runtime_one"));

        std::fs::write(
            &runtime_path,
            "def runtime_one() -> str: ...\ndef runtime_two() -> bool: ...",
        )
        .expect("rewrite runtime");

        let second = cache.get_or_parse("demo", &resolver).expect("re-parse after change");
        assert!(second.symbols.iter().any(|symbol| symbol.name == "runtime_two"));
    }
}
