use std::fmt;
use std::sync::{OnceLock, RwLock};

static FILE_INTERNER: OnceLock<RwLock<Vec<String>>> = OnceLock::new();

pub fn intern_file_name(name: &str) -> u32 {
    if name.is_empty() {
        return 0;
    }
    let lock = FILE_INTERNER.get_or_init(|| RwLock::new(Vec::new()));
    if let Ok(files) = lock.read() {
        if let Some(idx) = files.iter().position(|f| f == name) {
            return (idx + 1) as u32;
        }
    }
    if let Ok(mut files) = lock.write() {
        if let Some(idx) = files.iter().position(|f| f == name) {
            return (idx + 1) as u32;
        }
        files.push(name.to_string());
        return files.len() as u32;
    }
    0
}

pub fn lookup_file_name(file_id: u32) -> Option<String> {
    if file_id == 0 {
        return None;
    }
    let lock = FILE_INTERNER.get()?;
    let files = lock.read().ok()?;
    files.get((file_id - 1) as usize).cloned()
}

/// Represents a half-open byte range `[start, end)` in the source text,
/// along with optional 1-indexed line and column numbers (`0` when unset)
/// and an optional interned source file ID (`0` when unset).
#[derive(Clone, Copy, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub column: usize,
    pub file_id: u32,
}

impl PartialEq for Span {
    fn eq(&self, other: &Self) -> bool {
        self.start == other.start && self.end == other.end
    }
}

impl Eq for Span {}

impl std::hash::Hash for Span {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.start.hash(state);
        self.end.hash(state);
    }
}

impl Span {
    pub const fn new(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            line: 0,
            column: 0,
            file_id: 0,
        }
    }

    pub const fn with_location(start: usize, end: usize, line: usize, column: usize) -> Self {
        Self {
            start,
            end,
            line,
            column,
            file_id: 0,
        }
    }

    pub const fn with_file_location(
        start: usize,
        end: usize,
        line: usize,
        column: usize,
        file_id: u32,
    ) -> Self {
        Self {
            start,
            end,
            line,
            column,
            file_id,
        }
    }

    pub const fn empty(pos: usize) -> Self {
        Self {
            start: pos,
            end: pos,
            line: 0,
            column: 0,
            file_id: 0,
        }
    }

    pub fn file_path(self) -> Option<String> {
        lookup_file_name(self.file_id)
    }

    pub fn file_name(self) -> Option<String> {
        lookup_file_name(self.file_id).map(|p| {
            std::path::Path::new(&p)
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
                .unwrap_or(p)
        })
    }

    /// Formats the span's source location as `" (file:line:col)"`, `" (line:col)"`, or `""` if unknown.
    pub fn format_location_suffix(self) -> String {
        if self.line > 0 {
            match self.file_name() {
                Some(file) => format!(" ({}:{}:{})", file, self.line, self.column),
                None => format!(" ({}:{})", self.line, self.column),
            }
        } else {
            String::new()
        }
    }

    /// Extends this span to encompass `other`.
    pub fn merge(self, other: Span) -> Self {
        let (start, line, column, file_id) = if self.start < other.start {
            (
                self.start,
                self.line,
                self.column,
                if self.file_id > 0 { self.file_id } else { other.file_id },
            )
        } else if other.start < self.start {
            (
                other.start,
                other.line,
                other.column,
                if other.file_id > 0 { other.file_id } else { self.file_id },
            )
        } else if self.line > 0 {
            (
                self.start,
                self.line,
                self.column,
                if self.file_id > 0 { self.file_id } else { other.file_id },
            )
        } else {
            (
                self.start,
                other.line,
                other.column,
                if other.file_id > 0 { other.file_id } else { self.file_id },
            )
        };
        Self {
            start,
            end: self.end.max(other.end),
            line,
            column,
            file_id,
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
        if self.line > 0 {
            if let Some(file) = self.file_name() {
                write!(
                    f,
                    "{}..{}({}:{}:{})",
                    self.start, self.end, file, self.line, self.column
                )
            } else {
                write!(f, "{}..{}({}:{})", self.start, self.end, self.line, self.column)
            }
        } else {
            write!(f, "{}..{}", self.start, self.end)
        }
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
