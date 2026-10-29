use parley::style::{FontFamily, FontStyle, FontWeight, StyleProperty};
use parley::{FontContext, LayoutContext};
use std::cell::RefCell;

thread_local! {
    static FONT_CONTEXT: RefCell<FontContext> = RefCell::new(FontContext::new());
    static LAYOUT_CONTEXT: RefCell<LayoutContext<()>> = RefCell::new(LayoutContext::new());
}

/// Measures the rendered height of the given text when wrapped to `max_width`.
pub fn measure_text_height(
    text: &str,
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
    max_width: f64,
    end_at: Option<&str>,
    start_at: Option<&str>,
    line_height: Option<f64>,
) -> f64 {
    let (_, height) = measure_text_bounds(text, font_size, font_weight, font_family, Some(max_width), end_at, start_at, line_height);
    height
}

/// Measures the natural unconstrained single-line width of the given text.
pub fn measure_text_width(
    text: &str,
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
) -> f64 {
    let (width, _) = measure_text_bounds(text, font_size, font_weight, font_family, None, None, None, None);
    width
}

/// Computes the vertical rendering offset for text glyphs when start_at is specified.
pub fn text_y_offset(
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
    start_at: Option<&str>,
) -> f64 {
    let m = measure_font_metrics(font_size, font_weight, font_family);
    if let Some(s) = start_at {
        if s.eq_ignore_ascii_case("ascender") {
            return -(m.baseline - m.ascent);
        }
        if s.eq_ignore_ascii_case("emtop") || s.eq_ignore_ascii_case("em_top") {
            return -(m.baseline - m.em_top);
        }
    }
    // Default to Capital
    -(m.baseline - m.cap_height)
}

/// Measures the vertical distance from a Text node's top edge (at `start_at`) down to the first baseline.
pub fn text_baseline_offset(
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
    start_at: Option<&str>,
) -> f64 {
    let m = measure_font_metrics(font_size, font_weight, font_family);
    match start_at {
        Some(s) if s.eq_ignore_ascii_case("ascender") => m.ascent,
        Some(s) if s.eq_ignore_ascii_case("emtop") || s.eq_ignore_ascii_case("em_top") => m.em_top,
        Some(s) if s.eq_ignore_ascii_case("capital") => m.cap_height,
        _ => m.cap_height,
    }
}


/// Measures the layout boundaries `(width, height)` of the given text using Parley.
pub fn measure_text_bounds(
    text: &str,
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
    max_width: Option<f64>,
    end_at: Option<&str>,
    start_at: Option<&str>,
    line_height: Option<f64>,
) -> (f64, f64) {
    measure_rich_text_bounds(
        text,
        font_size,
        font_weight,
        font_family,
        max_width,
        end_at,
        start_at,
        line_height,
        &[],
    )
}

/// Measures the layout boundaries `(width, height)` of rich text with styled inline spans using Parley.
pub fn measure_rich_text_bounds(
    text: &str,
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
    max_width: Option<f64>,
    end_at: Option<&str>,
    start_at: Option<&str>,
    line_height: Option<f64>,
    spans: &[TextSpan],
) -> (f64, f64) {
    if text.is_empty() {
        return (0.0, 0.0);
    }

    let size = if font_size > 0.0 { font_size as f32 } else { 16.0 };
    let weight = if font_weight > 0.0 { font_weight as f32 } else { 400.0 };
    let m = measure_font_metrics(font_size, font_weight, font_family);

    FONT_CONTEXT.with(|font_cx_cell| {
        LAYOUT_CONTEXT.with(|layout_cx_cell| {
            let mut font_cx = font_cx_cell.borrow_mut();
            let mut layout_cx = layout_cx_cell.borrow_mut();

            let mut builder = layout_cx.ranged_builder(&mut font_cx, text, 1.0, true);
            builder.push_default(StyleProperty::FontSize(size));
            if (weight - 400.0).abs() > 1.0 {
                builder.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
            }
            if let Some(family) = font_family {
                if !family.is_empty() {
                    builder.push_default(StyleProperty::FontFamily(FontFamily::named(family)));
                }
            }
            if let Some(lh) = line_height {
                if lh > 0.0 {
                    builder.push_default(StyleProperty::LineHeight(parley::style::LineHeight::Absolute(lh as f32)));
                }
            }
            for span in spans {
                if !span.range.is_empty() && span.range.end <= text.len() {
                    if let Some(sz) = span.style.font_size {
                        if sz > 0.0 {
                            builder.push(StyleProperty::FontSize(sz as f32), span.range.clone());
                        }
                    }
                    if let Some(wt) = span.style.font_weight {
                        if wt > 0.0 {
                            builder.push(StyleProperty::FontWeight(FontWeight::new(wt as f32)), span.range.clone());
                        }
                    }
                    if let Some(fam) = &span.style.font_family {
                        if !fam.is_empty() {
                            builder.push(StyleProperty::FontFamily(FontFamily::named(fam)), span.range.clone());
                        }
                    }
                    if span.style.italic {
                        builder.push(StyleProperty::FontStyle(FontStyle::Italic), span.range.clone());
                    }
                }
            }

            let mut layout = builder.build(text);
            layout.break_all_lines(max_width.and_then(|w| if w > 0.0 { Some(w as f32) } else { None }));

            let width = if max_width.is_some() {
                layout.width() as f64
            } else {
                layout.full_width() as f64
            };

            let first_line = layout.lines().next();
            let last_line = layout.lines().last();

            let first_baseline = first_line
                .map(|l| l.metrics().baseline as f64)
                .unwrap_or(m.baseline);

            let top_coord = match start_at {
                Some(s) if s.eq_ignore_ascii_case("ascender") => {
                    first_baseline - m.ascent
                }
                Some(s) if s.eq_ignore_ascii_case("emtop") || s.eq_ignore_ascii_case("em_top") => {
                    first_baseline - m.em_top
                }
                Some(s) if s.eq_ignore_ascii_case("capital") => {
                    first_baseline - m.cap_height
                }
                Some(s) if s.eq_ignore_ascii_case("line_height") || s.eq_ignore_ascii_case("full") => {
                    0.0
                }
                _ => {
                    // Default to Capital when unspecified
                    first_baseline - m.cap_height
                }
            };

            let bottom_coord = match end_at {
                Some(s) if s.eq_ignore_ascii_case("descender") => {
                    if let Some(last_line) = last_line {
                        let lm = last_line.metrics();
                        lm.baseline as f64 + m.descent
                    } else {
                        layout.height() as f64
                    }
                }
                Some(s) if s.eq_ignore_ascii_case("gapbottom") || s.eq_ignore_ascii_case("gap_bottom") => {
                    if let Some(last_line) = last_line {
                        let lm = last_line.metrics();
                        lm.baseline as f64 + m.gap_bottom
                    } else {
                        layout.height() as f64
                    }
                }
                Some(s) if s.eq_ignore_ascii_case("line_height") || s.eq_ignore_ascii_case("full") => {
                    layout.height() as f64
                }
                Some(s) if s.eq_ignore_ascii_case("baseline") => {
                    if let Some(last_line) = last_line {
                        let lm = last_line.metrics();
                        lm.baseline as f64
                    } else {
                        layout.height() as f64
                    }
                }
                _ => {
                    // Default to BASELINE when unspecified
                    if let Some(last_line) = last_line {
                        let lm = last_line.metrics();
                        lm.baseline as f64
                    } else {
                        layout.height() as f64
                    }
                }
            };

            let height = (bottom_coord - top_coord).max(0.0);

            (width, height)
        })
    })
}

/// Font metrics describing vertical typographic landmarks for layout equations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontMetrics {
    pub size: f64,
    pub weight: f64,
    pub cap_height: f64,
    pub x_height: f64,
    pub ascent: f64,
    pub descent: f64,
    pub em_top: f64,
    pub line_gap: f64,
    pub gap_bottom: f64,
    pub line_height: f64,
    pub baseline: f64,
}

/// Measures typographic vertical landmarks for a font given size, weight, and family.
pub fn measure_font_metrics(
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
) -> FontMetrics {
    use skrifa::instance::{LocationRef, Size};
    use skrifa::raw::TableProvider;
    use skrifa::MetadataProvider;

    let size = if font_size > 0.0 { font_size as f32 } else { 16.0 };
    let weight = if font_weight > 0.0 { font_weight as f32 } else { 400.0 };

    FONT_CONTEXT.with(|font_cx_cell| {
        LAYOUT_CONTEXT.with(|layout_cx_cell| {
            let mut font_cx = font_cx_cell.borrow_mut();
            let mut layout_cx = layout_cx_cell.borrow_mut();

            let sample = "Hx";
            let mut builder = layout_cx.ranged_builder(&mut font_cx, sample, 1.0, true);
            builder.push_default(StyleProperty::FontSize(size));
            if (weight - 400.0).abs() > 1.0 {
                builder.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
            }
            if let Some(family) = font_family {
                if !family.is_empty() {
                    builder.push_default(StyleProperty::FontFamily(FontFamily::named(family)));
                }
            }

            let mut layout = builder.build(sample);
            layout.break_all_lines(None);

            let mut ascent = size as f64 * 0.8;
            let mut descent = size as f64 * 0.25;
            let mut line_gap = 0.0;
            let mut baseline = size as f64 * 0.8;
            let mut cap_height = None;
            let mut x_height = None;

            if let Some(line) = layout.lines().next() {
                let m = line.metrics();
                ascent = m.ascent as f64;
                descent = m.descent as f64;
                line_gap = m.leading as f64;
                baseline = m.baseline as f64;

                for item in line.items() {
                    if let parley::layout::PositionedLayoutItem::GlyphRun(glyph_run) = item {
                        let font = glyph_run.run().font();
                        if let Ok(font_ref) =
                            skrifa::FontRef::from_index(font.data.as_ref(), font.index)
                        {
                            let skrifa_size = Size::new(size);
                            let loc = LocationRef::default();
                            let metrics = font_ref.metrics(skrifa_size, loc);
                            let charmap = font_ref.charmap();
                            let glyph_metrics = font_ref.glyph_metrics(skrifa_size, loc);

                            cap_height = metrics
                                .cap_height
                                .filter(|&v| v > 0.0)
                                .or_else(|| {
                                    charmap
                                        .map('H')
                                        .and_then(|gid| glyph_metrics.bounds(gid))
                                        .map(|b| b.y_max)
                                })
                                .map(|v| v as f64);

                            x_height = metrics
                                .x_height
                                .filter(|&v| v > 0.0)
                                .or_else(|| {
                                    charmap
                                        .map('x')
                                        .and_then(|gid| glyph_metrics.bounds(gid))
                                        .map(|b| b.y_max)
                                })
                                .map(|v| v as f64);

                            let upem = metrics.units_per_em as f64;
                            if upem > 0.0 {
                                let scale = (size as f64) / upem;
                                if let Ok(os2) = font_ref.os2() {
                                    let typo_asc = os2.s_typo_ascender() as f64 * scale;
                                    let typo_desc = -(os2.s_typo_descender() as f64) * scale;
                                    let typo_gap = (os2.s_typo_line_gap() as f64 * scale).max(0.0);
                                    if typo_asc > 0.0 {
                                        ascent = typo_asc;
                                    }
                                    if typo_desc > 0.0 {
                                        descent = typo_desc;
                                    }
                                    line_gap = typo_gap;
                                } else {
                                    if let Some(h_top) = charmap
                                        .map('h')
                                        .and_then(|gid| glyph_metrics.bounds(gid))
                                        .map(|b| b.y_max as f64)
                                    {
                                        if h_top > 0.0 {
                                            ascent = h_top;
                                        }
                                    }
                                    if let Some(p_bot) = charmap
                                        .map('p')
                                        .and_then(|gid| glyph_metrics.bounds(gid))
                                        .map(|b| -(b.y_min as f64))
                                    {
                                        if p_bot > 0.0 {
                                            descent = p_bot;
                                        }
                                    }
                                }
                            }
                            break;
                        }
                    }
                }
            }

            let size_f64 = size as f64;
            let cap_height = cap_height.unwrap_or(size_f64 * 0.71);
            let x_height = x_height.unwrap_or(size_f64 * 0.52);
            let em_top = (size_f64 - descent).max(0.0);
            let gap_bottom = descent + line_gap;
            let line_height = em_top + gap_bottom;

            FontMetrics {
                size: size_f64,
                weight: weight as f64,
                cap_height,
                x_height,
                ascent,
                descent,
                em_top,
                line_gap,
                gap_bottom,
                line_height,
                baseline,
            }
        })
    })
}

/// Standard W3C / CSS generic font families that resolve to system fallbacks.
const GENERIC_FONT_FAMILIES: &[&str] = &[
    "sans-serif",
    "serif",
    "monospace",
    "cursive",
    "fantasy",
    "system-ui",
    "ui-serif",
    "ui-sans-serif",
    "ui-monospace",
    "ui-rounded",
    "emoji",
    "math",
    "fangsong",
];

/// Checks whether a font family exists in the system font database or is a standard generic family.
pub fn font_family_exists(family: &str) -> bool {
    let trimmed = family.trim();
    if trimmed.is_empty() {
        return true;
    }
    if GENERIC_FONT_FAMILIES
        .iter()
        .any(|g| g.eq_ignore_ascii_case(trimmed))
    {
        return true;
    }
    FONT_CONTEXT.with(|font_cx_cell| {
        let mut font_cx = font_cx_cell.borrow_mut();
        font_cx.collection.family_id(trimmed).is_some()
    })
}

use crate::compiler::expanded::NodeId;
use crate::compiler::layout::Rect;
use std::collections::HashMap;
use std::ops::Range;

/// Computed layout metrics and line fragments for an inline span.
#[derive(Debug, Clone, PartialEq)]
pub struct SpanLayoutMetrics {
    pub fragments: Vec<Rect>,
    pub rect: Rect,
    pub baseline: f64,
}

/// Measures a rich text layout and returns the line fragment bounding boxes for each span.
pub fn compute_span_fragments(
    text: &str,
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
    max_width: Option<f64>,
    align: Option<&str>,
    spans: &[TextSpan],
    origin_x: f64,
    origin_y: f64,
) -> HashMap<NodeId, Vec<Rect>> {
    compute_span_fragments_with_metrics(
        text,
        font_size,
        font_weight,
        font_family,
        max_width,
        None,
        align,
        spans,
        &HashMap::new(),
        origin_x,
        origin_y,
    )
    .into_iter()
    .map(|(k, v)| (k, v.fragments))
    .collect()
}

/// Measures a rich text layout and returns full `SpanLayoutMetrics` (fragments, union rect, and first-line baseline)
/// for every inline `NodeId` referenced in `spans` (including outer spans in `span.node_ids`).
pub fn compute_span_fragments_with_metrics(
    text: &str,
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
    max_width: Option<f64>,
    line_height: Option<f64>,
    align: Option<&str>,
    spans: &[TextSpan],
    child_shifts: &HashMap<NodeId, f64>,
    origin_x: f64,
    origin_y: f64,
) -> HashMap<NodeId, SpanLayoutMetrics> {
    let mut result: HashMap<NodeId, SpanLayoutMetrics> = HashMap::new();
    if text.is_empty() || spans.is_empty() {
        return result;
    }

    let size = if font_size > 0.0 { font_size as f32 } else { 16.0 };
    let weight = if font_weight > 0.0 { font_weight as f32 } else { 400.0 };
    let m = measure_font_metrics(font_size, font_weight, font_family);

    // Collect all unique NodeIds across `node_id` and `node_ids`
    let mut all_node_ids: Vec<NodeId> = Vec::new();
    for span in spans {
        if let Some(id) = span.node_id {
            if !all_node_ids.contains(&id) {
                all_node_ids.push(id);
            }
        }
        for &id in &span.node_ids {
            if !all_node_ids.contains(&id) {
                all_node_ids.push(id);
            }
        }
    }

    let mut frags_by_node: HashMap<NodeId, Vec<Rect>> = HashMap::new();
    let mut baseline_by_node: HashMap<NodeId, f64> = HashMap::new();
    let mut default_first_baseline = m.baseline;

    FONT_CONTEXT.with(|font_cx_cell| {
        let mut font_cx = font_cx_cell.borrow_mut();
        let mut layout_cx = LayoutContext::<[u8; 4]>::new();

        let mut builder = layout_cx.ranged_builder(&mut font_cx, text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(size));
        builder.push_default(StyleProperty::Brush([0u8, 0u8, 0u8, 255u8]));
        if (weight - 400.0).abs() > 1.0 {
            builder.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
        }
        if let Some(family) = font_family {
            if !family.is_empty() {
                builder.push_default(StyleProperty::FontFamily(FontFamily::named(family)));
            }
        }
        if let Some(lh) = line_height {
            if lh > 0.0 {
                builder.push_default(StyleProperty::LineHeight(parley::style::LineHeight::Absolute(lh as f32)));
            }
        }
        for (idx, span) in spans.iter().enumerate() {
            if !span.range.is_empty() && span.range.end <= text.len() {
                let brush_id = ((idx + 1) % 250 + 1) as u8;
                builder.push(StyleProperty::Brush([brush_id, 0, 0, 255]), span.range.clone());
                if let Some(sz) = span.style.font_size {
                    if sz > 0.0 {
                        builder.push(StyleProperty::FontSize(sz as f32), span.range.clone());
                    }
                }
                if let Some(wt) = span.style.font_weight {
                    if wt > 0.0 {
                        builder.push(StyleProperty::FontWeight(FontWeight::new(wt as f32)), span.range.clone());
                    }
                }
                if let Some(fam) = &span.style.font_family {
                    if !fam.is_empty() {
                        builder.push(StyleProperty::FontFamily(FontFamily::named(fam)), span.range.clone());
                    }
                }
                if span.style.italic {
                    builder.push(StyleProperty::FontStyle(FontStyle::Italic), span.range.clone());
                }
                if span.style.underline {
                    builder.push(StyleProperty::Underline(true), span.range.clone());
                }
            }
        }

        let mut layout = builder.build(text);
        if let Some(w) = max_width {
            if w > 0.0 {
                layout.break_all_lines(Some(w as f32));
            }
        }

        let alignment = match align {
            Some("center") | Some("Center") => parley::Alignment::Center,
            Some("right") | Some("Right") | Some("end") | Some("End") => parley::Alignment::End,
            Some("justify") | Some("Justify") => parley::Alignment::Justify,
            _ => parley::Alignment::Start,
        };
        layout.align(alignment, parley::layout::AlignmentOptions::default());

        if let Some(first_line) = layout.lines().next() {
            default_first_baseline = first_line.metrics().baseline as f64;
        }

        for line in layout.lines() {
            let line_metrics = line.metrics();
            let line_top_base = origin_y + line_metrics.block_min_coord as f64;
            let line_height_val = (line_metrics.block_max_coord - line_metrics.block_min_coord) as f64;
            let line_baseline_base = origin_y + line_metrics.baseline as f64;

            for &child_id in &all_node_ids {
                let own_shift = child_shifts
                    .get(&child_id)
                    .copied()
                    .or_else(|| {
                        spans
                            .iter()
                            .find(|s| s.node_id == Some(child_id) || s.node_ids.contains(&child_id))
                            .map(|s| s.style.baseline_shift)
                    })
                    .unwrap_or(0.0);

                let mut min_gx = f32::MAX;
                let mut max_gx = f32::MIN;

                for (idx, span) in spans.iter().enumerate() {
                    if span.node_id == Some(child_id) || span.node_ids.contains(&child_id) {
                        let expected_brush_id = ((idx + 1) % 250 + 1) as u8;
                        for item in line.items() {
                            if let parley::layout::PositionedLayoutItem::GlyphRun(glyph_run) = item {
                                if glyph_run.style().brush[0] == expected_brush_id {
                                    for g in glyph_run.positioned_glyphs() {
                                        if g.x < min_gx {
                                            min_gx = g.x;
                                        }
                                        let gx_end = g.x + g.advance;
                                        if gx_end > max_gx {
                                            max_gx = gx_end;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if min_gx < max_gx {
                    let frag_rect = Rect::new(
                        origin_x + min_gx as f64,
                        line_top_base + own_shift,
                        (max_gx - min_gx) as f64,
                        line_height_val,
                    );
                    frags_by_node.entry(child_id).or_default().push(frag_rect);
                    baseline_by_node
                        .entry(child_id)
                        .or_insert(line_baseline_base + own_shift);
                }
            }
        }
    });

    // Handle zero-width inline bookmark spans (e.g. \Anchor("bookmark") or empty \Span inside \Text)
    for &child_id in &all_node_ids {
        let own_shift = child_shifts
            .get(&child_id)
            .copied()
            .or_else(|| {
                spans
                    .iter()
                    .find(|s| s.node_id == Some(child_id) || s.node_ids.contains(&child_id))
                    .map(|s| s.style.baseline_shift)
            })
            .unwrap_or(0.0);
        if !frags_by_node.contains_key(&child_id) {
            if let Some(span) = spans
                .iter()
                .find(|s| s.node_id == Some(child_id) || s.node_ids.contains(&child_id))
            {
                let char_offset = span.range.start;
                let prefix_text = &text[0..char_offset.min(text.len())];
                let (px, py) = measure_rich_text_bounds(
                    prefix_text,
                    font_size,
                    font_weight,
                    font_family,
                    max_width,
                    None,
                    None,
                    line_height,
                    spans,
                );
                let frag_rect = Rect::new(origin_x + px, origin_y + py + own_shift, 0.0, font_size);
                frags_by_node.insert(child_id, vec![frag_rect]);
                baseline_by_node.insert(child_id, origin_y + default_first_baseline + own_shift);
            }
        }
    }

    for child_id in all_node_ids {
        if let Some(fragments) = frags_by_node.remove(&child_id) {
            if let Some(rect) = Rect::bounding_union(&fragments) {
                let baseline = baseline_by_node
                    .remove(&child_id)
                    .unwrap_or(origin_y + default_first_baseline);
                result.insert(
                    child_id,
                    SpanLayoutMetrics {
                        fragments,
                        rect,
                        baseline,
                    },
                );
            }
        }
    }

    result
}

/// Computes the post-shaping `SpanLayoutMetrics` for a specific byte range `span_range` inside a rich text block.
pub fn compute_span_range_metrics(
    text: &str,
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
    max_width: Option<f64>,
    start_at: Option<&str>,
    line_height: Option<f64>,
    align: Option<&str>,
    spans: &[TextSpan],
    text_x: f64,
    text_y: f64,
    span_range: Range<usize>,
    own_shift: f64,
) -> SpanLayoutMetrics {
    let y_offset = text_y_offset(font_size, font_weight, font_family, start_at);
    let origin_x = text_x;
    let origin_y = text_y + y_offset;

    if text.is_empty() {
        return SpanLayoutMetrics {
            fragments: Vec::new(),
            rect: Rect::new(text_x, text_y + own_shift, 0.0, 0.0),
            baseline: text_y + text_baseline_offset(font_size, font_weight, font_family, start_at) + own_shift,
        };
    }

    let size = if font_size > 0.0 { font_size as f32 } else { 16.0 };
    let weight = if font_weight > 0.0 { font_weight as f32 } else { 400.0 };
    let m = measure_font_metrics(font_size, font_weight, font_family);

    let mut fragments = Vec::new();
    let mut first_line_baseline: Option<f64> = None;
    let mut default_first_baseline = m.baseline;

    FONT_CONTEXT.with(|font_cx_cell| {
        let mut font_cx = font_cx_cell.borrow_mut();
        let mut layout_cx = LayoutContext::<[u8; 4]>::new();

        let mut builder = layout_cx.ranged_builder(&mut font_cx, text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(size));
        builder.push_default(StyleProperty::Brush([0u8, 0u8, 0u8, 255u8]));
        if (weight - 400.0).abs() > 1.0 {
            builder.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
        }
        if let Some(family) = font_family {
            if !family.is_empty() {
                builder.push_default(StyleProperty::FontFamily(FontFamily::named(family)));
            }
        }
        if let Some(lh) = line_height {
            if lh > 0.0 {
                builder.push_default(StyleProperty::LineHeight(parley::style::LineHeight::Absolute(lh as f32)));
            }
        }
        for (idx, span) in spans.iter().enumerate() {
            if !span.range.is_empty() && span.range.end <= text.len() {
                let brush_id = ((idx + 1) % 250 + 1) as u8;
                builder.push(StyleProperty::Brush([brush_id, 0, 0, 255]), span.range.clone());
                if let Some(sz) = span.style.font_size {
                    if sz > 0.0 {
                        builder.push(StyleProperty::FontSize(sz as f32), span.range.clone());
                    }
                }
                if let Some(wt) = span.style.font_weight {
                    if wt > 0.0 {
                        builder.push(StyleProperty::FontWeight(FontWeight::new(wt as f32)), span.range.clone());
                    }
                }
                if let Some(fam) = &span.style.font_family {
                    if !fam.is_empty() {
                        builder.push(StyleProperty::FontFamily(FontFamily::named(fam)), span.range.clone());
                    }
                }
                if span.style.italic {
                    builder.push(StyleProperty::FontStyle(FontStyle::Italic), span.range.clone());
                }
                if span.style.underline {
                    builder.push(StyleProperty::Underline(true), span.range.clone());
                }
            }
        }

        let mut layout = builder.build(text);
        if let Some(w) = max_width {
            if w > 0.0 {
                layout.break_all_lines(Some(w as f32));
            }
        }

        let alignment = match align {
            Some("center") | Some("Center") => parley::Alignment::Center,
            Some("right") | Some("Right") | Some("end") | Some("End") => parley::Alignment::End,
            Some("justify") | Some("Justify") => parley::Alignment::Justify,
            _ => parley::Alignment::Start,
        };
        layout.align(alignment, parley::layout::AlignmentOptions::default());

        if let Some(first_line) = layout.lines().next() {
            default_first_baseline = first_line.metrics().baseline as f64;
        }

        for line in layout.lines() {
            let line_metrics = line.metrics();
            let line_top_base = origin_y + line_metrics.block_min_coord as f64;
            let line_height_val = (line_metrics.block_max_coord - line_metrics.block_min_coord) as f64;
            let line_baseline_base = origin_y + line_metrics.baseline as f64;

            let mut min_gx = f32::MAX;
            let mut max_gx = f32::MIN;

            for (idx, span) in spans.iter().enumerate() {
                if !span.range.is_empty()
                    && span.range.start >= span_range.start
                    && span.range.end <= span_range.end
                {
                    let expected_brush_id = ((idx + 1) % 250 + 1) as u8;
                    for item in line.items() {
                        if let parley::layout::PositionedLayoutItem::GlyphRun(glyph_run) = item {
                            if glyph_run.style().brush[0] == expected_brush_id {
                                for g in glyph_run.positioned_glyphs() {
                                    if g.x < min_gx {
                                        min_gx = g.x;
                                    }
                                    let gx_end = g.x + g.advance;
                                    if gx_end > max_gx {
                                        max_gx = gx_end;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if min_gx < max_gx {
                fragments.push(Rect::new(
                    origin_x + min_gx as f64,
                    line_top_base + own_shift,
                    (max_gx - min_gx) as f64,
                    line_height_val,
                ));
                if first_line_baseline.is_none() {
                    first_line_baseline = Some(line_baseline_base + own_shift);
                }
            }
        }
    });

    if fragments.is_empty() {
        let char_offset = span_range.start.min(text.len());
        let prefix_text = &text[0..char_offset];
        let (px, py) = measure_rich_text_bounds(
            prefix_text,
            font_size,
            font_weight,
            font_family,
            max_width,
            None,
            None,
            line_height,
            spans,
        );
        let frag_rect = Rect::new(origin_x + px, origin_y + py + own_shift, 0.0, font_size);
        fragments.push(frag_rect);
    }

    let rect = Rect::bounding_union(&fragments)
        .unwrap_or_else(|| Rect::new(text_x, text_y + own_shift, 0.0, 0.0));
    let baseline = first_line_baseline.unwrap_or(origin_y + default_first_baseline + own_shift);

    SpanLayoutMetrics {
        fragments,
        rect,
        baseline,
    }
}

/// A styled range of text within a parent text element.
#[derive(Debug, Clone, PartialEq)]
pub struct TextSpan {
    /// Byte range within the parent node's concatenated `text_content`.
    pub range: Range<usize>,
    /// Innermost inline component (e.g. NodeId of \Link or \Bold), if this span corresponds to a node.
    pub node_id: Option<NodeId>,
    /// Chain of inline component NodeIds covering this range, from innermost to outermost.
    pub node_ids: Vec<NodeId>,
    /// Typographic style overrides for this range.
    pub style: SpanStyle,
}

impl TextSpan {
    pub fn new(range: Range<usize>) -> Self {
        Self {
            range,
            node_id: None,
            node_ids: Vec::new(),
            style: SpanStyle::default(),
        }
    }

    pub fn with_node(mut self, node_id: NodeId) -> Self {
        self.node_id = Some(node_id);
        if !self.node_ids.contains(&node_id) {
            self.node_ids.push(node_id);
        }
        self
    }

    pub fn push_parent_node(&mut self, node_id: NodeId) {
        if self.node_id.is_none() {
            self.node_id = Some(node_id);
        }
        if !self.node_ids.contains(&node_id) {
            self.node_ids.push(node_id);
        }
    }

    pub fn with_style(mut self, style: SpanStyle) -> Self {
        self.style = style;
        self
    }
}

/// Typographic style overrides for a `TextSpan`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpanStyle {
    pub color: Option<String>,
    pub bg_color: Option<String>,
    pub font_size: Option<f64>,
    pub font_weight: Option<f64>,
    pub font_family: Option<String>,
    pub italic: bool,
    pub underline: bool,
    pub baseline_shift: f64,
    pub url: Option<String>,
    pub cursor: Option<CursorKind>,
}

/// Mouse cursor representation for interactive inline elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CursorKind {
    #[default]
    Default,
    Pointer,
    Text,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_real_per_font_metrics() {
        let arial = measure_font_metrics(120.0, 700.0, Some("Arial"));
        let times = measure_font_metrics(120.0, 700.0, Some("Times New Roman"));
        let georgia = measure_font_metrics(120.0, 700.0, Some("Georgia"));
        let courier = measure_font_metrics(120.0, 700.0, Some("Courier New"));

        // Each font should report its own distinct cap_height and x_height from OpenType tables
        assert!((arial.cap_height - times.cap_height).abs() > 1.0);
        assert!((georgia.cap_height - courier.cap_height).abs() > 1.0);
        assert!((arial.x_height - times.x_height).abs() > 1.0);

        // Typographic ascent + descent fits within the 1em (120px) body square with shoulder clearance
        for m in [arial, times, georgia, courier] {
            assert!(m.cap_height > m.x_height);
            assert!(m.ascent >= m.cap_height);
            assert!(m.descent > 0.0);
            assert!(m.ascent + m.descent <= m.size);
        }
    }
}

