use std::collections::HashSet;

use crate::compiler::expanded::NodeId;
use crate::compiler::layout::ResolvedLayout;
use crate::dom::NodeHandle;

/// Active tab in the DevTools inspector detail pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InspectorTab {
    #[default]
    Elements,
    BoxModel,
    PortEquations,
    Console,
}

/// Interactive state machine for the DevTools DOM Inspector.
#[derive(Debug, Clone, Default)]
pub struct InspectorState {
    /// Currently hovered node handle under the inspect cursor.
    pub hovered_node: Option<NodeHandle>,

    /// Currently selected / pinned node handle for inspection.
    pub selected_node: Option<NodeHandle>,

    /// Currently hovered layout NodeId.
    pub hovered_id: Option<NodeId>,

    /// Currently selected / pinned layout NodeId.
    pub selected_id: Option<NodeId>,

    /// Whether the spatial element picker cursor (`[↖]`) is actively picking elements.
    pub inspect_cursor_active: bool,

    /// Whether the cursor icon button in the toolbar is hovered by the mouse.
    pub inspect_cursor_hovered: bool,

    /// Currently active inspection tab.
    pub active_tab: InspectorTab,

    /// Set of expanded parent node handles in the DOM tree view.
    pub expanded_nodes: HashSet<NodeHandle>,

    /// Set of explicitly collapsed NodeIds in the tree view (nodes start expanded by default).
    pub collapsed_ids: HashSet<NodeId>,

    /// Vertical scroll offset (in logical pixels) inside the tree view.
    pub scroll_offset: f64,

    /// Vertical scroll offset (in logical pixels) inside the component details bottom panel.
    pub detail_scroll_offset: f64,

    /// Set of expanded reference properties in the component details bottom panel.
    /// Keyed by (referring_node_id, property_key).
    pub expanded_property_refs: HashSet<(NodeId, String)>,
}

impl InspectorState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the hovered node handle. Returns true if the hovered node changed.
    pub fn set_hovered(&mut self, handle: Option<NodeHandle>) -> bool {
        if self.hovered_node != handle {
            self.hovered_node = handle;
            true
        } else {
            false
        }
    }

    /// Sets the selected node handle. Returns true if the selected node changed.
    pub fn set_selected(&mut self, handle: Option<NodeHandle>) -> bool {
        if self.selected_node != handle {
            self.selected_node = handle;
            true
        } else {
            false
        }
    }

    /// Sets the hovered layout NodeId. Returns true if changed.
    pub fn set_hovered_id(&mut self, id: Option<NodeId>) -> bool {
        if self.hovered_id != id {
            self.hovered_id = id;
            true
        } else {
            false
        }
    }

    /// Sets the selected layout NodeId. Returns true if changed.
    pub fn set_selected_id(&mut self, id: Option<NodeId>) -> bool {
        if self.selected_id != id {
            self.selected_id = id;
            true
        } else {
            false
        }
    }

    /// Toggles whether the inspect cursor is active.
    pub fn toggle_inspect_cursor(&mut self) -> bool {
        self.inspect_cursor_active = !self.inspect_cursor_active;
        self.inspect_cursor_active
    }

    /// Sets whether the inspect cursor is active.
    pub fn set_inspect_cursor(&mut self, active: bool) {
        self.inspect_cursor_active = active;
    }

    /// Toggles the expanded/collapsed state of a node in the DOM tree view.
    pub fn toggle_expanded(&mut self, handle: NodeHandle) -> bool {
        if self.expanded_nodes.contains(&handle) {
            self.expanded_nodes.remove(&handle);
            false
        } else {
            self.expanded_nodes.insert(handle);
            true
        }
    }

    pub fn is_expanded(&self, handle: NodeHandle) -> bool {
        self.expanded_nodes.contains(&handle)
    }

    /// Toggles the expanded/collapsed state of a layout NodeId.
    pub fn toggle_expanded_id(&mut self, id: NodeId) -> bool {
        if self.collapsed_ids.contains(&id) {
            self.collapsed_ids.remove(&id);
            true
        } else {
            self.collapsed_ids.insert(id);
            false
        }
    }

    /// Returns true if a layout NodeId is expanded (default: expanded).
    pub fn is_expanded_id(&self, id: NodeId) -> bool {
        !self.collapsed_ids.contains(&id)
    }

    /// Expands all ancestor nodes of `id` in the layout tree so that `id` is visible.
    pub fn expand_ancestors(&mut self, id: NodeId, layout: &ResolvedLayout) {
        let mut curr = layout.get_node(id).and_then(|n| n.parent);
        while let Some(pid) = curr {
            self.collapsed_ids.remove(&pid);
            curr = layout.get_node(pid).and_then(|n| n.parent);
        }
    }

    /// Scrolls the tree view by `delta`, clamped between `0.0` and `max_scroll`.
    pub fn scroll_by(&mut self, delta: f64, max_scroll: f64) {
        self.scroll_offset = (self.scroll_offset + delta).clamp(0.0, max_scroll.max(0.0));
    }

    /// Scrolls the component details view by `delta`, clamped between `0.0` and `max_scroll`.
    pub fn scroll_detail_by(&mut self, delta: f64, max_scroll: f64) {
        self.detail_scroll_offset = (self.detail_scroll_offset + delta).clamp(0.0, max_scroll.max(0.0));
    }

    pub fn clear_selection(&mut self) {
        self.selected_node = None;
        self.hovered_node = None;
        self.selected_id = None;
        self.hovered_id = None;
    }

    /// Returns the node that should currently be visually highlighted on screen:
    /// prioritizes hovered node while picking, otherwise falls back to selected node.
    pub fn primary_highlight_target(&self) -> Option<(NodeHandle, bool)> {
        if let Some(h) = self.hovered_node {
            Some((h, self.selected_node == Some(h)))
        } else if let Some(s) = self.selected_node {
            Some((s, true))
        } else {
            None
        }
    }

    /// Returns whether an object reference property is expanded in the details panel.
    pub fn is_property_ref_expanded(&self, node_id: NodeId, prop_key: &str) -> bool {
        self.expanded_property_refs.contains(&(node_id, prop_key.to_string()))
    }

    /// Toggles the expansion state of an object reference property in the details panel.
    pub fn toggle_property_ref_expanded(&mut self, node_id: NodeId, prop_key: &str) {
        let key = (node_id, prop_key.to_string());
        if self.expanded_property_refs.contains(&key) {
            self.expanded_property_refs.remove(&key);
        } else {
            self.expanded_property_refs.insert(key);
        }
    }
}
