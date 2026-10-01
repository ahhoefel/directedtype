use crate::ast::Expr;
use crate::compiler::expanded::NodeId;
use crate::compiler::layout::ResolvedLayout;
use crate::dom::{Dom, NodeHandle};
use crate::inspector::model::{BoxModelValues, PortEquationEntry};
use crate::inspector::state::InspectorState;

/// A formatted row in the interactive Component DOM tree view.
#[derive(Debug, Clone, PartialEq)]
pub struct DomTreeItem {
    pub node_id: Option<NodeId>,
    pub handle: NodeHandle,
    pub depth: usize,
    pub tag: String,
    pub id_name: Option<String>,
    pub has_children: bool,
    pub is_expanded: bool,
    pub is_selected: bool,
    pub is_hovered: bool,
    pub port_summary: String,
    pub bounds_summary: String,
    pub var_name: Option<String>,
}

impl DomTreeItem {
    /// Returns true if this element represents a paint primitive rather than an authored component.
    pub fn is_primitive(&self) -> bool {
        self.tag == "Rect" || self.tag == "Text" || self.tag == "Font" || self.tag == "Clip" || self.tag == "Box"
    }

    /// Formats the tree item display line (e.g. `"▼ \ScrollView#main (height: 200) [380 × 180]"`).
    pub fn display_text(&self) -> String {
        let indent = "  ".repeat(self.depth);
        let chevron = if self.has_children {
            if self.is_expanded { "▼ " } else { "► " }
        } else {
            "  "
        };
        let mut line = if let Some(var) = &self.var_name {
            format!("{indent}{chevron}{var}: \\{}", self.tag)
        } else {
            format!("{indent}{chevron}\\{}", self.tag)
        };
        if let Some(id) = &self.id_name {
            line.push('#');
            line.push_str(id);
        }
        if !self.port_summary.is_empty() {
            line.push(' ');
            line.push_str(&self.port_summary);
        }
        line
    }
}

/// Recursively traverses the Component DOM to build the visible tree rows.
pub fn build_tree_items(dom: &Dom, state: &InspectorState) -> Vec<DomTreeItem> {
    let mut items = Vec::new();
    for &root in dom.roots() {
        collect_tree_items(dom, root, 0, state, &mut items);
    }
    items
}

fn collect_tree_items(
    dom: &Dom,
    handle: NodeHandle,
    depth: usize,
    state: &InspectorState,
    out: &mut Vec<DomTreeItem>,
) {
    let tag = dom.tag_name(handle).unwrap_or("Node").to_string();
    let declared = dom.declared_ports(handle).ok();

    let id_name = declared.as_ref().and_then(|p| {
        p.get("id").and_then(|e| match e {
            Expr::Ident(id) => Some(id.as_str().to_string()),
            Expr::Literal(crate::ast::Literal::String(s, _)) => Some(s.clone()),
            _ => None,
        })
    });

    let mut summary_parts = Vec::new();
    if let Some(ports) = declared {
        for (k, v) in ports {
            if k != "id" {
                match v {
                    Expr::Literal(crate::ast::Literal::Number(n, _)) => {
                        summary_parts.push(format!("{k}: {n}"));
                    }
                    Expr::Literal(crate::ast::Literal::String(s, _)) => {
                        summary_parts.push(format!("{k}: \"{s}\""));
                    }
                    _ => {}
                }
            }
        }
    }
    let port_summary = if summary_parts.is_empty() {
        String::new()
    } else {
        format!("({})", summary_parts.join(", "))
    };

    let bounds_summary = dom
        .computed_rect(handle)
        .map(|r| format!("[{} × {}]", r.width.round() as i64, r.height.round() as i64))
        .unwrap_or_default();

    let children = dom.children(handle).unwrap_or(&[]);
    let has_children = !children.is_empty();
    let is_expanded = state.is_expanded(handle) || !state.expanded_nodes.contains(&handle);
    let is_selected = state.selected_node == Some(handle);
    let is_hovered = state.hovered_node == Some(handle);

    let node_id = dom.node_handle_to_id(handle);

    out.push(DomTreeItem {
        node_id,
        handle,
        depth,
        tag,
        id_name,
        has_children,
        is_expanded,
        is_selected,
        is_hovered,
        port_summary,
        bounds_summary,
        var_name: None,
    });

    if has_children && is_expanded {
        for &child in children {
            collect_tree_items(dom, child, depth + 1, state, out);
        }
    }
}

/// Traverses a ResolvedLayout tree to produce the flattened list of visible tree rows.
pub fn build_tree_items_from_layout(
    layout: &ResolvedLayout,
    state: &InspectorState,
) -> Vec<DomTreeItem> {
    let mut items = Vec::new();
    for &root_id in &layout.roots {
        collect_layout_tree_items(layout, root_id, 0, state, &mut items);
    }
    items
}

fn collect_layout_tree_items(
    layout: &ResolvedLayout,
    node_id: NodeId,
    depth: usize,
    state: &InspectorState,
    out: &mut Vec<DomTreeItem>,
) {
    let node = match layout.get_node(node_id) {
        Some(n) => n,
        None => return,
    };

    let tag = node.name.clone();
    let id_name = node
        .properties
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let bounds_summary = format!(
        "[{} × {}]",
        node.rect.width.round() as i64,
        node.rect.height.round() as i64
    );

    let mut port_summary = String::new();
    if let Some(text) = &node.text_content {
        let snippet: String = text.chars().take(24).collect();
        port_summary = if text.chars().count() > 24 {
            format!("\"{snippet}...\"")
        } else {
            format!("\"{snippet}\"")
        };
    }

    let has_children = !node.children.is_empty();
    let is_expanded = state.is_expanded_id(node_id);
    let is_selected = state.selected_id == Some(node_id)
        || (node.handle.is_some() && state.selected_node == node.handle);
    let is_hovered = state.hovered_id == Some(node_id)
        || (node.handle.is_some() && state.hovered_node == node.handle);

    out.push(DomTreeItem {
        node_id: Some(node_id),
        handle: node.handle.unwrap_or_else(|| NodeHandle::new(0, 0)),
        depth,
        tag,
        id_name,
        has_children,
        is_expanded,
        is_selected,
        is_hovered,
        port_summary,
        bounds_summary,
        var_name: node.var_name.clone(),
    });

    if has_children && is_expanded {
        for &child_id in &node.children {
            collect_layout_tree_items(layout, child_id, depth + 1, state, out);
        }
    }
}

/// Gathers the declared algebraic equations and their solved values for `handle`.
pub fn extract_port_equations(dom: &Dom, handle: NodeHandle) -> Vec<PortEquationEntry> {
    let mut entries = Vec::new();
    if let Ok(declared) = dom.declared_ports(handle) {
        let mut sorted_keys: Vec<_> = declared.keys().collect();
        sorted_keys.sort();

        for key in sorted_keys {
            let expr = &declared[key];
            let authored = format!("{expr:?}");
            let evaluated = dom.computed_value(handle, key).map(|v| format!("{v}"));

            entries.push(PortEquationEntry {
                name: key.clone(),
                authored_equation: authored,
                evaluated_value: evaluated,
            });
        }
    }
    entries
}

/// Gathers the box model dimensions for `handle`.
pub fn extract_box_model(dom: &Dom, handle: NodeHandle) -> Option<BoxModelValues> {
    let rect = dom.computed_rect(handle)?;
    let z = dom
        .node_handle_to_id(handle)
        .and_then(|nid| dom.layout()?.get_node(nid))
        .map(|n| n.z)
        .unwrap_or(0.0);

    let clip_context = dom.clip_context(handle).map(|ch| {
        format!("{:?}", ch)
    });

    Some(BoxModelValues {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        z,
        clip_context,
    })
}
