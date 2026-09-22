use crate::error::ParseError;
use crate::span::Span;
use crate::token::Token;
use logos::Logos;

pub struct Lexer<'a> {
    source: &'a str,
    inner: logos::Lexer<'a, Token>,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            inner: Token::lexer(source),
        }
    }

    pub fn source(&self) -> &'a str {
        self.source
    }

    pub fn next_token(&mut self) -> Result<Option<(Token, Span)>, ParseError> {
        match self.inner.next() {
            Some(Ok(tok)) => {
                let span = Span::from(self.inner.span());
                Ok(Some((tok, span)))
            }
            Some(Err(())) => {
                let span = Span::from(self.inner.span());
                Err(ParseError::LexError { span })
            }
            None => Ok(None),
        }
    }

    pub fn current_span(&self) -> Span {
        Span::from(self.inner.span())
    }

    pub fn remainder(&self) -> &'a str {
        self.inner.remainder()
    }

    pub fn bump(&mut self, n: usize) {
        self.inner.bump(n);
    }
}
