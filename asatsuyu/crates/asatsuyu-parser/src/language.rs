//! Rowan [`Language`](rowan::Language) integration for Asatsuyu.
//!
//! Bridges [`SyntaxKind`] to rowan's generic tree infrastructure and provides
//! convenience type aliases for the concrete syntax tree.

use asatsuyu_syntax::SyntaxKind;
use rowan::Language;

/// Marker type for Asatsuyu's concrete syntax tree.
///
/// This empty enum exists solely to implement [`rowan::Language`], connecting
/// [`SyntaxKind`] to rowan's `SyntaxNode` / `SyntaxToken` / `SyntaxElement`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AsatsuyuLanguage {}

impl rowan::Language for AsatsuyuLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> SyntaxKind {
        SyntaxKind::from(raw.0)
    }

    fn kind_to_raw(kind: SyntaxKind) -> rowan::SyntaxKind {
        rowan::SyntaxKind(kind.into())
    }
}

/// A node in the Asatsuyu concrete syntax tree.
pub type SyntaxNode = rowan::SyntaxNode<AsatsuyuLanguage>;

/// A token (leaf) in the Asatsuyu concrete syntax tree.
pub type SyntaxToken = rowan::SyntaxToken<AsatsuyuLanguage>;

/// Either a node or a token in the Asatsuyu concrete syntax tree.
pub type SyntaxElement = rowan::SyntaxElement<AsatsuyuLanguage>;

/// Convert an [`asatsuyu_syntax::SyntaxKind`] to a [`rowan::SyntaxKind`].
#[inline]
pub(crate) fn raw(kind: SyntaxKind) -> rowan::SyntaxKind {
    AsatsuyuLanguage::kind_to_raw(kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_roundtrip() {
        for raw_val in 0..SyntaxKind::__LAST as u16 {
            let kind = SyntaxKind::from(raw_val);
            let rowan_kind = AsatsuyuLanguage::kind_to_raw(kind);
            let back = AsatsuyuLanguage::kind_from_raw(rowan_kind);
            assert_eq!(kind, back);
        }
    }
}
