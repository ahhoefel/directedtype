use crate::ast::{ComponentKey, ContentItem, Expr, Ident, TypeRef};
use crate::dom::handle::NodeHandle;
use crate::span::Span;
use std::collections::HashMap;

/// An ordered, lossless content entry inside a `DomNode`'s `{ ... }` slot.
#[derive(Debug, Clone, PartialEq)]
pub enum DomContentItem {
    Text(String),
    ParagraphBreak,
    Child(NodeHandle),
    Directive(ContentItem),
}

/// An authoring-level node in the Component DOM.
#[derive(Debug, Clone, PartialEq)]
pub struct DomNode {
    /// Opaque generational handle for this node.
    pub handle: NodeHandle,

    /// Component tag or primitive name (e.g. "Rect", "Text", "ScrollView", "Flow").
    pub tag: String,

    /// Structured identity key (e.g. `(row, col)` or `"submit_btn"`).
    pub key: Option<ComponentKey>,

    /// Enclosing parent node, if attached.
    pub parent: Option<NodeHandle>,

    /// Ordered sequence of child node handles.
    pub children: Vec<NodeHandle>,

    /// Ordered lossless content sequence (interleaved text, paragraph breaks, children, directives).
    pub content_items: Vec<DomContentItem>,

    /// Preserved `\let name [: Type] = \Elem(...)` wrapper when this node was declared via a `\let` item.
    pub root_let_binding: Option<(Ident, Option<TypeRef>, Span)>,

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
            key: None,
            parent: None,
            children: Vec::new(),
            content_items: Vec::new(),
            root_let_binding: None,
            ports: HashMap::new(),
            text_content: None,
            span,
        }
    }
}
