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
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub rect: Rect,
    pub z: f64,
    pub clip: Option<NodeId>,
    pub text_content: Option<String>,
    pub properties: HashMap<String, Value>,
    pub span: Span,
}

impl ResolvedNode {
    pub fn is_paint_primitive(&self) -> bool {
        self.name == "Rect" || self.text_content.is_some()
    }
}

/// The final computed layout produced by the DirectedType graph compiler.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedLayout {
    pub roots: Vec<NodeId>,
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

    /// Formats the resolved layout hierarchy into DTML syntax: `\Name(ports) { body }`.
    pub fn format_dom(&self) -> String {
        let mut out = String::new();
        let mut visited = std::collections::HashSet::new();

        for &root_id in &self.roots {
            self.format_node_dtml(root_id, 0, &mut visited, &mut out);
        }

        // Output any unvisited / detached nodes (e.g. from local let bindings)
        let unvisited: Vec<&ResolvedNode> = self
            .nodes
            .iter()
            .filter(|n| !visited.contains(&n.id))
            .collect();

        if !unvisited.is_empty() {
            out.push_str("\n// Detached nodes\n");
            for node in unvisited {
                self.format_node_dtml(node.id, 0, &mut visited, &mut out);
            }
        }

        out
    }

    fn format_node_dtml(
        &self,
        node_id: NodeId,
        depth: usize,
        visited: &mut std::collections::HashSet<NodeId>,
        out: &mut String,
    ) {
        if !visited.insert(node_id) {
            return;
        }

        let node = match self.get_node(node_id) {
            Some(n) => n,
            None => return,
        };

        let indent = "    ".repeat(depth);
        let ports_str = self.format_node_ports(node);

        let has_child_components = !node.children.is_empty();
        let text_trimmed = node.text_content.as_deref().map(str::trim).filter(|s| !s.is_empty());

        if has_child_components {
            // Line break after the brace and before the body if the body contains components
            out.push_str(&format!("{}\\{}({}) {{\n", indent, node.name, ports_str));

            if let Some(text) = text_trimmed {
                out.push_str(&format!("{}    {}\n", indent, text));
            }

            for &child_id in &node.children {
                self.format_node_dtml(child_id, depth + 1, visited, out);
            }

            out.push_str(&format!("{}}}\n", indent));
        } else if let Some(text) = text_trimmed {
            // Body contains only text (no components)
            out.push_str(&format!("{}\\{}({}) {{ {} }}\n", indent, node.name, ports_str, text));
        } else {
            // No body
            out.push_str(&format!("{}\\{}({})\n", indent, node.name, ports_str));
        }
    }

    fn format_node_ports(&self, node: &ResolvedNode) -> String {
        let mut ports = Vec::new();

        if node.name == "Clip" {
            if let Some(box_val) = node.properties.get("box").and_then(|v| v.as_node()) {
                let box_str = if box_val.is_window() {
                    "window".to_string()
                } else {
                    format!("__node_{}", box_val.0)
                };
                ports.push(format!("box: {}", box_str));
            }
            if let Some(up_val) = node.properties.get("up").and_then(|v| v.as_node()) {
                let up_str = if up_val.is_window() {
                    "window.clip".to_string()
                } else {
                    format!("__node_{}.clip", up_val.0)
                };
                ports.push(format!("up: {}", up_str));
            }
        } else {
            ports.push(format!("x: {}", format_num(node.rect.x)));
            ports.push(format!("y: {}", format_num(node.rect.y)));
            ports.push(format!("width: {}", format_num(node.rect.width)));
            ports.push(format!("height: {}", format_num(node.rect.height)));
        }

        if node.z != 0.0 {
            ports.push(format!("z: {}", format_num(node.z)));
        }

        if let Some(clip_id) = node.clip {
            if clip_id.is_window() {
                ports.push("clip: window.clip".to_string());
            } else {
                ports.push(format!("clip: __node_{}", clip_id.0));
            }
        }

        if let Some(Value::Color(c) | Value::String(c)) = node
            .properties
            .get("color")
            .or_else(|| node.properties.get("bg_color"))
        {
            ports.push(format!("color: {}", c));
        }

        if let Some(radius) = node.properties.get("radius").and_then(|v| v.as_f64()) {
            if radius > 0.0 {
                ports.push(format!("radius: {}", format_num(radius)));
            }
        }

        if let Some(border_w) = node.properties.get("border_width").and_then(|v| v.as_f64()) {
            if border_w > 0.0 {
                ports.push(format!("border_width: {}", format_num(border_w)));
                if let Some(Value::Color(c) | Value::String(c)) = node.properties.get("border_color") {
                    ports.push(format!("border_color: {}", c));
                }
            }
        }

        if let Some(size) = node.properties.get("size").and_then(|v| v.as_f64()) {
            ports.push(format!("size: {}", format_num(size)));
        }

        if let Some(weight) = node.properties.get("weight").and_then(|v| v.as_f64()) {
            ports.push(format!("weight: {}", format_num(weight)));
        }

        let standard_keys = [
            "x", "y", "width", "height", "z", "clip", "box", "up",
            "color", "bg_color", "radius", "border_width", "border_color",
            "size", "weight", "text_height",
        ];
        let mut extra_keys: Vec<&String> = node
            .properties
            .keys()
            .filter(|k| !standard_keys.contains(&k.as_str()) && !k.starts_with("__"))
            .collect();
        extra_keys.sort();

        for key in extra_keys {
            if let Some(val) = node.properties.get(key) {
                match val {
                    Value::Number(n) => ports.push(format!("{}: {}", key, format_num(*n))),
                    Value::Color(c) | Value::String(c) => ports.push(format!("{}: {}", key, c)),
                    Value::Node(id) => {
                        if id.is_window() {
                            ports.push(format!("{}: window", key));
                        } else {
                            ports.push(format!("{}: __node_{}", key, id.0));
                        }
                    }
                    _ => {}
                }
            }
        }

        ports.join(", ")
    }

    /// Prints the formatted DOM tree directly to stdout.
    pub fn print_dom(&self) {
        println!("{}", self.format_dom());
    }
}

fn format_num(n: f64) -> String {
    if (n - n.round()).abs() < 1e-4 {
        format!("{:.0}", n.round())
    } else {
        let s = format!("{:.2}", n);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
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

        let clip = values
            .get(&VarId::new(node.id, "clip"))
            .and_then(|v| v.as_node())
            .filter(|id| !id.is_window());

        resolved_nodes.push(ResolvedNode {
            id: node.id,
            name: node.name.clone(),
            parent: node.parent,
            children: node.children.clone(),
            rect: Rect::new(x, y, width, height),
            z,
            clip,
            text_content: node.text_content.clone(),
            properties,
            span: node.span,
        });
    }

    ResolvedLayout {
        roots: doc.roots.clone(),
        nodes: resolved_nodes,
        values,
    }
}
