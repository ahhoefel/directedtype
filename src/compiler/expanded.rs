use crate::ast::{ComponentKey, Expr};
use crate::dom::NodeHandle;
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
    pub key: Option<ComponentKey>,
    pub parent: Option<NodeId>,
    pub prev_sibling: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub ports: HashMap<String, Expr>,
    pub authored_ports: HashMap<String, Expr>,
    pub text_content: Option<String>,
    pub span: Span,
    pub handle: Option<NodeHandle>,
    pub font: Option<NodeId>,
    pub var_name: Option<String>,
}

impl ExpandedNode {
    pub fn new(id: NodeId, name: impl Into<String>, span: Span) -> Self {
        Self {
            id,
            name: name.into(),
            key: None,
            parent: None,
            prev_sibling: None,
            children: Vec::new(),
            ports: HashMap::new(),
            authored_ports: HashMap::new(),
            text_content: None,
            span,
            handle: None,
            font: None,
            var_name: None,
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
    pub window_ports: HashMap<String, Expr>,
}

impl ExpandedDocument {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            roots: Vec::new(),
            window_ports: HashMap::new(),
        }
    }

    pub fn get_node(&self, id: NodeId) -> Option<&ExpandedNode> {
        self.nodes.get(id.0)
    }

    pub fn get_node_mut(&mut self, id: NodeId) -> Option<&mut ExpandedNode> {
        self.nodes.get_mut(id.0)
    }

    pub fn find_by_key(&self, parent: Option<NodeId>, key: &ComponentKey) -> Option<&ExpandedNode> {
        self.nodes.iter().find(|n| {
            if let Some(expected_parent) = parent {
                if n.parent != Some(expected_parent) {
                    return false;
                }
            }
            n.key.as_ref() == Some(key)
        })
    }
}

impl Default for ExpandedDocument {
    fn default() -> Self {
        Self::new()
    }
}
