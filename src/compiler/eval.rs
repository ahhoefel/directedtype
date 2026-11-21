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

    fn image_assets(&self) -> Option<&crate::compiler::image::ImageAssetStore> {
        None
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
    self_node: NodeId,
}

impl<'a> EvalEnv for IndexedEvalEnv<'a> {
    #[inline]
    fn get_var_sym(&self, node: NodeId, port: Symbol) -> Option<&Value> {
        let resolved_node = if node.is_template_self() {
            self.self_node
        } else {
            node
        };
        let idx = self.graph.lookup_var_index_sym(resolved_node, port)?;
        self.slots.get(idx.0 as usize)?.as_ref()
    }

    #[inline]
    fn image_assets(&self) -> Option<&crate::compiler::image::ImageAssetStore> {
        Some(&self.graph.image_assets)
    }
}

struct NodeMapEnv<'a> {
    map: &'a HashMap<VarId, Value>,
    self_node: NodeId,
}

impl<'a> EvalEnv for NodeMapEnv<'a> {
    #[inline]
    fn get_var_sym(&self, node: NodeId, port: Symbol) -> Option<&Value> {
        let resolved_node = if node.is_template_self() {
            self.self_node
        } else {
            node
        };
        self.map.get(&VarId::from_symbol(resolved_node, port))
    }
}

#[inline]
fn window_dpi(env: &impl EvalEnv) -> f64 {
    env.get_var_sym(NodeId::WINDOW, Symbol::DPI)
        .and_then(|v| v.as_f64())
        .unwrap_or(96.0)
}

fn convert_distance_to_unit(
    val: &Value,
    target_unit: crate::ast::DistanceUnit,
    env: &impl EvalEnv,
    fn_name: &str,
    err_verb: &str,
    span: crate::span::Span,
) -> Result<Value, CompileError> {
    let d_px = match val {
        Value::Distance(d) => *d,
        Value::Integer(0) => return Ok(Value::Float(0.0)),
        other => {
            return Err(CompileError::Custom {
                message: format!("{fn_name}() {err_verb} Distance, got {}", other.type_name()),
                span,
            });
        }
    };
    if let Some(pts_per_unit) = target_unit.points_per_unit() {
        Ok(Value::Float(d_px / (pts_per_unit * (window_dpi(env) / 72.0))))
    } else {
        match target_unit {
            crate::ast::DistanceUnit::Px => Ok(Value::Float(d_px)),
            crate::ast::DistanceUnit::Em => Err(CompileError::UndefinedEnvVariable {
                name: "font".to_string(),
                span,
            }),
            _ => unreachable!("all physical units handled by points_per_unit()"),
        }
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
            let val = eval_expr_with_env(
                &node.equation,
                &IndexedEvalEnv {
                    graph,
                    slots: &slots,
                    self_node: node.id.node,
                },
            )?;
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
            let val = eval_expr_for_node(&node.equation, node.id.node, &env)?;
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
                eval_expr_for_node(&node.equation, node.id.node, current_values)?
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
                eval_expr_for_node(&node.equation, node.id.node, current_values)?
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

/// Decodes the 11-element paragraph layout options slice matching `build_paragraph_layout_option_args`.
fn decode_paragraph_layout_options(
    slice: &[Value],
) -> crate::compiler::text::ParagraphLayoutOptions {
    let mut opts = crate::compiler::text::ParagraphLayoutOptions::default();
    if let Some(lb) = slice.first().and_then(|v| v.as_str()) {
        opts.line_break = crate::compiler::text::LineBreakMode::from_str_opt(Some(lb));
    }
    if let Some(ws) = slice.get(1).and_then(|v| v.as_f64()) {
        opts.word_spacing = ws;
    }
    if let Some(ss) = slice.get(2).and_then(|v| v.as_f64()) {
        opts.space_stretch = ss.max(0.0);
    }
    if let Some(sk) = slice.get(3).and_then(|v| v.as_f64()) {
        opts.space_shrink = sk.clamp(0.0, 0.99);
    }
    if let Some(tol) = slice.get(4).and_then(|v| v.as_f64()) {
        opts.tolerance = tol.max(0.1);
    }
    if let Some(hp) = slice.get(5).and_then(|v| v.as_f64()) {
        opts.hyphen_penalty = hp;
    }
    if let Some(lp) = slice.get(6).and_then(|v| v.as_f64()) {
        opts.line_penalty = lp.max(0.0);
    }
    if let Some(fd) = slice.get(7).and_then(|v| v.as_f64()) {
        opts.flagged_demerits = fd.max(0.0);
    }
    if let Some(ad) = slice.get(8).and_then(|v| v.as_f64()) {
        opts.fitness_demerits = ad.max(0.0);
    }
    if let Some(sent) = slice.get(9).and_then(|v| v.as_f64()) {
        opts.sentence_space = sent;
    }
    if let Some(l) = slice.get(10).and_then(|v| v.as_f64()) {
        opts.looseness = l.round() as i32;
    }
    opts
}

/// Decodes flat 8-element span/inline-box style chunks appended to `text_height`, `text_width`, and `span_metric`.
fn decode_rich_text_style_chunks<E: EvalEnv>(
    chunks: &[Value],
    env: &E,
    span: crate::span::Span,
) -> Result<(Vec<crate::compiler::text::TextSpan>, Vec<crate::compiler::text::InlineBoxSpan>), CompileError> {
    let mut spans = Vec::new();
    let mut inline_boxes = Vec::new();
    for chunk in chunks.chunks_exact(8) {
        let raw_first = chunk[0].as_f64().unwrap_or(0.0);
        if raw_first < 0.0 {
            let node_id = NodeId((-raw_first.round() as usize).saturating_sub(1));
            let b_idx = chunk[1].as_f64().unwrap_or(0.0).max(0.0) as usize;
            let b_w = chunk[2].as_f64().unwrap_or(0.0).max(0.0);
            let b_h = chunk[3].as_f64().unwrap_or(0.0).max(0.0);
            let b_off = chunk[4].as_f64().unwrap_or(b_h);
            inline_boxes.push(crate::compiler::text::InlineBoxSpan {
                node_id,
                index: b_idx,
                width: b_w,
                height: b_h,
                baseline_offset: b_off,
            });
        } else {
            let s_start = raw_first.max(0.0) as usize;
            let s_end = chunk[1].as_f64().unwrap_or(0.0).max(0.0) as usize;
            let s_size = chunk[2].as_f64().filter(|&v| v > 0.0);
            let s_weight = chunk[3].as_f64().filter(|&v| v > 0.0);
            let s_family = get_family_from_val(Some(&chunk[4]), env).map(|s| s.to_string());
            if let Some(ref fam) = s_family {
                validate_font_family(Some(fam.as_str()), span)?;
            }
            let s_italic = chunk[5].as_bool().unwrap_or(false);
            let s_small_caps = chunk[6].as_bool();
            let s_shift = chunk[7].as_f64().unwrap_or(0.0);
            spans.push(
                crate::compiler::text::TextSpan::new(s_start..s_end).with_style(
                    crate::compiler::text::SpanStyle {
                        font_size: s_size,
                        font_weight: s_weight,
                        font_family: s_family,
                        italic: s_italic,
                        small_caps: s_small_caps,
                        baseline_shift: s_shift,
                        ..Default::default()
                    },
                ),
            );
        }
    }
    Ok((spans, inline_boxes))
}

/// Evaluates an algebraic expression in the given environment.
pub fn eval_expr(expr: &Expr, env: &HashMap<VarId, Value>) -> Result<Value, CompileError> {
    eval_expr_with_env(expr, env)
}

/// Evaluates an algebraic expression owned by `self_node`, resolving `NodeId::TEMPLATE_SELF` to `self_node`.
pub fn eval_expr_for_node(
    expr: &Expr,
    self_node: NodeId,
    env: &HashMap<VarId, Value>,
) -> Result<Value, CompileError> {
    eval_expr_with_env(expr, &NodeMapEnv { map: env, self_node })
}

/// Evaluates an algebraic expression against any `EvalEnv` (flat slot table or `HashMap`).
pub fn eval_expr_with_env<E: EvalEnv>(expr: &Expr, env: &E) -> Result<Value, CompileError> {
    match expr {
        Expr::Literal(lit) => match lit {
            Literal::Integer(n, _) => Ok(Value::Integer(*n)),
            Literal::Float(n, _) => Ok(Value::Float(*n)),
            Literal::Distance(d, unit, span) => {
                if let Some(pts_per_unit) = unit.points_per_unit() {
                    let dpi = window_dpi(env);
                    Ok(Value::Distance(*d * pts_per_unit * (dpi / 72.0)))
                } else {
                    match unit {
                        crate::ast::DistanceUnit::Px => Ok(Value::Distance(*d)),
                        crate::ast::DistanceUnit::Em => Err(CompileError::UndefinedEnvVariable {
                            name: "font".to_string(),
                            span: *span,
                        }),
                        _ => unreachable!("all physical units handled by points_per_unit()"),
                    }
                }
            }
            Literal::Area(a, unit, span) => {
                if let Some(pts_per_unit) = unit.points_per_unit() {
                    let dpi = window_dpi(env);
                    let factor = (dpi / 72.0) * pts_per_unit;
                    Ok(Value::Area(*a * factor * factor))
                } else {
                    match unit {
                        crate::ast::DistanceUnit::Px => Ok(Value::Area(*a)),
                        crate::ast::DistanceUnit::Em => Err(CompileError::UndefinedEnvVariable {
                            name: "font".to_string(),
                            span: *span,
                        }),
                        _ => unreachable!("all physical units handled by points_per_unit()"),
                    }
                }
            }
            Literal::Duration(d, _, _) => Ok(Value::Duration(*d)),
            Literal::String(s, _) => Ok(Value::String(s.clone())),
            Literal::Path(p, _) => Ok(Value::Path(p.clone())),
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
            Symbol::PAGE => Err(CompileError::PageOutsidePagedFlow { span: id.span }),
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
        Expr::DomNodeRef(h, span) => Err(CompileError::Custom {
            message: format!("Unlowered DOM node handle reference #{}", h.to_u64()),
            span: *span,
        }),

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

            let mut target_unwrap = m.target.as_ref();
            while let Expr::Paren(inner, _) = target_unwrap {
                target_unwrap = inner.as_ref();
            }
            if let Expr::Index(idx) = target_unwrap {
                let bucket_val = eval_expr_with_env(&idx.target, env)?;
                let bucket_id = match bucket_val {
                    Value::Node(id) => id,
                    other => {
                        return Err(CompileError::Custom {
                            message: format!(
                                "Cannot index non-bucket value of type '{}'",
                                other.type_name()
                            ),
                            span: idx.target.span(),
                        });
                    }
                };
                let idx_val = eval_expr_with_env(&idx.index, env)?;
                let page_num: Option<i64> = match idx_val {
                    Value::Integer(n) => Some(n),
                    Value::Unset => None,
                    Value::Node(nid) => {
                        if nid.is_page() {
                            env.get_var_sym(NodeId::PAGE, Symbol::NUMBER)
                                .and_then(|v| v.as_i64())
                        } else {
                            env.get_var(nid, "__page_number").and_then(|v| v.as_i64())
                        }
                    }
                    other => {
                        return Err(CompileError::TypeMismatch {
                            expected: "Integer or Page".to_string(),
                            actual: other.type_name().to_string(),
                            span: idx.index.span(),
                        });
                    }
                };
                let member_name = m.member.as_str();
                return match page_num {
                    Some(p) => {
                        let synth_port = format!("{member_name}_p_{p}");
                        if let Some(val) = env.get_var(bucket_id, &synth_port) {
                            Ok(val.clone())
                        } else {
                            match member_name {
                                "has_value" => Ok(Value::Bool(false)),
                                "len" => Ok(Value::Integer(0)),
                                "value" | "first" | "last" => Ok(Value::Unset),
                                other => Err(CompileError::Custom {
                                    message: format!("Bucket slice has no property '{other}'"),
                                    span: m.span,
                                }),
                            }
                        }
                    }
                    None => match member_name {
                        "has_value" => Ok(Value::Bool(false)),
                        "len" => Ok(Value::Integer(0)),
                        "value" | "first" | "last" => Ok(Value::Unset),
                        other => Err(CompileError::Custom {
                            message: format!("Bucket slice has no property '{other}'"),
                            span: m.span,
                        }),
                    },
                };
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
            if b.op == BinaryOp::And {
                let left = eval_expr_with_env(&b.left, env)?;
                let l = eval_truthy(&left, b.left.span())?;
                if !l {
                    return Ok(Value::Bool(false));
                }
                let right = eval_expr_with_env(&b.right, env)?;
                let r = eval_truthy(&right, b.right.span())?;
                return Ok(Value::Bool(r));
            }
            if b.op == BinaryOp::Or {
                let left = eval_expr_with_env(&b.left, env)?;
                let l = eval_truthy(&left, b.left.span())?;
                if l {
                    return Ok(Value::Bool(true));
                }
                let right = eval_expr_with_env(&b.right, env)?;
                let r = eval_truthy(&right, b.right.span())?;
                return Ok(Value::Bool(r));
            }
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
                    Expr::Ident(unit_id) => match crate::ast::DistanceUnit::from_str(unit_id.as_str()) {
                        Some(unit) if unit.points_per_unit().is_some() => {
                            unit.points_per_unit().unwrap() * (window_dpi(env) / 72.0)
                        }
                        Some(crate::ast::DistanceUnit::Px) => 1.0,
                        Some(crate::ast::DistanceUnit::Em) => {
                            return Err(CompileError::UndefinedEnvVariable {
                                name: "font".to_string(),
                                span: unit_id.span,
                            });
                        }
                        _ => {
                            return Err(CompileError::Custom {
                                message: format!(
                                    "Unknown unit '{}' in distance_to_float(). Supported units are 'px', 'pt', 'em', 'in', 'pc', 'mm', and 'cm'",
                                    unit_id.as_str()
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
                name if crate::ast::DistanceUnit::from_conversion_fn(name).is_some() => {
                    let target_unit = crate::ast::DistanceUnit::from_conversion_fn(name).unwrap();
                    let Some(arg) = evaluated_args.first() else {
                        return Err(CompileError::Custom {
                            message: format!("{name}() expects 1 argument"),
                            span: c.span,
                        });
                    };
                    convert_distance_to_unit(arg, target_unit, env, name, "expects a", c.span)
                }
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

                    let has_full_opts = evaluated_args.len() >= 24
                        && (evaluated_args.len() - 24) % 8 == 0
                        && evaluated_args.get(10).and_then(|v| v.as_str()).is_some();
                    let has_small_caps = !has_full_opts
                        && evaluated_args.len() >= 10
                        && (evaluated_args.len() - 10) % 8 == 0;

                    let (indent, font_small_caps, align, letter_spacing, font_italic, para_opts, spans, inline_boxes) =
                        if has_full_opts {
                            let indent = evaluated_args[8].as_f64().unwrap_or(0.0);
                            let font_small_caps = evaluated_args[9].as_bool().unwrap_or(false);
                            let align = evaluated_args[10].as_str();
                            let letter_spacing = evaluated_args[11].as_f64().unwrap_or(0.0);
                            let font_italic = evaluated_args[12].as_bool().unwrap_or(false);
                            let para_opts = decode_paragraph_layout_options(&evaluated_args[13..24]);
                            let (spans, inline_boxes) =
                                decode_rich_text_style_chunks(&evaluated_args[24..], env, c.span)?;
                            (indent, font_small_caps, align, letter_spacing, font_italic, para_opts, spans, inline_boxes)
                        } else if has_small_caps {
                            let indent = evaluated_args[8].as_f64().unwrap_or(0.0);
                            let font_small_caps = evaluated_args[9].as_bool().unwrap_or(false);
                            let (spans, inline_boxes) =
                                decode_rich_text_style_chunks(&evaluated_args[10..], env, c.span)?;
                            (indent, font_small_caps, None, 0.0, false, crate::compiler::text::ParagraphLayoutOptions::default(), spans, inline_boxes)
                        } else {
                            let indent = evaluated_args.get(8).and_then(|v| v.as_f64()).unwrap_or(0.0);
                            (indent, false, None, 0.0, false, crate::compiler::text::ParagraphLayoutOptions::default(), Vec::new(), Vec::new())
                        };

                    let (_, h) = crate::compiler::text::measure_rich_text_bounds_full(
                        text,
                        size,
                        weight,
                        family,
                        font_italic,
                        letter_spacing,
                        Some(max_width),
                        end_at,
                        start_at,
                        line_height,
                        align,
                        indent,
                        &spans,
                        &inline_boxes,
                        font_small_caps,
                        &para_opts,
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
                "__image_intrinsic_width" => {
                    let path = evaluated_args
                        .first()
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| CompileError::Custom {
                            message: "__image_intrinsic_width() expects a Path".to_string(),
                            span: c.span,
                        })?;
                    let dpi = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or_else(|| window_dpi(env));
                    let win_dpi = evaluated_args.get(2).and_then(|v| v.as_f64()).unwrap_or_else(|| window_dpi(env));
                    if dpi <= 0.0 {
                        return Err(CompileError::Custom {
                            message: format!("Image 'dpi' must be positive, got {}", dpi),
                            span: c.span,
                        });
                    }
                    let asset = crate::compiler::image::lookup_loaded_image(path, env.image_assets())
                        .map_err(|msg| CompileError::Custom {
                            message: msg,
                            span: c.span,
                        })?;
                    Ok(Value::Distance(asset.metadata.intrinsic_width(dpi, win_dpi)))
                }
                "__image_intrinsic_height" => {
                    let path = evaluated_args
                        .first()
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| CompileError::Custom {
                            message: "__image_intrinsic_height() expects a Path".to_string(),
                            span: c.span,
                        })?;
                    let dpi = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or_else(|| window_dpi(env));
                    let win_dpi = evaluated_args.get(2).and_then(|v| v.as_f64()).unwrap_or_else(|| window_dpi(env));
                    if dpi <= 0.0 {
                        return Err(CompileError::Custom {
                            message: format!("Image 'dpi' must be positive, got {}", dpi),
                            span: c.span,
                        });
                    }
                    let asset = crate::compiler::image::lookup_loaded_image(path, env.image_assets())
                        .map_err(|msg| CompileError::Custom {
                            message: msg,
                            span: c.span,
                        })?;
                    Ok(Value::Distance(asset.metadata.intrinsic_height(dpi, win_dpi)))
                }
                "__image_aspect_ratio" => {
                    let path = evaluated_args
                        .first()
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| CompileError::Custom {
                            message: "__image_aspect_ratio() expects a Path".to_string(),
                            span: c.span,
                        })?;
                    let asset = crate::compiler::image::lookup_loaded_image(path, env.image_assets())
                        .map_err(|msg| CompileError::Custom {
                            message: msg,
                            span: c.span,
                        })?;
                    Ok(Value::Float(asset.metadata.aspect_ratio))
                }
                "__image_file_dpi" => {
                    let path = evaluated_args
                        .first()
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| CompileError::Custom {
                            message: "__image_file_dpi() expects a Path".to_string(),
                            span: c.span,
                        })?;
                    let win_dpi = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or_else(|| window_dpi(env));
                    let asset = crate::compiler::image::lookup_loaded_image(path, env.image_assets())
                        .map_err(|msg| CompileError::Custom {
                            message: msg,
                            span: c.span,
                        })?;
                    Ok(Value::Float(asset.metadata.resolved_file_dpi(win_dpi)))
                }
                "text_width" => {
                    let text = evaluated_args.first().and_then(|v| v.as_str()).unwrap_or("");
                    let size = evaluated_args.get(1).and_then(|v| v.as_f64()).unwrap_or(16.0);
                    let weight = evaluated_args.get(2).and_then(|v| v.as_f64()).unwrap_or(400.0);
                    let family = get_family_from_val(evaluated_args.get(3), env);
                    validate_font_family(family, c.span)?;

                    let has_word_spacing =
                        evaluated_args.len() >= 8 && (evaluated_args.len() - 8) % 8 == 0;
                    let has_small_caps = !has_word_spacing
                        && evaluated_args.len() >= 7
                        && (evaluated_args.len() - 7) % 8 == 0;
                    let (letter_spacing, indent, font_small_caps, word_spacing, spans, inline_boxes) =
                        if has_word_spacing {
                            let ls = evaluated_args[4].as_f64().unwrap_or(0.0);
                            let ind = evaluated_args[5].as_f64().unwrap_or(0.0);
                            let sc = evaluated_args[6].as_bool().unwrap_or(false);
                            let ws = evaluated_args[7].as_f64().unwrap_or(0.0);
                            let (spans, inline_boxes) =
                                decode_rich_text_style_chunks(&evaluated_args[8..], env, c.span)?;
                            (ls, ind, sc, ws, spans, inline_boxes)
                        } else if has_small_caps {
                            let ls = evaluated_args[4].as_f64().unwrap_or(0.0);
                            let ind = evaluated_args[5].as_f64().unwrap_or(0.0);
                            let sc = evaluated_args[6].as_bool().unwrap_or(false);
                            let (spans, inline_boxes) =
                                decode_rich_text_style_chunks(&evaluated_args[7..], env, c.span)?;
                            (ls, ind, sc, 0.0, spans, inline_boxes)
                        } else {
                            let ls = evaluated_args.get(4).and_then(|v| v.as_f64()).unwrap_or(0.0);
                            let ind = evaluated_args.get(5).and_then(|v| v.as_f64()).unwrap_or(0.0);
                            (ls, ind, false, 0.0, Vec::new(), Vec::new())
                        };

                    let para_opts = crate::compiler::text::ParagraphLayoutOptions {
                        word_spacing,
                        ..Default::default()
                    };
                    let (w, _) = crate::compiler::text::measure_rich_text_bounds_full(
                        text,
                        size,
                        weight,
                        family,
                        false,
                        letter_spacing,
                        None,
                        None,
                        None,
                        None,
                        None,
                        indent,
                        &spans,
                        &inline_boxes,
                        font_small_caps,
                        &para_opts,
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

                    let has_full_opts = evaluated_args.len() >= 29
                        && (evaluated_args.len() - 29) % 8 == 0
                        && evaluated_args.get(18).and_then(|v| v.as_str()).is_some();
                    let has_small_caps = !has_full_opts
                        && evaluated_args.len() >= 16
                        && (evaluated_args.len() - 16) % 8 == 0;
                    let (indent, font_small_caps, letter_spacing, font_italic, para_opts, spans, inline_boxes) =
                        if has_full_opts {
                            let ind = evaluated_args[14].as_f64().unwrap_or(0.0);
                            let sc = evaluated_args[15].as_bool().unwrap_or(false);
                            let ls = evaluated_args[16].as_f64().unwrap_or(0.0);
                            let it = evaluated_args[17].as_bool().unwrap_or(false);
                            let opts = decode_paragraph_layout_options(&evaluated_args[18..29]);
                            let (spans, inline_boxes) =
                                decode_rich_text_style_chunks(&evaluated_args[29..], env, c.span)?;
                            (ind, sc, ls, it, opts, spans, inline_boxes)
                        } else if has_small_caps {
                            let ind = evaluated_args[14].as_f64().unwrap_or(0.0);
                            let sc = evaluated_args[15].as_bool().unwrap_or(false);
                            let (spans, inline_boxes) =
                                decode_rich_text_style_chunks(&evaluated_args[16..], env, c.span)?;
                            (ind, sc, 0.0, false, crate::compiler::text::ParagraphLayoutOptions::default(), spans, inline_boxes)
                        } else {
                            let ind = evaluated_args
                                .get(14)
                                .and_then(|v| v.as_f64())
                                .unwrap_or(0.0);
                            (ind, false, 0.0, false, crate::compiler::text::ParagraphLayoutOptions::default(), Vec::new(), Vec::new())
                        };

                    let sm = crate::compiler::text::compute_span_range_metrics_with_options(
                        text,
                        size,
                        weight,
                        family,
                        font_italic,
                        letter_spacing,
                        max_width,
                        start_at,
                        line_height,
                        align,
                        indent,
                        font_small_caps,
                        &spans,
                        &inline_boxes,
                        text_x,
                        text_y,
                        span_start..span_end,
                        own_shift,
                        target_inline_box,
                        &para_opts,
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
                name if crate::ast::DistanceUnit::from_conversion_fn(name).is_some() => {
                    let target_unit = crate::ast::DistanceUnit::from_conversion_fn(name).unwrap();
                    convert_distance_to_unit(
                        &target_val,
                        target_unit,
                        env,
                        name,
                        "can only be called on",
                        mc.span,
                    )
                }
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
        Expr::Index(idx) => Err(CompileError::Custom {
            message: "Bucket slice expression must be indexed with '.has_value', '.len', '.value', '.first', '.last', or used inside \\Collect(from: ...)".to_string(),
            span: idx.span,
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


