# DirectedType Design Document: Text Shaping Abstraction, Hyphenation, & Knuth-Plass Paragraph Optimization

- **Status:** Proposed (Future Enhancement — Orthogonal to Initial PDF & Pagination Rollout)
- **Date:** November 2026
- **Target Area:** [`src/compiler/text.rs`](file:///Users/hoefel/dev/directedtype/src/compiler/text.rs), [`src/compiler/layout.rs`](file:///Users/hoefel/dev/directedtype/src/compiler/layout.rs), [`src/render/scene.rs`](file:///Users/hoefel/dev/directedtype/src/render/scene.rs), and future `src/render/pdf.rs`

---

## 1. Executive Summary & Key Conclusions

DirectedType currently uses `parley` for rich text shaping, bidirectional analysis, inline-box flow, and line breaking. When planning a PDF backend and TeX-style book/article pagination, a natural architectural question arises: *should we replace Parley now, or keep it while decoupling our architecture so a TeX-grade line breaker can replace it later?*

This document records the architectural answer and the concrete design for a future Knuth-Plass + hyphenation text engine:

1. **No Blocker for PDF or Pagination:** Keeping `parley` today works identically for both interactive windowed rendering (`vello`) and paginated print output (`krilla` PDF). Neither the choice of PDF library (`krilla` / `pdf-writer`) nor the DAG pagination design (`\PagedFlow`) depends on whether lines were broken by Parley or by a custom Knuth-Plass engine—both emit compact PDF `Tj`/`TJ` line operators from pre-shaped glyph runs, and both paginate over the exact same sequence of broken lines.
2. **Small Current Footprint:** Across the core compiler and document renderer, `parley` is already confined to just two files: [`src/compiler/text.rs`](file:///Users/hoefel/dev/directedtype/src/compiler/text.rs) (DAG measurement, `\Span` fragments, and `\InlineBox` positioning) and [`src/render/scene.rs`](file:///Users/hoefel/dev/directedtype/src/render/scene.rs) (glyph run extraction into Vello). *(Auxiliary chrome modules such as [`src/render/browser_bar.rs`](file:///Users/hoefel/dev/directedtype/src/render/browser_bar.rs), [`src/inspector/overlay.rs`](file:///Users/hoefel/dev/directedtype/src/inspector/overlay.rs), [`src/inspector/panel.rs`](file:///Users/hoefel/dev/directedtype/src/inspector/panel.rs), and [`src/bin/vello_testbed.rs`](file:///Users/hoefel/dev/directedtype/src/bin/vello_testbed.rs) use `parley` only for UI strings outside the document pipeline.)* Meanwhile, DirectedType's vertical typographic landmarks (`TextStart::Capital`, `Ascender`, `EmTop`, `TextEnd::Baseline`, `Descender`) are already computed directly via **`skrifa`** in [`measure_font_metrics`](file:///Users/hoefel/dev/directedtype/src/compiler/text.rs#L388-L540), independent of Parley's line-breaker.
3. **Why Eventually Replace or Augment Parley:** Parley implements browser-style **greedy** line-breaking and lacks built-in dictionary hyphenation, global Knuth-Plass paragraph optimization, and microtypography (optical margin protrusion and subtle font/tracking expansion). Those features make a dramatic visual difference in narrow, justified (`Align.Justify`) book and academic columns.
4. **The Single Prerequisite Worth Doing Early:** Introduce an engine-agnostic **`ShapedParagraph` Intermediate Representation (IR)** and **`TextLayoutEngine` trait** inside [`src/compiler/text.rs`](file:///Users/hoefel/dev/directedtype/src/compiler/text.rs) so the DAG shapes each `\Text` node once, caches the resulting `Arc<ShapedParagraph>` on `ResolvedNode`, and feeds it unchanged to Vello, `krilla` PDF, and pagination.

---

## 2. Deconstructing What Parley Actually Does Under the Hood

"Swapping out Parley" does **not** mean rewriting OpenType font parsing, glyph shaping, or script fallback from scratch. Parley is a coordinator over four distinct stages, three of which are handled by standalone Rust crates that DirectedType can continue to use directly:

| Pipeline Stage | Underlying Responsibility | Crate(s) Used | Keep or Replace Later? |
| :--- | :--- | :--- | :--- |
| **1. Font Discovery & Fallback** | System font enumeration, weight/style matching, per-character script & emoji fallback chains | `fontique` | **Keep** (or wrap directly) |
| **2. Unicode Analysis** | Bidirectional text reordering (UAX #9), script/cluster segmentation, legal line-break opportunities (UAX #14) | `unicode-bidi`, `unicode-linebreak` / ICU4X | **Keep** via standard Unicode crates |
| **3. OpenType Shaping & Font Metrics** | GSUB/GPOS shaping (ligatures, kerning, diacritic marks), `OS/2` & `cap_height`/`x_height` metrics | `harfrust`, `skrifa` | **Keep** (`skrifa` is already a direct dependency in [`Cargo.toml`](file:///Users/hoefel/dev/directedtype/Cargo.toml)) |
| **4. Line Breaking & Justification** | Choosing break points, inserting discretionary hyphens, distributing inter-word glue, computing line `x`/`y` positions | Currently greedy inside `parley` | **Replace** with Knuth-Liang hyphenation (`hypher`) + Knuth-Plass DP |

### Why One-Engine Parity Must Hold Across Screen and Print

In DirectedType, text shaping is part of **Phase 2 DAG Evaluation**, not an isolated post-layout rendering step:
- Container heights (`\Card`, `\VStackStretch`) depend algebraically on `\Text.height`.
- `\InlineBox` components (`\Kbd`, `\Badge`) inject intrinsic `(width, height, baseline_offset)` *into* line-breaking and receive `(x, y)` coordinates *out*.
- `\Span` output ports (`x`, `y`, `width`, `height`, `baseline` in [`examples/text_flow_demo.dt`](file:///Users/hoefel/dev/directedtype/examples/text_flow_demo.dt)) wire directly to downstream nodes such as margin sidenotes.

Therefore, whichever engine (`ParleyEngine` or `KnuthPlassEngine`) breaks lines during DAG evaluation **must** supply the exact positioned glyphs to both the Vello GPU renderer and the PDF backend. Running Parley during DAG evaluation and a different line-breaker during PDF export would cause line-count drift and box collisions.

---

## 3. Architectural Prerequisite: The Engine-Agnostic `ShapedParagraph` IR

Abstracting Parley behind an intermediate representation both future-proofs the compiler for Knuth-Plass and eliminates two concrete inefficiencies in the current codebase.

### 3.1 Current Coupling & Redundancy in the Codebase

1. **Double (and Triple) Shaping:** Today, [`src/compiler/text.rs`](file:///Users/hoefel/dev/directedtype/src/compiler/text.rs) invokes Parley during DAG evaluation inside `measure_rich_text_bounds_with_boxes` ([`src/compiler/text.rs`](file:///Users/hoefel/dev/directedtype/src/compiler/text.rs#L198-L369)), [`compute_span_fragments_with_metrics_and_boxes`](file:///Users/hoefel/dev/directedtype/src/compiler/text.rs#L653-L888), and [`compute_span_range_metrics_with_boxes`](file:///Users/hoefel/dev/directedtype/src/compiler/text.rs#L927-L1143). Then [`build_scene`](file:///Users/hoefel/dev/directedtype/src/render/scene.rs#L864-L954) runs `text_layout_cx.ranged_builder` and `layout_text.break_all_lines` *all over again* from scratch on the first frame to extract `vello::Glyph` runs.
2. **The 250-Span Dummy Brush Workaround:** Because Parley's `StyleProperty` does not carry an arbitrary span tag ID, [`src/compiler/text.rs`](file:///Users/hoefel/dev/directedtype/src/compiler/text.rs#L716-L719) and [`src/compiler/text.rs`](file:///Users/hoefel/dev/directedtype/src/compiler/text.rs#L983-L986) encode span indices into synthetic RGBA brush colors (`((idx + 1) % 250 + 1) as u8`) to recover which `\Span` owns each `GlyphRun`—wrapping silently past 250 spans in a single paragraph.

### 3.2 Proposed IR Types & `TextLayoutEngine` Trait

During Phase 2 DAG evaluation, each `\Text` node is shaped **once** (per resolved `(text, width, font, spans, inline_boxes)` tuple) into an immutable `Arc<ShapedParagraph>` stored on `ResolvedNode` in [`src/compiler/layout.rs`](file:///Users/hoefel/dev/directedtype/src/compiler/layout.rs):

```rust
/// Engine-agnostic shaped paragraph produced once during DAG evaluation
/// and shared by Vello, Krilla PDF, and pagination (\PagedFlow).
#[derive(Debug, Clone)]
pub struct ShapedParagraph {
    pub lines: Vec<ShapedLine>,
    /// Resolved paragraph width (tight content width or constrained box width).
    pub width: f64,
    /// Total height according to the active TextBounds (start_at .. end_at).
    pub height: f64,
    /// Vertical offset aligning the first line to `start_at` (Capital, Ascender, EmTop, LineHeight).
    pub y_offset: f64,
}

#[derive(Debug, Clone)]
pub struct ShapedLine {
    /// Y coordinate of the baseline relative to the top of the unshifted text layout.
    pub baseline: f64,
    pub ascent: f64,
    pub descent: f64,
    /// Line box top/bottom bounds for pagination slicing and highlight rects.
    pub block_min: f64,
    pub block_max: f64,
    /// True if this line ends with an inserted discretionary hyphen (used for PDF copy-paste
    /// reconstruction and TeX-style `\brokenpenalty` during page breaking).
    pub ends_with_hyphen: bool,
    pub items: Vec<ShapedLineItem>,
}

#[derive(Debug, Clone)]
pub enum ShapedLineItem {
    GlyphRun(ShapedGlyphRun),
    InlineBox {
        node_id: NodeId,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    },
}

#[derive(Debug, Clone)]
pub struct ShapedGlyphRun {
    /// Shared reference to the resolved OpenType font bytes (e.g. from `fontique` / `peniko::Blob`).
    pub font_data: Arc<[u8]>,
    pub font_index: u32,
    pub font_size: f32,
    /// Owning `\Span` node, if any (for dynamic hover/focus color, underline, and link hit regions).
    pub span_node_id: Option<NodeId>,
    /// UTF-8 byte range in the parent `\Text` content (excluding inserted discretionary hyphens).
    pub text_range: Range<usize>,
    pub color: [u8; 4],
    pub underline: bool,
    pub baseline_shift: f32,
    pub glyphs: Vec<PositionedGlyph>,
}

#[derive(Debug, Clone, Copy)]
pub struct PositionedGlyph {
    pub glyph_id: u16,
    /// UTF-8 byte cluster range relative to the parent `\Text` string (for PDF `/ToUnicode` & selection).
    pub cluster_start: u32,
    pub cluster_end: u32,
    /// True if this glyph is an inserted soft/automatic hyphen not present in the source string.
    pub is_inserted_hyphen: bool,
    /// Position relative to the paragraph's local origin `(0.0, 0.0)`.
    pub x: f32,
    pub y: f32,
    pub advance: f32,
}

pub trait TextLayoutEngine: Send + Sync {
    fn shape_paragraph(&mut self, req: &TextLayoutRequest<'_>) -> ShapedParagraph;
}
```

### 3.3 Architectural Payoffs of the `ShapedParagraph` Boundary

```text
  \Text AST + Ports + Spans + InlineBoxes
                   │
                   ▼   (Phase 2 DAG Evaluation — shapes ONCE)
      dyn TextLayoutEngine
      ├── V1: ParleyEngine
      └── V2: KnuthPlassEngine (harfrust + hypher + DP)
                   │
                   ▼
          Arc<ShapedParagraph>
       ┌───────────┼───────────────┐
       ▼           ▼               ▼
  Pagination    Vello Scene    Krilla PDF
 (\PagedFlow)  (scene.rs)      (pdf.rs)
 slices lines  draws glyphs   draws glyphs +
 by page       + local cache  /ToUnicode CMap
```

1. **Zero `parley` Imports Outside `compiler/text`:** [`src/render/scene.rs`](file:///Users/hoefel/dev/directedtype/src/render/scene.rs) and the future `src/render/pdf.rs` read `&ShapedParagraph` directly. Neither renderer ever instantiates a Parley `LayoutContext` or `FontContext`.
2. **Clean Pagination Slicing:** `\PagedFlow` splits a tall `\Text` node across pages by slicing `paragraph.lines[start_line..end_line]` using `block_min`/`block_max` and `baseline`, without knowing anything about Parley or Knuth-Plass. It can even read `line.ends_with_hyphen` to discourage page breaks immediately after a hyphenated line (TeX's `\brokenpenalty`).
3. **Exact PDF Search & Copy-Paste:** Recording `cluster_start..cluster_end` and `is_inserted_hyphen` on `PositionedGlyph` lets `krilla` build accurate `/ToUnicode` CMaps and avoids polluting copied PDF text with spurious hyphens when a word breaks across a line.
4. **Span & InlineBox Metrics Without Re-Shaping:** All `\Span` fragment rects, `first_line_baseline`, and `\InlineBox` `(x, y)` positions are extracted in a single pass directly from `ShapedParagraph.lines`, eliminating the `% 250` dummy brush hack and redundant shaping calls.

---

## 4. Benefits & Technical Design of a Future Knuth-Plass + Hyphenation Engine

Once `ShapedParagraph` is in place, upgrading from `ParleyEngine` to `KnuthPlassEngine` unlocks four book-publishing capabilities that greedy line-breakers cannot achieve.

### 4.1 Global Paragraph Optimization (Eliminating "Rivers")

- **Greedy Breaking (Parley / Browsers):** Fills each line until the next item overflows `max_width`, then breaks immediately. If a long word starts the next line, the current line stretches wildly while surrounding lines stay tight—creating distracting vertical whitespace channels ("rivers") in justified text.
- **Knuth-Plass Dynamic Programming:** Models a paragraph as a stream of **Boxes** (shaped word segments / inline boxes), **Glue** (stretchable/shrinkable inter-word spaces), and **Penalties** (explicit breaks, UAX #14 opportunities, and hyphenation points). It finds the globally optimal set of line breaks $B = (b_0, b_1, \dots, b_k)$ minimizing total **demerits** across the entire paragraph:
  $$\text{demerits}(i, j) = (1 + \text{badness}(r_{i,j}) + p_j)^2 + \Delta_{\text{adj}} + \Delta_{\text{double\_hyphen}}$$
  where:
  - $r_{i,j}$ is the **adjustment ratio** measuring how much the line's glue stretches ($r > 0$) or shrinks ($r < 0$) to hit the target column width, and $\text{badness}(r) \approx 100 \cdot |r|^3$ strongly penalizes over-stretched lines.
  - $\Delta_{\text{adj}}$ penalizes jarring transitions between tight ($r < -0.5$), normal, loose ($r > 0.5$), and very loose adjacent lines.
  - $\Delta_{\text{double\_hyphen}}$ (`\doublehyphendemerits`) discourages hyphenating two consecutive lines.
  - A **runt-avoidance penalty** (`\finalhyphendemerits` / last-line minimum fill) prevents stranding a tiny 2–3 character word fragment alone on the final line of a paragraph.

### 4.2 Dictionary Hyphenation & Discretionary Ligature Reshaping

Justifying narrow book columns without hyphenation mathematically forces excessive word spacing whenever long words appear.

- **Pattern Dictionary (`hypher` / `hyphenation`):** Using Knuth-Liang hyphenation tries (specifically `hypher`, the lightweight pure-Rust crate used by Typst, with zero heap allocations per lookup), the shaper locates valid syllable split indices for each word based on `env lang` (default `"en-US"`) when `hyphenate: true` is active.
- **Handling Ligatures & Contextual Shaping Across Breaks:**
  Naïvely inserting `U+00AD` or splitting already-shaped glyph runs fails on OpenType ligatures and kerning. For example, in `"difficult"`, `harfrust` fuses `f` + `f` + `i` into a single `ffi` ligature glyph. If Knuth-Plass breaks the word as `"dif-"` / `"ficult"`, that ligature must split into `f` + `-` at the end of line $n$ and the `fi` ligature at the start of line $n+1$.
  - **Design Solution:** Shape the un-broken word once for fast-path Box measurement. Only for words that survive as **candidate** break points near a line edge, lazily shape and memoize the `(prefix + "-", suffix)` pair via `harfrust` in a per-paragraph segment cache. Because a typical paragraph has only a handful of candidate break words per line, total shaping work stays fast while guaranteeing exact OpenType metrics for both broken and unbroken forms.

### 4.3 Microtypography (Optical Margin Protrusion & Font Expansion)

With full ownership of the Knuth-Plass cost function and `PositionedGlyph` placement:
1. **Optical Margin Alignment (Character Protrusion):** Glyphs like trailing hyphens (`-`), periods (`.`), commas (`,`), and opening/closing quotation marks (`"`, `'`) carry negligible visual ink weight. Hanging them slightly outside the column edge (`0.05em–0.25em`, e.g., protruding a trailing hyphen by `~0.5 * advance`) makes justified left and right column edges look optically razor-straight.
2. **Subtle Tracking / Font Expansion (Hz-algorithm):** Allowing inter-glyph tracking or horizontal scale to flex within $\pm 1\%\text{–}2\%$ adds continuous micro-elasticity across every character on a line, cutting badness dramatically in narrow multi-column or sidenote-heavy layouts without perceptible distortion.

### 4.4 Variable-Width Paragraph Shapes (`\parshape` / Drop Caps)

Because Knuth-Plass evaluates candidate line $k$ against `target_width(k)` inside its DP graph:
- Supporting **drop caps** (`\DropCap` indenting lines $0..3$ differently from lines $4..N$) or **text wrapping around an inset figure** becomes a straightforward lookup `target_width(line_index)` rather than an architectural rewrite.
- Similarly, if a future V2+ pagination mode allows differing column widths across recto/verso pages mid-paragraph, the line-breaker can naturally adjust `target_width` per line once the starting line's page offset is known.

---

## 5. Staged Rollout & Language Surface

Because the `ShapedParagraph` IR decouples rendering and pagination from the line-breaking algorithm, this work can proceed in three low-risk stages without blocking PDF export:

### Stage 0 — `ShapedParagraph` IR Extraction (Alongside PDF Backend V1)
- Refactor [`src/compiler/text.rs`](file:///Users/hoefel/dev/directedtype/src/compiler/text.rs) to expose `TextLayoutEngine` backed by `ParleyEngine`.
- Store `Arc<ShapedParagraph>` on `ResolvedNode` in [`src/compiler/layout.rs`](file:///Users/hoefel/dev/directedtype/src/compiler/layout.rs).
- Update [`src/render/scene.rs`](file:///Users/hoefel/dev/directedtype/src/render/scene.rs) and the new `src/render/pdf.rs` (`krilla`) to draw strictly from `&ShapedParagraph`, removing `parley` imports from all document renderers.

### Stage 1 — Optional Soft-Hyphen Injection on `ParleyEngine`
- Add `hypher` as an optional hyphenation preprocessor that inserts `U+00AD` soft hyphens into words above a minimum length (e.g., $\ge 5$ chars) when `hyphenate: true`.
- Gives immediate relief for narrow justified columns with minimal code, while keeping Parley's greedy breaker.

### Stage 2 — Full `KnuthPlassEngine`
- Implement `KnuthPlassEngine` using `fontique` + `unicode-bidi` + `harfrust` + `skrifa` + `hypher` + Knuth-Plass DP + optical protrusion.
- Expose clean opt-in controls on `\Text` / `\env` (preserving exact backward compatibility for existing UI examples):
  - `hyphenate: Boolean = false` (or `env hyphenate: Boolean`)
  - `lang: String = "en-US"` (or `env lang: String`)
  - `line_break: LineBreak = LineBreak.Optimal | LineBreak.Greedy` (defaulting to `Greedy` for single-line UI labels / fast interactive frames and `Optimal` for `Align.Justify` or print/book themes)
  - `protrusion: Boolean = false` (optical hanging punctuation)

---

## 6. Compatibility with DirectedType's DAG & Caching Model

1. **Deterministic Single-Pass DAG Preservation:** `KnuthPlassEngine::shape_paragraph` is a pure function of `(text, width, font, spans, inline_boxes, lang, line_break_mode)`. It adds zero feedback edges and preserves DirectedType's strict topological evaluation.
2. **Caching During Interaction & Scrolling:** Because `ShapedParagraph` is keyed by content and `width` (excluding `node.rect.x` and `node.rect.y`), scrolling in the interactive Vello viewer continues to hit 100% of cached `vello::Scene` and `ShapedParagraph` entries without re-running Knuth-Plass DP.
3. **Fallback Safety:** `ParleyEngine` remains available as a reference implementation and for UI chrome (`browser_bar.rs`, inspector panels), allowing golden image/PDF regression tests to diff `ParleyEngine` vs. `KnuthPlassEngine` line-for-line during development.
