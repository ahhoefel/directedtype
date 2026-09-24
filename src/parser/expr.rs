use crate::ast::{
    BinaryExpr, BinaryOp, CallExpr, Expr, Ident, Literal, MemberAccessExpr, TernaryExpr, UnaryExpr,
    UnaryOp,
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

        // Postfix: Function call `(...)`
        // Only valid if lhs is an Ident
        if peeked.0 == Token::LParen {
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
    if cursor.peek_token()?.is_some_and(|(t, _)| t == &Token::Backslash)
        && cursor.peek_nth(1)?.is_some_and(|(t, _)| t != &Token::Children)
    {
        let node = crate::parser::node::parse_element_node(cursor)?;
        return Ok(Expr::Node(Box::new(node)));
    }

    let (tok, span) = cursor.expect_token("expression")?;

    match tok {
        Token::Number(val) => Ok(Expr::Literal(Literal::Number(val, span))),
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
