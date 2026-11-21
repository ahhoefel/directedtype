use std::collections::HashMap;
use std::ops::Range;

use krilla::geom::{PathBuilder, Point, Rect};
use krilla::paint::{Fill, FillRule};
use krilla::surface::Surface;
use krilla::text::{Font as KrillaFont, GlyphId, KrillaGlyph, Tag as KrillaTag};
use parley::layout::{AlignmentOptions, PositionedLayoutItem};
use parley::style::{FontFamily, FontStyle, FontWeight, StyleProperty};
use parley::{Alignment, FontContext, LayoutContext};
use skrifa::MetadataProvider;

use crate::compiler::layout::{ResolvedLayout, ResolvedNode};
use crate::compiler::text::create_font_context;
use crate::compiler::value::Value;
use crate::render::color::{parse_color_rgba8, Rgba8};

use super::lower::build_rect_path;
use super::{rgba_to_krilla, TextSliceSpec};

/// Per-document font & shaping cache shared across all pages of a PDF export.
pub struct PdfFontCache {
    pub font_cx: FontContext,
    pub layout_cx: LayoutContext<[u8; 4]>,
    fonts: HashMap<(usize, usize, u32, Vec<i16>), KrillaFont>,
}

impl Default for PdfFontCache {
    fn default() -> Self {
        Self::new()
    }
}

impl PdfFontCache {
    pub fn new() -> Self {
        Self {
            font_cx: create_font_context(),
            layout_cx: LayoutContext::new(),
            fonts: HashMap::new(),
        }
    }

    pub fn get_or_load(
        &mut self,
        font_data: &parley::FontData,
        normalized_coords: &[i16],
    ) -> Option<KrillaFont> {
        let bytes = font_data.data.as_ref();
        let key = (
            bytes.as_ptr() as usize,
            bytes.len(),
            font_data.index,
            normalized_coords.to_vec(),
        );
        if let Some(cached) = self.fonts.get(&key) {
            return Some(cached.clone());
        }

        let krilla_data: krilla::Data = bytes.to_vec().into();
        let loaded = if normalized_coords.iter().any(|&c| c != 0) {
            let var_coords =
                normalized_to_user_coords(bytes, font_data.index, normalized_coords);
            if var_coords.is_empty() {
                KrillaFont::new(krilla_data, font_data.index)
            } else {
                KrillaFont::new_variable(krilla_data, font_data.index, &var_coords)
                    .or_else(|| KrillaFont::new(bytes.to_vec().into(), font_data.index))
            }
        } else {
            KrillaFont::new(krilla_data, font_data.index)
        }?;

        self.fonts.insert(key, loaded.clone());
        Some(loaded)
    }
}

fn normalized_to_user_coords(
    font_bytes: &[u8],
    index: u32,
    normalized_coords: &[i16],
) -> Vec<(KrillaTag, f32)> {
    let Ok(font_ref) = skrifa::FontRef::from_index(font_bytes, index) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, axis) in font_ref.axes().iter().enumerate() {
        let Some(&norm_raw) = normalized_coords.get(i) else {
            break;
        };
        let norm = (norm_raw as f32) / 16384.0;
        if norm.abs() < 1e-6 {
            continue;
        }
        let user_val = if norm < 0.0 {
            axis.default_value() + norm * (axis.default_value() - axis.min_value())
        } else {
            axis.default_value() + norm * (axis.max_value() - axis.default_value())
        };
        out.push((KrillaTag::new(&axis.tag().into_bytes()), user_val));
    }
    out
}

/// Checks whether a node should be painted as a `\Text` primitive (matching `scene.rs:707-717`).
pub(super) fn is_text_primitive(node: &ResolvedNode, layout: &ResolvedLayout) -> bool {
    let is_standalone_span_text = node.text_content.is_some()
        && !node
            .parent
            .and_then(|pid| layout.get_node(pid))
            .is_some_and(|p| p.text_content.is_some());
    node.name == "Text"
        || is_standalone_span_text
        || (node.children.is_empty() && node.properties.contains_key("text"))
}

/// Lowers a `\Text` primitive (or standalone text span) onto the `krilla` `Surface`.
pub(super) fn draw_text_node(
    surface: &mut Surface<'_>,
    node: &ResolvedNode,
    layout: &ResolvedLayout,
    font_cache: &mut PdfFontCache,
    node_offset: (f64, f64),
    slice_spec: Option<&TextSliceSpec>,
) {
    let (active_text_opt, resolved_spans, resolved_boxes, _) =
        crate::compiler::text::resolve_active_rich_text(node, layout);
    let Some(ref active_text_owned) = active_text_opt else {
        return;
    };
    let active_text = active_text_owned.as_str();
    if active_text.is_empty() && resolved_boxes.is_empty() {
        return;
    }

    let fnode = node
        .font
        .or_else(|| node.properties.get("font").and_then(|v| v.as_node()))
        .and_then(|fid| layout.get_node(fid));

    let font_size = fnode
        .and_then(|f| f.properties.get("size"))
        .and_then(|v| v.as_f64())
        .unwrap_or(16.0) as f32;

    let font_weight = fnode
        .and_then(|f| f.properties.get("weight"))
        .and_then(|v| v.as_f64())
        .unwrap_or(400.0) as f32;

    let font_family = fnode
        .and_then(|f| f.properties.get("family"))
        .and_then(|v| v.as_str());

    let font_italic = fnode
        .and_then(|f| f.properties.get("italic"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let font_letter_spacing = fnode
        .and_then(|f| f.properties.get("letter_spacing"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;

    let text_rgba = fnode
        .and_then(|f| f.properties.get("color"))
        .map(|v| match v {
            Value::Color(c) | Value::String(c) => parse_color_rgba8(c),
            _ => Rgba8::from_rgba8(0, 0, 0, 255),
        })
        .unwrap_or_else(|| Rgba8::from_rgba8(0, 0, 0, 255));

    let align_str = node.properties.get("align").and_then(|v| v.as_str());
    let indent = node
        .properties
        .get("indent")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let start_at_prop = node.properties.get("start_at").and_then(|v| v.as_str());
    let line_height_prop = fnode
        .filter(|f| f.formulas.has_custom_line_height())
        .and_then(|f| f.properties.get("line_height"))
        .and_then(|v| v.as_f64())
        .filter(|&h| h > 0.0);

    let mat = crate::compiler::text::materialize_soft_hyphens_with_font_cx(
        &mut font_cache.font_cx,
        active_text,
        font_size,
        font_weight,
        font_family,
        font_italic,
        line_height_prop,
        (node.rect.width > 0.0).then_some(node.rect.width),
        indent,
        &resolved_spans,
        &resolved_boxes,
    );
    let (active_text, resolved_spans_slice, resolved_boxes_slice) = match &mat {
        Some(m_rt) => (
            m_rt.text.as_str(),
            m_rt.spans.as_slice(),
            m_rt.inline_boxes.as_slice(),
        ),
        None => (
            active_text,
            resolved_spans.as_slice(),
            resolved_boxes.as_slice(),
        ),
    };

    let mut builder =
        font_cache
            .layout_cx
            .ranged_builder(&mut font_cache.font_cx, active_text, 1.0, true);
    builder.push_default(StyleProperty::FontSize(font_size));
    builder.push_default(StyleProperty::Brush(text_rgba.to_array()));
    if (font_weight - 400.0).abs() > 1.0 {
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(font_weight)));
    }
    if let Some(family) = font_family {
        if !family.is_empty() {
            builder.push_default(StyleProperty::FontFamily(FontFamily::from(family)));
        }
    }
    if font_italic {
        builder.push_default(StyleProperty::FontStyle(FontStyle::Italic));
    }
    if font_letter_spacing.abs() > 1e-4 {
        builder.push_default(StyleProperty::LetterSpacing(font_letter_spacing));
    }
    if let Some(lh) = line_height_prop {
        builder.push_default(StyleProperty::LineHeight(
            parley::style::LineHeight::Absolute(lh as f32),
        ));
    }

    for span in resolved_spans_slice {
        if span.range.is_empty() || span.range.end > active_text.len() {
            continue;
        }
        let mut color_str = span.style.color.as_deref();
        let mut underline = span.style.underline;

        if let Some(child_id) = span.node_id {
            if let Some(child_node) = layout.get_node(child_id) {
                if let Some(c) = child_node
                    .properties
                    .get("current_color")
                    .or_else(|| child_node.properties.get("color"))
                    .and_then(|v| v.as_str())
                {
                    color_str = Some(c);
                }
                if let Some(u) = child_node
                    .properties
                    .get("current_underline")
                    .or_else(|| child_node.properties.get("underline"))
                    .and_then(|v| v.as_bool())
                {
                    underline = u;
                }
            }
        }

        if let Some(sz) = span.style.font_size {
            if sz > 0.0 {
                builder.push(StyleProperty::FontSize(sz as f32), span.range.clone());
            }
        }
        if let Some(wt) = span.style.font_weight {
            if wt > 0.0 {
                builder.push(
                    StyleProperty::FontWeight(FontWeight::new(wt as f32)),
                    span.range.clone(),
                );
            }
        }
        if let Some(ref fam) = span.style.font_family {
            if !fam.is_empty() {
                builder.push(
                    StyleProperty::FontFamily(FontFamily::from(fam.as_str())),
                    span.range.clone(),
                );
            }
        }
        if span.style.italic {
            builder.push(
                StyleProperty::FontStyle(FontStyle::Italic),
                span.range.clone(),
            );
        }
        if let Some(c_str) = color_str {
            let c = parse_color_rgba8(c_str);
            builder.push(StyleProperty::Brush(c.to_array()), span.range.clone());
        }
        if underline {
            builder.push(StyleProperty::Underline(true), span.range.clone());
        }
    }

    crate::compiler::text::push_first_line_indent_box(
        &mut builder,
        indent,
        !active_text.is_empty() || !resolved_boxes_slice.is_empty(),
    );

    for ib in resolved_boxes_slice {
        builder.push_inline_box(parley::InlineBox {
            id: ib.node_id.0 as u64,
            kind: parley::InlineBoxKind::InFlow,
            index: ib.index.min(active_text.len()),
            width: ib.width.max(0.0) as f32,
            height: ib.baseline_offset.max(0.0) as f32,
        });
    }

    let mut layout_text = builder.build(active_text);
    if node.rect.width > 0.0 {
        layout_text.break_all_lines(Some(node.rect.width as f32));
    }

    let alignment = match align_str {
        Some("center") | Some("Center") => Alignment::Center,
        Some("right") | Some("Right") | Some("end") | Some("End") => Alignment::End,
        Some("justify") | Some("Justify") | Some("justify_all") | Some("JustifyAll") => {
            Alignment::Justify
        }
        _ => Alignment::Start,
    };
    layout_text.align(alignment, AlignmentOptions::default());

    let m = crate::compiler::text::measure_font_metrics(
        font_size as f64,
        font_weight as f64,
        font_family,
    );
    let first_baseline = layout_text
        .lines()
        .next()
        .map(|l| l.metrics().baseline as f64)
        .unwrap_or(m.baseline);
    let full_y_offset = match start_at_prop {
        Some(s) if s.eq_ignore_ascii_case("ascender") => -(first_baseline - m.ascent),
        Some(s) if s.eq_ignore_ascii_case("emtop") || s.eq_ignore_ascii_case("em_top") => {
            -(first_baseline - m.em_top)
        }
        Some(s) if s.eq_ignore_ascii_case("capital") => -(first_baseline - m.cap_height),
        Some(s)
            if s.eq_ignore_ascii_case("lineheight")
                || s.eq_ignore_ascii_case("line_height")
                || s.eq_ignore_ascii_case("full") =>
        {
            0.0
        }
        _ => -(first_baseline - m.cap_height),
    };

    let origin_x = node.rect.x + node_offset.0;
    let origin_y = match slice_spec {
        Some(spec) if spec.line_range.start > 0 => {
            node.rect.y + node_offset.1 - spec.slice_top_offset
        }
        _ => node.rect.y + node_offset.1 + full_y_offset,
    };

    // Determine active line text byte span when rendering a line slice, so span background highlights
    // can filter to fragments overlapping the active slice's Y coordinates.
    let (active_y_min, active_y_max) = if let Some(spec) = slice_spec {
        let mut y_min = f64::MAX;
        let mut y_max = f64::MIN;
        for (idx, line) in layout_text.lines().enumerate() {
            if spec.line_range.contains(&idx) {
                let lm = line.metrics();
                y_min = y_min.min(lm.block_min_coord as f64);
                y_max = y_max.max(lm.block_max_coord as f64);
            }
        }
        if y_min <= y_max {
            (
                Some(origin_y + y_min - 1.0),
                Some(origin_y + y_max + 1.0),
            )
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    // 1. Pre-glyph span background highlights (`bg` / `current_bg`)
    for span in &resolved_spans {
        if let Some(child_id) = span.node_id {
            if let Some(child_node) = layout.get_node(child_id) {
                let bg_opt = child_node
                    .properties
                    .get("current_bg")
                    .or_else(|| child_node.properties.get("bg"))
                    .and_then(|v| v.as_str())
                    .or(span.style.bg_color.as_deref());
                if let Some(bg_str) = bg_opt {
                    let bg_rgba = parse_color_rgba8(bg_str);
                    if bg_rgba.a > 0 {
                        let (bg_rgb, bg_alpha) = rgba_to_krilla(bg_rgba);
                        for frag in &child_node.fragments {
                            // `frag` is already in un-sliced document layout coordinates (`node.rect.y + full_y_offset + line_coord`).
                            // Translate to active page coordinates cleanly:
                            let fx = frag.x + node_offset.0;
                            let fy = if slice_spec.is_some_and(|s| s.line_range.start > 0) {
                                frag.y - full_y_offset
                                    + (origin_y - node.rect.y)
                            } else {
                                frag.y + node_offset.1
                            };

                            if let (Some(ymin), Some(ymax)) = (active_y_min, active_y_max) {
                                let mid_y = fy + frag.height * 0.5;
                                if mid_y < ymin || mid_y > ymax {
                                    continue;
                                }
                            }

                            if let Some(bg_path) = build_rect_path(
                                fx - 2.0,
                                fy,
                                frag.width + 4.0,
                                frag.height,
                                (4.0, 4.0, 4.0, 4.0),
                            ) {
                                surface.set_fill(Some(Fill {
                                    paint: bg_rgb.into(),
                                    opacity: bg_alpha,
                                    rule: FillRule::NonZero,
                                }));
                                surface.set_stroke(None);
                                surface.draw_path(&bg_path);
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. Render lines & glyph runs with `/ToUnicode` cluster ranges and post-glyph underlines
    let lines_info: Vec<(usize, parley::layout::Line<'_, [u8; 4]>)> =
        layout_text.lines().enumerate().collect();
    let total_lines = lines_info.len();

    for (line_idx, line) in lines_info {
        if let Some(spec) = slice_spec {
            if !spec.line_range.contains(&line_idx) {
                continue;
            }
        }

        let (line_space_extra, line_space_limit) =
            crate::compiler::text::compute_manual_justify_line_spaces(
                &line,
                line_idx,
                total_lines,
                active_text,
                align_str,
                node.rect.width,
            );

        let mut spaces_applied = 0usize;
        let mut line_extra_x = 0.0f32;

        // Track cluster slice offset within each underlying Parley `LineItem::Run` because
        // `line.items()` can split a single `Run` into multiple styled `GlyphRun`s.
        let mut run_cursor: Option<(usize, usize)> = None;

        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };
            let run = glyph_run.run();
            let glyph_count = glyph_run.glyphs().count();
            let start_idx = match run_cursor {
                Some((ri, idx)) if ri == run.index() => idx,
                _ => 0,
            };

            let glyphs_with_ranges: Vec<(parley::layout::Glyph, Range<usize>)> = run
                .visual_clusters()
                .flat_map(|c| {
                    let r = c.text_range();
                    c.glyphs().map(move |g| (g, r.clone()))
                })
                .skip(start_idx)
                .take(glyph_count)
                .collect();
            run_cursor = Some((run.index(), start_idx + glyphs_with_ranges.len()));

            if glyphs_with_ranges.is_empty() {
                continue;
            }

            let run_font_size = run.font_size().max(0.01);
            let Some(krilla_font) = font_cache.get_or_load(run.font(), run.normalized_coords())
            else {
                continue;
            };

            let brush_u8 = glyph_run.style().brush;
            let run_rgba = Rgba8::from_rgba8(brush_u8[0], brush_u8[1], brush_u8[2], brush_u8[3]);
            let (run_rgb, run_alpha) = rgba_to_krilla(run_rgba);

            let run_start_extra_x = line_extra_x;
            let mut krilla_glyphs = Vec::with_capacity(glyphs_with_ranges.len());
            let mut cursor_x = 0.0f32;
            let mut min_gx = f32::MAX;
            let mut max_gx = f32::MIN;
            let mut underline_gy = 0.0f32;

            for (g, cluster_range) in glyphs_with_ranges {
                let cluster_y_shift = resolved_spans_slice
                    .iter()
                    .rfind(|s| {
                        !s.range.is_empty()
                            && s.range.start <= cluster_range.start
                            && cluster_range.end <= s.range.end
                            && s.style.baseline_shift != 0.0
                    })
                    .map(|s| s.style.baseline_shift as f32)
                    .unwrap_or(0.0);

                let is_internal_space = spaces_applied < line_space_limit
                    && active_text
                        .get(cluster_range.clone())
                        .is_some_and(|s| !s.is_empty() && s.chars().all(|ch| ch == ' ' || ch == '\u{00A0}'));
                let extra_adv = if is_internal_space {
                    spaces_applied += 1;
                    line_extra_x += line_space_extra;
                    line_space_extra
                } else {
                    0.0
                };
                let eff_advance = g.advance + extra_adv;

                let gx_abs =
                    origin_x as f32 + glyph_run.offset() + run_start_extra_x + cursor_x + g.x;
                let gx_end =
                    origin_x as f32 + glyph_run.offset() + run_start_extra_x + cursor_x + eff_advance;
                if gx_abs < min_gx {
                    min_gx = gx_abs;
                }
                if gx_end > max_gx {
                    max_gx = gx_end;
                }
                underline_gy = origin_y as f32 + glyph_run.baseline() + g.y + cluster_y_shift;
                cursor_x += eff_advance;

                krilla_glyphs.push(KrillaGlyph::new(
                    GlyphId::new(g.id),
                    eff_advance / run_font_size,
                    g.x / run_font_size,
                    -(g.y + cluster_y_shift) / run_font_size,
                    0.0,
                    cluster_range,
                    None,
                ));
            }

            let start = Point::from_xy(
                origin_x as f32 + glyph_run.offset() + run_start_extra_x,
                origin_y as f32 + glyph_run.baseline(),
            );

            surface.set_fill(Some(Fill {
                paint: run_rgb.into(),
                opacity: run_alpha,
                rule: FillRule::NonZero,
            }));
            surface.set_stroke(None);
            surface.draw_glyphs(
                start,
                &krilla_glyphs,
                krilla_font,
                active_text,
                run_font_size,
                false,
            );

            // Post-glyph underline matching `scene.rs:1035-1065`
            if glyph_run.style().underline.is_some() && min_gx < max_gx {
                if let Some(ul_rect) = Rect::from_ltrb(
                    min_gx,
                    underline_gy + 2.0,
                    max_gx,
                    underline_gy + 3.2,
                ) {
                    let mut pb = PathBuilder::new();
                    pb.push_rect(ul_rect);
                    if let Some(ul_path) = pb.finish() {
                        surface.set_fill(Some(Fill {
                            paint: run_rgb.into(),
                            opacity: run_alpha,
                            rule: FillRule::NonZero,
                        }));
                        surface.set_stroke(None);
                        surface.draw_path(&ul_path);
                    }
                }
            }
        }
    }
}
