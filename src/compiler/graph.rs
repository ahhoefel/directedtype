use crate::ast::*;
use crate::compiler::error::CompileError;
use crate::compiler::expanded::{ExpandedDocument, NodeId};
use crate::span::Span;
use std::collections::HashMap;
use std::fmt;

/// Unique identifier for a variable / port in the layout graph: `(NodeId, PortName)`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VarId {
    pub node: NodeId,
    pub port: String,
}

impl VarId {
    pub fn new(node: NodeId, port: impl Into<String>) -> Self {
        Self {
            node,
            port: port.into(),
        }
    }
}

impl fmt::Display for VarId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.node.canonical_name(), self.port)
    }
}

/// A variable node in the DAG containing its defining algebraic equation.
#[derive(Debug, Clone, PartialEq)]
pub struct VariableNode {
    pub id: VarId,
    pub equation: Expr,
    pub span: Span,
}

/// Metadata about a node in the layout graph for diagnostic reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNodeInfo {
    pub name: String,
    pub span: Span,
}

pub fn unmangle_component_name(name: &str) -> &str {
    if let Some(rest) = name.strip_prefix("__dt_") {
        if let Some((hash_part, original)) = rest.split_once('_') {
            if !hash_part.is_empty() && hash_part.chars().all(|c| c.is_ascii_hexdigit()) {
                return original;
            }
        }
    }
    name
}

/// Compact index of a variable in `VariableGraph::var_nodes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VarIndex(pub u32);

/// The flat layout dependency graph.
#[derive(Debug, Clone, PartialEq)]
pub struct VariableGraph {
    /// All variables in the DAG keyed by VarId
    pub variables: HashMap<VarId, VariableNode>,
    /// Upstream dependencies: VarId -> list of variables it reads from
    pub upstream: HashMap<VarId, Vec<VarId>>,
    /// Downstream dependents: VarId -> list of variables that depend on it
    pub downstream: HashMap<VarId, Vec<VarId>>,
    /// Node metadata (component/element type name and source span) keyed by NodeId
    pub node_info: HashMap<NodeId, GraphNodeInfo>,
    /// Contiguous arena of variable nodes indexed by `VarIndex`
    pub var_nodes: Vec<VariableNode>,
    /// Upstream dependencies indexed by `VarIndex`
    pub upstream_idx: Vec<Vec<VarIndex>>,
    /// Downstream dependents indexed by `VarIndex`
    pub downstream_idx: Vec<Vec<VarIndex>>,
    /// Per-node lookup from port name to `VarIndex` (indexed by `NodeId.0`)
    pub node_port_to_idx: Vec<HashMap<String, VarIndex>>,
    /// Window port lookup from port name to `VarIndex`
    pub window_port_to_idx: HashMap<String, VarIndex>,
}

impl VariableGraph {
    pub fn new() -> Self {
        Self {
            variables: HashMap::new(),
            upstream: HashMap::new(),
            downstream: HashMap::new(),
            node_info: HashMap::new(),
            var_nodes: Vec::new(),
            upstream_idx: Vec::new(),
            downstream_idx: Vec::new(),
            node_port_to_idx: Vec::new(),
            window_port_to_idx: HashMap::new(),
        }
    }

    pub fn with_node_capacity(num_nodes: usize) -> Self {
        let est_vars = num_nodes.saturating_mul(14).max(32);
        Self {
            variables: HashMap::with_capacity(est_vars),
            upstream: HashMap::with_capacity(est_vars),
            downstream: HashMap::with_capacity(est_vars),
            node_info: HashMap::with_capacity(num_nodes + 1),
            var_nodes: Vec::with_capacity(est_vars),
            upstream_idx: Vec::new(),
            downstream_idx: Vec::new(),
            node_port_to_idx: vec![HashMap::new(); num_nodes],
            window_port_to_idx: HashMap::with_capacity(16),
        }
    }

    /// Looks up a variable's `VarIndex` by `(NodeId, &str)` without heap allocation.
    #[inline]
    pub fn lookup_var_index(&self, node: NodeId, port: &str) -> Option<VarIndex> {
        if node.is_window() {
            self.window_port_to_idx.get(port).copied()
        } else {
            self.node_port_to_idx.get(node.0)?.get(port).copied()
        }
    }

    /// Registers diagnostic metadata (component/element type name and span) for a node.
    pub fn set_node_info(&mut self, id: NodeId, name: impl Into<String>, span: Span) {
        self.node_info.insert(
            id,
            GraphNodeInfo {
                name: name.into(),
                span,
            },
        );
    }

    /// Adds a variable to the graph with its defining equation.
    pub fn add_variable(&mut self, id: VarId, equation: Expr, span: Span) {
        let deps = extract_var_dependencies(&equation);

        let up_entry = self.upstream.entry(id.clone()).or_default();
        up_entry.clear();
        up_entry.reserve(deps.len());
        for dep in &deps {
            up_entry.push(dep.clone());
            self.downstream
                .entry(dep.clone())
                .or_default()
                .push(id.clone());
        }

        self.downstream.entry(id.clone()).or_default();

        let vnode = VariableNode {
            id: id.clone(),
            equation,
            span,
        };

        if id.node.is_window() {
            if let Some(&idx) = self.window_port_to_idx.get(&id.port) {
                self.var_nodes[idx.0 as usize] = vnode.clone();
            } else {
                let idx = VarIndex(self.var_nodes.len() as u32);
                self.window_port_to_idx.insert(id.port.clone(), idx);
                self.var_nodes.push(vnode.clone());
            }
        } else {
            if id.node.0 >= self.node_port_to_idx.len() {
                self.node_port_to_idx.resize_with(id.node.0 + 1, HashMap::new);
            }
            if let Some(&idx) = self.node_port_to_idx[id.node.0].get(&id.port) {
                self.var_nodes[idx.0 as usize] = vnode.clone();
            } else {
                let idx = VarIndex(self.var_nodes.len() as u32);
                self.node_port_to_idx[id.node.0].insert(id.port.clone(), idx);
                self.var_nodes.push(vnode.clone());
            }
        }

        self.variables.insert(id, vnode);
    }

    pub fn update_variable_equation(&mut self, id: &VarId, equation: Expr) {
        if let Some(idx) = self.lookup_var_index(id.node, &id.port) {
            if let Some(var) = self.var_nodes.get_mut(idx.0 as usize) {
                var.equation = equation.clone();
            }
        }
        if let Some(var) = self.variables.get_mut(id) {
            var.equation = equation;
        }
    }

    pub fn get_variable(&self, id: &VarId) -> Option<&VariableNode> {
        if let Some(idx) = self.lookup_var_index(id.node, &id.port) {
            self.var_nodes.get(idx.0 as usize)
        } else {
            self.variables.get(id)
        }
    }

    /// Returns the in-degree (number of upstream dependencies) for a variable.
    pub fn in_degree(&self, id: &VarId) -> usize {
        self.upstream.get(id).map_or(0, |deps| deps.len())
    }

    /// Returns the out-degree (number of downstream dependents) for a variable.
    pub fn out_degree(&self, id: &VarId) -> usize {
        self.downstream.get(id).map_or(0, |deps| deps.len())
    }
}

impl Default for VariableGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Builds a `VariableGraph` from an `ExpandedDocument` using the default window size (800x600).
pub fn build_variable_graph(doc: &ExpandedDocument) -> Result<VariableGraph, CompileError> {
    build_variable_graph_with_window(doc, 800.0, 600.0)
}

/// Builds a `VariableGraph` from an `ExpandedDocument` with specific viewport/window dimensions.
pub fn build_variable_graph_with_window(
    doc: &ExpandedDocument,
    window_width: f64,
    window_height: f64,
) -> Result<VariableGraph, CompileError> {
    let mut graph = VariableGraph::with_node_capacity(doc.nodes.len());

    // 0. Ambient Window Node Variables
    let window_span = Span::default();
    graph.set_node_info(NodeId::WINDOW, "Window", window_span);
    graph.add_variable(
        VarId::new(NodeId::WINDOW, "x"),
        Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, window_span)),
        window_span,
    );
    graph.add_variable(
        VarId::new(NodeId::WINDOW, "y"),
        Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, window_span)),
        window_span,
    );
    graph.add_variable(
        VarId::new(NodeId::WINDOW, "z"),
        Expr::Literal(Literal::Integer(0, window_span)),
        window_span,
    );
    graph.add_variable(
        VarId::new(NodeId::WINDOW, "width"),
        Expr::Literal(Literal::Distance(window_width, DistanceUnit::Px, window_span)),
        window_span,
    );
    graph.add_variable(
        VarId::new(NodeId::WINDOW, "height"),
        Expr::Literal(Literal::Distance(window_height, DistanceUnit::Px, window_span)),
        window_span,
    );
    graph.add_variable(
        VarId::new(NodeId::WINDOW, "dpi"),
        Expr::Literal(Literal::Float(96.0, window_span)),
        window_span,
    );
    graph.add_variable(
        VarId::new(NodeId::WINDOW, "clip"),
        Expr::Ident(Ident::from_node(NodeId::WINDOW, window_span)),
        window_span,
    );
    graph.add_variable(
        VarId::new(NodeId::WINDOW, "left"),
        Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, window_span)),
        window_span,
    );
    graph.add_variable(
        VarId::new(NodeId::WINDOW, "top"),
        Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, window_span)),
        window_span,
    );
    graph.add_variable(
        VarId::new(NodeId::WINDOW, "right"),
        Expr::Literal(Literal::Distance(window_width, DistanceUnit::Px, window_span)),
        window_span,
    );
    graph.add_variable(
        VarId::new(NodeId::WINDOW, "bottom"),
        Expr::Literal(Literal::Distance(window_height, DistanceUnit::Px, window_span)),
        window_span,
    );

    for (port_name, expr) in &doc.window_ports {
        let span = expr.span();
        graph.add_variable(
            VarId::new(NodeId::WINDOW, port_name.clone()),
            expr.clone(),
            span,
        );
    }

    for node in &doc.nodes {
        graph.set_node_info(
            node.id,
            unmangle_component_name(&node.name),
            node.span,
        );
        let is_root = doc.roots.contains(&node.id) || node.parent.is_none_or(|p| p.is_window());
        let is_prim_spatial = !node.is_component && node.name != "Font";

        for (port_name, expr) in &node.ports {
            if port_name.starts_with("on_")
                || node.port_types.get(port_name).map(|s| s.as_str()) == Some("Handler")
            {
                continue;
            }
            graph.add_variable(
                VarId::new(node.id, port_name.clone()),
                expr.clone(),
                node.span,
            );
        }

        if is_prim_spatial {
            // Default spatial ports for primitive elements
            if !node.ports.contains_key("x") {
                graph.add_variable(
                    VarId::new(node.id, "x"),
                    Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, node.span)),
                    node.span,
                );
            }
            if !node.ports.contains_key("y") {
                graph.add_variable(
                    VarId::new(node.id, "y"),
                    Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, node.span)),
                    node.span,
                );
            }
            if !node.ports.contains_key("z") {
                graph.add_variable(
                    VarId::new(node.id, "z"),
                    Expr::Literal(Literal::Integer(0, node.span)),
                    node.span,
                );
            }
            if !node.ports.contains_key("width") {
                let w_expr = if is_root {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::from_node(NodeId::WINDOW, node.span))),
                        member: Ident::new("width", node.span),
                        span: node.span,
                    })
                } else {
                    Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, node.span))
                };
                graph.add_variable(VarId::new(node.id, "width"), w_expr, node.span);
            }
            if !node.ports.contains_key("height") {
                let h_expr = if is_root {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::from_node(NodeId::WINDOW, node.span))),
                        member: Ident::new("height", node.span),
                        span: node.span,
                    })
                } else {
                    Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, node.span))
                };
                graph.add_variable(VarId::new(node.id, "height"), h_expr, node.span);
            }
        }

        // Default spatial alias ports only when their input ports are present and no alias is defined
        let self_ident = Expr::Ident(Ident::from_node(node.id, node.span));
        let has_x = is_prim_spatial || node.ports.contains_key("x");
        let has_y = is_prim_spatial || node.ports.contains_key("y");
        let has_w = is_prim_spatial || node.ports.contains_key("width");
        let has_h = is_prim_spatial || node.ports.contains_key("height");
        let has_left_port = node.ports.contains_key("left");
        let has_top_port = node.ports.contains_key("top");

        if has_x && !has_left_port {
            graph.add_variable(
                VarId::new(node.id, "left"),
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("x", node.span),
                    span: node.span,
                }),
                node.span,
            );
        }
        if has_y && !has_top_port {
            graph.add_variable(
                VarId::new(node.id, "top"),
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("y", node.span),
                    span: node.span,
                }),
                node.span,
            );
        }
        if (has_x || has_left_port) && has_w && !node.ports.contains_key("right") {
            let left_member = if has_x { "x" } else { "left" };
            let x = Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(self_ident.clone()),
                member: Ident::new(left_member, node.span),
                span: node.span,
            });
            let w = Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(self_ident.clone()),
                member: Ident::new("width", node.span),
                span: node.span,
            });
            graph.add_variable(
                VarId::new(node.id, "right"),
                Expr::Binary(BinaryExpr {
                    op: BinaryOp::Add,
                    left: Box::new(x),
                    right: Box::new(w),
                    span: node.span,
                }),
                node.span,
            );
        }
        if (has_y || has_top_port) && has_h && !node.ports.contains_key("bottom") {
            let top_member = if has_y { "y" } else { "top" };
            let y = Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(self_ident.clone()),
                member: Ident::new(top_member, node.span),
                span: node.span,
            });
            let h = Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(self_ident),
                member: Ident::new("height", node.span),
                span: node.span,
            });
            graph.add_variable(
                VarId::new(node.id, "bottom"),
                Expr::Binary(BinaryExpr {
                    op: BinaryOp::Add,
                    left: Box::new(y),
                    right: Box::new(h),
                    span: node.span,
                }),
                node.span,
            );
        }
    }

    // Validate that all referenced VarIds actually exist as defined variables in the graph,
    // and populate flat VarIndex edge lists for fast topological sort and evaluation.
    let num_vars = graph.var_nodes.len();
    graph.upstream_idx = vec![Vec::new(); num_vars];
    graph.downstream_idx = vec![Vec::new(); num_vars];

    for var_idx in 0..num_vars {
        let u_idx = VarIndex(var_idx as u32);
        let var_node = &graph.var_nodes[var_idx];
        let var_span = var_node.span;
        let var_id_node = var_node.id.node;
        if let Some(deps) = graph.upstream.get(&var_node.id) {
            let mut up_indices = Vec::with_capacity(deps.len());
            for dep in deps {
                let Some(d_idx) = graph.lookup_var_index(dep.node, &dep.port) else {
                    let node_name = doc
                        .get_node(dep.node)
                        .map(|n| n.name.as_str())
                        .unwrap_or("unknown");
                    return Err(CompileError::Custom {
                        message: format!(
                            "Node '{}' has no public property '{}'",
                            node_name, dep.port
                        ),
                        span: var_span,
                    });
                };
                if let Some(target_node) = doc.get_node(dep.node) {
                    if target_node.private_ports.contains(&dep.port) {
                        let source_node = doc.get_node(var_id_node);
                        let is_internal = var_id_node == dep.node
                            || source_node.and_then(|n| n.enclosing_component) == Some(dep.node)
                            || (target_node.is_span && dep.port == "baseline_shift");
                        let is_ambient = source_node
                            .is_some_and(|n| n.ambient_ports.contains(&var_node.id.port));

                        if !is_internal && !is_ambient {
                            return Err(CompileError::Custom {
                                message: format!(
                                    "Node '{}' has no public property '{}'",
                                    target_node.name, dep.port
                                ),
                                span: var_span,
                            });
                        }
                    }
                }
                up_indices.push(d_idx);
            }
            for &d_idx in &up_indices {
                graph.downstream_idx[d_idx.0 as usize].push(u_idx);
            }
            graph.upstream_idx[var_idx] = up_indices;
        }
    }

    Ok(graph)
}

/// Extracts all referenced variables (`VarId`) from an expression.
pub fn extract_var_dependencies(expr: &Expr) -> Vec<VarId> {
    let mut deps = Vec::new();
    collect_dependencies(expr, &mut deps);
    deps.sort_unstable();
    deps.dedup();
    deps
}

fn collect_dependencies(expr: &Expr, out: &mut Vec<VarId>) {
    match expr {
        Expr::MemberAccess(m) => {
            if let Expr::Ident(target_id) = m.target.as_ref() {
                if let Some(node_id) = NodeId::from_canonical_name(target_id.as_str()) {
                    out.push(VarId::new(node_id, m.member.as_str()));
                }
            }
            collect_dependencies(&m.target, out);
        }
        Expr::Binary(b) => {
            collect_dependencies(&b.left, out);
            collect_dependencies(&b.right, out);
        }
        Expr::Unary(u) => {
            collect_dependencies(&u.operand, out);
        }
        Expr::Call(c) => {
            for arg in &c.args {
                collect_dependencies(arg, out);
            }
        }
        Expr::MethodCall(mc) => {
            if mc.method.as_str() == "append" {
                if let Expr::Ident(target_id) = mc.target.as_ref() {
                    if let Some(node_id) = NodeId::from_canonical_name(target_id.as_str()) {
                        out.push(VarId::new(node_id, "path"));
                    }
                }
            }
            collect_dependencies(&mc.target, out);
            for arg in &mc.args {
                collect_dependencies(arg, out);
            }
        }
        Expr::Ternary(t) => {
            collect_dependencies(&t.condition, out);
            collect_dependencies(&t.then_expr, out);
            collect_dependencies(&t.else_expr, out);
        }
        Expr::Paren(p, _) => {
            collect_dependencies(p, out);
        }
        Expr::Node(n) => {
            for port in &n.ports {
                collect_dependencies(&port.expr, out);
            }
        }
        Expr::Ident(_) | Expr::Literal(_) => {}
    }
}
