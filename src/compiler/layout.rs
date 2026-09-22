use crate::compiler::expanded::{ExpandedDocument, NodeId};
use crate::compiler::graph::VarId;
use crate::compiler::value::Value;
use crate::span::Span;
use std::collections::HashMap;

/// A 2D spatial rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn left(&self) -> f64 {
        self.x
    }

    pub fn top(&self) -> f64 {
        self.y
    }

    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }
}

/// A resolved visual element with concrete spatial boundaries and computed properties.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedNode {
    pub id: NodeId,
    pub name: String,
    pub rect: Rect,
    pub z: f64,
    pub text_content: Option<String>,
    pub properties: HashMap<String, Value>,
    pub span: Span,
}

/// The final computed layout produced by the DirectedType graph compiler.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedLayout {
    pub nodes: Vec<ResolvedNode>,
    pub values: HashMap<VarId, Value>,
}

impl ResolvedLayout {
    /// Returns the nodes ordered for rendering (Painter's Algorithm).
    ///
    /// Primitives are sorted primarily by `z` coordinate ascending,
    /// and secondarily by original AST array declaration order.
    pub fn render_order(&self) -> Vec<&ResolvedNode> {
        let mut indexed: Vec<(usize, &ResolvedNode)> = self.nodes.iter().enumerate().collect();
        indexed.sort_by(|(idx_a, node_a), (idx_b, node_b)| {
            node_a
                .z
                .partial_cmp(&node_b.z)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| idx_a.cmp(idx_b))
        });
        indexed.into_iter().map(|(_, n)| n).collect()
    }

    pub fn get_node(&self, id: NodeId) -> Option<&ResolvedNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn get_value(&self, node_id: NodeId, port: &str) -> Option<&Value> {
        self.values.get(&VarId::new(node_id, port))
    }
}

/// Projects evaluated variable values back onto the expanded node tree,
/// producing a complete `ResolvedLayout`.
pub fn resolve_layout(
    doc: &ExpandedDocument,
    values: HashMap<VarId, Value>,
) -> ResolvedLayout {
    let mut resolved_nodes = Vec::with_capacity(doc.nodes.len());

    for node in &doc.nodes {
        let get_num = |port: &str| -> f64 {
            values
                .get(&VarId::new(node.id, port))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
        };

        let x = get_num("x");
        let y = get_num("y");
        let width = get_num("width");
        let height = get_num("height");
        let z = get_num("z");

        let mut properties = HashMap::new();
        for (var_id, val) in &values {
            if var_id.node == node.id {
                properties.insert(var_id.port.clone(), val.clone());
            }
        }

        resolved_nodes.push(ResolvedNode {
            id: node.id,
            name: node.name.clone(),
            rect: Rect::new(x, y, width, height),
            z,
            text_content: node.text_content.clone(),
            properties,
            span: node.span,
        });
    }

    ResolvedLayout {
        nodes: resolved_nodes,
        values,
    }
}
