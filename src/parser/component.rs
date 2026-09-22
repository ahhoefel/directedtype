use crate::ast::{
    ComponentBodyItem, ComponentDef, Ident, LetBinding, LetValue, ParamDef, TypeRef,
};
use crate::error::ParseError;
use crate::parser::cursor::ParserCursor;
use crate::parser::expr::{parse_expr, parse_ident};
use crate::parser::node::{parse_children_directive, parse_element_node};
use crate::span::Span;
use crate::token::Token;

/// Parses a parameter definition in a component signature:
/// e.g. `bg_color: Color`, `gap: Number: 16`, `gap: Number = 16`, or `width: max(children.width) + 32`
pub fn parse_param_def(cursor: &mut ParserCursor<'_>) -> Result<ParamDef, ParseError> {
    let name = parse_ident(cursor)?;

    let mut type_annotation = None;
    let mut default_edge = None;

    if let Some((Token::Colon, _)) = cursor.peek_token()? {
        cursor.consume_token(&Token::Colon)?;

        // Inspect the token right after the colon
        let tok0 = cursor.peek_token()?.cloned();
        let tok1 = cursor.peek_nth(1)?.cloned();

        match (tok0, tok1) {
            // Check if tok0 is an uppercase identifier (e.g. Color, Number, String)
            (Some((Token::Ident(type_name), span0)), Some((Token::Colon, _)))
            | (Some((Token::Ident(type_name), span0)), Some((Token::Eq, _)))
                if type_name.chars().next().is_some_and(|c| c.is_uppercase()) =>
            {
                // `name: Type: expr` or `name: Type = expr`
                cursor.next_token()?; // consume type_name
                type_annotation = Some(TypeRef {
                    name: Ident::new(type_name, span0),
                    span: span0,
                });
                if cursor.peek_token()?.is_some_and(|(t, _)| t == &Token::Eq) {
                    cursor.consume_token(&Token::Eq)?;
                } else {
                    cursor.consume_token(&Token::Colon)?;
                }
                default_edge = Some(parse_expr(cursor)?);
            }
            (Some((Token::Ident(type_name), span0)), Some((Token::Comma, _)))
            | (Some((Token::Ident(type_name), span0)), Some((Token::RParen, _)))
                if type_name.chars().next().is_some_and(|c| c.is_uppercase()) =>
            {
                // `name: Type`
                cursor.next_token()?; // consume type_name
                type_annotation = Some(TypeRef {
                    name: Ident::new(type_name, span0),
                    span: span0,
                });
            }
            _ => {
                // `name: expr` (e.g. `width: max(...) + 32` or `x: 10`)
                default_edge = Some(parse_expr(cursor)?);
            }
        }
    } else if let Some((Token::Eq, _)) = cursor.peek_token()? {
        // `name = expr`
        cursor.consume_token(&Token::Eq)?;
        default_edge = Some(parse_expr(cursor)?);
    }

    let end_span = default_edge
        .as_ref()
        .map(|e| e.span())
        .or_else(|| type_annotation.as_ref().map(|t| t.span))
        .unwrap_or(name.span);

    let span = name.span.merge(end_span);

    Ok(ParamDef {
        name,
        type_annotation,
        default_edge,
        span,
    })
}

/// Parses a list of parameter definitions: `(param1, param2, ...)`
pub fn parse_param_list(cursor: &mut ParserCursor<'_>) -> Result<(Vec<ParamDef>, Span), ParseError> {
    let open_span = cursor.consume_token(&Token::LParen)?;
    let mut params = Vec::new();

    while let Some((tok, _)) = cursor.peek_token()? {
        if tok == &Token::RParen {
            break;
        }
        params.push(parse_param_def(cursor)?);
        if let Some((Token::Comma, _)) = cursor.peek_token()? {
            cursor.consume_token(&Token::Comma)?;
        } else {
            break;
        }
    }

    let close_span = cursor.consume_token(&Token::RParen)?;
    let span = open_span.merge(close_span);
    Ok((params, span))
}

/// Parses a component definition: `\Component Name(params) { body }`
pub fn parse_component_def(cursor: &mut ParserCursor<'_>) -> Result<ComponentDef, ParseError> {
    let slash_span = cursor.consume_token(&Token::Backslash)?;
    cursor.consume_token(&Token::Component)?;
    let name = parse_ident(cursor)?;

    let (params, _) = if let Some((Token::LParen, _)) = cursor.peek_token()? {
        parse_param_list(cursor)?
    } else {
        (Vec::new(), slash_span)
    };

    let lbrace_span = cursor.consume_token(&Token::LBrace)?;
    let mut body = Vec::new();

    while let Some((tok, _)) = cursor.peek_token()? {
        if tok == &Token::RBrace {
            break;
        }

        if tok == &Token::Let {
            let let_binding = parse_let_binding(cursor)?;
            body.push(ComponentBodyItem::Let(let_binding));
            continue;
        }

        let is_children = tok == &Token::Backslash
            && cursor.peek_nth(1)?.is_some_and(|(t, _)| t == &Token::Children);

        if is_children {
            let directive = parse_children_directive(cursor)?;
            body.push(ComponentBodyItem::Children(directive));
        } else {
            let node = parse_element_node(cursor)?;
            body.push(ComponentBodyItem::Node(node));
        }
    }

    let rbrace_span = cursor.consume_token(&Token::RBrace)?;
    let span = slash_span.merge(lbrace_span).merge(rbrace_span);

    Ok(ComponentDef {
        name,
        params,
        body,
        span,
    })
}

/// Parses a local let binding: `let name = expr;` or `let name: Type = expr;` or `let name = \Node(...);`
pub fn parse_let_binding(cursor: &mut ParserCursor<'_>) -> Result<LetBinding, ParseError> {
    let let_span = cursor.consume_token(&Token::Let)?;
    let name = parse_ident(cursor)?;

    let mut type_annotation = None;
    if let Some((Token::Colon, _)) = cursor.peek_token()? {
        cursor.consume_token(&Token::Colon)?;
        let (tok, span) = cursor.expect_token("type name")?;
        if let Token::Ident(type_name) = tok {
            type_annotation = Some(TypeRef {
                name: Ident::new(type_name, span),
                span,
            });
        } else {
            return Err(ParseError::UnexpectedToken {
                expected: "type name identifier".to_string(),
                found: tok.to_string(),
                span,
            });
        }
    }

    cursor.consume_token(&Token::Eq)?;

    let value = if let Some((Token::Backslash, _)) = cursor.peek_token()? {
        let node = parse_element_node(cursor)?;
        LetValue::Node(node)
    } else {
        let expr = parse_expr(cursor)?;
        LetValue::Expr(expr)
    };

    if let Some((Token::Semicolon, _)) = cursor.peek_token()? {
        cursor.consume_token(&Token::Semicolon)?;
    }

    let span = let_span.merge(value.span());
    Ok(LetBinding {
        name,
        type_annotation,
        value,
        span,
    })
}
