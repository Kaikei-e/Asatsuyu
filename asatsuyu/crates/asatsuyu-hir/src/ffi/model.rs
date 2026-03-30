//! FFI model types for representing Python module type information.
//!
//! These types form a neutral IR between typeshed/.pyi parsing and the
//! Asatsuyu type system. They carry no Python runtime dependency.

use smol_str::SmolStr;

// ── Source tracking ────────────────────────────────────────────────

/// Where the type information for an FFI module was obtained.
///
/// Follows PEP 561 lookup priority:
/// 1. `py.typed` marker (inline annotations)
/// 2. Stub-only package (`*-stubs`)
/// 3. Typeshed (bundled stdlib + third-party stubs)
/// 4. Builtin (hand-crafted signatures for MVP)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfiSource {
    /// Package ships inline type annotations (`py.typed` marker).
    PyTyped,
    /// Stub-only package (e.g., `types-requests`).
    StubPackage,
    /// Bundled typeshed stubs.
    Typeshed,
    /// Hand-crafted Rust signatures (MVP fallback).
    Builtin,
}

// ── Trust levels ───────────────────────────────────────────────────

/// FFI trust level, determining how the compiler treats imported symbols.
///
/// Variants are ordered from least to most trusted so that `Ord` gives
/// the correct minimum semantics: `Unsafe < Checked < Verified`.
///
/// See the language charter (principles.md §4.1) for the full model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FfiTrustLevel {
    /// Dynamic or untyped surface — isolated as opaque values.
    Unsafe,
    /// Static types present but not fully sound — requires runtime wrappers.
    Checked,
    /// Fully typed, no `Any` leaks — flows into THIR as normal types.
    Verified,
}

// ── Module ─────────────────────────────────────────────────────────

/// Type information for a Python module resolved by the FFI system.
#[derive(Debug, Clone)]
pub struct FfiModule {
    /// Module name (e.g., `"pathlib"`, `"json"`).
    pub name: SmolStr,
    /// Where the type info came from.
    pub source: FfiSource,
    /// Trust level for this module's surface.
    pub trust_level: FfiTrustLevel,
    /// Exported symbols.
    pub symbols: Vec<FfiSymbol>,
}

// ── Symbol ─────────────────────────────────────────────────────────

/// A single exported symbol within an FFI module.
#[derive(Debug, Clone)]
pub struct FfiSymbol {
    pub name: SmolStr,
    pub kind: FfiSymbolKind,
    /// Trust level for this specific symbol, computed by the admissibility
    /// checker. `None` before admissibility analysis has run.
    pub trust_level: Option<FfiTrustLevel>,
}

/// The kind of an FFI symbol.
#[derive(Debug, Clone)]
pub enum FfiSymbolKind {
    /// A module-level function.
    Function(FfiSignature),
    /// A class with constructor, methods, and properties.
    Class(FfiClass),
    /// A module-level constant or attribute.
    Constant(FfiType),
}

// ── Signature ──────────────────────────────────────────────────────

/// A function or method signature in FFI.
#[derive(Debug, Clone)]
pub struct FfiSignature {
    pub params: Vec<FfiParam>,
    pub return_ty: FfiType,
}

/// A parameter in an FFI signature.
#[derive(Debug, Clone)]
pub struct FfiParam {
    pub name: SmolStr,
    pub ty: FfiType,
    /// Whether this parameter has a default value (and can be omitted).
    pub has_default: bool,
}

// ── Class ──────────────────────────────────────────────────────────

/// A Python class exposed through FFI.
#[derive(Debug, Clone)]
pub struct FfiClass {
    pub name: SmolStr,
    /// Constructor signature (`__init__`), if available.
    pub constructor: Option<FfiSignature>,
    /// Instance methods.
    pub methods: Vec<(SmolStr, FfiSignature)>,
    /// Read-only properties.
    pub properties: Vec<(SmolStr, FfiType)>,
}

// ── Type ───────────────────────────────────────────────────────────

/// A Python type expressed in FFI terms.
///
/// This is intentionally kept minimal for MVP. Complex types like
/// `Protocol`, `Literal`, `ParamSpec` are deferred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FfiType {
    // Primitives
    Int,
    Float,
    Str,
    Bool,
    NoneType,
    Bytes,
    // Containers
    List(Box<FfiType>),
    Dict(Box<FfiType>, Box<FfiType>),
    Tuple(Vec<FfiType>),
    Optional(Box<FfiType>),
    Union(Vec<FfiType>),
    /// A named type from Python (e.g., `pathlib.Path`, `io.TextIOWrapper`).
    Named {
        module: SmolStr,
        name: SmolStr,
    },
    /// Python `Any` — treated as unsafe in Asatsuyu.
    Any,
}

impl FfiType {
    /// Returns `true` if `Any` appears anywhere in this type tree.
    pub fn contains_any(&self) -> bool {
        match self {
            Self::Any => true,
            Self::List(inner) | Self::Optional(inner) => inner.contains_any(),
            Self::Dict(k, v) => k.contains_any() || v.contains_any(),
            Self::Tuple(elems) => elems.iter().any(Self::contains_any),
            Self::Union(variants) => variants.iter().any(Self::contains_any),
            Self::Int
            | Self::Float
            | Self::Str
            | Self::Bool
            | Self::NoneType
            | Self::Bytes
            | Self::Named { .. } => false,
        }
    }
}

// ── Admissibility ─────────────────────────────────────────────────

/// Result of admissibility analysis for an entire FFI module.
#[derive(Debug, Clone)]
pub struct AdmissibilityReport {
    /// Module-level trust: the minimum across all symbol trust levels.
    pub module_trust: FfiTrustLevel,
    /// Per-symbol admissibility determinations.
    pub symbols: Vec<SymbolAdmissibility>,
}

/// Admissibility result for a single FFI symbol.
#[derive(Debug, Clone)]
pub struct SymbolAdmissibility {
    pub name: SmolStr,
    pub trust_level: FfiTrustLevel,
    /// Why this symbol was classified at this level.
    pub reason: AdmissibilityReason,
}

/// Reason for a symbol's trust level classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissibilityReason {
    /// All types in the surface are complete (no `Any`, no bare generics).
    FullyTyped,
    /// Surface contains `Any` somewhere.
    ContainsAny,
    /// No type information available.
    Untyped,
}
