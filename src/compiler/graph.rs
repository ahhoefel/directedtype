use crate::ast::*;
use crate::compiler::error::CompileError;
use crate::compiler::expanded::{ExpandedDocument, NodeId};
use crate::span::Span;
use std::collections::HashMap;
use std::fmt;

/// Unique identifier for a variable / port in the layout graph: `(NodeId, Symbol)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VarId {
    pub node: NodeId,
    pub port: Symbol,
}

impl VarId {
    #[inline]
    pub fn new(node: NodeId, port: impl Into<Symbol>) -> Self {
        Self {
            node,
            port: port.into(),
        }
    }

    #[inline]
    pub const fn from_symbol(node: NodeId, port: Symbol) -> Self {
        Self { node, port }
    }
}

impl fmt::Display for VarId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.node.canonical_str(), self.port)
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
    /// Node metadata (component/element type name and source span) keyed by NodeId
    pub node_info: HashMap<NodeId, GraphNodeInfo>,
    /// Contiguous arena of variable nodes indexed by `VarIndex`
    pub var_nodes: Vec<VariableNode>,
    /// Raw upstream `VarId` dependencies indexed by `VarIndex`
    pub raw_upstream: Vec<Vec<VarId>>,
    /// Upstream dependencies indexed by `VarIndex`
    pub upstream_idx: Vec<Vec<VarIndex>>,
    /// Downstream dependents indexed by `VarIndex`
    pub downstream_idx: Vec<Vec<VarIndex>>,
    /// Per-node lookup from port symbol to `VarIndex` (indexed by `NodeId.0`)
    pub node_port_to_idx: Vec<HashMap<Symbol, VarIndex>>,
    /// Window port lookup from port symbol to `VarIndex`
    pub window_port_to_idx: HashMap<Symbol, VarIndex>,
    /// Ambient `page` port lookup from port symbol to `VarIndex`
    pub page_port_to_idx: HashMap<Symbol, VarIndex>,
}

impl VariableGraph {
    pub fn new() -> Self {
        Self {
            node_info: HashMap::new(),
            var_nodes: Vec::new(),
            raw_upstream: Vec::new(),
            upstream_idx: Vec::new(),
            downstream_idx: Vec::new(),
            node_port_to_idx: Vec::new(),
            window_port_to_idx: HashMap::new(),
            page_port_to_idx: HashMap::new(),
        }
    }

    pub fn with_node_capacity(num_nodes: usize) -> Self {
        let est_vars = num_nodes.saturating_mul(14).max(32);
        Self {
            node_info: HashMap::with_capacity(num_nodes + 1),
            var_nodes: Vec::with_capacity(est_vars),
            raw_upstream: Vec::with_capacity(est_vars),
            upstream_idx: Vec::new(),
            downstream_idx: Vec::new(),
            node_port_to_idx: vec![HashMap::new(); num_nodes],
            window_port_to_idx: HashMap::with_capacity(16),
            page_port_to_idx: HashMap::with_capacity(16),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.var_nodes.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.var_nodes.is_empty()
    }

    /// Looks up a variable's `VarIndex` by `(NodeId, Symbol)` without heap allocation.
    #[inline]
    pub fn lookup_var_index(&self, node: NodeId, port: impl Into<Symbol>) -> Option<VarIndex> {
        self.lookup_var_index_sym(node, port.into())
    }

    /// Looks up a variable's `VarIndex` by `(NodeId, Symbol)`.
    #[inline]
    pub fn lookup_var_index_sym(&self, node: NodeId, port: Symbol) -> Option<VarIndex> {
        if node.is_window() {
            self.window_port_to_idx.get(&port).copied()
        } else if node.is_page() {
            self.page_port_to_idx.get(&port).copied()
        } else {
            self.node_port_to_idx.get(node.0)?.get(&port).copied()
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
        let deps = extract_var_dependencies_for_node(&equation, id.node);
        let node_id = id.node;

        if node_id.is_window() {
            if let Some(&idx) = self.window_port_to_idx.get(&id.port) {
                let i = idx.0 as usize;
                self.var_nodes[i] = VariableNode { id, equation, span };
                self.raw_upstream[i] = deps;
            } else {
                let idx = VarIndex(self.var_nodes.len() as u32);
                self.window_port_to_idx.insert(id.port, idx);
                self.var_nodes.push(VariableNode { id, equation, span });
                self.raw_upstream.push(deps);
            }
        } else if node_id.is_page() {
            if let Some(&idx) = self.page_port_to_idx.get(&id.port) {
                let i = idx.0 as usize;
                self.var_nodes[i] = VariableNode { id, equation, span };
                self.raw_upstream[i] = deps;
            } else {
                let idx = VarIndex(self.var_nodes.len() as u32);
                self.page_port_to_idx.insert(id.port, idx);
                self.var_nodes.push(VariableNode { id, equation, span });
                self.raw_upstream.push(deps);
            }
        } else {
            if node_id.0 >= self.node_port_to_idx.len() {
                self.node_port_to_idx.resize_with(node_id.0 + 1, HashMap::new);
            }
            if let Some(&idx) = self.node_port_to_idx[node_id.0].get(&id.port) {
                let i = idx.0 as usize;
                self.var_nodes[i] = VariableNode { id, equation, span };
                self.raw_upstream[i] = deps;
            } else {
                let idx = VarIndex(self.var_nodes.len() as u32);
                self.node_port_to_idx[node_id.0].insert(id.port, idx);
                self.var_nodes.push(VariableNode { id, equation, span });
                self.raw_upstream.push(deps);
            }
        }
    }

    pub fn update_variable_equation(&mut self, id: &VarId, equation: Expr) {
        if let Some(idx) = self.lookup_var_index_sym(id.node, id.port) {
            if let Some(var) = self.var_nodes.get_mut(idx.0 as usize) {
                var.equation = equation;
            }
        }
    }

    pub fn get_variable(&self, id: &VarId) -> Option<&VariableNode> {
        let idx = self.lookup_var_index_sym(id.node, id.port)?;
        self.var_nodes.get(idx.0 as usize)
    }

    /// Returns the slice of upstream `VarId` dependencies for a variable.
    pub fn upstream_of(&self, id: &VarId) -> Option<&[VarId]> {
        let idx = self.lookup_var_index_sym(id.node, id.port)?;
        self.raw_upstream.get(idx.0 as usize).map(|v| v.as_slice())
    }

    /// Returns the downstream `VarId` dependents for a variable.
    pub fn downstream_of(&self, id: &VarId) -> Option<Vec<VarId>> {
        let idx = self.lookup_var_index_sym(id.node, id.port)?;
        if self.downstream_idx.len() == self.var_nodes.len() {
            let down = self.downstream_idx.get(idx.0 as usize)?;
            Some(
                down.iter()
                    .map(|&d| self.var_nodes[d.0 as usize].id)
                    .collect(),
            )
        } else {
            let mut out = Vec::new();
            for (other_idx, ups) in self.raw_upstream.iter().enumerate() {
                if ups.contains(id) {
                    out.push(self.var_nodes[other_idx].id);
                }
            }
            Some(out)
        }
    }

    /// Returns the in-degree (number of upstream dependencies) for a variable.
    pub fn in_degree(&self, id: &VarId) -> usize {
        self.upstream_of(id).map_or(0, |deps| deps.len())
    }

    /// Returns the out-degree (number of downstream dependents) for a variable.
    pub fn out_degree(&self, id: &VarId) -> usize {
        let Some(idx) = self.lookup_var_index_sym(id.node, id.port) else {
            return 0;
        };
        if let Some(down) = self.downstream_idx.get(idx.0 as usize) {
            down.len()
        } else {
            self.raw_upstream
                .iter()
                .filter(|ups| ups.contains(id))
                .count()
        }
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

/// Builds a `VariableGraph` from an `ExpandedDocument` with specific viewport/window dimensions
/// and default DPI (`96.0` for interactive screen viewing, `72.0` for PDF/print output).
pub fn build_variable_graph_with_window(
    doc: &ExpandedDocument,
    window_width: f64,
    window_height: f64,
) -> Result<VariableGraph, CompileError> {
    build_variable_graph_with_window_and_dpi(doc, window_width, window_height, 96.0)
}

/// Builds a `VariableGraph` from an `ExpandedDocument` with explicit viewport dimensions and default DPI.
pub fn build_variable_graph_with_window_and_dpi(
    doc: &ExpandedDocument,
    window_width: f64,
    window_height: f64,
    default_dpi: f64,
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
        Expr::Literal(Literal::Float(default_dpi, window_span)),
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
        if matches!(expr, Expr::ExprDef(_))
            || doc
                .window_port_types
                .get(port_name)
                .is_some_and(|s| matches!(s.as_str(), "Expr" | "Expression"))
        {
            continue;
        }
        let span = expr.span();
        graph.add_variable(
            VarId::new(NodeId::WINDOW, port_name.as_str()),
            expr.clone(),
            span,
        );
    }

    if !doc.page_ports.is_empty() {
        graph.set_node_info(NodeId::PAGE, "Page", window_span);
        for (port_name, expr) in &doc.page_ports {
            let span = expr.span();
            graph.add_variable(
                VarId::new(NodeId::PAGE, port_name.as_str()),
                expr.clone(),
                span,
            );
        }
    }

    for node in &doc.nodes {
        graph.set_node_info(
            node.id,
            unmangle_component_name(&node.name),
            node.span,
        );
        if node.id.0 < graph.node_port_to_idx.len() {
            graph.node_port_to_idx[node.id.0].reserve(node.ports.len() + 9);
        }
        let is_root = doc.roots.contains(&node.id) || node.parent.is_none_or(|p| p.is_window());
        let is_prim_spatial = !node.is_component
            && !matches!(
                node.name.as_str(),
                "Font"
                    | "Pagination"
                    | "PageBreak"
                    | "PageValue"
                    | "PageList"
                    | "Deposit"
                    | "Collect"
                    | "PageContext"
            );

        for (port_name, expr) in &node.ports {
            if port_name.starts_with("on_")
                || matches!(expr, Expr::ExprDef(_))
                || node
                    .port_types
                    .get(port_name)
                    .is_some_and(|s| matches!(s.as_str(), "Handler" | "Expr" | "Expression"))
            {
                continue;
            }
            graph.add_variable(
                VarId::new(node.id, port_name.as_str()),
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
                        target: std::sync::Arc::new(Expr::Ident(Ident::from_node(NodeId::WINDOW, node.span))),
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
                        target: std::sync::Arc::new(Expr::Ident(Ident::from_node(NodeId::WINDOW, node.span))),
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
        let has_x = is_prim_spatial || node.ports.contains_key("x");
        let has_y = is_prim_spatial || node.ports.contains_key("y");
        let has_w = is_prim_spatial || node.ports.contains_key("width");
        let has_h = is_prim_spatial || node.ports.contains_key("height");
        let has_left_port = node.ports.contains_key("left");
        let has_top_port = node.ports.contains_key("top");

        let need_left = has_x && !has_left_port;
        let need_top = has_y && !has_top_port;
        let need_right = (has_x || has_left_port) && has_w && !node.ports.contains_key("right");
        let need_bottom = (has_y || has_top_port) && has_h && !node.ports.contains_key("bottom");

        if need_left || need_top || need_right || need_bottom {
            let self_ident = Expr::Ident(Ident::from_node(node.id, node.span));
            if need_left {
                graph.add_variable(
                    VarId::new(node.id, "left"),
                    Expr::MemberAccess(MemberAccessExpr {
                        target: std::sync::Arc::new(self_ident.clone()),
                        member: Ident::new("x", node.span),
                        span: node.span,
                    }),
                    node.span,
                );
            }
            if need_top {
                graph.add_variable(
                    VarId::new(node.id, "top"),
                    Expr::MemberAccess(MemberAccessExpr {
                        target: std::sync::Arc::new(self_ident.clone()),
                        member: Ident::new("y", node.span),
                        span: node.span,
                    }),
                    node.span,
                );
            }
            if need_right {
                let left_member = if has_x { "x" } else { "left" };
                let x = Expr::MemberAccess(MemberAccessExpr {
                    target: std::sync::Arc::new(self_ident.clone()),
                    member: Ident::new(left_member, node.span),
                    span: node.span,
                });
                let w = Expr::MemberAccess(MemberAccessExpr {
                    target: std::sync::Arc::new(self_ident.clone()),
                    member: Ident::new("width", node.span),
                    span: node.span,
                });
                graph.add_variable(
                    VarId::new(node.id, "right"),
                    Expr::Binary(BinaryExpr {
                        op: BinaryOp::Add,
                        left: std::sync::Arc::new(x),
                        right: std::sync::Arc::new(w),
                        span: node.span,
                    }),
                    node.span,
                );
            }
            if need_bottom {
                let top_member = if has_y { "y" } else { "top" };
                let y = Expr::MemberAccess(MemberAccessExpr {
                    target: std::sync::Arc::new(self_ident.clone()),
                    member: Ident::new(top_member, node.span),
                    span: node.span,
                });
                let h = Expr::MemberAccess(MemberAccessExpr {
                    target: std::sync::Arc::new(self_ident),
                    member: Ident::new("height", node.span),
                    span: node.span,
                });
                graph.add_variable(
                    VarId::new(node.id, "bottom"),
                    Expr::Binary(BinaryExpr {
                        op: BinaryOp::Add,
                        left: std::sync::Arc::new(y),
                        right: std::sync::Arc::new(h),
                        span: node.span,
                    }),
                    node.span,
                );
            }
        }
    }

    // Augment bucket `[page]` and payload dependencies once all bucket slice ports and spatial aliases exist.
    augment_bucket_and_page_dependencies(doc, &mut graph);

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
        let deps = &graph.raw_upstream[var_idx];
        if !deps.is_empty() {
            let mut up_indices = Vec::with_capacity(deps.len());
            for dep in deps {
                let Some(d_idx) = graph.lookup_var_index_sym(dep.node, dep.port) else {
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
                    if target_node.private_ports.contains(dep.port.as_str()) {
                        let source_node = doc.get_node(var_id_node);
                        let is_internal = var_id_node == dep.node
                            || source_node.and_then(|n| n.enclosing_component) == Some(dep.node)
                            || (target_node.is_span && dep.port == Symbol::BASELINE_SHIFT)
                            || var_node.id.port.as_str().starts_with("__expr_");
                        let is_ambient = source_node
                            .is_some_and(|n| n.ambient_ports.contains(var_node.id.port.as_str()));

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
    extract_var_dependencies_for_node(expr, NodeId::TEMPLATE_SELF)
}

/// Extracts all referenced variables (`VarId`) from an expression owned by `owner_node`,
/// resolving any `NodeId::TEMPLATE_SELF` references to `owner_node`.
pub fn extract_var_dependencies_for_node(expr: &Expr, owner_node: NodeId) -> Vec<VarId> {
    if matches!(expr, Expr::Literal(_) | Expr::Ident(_)) {
        return Vec::new();
    }
    let mut deps = Vec::new();
    collect_dependencies(expr, owner_node, &mut deps);
    if deps.len() > 1 {
        deps.sort_unstable();
        deps.dedup();
    }
    deps
}

fn collect_dependencies(expr: &Expr, owner_node: NodeId, out: &mut Vec<VarId>) {
    match expr {
        Expr::MemberAccess(m) => {
            if let Expr::Ident(target_id) = m.target.as_ref() {
                if let Some(raw_node_id) = target_id.node_id() {
                    let node_id = if raw_node_id.is_template_self() {
                        owner_node
                    } else {
                        raw_node_id
                    };
                    let var_id = VarId::from_symbol(node_id, m.member.name);
                    if !out.contains(&var_id) {
                        out.push(var_id);
                    }
                }
            } else {
                collect_dependencies(&m.target, owner_node, out);
            }
        }
        Expr::Binary(b) => {
            collect_dependencies(&b.left, owner_node, out);
            collect_dependencies(&b.right, owner_node, out);
        }
        Expr::Unary(u) => {
            collect_dependencies(&u.operand, owner_node, out);
        }
        Expr::Call(c) => {
            if DistanceUnit::from_conversion_fn(c.callee.as_str())
                .and_then(|u| u.points_per_unit())
                .is_some()
            {
                let dpi_var = VarId::from_symbol(NodeId::WINDOW, Symbol::DPI);
                if !out.contains(&dpi_var) {
                    out.push(dpi_var);
                }
            }
            for arg in &c.args {
                collect_dependencies(arg, owner_node, out);
            }
        }
        Expr::MethodCall(mc) => {
            if DistanceUnit::from_conversion_fn(mc.method.as_str())
                .and_then(|u| u.points_per_unit())
                .is_some()
            {
                let dpi_var = VarId::from_symbol(NodeId::WINDOW, Symbol::DPI);
                if !out.contains(&dpi_var) {
                    out.push(dpi_var);
                }
            }
            if mc.method.name == Symbol::APPEND {
                if let Expr::Ident(target_id) = mc.target.as_ref() {
                    if let Some(raw_node_id) = target_id.node_id() {
                        let node_id = if raw_node_id.is_template_self() {
                            owner_node
                        } else {
                            raw_node_id
                        };
                        let var_id = VarId::from_symbol(node_id, Symbol::PATH);
                        if !out.contains(&var_id) {
                            out.push(var_id);
                        }
                    }
                }
            }
            collect_dependencies(&mc.target, owner_node, out);
            for arg in &mc.args {
                collect_dependencies(arg, owner_node, out);
            }
        }
        Expr::Ternary(t) => {
            collect_dependencies(&t.condition, owner_node, out);
            collect_dependencies(&t.then_expr, owner_node, out);
            collect_dependencies(&t.else_expr, owner_node, out);
        }
        Expr::Paren(p, _) => {
            collect_dependencies(p, owner_node, out);
        }
        Expr::Index(idx) => {
            let mut tgt = idx.target.as_ref();
            while let Expr::Paren(inner, _) = tgt {
                tgt = inner;
            }
            if let Expr::Ident(bucket_ident) = tgt {
                if let Some(bucket_id) = bucket_ident.node_id() {
                    let var_id = VarId::new(bucket_id, "__bucket_count");
                    if !out.contains(&var_id) {
                        out.push(var_id);
                    }
                }
            }
            let mut idx_ref = idx.index.as_ref();
            while let Expr::Paren(inner, _) = idx_ref {
                idx_ref = inner;
            }
            if let Expr::Ident(page_ident) = idx_ref {
                if page_ident.node_id().is_some_and(|nid| nid.is_page()) {
                    let var_id = VarId::from_symbol(NodeId::PAGE, Symbol::NUMBER);
                    if !out.contains(&var_id) {
                        out.push(var_id);
                    }
                }
            }
            collect_dependencies(&idx.target, owner_node, out);
            collect_dependencies(&idx.index, owner_node, out);
        }
        Expr::Node(n) => {
            for port in &n.ports {
                collect_dependencies(&port.expr, owner_node, out);
            }
        }
        Expr::Ident(_) | Expr::Literal(_) | Expr::ExprDef(_) | Expr::Match(_) | Expr::DomNodeRef(_, _) => {}
    }
}

fn unwrap_parens_ref(mut e: &Expr) -> &Expr {
    while let Expr::Paren(inner, _) = e {
        e = inner;
    }
    e
}

fn resolve_static_index_page(idx_expr: &Expr, doc: &ExpandedDocument) -> Option<i64> {
    match unwrap_parens_ref(idx_expr) {
        Expr::Literal(Literal::Integer(p, _)) => Some(*p),
        Expr::Ident(id) => {
            let nid = id.node_id()?;
            match doc.get_node(nid)?.ports.get("__page_number")? {
                Expr::Literal(Literal::Integer(p, _)) => Some(*p),
                _ => None,
            }
        }
        Expr::MemberAccess(m) if m.member.as_str() == "prev" => {
            if let Expr::Ident(id) = unwrap_parens_ref(&m.target) {
                let nid = id.node_id()?;
                if let Some(Expr::Ident(prev_id)) = doc.get_node(nid)?.ports.get("prev") {
                    let prev_nid = prev_id.node_id()?;
                    if let Some(Expr::Literal(Literal::Integer(p, _))) =
                        doc.get_node(prev_nid)?.ports.get("__page_number")
                    {
                        return Some(*p);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn collect_bucket_deps_in_expr(
    expr: &Expr,
    doc: &ExpandedDocument,
    out: &mut Vec<VarId>,
) {
    match expr {
        Expr::MemberAccess(m) => {
            let tgt = unwrap_parens_ref(&m.target);
            if let Expr::Index(idx) = tgt {
                if let Some(bucket_id) =
                    crate::compiler::expand::builtins::pagination::resolve_bucket_node_id(
                        &idx.target,
                        doc,
                    )
                {
                    let mem = m.member.as_str();
                    if let Some(bnode) = doc.get_node(bucket_id) {
                        if let Some(p) = resolve_static_index_page(&idx.index, doc) {
                            let synth_port = format!("{mem}_p_{p}");
                            if bnode.ports.contains_key(&synth_port) {
                                let v = VarId::new(bucket_id, &synth_port);
                                if !out.contains(&v) {
                                    out.push(v);
                                }
                            }
                        } else {
                            let prefix = format!("{mem}_p_");
                            for pname in bnode.ports.keys() {
                                if pname.starts_with(&prefix) {
                                    let v = VarId::new(bucket_id, pname);
                                    if !out.contains(&v) {
                                        out.push(v);
                                    }
                                }
                            }
                        }
                    }
                }
            } else if let Expr::MemberAccess(inner_m) = tgt {
                let inner_mem = inner_m.member.as_str();
                if inner_mem == "prev" {
                    if let Expr::Ident(id) = unwrap_parens_ref(&inner_m.target) {
                        if let Some(nid) = id.node_id() {
                            if let Some(Expr::Ident(prev_id)) =
                                doc.get_node(nid).and_then(|n| n.ports.get("prev"))
                            {
                                if let Some(prev_nid) = prev_id.node_id() {
                                    let v = VarId::new(prev_nid, m.member.as_str());
                                    if !out.contains(&v) {
                                        out.push(v);
                                    }
                                }
                            }
                        }
                    }
                } else if matches!(inner_mem, "value" | "first" | "last") {
                    if let Expr::Index(idx) = unwrap_parens_ref(&inner_m.target) {
                        if let Some(bucket_id) =
                            crate::compiler::expand::builtins::pagination::resolve_bucket_node_id(
                                &idx.target,
                                doc,
                            )
                        {
                            if let Some(bnode) = doc.get_node(bucket_id) {
                                let page_opt = resolve_static_index_page(&idx.index, doc);
                                let push_payload = |pexpr: &Expr, out: &mut Vec<VarId>| {
                                    if let Expr::Ident(id) = unwrap_parens_ref(pexpr) {
                                        if let Some(payload_nid) = id.node_id() {
                                            if doc.get_node(payload_nid).is_some() {
                                                let v = VarId::new(payload_nid, m.member.as_str());
                                                if !out.contains(&v) {
                                                    out.push(v);
                                                }
                                            }
                                        }
                                    }
                                };
                                if let Some(p) = page_opt {
                                    let sp = format!("{inner_mem}_p_{p}");
                                    if let Some(pexpr) = bnode.ports.get(&sp) {
                                        push_payload(pexpr, out);
                                    }
                                } else {
                                    let prefix = format!("{inner_mem}_p_");
                                    for (pname, pexpr) in &bnode.ports {
                                        if pname.starts_with(&prefix) {
                                            push_payload(pexpr, out);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            collect_bucket_deps_in_expr(&m.target, doc, out);
        }
        Expr::Index(idx) => {
            match unwrap_parens_ref(&idx.index) {
                Expr::Ident(id) => {
                    if let Some(nid) = id.node_id() {
                        if doc
                            .get_node(nid)
                            .is_some_and(|n| n.ports.contains_key("__page_number"))
                        {
                            let v = VarId::new(nid, "__page_number");
                            if !out.contains(&v) {
                                out.push(v);
                            }
                        }
                    }
                }
                Expr::MemberAccess(m) if m.member.as_str() == "prev" => {
                    if let Expr::Ident(id) = unwrap_parens_ref(&m.target) {
                        if let Some(nid) = id.node_id() {
                            if let Some(Expr::Ident(prev_id)) =
                                doc.get_node(nid).and_then(|n| n.ports.get("prev"))
                            {
                                if let Some(prev_nid) = prev_id.node_id() {
                                    if doc
                                        .get_node(prev_nid)
                                        .is_some_and(|n| n.ports.contains_key("__page_number"))
                                    {
                                        let v = VarId::new(prev_nid, "__page_number");
                                        if !out.contains(&v) {
                                            out.push(v);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            collect_bucket_deps_in_expr(&idx.target, doc, out);
            collect_bucket_deps_in_expr(&idx.index, doc, out);
        }
        Expr::Binary(b) => {
            collect_bucket_deps_in_expr(&b.left, doc, out);
            collect_bucket_deps_in_expr(&b.right, doc, out);
        }
        Expr::Unary(u) => collect_bucket_deps_in_expr(&u.operand, doc, out),
        Expr::Call(c) => {
            for arg in &c.args {
                collect_bucket_deps_in_expr(arg, doc, out);
            }
        }
        Expr::MethodCall(mc) => {
            collect_bucket_deps_in_expr(&mc.target, doc, out);
            for arg in &mc.args {
                collect_bucket_deps_in_expr(arg, doc, out);
            }
        }
        Expr::Ternary(t) => {
            collect_bucket_deps_in_expr(&t.condition, doc, out);
            collect_bucket_deps_in_expr(&t.then_expr, doc, out);
            collect_bucket_deps_in_expr(&t.else_expr, doc, out);
        }
        Expr::Paren(p, _) => collect_bucket_deps_in_expr(p, doc, out),
        Expr::Node(n) => {
            for p in &n.ports {
                collect_bucket_deps_in_expr(&p.expr, doc, out);
            }
        }
        Expr::Match(m) => {
            if let Some(ref s) = m.scrutinee {
                collect_bucket_deps_in_expr(s, doc, out);
            }
            for arm in &m.arms {
                if let MatchPattern::Condition(cond) = &arm.pattern {
                    collect_bucket_deps_in_expr(cond, doc, out);
                }
                collect_bucket_deps_in_expr(&arm.body, doc, out);
            }
        }
        Expr::Ident(_) | Expr::Literal(_) | Expr::ExprDef(_) | Expr::DomNodeRef(_, _) => {}
    }
}

fn resolve_indirect_node_target(
    expr: &Expr,
    doc: &ExpandedDocument,
    depth: usize,
) -> Option<NodeId> {
    if depth > 8 {
        return None;
    }
    match unwrap_parens_ref(expr) {
        Expr::Ident(id) => id
            .node_id()
            .filter(|nid| !nid.is_window() && !nid.is_page() && !nid.is_template_self()),
        Expr::MemberAccess(m) => {
            let base = resolve_indirect_node_target(&m.target, doc, depth + 1)?;
            let port_expr = doc.get_node(base)?.ports.get(m.member.as_str())?;
            resolve_indirect_node_target(port_expr, doc, depth + 1)
        }
        _ => None,
    }
}

fn collect_indirect_node_deps_in_expr(
    expr: &Expr,
    doc: &ExpandedDocument,
    out: &mut Vec<VarId>,
) {
    match expr {
        Expr::MemberAccess(m) => {
            if !matches!(unwrap_parens_ref(&m.target), Expr::Ident(id) if id.node_id().is_some()) {
                if let Some(target_nid) = resolve_indirect_node_target(&m.target, doc, 0) {
                    let v = VarId::new(target_nid, m.member.as_str());
                    if !out.contains(&v) {
                        out.push(v);
                    }
                }
            }
            collect_indirect_node_deps_in_expr(&m.target, doc, out);
        }
        Expr::Index(idx) => {
            collect_indirect_node_deps_in_expr(&idx.target, doc, out);
            collect_indirect_node_deps_in_expr(&idx.index, doc, out);
        }
        Expr::Binary(b) => {
            collect_indirect_node_deps_in_expr(&b.left, doc, out);
            collect_indirect_node_deps_in_expr(&b.right, doc, out);
        }
        Expr::Unary(u) => collect_indirect_node_deps_in_expr(&u.operand, doc, out),
        Expr::Call(c) => {
            for arg in &c.args {
                collect_indirect_node_deps_in_expr(arg, doc, out);
            }
        }
        Expr::MethodCall(mc) => {
            collect_indirect_node_deps_in_expr(&mc.target, doc, out);
            for arg in &mc.args {
                collect_indirect_node_deps_in_expr(arg, doc, out);
            }
        }
        Expr::Ternary(t) => {
            collect_indirect_node_deps_in_expr(&t.condition, doc, out);
            collect_indirect_node_deps_in_expr(&t.then_expr, doc, out);
            collect_indirect_node_deps_in_expr(&t.else_expr, doc, out);
        }
        Expr::Paren(p, _) => collect_indirect_node_deps_in_expr(p, doc, out),
        Expr::Node(n) => {
            for p in &n.ports {
                collect_indirect_node_deps_in_expr(&p.expr, doc, out);
            }
        }
        Expr::Match(m) => {
            if let Some(ref s) = m.scrutinee {
                collect_indirect_node_deps_in_expr(s, doc, out);
            }
            for arm in &m.arms {
                if let MatchPattern::Condition(cond) = &arm.pattern {
                    collect_indirect_node_deps_in_expr(cond, doc, out);
                }
                collect_indirect_node_deps_in_expr(&arm.body, doc, out);
            }
        }
        Expr::Ident(_) | Expr::Literal(_) | Expr::ExprDef(_) | Expr::DomNodeRef(_, _) => {}
    }
}

fn augment_bucket_and_page_dependencies(doc: &ExpandedDocument, graph: &mut VariableGraph) {
    let mut extra = Vec::new();
    let mut indirect = Vec::new();
    for var_idx in 0..graph.var_nodes.len() {
        extra.clear();
        indirect.clear();
        collect_bucket_deps_in_expr(&graph.var_nodes[var_idx].equation, doc, &mut extra);
        collect_indirect_node_deps_in_expr(&graph.var_nodes[var_idx].equation, doc, &mut indirect);
        indirect.retain(|dep| graph.lookup_var_index_sym(dep.node, dep.port).is_some());
        let raw = &mut graph.raw_upstream[var_idx];
        for dep in &extra {
            if !raw.contains(dep) {
                raw.push(*dep);
            }
        }
        for dep in &indirect {
            if !raw.contains(dep) {
                raw.push(*dep);
            }
        }
    }
}
