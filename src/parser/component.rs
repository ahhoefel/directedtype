use crate::ast::{
    AliasBinding, AssertDirective, ComponentDef, EnumDef, EnvBinding, Expr, Ident, LetBinding, LetValue, Literal,
    ParamDef, StateBinding, TypeRef, UseDeclaration, UseItem,
};
use crate::error::ParseError;
use crate::parser::cursor::{ParserCursor, SlotMode};
use crate::parser::expr::{parse_expr, parse_ident};
use crate::parser::node::{parse_content_item, parse_element_node};
use crate::span::Span;
use crate::token::Token;

/// Parses a parameter definition in a component signature:
/// e.g. `bg_color: Color`, `gap: Distance = 16px`, `env font: Font`, or `width: Distance = max(children.width) + 32px`
pub fn parse_param_def(cursor: &mut ParserCursor<'_>) -> Result<ParamDef, ParseError> {
    let (is_env, env_span) = if let Some((Token::Env, _)) = cursor.peek_token()? {
        let span = cursor.consume_token(&Token::Env)?;
        (true, Some(span))
    } else {
        (false, None)
    };

    let name = parse_ident(cursor)?;

    cursor.consume_token(&Token::Colon)?;
    let (tok, type_span) = cursor.expect_token("type name")?;
    let type_annotation = if let Token::Ident(type_name) = tok {
        Some(TypeRef {
            name: Ident::new(type_name, type_span),
            span: type_span,
        })
    } else {
        return Err(ParseError::UnexpectedToken {
            expected: "type name identifier".to_string(),
            found: tok.to_string(),
            span: type_span,
        });
    };

    let default_edge = if let Some((Token::Eq, _)) = cursor.peek_token()? {
        cursor.consume_token(&Token::Eq)?;
        Some(parse_expr(cursor)?)
    } else {
        None
    };

    let end_span = default_edge
        .as_ref()
        .map(|e| e.span())
        .or_else(|| type_annotation.as_ref().map(|t| t.span))
        .unwrap_or(name.span);

    let start_span = env_span.unwrap_or(name.span);
    let span = start_span.merge(end_span);

    Ok(ParamDef {
        is_env,
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

/// Parses a component definition: `\Component Name(params) { body }` or `\Span Name(params) { body }`
pub fn parse_component_def(cursor: &mut ParserCursor<'_>) -> Result<ComponentDef, ParseError> {
    let slash_span = cursor.consume_token(&Token::Backslash)?;
    let (kind, comp_span) = match cursor.peek_token()?.cloned() {
        Some((Token::Component, _)) => (
            crate::ast::ComponentKind::Component,
            cursor.consume_token(&Token::Component)?,
        ),
        Some((Token::Span, _)) => (
            crate::ast::ComponentKind::Span,
            cursor.consume_token(&Token::Span)?,
        ),
        Some((other, span)) => {
            return Err(ParseError::UnexpectedToken {
                expected: "Component or Span".to_string(),
                found: other.to_string(),
                span,
            });
        }
        None => {
            return Err(ParseError::UnexpectedEof {
                expected: "Component or Span".to_string(),
                span: slash_span,
            });
        }
    };
    let name = parse_ident(cursor)?;

    let (params, _) = if let Some((Token::LParen, _)) = cursor.peek_token()? {
        parse_param_list(cursor)?
    } else {
        (Vec::new(), slash_span)
    };

    let slot = cursor.parse_content_slot_mode(SlotMode::ComponentDef, |c| {
        parse_content_item(c, true)
    })?;

    let span = slash_span.merge(comp_span).merge(slot.span);

    Ok(ComponentDef {
        kind,
        name,
        params,
        body: slot.items,
        span,
    })
}

/// Parses a local let binding: `\let name = expr;`, `\let name: Type = expr;`, `\let name = \Node(...);`, or uninitialized `\let name;`
pub fn parse_let_binding(cursor: &mut ParserCursor<'_>) -> Result<LetBinding, ParseError> {
    let bs_span = cursor.consume_token(&Token::Backslash)?;
    let let_span = cursor.consume_token(&Token::Let)?;
    let start_span = bs_span.merge(let_span);
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

    let (value, end_span) = if let Some((Token::Eq, _)) = cursor.peek_token()? {
        cursor.consume_token(&Token::Eq)?;
        let val = if let Some((Token::Backslash, _)) = cursor.peek_token()? {
            let node = parse_element_node(cursor)?;
            LetValue::Node(node)
        } else {
            let expr = parse_expr(cursor)?;
            LetValue::Expr(expr)
        };
        let semi_span = if let Some((Token::Semicolon, _)) = cursor.peek_token()? {
            cursor.consume_token(&Token::Semicolon)?
        } else {
            val.span()
        };
        (Some(val), semi_span)
    } else {
        // Uninitialized `\let name;`
        let semi_span = cursor.consume_token(&Token::Semicolon)?;
        (None, semi_span)
    };

    let span = start_span.merge(end_span);
    Ok(LetBinding {
        name,
        type_annotation,
        value,
        span,
    })
}

/// Parses an environmental variable binding: `\env name = expr;`, `\env name: Type = expr;`, or uninitialized `\env name;`
pub fn parse_env_binding(cursor: &mut ParserCursor<'_>) -> Result<EnvBinding, ParseError> {
    let bs_span = cursor.consume_token(&Token::Backslash)?;
    let env_span = cursor.consume_token(&Token::Env)?;
    let start_span = bs_span.merge(env_span);
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

    let (value, end_span) = if let Some((Token::Eq, _)) = cursor.peek_token()? {
        cursor.consume_token(&Token::Eq)?;
        let expr = parse_expr(cursor)?;
        let semi_span = if let Some((Token::Semicolon, _)) = cursor.peek_token()? {
            cursor.consume_token(&Token::Semicolon)?
        } else {
            expr.span()
        };
        (Some(expr), semi_span)
    } else {
        // Uninitialized `\env name;`
        let semi_span = cursor.consume_token(&Token::Semicolon)?;
        (None, semi_span)
    };

    let span = start_span.merge(end_span);
    Ok(EnvBinding {
        name,
        type_annotation,
        value,
        span,
    })
}

/// Parses a public immutable alias binding: `\alias name = expr;` or `\alias name: Type = expr;`
pub fn parse_alias_binding(cursor: &mut ParserCursor<'_>) -> Result<AliasBinding, ParseError> {
    let bs_span = cursor.consume_token(&Token::Backslash)?;
    let alias_span = cursor.consume_token(&Token::Alias)?;
    let start_span = bs_span.merge(alias_span);
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

    let value = parse_expr(cursor)?;
    let semi_span = if let Some((Token::Semicolon, _)) = cursor.peek_token()? {
        cursor.consume_token(&Token::Semicolon)?
    } else {
        value.span()
    };

    let span = start_span.merge(semi_span);
    Ok(AliasBinding {
        name,
        type_annotation,
        value,
        span,
    })
}

/// Parses a reactive state variable binding:
/// `\state name: Type = expr;` or uninitialized `\state name: Type;`
pub fn parse_state_binding(cursor: &mut ParserCursor<'_>) -> Result<StateBinding, ParseError> {
    let bs_span = cursor.consume_token(&Token::Backslash)?;
    let state_span = cursor.consume_token(&Token::State)?;
    let start_span = bs_span.merge(state_span);
    let name = parse_ident(cursor)?;

    cursor.consume_token(&Token::Colon)?;
    let (tok, type_span) = cursor.expect_token("type name")?;
    let type_annotation = if let Token::Ident(type_name) = tok {
        Some(TypeRef {
            name: Ident::new(type_name, type_span),
            span: type_span,
        })
    } else {
        return Err(ParseError::UnexpectedToken {
            expected: "type name identifier".to_string(),
            found: tok.to_string(),
            span: type_span,
        });
    };

    let default = if let Some((Token::Eq, _)) = cursor.peek_token()? {
        cursor.consume_token(&Token::Eq)?;
        Some(parse_expr(cursor)?)
    } else if let Some((Token::Colon, colon_span)) = cursor.peek_token()?.cloned() {
        return Err(ParseError::UnexpectedToken {
            expected: "'=' or ';'".to_string(),
            found: "':'".to_string(),
            span: colon_span,
        });
    } else {
        None
    };

    let semi_span = if let Some((Token::Semicolon, _)) = cursor.peek_token()? {
        cursor.consume_token(&Token::Semicolon)?
    } else if let Some(expr) = &default {
        expr.span()
    } else if let Some(ty) = &type_annotation {
        ty.span
    } else {
        name.span
    };

    let span = start_span.merge(semi_span);
    Ok(StateBinding {
        name,
        type_annotation,
        default,
        span,
    })
}

/// Parses an assertion directive:
/// `\assert condition as "message";` or `\assert condition;`
pub fn parse_assert_directive(cursor: &mut ParserCursor<'_>) -> Result<AssertDirective, ParseError> {
    let bs_span = cursor.consume_token(&Token::Backslash)?;
    let assert_span = cursor.consume_token(&Token::Assert)?;
    let start_span = bs_span.merge(assert_span);

    let condition = parse_expr(cursor)?;

    let mut message = None;
    let mut end_span = condition.span();

    if let Some((Token::As, _)) = cursor.peek_token()? {
        cursor.consume_token(&Token::As)?;
        if let Some((Token::String(msg), span)) = cursor.peek_token()?.cloned() {
            cursor.next_token()?;
            message = Some(msg);
            end_span = span;
        } else {
            let msg_expr = parse_expr(cursor)?;
            end_span = msg_expr.span();
            if let Expr::Literal(Literal::String(s, _)) = msg_expr {
                message = Some(s);
            }
        }
    }

    if let Some((Token::Semicolon, span)) = cursor.peek_token()?.cloned() {
        end_span = span;
        cursor.consume_token(&Token::Semicolon)?;
    }

    let span = start_span.merge(end_span);
    Ok(AssertDirective {
        condition,
        message,
        span,
    })
}

/// Parses a module use/import declaration:
/// - Path import: `\use "path" [as Alias];`
/// - Named import: `\use { item1, item2 as alias2 } from "path";`
pub fn parse_use_declaration(cursor: &mut ParserCursor<'_>) -> Result<UseDeclaration, ParseError> {
    let bs_span = cursor.consume_token(&Token::Backslash)?;
    cursor.consume_token(&Token::Use)?;

    // Check if this is a named import: `\use { ... } from "path"`
    if let Some((Token::LBrace, _)) = cursor.peek_token()? {
        cursor.consume_token(&Token::LBrace)?;
        let mut items = Vec::new();

        while let Some((tok, _)) = cursor.peek_token()? {
            if tok == &Token::RBrace {
                break;
            }

            let name = parse_ident(cursor)?;
            let alias = if let Some((Token::As, _)) = cursor.peek_token()? {
                cursor.consume_token(&Token::As)?;
                Some(parse_ident(cursor)?)
            } else {
                None
            };

            let item_span = match &alias {
                Some(a) => name.span.merge(a.span),
                None => name.span,
            };

            items.push(UseItem {
                name,
                alias,
                span: item_span,
            });

            if let Some((Token::Comma, _)) = cursor.peek_token()? {
                cursor.consume_token(&Token::Comma)?;
            } else {
                break;
            }
        }

        let rbrace_span = cursor.consume_token(&Token::RBrace)?;
        if items.is_empty() {
            return Err(ParseError::UnexpectedToken {
                expected: "at least one imported symbol inside '{ ... }'".to_string(),
                found: "}".to_string(),
                span: rbrace_span,
            });
        }

        // Expect contextual keyword `from`
        match cursor.next_token()? {
            Some((Token::Ident(s), _)) if s == "from" => {}
            Some((tok, span)) => {
                return Err(ParseError::UnexpectedToken {
                    expected: "'from' keyword after import list".to_string(),
                    found: tok.to_string(),
                    span,
                });
            }
            None => {
                return Err(ParseError::UnexpectedEof {
                    expected: "'from' keyword after import list".to_string(),
                    span: cursor.empty_span(cursor.pos()),
                });
            }
        }

        // Expect path string
        let (path, path_span) = match cursor.next_token()? {
            Some((Token::String(s), span)) => (s, span),
            Some((tok, span)) => {
                return Err(ParseError::UnexpectedToken {
                    expected: "string literal file path after 'from'".to_string(),
                    found: tok.to_string(),
                    span,
                });
            }
            None => {
                return Err(ParseError::UnexpectedEof {
                    expected: "string literal file path after 'from'".to_string(),
                    span: cursor.empty_span(cursor.pos()),
                });
            }
        };

        let semi_span = if let Some((Token::Semicolon, _)) = cursor.peek_token()? {
            cursor.consume_token(&Token::Semicolon)?
        } else {
            path_span
        };

        let span = bs_span.merge(semi_span);
        Ok(UseDeclaration {
            path,
            alias: None,
            items,
            span,
        })
    } else {
        let (path, path_span) = match cursor.next_token()? {
            Some((Token::String(s), span)) => (s, span),
            Some((tok, span)) => {
                return Err(ParseError::UnexpectedToken {
                    expected: "string literal file path or '{' after '\\use'".to_string(),
                    found: tok.to_string(),
                    span,
                });
            }
            None => {
                return Err(ParseError::UnexpectedEof {
                    expected: "string literal file path or '{' after '\\use'".to_string(),
                    span: cursor.empty_span(cursor.pos()),
                });
            }
        };

        let alias = if let Some((Token::As, _)) = cursor.peek_token()? {
            cursor.consume_token(&Token::As)?;
            match cursor.next_token()? {
                Some((Token::Ident(name), span)) => Some(Ident::new(name, span)),
                Some((tok, span)) => {
                    return Err(ParseError::UnexpectedToken {
                        expected: "identifier alias after 'as'".to_string(),
                        found: tok.to_string(),
                        span,
                    });
                }
                None => {
                    return Err(ParseError::UnexpectedEof {
                        expected: "identifier alias after 'as'".to_string(),
                        span: cursor.empty_span(cursor.pos()),
                    });
                }
            }
        } else {
            None
        };

        let semi_span = if let Some((Token::Semicolon, _)) = cursor.peek_token()? {
            cursor.consume_token(&Token::Semicolon)?
        } else if let Some(a) = &alias {
            a.span
        } else {
            path_span
        };

        let span = bs_span.merge(semi_span);
        Ok(UseDeclaration {
            path,
            alias,
            items: Vec::new(),
            span,
        })
    }
}

/// Parses an enum definition: `\Enum Name { Variant1, Variant2, ... }`
pub fn parse_enum_def(cursor: &mut ParserCursor<'_>) -> Result<EnumDef, ParseError> {
    let slash_span = cursor.consume_token(&Token::Backslash)?;
    cursor.consume_token(&Token::Enum)?;
    let name = parse_ident(cursor)?;
    cursor.consume_token(&Token::LBrace)?;

    let mut variants = Vec::new();
    while let Some((tok, _)) = cursor.peek_token()? {
        if tok == &Token::RBrace {
            break;
        }
        let variant = parse_ident(cursor)?;
        variants.push(variant);

        if let Some((Token::Comma, _)) = cursor.peek_token()? {
            cursor.consume_token(&Token::Comma)?;
        } else {
            break;
        }
    }

    let rbrace_span = cursor.consume_token(&Token::RBrace)?;

    // Optional trailing semicolon: e.g. `\Enum Align { ... };`
    let end_span = if let Some((Token::Semicolon, semi_span)) = cursor.peek_token()? {
        let span = *semi_span;
        cursor.consume_token(&Token::Semicolon)?;
        span
    } else {
        rbrace_span
    };

    let span = slash_span.merge(end_span);
    Ok(EnumDef {
        name,
        variants,
        span,
    })
}
