use crate::error::ParseError;
use crate::span::{LineIndex, Span};
use crate::token::Token;
use logos::Logos;

pub struct Lexer<'a> {
    source: &'a str,
    line_index: LineIndex<'a>,
    inner: logos::Lexer<'a, Token>,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            line_index: LineIndex::new(source),
            inner: Token::lexer(source),
        }
    }

    pub fn source(&self) -> &'a str {
        self.source
    }

    pub fn next_token(&mut self) -> Result<Option<(Token, Span)>, ParseError> {
        match self.inner.next() {
            Some(Ok(tok)) => {
                let span = self.current_span();
                Ok(Some((tok, span)))
            }
            Some(Err(())) => {
                let span = self.current_span();
                Err(ParseError::LexError { span })
            }
            None => Ok(None),
        }
    }

    pub fn current_span(&self) -> Span {
        let range = self.inner.span();
        let loc = self.line_index.location(range.start);
        Span::with_location(range.start, range.end, loc.line, loc.column)
    }

    pub fn remainder(&self) -> &'a str {
        self.inner.remainder()
    }

    pub fn bump(&mut self, n: usize) {
        self.inner.bump(n);
    }
}
