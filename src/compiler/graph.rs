use crate::ast::*;
use crate::compiler::error::CompileError;
use crate::compiler::expanded::{ExpandedDocument, NodeId};
use crate::span::Span;
use std::collections::HashMap;
use std::fmt;

/// Unique identifier for a variable / port in the layout graph: `(NodeId, PortName)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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

/// The flat layout dependency graph.
#[derive(Debug, Clone, PartialEq)]
pub struct VariableGraph {
    /// All variables in the DAG keyed by VarId
    pub variables: HashMap<VarId, VariableNode>,
    /// Upstream dependencies: VarId -> list of variables it reads from
    pub upstream: HashMap<VarId, Vec<VarId>>,
    /// Downstream dependents: VarId -> list of variables that depend on it
    pub downstream: HashMap<VarId, Vec<VarId>>,
}

impl VariableGraph {
    pub fn new() -> Self {
        Self {
            variables: HashMap::new(),
            upstream: HashMap::new(),
            downstream: HashMap::new(),
        }
    }

    /// Adds a variable to the graph with its defining equation.
    pub fn add_variable(&mut self, id: VarId, equation: Expr, span: Span) {
        let deps = extract_var_dependencies(&equation);

        for dep in &deps {
            self.upstream
                .entry(id.clone())
                .or_default()
                .push(dep.clone());
            self.downstream
                .entry(dep.clone())
                .or_default()
                .push(id.clone());
        }

        self.upstream.entry(id.clone()).or_default();
        self.downstream.entry(id.clone()).or_default();

        self.variables.insert(
            id.clone(),
            VariableNode {
                id,
                equation,
                span,
            },
        );
    }

    pub fn get_variable(&self, id: &VarId) -> Option<&VariableNode> {
        self.variables.get(id)
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
    let mut graph = VariableGraph::new();

    // 0. Ambient Window Node Variables
    let window_span = Span::default();
    graph.add_variable(
        VarId::new(NodeId::WINDOW, "x"),
        Expr::Literal(Literal::Number(0.0, window_span)),
        window_span,
    );
    graph.add_variable(
        VarId::new(NodeId::WINDOW, "y"),
        Expr::Literal(Literal::Number(0.0, window_span)),
        window_span,
    );
    graph.add_variable(
        VarId::new(NodeId::WINDOW, "z"),
        Expr::Literal(Literal::Number(0.0, window_span)),
        window_span,
    );
    graph.add_variable(
        VarId::new(NodeId::WINDOW, "width"),
        Expr::Literal(Literal::Number(window_width, window_span)),
        window_span,
    );
    graph.add_variable(
        VarId::new(NodeId::WINDOW, "height"),
        Expr::Literal(Literal::Number(window_height, window_span)),
        window_span,
    );

    for node in &doc.nodes {
        let is_root = doc.roots.contains(&node.id) || node.parent.is_none_or(|p| p.is_window());
        let mut ports = node.ports.clone();

        // Base Spatial Trait: Every node has x, y, width, height, z
        if is_root {
            ports
                .entry("x".to_string())
                .or_insert_with(|| Expr::Literal(Literal::Number(0.0, node.span)));
            ports
                .entry("y".to_string())
                .or_insert_with(|| Expr::Literal(Literal::Number(0.0, node.span)));
            ports
                .entry("z".to_string())
                .or_insert_with(|| Expr::Literal(Literal::Number(0.0, node.span)));
            ports
                .entry("width".to_string())
                .or_insert_with(|| {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(
                            NodeId::WINDOW.canonical_name(),
                            node.span,
                        ))),
                        member: Ident::new("width", node.span),
                        span: node.span,
                    })
                });
            ports
                .entry("height".to_string())
                .or_insert_with(|| {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(
                            NodeId::WINDOW.canonical_name(),
                            node.span,
                        ))),
                        member: Ident::new("height", node.span),
                        span: node.span,
                    })
                });
        } else {
            ports
                .entry("x".to_string())
                .or_insert_with(|| Expr::Literal(Literal::Number(0.0, node.span)));
            ports
                .entry("y".to_string())
                .or_insert_with(|| Expr::Literal(Literal::Number(0.0, node.span)));
            ports
                .entry("z".to_string())
                .or_insert_with(|| Expr::Literal(Literal::Number(0.0, node.span)));
            ports
                .entry("width".to_string())
                .or_insert_with(|| Expr::Literal(Literal::Number(100.0, node.span)));
            ports
                .entry("height".to_string())
                .or_insert_with(|| Expr::Literal(Literal::Number(24.0, node.span)));
        }

        for (port_name, expr) in ports {
            let var_id = VarId::new(node.id, port_name);
            graph.add_variable(var_id, expr, node.span);
        }
    }

    Ok(graph)
}

/// Extracts all referenced variables (`VarId`) from an expression.
pub fn extract_var_dependencies(expr: &Expr) -> Vec<VarId> {
    let mut deps = Vec::new();
    collect_dependencies(expr, &mut deps);
    deps.sort_by_key(|a| a.to_string());
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
        Expr::Ternary(t) => {
            collect_dependencies(&t.condition, out);
            collect_dependencies(&t.then_expr, out);
            collect_dependencies(&t.else_expr, out);
        }
        Expr::Paren(p, _) => {
            collect_dependencies(p, out);
        }
        Expr::Ident(_) | Expr::Literal(_) => {}
    }
}
