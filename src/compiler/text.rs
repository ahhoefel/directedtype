pub use crate::compiler::knuth_plass::{LineBreakMode, ParagraphLayoutOptions};
use parley::style::{FontFamily, FontFeature, FontFeatures, FontStyle, FontWeight, StyleProperty};
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
    static FONT_SMCP_CACHE: RefCell<std::collections::HashMap<(String, u32, bool), bool>> =
        RefCell::new(std::collections::HashMap::new());
}

#[inline]
fn sync_thread_local_font_context() {
    let global_gen = FONT_GENERATION.load(Ordering::Acquire);
    LOCAL_FONT_GEN.with(|cell| {
        if cell.get() != global_gen {
            FONT_CONTEXT.with(|fc| *fc.borrow_mut() = create_font_context());
            FONT_METRICS_CACHE.with(|c| c.borrow_mut().clear());
            FONT_SMCP_CACHE.with(|c| c.borrow_mut().clear());
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

const SOFT_HYPHEN: char = '\u{00AD}';
const SOFT_HYPHEN_LEN: usize = SOFT_HYPHEN.len_utf8(); // 2 UTF-8 bytes

/// Marker string emitted by parser escape `\|` for a forced justified line break.
/// Parley breaks at `'\n'` and suppresses `'\u{2060}'` (Word Joiner) with zero glyph output;
/// renderers and inline-box layout recognize `s.ends_with(FORCED_JUSTIFY_BREAK)` to justify
/// the line under `Align.Justify` while keeping plain `"\n"` and the final paragraph line ragged.
pub const FORCED_JUSTIFY_BREAK: &str = "\u{2060}\n";

/// Breaks lines on a Parley `Layout`, respecting pre-computed explicit Knuth–Plass breaks.
///
/// When `has_explicit_breaks` is true, lines for `Left`/`Justify`/`JustifyAll` are broken with
/// `None` (only at explicit `\n` / `\|`) so tight shrunk Knuth–Plass lines are never re-wrapped by
/// Parley's greedy breaker, while `Center`/`Right` still pass `max_width` so Parley centers/right-aligns
/// against the container width.
pub fn break_lines_for_layout<B: parley::style::Brush>(
    layout: &mut parley::Layout<B>,
    max_width: Option<f64>,
    has_explicit_breaks: bool,
    align: Option<&str>,
) {
    let is_center_or_right = matches!(
        align,
        Some("center") | Some("Center") | Some("right") | Some("Right") | Some("end") | Some("End")
    );
    if has_explicit_breaks && !is_center_or_right {
        layout.break_all_lines(None);
    } else if let Some(w) = max_width.filter(|&w| w > 0.0) {
        layout.break_all_lines(Some(w as f32));
    } else {
        layout.break_all_lines(None);
    }
}

/// Computes `(space_extra_advance, space_count)` for lines requiring manual space-stretching
/// or space-shrinking (e.g. Knuth–Plass tight lines):
/// - Under `Align.JustifyAll`: the final line (`line_idx + 1 == total_lines`) or any `\n`-broken line.
/// - Under `Align.Justify`: any non-final forced justified break (`\|` -> `FORCED_JUSTIFY_BREAK`),
///   leaving soft-wrapped lines (already justified by Parley), plain `"\n"` lines, and the final
///   paragraph line ragged.
pub fn compute_manual_justify_line_spaces<B: parley::style::Brush>(
    line: &parley::layout::Line<'_, B>,
    line_idx: usize,
    total_lines: usize,
    active_text: &str,
    align_str: Option<&str>,
    target_width: f64,
) -> (f32, usize) {
    let is_justify_all = matches!(align_str, Some("justify_all") | Some("JustifyAll"));
    let is_justify = is_justify_all || matches!(align_str, Some("justify") | Some("Justify"));
    let line_slice = active_text.get(line.text_range());
    let needs_manual_justify = target_width > 0.0
        && ((is_justify_all
            && (line_idx + 1 == total_lines || line_slice.is_some_and(|s| s.ends_with('\n'))))
            || (is_justify
                && line_idx + 1 < total_lines
                && line_slice.is_some_and(|s| s.ends_with(FORCED_JUSTIFY_BREAK))));
    if !needs_manual_justify {
        return (0.0, 0);
    }

    let lm = line.metrics();
    let free = (target_width as f32) - lm.offset - (lm.advance - lm.trailing_whitespace);
    if free.abs() <= 0.05 {
        return (0.0, 0);
    }

    let mut temp_cursor: Option<(usize, usize)> = None;
    let mut space_flags: Vec<(bool, f32)> = Vec::new();
    for item in line.items() {
        match item {
            parley::layout::PositionedLayoutItem::GlyphRun(gr) => {
                let r = gr.run();
                let gc = gr.glyphs().count();
                let s_idx = match temp_cursor {
                    Some((ri, idx)) if ri == r.index() => idx,
                    _ => 0,
                };
                let gs: Vec<(std::ops::Range<usize>, f32)> = r
                    .visual_clusters()
                    .flat_map(|c| {
                        let tr = c.text_range();
                        c.glyphs().map(move |g| (tr.clone(), g.advance))
                    })
                    .skip(s_idx)
                    .take(gc)
                    .collect();
                temp_cursor = Some((r.index(), s_idx + gs.len()));
                for (cr, adv) in gs {
                    let is_sp = active_text.get(cr).is_some_and(|s| {
                        !s.is_empty() && s.chars().all(|ch| ch == ' ' || ch == '\u{00A0}')
                    });
                    space_flags.push((is_sp, adv));
                }
            }
            parley::layout::PositionedLayoutItem::InlineBox(_) => {
                space_flags.push((false, 0.0));
            }
        }
    }
    while space_flags.last().is_some_and(|&(is_sp, _)| is_sp) {
        space_flags.pop();
    }
    let mut n_spaces = 0usize;
    let mut min_space_adv = f32::MAX;
    for &(is_sp, adv) in &space_flags {
        if is_sp {
            n_spaces += 1;
            if adv < min_space_adv {
                min_space_adv = adv;
            }
        }
    }
    if n_spaces > 0 {
        let mut extra = free / (n_spaces as f32);
        if extra < 0.0 && min_space_adv.is_finite() {
            // Never shrink a space below 10% of its natural width even on an unavoidable overfull line.
            extra = extra.max(-min_space_adv * 0.9);
        }
        (extra, n_spaces)
    } else {
        (0.0, 0)
    }
}

/// Synthesized small-caps height ratio used when a font lacks OpenType `smcp`.
pub const SYNTHETIC_SMALL_CAPS_SCALE: f64 = 0.74;

type OffsetEntry = (usize, usize, usize, usize);

#[inline]
fn map_pos_floor(x: usize, entries: &[OffsetEntry]) -> usize {
    let mut delta: isize = 0;
    for &(s_start, s_len, d_start, d_len) in entries {
        if x < s_start {
            break;
        }
        if x < s_start + s_len {
            return d_start;
        }
        delta += (d_len as isize) - (s_len as isize);
    }
    ((x as isize) + delta).max(0) as usize
}

#[inline]
fn map_pos_ceil(x: usize, entries: &[OffsetEntry]) -> usize {
    let mut delta: isize = 0;
    for &(s_start, s_len, d_start, d_len) in entries {
        if x <= s_start {
            break;
        }
        if x < s_start + s_len {
            return d_start + d_len;
        }
        delta += (d_len as isize) - (s_len as isize);
    }
    ((x as isize) + delta).max(0) as usize
}

/// Checks whether the font resolved for `(font_family, font_weight, font_italic)` contains an OpenType `smcp` feature in its `GSUB` table.
pub fn font_supports_smcp_with_font_cx(
    font_cx: &mut FontContext,
    font_family: Option<&str>,
    font_weight: f32,
    font_italic: bool,
) -> bool {
    use skrifa::raw::TableProvider;

    let fam_key = font_family.unwrap_or("").to_string();
    let weight_bits = if font_weight > 0.0 {
        font_weight.to_bits()
    } else {
        400.0f32.to_bits()
    };
    let cache_key = (fam_key, weight_bits, font_italic);
    if let Some(cached) = FONT_SMCP_CACHE.with(|c| c.borrow().get(&cache_key).copied()) {
        return cached;
    }

    let mut probe_layout_cx = LayoutContext::<()>::new();
    let sample = "x";
    let mut builder = probe_layout_cx.ranged_builder(font_cx, sample, 1.0, true);
    builder.push_default(StyleProperty::FontSize(16.0));
    if (font_weight - 400.0).abs() > 1.0 && font_weight > 0.0 {
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
    let mut layout = builder.build(sample);
    layout.break_all_lines(None);

    let mut supported = false;
    'outer: for line in layout.lines() {
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::GlyphRun(gr) = item {
                let font = gr.run().font();
                if let Ok(font_ref) = skrifa::FontRef::from_index(font.data.as_ref(), font.index) {
                    if let Ok(gsub) = font_ref.gsub() {
                        if let Ok(fl) = gsub.feature_list() {
                            if fl
                                .feature_records()
                                .iter()
                                .any(|rec| rec.feature_tag() == skrifa::Tag::new(b"smcp"))
                            {
                                supported = true;
                                break 'outer;
                            }
                        }
                    }
                }
            }
        }
    }

    FONT_SMCP_CACHE.with(|c| {
        c.borrow_mut().insert(cache_key, supported);
    });
    supported
}

/// Convenience wrapper checking OpenType `smcp` support on the thread-local `FontContext`.
pub fn font_supports_smcp(
    font_family: Option<&str>,
    font_weight: f64,
    font_italic: bool,
) -> bool {
    sync_thread_local_font_context();
    FONT_CONTEXT.with(|fc_cell| {
        let mut font_cx = fc_cell.borrow_mut();
        let w = if font_weight > 0.0 {
            font_weight as f32
        } else {
            400.0
        };
        font_supports_smcp_with_font_cx(&mut font_cx, font_family, w, font_italic)
    })
}

/// Result of resolving both small caps (native OpenType `smcp` or synthesized uppercase+scale)
/// and soft hyphens (`U+00AD`) for shaping, together with original-casing `pdf_text` for PDF `/ToUnicode`.
#[derive(Debug, Clone)]
pub struct PreparedRichText {
    pub text: String,
    pub pdf_text: String,
    pub spans: Vec<TextSpan>,
    pub inline_boxes: Vec<InlineBoxSpan>,
    pub smcp_ranges: Vec<(Range<usize>, bool)>,
    pub synthetic_size_ranges: Vec<(Range<usize>, f32)>,
    pub byte_deltas: Vec<(usize, isize)>,
    pub orig_to_sc_entries: Vec<OffsetEntry>,
    pub shaped_to_pdf_entries: Vec<OffsetEntry>,
    /// True when optimal Knuth–Plass line breaking materialized explicit line breaks into `text`.
    pub has_explicit_breaks: bool,
}

impl PreparedRichText {
    #[inline]
    pub fn map_orig_offset_floor(&self, orig_offset: usize) -> usize {
        let sc = map_pos_floor(orig_offset, &self.orig_to_sc_entries);
        map_byte_offset_with_deltas(sc, &self.byte_deltas)
    }

    #[inline]
    pub fn map_orig_range(&self, orig_range: Range<usize>) -> Range<usize> {
        let sc_s = map_pos_floor(orig_range.start, &self.orig_to_sc_entries);
        let sc_e = map_pos_ceil(orig_range.end, &self.orig_to_sc_entries).max(sc_s);
        let s = map_byte_offset_with_deltas(sc_s, &self.byte_deltas);
        let e = map_byte_offset_with_deltas(sc_e, &self.byte_deltas);
        s..e.max(s)
    }

    #[inline]
    pub fn map_shaped_to_pdf_range(&self, shaped_range: Range<usize>) -> Range<usize> {
        if self.shaped_to_pdf_entries.is_empty() {
            return shaped_range;
        }
        let s = map_pos_floor(shaped_range.start, &self.shaped_to_pdf_entries);
        let e = map_pos_ceil(shaped_range.end, &self.shaped_to_pdf_entries).max(s);
        s..e
    }
}

/// Pushes native OpenType `smcp` feature ranges and synthesized small-caps size overrides
/// onto a `parley::RangedBuilder`. Must be called AFTER span `FontSize` styles are pushed.
pub fn push_small_caps_styles<B: parley::style::Brush>(
    builder: &mut parley::RangedBuilder<'_, B>,
    smcp_ranges: &[(Range<usize>, bool)],
    synthetic_size_ranges: &[(Range<usize>, f32)],
    text_len: usize,
) {
    const SMCP_ON: &[FontFeature] = &[FontFeature::new(parley::setting::Tag::new(b"smcp"), 1)];
    for (range, enabled) in smcp_ranges {
        if *enabled && !range.is_empty() && range.end <= text_len {
            builder.push(
                StyleProperty::FontFeatures(FontFeatures::List(std::borrow::Cow::Borrowed(SMCP_ON))),
                range.clone(),
            );
        }
    }
    for (range, scaled_sz) in synthetic_size_ranges {
        if *scaled_sz > 0.0 && !range.is_empty() && range.end <= text_len {
            builder.push(StyleProperty::FontSize(*scaled_sz), range.clone());
        }
    }
}

/// Result of resolving soft hyphens (`U+00AD`) for a given `max_width`:
/// taken line-ending breaks become literal `'-'`, while untaken soft hyphens are stripped.
#[derive(Debug, Clone)]
pub struct MaterializedRichText {
    pub text: String,
    pub spans: Vec<TextSpan>,
    pub inline_boxes: Vec<InlineBoxSpan>,
    /// Sorted `(orig_shy_byte_pos, byte_delta)` pairs (`-1` when replaced by `'-'`, `-2` when stripped).
    pub byte_deltas: Vec<(usize, isize)>,
}

impl MaterializedRichText {
    #[inline]
    pub fn map_offset(&self, orig_offset: usize) -> usize {
        map_byte_offset_with_deltas(orig_offset, &self.byte_deltas)
    }

    #[inline]
    pub fn map_range(&self, orig_range: Range<usize>) -> Range<usize> {
        let s = self.map_offset(orig_range.start);
        let e = self.map_offset(orig_range.end);
        s..e.max(s)
    }
}

#[inline]
pub fn map_byte_offset_with_deltas(orig_offset: usize, byte_deltas: &[(usize, isize)]) -> usize {
    if byte_deltas.is_empty() {
        return orig_offset;
    }
    let mut delta_sum: isize = 0;
    for &(shy_pos, delta) in byte_deltas {
        if shy_pos + SOFT_HYPHEN_LEN <= orig_offset {
            delta_sum += delta;
        } else if shy_pos < orig_offset {
            // Mid-soft-hyphen offset clamps to the start of that soft hyphen
            return ((shy_pos as isize) + delta_sum).max(0) as usize;
        } else {
            break;
        }
    }
    ((orig_offset as isize) + delta_sum).max(0) as usize
}

#[inline]
fn adjust_offset_for_splice(offset: usize, pos: usize, old_len: usize, new_len: usize) -> usize {
    if offset <= pos {
        offset
    } else if offset >= pos + old_len {
        (offset - old_len) + new_len
    } else {
        pos
    }
}

fn splice_rich_text(
    text: &mut String,
    spans: &mut [TextSpan],
    boxes: &mut [InlineBoxSpan],
    pos: usize,
    old_len: usize,
    replacement: &str,
) {
    let new_len = replacement.len();
    text.replace_range(pos..pos + old_len, replacement);
    for span in spans.iter_mut() {
        let s = adjust_offset_for_splice(span.range.start, pos, old_len, new_len);
        let e = adjust_offset_for_splice(span.range.end, pos, old_len, new_len);
        span.range = s..e.max(s);
    }
    for ib in boxes.iter_mut() {
        if old_len == 0 && ib.index == pos {
            ib.index = pos + new_len;
        } else {
            ib.index = adjust_offset_for_splice(ib.index, pos, old_len, new_len);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn splice_prepared_rich_text(
    text: &mut String,
    pdf_text: &mut String,
    spans: &mut [TextSpan],
    boxes: &mut [InlineBoxSpan],
    smcp_ranges: &mut Vec<(Range<usize>, bool)>,
    synthetic_size_ranges: &mut Vec<(Range<usize>, f32)>,
    shaped_to_pdf_entries: &mut [OffsetEntry],
    shy_sc_and_pdf: &mut Vec<(usize, usize)>,
    sc_pos: usize,
    pdf_pos: usize,
    old_len: usize,
    replacement: &str,
) {
    let new_len = replacement.len();
    splice_rich_text(text, spans, boxes, sc_pos, old_len, replacement);
    pdf_text.replace_range(pdf_pos..pdf_pos + old_len, replacement);
    for (r, _) in smcp_ranges.iter_mut() {
        let s = adjust_offset_for_splice(r.start, sc_pos, old_len, new_len);
        let e = adjust_offset_for_splice(r.end, sc_pos, old_len, new_len);
        *r = s..e.max(s);
    }
    for (r, _) in synthetic_size_ranges.iter_mut() {
        let s = adjust_offset_for_splice(r.start, sc_pos, old_len, new_len);
        let e = adjust_offset_for_splice(r.end, sc_pos, old_len, new_len);
        *r = s..e.max(s);
    }
    for entry in shaped_to_pdf_entries.iter_mut() {
        entry.0 = adjust_offset_for_splice(entry.0, sc_pos, old_len, new_len);
        entry.2 = adjust_offset_for_splice(entry.2, pdf_pos, old_len, new_len);
    }
    for (sp, pp) in shy_sc_and_pdf.iter_mut() {
        *sp = adjust_offset_for_splice(*sp, sc_pos, old_len, new_len);
        *pp = adjust_offset_for_splice(*pp, pdf_pos, old_len, new_len);
    }
}

/// Reserved inline-box ID representing a paragraph's first-line indent.
pub const INDENT_INLINE_BOX_ID: u64 = u64::MAX;

/// Pushes a zero-height inline box at byte index 0 to implement first-line indentation in Parley.
pub fn push_first_line_indent_box<B: parley::style::Brush>(
    builder: &mut parley::RangedBuilder<'_, B>,
    indent: f64,
    has_content: bool,
) {
    if indent > 0.0 && has_content {
        builder.push_inline_box(parley::InlineBox {
            id: INDENT_INLINE_BOX_ID,
            kind: parley::InlineBoxKind::InFlow,
            index: 0,
            width: indent as f32,
            height: 0.0,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn build_probe_layout(
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    text: &str,
    font_size: f32,
    font_weight: f32,
    font_family: Option<&str>,
    font_italic: bool,
    letter_spacing: f64,
    word_spacing: f64,
    line_height: Option<f64>,
    max_width: Option<f32>,
    indent: f64,
    spans: &[TextSpan],
    smcp_ranges: &[(Range<usize>, bool)],
    synthetic_size_ranges: &[(Range<usize>, f32)],
    inline_boxes: &[InlineBoxSpan],
) -> parley::Layout<()> {
    let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
    builder.push_default(StyleProperty::FontSize(font_size));
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
    if letter_spacing.abs() > 1e-4 {
        builder.push_default(StyleProperty::LetterSpacing(letter_spacing as f32));
    }
    if word_spacing.abs() > 1e-4 {
        builder.push_default(StyleProperty::WordSpacing(word_spacing as f32));
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
                    builder.push(
                        StyleProperty::FontWeight(FontWeight::new(wt as f32)),
                        span.range.clone(),
                    );
                }
            }
            if let Some(fam) = &span.style.font_family {
                if !fam.is_empty() {
                    builder.push(
                        StyleProperty::FontFamily(FontFamily::from(fam.as_str())),
                        span.range.clone(),
                    );
                }
            }
            if span.style.italic {
                builder.push(StyleProperty::FontStyle(FontStyle::Italic), span.range.clone());
            }
        }
    }
    push_small_caps_styles(&mut builder, smcp_ranges, synthetic_size_ranges, text.len());
    push_first_line_indent_box(
        &mut builder,
        indent,
        !text.is_empty() || !inline_boxes.is_empty(),
    );
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
    layout.break_all_lines(max_width);
    layout
}

fn is_sentence_end_before(text: &str, byte_idx: usize) -> bool {
    let prefix = &text[..byte_idx];
    for ch in prefix.chars().rev() {
        if matches!(ch, '”' | '"' | '\'' | '’' | ')' | ']' | '\u{2060}') {
            continue;
        }
        return matches!(ch, '.' | '?' | '!');
    }
    false
}

#[derive(Debug, Clone, Copy)]
enum KpBreakAction {
    ReplaceSpace { cur_start: usize, cur_end: usize },
    TakeSoftHyphen { cur_pos: usize },
    InsertBreakAfter { cur_pos: usize },
    ExistingBreak,
}

#[allow(clippy::too_many_arguments)]
fn measure_hyphen_width_cached(
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    cache: &mut std::collections::HashMap<(u32, u32, Option<String>, bool, u32), f64>,
    size: f32,
    weight: f32,
    family: Option<&str>,
    italic: bool,
    letter_spacing: f64,
) -> f64 {
    let key = (
        size.to_bits(),
        weight.to_bits(),
        family.map(|s| s.to_string()),
        italic,
        (letter_spacing as f32).to_bits(),
    );
    if let Some(&w) = cache.get(&key) {
        return w;
    }
    let mut builder = layout_cx.ranged_builder(font_cx, "-", 1.0, true);
    builder.push_default(StyleProperty::FontSize(size));
    if (weight - 400.0).abs() > 1.0 {
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
    }
    if let Some(fam) = family {
        if !fam.is_empty() {
            builder.push_default(StyleProperty::FontFamily(FontFamily::from(fam)));
        }
    }
    if italic {
        builder.push_default(StyleProperty::FontStyle(FontStyle::Italic));
    }
    if letter_spacing.abs() > 1e-4 {
        builder.push_default(StyleProperty::LetterSpacing(letter_spacing as f32));
    }
    let mut layout = builder.build("-");
    layout.break_all_lines(None);
    let w = layout.full_width() as f64;
    cache.insert(key, w);
    w
}

/// Runs Knuth–Plass optimal paragraph line breaking on `cur_text` (post-small-caps), splicing
/// the chosen breaks (and materializing/stripping discretionary `U+00AD` soft hyphens) directly
/// into `cur_text`, `pdf_text`, `cur_spans`, `cur_boxes`, and offset tables.
#[allow(clippy::too_many_arguments)]
fn apply_knuth_plass_breaks(
    font_cx: &mut FontContext,
    cur_text: &mut String,
    pdf_text: &mut String,
    cur_spans: &mut Vec<TextSpan>,
    cur_boxes: &mut Vec<InlineBoxSpan>,
    smcp_ranges: &mut Vec<(Range<usize>, bool)>,
    synthetic_size_ranges: &mut Vec<(Range<usize>, f32)>,
    shaped_to_pdf_entries: &mut Vec<OffsetEntry>,
    byte_deltas: &mut Vec<(usize, isize)>,
    font_size: f32,
    font_weight: f32,
    font_family: Option<&str>,
    font_italic: bool,
    letter_spacing: f64,
    line_height: Option<f64>,
    max_width: f64,
    align: Option<&str>,
    indent: f64,
    options: &ParagraphLayoutOptions,
) {
    use crate::compiler::knuth_plass::{self as kp, Item, INF_PENALTY, INF_STRETCH};

    // 1. Build a soft-hyphen-stripped `probe_text` with exact byte mapping back to `cur_text`.
    let mut probe_text = String::with_capacity(cur_text.len());
    let mut probe_spans = cur_spans.clone();
    let mut probe_boxes = cur_boxes.clone();
    let mut probe_smcp = smcp_ranges.clone();
    let mut probe_synth = synthetic_size_ranges.clone();

    // Maps each byte offset `p` in `0..=probe_text.len()` to the byte offset in `cur_text`
    // immediately BEFORE any soft hyphen at `p` (for range ends) and AFTER any soft hyphen at `p` (for range starts).
    let mut probe_to_cur_start: Vec<usize> = Vec::with_capacity(cur_text.len() + 1);
    let mut probe_to_cur_end: Vec<usize> = Vec::with_capacity(cur_text.len() + 1);
    let mut shys_at_probe: Vec<(usize, usize)> = Vec::new(); // (probe_pos, cur_pos)

    let mut cur_to_probe: Vec<usize> = vec![0; cur_text.len() + 1];
    let mut p_idx = 0usize;
    let mut i = 0usize;
    let bytes = cur_text.as_bytes();
    while i < bytes.len() {
        let ch = cur_text[i..].chars().next().unwrap();
        let ch_len = ch.len_utf8();
        if ch == SOFT_HYPHEN {
            shys_at_probe.push((p_idx, i));
            for k in 0..ch_len {
                cur_to_probe[i + k] = p_idx;
            }
            i += ch_len;
            continue;
        }
        // Record `probe_to_cur_end` at boundary before pushing new bytes if first byte
        if probe_to_cur_end.len() == p_idx {
            probe_to_cur_end.push(i);
        }
        for k in 0..ch_len {
            probe_to_cur_start.push(i + k);
            if k > 0 {
                probe_to_cur_end.push(i + k);
            }
            cur_to_probe[i + k] = p_idx + k;
        }
        probe_text.push(ch);
        p_idx += ch_len;
        probe_to_cur_end.push(i + ch_len);
        i += ch_len;
    }
    probe_to_cur_start.push(cur_text.len());
    if probe_to_cur_end.len() <= p_idx {
        probe_to_cur_end.push(cur_text.len());
    }
    cur_to_probe[cur_text.len()] = p_idx;

    if !shys_at_probe.is_empty() {
        for sp in &mut probe_spans {
            let s = cur_to_probe[sp.range.start.min(cur_text.len())];
            let e = cur_to_probe[sp.range.end.min(cur_text.len())];
            sp.range = s..e.max(s);
        }
        for ib in &mut probe_boxes {
            ib.index = cur_to_probe[ib.index.min(cur_text.len())];
        }
        for (r, _) in &mut probe_smcp {
            let s = cur_to_probe[r.start.min(cur_text.len())];
            let e = cur_to_probe[r.end.min(cur_text.len())];
            *r = s..e.max(s);
        }
        for (r, _) in &mut probe_synth {
            let s = cur_to_probe[r.start.min(cur_text.len())];
            let e = cur_to_probe[r.end.min(cur_text.len())];
            *r = s..e.max(s);
        }
    }

    // 2. Shape unconstrained with Parley to extract exact shaped glyph/space/inline-box advances.
    let mut probe_layout_cx = LayoutContext::<()>::new();
    let probe_layout = build_probe_layout(
        font_cx,
        &mut probe_layout_cx,
        &probe_text,
        font_size,
        font_weight,
        font_family,
        font_italic,
        letter_spacing,
        options.word_spacing,
        line_height,
        None,
        indent,
        &probe_spans,
        &probe_smcp,
        &probe_synth,
        &probe_boxes,
    );

    let is_justify_all = matches!(align, Some("justify_all") | Some("JustifyAll"));
    let is_justify = is_justify_all || matches!(align, Some("justify") | Some("Justify"));
    let is_center_or_right = matches!(
        align,
        Some("center") | Some("Center") | Some("right") | Some("Right") | Some("end") | Some("End")
    );
    let ragged_stretch = (font_size as f64).max(8.0) * 3.0;

    // 3. Convert Parley's unconstrained hard-break lines into Knuth–Plass items + break actions.
    let mut items: Vec<Item> = Vec::with_capacity(probe_text.len() / 2 + 16);
    let mut actions: Vec<Option<KpBreakAction>> = Vec::with_capacity(probe_text.len() / 2 + 16);
    let mut hyphen_cache: std::collections::HashMap<(u32, u32, Option<String>, bool, u32), f64> =
        std::collections::HashMap::new();

    #[derive(Debug, Clone)]
    enum TokenUnit {
        InlineBox { index: usize, width: f64, is_indent: bool },
        Cluster { range: Range<usize>, advance: f64 },
    }

    let lines: Vec<_> = probe_layout.lines().collect();
    let num_probe_lines = lines.len();

    let push_item = |items: &mut Vec<Item>,
                     actions: &mut Vec<Option<KpBreakAction>>,
                     item: Item,
                     action: Option<KpBreakAction>| {
        items.push(item);
        actions.push(action);
    };

    // Index into `shys_at_probe` (sorted ascending by probe_pos).
    let mut shy_cursor = 0usize;

    for (line_idx, line) in lines.into_iter().enumerate() {
        let mut units: Vec<TokenUnit> = Vec::new();
        let mut run_cursor: Option<(usize, usize)> = None;

        for item in line.items() {
            match item {
                parley::layout::PositionedLayoutItem::GlyphRun(gr) => {
                    let run = gr.run();
                    let glyph_count = gr.glyphs().count();
                    let start_g = match run_cursor {
                        Some((ri, idx)) if ri == run.index() => idx,
                        _ => 0,
                    };
                    let mut seen_g = 0usize;
                    let mut taken_g = 0usize;
                    for cluster in run.visual_clusters() {
                        let cg = cluster.glyphs().count();
                        if seen_g + cg <= start_g {
                            seen_g += cg;
                            continue;
                        }
                        if taken_g >= glyph_count {
                            break;
                        }
                        let tr = cluster.text_range();
                        let adv = cluster.advance() as f64;
                        units.push(TokenUnit::Cluster {
                            range: tr,
                            advance: adv,
                        });
                        seen_g += cg;
                        taken_g += cg;
                    }
                    run_cursor = Some((run.index(), start_g + taken_g));
                }
                parley::layout::PositionedLayoutItem::InlineBox(ib) => {
                    let is_indent = ib.id == INDENT_INLINE_BOX_ID;
                    let ib_index = if is_indent {
                        0
                    } else {
                        probe_boxes
                            .iter()
                            .find(|b| b.node_id.0 as u64 == ib.id)
                            .map(|b| b.index.min(probe_text.len()))
                            .unwrap_or(0)
                    };
                    units.push(TokenUnit::InlineBox {
                        index: ib_index,
                        width: ib.width as f64,
                        is_indent,
                    });
                }
            }
        }

        // Order units in logical text order (indent box first, then inline boxes before clusters at same index).
        units.sort_by_key(|u| match u {
            TokenUnit::InlineBox {
                index, is_indent, ..
            } => (*index, if *is_indent { 0u8 } else { 1u8 }),
            TokenUnit::Cluster { range, .. } => (range.start, 2u8),
        });

        let mut box_w = 0.0f64;
        let flush_box = |box_w: &mut f64,
                         items: &mut Vec<Item>,
                         actions: &mut Vec<Option<KpBreakAction>>| {
            if *box_w > 0.0 {
                items.push(Item::Box { width: *box_w });
                actions.push(None);
                *box_w = 0.0;
            }
        };

        for unit in units {
            match unit {
                TokenUnit::InlineBox { width, .. } => {
                    box_w += width.max(0.0);
                }
                TokenUnit::Cluster { range, advance } => {
                    let c_start = range.start.min(probe_text.len());
                    let c_end = range.end.min(probe_text.len());
                    if c_start >= c_end {
                        continue;
                    }
                    let slice = &probe_text[c_start..c_end];

                    // Advance past any soft hyphens strictly before `c_start`
                    while shy_cursor < shys_at_probe.len() && shys_at_probe[shy_cursor].0 < c_start {
                        shy_cursor += 1;
                    }

                    // Emit discretionary hyphen penalty for any soft hyphen sitting right at `c_start`
                    if shy_cursor < shys_at_probe.len() && shys_at_probe[shy_cursor].0 == c_start {
                        let (_, shy_cur_pos) = shys_at_probe[shy_cursor];
                        shy_cursor += 1;
                        if box_w > 0.0 {
                            flush_box(&mut box_w, &mut items, &mut actions);
                            // Resolve effective font style at `shy_cur_pos`
                            let mut h_size = font_size;
                            let mut h_weight = font_weight;
                            let mut h_family = font_family;
                            let mut h_italic = font_italic;
                            for sp in cur_spans.iter() {
                                if sp.range.start <= shy_cur_pos && shy_cur_pos < sp.range.end {
                                    if let Some(sz) = sp.style.font_size {
                                        if sz > 0.0 {
                                            h_size = sz as f32;
                                        }
                                    }
                                    if let Some(wt) = sp.style.font_weight {
                                        if wt > 0.0 {
                                            h_weight = wt as f32;
                                        }
                                    }
                                    if let Some(ref fam) = sp.style.font_family {
                                        if !fam.is_empty() {
                                            h_family = Some(fam.as_str());
                                        }
                                    }
                                    if sp.style.italic {
                                        h_italic = true;
                                    }
                                }
                            }
                            for (sr, sz) in synthetic_size_ranges.iter() {
                                if sr.start <= shy_cur_pos && shy_cur_pos < sr.end {
                                    h_size = *sz;
                                }
                            }
                            let hyphen_w = measure_hyphen_width_cached(
                                font_cx,
                                &mut probe_layout_cx,
                                &mut hyphen_cache,
                                h_size,
                                h_weight,
                                h_family,
                                h_italic,
                                letter_spacing,
                            );
                            push_item(
                                &mut items,
                                &mut actions,
                                Item::Penalty {
                                    width: hyphen_w,
                                    penalty: options.hyphen_penalty,
                                    flagged: true,
                                },
                                Some(KpBreakAction::TakeSoftHyphen {
                                    cur_pos: shy_cur_pos,
                                }),
                            );
                        }
                    }

                    // Zero-width markers (`\n`, `\u{2060}`) are handled at line end.
                    if slice == "\n" || slice == "\r\n" || slice == "\u{2060}" {
                        continue;
                    }

                    // Normal breakable space (`' '`)
                    if slice == " " {
                        flush_box(&mut box_w, &mut items, &mut actions);
                        let base_w = advance.max(0.0);
                        let is_sent = options.sentence_space != 0.0
                            && is_sentence_end_before(&probe_text, c_start);
                        let (eff_w, eff_y, eff_z) = if is_sent {
                            let s = options.sentence_space;
                            (
                                (base_w * (1.0 + s)).max(0.0),
                                (base_w * options.space_stretch * (1.0 + 2.0 * s.max(0.0))).max(0.0),
                                (base_w * options.space_shrink / (1.0 + s.max(0.0))).max(0.0),
                            )
                        } else {
                            (
                                base_w,
                                base_w * options.space_stretch,
                                base_w * options.space_shrink,
                            )
                        };
                        let cur_s = probe_to_cur_start[c_start];
                        let cur_e = probe_to_cur_end[c_end];
                        if is_justify {
                            push_item(
                                &mut items,
                                &mut actions,
                                Item::Glue {
                                    width: eff_w,
                                    stretch: eff_y,
                                    shrink: eff_z,
                                },
                                Some(KpBreakAction::ReplaceSpace {
                                    cur_start: cur_s,
                                    cur_end: cur_e,
                                }),
                            );
                        } else {
                            // Knuth–Plass 1981 Section 4 ragged-right triple:
                            // Penalty(+INF), Glue(0, ragged_stretch, 0), Penalty(0) [BREAK], Glue(w, -ragged_stretch, 0)
                            push_item(
                                &mut items,
                                &mut actions,
                                Item::Penalty {
                                    width: 0.0,
                                    penalty: INF_PENALTY,
                                    flagged: false,
                                },
                                None,
                            );
                            push_item(
                                &mut items,
                                &mut actions,
                                Item::Glue {
                                    width: 0.0,
                                    stretch: ragged_stretch,
                                    shrink: 0.0,
                                },
                                None,
                            );
                            push_item(
                                &mut items,
                                &mut actions,
                                Item::Penalty {
                                    width: 0.0,
                                    penalty: 0.0,
                                    flagged: false,
                                },
                                Some(KpBreakAction::ReplaceSpace {
                                    cur_start: cur_s,
                                    cur_end: cur_e,
                                }),
                            );
                            push_item(
                                &mut items,
                                &mut actions,
                                Item::Glue {
                                    width: eff_w,
                                    stretch: -ragged_stretch,
                                    shrink: 0.0,
                                },
                                None,
                            );
                        }
                        continue;
                    }

                    // Non-breaking space (`\u{00A0}`)
                    if slice == "\u{00A0}" {
                        flush_box(&mut box_w, &mut items, &mut actions);
                        let w = advance.max(0.0);
                        if is_justify {
                            push_item(
                                &mut items,
                                &mut actions,
                                Item::Penalty {
                                    width: 0.0,
                                    penalty: INF_PENALTY,
                                    flagged: false,
                                },
                                None,
                            );
                            push_item(
                                &mut items,
                                &mut actions,
                                Item::Glue {
                                    width: w,
                                    stretch: w * options.space_stretch,
                                    shrink: w * options.space_shrink,
                                },
                                None,
                            );
                        } else {
                            box_w += w;
                        }
                        continue;
                    }

                    // Regular text cluster
                    box_w += advance.max(0.0);

                    // Check explicit breakable hyphen (`'-'`, NOT `\=` which is `"\u{2060}-\u{2060}"`)
                    if slice.ends_with('-') {
                        let prev_ch = probe_text[..c_end - 1].chars().next_back();
                        let next_ch = probe_text[c_end..].chars().next();
                        let glued = prev_ch == Some('\u{2060}') || next_ch == Some('\u{2060}');
                        let prev_ok = prev_ch.is_some_and(|c| !c.is_whitespace() && c != '-');
                        let next_ok = next_ch.is_some_and(|c| {
                            !c.is_whitespace()
                                && c != '\n'
                                && c != '-'
                                && !matches!(c, '”' | '’' | '"' | '\'' | ')' | ']' | ',' | '.' | ';' | ':' | '!' | '?')
                        });
                        if !glued && prev_ok && next_ok {
                            flush_box(&mut box_w, &mut items, &mut actions);
                            let cur_pos = probe_to_cur_end[c_end];
                            push_item(
                                &mut items,
                                &mut actions,
                                Item::Penalty {
                                    width: 0.0,
                                    penalty: options.hyphen_penalty,
                                    flagged: true,
                                },
                                Some(KpBreakAction::InsertBreakAfter { cur_pos }),
                            );
                        }
                    } else if slice.ends_with('\u{2014}') {
                        // Internal em-dash (`—`): allow break right after `—` only when preceded by
                        // non-opening content (never break after opening `“—` or `(—`) and followed
                        // by non-closing content.
                        let dash_start = c_end - '\u{2014}'.len_utf8();
                        let prev_ch = probe_text[..dash_start].chars().next_back();
                        let prev_ok = prev_ch.is_some_and(|c| {
                            !c.is_whitespace()
                                && c != '\n'
                                && !matches!(c, '“' | '‘' | '"' | '\'' | '(' | '[' | '\u{2060}')
                        });
                        let next_ch = probe_text[c_end..].chars().next();
                        let next_ok = next_ch.is_some_and(|c| {
                            !c.is_whitespace()
                                && c != '\n'
                                && c != '\u{2060}'
                                && c != '\u{2014}'
                                && !matches!(c, '”' | '’' | '"' | '\'' | ')' | ']' | ',' | '.' | ';' | ':' | '!' | '?')
                        });
                        if prev_ok && next_ok {
                            flush_box(&mut box_w, &mut items, &mut actions);
                            let cur_pos = probe_to_cur_end[c_end];
                            push_item(
                                &mut items,
                                &mut actions,
                                Item::Penalty {
                                    width: 0.0,
                                    penalty: options.hyphen_penalty,
                                    flagged: false,
                                },
                                Some(KpBreakAction::InsertBreakAfter { cur_pos }),
                            );
                        }
                    }
                }
            }
        }

        flush_box(&mut box_w, &mut items, &mut actions);

        // Terminate this hard-break segment cleanly in the Knuth–Plass stream.
        let is_last_line = line_idx + 1 == num_probe_lines;
        let line_slice = probe_text.get(line.text_range()).unwrap_or("");
        let is_forced_justify = !is_last_line && line_slice.ends_with(FORCED_JUSTIFY_BREAK);

        let needs_inf_stretch = if is_justify_all {
            false
        } else if is_justify {
            !is_forced_justify
        } else {
            true
        };

        if needs_inf_stretch {
            push_item(
                &mut items,
                &mut actions,
                Item::Glue {
                    width: 0.0,
                    stretch: INF_STRETCH,
                    shrink: 0.0,
                },
                None,
            );
        }
        push_item(
            &mut items,
            &mut actions,
            Item::Penalty {
                width: 0.0,
                penalty: -INF_PENALTY,
                flagged: is_last_line,
            },
            Some(KpBreakAction::ExistingBreak),
        );
    }

    // 4. Solve optimal breakpoints via Knuth–Plass!
    let kp_measure = if is_center_or_right {
        (max_width - 0.05).max(1.0)
    } else {
        max_width
    };
    let result = kp::break_items(&items, |_| kp_measure, options);

    // 5. Collect splices: chosen break points + stripping untaken soft hyphens.
    let break_str = if is_justify {
        FORCED_JUSTIFY_BREAK
    } else {
        "\n"
    };
    let hyphen_break_str = if is_justify {
        "-\u{2060}\n"
    } else {
        "-\n"
    };

    let mut taken_shys: std::collections::HashSet<usize> = std::collections::HashSet::new();
    // Each edit: (orig_sc_pos, pdf_pos, old_len, replacement)
    let mut edits: Vec<(usize, usize, usize, &'static str)> = Vec::new();

    for &b_idx in &result.breaks {
        let Some(action) = actions.get(b_idx).and_then(|a| *a) else {
            continue;
        };
        match action {
            KpBreakAction::ReplaceSpace { cur_start, cur_end } => {
                let pdf_pos = map_pos_floor(cur_start, shaped_to_pdf_entries);
                edits.push((cur_start, pdf_pos, cur_end - cur_start, break_str));
            }
            KpBreakAction::TakeSoftHyphen { cur_pos } => {
                taken_shys.insert(cur_pos);
                let pdf_pos = map_pos_floor(cur_pos, shaped_to_pdf_entries);
                edits.push((cur_pos, pdf_pos, SOFT_HYPHEN_LEN, hyphen_break_str));
            }
            KpBreakAction::InsertBreakAfter { cur_pos } => {
                let pdf_pos = map_pos_floor(cur_pos, shaped_to_pdf_entries);
                edits.push((cur_pos, pdf_pos, 0, break_str));
            }
            KpBreakAction::ExistingBreak => {}
        }
    }

    for &(_, shy_cur_pos) in &shys_at_probe {
        if !taken_shys.contains(&shy_cur_pos) {
            let pdf_pos = map_pos_floor(shy_cur_pos, shaped_to_pdf_entries);
            edits.push((shy_cur_pos, pdf_pos, SOFT_HYPHEN_LEN, ""));
        }
    }

    // Apply edits from back to front so earlier byte offsets remain invariant!
    edits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.2.cmp(&a.2)));
    let mut dummy_shys: Vec<(usize, usize)> = Vec::new();
    for (sc_pos, pdf_pos, old_len, repl) in edits {
        splice_prepared_rich_text(
            cur_text,
            pdf_text,
            cur_spans,
            cur_boxes,
            smcp_ranges,
            synthetic_size_ranges,
            shaped_to_pdf_entries,
            &mut dummy_shys,
            sc_pos,
            pdf_pos,
            old_len,
            repl,
        );
        let delta = (repl.len() as isize) - (old_len as isize);
        if delta != 0 {
            byte_deltas.push((sc_pos, delta));
        }
    }
    byte_deltas.sort_by_key(|&(pos, _)| pos);
}

/// Resolves small caps (native OpenType `smcp` or synthesized uppercase+scale), discretionary soft
/// hyphens (`U+00AD`), and optional Knuth–Plass optimal line breaking for `max_width`.
/// Returns `None` on the zero-cost fast path when none of these transformations are active.
#[allow(clippy::too_many_arguments)]
pub fn prepare_rich_text_for_shaping(
    font_cx: &mut FontContext,
    text: &str,
    font_size: f32,
    font_weight: f32,
    font_family: Option<&str>,
    font_italic: bool,
    font_small_caps: bool,
    line_height: Option<f64>,
    max_width: Option<f64>,
    indent: f64,
    spans: &[TextSpan],
    inline_boxes: &[InlineBoxSpan],
) -> Option<PreparedRichText> {
    prepare_rich_text_for_shaping_with_options(
        font_cx,
        text,
        font_size,
        font_weight,
        font_family,
        font_italic,
        font_small_caps,
        0.0,
        line_height,
        max_width,
        None,
        indent,
        spans,
        inline_boxes,
        &ParagraphLayoutOptions::default(),
    )
}

/// Full rich-text preparation supporting small caps, soft hyphens, and Knuth–Plass line breaking.
#[allow(clippy::too_many_arguments)]
pub fn prepare_rich_text_for_shaping_with_options(
    font_cx: &mut FontContext,
    text: &str,
    font_size: f32,
    font_weight: f32,
    font_family: Option<&str>,
    font_italic: bool,
    font_small_caps: bool,
    letter_spacing: f64,
    line_height: Option<f64>,
    max_width: Option<f64>,
    align: Option<&str>,
    indent: f64,
    spans: &[TextSpan],
    inline_boxes: &[InlineBoxSpan],
    options: &ParagraphLayoutOptions,
) -> Option<PreparedRichText> {
    let any_small_caps =
        font_small_caps || spans.iter().any(|s| s.style.small_caps == Some(true));
    let has_shy_raw = text.contains(SOFT_HYPHEN);
    let run_kp = options.is_optimal() && max_width.is_some_and(|w| w > 0.0);
    if !any_small_caps && !has_shy_raw && !run_kp {
        return None;
    }

    let mut cur_text = String::with_capacity(text.len());
    let mut cur_spans = spans.to_vec();
    let mut cur_boxes = inline_boxes.to_vec();
    let mut smcp_ranges: Vec<(Range<usize>, bool)> = Vec::new();
    let mut synthetic_size_ranges: Vec<(Range<usize>, f32)> = Vec::new();
    let mut orig_to_sc_entries: Vec<OffsetEntry> = Vec::new();
    let mut shaped_to_pdf_entries: Vec<OffsetEntry> = Vec::new();

    if any_small_caps {
        let mut boundaries: Vec<usize> = Vec::with_capacity(spans.len() * 2 + 2);
        boundaries.push(0);
        boundaries.push(text.len());
        for s in spans {
            if !s.range.is_empty() {
                let cs = s.range.start.min(text.len());
                let ce = s.range.end.min(text.len());
                if cs < ce && text.is_char_boundary(cs) && text.is_char_boundary(ce) {
                    boundaries.push(cs);
                    boundaries.push(ce);
                }
            }
        }
        boundaries.sort_unstable();
        boundaries.dedup();

        for w in boundaries.windows(2) {
            let (seg_start, seg_end) = (w[0], w[1]);
            if seg_start >= seg_end {
                continue;
            }
            let mut eff_size = font_size as f64;
            let mut eff_weight = font_weight;
            let mut eff_family = font_family;
            let mut eff_italic = font_italic;
            let mut eff_small_caps = font_small_caps;
            for sp in spans {
                let rs = sp.range.start.min(text.len());
                let re = sp.range.end.min(text.len());
                if rs <= seg_start && seg_end <= re {
                    if let Some(sz) = sp.style.font_size {
                        if sz > 0.0 {
                            eff_size = sz;
                        }
                    }
                    if let Some(wt) = sp.style.font_weight {
                        if wt > 0.0 {
                            eff_weight = wt as f32;
                        }
                    }
                    if let Some(ref fam) = sp.style.font_family {
                        if !fam.is_empty() {
                            eff_family = Some(fam.as_str());
                        }
                    }
                    if sp.style.italic {
                        eff_italic = true;
                    }
                    if let Some(sc) = sp.style.small_caps {
                        eff_small_caps = sc;
                    }
                }
            }

            let seg_slice = &text[seg_start..seg_end];
            if !eff_small_caps {
                cur_text.push_str(seg_slice);
                continue;
            }

            if font_supports_smcp_with_font_cx(font_cx, eff_family, eff_weight, eff_italic) {
                let out_start = cur_text.len();
                cur_text.push_str(seg_slice);
                let out_end = cur_text.len();
                if smcp_ranges
                    .last()
                    .is_some_and(|(last_r, enabled)| *enabled && last_r.end == out_start)
                {
                    smcp_ranges.last_mut().unwrap().0.end = out_end;
                } else {
                    smcp_ranges.push((out_start..out_end, true));
                }
            } else {
                let scaled_sz = (eff_size * SYNTHETIC_SMALL_CAPS_SCALE) as f32;
                for (rel_off, ch) in seg_slice.char_indices() {
                    let orig_pos = seg_start + rel_off;
                    let ch_len = ch.len_utf8();
                    let out_start = cur_text.len();
                    if ch.is_lowercase() {
                        let mut upper_chars = 0usize;
                        for uch in ch.to_uppercase() {
                            cur_text.push(uch);
                            upper_chars += 1;
                        }
                        let out_end = cur_text.len();
                        let upper_len = out_end - out_start;
                        if upper_len != ch_len || upper_chars != 1 || ch_len > 1 {
                            orig_to_sc_entries.push((orig_pos, ch_len, out_start, upper_len));
                            shaped_to_pdf_entries.push((out_start, upper_len, orig_pos, ch_len));
                        }
                        if synthetic_size_ranges.last().is_some_and(|(last_r, prev_sz)| {
                            last_r.end == out_start && (*prev_sz - scaled_sz).abs() < 1e-4
                        }) {
                            synthetic_size_ranges.last_mut().unwrap().0.end = out_end;
                        } else {
                            synthetic_size_ranges.push((out_start..out_end, scaled_sz));
                        }
                    } else {
                        cur_text.push(ch);
                    }
                }
            }
        }

        if !orig_to_sc_entries.is_empty() {
            for span in cur_spans.iter_mut() {
                let s = map_pos_floor(span.range.start, &orig_to_sc_entries);
                let e = map_pos_ceil(span.range.end, &orig_to_sc_entries).max(s);
                span.range = s..e;
            }
            for ib in cur_boxes.iter_mut() {
                ib.index = map_pos_floor(ib.index, &orig_to_sc_entries);
            }
        }
    } else {
        cur_text.push_str(text);
    }

    let mut pdf_text = text.to_string();
    let mut byte_deltas: Vec<(usize, isize)> = Vec::new();
    let mut has_explicit_breaks = false;

    if run_kp {
        if let Some(w) = max_width.filter(|&w| w > 0.0) {
            apply_knuth_plass_breaks(
                font_cx,
                &mut cur_text,
                &mut pdf_text,
                &mut cur_spans,
                &mut cur_boxes,
                &mut smcp_ranges,
                &mut synthetic_size_ranges,
                &mut shaped_to_pdf_entries,
                &mut byte_deltas,
                font_size,
                font_weight,
                font_family,
                font_italic,
                letter_spacing,
                line_height,
                w,
                align,
                indent,
                options,
            );
            has_explicit_breaks = true;
        }
    } else if cur_text.contains(SOFT_HYPHEN) {
        let mut shy_sc_and_pdf: Vec<(usize, usize)> = cur_text
            .match_indices(SOFT_HYPHEN)
            .map(|(idx, _)| idx)
            .zip(pdf_text.match_indices(SOFT_HYPHEN).map(|(idx, _)| idx))
            .collect();
        let mut orig_sc_shys: Vec<usize> = cur_text
            .match_indices(SOFT_HYPHEN)
            .map(|(idx, _)| idx)
            .collect();
        byte_deltas.reserve(orig_sc_shys.len());

        if let Some(w) = max_width.filter(|&w| w > 0.0) {
            let mut probe_layout_cx = LayoutContext::<()>::new();
            let w_f32 = w as f32;

            while cur_text.contains(SOFT_HYPHEN) {
                let layout = build_probe_layout(
                    font_cx,
                    &mut probe_layout_cx,
                    &cur_text,
                    font_size,
                    font_weight,
                    font_family,
                    font_italic,
                    letter_spacing,
                    options.word_spacing,
                    line_height,
                    Some(w_f32),
                    indent,
                    &cur_spans,
                    &smcp_ranges,
                    &synthetic_size_ranges,
                    &cur_boxes,
                );
                let lines: Vec<_> = layout.lines().collect();
                let num_lines = lines.len();
                let mut found_break: Option<(usize, usize)> = None;
                for (idx, line) in lines.iter().enumerate() {
                    if idx + 1 >= num_lines {
                        break;
                    }
                    let r = line.text_range();
                    if r.end >= SOFT_HYPHEN_LEN && cur_text[..r.end].ends_with(SOFT_HYPHEN) {
                        found_break = Some((idx, r.end - SOFT_HYPHEN_LEN));
                        break;
                    }
                }

                let Some((line_idx, sc_shy_pos)) = found_break else {
                    break;
                };

                let k = cur_text[..sc_shy_pos].matches(SOFT_HYPHEN).count();
                let orig_sc_pos = orig_sc_shys.remove(k);
                let (_, pdf_shy_pos) = shy_sc_and_pdf.remove(k);

                splice_prepared_rich_text(
                    &mut cur_text,
                    &mut pdf_text,
                    &mut cur_spans,
                    &mut cur_boxes,
                    &mut smcp_ranges,
                    &mut synthetic_size_ranges,
                    &mut shaped_to_pdf_entries,
                    &mut shy_sc_and_pdf,
                    sc_shy_pos,
                    pdf_shy_pos,
                    SOFT_HYPHEN_LEN,
                    "-",
                );
                // Find shifted pdf_shy_pos for potential rollback below (at pdf_shy_pos with len 1)
                let inserted_pdf_pos = pdf_shy_pos;

                let verify_layout = build_probe_layout(
                    font_cx,
                    &mut probe_layout_cx,
                    &cur_text,
                    font_size,
                    font_weight,
                    font_family,
                    font_italic,
                    letter_spacing,
                    options.word_spacing,
                    line_height,
                    Some(w_f32),
                    indent,
                    &cur_spans,
                    &smcp_ranges,
                    &synthetic_size_ranges,
                    &cur_boxes,
                );
                let fits_with_hyphen = verify_layout.lines().nth(line_idx).is_some_and(|l| {
                    let nr = l.text_range();
                    nr.start <= sc_shy_pos && sc_shy_pos < nr.end
                });

                if fits_with_hyphen {
                    byte_deltas.push((orig_sc_pos, -1));
                } else {
                    splice_prepared_rich_text(
                        &mut cur_text,
                        &mut pdf_text,
                        &mut cur_spans,
                        &mut cur_boxes,
                        &mut smcp_ranges,
                        &mut synthetic_size_ranges,
                        &mut shaped_to_pdf_entries,
                        &mut shy_sc_and_pdf,
                        sc_shy_pos,
                        inserted_pdf_pos,
                        1,
                        "",
                    );
                    byte_deltas.push((orig_sc_pos, -(SOFT_HYPHEN_LEN as isize)));
                }
            }
        }

        while let Some(sc_shy_pos) = cur_text.find(SOFT_HYPHEN) {
            let orig_sc_pos = orig_sc_shys.remove(0);
            let (_, pdf_shy_pos) = shy_sc_and_pdf.remove(0);
            splice_prepared_rich_text(
                &mut cur_text,
                &mut pdf_text,
                &mut cur_spans,
                &mut cur_boxes,
                &mut smcp_ranges,
                &mut synthetic_size_ranges,
                &mut shaped_to_pdf_entries,
                &mut shy_sc_and_pdf,
                sc_shy_pos,
                pdf_shy_pos,
                SOFT_HYPHEN_LEN,
                "",
            );
            byte_deltas.push((orig_sc_pos, -(SOFT_HYPHEN_LEN as isize)));
        }

        byte_deltas.sort_by_key(|&(pos, _)| pos);
    }

    Some(PreparedRichText {
        text: cur_text,
        pdf_text,
        spans: cur_spans,
        inline_boxes: cur_boxes,
        smcp_ranges,
        synthetic_size_ranges,
        byte_deltas,
        orig_to_sc_entries,
        shaped_to_pdf_entries,
        has_explicit_breaks,
    })
}

/// Resolves discretionary soft hyphens (`U+00AD`) for `max_width` using `font_cx`.
/// Returns `None` on the zero-cost fast path when `text` contains no `U+00AD`.
#[allow(clippy::too_many_arguments)]
pub fn materialize_soft_hyphens_with_font_cx(
    font_cx: &mut FontContext,
    text: &str,
    font_size: f32,
    font_weight: f32,
    font_family: Option<&str>,
    font_italic: bool,
    line_height: Option<f64>,
    max_width: Option<f64>,
    indent: f64,
    spans: &[TextSpan],
    inline_boxes: &[InlineBoxSpan],
) -> Option<MaterializedRichText> {
    if !text.contains(SOFT_HYPHEN) {
        return None;
    }
    let prep = prepare_rich_text_for_shaping(
        font_cx,
        text,
        font_size,
        font_weight,
        font_family,
        font_italic,
        false,
        line_height,
        max_width,
        indent,
        spans,
        inline_boxes,
    )?;
    Some(MaterializedRichText {
        text: prep.text,
        spans: prep.spans,
        inline_boxes: prep.inline_boxes,
        byte_deltas: prep.byte_deltas,
    })
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
    measure_rich_text_bounds_with_boxes_and_spacing(
        text,
        font_size,
        font_weight,
        font_family,
        0.0,
        max_width,
        end_at,
        start_at,
        line_height,
        0.0,
        spans,
        inline_boxes,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn measure_rich_text_bounds_with_boxes_and_spacing(
    text: &str,
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
    letter_spacing: f64,
    max_width: Option<f64>,
    end_at: Option<&str>,
    start_at: Option<&str>,
    line_height: Option<f64>,
    indent: f64,
    spans: &[TextSpan],
    inline_boxes: &[InlineBoxSpan],
    font_small_caps: bool,
) -> (f64, f64) {
    measure_rich_text_bounds_full(
        text,
        font_size,
        font_weight,
        font_family,
        false,
        letter_spacing,
        max_width,
        end_at,
        start_at,
        line_height,
        None,
        indent,
        spans,
        inline_boxes,
        font_small_caps,
        &ParagraphLayoutOptions::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn measure_rich_text_bounds_full(
    text: &str,
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
    font_italic: bool,
    letter_spacing: f64,
    max_width: Option<f64>,
    end_at: Option<&str>,
    start_at: Option<&str>,
    line_height: Option<f64>,
    align: Option<&str>,
    indent: f64,
    spans: &[TextSpan],
    inline_boxes: &[InlineBoxSpan],
    font_small_caps: bool,
    options: &ParagraphLayoutOptions,
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

            let prep = prepare_rich_text_for_shaping_with_options(
                &mut font_cx,
                text,
                size,
                weight,
                font_family,
                font_italic,
                font_small_caps,
                letter_spacing,
                line_height,
                max_width,
                align,
                indent,
                spans,
                inline_boxes,
                options,
            );
            let has_explicit_breaks = prep.as_ref().is_some_and(|p| p.has_explicit_breaks);
            let (eff_text, eff_spans, eff_boxes, smcp_ranges, synth_ranges): (
                &str,
                &[TextSpan],
                &[InlineBoxSpan],
                &[(Range<usize>, bool)],
                &[(Range<usize>, f32)],
            ) = match &prep {
                Some(p) => (
                    p.text.as_str(),
                    p.spans.as_slice(),
                    p.inline_boxes.as_slice(),
                    p.smcp_ranges.as_slice(),
                    p.synthetic_size_ranges.as_slice(),
                ),
                None => (text, spans, inline_boxes, &[], &[]),
            };

            let mut builder = layout_cx.ranged_builder(&mut font_cx, eff_text, 1.0, true);
            builder.push_default(StyleProperty::FontSize(size));
            if (weight - 400.0).abs() > 1.0 {
                builder.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
            }
            if let Some(family) = font_family {
                if !family.is_empty() {
                    builder.push_default(StyleProperty::FontFamily(FontFamily::from(family)));
                }
            }
            if font_italic {
                builder.push_default(StyleProperty::FontStyle(FontStyle::Italic));
            }
            if letter_spacing.abs() > 1e-4 {
                builder.push_default(StyleProperty::LetterSpacing(letter_spacing as f32));
            }
            if options.word_spacing.abs() > 1e-4 {
                builder.push_default(StyleProperty::WordSpacing(options.word_spacing as f32));
            }
            if let Some(lh) = line_height {
                if lh > 0.0 {
                    builder.push_default(StyleProperty::LineHeight(parley::style::LineHeight::Absolute(lh as f32)));
                }
            }
            for span in eff_spans {
                if !span.range.is_empty() && span.range.end <= eff_text.len() {
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
            push_small_caps_styles(&mut builder, smcp_ranges, synth_ranges, eff_text.len());
            push_first_line_indent_box(
                &mut builder,
                indent,
                !eff_text.is_empty() || !eff_boxes.is_empty(),
            );
            for ib in eff_boxes {
                builder.push_inline_box(parley::InlineBox {
                    id: ib.node_id.0 as u64,
                    kind: parley::InlineBoxKind::InFlow,
                    index: ib.index.min(eff_text.len()),
                    width: ib.width.max(0.0) as f32,
                    height: ib.baseline_offset.max(0.0) as f32,
                });
            }

            let mut layout = builder.build(eff_text);
            break_lines_for_layout(&mut layout, max_width, has_explicit_breaks, align);

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
    pub byte_deltas: Vec<(usize, isize)>,
    pub sc_entries: Vec<(usize, usize, usize, usize)>,
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
            .find(|&(_, ch)| !ch.is_whitespace() && ch != SOFT_HYPHEN)
            .map(|(idx, _)| {
                let sc = map_pos_floor(idx, &self.sc_entries);
                map_byte_offset_with_deltas(sc, &self.byte_deltas)
            });
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
    indent: f64,
    font_italic: bool,
    font_small_caps: bool,
    spans: &[TextSpan],
    inline_boxes: &[InlineBoxSpan],
) -> ShapedParagraph {
    shape_paragraph_lines_with_options(
        text,
        font_size,
        font_weight,
        font_family,
        0.0,
        max_width,
        end_at,
        start_at,
        line_height,
        align,
        indent,
        font_italic,
        font_small_caps,
        spans,
        inline_boxes,
        &ParagraphLayoutOptions::default(),
    )
}

/// Shapes a paragraph at constant `max_width` once into line slices using exact `Text` landmark geometry
/// and explicit `ParagraphLayoutOptions` (including Knuth–Plass line breaking and `word_spacing`).
#[allow(clippy::too_many_arguments)]
pub fn shape_paragraph_lines_with_options(
    text: &str,
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
    letter_spacing: f64,
    max_width: Option<f64>,
    end_at: Option<&str>,
    start_at: Option<&str>,
    line_height: Option<f64>,
    align: Option<&str>,
    indent: f64,
    font_italic: bool,
    font_small_caps: bool,
    spans: &[TextSpan],
    inline_boxes: &[InlineBoxSpan],
    options: &ParagraphLayoutOptions,
) -> ShapedParagraph {
    if text.is_empty() && inline_boxes.is_empty() {
        return ShapedParagraph {
            lines: Vec::new(),
            total_width: 0.0,
            total_height: 0.0,
            first_top_coord: 0.0,
            last_bottom_coord: 0.0,
            full_y_offset: 0.0,
            byte_deltas: Vec::new(),
            sc_entries: Vec::new(),
        };
    }

    let size = if font_size > 0.0 { font_size as f32 } else { 16.0 };
    let weight = if font_weight > 0.0 { font_weight as f32 } else { 400.0 };
    let m = measure_font_metrics(font_size, font_weight, font_family);

    FONT_CONTEXT.with(|font_cx_cell| {
        LAYOUT_CONTEXT.with(|layout_cx_cell| {
            let mut font_cx = font_cx_cell.borrow_mut();
            let mut layout_cx = layout_cx_cell.borrow_mut();

            let prep = prepare_rich_text_for_shaping_with_options(
                &mut font_cx,
                text,
                size,
                weight,
                font_family,
                font_italic,
                font_small_caps,
                letter_spacing,
                line_height,
                max_width,
                align,
                indent,
                spans,
                inline_boxes,
                options,
            );
            let has_explicit_breaks = prep.as_ref().is_some_and(|p| p.has_explicit_breaks);
            let (eff_text, eff_spans, eff_boxes, smcp_ranges, synth_ranges, byte_deltas, sc_entries): (
                &str,
                &[TextSpan],
                &[InlineBoxSpan],
                &[(Range<usize>, bool)],
                &[(Range<usize>, f32)],
                Vec<(usize, isize)>,
                Vec<(usize, usize, usize, usize)>,
            ) = match &prep {
                Some(p) => (
                    p.text.as_str(),
                    p.spans.as_slice(),
                    p.inline_boxes.as_slice(),
                    p.smcp_ranges.as_slice(),
                    p.synthetic_size_ranges.as_slice(),
                    p.byte_deltas.clone(),
                    p.orig_to_sc_entries.clone(),
                ),
                None => (text, spans, inline_boxes, &[], &[], Vec::new(), Vec::new()),
            };

            let mut builder = layout_cx.ranged_builder(&mut font_cx, eff_text, 1.0, true);
            builder.push_default(StyleProperty::FontSize(size));
            if (weight - 400.0).abs() > 1.0 {
                builder.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
            }
            if let Some(family) = font_family {
                if !family.is_empty() {
                    builder.push_default(StyleProperty::FontFamily(FontFamily::from(family)));
                }
            }
            if font_italic {
                builder.push_default(StyleProperty::FontStyle(FontStyle::Italic));
            }
            if letter_spacing.abs() > 1e-4 {
                builder.push_default(StyleProperty::LetterSpacing(letter_spacing as f32));
            }
            if options.word_spacing.abs() > 1e-4 {
                builder.push_default(StyleProperty::WordSpacing(options.word_spacing as f32));
            }
            if let Some(lh) = line_height {
                if lh > 0.0 {
                    builder.push_default(StyleProperty::LineHeight(
                        parley::style::LineHeight::Absolute(lh as f32),
                    ));
                }
            }
            for span in eff_spans {
                if !span.range.is_empty() && span.range.end <= eff_text.len() {
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
            push_small_caps_styles(&mut builder, smcp_ranges, synth_ranges, eff_text.len());
            push_first_line_indent_box(
                &mut builder,
                indent,
                !eff_text.is_empty() || !eff_boxes.is_empty(),
            );
            for ib in eff_boxes {
                builder.push_inline_box(parley::InlineBox {
                    id: ib.node_id.0 as u64,
                    kind: parley::InlineBoxKind::InFlow,
                    index: ib.index.min(eff_text.len()),
                    width: ib.width.max(0.0) as f32,
                    height: ib.baseline_offset.max(0.0) as f32,
                });
            }

            let mut layout = builder.build(eff_text);
            break_lines_for_layout(&mut layout, max_width, has_explicit_breaks, align);
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
                            if let Some((ib_idx, ib)) = eff_boxes
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
                byte_deltas,
                sc_entries,
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
        0.0,
        false,
        false,
        spans,
        &[],
        child_shifts,
        origin_x,
        origin_y,
    )
}

/// Measures a rich text layout (with inline boxes) and returns full `SpanLayoutMetrics`
/// for every inline `NodeId` referenced in `spans`.
#[allow(clippy::too_many_arguments)]
pub fn compute_span_fragments_with_metrics_and_boxes(
    text: &str,
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
    max_width: Option<f64>,
    line_height: Option<f64>,
    align: Option<&str>,
    indent: f64,
    font_italic: bool,
    font_small_caps: bool,
    spans: &[TextSpan],
    inline_boxes: &[InlineBoxSpan],
    child_shifts: &HashMap<NodeId, f64>,
    origin_x: f64,
    origin_y: f64,
) -> HashMap<NodeId, SpanLayoutMetrics> {
    compute_span_fragments_with_options(
        text,
        font_size,
        font_weight,
        font_family,
        0.0,
        max_width,
        line_height,
        align,
        indent,
        font_italic,
        font_small_caps,
        spans,
        inline_boxes,
        child_shifts,
        origin_x,
        origin_y,
        &ParagraphLayoutOptions::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn compute_span_fragments_with_options(
    text: &str,
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
    letter_spacing: f64,
    max_width: Option<f64>,
    line_height: Option<f64>,
    align: Option<&str>,
    indent: f64,
    font_italic: bool,
    font_small_caps: bool,
    spans: &[TextSpan],
    inline_boxes: &[InlineBoxSpan],
    child_shifts: &HashMap<NodeId, f64>,
    origin_x: f64,
    origin_y: f64,
    options: &ParagraphLayoutOptions,
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

        let prep = prepare_rich_text_for_shaping_with_options(
            &mut font_cx,
            text,
            size,
            weight,
            font_family,
            font_italic,
            font_small_caps,
            letter_spacing,
            line_height,
            max_width,
            align,
            indent,
            spans,
            inline_boxes,
            options,
        );
        let has_explicit_breaks = prep.as_ref().is_some_and(|p| p.has_explicit_breaks);
        let (eff_text, eff_spans, eff_boxes, smcp_ranges, synth_ranges): (
            &str,
            &[TextSpan],
            &[InlineBoxSpan],
            &[(Range<usize>, bool)],
            &[(Range<usize>, f32)],
        ) = match &prep {
            Some(p) => (
                p.text.as_str(),
                p.spans.as_slice(),
                p.inline_boxes.as_slice(),
                p.smcp_ranges.as_slice(),
                p.synthetic_size_ranges.as_slice(),
            ),
            None => (text, spans, inline_boxes, &[], &[]),
        };

        let mut builder = layout_cx.ranged_builder(&mut font_cx, eff_text, 1.0, true);
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
        if font_italic {
            builder.push_default(StyleProperty::FontStyle(FontStyle::Italic));
        }
        if letter_spacing.abs() > 1e-4 {
            builder.push_default(StyleProperty::LetterSpacing(letter_spacing as f32));
        }
        if options.word_spacing.abs() > 1e-4 {
            builder.push_default(StyleProperty::WordSpacing(options.word_spacing as f32));
        }
        if let Some(lh) = line_height {
            if lh > 0.0 {
                builder.push_default(StyleProperty::LineHeight(parley::style::LineHeight::Absolute(lh as f32)));
            }
        }
        for (idx, span) in eff_spans.iter().enumerate() {
            if !span.range.is_empty() && span.range.end <= eff_text.len() {
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
        push_small_caps_styles(&mut builder, smcp_ranges, synth_ranges, eff_text.len());
        push_first_line_indent_box(
            &mut builder,
            indent,
            !eff_text.is_empty() || !eff_boxes.is_empty(),
        );
        for ib in eff_boxes {
            builder.push_inline_box(parley::InlineBox {
                id: ib.node_id.0 as u64,
                kind: parley::InlineBoxKind::InFlow,
                index: ib.index.min(eff_text.len()),
                width: ib.width.max(0.0) as f32,
                height: ib.baseline_offset.max(0.0) as f32,
            });
        }

        let mut layout = builder.build(eff_text);
        if max_width.is_some_and(|w| w > 0.0) {
            break_lines_for_layout(&mut layout, max_width, has_explicit_breaks, align);
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

        let all_lines: Vec<_> = layout.lines().collect();
        let total_lines = all_lines.len();
        let target_w = max_width.unwrap_or(0.0);

        for (line_idx, line) in all_lines.into_iter().enumerate() {
            let line_metrics = line.metrics();
            let line_top_base = origin_y + line_metrics.block_min_coord as f64;
            let line_height_val = (line_metrics.block_max_coord - line_metrics.block_min_coord) as f64;
            let line_baseline_base = origin_y + line_metrics.baseline as f64;

            let (line_space_extra, line_space_limit) = compute_manual_justify_line_spaces(
                &line,
                line_idx,
                total_lines,
                eff_text,
                align,
                target_w,
            );
            let mut spaces_applied = 0usize;
            let mut line_extra_x = 0.0f32;
            let mut run_cursor: Option<(usize, usize)> = None;
            let mut adjusted_glyphs: Vec<(u8, f32, f32)> = Vec::new();
            for item in line.items() {
                if let parley::layout::PositionedLayoutItem::GlyphRun(glyph_run) = item {
                    let run = glyph_run.run();
                    let glyph_count = glyph_run.glyphs().count();
                    let start_idx = match run_cursor {
                        Some((ri, idx)) if ri == run.index() => idx,
                        _ => 0,
                    };
                    let cluster_ranges: Vec<Range<usize>> = if line_space_limit > 0 {
                        let crs: Vec<Range<usize>> = run
                            .visual_clusters()
                            .flat_map(|c| {
                                let r = c.text_range();
                                c.glyphs().map(move |_| r.clone())
                            })
                            .skip(start_idx)
                            .take(glyph_count)
                            .collect();
                        run_cursor = Some((run.index(), start_idx + crs.len()));
                        crs
                    } else {
                        Vec::new()
                    };
                    let b0 = glyph_run.style().brush[0];
                    for (idx_g, g) in glyph_run.positioned_glyphs().enumerate() {
                        let gx = g.x + line_extra_x;
                        let is_sp = spaces_applied < line_space_limit
                            && cluster_ranges
                                .get(idx_g)
                                .and_then(|cr| eff_text.get(cr.clone()))
                                .is_some_and(|s| {
                                    !s.is_empty()
                                        && s.chars().all(|ch| ch == ' ' || ch == '\u{00A0}')
                                });
                        let adv = if is_sp {
                            spaces_applied += 1;
                            line_extra_x += line_space_extra;
                            g.advance + line_space_extra
                        } else {
                            g.advance
                        };
                        adjusted_glyphs.push((b0, gx, gx + adv));
                    }
                }
            }

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
                        for &(b0, gx, gx_end) in &adjusted_glyphs {
                            if b0 == expected_brush_id {
                                if gx < min_gx {
                                    min_gx = gx;
                                }
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
                let (mut px, py) = measure_rich_text_bounds_with_boxes_and_spacing(
                    prefix_text,
                    font_size,
                    font_weight,
                    font_family,
                    0.0,
                    max_width,
                    None,
                    None,
                    line_height,
                    indent,
                    spans,
                    inline_boxes,
                    font_small_caps,
                );
                if prefix_text.is_empty() && inline_boxes.is_empty() && !text.is_empty() && indent > 0.0 {
                    px = indent;
                }
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
        0.0,
        false,
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
#[allow(clippy::too_many_arguments)]
pub fn compute_span_range_metrics_with_boxes(
    text: &str,
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
    max_width: Option<f64>,
    start_at: Option<&str>,
    line_height: Option<f64>,
    align: Option<&str>,
    indent: f64,
    font_small_caps: bool,
    spans: &[TextSpan],
    inline_boxes: &[InlineBoxSpan],
    text_x: f64,
    text_y: f64,
    span_range: Range<usize>,
    own_shift: f64,
    target_inline_box: Option<NodeId>,
) -> SpanLayoutMetrics {
    compute_span_range_metrics_with_options(
        text,
        font_size,
        font_weight,
        font_family,
        false,
        0.0,
        max_width,
        start_at,
        line_height,
        align,
        indent,
        font_small_caps,
        spans,
        inline_boxes,
        text_x,
        text_y,
        span_range,
        own_shift,
        target_inline_box,
        &ParagraphLayoutOptions::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn compute_span_range_metrics_with_options(
    text: &str,
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
    font_italic: bool,
    letter_spacing: f64,
    max_width: Option<f64>,
    start_at: Option<&str>,
    line_height: Option<f64>,
    align: Option<&str>,
    indent: f64,
    font_small_caps: bool,
    spans: &[TextSpan],
    inline_boxes: &[InlineBoxSpan],
    text_x: f64,
    text_y: f64,
    span_range: Range<usize>,
    own_shift: f64,
    target_inline_box: Option<NodeId>,
    options: &ParagraphLayoutOptions,
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

        let prep = prepare_rich_text_for_shaping_with_options(
            &mut font_cx,
            text,
            size,
            weight,
            font_family,
            font_italic,
            font_small_caps,
            letter_spacing,
            line_height,
            max_width,
            align,
            indent,
            spans,
            inline_boxes,
            options,
        );
        let has_explicit_breaks = prep.as_ref().is_some_and(|p| p.has_explicit_breaks);
        let (eff_text, eff_spans, eff_boxes, smcp_ranges, synth_ranges): (
            &str,
            &[TextSpan],
            &[InlineBoxSpan],
            &[(Range<usize>, bool)],
            &[(Range<usize>, f32)],
        ) = match &prep {
            Some(p) => (
                p.text.as_str(),
                p.spans.as_slice(),
                p.inline_boxes.as_slice(),
                p.smcp_ranges.as_slice(),
                p.synthetic_size_ranges.as_slice(),
            ),
            None => (text, spans, inline_boxes, &[], &[]),
        };

        let mut builder = layout_cx.ranged_builder(&mut font_cx, eff_text, 1.0, true);
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
        if font_italic {
            builder.push_default(StyleProperty::FontStyle(FontStyle::Italic));
        }
        if letter_spacing.abs() > 1e-4 {
            builder.push_default(StyleProperty::LetterSpacing(letter_spacing as f32));
        }
        if options.word_spacing.abs() > 1e-4 {
            builder.push_default(StyleProperty::WordSpacing(options.word_spacing as f32));
        }
        if let Some(lh) = line_height {
            if lh > 0.0 {
                builder.push_default(StyleProperty::LineHeight(parley::style::LineHeight::Absolute(lh as f32)));
            }
        }
        for (idx, span) in eff_spans.iter().enumerate() {
            if !span.range.is_empty() && span.range.end <= eff_text.len() {
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
        push_small_caps_styles(&mut builder, smcp_ranges, synth_ranges, eff_text.len());
        push_first_line_indent_box(
            &mut builder,
            indent,
            !eff_text.is_empty() || !eff_boxes.is_empty(),
        );
        for ib in eff_boxes {
            builder.push_inline_box(parley::InlineBox {
                id: ib.node_id.0 as u64,
                kind: parley::InlineBoxKind::InFlow,
                index: ib.index.min(eff_text.len()),
                width: ib.width.max(0.0) as f32,
                height: ib.baseline_offset.max(0.0) as f32,
            });
        }

        let mut layout = builder.build(eff_text);
        if max_width.is_some_and(|w| w > 0.0) {
            break_lines_for_layout(&mut layout, max_width, has_explicit_breaks, align);
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

        let all_lines: Vec<_> = layout.lines().collect();
        let total_lines = all_lines.len();
        let target_w = max_width.unwrap_or(0.0);

        for (line_idx, line) in all_lines.into_iter().enumerate() {
            let line_metrics = line.metrics();
            let line_top_base = origin_y + line_metrics.block_min_coord as f64;
            let line_height_val = (line_metrics.block_max_coord - line_metrics.block_min_coord) as f64;
            let line_baseline_base = origin_y + line_metrics.baseline as f64;

            let (line_space_extra, line_space_limit) = compute_manual_justify_line_spaces(
                &line,
                line_idx,
                total_lines,
                eff_text,
                align,
                target_w,
            );
            let mut spaces_applied = 0usize;
            let mut line_extra_x = 0.0f32;
            let mut run_cursor: Option<(usize, usize)> = None;

            if let Some(box_node_id) = target_inline_box {
                for item in line.items() {
                    match item {
                        parley::layout::PositionedLayoutItem::GlyphRun(glyph_run) => {
                            if line_space_limit > 0 {
                                let run = glyph_run.run();
                                let glyph_count = glyph_run.glyphs().count();
                                let start_idx = match run_cursor {
                                    Some((ri, idx)) if ri == run.index() => idx,
                                    _ => 0,
                                };
                                let crs: Vec<Range<usize>> = run
                                    .visual_clusters()
                                    .flat_map(|c| {
                                        let r = c.text_range();
                                        c.glyphs().map(move |_| r.clone())
                                    })
                                    .skip(start_idx)
                                    .take(glyph_count)
                                    .collect();
                                run_cursor = Some((run.index(), start_idx + crs.len()));
                                for cr in crs {
                                    if spaces_applied < line_space_limit
                                        && eff_text.get(cr).is_some_and(|s| {
                                            !s.is_empty()
                                                && s.chars().all(|ch| ch == ' ' || ch == '\u{00A0}')
                                        })
                                    {
                                        spaces_applied += 1;
                                        line_extra_x += line_space_extra;
                                    }
                                }
                            }
                        }
                        parley::layout::PositionedLayoutItem::InlineBox(pib) => {
                            if pib.id == box_node_id.0 as u64 {
                                let ib_info = eff_boxes.iter().find(|b| b.node_id == box_node_id);
                                let b_w = ib_info.map(|b| b.width).unwrap_or(pib.width as f64);
                                let b_h = ib_info.map(|b| b.height).unwrap_or(pib.height as f64);
                                let b_off = ib_info.map(|b| b.baseline_offset).unwrap_or(b_h);
                                let box_x = origin_x + (pib.x + line_extra_x) as f64;
                                let box_y = line_baseline_base - b_off;
                                fragments.push(Rect::new(box_x, box_y, b_w, b_h));
                                if first_line_baseline.is_none() {
                                    first_line_baseline = Some(line_baseline_base);
                                }
                            }
                        }
                    }
                }
                continue;
            }

            let mut adjusted_glyphs: Vec<(u8, f32, f32)> = Vec::new();
            for item in line.items() {
                if let parley::layout::PositionedLayoutItem::GlyphRun(glyph_run) = item {
                    let run = glyph_run.run();
                    let glyph_count = glyph_run.glyphs().count();
                    let start_idx = match run_cursor {
                        Some((ri, idx)) if ri == run.index() => idx,
                        _ => 0,
                    };
                    let cluster_ranges: Vec<Range<usize>> = if line_space_limit > 0 {
                        let crs: Vec<Range<usize>> = run
                            .visual_clusters()
                            .flat_map(|c| {
                                let r = c.text_range();
                                c.glyphs().map(move |_| r.clone())
                            })
                            .skip(start_idx)
                            .take(glyph_count)
                            .collect();
                        run_cursor = Some((run.index(), start_idx + crs.len()));
                        crs
                    } else {
                        Vec::new()
                    };
                    let b0 = glyph_run.style().brush[0];
                    for (idx_g, g) in glyph_run.positioned_glyphs().enumerate() {
                        let gx = g.x + line_extra_x;
                        let is_sp = spaces_applied < line_space_limit
                            && cluster_ranges
                                .get(idx_g)
                                .and_then(|cr| eff_text.get(cr.clone()))
                                .is_some_and(|s| {
                                    !s.is_empty()
                                        && s.chars().all(|ch| ch == ' ' || ch == '\u{00A0}')
                                });
                        let adv = if is_sp {
                            spaces_applied += 1;
                            line_extra_x += line_space_extra;
                            g.advance + line_space_extra
                        } else {
                            g.advance
                        };
                        adjusted_glyphs.push((b0, gx, gx + adv));
                    }
                }
            }

            let eff_span_range = match &prep {
                Some(p) => p.map_orig_range(span_range.clone()),
                None => span_range.clone(),
            };
            let mut min_gx = f32::MAX;
            let mut max_gx = f32::MIN;

            for (idx, span) in eff_spans.iter().enumerate() {
                if !span.range.is_empty()
                    && span.range.start >= eff_span_range.start
                    && span.range.end <= eff_span_range.end
                {
                    let expected_brush_id = ((idx + 1) % 250 + 1) as u8;
                    for &(b0, gx, gx_end) in &adjusted_glyphs {
                        if b0 == expected_brush_id {
                            if gx < min_gx {
                                min_gx = gx;
                            }
                            if gx_end > max_gx {
                                max_gx = gx_end;
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
        let (mut px, py) = measure_rich_text_bounds_with_boxes_and_spacing(
            prefix_text,
            font_size,
            font_weight,
            font_family,
            0.0,
            max_width,
            None,
            None,
            line_height,
            indent,
            spans,
            inline_boxes,
            font_small_caps,
        );
        if prefix_text.is_empty() && inline_boxes.is_empty() && !text.is_empty() && indent > 0.0 {
            px = indent;
        }
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
    pub small_caps: Option<bool>,
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
pub fn resolve_active_rich_text(
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

    #[test]
    fn test_manual_soft_hyphen_and_nonbreaking_hyphen() {
        // 1. Parser escapes `\-` -> U+00AD and `\=` -> "\u{2060}-\u{2060}" in both content slots and strings
        let doc = crate::parse(
            r#"
            \Text(text: "cu\-ri\=ouser") {
                Curiouser and cu\-ri\-ouser with bread\=and\=butter!
            }
            "#,
        )
        .expect("should parse hyphen escapes");
        let crate::ast::Item::Node(el) = &doc.items[0] else {
            panic!("expected element node");
        };
        let crate::ast::Expr::Literal(crate::ast::Literal::String(port_str, _)) = &el.ports[0].expr else {
            panic!("expected string expr");
        };
        assert_eq!(port_str, "cu\u{00AD}ri\u{2060}-\u{2060}ouser");
        let crate::ast::ContentItem::Text(chunk) = &el.content.as_ref().unwrap().items[0] else {
            panic!("expected text chunk");
        };
        assert_eq!(
            chunk.text,
            "Curiouser and cu\u{00AD}ri\u{00AD}ouser with bread\u{2060}-\u{2060}and\u{2060}-\u{2060}butter!"
        );

        // 2. Soft hyphen materialization:
        //    - Taken break becomes '-' and fits within max_width
        //    - Untaken soft hyphens are stripped completely
        //    - Boundary overflow (where syllable fits without '-' but overflows with '-') falls back to earlier syllable
        let prefix_w = measure_text_width("alpha cu", 16.0, 400.0, Some("Arial"));
        let prefix_ri_no_hyph = measure_text_width("alpha curi", 16.0, 400.0, Some("Arial"));
        let prefix_ri_with_hyph = measure_text_width("alpha curi-", 16.0, 400.0, Some("Arial"));
        assert!(prefix_ri_with_hyph > prefix_ri_no_hyph + 1.0);

        // Width tight enough that "alpha curi" fits WITHOUT '-', but "alpha curi-" overflows -> must fall back to "alpha cu-"!
        let tight_w = (prefix_ri_no_hyph + prefix_ri_with_hyph) * 0.5;
        assert!(tight_w > prefix_w + 4.0);

        let raw = "alpha cu\u{00AD}ri\u{00AD}ouser b\u{00AD}c";
        // Span covering "b\u{00AD}c" at the end (fits on line 1 alongside "riouser ", so untaken!)
        let bc_start = raw.find("b\u{00AD}c").unwrap();
        let spans = vec![TextSpan::new(bc_start..raw.len()).with_style(SpanStyle {
            italic: true,
            ..Default::default()
        })];

        FONT_CONTEXT.with(|fc| {
            let mut font_cx = fc.borrow_mut();
            let mat = materialize_soft_hyphens_with_font_cx(
                &mut font_cx,
                raw,
                16.0,
                400.0,
                Some("Arial"),
                false,
                None,
                Some(tight_w),
                0.0,
                &spans,
                &[],
            )
            .expect("should materialize soft hyphens");

            assert!(
                !mat.text.contains('\u{00AD}'),
                "No raw U+00AD should remain in materialized text: {:?}",
                mat.text
            );
            assert_eq!(
                mat.text, "alpha cu-riouser bc",
                "Expected fallback to 'cu-' when 'curi-' overflows tight_w, and untaken soft hyphens stripped"
            );
            assert_eq!(
                &mat.text[mat.spans[0].range.clone()],
                "bc",
                "Subsequent span byte range must adjust accurately after taken and stripped soft hyphens"
            );
        });

        // 3. Non-breaking hyphen (`\=`) prevents breaking where regular `-` breaks
        let breakable = "eating bread-and-butter";
        let nonbreaking = "eating bread\u{2060}-\u{2060}and\u{2060}-\u{2060}butter";
        let eating_bread_w = measure_text_width("eating bread-", 16.0, 400.0, Some("Arial"));
        let full_w = measure_text_width(breakable, 16.0, 400.0, Some("Arial"));
        let mid_w = (eating_bread_w + full_w) * 0.5;

        let shaped_break = shape_paragraph_lines(
            breakable,
            16.0,
            400.0,
            Some("Arial"),
            Some(mid_w),
            None,
            None,
            None,
            None,
            0.0,
            false,
            false,
            &[],
            &[],
        );
        let shaped_nb = shape_paragraph_lines(
            nonbreaking,
            16.0,
            400.0,
            Some("Arial"),
            Some(mid_w),
            None,
            None,
            None,
            None,
            0.0,
            false,
            false,
            &[],
            &[],
        );
        // `breakable` keeps "eating bread-" on line 0 (`> "eating".len()`), whereas `nonbreaking` pushes the whole compound to line 1
        assert!(
            shaped_break.lines[0].byte_range.end > "eating ".len(),
            "Regular hyphen should break after 'bread-' or 'and-' on line 0"
        );
        assert!(
            shaped_nb.lines[0].byte_range.end <= "eating ".len(),
            "Non-breaking hyphen must keep 'bread-and-butter' intact on line 1"
        );
    }

    #[test]
    fn test_font_supports_smcp_detection() {
        register_local_fonts_for_base_dir(std::path::Path::new("examples/alice"));
        assert!(
            !font_supports_smcp(Some("Old Standard TT"), 400.0, false),
            "Old Standard TT lacks OpenType smcp and should use synthetic fallback"
        );
        assert!(
            !font_supports_smcp(Some("Arial"), 400.0, false),
            "Arial lacks OpenType smcp and should use synthetic fallback"
        );
        for native_family in ["STIX Two Text", "Avenir Next"] {
            if font_family_exists(native_family) {
                assert!(
                    font_supports_smcp(Some(native_family), 400.0, false),
                    "{} should report native OpenType smcp support",
                    native_family
                );
            }
        }
    }

    #[test]
    fn test_small_caps_synthetic_and_native_shaping() {
        sync_thread_local_font_context();
        FONT_CONTEXT.with(|fc| {
            let mut font_cx = fc.borrow_mut();

            // 1. Synthetic fallback preserves uppercase/punctuation sizes and keeps lowercase pdf_text
            let prep_for = prepare_rich_text_for_shaping(
                &mut font_cx,
                "For.",
                16.0,
                400.0,
                Some("Arial"),
                false,
                true,
                None,
                None,
                0.0,
                &[],
                &[],
            )
            .expect("synthetic small caps should prepare text");
            assert_eq!(prep_for.text, "FOR.");
            assert_eq!(prep_for.pdf_text, "For.");
            assert!(prep_for.smcp_ranges.is_empty());
            assert_eq!(
                prep_for.synthetic_size_ranges,
                vec![(1..3, (16.0 * SYNTHETIC_SMALL_CAPS_SCALE) as f32)]
            );

            // 2. Multi-byte uppercase expansion ("ß" -> "SS") maps clusters back to valid UTF-8 boundaries in pdf_text
            let prep_strasse = prepare_rich_text_for_shaping(
                &mut font_cx,
                "straße",
                16.0,
                400.0,
                Some("Arial"),
                false,
                true,
                None,
                None,
                0.0,
                &[],
                &[],
            )
            .expect("synthetic small caps should prepare strasse");
            assert_eq!(prep_strasse.text, "STRASSE");
            assert_eq!(prep_strasse.pdf_text, "straße");
            assert_eq!(prep_strasse.map_shaped_to_pdf_range(4..5), 4..6);
            assert_eq!(prep_strasse.map_shaped_to_pdf_range(5..6), 4..6);
            assert_eq!(prep_strasse.map_shaped_to_pdf_range(6..7), 6..7);
            assert!(prep_strasse.pdf_text.is_char_boundary(4));
            assert!(prep_strasse.pdf_text.is_char_boundary(6));
            assert_eq!(&prep_strasse.pdf_text[4..6], "ß");
        });

        let (sc_w, _) = measure_rich_text_bounds_with_boxes_and_spacing(
            "Hello",
            16.0,
            400.0,
            Some("Arial"),
            0.0,
            None,
            None,
            None,
            None,
            0.0,
            &[],
            &[],
            true,
        );
        let (all_caps_w, _) = measure_rich_text_bounds_with_boxes_and_spacing(
            "HELLO",
            16.0,
            400.0,
            Some("Arial"),
            0.0,
            None,
            None,
            None,
            None,
            0.0,
            &[],
            &[],
            false,
        );
        let (normal_w, _) = measure_rich_text_bounds_with_boxes_and_spacing(
            "Hello",
            16.0,
            400.0,
            Some("Arial"),
            0.0,
            None,
            None,
            None,
            None,
            0.0,
            &[],
            &[],
            false,
        );
        assert!(sc_w < all_caps_w, "sc_w ({}) < all_caps_w ({})", sc_w, all_caps_w);
        assert!(
            (sc_w - normal_w).abs() > 0.1,
            "sc_w ({}) should differ from normal_w ({})",
            sc_w,
            normal_w
        );

        // 3. Native OpenType smcp path keeps original text without synthetic size scaling
        if font_family_exists("STIX Two Text") && font_supports_smcp(Some("STIX Two Text"), 400.0, false) {
            FONT_CONTEXT.with(|fc| {
                LAYOUT_CONTEXT.with(|lc| {
                    let mut font_cx = fc.borrow_mut();
                    let prep_native = prepare_rich_text_for_shaping(
                        &mut font_cx,
                        "Hello",
                        16.0,
                        400.0,
                        Some("STIX Two Text"),
                        false,
                        true,
                        None,
                        None,
                        0.0,
                        &[],
                        &[],
                    )
                    .expect("native smcp should prepare text");
                    assert_eq!(prep_native.text, "Hello");
                    assert_eq!(prep_native.pdf_text, "Hello");
                    assert!(prep_native.synthetic_size_ranges.is_empty());
                    assert_eq!(prep_native.smcp_ranges, vec![(0..5, true)]);

                    let mut layout_cx = lc.borrow_mut();
                    let smcp_layout = build_probe_layout(
                        &mut font_cx,
                        &mut layout_cx,
                        &prep_native.text,
                        16.0,
                        400.0,
                        Some("STIX Two Text"),
                        false,
                        0.0,
                        0.0,
                        None,
                        Some(f32::MAX),
                        0.0,
                        &[],
                        &prep_native.smcp_ranges,
                        &prep_native.synthetic_size_ranges,
                        &[],
                    );
                    let plain_layout = build_probe_layout(
                        &mut font_cx,
                        &mut layout_cx,
                        "Hello",
                        16.0,
                        400.0,
                        Some("STIX Two Text"),
                        false,
                        0.0,
                        0.0,
                        None,
                        Some(f32::MAX),
                        0.0,
                        &[],
                        &[],
                        &[],
                        &[],
                    );
                    let collect_glyph_ids = |layout: &parley::Layout<()>| -> Vec<u16> {
                        layout
                            .lines()
                            .flat_map(|line| {
                                line.items().filter_map(|item| match item {
                                    parley::layout::PositionedLayoutItem::GlyphRun(gr) => {
                                        Some(gr.positioned_glyphs().map(|g| g.id as u16).collect::<Vec<_>>())
                                    }
                                    _ => None,
                                })
                            })
                            .flatten()
                            .collect()
                    };
                    let smcp_ids = collect_glyph_ids(&smcp_layout);
                    let plain_ids = collect_glyph_ids(&plain_layout);
                    assert_eq!(smcp_ids.len(), 5);
                    assert_eq!(plain_ids.len(), 5);
                    assert_eq!(smcp_ids[0], plain_ids[0], "Uppercase 'H' should use the same glyph");
                    assert_ne!(
                        &smcp_ids[1..],
                        &plain_ids[1..],
                        "Lowercase 'ello' should substitute native smcp glyph IDs"
                    );
                });
            });
        }
    }
}

