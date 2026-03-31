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
    #[must_use]
    pub fn new(file_id: FileId, start: u32, end: u32) -> Self {
        debug_assert!(start <= end, "span start ({start}) must not exceed end ({end})");
        Self { file_id, start, end }
    }

    /// Creates a dummy span for use in tests or synthesized nodes.
    #[inline]
    #[must_use]
    pub fn dummy() -> Self {
        Self { file_id: FileId(0), start: 0, end: 0 }
    }

    /// Returns the byte length of this span.
    #[inline]
    #[must_use]
    pub fn len(self) -> u32 {
        self.end - self.start
    }

    /// Returns `true` if this span is empty (zero length).
    #[inline]
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Returns `true` if the given byte offset falls within this span.
    #[inline]
    #[must_use]
    pub fn contains(self, offset: u32) -> bool {
        self.start <= offset && offset < self.end
    }
}

/// A 1-based line and column position in a source file.
///
/// Column is measured in bytes from the start of the line (matching rustc
/// convention), not Unicode characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineCol {
    /// 1-based line number.
    pub line: u32,
    /// 1-based column number (byte offset from line start).
    pub column: u32,
}

/// Pre-computed index of line start byte offsets for efficient byte→line/column
/// conversion.
///
/// Build once per source file via [`LineIndex::new`], then call
/// [`LineIndex::line_col`] for each byte offset. Used by the CLI for JSON
/// diagnostic output and by the future LSP.
#[derive(Debug, Clone)]
pub struct LineIndex {
    /// Byte offsets of each line start. `line_starts[0]` is always 0.
    line_starts: Vec<u32>,
    /// Total length of the source in bytes.
    source_len: u32,
}

impl LineIndex {
    /// Build a line index from source text.
    #[must_use]
    pub fn new(source: &str) -> Self {
        let mut line_starts = vec![0u32];
        for (i, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                #[allow(clippy::cast_possible_truncation)]
                line_starts.push((i + 1) as u32);
            }
        }
        #[allow(clippy::cast_possible_truncation)]
        let source_len = source.len() as u32;
        Self { line_starts, source_len }
    }

    /// Convert a byte offset to a 1-based line and column.
    ///
    /// Returns `None` if the offset is beyond the source length.
    #[must_use]
    pub fn line_col(&self, offset: u32) -> Option<LineCol> {
        if offset > self.source_len {
            return None;
        }
        // partition_point returns the first index where line_start > offset,
        // so we subtract 1 to get the line that contains this offset.
        let line_idx = self.line_starts.partition_point(|&start| start <= offset);
        if line_idx == 0 {
            return None;
        }
        let line_idx = line_idx - 1;
        let line_start = self.line_starts[line_idx];
        // line_idx is bounded by the number of lines in source (well within u32
        // range for any realistic file), so truncation cannot happen in practice.
        #[allow(clippy::cast_possible_truncation)]
        let line = (line_idx as u32) + 1;
        Some(LineCol { line, column: (offset - line_start) + 1 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_index_single_line() {
        let idx = LineIndex::new("hello");
        assert_eq!(idx.line_col(0), Some(LineCol { line: 1, column: 1 }));
        assert_eq!(idx.line_col(4), Some(LineCol { line: 1, column: 5 }));
    }

    #[test]
    fn line_index_multi_line() {
        let idx = LineIndex::new("ab\ncd\nef");
        assert_eq!(idx.line_col(0), Some(LineCol { line: 1, column: 1 }));
        assert_eq!(idx.line_col(2), Some(LineCol { line: 1, column: 3 })); // 'b' → '\n'
        assert_eq!(idx.line_col(3), Some(LineCol { line: 2, column: 1 }));
        assert_eq!(idx.line_col(6), Some(LineCol { line: 3, column: 1 }));
        assert_eq!(idx.line_col(7), Some(LineCol { line: 3, column: 2 }));
    }

    #[test]
    fn line_index_empty() {
        let idx = LineIndex::new("");
        assert_eq!(idx.line_col(0), Some(LineCol { line: 1, column: 1 }));
    }

    #[test]
    fn line_index_trailing_newline() {
        let idx = LineIndex::new("abc\n");
        assert_eq!(idx.line_col(3), Some(LineCol { line: 1, column: 4 })); // '\n' itself
        assert_eq!(idx.line_col(4), Some(LineCol { line: 2, column: 1 })); // after '\n'
    }

    #[test]
    fn line_index_out_of_range() {
        let idx = LineIndex::new("hi");
        // offset 2 is one-past-end, still valid (end-of-file position)
        assert_eq!(idx.line_col(2), Some(LineCol { line: 1, column: 3 }));
        // offset 100 is way beyond source
        assert_eq!(idx.line_col(100), None);
    }

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
