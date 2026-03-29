/// Identifies a source file within the compilation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId(pub u32);

/// A byte-offset range within a source file.
///
/// Uses `u32` offsets (supports files up to ~4 GiB), following the same
/// approach as Ruff and rust-analyzer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub file_id: FileId,
    pub start: u32,
    pub end: u32,
}

impl Span {
    /// Creates a new span.
    #[inline]
    pub fn new(file_id: FileId, start: u32, end: u32) -> Self {
        debug_assert!(start <= end, "span start ({start}) must not exceed end ({end})");
        Self { file_id, start, end }
    }

    /// Creates a dummy span for use in tests or synthesized nodes.
    #[inline]
    pub fn dummy() -> Self {
        Self { file_id: FileId(0), start: 0, end: 0 }
    }

    /// Returns the byte length of this span.
    #[inline]
    pub fn len(self) -> u32 {
        self.end - self.start
    }

    /// Returns `true` if this span is empty (zero length).
    #[inline]
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Returns `true` if the given byte offset falls within this span.
    #[inline]
    pub fn contains(self, offset: u32) -> bool {
        self.start <= offset && offset < self.end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_new() {
        let span = Span::new(FileId(1), 10, 20);
        assert_eq!(span.file_id, FileId(1));
        assert_eq!(span.start, 10);
        assert_eq!(span.end, 20);
        assert_eq!(span.len(), 10);
        assert!(!span.is_empty());
    }

    #[test]
    fn span_dummy() {
        let span = Span::dummy();
        assert_eq!(span.file_id, FileId(0));
        assert_eq!(span.start, 0);
        assert_eq!(span.end, 0);
        assert!(span.is_empty());
        assert_eq!(span.len(), 0);
    }

    #[test]
    fn span_contains() {
        let span = Span::new(FileId(0), 5, 15);
        assert!(!span.contains(4));
        assert!(span.contains(5));
        assert!(span.contains(10));
        assert!(span.contains(14));
        assert!(!span.contains(15));
    }
}
