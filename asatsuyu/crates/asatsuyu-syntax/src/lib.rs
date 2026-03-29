//! Shared type definitions for the Asatsuyu compiler.
//!
//! This crate is the lowest-level dependency in the compiler, depended upon by
//! all other crates. It has **zero external dependencies** by design.
//!
//! Provides:
//! - [`SyntaxKind`] — unified token/node classification (rowan-compatible)
//! - [`Span`] / [`FileId`] — source location tracking
//! - [`Diagnostic`] / [`Label`] — compiler error and warning messages

pub mod diagnostic;
pub mod span;
pub mod syntax_kind;

pub use diagnostic::{Diagnostic, DiagnosticCode, Label, LabelStyle, Severity};
pub use span::{FileId, Span};
pub use syntax_kind::SyntaxKind;
