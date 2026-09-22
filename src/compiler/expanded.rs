use crate::ast::Expr;
use crate::span::Span;
use std::collections::HashMap;

/// A unique, zero-based index for an expanded node in the layout tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub usize);

impl NodeId {
    /// Sentinel NodeId representing the ambient window root container.
    pub const WINDOW: NodeId = NodeId(usize::MAX);

    pub fn is_window(&self) -> bool {
        self.0 == usize::MAX
    }

    pub fn canonical_name(&self) -> String {
        if self.is_window() {
            "__window".to_string()
        } else {
            format!("__node_{}", self.0)
        }
    }

    pub fn from_canonical_name(name: &str) -> Option<Self> {
        if name == "__window" || name == "window" {
            Some(NodeId::WINDOW)
        } else {
            name.strip_prefix("__node_")
                .and_then(|num_str| num_str.parse::<usize>().ok())
                .map(NodeId)
        }
    }
}

/// An expanded layout element with concrete node identity, hierarchy, and port equations.
#[derive(Debug, Clone, PartialEq)]
pub struct ExpandedNode {
    pub id: NodeId,
    pub name: String,
    pub parent: Option<NodeId>,
    pub prev_sibling: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub ports: HashMap<String, Expr>,
    pub text_content: Option<String>,
    pub span: Span,
}

impl ExpandedNode {
    pub fn new(id: NodeId, name: impl Into<String>, span: Span) -> Self {
        Self {
            id,
            name: name.into(),
            parent: None,
            prev_sibling: None,
            children: Vec::new(),
            ports: HashMap::new(),
            text_content: None,
            span,
        }
    }

    pub fn is_paint_primitive(&self) -> bool {
        self.name == "Rect" || self.text_content.is_some()
    }
}

/// The collection of all expanded nodes in the document.
#[derive(Debug, Clone, PartialEq)]
pub struct ExpandedDocument {
    pub nodes: Vec<ExpandedNode>,
    pub roots: Vec<NodeId>,
}

impl ExpandedDocument {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            roots: Vec::new(),
        }
    }

    pub fn get_node(&self, id: NodeId) -> Option<&ExpandedNode> {
        self.nodes.get(id.0)
    }

    pub fn get_node_mut(&mut self, id: NodeId) -> Option<&mut ExpandedNode> {
        self.nodes.get_mut(id.0)
    }
}

impl Default for ExpandedDocument {
    fn default() -> Self {
        Self::new()
    }
}
