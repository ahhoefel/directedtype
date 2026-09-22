use std::fmt;

/// Represents a half-open byte range `[start, end)` in the source text.
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub const fn empty(pos: usize) -> Self {
        Self {
            start: pos,
            end: pos,
        }
    }

    /// Extends this span to encompass `other`.
    pub fn merge(self, other: Span) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    pub fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(self) -> bool {
        self.start >= self.end
    }
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

impl From<std::ops::Range<usize>> for Span {
    fn from(range: std::ops::Range<usize>) -> Self {
        Self::new(range.start, range.end)
    }
}

/// A line and column location in human-readable (1-indexed) format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLocation {
    pub line: usize,
    pub column: usize,
}

/// Helper to compute line and column numbers from byte offsets.
pub struct LineIndex<'a> {
    source: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub fn new(source: &'a str) -> Self {
        let mut line_starts = vec![0];
        for (i, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self {
            source,
            line_starts,
        }
    }

    pub fn location(&self, byte_offset: usize) -> SourceLocation {
        let offset = byte_offset.min(self.source.len());
        let line_idx = match self.line_starts.binary_search(&offset) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        };
        let line_start = self.line_starts[line_idx];
        let col = offset - line_start + 1;
        SourceLocation {
            line: line_idx + 1,
            column: col,
        }
    }
}
