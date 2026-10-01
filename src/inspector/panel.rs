use parley::layout::PositionedLayoutItem;
use parley::style::{FontFamily, FontWeight, StyleProperty};
#[cfg(not(target_os = "macos"))]
use parley::style::GenericFamily;
use parley::{Alignment, FontContext, LayoutContext};
use vello::kurbo::{Affine, Circle, Line, Rect, RoundedRect, Stroke};
use vello::peniko::{Brush, Color, Fill};
use vello::Scene;

use crate::compiler::expanded::NodeId;
use crate::compiler::layout::ResolvedLayout;
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
    pub fn new() -> Self {
        Self::default()
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

    /// Computes the maximum vertical scroll offset for the bottom details panel.
    pub fn max_detail_scroll(&self, selected_id: Option<NodeId>, layout: &ResolvedLayout, win_h: f64) -> f64 {
        let divider_y = self.divider_y(win_h);
        let detail_visible_h = (win_h - (divider_y + self.divider_height)).max(0.0);
        let content_h = if let Some(node) = selected_id.and_then(|id| layout.get_node(id)) {
            let num_props = node.properties.len() + 5;
            110.0 + (num_props as f64) * 36.0 + 30.0
        } else {
            120.0
        };
        (content_h - detail_visible_h).max(0.0)
    }

    /// Returns true if `(px, py)` is inside the header pick tool (`[↖]`) button.
    pub fn is_cursor_btn_hovered(&self, px: f64, py: f64, panel_x: f64) -> bool {
        px >= panel_x + 4.0 && px <= panel_x + 36.0 && py >= 2.0 && py <= self.header_height - 2.0
    }

    /// Determines the action triggered by clicking at `(px, py)` in logical window space.
    pub fn handle_click(
        &self,
        px: f64,
        py: f64,
        panel_x: f64,
        win_h: f64,
        items: &[DomTreeItem],
        state: &InspectorState,
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
                let indent_x = panel_x + 8.0 + item.depth as f64 * self.indent_step;
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
            let indent_x = panel_x + 8.0 + item.depth as f64 * self.indent_step;
            if item.has_children {
                let chevron_str = if item.is_expanded { "▼" } else { "►" };
                let chevron_col = if item.is_expanded {
                    Color::from_rgb8(148, 163, 184) // slate 400
                } else {
                    Color::from_rgb8(100, 116, 139) // slate 500
                };
                self.draw_text_snippet(
                    scene,
                    font_cx,
                    layout_cx,
                    chevron_str,
                    9.5,
                    FontWeight::NORMAL,
                    chevron_col,
                    false,
                    indent_x,
                    row_y + 2.75,
                );
            }

            let mut cur_x = indent_x + 14.0;

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

            // 2. ID name: amber
            if let Some(id) = &item.id_name {
                let id_str = format!("#{id}");
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

        // Top Tree Scrollbar Indicator (if tree content overflows)
        let total_tree_h = items.len() as f64 * self.row_height;
        let visible_tree_h = divider_y - self.header_height;
        if total_tree_h > visible_tree_h && visible_tree_h > 30.0 {
            let bar_w = 4.0;
            let track_x = panel_x + panel_w - bar_w - 2.0;
            let thumb_h = ((visible_tree_h / total_tree_h) * visible_tree_h).max(18.0).min(visible_tree_h);
            let scroll_ratio = state.scroll_offset / (total_tree_h - visible_tree_h).max(1.0);
            let thumb_y = self.header_height + scroll_ratio * (visible_tree_h - thumb_h);

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

        // Left title in divider
        self.draw_text_snippet(
            scene,
            font_cx,
            layout_cx,
            "COMPONENT DETAILS",
            10.0,
            FontWeight::BOLD,
            Color::from_rgb8(226, 232, 240), // slate 200
            false,
            panel_x + 10.0,
            divider_y + 6.0,
        );

        let selected_node = state.selected_id.and_then(|id| layout.get_node(id));

        // Right pill in divider: selected component name & ID
        if let Some(node) = selected_node {
            let id_str = node.properties.get("id").and_then(|v| v.as_str());
            let badge_str = if let Some(id) = id_str {
                format!("\\{}#{}", node.name, id)
            } else {
                format!("\\{}", node.name)
            };
            self.draw_text_snippet(
                scene,
                font_cx,
                layout_cx,
                &badge_str,
                10.0,
                FontWeight::BOLD,
                Color::from_rgb8(56, 189, 248), // cyan 400
                true,
                panel_x + 160.0,
                divider_y + 6.0,
            );
        } else {
            self.draw_text_snippet(
                scene,
                font_cx,
                layout_cx,
                "(no selection)",
                9.5,
                FontWeight::NORMAL,
                Color::from_rgb8(100, 116, 139), // slate 500
                false,
                panel_x + 160.0,
                divider_y + 6.5,
            );
        }

        // 6. Bottom Section: Component Details Area (Dimensions, Properties, Formulas & Evaluations)
        let detail_y = divider_y + self.divider_height;
        let detail_h = (win_h - detail_y).max(0.0);
        let detail_clip = Rect::new(panel_x, detail_y, panel_x + panel_w, win_h);
        scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &detail_clip);

        let mut total_detail_content_h = 100.0;

        if let Some(node) = selected_node {
            let base_y = detail_y - state.detail_scroll_offset;
            let mut cur_y = base_y + 10.0;

            // --- Section A: Dimensions & Box Model ---
            self.draw_text_snippet(
                scene,
                font_cx,
                layout_cx,
                "DIMENSIONS & BOUNDS",
                9.0,
                FontWeight::BOLD,
                Color::from_rgb8(148, 163, 184), // slate 400
                false,
                panel_x + 12.0,
                cur_y,
            );
            cur_y += 14.0;

            let card_x = panel_x + 10.0;
            let card_w = panel_w - 20.0;
            let card_h = 76.0;
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

            // Coordinates Row
            let coord_y = cur_y + 7.0;
            let x_str = format!("x: {:.1}", node.rect.x);
            self.draw_text_snippet(scene, font_cx, layout_cx, &x_str, 10.0, FontWeight::BOLD, Color::from_rgb8(251, 191, 36), true, card_x + 12.0, coord_y);

            let y_str = format!("y: {:.1}", node.rect.y);
            self.draw_text_snippet(scene, font_cx, layout_cx, &y_str, 10.0, FontWeight::BOLD, Color::from_rgb8(251, 191, 36), true, card_x + 88.0, coord_y);

            let z_str = format!("z: {:.0}", node.z);
            self.draw_text_snippet(scene, font_cx, layout_cx, &z_str, 10.0, FontWeight::NORMAL, Color::from_rgb8(203, 213, 225), true, card_x + 164.0, coord_y);

            let clip_str = if let Some(cid) = node.clip {
                format!("clip: Box#{}", cid.0)
            } else {
                "clip: none".to_string()
            };
            self.draw_text_snippet(scene, font_cx, layout_cx, &clip_str, 9.5, FontWeight::NORMAL, Color::from_rgb8(148, 163, 184), true, card_x + 230.0, coord_y + 0.5);

            // Center Box Model Pill
            let box_x = card_x + 10.0;
            let box_y = cur_y + 28.0;
            let box_w = card_w - 20.0;
            let box_h = 34.0;
            let box_rrect = RoundedRect::new(box_x, box_y, box_x + box_w, box_y + box_h, 4.0);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Brush::Solid(Color::from_rgba8(2, 132, 199, 35)), // sky 950 tint
                None,
                &box_rrect,
            );
            let box_stroke = Stroke::new(1.0);
            scene.stroke(
                &box_stroke,
                Affine::IDENTITY,
                Brush::Solid(Color::from_rgb8(2, 132, 199)), // sky 600
                None,
                &box_rrect,
            );
            let dim_str = format!("{:.1} × {:.1} px", node.rect.width, node.rect.height);
            self.draw_text_snippet(
                scene,
                font_cx,
                layout_cx,
                &dim_str,
                11.5,
                FontWeight::BOLD,
                Color::from_rgb8(248, 250, 252), // white
                true,
                box_x + (box_w - 110.0).max(10.0) / 2.0,
                box_y + 8.5,
            );

            cur_y += card_h + 16.0;

            // --- Section B: Properties, Formulas & Evaluations ---
            self.draw_text_snippet(
                scene,
                font_cx,
                layout_cx,
                "PROPERTIES & FORMULAS",
                9.0,
                FontWeight::BOLD,
                Color::from_rgb8(148, 163, 184), // slate 400
                false,
                panel_x + 12.0,
                cur_y,
            );
            cur_y += 18.0;

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
                    "color" | "bg_color" => 6,
                    "border_color" | "border_width" => 7,
                    "radius" | "corner_radius" => 8,
                    "clip" => 9,
                    _ => 10,
                };
                rank(a).cmp(&rank(b)).then_with(|| a.cmp(b))
            });

            for key in &prop_keys {
                let eval_str = if let Some(val) = node.properties.get(key) {
                    format!("{val}")
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

                let formula_str = node.formulas.get(key).cloned().unwrap_or_else(|| eval_str.clone());
                let has_formula = formula_str != eval_str && !formula_str.is_empty();
                let row_h = if has_formula { 36.0 } else { 22.0 };

                // Only draw if within visible viewport
                if cur_y + row_h >= detail_y && cur_y <= win_h {
                    let prop_x = panel_x + 12.0;

                    // Line 1: Bullet + Property Name + Value
                    // Typographic bullet: centered on the optical midpoint of lowercase letters (x-height center)
                    // aligning with the colon and hyphen, rather than at the top of lowercase x.
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

                    // If evaluated value is a color hex, render a small color swatch box!
                    if let Some(swatch_col) = parse_hex_color(&eval_str) {
                        let swatch_rrect = RoundedRect::new(val_x, cur_y + 2.5, val_x + 11.0, cur_y + 13.5, 2.0);
                        scene.fill(Fill::NonZero, Affine::IDENTITY, Brush::Solid(swatch_col), None, &swatch_rrect);
                        let swatch_stroke = Stroke::new(1.0);
                        scene.stroke(&swatch_stroke, Affine::IDENTITY, Brush::Solid(Color::from_rgb8(71, 85, 105)), None, &swatch_rrect);
                        val_x += 16.0;
                    }

                    let val_col = if parse_hex_color(&eval_str).is_some() {
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

                    if !has_formula {
                        self.draw_text_snippet(
                            scene,
                            font_cx,
                            layout_cx,
                            " (literal)",
                            9.0,
                            FontWeight::NORMAL,
                            Color::from_rgb8(100, 116, 139), // slate 500
                            false,
                            val_x + val_adv + 4.0,
                            cur_y + 2.5,
                        );
                    }

                    // Line 2: Formula / Equation (if non-trivial)
                    if has_formula {
                        let formula_line = format!("  ↳ {formula_str}");
                        self.draw_text_snippet(
                            scene,
                            font_cx,
                            layout_cx,
                            &formula_line,
                            9.5,
                            FontWeight::NORMAL,
                            Color::from_rgb8(203, 213, 225), // slate 300
                            true,
                            prop_x + 6.0,
                            cur_y + 18.0,
                        );
                    }

                    // Subtle separator line under each property row
                    let sep_y = cur_y + row_h - 1.0;
                    let sep_stroke = Stroke::new(1.0);
                    scene.stroke(
                        &sep_stroke,
                        Affine::IDENTITY,
                        Brush::Solid(Color::from_rgb8(24, 32, 47)),
                        None,
                        &Line::new((prop_x, sep_y), (panel_x + panel_w - 12.0, sep_y)),
                    );
                }

                cur_y += row_h;
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

        // Bottom Details Scrollbar (if content overflows bottom panel height)
        if total_detail_content_h > detail_h && detail_h > 30.0 {
            let bar_w = 4.0;
            let track_x = panel_x + panel_w - bar_w - 2.0;
            let thumb_h = ((detail_h / total_detail_content_h) * detail_h).max(18.0).min(detail_h);
            let scroll_ratio = state.detail_scroll_offset / (total_detail_content_h - detail_h).max(1.0);
            let thumb_y = detail_y + scroll_ratio * (detail_h - thumb_h);

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

        layout.width() as f64
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
