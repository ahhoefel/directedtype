pub mod component;
pub mod cursor;
pub mod expr;
pub mod node;

use crate::ast::{Document, Item};
use crate::error::ParseError;
use crate::parser::component::{parse_component_def, parse_let_binding};
use crate::parser::cursor::ParserCursor;
use crate::parser::node::parse_element_node;
use crate::span::Span;
use crate::token::Token;

/// Parses a full DirectedType source string into an un-evaluated AST `Document`.
pub fn parse_document(source: &str) -> Result<Document, ParseError> {
    let mut cursor = ParserCursor::new(source);
    let mut items = Vec::new();
    let start_pos = cursor.pos();

    while let Some((tok, span)) = cursor.peek_token()?.cloned() {
        match tok {
            Token::Let => {
                let let_binding = parse_let_binding(&mut cursor)?;
                items.push(Item::Let(let_binding));
            }
            Token::Backslash => {
                let next_tok = cursor.peek_nth(1)?.cloned();
                match next_tok {
                    Some((Token::Component, _)) => {
                        let comp = parse_component_def(&mut cursor)?;
                        items.push(Item::Component(comp));
                    }
                    Some((Token::Ident(_), _)) | Some((Token::Children, _)) => {
                        let node = parse_element_node(&mut cursor)?;
                        items.push(Item::Node(node));
                    }
                    Some((other, other_span)) => {
                        return Err(ParseError::UnexpectedToken {
                            expected: "Component or element name after '\\'".to_string(),
                            found: other.to_string(),
                            span: other_span,
                        });
                    }
                    None => {
                        return Err(ParseError::UnexpectedEof {
                            expected: "Component or element name after '\\'".to_string(),
                            span,
                        });
                    }
                }
            }
            other => {
                return Err(ParseError::UnexpectedToken {
                    expected: "top-level declaration starting with '\\' or 'let'".to_string(),
                    found: other.to_string(),
                    span,
                });
            }
        }
    }

    let end_pos = cursor.pos();
    let doc_span = Span::new(start_pos, end_pos);

    Ok(Document {
        items,
        span: doc_span,
    })
}
