use parley::layout::PositionedLayoutItem;
use parley::style::{FontFamily, FontWeight, StyleProperty};
#[cfg(not(target_os = "macos"))]
use parley::style::GenericFamily;
use parley::{Alignment, FontContext, LayoutContext};
use vello::kurbo::{Affine, BezPath, Circle, Line, Rect, RoundedRect, Stroke};
use vello::peniko::{Brush, Color, Fill};
use vello::Scene;

use crate::compiler::expanded::NodeId;
use crate::compiler::layout::ResolvedLayout;
use crate::compiler::Value;
use crate::inspector::state::InspectorState;
use crate::inspector::view::DomTreeItem;

/// Results of a user mouse click within the inspector side panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelHitResult {
    /// Click fell outside interactive panel controls.
    None,
    /// Clicked the spatial element picker button (`[↖]`).
    ToggleInspectCursor,
    /// Clicked the expand/collapse chevron for a tree node.
    ToggleExpand(NodeId),
    /// Clicked a tree row to select a node.
    SelectNode(NodeId),
    /// Clicked an expandable object reference property row in details panel.
    TogglePropertyRef(NodeId, String),
    /// Clicked a property/port row to inspect its value origin through the DAG.
    SelectProperty(NodeId, String),
    /// Clicked back button in property details to return to component details.
    BackToComponentDetails,
}

/// Interactive scrollbar kinds within the inspector side panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectorScrollbarKind {
    TreeVertical,
    TreeHorizontal,
    DetailVertical,
    DetailHorizontal,
}

/// The docked side panel component rendering the Component DOM tree and selected component details.
#[derive(Debug, Clone)]
pub struct InspectPanelComponent {
    /// Width of the side panel in logical pixels (default: 380px).
    pub width: f64,
    /// Height of the top header toolbar (default: 36px).
    pub header_height: f64,
    /// Height of each tree item row (default: 18px).
    pub row_height: f64,
    /// Pixel indentation step per depth level (default: 14px).
    pub indent_step: f64,
    /// Height of the section divider between DOM tree and Component Details (default: 26px).
    pub divider_height: f64,
}

impl Default for InspectPanelComponent {
    fn default() -> Self {
        Self {
            width: 380.0,
            header_height: 36.0,
            row_height: 18.0,
            indent_step: 14.0,
            divider_height: 26.0,
        }
    }
}

impl InspectPanelComponent {
    /// Minimum width of the inspector panel in logical pixels.
    pub const MIN_WIDTH: f64 = 220.0;

    /// Minimum width reserved for the main document panel in logical pixels.
    pub const MIN_MAIN_PANEL_WIDTH: f64 = 100.0;

    /// Half-width of the draggable resize border hit zone around `panel_x` in logical pixels.
    pub const RESIZE_BORDER_HIT_HALF_WIDTH: f64 = 5.0;

    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if `(px, py)` (where `py` is relative to the top of the panel)
    /// falls within the draggable left border hit zone around `panel_x`.
    pub fn is_resize_border_hovered(&self, px: f64, py: f64, panel_x: f64, win_h: f64) -> bool {
        py >= 0.0 && py <= win_h && (px - panel_x).abs() <= Self::RESIZE_BORDER_HIT_HALF_WIDTH
    }

    /// Sets the panel width clamped to valid bounds for a window of width `win_w`.
    /// Returns `true` if the width changed.
    pub fn set_width_clamped(&mut self, new_width: f64, win_w: f64) -> bool {
        let max_width = (win_w - Self::MIN_MAIN_PANEL_WIDTH).max(Self::MIN_WIDTH);
        let clamped = new_width.clamp(Self::MIN_WIDTH, max_width);
        if (self.width - clamped).abs() > 0.01 {
            self.width = clamped;
            true
        } else {
            false
        }
    }

    /// Computes the Y coordinate where the top DOM tree ends and the section divider begins.
    pub fn divider_y(&self, win_h: f64) -> f64 {
        let content_h = (win_h - self.header_height - self.divider_height).max(100.0);
        self.header_height + (content_h * 0.44).clamp(100.0, 320.0)
    }

    /// Computes the maximum vertical scroll offset for `total_items` within the top DOM tree.
    pub fn max_scroll(&self, total_items: usize, win_h: f64) -> f64 {
        let content_h = total_items as f64 * self.row_height;
        let visible_h = self.divider_y(win_h) - self.header_height;
        (content_h - visible_h).max(0.0)
    }

    /// Computes the total content width of tree items.
    pub fn total_tree_width(&self, items: &[DomTreeItem]) -> f64 {
        let mut max_w = self.width;
        for item in items {
            let indent_x = 8.0 + item.depth as f64 * self.indent_step;
            let mut w = indent_x + 14.0;
            if let Some(var) = &item.var_name {
                w += var.len() as f64 * 7.5 + 16.0;
            }
            w += item.tag.len() as f64 * 8.0 + 8.0;
            if let Some(kid) = item.key.as_ref().map(|k| k.format_key()).or_else(|| item.id_name.clone()) {
                w += kid.len() as f64 * 7.5 + 8.0;
            }
            if !item.port_summary.is_empty() {
                w += item.port_summary.len() as f64 * 7.0 + 10.0;
            }
            w += 24.0;
            if w > max_w {
                max_w = w;
            }
        }
        max_w
    }

    /// Computes the maximum horizontal scroll offset for tree items.
    pub fn max_tree_scroll_x(&self, items: &[DomTreeItem]) -> f64 {
        let total_w = self.total_tree_width(items);
        (total_w - self.width).max(0.0)
    }

    /// Computes the total vertical content height of the bottom details panel.
    pub fn total_detail_content_height(
        &self,
        selected_id: Option<NodeId>,
        layout: &ResolvedLayout,
        state: Option<&InspectorState>,
    ) -> f64 {
        if let Some((sel_nid, ref sel_prop)) = state.and_then(|s| s.selected_property.as_ref()) {
            if let Some(trace) = crate::inspector::build_property_dag_trace(layout, *sel_nid, sel_prop) {
                let mut h = 10.0 + 48.0 + 14.0;
                h += 18.0 + trace.upstream_dependencies.len().max(1) as f64 * 18.0 + 10.0;
                h += 18.0 + trace.downstream_dependents.len().max(1) as f64 * 18.0 + 10.0;
                h += 50.0;
                return h;
            }
        }

        if let Some(node) = selected_id.and_then(|id| layout.get_node(id)) {
            let mut h = 10.0;
            if !node.state_vars.is_empty() {
                h += 18.0 + node.state_vars.len() as f64 * 24.0 + 10.0;
            }
            let mut prop_keys: Vec<String> = node.properties.keys().cloned().collect();
            prop_keys.retain(|k| k != "clip");
            for geom in ["x", "y", "width", "height", "z"] {
                if !prop_keys.contains(&geom.to_string()) {
                    prop_keys.push(geom.to_string());
                }
            }
            for key in &prop_keys {
                h += 18.0;

                if let Some(Value::Node(ref_id)) = node.properties.get(key) {
                    let is_expanded = state.is_some_and(|s| s.is_property_ref_expanded(node.id, key));
                    if is_expanded {
                        if let Some(rn) = layout.get_node(*ref_id) {
                            let mut child_keys: Vec<String> = rn.properties.keys().cloned().collect();
                            child_keys.retain(|k| k != "clip");
                            h += child_keys.len() as f64 * 17.0;
                        }
                    }
                }
            }
            h + 30.0
        } else {
            120.0
        }
    }

    /// Computes the maximum vertical scroll offset for the bottom details panel.
    pub fn max_detail_scroll(&self, selected_id: Option<NodeId>, layout: &ResolvedLayout, win_h: f64) -> f64 {
        self.max_detail_scroll_with_state(selected_id, layout, None, win_h)
    }

    /// Computes the maximum vertical scroll offset for the bottom details panel taking state into account.
    pub fn max_detail_scroll_with_state(
        &self,
        selected_id: Option<NodeId>,
        layout: &ResolvedLayout,
        state: Option<&InspectorState>,
        win_h: f64,
    ) -> f64 {
        let detail_y = self.divider_y(win_h);
        let detail_visible_h = (win_h - (detail_y + self.divider_height)).max(0.0);
        let content_h = self.total_detail_content_height(selected_id, layout, state);
        (content_h - detail_visible_h).max(0.0)
    }

    /// Computes the total content width of the bottom details panel.
    pub fn total_detail_content_width(
        &self,
        selected_id: Option<NodeId>,
        layout: &ResolvedLayout,
        state: Option<&InspectorState>,
    ) -> f64 {
        let mut max_w = self.width;

        if let Some((sel_nid, ref sel_prop)) = state.and_then(|s| s.selected_property.as_ref()) {
            if let Some(trace) = crate::inspector::build_property_dag_trace(layout, *sel_nid, sel_prop) {
                let line1_w = 24.0
                    + (trace.target_label.len() + 3 + trace.port_name.len() + 3 + trace.evaluated_value_str.len()) as f64 * 8.0
                    + 120.0;
                if line1_w > max_w {
                    max_w = line1_w;
                }
                for dep in &trace.upstream_dependencies {
                    let has_formula = !dep.equation_str.is_empty() && dep.equation_str != dep.value_str;
                    let mut w = 24.0 + (dep.node_label.len() + 1 + dep.port_name.len()) as f64 * 8.0 + dep.value_str.len() as f64 * 8.0 + 40.0;
                    if has_formula {
                        w += (4 + dep.equation_str.len()) as f64 * 7.5;
                    }
                    if w > max_w {
                        max_w = w;
                    }
                }
                for dep in &trace.downstream_dependents {
                    let has_formula = !dep.equation_str.is_empty() && dep.equation_str != dep.value_str;
                    let mut w = 24.0 + (dep.node_label.len() + 1 + dep.port_name.len()) as f64 * 8.0 + dep.value_str.len() as f64 * 8.0 + 40.0;
                    if has_formula {
                        w += (4 + dep.equation_str.len()) as f64 * 7.5;
                    }
                    if w > max_w {
                        max_w = w;
                    }
                }
                let eq_hero_w = 24.0 + trace.equation_str.len() as f64 * 8.0 + 40.0;
                if eq_hero_w > max_w {
                    max_w = eq_hero_w;
                }
                return max_w;
            }
        }

        if let Some(node) = selected_id.and_then(|id| layout.get_node(id)) {
            for (s_name, _) in &node.state_vars {
                let val_str = layout.get_value(node.id, s_name).map(|v| format!("{v}")).unwrap_or_default();
                let w = 24.0 + s_name.len() as f64 * 8.0 + val_str.len() as f64 * 8.0 + 50.0;
                if w > max_w {
                    max_w = w;
                }
            }
            let mut prop_keys: Vec<String> = node.properties.keys().cloned().collect();
            prop_keys.retain(|k| k != "clip");
            for geom in ["x", "y", "width", "height", "z"] {
                if !prop_keys.contains(&geom.to_string()) {
                    prop_keys.push(geom.to_string());
                }
            }
            for key in &prop_keys {
                let eval_str = if let Some(val) = node.properties.get(key) {
                    format!("{val}")
                } else {
                    String::new()
                };
                let formula_str = node.formulas.get(key).cloned().unwrap_or_default();
                let has_formula = !formula_str.is_empty() && formula_str != eval_str;
                let mut prop_w = 24.0 + key.len() as f64 * 8.0 + eval_str.len() as f64 * 8.0 + 40.0;
                if has_formula {
                    prop_w += (4 + formula_str.len()) as f64 * 7.5;
                }
                if prop_w > max_w {
                    max_w = prop_w;
                }

                if let Some(Value::Node(ref_id)) = node.properties.get(key) {
                    let is_expanded = state.is_some_and(|s| s.is_property_ref_expanded(node.id, key));
                    if is_expanded {
                        if let Some(rn) = layout.get_node(*ref_id) {
                            let mut child_keys: Vec<String> = rn.properties.keys().cloned().collect();
                            child_keys.retain(|k| k != "clip");
                            for ck in &child_keys {
                                let c_val = rn.properties.get(ck);
                                let c_eval = c_val.map(|v| format!("{v}")).unwrap_or_default();
                                let c_form = rn.formulas.get(ck).cloned().unwrap_or_else(|| c_eval.clone());
                                let c_has = !c_form.is_empty() && c_form != c_eval;
                                let mut c_w = 40.0 + ck.len() as f64 * 8.0 + c_eval.len() as f64 * 8.0 + 40.0;
                                if c_has {
                                    c_w += (4 + c_form.len()) as f64 * 7.5;
                                }
                                if c_w > max_w {
                                    max_w = c_w;
                                }
                            }
                        }
                    }
                }
            }
        }
        max_w
    }

    /// Computes the maximum horizontal scroll offset for the bottom details panel.
    pub fn max_detail_scroll_x(
        &self,
        selected_id: Option<NodeId>,
        layout: &ResolvedLayout,
        state: Option<&InspectorState>,
    ) -> f64 {
        let content_w = self.total_detail_content_width(selected_id, layout, state);
        (content_w - self.width).max(0.0)
    }

    /// Hit-tests scrollbar thumbs / tracks within the inspector panel.
    pub fn hit_test_scrollbar(
        &self,
        px: f64,
        py: f64,
        panel_x: f64,
        win_h: f64,
        items: &[DomTreeItem],
        state: &InspectorState,
        layout: &ResolvedLayout,
    ) -> Option<InspectorScrollbarKind> {
        let divider_y = self.divider_y(win_h);
        let panel_w = self.width;

        // Tree vertical scrollbar
        let total_tree_h = items.len() as f64 * self.row_height;
        let visible_tree_h = divider_y - self.header_height;
        if total_tree_h > visible_tree_h && visible_tree_h > 30.0 {
            if px >= panel_x + panel_w - 12.0 && px <= panel_x + panel_w && py >= self.header_height && py < divider_y {
                return Some(InspectorScrollbarKind::TreeVertical);
            }
        }

        // Tree horizontal scrollbar
        let total_tree_w = self.total_tree_width(items);
        if total_tree_w > panel_w && visible_tree_h > 30.0 {
            if py >= divider_y - 10.0 && py < divider_y && px >= panel_x && px <= panel_x + panel_w - 12.0 {
                return Some(InspectorScrollbarKind::TreeHorizontal);
            }
        }

        let detail_y = divider_y + self.divider_height;
        let detail_h = (win_h - detail_y).max(0.0);

        // Detail vertical scrollbar
        let total_detail_h = self.total_detail_content_height(state.selected_id, layout, Some(state));
        if total_detail_h > detail_h && detail_h > 30.0 {
            if px >= panel_x + panel_w - 12.0 && px <= panel_x + panel_w && py >= detail_y && py <= win_h {
                return Some(InspectorScrollbarKind::DetailVertical);
            }
        }

        // Detail horizontal scrollbar
        let total_detail_w = self.total_detail_content_width(state.selected_id, layout, Some(state));
        if total_detail_w > panel_w && detail_h > 30.0 {
            if py >= win_h - 10.0 && py <= win_h && px >= panel_x && px <= panel_x + panel_w - 12.0 {
                return Some(InspectorScrollbarKind::DetailHorizontal);
            }
        }

        None
    }

    /// Returns true if `(px, py)` is inside the header pick tool (`[↖]`) button.
    pub fn is_cursor_btn_hovered(&self, px: f64, py: f64, panel_x: f64) -> bool {
        px >= panel_x + 4.0 && px <= panel_x + 36.0 && py >= 2.0 && py <= self.header_height - 2.0
    }

    /// Determines the action triggered by clicking at `(px, py)` in logical window space.
    #[allow(clippy::too_many_arguments)]
    pub fn handle_click(
        &self,
        px: f64,
        py: f64,
        panel_x: f64,
        win_h: f64,
        items: &[DomTreeItem],
        state: &InspectorState,
        layout: Option<&ResolvedLayout>,
    ) -> PanelHitResult {
        if px < panel_x || px > panel_x + self.width {
            return PanelHitResult::None;
        }

        // Header toolbar clicks
        if py < self.header_height {
            if self.is_cursor_btn_hovered(px, py, panel_x) {
                return PanelHitResult::ToggleInspectCursor;
            }
            return PanelHitResult::None;
        }

        let divider_y = self.divider_y(win_h);
        // Only clicks within the DOM tree area interact with tree items
        if py < divider_y {
            let total_tree_h = items.len() as f64 * self.row_height;
            let visible_tree_h = divider_y - self.header_height;
            if total_tree_h > visible_tree_h && px >= panel_x + self.width - 12.0 {
                return PanelHitResult::None;
            }
            let total_tree_w = self.total_tree_width(items);
            if total_tree_w > self.width && py >= divider_y - 8.0 {
                return PanelHitResult::None;
            }

            let content_y = py - self.header_height + state.scroll_offset;
            if content_y < 0.0 {
                return PanelHitResult::None;
            }
            let row_idx = (content_y / self.row_height) as usize;
            if row_idx >= items.len() {
                return PanelHitResult::None;
            }

            let item = &items[row_idx];
            if let Some(node_id) = item.node_id {
                let indent_x = panel_x + 8.0 + item.depth as f64 * self.indent_step - state.scroll_offset_x;
                // If clicking directly on or near the chevron area
                if item.has_children && px >= indent_x - 4.0 && px <= indent_x + 18.0 {
                    PanelHitResult::ToggleExpand(node_id)
                } else {
                    PanelHitResult::SelectNode(node_id)
                }
            } else {
                PanelHitResult::None
            }
        } else {
            let detail_y = divider_y + self.divider_height;
            if py >= divider_y && py < detail_y {
                if state.selected_property.is_some()
                    && px >= panel_x + self.width - 34.0
                    && px <= panel_x + self.width - 6.0
                {
                    return PanelHitResult::BackToComponentDetails;
                }
                return PanelHitResult::None;
            }
            if py >= detail_y {
                if let Some(layout) = layout {
                    let total_detail_h = self.total_detail_content_height(state.selected_id, layout, Some(state));
                    let detail_h = (win_h - detail_y).max(0.0);
                    if total_detail_h > detail_h && px >= panel_x + self.width - 12.0 {
                        return PanelHitResult::None;
                    }
                    let total_detail_w = self.total_detail_content_width(state.selected_id, layout, Some(state));
                    if total_detail_w > self.width && py >= win_h - 8.0 {
                        return PanelHitResult::None;
                    }

                    if let Some((sel_nid, ref sel_prop)) = state.selected_property {
                        let base_y = detail_y - state.detail_scroll_offset;
                        let mut cur_y = base_y + 10.0;

                        if let Some(trace) = crate::inspector::build_property_dag_trace(layout, sel_nid, sel_prop) {
                            cur_y += 48.0 + 14.0; // port card

                            // Upstream dependencies
                            cur_y += 18.0; // header
                            if trace.upstream_dependencies.is_empty() {
                                cur_y += 18.0 + 10.0;
                            } else {
                                for dep in &trace.upstream_dependencies {
                                    let row_h = 18.0;
                                    if py >= cur_y && py < cur_y + row_h {
                                        return PanelHitResult::SelectProperty(dep.node_id, dep.port_name.clone());
                                    }
                                    cur_y += row_h;
                                }
                                cur_y += 10.0;
                            }

                            // Downstream dependents
                            cur_y += 18.0; // header
                            if trace.downstream_dependents.is_empty() {
                                // Leaf port
                            } else {
                                for dep in &trace.downstream_dependents {
                                    let row_h = 18.0;
                                    if py >= cur_y && py < cur_y + row_h {
                                        return PanelHitResult::SelectProperty(dep.node_id, dep.port_name.clone());
                                    }
                                    cur_y += row_h;
                                }
                            }
                        }

                        return PanelHitResult::None;
                    }

                    if let Some(node) = state.selected_id.and_then(|id| layout.get_node(id)) {
                        let base_y = detail_y - state.detail_scroll_offset;
                        let mut cur_y = base_y + 10.0;

                        if !node.state_vars.is_empty() {
                            cur_y += 18.0;
                            let mut state_names: Vec<String> = node.state_vars.keys().cloned().collect();
                            state_names.sort();
                            for s_name in &state_names {
                                let row_h = 24.0;
                                if py >= cur_y && py < cur_y + row_h {
                                    return PanelHitResult::SelectProperty(node.id, s_name.clone());
                                }
                                cur_y += row_h;
                            }
                            cur_y += 10.0;
                        }

                        let mut prop_keys: Vec<String> = node.properties.keys().cloned().collect();
                        prop_keys.retain(|k| k != "clip");
                        for geom in ["x", "y", "width", "height", "z"] {
                            if !prop_keys.contains(&geom.to_string()) {
                                prop_keys.push(geom.to_string());
                            }
                        }
                        prop_keys.sort_by(|a, b| {
                            let rank = |k: &str| match k {
                                "x" => 1,
                                "y" => 2,
                                "width" => 3,
                                "height" => 4,
                                "z" => 5,
                                "left" => 6,
                                "top" => 7,
                                "right" => 8,
                                "bottom" => 9,
                                "color" | "bg_color" => 10,
                                "border_color" | "border_width" => 11,
                                "radius" | "corner_radius" => 12,
                                "clip" => 13,
                                _ => 14,
                            };
                            rank(a).cmp(&rank(b)).then_with(|| a.cmp(b))
                        });

                        for key in &prop_keys {
                            let val = node.properties.get(key);
                            let is_node_ref = matches!(val, Some(Value::Node(_)));
                            let row_h = 18.0;

                            if is_node_ref {
                                if py >= cur_y && py < cur_y + row_h {
                                    return PanelHitResult::TogglePropertyRef(node.id, key.clone());
                                }
                            } else {
                                if py >= cur_y && py < cur_y + row_h {
                                    return PanelHitResult::SelectProperty(node.id, key.clone());
                                }
                            }

                            cur_y += row_h;

                            if is_node_ref && state.is_property_ref_expanded(node.id, key) {
                                if let Some(Value::Node(ref_id)) = val {
                                    if let Some(rn) = layout.get_node(*ref_id) {
                                        let mut child_keys: Vec<String> = rn.properties.keys().cloned().collect();
                                        child_keys.retain(|k| k != "clip");
                                        for ck in &child_keys {
                                            let c_h = 17.0;
                                            if py >= cur_y && py < cur_y + c_h {
                                                return PanelHitResult::SelectProperty(*ref_id, ck.clone());
                                            }
                                            cur_y += c_h;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            PanelHitResult::None
        }
    }

    /// Determines the hovered `NodeId` from a mouse position at `(px, py)`.
    pub fn handle_mouse_move(
        &self,
        px: f64,
        py: f64,
        panel_x: f64,
        win_h: f64,
        items: &[DomTreeItem],
        state: &InspectorState,
    ) -> Option<NodeId> {
        let divider_y = self.divider_y(win_h);
        if px < panel_x || px > panel_x + self.width || py < self.header_height || py >= divider_y {
            return None;
        }

        let total_tree_h = items.len() as f64 * self.row_height;
        let visible_tree_h = divider_y - self.header_height;
        if total_tree_h > visible_tree_h && px >= panel_x + self.width - 12.0 {
            return None;
        }
        let total_tree_w = self.total_tree_width(items);
        if total_tree_w > self.width && py >= divider_y - 8.0 {
            return None;
        }

        let content_y = py - self.header_height + state.scroll_offset;
        if content_y < 0.0 {
            return None;
        }
        let row_idx = (content_y / self.row_height) as usize;
        if row_idx < items.len() {
            items[row_idx].node_id
        } else {
            None
        }
    }

    /// Renders the complete inspector side panel onto `scene`.
    #[allow(clippy::too_many_arguments)]
    pub fn render_to_scene(
        &self,
        scene: &mut Scene,
        panel_x: f64,
        win_h: f64,
        items: &[DomTreeItem],
        state: &InspectorState,
        layout: &ResolvedLayout,
        font_cx: &mut FontContext,
        layout_cx: &mut LayoutContext<()>,
    ) {
        let panel_w = self.width;
        let divider_y = self.divider_y(win_h);

        // 1. Panel Background (slate 900)
        let panel_rect = Rect::new(panel_x, 0.0, panel_x + panel_w, win_h);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Brush::Solid(Color::from_rgb8(15, 23, 42)), // #0f172a
            None,
            &panel_rect,
        );

        // 2. Left dividing border (slate 800)
        let div_stroke = Stroke::new(1.0);
        scene.stroke(
            &div_stroke,
            Affine::IDENTITY,
            Brush::Solid(Color::from_rgb8(30, 41, 59)), // #1e293b
            None,
            &Line::new((panel_x, 0.0), (panel_x, win_h)),
        );

        // 3. Top Section: DOM Tree Rows (clipped beneath the header toolbar and above the divider)
        let tree_clip_rect = Rect::new(panel_x, self.header_height, panel_x + panel_w, divider_y);
        scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &tree_clip_rect);

        let scroll_y = state.scroll_offset;
        for (idx, item) in items.iter().enumerate() {
            let row_y = self.header_height + (idx as f64) * self.row_height - scroll_y;
            // Cull rows outside visible tree extents
            if row_y + self.row_height < self.header_height || row_y > divider_y {
                continue;
            }

            let row_rect = Rect::new(panel_x, row_y, panel_x + panel_w, row_y + self.row_height);

            // Row background highlight
            if item.is_selected {
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    Brush::Solid(Color::from_rgba8(2, 132, 199, 90)), // sky 600 tint
                    None,
                    &row_rect,
                );
                // Left 3px active indicator bar
                let bar = Rect::new(panel_x, row_y, panel_x + 3.0, row_y + self.row_height);
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    Brush::Solid(Color::from_rgb8(56, 189, 248)), // #38bdf8
                    None,
                    &bar,
                );
            } else if item.is_hovered {
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    Brush::Solid(Color::from_rgba8(30, 41, 59, 180)), // #1e293b
                    None,
                    &row_rect,
                );
            }

            // Chevron (aligned with text baseline)
            let indent_x = panel_x + 8.0 + item.depth as f64 * self.indent_step - state.scroll_offset_x;
            if item.has_children {
                let chevron_col = if item.is_expanded {
                    Color::from_rgb8(148, 163, 184) // slate 400
                } else {
                    Color::from_rgb8(100, 116, 139) // slate 500
                };
                Self::draw_chevron(
                    scene,
                    indent_x + 5.0,
                    row_y + 8.5,
                    7.0,
                    item.is_expanded,
                    chevron_col,
                );
            }

            let mut cur_x = indent_x + 14.0;

            // 0. Variable name (if let-bound or env-bound): e.g. "heading_font: "
            if let Some(var) = &item.var_name {
                let var_str = format!("{var}: ");
                cur_x += self.draw_text_snippet(
                    scene,
                    font_cx,
                    layout_cx,
                    &var_str,
                    10.5,
                    FontWeight::NORMAL,
                    Color::from_rgb8(203, 213, 225), // slate 300
                    true,
                    cur_x,
                    row_y + 2.75,
                );
            }

            // 1. Tag name: cyan for primitives, purple for authored components
            // Vertically centered by cap-height within the 18px row, enclosing descenders
            let tag_str = format!("\\{}", item.tag);
            let tag_col = if item.is_primitive() {
                Color::from_rgb8(56, 189, 248) // cyan 400
            } else {
                Color::from_rgb8(192, 132, 252) // purple 400
            };
            cur_x += self.draw_text_snippet(
                scene,
                font_cx,
                layout_cx,
                &tag_str,
                10.5,
                FontWeight::BOLD,
                tag_col,
                true,
                cur_x,
                row_y + 2.75,
            );

            // 2. Key or ID name: amber
            let key_or_id = item
                .key
                .as_ref()
                .map(|k| k.format_key())
                .or_else(|| item.id_name.clone());
            if let Some(kid) = &key_or_id {
                let id_str = format!("#{kid}");
                cur_x += self.draw_text_snippet(
                    scene,
                    font_cx,
                    layout_cx,
                    &id_str,
                    10.5,
                    FontWeight::NORMAL,
                    Color::from_rgb8(251, 191, 36), // amber 400
                    true,
                    cur_x,
                    row_y + 2.75,
                );
            }

            // 3. Port / text summary (e.g. text content preview)
            if !item.port_summary.is_empty() {
                let sum_str = format!(" {}", item.port_summary);
                self.draw_text_snippet(
                    scene,
                    font_cx,
                    layout_cx,
                    &sum_str,
                    10.0,
                    FontWeight::NORMAL,
                    Color::from_rgb8(148, 163, 184), // slate 400
                    true,
                    cur_x,
                    row_y + 3.75,
                );
            }
            // Note: spatial dimensions [W x H] are intentionally removed from the DOM tree rows!
        }

        scene.pop_layer();

        // Top Tree Scrollbars (vertical and horizontal indicators)
        let total_tree_h = items.len() as f64 * self.row_height;
        let visible_tree_h = divider_y - self.header_height;
        let total_tree_w = self.total_tree_width(items);
        let has_h_bar = total_tree_w > panel_w && visible_tree_h > 30.0;
        let has_v_bar = total_tree_h > visible_tree_h && visible_tree_h > 30.0;

        let v_track_h = if has_h_bar { visible_tree_h - 8.0 } else { visible_tree_h };
        let h_track_w = if has_v_bar { panel_w - 8.0 } else { panel_w - 4.0 };

        if has_v_bar {
            let bar_w = 4.0;
            let track_x = panel_x + panel_w - bar_w - 2.0;
            let thumb_h = ((visible_tree_h / total_tree_h) * v_track_h).max(18.0).min(v_track_h);
            let scroll_ratio = (state.scroll_offset / (total_tree_h - visible_tree_h).max(1.0)).clamp(0.0, 1.0);
            let thumb_y = self.header_height + scroll_ratio * (v_track_h - thumb_h);

            let thumb_rrect = RoundedRect::new(
                track_x,
                thumb_y,
                track_x + bar_w,
                thumb_y + thumb_h,
                2.0,
            );
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Brush::Solid(Color::from_rgba8(148, 163, 184, 120)),
                None,
                &thumb_rrect,
            );
        }

        if has_h_bar {
            let bar_h = 4.0;
            let track_y = divider_y - bar_h - 2.0;
            let thumb_w = ((panel_w / total_tree_w) * h_track_w).max(18.0).min(h_track_w);
            let scroll_ratio = (state.scroll_offset_x / (total_tree_w - panel_w).max(1.0)).clamp(0.0, 1.0);
            let thumb_x = panel_x + 2.0 + scroll_ratio * (h_track_w - thumb_w);

            let thumb_rrect = RoundedRect::new(
                thumb_x,
                track_y,
                thumb_x + thumb_w,
                track_y + bar_h,
                2.0,
            );
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Brush::Solid(Color::from_rgba8(148, 163, 184, 120)),
                None,
                &thumb_rrect,
            );
        }

        // 4. Header Toolbar (rendered on top of tree items)
        let header_rect = Rect::new(panel_x, 0.0, panel_x + panel_w, self.header_height);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Brush::Solid(Color::from_rgb8(30, 41, 59)), // slate 800
            None,
            &header_rect,
        );
        let header_stroke = Stroke::new(1.0);
        scene.stroke(
            &header_stroke,
            Affine::IDENTITY,
            Brush::Solid(Color::from_rgb8(51, 65, 85)), // slate 700
            None,
            &Line::new((panel_x, self.header_height), (panel_x + panel_w, self.header_height)),
        );

        // Pick Tool Button (`[↖]`)
        let btn_x = panel_x + 8.0;
        let btn_y = 6.0;
        let btn_w = 24.0;
        let btn_h = 24.0;
        let btn_rrect = RoundedRect::new(btn_x, btn_y, btn_x + btn_w, btn_y + btn_h, 4.0);
        let (btn_bg, btn_stroke, icon_col) = if state.inspect_cursor_active {
            (
                Color::from_rgb8(2, 132, 199),  // sky 600
                Color::from_rgb8(56, 189, 248), // sky 400
                Color::WHITE,
            )
        } else if state.inspect_cursor_hovered {
            (
                Color::from_rgb8(71, 85, 105),  // slate 600
                Color::from_rgb8(100, 116, 139), // slate 500
                Color::from_rgb8(241, 245, 249), // slate 100
            )
        } else {
            (
                Color::from_rgb8(51, 65, 85),   // slate 700
                Color::from_rgb8(71, 85, 105),  // slate 600
                Color::from_rgb8(148, 163, 184), // slate 400
            )
        };
        scene.fill(Fill::NonZero, Affine::IDENTITY, Brush::Solid(btn_bg), None, &btn_rrect);
        let btn_stroke_obj = Stroke::new(1.0);
        scene.stroke(&btn_stroke_obj, Affine::IDENTITY, Brush::Solid(btn_stroke), None, &btn_rrect);
        self.draw_text_snippet(
            scene,
            font_cx,
            layout_cx,
            "↖",
            12.0,
            FontWeight::BOLD,
            icon_col,
            false,
            btn_x + 6.0,
            btn_y + 4.0,
        );

        // Header Title
        self.draw_text_snippet(
            scene,
            font_cx,
            layout_cx,
            "DOM Elements",
            12.0,
            FontWeight::BOLD,
            Color::from_rgb8(241, 245, 249), // slate 100
            false,
            panel_x + 38.0,
            9.5,
        );

        // Node count badge pill
        let count_str = format!("{} nodes", items.len());
        let count_x = panel_x + 138.0;
        self.draw_text_snippet(
            scene,
            font_cx,
            layout_cx,
            &count_str,
            10.0,
            FontWeight::NORMAL,
            Color::from_rgb8(148, 163, 184), // slate 400
            false,
            count_x,
            11.0,
        );

        // Keyboard close hint
        self.draw_text_snippet(
            scene,
            font_cx,
            layout_cx,
            "[i] close",
            10.0,
            FontWeight::NORMAL,
            Color::from_rgb8(100, 116, 139), // slate 500
            false,
            panel_x + panel_w - 55.0,
            11.0,
        );

        // 5. Horizontal Section Divider Bar
        let div_bar_rect = Rect::new(panel_x, divider_y, panel_x + panel_w, divider_y + self.divider_height);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Brush::Solid(Color::from_rgb8(30, 41, 59)), // slate 800
            None,
            &div_bar_rect,
        );
        let div_stroke_obj = Stroke::new(1.0);
        // Top border
        scene.stroke(
            &div_stroke_obj,
            Affine::IDENTITY,
            Brush::Solid(Color::from_rgb8(51, 65, 85)), // slate 700
            None,
            &Line::new((panel_x, divider_y), (panel_x + panel_w, divider_y)),
        );
        // Bottom border
        scene.stroke(
            &div_stroke_obj,
            Affine::IDENTITY,
            Brush::Solid(Color::from_rgb8(15, 23, 42)), // slate 900
            None,
            &Line::new((panel_x, divider_y + self.divider_height), (panel_x + panel_w, divider_y + self.divider_height)),
        );

        let is_property_view = state.selected_property.is_some();
        let selected_node = state.selected_id.and_then(|id| layout.get_node(id));

        if is_property_view {
            self.draw_text_snippet_centered(
                scene,
                font_cx,
                layout_cx,
                "PROPERTY",
                9.5,
                FontWeight::BOLD,
                Color::from_rgb8(226, 232, 240), // slate 200
                false,
                panel_x + 10.0,
                divider_y,
                self.divider_height,
            );

            // Right-aligned back button with arrow only in the property title bar
            let btn_w = 22.0;
            let btn_h = 20.0;
            let btn_x = panel_x + panel_w - btn_w - 8.0;
            let btn_y = divider_y + 4.0;
            let btn_rrect = RoundedRect::new(btn_x, btn_y, btn_x + btn_w, btn_y + btn_h, 4.0);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Brush::Solid(Color::from_rgb8(15, 23, 42)), // slate 900
                None,
                &btn_rrect,
            );
            let btn_stroke = Stroke::new(1.0);
            scene.stroke(
                &btn_stroke,
                Affine::IDENTITY,
                Brush::Solid(Color::from_rgb8(71, 85, 105)), // slate 600
                None,
                &btn_rrect,
            );
            self.draw_text_snippet_centered(
                scene,
                font_cx,
                layout_cx,
                "←",
                11.0,
                FontWeight::BOLD,
                Color::from_rgb8(241, 245, 249),
                false,
                btn_x + 5.0,
                divider_y,
                self.divider_height,
            );
        } else {
            self.draw_text_snippet_centered(
                scene,
                font_cx,
                layout_cx,
                "COMPONENT DETAILS",
                9.5,
                FontWeight::BOLD,
                Color::from_rgb8(226, 232, 240), // slate 200
                false,
                panel_x + 10.0,
                divider_y,
                self.divider_height,
            );

            if let Some(node) = selected_node {
                let id_str = node.properties.get("id").and_then(|v| v.as_str());
                let badge_str = if let Some(id) = id_str {
                    format!("\\{}#{}", node.name, id)
                } else {
                    format!("\\{}", node.name)
                };
                self.draw_text_snippet_centered(
                    scene,
                    font_cx,
                    layout_cx,
                    &badge_str,
                    10.0,
                    FontWeight::BOLD,
                    Color::from_rgb8(56, 189, 248), // cyan 400
                    true,
                    panel_x + 160.0,
                    divider_y,
                    self.divider_height,
                );
            } else {
                self.draw_text_snippet_centered(
                    scene,
                    font_cx,
                    layout_cx,
                    "(no selection)",
                    9.5,
                    FontWeight::NORMAL,
                    Color::from_rgb8(100, 116, 139), // slate 500
                    false,
                    panel_x + 160.0,
                    divider_y,
                    self.divider_height,
                );
            }
        }

        // 6. Bottom Section: Component Details Area (Properties, Formulas & Evaluations)
        let detail_y = divider_y + self.divider_height;
        let detail_h = (win_h - detail_y).max(0.0);
        let detail_clip = Rect::new(panel_x, detail_y, panel_x + panel_w, win_h);
        scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &detail_clip);

        let mut total_detail_content_h = self.total_detail_content_height(state.selected_id, layout, Some(state));
        let total_detail_content_w = self.total_detail_content_width(state.selected_id, layout, Some(state));

        if let Some((sel_nid, ref sel_prop)) = state.selected_property {
            if let Some(trace) = crate::inspector::build_property_dag_trace(layout, sel_nid, sel_prop) {
                self.render_property_details(
                    scene,
                    font_cx,
                    layout_cx,
                    panel_x,
                    panel_w,
                    detail_y,
                    win_h,
                    state,
                    &trace,
                    total_detail_content_w,
                );
            }
        } else if let Some(node) = selected_node {
            let base_y = detail_y - state.detail_scroll_offset;
            let base_x = panel_x - state.detail_scroll_offset_x;
            let card_w = (panel_w - 20.0).max(total_detail_content_w - 20.0);
            let mut cur_y = base_y + 10.0;

            // --- Component Reactive State ---
            if !node.state_vars.is_empty() {
                self.draw_text_snippet(
                    scene,
                    font_cx,
                    layout_cx,
                    "COMPONENT STATE",
                    9.0,
                    FontWeight::BOLD,
                    Color::from_rgb8(192, 132, 252), // purple 400
                    false,
                    base_x + 12.0,
                    cur_y,
                );
                cur_y += 18.0;

                let mut state_names: Vec<String> = node.state_vars.keys().cloned().collect();
                state_names.sort();

                for s_name in &state_names {
                    let type_annot = node.state_vars.get(s_name).cloned().flatten();
                    let val_opt = layout.get_value(node.id, s_name);

                    let row_h = 24.0;
                    if cur_y + row_h >= detail_y && cur_y <= win_h {
                        let state_x = base_x + 12.0;

                        // Live state indicator dot (purple halo + vibrant dot)
                        let dot_cx = state_x + 4.0;
                        let dot_cy = cur_y + 8.65;
                        let halo = Circle::new((dot_cx, dot_cy), 3.5);
                        scene.fill(
                            Fill::NonZero,
                            Affine::IDENTITY,
                            Brush::Solid(Color::from_rgba8(168, 85, 247, 40)),
                            None,
                            &halo,
                        );
                        let dot = Circle::new((dot_cx, dot_cy), 2.0);
                        scene.fill(
                            Fill::NonZero,
                            Affine::IDENTITY,
                            Brush::Solid(Color::from_rgb8(192, 132, 252)),
                            None,
                            &dot,
                        );

                        // Variable Name
                        let name_adv = self.draw_text_snippet(
                            scene,
                            font_cx,
                            layout_cx,
                            s_name,
                            10.5,
                            FontWeight::BOLD,
                            Color::from_rgb8(216, 180, 254), // purple 300
                            true,
                            state_x + 12.0,
                            cur_y + 1.5,
                        );

                        // Separator ": "
                        let sep_adv = self.draw_text_snippet(
                            scene,
                            font_cx,
                            layout_cx,
                            ": ",
                            10.5,
                            FontWeight::NORMAL,
                            Color::from_rgb8(100, 116, 139), // slate 500
                            true,
                            state_x + 12.0 + name_adv,
                            cur_y + 1.5,
                        );

                        let mut val_x = state_x + 12.0 + name_adv + sep_adv;

                        let eval_str = if let Some(v) = val_opt {
                            format!("{v}")
                        } else {
                            "-".to_string()
                        };

                        if let Some(swatch_col) = parse_hex_color(&eval_str) {
                            let swatch_rrect =
                                RoundedRect::new(val_x, cur_y + 2.5, val_x + 11.0, cur_y + 13.5, 2.0);
                            scene.fill(
                                Fill::NonZero,
                                Affine::IDENTITY,
                                Brush::Solid(swatch_col),
                                None,
                                &swatch_rrect,
                            );
                            let swatch_stroke = Stroke::new(1.0);
                            scene.stroke(
                                &swatch_stroke,
                                Affine::IDENTITY,
                                Brush::Solid(Color::from_rgb8(71, 85, 105)),
                                None,
                                &swatch_rrect,
                            );
                            val_x += 16.0;
                        }

                        let val_col = match val_opt {
                            Some(Value::Bool(true)) => Color::from_rgb8(52, 211, 153), // emerald 400
                            Some(Value::Bool(false)) => Color::from_rgb8(248, 113, 113), // red 400
                            Some(Value::Number(_)) => Color::from_rgb8(251, 191, 36), // amber 400
                            Some(Value::String(_)) => Color::from_rgb8(251, 191, 36), // amber
                            _ => Color::from_rgb8(241, 245, 249),
                        };

                        let val_adv = self.draw_text_snippet(
                            scene,
                            font_cx,
                            layout_cx,
                            &eval_str,
                            10.5,
                            FontWeight::BOLD,
                            val_col,
                            true,
                            val_x,
                            cur_y + 1.5,
                        );

                        if let Some(t_name) = &type_annot {
                            let annot_str = format!(" ({t_name})");
                            self.draw_text_snippet(
                                scene,
                                font_cx,
                                layout_cx,
                                &annot_str,
                                9.0,
                                FontWeight::NORMAL,
                                Color::from_rgb8(148, 163, 184), // slate 400
                                false,
                                val_x + val_adv + 4.0,
                                cur_y + 2.5,
                            );
                        }

                        // Subtle separator line under each state row
                        let sep_y = cur_y + row_h - 1.0;
                        let sep_stroke = Stroke::new(1.0);
                        scene.stroke(
                            &sep_stroke,
                            Affine::IDENTITY,
                            Brush::Solid(Color::from_rgb8(24, 32, 47)),
                            None,
                            &Line::new((state_x, sep_y), (base_x + 10.0 + card_w, sep_y)),
                        );
                    }

                    cur_y += row_h;
                }

                cur_y += 10.0;
            }

            // Collect and order properties:
            // 1. Spatial: x, y, width, height, z
            // 2. Styling: color, bg_color, border_color, border_width, radius, clip
            // 3. Other parameters & custom equations
            let mut prop_keys: Vec<String> = node.properties.keys().cloned().collect();
            prop_keys.retain(|k| k != "clip");
            for geom in ["x", "y", "width", "height", "z"] {
                if !prop_keys.contains(&geom.to_string()) {
                    prop_keys.push(geom.to_string());
                }
            }
            prop_keys.sort_by(|a, b| {
                let rank = |k: &str| match k {
                    "x" => 1,
                    "y" => 2,
                    "width" => 3,
                    "height" => 4,
                    "z" => 5,
                    "left" => 6,
                    "top" => 7,
                    "right" => 8,
                    "bottom" => 9,
                    "color" | "bg_color" => 10,
                    "border_color" | "border_width" => 11,
                    "radius" | "corner_radius" => 12,
                    "clip" => 13,
                    _ => 14,
                };
                rank(a).cmp(&rank(b)).then_with(|| a.cmp(b))
            });

            for key in &prop_keys {
                let val_opt = node.properties.get(key);
                let is_node_ref = matches!(val_opt, Some(Value::Node(_)));

                let eval_str = if let Some(val) = val_opt {
                    match val {
                        Value::Node(ref_id) => {
                            if let Some(rn) = layout.get_node(*ref_id) {
                                if let Some(var) = &rn.var_name {
                                    format!("{var} (\\{})", rn.name)
                                } else {
                                    format!("\\{}", rn.name)
                                }
                            } else {
                                format!("{val}")
                            }
                        }
                        _ => format!("{val}"),
                    }
                } else {
                    match key.as_str() {
                        "x" => format!("{:.1}", node.rect.x),
                        "y" => format!("{:.1}", node.rect.y),
                        "width" => format!("{:.1}", node.rect.width),
                        "height" => format!("{:.1}", node.rect.height),
                        "z" => format!("{:.0}", node.z),
                        _ => "-".to_string(),
                    }
                };

                let formula_str = node.formulas.get(key).cloned().unwrap_or_default();
                let has_formula = !formula_str.is_empty() && formula_str != eval_str;
                let row_h = 18.0;

                let is_ref_expanded = is_node_ref && state.is_property_ref_expanded(node.id, key);

                // Only draw if within visible viewport
                if cur_y + row_h >= detail_y && cur_y <= win_h {
                    let prop_x = base_x + 12.0;

                    // Chevron (for node refs) or Typographic bullet (for standard props)
                    if is_node_ref {
                        let chevron_col = if is_ref_expanded {
                            Color::from_rgb8(192, 132, 252) // purple 400
                        } else {
                            Color::from_rgb8(148, 163, 184) // slate 400
                        };
                        Self::draw_chevron(
                            scene,
                            prop_x + 4.0,
                            cur_y + 8.65,
                            6.5,
                            is_ref_expanded,
                            chevron_col,
                        );
                    } else {
                        // Typographic bullet: centered on the optical midpoint of lowercase letters (x-height center)
                        let bullet_cx = prop_x + 4.0;
                        let bullet_cy = cur_y + 8.65;
                        let bullet_circle = Circle::new((bullet_cx, bullet_cy), 2.0);
                        scene.fill(
                            Fill::NonZero,
                            Affine::IDENTITY,
                            Brush::Solid(Color::from_rgb8(56, 189, 248)), // cyan 400
                            None,
                            &bullet_circle,
                        );
                    }

                    let name_adv = self.draw_text_snippet(
                        scene,
                        font_cx,
                        layout_cx,
                        key,
                        10.5,
                        FontWeight::BOLD,
                        Color::from_rgb8(56, 189, 248), // cyan 400
                        true,
                        prop_x + 12.0,
                        cur_y + 1.5,
                    );

                    let sep_adv = self.draw_text_snippet(
                        scene,
                        font_cx,
                        layout_cx,
                        ": ",
                        10.5,
                        FontWeight::NORMAL,
                        Color::from_rgb8(100, 116, 139), // slate 500
                        true,
                        prop_x + 12.0 + name_adv,
                        cur_y + 1.5,
                    );

                    let mut val_x = prop_x + 12.0 + name_adv + sep_adv;

                    if !is_node_ref {
                        // If evaluated value is a color hex, render a small color swatch box!
                        if let Some(swatch_col) = parse_hex_color(&eval_str) {
                            let swatch_rrect = RoundedRect::new(val_x, cur_y + 2.5, val_x + 11.0, cur_y + 13.5, 2.0);
                            scene.fill(Fill::NonZero, Affine::IDENTITY, Brush::Solid(swatch_col), None, &swatch_rrect);
                            let swatch_stroke = Stroke::new(1.0);
                            scene.stroke(&swatch_stroke, Affine::IDENTITY, Brush::Solid(Color::from_rgb8(71, 85, 105)), None, &swatch_rrect);
                            val_x += 16.0;
                        }
                    }

                    let val_col = if is_node_ref {
                        Color::from_rgb8(192, 132, 252) // purple 400 for node references
                    } else if parse_hex_color(&eval_str).is_some() {
                        Color::from_rgb8(241, 245, 249) // white
                    } else if eval_str.starts_with('"') {
                        Color::from_rgb8(251, 191, 36) // amber for strings
                    } else {
                        Color::from_rgb8(52, 211, 153) // emerald for numbers/bools
                    };

                    let val_adv = self.draw_text_snippet(
                        scene,
                        font_cx,
                        layout_cx,
                        &eval_str,
                        10.5,
                        FontWeight::BOLD,
                        val_col,
                        true,
                        val_x,
                        cur_y + 1.5,
                    );

                    // Inline formula: `← formula`
                    if has_formula {
                        let arrow_adv = self.draw_text_snippet(
                            scene,
                            font_cx,
                            layout_cx,
                            " ← ",
                            10.5,
                            FontWeight::NORMAL,
                            Color::from_rgb8(100, 116, 139), // slate 500
                            true,
                            val_x + val_adv,
                            cur_y + 1.5,
                        );
                        self.draw_text_snippet(
                            scene,
                            font_cx,
                            layout_cx,
                            &formula_str,
                            10.5,
                            FontWeight::NORMAL,
                            Color::from_rgb8(203, 213, 225), // slate 300
                            true,
                            val_x + val_adv + arrow_adv,
                            cur_y + 1.5,
                        );
                    }
                }

                cur_y += row_h;

                // Indented child properties if reference is expanded
                if is_ref_expanded {
                    if let Some(Value::Node(ref_id)) = val_opt {
                        if let Some(rn) = layout.get_node(*ref_id) {
                            let mut child_keys: Vec<String> = rn.properties.keys().cloned().collect();
                            child_keys.retain(|k| k != "clip");
                            child_keys.sort_by(|a, b| {
                                let rank = |k: &str| match k {
                                    "size" => 1,
                                    "weight" => 2,
                                    "family" => 3,
                                    "line_height" => 4,
                                    "cap_height" => 5,
                                    "x_height" => 6,
                                    "ascent" => 7,
                                    "descent" => 8,
                                    _ => 10,
                                };
                                rank(a).cmp(&rank(b)).then_with(|| a.cmp(b))
                            });

                            let child_start_y = cur_y;
                            let guide_x = base_x + 20.0;
                            let child_indent_x = base_x + 28.0;

                            for ck in &child_keys {
                                let c_val = rn.properties.get(ck);
                                let c_eval = c_val.map(|v| format!("{v}")).unwrap_or_default();
                                let c_form = rn.formulas.get(ck).cloned().unwrap_or_else(|| c_eval.clone());
                                let c_has = c_form != c_eval && !c_form.is_empty();
                                let c_row_h = 17.0;

                                if cur_y + c_row_h >= detail_y && cur_y <= win_h {
                                    // Bullet
                                    let c_bullet = Circle::new((child_indent_x + 2.0, cur_y + 7.5), 1.5);
                                    scene.fill(
                                        Fill::NonZero,
                                        Affine::IDENTITY,
                                        Brush::Solid(Color::from_rgb8(125, 211, 252)), // sky 300
                                        None,
                                        &c_bullet,
                                    );

                                    // Key
                                    let c_name_adv = self.draw_text_snippet(
                                        scene,
                                        font_cx,
                                        layout_cx,
                                        ck,
                                        9.5,
                                        FontWeight::BOLD,
                                        Color::from_rgb8(125, 211, 252), // sky 300
                                        true,
                                        child_indent_x + 8.0,
                                        cur_y + 1.0,
                                    );

                                    // Separator ": "
                                    let c_sep_adv = self.draw_text_snippet(
                                        scene,
                                        font_cx,
                                        layout_cx,
                                        ": ",
                                        9.5,
                                        FontWeight::NORMAL,
                                        Color::from_rgb8(100, 116, 139), // slate 500
                                        true,
                                        child_indent_x + 8.0 + c_name_adv,
                                        cur_y + 1.0,
                                    );

                                    let mut c_val_x = child_indent_x + 8.0 + c_name_adv + c_sep_adv;

                                    if let Some(swatch_col) = parse_hex_color(&c_eval) {
                                        let swatch_rrect = RoundedRect::new(c_val_x, cur_y + 2.0, c_val_x + 10.0, cur_y + 12.0, 2.0);
                                        scene.fill(Fill::NonZero, Affine::IDENTITY, Brush::Solid(swatch_col), None, &swatch_rrect);
                                        let swatch_stroke = Stroke::new(1.0);
                                        scene.stroke(&swatch_stroke, Affine::IDENTITY, Brush::Solid(Color::from_rgb8(71, 85, 105)), None, &swatch_rrect);
                                        c_val_x += 14.0;
                                    }

                                    let c_val_col = if parse_hex_color(&c_eval).is_some() {
                                        Color::from_rgb8(241, 245, 249)
                                    } else if c_eval.starts_with('"') {
                                        Color::from_rgb8(251, 191, 36)
                                    } else {
                                        Color::from_rgb8(52, 211, 153)
                                    };

                                    let c_val_adv = self.draw_text_snippet(
                                        scene,
                                        font_cx,
                                        layout_cx,
                                        &c_eval,
                                        9.5,
                                        FontWeight::BOLD,
                                        c_val_col,
                                        true,
                                        c_val_x,
                                        cur_y + 1.0,
                                    );

                                    if c_has {
                                        let c_arrow_adv = self.draw_text_snippet(
                                            scene,
                                            font_cx,
                                            layout_cx,
                                            " ← ",
                                            9.5,
                                            FontWeight::NORMAL,
                                            Color::from_rgb8(100, 116, 139),
                                            true,
                                            c_val_x + c_val_adv,
                                            cur_y + 1.0,
                                        );
                                        self.draw_text_snippet(
                                            scene,
                                            font_cx,
                                            layout_cx,
                                            &c_form,
                                            9.5,
                                            FontWeight::NORMAL,
                                            Color::from_rgb8(203, 213, 225),
                                            true,
                                            c_val_x + c_val_adv + c_arrow_adv,
                                            cur_y + 1.0,
                                        );
                                    }
                                }
                                cur_y += c_row_h;
                            }

                            // Guideline spanning the indented children
                            if cur_y > child_start_y && child_start_y <= win_h {
                                let guide_top = child_start_y.max(detail_y);
                                let guide_bottom = (cur_y - 3.0).min(win_h);
                                if guide_bottom > guide_top {
                                    scene.stroke(
                                        &Stroke::new(1.0),
                                        Affine::IDENTITY,
                                        Brush::Solid(Color::from_rgba8(148, 163, 184, 70)),
                                        None,
                                        &Line::new((guide_x, guide_top), (guide_x, guide_bottom)),
                                    );
                                }
                            }
                        }
                    }
                }
            }

            total_detail_content_h = cur_y - base_y + 20.0;
        } else {
            // Empty state placeholder
            let empty_y = detail_y + 40.0;
            let empty_card = RoundedRect::new(panel_x + 20.0, empty_y, panel_x + panel_w - 20.0, empty_y + 70.0, 6.0);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Brush::Solid(Color::from_rgb8(24, 32, 47)),
                None,
                &empty_card,
            );
            let empty_stroke = Stroke::new(1.0);
            scene.stroke(
                &empty_stroke,
                Affine::IDENTITY,
                Brush::Solid(Color::from_rgb8(51, 65, 85)),
                None,
                &empty_card,
            );
            self.draw_text_snippet(
                scene,
                font_cx,
                layout_cx,
                "No component selected",
                11.0,
                FontWeight::BOLD,
                Color::from_rgb8(148, 163, 184), // slate 400
                false,
                panel_x + 40.0,
                empty_y + 16.0,
            );
            self.draw_text_snippet(
                scene,
                font_cx,
                layout_cx,
                "Click a node in the tree or use [↖] to inspect",
                9.5,
                FontWeight::NORMAL,
                Color::from_rgb8(100, 116, 139), // slate 500
                false,
                panel_x + 40.0,
                empty_y + 36.0,
            );
        }

        scene.pop_layer();

        // Bottom Details Scrollbars (vertical and horizontal indicators)
        let has_detail_v = total_detail_content_h > detail_h && detail_h > 30.0;
        let has_detail_h = total_detail_content_w > panel_w && detail_h > 30.0;

        let v_track_h = if has_detail_h { (detail_h - 8.0).max(18.0) } else { detail_h };
        let h_track_w = if has_detail_v { (panel_w - 8.0).max(18.0) } else { panel_w - 4.0 };

        if has_detail_v {
            let bar_w = 4.0;
            let track_x = panel_x + panel_w - bar_w - 2.0;
            let thumb_h = ((detail_h / total_detail_content_h) * v_track_h).max(18.0).min(v_track_h);
            let scroll_ratio = (state.detail_scroll_offset / (total_detail_content_h - detail_h).max(1.0)).clamp(0.0, 1.0);
            let thumb_y = detail_y + scroll_ratio * (v_track_h - thumb_h);

            let thumb_rrect = RoundedRect::new(
                track_x,
                thumb_y,
                track_x + bar_w,
                thumb_y + thumb_h,
                2.0,
            );
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Brush::Solid(Color::from_rgba8(148, 163, 184, 120)),
                None,
                &thumb_rrect,
            );
        }

        if has_detail_h {
            let bar_h = 4.0;
            let track_y = win_h - bar_h - 2.0;
            let thumb_w = ((panel_w / total_detail_content_w) * h_track_w).max(18.0).min(h_track_w);
            let scroll_ratio = (state.detail_scroll_offset_x / (total_detail_content_w - panel_w).max(1.0)).clamp(0.0, 1.0);
            let thumb_x = panel_x + 2.0 + scroll_ratio * (h_track_w - thumb_w);

            let thumb_rrect = RoundedRect::new(
                thumb_x,
                track_y,
                thumb_x + thumb_w,
                track_y + bar_h,
                2.0,
            );
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Brush::Solid(Color::from_rgba8(148, 163, 184, 120)),
                None,
                &thumb_rrect,
            );
        }

        // Left resize border highlight when hovered or actively resizing
        if state.resize_border_hovered || state.is_resizing_panel {
            let border_stroke = Stroke::new(2.5);
            let border_color = if state.is_resizing_panel {
                Color::from_rgb8(56, 189, 248) // sky-400
            } else {
                Color::from_rgb8(14, 165, 233) // sky-500
            };
            let border_line = Line::new((panel_x, 0.0), (panel_x, win_h));
            scene.stroke(
                &border_stroke,
                Affine::IDENTITY,
                Brush::Solid(border_color),
                None,
                &border_line,
            );
        }
    }

    /// Renders the DAG Property Details view (tracing value origin, dependencies, and dependents).
    #[allow(clippy::too_many_arguments)]
    fn render_property_details(
        &self,
        scene: &mut Scene,
        font_cx: &mut FontContext,
        layout_cx: &mut LayoutContext<()>,
        panel_x: f64,
        panel_w: f64,
        detail_y: f64,
        _win_h: f64,
        state: &InspectorState,
        trace: &crate::inspector::DagPropertyTrace,
        content_w: f64,
    ) {
        let base_y = detail_y - state.detail_scroll_offset;
        let base_x = panel_x - state.detail_scroll_offset_x;
        let card_w = (panel_w - 20.0).max(content_w - 20.0);
        let mut cur_y = base_y + 10.0;

        // 1. Inspected Property Port Card
        let card_x = base_x + 10.0;
        let card_h = 48.0;
        let card_rrect = RoundedRect::new(card_x, cur_y, card_x + card_w, cur_y + card_h, 6.0);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Brush::Solid(Color::from_rgb8(24, 32, 47)), // slate 900+
            None,
            &card_rrect,
        );
        let card_stroke = Stroke::new(1.0);
        scene.stroke(
            &card_stroke,
            Affine::IDENTITY,
            Brush::Solid(Color::from_rgb8(51, 65, 85)), // slate 700
            None,
            &card_rrect,
        );

        // Line 1: "<Component> . <portname> : <value>"
        let mut line1_x = card_x + 12.0;
        let comp_adv = self.draw_text_snippet(
            scene,
            font_cx,
            layout_cx,
            &trace.target_label,
            10.5,
            FontWeight::BOLD,
            Color::from_rgb8(56, 189, 248), // cyan 400
            true,
            line1_x,
            cur_y + 7.0,
        );
        line1_x += comp_adv;

        let dot_adv = self.draw_text_snippet(
            scene,
            font_cx,
            layout_cx,
            " . ",
            10.5,
            FontWeight::NORMAL,
            Color::from_rgb8(148, 163, 184), // slate 400
            true,
            line1_x,
            cur_y + 7.0,
        );
        line1_x += dot_adv;

        let port_adv = self.draw_text_snippet(
            scene,
            font_cx,
            layout_cx,
            &trace.port_name,
            10.5,
            FontWeight::BOLD,
            Color::from_rgb8(56, 189, 248), // cyan 400
            true,
            line1_x,
            cur_y + 7.0,
        );
        line1_x += port_adv;

        let colon_adv = self.draw_text_snippet(
            scene,
            font_cx,
            layout_cx,
            " : ",
            10.5,
            FontWeight::NORMAL,
            Color::from_rgb8(148, 163, 184), // slate 400
            true,
            line1_x,
            cur_y + 7.0,
        );
        line1_x += colon_adv;

        if let Some(swatch_col) = parse_hex_color(&trace.evaluated_value_str) {
            let swatch_rrect = RoundedRect::new(line1_x, cur_y + 8.0, line1_x + 11.0, cur_y + 19.0, 2.0);
            scene.fill(Fill::NonZero, Affine::IDENTITY, Brush::Solid(swatch_col), None, &swatch_rrect);
            let swatch_stroke = Stroke::new(1.0);
            scene.stroke(&swatch_stroke, Affine::IDENTITY, Brush::Solid(Color::from_rgb8(71, 85, 105)), None, &swatch_rrect);
            line1_x += 16.0;
        }

        let val_col = if parse_hex_color(&trace.evaluated_value_str).is_some() {
            Color::from_rgb8(241, 245, 249)
        } else if trace.evaluated_value_str.starts_with('"') {
            Color::from_rgb8(251, 191, 36)
        } else {
            Color::from_rgb8(52, 211, 153)
        };

        self.draw_text_snippet(
            scene,
            font_cx,
            layout_cx,
            &trace.evaluated_value_str,
            10.5,
            FontWeight::BOLD,
            val_col,
            true,
            line1_x,
            cur_y + 7.0,
        );

        // Line 2: Formula (no "Formula:" prefix)
        self.draw_text_snippet(
            scene,
            font_cx,
            layout_cx,
            &trace.equation_str,
            10.0,
            FontWeight::NORMAL,
            Color::from_rgb8(203, 213, 225), // slate 300
            true,
            card_x + 12.0,
            cur_y + 26.0,
        );

        cur_y += card_h + 14.0;

        // 3. Section: Upstream DAG Dependencies
        self.draw_text_snippet(
            scene,
            font_cx,
            layout_cx,
            "UPSTREAM DEPENDENCIES (reads from)",
            9.0,
            FontWeight::BOLD,
            Color::from_rgb8(148, 163, 184),
            false,
            base_x + 12.0,
            cur_y,
        );
        let in_count = format!("({} input{})", trace.upstream_dependencies.len(), if trace.upstream_dependencies.len() == 1 { "" } else { "s" });
        self.draw_text_snippet(
            scene,
            font_cx,
            layout_cx,
            &in_count,
            9.0,
            FontWeight::NORMAL,
            Color::from_rgb8(100, 116, 139),
            false,
            base_x + 230.0,
            cur_y,
        );
        cur_y += 18.0;

        if trace.upstream_dependencies.is_empty() {
            let prop_x = base_x + 12.0;
            let bullet_circle = Circle::new((prop_x + 4.0, cur_y + 8.65), 2.0);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Brush::Solid(Color::from_rgb8(100, 116, 139)), // slate 500
                None,
                &bullet_circle,
            );
            self.draw_text_snippet(
                scene,
                font_cx,
                layout_cx,
                "None (root value)",
                10.0,
                FontWeight::NORMAL,
                Color::from_rgb8(148, 163, 184), // slate 400
                false,
                prop_x + 12.0,
                cur_y + 1.5,
            );
            cur_y += 18.0 + 10.0;
        } else {
            for dep in &trace.upstream_dependencies {
                let row_h = 18.0;
                let prop_x = base_x + 12.0;
                let has_formula = !dep.equation_str.is_empty() && dep.equation_str != dep.value_str;

                self.draw_text_snippet(
                    scene,
                    font_cx,
                    layout_cx,
                    "↑",
                    10.5,
                    FontWeight::BOLD,
                    Color::from_rgb8(56, 189, 248), // cyan 400
                    true,
                    prop_x,
                    cur_y + 1.5,
                );

                let name_str = format!("{}.{}", dep.node_label, dep.port_name);
                let name_adv = self.draw_text_snippet(
                    scene,
                    font_cx,
                    layout_cx,
                    &name_str,
                    10.5,
                    FontWeight::BOLD,
                    Color::from_rgb8(56, 189, 248), // cyan 400
                    true,
                    prop_x + 12.0,
                    cur_y + 1.5,
                );

                let sep_adv = self.draw_text_snippet(
                    scene,
                    font_cx,
                    layout_cx,
                    ": ",
                    10.5,
                    FontWeight::NORMAL,
                    Color::from_rgb8(100, 116, 139), // slate 500
                    true,
                    prop_x + 12.0 + name_adv,
                    cur_y + 1.5,
                );

                let mut val_x = prop_x + 12.0 + name_adv + sep_adv;

                if let Some(swatch_col) = parse_hex_color(&dep.value_str) {
                    let swatch_rrect = RoundedRect::new(val_x, cur_y + 2.5, val_x + 11.0, cur_y + 13.5, 2.0);
                    scene.fill(Fill::NonZero, Affine::IDENTITY, Brush::Solid(swatch_col), None, &swatch_rrect);
                    let swatch_stroke = Stroke::new(1.0);
                    scene.stroke(&swatch_stroke, Affine::IDENTITY, Brush::Solid(Color::from_rgb8(71, 85, 105)), None, &swatch_rrect);
                    val_x += 16.0;
                }

                let val_col = if parse_hex_color(&dep.value_str).is_some() {
                    Color::from_rgb8(241, 245, 249) // white
                } else if dep.value_str.starts_with('"') {
                    Color::from_rgb8(251, 191, 36) // amber for strings
                } else {
                    Color::from_rgb8(52, 211, 153) // emerald for numbers/bools
                };

                let val_adv = self.draw_text_snippet(
                    scene,
                    font_cx,
                    layout_cx,
                    &dep.value_str,
                    10.5,
                    FontWeight::BOLD,
                    val_col,
                    true,
                    val_x,
                    cur_y + 1.5,
                );

                if has_formula {
                    let arrow_adv = self.draw_text_snippet(
                        scene,
                        font_cx,
                        layout_cx,
                        " ← ",
                        10.5,
                        FontWeight::NORMAL,
                        Color::from_rgb8(100, 116, 139), // slate 500
                        true,
                        val_x + val_adv,
                        cur_y + 1.5,
                    );
                    self.draw_text_snippet(
                        scene,
                        font_cx,
                        layout_cx,
                        &dep.equation_str,
                        10.5,
                        FontWeight::NORMAL,
                        Color::from_rgb8(203, 213, 225), // slate 300
                        true,
                        val_x + val_adv + arrow_adv,
                        cur_y + 1.5,
                    );
                }

                cur_y += row_h;
            }
            cur_y += 10.0;
        }

        // 4. Section: Downstream DAG Dependents
        self.draw_text_snippet(
            scene,
            font_cx,
            layout_cx,
            "DOWNSTREAM DEPENDENTS (feeds into)",
            9.0,
            FontWeight::BOLD,
            Color::from_rgb8(148, 163, 184),
            false,
            base_x + 12.0,
            cur_y,
        );
        let out_count = format!("({} dependent{})", trace.downstream_dependents.len(), if trace.downstream_dependents.len() == 1 { "" } else { "s" });
        self.draw_text_snippet(
            scene,
            font_cx,
            layout_cx,
            &out_count,
            9.0,
            FontWeight::NORMAL,
            Color::from_rgb8(100, 116, 139),
            false,
            base_x + 230.0,
            cur_y,
        );
        cur_y += 18.0;

        if trace.downstream_dependents.is_empty() {
            let prop_x = base_x + 12.0;
            let bullet_circle = Circle::new((prop_x + 4.0, cur_y + 8.65), 2.0);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Brush::Solid(Color::from_rgb8(100, 116, 139)), // slate 500
                None,
                &bullet_circle,
            );
            self.draw_text_snippet(
                scene,
                font_cx,
                layout_cx,
                "None (leaf port)",
                10.0,
                FontWeight::NORMAL,
                Color::from_rgb8(148, 163, 184), // slate 400
                false,
                prop_x + 12.0,
                cur_y + 1.5,
            );
            cur_y += 18.0 + 10.0;
        } else {
            for dep in &trace.downstream_dependents {
                let row_h = 18.0;
                let prop_x = base_x + 12.0;
                let has_formula = !dep.equation_str.is_empty() && dep.equation_str != dep.value_str;

                self.draw_text_snippet(
                    scene,
                    font_cx,
                    layout_cx,
                    "↓",
                    10.5,
                    FontWeight::BOLD,
                    Color::from_rgb8(192, 132, 252), // purple 400
                    true,
                    prop_x,
                    cur_y + 1.5,
                );

                let name_str = format!("{}.{}", dep.node_label, dep.port_name);
                let name_adv = self.draw_text_snippet(
                    scene,
                    font_cx,
                    layout_cx,
                    &name_str,
                    10.5,
                    FontWeight::BOLD,
                    Color::from_rgb8(56, 189, 248), // cyan 400
                    true,
                    prop_x + 12.0,
                    cur_y + 1.5,
                );

                let sep_adv = self.draw_text_snippet(
                    scene,
                    font_cx,
                    layout_cx,
                    ": ",
                    10.5,
                    FontWeight::NORMAL,
                    Color::from_rgb8(100, 116, 139), // slate 500
                    true,
                    prop_x + 12.0 + name_adv,
                    cur_y + 1.5,
                );

                let mut val_x = prop_x + 12.0 + name_adv + sep_adv;

                if let Some(swatch_col) = parse_hex_color(&dep.value_str) {
                    let swatch_rrect = RoundedRect::new(val_x, cur_y + 2.5, val_x + 11.0, cur_y + 13.5, 2.0);
                    scene.fill(Fill::NonZero, Affine::IDENTITY, Brush::Solid(swatch_col), None, &swatch_rrect);
                    let swatch_stroke = Stroke::new(1.0);
                    scene.stroke(&swatch_stroke, Affine::IDENTITY, Brush::Solid(Color::from_rgb8(71, 85, 105)), None, &swatch_rrect);
                    val_x += 16.0;
                }

                let val_col = if parse_hex_color(&dep.value_str).is_some() {
                    Color::from_rgb8(241, 245, 249) // white
                } else if dep.value_str.starts_with('"') {
                    Color::from_rgb8(251, 191, 36) // amber for strings
                } else {
                    Color::from_rgb8(52, 211, 153) // emerald for numbers/bools
                };

                let val_adv = self.draw_text_snippet(
                    scene,
                    font_cx,
                    layout_cx,
                    &dep.value_str,
                    10.5,
                    FontWeight::BOLD,
                    val_col,
                    true,
                    val_x,
                    cur_y + 1.5,
                );

                if has_formula {
                    let arrow_adv = self.draw_text_snippet(
                        scene,
                        font_cx,
                        layout_cx,
                        " ← ",
                        10.5,
                        FontWeight::NORMAL,
                        Color::from_rgb8(100, 116, 139), // slate 500
                        true,
                        val_x + val_adv,
                        cur_y + 1.5,
                    );
                    self.draw_text_snippet(
                        scene,
                        font_cx,
                        layout_cx,
                        &dep.equation_str,
                        10.5,
                        FontWeight::NORMAL,
                        Color::from_rgb8(203, 213, 225), // slate 300
                        true,
                        val_x + val_adv + arrow_adv,
                        cur_y + 1.5,
                    );
                }

                cur_y += row_h;
            }
            cur_y += 10.0;
        }

        // 5. Section: DAG Identifiers & Metrics
        let stats_rrect = RoundedRect::new(base_x + 10.0, cur_y, base_x + 10.0 + card_w, cur_y + 44.0, 4.0);
        scene.fill(Fill::NonZero, Affine::IDENTITY, Brush::Solid(Color::from_rgb8(17, 24, 39)), None, &stats_rrect);
        let stats_stroke = Stroke::new(1.0);
        scene.stroke(&stats_stroke, Affine::IDENTITY, Brush::Solid(Color::from_rgb8(30, 41, 59)), None, &stats_rrect);

        let var_id_str = format!("DAG VarId: {}", trace.canonical_var);
        self.draw_text_snippet(scene, font_cx, layout_cx, &var_id_str, 9.0, FontWeight::NORMAL, Color::from_rgb8(148, 163, 184), true, base_x + 18.0, cur_y + 7.0);

        let metrics_str = format!("In-degree: {} | Out-degree: {}", trace.upstream_dependencies.len(), trace.downstream_dependents.len());
        self.draw_text_snippet(scene, font_cx, layout_cx, &metrics_str, 9.0, FontWeight::NORMAL, Color::from_rgb8(100, 116, 139), true, base_x + 18.0, cur_y + 24.0);
    }

    /// Internal helper to render a single-line text snippet, returning its advance width.
    #[allow(clippy::too_many_arguments)]
    fn draw_text_snippet(
        &self,
        scene: &mut Scene,
        font_cx: &mut FontContext,
        layout_cx: &mut LayoutContext<()>,
        text: &str,
        font_size: f32,
        font_weight: FontWeight,
        color: Color,
        is_monospace: bool,
        x: f64,
        y: f64,
    ) -> f64 {
        if text.is_empty() {
            return 0.0;
        }

        let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(font_size));
        if (font_weight.value() - 400.0).abs() > 1.0 {
            builder.push_default(StyleProperty::FontWeight(font_weight));
        }
        if is_monospace {
            #[cfg(target_os = "macos")]
            builder.push_default(StyleProperty::FontFamily(FontFamily::named("Menlo")));
            #[cfg(not(target_os = "macos"))]
            builder.push_default(StyleProperty::FontFamily(FontFamily::Generic(GenericFamily::Monospace)));
        }

        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(Alignment::Start, parley::layout::AlignmentOptions::default());

        let text_affine = Affine::translate((x, y));
        for line in layout.lines() {
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(glyph_run) = item {
                    let run = glyph_run.run();
                    let font = run.font();
                    let glyphs = glyph_run.positioned_glyphs().map(|g| vello::Glyph {
                        id: g.id,
                        x: g.x,
                        y: g.y,
                    });

                    scene
                        .draw_glyphs(font)
                        .font_size(run.font_size())
                        .transform(text_affine)
                        .brush(Brush::Solid(color))
                        .draw(Fill::NonZero, glyphs);
                }
            }
        }

        layout.full_width() as f64
    }

    /// Internal helper to render a single-line text snippet vertically centered by cap-height
    /// within `[box_y, box_y + box_h]`, returning its advance width.
    #[allow(clippy::too_many_arguments)]
    fn draw_text_snippet_centered(
        &self,
        scene: &mut Scene,
        font_cx: &mut FontContext,
        layout_cx: &mut LayoutContext<()>,
        text: &str,
        font_size: f32,
        font_weight: FontWeight,
        color: Color,
        is_monospace: bool,
        x: f64,
        box_y: f64,
        box_h: f64,
    ) -> f64 {
        if text.is_empty() {
            return 0.0;
        }

        let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(font_size));
        if (font_weight.value() - 400.0).abs() > 1.0 {
            builder.push_default(StyleProperty::FontWeight(font_weight));
        }
        if is_monospace {
            #[cfg(target_os = "macos")]
            builder.push_default(StyleProperty::FontFamily(FontFamily::named("Menlo")));
            #[cfg(not(target_os = "macos"))]
            builder.push_default(StyleProperty::FontFamily(FontFamily::Generic(GenericFamily::Monospace)));
        }

        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(Alignment::Start, parley::layout::AlignmentOptions::default());

        let cap_height = (font_size * 0.71) as f64;
        let baseline_offset = layout
            .lines()
            .next()
            .map(|l| l.metrics().baseline as f64)
            .unwrap_or((font_size * 0.8) as f64);
        let target_baseline = (box_h + cap_height) / 2.0;
        let y = box_y + target_baseline - baseline_offset;

        let text_affine = Affine::translate((x, y));
        for line in layout.lines() {
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(glyph_run) = item {
                    let run = glyph_run.run();
                    let font = run.font();
                    let glyphs = glyph_run.positioned_glyphs().map(|g| vello::Glyph {
                        id: g.id,
                        x: g.x,
                        y: g.y,
                    });

                    scene
                        .draw_glyphs(font)
                        .font_size(run.font_size())
                        .transform(text_affine)
                        .brush(Brush::Solid(color))
                        .draw(Fill::NonZero, glyphs);
                }
            }
        }

        layout.full_width() as f64
    }

    /// Draws a crisp geometric vector triangle chevron for tree and property folding.
    fn draw_chevron(scene: &mut Scene, cx: f64, cy: f64, size: f64, is_expanded: bool, color: Color) {
        let mut path = BezPath::new();
        if is_expanded {
            // Downward-pointing equilateral triangle centered at (cx, cy)
            let half_w = size * 0.5;
            let h = size * 0.75;
            let top_y = cy - h * 0.4;
            let bot_y = cy + h * 0.6;
            path.move_to((cx - half_w, top_y));
            path.line_to((cx + half_w, top_y));
            path.line_to((cx, bot_y));
            path.close_path();
        } else {
            // Rightward-pointing equilateral triangle centered at (cx, cy)
            let half_h = size * 0.5;
            let w = size * 0.75;
            let left_x = cx - w * 0.4;
            let right_x = cx + w * 0.6;
            path.move_to((left_x, cy - half_h));
            path.line_to((right_x, cy));
            path.line_to((left_x, cy + half_h));
            path.close_path();
        }
        scene.fill(Fill::NonZero, Affine::IDENTITY, Brush::Solid(color), None, &path);
    }
}

/// Helper function to parse hex colors for rendering inline swatches.
fn parse_hex_color(hex: &str) -> Option<Color> {
    let s = hex.trim_matches('"');
    let s = s.strip_prefix('#')?;
    if s.len() == 6 {
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some(Color::from_rgb8(r, g, b))
    } else if s.len() == 3 {
        let r = u8::from_str_radix(&s[0..1], 16).ok()? * 17;
        let g = u8::from_str_radix(&s[1..2], 16).ok()? * 17;
        let b = u8::from_str_radix(&s[2..3], 16).ok()? * 17;
        Some(Color::from_rgb8(r, g, b))
    } else {
        None
    }
}
