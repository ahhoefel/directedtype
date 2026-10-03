use crate::ast::{ComponentKey, Expr};
use crate::compiler::expanded::NodeId;
use crate::compiler::layout::{Rect, ResolvedLayout};
use crate::compiler::value::Value;
use crate::dom::{Dom, NodeHandle};

/// Complete inspection data extracted for a target node in the Component DOM.
#[derive(Debug, Clone, PartialEq)]
pub struct InspectTargetInfo {
    pub handle: Option<NodeHandle>,
    pub node_id: Option<NodeId>,
    pub tag: String,
    pub id_name: Option<String>,
    pub key: Option<ComponentKey>,
    pub rect: Rect,
    pub z: f64,
    pub clip_handle: Option<NodeHandle>,
    pub clip_rect: Option<Rect>,
    pub declared_ports: Vec<(String, Expr)>,
    pub computed_values: Vec<(String, Value)>,
    pub state_vars: Vec<(String, Option<String>, Option<Value>)>,
}

impl InspectTargetInfo {
    /// Extracts target info from a stateful `Dom` by `NodeHandle`.
    pub fn from_dom(dom: &Dom, handle: NodeHandle) -> Option<Self> {
        let rect = dom.computed_rect(handle)?;
        let tag = dom.tag_name(handle).unwrap_or("Node").to_string();
        let declared = dom.declared_ports(handle).ok().cloned().unwrap_or_default();

        let id_name = declared.get("id").and_then(|expr| match expr {
            Expr::Ident(id) => Some(id.as_str().to_string()),
            Expr::Literal(crate::ast::Literal::String(s, _)) => Some(s.clone()),
            _ => None,
        });

        let key = dom.node_key(handle).cloned();

        let mut declared_ports: Vec<(String, Expr)> = declared.into_iter().collect();
        declared_ports.sort_by(|a, b| a.0.cmp(&b.0));

        let node_id = dom.node_handle_to_id(handle);
        let mut computed_values = Vec::new();
        let mut state_vars = Vec::new();
        let mut z = 0.0;
        let mut clip_handle = None;
        let mut clip_rect = None;

        if let Some(nid) = node_id {
            if let Some(layout) = dom.layout() {
                if let Some(resolved) = layout.get_node(nid) {
                    z = resolved.z;
                    for (k, v) in &resolved.properties {
                        computed_values.push((k.clone(), v.clone()));
                    }
                    computed_values.sort_by(|a, b| a.0.cmp(&b.0));

                    let mut sorted_states: Vec<_> = resolved.state_vars.keys().collect();
                    sorted_states.sort();
                    for &s_name in &sorted_states {
                        let type_annot = resolved.state_vars.get(s_name).cloned().flatten();
                        let val = dom
                            .get_state(handle, s_name.as_str())
                            .cloned()
                            .or_else(|| layout.get_value(nid, s_name.as_str()).cloned());
                        state_vars.push((s_name.clone(), type_annot, val));
                    }

                    if let Some(clip_id) = resolved.clip {
                        clip_handle = dom.node_id_to_handle(clip_id);
                        clip_rect = layout.get_node(clip_id).map(|n| n.rect);
                    }
                }
            }
        }

        Some(Self {
            handle: Some(handle),
            node_id,
            tag,
            id_name,
            key,
            rect,
            z,
            clip_handle,
            clip_rect,
            declared_ports,
            computed_values,
            state_vars,
        })
    }

    /// Extracts target info from a `ResolvedLayout` by internal `NodeId`.
    pub fn from_layout(layout: &ResolvedLayout, id: NodeId) -> Option<Self> {
        let node = layout.get_node(id)?;
        let mut computed_values: Vec<(String, Value)> = node
            .properties
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        computed_values.sort_by(|a, b| a.0.cmp(&b.0));

        let id_name = node.properties.get("id").and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            _ => None,
        });

        let key = node.key.clone();

        let mut state_vars = Vec::new();
        let mut sorted_states: Vec<_> = node.state_vars.keys().collect();
        sorted_states.sort();
        for &s_name in &sorted_states {
            let type_annot = node.state_vars.get(s_name).cloned().flatten();
            let val = layout.get_value(id, s_name.as_str()).cloned();
            state_vars.push((s_name.clone(), type_annot, val));
        }

        let clip_rect = node.clip.and_then(|cid| layout.get_node(cid)).map(|n| n.rect);

        Some(Self {
            handle: node.handle,
            node_id: Some(id),
            tag: node.name.clone(),
            id_name,
            key,
            rect: node.rect,
            z: node.z,
            clip_handle: None,
            clip_rect,
            declared_ports: Vec::new(),
            computed_values,
            state_vars,
        })
    }

    /// Formats the short header badge label, e.g. `"Rect#card [380 × 180]"` or `"Cell#(0, 0) [80 × 32]"`.
    pub fn badge_label(&self) -> String {
        let mut s = self.tag.clone();
        if let Some(key) = &self.key {
            s.push('#');
            s.push_str(&key.format_key());
        } else if let Some(id) = &self.id_name {
            s.push('#');
            s.push_str(id);
        }
        s.push_str(&format!(" [{} × {}]", self.rect.width.round() as i64, self.rect.height.round() as i64));
        s
    }

    /// Computes the bounding rectangle for the floating badge pill above (or below) the element.
    pub fn badge_rect(&self, badge_width: f64, badge_height: f64, window_width: f64, window_height: f64) -> Rect {
        let padding = 4.0;
        let mut x = self.rect.x;
        // Clamp horizontal position so badge doesn't clip off window edges
        if x + badge_width > window_width - padding {
            x = (window_width - badge_width - padding).max(padding);
        }
        if x < padding {
            x = padding;
        }

        // Place above target if room; otherwise flip below
        let y = if self.rect.y >= badge_height + padding + 2.0 {
            self.rect.y - badge_height - 4.0
        } else {
            (self.rect.y + self.rect.height + 4.0).min(window_height - badge_height - padding)
        };

        Rect::new(x, y, badge_width, badge_height)
    }
}

/// Evaluated box model boundary values.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxModelValues {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub z: f64,
    pub clip_context: Option<String>,
}

/// Formatted algebraic port equation entry.
#[derive(Debug, Clone, PartialEq)]
pub struct PortEquationEntry {
    pub name: String,
    pub authored_equation: String,
    pub evaluated_value: Option<String>,
}
