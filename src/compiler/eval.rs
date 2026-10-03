use crate::ast::{BinaryOp, Expr, Literal, UnaryOp};
use crate::compiler::error::CompileError;
use crate::compiler::expanded::NodeId;
use crate::compiler::graph::{VarId, VariableGraph};
use crate::compiler::topo::TopologicalSchedule;
use crate::compiler::value::Value;
use std::collections::{HashMap, HashSet};

/// Evaluates all variables in the graph according to the topological schedule,
/// returning an environment mapping each `VarId` to its computed `Value`.
pub fn evaluate_graph(
    graph: &VariableGraph,
    schedule: &TopologicalSchedule,
) -> Result<HashMap<VarId, Value>, CompileError> {
    evaluate_graph_with_state(graph, schedule, &HashMap::new())
}

/// Evaluates all variables in the graph according to the topological schedule,
/// injecting any active runtime state overrides into their corresponding cells.
pub fn evaluate_graph_with_state(
    graph: &VariableGraph,
    schedule: &TopologicalSchedule,
    state_overrides: &HashMap<VarId, Value>,
) -> Result<HashMap<VarId, Value>, CompileError> {
    let mut env = HashMap::with_capacity(schedule.len());

    for var_id in schedule.iter() {
        if let Some(override_val) = state_overrides.get(var_id) {
            env.insert(var_id.clone(), override_val.clone());
        } else if let Some(node) = graph.get_variable(var_id) {
            let val = eval_expr(&node.equation, &env)?;
            env.insert(var_id.clone(), val);
        }
    }

    Ok(env)
}

/// Computes the complete set of transitively downstream variables in the graph
/// affected by changes to `dirty_roots`.
pub fn find_downstream_dependents(
    graph: &VariableGraph,
    dirty_roots: &[VarId],
) -> HashSet<VarId> {
    let mut dirty = HashSet::new();
    let mut queue = Vec::new();

    for root in dirty_roots {
        dirty.insert(root.clone());
        queue.push(root.clone());
    }

    while let Some(current) = queue.pop() {
        if let Some(downstream) = graph.downstream.get(&current) {
            for dep in downstream {
                if dirty.insert(dep.clone()) {
                    queue.push(dep.clone());
                }
            }
        }
    }

    dirty
}

/// Incrementally re-evaluates all variables downstream of `dirty_roots` in topological order.
///
/// Returns the set of `VarId`s whose computed values actually changed.
pub fn invalidate_and_reevaluate(
    graph: &VariableGraph,
    schedule: &TopologicalSchedule,
    current_values: &mut HashMap<VarId, Value>,
    dirty_roots: &[VarId],
    state_overrides: &HashMap<VarId, Value>,
) -> Result<HashSet<VarId>, CompileError> {
    let dirty = find_downstream_dependents(graph, dirty_roots);
    let mut changed = HashSet::new();

    for var_id in schedule.iter() {
        if dirty.contains(var_id) {
            let new_val = if let Some(override_val) = state_overrides.get(var_id) {
                override_val.clone()
            } else if let Some(node) = graph.get_variable(var_id) {
                eval_expr(&node.equation, current_values)?
            } else {
                continue;
            };

            let prev_val = current_values.insert(var_id.clone(), new_val.clone());
            if prev_val.as_ref() != Some(&new_val) {
                changed.insert(var_id.clone());
            }
        }
    }

    Ok(changed)
}

fn get_family_from_val<'a>(val: Option<&'a Value>, env: &'a HashMap<VarId, Value>) -> Option<&'a str> {
    match val {
        Some(Value::String(s)) => Some(s.as_str()),
        Some(Value::Node(id)) => {
            env.get(&VarId::new(*id, "family"))
                .or_else(|| env.get(&VarId::new(*id, "font")))
                .and_then(|v| v.as_str())
        }
        _ => None,
    }
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
            other => {
                if let Some(node_id) = NodeId::from_canonical_name(other) {
                    return Ok(Value::Node(node_id));
                }
                Err(CompileError::Custom {
                    message: format!("Unresolved identifier in expression: '{}'", other),
                    span: id.span,
                })
            }
        },

        Expr::MemberAccess(m) => {
            let target_node_id = match m.target.as_ref() {
                Expr::Ident(target_id) => NodeId::from_canonical_name(target_id.as_str()),
                other => match eval_expr(other, env)? {
                    Value::Node(id) => Some(id),
                    _ => None,
                },
            };

            if let Some(node_id) = target_node_id {
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
                    } else if matches!(left, Value::String(_)) || matches!(right, Value::String(_)) {
                        Ok(Value::String(format!(
                            "{}{}",
                            left.to_display_string(),
                            right.to_display_string()
                        )))
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
                    let family = get_family_from_val(evaluated_args.get(3), env);
                    let max_width = evaluated_args.get(4).and_then(|v| v.as_f64()).unwrap_or(0.0);

                    let h = crate::compiler::text::measure_text_height(text, size, weight, family, max_width);
                    Ok(Value::Number(h))
                }
                "text_width" => {
                    let text = evaluated_args.first().and_then(|v| v.as_str()).unwrap_or("");
                    let size = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(16.0);
                    let weight = evaluated_args.get(2).and_then(|v| v.as_f64()).unwrap_or(400.0);
                    let family = get_family_from_val(evaluated_args.get(3), env);

                    let w = crate::compiler::text::measure_text_width(text, size, weight, family);
                    Ok(Value::Number(w))
                }
                "font_cap_height" => {
                    let size = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(16.0);
                    let weight = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(400.0);
                    let family = get_family_from_val(evaluated_args.get(2), env);

                    let m = crate::compiler::text::measure_font_metrics(size, weight, family);
                    Ok(Value::Number(m.cap_height))
                }
                "font_x_height" => {
                    let size = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(16.0);
                    let weight = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(400.0);
                    let family = get_family_from_val(evaluated_args.get(2), env);

                    let m = crate::compiler::text::measure_font_metrics(size, weight, family);
                    Ok(Value::Number(m.x_height))
                }
                "font_descent" => {
                    let size = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(16.0);
                    let weight = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(400.0);
                    let family = get_family_from_val(evaluated_args.get(2), env);

                    let m = crate::compiler::text::measure_font_metrics(size, weight, family);
                    Ok(Value::Number(m.descent))
                }
                "font_ascent" => {
                    let size = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(16.0);
                    let weight = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(400.0);
                    let family = get_family_from_val(evaluated_args.get(2), env);

                    let m = crate::compiler::text::measure_font_metrics(size, weight, family);
                    Ok(Value::Number(m.ascent))
                }
                "font_line_height" => {
                    let size = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(16.0);
                    let weight = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(400.0);
                    let family = get_family_from_val(evaluated_args.get(2), env);

                    let m = crate::compiler::text::measure_font_metrics(size, weight, family);
                    Ok(Value::Number(m.line_height))
                }
                other => Err(CompileError::Custom {
                    message: format!("Unknown math/collection function '{}'", other),
                    span: c.span,
                }),
            }
        }

        Expr::Paren(inner, _) => eval_expr(inner, env),
        Expr::Node(n) => Err(CompileError::Custom {
            message: format!("Unexpanded node in expression: '{}'", n.name.as_str()),
            span: n.span,
        }),
    }
}
