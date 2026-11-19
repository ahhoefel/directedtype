use parley::style::{FontFamily, FontStyle, FontWeight, StyleProperty};
use parley::{FontContext, LayoutContext};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

struct SharedFontStore {
    font_cx: FontContext,
    registered_dirs: HashSet<PathBuf>,
}

static SHARED_FONT_CONTEXT: OnceLock<Mutex<SharedFontStore>> = OnceLock::new();
static PREWARM_STARTED: AtomicBool = AtomicBool::new(false);
static FONT_GENERATION: AtomicU64 = AtomicU64::new(1);

fn init_shared_font_store() -> Mutex<SharedFontStore> {
    let mut store = SharedFontStore {
        font_cx: FontContext::new(),
        registered_dirs: HashSet::new(),
    };
    let _ = register_font_dir_into(&mut store, Path::new("fonts"));
    Mutex::new(store)
}

fn register_font_dir_into(store: &mut SharedFontStore, dir: &Path) -> bool {
    let Ok(canon) = std::fs::canonicalize(dir) else {
        return false;
    };
    if !canon.is_dir() || !store.registered_dirs.insert(canon.clone()) {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(&canon) else {
        return false;
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc"))
                .unwrap_or(false)
        })
        .collect();
    paths.sort();
    let mut loaded_any = false;
    for path in paths {
        if let Ok(bytes) = std::fs::read(&path) {
            let blob = parley::fontique::Blob::new(Arc::new(bytes));
            store.font_cx.collection.register_fonts(blob, None);
            loaded_any = true;
        }
    }
    loaded_any
}

/// Registers any `.ttf`, `.otf`, or `.ttc` font files found in `<base_dir>/fonts`,
/// `<base_dir>/../fonts`, or `./fonts` into the shared `FontContext`.
pub fn register_local_fonts_for_base_dir(base_dir: &Path) {
    let mutex = SHARED_FONT_CONTEXT.get_or_init(init_shared_font_store);
    let mut store = mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut changed = false;
    changed |= register_font_dir_into(&mut store, &base_dir.join("fonts"));
    if let Some(parent) = base_dir.parent() {
        changed |= register_font_dir_into(&mut store, &parent.join("fonts"));
    }
    changed |= register_font_dir_into(&mut store, Path::new("fonts"));
    if changed {
        FONT_GENERATION.fetch_add(1, Ordering::Release);
    }
}

/// Spawns a background thread to scan system fonts and initialize the shared `FontContext`
/// while parsing and AST expansion proceed on the main thread.
pub fn prewarm_font_context() {
    if !PREWARM_STARTED.swap(true, Ordering::Relaxed) {
        std::thread::spawn(|| {
            let _ = SHARED_FONT_CONTEXT.get_or_init(init_shared_font_store);
        });
    }
}

/// Returns a `FontContext` cloned from the process-wide pre-warmed system font collection,
/// avoiding redundant system font directory scans across threads and renderer instances.
pub fn create_font_context() -> FontContext {
    let mutex = SHARED_FONT_CONTEXT.get_or_init(init_shared_font_store);
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .font_cx
        .clone()
}

thread_local! {
    static FONT_CONTEXT: RefCell<FontContext> = RefCell::new(create_font_context());
    static LOCAL_FONT_GEN: Cell<u64> = Cell::new(FONT_GENERATION.load(Ordering::Acquire));
    static LAYOUT_CONTEXT: RefCell<LayoutContext<()>> = RefCell::new(LayoutContext::new());
    static FONT_METRICS_CACHE: RefCell<std::collections::HashMap<(u64, u64, String), FontMetrics>> =
        RefCell::new(std::collections::HashMap::new());
}

#[inline]
fn sync_thread_local_font_context() {
    let global_gen = FONT_GENERATION.load(Ordering::Acquire);
    LOCAL_FONT_GEN.with(|cell| {
        if cell.get() != global_gen {
            FONT_CONTEXT.with(|fc| *fc.borrow_mut() = create_font_context());
            FONT_METRICS_CACHE.with(|c| c.borrow_mut().clear());
            cell.set(global_gen);
        }
    });
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
        if s.eq_ignore_ascii_case("lineheight")
            || s.eq_ignore_ascii_case("line_height")
            || s.eq_ignore_ascii_case("full")
        {
            return 0.0;
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
    line_height: Option<f64>,
) -> f64 {
    let m = measure_font_metrics(font_size, font_weight, font_family);
    match start_at {
        Some(s) if s.eq_ignore_ascii_case("ascender") => m.ascent,
        Some(s) if s.eq_ignore_ascii_case("emtop") || s.eq_ignore_ascii_case("em_top") => m.em_top,
        Some(s)
            if s.eq_ignore_ascii_case("lineheight")
                || s.eq_ignore_ascii_case("line_height")
                || s.eq_ignore_ascii_case("full") =>
        {
            if let Some(lh) = line_height.filter(|&h| h > 0.0) {
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
                                builder.push_default(StyleProperty::FontFamily(FontFamily::from(family)));
                            }
                        }
                        builder.push_default(StyleProperty::LineHeight(
                            parley::style::LineHeight::Absolute(lh as f32),
                        ));
                        let mut layout = builder.build(sample);
                        layout.break_all_lines(None);
                        let b = layout
                            .lines()
                            .next()
                            .map(|l| l.metrics().baseline as f64)
                            .unwrap_or(m.baseline);
                        b
                    })
                })
            } else {
                m.baseline
            }
        }
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
    measure_rich_text_bounds_with_boxes(
        text,
        font_size,
        font_weight,
        font_family,
        max_width,
        end_at,
        start_at,
        line_height,
        spans,
        &[],
    )
}

/// Measures the layout boundaries `(width, height)` of rich text with styled inline spans and inline boxes using Parley.
pub fn measure_rich_text_bounds_with_boxes(
    text: &str,
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
    max_width: Option<f64>,
    end_at: Option<&str>,
    start_at: Option<&str>,
    line_height: Option<f64>,
    spans: &[TextSpan],
    inline_boxes: &[InlineBoxSpan],
) -> (f64, f64) {
    if text.is_empty() && inline_boxes.is_empty() {
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
                    builder.push_default(StyleProperty::FontFamily(FontFamily::from(family)));
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
                            builder.push(StyleProperty::FontFamily(FontFamily::from(fam.as_str())), span.range.clone());
                        }
                    }
                    if span.style.italic {
                        builder.push(StyleProperty::FontStyle(FontStyle::Italic), span.range.clone());
                    }
                }
            }
            for ib in inline_boxes {
                builder.push_inline_box(parley::InlineBox {
                    id: ib.node_id.0 as u64,
                    kind: parley::InlineBoxKind::InFlow,
                    index: ib.index.min(text.len()),
                    width: ib.width.max(0.0) as f32,
                    height: ib.baseline_offset.max(0.0) as f32,
                });
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

            let mut top_coord = match start_at {
                Some(s) if s.eq_ignore_ascii_case("ascender") => {
                    first_baseline - m.ascent
                }
                Some(s) if s.eq_ignore_ascii_case("emtop") || s.eq_ignore_ascii_case("em_top") => {
                    first_baseline - m.em_top
                }
                Some(s) if s.eq_ignore_ascii_case("capital") => {
                    first_baseline - m.cap_height
                }
                Some(s) if s.eq_ignore_ascii_case("lineheight") || s.eq_ignore_ascii_case("line_height") || s.eq_ignore_ascii_case("full") => {
                    0.0
                }
                _ => {
                    // Default to Capital when unspecified
                    first_baseline - m.cap_height
                }
            };

            let mut bottom_coord = match end_at {
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
                Some(s) if s.eq_ignore_ascii_case("lineheight") || s.eq_ignore_ascii_case("line_height") || s.eq_ignore_ascii_case("full") => {
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

            if !inline_boxes.is_empty() {
                for line in layout.lines() {
                    let lm = line.metrics();
                    for item in line.items() {
                        if let parley::layout::PositionedLayoutItem::InlineBox(pib) = item {
                            if let Some(ib) = inline_boxes.iter().find(|b| b.node_id.0 as u64 == pib.id) {
                                let box_top = lm.baseline as f64 - ib.baseline_offset;
                                let box_bottom = lm.baseline as f64 + (ib.height - ib.baseline_offset).max(0.0);
                                if box_top < top_coord {
                                    top_coord = box_top;
                                }
                                if box_bottom > bottom_coord {
                                    bottom_coord = box_bottom;
                                }
                            }
                        }
                    }
                }
            }

            let height = (bottom_coord - top_coord).max(0.0);

            (width, height)
        })
    })
}

/// Metrics and byte/box associations for a single laid-out line inside a `ShapedParagraph`.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedLine {
    pub line_index: usize,
    pub byte_range: std::ops::Range<usize>,
    pub top_y: f64,
    pub bottom_y: f64,
    pub height: f64,
    pub baseline_y: f64,
    pub parley_min_y: f64,
    pub parley_max_y: f64,
    pub inline_box_indices: Vec<usize>,
}

/// Pre-shaped multi-line paragraph computed once at constant column width for O(1) page packing.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedParagraph {
    pub lines: Vec<ShapedLine>,
    pub total_width: f64,
    pub total_height: f64,
    pub first_top_coord: f64,
    pub last_bottom_coord: f64,
    pub full_y_offset: f64,
}

impl ShapedParagraph {
    /// Returns the visual top coordinate (in Parley layout space) of line `start`.
    #[inline]
    pub fn slice_top(&self, start: usize) -> f64 {
        if self.lines.is_empty() || start == 0 {
            self.first_top_coord
        } else {
            self.lines[start.min(self.lines.len() - 1)].top_y
        }
    }

    /// Returns the visual bottom coordinate (in Parley layout space) of line `end - 1`.
    #[inline]
    pub fn slice_bottom(&self, end: usize) -> f64 {
        if self.lines.is_empty() {
            return self.last_bottom_coord;
        }
        if end >= self.lines.len() {
            self.last_bottom_coord
        } else if end == 0 {
            self.first_top_coord
        } else {
            self.lines[end - 1].bottom_y
        }
    }

    /// Returns the vertical extent of `lines[start..end]`, matching `Text.height` bit-for-bit when `0..len`.
    #[inline]
    pub fn slice_height(&self, start: usize, end: usize) -> f64 {
        if self.lines.is_empty() || start >= end {
            return 0.0;
        }
        if start == 0 && end >= self.lines.len() {
            return self.total_height;
        }
        (self.slice_bottom(end) - self.slice_top(start)).max(0.0)
    }

    /// Maps an inline `\Deposit` byte offset in `text` to the line index carrying its immediately
    /// preceding non-whitespace character (or line `0` at paragraph start).
    pub fn line_for_deposit_byte(&self, byte_index: usize, text: &str) -> usize {
        if self.lines.len() <= 1 || byte_index == 0 {
            return 0;
        }
        let clamped = byte_index.min(text.len());
        let target_byte = text[..clamped]
            .char_indices()
            .rev()
            .find(|&(_, ch)| !ch.is_whitespace())
            .map(|(idx, _)| idx);
        let Some(anchor_byte) = target_byte else {
            return 0;
        };
        for line in &self.lines {
            if line.byte_range.start <= anchor_byte && anchor_byte < line.byte_range.end {
                return line.line_index;
            }
        }
        // Fallback: latest line whose start <= anchor_byte
        self.lines
            .iter()
            .rev()
            .find(|l| l.byte_range.start <= anchor_byte)
            .map(|l| l.line_index)
            .unwrap_or(0)
    }
}

/// Shapes a paragraph at constant `max_width` once into line slices using exact `Text` landmark geometry.
#[allow(clippy::too_many_arguments)]
pub fn shape_paragraph_lines(
    text: &str,
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
    max_width: Option<f64>,
    end_at: Option<&str>,
    start_at: Option<&str>,
    line_height: Option<f64>,
    align: Option<&str>,
    spans: &[TextSpan],
    inline_boxes: &[InlineBoxSpan],
) -> ShapedParagraph {
    if text.is_empty() && inline_boxes.is_empty() {
        return ShapedParagraph {
            lines: Vec::new(),
            total_width: 0.0,
            total_height: 0.0,
            first_top_coord: 0.0,
            last_bottom_coord: 0.0,
            full_y_offset: 0.0,
        };
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
                    builder.push_default(StyleProperty::FontFamily(FontFamily::from(family)));
                }
            }
            if let Some(lh) = line_height {
                if lh > 0.0 {
                    builder.push_default(StyleProperty::LineHeight(
                        parley::style::LineHeight::Absolute(lh as f32),
                    ));
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
                            builder.push(StyleProperty::FontFamily(FontFamily::from(fam.as_str())), span.range.clone());
                        }
                    }
                    if span.style.italic {
                        builder.push(StyleProperty::FontStyle(FontStyle::Italic), span.range.clone());
                    }
                }
            }
            for ib in inline_boxes {
                builder.push_inline_box(parley::InlineBox {
                    id: ib.node_id.0 as u64,
                    kind: parley::InlineBoxKind::InFlow,
                    index: ib.index.min(text.len()),
                    width: ib.width.max(0.0) as f32,
                    height: ib.baseline_offset.max(0.0) as f32,
                });
            }

            let mut layout = builder.build(text);
            layout.break_all_lines(max_width.and_then(|w| if w > 0.0 { Some(w as f32) } else { None }));
            let alignment = match align {
                Some("center") | Some("Center") => parley::Alignment::Center,
                Some("right") | Some("Right") | Some("end") | Some("End") => parley::Alignment::End,
                Some("justify") | Some("Justify") | Some("justify_all") | Some("JustifyAll") => {
                    parley::Alignment::Justify
                }
                _ => parley::Alignment::Start,
            };
            layout.align(alignment, parley::layout::AlignmentOptions::default());

            let total_width = if max_width.is_some() {
                layout.width() as f64
            } else {
                layout.full_width() as f64
            };

            let first_line = layout.lines().next();
            let last_line = layout.lines().last();
            let first_baseline = first_line
                .map(|l| l.metrics().baseline as f64)
                .unwrap_or(m.baseline);

            let mut top_coord = match start_at {
                Some(s) if s.eq_ignore_ascii_case("ascender") => first_baseline - m.ascent,
                Some(s) if s.eq_ignore_ascii_case("emtop") || s.eq_ignore_ascii_case("em_top") => {
                    first_baseline - m.em_top
                }
                Some(s) if s.eq_ignore_ascii_case("capital") => first_baseline - m.cap_height,
                Some(s)
                    if s.eq_ignore_ascii_case("lineheight")
                        || s.eq_ignore_ascii_case("line_height")
                        || s.eq_ignore_ascii_case("full") =>
                {
                    0.0
                }
                _ => first_baseline - m.cap_height,
            };

            let mut bottom_coord = match end_at {
                Some(s) if s.eq_ignore_ascii_case("descender") => {
                    if let Some(last_line) = last_line {
                        last_line.metrics().baseline as f64 + m.descent
                    } else {
                        layout.height() as f64
                    }
                }
                Some(s)
                    if s.eq_ignore_ascii_case("gapbottom") || s.eq_ignore_ascii_case("gap_bottom") =>
                {
                    if let Some(last_line) = last_line {
                        last_line.metrics().baseline as f64 + m.gap_bottom
                    } else {
                        layout.height() as f64
                    }
                }
                Some(s)
                    if s.eq_ignore_ascii_case("lineheight")
                        || s.eq_ignore_ascii_case("line_height")
                        || s.eq_ignore_ascii_case("full") =>
                {
                    layout.height() as f64
                }
                _ => {
                    if let Some(last_line) = last_line {
                        last_line.metrics().baseline as f64
                    } else {
                        layout.height() as f64
                    }
                }
            };

            let base_top_coord = top_coord;
            let base_last_bottom_offset = if let Some(last_line) = last_line {
                bottom_coord - last_line.metrics().baseline as f64
            } else {
                0.0
            };
            let top_advance = first_baseline - base_top_coord;

            let parley_lines: Vec<_> = layout.lines().collect();
            let num_lines = parley_lines.len();
            let mut shaped_lines = Vec::with_capacity(num_lines);

            for (i, line) in parley_lines.iter().enumerate() {
                let lm = line.metrics();
                let baseline_y = lm.baseline as f64;
                let parley_min_y = lm.block_min_coord as f64;
                let parley_max_y = lm.block_max_coord as f64;

                let mut line_top = if i == 0 {
                    base_top_coord
                } else {
                    baseline_y - top_advance
                };
                let mut line_bottom = if i + 1 == num_lines {
                    bottom_coord
                } else {
                    (baseline_y + m.descent)
                        .max(parley_max_y)
                        .max(baseline_y + base_last_bottom_offset)
                };

                let mut byte_start = usize::MAX;
                let mut byte_end = 0usize;
                let mut box_indices = Vec::new();

                for item in line.items() {
                    match item {
                        parley::layout::PositionedLayoutItem::GlyphRun(glyph_run) => {
                            let r = glyph_run.run().text_range();
                            byte_start = byte_start.min(r.start);
                            byte_end = byte_end.max(r.end);
                        }
                        parley::layout::PositionedLayoutItem::InlineBox(pib) => {
                            if let Some((ib_idx, ib)) = inline_boxes
                                .iter()
                                .enumerate()
                                .find(|(_, b)| b.node_id.0 as u64 == pib.id)
                            {
                                box_indices.push(ib_idx);
                                byte_start = byte_start.min(ib.index);
                                byte_end = byte_end.max(ib.index);
                                let box_top = baseline_y - ib.baseline_offset;
                                let box_bottom = baseline_y + (ib.height - ib.baseline_offset).max(0.0);
                                line_top = line_top.min(box_top);
                                line_bottom = line_bottom.max(box_bottom);
                                top_coord = top_coord.min(box_top);
                                bottom_coord = bottom_coord.max(box_bottom);
                            }
                        }
                    }
                }

                if byte_start == usize::MAX {
                    byte_start = 0;
                }
                shaped_lines.push(ShapedLine {
                    line_index: i,
                    byte_range: byte_start..byte_end,
                    top_y: line_top,
                    bottom_y: line_bottom,
                    height: (line_bottom - line_top).max(0.0),
                    baseline_y,
                    parley_min_y,
                    parley_max_y,
                    inline_box_indices: box_indices,
                });
            }

            if let Some(first) = shaped_lines.first_mut() {
                first.top_y = top_coord;
                first.height = (first.bottom_y - first.top_y).max(0.0);
            }
            if let Some(last) = shaped_lines.last_mut() {
                last.bottom_y = bottom_coord;
                last.height = (last.bottom_y - last.top_y).max(0.0);
            }

            let total_height = (bottom_coord - top_coord).max(0.0);
            ShapedParagraph {
                lines: shaped_lines,
                total_width,
                total_height,
                first_top_coord: top_coord,
                last_bottom_coord: bottom_coord,
                full_y_offset: -top_coord,
            }
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
    sync_thread_local_font_context();
    let family_key = font_family.unwrap_or("").to_string();
    let cache_key = ((size as f64).to_bits(), (weight as f64).to_bits(), family_key);
    if let Some(cached) = FONT_METRICS_CACHE.with(|c| c.borrow().get(&cache_key).copied()) {
        return cached;
    }

    let computed = FONT_CONTEXT.with(|font_cx_cell| {
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
                    builder.push_default(StyleProperty::FontFamily(FontFamily::from(family)));
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
    });
    FONT_METRICS_CACHE.with(|c| {
        c.borrow_mut().insert(cache_key, computed);
    });
    computed
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
    sync_thread_local_font_context();
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
    compute_span_fragments_with_metrics_and_boxes(
        text,
        font_size,
        font_weight,
        font_family,
        max_width,
        line_height,
        align,
        spans,
        &[],
        child_shifts,
        origin_x,
        origin_y,
    )
}

/// Measures a rich text layout (with inline boxes) and returns full `SpanLayoutMetrics`
/// for every inline `NodeId` referenced in `spans`.
pub fn compute_span_fragments_with_metrics_and_boxes(
    text: &str,
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
    max_width: Option<f64>,
    line_height: Option<f64>,
    align: Option<&str>,
    spans: &[TextSpan],
    inline_boxes: &[InlineBoxSpan],
    child_shifts: &HashMap<NodeId, f64>,
    origin_x: f64,
    origin_y: f64,
) -> HashMap<NodeId, SpanLayoutMetrics> {
    let mut result: HashMap<NodeId, SpanLayoutMetrics> = HashMap::new();
    if (text.is_empty() && inline_boxes.is_empty()) || spans.is_empty() {
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
    let mut origin_y = origin_y;

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
                builder.push_default(StyleProperty::FontFamily(FontFamily::from(family)));
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
                        builder.push(StyleProperty::FontFamily(FontFamily::from(fam.as_str())), span.range.clone());
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
        for ib in inline_boxes {
            builder.push_inline_box(parley::InlineBox {
                id: ib.node_id.0 as u64,
                kind: parley::InlineBoxKind::InFlow,
                index: ib.index.min(text.len()),
                width: ib.width.max(0.0) as f32,
                height: ib.baseline_offset.max(0.0) as f32,
            });
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
            Some("justify") | Some("Justify") | Some("justify_all") | Some("JustifyAll") => {
                parley::Alignment::Justify
            }
            _ => parley::Alignment::Start,
        };
        layout.align(alignment, parley::layout::AlignmentOptions::default());

        if let Some(first_line) = layout.lines().next() {
            default_first_baseline = first_line.metrics().baseline as f64;
        }
        origin_y += m.baseline - default_first_baseline;

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
                let (px, py) = measure_rich_text_bounds_with_boxes(
                    prefix_text,
                    font_size,
                    font_weight,
                    font_family,
                    max_width,
                    None,
                    None,
                    line_height,
                    spans,
                    inline_boxes,
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
    compute_span_range_metrics_with_boxes(
        text,
        font_size,
        font_weight,
        font_family,
        max_width,
        start_at,
        line_height,
        align,
        spans,
        &[],
        text_x,
        text_y,
        span_range,
        own_shift,
        None,
    )
}

/// Computes the post-shaping `SpanLayoutMetrics` for a specific byte range `span_range` (or a specific `InlineBox` node)
/// inside a rich text block containing styled spans and inline boxes.
pub fn compute_span_range_metrics_with_boxes(
    text: &str,
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
    max_width: Option<f64>,
    start_at: Option<&str>,
    line_height: Option<f64>,
    align: Option<&str>,
    spans: &[TextSpan],
    inline_boxes: &[InlineBoxSpan],
    text_x: f64,
    text_y: f64,
    span_range: Range<usize>,
    own_shift: f64,
    target_inline_box: Option<NodeId>,
) -> SpanLayoutMetrics {
    let origin_x = text_x;

    if text.is_empty() && inline_boxes.is_empty() {
        return SpanLayoutMetrics {
            fragments: Vec::new(),
            rect: Rect::new(text_x, text_y + own_shift, 0.0, 0.0),
            baseline: text_y + text_baseline_offset(font_size, font_weight, font_family, start_at, line_height) + own_shift,
        };
    }

    let size = if font_size > 0.0 { font_size as f32 } else { 16.0 };
    let weight = if font_weight > 0.0 { font_weight as f32 } else { 400.0 };
    let m = measure_font_metrics(font_size, font_weight, font_family);

    let mut fragments = Vec::new();
    let mut first_line_baseline: Option<f64> = None;
    let mut default_first_baseline = m.baseline;
    let mut origin_y = text_y + text_y_offset(font_size, font_weight, font_family, start_at);

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
                builder.push_default(StyleProperty::FontFamily(FontFamily::from(family)));
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
                        builder.push(StyleProperty::FontFamily(FontFamily::from(fam.as_str())), span.range.clone());
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
        for ib in inline_boxes {
            builder.push_inline_box(parley::InlineBox {
                id: ib.node_id.0 as u64,
                kind: parley::InlineBoxKind::InFlow,
                index: ib.index.min(text.len()),
                width: ib.width.max(0.0) as f32,
                height: ib.baseline_offset.max(0.0) as f32,
            });
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
            Some("justify") | Some("Justify") | Some("justify_all") | Some("JustifyAll") => {
                parley::Alignment::Justify
            }
            _ => parley::Alignment::Start,
        };
        layout.align(alignment, parley::layout::AlignmentOptions::default());

        if let Some(first_line) = layout.lines().next() {
            default_first_baseline = first_line.metrics().baseline as f64;
        }

        let y_offset = match start_at {
            Some(s) if s.eq_ignore_ascii_case("ascender") => -(default_first_baseline - m.ascent),
            Some(s) if s.eq_ignore_ascii_case("emtop") || s.eq_ignore_ascii_case("em_top") => -(default_first_baseline - m.em_top),
            Some(s) if s.eq_ignore_ascii_case("capital") => -(default_first_baseline - m.cap_height),
            Some(s) if s.eq_ignore_ascii_case("lineheight") || s.eq_ignore_ascii_case("line_height") || s.eq_ignore_ascii_case("full") => 0.0,
            _ => -(default_first_baseline - m.cap_height),
        };
        origin_y = text_y + y_offset;

        for line in layout.lines() {
            let line_metrics = line.metrics();
            let line_top_base = origin_y + line_metrics.block_min_coord as f64;
            let line_height_val = (line_metrics.block_max_coord - line_metrics.block_min_coord) as f64;
            let line_baseline_base = origin_y + line_metrics.baseline as f64;

            if let Some(box_node_id) = target_inline_box {
                for item in line.items() {
                    if let parley::layout::PositionedLayoutItem::InlineBox(pib) = item {
                        if pib.id == box_node_id.0 as u64 {
                            let ib_info = inline_boxes.iter().find(|b| b.node_id == box_node_id);
                            let b_w = ib_info.map(|b| b.width).unwrap_or(pib.width as f64);
                            let b_h = ib_info.map(|b| b.height).unwrap_or(pib.height as f64);
                            let b_off = ib_info.map(|b| b.baseline_offset).unwrap_or(b_h);
                            let box_x = origin_x + pib.x as f64;
                            let box_y = line_baseline_base - b_off;
                            fragments.push(Rect::new(box_x, box_y, b_w, b_h));
                            if first_line_baseline.is_none() {
                                first_line_baseline = Some(line_baseline_base);
                            }
                        }
                    }
                }
                continue;
            }

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
        let (px, py) = measure_rich_text_bounds_with_boxes(
            prefix_text,
            font_size,
            font_weight,
            font_family,
            max_width,
            None,
            None,
            line_height,
            spans,
            inline_boxes,
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

/// An inline box element embedded at a byte index within a parent text element.
#[derive(Debug, Clone, PartialEq)]
pub struct InlineBoxSpan {
    /// The `NodeId` of the `\InlineBox` component instance.
    pub node_id: NodeId,
    /// Byte offset in the parent `Text` node's concatenated `text_content` where this inline box sits.
    pub index: usize,
    /// Evaluated width of the inline box (0.0 at AST expansion time; populated during evaluation/resolution).
    pub width: f64,
    /// Evaluated height of the inline box (0.0 at AST expansion time; populated during evaluation/resolution).
    pub height: f64,
    /// Evaluated baseline offset (`baseline - y`, defaulting to `height`).
    pub baseline_offset: f64,
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

pub(crate) fn format_text_port_value(v: &crate::compiler::value::Value) -> Option<String> {
    match v {
        crate::compiler::value::Value::String(s) => Some(s.clone()),
        crate::compiler::value::Value::Integer(i) => Some(format!("{i}")),
        crate::compiler::value::Value::Float(n) | crate::compiler::value::Value::Distance(n) => {
            Some(format!("{n}"))
        }
        crate::compiler::value::Value::Bool(b) => Some(format!("{b}")),
        _ => None,
    }
}

/// Resolves a text node's effective text string, adjusted `TextSpan` ranges, `InlineBoxSpan` byte
/// indices, and `(orig_pos, inserted_len)` shifts after splicing any inline dynamic `text:` child
/// placeholders (`range.is_empty()` pointing to a child with `text_content == None` and `properties["text"]`).
pub(crate) fn resolve_active_rich_text(
    node: &crate::compiler::layout::ResolvedNode,
    layout: &crate::compiler::layout::ResolvedLayout,
) -> (
    Option<String>,
    Vec<TextSpan>,
    Vec<InlineBoxSpan>,
    Vec<(usize, usize)>,
) {
    let mut placeholders: Vec<(usize, usize, String)> = Vec::new();
    for (s_idx, ts) in node.text_spans.iter().enumerate() {
        if ts.range.is_empty() {
            if let Some(cid) = ts.node_id {
                if let Some(cnode) = layout.get_node(cid) {
                    if cnode.text_content.is_none() {
                        if let Some(s) =
                            cnode.properties.get("text").and_then(format_text_port_value)
                        {
                            placeholders.push((s_idx, ts.range.start, s));
                        }
                    }
                }
            }
        }
    }

    if placeholders.is_empty() {
        let active = node
            .properties
            .get("text")
            .and_then(format_text_port_value)
            .or_else(|| node.text_content.clone());
        return (
            active,
            node.text_spans.clone(),
            node.inline_boxes.clone(),
            Vec::new(),
        );
    }

    let base = node.text_content.as_deref().unwrap_or("");
    let total_ins: usize = placeholders.iter().map(|p| p.2.len()).sum();
    let mut result = String::with_capacity(base.len() + total_ins);
    let mut cursor = 0usize;
    let mut placeholder_new_ranges: Vec<(usize, std::ops::Range<usize>)> =
        Vec::with_capacity(placeholders.len());
    let mut shifts: Vec<(usize, usize)> = Vec::with_capacity(placeholders.len());

    for &(s_idx, orig_pos, ref ins) in &placeholders {
        let clamped = orig_pos.min(base.len()).max(cursor);
        result.push_str(&base[cursor..clamped]);
        cursor = clamped;
        let new_start = result.len();
        result.push_str(ins);
        let new_end = result.len();
        placeholder_new_ranges.push((s_idx, new_start..new_end));
        shifts.push((clamped, ins.len()));
    }
    if cursor < base.len() {
        result.push_str(&base[cursor..]);
    }

    let mut adjusted_spans = Vec::with_capacity(node.text_spans.len());
    let mut p_cursor = 0usize;
    for (s_idx, ts) in node.text_spans.iter().enumerate() {
        let mut cloned = ts.clone();
        if p_cursor < placeholder_new_ranges.len() && placeholder_new_ranges[p_cursor].0 == s_idx {
            cloned.range = placeholder_new_ranges[p_cursor].1.clone();
            p_cursor += 1;
        } else {
            let start_shift: usize = placeholders
                .iter()
                .filter(|&&(p_idx, p_pos, _)| {
                    p_pos < ts.range.start || (p_pos == ts.range.start && p_idx < s_idx)
                })
                .map(|(_, _, ins)| ins.len())
                .sum();
            let end_shift: usize = placeholders
                .iter()
                .filter(|&&(p_idx, p_pos, _)| {
                    p_pos < ts.range.end || (p_pos == ts.range.end && p_idx < s_idx)
                })
                .map(|(_, _, ins)| ins.len())
                .sum();
            cloned.range = (ts.range.start + start_shift)..(ts.range.end + end_shift);
        }
        adjusted_spans.push(cloned);
    }

    let mut adjusted_boxes = Vec::with_capacity(node.inline_boxes.len());
    for mut ib in node.inline_boxes.clone() {
        let box_shift: usize = shifts
            .iter()
            .filter(|&&(p_pos, _)| p_pos <= ib.index)
            .map(|&(_, len)| len)
            .sum();
        ib.index += box_shift;
        adjusted_boxes.push(ib);
    }

    (Some(result), adjusted_spans, adjusted_boxes, shifts)
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

    #[test]
    fn test_parley_inline_box_behavior() {
        FONT_CONTEXT.with(|font_cx_cell| {
            LAYOUT_CONTEXT.with(|layout_cx_cell| {
                let mut font_cx = font_cx_cell.borrow_mut();
                let mut layout_cx = layout_cx_cell.borrow_mut();

                let text = "Hello world";
                let mut builder = layout_cx.ranged_builder(&mut font_cx, text, 1.0, true);
                builder.push_default(StyleProperty::FontSize(16.0));
                builder.push_inline_box(parley::InlineBox {
                    id: 42,
                    kind: parley::InlineBoxKind::InFlow,
                    index: 6,
                    width: 50.0,
                    height: 20.0,
                });
                let mut layout = builder.build(text);
                layout.break_all_lines(None);

                let line = layout.lines().next().unwrap();
                let mut found_box = None;
                let mut glyph_count = 0;
                for item in line.items() {
                    match item {
                        parley::layout::PositionedLayoutItem::GlyphRun(gr) => {
                            glyph_count += gr.positioned_glyphs().count();
                        }
                        parley::layout::PositionedLayoutItem::InlineBox(ib) => {
                            found_box = Some((ib.id, ib.x, ib.y, ib.width, ib.height));
                        }
                    }
                }
                assert_eq!(glyph_count, 11);
                assert!(found_box.is_some());
                let (id, bx, by, bw, bh) = found_box.unwrap();
                assert_eq!(id, 42);
                assert!(bx > 0.0);
                assert_eq!(bw, 50.0);
                assert_eq!(bh, 20.0);
                let _ = by;

                // Also test empty text with two adjacent inline boxes at index 0
                let empty_text = "";
                let mut builder2 = layout_cx.ranged_builder(&mut font_cx, empty_text, 1.0, true);
                builder2.push_default(StyleProperty::FontSize(16.0));
                builder2.push_inline_box(parley::InlineBox {
                    id: 1,
                    kind: parley::InlineBoxKind::InFlow,
                    index: 0,
                    width: 30.0,
                    height: 15.0,
                });
                builder2.push_inline_box(parley::InlineBox {
                    id: 2,
                    kind: parley::InlineBoxKind::InFlow,
                    index: 0,
                    width: 40.0,
                    height: 18.0,
                });
                let mut layout2 = builder2.build(empty_text);
                layout2.break_all_lines(None);
                assert!((layout2.full_width() - 70.0).abs() < 0.01);
                let line2 = layout2.lines().next().unwrap();
                let mut boxes = Vec::new();
                for item in line2.items() {
                    if let parley::layout::PositionedLayoutItem::InlineBox(ib) = item {
                        boxes.push((ib.id, ib.x, ib.y, ib.width, ib.height));
                    }
                }
                assert_eq!(boxes.len(), 2);
                assert_eq!(boxes[0].0, 1);
                assert!((boxes[0].1 - 0.0).abs() < 0.01);
                assert_eq!(boxes[1].0, 2);
                assert!((boxes[1].1 - 30.0).abs() < 0.01);
            });
        });
    }

    #[test]
    fn test_generic_monospace_font_family() {
        // In a proportional font like Arial, "WWWWWWWWWW" is much wider than "iiiiiiiiii".
        let arial_i = measure_text_width("iiiiiiiiii", 16.0, 400.0, Some("Arial"));
        let arial_w = measure_text_width("WWWWWWWWWW", 16.0, 400.0, Some("Arial"));
        assert!(
            arial_w > arial_i * 1.5,
            "Expected proportional Arial to have W wider than i, got W={} i={}",
            arial_w,
            arial_i
        );

        // With generic "monospace", both 10-character strings must have identical advance width.
        let mono_i = measure_text_width("iiiiiiiiii", 16.0, 400.0, Some("monospace"));
        let mono_w = measure_text_width("WWWWWWWWWW", 16.0, 400.0, Some("monospace"));
        assert!(
            (mono_w - mono_i).abs() < 0.05,
            "Expected generic monospace to have equal width for 10 'W's ({}) and 10 'i's ({})",
            mono_w,
            mono_i
        );

        // Also verify span-level font_family: "monospace" inside an "Arial" paragraph.
        let span_style = SpanStyle {
            font_family: Some("monospace".to_string()),
            ..Default::default()
        };
        let (span_i, _) = measure_rich_text_bounds(
            "iiiiiiiiii",
            16.0,
            400.0,
            Some("Arial"),
            None,
            None,
            None,
            None,
            &[TextSpan::new(0..10).with_style(span_style.clone())],
        );
        let (span_w, _) = measure_rich_text_bounds(
            "WWWWWWWWWW",
            16.0,
            400.0,
            Some("Arial"),
            None,
            None,
            None,
            None,
            &[TextSpan::new(0..10).with_style(span_style)],
        );
        assert!(
            (span_w - span_i).abs() < 0.05,
            "Expected span-level monospace to have equal width for 10 'W's ({}) and 10 'i's ({})",
            span_w,
            span_i
        );
    }
}

