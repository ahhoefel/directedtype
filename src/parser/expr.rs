use crate::ast::{
    BinaryExpr, BinaryOp, CallExpr, Expr, ExprDef, Ident, Literal, MemberAccessExpr,
    MethodCallExpr, ParamDef, TernaryExpr, TypeRef, UnaryExpr, UnaryOp,
};
use crate::error::ParseError;
use crate::parser::cursor::ParserCursor;
use crate::token::Token;

pub fn parse_expr(cursor: &mut ParserCursor<'_>) -> Result<Expr, ParseError> {
    parse_expr_bp(cursor, 0)
}

fn parse_expr_bp(cursor: &mut ParserCursor<'_>, min_bp: u8) -> Result<Expr, ParseError> {
    // 1. Parse prefix / primary expression
    let mut lhs = parse_prefix(cursor)?;

    // 2. Parse postfix / infix operators
    while let Some(peeked) = cursor.peek_token()?.cloned() {
        // Postfix: Member access `.ident`
        if peeked.0 == Token::Dot {
            cursor.consume_token(&Token::Dot)?;
            let member = parse_ident(cursor)?;
            let span = lhs.span().merge(member.span);
            lhs = Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(lhs),
                member,
                span,
            });
            continue;
        }

        // Postfix: Method call `obj.method(...)` or function call `func(...)`
        if peeked.0 == Token::LParen {
            if let Expr::MemberAccess(m) = lhs {
                let open_span = cursor.consume_token(&Token::LParen)?;
                let mut args = Vec::new();
                while let Some((tok, _)) = cursor.peek_token()? {
                    if tok == &Token::RParen {
                        break;
                    }
                    args.push(parse_expr_bp(cursor, 0)?);
                    if let Some((Token::Comma, _)) = cursor.peek_token()? {
                        cursor.consume_token(&Token::Comma)?;
                    } else {
                        break;
                    }
                }
                let close_span = cursor.consume_token(&Token::RParen)?;
                let call_span = open_span.merge(close_span);
                let full_span = m.span.merge(call_span);
                lhs = Expr::MethodCall(MethodCallExpr {
                    target: m.target,
                    method: m.member,
                    args,
                    span: full_span,
                });
                continue;
            }

            if let Expr::Ident(callee) = &lhs {
                let callee = callee.clone();
                let open_span = cursor.consume_token(&Token::LParen)?;
                let mut args = Vec::new();
                while let Some((tok, _)) = cursor.peek_token()? {
                    if tok == &Token::RParen {
                        break;
                    }
                    args.push(parse_expr_bp(cursor, 0)?);
                    if let Some((Token::Comma, _)) = cursor.peek_token()? {
                        cursor.consume_token(&Token::Comma)?;
                    } else {
                        break;
                    }
                }
                let close_span = cursor.consume_token(&Token::RParen)?;
                let call_span = open_span.merge(close_span);
                lhs = Expr::Call(CallExpr {
                    callee,
                    args,
                    span: lhs.span().merge(call_span),
                });
                continue;
            }
        }

        // Ternary conditional `? then : else`
        if peeked.0 == Token::Question {
            // Ternary has lowest precedence above 0
            if min_bp > 0 {
                break;
            }
            cursor.consume_token(&Token::Question)?;
            let then_expr = parse_expr_bp(cursor, 0)?;
            cursor.consume_token(&Token::Colon)?;
            let else_expr = parse_expr_bp(cursor, 0)?;
            let span = lhs.span().merge(else_expr.span());
            lhs = Expr::Ternary(TernaryExpr {
                condition: Box::new(lhs),
                then_expr: Box::new(then_expr),
                else_expr: Box::new(else_expr),
                span,
            });
            continue;
        }

        // Infix binary operators
        if let Some((bin_op, left_bp, right_bp)) = infix_binding_power(&peeked.0) {
            if left_bp < min_bp {
                break;
            }
            // Consume operator
            cursor.next_token()?;
            let rhs = parse_expr_bp(cursor, right_bp)?;
            let span = lhs.span().merge(rhs.span());
            lhs = Expr::Binary(BinaryExpr {
                op: bin_op,
                left: Box::new(lhs),
                right: Box::new(rhs),
                span,
            });
            continue;
        }

        break;
    }

    Ok(lhs)
}

fn parse_prefix(cursor: &mut ParserCursor<'_>) -> Result<Expr, ParseError> {
    if cursor.peek_token()?.is_some_and(|(t, _)| t == &Token::Backslash) {
        if matches!(
            cursor.peek_nth(1)?,
            Some((Token::Ident(ref n), _)) if n == "Expr"
        ) {
            return parse_expr_def(cursor);
        }
        if cursor.peek_nth(1)?.is_some_and(|(t, _)| t != &Token::Children) {
            let node = crate::parser::node::parse_element_node(cursor)?;
            return Ok(Expr::Node(Box::new(node)));
        }
    }

    let (tok, span) = cursor.expect_token("expression")?;

    match tok {
        Token::Integer(val) => Ok(Expr::Literal(Literal::Integer(val, span))),
        Token::Float(val) => Ok(Expr::Literal(Literal::Float(val, span))),
        Token::Dimension((val, unit_str)) => {
            if let Some(unit) = crate::ast::DistanceUnit::from_str(&unit_str) {
                Ok(Expr::Literal(Literal::Distance(val, unit, span)))
            } else if let Some(unit) = crate::ast::DurationUnit::from_str(&unit_str) {
                let raw = &cursor.source()[span.start..span.end];
                let num_str = raw.strip_suffix(&unit_str).unwrap_or(raw);
                let micros = unit.parse_micros(num_str).map_err(|message| ParseError::Custom {
                    message,
                    span,
                })?;
                Ok(Expr::Literal(Literal::Duration(micros, unit, span)))
            } else {
                Err(ParseError::Custom {
                    message: format!(
                        "Unknown distance unit '{}'. Supported units are 'px', 'pt', 'em', 's', and 'ms'",
                        unit_str
                    ),
                    span,
                })
            }
        }
        Token::String(val) => Ok(Expr::Literal(Literal::String(val, span))),
        Token::Color(val) => Ok(Expr::Literal(Literal::Color(val, span))),
        Token::True => Ok(Expr::Literal(Literal::Bool(true, span))),
        Token::False => Ok(Expr::Literal(Literal::Bool(false, span))),
        Token::Ident(name) => Ok(Expr::Ident(Ident::new(name, span))),
        Token::Children => Ok(Expr::Ident(Ident::new("Children", span))),
        Token::Env => Ok(Expr::Ident(Ident::new("env", span))),
        Token::Minus => {
            let operand = parse_expr_bp(cursor, 13)?;
            let total_span = span.merge(operand.span());
            Ok(Expr::Unary(UnaryExpr {
                op: UnaryOp::Neg,
                operand: Box::new(operand),
                span: total_span,
            }))
        }
        Token::Bang => {
            let operand = parse_expr_bp(cursor, 13)?;
            let total_span = span.merge(operand.span());
            Ok(Expr::Unary(UnaryExpr {
                op: UnaryOp::Not,
                operand: Box::new(operand),
                span: total_span,
            }))
        }
        Token::LParen => {
            let inner = parse_expr_bp(cursor, 0)?;
            let rparen_span = cursor.consume_token(&Token::RParen)?;
            let total_span = span.merge(rparen_span);
            Ok(Expr::Paren(Box::new(inner), total_span))
        }
        other => Err(ParseError::UnexpectedToken {
            expected: "expression (literal, identifier, unary operator, or parenthesis)".to_string(),
            found: other.to_string(),
            span,
        }),
    }
}

fn parse_expr_param_def(cursor: &mut ParserCursor<'_>) -> Result<ParamDef, ParseError> {
    if let Some((Token::Env, span)) = cursor.peek_token()?.cloned() {
        return Err(ParseError::Custom {
            message: "'env' modifier is not allowed on \\Expr parameters".to_string(),
            span,
        });
    }

    let name = parse_ident(cursor)?;

    let type_annotation = if let Some((Token::Colon, _)) = cursor.peek_token()? {
        cursor.consume_token(&Token::Colon)?;
        let (tok, type_span) = cursor.expect_token("type name")?;
        if let Token::Ident(type_name) = tok {
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
        }
    } else {
        None
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

    Ok(ParamDef {
        is_env: false,
        name: name.clone(),
        type_annotation,
        default_edge,
        span: name.span.merge(end_span),
    })
}

fn expr_contains_non_value_node(expr: &Expr) -> bool {
    match expr {
        Expr::Node(node) => !matches!(node.name.as_str(), "Timer" | "IndefiniteTimer"),
        Expr::MemberAccess(m) => expr_contains_non_value_node(&m.target),
        Expr::Ternary(t) => {
            expr_contains_non_value_node(&t.condition)
                || expr_contains_non_value_node(&t.then_expr)
                || expr_contains_non_value_node(&t.else_expr)
        }
        Expr::Call(c) => c.args.iter().any(expr_contains_non_value_node),
        Expr::MethodCall(m) => {
            expr_contains_non_value_node(&m.target)
                || m.args.iter().any(expr_contains_non_value_node)
        }
        Expr::Binary(b) => {
            expr_contains_non_value_node(&b.left) || expr_contains_non_value_node(&b.right)
        }
        Expr::Unary(u) => expr_contains_non_value_node(&u.operand),
        Expr::Paren(inner, _) => expr_contains_non_value_node(inner),
        Expr::ExprDef(def) => expr_contains_non_value_node(&def.body),
        Expr::Literal(_) | Expr::Ident(_) => false,
    }
}

fn parse_expr_def(cursor: &mut ParserCursor<'_>) -> Result<Expr, ParseError> {
    let bs_span = cursor.consume_token(&Token::Backslash)?;
    let (_, expr_kw_span) = cursor.expect_token("Expr")?;
    let start_span = bs_span.merge(expr_kw_span);

    let params = if let Some((Token::LParen, _)) = cursor.peek_token()? {
        cursor.consume_token(&Token::LParen)?;
        let mut params = Vec::new();
        while let Some((tok, _)) = cursor.peek_token()? {
            if tok == &Token::RParen {
                break;
            }
            params.push(parse_expr_param_def(cursor)?);
            if let Some((Token::Comma, _)) = cursor.peek_token()? {
                cursor.consume_token(&Token::Comma)?;
            } else {
                break;
            }
        }
        cursor.consume_token(&Token::RParen)?;
        params
    } else {
        Vec::new()
    };

    let open_brace_span = match cursor.peek_token()?.cloned() {
        Some((Token::LBrace, _)) => cursor.consume_token(&Token::LBrace)?,
        Some((other, span)) => {
            return Err(ParseError::UnexpectedToken {
                expected: "'(' or '{' after \\Expr".to_string(),
                found: other.to_string(),
                span,
            });
        }
        None => {
            return Err(ParseError::UnexpectedEof {
                expected: "'{' after \\Expr".to_string(),
                span: start_span,
            });
        }
    };

    if let Some((Token::RBrace, close_span)) = cursor.peek_token()?.cloned() {
        return Err(ParseError::Custom {
            message: "\\Expr body cannot be empty; expected an expression inside '{ ... }'"
                .to_string(),
            span: open_brace_span.merge(close_span),
        });
    }

    if let Some((Token::Backslash, bs)) = cursor.peek_token()?.cloned() {
        match cursor.peek_nth(1)?.cloned() {
            Some((
                Token::Let
                | Token::State
                | Token::Env
                | Token::Alias
                | Token::Assert
                | Token::Children
                | Token::Component
                | Token::Span
                | Token::InlineBox
                | Token::Enum
                | Token::Use,
                kw_span,
            )) => {
                return Err(ParseError::Custom {
                    message:
                        "\\Expr body must be a single expression, not a statement or directive"
                            .to_string(),
                    span: bs.merge(kw_span),
                });
            }
            Some((Token::Ident(ref n), id_span))
                if !matches!(n.as_str(), "Expr" | "Timer" | "IndefiniteTimer") =>
            {
                return Err(ParseError::Custom {
                    message: format!(
                        "\\Expr body must be a single expression, not element node '\\{}'",
                        n
                    ),
                    span: bs.merge(id_span),
                });
            }
            _ => {}
        }
    }

    let body = parse_expr_bp(cursor, 0).map_err(|err| match err {
        ParseError::UnexpectedToken { span, .. } => ParseError::Custom {
            message: "\\Expr body must contain a valid expression inside '{ ... }'".to_string(),
            span,
        },
        other => other,
    })?;

    if expr_contains_non_value_node(&body) {
        return Err(ParseError::Custom {
            message:
                "\\Expr body must be a single expression and cannot contain layout element nodes"
                    .to_string(),
            span: body.span(),
        });
    }

    if let Some((Token::Semicolon, semi_span)) = cursor.peek_token()?.cloned() {
        return Err(ParseError::Custom {
            message: "\\Expr body must be a single expression without a trailing ';'".to_string(),
            span: semi_span,
        });
    }

    let close_brace_span = match cursor.peek_token()?.cloned() {
        Some((Token::RBrace, _)) => cursor.consume_token(&Token::RBrace)?,
        Some((other, span)) => {
            return Err(ParseError::Custom {
                message: format!(
                    "\\Expr body must contain a single expression followed by '}}', found {}",
                    other
                ),
                span,
            });
        }
        None => {
            return Err(ParseError::UnclosedDelimiter {
                delimiter: '}',
                open_span: open_brace_span,
            });
        }
    };

    Ok(Expr::ExprDef(Box::new(ExprDef {
        params,
        body: Box::new(body),
        span: start_span.merge(close_brace_span),
    })))
}

pub fn parse_ident(cursor: &mut ParserCursor<'_>) -> Result<Ident, ParseError> {
    let (tok, span) = cursor.expect_token("identifier")?;
    match tok {
        Token::Ident(name) => Ok(Ident::new(name, span)),
        Token::Children => Ok(Ident::new("Children", span)),
        Token::Component => Ok(Ident::new("Component", span)),
        other => Err(ParseError::UnexpectedToken {
            expected: "identifier".to_string(),
            found: other.to_string(),
            span,
        }),
    }
}

fn infix_binding_power(token: &Token) -> Option<(BinaryOp, u8, u8)> {
    match token {
        Token::PipePipe => Some((BinaryOp::Or, 1, 2)),
        Token::AmpAmp => Some((BinaryOp::And, 3, 4)),
        Token::EqEq => Some((BinaryOp::Eq, 5, 6)),
        Token::BangEq => Some((BinaryOp::Ne, 5, 6)),
        Token::Lt => Some((BinaryOp::Lt, 7, 8)),
        Token::LtEq => Some((BinaryOp::Le, 7, 8)),
        Token::Gt => Some((BinaryOp::Gt, 7, 8)),
        Token::GtEq => Some((BinaryOp::Ge, 7, 8)),
        Token::Plus => Some((BinaryOp::Add, 9, 10)),
        Token::Minus => Some((BinaryOp::Sub, 9, 10)),
        Token::Star => Some((BinaryOp::Mul, 11, 12)),
        Token::Slash => Some((BinaryOp::Div, 11, 12)),
        Token::Percent => Some((BinaryOp::Rem, 11, 12)),
        _ => None,
    }
}
