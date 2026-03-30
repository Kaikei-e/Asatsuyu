//! Python FFI module resolution for the Asatsuyu compiler.
//!
//! Provides the model types for representing Python module type information
//! and a resolver framework that follows PEP 561 lookup order.
//!
//! # Architecture
//!
//! - [`model`]: Core types — [`FfiModule`], [`FfiType`], [`FfiSource`], [`FfiTrustLevel`]
//! - [`resolver`]: Resolver trait and [`ChainResolver`] composite
//! - [`builtins`]: Hand-crafted signatures for Phase 1 stdlib modules

pub mod admissibility;
pub mod builtins;
pub mod model;
pub mod resolver;

pub use model::{
    AdmissibilityReason, AdmissibilityReport, FfiClass, FfiModule, FfiParam, FfiSignature,
    FfiSource, FfiSymbol, FfiSymbolKind, FfiTrustLevel, FfiType, SymbolAdmissibility,
};
pub use resolver::{BuiltinResolver, ChainResolver, FfiModuleResolver};
