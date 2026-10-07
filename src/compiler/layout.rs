use crate::ast::{ComponentKey, Expr, Literal};
use crate::compiler::eval::eval_expr;
use crate::compiler::expanded::{ExpandedDocument, NodeId};
use crate::compiler::graph::VarId;
pub use crate::compiler::text::{CursorKind, SpanStyle, TextSpan};
use crate::compiler::scope::{ScopeId, ScopeTree};
use crate::compiler::value::Value;
use crate::dom::NodeHandle;
use crate::interaction::{HitTestResult, Point};
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

    pub fn contains_point(&self, point: Point) -> bool {
        self.contains(point.x, point.y)
    }

    pub fn contains(&self, px: f64, py: f64) -> bool {
        self.width > 0.0
            && self.height > 0.0
            && px >= self.x
            && px <= self.x + self.width
            && py >= self.y
            && py <= self.y + self.height
    }

    /// Computes the minimal bounding box enclosing all given rectangles.
    pub fn bounding_union(rects: &[Rect]) -> Option<Rect> {
        let first = rects.first()?;
        let mut min_x = first.x;
        let mut min_y = first.y;
        let mut max_x = first.x + first.width;
        let mut max_y = first.y + first.height;

        for r in &rects[1..] {
            if r.x < min_x {
                min_x = r.x;
            }
            if r.y < min_y {
                min_y = r.y;
            }
            let right = r.x + r.width;
            let bottom = r.y + r.height;
            if right > max_x {
                max_x = right;
            }
            if bottom > max_y {
                max_y = bottom;
            }
        }

        Some(Rect::new(min_x, min_y, max_x - min_x, max_y - min_y))
    }
}

/// Tests whether a point is inside a rectangle with an optional corner radius.
pub fn rounded_rect_contains(rect: &Rect, radius: f64, point: Point) -> bool {
    if !rect.contains_point(point) {
        return false;
    }
    if radius <= 0.0 {
        return true;
    }
    let r = radius.min(rect.width / 2.0).min(rect.height / 2.0);
    let px = point.x;
    let py = point.y;
    let x0 = rect.x;
    let y0 = rect.y;
    let x1 = rect.x + rect.width;
    let y1 = rect.y + rect.height;

    // Fast-path: if the point is within the inner cross bands, it is inside the rounded rect.
    if (px >= x0 + r && px <= x1 - r) || (py >= y0 + r && py <= y1 - r) {
        return true;
    }

    // Determine which corner quadrant the point is in:
    let (cx, cy) = match (px < x0 + r, py < y0 + r) {
        (true, true) => (x0 + r, y0 + r),       // Top-left
        (false, true) => (x1 - r, y0 + r),      // Top-right
        (true, false) => (x0 + r, y1 - r),      // Bottom-left
        (false, false) => (x1 - r, y1 - r),    // Bottom-right
    };

    let dx = px - cx;
    let dy = py - cy;
    dx * dx + dy * dy <= r * r
}

/// A resolved visual element with concrete spatial boundaries and computed properties.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedNode {
    pub id: NodeId,
    pub name: String,
    pub key: Option<ComponentKey>,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub rect: Rect,
    pub z: f64,
    pub clip: Option<NodeId>,
    pub text_content: Option<String>,
    pub text_spans: Vec<TextSpan>,
    pub fragments: Vec<Rect>,
    pub anchor_name: Option<String>,
    pub scope_id: Option<ScopeId>,
    pub properties: HashMap<String, Value>,
    pub state_vars: HashMap<String, Option<String>>,
    pub event_handlers: HashMap<String, crate::component::EventHandlerBinding>,
    pub formulas: HashMap<String, String>,
    pub span: Span,
    pub handle: Option<NodeHandle>,
    pub font: Option<NodeId>,
    pub var_name: Option<String>,
}

impl ResolvedNode {
    pub fn is_paint_primitive(&self) -> bool {
        self.name == "Rect"
            || self.name == "Text"
            || self.name == "Link"
            || !self.fragments.is_empty()
            || self.text_content.is_some()
            || self.properties.contains_key("text")
            || self.properties.contains_key("content")
    }

    /// Returns the text span associated with a child node, if any.
    pub fn span_for_node(&self, child_id: NodeId) -> Option<&TextSpan> {
        self.text_spans.iter().find(|s| s.node_id == Some(child_id))
    }
}

/// The final computed layout produced by the DirectedType graph compiler.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedLayout {
    pub roots: Vec<NodeId>,
    pub nodes: Vec<ResolvedNode>,
    pub values: HashMap<VarId, Value>,
    pub scope_tree: ScopeTree,
}

impl ResolvedLayout {
    /// Resolves an anchor URL or path from a given source node.
    pub fn resolve_anchor(&self, from_node: NodeId, target_path: &str) -> Option<(NodeId, ScopeId)> {
        let from_scope = self
            .nodes
            .iter()
            .find(|n| n.id == from_node)
            .and_then(|n| n.scope_id)
            .or_else(|| self.scope_tree.node_to_scope.get(&from_node).copied())
            .unwrap_or(ScopeId::ROOT);
        self.scope_tree.resolve_anchor(from_scope, target_path)
    }
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

    /// Constructs the bubble path (from target up to the root window) for a given node.
    pub fn bubble_path_for_node(&self, target: NodeId) -> Vec<NodeId> {
        let mut bubble_path = Vec::new();
        bubble_path.push(target);

        let mut curr_parent = self.get_node(target).and_then(|n| n.parent);
        let mut visited = std::collections::HashSet::new();
        visited.insert(target);

        let max_depth = self.nodes.len() + 1;
        while let Some(parent_id) = curr_parent {
            if parent_id.is_window() || bubble_path.len() >= max_depth || !visited.insert(parent_id) {
                break;
            }
            bubble_path.push(parent_id);
            curr_parent = self.get_node(parent_id).and_then(|n| n.parent);
        }

        bubble_path
    }

    pub fn get_by_handle(&self, handle: NodeHandle) -> Option<&ResolvedNode> {
        self.nodes.iter().find(|n| n.handle == Some(handle))
    }

    pub fn find_by_key(&self, parent: Option<NodeId>, key: &ComponentKey) -> Option<&ResolvedNode> {
        self.nodes.iter().find(|n| {
            if let Some(expected_parent) = parent {
                if n.parent != Some(expected_parent) {
                    return false;
                }
            }
            n.key.as_ref() == Some(key)
        })
    }

    pub fn get_value(&self, node_id: NodeId, port: &str) -> Option<&Value> {
        self.values.get(&VarId::new(node_id, port)).or_else(|| {
            self.get_node(node_id).and_then(|n| {
                n.font.and_then(|fid| self.values.get(&VarId::new(fid, port)))
            })
        })
    }

    /// Validates whether a point is within all active clip boundaries for a given clip ID.
    pub fn clip_chain_contains(&self, clip_id: Option<NodeId>, point: Point) -> bool {
        let mut curr = clip_id;
        let mut visited = std::collections::HashSet::new();
        let mut depth = 0;

        while let Some(id) = curr {
            if id.is_window() || depth >= 64 || !visited.insert(id) {
                break;
            }
            depth += 1;

            if let Some(box_id) = self.get_value(id, "box").and_then(|v| v.as_node()) {
                if !box_id.is_window() {
                    let get_box_val = |port: &str| -> f64 {
                        self.get_value(box_id, port)
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0)
                    };
                    let x = get_box_val("x");
                    let y = get_box_val("y");
                    let w = get_box_val("width");
                    let h = get_box_val("height");
                    let radius = self
                        .get_value(box_id, "radius")
                        .or_else(|| self.get_value(box_id, "corner_radius"))
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);

                    let clip_rect = Rect::new(x, y, w, h);
                    if !rounded_rect_contains(&clip_rect, radius, point) {
                        return false;
                    }
                }
            }

            curr = self
                .get_value(id, "up")
                .and_then(|v| v.as_node())
                .filter(|up_id| !up_id.is_window());
        }

        true
    }

    /// Queries the topmost visual element at the given logical coordinate.
    ///
    /// Evaluates in Reverse Painter's Order (descending `z`, reverse AST order),
    /// strictly enforcing active hardware `\Clip` boundaries and corner radii.
    pub fn hit_test(&self, point: Point) -> Option<HitTestResult> {
        for node in self.render_order().into_iter().rev() {
            // Only visual paint primitives can be directly hit
            if !node.is_paint_primitive() {
                continue;
            }

            // Exclude primitives with non-positive dimensions
            if node.rect.width <= 0.0 || node.rect.height <= 0.0 {
                continue;
            }

            // Check if point is outside active clip boundary
            if !self.clip_chain_contains(node.clip, point) {
                continue;
            }

            // Check primitive geometry
            let radius = node
                .properties
                .get("radius")
                .or_else(|| node.properties.get("corner_radius"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);

            if !node.fragments.is_empty() {
                let hits_fragment = node.fragments.iter().any(|frag| {
                    point.x >= frag.x && point.x <= frag.x + frag.width
                        && point.y >= frag.y && point.y <= frag.y + frag.height
                });
                if !hits_fragment {
                    continue;
                }
            } else if !rounded_rect_contains(&node.rect, radius, point) {
                continue;
            }

            let bubble_path = self.bubble_path_for_node(node.id);

            let local_point = Point::new(point.x - node.rect.x, point.y - node.rect.y);

            return Some(HitTestResult {
                target: node.id,
                global_point: point,
                local_point,
                bubble_path,
            });
        }

        None
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

        if node.name == "Font" {
            if let Some(size) = node.properties.get("size").and_then(|v| v.as_f64()) {
                ports.push(format!("size: {}", format_num(size)));
            }
            if let Some(weight) = node.properties.get("weight").and_then(|v| v.as_f64()) {
                ports.push(format!("weight: {}", format_num(weight)));
            }
            if let Some(family) = node
                .properties
                .get("family")
                .or_else(|| node.properties.get("font"))
                .and_then(|v| v.as_str())
            {
                if !family.is_empty() {
                    ports.push(format!("family: \"{}\"", family));
                }
            }
            return ports.join(", ");
        }

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
            "x", "y", "width", "height", "z", "left", "top", "right", "bottom", "clip", "box", "up",
            "color", "bg_color", "radius", "border_width", "border_color",
            "size", "weight", "text_height", "font", "family",
            "cap_height", "x_height", "descent", "ascent", "line_height",
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
                    Value::Enum { enum_name, variant } => {
                        ports.push(format!("{}: {}.{}", key, enum_name, variant))
                    }
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

        let mut formulas = HashMap::new();
        for (port, expr) in &node.authored_ports {
            formulas.insert(port.clone(), expr.to_string());
        }

        let font = node.font.or_else(|| {
            values
                .get(&VarId::new(node.id, "font"))
                .and_then(|v| v.as_node())
        });

        let resolved_key = node.key.as_ref().map(|k| {
            let resolved_parts = k
                .parts
                .iter()
                .map(|part| {
                    if let Ok(val) = eval_expr(part, &values) {
                        match val {
                            Value::Number(n) => Expr::Literal(Literal::Number(n, part.span())),
                            Value::String(s) => Expr::Literal(Literal::String(s, part.span())),
                            Value::Bool(b) => Expr::Literal(Literal::Bool(b, part.span())),
                            Value::Color(c) => Expr::Literal(Literal::Color(c, part.span())),
                            Value::Enum { enum_name, variant } => {
                                Expr::Literal(Literal::Enum(enum_name, variant, part.span()))
                            }
                            _ => part.clone(),
                        }
                    } else {
                        part.clone()
                    }
                })
                .collect();
            ComponentKey::new(resolved_parts, k.span)
        });

        resolved_nodes.push(ResolvedNode {
            id: node.id,
            name: node.name.clone(),
            key: resolved_key,
            parent: node.parent,
            children: node.children.clone(),
            rect: Rect::new(x, y, width, height),
            z,
            clip,
            text_content: node.text_content.clone(),
            text_spans: node.text_spans.clone(),
            fragments: Vec::new(),
            anchor_name: node.anchor_name.clone(),
            scope_id: node.scope_id,
            properties,
            state_vars: node.state_vars.clone(),
            event_handlers: node.event_handlers.clone(),
            formulas,
            span: node.span,
            handle: node.handle,
            font,
            var_name: node.var_name.clone(),
        });
    }

    let mut layout = ResolvedLayout {
        roots: doc.roots.clone(),
        nodes: resolved_nodes,
        values,
        scope_tree: doc.scope_tree.clone(),
    };

    project_inline_fragments(&mut layout);

    layout
}

/// Updates an existing `ResolvedLayout` in-place using newly computed values for dirty/changed variables.
pub fn update_resolved_layout(
    layout: &mut ResolvedLayout,
    doc: &ExpandedDocument,
    changed_vars: &std::collections::HashSet<VarId>,
) {
    if changed_vars.is_empty() {
        return;
    }

    let affected_node_ids: std::collections::HashSet<NodeId> =
        changed_vars.iter().map(|v| v.node).collect();

    for node in &mut layout.nodes {
        if affected_node_ids.contains(&node.id) {
            let get_num = |port: &str| -> f64 {
                layout
                    .values
                    .get(&VarId::new(node.id, port))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0)
            };

            node.rect.x = get_num("x");
            node.rect.y = get_num("y");
            node.rect.width = get_num("width");
            node.rect.height = get_num("height");
            node.z = get_num("z");

            for var_id in changed_vars {
                if var_id.node == node.id {
                    if let Some(val) = layout.values.get(var_id) {
                        node.properties.insert(var_id.port.clone(), val.clone());
                    }
                }
            }

            node.clip = layout
                .values
                .get(&VarId::new(node.id, "clip"))
                .and_then(|v| v.as_node())
                .filter(|id| !id.is_window());
        }

        // Re-evaluate structured component keys if the node has a key
        if let Some(expanded_node) = doc.get_node(node.id) {
            if let Some(k) = &expanded_node.key {
                let resolved_parts = k
                    .parts
                    .iter()
                    .map(|part| {
                        if let Ok(val) = eval_expr(part, &layout.values) {
                            match val {
                                Value::Number(n) => Expr::Literal(Literal::Number(n, part.span())),
                                Value::String(s) => Expr::Literal(Literal::String(s, part.span())),
                                Value::Bool(b) => Expr::Literal(Literal::Bool(b, part.span())),
                                Value::Color(c) => Expr::Literal(Literal::Color(c, part.span())),
                                Value::Enum { enum_name, variant } => {
                                    Expr::Literal(Literal::Enum(enum_name, variant, part.span()))
                                }
                                _ => part.clone(),
                            }
                        } else {
                            part.clone()
                        }
                    })
                    .collect();
                node.key = Some(ComponentKey::new(resolved_parts, k.span));
            }
        }
    }

    project_inline_fragments(layout);
}

/// Projects Parley line fragments onto inline child nodes within rich text blocks.
pub fn project_inline_fragments(layout: &mut ResolvedLayout) {
    let text_nodes: Vec<(
        NodeId,
        String,
        f64,
        f64,
        Option<String>,
        Option<f64>,
        Option<String>,
        Vec<TextSpan>,
        f64,
        f64,
    )> = layout
        .nodes
        .iter()
        .filter(|n| {
            n.name == "Text"
                && n.text_content.is_some()
                && n.text_spans.iter().any(|s| s.node_id.is_some())
        })
        .map(|n| {
            let text = n.text_content.clone().unwrap_or_default();
            let font_size = n
                .properties
                .get("size")
                .or_else(|| n.properties.get("font_size"))
                .and_then(|v| v.as_f64())
                .unwrap_or(16.0);
            let font_weight = n
                .properties
                .get("weight")
                .or_else(|| n.properties.get("font_weight"))
                .and_then(|v| v.as_f64())
                .unwrap_or(400.0);
            let font_family = n
                .properties
                .get("family")
                .or_else(|| n.properties.get("font_family"))
                .and_then(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    _ => None,
                });
            let max_width = if n.rect.width > 0.0 {
                Some(n.rect.width)
            } else {
                None
            };
            let align = n
                .properties
                .get("align")
                .or_else(|| n.properties.get("text_align"))
                .and_then(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    _ => None,
                });
            (
                n.id,
                text,
                font_size,
                font_weight,
                font_family,
                max_width,
                align,
                n.text_spans.clone(),
                n.rect.x,
                n.rect.y,
            )
        })
        .collect();

    for (_text_id, text, size, weight, family, max_w, align, spans, ox, oy) in text_nodes {
        let child_frags = crate::compiler::text::compute_span_fragments(
            &text,
            size,
            weight,
            family.as_deref(),
            max_w,
            align.as_deref(),
            &spans,
            ox,
            oy,
        );

        for (child_id, fragments) in child_frags {
            if let Some(union_rect) = Rect::bounding_union(&fragments) {
                let span_style = spans.iter().find(|s| s.node_id == Some(child_id)).map(|s| &s.style);

                if let Some(child_node) = layout.nodes.iter_mut().find(|n| n.id == child_id) {
                    child_node.fragments = fragments;
                    child_node.rect = union_rect;

                    child_node.properties.insert("x".to_string(), Value::Number(union_rect.x));
                    child_node.properties.insert("y".to_string(), Value::Number(union_rect.y));
                    child_node.properties.insert("width".to_string(), Value::Number(union_rect.width));
                    child_node.properties.insert("height".to_string(), Value::Number(union_rect.height));

                    if let Some(style) = span_style {
                        if let Some(ref u) = style.url {
                            child_node.properties.insert("url".to_string(), Value::String(u.clone()));
                            layout.values.insert(VarId::new(child_id, "url"), Value::String(u.clone()));
                        }
                        if let Some(ref c) = style.cursor {
                            let c_str = match c {
                                CursorKind::Pointer => "Pointer",
                                CursorKind::Default => "Default",
                                CursorKind::Text => "Text",
                            };
                            child_node.properties.insert("cursor".to_string(), Value::String(c_str.to_string()));
                        }
                    }

                    layout.values.insert(VarId::new(child_id, "x"), Value::Number(union_rect.x));
                    layout.values.insert(VarId::new(child_id, "y"), Value::Number(union_rect.y));
                    layout.values.insert(VarId::new(child_id, "width"), Value::Number(union_rect.width));
                    layout.values.insert(VarId::new(child_id, "height"), Value::Number(union_rect.height));
                }
            }
        }
    }
}
