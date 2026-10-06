use parley::layout::{AlignmentOptions, PositionedLayoutItem};
use parley::style::{FontFamily, FontWeight, StyleProperty};
use parley::{Alignment, FontContext, LayoutContext};
use vello::kurbo::{Affine, Rect, RoundedRect, Stroke};
use vello::peniko::{Brush, Color, Fill};
use vello::Scene;

use crate::compiler::expanded::NodeId;
use crate::compiler::layout::ResolvedLayout;
use crate::compiler::value::Value;
use crate::render::color::{color_to_rgba8, parse_color};

/// Options for building a Vello scene from a ResolvedLayout.
#[derive(Debug, Clone)]
pub struct SceneOptions {
    /// Optional background color for the viewport / canvas.
    pub background: Option<Color>,
    /// Display / DPI scale factor (e.g. 2.0 on macOS Retina displays).
    pub scale_factor: f64,
}

impl Default for SceneOptions {
    fn default() -> Self {
        Self {
            background: Some(Color::WHITE),
            scale_factor: 1.0,
        }
    }
}

fn get_clip_chain(
    clip_id: Option<NodeId>,
    layout: &ResolvedLayout,
) -> Vec<NodeId> {
    let mut chain = Vec::new();
    let mut curr = clip_id;
    let mut visited = std::collections::HashSet::new();

    while let Some(id) = curr {
        if id.is_window() || !visited.insert(id) {
            break;
        }
        chain.push(id);
        curr = layout
            .get_value(id, "up")
            .and_then(|v| v.as_node())
            .filter(|up_id| !up_id.is_window());
    }

    chain.reverse(); // root-to-leaf order
    chain
}

/// Builds a `vello::Scene` from a `ResolvedLayout`.
pub fn build_scene(
    layout: &ResolvedLayout,
    font_cx: &mut FontContext,
    _layout_cx: &mut LayoutContext<()>,
    options: &SceneOptions,
) -> Scene {
    let mut scene = Scene::new();
    let mut text_layout_cx = LayoutContext::<[u8; 4]>::new();

    // 1. Draw canvas background if requested
    if let Some(bg) = options.background {
        if bg.components[3] > 0.0 {
            // Find bounding box or maximum extents
            let mut max_w: f64 = 0.0;
            let mut max_h: f64 = 0.0;
            for node in &layout.nodes {
                max_w = max_w.max(node.rect.x + node.rect.width);
                max_h = max_h.max(node.rect.y + node.rect.height);
            }
            if max_w > 0.0 && max_h > 0.0 {
                let rect = Rect::new(0.0, 0.0, max_w, max_h);
                scene.fill(Fill::NonZero, Affine::IDENTITY, Brush::Solid(bg), None, &rect);
            }
        }
    }

    let mut active_clip_stack: Vec<NodeId> = Vec::new();

    // 2. Iterate in topological painter's order
    for node in layout.render_order() {
        if !node.is_paint_primitive() {
            continue;
        }

        // Inline children of a Text node are rendered as styled spans by the parent Text node
        if let Some(parent_id) = node.parent {
            if layout.get_node(parent_id).map(|p| p.name.as_str()) == Some("Text") {
                continue;
            }
        }

        let target_chain = get_clip_chain(node.clip, layout);
        let common_len = active_clip_stack
            .iter()
            .zip(&target_chain)
            .take_while(|(a, b)| a == b)
            .count();

        while active_clip_stack.len() > common_len {
            scene.pop_layer();
            active_clip_stack.pop();
        }

        for &clip_node_id in &target_chain[common_len..] {
            if let Some(box_node_id) = layout.get_value(clip_node_id, "box").and_then(|v| v.as_node()) {
                let get_box_val = |port: &str| -> f64 {
                    layout
                        .get_value(box_node_id, port)
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0)
                };
                let x = get_box_val("x");
                let y = get_box_val("y");
                let w = get_box_val("width");
                let h = get_box_val("height");
                let radius = layout
                    .get_value(box_node_id, "radius")
                    .or_else(|| layout.get_value(box_node_id, "corner_radius"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);

                let x0 = x;
                let y0 = y;
                let x1 = x0 + w;
                let y1 = y0 + h;

                if radius > 0.0 {
                    let rrect = RoundedRect::new(x0, y0, x1, y1, radius);
                    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rrect);
                } else {
                    let rect = Rect::new(x0, y0, x1, y1);
                    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rect);
                }
            }
            active_clip_stack.push(clip_node_id);
        }

        let is_rect = node.name == "Rect";

        // Render filled rectangle (only for \Rect paint primitives)
        if is_rect && node.rect.width > 0.0 && node.rect.height > 0.0 {
            let color_val = node
                .properties
                .get("color")
                .or_else(|| node.properties.get("bg_color"));

            let color = match color_val {
                Some(Value::Color(c)) | Some(Value::String(c)) => parse_color(c),
                _ => Color::TRANSPARENT,
            };

            let radius = node
                .properties
                .get("radius")
                .or_else(|| node.properties.get("corner_radius"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);

            let x0 = node.rect.x;
            let y0 = node.rect.y;
            let x1 = x0 + node.rect.width;
            let y1 = y0 + node.rect.height;

            if color.components[3] > 0.0 {
                if radius > 0.0 {
                    let rrect = RoundedRect::new(x0, y0, x1, y1, radius);
                    scene.fill(Fill::NonZero, Affine::IDENTITY, Brush::Solid(color), None, &rrect);
                } else {
                    let rect = Rect::new(x0, y0, x1, y1);
                    scene.fill(Fill::NonZero, Affine::IDENTITY, Brush::Solid(color), None, &rect);
                }
            }

            // Stroke / border
            let stroke_width = node
                .properties
                .get("border_width")
                .or_else(|| node.properties.get("stroke_width"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);

            if stroke_width > 0.0 {
                let stroke_color = node
                    .properties
                    .get("border_color")
                    .or_else(|| node.properties.get("stroke_color"))
                    .map(|v| match v {
                        Value::Color(c) | Value::String(c) => parse_color(c),
                        _ => Color::BLACK,
                    })
                    .unwrap_or(Color::BLACK);

                let stroke = Stroke::new(stroke_width);
                if radius > 0.0 {
                    let rrect = RoundedRect::new(x0, y0, x1, y1, radius);
                    scene.stroke(&stroke, Affine::IDENTITY, Brush::Solid(stroke_color), None, &rrect);
                } else {
                    let rect = Rect::new(x0, y0, x1, y1);
                    scene.stroke(&stroke, Affine::IDENTITY, Brush::Solid(stroke_color), None, &rect);
                }
            }
        }

        // Render text
        let dynamic_text = node
            .properties
            .get("text")
            .or_else(|| node.properties.get("content"))
            .and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(format!("{n}")),
                Value::Bool(b) => Some(format!("{b}")),
                _ => None,
            });
        let active_text = dynamic_text.as_ref().or(node.text_content.as_ref());
        if let Some(text) = active_text {
            if !text.is_empty() {
                let font_size = node
                    .properties
                    .get("font_size")
                    .or_else(|| node.properties.get("size"))
                    .and_then(|v| v.as_f64())
                    .or_else(|| {
                        node.font.and_then(|fid| {
                            layout.get_node(fid).and_then(|fn_node| {
                                fn_node
                                    .properties
                                    .get("size")
                                    .or_else(|| fn_node.properties.get("font_size"))
                                    .and_then(|v| v.as_f64())
                            })
                        })
                    })
                    .unwrap_or(16.0) as f32;

                let font_weight = node
                    .properties
                    .get("font_weight")
                    .or_else(|| node.properties.get("weight"))
                    .and_then(|v| v.as_f64())
                    .or_else(|| {
                        node.font.and_then(|fid| {
                            layout.get_node(fid).and_then(|fn_node| {
                                fn_node
                                    .properties
                                    .get("weight")
                                    .or_else(|| fn_node.properties.get("font_weight"))
                                    .and_then(|v| v.as_f64())
                            })
                        })
                    })
                    .unwrap_or(400.0) as f32;

                let font_family = node
                    .properties
                    .get("font_family")
                    .or_else(|| node.properties.get("family"))
                    .and_then(|v| v.as_str())
                    .or_else(|| {
                        node.properties.get("font").and_then(|v| match v {
                            Value::String(s) => Some(s.as_str()),
                            _ => None,
                        })
                    })
                    .or_else(|| {
                        node.font.and_then(|fid| {
                            layout.get_node(fid).and_then(|fn_node| {
                                fn_node
                                    .properties
                                    .get("family")
                                    .or_else(|| fn_node.properties.get("font"))
                                    .and_then(|v| v.as_str())
                            })
                        })
                    });

                let text_color = node
                    .properties
                    .get("color")
                    .or_else(|| node.properties.get("text_color"))
                    .map(|v| match v {
                        Value::Color(c) | Value::String(c) => parse_color(c),
                        _ => Color::BLACK,
                    })
                    .unwrap_or(Color::BLACK);

                let mut builder = text_layout_cx.ranged_builder(font_cx, text, 1.0, true);
                builder.push_default(StyleProperty::FontSize(font_size));
                builder.push_default(StyleProperty::Brush(color_to_rgba8(&text_color)));
                if (font_weight - 400.0).abs() > 1.0 {
                    builder.push_default(StyleProperty::FontWeight(FontWeight::new(font_weight)));
                }
                if let Some(family) = font_family {
                    builder.push_default(StyleProperty::FontFamily(FontFamily::named(family)));
                }

                // Push span styles (colors & underlines) to range builder
                for span in &node.text_spans {
                    if let Some(c_str) = &span.style.color {
                        let c = parse_color(c_str);
                        builder.push(StyleProperty::Brush(color_to_rgba8(&c)), span.range.clone());
                    }
                    if span.style.underline {
                        builder.push(StyleProperty::Underline(true), span.range.clone());
                    }
                }

                let mut layout = builder.build(text);
                if node.rect.width > 0.0 {
                    layout.break_all_lines(Some(node.rect.width as f32));
                }

                let align_str = node
                    .properties
                    .get("align")
                    .or_else(|| node.properties.get("text_align"))
                    .and_then(|v| v.as_str());

                let alignment = match align_str {
                    Some("center") | Some("Center") => Alignment::Center,
                    Some("right") | Some("Right") | Some("end") | Some("End") => Alignment::End,
                    Some("justify") | Some("Justify") => Alignment::Justify,
                    _ => Alignment::Start,
                };
                layout.align(alignment, AlignmentOptions::default());

                // Render lines and glyphs
                for line in layout.lines() {
                    for item in line.items() {
                        if let PositionedLayoutItem::GlyphRun(glyph_run) = item {
                            let run = glyph_run.run();
                            let font = run.font();
                            let brush_u8 = glyph_run.style().brush;
                            let run_color = Color::from_rgba8(brush_u8[0], brush_u8[1], brush_u8[2], brush_u8[3]);

                            let glyphs = glyph_run.positioned_glyphs().map(|g| vello::Glyph {
                                id: g.id,
                                x: g.x,
                                y: g.y,
                            });
                            scene
                                .draw_glyphs(font)
                                .font_size(run.font_size())
                                .transform(Affine::translate((node.rect.x, node.rect.y)))
                                .brush(Brush::Solid(run_color))
                                .draw(Fill::NonZero, glyphs);

                            // Draw underline if requested by span/style
                            if glyph_run.style().underline.is_some() {
                                let mut min_gx = f32::MAX;
                                let mut max_gx = f32::MIN;
                                let mut gy = 0.0;
                                for g in glyph_run.positioned_glyphs() {
                                    if g.x < min_gx {
                                        min_gx = g.x;
                                    }
                                    let end_x = g.x + g.advance;
                                    if end_x > max_gx {
                                        max_gx = end_x;
                                    }
                                    gy = g.y;
                                }

                                if min_gx < max_gx {
                                    let underline_rect = vello::kurbo::Rect::new(
                                        node.rect.x + min_gx as f64,
                                        node.rect.y + gy as f64 + 2.0,
                                        node.rect.x + max_gx as f64,
                                        node.rect.y + gy as f64 + 3.2,
                                    );
                                    scene.fill(
                                        Fill::NonZero,
                                        Affine::IDENTITY,
                                        Brush::Solid(run_color),
                                        None,
                                        &underline_rect,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    while !active_clip_stack.is_empty() {
        scene.pop_layer();
        active_clip_stack.pop();
    }

    let scale = if options.scale_factor > 0.0 {
        options.scale_factor
    } else {
        1.0
    };
    if (scale - 1.0).abs() > 0.001 {
        let mut scaled_scene = Scene::new();
        scaled_scene.append(&scene, Some(Affine::scale(scale)));
        scaled_scene
    } else {
        scene
    }
}
