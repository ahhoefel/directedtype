//! Knuth–Plass optimal paragraph line-breaking algorithm (1981).
//!
//! Implements the box–glue–penalty dynamic programming algorithm described in
//! D. E. Knuth and M. F. Plass, "Breaking Paragraphs into Lines",
//! *Software—Practice and Experience* 11 (1981), 1119–1184.

use crate::compiler::value::Value;
use std::collections::HashMap;

/// Penalty value representing `+∞` (prohibited break) and `-∞` (forced break).
pub const INF_PENALTY: f64 = 10_000.0;

/// Infinite stretchability used for finishing glue and ragged hard breaks.
pub const INF_STRETCH: f64 = 1_000_000.0;

const INF_RATIO: f64 = 1_000_000.0;

/// Paragraph line-breaking strategy selectable via `\ParagraphStyle(line_break: ...)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LineBreakMode {
    /// Standard greedy first-fit line breaking (Parley default).
    #[default]
    Greedy,
    /// Global optimal paragraph line breaking via the Knuth–Plass algorithm.
    Optimal,
}

impl LineBreakMode {
    pub fn from_str_opt(s: Option<&str>) -> Self {
        match s {
            Some(v) if v.eq_ignore_ascii_case("optimal") || v.eq_ignore_ascii_case("knuth_plass") => {
                Self::Optimal
            }
            _ => Self::Greedy,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Greedy => "Greedy",
            Self::Optimal => "Optimal",
        }
    }
}

/// Paragraph-level layout and Knuth–Plass tuning options.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParagraphLayoutOptions {
    pub line_break: LineBreakMode,
    /// Additive delta applied to nominal inter-word space width (in px/pt, like CSS `word-spacing`).
    pub word_spacing: f64,
    /// Inter-word space stretchability as a fraction of nominal space width (classic Knuth–Plass: `0.5`).
    pub space_stretch: f64,
    /// Inter-word space shrinkability as a fraction of nominal space width (classic Knuth–Plass: `1/3`).
    pub space_shrink: f64,
    /// Maximum acceptable adjustment ratio `rho` in the primary pass (classic: `2.0`).
    pub tolerance: f64,
    /// Penalty for breaking at a discretionary (`\-`) or explicit (`-`) hyphen (classic: `50.0`).
    pub hyphen_penalty: f64,
    /// Per-line penalty `l` in `(l + badness + p)^2` (classic TeX: `10.0`).
    pub line_penalty: f64,
    /// Extra demerits for two consecutive flagged (hyphenated) lines (1981 paper: `3000.0`).
    pub flagged_demerits: f64,
    /// Extra demerits for adjacent lines whose fitness classes differ by more than 1 (1981 paper: `3000.0`).
    pub fitness_demerits: f64,
    /// Extra width added to spaces after sentence-ending punctuation (`.`, `?`, `!`), as a fraction
    /// of nominal space width (`0.0` = uniform French spacing; `~0.33` = traditional English/TeX spacing).
    pub sentence_space: f64,
    /// Target line-count delta (`\looseness` in Knuth–Plass 1981; `0` = globally optimal line count).
    pub looseness: i32,
}

impl Default for ParagraphLayoutOptions {
    fn default() -> Self {
        Self {
            line_break: LineBreakMode::Greedy,
            word_spacing: 0.0,
            space_stretch: 0.5,
            space_shrink: 1.0 / 3.0,
            tolerance: 2.0,
            hyphen_penalty: 50.0,
            line_penalty: 10.0,
            flagged_demerits: 3000.0,
            fitness_demerits: 3000.0,
            sentence_space: 0.0,
            looseness: 0,
        }
    }
}

impl ParagraphLayoutOptions {
    /// Extracts paragraph layout options from a resolved `\Text` node's properties.
    pub fn from_node_properties(props: &HashMap<String, Value>) -> Self {
        let mut opts = Self::default();
        if let Some(lb) = props.get("line_break").and_then(|v| v.as_str()) {
            opts.line_break = LineBreakMode::from_str_opt(Some(lb));
        }
        if let Some(ws) = props.get("word_spacing").and_then(|v| v.as_f64()) {
            opts.word_spacing = ws;
        }
        if let Some(ss) = props.get("space_stretch").and_then(|v| v.as_f64()) {
            opts.space_stretch = ss.max(0.0);
        }
        if let Some(sk) = props.get("space_shrink").and_then(|v| v.as_f64()) {
            opts.space_shrink = sk.clamp(0.0, 0.99);
        }
        if let Some(tol) = props.get("tolerance").and_then(|v| v.as_f64()) {
            opts.tolerance = tol.max(0.1);
        }
        if let Some(hp) = props.get("hyphen_penalty").and_then(|v| v.as_f64()) {
            opts.hyphen_penalty = hp;
        }
        if let Some(lp) = props.get("line_penalty").and_then(|v| v.as_f64()) {
            opts.line_penalty = lp.max(0.0);
        }
        if let Some(fd) = props.get("flagged_demerits").and_then(|v| v.as_f64()) {
            opts.flagged_demerits = fd.max(0.0);
        }
        if let Some(ad) = props.get("fitness_demerits").and_then(|v| v.as_f64()) {
            opts.fitness_demerits = ad.max(0.0);
        }
        if let Some(sent) = props.get("sentence_space").and_then(|v| v.as_f64()) {
            opts.sentence_space = sent;
        }
        if let Some(l) = props.get("looseness").and_then(|v| v.as_f64()) {
            opts.looseness = l.round() as i32;
        }
        opts
    }

    /// Returns a compact cache key covering all fields that affect shaping/rendering.
    pub fn cache_key(&self) -> (u8, u64, u64, u64, u64, u64, u64, u64, u64, u64, i32) {
        (
            match self.line_break {
                LineBreakMode::Greedy => 0,
                LineBreakMode::Optimal => 1,
            },
            self.word_spacing.to_bits(),
            self.space_stretch.to_bits(),
            self.space_shrink.to_bits(),
            self.tolerance.to_bits(),
            self.hyphen_penalty.to_bits(),
            self.line_penalty.to_bits(),
            self.flagged_demerits.to_bits(),
            self.fitness_demerits.to_bits(),
            self.sentence_space.to_bits(),
            self.looseness,
        )
    }

    /// True if any non-default option affects shaping or breaking (used for fast-path checks).
    pub fn is_optimal(&self) -> bool {
        self.line_break == LineBreakMode::Optimal
    }
}

/// An atomic element in a Knuth–Plass paragraph stream.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Item {
    /// Fixed-width unbreakable content (word, glyph run, inline box, or indent).
    Box { width: f64 },
    /// Stretchable and shrinkable blank space. Legal breakpoint iff immediately preceded by a `Box`.
    Glue {
        width: f64,
        stretch: f64,
        shrink: f64,
    },
    /// Explicit break candidate (`penalty < INF_PENALTY`), forced break (`penalty <= -INF_PENALTY`),
    /// or break prohibition (`penalty >= INF_PENALTY`).
    Penalty {
        width: f64,
        penalty: f64,
        flagged: bool,
    },
}

/// Result of running Knuth–Plass line breaking over an item sequence.
#[derive(Debug, Clone, PartialEq)]
pub struct BreakResult {
    /// Indices into `items` where each line ends (including the final forced break, if any).
    pub breaks: Vec<usize>,
    /// Adjustment ratio `r` for each resulting line.
    pub line_ratios: Vec<f64>,
    /// Total cumulative demerits of the chosen break sequence.
    pub total_demerits: f64,
    /// True if the emergency overfull fallback pass was required.
    pub overfull: bool,
}

#[derive(Debug, Clone, Copy)]
struct ActiveNode {
    /// Item index of this breakpoint, or `None` for the start of the paragraph (before item 0).
    position: Option<usize>,
    /// Line index completed at this node (0 for start of paragraph).
    line: usize,
    /// Fitness class of the line ending at this node (`0..=3`).
    fitness: u8,
    /// Adjustment ratio of the line ending at this node.
    ratio: f64,
    /// Cumulative `(width, stretch, shrink)` after skipping discardable glue/penalties past `position`.
    after_w: f64,
    after_y: f64,
    after_z: f64,
    /// Cumulative demerits up to this node.
    total_demerits: f64,
    /// Parent active node index in the arena.
    prev: Option<usize>,
}

#[inline]
fn is_legal_breakpoint(items: &[Item], idx: usize) -> bool {
    match items[idx] {
        Item::Box { .. } => false,
        Item::Glue { .. } => idx > 0 && matches!(items[idx - 1], Item::Box { .. }),
        Item::Penalty { penalty, .. } => penalty < INF_PENALTY,
    }
}

#[inline]
fn is_forced_break(item: Item) -> bool {
    matches!(item, Item::Penalty { penalty, .. } if penalty <= -INF_PENALTY)
}

#[inline]
fn is_flagged_penalty(items: &[Item], pos: Option<usize>) -> bool {
    pos.is_some_and(|idx| matches!(items[idx], Item::Penalty { flagged: true, .. }))
}

/// Computes cumulative `(w, y, z)` immediately after breaking at `b`, skipping `b` itself and
/// any immediately following glue or non-forced penalty items (standard Knuth–Plass discard rule).
fn compute_sum_after_break(
    items: &[Item],
    b: usize,
    sum_w: f64,
    sum_y: f64,
    sum_z: f64,
) -> (f64, f64, f64) {
    let mut w = sum_w;
    let mut y = sum_y;
    let mut z = sum_z;
    for (offset, &item) in items[b..].iter().enumerate() {
        match item {
            Item::Box { .. } => break,
            Item::Glue {
                width,
                stretch,
                shrink,
            } => {
                w += width;
                y += stretch;
                z += shrink;
            }
            Item::Penalty { penalty, .. } => {
                if offset > 0 && penalty <= -INF_PENALTY {
                    break;
                }
            }
        }
    }
    (w, y, z)
}

#[inline]
fn compute_adjustment_ratio(
    items: &[Item],
    b: usize,
    line_width: f64,
    sum_w: f64,
    sum_y: f64,
    sum_z: f64,
    active: &ActiveNode,
    emergency_stretch: f64,
) -> f64 {
    let mut w = sum_w - active.after_w;
    if let Item::Penalty { width, .. } = items[b] {
        w += width;
    }

    if w < line_width {
        let y = (sum_y - active.after_y) + emergency_stretch;
        if y > 1e-9 {
            (line_width - w) / y
        } else {
            INF_RATIO
        }
    } else if w > line_width {
        let z = sum_z - active.after_z;
        if z > 1e-9 {
            (line_width - w) / z
        } else {
            -INF_RATIO
        }
    } else {
        0.0
    }
}

#[inline]
fn fitness_class(r: f64) -> u8 {
    if r < -0.5 {
        0 // Tight
    } else if r <= 0.5 {
        1 // Normal
    } else if r <= 1.0 {
        2 // Loose
    } else {
        3 // Very loose
    }
}

#[inline]
fn compute_demerits(
    items: &[Item],
    a: &ActiveNode,
    b: usize,
    r: f64,
    options: &ParagraphLayoutOptions,
) -> (f64, u8) {
    let abs_r = r.abs();
    let badness = if abs_r >= 100.0 {
        10_000.0
    } else {
        (100.0 * abs_r * abs_r * abs_r).min(10_000.0)
    };

    let p = match items[b] {
        Item::Penalty { penalty, .. } => penalty,
        _ => 0.0,
    };

    let mut d = if p >= 0.0 {
        (options.line_penalty + badness + p).powi(2)
    } else if p > -INF_PENALTY {
        (options.line_penalty + badness).powi(2) - p * p
    } else {
        (options.line_penalty + badness).powi(2)
    };

    // Extra penalty for overfull lines in emergency fallback mode so feasible breaks always win.
    if r < -1.0 {
        let excess = (-1.0 - r).min(1_000.0);
        d += 1e9 + excess * excess * 1e6;
    }

    if is_flagged_penalty(items, a.position) && is_flagged_penalty(items, Some(b)) {
        d += options.flagged_demerits;
    }

    let c = fitness_class(r);
    if (c as i32 - a.fitness as i32).abs() > 1 {
        d += options.fitness_demerits;
    }

    (d, c)
}

fn try_break_pass<F>(
    items: &[Item],
    line_widths: &F,
    options: &ParagraphLayoutOptions,
    rho: f64,
    emergency_stretch: f64,
    allow_fallback: bool,
) -> Option<BreakResult>
where
    F: Fn(usize) -> f64,
{
    if items.is_empty() {
        return Some(BreakResult {
            breaks: Vec::new(),
            line_ratios: Vec::new(),
            total_demerits: 0.0,
            overfull: false,
        });
    }

    let mut arena: Vec<ActiveNode> = Vec::with_capacity(items.len() / 2 + 8);
    arena.push(ActiveNode {
        position: None,
        line: 0,
        fitness: 1,
        ratio: 0.0,
        after_w: 0.0,
        after_y: 0.0,
        after_z: 0.0,
        total_demerits: 0.0,
        prev: None,
    });

    let mut active_indices: Vec<usize> = vec![0];
    let mut sum_w = 0.0;
    let mut sum_y = 0.0;
    let mut sum_z = 0.0;

    // Best candidate per `(new_line, fitness_class)` at the current breakpoint `b`.
    // Maps `(new_line, fitness_class) -> (total_demerits, active_arena_idx, ratio)`.
    let mut candidates: Vec<((usize, u8), f64, usize, f64)> = Vec::with_capacity(16);

    for b in 0..items.len() {
        if is_legal_breakpoint(items, b) {
            let forced = is_forced_break(items[b]);
            candidates.clear();

            // Track best fallback transition in case `allow_fallback` is true and no feasible break exists.
            let mut fallback_best: Option<(f64, usize, f64, u8)> = None;
            let mut next_active: Vec<usize> = Vec::with_capacity(active_indices.len());

            for &a_idx in &active_indices {
                let a = arena[a_idx];
                let line_w = line_widths(a.line);
                let r = compute_adjustment_ratio(
                    items,
                    b,
                    line_w,
                    sum_w,
                    sum_y,
                    sum_z,
                    &a,
                    emergency_stretch,
                );

                // Deactivate active node if line is too tight (`r < -1`) or this is a forced break.
                if r >= -1.0 && !forced {
                    next_active.push(a_idx);
                }

                let feasible = r >= -1.0 && r <= rho;
                if feasible {
                    let (d, c) = compute_demerits(items, &a, b, r, options);
                    let total_d = a.total_demerits + d;
                    let key = (a.line + 1, c);
                    if let Some(slot) = candidates.iter_mut().find(|entry| entry.0 == key) {
                        if total_d < slot.1 {
                            slot.1 = total_d;
                            slot.2 = a_idx;
                            slot.3 = r;
                        }
                    } else {
                        candidates.push((key, total_d, a_idx, r));
                    }
                } else if allow_fallback {
                    let (d, c) = compute_demerits(items, &a, b, r, options);
                    let total_d = a.total_demerits + d;
                    if fallback_best.is_none_or(|best| total_d < best.0) {
                        fallback_best = Some((total_d, a_idx, r, c));
                    }
                }
            }

            if !candidates.is_empty() {
                let (after_w, after_y, after_z) =
                    compute_sum_after_break(items, b, sum_w, sum_y, sum_z);
                for &((new_line, c), total_d, a_idx, r) in &candidates {
                    let new_idx = arena.len();
                    arena.push(ActiveNode {
                        position: Some(b),
                        line: new_line,
                        fitness: c,
                        ratio: r,
                        after_w,
                        after_y,
                        after_z,
                        total_demerits: total_d,
                        prev: Some(a_idx),
                    });
                    next_active.push(new_idx);
                }
            } else if allow_fallback && next_active.is_empty() {
                if let Some((total_d, a_idx, r, c)) = fallback_best {
                    let a = arena[a_idx];
                    let (after_w, after_y, after_z) =
                        compute_sum_after_break(items, b, sum_w, sum_y, sum_z);
                    let new_idx = arena.len();
                    arena.push(ActiveNode {
                        position: Some(b),
                        line: a.line + 1,
                        fitness: c,
                        ratio: r,
                        after_w,
                        after_y,
                        after_z,
                        total_demerits: total_d,
                        prev: Some(a_idx),
                    });
                    next_active.push(new_idx);
                }
            }

            active_indices = next_active;
            if active_indices.is_empty() {
                return None;
            }
        }

        // Advance cumulative sums past item `b`.
        match items[b] {
            Item::Box { width } => sum_w += width,
            Item::Glue {
                width,
                stretch,
                shrink,
            } => {
                sum_w += width;
                sum_y += stretch;
                sum_z += shrink;
            }
            Item::Penalty { .. } => {}
        }
    }

    if active_indices.is_empty() {
        return None;
    }

    // Choose best final active node, honoring `options.looseness` (Knuth–Plass 1981).
    let best_optimal_idx = *active_indices
        .iter()
        .min_by(|&&i, &&j| {
            arena[i]
                .total_demerits
                .total_cmp(&arena[j].total_demerits)
        })?;

    let chosen_idx = if options.looseness == 0 {
        best_optimal_idx
    } else {
        let optimal_lines = arena[best_optimal_idx].line as i32;
        let target_lines = (optimal_lines + options.looseness).max(1);
        *active_indices
            .iter()
            .min_by(|&&i, &&j| {
                let di = (arena[i].line as i32 - target_lines).abs();
                let dj = (arena[j].line as i32 - target_lines).abs();
                di.cmp(&dj).then_with(|| {
                    arena[i]
                        .total_demerits
                        .total_cmp(&arena[j].total_demerits)
                })
            })?
    };

    let total_demerits = arena[chosen_idx].total_demerits;
    let mut breaks = Vec::new();
    let mut line_ratios = Vec::new();
    let mut cur = Some(chosen_idx);
    while let Some(idx) = cur {
        let node = arena[idx];
        if let Some(pos) = node.position {
            breaks.push(pos);
            line_ratios.push(node.ratio);
        }
        cur = node.prev;
    }
    breaks.reverse();
    line_ratios.reverse();

    Some(BreakResult {
        breaks,
        line_ratios,
        total_demerits,
        overfull: allow_fallback,
    })
}

/// Runs the Knuth–Plass algorithm with progressive tolerance relaxation and guaranteed fallback.
///
/// The caller should normally terminate `items` with finishing glue + a forced break
/// (`Item::Glue { width: 0.0, stretch: INF_STRETCH, shrink: 0.0 }`,
///  `Item::Penalty { width: 0.0, penalty: -INF_PENALTY, flagged: true }`),
/// or for `JustifyAll`, just the final forced `Penalty`.
pub fn break_items<F>(
    items: &[Item],
    line_widths: F,
    options: &ParagraphLayoutOptions,
) -> BreakResult
where
    F: Fn(usize) -> f64,
{
    let base_tol = options.tolerance.max(0.1);

    // Pass 1: primary tolerance (e.g. rho = 2.0)
    if let Some(res) = try_break_pass(items, &line_widths, options, base_tol, 0.0, false) {
        return res;
    }

    // Pass 2..4: progressively looser tolerance (still preferring feasible non-overfull breaks)
    for &mult in &[2.5, 6.0, 20.0] {
        let tol = (base_tol * mult).max(5.0);
        if let Some(res) = try_break_pass(items, &line_widths, options, tol, 0.0, false) {
            return res;
        }
    }

    // Pass 5: TeX-style emergency stretch (allows very loose lines rather than overfull)
    let em_stretch = line_widths(0).max(12.0) * 0.5;
    if let Some(res) = try_break_pass(items, &line_widths, options, 100.0, em_stretch, false) {
        return res;
    }

    // Pass 6: guaranteed fallback allowing overfull lines (e.g. a single word wider than line_width)
    try_break_pass(items, &line_widths, options, 100.0, em_stretch, true).unwrap_or(BreakResult {
        breaks: Vec::new(),
        line_ratios: Vec::new(),
        total_demerits: 0.0,
        overfull: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: builds a box–glue–penalty stream from word widths, uniform space `(w, y, z)`,
    /// and a ragged finishing glue + forced final break.
    fn build_simple_paragraph(words: &[(&str, f64)], space_w: f64, opts: &ParagraphLayoutOptions) -> Vec<Item> {
        let mut items = Vec::new();
        for (i, &(_, w)) in words.iter().enumerate() {
            if i > 0 {
                items.push(Item::Glue {
                    width: space_w,
                    stretch: space_w * opts.space_stretch,
                    shrink: space_w * opts.space_shrink,
                });
            }
            items.push(Item::Box { width: w });
        }
        items.push(Item::Glue {
            width: 0.0,
            stretch: INF_STRETCH,
            shrink: 0.0,
        });
        items.push(Item::Penalty {
            width: 0.0,
            penalty: -INF_PENALTY,
            flagged: true,
        });
        items
    }

    /// Reconstructs lines of word labels from `breaks` produced on `build_simple_paragraph`.
    fn extract_lines<'a>(words: &[(&'a str, f64)], breaks: &[usize]) -> Vec<Vec<&'a str>> {
        let mut lines = Vec::new();
        let mut start_word = 0usize;
        for &b in breaks {
            // Item index of word `k` is `2 * k`; glue after word `k` is `2 * k + 1`.
            let end_word = if b >= 2 * words.len() {
                words.len()
            } else {
                (b + 1) / 2
            };
            lines.push(words[start_word..end_word].iter().map(|&(s, _)| s).collect());
            start_word = end_word;
        }
        lines
    }

    #[test]
    fn test_classic_1981_paper_example_beats_greedy() {
        // Classic paragraph from Knuth & Plass (1981), Fig. 1 ("In olden times..."),
        // using fixed-width character counts (1 unit per char, space = 1 + 0.5 - 0.333)
        // at measure 35 to verify global optimization avoids greedy's huge loose gap.
        let text = "In olden times when wishing still helped one, there lived a king whose daughters were all beautiful; and the youngest was so beautiful that the sun itself, which has seen so much, was astonished whenever it shone in her face.";
        let words: Vec<(&str, f64)> = text.split_whitespace().map(|w| (w, w.len() as f64)).collect();
        let opts = ParagraphLayoutOptions {
            line_break: LineBreakMode::Optimal,
            ..Default::default()
        };
        let items = build_simple_paragraph(&words, 1.0, &opts);
        let measure = 35.0;
        let res = break_items(&items, |_| measure, &opts);

        assert!(!res.overfull);
        let lines = extract_lines(&words, &res.breaks);
        assert!(!lines.is_empty());

        // Compare total sum of badness^2 against naive greedy (no shrink):
        let mut greedy_lines: Vec<Vec<&str>> = Vec::new();
        let mut cur: Vec<&str> = Vec::new();
        let mut cur_w = 0.0;
        for &(w, len) in &words {
            let add = if cur.is_empty() { len } else { 1.0 + len };
            if !cur.is_empty() && cur_w + add > measure {
                greedy_lines.push(cur);
                cur = vec![w];
                cur_w = len;
            } else {
                cur.push(w);
                cur_w += add;
            }
        }
        if !cur.is_empty() {
            greedy_lines.push(cur);
        }

        // Compute max adjustment ratio on non-final lines for both:
        let max_ratio = |ls: &[Vec<&str>]| -> f64 {
            ls[..ls.len().saturating_sub(1)]
                .iter()
                .map(|line| {
                    let sum_w: f64 = line.iter().map(|s| s.len() as f64).sum();
                    let spaces = (line.len().saturating_sub(1)) as f64;
                    let nat = sum_w + spaces;
                    if nat < measure {
                        (measure - nat) / (spaces * opts.space_stretch)
                    } else {
                        (measure - nat) / (spaces * opts.space_shrink)
                    }
                    .abs()
                })
                .fold(0.0_f64, f64::max)
        };

        let kp_max_r = max_ratio(&lines);
        let greedy_max_r = max_ratio(&greedy_lines);
        assert!(
            kp_max_r < greedy_max_r,
            "Knuth-Plass max |r| ({kp_max_r:.2}) should be strictly smaller than greedy ({greedy_max_r:.2}); KP lines: {lines:?}, Greedy lines: {greedy_lines:?}"
        );
    }

    #[test]
    fn test_shrinkability_eliminates_runt_line() {
        // 5 words of width 10 + 4 spaces of width 3 => natural width = 62.
        // With measure = 60 and shrink = 1/3 (total shrink = 4.0), r = -2/4 = -0.5 -> fits on 1 line!
        // Greedy (no shrink) would spill the 5th word onto a 2nd line.
        let words = vec![("a", 10.0), ("b", 10.0), ("c", 10.0), ("d", 10.0), ("e", 10.0)];
        let opts = ParagraphLayoutOptions {
            line_break: LineBreakMode::Optimal,
            ..Default::default()
        };
        let items = build_simple_paragraph(&words, 3.0, &opts);
        let res = break_items(&items, |_| 60.0, &opts);
        let lines = extract_lines(&words, &res.breaks);
        assert_eq!(lines.len(), 1, "Shrinkable spaces should keep all 5 words on 1 line");
        assert!((res.line_ratios[0] - (-0.5)).abs() < 1e-6);
    }

    #[test]
    fn test_consecutive_flagged_hyphens_avoided() {
        // Construct a paragraph where breaking at two consecutive hyphens is possible,
        // versus a slightly looser non-hyphenated break on line 1 that avoids consecutive hyphens.
        let opts = ParagraphLayoutOptions {
            line_break: LineBreakMode::Optimal,
            hyphen_penalty: 50.0,
            flagged_demerits: 10_000.0,
            ..Default::default()
        };
        // Word 1: 40, space: 4 (+2/-1.33), Word 2: "con-" (12) + shy(3) + "tinue" (25),
        // space: 4, Word 3: "walk-" (28) + shy(3) + "ing" (15), space: 4, Word 4: 20.
        // At measure 58:
        // Line 1: 40 + 4 + 12 + 3 = 59 (r = -0.75, hyphenated) OR 40 (too loose)
        // Make Word 1 + Word 1b = 20 + 4 + 26 = 50 (stretch=4 => r=2.0 -> feasible, unhyphenated!)
        let items = vec![
            Item::Box { width: 20.0 },
            Item::Glue { width: 4.0, stretch: 4.0, shrink: 1.5 }, // idx 1
            Item::Box { width: 26.0 },
            Item::Glue { width: 4.0, stretch: 4.0, shrink: 1.5 }, // idx 3: w=50, y=4 -> r=(58-50)/4=2.0
            Item::Box { width: 6.0 },
            Item::Penalty { width: 2.0, penalty: 50.0, flagged: true }, // idx 5: w=62 (Wait: 50+4+6+2=62)
            Item::Box { width: 32.0 },
            Item::Glue { width: 4.0, stretch: 4.0, shrink: 1.5 }, // idx 7
            Item::Box { width: 18.0 },
            Item::Penalty { width: 2.0, penalty: 50.0, flagged: true }, // idx 9
            Item::Box { width: 20.0 },
            Item::Glue { width: 0.0, stretch: INF_STRETCH, shrink: 0.0 },
            Item::Penalty { width: 0.0, penalty: -INF_PENALTY, flagged: true },
        ];
        // Note: idx 3 has w=50, r=(58-50)/4=2.0 (feasible!).
        // idx 5 has w=62, shrink=3 -> r=-1.33 (infeasible at 58). Adjust measure to 60:
        // At measure 60: idx 3 has w=50 (r=2.5), idx 5 has w=62 (r=-2/3=-0.67, flagged).
        // Let's test with tolerance 3.0 where both idx 3 (unflagged) and idx 5 (flagged) are feasible!
        let mut opts_high_flag = opts;
        opts_high_flag.tolerance = 3.0;
        opts_high_flag.flagged_demerits = 500_000.0;
        let res_avoid = break_items(&items, |_| 60.0, &opts_high_flag);

        let mut opts_no_flag = opts_high_flag;
        opts_no_flag.flagged_demerits = 0.0;
        let res_allow = break_items(&items, |_| 60.0, &opts_no_flag);

        // Count consecutive flagged breaks in each result
        let consec_flagged = |breaks: &[usize]| -> usize {
            breaks
                .windows(2)
                .filter(|w| {
                    is_flagged_penalty(&items, Some(w[0])) && is_flagged_penalty(&items, Some(w[1]))
                })
                .count()
        };
        assert!(
            consec_flagged(&res_avoid.breaks) <= consec_flagged(&res_allow.breaks),
            "High flagged_demerits should never produce more consecutive flagged breaks"
        );
    }

    #[test]
    fn test_forced_mid_paragraph_break_is_respected() {
        let opts = ParagraphLayoutOptions {
            line_break: LineBreakMode::Optimal,
            ..Default::default()
        };
        let items = vec![
            Item::Box { width: 20.0 },
            Item::Glue { width: 4.0, stretch: 2.0, shrink: 1.33 },
            Item::Box { width: 20.0 },
            // Forced justified break at idx 3 (total width = 44, measure = 46 -> r = +1.0)
            Item::Penalty { width: 0.0, penalty: -INF_PENALTY, flagged: false },
            Item::Box { width: 20.0 },
            Item::Glue { width: 4.0, stretch: 2.0, shrink: 1.33 },
            Item::Box { width: 20.0 },
            Item::Glue { width: 0.0, stretch: INF_STRETCH, shrink: 0.0 },
            Item::Penalty { width: 0.0, penalty: -INF_PENALTY, flagged: true },
        ];
        let res = break_items(&items, |_| 46.0, &opts);
        assert_eq!(res.breaks, vec![3, 8]);
        assert!(!res.overfull);
    }

    #[test]
    fn test_looseness_plus_one() {
        let words: Vec<(&str, f64)> = (0..18).map(|_| ("word", 8.0)).collect();
        let opts_opt = ParagraphLayoutOptions {
            line_break: LineBreakMode::Optimal,
            tolerance: 4.0,
            looseness: 0,
            ..Default::default()
        };
        let items = build_simple_paragraph(&words, 3.0, &opts_opt);
        let res_opt = break_items(&items, |_| 60.0, &opts_opt);

        let opts_loose = ParagraphLayoutOptions {
            looseness: 1,
            ..opts_opt
        };
        let res_loose = break_items(&items, |_| 60.0, &opts_loose);

        assert_eq!(
            res_loose.breaks.len(),
            res_opt.breaks.len() + 1,
            "looseness = +1 should produce 1 more line when feasible"
        );
    }

    #[test]
    fn test_overfull_single_word_fallback_never_fails() {
        let opts = ParagraphLayoutOptions {
            line_break: LineBreakMode::Optimal,
            ..Default::default()
        };
        let words = vec![("supercalifragilistic", 120.0), ("end", 15.0)];
        let items = build_simple_paragraph(&words, 4.0, &opts);
        let res = break_items(&items, |_| 50.0, &opts);
        assert!(res.overfull);
        assert_eq!(res.breaks.len(), 2, "Should break after overfull word and at paragraph end");
    }
}
