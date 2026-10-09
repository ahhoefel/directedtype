use parley::style::{FontFamily, FontWeight, StyleProperty};
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
    ends_at: Option<&str>,
    start_at: Option<&str>,
    line_height: Option<f64>,
) -> f64 {
    let (_, height) = measure_text_bounds(text, font_size, font_weight, font_family, Some(max_width), ends_at, start_at, line_height);
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
    if let Some(s) = start_at {
        if s.eq_ignore_ascii_case("ascender") {
            return 0.0;
        }
    }
    // Default to Capital
    let m = measure_font_metrics(font_size, font_weight, font_family);
    -(m.ascent - m.cap_height)
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
    ends_at: Option<&str>,
    start_at: Option<&str>,
    line_height: Option<f64>,
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

            let extra_top_leading = if line_height.is_some() {
                (first_baseline - m.baseline).max(0.0)
            } else {
                0.0
            };

            let top_coord = match start_at {
                Some(s) if s.eq_ignore_ascii_case("ascender") => {
                    extra_top_leading
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

            let bottom_coord = match ends_at {
                Some(s) if s.eq_ignore_ascii_case("descender") => {
                    if let Some(last_line) = last_line {
                        let m = last_line.metrics();
                        (m.baseline + m.descent) as f64
                    } else {
                        layout.height() as f64
                    }
                }
                Some(s) if s.eq_ignore_ascii_case("line_height") || s.eq_ignore_ascii_case("full") => {
                    layout.height() as f64
                }
                Some(s) if s.eq_ignore_ascii_case("baseline") => {
                    if let Some(last_line) = last_line {
                        let m = last_line.metrics();
                        m.baseline as f64
                    } else {
                        layout.height() as f64
                    }
                }
                _ => {
                    // Default to BASELINE when unspecified
                    if let Some(last_line) = last_line {
                        let m = last_line.metrics();
                        m.baseline as f64
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
    pub line_height: f64,
    pub baseline: f64,
}

/// Measures typographic vertical landmarks for a font given size, weight, and family.
pub fn measure_font_metrics(
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
) -> FontMetrics {
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

            let (ascent, descent, leading, baseline) = if let Some(line) = layout.lines().next() {
                let m = line.metrics();
                (m.ascent as f64, m.descent as f64, m.leading as f64, m.baseline as f64)
            } else {
                (size as f64 * 0.8, size as f64 * 0.25, 0.0, size as f64 * 0.8)
            };

            let cap_height = size as f64 * 0.71;
            let x_height = size as f64 * 0.52;
            let line_height = ascent + descent + leading;

            FontMetrics {
                size: size as f64,
                weight: weight as f64,
                cap_height,
                x_height,
                ascent,
                descent,
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
    let mut result: HashMap<NodeId, Vec<Rect>> = HashMap::new();
    if text.is_empty() || spans.is_empty() {
        return result;
    }

    let size = if font_size > 0.0 { font_size as f32 } else { 16.0 };
    let weight = if font_weight > 0.0 { font_weight as f32 } else { 400.0 };

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
        for (idx, span) in spans.iter().enumerate() {
            if !span.range.is_empty() {
                let brush_id = ((idx + 1) % 250 + 1) as u8;
                builder.push(StyleProperty::Brush([brush_id, 0, 0, 255]), span.range.clone());
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

        for line in layout.lines() {
            let line_metrics = line.metrics();
            let line_top = origin_y + line_metrics.block_min_coord as f64;
            let line_height = (line_metrics.block_max_coord - line_metrics.block_min_coord) as f64;

            for (idx, span) in spans.iter().enumerate() {
                if let Some(child_id) = span.node_id {
                    let expected_brush_id = ((idx + 1) % 250 + 1) as u8;
                    let mut min_gx = f32::MAX;
                    let mut max_gx = f32::MIN;

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

                    if min_gx < max_gx {
                        let frag_rect = Rect::new(
                            origin_x + min_gx as f64,
                            line_top,
                            (max_gx - min_gx) as f64,
                            line_height,
                        );
                        result.entry(child_id).or_default().push(frag_rect);
                    }
                }
            }
        }
    });

    // Handle zero-width inline bookmark spans (e.g. \Anchor("bookmark") inside \Text)
    for span in spans {
        if span.range.is_empty() {
            if let Some(child_id) = span.node_id {
                result.entry(child_id).or_insert_with(|| {
                    let char_offset = span.range.start;
                    let prefix_text = &text[0..char_offset.min(text.len())];
                    let (px, py) = measure_text_bounds(prefix_text, font_size, font_weight, font_family, max_width, None, None, None);
                    let frag_rect = Rect::new(origin_x + px, origin_y + py, 0.0, font_size);
                    vec![frag_rect]
                });
            }
        }
    }

    result
}

/// A styled range of text within a parent text element.
#[derive(Debug, Clone, PartialEq)]
pub struct TextSpan {
    /// Byte range within the parent node's concatenated `text_content`.
    pub range: Range<usize>,
    /// Associated inline component (e.g. NodeId of \Link), if this span corresponds to a node.
    pub node_id: Option<NodeId>,
    /// Typographic style overrides for this range.
    pub style: SpanStyle,
}

impl TextSpan {
    pub fn new(range: Range<usize>) -> Self {
        Self {
            range,
            node_id: None,
            style: SpanStyle::default(),
        }
    }

    pub fn with_node(mut self, node_id: NodeId) -> Self {
        self.node_id = Some(node_id);
        self
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
    pub font_size: Option<f64>,
    pub font_weight: Option<f64>,
    pub font_family: Option<String>,
    pub underline: bool,
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

