//! Python type source resolution following PEP 561 lookup order.
//!
//! Resolves a Python module name to the physical location of its type
//! information (`.pyi` stub files, `py.typed` packages, or bundled typeshed).
//!
//! Resolution order (PEP 561):
//! 1. Custom stub paths (user-specified)
//! 2. Bundled typeshed stdlib
//! 3. Stub-only packages (`*-stubs`) in site-packages
//! 4. Inline typed packages (`py.typed` marker) in site-packages
//!
//! The [`BuiltinResolver`](super::resolver::BuiltinResolver) serves as a
//! final fallback when no `.pyi` source is found.

use std::path::{Path, PathBuf};

use include_dir::{Dir, include_dir};
use smol_str::SmolStr;

// ── Bundled typeshed ─────────────────────────────────────────────

/// Typeshed stdlib stubs embedded at compile time.
static BUNDLED_TYPESHED: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../vendor/typeshed/stdlib");

// ── Resolved source ──────────────────────────────────────────────

/// Where a module's type information was physically found.
#[derive(Debug, Clone)]
pub struct ResolvedTypeSource {
    /// Fully qualified module name (e.g., `"pathlib"`, `"os.path"`).
    pub module_name: SmolStr,
    /// How the source was located.
    pub source_kind: TypeSourceKind,
    /// Physical file paths containing the type information.
    /// For bundled typeshed, this is empty (content is in-memory).
    ///
    /// When resolving a partial stub package, the first path is the stub file
    /// and any remaining paths are runtime fallback sources to merge in.
    pub paths: Vec<PathBuf>,
    /// Whether this is a partial stub package (`py.typed` contains `partial\n`).
    pub is_partial: bool,
}

/// Classification of where type information was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeSourceKind {
    /// User-specified stub path (highest priority).
    CustomStubs,
    /// Bundled or external typeshed stdlib stubs.
    Typeshed,
    /// Stub-only package (`*-stubs`) from site-packages.
    StubPackage,
    /// Inline typed package (`py.typed` marker) from site-packages.
    PyTypedInline,
    /// Hand-crafted builtin signatures (fallback).
    Builtin,
}

// ── Bundled stub content ─────────────────────────────────────────

/// In-memory stub content from the bundled typeshed.
#[derive(Debug, Clone)]
pub struct BundledStubContent {
    /// The `.pyi` source text.
    pub source: String,
    /// Module name.
    pub module_name: SmolStr,
}

// ── Type source resolver ─────────────────────────────────────────

/// Resolves Python module names to their type information sources.
///
/// Implements PEP 561 resolution order with support for bundled typeshed,
/// custom stub paths, and site-packages scanning.
pub struct TypeSourceResolver {
    /// User-specified directories containing `.pyi` stubs.
    custom_stub_paths: Vec<PathBuf>,
    /// Where to find typeshed stubs.
    typeshed_source: TypeshedSource,
    /// Python site-packages directories for stub/typed package discovery.
    site_packages_paths: Vec<PathBuf>,
    /// Parsed VERSIONS file: (`module_name`, `min_version`, `max_version`).
    /// `None` for max means "still available".
    typeshed_versions: Vec<TypeshedVersionEntry>,
}

/// Parsed entry from typeshed's `VERSIONS` file.
#[derive(Debug, Clone)]
struct TypeshedVersionEntry {
    module: String,
    min_major: u32,
    min_minor: u32,
    /// `None` means module is still present in the latest Python.
    max: Option<(u32, u32)>,
}

/// Where typeshed stubs come from.
enum TypeshedSource {
    /// Stubs embedded in the binary at compile time.
    Bundled,
    /// Stubs loaded from an external filesystem path.
    External(PathBuf),
}

impl TypeSourceResolver {
    /// Create a new resolver with the given configuration.
    #[must_use]
    pub fn new(
        custom_stub_paths: Vec<PathBuf>,
        typeshed_path: Option<PathBuf>,
        site_packages_paths: Vec<PathBuf>,
    ) -> Self {
        let typeshed_source = match typeshed_path {
            Some(path) => TypeshedSource::External(path),
            None => TypeshedSource::Bundled,
        };

        let typeshed_versions = match &typeshed_source {
            TypeshedSource::Bundled => parse_bundled_versions(),
            TypeshedSource::External(path) => parse_external_versions(path),
        };

        Self { custom_stub_paths, typeshed_source, site_packages_paths, typeshed_versions }
    }

    /// Create a resolver using only the bundled typeshed (no custom paths).
    #[must_use]
    pub fn bundled_only() -> Self {
        Self::new(Vec::new(), None, Vec::new())
    }

    /// Resolve a module name to its type source.
    ///
    /// Returns `None` if no type information is found at any source.
    /// The caller should fall back to the [`BuiltinResolver`] in that case.
    #[must_use]
    pub fn resolve(&self, module_name: &str) -> Option<ResolvedTypeSource> {
        // 1. Custom stub paths
        if let Some(source) = self.resolve_custom_stubs(module_name) {
            return Some(source);
        }

        // 2. Typeshed stdlib
        if let Some(source) = self.resolve_typeshed(module_name) {
            return Some(source);
        }

        // 3. Stub-only packages (*-stubs)
        if let Some(source) = self.resolve_stub_package(module_name) {
            return Some(source);
        }

        // 4. Inline typed packages (py.typed)
        if let Some(source) = self.resolve_py_typed_package(module_name) {
            return Some(source);
        }

        None
    }

    /// Return top-level stdlib module names available for Python 3.12+.
    ///
    /// Used by LSP completion to suggest modules after `from python import `.
    /// Only returns top-level names (no dotted submodules like `os.path`).
    #[must_use]
    pub fn available_stdlib_modules(&self) -> Vec<String> {
        self.typeshed_versions
            .iter()
            .filter(|e| !e.module.contains('.'))
            .filter(|e| self.is_available_in_312(&e.module))
            .map(|e| e.module.clone())
            .collect()
    }

    /// Read the `.pyi` source content for a resolved type source.
    ///
    /// For bundled typeshed, reads from the embedded directory.
    /// For filesystem sources, reads from disk.
    #[must_use]
    pub fn read_stub_content(&self, source: &ResolvedTypeSource) -> Option<String> {
        match source.source_kind {
            TypeSourceKind::Typeshed => match &self.typeshed_source {
                TypeshedSource::Bundled => read_bundled_module_source(&source.module_name),
                TypeshedSource::External(_) => {
                    source.paths.first().and_then(|p| std::fs::read_to_string(p).ok())
                }
            },
            TypeSourceKind::CustomStubs
            | TypeSourceKind::StubPackage
            | TypeSourceKind::PyTypedInline => {
                source.paths.first().and_then(|p| std::fs::read_to_string(p).ok())
            }
            TypeSourceKind::Builtin => None,
        }
    }

    /// Read all source texts that contribute to a resolved type source.
    ///
    /// For bundled typeshed this returns a single in-memory stub. For partial
    /// stub packages this may return multiple texts in precedence order: the
    /// partial stub first, followed by runtime fallback sources.
    #[must_use]
    pub fn read_stub_contents(&self, source: &ResolvedTypeSource) -> Option<Vec<String>> {
        match source.source_kind {
            TypeSourceKind::Typeshed => self.read_stub_content(source).map(|content| vec![content]),
            TypeSourceKind::CustomStubs
            | TypeSourceKind::StubPackage
            | TypeSourceKind::PyTypedInline => {
                let contents: Vec<String> = source
                    .paths
                    .iter()
                    .filter_map(|path| std::fs::read_to_string(path).ok())
                    .collect();
                (!contents.is_empty()).then_some(contents)
            }
            TypeSourceKind::Builtin => None,
        }
    }

    // ── Private resolution methods ───────────────────────────────

    fn resolve_custom_stubs(&self, module_name: &str) -> Option<ResolvedTypeSource> {
        for stub_dir in &self.custom_stub_paths {
            if let Some(path) = find_stub_file(stub_dir, module_name) {
                return Some(ResolvedTypeSource {
                    module_name: SmolStr::from(module_name),
                    source_kind: TypeSourceKind::CustomStubs,
                    paths: vec![path],
                    is_partial: false,
                });
            }
        }
        None
    }

    fn resolve_typeshed(&self, module_name: &str) -> Option<ResolvedTypeSource> {
        // Check VERSIONS to see if module is available for Python 3.12+
        if !self.is_available_in_312(module_name) {
            return None;
        }

        match &self.typeshed_source {
            TypeshedSource::Bundled => {
                if bundled_module_exists(module_name) {
                    Some(ResolvedTypeSource {
                        module_name: SmolStr::from(module_name),
                        source_kind: TypeSourceKind::Typeshed,
                        paths: Vec::new(), // bundled, no filesystem path
                        is_partial: false,
                    })
                } else {
                    None
                }
            }
            TypeshedSource::External(typeshed_path) => {
                let stdlib_dir = typeshed_path.join("stdlib");
                find_stub_file(&stdlib_dir, module_name).map(|path| ResolvedTypeSource {
                    module_name: SmolStr::from(module_name),
                    source_kind: TypeSourceKind::Typeshed,
                    paths: vec![path],
                    is_partial: false,
                })
            }
        }
    }

    fn resolve_stub_package(&self, module_name: &str) -> Option<ResolvedTypeSource> {
        let stubs_name = format!("{}-stubs", top_level_package_name(module_name));
        for site_dir in &self.site_packages_paths {
            let stubs_dir = site_dir.join(&stubs_name);
            if stubs_dir.is_dir()
                && let Some(path) = find_stub_file(&stubs_dir, module_name)
            {
                let marker = stubs_dir.join("py.typed");
                let is_partial = marker.is_file()
                    && std::fs::read_to_string(&marker)
                        .ok()
                        .is_some_and(|content| content.trim() == "partial");

                let mut paths = vec![path];
                if is_partial
                    && let Some(runtime_path) = find_module_source_file(site_dir, module_name)
                {
                    paths.push(runtime_path);
                }

                return Some(ResolvedTypeSource {
                    module_name: SmolStr::from(module_name),
                    source_kind: TypeSourceKind::StubPackage,
                    paths,
                    is_partial,
                });
            }
        }
        None
    }

    fn resolve_py_typed_package(&self, module_name: &str) -> Option<ResolvedTypeSource> {
        for site_dir in &self.site_packages_paths {
            let pkg_dir = site_dir.join(top_level_package_path(module_name));
            let marker = pkg_dir.join("py.typed");
            if marker.is_file() {
                let is_partial = std::fs::read_to_string(&marker)
                    .ok()
                    .is_some_and(|content| content.trim() == "partial");

                let Some(path) = find_module_source_file(site_dir, module_name) else {
                    continue;
                };

                return Some(ResolvedTypeSource {
                    module_name: SmolStr::from(module_name),
                    source_kind: TypeSourceKind::PyTypedInline,
                    paths: vec![path],
                    is_partial,
                });
            }
        }
        None
    }

    /// Check if a module is available in Python 3.12+ per the VERSIONS file.
    fn is_available_in_312(&self, module_name: &str) -> bool {
        // If no version entry found, assume available (conservative).
        let entry = self.typeshed_versions.iter().find(|e| e.module == module_name);
        match entry {
            Some(e) => {
                // Must have been introduced by 3.12
                let introduced_ok = e.min_major < 3 || (e.min_major == 3 && e.min_minor <= 12);
                // Must not have been removed before 3.12
                let not_removed = match e.max {
                    None => true,
                    Some((maj, min)) => maj > 3 || (maj == 3 && min >= 12),
                };
                introduced_ok && not_removed
            }
            None => true,
        }
    }
}

// ── Bundled typeshed helpers ─────────────────────────────────────

/// Check if a module exists in the bundled typeshed.
fn bundled_module_exists(module_name: &str) -> bool {
    // Try <module>/__init__.pyi
    let dir_path = format!("{}/__init__.pyi", module_name.replace('.', "/"));
    if BUNDLED_TYPESHED.get_file(&dir_path).is_some() {
        return true;
    }
    // Try <module>.pyi
    let file_path = format!("{}.pyi", module_name.replace('.', "/"));
    BUNDLED_TYPESHED.get_file(&file_path).is_some()
}

/// Read module source from bundled typeshed.
fn read_bundled_module_source(module_name: &str) -> Option<String> {
    // Try <module>/__init__.pyi first
    let dir_path = format!("{}/__init__.pyi", module_name.replace('.', "/"));
    if let Some(file) = BUNDLED_TYPESHED.get_file(&dir_path) {
        return file.contents_utf8().map(String::from);
    }
    // Try <module>.pyi
    let file_path = format!("{}.pyi", module_name.replace('.', "/"));
    BUNDLED_TYPESHED.get_file(&file_path).and_then(|f| f.contents_utf8()).map(String::from)
}

/// Parse the bundled VERSIONS file.
fn parse_bundled_versions() -> Vec<TypeshedVersionEntry> {
    BUNDLED_TYPESHED
        .get_file("VERSIONS")
        .and_then(|f| f.contents_utf8())
        .map(parse_versions_content)
        .unwrap_or_default()
}

/// Parse an external VERSIONS file.
fn parse_external_versions(typeshed_path: &Path) -> Vec<TypeshedVersionEntry> {
    let versions_path = typeshed_path.join("stdlib").join("VERSIONS");
    std::fs::read_to_string(versions_path)
        .ok()
        .map(|content| parse_versions_content(&content))
        .unwrap_or_default()
}

/// Parse VERSIONS file content.
///
/// Format: `module_name: X.Y-` or `module_name: X.Y-A.B`
fn parse_versions_content(content: &str) -> Vec<TypeshedVersionEntry> {
    let mut entries = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Split on first ':'
        let Some((module, version_spec)) = line.split_once(':') else {
            continue;
        };
        let module = module.trim();
        let version_spec = version_spec.trim();

        // Remove inline comments
        let version_spec = version_spec.split('#').next().unwrap_or("").trim();

        // Parse "X.Y-" or "X.Y-A.B"
        let Some((min_part, max_part)) = version_spec.split_once('-') else {
            continue;
        };

        let Some((min_major, min_minor)) = parse_version(min_part.trim()) else {
            continue;
        };

        let max = if max_part.trim().is_empty() { None } else { parse_version(max_part.trim()) };

        entries.push(TypeshedVersionEntry {
            module: module.to_string(),
            min_major,
            min_minor,
            max,
        });
    }
    entries
}

/// Parse "X.Y" into (major, minor).
fn parse_version(s: &str) -> Option<(u32, u32)> {
    let (major, minor) = s.split_once('.')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

// ── Filesystem helpers ───────────────────────────────────────────

/// Find a `.pyi` stub file for a module in a directory.
///
/// Checks `<dir>/<module>/__init__.pyi` and `<dir>/<module>.pyi`.
fn find_stub_file(dir: &Path, module_name: &str) -> Option<PathBuf> {
    let module_path = module_path(module_name);
    let pkg_init = dir.join(&module_path).join("__init__.pyi");
    if pkg_init.is_file() {
        return Some(pkg_init);
    }
    let single = dir.join(&module_path).with_extension("pyi");
    if single.is_file() {
        return Some(single);
    }
    None
}

/// Find either a `.pyi` or `.py` source for a module in a directory.
fn find_module_source_file(dir: &Path, module_name: &str) -> Option<PathBuf> {
    find_stub_file(dir, module_name).or_else(|| {
        let module_path = module_path(module_name);
        let pkg_init = dir.join(&module_path).join("__init__.py");
        if pkg_init.is_file() {
            return Some(pkg_init);
        }

        let single = dir.join(&module_path).with_extension("py");
        single.is_file().then_some(single)
    })
}

fn module_path(module_name: &str) -> PathBuf {
    let mut path = PathBuf::new();
    for segment in module_name.split('.') {
        path.push(segment);
    }
    path
}

fn top_level_package_name(module_name: &str) -> &str {
    module_name.split('.').next().unwrap_or(module_name)
}

fn top_level_package_path(module_name: &str) -> PathBuf {
    PathBuf::from(top_level_package_name(module_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_typeshed_contains_pathlib() {
        assert!(bundled_module_exists("pathlib"));
    }

    #[test]
    fn bundled_typeshed_contains_os() {
        assert!(bundled_module_exists("os"));
    }

    #[test]
    fn bundled_typeshed_contains_sys() {
        assert!(bundled_module_exists("sys"));
    }

    #[test]
    fn bundled_typeshed_contains_json() {
        assert!(bundled_module_exists("json"));
    }

    #[test]
    fn bundled_typeshed_contains_builtins() {
        assert!(bundled_module_exists("builtins"));
    }

    #[test]
    fn bundled_typeshed_contains_typing() {
        assert!(bundled_module_exists("typing"));
    }

    #[test]
    fn bundled_typeshed_not_contains_unknown() {
        assert!(!bundled_module_exists("nonexistent_module_xyz"));
    }

    #[test]
    fn read_pathlib_source() {
        let source = read_bundled_module_source("pathlib").expect("should read pathlib");
        assert!(source.contains("class Path"));
        assert!(source.contains("def exists"));
    }

    #[test]
    fn read_os_source() {
        let source = read_bundled_module_source("os").expect("should read os");
        assert!(source.contains("def getcwd"));
    }

    #[test]
    fn versions_parsing() {
        let content = "pathlib: 3.4-\nos: 3.0-\nossaudiodev: 3.0-3.12\n# comment\n";
        let entries = parse_versions_content(content);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].module, "pathlib");
        assert_eq!(entries[0].min_major, 3);
        assert_eq!(entries[0].min_minor, 4);
        assert!(entries[0].max.is_none());
        assert_eq!(entries[2].module, "ossaudiodev");
        assert_eq!(entries[2].max, Some((3, 12)));
    }

    #[test]
    fn version_available_in_312() {
        let resolver = TypeSourceResolver::bundled_only();
        // pathlib: 3.4- → available
        assert!(resolver.is_available_in_312("pathlib"));
        // os: 3.0- → available
        assert!(resolver.is_available_in_312("os"));
    }

    #[test]
    fn version_removed_before_312() {
        let resolver = TypeSourceResolver::bundled_only();
        // ossaudiodev: 3.0-3.12 → max is 3.12, should still be available
        assert!(resolver.is_available_in_312("ossaudiodev"));
    }

    #[test]
    fn resolver_finds_pathlib_in_bundled() {
        let resolver = TypeSourceResolver::bundled_only();
        let source = resolver.resolve("pathlib").expect("should find pathlib");
        assert_eq!(source.source_kind, TypeSourceKind::Typeshed);
        assert!(!source.is_partial);
    }

    #[test]
    fn resolver_finds_json_in_bundled() {
        let resolver = TypeSourceResolver::bundled_only();
        let source = resolver.resolve("json").expect("should find json");
        assert_eq!(source.source_kind, TypeSourceKind::Typeshed);
    }

    #[test]
    fn resolver_returns_none_for_unknown() {
        let resolver = TypeSourceResolver::bundled_only();
        assert!(resolver.resolve("nonexistent_xyz").is_none());
    }

    #[test]
    fn resolver_reads_stub_content() {
        let resolver = TypeSourceResolver::bundled_only();
        let source = resolver.resolve("pathlib").expect("should find pathlib");
        let content = resolver.read_stub_content(&source).expect("should read content");
        assert!(content.contains("class Path"));
    }

    #[test]
    fn custom_stubs_take_priority() {
        use std::io::Write;

        let dir = tempfile::tempdir().expect("tmpdir");
        let stub_path = dir.path().join("pathlib.pyi");
        let mut f = std::fs::File::create(&stub_path).expect("create");
        writeln!(f, "# custom stub\nclass Path: ...").expect("write");

        let resolver = TypeSourceResolver::new(vec![dir.path().to_path_buf()], None, Vec::new());
        let source = resolver.resolve("pathlib").expect("should find pathlib");
        assert_eq!(source.source_kind, TypeSourceKind::CustomStubs);
    }

    #[test]
    fn dotted_module_paths_are_supported() {
        use std::io::Write;

        let dir = tempfile::tempdir().expect("tmpdir");
        let pkg_dir = dir.path().join("pkg");
        std::fs::create_dir_all(&pkg_dir).expect("create pkg");
        let stub_path = pkg_dir.join("submodule.pyi");
        let mut f = std::fs::File::create(&stub_path).expect("create stub");
        writeln!(f, "def value() -> int: ...").expect("write");

        let resolver = TypeSourceResolver::new(vec![dir.path().to_path_buf()], None, Vec::new());
        let source = resolver.resolve("pkg.submodule").expect("should resolve dotted module");
        assert_eq!(source.source_kind, TypeSourceKind::CustomStubs);
        assert_eq!(source.paths, vec![stub_path]);
    }

    #[test]
    fn partial_stub_package_merges_runtime_source() {
        use std::io::Write;

        let dir = tempfile::tempdir().expect("tmpdir");
        let stubs_dir = dir.path().join("demo-stubs").join("demo");
        std::fs::create_dir_all(&stubs_dir).expect("create stubs");
        let stub_path = stubs_dir.join("__init__.pyi");
        let mut stub = std::fs::File::create(&stub_path).expect("create stub");
        writeln!(stub, "def stubbed() -> int: ...").expect("write stub");
        std::fs::write(dir.path().join("demo-stubs").join("py.typed"), "partial\n")
            .expect("write marker");

        let runtime_dir = dir.path().join("demo");
        std::fs::create_dir_all(&runtime_dir).expect("create runtime");
        let runtime_path = runtime_dir.join("__init__.py");
        std::fs::write(&runtime_path, "def runtime_only() -> str: ...").expect("write runtime");

        let resolver = TypeSourceResolver::new(Vec::new(), None, vec![dir.path().to_path_buf()]);
        let source = resolver.resolve("demo").expect("should resolve partial stub package");
        assert_eq!(source.source_kind, TypeSourceKind::StubPackage);
        assert!(source.is_partial);
        assert_eq!(source.paths, vec![stub_path, runtime_path]);
    }
}
