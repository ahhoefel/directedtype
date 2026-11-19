use std::collections::HashSet;

use krilla::geom::{Path, PathBuilder, Rect, Transform};
use krilla::page::Page;
use krilla::paint::{Fill, FillRule, LineCap, Stroke};
use krilla::surface::Surface;

use crate::compiler::expanded::NodeId;
use crate::compiler::layout::{extract_corner_radii, ResolvedLayout, ResolvedNode};
use crate::compiler::value::Value;
use crate::render::color::{parse_color_rgba8, Rgba8};

use super::links::{emit_page_links, AnchorIndex};
use super::shadow::{clamp_radii, draw_rect_shadow};
use super::text::{draw_text_node, is_text_primitive, PdfFontCache};
use super::{norm_f32, rgba_to_krilla, PageScene, PdfExportError};

/// Builds a closed `krilla::geom::Path` for a rectangle with per-corner `(tl, tr, br, bl)` radii.
pub(super) fn build_rect_path(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    radii: (f64, f64, f64, f64),
) -> Option<Path> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }

    let (tl, tr, br, bl) = clamp_radii(w, h, radii);
    if tl <= 1e-6 && tr <= 1e-6 && br <= 1e-6 && bl <= 1e-6 {
        let rect = Rect::from_xywh(x as f32, y as f32, w as f32, h as f32)?;
        let mut pb = PathBuilder::new();
        pb.push_rect(rect);
        return pb.finish();
    }

    // Standard quarter-circle cubic Bezier constant kappa = 4*(sqrt(2)-1)/3
    const KAPPA: f64 = 0.5522847498307936;
    let x0 = x;
    let y0 = y;
    let x1 = x + w;
    let y1 = y + h;

    let mut pb = PathBuilder::new();
    pb.move_to((x0 + tl) as f32, y0 as f32);
    pb.line_to((x1 - tr) as f32, y0 as f32);
    if tr > 1e-6 {
        let k = tr * KAPPA;
        pb.cubic_to(
            (x1 - tr + k) as f32,
            y0 as f32,
            x1 as f32,
            (y0 + tr - k) as f32,
            x1 as f32,
            (y0 + tr) as f32,
        );
    }

    pb.line_to(x1 as f32, (y1 - br) as f32);
    if br > 1e-6 {
        let k = br * KAPPA;
        pb.cubic_to(
            x1 as f32,
            (y1 - br + k) as f32,
            (x1 - br + k) as f32,
            y1 as f32,
            (x1 - br) as f32,
            y1 as f32,
        );
    }

    pb.line_to((x0 + bl) as f32, y1 as f32);
    if bl > 1e-6 {
        let k = bl * KAPPA;
        pb.cubic_to(
            (x0 + bl - k) as f32,
            y1 as f32,
            x0 as f32,
            (y1 - bl + k) as f32,
            x0 as f32,
            (y1 - bl) as f32,
        );
    }

    pb.line_to(x0 as f32, (y0 + tl) as f32);
    if tl > 1e-6 {
        let k = tl * KAPPA;
        pb.cubic_to(
            x0 as f32,
            (y0 + tl - k) as f32,
            (x0 + tl - k) as f32,
            y0 as f32,
            (x0 + tl) as f32,
            y0 as f32,
        );
    }

    pb.close();
    pb.finish()
}

/// Resolves the ancestor `\Clip` chain from root clip down to `clip_id`.
pub(super) fn get_clip_chain(clip_id: Option<NodeId>, layout: &ResolvedLayout) -> Vec<NodeId> {
    let mut chain = Vec::new();
    let mut curr = clip_id;
    let mut visited = HashSet::new();

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

    chain.reverse();
    chain
}

struct PdfClipEntry {
    clip_id: NodeId,
    has_scroll_transform: bool,
}

fn pop_clip_entry(entry: PdfClipEntry, surface: &mut Surface<'_>) {
    if entry.has_scroll_transform {
        surface.pop();
    }
    surface.pop();
}

fn is_inline_descendant(node: &ResolvedNode, layout: &ResolvedLayout) -> bool {
    let mut curr_parent = node.parent;
    while let Some(pid) = curr_parent {
        if let Some(pnode) = layout.get_node(pid) {
            if pnode.is_inline_box {
                return false;
            }
            if pnode.name == "Text" {
                return true;
            }
            curr_parent = pnode.parent;
        } else {
            return false;
        }
    }
    false
}

/// Lowers one `PageScene` onto a `krilla::page::Page` in top-down PDF user-space coordinates.
pub(super) fn render_page_scene(
    page_scene: &PageScene,
    page: &mut Page<'_>,
    font_cache: &mut PdfFontCache,
    anchors: &AnchorIndex,
    strict_links: bool,
) -> Result<(), PdfExportError> {
    {
        let mut surface = page.surface();
        let has_dpi_scale = (page_scene.dpi_scale - 1.0).abs() > 1e-6;
        if has_dpi_scale {
            surface.push_transform(&Transform::from_scale(
                page_scene.dpi_scale,
                page_scene.dpi_scale,
            ));
        }

        let layout = &page_scene.layout;
        let mut clip_stack: Vec<PdfClipEntry> = Vec::new();

        for node in page_scene.render_order() {
            if !node.is_paint_primitive() || is_inline_descendant(node, layout) {
                continue;
            }

            let target_chain = get_clip_chain(node.clip, layout);
            let common_len = clip_stack
                .iter()
                .map(|e| e.clip_id)
                .zip(&target_chain)
                .take_while(|(a, b)| a == *b)
                .count();

            while clip_stack.len() > common_len {
                let popped = clip_stack.pop().unwrap();
                pop_clip_entry(popped, &mut surface);
            }

            for &clip_node_id in &target_chain[common_len..] {
                let clip_off = page_scene
                    .node_offsets
                    .get(&clip_node_id)
                    .copied()
                    .unwrap_or((0.0, 0.0));
                let (cx, cy, cw, ch, c_radii) = if let Some(box_node_id) =
                    layout.get_value(clip_node_id, "box").and_then(|v| v.as_node())
                {
                    let get_box_val = |port: &str| -> f64 {
                        layout
                            .get_value(box_node_id, port)
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0)
                    };
                    let radii = layout
                        .get_node(box_node_id)
                        .map(|bn| extract_corner_radii(&bn.properties))
                        .unwrap_or((0.0, 0.0, 0.0, 0.0));
                    (
                        get_box_val("x") + clip_off.0,
                        get_box_val("y") + clip_off.1,
                        get_box_val("width"),
                        get_box_val("height"),
                        radii,
                    )
                } else {
                    (clip_off.0, clip_off.1, 0.0, 0.0, (0.0, 0.0, 0.0, 0.0))
                };

                let clip_path = build_rect_path(cx, cy, cw.max(0.001), ch.max(0.001), c_radii)
                    .unwrap_or_else(|| {
                        let mut pb = PathBuilder::new();
                        pb.push_rect(Rect::from_xywh(cx as f32, cy as f32, 0.001, 0.001).unwrap());
                        pb.finish().unwrap()
                    });
                surface.push_clip_path(&clip_path, &FillRule::NonZero);

                let scroll_x = layout
                    .get_value(clip_node_id, "scroll_x")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let scroll_y = layout
                    .get_value(clip_node_id, "scroll_y")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let has_scroll_transform = scroll_x.abs() > 1e-6 || scroll_y.abs() > 1e-6;
                if has_scroll_transform {
                    surface.push_transform(&Transform::from_translate(
                        -scroll_x as f32,
                        -scroll_y as f32,
                    ));
                }

                clip_stack.push(PdfClipEntry {
                    clip_id: clip_node_id,
                    has_scroll_transform,
                });
            }

            let node_offset = page_scene
                .node_offsets
                .get(&node.id)
                .copied()
                .unwrap_or((0.0, 0.0));

            let opacity_opt = node
                .properties
                .get("opacity")
                .and_then(|v| v.as_f64())
                .map(|op| op.clamp(0.0, 1.0));
            let pushed_opacity = if let Some(op) = opacity_opt {
                if op < 1.0 - 1e-6 {
                    surface.push_opacity(norm_f32(op));
                    true
                } else {
                    false
                }
            } else {
                false
            };

            if node.name == "Rect" && node.rect.width > 0.0 && node.rect.height > 0.0 {
                let x0 = node.rect.x + node_offset.0;
                let y0 = node.rect.y + node_offset.1;
                let x1 = x0 + node.rect.width;
                let y1 = y0 + node.rect.height;
                let radii = extract_corner_radii(&node.properties);

                draw_rect_shadow(&mut surface, &node.properties, x0, y0, x1, y1, radii);

                if let Some(rect_path) =
                    build_rect_path(x0, y0, node.rect.width, node.rect.height, radii)
                {
                    let fill_rgba = match node.properties.get("color") {
                        Some(Value::Color(c) | Value::String(c)) => parse_color_rgba8(c),
                        _ => Rgba8::from_rgba8(0, 0, 0, 0),
                    };

                    if fill_rgba.a > 0 {
                        let (fill_rgb, fill_alpha) = rgba_to_krilla(fill_rgba);
                        surface.set_fill(Some(Fill {
                            paint: fill_rgb.into(),
                            opacity: fill_alpha,
                            rule: FillRule::NonZero,
                        }));
                        surface.set_stroke(None);
                        surface.draw_path(&rect_path);
                    }

                    let border_width = node
                        .properties
                        .get("border_width")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);
                    if border_width > 0.0 {
                        let border_rgba = node
                            .properties
                            .get("border_color")
                            .map(|v| match v {
                                Value::Color(c) | Value::String(c) => parse_color_rgba8(c),
                                _ => Rgba8::from_rgba8(0, 0, 0, 255),
                            })
                            .unwrap_or_else(|| Rgba8::from_rgba8(0, 0, 0, 255));
                        if border_rgba.a > 0 || pushed_opacity {
                            let (border_rgb, border_alpha) = rgba_to_krilla(border_rgba);
                            surface.set_fill(None);
                            surface.set_stroke(Some(Stroke {
                                paint: border_rgb.into(),
                                width: border_width as f32,
                                opacity: border_alpha,
                                ..Default::default()
                            }));
                            surface.draw_path(&rect_path);
                        }
                    }
                }
            }

            if node.name == "Line" {
                let x1 = node
                    .properties
                    .get("x1")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(node.rect.x)
                    + node_offset.0;
                let y1 = node
                    .properties
                    .get("y1")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(node.rect.y)
                    + node_offset.1;
                let x2 = node
                    .properties
                    .get("x2")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(node.rect.x + node.rect.width)
                    + node_offset.0;
                let y2 = node
                    .properties
                    .get("y2")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(node.rect.y + node.rect.height)
                    + node_offset.1;
                let thickness = node
                    .properties
                    .get("thickness")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1.0);

                if thickness > 0.0 {
                    let line_rgba = match node.properties.get("color") {
                        Some(Value::Color(c) | Value::String(c)) => parse_color_rgba8(c),
                        _ => Rgba8::from_rgba8(0, 0, 0, 255),
                    };
                    if line_rgba.a > 0 || pushed_opacity {
                        let line_cap = match node.properties.get("cap") {
                            Some(Value::Enum { variant, .. }) => match variant.as_str() {
                                "Round" | "round" => LineCap::Round,
                                "Square" | "square" => LineCap::Square,
                                _ => LineCap::Butt,
                            },
                            Some(Value::String(s)) => match s.to_ascii_lowercase().as_str() {
                                "round" => LineCap::Round,
                                "square" => LineCap::Square,
                                _ => LineCap::Butt,
                            },
                            _ => LineCap::Butt,
                        };
                        let mut pb = PathBuilder::new();
                        pb.move_to(x1 as f32, y1 as f32);
                        pb.line_to(x2 as f32, y2 as f32);
                        if let Some(line_path) = pb.finish() {
                            let (line_rgb, line_alpha) = rgba_to_krilla(line_rgba);
                            surface.set_fill(None);
                            surface.set_stroke(Some(Stroke {
                                paint: line_rgb.into(),
                                width: thickness as f32,
                                line_cap,
                                opacity: line_alpha,
                                ..Default::default()
                            }));
                            surface.draw_path(&line_path);
                        }
                    }
                }
            }

            if is_text_primitive(node, layout) {
                let slice_spec = page_scene.text_line_slices.get(&node.id);
                draw_text_node(
                    &mut surface,
                    node,
                    layout,
                    font_cache,
                    node_offset,
                    slice_spec,
                );
            }

            if pushed_opacity {
                surface.pop();
            }
        }

        while let Some(popped) = clip_stack.pop() {
            pop_clip_entry(popped, &mut surface);
        }

        if has_dpi_scale {
            surface.pop();
        }

        surface.finish();
    }

    emit_page_links(page_scene, page, anchors, strict_links)
        .map_err(PdfExportError::Render)?;

    Ok(())
}
