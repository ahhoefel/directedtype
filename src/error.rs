use crate::span::{LineIndex, Span};
use std::fmt;
use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq)]
pub enum ParseError {
    #[error("Expected {expected}, found {found}")]
    UnexpectedToken {
        expected: String,
        found: String,
        span: Span,
    },

    #[error("Unexpected end of input, expected {expected}")]
    UnexpectedEof {
        expected: String,
        span: Span,
    },

    #[error("Invalid escape sequence '{escape}'")]
    InvalidEscape {
        escape: String,
        span: Span,
    },

    #[error("Unclosed delimiter '{delimiter}', opened at {open_span:?}")]
    UnclosedDelimiter {
        delimiter: char,
        open_span: Span,
    },

    #[error("Invalid or unrecognized token")]
    LexError {
        span: Span,
    },

    #[error("{message}")]
    Custom {
        message: String,
        span: Span,
    },
}

impl ParseError {
    pub fn span(&self) -> Span {
        match self {
            ParseError::UnexpectedToken { span, .. }
            | ParseError::UnexpectedEof { span, .. }
            | ParseError::InvalidEscape { span, .. }
            | ParseError::LexError { span }
            | ParseError::Custom { span, .. } => *span,
            ParseError::UnclosedDelimiter { open_span, .. } => *open_span,
        }
    }

    /// Formats the error with human-readable line and column numbers.
    pub fn display_with_source<'a>(&'a self, source: &'a str) -> FormattedParseError<'a> {
        FormattedParseError {
            error: self,
            source,
        }
    }
}

pub struct FormattedParseError<'a> {
    error: &'a ParseError,
    source: &'a str,
}

impl<'a> fmt::Display for FormattedParseError<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let index = LineIndex::new(self.source);
        let span = self.error.span();
        let loc = index.location(span.start);
        write!(
            f,
            "Parse error at line {}, column {}: {}",
            loc.line, loc.column, self.error
        )
    }
}
