use crate::ast::{BinaryOp, Expr, Literal, Symbol, UnaryOp};
use crate::compiler::error::CompileError;
use crate::compiler::expanded::NodeId;
use crate::compiler::graph::{VarId, VariableGraph};
use crate::compiler::topo::TopologicalSchedule;
use crate::compiler::value::Value;
use std::collections::{HashMap, HashSet};

pub trait EvalEnv {
    fn get_var_sym(&self, node: NodeId, port: Symbol) -> Option<&Value>;

    #[inline]
    fn get_var(&self, node: NodeId, port: &str) -> Option<&Value> {
        self.get_var_sym(node, Symbol::new(port))
    }
}

impl EvalEnv for HashMap<VarId, Value> {
    #[inline]
    fn get_var_sym(&self, node: NodeId, port: Symbol) -> Option<&Value> {
        self.get(&VarId::from_symbol(node, port))
    }
}

struct IndexedEvalEnv<'a> {
    graph: &'a VariableGraph,
    slots: &'a [Option<Value>],
}

impl<'a> EvalEnv for IndexedEvalEnv<'a> {
    #[inline]
    fn get_var_sym(&self, node: NodeId, port: Symbol) -> Option<&Value> {
        let idx = self.graph.lookup_var_index_sym(node, port)?;
        self.slots.get(idx.0 as usize)?.as_ref()
    }
}

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
    let num_vars = graph.var_nodes.len();
    if schedule.order_indices.len() == num_vars {
        let mut slots: Vec<Option<Value>> = vec![None; num_vars];
        let has_overrides = !state_overrides.is_empty();

        for &var_idx in &schedule.order_indices {
            let idx = var_idx.0 as usize;
            let node = &graph.var_nodes[idx];
            if has_overrides {
                if let Some(override_val) = state_overrides.get(&node.id) {
                    if node.id.port == Symbol::FAMILY {
                        if let Value::String(ref s) = override_val {
                            if !s.is_empty() && !crate::compiler::text::font_family_exists(s) {
                                return Err(CompileError::FontFamilyNotFound {
                                    family: s.clone(),
                                    span: node.span,
                                });
                            }
                        }
                    }
                    slots[idx] = Some(override_val.clone());
                    continue;
                }
            }
            let val = eval_expr_with_env(&node.equation, &IndexedEvalEnv { graph, slots: &slots })?;
            if node.id.port == Symbol::FAMILY {
                if let Value::String(ref s) = val {
                    if !s.is_empty() && !crate::compiler::text::font_family_exists(s) {
                        return Err(CompileError::FontFamilyNotFound {
                            family: s.clone(),
                            span: node.span,
                        });
                    }
                }
            }
            slots[idx] = Some(val);
        }

        let mut env = HashMap::with_capacity(num_vars);
        for (node, slot) in graph.var_nodes.iter().zip(slots.into_iter()) {
            if let Some(val) = slot {
                env.insert(node.id, val);
            }
        }
        return Ok(env);
    }

    let mut env = HashMap::with_capacity(schedule.len());

    for var_id in schedule.iter() {
        if let Some(override_val) = state_overrides.get(var_id) {
            if var_id.port == Symbol::FAMILY {
                if let Value::String(ref s) = override_val {
                    if !s.is_empty() && !crate::compiler::text::font_family_exists(s) {
                        let span = graph
                            .get_variable(var_id)
                            .map(|n| n.span)
                            .unwrap_or_default();
                        return Err(CompileError::FontFamilyNotFound {
                            family: s.clone(),
                            span,
                        });
                    }
                }
            }
            env.insert(*var_id, override_val.clone());
        } else if let Some(node) = graph.get_variable(var_id) {
            let val = eval_expr(&node.equation, &env)?;
            if var_id.port == Symbol::FAMILY {
                if let Value::String(ref s) = val {
                    if !s.is_empty() && !crate::compiler::text::font_family_exists(s) {
                        return Err(CompileError::FontFamilyNotFound {
                            family: s.clone(),
                            span: node.span,
                        });
                    }
                }
            }
            env.insert(*var_id, val);
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
    let mut visited_idx = HashSet::new();
    let mut queue = Vec::new();

    for root in dirty_roots {
        dirty.insert(*root);
        if let Some(idx) = graph.lookup_var_index_sym(root.node, root.port) {
            if visited_idx.insert(idx) {
                queue.push(idx);
            }
        }
    }

    while let Some(curr_idx) = queue.pop() {
        if let Some(downstream) = graph.downstream_idx.get(curr_idx.0 as usize) {
            for &dep_idx in downstream {
                if visited_idx.insert(dep_idx) {
                    dirty.insert(graph.var_nodes[dep_idx.0 as usize].id);
                    queue.push(dep_idx);
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
    if dirty_roots.is_empty() {
        return Ok(HashSet::new());
    }

    let num_vars = graph.var_nodes.len();
    if schedule.order_indices.len() == num_vars && graph.downstream_idx.len() == num_vars {
        let mut dirty_mask = vec![false; num_vars];
        let mut any_dirty = false;
        for root in dirty_roots {
            if let Some(idx) = graph.lookup_var_index_sym(root.node, root.port) {
                dirty_mask[idx.0 as usize] = true;
                any_dirty = true;
            }
        }
        if !any_dirty {
            return Ok(HashSet::new());
        }

        let mut changed = HashSet::new();
        for &var_idx in &schedule.order_indices {
            let idx = var_idx.0 as usize;
            if !dirty_mask[idx] {
                continue;
            }
            let node = &graph.var_nodes[idx];
            let new_val = if let Some(override_val) = state_overrides.get(&node.id) {
                override_val.clone()
            } else {
                eval_expr(&node.equation, current_values)?
            };

            if node.id.port == Symbol::FAMILY {
                if let Value::String(ref s) = new_val {
                    if !s.is_empty() && !crate::compiler::text::font_family_exists(s) {
                        return Err(CompileError::FontFamilyNotFound {
                            family: s.clone(),
                            span: node.span,
                        });
                    }
                }
            }

            if current_values.get(&node.id) != Some(&new_val) {
                current_values.insert(node.id, new_val);
                changed.insert(node.id);
                for &down_idx in &graph.downstream_idx[idx] {
                    dirty_mask[down_idx.0 as usize] = true;
                }
            }
        }

        return Ok(changed);
    }

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

            if var_id.port == Symbol::FAMILY {
                if let Value::String(ref s) = new_val {
                    if !s.is_empty() && !crate::compiler::text::font_family_exists(s) {
                        let span = graph
                            .get_variable(var_id)
                            .map(|n| n.span)
                            .unwrap_or_default();
                        return Err(CompileError::FontFamilyNotFound {
                            family: s.clone(),
                            span,
                        });
                    }
                }
            }

            let prev_val = current_values.insert(*var_id, new_val.clone());
            if prev_val.as_ref() != Some(&new_val) {
                changed.insert(*var_id);
            }
        }
    }

    Ok(changed)
}

fn get_family_from_val<'a, E: EvalEnv>(val: Option<&'a Value>, env: &'a E) -> Option<&'a str> {
    match val {
        Some(Value::String(s)) => Some(s.as_str()),
        Some(Value::Node(id)) => env.get_var_sym(*id, Symbol::FAMILY).and_then(|v| v.as_str()),
        _ => None,
    }
}

fn validate_font_family(family: Option<&str>, span: crate::span::Span) -> Result<(), CompileError> {
    if let Some(f) = family {
        if !f.is_empty() && !crate::compiler::text::font_family_exists(f) {
            return Err(CompileError::FontFamilyNotFound {
                family: f.to_string(),
                span,
            });
        }
    }
    Ok(())
}

/// Evaluates an algebraic expression in the given environment.
pub fn eval_expr(expr: &Expr, env: &HashMap<VarId, Value>) -> Result<Value, CompileError> {
    eval_expr_with_env(expr, env)
}

/// Evaluates an algebraic expression against any `EvalEnv` (flat slot table or `HashMap`).
pub fn eval_expr_with_env<E: EvalEnv>(expr: &Expr, env: &E) -> Result<Value, CompileError> {
    match expr {
        Expr::Literal(lit) => match lit {
            Literal::Integer(n, _) => Ok(Value::Integer(*n)),
            Literal::Float(n, _) => Ok(Value::Float(*n)),
            Literal::Distance(d, unit, span) => match unit {
                crate::ast::DistanceUnit::Px => Ok(Value::Distance(*d)),
                crate::ast::DistanceUnit::Pt => {
                    let dpi = env
                        .get_var_sym(NodeId::WINDOW, Symbol::DPI)
                        .and_then(|v| v.as_f64())
                        .unwrap_or(96.0);
                    Ok(Value::Distance(*d * (dpi / 72.0)))
                }
                crate::ast::DistanceUnit::Em => Err(CompileError::UndefinedEnvVariable {
                    name: "font".to_string(),
                    span: *span,
                }),
            },
            Literal::Area(a, unit, span) => match unit {
                crate::ast::DistanceUnit::Px => Ok(Value::Area(*a)),
                crate::ast::DistanceUnit::Pt => {
                    let dpi = env
                        .get_var_sym(NodeId::WINDOW, Symbol::DPI)
                        .and_then(|v| v.as_f64())
                        .unwrap_or(96.0);
                    let factor = dpi / 72.0;
                    Ok(Value::Area(*a * factor * factor))
                }
                crate::ast::DistanceUnit::Em => Err(CompileError::UndefinedEnvVariable {
                    name: "font".to_string(),
                    span: *span,
                }),
            },
            Literal::Duration(d, _, _) => Ok(Value::Duration(*d)),
            Literal::String(s, _) => Ok(Value::String(s.clone())),
            Literal::Bool(b, _) => Ok(Value::Bool(*b)),
            Literal::Color(c, _) => Ok(Value::Color(c.clone())),
            Literal::Enum(e, v, _) => Ok(Value::Enum {
                enum_name: e.clone(),
                variant: v.clone(),
            }),
        },

        Expr::Ident(id) => match id.name {
            Symbol::TRUE => Ok(Value::Bool(true)),
            Symbol::FALSE => Ok(Value::Bool(false)),
            Symbol::UNSET | Symbol::NONE => Ok(Value::Unset),
            sym => {
                if let Some(node_id) = sym.node_id() {
                    return Ok(Value::Node(node_id));
                }
                Err(CompileError::UnresolvedIdentifier {
                    name: sym.as_str().to_string(),
                    span: id.span,
                })
            }
        },

        Expr::MemberAccess(m) => {
            if let Expr::Ident(target_id) = m.target.as_ref() {
                if let Some(node_id) = target_id.node_id() {
                    if let Some(val) = env.get_var_sym(node_id, m.member.name) {
                        return Ok(val.clone());
                    } else {
                        return Err(CompileError::Custom {
                            message: format!(
                                "Variable '{}.{}' evaluated before being set in environment",
                                node_id.canonical_str(),
                                m.member.as_str()
                            ),
                            span: m.span,
                        });
                    }
                }
            }

            let target_val = eval_expr_with_env(&m.target, env)?;
            match target_val {
                Value::Node(node_id) => {
                    if let Some(val) = env.get_var_sym(node_id, m.member.name) {
                        Ok(val.clone())
                    } else {
                        Err(CompileError::Custom {
                            message: format!(
                                "Variable '{}.{}' evaluated before being set in environment",
                                node_id.canonical_str(),
                                m.member.as_str()
                            ),
                            span: m.span,
                        })
                    }
                }
                Value::Timer {
                    time,
                    duration,
                    dt,
                    frames,
                } => match m.member.as_str() {
                    "time" => Ok(Value::Duration(time)),
                    "duration" => Ok(Value::Duration(duration)),
                    "elapsed" => Ok(Value::Float(if duration > 0 {
                        (time as f64 / duration as f64).clamp(0.0, 1.0)
                    } else {
                        1.0
                    })),
                    "dt" => Ok(Value::Duration(dt)),
                    "frames" => Ok(Value::Integer(frames)),
                    other => Err(CompileError::Custom {
                        message: format!("Node 'Timer' has no public property '{}'", other),
                        span: m.span,
                    }),
                },
                Value::IndefiniteTimer { time, dt, frames } => match m.member.as_str() {
                    "time" => Ok(Value::Duration(time)),
                    "dt" => Ok(Value::Duration(dt)),
                    "frames" => Ok(Value::Integer(frames)),
                    other => Err(CompileError::Custom {
                        message: format!(
                            "Node 'IndefiniteTimer' has no public property '{}'",
                            other
                        ),
                        span: m.span,
                    }),
                },
                Value::Unset => Err(CompileError::Custom {
                    message: format!("Cannot access property '{}' on an unset value", m.member),
                    span: m.span,
                }),
                _ => Err(CompileError::Custom {
                    message: format!("Unsupported member access target in evaluation: {:?}", m.target),
                    span: m.span,
                }),
            }
        }

        Expr::Binary(b) => {
            let left = eval_expr_with_env(&b.left, env)?;
            let right = eval_expr_with_env(&b.right, env)?;

            match b.op {
                BinaryOp::Add => match (&left, &right) {
                    (Value::Integer(l), Value::Integer(r)) => Ok(Value::Integer(l.wrapping_add(*r))),
                    (Value::Float(l), Value::Float(r)) => Ok(Value::Float(l + r)),
                    (Value::Integer(l), Value::Float(r)) => Ok(Value::Float(*l as f64 + r)),
                    (Value::Float(l), Value::Integer(r)) => Ok(Value::Float(l + *r as f64)),
                    (Value::Distance(l), Value::Distance(r)) => Ok(Value::Distance(l + r)),
                    (Value::Distance(l), Value::Integer(0))
                    | (Value::Integer(0), Value::Distance(l)) => Ok(Value::Distance(*l)),
                    (Value::Area(l), Value::Area(r)) => Ok(Value::Area(l + r)),
                    (Value::Area(l), Value::Integer(0))
                    | (Value::Integer(0), Value::Area(l)) => Ok(Value::Area(*l)),
                    (Value::Duration(l), Value::Duration(r)) => {
                        Ok(Value::Duration(l.wrapping_add(*r)))
                    }
                    (Value::Duration(l), Value::Integer(0))
                    | (Value::Integer(0), Value::Duration(l)) => Ok(Value::Duration(*l)),
                    (Value::String(_), _) | (_, Value::String(_)) => Ok(Value::String(format!(
                        "{}{}",
                        left.to_display_string(),
                        right.to_display_string()
                    ))),
                    _ => Err(CompileError::Custom {
                        message: format!(
                            "Cannot add {} ({}) and {} ({})",
                            left,
                            left.type_name(),
                            right,
                            right.type_name()
                        ),
                        span: b.span,
                    }),
                },
                BinaryOp::Sub => match (&left, &right) {
                    (Value::Integer(l), Value::Integer(r)) => Ok(Value::Integer(l.wrapping_sub(*r))),
                    (Value::Float(l), Value::Float(r)) => Ok(Value::Float(l - r)),
                    (Value::Integer(l), Value::Float(r)) => Ok(Value::Float(*l as f64 - r)),
                    (Value::Float(l), Value::Integer(r)) => Ok(Value::Float(l - *r as f64)),
                    (Value::Distance(l), Value::Distance(r)) => Ok(Value::Distance(l - r)),
                    (Value::Distance(l), Value::Integer(0)) => Ok(Value::Distance(*l)),
                    (Value::Integer(0), Value::Distance(r)) => Ok(Value::Distance(-*r)),
                    (Value::Area(l), Value::Area(r)) => Ok(Value::Area(l - r)),
                    (Value::Area(l), Value::Integer(0)) => Ok(Value::Area(*l)),
                    (Value::Integer(0), Value::Area(r)) => Ok(Value::Area(-*r)),
                    (Value::Duration(l), Value::Duration(r)) => {
                        Ok(Value::Duration(l.wrapping_sub(*r)))
                    }
                    (Value::Duration(l), Value::Integer(0)) => Ok(Value::Duration(*l)),
                    (Value::Integer(0), Value::Duration(r)) => {
                        Ok(Value::Duration(r.wrapping_neg()))
                    }
                    _ => Err(CompileError::Custom {
                        message: format!(
                            "Cannot subtract {} ({}) from {} ({})",
                            right,
                            right.type_name(),
                            left,
                            left.type_name()
                        ),
                        span: b.span,
                    }),
                },
                BinaryOp::Mul => match (&left, &right) {
                    (Value::Integer(l), Value::Integer(r)) => Ok(Value::Integer(l.wrapping_mul(*r))),
                    (Value::Float(l), Value::Float(r)) => Ok(Value::Float(l * r)),
                    (Value::Integer(l), Value::Float(r)) => Ok(Value::Float(*l as f64 * r)),
                    (Value::Float(l), Value::Integer(r)) => Ok(Value::Float(l * *r as f64)),
                    (Value::Distance(d), Value::Integer(i))
                    | (Value::Integer(i), Value::Distance(d)) => Ok(Value::Distance(d * *i as f64)),
                    (Value::Distance(d), Value::Float(f))
                    | (Value::Float(f), Value::Distance(d)) => Ok(Value::Distance(d * f)),
                    (Value::Distance(l), Value::Distance(r)) => Ok(Value::Area(l * r)),
                    (Value::Area(a), Value::Integer(i))
                    | (Value::Integer(i), Value::Area(a)) => Ok(Value::Area(a * *i as f64)),
                    (Value::Area(a), Value::Float(f))
                    | (Value::Float(f), Value::Area(a)) => Ok(Value::Area(a * f)),
                    (Value::Duration(d), Value::Integer(i))
                    | (Value::Integer(i), Value::Duration(d)) => {
                        Ok(Value::Duration(d.wrapping_mul(*i)))
                    }
                    (Value::Duration(d), Value::Float(f))
                    | (Value::Float(f), Value::Duration(d)) => {
                        Ok(Value::Duration((*d as f64 * f).round() as i64))
                    }
                    _ => Err(CompileError::Custom {
                        message: format!(
                            "Cannot multiply {} ({}) and {} ({})",
                            left,
                            left.type_name(),
                            right,
                            right.type_name()
                        ),
                        span: b.span,
                    }),
                },
                BinaryOp::Div => match (&left, &right) {
                    (Value::Integer(l), Value::Integer(r)) => {
                        if *r == 0 {
                            Ok(Value::Float(0.0))
                        } else {
                            Ok(Value::Float(*l as f64 / *r as f64))
                        }
                    }
                    (Value::Float(l), Value::Float(r)) => {
                        if *r == 0.0 {
                            Ok(Value::Float(0.0))
                        } else {
                            Ok(Value::Float(l / r))
                        }
                    }
                    (Value::Integer(l), Value::Float(r)) => {
                        if *r == 0.0 {
                            Ok(Value::Float(0.0))
                        } else {
                            Ok(Value::Float(*l as f64 / r))
                        }
                    }
                    (Value::Float(l), Value::Integer(r)) => {
                        if *r == 0 {
                            Ok(Value::Float(0.0))
                        } else {
                            Ok(Value::Float(l / *r as f64))
                        }
                    }
                    (Value::Distance(d), Value::Integer(r)) => {
                        if *r == 0 {
                            Ok(Value::Distance(0.0))
                        } else {
                            Ok(Value::Distance(d / *r as f64))
                        }
                    }
                    (Value::Distance(d), Value::Float(r)) => {
                        if *r == 0.0 {
                            Ok(Value::Distance(0.0))
                        } else {
                            Ok(Value::Distance(d / r))
                        }
                    }
                    (Value::Distance(l), Value::Distance(r)) => {
                        if *r == 0.0 {
                            Ok(Value::Float(0.0))
                        } else {
                            Ok(Value::Float(l / r))
                        }
                    }
                    (Value::Integer(0), Value::Distance(_)) => Ok(Value::Float(0.0)),
                    (Value::Area(a), Value::Distance(d)) => {
                        if *d == 0.0 {
                            Ok(Value::Distance(0.0))
                        } else {
                            Ok(Value::Distance(a / d))
                        }
                    }
                    (Value::Integer(0), Value::Area(_)) => Ok(Value::Float(0.0)),
                    (Value::Area(l), Value::Area(r)) => {
                        if *r == 0.0 {
                            Ok(Value::Float(0.0))
                        } else {
                            Ok(Value::Float(l / r))
                        }
                    }
                    (Value::Area(a), Value::Integer(r)) => {
                        if *r == 0 {
                            Ok(Value::Area(0.0))
                        } else {
                            Ok(Value::Area(a / *r as f64))
                        }
                    }
                    (Value::Area(a), Value::Float(r)) => {
                        if *r == 0.0 {
                            Ok(Value::Area(0.0))
                        } else {
                            Ok(Value::Area(a / r))
                        }
                    }
                    (Value::Integer(0), Value::Duration(_)) => Ok(Value::Float(0.0)),
                    (Value::Duration(l), Value::Duration(r)) => {
                        if *r == 0 {
                            Ok(Value::Float(0.0))
                        } else {
                            Ok(Value::Float(*l as f64 / *r as f64))
                        }
                    }
                    (Value::Duration(d), Value::Integer(r)) => {
                        if *r == 0 {
                            Ok(Value::Duration(0))
                        } else {
                            Ok(Value::Duration((*d as f64 / *r as f64).round() as i64))
                        }
                    }
                    (Value::Duration(d), Value::Float(r)) => {
                        if *r == 0.0 {
                            Ok(Value::Duration(0))
                        } else {
                            Ok(Value::Duration((*d as f64 / r).round() as i64))
                        }
                    }
                    _ => Err(CompileError::Custom {
                        message: format!(
                            "Cannot divide {} ({}) by {} ({})",
                            left,
                            left.type_name(),
                            right,
                            right.type_name()
                        ),
                        span: b.span,
                    }),
                },
                BinaryOp::Rem => match (&left, &right) {
                    (Value::Integer(l), Value::Integer(r)) => {
                        if *r == 0 {
                            Ok(Value::Integer(0))
                        } else {
                            Ok(Value::Integer(l % r))
                        }
                    }
                    (Value::Float(l), Value::Float(r)) => {
                        if *r == 0.0 {
                            Ok(Value::Float(0.0))
                        } else {
                            Ok(Value::Float(l % r))
                        }
                    }
                    (Value::Integer(l), Value::Float(r)) => {
                        if *r == 0.0 {
                            Ok(Value::Float(0.0))
                        } else {
                            Ok(Value::Float((*l as f64) % r))
                        }
                    }
                    (Value::Float(l), Value::Integer(r)) => {
                        if *r == 0 {
                            Ok(Value::Float(0.0))
                        } else {
                            Ok(Value::Float(l % (*r as f64)))
                        }
                    }
                    (Value::Distance(l), Value::Distance(r)) => {
                        if *r == 0.0 {
                            Ok(Value::Distance(0.0))
                        } else {
                            Ok(Value::Distance(l % r))
                        }
                    }
                    (Value::Area(l), Value::Area(r)) => {
                        if *r == 0.0 {
                            Ok(Value::Area(0.0))
                        } else {
                            Ok(Value::Area(l % r))
                        }
                    }
                    (Value::Duration(l), Value::Duration(r)) => {
                        if *r == 0 {
                            Ok(Value::Duration(0))
                        } else {
                            Ok(Value::Duration(l % r))
                        }
                    }
                    _ => Err(CompileError::Custom {
                        message: format!(
                            "Cannot compute remainder of {} ({}) and {} ({})",
                            left,
                            left.type_name(),
                            right,
                            right.type_name()
                        ),
                        span: b.span,
                    }),
                },
                BinaryOp::Eq => Ok(Value::Bool(values_equal(&left, &right))),
                BinaryOp::Ne => Ok(Value::Bool(!values_equal(&left, &right))),
                BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                    let (l, r) = extract_comparable_pair(&left, &right, b.span)?;
                    let res = match b.op {
                        BinaryOp::Lt => l < r,
                        BinaryOp::Le => l <= r,
                        BinaryOp::Gt => l > r,
                        BinaryOp::Ge => l >= r,
                        _ => unreachable!(),
                    };
                    Ok(Value::Bool(res))
                }
                BinaryOp::And => {
                    let l = eval_truthy(&left, b.left.span())?;
                    let r = eval_truthy(&right, b.right.span())?;
                    Ok(Value::Bool(l && r))
                }
                BinaryOp::Or => {
                    let l = eval_truthy(&left, b.left.span())?;
                    let r = eval_truthy(&right, b.right.span())?;
                    Ok(Value::Bool(l || r))
                }
            }
        }

        Expr::Unary(u) => {
            let operand = eval_expr_with_env(&u.operand, env)?;
            match u.op {
                UnaryOp::Neg => match operand {
                    Value::Integer(n) => Ok(Value::Integer(-n)),
                    Value::Float(n) => Ok(Value::Float(-n)),
                    Value::Distance(d) => Ok(Value::Distance(-d)),
                    Value::Area(a) => Ok(Value::Area(-a)),
                    Value::Duration(d) => Ok(Value::Duration(d.wrapping_neg())),
                    other => Err(CompileError::Custom {
                        message: format!("Cannot negate value of type {}", other.type_name()),
                        span: u.span,
                    }),
                },
                UnaryOp::Not => {
                    let b = eval_truthy(&operand, u.operand.span())?;
                    Ok(Value::Bool(!b))
                }
            }
        }

        Expr::Ternary(t) => {
            let cond = eval_expr_with_env(&t.condition, env)?;
            if eval_truthy(&cond, t.condition.span())? {
                let val = eval_expr_with_env(&t.then_expr, env)?;
                if matches!(val, Value::Integer(0)) {
                    match eval_expr_with_env(&t.else_expr, env) {
                        Ok(Value::Distance(_)) => return Ok(Value::Distance(0.0)),
                        Ok(Value::Area(_)) => return Ok(Value::Area(0.0)),
                        Ok(Value::Duration(_)) => return Ok(Value::Duration(0)),
                        _ => {}
                    }
                }
                Ok(val)
            } else {
                let val = eval_expr_with_env(&t.else_expr, env)?;
                if matches!(val, Value::Integer(0)) {
                    match eval_expr_with_env(&t.then_expr, env) {
                        Ok(Value::Distance(_)) => return Ok(Value::Distance(0.0)),
                        Ok(Value::Area(_)) => return Ok(Value::Area(0.0)),
                        Ok(Value::Duration(_)) => return Ok(Value::Duration(0)),
                        _ => {}
                    }
                }
                Ok(val)
            }
        }

        Expr::Call(c) => {
            if c.callee.as_str() == "distance_to_float" {
                if c.args.len() != 2 {
                    return Err(CompileError::Custom {
                        message: "distance_to_float() expects 2 arguments: distance_to_float(distance, unit)".to_string(),
                        span: c.span,
                    });
                }
                let d_val = eval_expr_with_env(&c.args[0], env)?;
                let d_px = match d_val {
                    Value::Distance(d) => d,
                    Value::Integer(0) => 0.0,
                    other => {
                        return Err(CompileError::Custom {
                            message: format!(
                                "distance_to_float() first argument must be a Distance, found {}",
                                other.type_name()
                            ),
                            span: c.args[0].span(),
                        });
                    }
                };
                let unit_px = match &c.args[1] {
                    Expr::Ident(unit_id) => match unit_id.as_str() {
                        "px" => 1.0,
                        "pt" => {
                            let dpi = env
                                .get_var(NodeId::WINDOW, "dpi")
                                .and_then(|v| v.as_f64())
                                .unwrap_or(96.0);
                            dpi / 72.0
                        }
                        "em" => {
                            return Err(CompileError::UndefinedEnvVariable {
                                name: "font".to_string(),
                                span: unit_id.span,
                            });
                        }
                        other => {
                            return Err(CompileError::Custom {
                                message: format!(
                                    "Unknown unit '{}' in distance_to_float(). Supported units are 'px', 'pt', and 'em'",
                                    other
                                ),
                                span: unit_id.span,
                            });
                        }
                    },
                    other_expr => {
                        let unit_val = eval_expr_with_env(other_expr, env)?;
                        unit_val.as_f64().unwrap_or(1.0)
                    }
                };
                if unit_px == 0.0 {
                    return Ok(Value::Float(0.0));
                }
                return Ok(Value::Float(d_px / unit_px));
            }

            let evaluated_args: Vec<Value> = c
                .args
                .iter()
                .map(|arg| eval_expr_with_env(arg, env))
                .collect::<Result<_, _>>()?;

            match c.callee.as_str() {
                "max" => eval_aggregate_fn("max", &evaluated_args, c.span, |vals| {
                    let m = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    if m.is_finite() { m } else { 0.0 }
                }),
                "min" => eval_aggregate_fn("min", &evaluated_args, c.span, |vals| {
                    let m = vals.iter().copied().fold(f64::INFINITY, f64::min);
                    if m.is_finite() { m } else { 0.0 }
                }),
                "sum" => eval_aggregate_fn("sum", &evaluated_args, c.span, |vals| {
                    vals.iter().copied().sum()
                }),
                "only" => {
                    if evaluated_args.len() == 1 {
                        Ok(evaluated_args[0].clone())
                    } else {
                        Err(CompileError::AssertionFailed {
                            message: format!(
                                "only() expected exactly one child or argument, found {}",
                                evaluated_args.len()
                            ),
                            span: c.span,
                        })
                    }
                }
                "clamp" => {
                    if evaluated_args.len() >= 3 {
                        eval_aggregate_fn("clamp", &evaluated_args[..3], c.span, |vals| {
                            vals[0].clamp(vals[1], vals[2])
                        })
                    } else {
                        Err(CompileError::Custom {
                            message: "clamp() expects 3 arguments: clamp(value, min, max)".to_string(),
                            span: c.span,
                        })
                    }
                }
                "round" => match evaluated_args.first() {
                    Some(Value::Distance(d)) => Ok(Value::Distance(d.round())),
                    Some(Value::Area(a)) => Ok(Value::Area(a.round())),
                    Some(Value::Duration(d)) => Ok(Value::Duration(*d)),
                    Some(Value::Float(f)) => Ok(Value::Integer(f.round() as i64)),
                    Some(Value::Integer(i)) => Ok(Value::Integer(*i)),
                    _ => Ok(Value::Integer(0)),
                },
                "floor" => match evaluated_args.first() {
                    Some(Value::Distance(d)) => Ok(Value::Distance(d.floor())),
                    Some(Value::Area(a)) => Ok(Value::Area(a.floor())),
                    Some(Value::Duration(d)) => Ok(Value::Duration(*d)),
                    Some(Value::Float(f)) => Ok(Value::Integer(f.floor() as i64)),
                    Some(Value::Integer(i)) => Ok(Value::Integer(*i)),
                    _ => Ok(Value::Integer(0)),
                },
                "ceil" => match evaluated_args.first() {
                    Some(Value::Distance(d)) => Ok(Value::Distance(d.ceil())),
                    Some(Value::Area(a)) => Ok(Value::Area(a.ceil())),
                    Some(Value::Duration(d)) => Ok(Value::Duration(*d)),
                    Some(Value::Float(f)) => Ok(Value::Integer(f.ceil() as i64)),
                    Some(Value::Integer(i)) => Ok(Value::Integer(*i)),
                    _ => Ok(Value::Integer(0)),
                },
                "abs" => match evaluated_args.first() {
                    Some(Value::Distance(d)) => Ok(Value::Distance(d.abs())),
                    Some(Value::Area(a)) => Ok(Value::Area(a.abs())),
                    Some(Value::Duration(d)) => Ok(Value::Duration(d.abs())),
                    Some(Value::Float(f)) => Ok(Value::Float(f.abs())),
                    Some(Value::Integer(i)) => Ok(Value::Integer(i.abs())),
                    _ => Ok(Value::Integer(0)),
                },
                "hypot" => {
                    if evaluated_args.len() >= 2 {
                        let a = &evaluated_args[0];
                        let b = &evaluated_args[1];
                        let has_distance =
                            matches!(a, Value::Distance(_)) || matches!(b, Value::Distance(_));
                        if has_distance {
                            eval_aggregate_fn("hypot", &evaluated_args[..2], c.span, |vals| {
                                vals[0].hypot(vals[1])
                            })
                        } else {
                            let x = a.as_f64().unwrap_or(0.0);
                            let y = b.as_f64().unwrap_or(0.0);
                            Ok(Value::Float(x.hypot(y)))
                        }
                    } else {
                        Err(CompileError::Custom {
                            message: "hypot() expects 2 arguments: hypot(dx, dy)".to_string(),
                            span: c.span,
                        })
                    }
                }
                "sqrt" => match evaluated_args.first() {
                    Some(Value::Area(a)) => Ok(Value::Distance(a.max(0.0).sqrt())),
                    first => {
                        let x = first.and_then(|v| v.as_f64()).unwrap_or(0.0);
                        Ok(Value::Float(x.max(0.0).sqrt()))
                    }
                },
                "sin" => {
                    let x = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(0.0);
                    Ok(Value::Float(x.sin()))
                }
                "cos" => {
                    let x = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(0.0);
                    Ok(Value::Float(x.cos()))
                }
                "tan" => {
                    let x = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(0.0);
                    Ok(Value::Float(x.tan()))
                }
                "atan2" => {
                    let y = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let x = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    Ok(Value::Float(y.atan2(x)))
                }
                "hsl" | "hsla" => {
                    let h = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let s = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(100.0);
                    let l = evaluated_args.get(2).and_then(|v| v.as_f64()).unwrap_or(50.0);
                    let s_norm = if s > 1.0 { (s / 100.0).clamp(0.0, 1.0) } else { s.clamp(0.0, 1.0) };
                    let l_norm = if l > 1.0 { (l / 100.0).clamp(0.0, 1.0) } else { l.clamp(0.0, 1.0) };
                    let (r, g, b) = hsl_to_rgb(h, s_norm, l_norm);
                    if c.callee.as_str() == "hsla" || evaluated_args.len() >= 4 {
                        let a = evaluated_args.get(3).and_then(|v| v.as_f64()).unwrap_or(1.0);
                        let a_u8 = (if a <= 1.0 { a.clamp(0.0, 1.0) * 255.0 } else { a.clamp(0.0, 255.0) }).round() as u8;
                        Ok(Value::Color(format!("#{:02x}{:02x}{:02x}{:02x}", r, g, b, a_u8)))
                    } else {
                        Ok(Value::Color(format!("#{:02x}{:02x}{:02x}", r, g, b)))
                    }
                }
                "rgb" | "rgba" => {
                    let r = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(0.0).clamp(0.0, 255.0).round() as u8;
                    let g = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0).clamp(0.0, 255.0).round() as u8;
                    let b = evaluated_args.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0).clamp(0.0, 255.0).round() as u8;
                    if c.callee.as_str() == "rgba" || evaluated_args.len() >= 4 {
                        let a = evaluated_args.get(3).and_then(|v| v.as_f64()).unwrap_or(1.0);
                        let a_u8 = (if a <= 1.0 { a.clamp(0.0, 1.0) * 255.0 } else { a.clamp(0.0, 255.0) }).round() as u8;
                        Ok(Value::Color(format!("#{:02x}{:02x}{:02x}{:02x}", r, g, b, a_u8)))
                    } else {
                        Ok(Value::Color(format!("#{:02x}{:02x}{:02x}", r, g, b)))
                    }
                }

                "to_fixed" => {
                    let val = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let decimals = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0).max(0.0) as usize;
                    Ok(Value::String(format_fixed(val, decimals)))
                }
                "to_precision" => {
                    let val = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let sigfigs = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(1.0).max(1.0) as usize;
                    Ok(Value::String(format_sigfigs(val, sigfigs)))
                }
                "to_px" => match evaluated_args.first() {
                    Some(Value::Distance(d)) => Ok(Value::Float(*d)),
                    Some(Value::Integer(0)) => Ok(Value::Float(0.0)),
                    Some(other) => Err(CompileError::Custom {
                        message: format!("to_px() expects a Distance, found {}", other.type_name()),
                        span: c.span,
                    }),
                    None => Err(CompileError::Custom {
                        message: "to_px() expects 1 argument".to_string(),
                        span: c.span,
                    }),
                },
                "to_pt" => match evaluated_args.first() {
                    Some(Value::Distance(d)) => {
                        let dpi = env
                            .get_var(NodeId::WINDOW, "dpi")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(96.0);
                        Ok(Value::Float(*d / (dpi / 72.0)))
                    }
                    Some(Value::Integer(0)) => Ok(Value::Float(0.0)),
                    Some(other) => Err(CompileError::Custom {
                        message: format!("to_pt() expects a Distance, found {}", other.type_name()),
                        span: c.span,
                    }),
                    None => Err(CompileError::Custom {
                        message: "to_pt() expects 1 argument".to_string(),
                        span: c.span,
                    }),
                },
                "to_float" | "to_number" => match evaluated_args.first() {
                    Some(Value::Integer(i)) => Ok(Value::Float(*i as f64)),
                    Some(Value::Float(f)) => Ok(Value::Float(*f)),
                    Some(other) => Err(CompileError::Custom {
                        message: format!(
                            "{}() expects an Integer or Float, found {}",
                            c.callee.as_str(),
                            other.type_name()
                        ),
                        span: c.span,
                    }),
                    None => Err(CompileError::Custom {
                        message: format!("{}() expects 1 argument", c.callee.as_str()),
                        span: c.span,
                    }),
                },
                "to_integer" => match evaluated_args.first() {
                    Some(Value::Integer(i)) => Ok(Value::Integer(*i)),
                    Some(Value::Float(f)) => Ok(Value::Integer(*f as i64)),
                    Some(other) => Err(CompileError::Custom {
                        message: format!("to_integer() expects an Integer or Float, found {}", other.type_name()),
                        span: c.span,
                    }),
                    None => Err(CompileError::Custom {
                        message: "to_integer() expects 1 argument".to_string(),
                        span: c.span,
                    }),
                },

                "text_height" => {
                    let text = evaluated_args.first().and_then(|v| v.as_str()).unwrap_or("");
                    let size = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(16.0);
                    let weight = evaluated_args.get(2).and_then(|v| v.as_f64()).unwrap_or(400.0);
                    let family = get_family_from_val(evaluated_args.get(3), env);
                    let max_width = evaluated_args.get(4).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let end_at = evaluated_args.get(5).and_then(|v| v.as_str());
                    let start_at = evaluated_args.get(6).and_then(|v| v.as_str());
                    let line_height = evaluated_args.get(7).and_then(|v| v.as_f64());
                    validate_font_family(family, c.span)?;

                    let mut spans = Vec::new();
                    let mut inline_boxes = Vec::new();
                    let mut idx = 8;
                    while idx + 6 < evaluated_args.len() {
                        let raw_first = evaluated_args[idx].as_f64().unwrap_or(0.0);
                        if raw_first < 0.0 {
                            let node_id = NodeId((-raw_first.round() as usize).saturating_sub(1));
                            let b_idx = evaluated_args[idx + 1].as_f64().unwrap_or(0.0).max(0.0) as usize;
                            let b_w = evaluated_args[idx + 2].as_f64().unwrap_or(0.0).max(0.0);
                            let b_h = evaluated_args[idx + 3].as_f64().unwrap_or(0.0).max(0.0);
                            let b_off = evaluated_args[idx + 4].as_f64().unwrap_or(b_h);
                            inline_boxes.push(crate::compiler::text::InlineBoxSpan {
                                node_id,
                                index: b_idx,
                                width: b_w,
                                height: b_h,
                                baseline_offset: b_off,
                            });
                        } else {
                            let s_start = raw_first.max(0.0) as usize;
                            let s_end = evaluated_args[idx + 1].as_f64().unwrap_or(0.0).max(0.0) as usize;
                            let s_size = evaluated_args[idx + 2].as_f64().filter(|&v| v > 0.0);
                            let s_weight = evaluated_args[idx + 3].as_f64().filter(|&v| v > 0.0);
                            let s_family = get_family_from_val(evaluated_args.get(idx + 4), env).map(|s| s.to_string());
                            if let Some(ref fam) = s_family {
                                validate_font_family(Some(fam.as_str()), c.span)?;
                            }
                            let s_italic = evaluated_args[idx + 5].as_bool().unwrap_or(false);
                            let s_shift = evaluated_args[idx + 6].as_f64().unwrap_or(0.0);
                            spans.push(
                                crate::compiler::text::TextSpan::new(s_start..s_end).with_style(
                                    crate::compiler::text::SpanStyle {
                                        font_size: s_size,
                                        font_weight: s_weight,
                                        font_family: s_family,
                                        italic: s_italic,
                                        baseline_shift: s_shift,
                                        ..Default::default()
                                    },
                                ),
                            );
                        }
                        idx += 7;
                    }

                    let (_, h) = crate::compiler::text::measure_rich_text_bounds_with_boxes(
                        text,
                        size,
                        weight,
                        family,
                        Some(max_width),
                        end_at,
                        start_at,
                        line_height,
                        &spans,
                        &inline_boxes,
                    );
                    Ok(Value::Distance(h))
                }
                "text_baseline_offset" => {
                    let size = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(16.0);
                    let weight = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(400.0);
                    let family = get_family_from_val(evaluated_args.get(2), env);
                    let start_at = evaluated_args.get(3).and_then(|v| v.as_str());
                    let line_height = evaluated_args.get(4).and_then(|v| v.as_f64()).filter(|&h| h > 0.0);
                    validate_font_family(family, c.span)?;

                    let offset = crate::compiler::text::text_baseline_offset(size, weight, family, start_at, line_height);
                    Ok(Value::Distance(offset))
                }
                "text_width" => {
                    let text = evaluated_args.first().and_then(|v| v.as_str()).unwrap_or("");
                    let size = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(16.0);
                    let weight = evaluated_args.get(2).and_then(|v| v.as_f64()).unwrap_or(400.0);
                    let family = get_family_from_val(evaluated_args.get(3), env);
                    validate_font_family(family, c.span)?;

                    let mut spans = Vec::new();
                    let mut inline_boxes = Vec::new();
                    let mut idx = 4;
                    while idx + 6 < evaluated_args.len() {
                        let raw_first = evaluated_args[idx].as_f64().unwrap_or(0.0);
                        if raw_first < 0.0 {
                            let node_id = NodeId((-raw_first.round() as usize).saturating_sub(1));
                            let b_idx = evaluated_args[idx + 1].as_f64().unwrap_or(0.0).max(0.0) as usize;
                            let b_w = evaluated_args[idx + 2].as_f64().unwrap_or(0.0).max(0.0);
                            let b_h = evaluated_args[idx + 3].as_f64().unwrap_or(0.0).max(0.0);
                            let b_off = evaluated_args[idx + 4].as_f64().unwrap_or(b_h);
                            inline_boxes.push(crate::compiler::text::InlineBoxSpan {
                                node_id,
                                index: b_idx,
                                width: b_w,
                                height: b_h,
                                baseline_offset: b_off,
                            });
                        } else {
                            let s_start = raw_first.max(0.0) as usize;
                            let s_end = evaluated_args[idx + 1].as_f64().unwrap_or(0.0).max(0.0) as usize;
                            let s_size = evaluated_args[idx + 2].as_f64().filter(|&v| v > 0.0);
                            let s_weight = evaluated_args[idx + 3].as_f64().filter(|&v| v > 0.0);
                            let s_family = get_family_from_val(evaluated_args.get(idx + 4), env).map(|s| s.to_string());
                            if let Some(ref fam) = s_family {
                                validate_font_family(Some(fam.as_str()), c.span)?;
                            }
                            let s_italic = evaluated_args[idx + 5].as_bool().unwrap_or(false);
                            let s_shift = evaluated_args[idx + 6].as_f64().unwrap_or(0.0);
                            spans.push(
                                crate::compiler::text::TextSpan::new(s_start..s_end).with_style(
                                    crate::compiler::text::SpanStyle {
                                        font_size: s_size,
                                        font_weight: s_weight,
                                        font_family: s_family,
                                        italic: s_italic,
                                        baseline_shift: s_shift,
                                        ..Default::default()
                                    },
                                ),
                            );
                        }
                        idx += 7;
                    }

                    let (w, _) = crate::compiler::text::measure_rich_text_bounds_with_boxes(
                        text,
                        size,
                        weight,
                        family,
                        None,
                        None,
                        None,
                        None,
                        &spans,
                        &inline_boxes,
                    );
                    Ok(Value::Distance(w))
                }
                "span_metric" => {
                    let metric = evaluated_args.first().and_then(|v| v.as_str()).unwrap_or("x");
                    let raw_span_start = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let target_inline_box = if raw_span_start < 0.0 {
                        Some(NodeId((-raw_span_start.round() as usize).saturating_sub(1)))
                    } else {
                        None
                    };
                    let span_start = raw_span_start.max(0.0) as usize;
                    let span_end = evaluated_args.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0).max(0.0) as usize;
                    let own_shift = evaluated_args.get(3).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let text = evaluated_args.get(4).and_then(|v| v.as_str()).unwrap_or("");
                    let text_x = evaluated_args.get(5).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let text_y = evaluated_args.get(6).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let max_width = evaluated_args.get(7).and_then(|v| v.as_f64()).filter(|&w| w > 0.0);
                    let size = evaluated_args.get(8).and_then(|v| v.as_f64()).unwrap_or(16.0);
                    let weight = evaluated_args.get(9).and_then(|v| v.as_f64()).unwrap_or(400.0);
                    let family = get_family_from_val(evaluated_args.get(10), env);
                    let start_at = evaluated_args.get(11).and_then(|v| v.as_str());
                    let line_height = evaluated_args.get(12).and_then(|v| v.as_f64()).filter(|&h| h > 0.0);
                    let align = evaluated_args.get(13).and_then(|v| v.as_str());
                    validate_font_family(family, c.span)?;

                    let mut spans = Vec::new();
                    let mut inline_boxes = Vec::new();
                    let mut idx = 14;
                    while idx + 6 < evaluated_args.len() {
                        let raw_first = evaluated_args[idx].as_f64().unwrap_or(0.0);
                        if raw_first < 0.0 {
                            let node_id = NodeId((-raw_first.round() as usize).saturating_sub(1));
                            let b_idx = evaluated_args[idx + 1].as_f64().unwrap_or(0.0).max(0.0) as usize;
                            let b_w = evaluated_args[idx + 2].as_f64().unwrap_or(0.0).max(0.0);
                            let b_h = evaluated_args[idx + 3].as_f64().unwrap_or(0.0).max(0.0);
                            let b_off = evaluated_args[idx + 4].as_f64().unwrap_or(b_h);
                            inline_boxes.push(crate::compiler::text::InlineBoxSpan {
                                node_id,
                                index: b_idx,
                                width: b_w,
                                height: b_h,
                                baseline_offset: b_off,
                            });
                        } else {
                            let s_start = raw_first.max(0.0) as usize;
                            let s_end = evaluated_args[idx + 1].as_f64().unwrap_or(0.0).max(0.0) as usize;
                            let s_size = evaluated_args[idx + 2].as_f64().filter(|&v| v > 0.0);
                            let s_weight = evaluated_args[idx + 3].as_f64().filter(|&v| v > 0.0);
                            let s_family = get_family_from_val(evaluated_args.get(idx + 4), env).map(|s| s.to_string());
                            if let Some(ref fam) = s_family {
                                validate_font_family(Some(fam.as_str()), c.span)?;
                            }
                            let s_italic = evaluated_args[idx + 5].as_bool().unwrap_or(false);
                            let s_shift = evaluated_args[idx + 6].as_f64().unwrap_or(0.0);
                            spans.push(
                                crate::compiler::text::TextSpan::new(s_start..s_end).with_style(
                                    crate::compiler::text::SpanStyle {
                                        font_size: s_size,
                                        font_weight: s_weight,
                                        font_family: s_family,
                                        italic: s_italic,
                                        baseline_shift: s_shift,
                                        ..Default::default()
                                    },
                                ),
                            );
                        }
                        idx += 7;
                    }

                    let sm = crate::compiler::text::compute_span_range_metrics_with_boxes(
                        text,
                        size,
                        weight,
                        family,
                        max_width,
                        start_at,
                        line_height,
                        align,
                        &spans,
                        &inline_boxes,
                        text_x,
                        text_y,
                        span_start..span_end,
                        own_shift,
                        target_inline_box,
                    );
                    let val = match metric {
                        "x" => sm.rect.x,
                        "y" => sm.rect.y,
                        "width" => sm.rect.width,
                        "height" => sm.rect.height,
                        "baseline" => sm.baseline,
                        _ => 0.0,
                    };
                    Ok(Value::Distance(val))
                }
                "font_cap_height" => {
                    let size = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(16.0);
                    let weight = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(400.0);
                    let family = get_family_from_val(evaluated_args.get(2), env);
                    validate_font_family(family, c.span)?;

                    let m = crate::compiler::text::measure_font_metrics(size, weight, family);
                    Ok(Value::Distance(m.cap_height))
                }
                "font_x_height" => {
                    let size = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(16.0);
                    let weight = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(400.0);
                    let family = get_family_from_val(evaluated_args.get(2), env);
                    validate_font_family(family, c.span)?;

                    let m = crate::compiler::text::measure_font_metrics(size, weight, family);
                    Ok(Value::Distance(m.x_height))
                }
                "font_descent" => {
                    let size = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(16.0);
                    let weight = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(400.0);
                    let family = get_family_from_val(evaluated_args.get(2), env);
                    validate_font_family(family, c.span)?;

                    let m = crate::compiler::text::measure_font_metrics(size, weight, family);
                    Ok(Value::Distance(m.descent))
                }
                "font_ascent" => {
                    let size = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(16.0);
                    let weight = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(400.0);
                    let family = get_family_from_val(evaluated_args.get(2), env);
                    validate_font_family(family, c.span)?;

                    let m = crate::compiler::text::measure_font_metrics(size, weight, family);
                    Ok(Value::Distance(m.ascent))
                }
                "font_line_gap" => {
                    let size = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(16.0);
                    let weight = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(400.0);
                    let family = get_family_from_val(evaluated_args.get(2), env);
                    validate_font_family(family, c.span)?;

                    let m = crate::compiler::text::measure_font_metrics(size, weight, family);
                    Ok(Value::Distance(m.line_gap))
                }
                "font_line_height" => {
                    let size = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(16.0);
                    let weight = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(400.0);
                    let family = get_family_from_val(evaluated_args.get(2), env);
                    validate_font_family(family, c.span)?;

                    let m = crate::compiler::text::measure_font_metrics(size, weight, family);
                    Ok(Value::Distance(m.line_height))
                }
                other => Err(CompileError::Custom {
                    message: format!("Unknown math/collection function '{}'", other),
                    span: c.span,
                }),
            }
        }

        Expr::MethodCall(mc) => {
            let target_val = eval_expr_with_env(&mc.target, env)?;
            let evaluated_args: Vec<Value> = mc
                .args
                .iter()
                .map(|arg| eval_expr_with_env(arg, env))
                .collect::<Result<_, _>>()?;

            match mc.method.as_str() {
                "append" => {
                    let target_str = evaluated_args
                        .first()
                        .map(|v| v.to_display_string())
                        .unwrap_or_default();

                    let base_path = match target_val {
                        Value::Node(id) => env
                            .get_var(id, "path")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                        Value::String(s) => Some(s),
                        _ => None,
                    };

                    if let Some(path) = base_path {
                        let path_str = path.trim_start_matches('#');
                        let base = path_str.trim_matches('/');
                        let target = target_str.trim_matches('/');
                        let full_path = if base.is_empty() {
                            format!("#/{}", target)
                        } else {
                            format!("#/{}/{}", base, target)
                        };
                        Ok(Value::String(full_path))
                    } else {
                        let target = target_str.trim_matches('/');
                        Ok(Value::String(format!("#/{}", target)))
                    }
                }
                "to_fixed" => {
                    let decimals = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(0.0).max(0.0) as usize;
                    match target_val {
                        Value::Integer(n) => Ok(Value::String(format_fixed(n as f64, decimals))),
                        Value::Float(n) | Value::Distance(n) | Value::Area(n) => {
                            Ok(Value::String(format_fixed(n, decimals)))
                        }
                        Value::Duration(us) => Ok(Value::String(format_fixed(
                            us as f64 / 1_000_000.0,
                            decimals,
                        ))),
                        _ => Err(CompileError::Custom {
                            message: format!("to_fixed() can only be called on numeric values, got {}", target_val.type_name()),
                            span: mc.span,
                        }),
                    }
                }
                "to_precision" => {
                    let sigfigs = evaluated_args.first().and_then(|v| v.as_f64()).unwrap_or(1.0).max(1.0) as usize;
                    match target_val {
                        Value::Integer(n) => Ok(Value::String(format_sigfigs(n as f64, sigfigs))),
                        Value::Float(n) | Value::Distance(n) | Value::Area(n) => {
                            Ok(Value::String(format_sigfigs(n, sigfigs)))
                        }
                        Value::Duration(us) => Ok(Value::String(format_sigfigs(
                            us as f64 / 1_000_000.0,
                            sigfigs,
                        ))),
                        _ => Err(CompileError::Custom {
                            message: format!("to_precision() can only be called on numeric values, got {}", target_val.type_name()),
                            span: mc.span,
                        }),
                    }
                }
                "to_px" => match target_val {
                    Value::Distance(d) => Ok(Value::Float(d)),
                    Value::Integer(0) => Ok(Value::Float(0.0)),
                    other => Err(CompileError::Custom {
                        message: format!("to_px() can only be called on Distance, got {}", other.type_name()),
                        span: mc.span,
                    }),
                },
                "to_pt" => match target_val {
                    Value::Distance(d) => {
                        let dpi = env
                            .get_var(NodeId::WINDOW, "dpi")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(96.0);
                        Ok(Value::Float(d / (dpi / 72.0)))
                    }
                    Value::Integer(0) => Ok(Value::Float(0.0)),
                    other => Err(CompileError::Custom {
                        message: format!("to_pt() can only be called on Distance, got {}", other.type_name()),
                        span: mc.span,
                    }),
                },
                "to_seconds" => match target_val {
                    Value::Duration(d) => Ok(Value::Float(d as f64 / 1_000_000.0)),
                    Value::Integer(0) => Ok(Value::Float(0.0)),
                    other => Err(CompileError::Custom {
                        message: format!("to_seconds() can only be called on Duration, got {}", other.type_name()),
                        span: mc.span,
                    }),
                },
                "to_ms" => match target_val {
                    Value::Duration(d) => Ok(Value::Float(d as f64 / 1_000.0)),
                    Value::Integer(0) => Ok(Value::Float(0.0)),
                    other => Err(CompileError::Custom {
                        message: format!("to_ms() can only be called on Duration, got {}", other.type_name()),
                        span: mc.span,
                    }),
                },
                "to_float" | "to_number" => match target_val {
                    Value::Integer(i) => Ok(Value::Float(i as f64)),
                    Value::Float(f) => Ok(Value::Float(f)),
                    other => Err(CompileError::Custom {
                        message: format!(
                            "{}() can only be called on Integer or Float, got {}",
                            mc.method.as_str(),
                            other.type_name()
                        ),
                        span: mc.span,
                    }),
                },
                "to_integer" => match target_val {
                    Value::Integer(i) => Ok(Value::Integer(i)),
                    Value::Float(f) => Ok(Value::Integer(f as i64)),
                    other => Err(CompileError::Custom {
                        message: format!("to_integer() can only be called on Integer or Float, got {}", other.type_name()),
                        span: mc.span,
                    }),
                },
                other => Err(CompileError::Custom {
                    message: format!("Unknown method '{}' called on expression", other),
                    span: mc.span,
                }),
            }
        }

        Expr::Paren(inner, _) => eval_expr_with_env(inner, env),
        Expr::Node(n) => match n.name.as_str() {
            "Timer" => {
                if n.ports.is_empty() {
                    return Err(CompileError::MissingPort {
                        node: "Timer".to_string(),
                        port: "duration".to_string(),
                        span: n.span,
                    });
                }
                let mut duration_us: Option<i64> = None;
                for port in &n.ports {
                    if port.name.as_str() == "duration" {
                        let val = eval_expr_with_env(&port.expr, env)?;
                        let us = val.as_duration().ok_or_else(|| CompileError::TypeMismatch {
                            expected: "Duration".to_string(),
                            actual: val.type_name().to_string(),
                            span: port.expr.span(),
                        })?;
                        duration_us = Some(us);
                    } else {
                        return Err(CompileError::Custom {
                            message: format!(
                                "Node 'Timer' does not accept property '{}'",
                                port.name.as_str()
                            ),
                            span: port.name.span,
                        });
                    }
                }
                match duration_us {
                    Some(us) if us > 0 => Ok(Value::Timer {
                        time: 0,
                        duration: us,
                        dt: 0,
                        frames: 0,
                    }),
                    Some(_) => Ok(Value::Unset),
                    None => Err(CompileError::MissingPort {
                        node: "Timer".to_string(),
                        port: "duration".to_string(),
                        span: n.span,
                    }),
                }
            }
            "IndefiniteTimer" => {
                if let Some(port) = n.ports.first() {
                    return Err(CompileError::Custom {
                        message: format!(
                            "Node 'IndefiniteTimer' does not accept property '{}'",
                            port.name.as_str()
                        ),
                        span: port.name.span,
                    });
                }
                Ok(Value::IndefiniteTimer {
                    time: 0,
                    dt: 0,
                    frames: 0,
                })
            }
            _ => Err(CompileError::Custom {
                message: format!("Unexpanded node in expression: '{}'", n.name.as_str()),
                span: n.span,
            }),
        },
        Expr::ExprDef(def) => Err(CompileError::Custom {
            message: "Cannot evaluate uninvoked '\\Expr' as a value; call it with '(...)'"
                .to_string(),
            span: def.span,
        }),
        Expr::Match(m) => Err(CompileError::Custom {
            message: "Unlowered 'match' expression in evaluator".to_string(),
            span: m.span,
        }),
    }
}

fn eval_truthy(val: &Value, span: crate::span::Span) -> Result<bool, CompileError> {
    val.as_truthy().ok_or_else(|| CompileError::TypeMismatch {
        expected: "Boolean".to_string(),
        actual: val.type_name().to_string(),
        span,
    })
}

fn values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Integer(a), Value::Integer(b)) => a == b,
        (Value::Float(a), Value::Float(b)) => (a - b).abs() < 1e-9,
        (Value::Integer(a), Value::Float(b)) | (Value::Float(b), Value::Integer(a)) => {
            (*a as f64 - *b).abs() < 1e-9
        }
        (Value::Distance(a), Value::Distance(b)) => (a - b).abs() < 1e-9,
        (Value::Distance(a), Value::Integer(0)) | (Value::Integer(0), Value::Distance(a)) => {
            a.abs() < 1e-9
        }
        (Value::Area(a), Value::Area(b)) => (a - b).abs() < 1e-9,
        (Value::Area(a), Value::Integer(0)) | (Value::Integer(0), Value::Area(a)) => {
            a.abs() < 1e-9
        }
        (Value::Duration(a), Value::Duration(b)) => a == b,
        (Value::Duration(a), Value::Integer(0)) | (Value::Integer(0), Value::Duration(a)) => {
            *a == 0
        }
        _ => left == right,
    }
}

fn extract_comparable_pair(
    left: &Value,
    right: &Value,
    span: crate::span::Span,
) -> Result<(f64, f64), CompileError> {
    match (left, right) {
        (Value::Integer(a), Value::Integer(b)) => Ok((*a as f64, *b as f64)),
        (Value::Float(a), Value::Float(b)) => Ok((*a, *b)),
        (Value::Integer(a), Value::Float(b)) => Ok((*a as f64, *b)),
        (Value::Float(a), Value::Integer(b)) => Ok((*a, *b as f64)),
        (Value::Distance(a), Value::Distance(b)) => Ok((*a, *b)),
        (Value::Distance(a), Value::Integer(0)) => Ok((*a, 0.0)),
        (Value::Integer(0), Value::Distance(b)) => Ok((0.0, *b)),
        (Value::Area(a), Value::Area(b)) => Ok((*a, *b)),
        (Value::Area(a), Value::Integer(0)) => Ok((*a, 0.0)),
        (Value::Integer(0), Value::Area(b)) => Ok((0.0, *b)),
        (Value::Duration(a), Value::Duration(b)) => Ok((*a as f64, *b as f64)),
        (Value::Duration(a), Value::Integer(0)) => Ok((*a as f64, 0.0)),
        (Value::Integer(0), Value::Duration(b)) => Ok((0.0, *b as f64)),
        _ => Err(CompileError::Custom {
            message: format!(
                "Cannot compare {} ({}) and {} ({})",
                left,
                left.type_name(),
                right,
                right.type_name()
            ),
            span,
        }),
    }
}

fn eval_aggregate_fn<F>(
    name: &str,
    args: &[Value],
    span: crate::span::Span,
    f: F,
) -> Result<Value, CompileError>
where
    F: FnOnce(&[f64]) -> f64,
{
    if args.is_empty() {
        return Ok(Value::Integer(0));
    }
    let has_distance = args.iter().any(|v| matches!(v, Value::Distance(_)));
    let has_area = args.iter().any(|v| matches!(v, Value::Area(_)));
    let has_duration = args.iter().any(|v| matches!(v, Value::Duration(_)));
    let has_float = args.iter().any(|v| matches!(v, Value::Float(_)));

    if has_distance {
        let mut nums = Vec::with_capacity(args.len());
        for arg in args {
            match arg {
                Value::Distance(d) => nums.push(*d),
                Value::Integer(0) => nums.push(0.0),
                other => {
                    return Err(CompileError::Custom {
                        message: format!(
                            "{}() cannot mix Distance with {} ({})",
                            name,
                            other,
                            other.type_name()
                        ),
                        span,
                    });
                }
            }
        }
        Ok(Value::Distance(f(&nums)))
    } else if has_area {
        let mut nums = Vec::with_capacity(args.len());
        for arg in args {
            match arg {
                Value::Area(a) => nums.push(*a),
                Value::Integer(0) => nums.push(0.0),
                other => {
                    return Err(CompileError::Custom {
                        message: format!(
                            "{}() cannot mix Area with {} ({})",
                            name,
                            other,
                            other.type_name()
                        ),
                        span,
                    });
                }
            }
        }
        Ok(Value::Area(f(&nums)))
    } else if has_duration {
        let mut nums = Vec::with_capacity(args.len());
        for arg in args {
            match arg {
                Value::Duration(d) => nums.push(*d as f64),
                Value::Integer(0) => nums.push(0.0),
                other => {
                    return Err(CompileError::Custom {
                        message: format!(
                            "{}() cannot mix Duration with {} ({})",
                            name,
                            other,
                            other.type_name()
                        ),
                        span,
                    });
                }
            }
        }
        Ok(Value::Duration(f(&nums).round() as i64))
    } else if has_float {
        let mut nums = Vec::with_capacity(args.len());
        for arg in args {
            match arg {
                Value::Float(n) => nums.push(*n),
                Value::Integer(i) => nums.push(*i as f64),
                other => {
                    return Err(CompileError::Custom {
                        message: format!(
                            "{}() expected numeric arguments, found {}",
                            name,
                            other.type_name()
                        ),
                        span,
                    });
                }
            }
        }
        Ok(Value::Float(f(&nums)))
    } else {
        let mut nums = Vec::with_capacity(args.len());
        for arg in args {
            match arg {
                Value::Integer(i) => nums.push(*i as f64),
                other => {
                    return Err(CompileError::Custom {
                        message: format!(
                            "{}() expected numeric arguments, found {}",
                            name,
                            other.type_name()
                        ),
                        span,
                    });
                }
            }
        }
        Ok(Value::Integer(f(&nums) as i64))
    }
}

pub(crate) fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let h = ((h % 360.0) + 360.0) % 360.0;
    let s = s.clamp(0.0, 1.0);
    let l = l.clamp(0.0, 1.0);

    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (((h / 60.0) % 2.0) - 1.0).abs());
    let m = l - c / 2.0;

    let (r_prime, g_prime, b_prime) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    let r = ((r_prime + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    let g = ((g_prime + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    let b = ((b_prime + m) * 255.0).round().clamp(0.0, 255.0) as u8;

    (r, g, b)
}

pub(crate) fn format_fixed(val: f64, decimals: usize) -> String {
    if !val.is_finite() {
        return format!("{val}");
    }
    let rounded = if decimals == 0 {
        let r = val.round();
        if r == 0.0 { 0.0 } else { r }
    } else {
        val
    };
    let s = format!("{:.prec$}", rounded, prec = decimals);
    if s == format!("-{:.prec$}", 0.0, prec = decimals) {
        format!("{:.prec$}", 0.0, prec = decimals)
    } else {
        s
    }
}

pub(crate) fn format_sigfigs(val: f64, sigfigs: usize) -> String {
    if !val.is_finite() {
        return format!("{val}");
    }
    if sigfigs == 0 {
        return Value::Float(val).to_display_string();
    }
    if val == 0.0 {
        if sigfigs == 1 {
            return "0".to_string();
        } else {
            return format!("0.{:0<prec$}", "", prec = sigfigs - 1);
        }
    }

    let abs_val = val.abs();
    let magnitude = abs_val.log10().floor() as i32;
    let decimals = sigfigs as i32 - 1 - magnitude;

    let rounded = if decimals >= 0 {
        let factor = 10_f64.powi(decimals);
        (val * factor).round() / factor
    } else {
        let factor = 10_f64.powi(-decimals);
        (val / factor).round() * factor
    };

    let new_magnitude = if rounded == 0.0 { 0 } else { rounded.abs().log10().floor() as i32 };
    let final_decimals = (sigfigs as i32 - 1 - new_magnitude).max(0) as usize;

    let s = if final_decimals > 0 {
        format!("{:.prec$}", rounded, prec = final_decimals)
    } else {
        format!("{:.0}", rounded)
    };

    if s.starts_with("-0") && s.trim_start_matches('-').parse::<f64>().map(|v| v == 0.0).unwrap_or(false) {
        s.trim_start_matches('-').to_string()
    } else {
        s
    }
}


