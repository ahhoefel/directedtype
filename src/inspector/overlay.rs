use parley::layout::{AlignmentOptions, PositionedLayoutItem};
use parley::style::{FontWeight, StyleProperty};
use parley::{Alignment, FontContext, LayoutContext};
use vello::kurbo::{Affine, Rect as KRect, RoundedRect as KRoundedRect, Stroke};
use vello::peniko::{Brush, Color, Fill};
use vello::Scene;

use crate::compiler::layout::Rect;
use crate::inspector::model::InspectTargetInfo;

/// Visual styling configuration for the inspection overlay.
#[derive(Debug, Clone)]
pub struct InspectOverlayStyle {
    pub hover_fill: Color,
    pub hover_stroke: Color,
    pub select_fill: Color,
    pub select_stroke: Color,
    pub badge_bg: Color,
    pub badge_text: Color,
    pub clip_guide_stroke: Color,
    pub stroke_width: f64,
}

impl Default for InspectOverlayStyle {
    fn default() -> Self {
        Self {
            hover_fill: Color::from_rgba8(56, 189, 248, 30),     // Cyan translucent
            hover_stroke: Color::from_rgba8(56, 189, 248, 220),  // Cyan border
            select_fill: Color::from_rgba8(245, 158, 11, 40),    // Amber translucent
            select_stroke: Color::from_rgba8(245, 158, 11, 240), // Amber border
            badge_bg: Color::from_rgba8(15, 23, 42, 230),        // Dark slate pill
            badge_text: Color::from_rgba8(56, 189, 248, 255),    // Cyan text
            clip_guide_stroke: Color::from_rgba8(168, 85, 247, 180), // Purple clip guide
            stroke_width: 1.5,
        }
    }
}

/// A reusable component that renders the DOM inspection overlay (highlight box,
/// floating coordinate pill badge, and active clip guide) onto a Vello scene.
#[derive(Debug, Clone, Default)]
pub struct InspectOverlayComponent {
    pub style: InspectOverlayStyle,
}

impl InspectOverlayComponent {
    pub fn new(style: InspectOverlayStyle) -> Self {
        Self { style }
    }

    /// Renders the complete inspection overlay (highlight, clip guide, and floating badge) onto `scene`.
    pub fn render_to_scene(
        &self,
        scene: &mut Scene,
        transform: Affine,
        info: &InspectTargetInfo,
        is_selected: bool,
        font_cx: &mut FontContext,
        layout_cx: &mut LayoutContext<()>,
        window_width: f64,
        window_height: f64,
    ) {
        let (fill_color, stroke_color, stroke_width) = if is_selected {
            (
                self.style.select_fill,
                self.style.select_stroke,
                self.style.stroke_width * 1.5,
            )
        } else {
            (
                self.style.hover_fill,
                self.style.hover_stroke,
                self.style.stroke_width,
            )
        };

        // 1. Render active clip guide (if the element is bound by an ancestor clip node)
        if let Some(clip_r) = info.clip_rect {
            let clip_krect = KRect::new(
                clip_r.x,
                clip_r.y,
                clip_r.x + clip_r.width,
                clip_r.y + clip_r.height,
            );
            let clip_stroke = Stroke::new(1.0);
            scene.stroke(
                &clip_stroke,
                transform,
                Brush::Solid(self.style.clip_guide_stroke),
                None,
                &clip_krect,
            );
        }

        // 2. Render target element highlight box (translucent fill + crisp stroke)
        let target_krect = KRect::new(
            info.rect.x,
            info.rect.y,
            info.rect.x + info.rect.width,
            info.rect.y + info.rect.height,
        );

        // Translucent fill
        scene.fill(
            Fill::NonZero,
            transform,
            Brush::Solid(fill_color),
            None,
            &target_krect,
        );

        // Bounding outline
        let stroke = Stroke::new(stroke_width);
        scene.stroke(
            &stroke,
            transform,
            Brush::Solid(stroke_color),
            None,
            &target_krect,
        );

        // 3. Render floating coordinate & tag badge
        let label_text = info.badge_label();
        self.render_badge(
            scene,
            transform,
            &label_text,
            info.rect,
            font_cx,
            layout_cx,
            window_width,
            window_height,
        );
    }

    /// Renders the floating dimension & tag badge above or below the target element.
    fn render_badge(
        &self,
        scene: &mut Scene,
        transform: Affine,
        text: &str,
        target_rect: Rect,
        font_cx: &mut FontContext,
        layout_cx: &mut LayoutContext<()>,
        window_width: f64,
        window_height: f64,
    ) {
        // Measure text using Parley for exact typographic width
        let font_size = 11.0f32;
        let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(font_size));
        builder.push_default(StyleProperty::FontWeight(FontWeight::BOLD));

        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());

        let text_width = layout.width() as f64;
        let h_pad = 8.0;
        let v_pad = 3.0;
        let badge_width = text_width + h_pad * 2.0;
        let badge_height = 20.0;

        let padding = 4.0;
        let mut bx = target_rect.x;
        if bx + badge_width > window_width - padding {
            bx = (window_width - badge_width - padding).max(padding);
        }
        if bx < padding {
            bx = padding;
        }

        let by = if target_rect.y >= badge_height + padding + 2.0 {
            target_rect.y - badge_height - 4.0
        } else {
            (target_rect.y + target_rect.height + 4.0).min(window_height - badge_height - padding)
        };

        // Draw rounded badge background
        let badge_rrect = KRoundedRect::new(bx, by, bx + badge_width, by + badge_height, 4.0);
        scene.fill(
            Fill::NonZero,
            transform,
            Brush::Solid(self.style.badge_bg),
            None,
            &badge_rrect,
        );

        // Draw badge outline
        let badge_stroke = Stroke::new(1.0);
        scene.stroke(
            &badge_stroke,
            transform,
            Brush::Solid(self.style.badge_text),
            None,
            &badge_rrect,
        );

        // Draw badge label text
        let text_affine = transform * Affine::translate((bx + h_pad, by + v_pad));
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
                        .brush(Brush::Solid(self.style.badge_text))
                        .draw(Fill::NonZero, glyphs);
                }
            }
        }
    }

    /// Generates a reusable DTML element snippet instantiating `\InspectOverlay` for this target.
    pub fn build_dtml(&self, info: &InspectTargetInfo, is_selected: bool) -> String {
        format!(
            r#"\InspectOverlay(target_x: {}, target_y: {}, target_w: {}, target_h: {}, label: "{}", is_selected: {})"#,
            info.rect.x,
            info.rect.y,
            info.rect.width,
            info.rect.height,
            info.badge_label(),
            is_selected
        )
    }
}
