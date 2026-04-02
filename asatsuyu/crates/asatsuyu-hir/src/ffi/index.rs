//! Python API index IR for LSP consumption and FFI resolution.
//!
//! [`PythonApiIndex`] holds a richer representation of Python module APIs
//! than [`FfiModule`]. It supports overloaded signatures, method classification
//! (instance / class / static), and provenance tracking.
//!
//! The index is converted to [`FfiModule`] for consumption by the existing
//! type checker, keeping changes to downstream code minimal.

use std::collections::HashMap;

use smol_str::SmolStr;

use super::model::{
    FfiClass, FfiModule, FfiSignature, FfiSource, FfiSymbol, FfiSymbolKind, FfiTrustLevel, FfiType,
};
use super::source::{ResolvedTypeSource, TypeSourceKind};

// ── Index ─────────────────────────────────────────────────────────

/// Top-level index mapping module names to their parsed API info.
#[derive(Debug, Clone, Default)]
pub struct PythonApiIndex {
    pub modules: HashMap<SmolStr, PythonModuleInfo>,
}

impl PythonApiIndex {
    /// Create an empty index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a module into the index.
    pub fn insert(&mut self, module: PythonModuleInfo) {
        self.modules.insert(module.name.clone(), module);
    }

    /// Look up a module by name.
    #[must_use]
    pub fn get(&self, module_name: &str) -> Option<&PythonModuleInfo> {
        self.modules.get(module_name)
    }

    /// Reverse lookup: find modules that export a symbol with the given name.
    ///
    /// Returns `(module_name, symbol_info)` pairs for each match.
    #[must_use]
    pub fn find_modules_for_symbol(&self, symbol_name: &str) -> Vec<(&SmolStr, &PythonSymbolInfo)> {
        self.modules
            .values()
            .flat_map(|m| {
                m.symbols.iter().filter(|s| s.name == symbol_name).map(move |s| (&m.name, s))
            })
            .collect()
    }
}

// ── Module info ───────────────────────────────────────────────────

/// Parsed public API of a single Python module.
#[derive(Debug, Clone)]
pub struct PythonModuleInfo {
    /// Module name (e.g., `"pathlib"`, `"os"`).
    pub name: SmolStr,
    /// Where the type info was resolved from.
    pub source: ResolvedTypeSource,
    /// Public symbols exported by the module.
    pub symbols: Vec<PythonSymbolInfo>,
    /// Known submodule names (e.g., `"os.path"`).
    pub submodules: Vec<SmolStr>,
}

// ── Symbol info ───────────────────────────────────────────────────

/// A single public symbol within a Python module.
#[derive(Debug, Clone)]
pub struct PythonSymbolInfo {
    /// Symbol name (e.g., `"Path"`, `"loads"`, `"getcwd"`).
    pub name: SmolStr,
    /// What kind of symbol this is.
    pub kind: PythonSymbolKind,
    /// First line of docstring, if extracted.
    pub doc: Option<String>,
    /// Whether this symbol is `async def`.
    pub is_async: bool,
    /// Where this symbol's type info came from.
    pub provenance: TypeSourceKind,
}

/// Classification of a Python symbol.
#[derive(Debug, Clone)]
pub enum PythonSymbolKind {
    /// Module-level function (possibly overloaded).
    Function(PythonFunctionInfo),
    /// Class definition.
    Class(PythonClassInfo),
    /// Module-level constant or attribute.
    Constant(FfiType),
    /// Submodule reference.
    Module,
}

// ── Function info ─────────────────────────────────────────────────

/// A function that may have one or more overloaded signatures.
#[derive(Debug, Clone)]
pub struct PythonFunctionInfo {
    /// One or more signatures (multiple for `@overload` functions).
    pub signatures: Vec<FfiSignature>,
}

// ── Class info ────────────────────────────────────────────────────

/// A Python class with constructor, methods, and properties.
#[derive(Debug, Clone)]
pub struct PythonClassInfo {
    /// Class name.
    pub name: SmolStr,
    /// Constructor signature (`__init__`), if available.
    pub constructor: Option<FfiSignature>,
    /// Instance methods.
    pub methods: Vec<PythonMethodInfo>,
    /// Read-only properties (name → type).
    pub properties: Vec<(SmolStr, FfiType)>,
    /// Class methods (`@classmethod`).
    pub class_methods: Vec<PythonMethodInfo>,
    /// Static methods (`@staticmethod`).
    pub static_methods: Vec<PythonMethodInfo>,
}

// ── Method info ───────────────────────────────────────────────────

/// A method within a class, possibly overloaded.
#[derive(Debug, Clone)]
pub struct PythonMethodInfo {
    /// Method name.
    pub name: SmolStr,
    /// One or more signatures (multiple for `@overload`).
    pub signatures: Vec<FfiSignature>,
    /// Whether this is an `async def` method.
    pub is_async: bool,
}

// ── Conversion to FfiModule ──────────────────────────────────────

impl PythonModuleInfo {
    /// Convert to [`FfiModule`] for consumption by the existing type checker.
    ///
    /// This bridges the richer `PythonApiIndex` representation to the simpler
    /// `FfiModule` model that the type checker and LSP already understand.
    /// For overloaded functions, only the first signature is used (the type
    /// checker does not yet support overload resolution).
    #[must_use]
    pub fn to_ffi_module(&self) -> FfiModule {
        let source = match self.source.source_kind {
            TypeSourceKind::CustomStubs | TypeSourceKind::StubPackage => FfiSource::StubPackage,
            TypeSourceKind::Typeshed => FfiSource::Typeshed,
            TypeSourceKind::PyTypedInline => FfiSource::PyTyped,
            TypeSourceKind::Builtin => FfiSource::Builtin,
        };

        let symbols = self.symbols.iter().map(|sym| symbol_to_ffi(sym, &self.name)).collect();

        FfiModule {
            name: self.name.clone(),
            source,
            trust_level: if self.source.is_partial {
                FfiTrustLevel::Checked
            } else {
                FfiTrustLevel::Verified
            },
            symbols,
        }
    }
}

/// Convert a `PythonSymbolInfo` to an `FfiSymbol`.
fn symbol_to_ffi(sym: &PythonSymbolInfo, module_name: &SmolStr) -> FfiSymbol {
    let kind = match &sym.kind {
        PythonSymbolKind::Function(info) => {
            let sig = first_signature(&info.signatures, sym.is_async);
            FfiSymbolKind::Function(sig)
        }
        PythonSymbolKind::Class(info) => FfiSymbolKind::Class(class_to_ffi(info, module_name)),
        PythonSymbolKind::Constant(ty) => FfiSymbolKind::Constant(ty.clone()),
        PythonSymbolKind::Module => FfiSymbolKind::Constant(FfiType::Any),
    };

    FfiSymbol { name: sym.name.clone(), kind, trust_level: None }
}

/// Convert `PythonClassInfo` to `FfiClass`.
fn class_to_ffi(info: &PythonClassInfo, module_name: &SmolStr) -> FfiClass {
    let constructor = info.constructor.as_ref().map(|sig| {
        // Rewrite the constructor return type to Named { module, class }
        let mut ctor_sig = sig.clone();
        ctor_sig.return_ty =
            FfiType::Named { module: module_name.clone(), name: info.name.clone() };
        ctor_sig
    });

    let methods: Vec<(SmolStr, FfiSignature)> = info
        .methods
        .iter()
        .map(|m| {
            let sig = first_signature(&m.signatures, m.is_async);
            (m.name.clone(), sig)
        })
        .collect();

    // Include static methods and class methods as regular methods for now.
    // The type checker does not yet distinguish them.
    let mut all_methods = methods;
    for m in &info.static_methods {
        let sig = first_signature(&m.signatures, m.is_async);
        all_methods.push((m.name.clone(), sig));
    }
    for m in &info.class_methods {
        let sig = first_signature(&m.signatures, m.is_async);
        all_methods.push((m.name.clone(), sig));
    }

    FfiClass {
        name: info.name.clone(),
        constructor,
        methods: all_methods,
        properties: info.properties.clone(),
    }
}

/// Get the first signature from an overload list, applying async flag.
fn first_signature(signatures: &[FfiSignature], is_async: bool) -> FfiSignature {
    signatures.first().cloned().unwrap_or_else(|| FfiSignature {
        params: Vec::new(),
        return_ty: FfiType::Any,
        is_async,
    })
}

// ── Builder helpers ──────────────────────────────────────────────

impl PythonModuleInfo {
    /// Create a new module info with the given name and source.
    #[must_use]
    pub fn new(name: SmolStr, source: ResolvedTypeSource) -> Self {
        Self { name, source, symbols: Vec::new(), submodules: Vec::new() }
    }

    /// Add a symbol to this module.
    pub fn add_symbol(&mut self, symbol: PythonSymbolInfo) {
        self.symbols.push(symbol);
    }

    /// Merge missing API surface from a lower-priority source.
    ///
    /// This is used for partial stub packages, where the stub package defines
    /// only part of the surface and the runtime package fills in the rest.
    pub fn merge_from(&mut self, other: PythonModuleInfo) {
        for symbol in other.symbols {
            match self.symbols.iter_mut().find(|existing| existing.name == symbol.name) {
                Some(existing) => merge_symbol(existing, symbol),
                None => self.symbols.push(symbol),
            }
        }

        for submodule in other.submodules {
            if !self.submodules.contains(&submodule) {
                self.submodules.push(submodule);
            }
        }
    }
}

fn merge_symbol(existing: &mut PythonSymbolInfo, incoming: PythonSymbolInfo) {
    match (&mut existing.kind, incoming.kind) {
        (PythonSymbolKind::Function(existing_fn), PythonSymbolKind::Function(incoming_fn)) => {
            for signature in incoming_fn.signatures {
                if !existing_fn
                    .signatures
                    .iter()
                    .any(|current| signatures_equal(current, &signature))
                {
                    existing_fn.signatures.push(signature);
                }
            }
        }
        (PythonSymbolKind::Class(existing_cls), PythonSymbolKind::Class(incoming_cls)) => {
            merge_class(existing_cls, incoming_cls);
        }
        _ => {}
    }
}

fn merge_class(existing: &mut PythonClassInfo, incoming: PythonClassInfo) {
    if existing.constructor.is_none() {
        existing.constructor = incoming.constructor;
    }

    merge_methods(&mut existing.methods, incoming.methods);
    merge_methods(&mut existing.class_methods, incoming.class_methods);
    merge_methods(&mut existing.static_methods, incoming.static_methods);

    for property in incoming.properties {
        if !existing.properties.iter().any(|(name, _)| *name == property.0) {
            existing.properties.push(property);
        }
    }
}

fn merge_methods(existing: &mut Vec<PythonMethodInfo>, incoming: Vec<PythonMethodInfo>) {
    for method in incoming {
        if let Some(current) = existing.iter_mut().find(|candidate| candidate.name == method.name) {
            for signature in method.signatures {
                if !current.signatures.iter().any(|present| signatures_equal(present, &signature)) {
                    current.signatures.push(signature);
                }
            }
        } else {
            existing.push(method);
        }
    }
}

fn signatures_equal(left: &FfiSignature, right: &FfiSignature) -> bool {
    left.is_async == right.is_async
        && left.return_ty == right.return_ty
        && left.params.len() == right.params.len()
        && left
            .params
            .iter()
            .zip(&right.params)
            .all(|(l, r)| l.name == r.name && l.ty == r.ty && l.has_default == r.has_default)
}

impl PythonFunctionInfo {
    /// Create from a single signature.
    #[must_use]
    pub fn single(sig: FfiSignature) -> Self {
        Self { signatures: vec![sig] }
    }

    /// Add an overload signature.
    pub fn add_overload(&mut self, sig: FfiSignature) {
        self.signatures.push(sig);
    }
}

impl PythonClassInfo {
    /// Create a new class info with the given name.
    #[must_use]
    pub fn new(name: SmolStr) -> Self {
        Self {
            name,
            constructor: None,
            methods: Vec::new(),
            properties: Vec::new(),
            class_methods: Vec::new(),
            static_methods: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::model::FfiParam;
    use crate::ffi::source::TypeSourceKind;

    fn dummy_source(module_name: &str) -> ResolvedTypeSource {
        ResolvedTypeSource {
            module_name: SmolStr::from(module_name),
            source_kind: TypeSourceKind::Typeshed,
            paths: Vec::new(),
            is_partial: false,
        }
    }

    fn int_param(name: &str) -> FfiParam {
        FfiParam { name: SmolStr::from(name), ty: FfiType::Int, has_default: false }
    }

    #[test]
    fn module_to_ffi_basic() {
        let mut module = PythonModuleInfo::new(SmolStr::from("mymod"), dummy_source("mymod"));
        module.add_symbol(PythonSymbolInfo {
            name: SmolStr::from("hello"),
            kind: PythonSymbolKind::Function(PythonFunctionInfo::single(FfiSignature {
                params: vec![int_param("x")],
                return_ty: FfiType::Str,
                is_async: false,
            })),
            doc: None,
            is_async: false,
            provenance: TypeSourceKind::Typeshed,
        });

        let ffi = module.to_ffi_module();
        assert_eq!(ffi.name.as_str(), "mymod");
        assert_eq!(ffi.source, FfiSource::Typeshed);
        assert_eq!(ffi.symbols.len(), 1);
        assert_eq!(ffi.symbols[0].name.as_str(), "hello");
    }

    #[test]
    fn class_to_ffi_with_constructor() {
        let mut cls = PythonClassInfo::new(SmolStr::from("Path"));
        cls.constructor = Some(FfiSignature {
            params: vec![FfiParam {
                name: SmolStr::from("path"),
                ty: FfiType::Str,
                has_default: true,
            }],
            return_ty: FfiType::NoneType, // __init__ returns None in Python
            is_async: false,
        });
        cls.methods.push(PythonMethodInfo {
            name: SmolStr::from("exists"),
            signatures: vec![FfiSignature {
                params: Vec::new(),
                return_ty: FfiType::Bool,
                is_async: false,
            }],
            is_async: false,
        });
        cls.properties.push((SmolStr::from("name"), FfiType::Str));

        let mut module = PythonModuleInfo::new(SmolStr::from("pathlib"), dummy_source("pathlib"));
        module.add_symbol(PythonSymbolInfo {
            name: SmolStr::from("Path"),
            kind: PythonSymbolKind::Class(cls),
            doc: None,
            is_async: false,
            provenance: TypeSourceKind::Typeshed,
        });

        let ffi = module.to_ffi_module();
        let path_sym = &ffi.symbols[0];
        match &path_sym.kind {
            FfiSymbolKind::Class(cls) => {
                // Constructor return type should be rewritten to Named
                let ctor = cls.constructor.as_ref().expect("should have constructor");
                assert_eq!(
                    ctor.return_ty,
                    FfiType::Named {
                        module: SmolStr::from("pathlib"),
                        name: SmolStr::from("Path"),
                    }
                );
                // Methods should include exists
                assert_eq!(cls.methods.len(), 1);
                assert_eq!(cls.methods[0].0.as_str(), "exists");
                // Properties should include name
                assert_eq!(cls.properties.len(), 1);
                assert_eq!(cls.properties[0].0.as_str(), "name");
            }
            other => panic!("expected Class, got {other:?}"),
        }
    }

    #[test]
    fn overloaded_function_uses_first_signature() {
        let mut func_info = PythonFunctionInfo::single(FfiSignature {
            params: vec![int_param("x")],
            return_ty: FfiType::Int,
            is_async: false,
        });
        func_info.add_overload(FfiSignature {
            params: vec![FfiParam {
                name: SmolStr::from("x"),
                ty: FfiType::Str,
                has_default: false,
            }],
            return_ty: FfiType::Str,
            is_async: false,
        });
        assert_eq!(func_info.signatures.len(), 2);

        let mut module = PythonModuleInfo::new(SmolStr::from("test"), dummy_source("test"));
        module.add_symbol(PythonSymbolInfo {
            name: SmolStr::from("parse"),
            kind: PythonSymbolKind::Function(func_info),
            doc: None,
            is_async: false,
            provenance: TypeSourceKind::Typeshed,
        });

        let ffi = module.to_ffi_module();
        match &ffi.symbols[0].kind {
            FfiSymbolKind::Function(sig) => {
                // Should use first signature (Int -> Int)
                assert_eq!(sig.params[0].ty, FfiType::Int);
                assert_eq!(sig.return_ty, FfiType::Int);
            }
            other => panic!("expected Function, got {other:?}"),
        }
    }

    #[test]
    fn index_insert_and_get() {
        let mut index = PythonApiIndex::new();
        let module = PythonModuleInfo::new(SmolStr::from("pathlib"), dummy_source("pathlib"));
        index.insert(module);
        assert!(index.get("pathlib").is_some());
        assert!(index.get("nonexistent").is_none());
    }

    #[test]
    fn constant_symbol_to_ffi() {
        let mut module = PythonModuleInfo::new(SmolStr::from("os"), dummy_source("os"));
        module.add_symbol(PythonSymbolInfo {
            name: SmolStr::from("sep"),
            kind: PythonSymbolKind::Constant(FfiType::Str),
            doc: None,
            is_async: false,
            provenance: TypeSourceKind::Typeshed,
        });

        let ffi = module.to_ffi_module();
        match &ffi.symbols[0].kind {
            FfiSymbolKind::Constant(ty) => assert_eq!(*ty, FfiType::Str),
            other => panic!("expected Constant, got {other:?}"),
        }
    }

    #[test]
    fn merge_partial_stub_runtime_symbols() {
        let mut primary = PythonModuleInfo::new(SmolStr::from("demo"), dummy_source("demo"));
        primary.add_symbol(PythonSymbolInfo {
            name: SmolStr::from("stubbed"),
            kind: PythonSymbolKind::Function(PythonFunctionInfo::single(FfiSignature {
                params: vec![],
                return_ty: FfiType::Int,
                is_async: false,
            })),
            doc: None,
            is_async: false,
            provenance: TypeSourceKind::StubPackage,
        });

        let mut runtime = PythonModuleInfo::new(SmolStr::from("demo"), dummy_source("demo"));
        runtime.add_symbol(PythonSymbolInfo {
            name: SmolStr::from("runtime_only"),
            kind: PythonSymbolKind::Function(PythonFunctionInfo::single(FfiSignature {
                params: vec![],
                return_ty: FfiType::Str,
                is_async: false,
            })),
            doc: None,
            is_async: false,
            provenance: TypeSourceKind::PyTypedInline,
        });

        primary.merge_from(runtime);

        let names: Vec<&str> = primary.symbols.iter().map(|symbol| symbol.name.as_str()).collect();
        assert!(names.contains(&"stubbed"));
        assert!(names.contains(&"runtime_only"));
    }
}
