use crate::ast::{ContentItem, ContentSlot, TextChunk};
use crate::error::ParseError;
use crate::span::{intern_file_name, LineIndex, Span};
use crate::token::Token;
use logos::Logos;
use std::collections::VecDeque;

pub struct ParserCursor<'a> {
    source: &'a str,
    line_index: LineIndex<'a>,
    file_id: u32,
    pos: usize,
    peeked: VecDeque<(Token, Span)>,
}

/// Specifies the parsing mode for a block / content slot enclosed in `{ ... }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotMode {
    /// In a component definition: text is forbidden, top-level declarations like \state and \env are allowed
    ComponentDef,
    /// In an element content slot: text is allowed, \state and \env are forbidden
    ContentSlot,
}

impl<'a> ParserCursor<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            line_index: LineIndex::new(source),
            file_id: 0,
            pos: 0,
            peeked: VecDeque::new(),
        }
    }

    pub fn with_file(source: &'a str, file_name: &str) -> Self {
        Self {
            source,
            line_index: LineIndex::new(source),
            file_id: intern_file_name(file_name),
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

    pub fn span(&self, start: usize, end: usize) -> Span {
        let loc = self.line_index.location(start);
        Span::with_file_location(start, end, loc.line, loc.column, self.file_id)
    }

    pub fn empty_span(&self, pos: usize) -> Span {
        self.span(pos, pos)
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
                    let span = self.span(start_pos + range.start, start_pos + range.end);
                    self.pos = span.end;
                    self.peeked.push_back((tok, span));
                }
                Some(Err(())) => {
                    let span = self.span(start_pos, (start_pos + 1).min(self.source.len()));
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
        let span = self.empty_span(self.pos);
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

    /// Parses a content slot or component definition `{ ... }`, calling `parse_inline` when an inline item starting with `\` is encountered.
    pub fn parse_content_slot_mode<F>(
        &mut self,
        mode: SlotMode,
        mut parse_inline: F,
    ) -> Result<ContentSlot, ParseError>
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

        let file_id = self.file_id;
        let flush_text = |line_index: &LineIndex<'_>,
                          buf: &mut String,
                          items: &mut Vec<ContentItem>,
                          start: usize,
                          end: usize,
                          is_end: bool| {
            if buf.is_empty() {
                return;
            }
            let pieces = split_on_paragraph_breaks(buf);
            let num_pieces = pieces.len();

            for (idx, piece) in pieces.into_iter().enumerate() {
                match piece {
                    ParagraphPiece::Break(rel_start, rel_end) => {
                        // Trim any trailing unescaped space on the immediately preceding Text item
                        if let Some(ContentItem::Text(prev_text)) = items.last_mut() {
                            let trimmed_len = prev_text.text.trim_ascii_end().len();
                            prev_text.text.truncate(trimmed_len);
                            if prev_text.text.is_empty() {
                                items.pop();
                            }
                        }
                        let has_visual = items.iter().any(is_visual_content_item);
                        let already_break = matches!(items.last(), Some(ContentItem::ParagraphBreak(_)));
                        if has_visual && !already_break {
                            let abs_start = (start + rel_start).min(end);
                            let abs_end = (start + rel_end).min(end);
                            let loc = line_index.location(abs_start);
                            let brk_span = Span::with_file_location(
                                abs_start,
                                abs_end,
                                loc.line,
                                loc.column,
                                file_id,
                            );
                            items.push(ContentItem::ParagraphBreak(brk_span));
                        }
                    }
                    ParagraphPiece::Segment(seg_raw, rel_start, rel_end) => {
                        let abs_start = (start + rel_start).min(end);
                        let abs_end = (start + rel_end).min(end);
                        let loc = line_index.location(abs_start);
                        let seg_span = Span::with_file_location(
                            abs_start,
                            abs_end,
                            loc.line,
                            loc.column,
                            file_id,
                        );

                        let is_first_piece = idx == 0;
                        let is_last_piece = idx + 1 == num_pieces;

                        let prev_is_visual = items.last().is_some_and(is_visual_content_item);
                        let prev_text_ends_with_space = matches!(
                            items.last(),
                            Some(ContentItem::Text(t)) if t.text.ends_with(' ')
                        );

                        let mut slice = seg_raw;
                        if !is_first_piece || !prev_is_visual || prev_text_ends_with_space {
                            slice = slice.trim_ascii_start();
                        }
                        if !is_last_piece || is_end {
                            slice = slice.trim_ascii_end();
                        }

                        if slice.is_empty() {
                            continue;
                        }

                        let normalized = normalize_ascii_whitespace(slice);
                        if !normalized.is_empty() {
                            if let Some(ContentItem::Text(prev_text)) = items.last_mut() {
                                if prev_text.text.ends_with(' ') && normalized.starts_with(' ') {
                                    prev_text.text.push_str(&normalized[1..]);
                                } else {
                                    prev_text.text.push_str(&normalized);
                                }
                                prev_text.span = prev_text.span.merge(seg_span);
                            } else {
                                items.push(ContentItem::Text(TextChunk {
                                    text: normalized,
                                    span: seg_span,
                                }));
                            }
                        }
                    }
                }
            }
            buf.clear();
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
                if mode == SlotMode::ContentSlot {
                    flush_text(
                        &self.line_index,
                        &mut raw_buf,
                        &mut items,
                        text_start,
                        self.pos,
                        true,
                    );
                    finalize_content_slot_items(&mut items);
                }
                let close_span = self.span(self.pos, self.pos + 1);
                self.reset_pos(self.pos + 1);

                let slot_span = open_span.merge(close_span);
                return Ok(ContentSlot {
                    items,
                    span: slot_span,
                });
            }

            // Single line comment `//`
            if rem.starts_with("//") {
                self.pos += 2;
                while self.pos < self.source.len() && self.source.as_bytes()[self.pos] != b'\n' {
                    self.pos += 1;
                }
                if self.pos < self.source.len() && self.source.as_bytes()[self.pos] == b'\n' {
                    self.pos += 1;
                    if mode == SlotMode::ContentSlot {
                        // Consume leading horizontal indentation on the line following `//`
                        while self.pos < self.source.len() {
                            let b = self.source.as_bytes()[self.pos];
                            if b == b' ' || b == b'\t' || b == b'\r' {
                                self.pos += 1;
                            } else {
                                break;
                            }
                        }
                        // If the next line is a blank line, preserve the comment line's newline
                        // so that the blank line triggers a ParagraphBreak.
                        if self.pos < self.source.len() && self.source.as_bytes()[self.pos] == b'\n'
                        {
                            raw_buf.push('\n');
                        }
                    }
                }
                if mode == SlotMode::ComponentDef {
                    text_start = self.pos;
                }
                continue;
            }

            // Multi-line comment `/* ... */`
            if rem.starts_with("/*") {
                self.pos += 2;
                while self.pos + 1 < self.source.len()
                    && !self.source[self.pos..].starts_with("*/")
                {
                    self.pos += 1;
                }
                if self.source[self.pos..].starts_with("*/") {
                    self.pos += 2;
                }
                if mode == SlotMode::ComponentDef {
                    text_start = self.pos;
                }
                continue;
            }

            // In ComponentDef mode, skip whitespace between items
            if mode == SlotMode::ComponentDef {
                let first_char = rem.chars().next().unwrap();
                if first_char.is_whitespace() {
                    self.pos += first_char.len_utf8();
                    text_start = self.pos;
                    continue;
                }
            }

            // Backslash: check for TeX escape or inline item
            if let Some(after_backslash) = rem.strip_prefix('\\') {
                if mode == SlotMode::ContentSlot {
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
                    } else if after_backslash.starts_with(' ') {
                        raw_buf.push(ESCAPED_SPACE_SENTINEL);
                        self.pos += 2;
                        continue;
                    } else if after_backslash.starts_with('~') {
                        raw_buf.push('~');
                        self.pos += 2;
                        continue;
                    } else if after_backslash.starts_with('/') {
                        raw_buf.push('/');
                        self.pos += 2;
                        continue;
                    } else if after_backslash.starts_with('*') {
                        raw_buf.push('*');
                        self.pos += 2;
                        continue;
                    }
                }

                if after_backslash
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                {
                    if mode == SlotMode::ContentSlot {
                        flush_text(
                            &self.line_index,
                            &mut raw_buf,
                            &mut items,
                            text_start,
                            self.pos,
                            false,
                        );
                    }
                    self.reset_pos(self.pos);
                    let inline_item = parse_inline(self)?;
                    let item_end = inline_item.span().end;
                    items.push(inline_item);
                    self.reset_pos(item_end);
                    text_start = self.pos;
                    continue;
                } else if mode == SlotMode::ContentSlot {
                    let span = self.span(self.pos, self.pos + 2.min(rem.len()));
                    return Err(ParseError::InvalidEscape {
                        escape: rem[..2.min(rem.len())].to_string(),
                        span,
                    });
                } else {
                    let span = self.span(self.pos, self.pos + 1);
                    return Err(ParseError::UnexpectedToken {
                        expected: "let, env, alias, state, assert, Children, or element name after '\\'".to_string(),
                        found: rem[..1].to_string(),
                        span,
                    });
                }
            }

            // In ComponentDef mode, any non-whitespace, non-backslash character is illegal raw text!
            if mode == SlotMode::ComponentDef {
                let mut end = self.pos;
                while end < self.source.len() {
                    let c = self.source[end..].chars().next().unwrap();
                    if c.is_whitespace() || c == '\\' || c == '}' {
                        break;
                    }
                    end += c.len_utf8();
                }
                let span = self.span(self.pos, end);
                return Err(ParseError::UnexpectedToken {
                    expected: "'\\' before command or element in component body".to_string(),
                    found: self.source[self.pos..end].to_string(),
                    span,
                });
            }

            // Unescaped `{` in raw text is an error
            if rem.starts_with('{') {
                let span = self.span(self.pos, self.pos + 1);
                return Err(ParseError::Custom {
                    message: "Unescaped '{' in content text; use '\\{' to include a literal brace"
                        .to_string(),
                    span,
                });
            }

            // Tilde `~` in ContentSlot mode produces a non-breaking space (U+00A0)
            if rem.starts_with('~') {
                raw_buf.push('\u{00A0}');
                self.pos += 1;
                continue;
            }

            // Regular text character (in ContentSlot mode)
            let ch = rem.chars().next().unwrap();
            raw_buf.push(ch);
            self.pos += ch.len_utf8();
        }
    }

    /// Parses a content slot `{ ... }`, calling `parse_inline` when an inline node `\Ident` is encountered.
    pub fn parse_content_slot<F>(&mut self, parse_inline: F) -> Result<ContentSlot, ParseError>
    where
        F: FnMut(&mut Self) -> Result<ContentItem, ParseError>,
    {
        self.parse_content_slot_mode(SlotMode::ContentSlot, parse_inline)
    }
}

/// Sentinel character used internally during content slot parsing to represent an escaped space (`\ `)
/// so that it is preserved across ASCII whitespace trimming before being converted to `' '`.
const ESCAPED_SPACE_SENTINEL: char = '\u{E000}';

fn is_visual_content_item(item: &ContentItem) -> bool {
    match item {
        ContentItem::Text(_) | ContentItem::Node(_) | ContentItem::Children(_) => true,
        ContentItem::Let(l) => matches!(l.value, Some(crate::ast::LetValue::Node(_))),
        ContentItem::ParagraphBreak(_)
        | ContentItem::Alias(_)
        | ContentItem::State(_)
        | ContentItem::Env(_)
        | ContentItem::Assert(_) => false,
    }
}

enum ParagraphPiece<'a> {
    Segment(&'a str, usize, usize),
    Break(usize, usize),
}

/// Splits a raw text buffer on contiguous ASCII whitespace runs containing two or more newlines (`\n`).
fn split_on_paragraph_breaks(input: &str) -> Vec<ParagraphPiece<'_>> {
    let mut pieces = Vec::new();
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut seg_start = 0;
    let mut i = 0;

    while i < len {
        if bytes[i].is_ascii_whitespace() {
            let ws_start = i;
            let mut newline_count = 0;
            while i < len && bytes[i].is_ascii_whitespace() {
                if bytes[i] == b'\n' {
                    newline_count += 1;
                }
                i += 1;
            }
            let ws_end = i;
            if newline_count >= 2 {
                if seg_start < ws_start {
                    pieces.push(ParagraphPiece::Segment(
                        &input[seg_start..ws_start],
                        seg_start,
                        ws_start,
                    ));
                }
                pieces.push(ParagraphPiece::Break(ws_start, ws_end));
                seg_start = ws_end;
            }
        } else {
            i += 1;
        }
    }

    if seg_start < len {
        pieces.push(ParagraphPiece::Segment(
            &input[seg_start..len],
            seg_start,
            len,
        ));
    }

    pieces
}

/// Collapses any contiguous sequence of ASCII whitespace characters into a single space `' '`.
/// Non-ASCII whitespace (such as `\u{00A0}` from `~`) and `ESCAPED_SPACE_SENTINEL` are preserved.
fn normalize_ascii_whitespace(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut in_ws = false;
    for c in input.chars() {
        if c.is_ascii_whitespace() {
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

/// Cleans up trailing paragraph breaks, trailing unescaped spaces on the last visual text chunk,
/// and converts `ESCAPED_SPACE_SENTINEL` back to `' '`.
fn finalize_content_slot_items(items: &mut Vec<ContentItem>) {
    // 1. Remove any ParagraphBreak that appears after the last visual item
    while let Some(pos) = items
        .iter()
        .rposition(|item| matches!(item, ContentItem::ParagraphBreak(_)))
    {
        let has_visual_after = items[pos + 1..].iter().any(is_visual_content_item);
        if !has_visual_after {
            items.remove(pos);
        } else {
            break;
        }
    }

    // 2. If the last visual item is a Text chunk (e.g. followed only by non-visual directives),
    // strip any trailing unescaped ASCII whitespace from it.
    if let Some(last_visual_idx) = items.iter().rposition(is_visual_content_item) {
        if let ContentItem::Text(chunk) = &mut items[last_visual_idx] {
            let trimmed_len = chunk.text.trim_ascii_end().len();
            chunk.text.truncate(trimmed_len);
            if chunk.text.is_empty() {
                items.remove(last_visual_idx);
            }
        }
    }

    // 3. Replace ESCAPED_SPACE_SENTINEL with literal ' ' in all Text chunks
    for item in items.iter_mut() {
        if let ContentItem::Text(chunk) = item {
            if chunk.text.contains(ESCAPED_SPACE_SENTINEL) {
                chunk.text = chunk.text.replace(ESCAPED_SPACE_SENTINEL, " ");
            }
        }
    }
}
