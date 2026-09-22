use crate::ast::{ContentItem, ContentSlot, TextChunk};
use crate::error::ParseError;
use crate::span::Span;
use crate::token::Token;
use logos::Logos;
use std::collections::VecDeque;

pub struct ParserCursor<'a> {
    source: &'a str,
    pos: usize,
    peeked: VecDeque<(Token, Span)>,
}

impl<'a> ParserCursor<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            pos: 0,
            peeked: VecDeque::new(),
        }
    }

    pub fn source(&self) -> &'a str {
        self.source
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Returns the logical current position, accounting for peeked tokens in the queue.
    pub fn current_pos(&self) -> usize {
        if let Some((_, span)) = self.peeked.front() {
            span.start
        } else {
            self.pos
        }
    }

    /// Skips whitespace and comments in token mode.
    fn skip_whitespace_and_comments(&mut self) {
        let bytes = self.source.as_bytes();
        while self.pos < bytes.len() {
            if bytes[self.pos].is_ascii_whitespace() {
                self.pos += 1;
                continue;
            }
            if self.source[self.pos..].starts_with("//") {
                self.pos += 2;
                while self.pos < bytes.len() && bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }
                if self.pos < bytes.len() && bytes[self.pos] == b'\n' {
                    self.pos += 1;
                }
                continue;
            }
            if self.source[self.pos..].starts_with("/*") {
                self.pos += 2;
                while self.pos + 1 < bytes.len() && !self.source[self.pos..].starts_with("*/") {
                    self.pos += 1;
                }
                if self.source[self.pos..].starts_with("*/") {
                    self.pos += 2;
                }
                continue;
            }
            break;
        }
    }

    fn fill_peek(&mut self, n: usize) -> Result<(), ParseError> {
        while self.peeked.len() < n {
            self.skip_whitespace_and_comments();
            if self.pos >= self.source.len() {
                break;
            }
            let start_pos = self.pos;
            let mut lexer = Token::lexer(&self.source[self.pos..]);
            match lexer.next() {
                Some(Ok(tok)) => {
                    let range = lexer.span();
                    let span = Span::new(start_pos + range.start, start_pos + range.end);
                    self.pos = span.end;
                    self.peeked.push_back((tok, span));
                }
                Some(Err(())) => {
                    let span = Span::new(start_pos, (start_pos + 1).min(self.source.len()));
                    return Err(ParseError::LexError { span });
                }
                None => break,
            }
        }
        Ok(())
    }

    pub fn peek_token(&mut self) -> Result<Option<&(Token, Span)>, ParseError> {
        self.fill_peek(1)?;
        Ok(self.peeked.front())
    }

    pub fn peek_nth(&mut self, n: usize) -> Result<Option<&(Token, Span)>, ParseError> {
        self.fill_peek(n + 1)?;
        Ok(self.peeked.get(n))
    }

    pub fn next_token(&mut self) -> Result<Option<(Token, Span)>, ParseError> {
        self.fill_peek(1)?;
        Ok(self.peeked.pop_front())
    }

    pub fn expect_token(&mut self, expected_desc: &str) -> Result<(Token, Span), ParseError> {
        let span = Span::empty(self.pos);
        match self.next_token()? {
            Some(tok) => Ok(tok),
            None => Err(ParseError::UnexpectedEof {
                expected: expected_desc.to_string(),
                span,
            }),
        }
    }

    pub fn consume_token(&mut self, expected: &Token) -> Result<Span, ParseError> {
        let (tok, span) = self.expect_token(&expected.to_string())?;
        if &tok == expected {
            Ok(span)
        } else {
            Err(ParseError::UnexpectedToken {
                expected: expected.to_string(),
                found: tok.to_string(),
                span,
            })
        }
    }

    /// Resets the lexer buffer and sets the cursor position.
    pub fn reset_pos(&mut self, pos: usize) {
        self.pos = pos;
        self.peeked.clear();
    }

    /// Parses a content slot `{ ... }`, calling `parse_inline` when an inline node `\Ident` is encountered.
    pub fn parse_content_slot<F>(&mut self, mut parse_inline: F) -> Result<ContentSlot, ParseError>
    where
        F: FnMut(&mut Self) -> Result<ContentItem, ParseError>,
    {
        // Consume opening brace `{`
        let open_span = self.consume_token(&Token::LBrace)?;
        let current_pos = open_span.end;
        self.reset_pos(current_pos);

        let mut items = Vec::new();
        let mut raw_buf = String::new();
        let mut text_start = self.pos;

        let flush_text = |buf: &mut String,
                          items: &mut Vec<ContentItem>,
                          start: usize,
                          end: usize,
                          is_end: bool| {
            if buf.is_empty() {
                return;
            }
            let normalized = normalize_whitespace(buf);
            buf.clear();

            // Discard pure whitespace chunks between elements or at boundaries
            if normalized.trim().is_empty() {
                return;
            }

            let trimmed = if items.is_empty() && is_end {
                normalized.trim().to_string()
            } else if items.is_empty() {
                normalized.trim_start().to_string()
            } else if is_end {
                normalized.trim_end().to_string()
            } else {
                normalized
            };

            if !trimmed.is_empty() {
                items.push(ContentItem::Text(TextChunk {
                    text: trimmed,
                    span: Span::new(start, end),
                }));
            }
        };

        loop {
            if self.pos >= self.source.len() {
                return Err(ParseError::UnclosedDelimiter {
                    delimiter: '}',
                    open_span,
                });
            }

            let rem = &self.source[self.pos..];

            // Closing brace ends the content slot
            if rem.starts_with('}') {
                flush_text(&mut raw_buf, &mut items, text_start, self.pos, true);
                let close_span = Span::new(self.pos, self.pos + 1);
                self.reset_pos(self.pos + 1);

                let slot_span = open_span.merge(close_span);
                return Ok(ContentSlot {
                    items,
                    span: slot_span,
                });
            }

            // Single line comment `//`
            if rem.starts_with("//") {
                flush_text(&mut raw_buf, &mut items, text_start, self.pos, false);
                self.pos += 2;
                while self.pos < self.source.len() && self.source.as_bytes()[self.pos] != b'\n' {
                    self.pos += 1;
                }
                if self.pos < self.source.len() && self.source.as_bytes()[self.pos] == b'\n' {
                    self.pos += 1;
                }
                text_start = self.pos;
                continue;
            }

            // Multi-line comment `/* ... */`
            if rem.starts_with("/*") {
                flush_text(&mut raw_buf, &mut items, text_start, self.pos, false);
                self.pos += 2;
                while self.pos + 1 < self.source.len()
                    && !self.source[self.pos..].starts_with("*/")
                {
                    self.pos += 1;
                }
                if self.source[self.pos..].starts_with("*/") {
                    self.pos += 2;
                }
                text_start = self.pos;
                continue;
            }

            // Backslash: check for TeX escape or inline node
            if let Some(after_backslash) = rem.strip_prefix('\\') {
                if after_backslash.starts_with('\\') {
                    raw_buf.push('\\');
                    self.pos += 2;
                    continue;
                } else if after_backslash.starts_with('{') {
                    raw_buf.push('{');
                    self.pos += 2;
                    continue;
                } else if after_backslash.starts_with('}') {
                    raw_buf.push('}');
                    self.pos += 2;
                    continue;
                } else if after_backslash
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                {
                    flush_text(&mut raw_buf, &mut items, text_start, self.pos, false);
                    self.reset_pos(self.pos);
                    let inline_item = parse_inline(self)?;
                    items.push(inline_item);
                    self.pos = self.current_pos();
                    self.peeked.clear();
                    text_start = self.pos;
                    continue;
                } else {
                    let span = Span::new(self.pos, self.pos + 2.min(rem.len()));
                    return Err(ParseError::InvalidEscape {
                        escape: rem[..2.min(rem.len())].to_string(),
                        span,
                    });
                }
            }

            // Unescaped `{` in raw text is an error
            if rem.starts_with('{') {
                let span = Span::new(self.pos, self.pos + 1);
                return Err(ParseError::Custom {
                    message: "Unescaped '{' in content text; use '\\{' to include a literal brace"
                        .to_string(),
                    span,
                });
            }

            // Regular text character
            let ch = rem.chars().next().unwrap();
            raw_buf.push(ch);
            self.pos += ch.len_utf8();
        }
    }
}

/// Collapses any sequence of whitespace characters into a single space `' '`.
fn normalize_whitespace(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut in_ws = false;
    for c in input.chars() {
        if c.is_whitespace() {
            if !in_ws {
                result.push(' ');
                in_ws = true;
            }
        } else {
            result.push(c);
            in_ws = false;
        }
    }
    result
}
