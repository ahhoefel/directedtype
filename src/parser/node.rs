use crate::ast::{
    ChildrenDirective, ComponentKey, ContentItem, ElementNode, PortBinding,
};
use crate::error::ParseError;
use crate::parser::cursor::ParserCursor;
use crate::parser::expr::{parse_expr, parse_ident};
use crate::span::Span;
use crate::token::Token;

/// Parses a single port binding: `name: expr` or shorthand boolean flag `name` (meaning `name: true`)
pub fn parse_port_binding(cursor: &mut ParserCursor<'_>) -> Result<PortBinding, ParseError> {
    let name = parse_ident(cursor)?;
    if let Some((Token::Colon, _)) = cursor.peek_token()? {
        cursor.consume_token(&Token::Colon)?;
        let expr = parse_expr(cursor)?;
        let span = name.span.merge(expr.span());
        Ok(PortBinding { name, expr, span })
    } else {
        let span = name.span;
        let expr = crate::ast::Expr::bool(true);
        Ok(PortBinding { name, expr, span })
    }
}

fn has_top_level_semicolon(cursor: &mut ParserCursor<'_>) -> Result<bool, ParseError> {
    let mut depth = 1;
    let mut i = 0;
    loop {
        match cursor.peek_nth(i)? {
            Some((Token::LParen, _)) => depth += 1,
            Some((Token::RParen, _)) => {
                depth -= 1;
                if depth == 0 {
                    return Ok(false);
                }
            }
            Some((Token::Semicolon, _)) if depth == 1 => return Ok(true),
            Some(_) => {}
            None => return Ok(false),
        }
        i += 1;
    }
}

/// Parses an element's arguments enclosed in parentheses: `( [ key ; ] [ ports ] )`
pub fn parse_element_args(
    cursor: &mut ParserCursor<'_>,
) -> Result<(Option<ComponentKey>, Vec<PortBinding>, Span), ParseError> {
    let open_span = cursor.consume_token(&Token::LParen)?;

    let has_semicolon = has_top_level_semicolon(cursor)?;

    let key = if has_semicolon {
        let mut parts = Vec::new();
        let mut key_span = open_span;

        while let Some((tok, _)) = cursor.peek_token()? {
            if tok == &Token::Semicolon || tok == &Token::RParen {
                break;
            }
            let expr = parse_expr(cursor)?;
            key_span = key_span.merge(expr.span());
            parts.push(expr);

            if let Some((Token::Comma, _)) = cursor.peek_token()? {
                cursor.consume_token(&Token::Comma)?;
            } else {
                break;
            }
        }

        let semi_span = cursor.consume_token(&Token::Semicolon)?;
        key_span = key_span.merge(semi_span);

        if parts.is_empty() {
            None
        } else {
            Some(ComponentKey::new(parts, key_span))
        }
    } else {
        None
    };

    let mut ports = Vec::new();
    while let Some((tok, _)) = cursor.peek_token()? {
        if tok == &Token::RParen {
            break;
        }
        ports.push(parse_port_binding(cursor)?);
        if let Some((Token::Comma, _)) = cursor.peek_token()? {
            cursor.consume_token(&Token::Comma)?;
        } else {
            break;
        }
    }

    let close_span = cursor.consume_token(&Token::RParen)?;
    let span = open_span.merge(close_span);
    Ok((key, ports, span))
}

/// Parses a list of port bindings enclosed in parentheses: `(port1: expr1, port2: expr2)`
pub fn parse_port_list(cursor: &mut ParserCursor<'_>) -> Result<(Vec<PortBinding>, Span), ParseError> {
    let (_, ports, span) = parse_element_args(cursor)?;
    Ok((ports, span))
}

/// Parses the `\Children [ { ... } ]` directive with port bindings.
pub fn parse_children_directive(cursor: &mut ParserCursor<'_>) -> Result<ChildrenDirective, ParseError> {
    let slash_span = cursor.consume_token(&Token::Backslash)?;
    let children_span = cursor.consume_token(&Token::Children)?;

    if let Some((Token::LBrace, _)) = cursor.peek_token()? {
        cursor.consume_token(&Token::LBrace)?;

        let mut ports = Vec::new();
        while let Some((tok, _)) = cursor.peek_token()? {
            if tok == &Token::RBrace {
                break;
            }
            ports.push(parse_port_binding(cursor)?);
            if let Some((Token::Comma, _)) = cursor.peek_token()? {
                cursor.consume_token(&Token::Comma)?;
            }
        }

        let rbrace_span = cursor.consume_token(&Token::RBrace)?;
        let span = slash_span.merge(rbrace_span);
        Ok(ChildrenDirective { ports, span })
    } else {
        let span = slash_span.merge(children_span);
        Ok(ChildrenDirective {
            ports: Vec::new(),
            span,
        })
    }
}

/// Parses an element node: `\Name [ ( [ key ; ] [ ports ] ) ] [ { content } ]`
pub fn parse_element_node(cursor: &mut ParserCursor<'_>) -> Result<ElementNode, ParseError> {
    let slash_span = cursor.consume_token(&Token::Backslash)?;
    let name = parse_ident(cursor)?;

    let (key, ports, port_span) = if let Some((Token::LParen, _)) = cursor.peek_token()? {
        let (k, p, s) = parse_element_args(cursor)?;
        (k, p, Some(s))
    } else {
        (None, Vec::new(), None)
    };

    let (content, content_span) = if let Some((Token::LBrace, _)) = cursor.peek_token()? {
        let slot = cursor.parse_content_slot(|c| parse_content_item(c))?;
        let s = slot.span;
        (Some(slot), Some(s))
    } else {
        (None, None)
    };

    let total_span = slash_span
        .merge(name.span)
        .merge(port_span.unwrap_or(slash_span))
        .merge(content_span.unwrap_or(slash_span));

    Ok(ElementNode {
        name,
        key,
        ports,
        content,
        span: total_span,
        handle: None,
    })
}

/// Parses an inline item when `\` is encountered inside a content slot.
pub fn parse_content_item(cursor: &mut ParserCursor<'_>) -> Result<ContentItem, ParseError> {
    let is_children = cursor.peek_token()?.is_some_and(|(t, _)| t == &Token::Backslash)
        && cursor.peek_nth(1)?.is_some_and(|(t, _)| t == &Token::Children);

    if is_children {
        let directive = parse_children_directive(cursor)?;
        return Ok(ContentItem::Children(directive));
    }

    let node = parse_element_node(cursor)?;
    Ok(ContentItem::Node(node))
}
