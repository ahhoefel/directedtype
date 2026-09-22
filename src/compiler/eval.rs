use crate::ast::{BinaryOp, Expr, Literal, UnaryOp};
use crate::compiler::error::CompileError;
use crate::compiler::expanded::NodeId;
use crate::compiler::graph::{VarId, VariableGraph};
use crate::compiler::topo::TopologicalSchedule;
use crate::compiler::value::Value;
use std::collections::HashMap;

/// Evaluates all variables in the graph according to the topological schedule,
/// returning an environment mapping each `VarId` to its computed `Value`.
pub fn evaluate_graph(
    graph: &VariableGraph,
    schedule: &TopologicalSchedule,
) -> Result<HashMap<VarId, Value>, CompileError> {
    let mut env = HashMap::with_capacity(schedule.len());

    for var_id in schedule.iter() {
        if let Some(node) = graph.get_variable(var_id) {
            let val = eval_expr(&node.equation, &env)?;
            env.insert(var_id.clone(), val);
        }
    }

    Ok(env)
}

/// Evaluates an algebraic expression in the given environment.
pub fn eval_expr(expr: &Expr, env: &HashMap<VarId, Value>) -> Result<Value, CompileError> {
    match expr {
        Expr::Literal(lit) => match lit {
            Literal::Number(n, _) => Ok(Value::Number(*n)),
            Literal::String(s, _) => Ok(Value::String(s.clone())),
            Literal::Bool(b, _) => Ok(Value::Bool(*b)),
            Literal::Color(c, _) => Ok(Value::Color(c.clone())),
        },

        Expr::Ident(id) => match id.as_str() {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            other => Err(CompileError::Custom {
                message: format!("Unresolved identifier in expression: '{}'", other),
                span: id.span,
            }),
        },

        Expr::MemberAccess(m) => {
            if let Expr::Ident(target_id) = m.target.as_ref() {
                if let Some(node_id) = NodeId::from_canonical_name(target_id.as_str()) {
                    let var = VarId::new(node_id, m.member.as_str());
                    if let Some(val) = env.get(&var) {
                        return Ok(val.clone());
                    } else {
                        return Err(CompileError::Custom {
                            message: format!(
                                "Variable '{}' evaluated before being set in environment",
                                var
                            ),
                            span: m.span,
                        });
                    }
                }
            }
            Err(CompileError::Custom {
                message: format!("Unsupported member access target in evaluation: {:?}", m.target),
                span: m.span,
            })
        }

        Expr::Binary(b) => {
            let left = eval_expr(&b.left, env)?;
            let right = eval_expr(&b.right, env)?;

            match b.op {
                BinaryOp::Add => {
                    if let (Some(l), Some(r)) = (left.as_f64(), right.as_f64()) {
                        Ok(Value::Number(l + r))
                    } else if let (Some(l), Some(r)) = (left.as_str(), right.as_str()) {
                        Ok(Value::String(format!("{}{}", l, r)))
                    } else {
                        Err(CompileError::Custom {
                            message: format!("Cannot add {} and {}", left, right),
                            span: b.span,
                        })
                    }
                }
                BinaryOp::Sub => {
                    let l = left.as_f64().unwrap_or(0.0);
                    let r = right.as_f64().unwrap_or(0.0);
                    Ok(Value::Number(l - r))
                }
                BinaryOp::Mul => {
                    let l = left.as_f64().unwrap_or(0.0);
                    let r = right.as_f64().unwrap_or(0.0);
                    Ok(Value::Number(l * r))
                }
                BinaryOp::Div => {
                    let l = left.as_f64().unwrap_or(0.0);
                    let r = right.as_f64().unwrap_or(0.0);
                    if r == 0.0 {
                        // Graceful zero-division fallback for layout stability
                        Ok(Value::Number(0.0))
                    } else {
                        Ok(Value::Number(l / r))
                    }
                }
                BinaryOp::Rem => {
                    let l = left.as_f64().unwrap_or(0.0);
                    let r = right.as_f64().unwrap_or(1.0);
                    if r == 0.0 {
                        Ok(Value::Number(0.0))
                    } else {
                        Ok(Value::Number(l % r))
                    }
                }
                BinaryOp::Eq => Ok(Value::Bool(left == right)),
                BinaryOp::Ne => Ok(Value::Bool(left != right)),
                BinaryOp::Lt => {
                    let l = left.as_f64().unwrap_or(0.0);
                    let r = right.as_f64().unwrap_or(0.0);
                    Ok(Value::Bool(l < r))
                }
                BinaryOp::Le => {
                    let l = left.as_f64().unwrap_or(0.0);
                    let r = right.as_f64().unwrap_or(0.0);
                    Ok(Value::Bool(l <= r))
                }
                BinaryOp::Gt => {
                    let l = left.as_f64().unwrap_or(0.0);
                    let r = right.as_f64().unwrap_or(0.0);
                    Ok(Value::Bool(l > r))
                }
                BinaryOp::Ge => {
                    let l = left.as_f64().unwrap_or(0.0);
                    let r = right.as_f64().unwrap_or(0.0);
                    Ok(Value::Bool(l >= r))
                }
                BinaryOp::And => Ok(Value::Bool(left.is_truthy() && right.is_truthy())),
                BinaryOp::Or => Ok(Value::Bool(left.is_truthy() || right.is_truthy())),
            }
        }

        Expr::Unary(u) => {
            let operand = eval_expr(&u.operand, env)?;
            match u.op {
                UnaryOp::Neg => {
                    let n = operand.as_f64().unwrap_or(0.0);
                    Ok(Value::Number(-n))
                }
                UnaryOp::Not => Ok(Value::Bool(!operand.is_truthy())),
            }
        }

        Expr::Ternary(t) => {
            let cond = eval_expr(&t.condition, env)?;
            if cond.is_truthy() {
                eval_expr(&t.then_expr, env)
            } else {
                eval_expr(&t.else_expr, env)
            }
        }

        Expr::Call(c) => {
            let evaluated_args: Vec<Value> = c
                .args
                .iter()
                .map(|arg| eval_expr(arg, env))
                .collect::<Result<_, _>>()?;

            match c.callee.as_str() {
                "max" => {
                    let max_val = evaluated_args
                        .iter()
                        .filter_map(|v| v.as_f64())
                        .fold(f64::NEG_INFINITY, f64::max);
                    Ok(Value::Number(if max_val.is_finite() { max_val } else { 0.0 }))
                }
                "min" => {
                    let min_val = evaluated_args
                        .iter()
                        .filter_map(|v| v.as_f64())
                        .fold(f64::INFINITY, f64::min);
                    Ok(Value::Number(if min_val.is_finite() { min_val } else { 0.0 }))
                }
                "sum" => {
                    let sum_val: f64 = evaluated_args.iter().filter_map(|v| v.as_f64()).sum();
                    Ok(Value::Number(sum_val))
                }
                "clamp" => {
                    if evaluated_args.len() >= 3 {
                        let val = evaluated_args[0].as_f64().unwrap_or(0.0);
                        let min = evaluated_args[1].as_f64().unwrap_or(0.0);
                        let max = evaluated_args[2].as_f64().unwrap_or(0.0);
                        Ok(Value::Number(val.clamp(min, max)))
                    } else {
                        Err(CompileError::Custom {
                            message: "clamp() expects 3 arguments: clamp(value, min, max)".to_string(),
                            span: c.span,
                        })
                    }
                }
                "text_height" => {
                    let text = evaluated_args.first().and_then(|v| v.as_str()).unwrap_or("");
                    let size = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(16.0);
                    let weight = evaluated_args.get(2).and_then(|v| v.as_f64()).unwrap_or(400.0);
                    let family = evaluated_args.get(3).and_then(|v| v.as_str());
                    let max_width = evaluated_args.get(4).and_then(|v| v.as_f64()).unwrap_or(0.0);

                    let h = crate::compiler::text::measure_text_height(text, size, weight, family, max_width);
                    Ok(Value::Number(h))
                }
                "text_width" => {
                    let text = evaluated_args.first().and_then(|v| v.as_str()).unwrap_or("");
                    let size = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(16.0);
                    let weight = evaluated_args.get(2).and_then(|v| v.as_f64()).unwrap_or(400.0);
                    let family = evaluated_args.get(3).and_then(|v| v.as_str());

                    let w = crate::compiler::text::measure_text_width(text, size, weight, family);
                    Ok(Value::Number(w))
                }
                other => Err(CompileError::Custom {
                    message: format!("Unknown math/collection function '{}'", other),
                    span: c.span,
                }),
            }
        }

        Expr::Paren(inner, _) => eval_expr(inner, env),
    }
}
