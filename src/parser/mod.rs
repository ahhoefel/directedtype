pub mod component;
pub mod cursor;
pub mod expr;
pub mod node;

use crate::ast::{Document, Item};
use crate::error::ParseError;
use crate::parser::component::{
    parse_component_def, parse_enum_def, parse_env_binding, parse_let_binding, parse_state_binding,
    parse_use_declaration,
};
use crate::parser::cursor::ParserCursor;
use crate::parser::node::parse_element_node;
use crate::token::Token;

/// Parses a full DirectedType source string into an un-evaluated AST `Document`.
pub fn parse_document(source: &str) -> Result<Document, ParseError> {
    parse_with_cursor(ParserCursor::new(source))
}

/// Parses a full DirectedType source string with an associated file path into an AST `Document`.
pub fn parse_document_with_file(
    source: &str,
    path: &std::path::Path,
) -> Result<Document, ParseError> {
    let path_str = path.to_str().unwrap_or_default();
    let cursor = if path_str.is_empty() {
        ParserCursor::new(source)
    } else {
        ParserCursor::with_file(source, path_str)
    };
    parse_with_cursor(cursor)
}

fn parse_with_cursor(mut cursor: ParserCursor<'_>) -> Result<Document, ParseError> {
    let mut items = Vec::new();
    let start_pos = cursor.pos();

    while let Some((tok, span)) = cursor.peek_token()?.cloned() {
        match tok {
            Token::Backslash => {
                let next_tok = cursor.peek_nth(1)?.cloned();
                match next_tok {
                    Some((Token::Let, _)) => {
                        let let_binding = parse_let_binding(&mut cursor)?;
                        items.push(Item::Let(let_binding));
                    }
                    Some((Token::Env, _)) => {
                        let env_binding = parse_env_binding(&mut cursor)?;
                        items.push(Item::Env(env_binding));
                    }
                    Some((Token::State, _)) => {
                        let state_binding = parse_state_binding(&mut cursor)?;
                        items.push(Item::State(state_binding));
                    }
                    Some((Token::Component, _))
                    | Some((Token::Span, _))
                    | Some((Token::InlineBox, _)) => {
                        let comp = parse_component_def(&mut cursor)?;
                        items.push(Item::Component(comp));
                    }
                    Some((Token::Enum, _)) => {
                        let enum_def = parse_enum_def(&mut cursor)?;
                        items.push(Item::Enum(enum_def));
                    }
                    Some((Token::Use, _)) => {
                        let use_decl = parse_use_declaration(&mut cursor)?;
                        items.push(Item::Use(use_decl));
                    }
                    Some((Token::Ident(_), _)) | Some((Token::Children, _)) => {
                        let node = parse_element_node(&mut cursor)?;
                        items.push(Item::Node(node));
                    }
                    Some((other, other_span)) => {
                        return Err(ParseError::UnexpectedToken {
                            expected: "Component, Span, InlineBox, Enum, use, let, env, state, or element name after '\\'".to_string(),
                            found: other.to_string(),
                            span: other_span,
                        });
                    }
                    None => {
                        return Err(ParseError::UnexpectedEof {
                            expected: "Component, Span, InlineBox, Enum, use, let, env, state, or element name after '\\'".to_string(),
                            span,
                        });
                    }
                }
            }
            other => {
                return Err(ParseError::UnexpectedToken {
                    expected: "top-level declaration starting with '\\'".to_string(),
                    found: other.to_string(),
                    span,
                });
            }
        }
    }

    let end_pos = cursor.pos();
    let doc_span = cursor.span(start_pos, end_pos);

    Ok(Document {
        items,
        span: doc_span,
    })
}
