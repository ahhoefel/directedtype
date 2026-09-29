use std::collections::HashSet;

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

    /// Whether the spatial element picker cursor (`[↖]`) is actively picking elements.
    pub inspect_cursor_active: bool,

    /// Currently active inspection tab.
    pub active_tab: InspectorTab,

    /// Set of expanded parent node handles in the DOM tree view.
    pub expanded_nodes: HashSet<NodeHandle>,
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

    /// Toggles whether the inspect cursor is active.
    pub fn toggle_inspect_cursor(&mut self) -> bool {
        self.inspect_cursor_active = !self.inspect_cursor_active;
        self.inspect_cursor_active
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

    pub fn clear_selection(&mut self) {
        self.selected_node = None;
        self.hovered_node = None;
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
}
