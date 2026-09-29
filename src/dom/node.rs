use crate::ast::Expr;
use crate::dom::handle::NodeHandle;
use crate::span::Span;
use std::collections::HashMap;

/// An authoring-level node in the Component DOM.
#[derive(Debug, Clone, PartialEq)]
pub struct DomNode {
    /// Opaque generational handle for this node.
    pub handle: NodeHandle,

    /// Component tag or primitive name (e.g. "Rect", "Text", "ScrollView", "Flow").
    pub tag: String,

    /// Enclosing parent node, if attached.
    pub parent: Option<NodeHandle>,

    /// Ordered sequence of child node handles.
    pub children: Vec<NodeHandle>,

    /// Declared port equations (e.g. `width: 320`, `color: #6366f1`).
    pub ports: HashMap<String, Expr>,

    /// Explicit text content if this element represents a text leaf.
    pub text_content: Option<String>,

    /// Source span for diagnostic reporting.
    pub span: Span,
}

impl DomNode {
    pub fn new(handle: NodeHandle, tag: impl Into<String>, span: Span) -> Self {
        Self {
            handle,
            tag: tag.into(),
            parent: None,
            children: Vec::new(),
            ports: HashMap::new(),
            text_content: None,
            span,
        }
    }
}
