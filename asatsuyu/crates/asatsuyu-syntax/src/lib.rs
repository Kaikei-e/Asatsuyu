//! Shared type definitions for the Asatsuyu compiler.
//!
//! This crate defines the foundational types used by all other compiler crates:
//! - `TokenKind` — token classification for the lexer
//! - `NodeKind` — CST node classification for the parser
//! - `Span` — source location tracking
//! - `Diagnostic` — compiler error and warning messages
