use crate::ast::{ComponentKey, Expr};
use crate::compiler::expanded::NodeId;
use crate::compiler::graph::VarId;
use crate::compiler::layout::{Rect, ResolvedLayout};
use crate::compiler::value::Value;
use crate::dom::{Dom, NodeHandle};

/// Complete inspection data extracted for a target node in the Component DOM.
#[derive(Debug, Clone, PartialEq)]
pub struct InspectTargetInfo {
    pub handle: Option<NodeHandle>,
    pub node_id: Option<NodeId>,
    pub tag: String,
    pub id_name: Option<String>,
    pub key: Option<ComponentKey>,
    pub rect: Rect,
    pub z: f64,
    pub clip_handle: Option<NodeHandle>,
    pub clip_rect: Option<Rect>,
    pub declared_ports: Vec<(String, Expr)>,
    pub computed_values: Vec<(String, Value)>,
    pub state_vars: Vec<(String, Option<String>, Option<Value>)>,
}

impl InspectTargetInfo {
    /// Extracts target info from a stateful `Dom` by `NodeHandle`.
    pub fn from_dom(dom: &Dom, handle: NodeHandle) -> Option<Self> {
        let rect = dom.computed_rect(handle)?;
        let tag = dom.tag_name(handle).unwrap_or("Node").to_string();
        let declared = dom.declared_ports(handle).ok().cloned().unwrap_or_default();

        let id_name = declared.get("id").and_then(|expr| match expr {
            Expr::Ident(id) => Some(id.as_str().to_string()),
            Expr::Literal(crate::ast::Literal::String(s, _)) => Some(s.clone()),
            _ => None,
        });

        let key = dom.node_key(handle).cloned();

        let mut declared_ports: Vec<(String, Expr)> = declared.into_iter().collect();
        declared_ports.sort_by(|a, b| a.0.cmp(&b.0));

        let node_id = dom.node_handle_to_id(handle);
        let mut computed_values = Vec::new();
        let mut state_vars = Vec::new();
        let mut z = 0.0;
        let mut clip_handle = None;
        let mut clip_rect = None;

        if let Some(nid) = node_id {
            if let Some(layout) = dom.layout() {
                if let Some(resolved) = layout.get_node(nid) {
                    z = resolved.z;
                    for (k, v) in &resolved.properties {
                        computed_values.push((k.clone(), v.clone()));
                    }
                    computed_values.sort_by(|a, b| a.0.cmp(&b.0));

                    let mut sorted_states: Vec<_> = resolved.state_vars.keys().collect();
                    sorted_states.sort();
                    for &s_name in &sorted_states {
                        let type_annot = resolved.state_vars.get(s_name).cloned().flatten();
                        let val = dom
                            .get_state(handle, s_name.as_str())
                            .cloned()
                            .or_else(|| layout.get_value(nid, s_name.as_str()).cloned());
                        state_vars.push((s_name.clone(), type_annot, val));
                    }

                    if let Some(clip_id) = resolved.clip {
                        clip_handle = dom.node_id_to_handle(clip_id);
                        clip_rect = layout.get_node(clip_id).map(|n| n.rect);
                    }
                }
            }
        }

        Some(Self {
            handle: Some(handle),
            node_id,
            tag,
            id_name,
            key,
            rect,
            z,
            clip_handle,
            clip_rect,
            declared_ports,
            computed_values,
            state_vars,
        })
    }

    /// Extracts target info from a `ResolvedLayout` by internal `NodeId`.
    pub fn from_layout(layout: &ResolvedLayout, id: NodeId) -> Option<Self> {
        let node = layout.get_node(id)?;
        let mut computed_values: Vec<(String, Value)> = node
            .properties
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        computed_values.sort_by(|a, b| a.0.cmp(&b.0));

        let id_name = node.properties.get("id").and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            _ => None,
        });

        let key = node.key.clone();

        let mut state_vars = Vec::new();
        let mut sorted_states: Vec<_> = node.state_vars.keys().collect();
        sorted_states.sort();
        for &s_name in &sorted_states {
            let type_annot = node.state_vars.get(s_name).cloned().flatten();
            let val = layout.get_value(id, s_name.as_str()).cloned();
            state_vars.push((s_name.clone(), type_annot, val));
        }

        let clip_rect = node.clip.and_then(|cid| layout.get_node(cid)).map(|n| n.rect);

        Some(Self {
            handle: node.handle,
            node_id: Some(id),
            tag: node.name.clone(),
            id_name,
            key,
            rect: node.rect,
            z: node.z,
            clip_handle: None,
            clip_rect,
            declared_ports: Vec::new(),
            computed_values,
            state_vars,
        })
    }

    /// Formats the short header badge label, e.g. `"Rect#card [380 × 180]"` or `"Cell#(0, 0) [80 × 32]"`.
    pub fn badge_label(&self) -> String {
        let mut s = self.tag.clone();
        if let Some(key) = &self.key {
            s.push('#');
            s.push_str(&key.format_key());
        } else if let Some(id) = &self.id_name {
            s.push('#');
            s.push_str(id);
        }
        s.push_str(&format!(" [{} × {}]", self.rect.width.round() as i64, self.rect.height.round() as i64));
        s
    }

    /// Computes the bounding rectangle for the floating badge pill above (or below) the element.
    pub fn badge_rect(&self, badge_width: f64, badge_height: f64, window_width: f64, window_height: f64) -> Rect {
        let padding = 4.0;
        let mut x = self.rect.x;
        // Clamp horizontal position so badge doesn't clip off window edges
        if x + badge_width > window_width - padding {
            x = (window_width - badge_width - padding).max(padding);
        }
        if x < padding {
            x = padding;
        }

        // Place above target if room; otherwise flip below
        let y = if self.rect.y >= badge_height + padding + 2.0 {
            self.rect.y - badge_height - 4.0
        } else {
            (self.rect.y + self.rect.height + 4.0).min(window_height - badge_height - padding)
        };

        Rect::new(x, y, badge_width, badge_height)
    }
}

/// Evaluated box model boundary values.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxModelValues {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub z: f64,
    pub clip_context: Option<String>,
}

/// Formatted algebraic port equation entry.
#[derive(Debug, Clone, PartialEq)]
pub struct PortEquationEntry {
    pub name: String,
    pub authored_equation: String,
    pub evaluated_value: Option<String>,
}

/// Classification of the origin of a property or port value in the DAG.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortOriginKind {
    /// Explicit literal constant authored directly on the element in markup (e.g. `100.0`, `#cbd5e1`).
    Literal,
    /// Inferred or layout default value (e.g. synthesized default width 100, default height 24, font size 16).
    InferredDefault,
    /// Algebraic expression or formula computed from upstream dependencies (e.g. `parent.width - 44`).
    Expression,
    /// Reactive component internal state variable (e.g. `state focused: Boolean = false`).
    StateVariable,
    /// Dynamic engine function call (e.g. `text_width(...)`).
    DynamicFunction,
}

/// A node dependency in the DAG trace.
#[derive(Debug, Clone, PartialEq)]
pub struct DagTraceDependency {
    pub node_id: NodeId,
    pub port_name: String,
    pub node_label: String,
    pub value_str: String,
    pub equation_str: String,
    pub is_literal: bool,
}

/// Comprehensive DAG trace information for a specific property/port on a node.
#[derive(Debug, Clone, PartialEq)]
pub struct DagPropertyTrace {
    pub target_id: NodeId,
    pub target_label: String,
    pub port_name: String,
    pub canonical_var: String,
    pub evaluated_value_str: String,
    pub value_type: String,
    pub equation_str: String,
    pub origin_kind: PortOriginKind,
    pub origin_description: String,
    pub is_authored: bool,
    pub is_state: bool,
    pub upstream_dependencies: Vec<DagTraceDependency>,
    pub downstream_dependents: Vec<DagTraceDependency>,
    pub upstream_chain: Vec<DagTraceDependency>,
}

/// Formats an AST expression into a human-friendly equation string replacing canonical node IDs
/// with readable component labels (e.g. `__node_5.width` -> `\VStack#5.width` or `reader_pane.width`).
pub fn format_dag_expr(expr: &Expr, layout: &ResolvedLayout) -> String {
    match expr {
        Expr::Literal(lit) => format!("{lit}"),
        Expr::Ident(id) => {
            if id.as_str() == "__window" || id.as_str() == "window" {
                "window".to_string()
            } else if let Some(nid) = NodeId::from_canonical_name(id.as_str()) {
                layout.node_display_label(nid)
            } else {
                id.to_string()
            }
        }
        Expr::MemberAccess(m) => {
            let target_str = format_dag_expr(&m.target, layout);
            format!("{}.{}", target_str, m.member)
        }
        Expr::Binary(b) => {
            format!(
                "{} {} {}",
                format_dag_expr(&b.left, layout),
                b.op,
                format_dag_expr(&b.right, layout)
            )
        }
        Expr::Unary(u) => {
            format!("{}{}", u.op, format_dag_expr(&u.operand, layout))
        }
        Expr::Call(c) => {
            let args_str = c
                .args
                .iter()
                .map(|a| format_dag_expr(a, layout))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({})", c.callee, args_str)
        }
        Expr::MethodCall(m) => {
            let args_str = m
                .args
                .iter()
                .map(|a| format_dag_expr(a, layout))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}.{}({})", format_dag_expr(&m.target, layout), m.method, args_str)
        }
        Expr::Ternary(t) => {
            format!(
                "{} ? {} : {}",
                format_dag_expr(&t.condition, layout),
                format_dag_expr(&t.then_expr, layout),
                format_dag_expr(&t.else_expr, layout)
            )
        }
        Expr::Paren(inner, _) => {
            format!("({})", format_dag_expr(inner, layout))
        }
        Expr::Node(node) => {
            format!("\\{}(...)", node.name)
        }
    }
}

/// Constructs a full DAG trace for a specific property/port on `target_id`.
pub fn build_property_dag_trace(
    layout: &ResolvedLayout,
    target_id: NodeId,
    port: &str,
) -> Option<DagPropertyTrace> {
    let (target_label, target_node_opt) = if target_id.is_window() {
        ("window".to_string(), None)
    } else {
        let node = layout.get_node(target_id)?;
        (layout.node_display_label(target_id), Some(node))
    };

    let var_id = VarId::new(target_id, port);
    let canonical_var = var_id.to_string();

    let val_opt = layout.get_value(target_id, port).or_else(|| {
        layout.values.get(&var_id)
    });

    let evaluated_value_str = if let Some(v) = val_opt {
        match v {
            Value::Node(ref_id) => format!("Reference -> {}", layout.node_display_label(*ref_id)),
            _ => format!("{v}"),
        }
    } else if let Some(n) = target_node_opt {
        match port {
            "x" => format!("{:.1}", n.rect.x),
            "y" => format!("{:.1}", n.rect.y),
            "width" => format!("{:.1}", n.rect.width),
            "height" => format!("{:.1}", n.rect.height),
            "z" => format!("{:.0}", n.z),
            "clip" => n.clip.map(|c| layout.node_display_label(c)).unwrap_or_else(|| "none".to_string()),
            _ => "-".to_string(),
        }
    } else {
        "-".to_string()
    };

    let value_type = if let Some(v) = val_opt {
        match v {
            Value::Number(_) => "Number".to_string(),
            Value::String(_) => "String".to_string(),
            Value::Bool(_) => "Boolean".to_string(),
            Value::Color(_) => "Color".to_string(),
            Value::Node(_) => "Node Reference".to_string(),
            Value::Enum { enum_name, .. } => format!("Enum ({enum_name})"),
        }
    } else {
        match port {
            "x" | "y" | "width" | "height" | "z" => "Number".to_string(),
            "clip" => "Node Reference".to_string(),
            _ => "Unknown".to_string(),
        }
    };

    let is_state = target_node_opt.is_some_and(|n| n.state_vars.contains_key(port));
    let is_authored = target_node_opt.is_some_and(|n| n.formulas.contains_key(port));

    let equation_str = if let Some(graph) = &layout.graph {
        if let Some(vnode) = graph.get_variable(&var_id) {
            format_dag_expr(&vnode.equation, layout)
        } else if let Some(n) = target_node_opt {
            n.formulas.get(port).cloned().unwrap_or_else(|| evaluated_value_str.clone())
        } else {
            evaluated_value_str.clone()
        }
    } else if let Some(n) = target_node_opt {
        n.formulas.get(port).cloned().unwrap_or_else(|| evaluated_value_str.clone())
    } else {
        evaluated_value_str.clone()
    };

    let mut upstream_dependencies = Vec::new();
    let mut downstream_dependents = Vec::new();
    let mut upstream_chain = Vec::new();

    if let Some(graph) = &layout.graph {
        if let Some(deps) = graph.upstream.get(&var_id) {
            for dep in deps {
                let dep_label = layout.node_display_label(dep.node);
                let dep_val = layout.get_value(dep.node, &dep.port)
                    .or_else(|| layout.values.get(dep))
                    .map(|v| format!("{v}"))
                    .unwrap_or_else(|| "-".to_string());
                let dep_eq = if let Some(vnode) = graph.get_variable(dep) {
                    format_dag_expr(&vnode.equation, layout)
                } else {
                    String::new()
                };
                let is_lit = graph.upstream.get(dep).map_or(true, |u| u.is_empty());
                upstream_dependencies.push(DagTraceDependency {
                    node_id: dep.node,
                    port_name: dep.port.clone(),
                    node_label: dep_label,
                    value_str: dep_val,
                    equation_str: dep_eq,
                    is_literal: is_lit,
                });
            }
        }

        if let Some(deps) = graph.downstream.get(&var_id) {
            for dep in deps {
                let dep_label = layout.node_display_label(dep.node);
                let dep_val = layout.get_value(dep.node, &dep.port)
                    .or_else(|| layout.values.get(dep))
                    .map(|v| format!("{v}"))
                    .unwrap_or_else(|| "-".to_string());
                let dep_eq = if let Some(vnode) = graph.get_variable(dep) {
                    format_dag_expr(&vnode.equation, layout)
                } else {
                    String::new()
                };
                downstream_dependents.push(DagTraceDependency {
                    node_id: dep.node,
                    port_name: dep.port.clone(),
                    node_label: dep_label,
                    value_str: dep_val,
                    equation_str: dep_eq,
                    is_literal: false,
                });
            }
        }

        // Multi-hop upstream origin chain (breadth-first traversal up to 8 nodes)
        let mut visited = std::collections::HashSet::new();
        visited.insert(var_id.clone());
        let mut queue = std::collections::VecDeque::new();
        if let Some(direct) = graph.upstream.get(&var_id) {
            for d in direct {
                if visited.insert(d.clone()) {
                    queue.push_back(d.clone());
                }
            }
        }
        while let Some(curr_dep) = queue.pop_front() {
            if upstream_chain.len() >= 12 {
                break;
            }
            let label = layout.node_display_label(curr_dep.node);
            let val = layout.get_value(curr_dep.node, &curr_dep.port)
                .or_else(|| layout.values.get(&curr_dep))
                .map(|v| format!("{v}"))
                .unwrap_or_else(|| "-".to_string());
            let eq = if let Some(vnode) = graph.get_variable(&curr_dep) {
                format_dag_expr(&vnode.equation, layout)
            } else {
                String::new()
            };
            let is_lit = graph.upstream.get(&curr_dep).map_or(true, |u| u.is_empty());
            upstream_chain.push(DagTraceDependency {
                node_id: curr_dep.node,
                port_name: curr_dep.port.clone(),
                node_label: label,
                value_str: val,
                equation_str: eq,
                is_literal: is_lit,
            });

            if let Some(next_ups) = graph.upstream.get(&curr_dep) {
                for next in next_ups {
                    if visited.insert(next.clone()) {
                        queue.push_back(next.clone());
                    }
                }
            }
        }
    }

    let (origin_kind, origin_description) = if is_state {
        (
            PortOriginKind::StateVariable,
            "Internal reactive component state variable declared in component definition.".to_string(),
        )
    } else if !upstream_dependencies.is_empty() {
        if equation_str.starts_with("text_width(") || equation_str.starts_with("text_height(") {
            (
                PortOriginKind::DynamicFunction,
                "Computed dynamically by Parley typography engine based on upstream text content and font metrics.".to_string(),
            )
        } else {
            (
                PortOriginKind::Expression,
                format!("Computed reactively from {} upstream variable dependency(ies) in the layout DAG.", upstream_dependencies.len()),
            )
        }
    } else if is_authored {
        (
            PortOriginKind::Literal,
            "Explicit literal constant authored directly on this element in DTML markup.".to_string(),
        )
    } else if port == "width" || port == "height" {
        (
            PortOriginKind::InferredDefault,
            format!(
                "Inferred layout default ({evaluated_value_str}px). No explicit dimension was authored or inherited, so the layout compiler assigned a fallback constraint."
            ),
        )
    } else if port == "size" || port == "font_size" {
        (
            PortOriginKind::InferredDefault,
            format!(
                "Inferred default font size ({evaluated_value_str}px). Element does not inherit a custom font size, using DirectedType standard typography scale."
            ),
        )
    } else if port == "x" || port == "y" || port == "z" {
        (
            PortOriginKind::InferredDefault,
            format!(
                "Default spatial coordinate ({evaluated_value_str}). Value was defaulted to origin by the layout graph."
            ),
        )
    } else {
        (
            PortOriginKind::Literal,
            "Default constant value defined in component signature.".to_string(),
        )
    };

    Some(DagPropertyTrace {
        target_id,
        target_label,
        port_name: port.to_string(),
        canonical_var,
        evaluated_value_str,
        value_type,
        equation_str,
        origin_kind,
        origin_description,
        is_authored,
        is_state,
        upstream_dependencies,
        downstream_dependents,
        upstream_chain,
    })
}
