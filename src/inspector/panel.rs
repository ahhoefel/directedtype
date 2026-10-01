use parley::layout::PositionedLayoutItem;
use parley::style::{FontFamily, FontWeight, StyleProperty};
#[cfg(not(target_os = "macos"))]
use parley::style::GenericFamily;
use parley::{Alignment, FontContext, LayoutContext};
use vello::kurbo::{Affine, Line, Rect, RoundedRect, Stroke};
use vello::peniko::{Brush, Color, Fill};
use vello::Scene;

use crate::compiler::expanded::NodeId;
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

/// The docked side panel component rendering the live Component DOM tree.
#[derive(Debug, Clone)]
pub struct InspectPanelComponent {
    /// Width of the side panel in logical pixels (default: 360px).
    pub width: f64,
    /// Height of the top header toolbar (default: 36px).
    pub header_height: f64,
    /// Height of each tree item row (default: 24px).
    pub row_height: f64,
    /// Pixel indentation step per depth level (default: 14px).
    pub indent_step: f64,
}

impl Default for InspectPanelComponent {
    fn default() -> Self {
        Self {
            width: 360.0,
            header_height: 36.0,
            row_height: 19.0,
            indent_step: 14.0,
        }
    }
}

impl InspectPanelComponent {
    pub fn new() -> Self {
        Self::default()
    }

    /// Computes the maximum vertical scroll offset for `total_items` within `win_h`.
    pub fn max_scroll(&self, total_items: usize, win_h: f64) -> f64 {
        let content_h = total_items as f64 * self.row_height;
        let visible_h = win_h - self.header_height;
        (content_h - visible_h).max(0.0)
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
        _win_h: f64,
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

        // Tree row clicks
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
    }

    /// Determines the hovered `NodeId` from a mouse position at `(px, py)`.
    pub fn handle_mouse_move(
        &self,
        px: f64,
        py: f64,
        panel_x: f64,
        _win_h: f64,
        items: &[DomTreeItem],
        state: &InspectorState,
    ) -> Option<NodeId> {
        if px < panel_x || px > panel_x + self.width || py < self.header_height {
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
    pub fn render_to_scene(
        &self,
        scene: &mut Scene,
        panel_x: f64,
        win_h: f64,
        items: &[DomTreeItem],
        state: &InspectorState,
        font_cx: &mut FontContext,
        layout_cx: &mut LayoutContext<()>,
    ) {
        let panel_w = self.width;

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

        // 3. Tree Rows (rendered with clipping beneath the header toolbar)
        let clip_rect = Rect::new(panel_x, self.header_height, panel_x + panel_w, win_h);
        scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &clip_rect);

        let scroll_y = state.scroll_offset;
        for (idx, item) in items.iter().enumerate() {
            let row_y = self.header_height + (idx as f64) * self.row_height - scroll_y;
            // Cull rows outside visible panel vertical extents
            if row_y + self.row_height < self.header_height || row_y > win_h {
                continue;
            }

            let row_rect = Rect::new(panel_x, row_y, panel_x + panel_w, row_y + self.row_height);

            // Row background highlight
            if item.is_selected {
                // Active selection background (slate 800 with sky accent)
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

            // Chevron
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
                    8.5,
                    FontWeight::NORMAL,
                    chevron_col,
                    true,
                    indent_x,
                    row_y + 3.0,
                );
            }

            // Syntax-highlighted row contents:
            let mut cur_x = indent_x + 14.0;

            // 1. Tag name: violet for components, cyan for primitives
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
                row_y + 2.0,
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
                    row_y + 2.0,
                );
            }

            // 3. Port / text summary
            if !item.port_summary.is_empty() {
                let sum_str = format!(" {}", item.port_summary);
                cur_x += self.draw_text_snippet(
                    scene,
                    font_cx,
                    layout_cx,
                    &sum_str,
                    10.0,
                    FontWeight::NORMAL,
                    Color::from_rgb8(148, 163, 184), // slate 400
                    true,
                    cur_x,
                    row_y + 2.5,
                );
            }

            // 4. Spatial dimensions preview
            if !item.bounds_summary.is_empty() {
                let b_str = format!(" {}", item.bounds_summary);
                self.draw_text_snippet(
                    scene,
                    font_cx,
                    layout_cx,
                    &b_str,
                    9.5,
                    FontWeight::NORMAL,
                    Color::from_rgb8(100, 116, 139), // slate 500
                    true,
                    cur_x,
                    row_y + 2.5,
                );
            }
        }

        scene.pop_layer();

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

        // 5. Scrollbar Track & Thumb (if tree content overflows panel height)
        let total_content_h = items.len() as f64 * self.row_height;
        let visible_h = win_h - self.header_height;
        if total_content_h > visible_h && visible_h > 30.0 {
            let bar_w = 4.0;
            let track_x = panel_x + panel_w - bar_w - 2.0;
            let thumb_h = ((visible_h / total_content_h) * visible_h).max(20.0).min(visible_h);
            let scroll_ratio = state.scroll_offset / (total_content_h - visible_h).max(1.0);
            let thumb_y = self.header_height + scroll_ratio * (visible_h - thumb_h);

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
