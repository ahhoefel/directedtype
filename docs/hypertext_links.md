# DirectedType (DTML) Specification: Hypertext Links & Inline Flow

**Status:** Proposed Architecture  
**Topic:** Inline Components, Mixed Text Content, & Hypertext Navigation  
**Target Backend:** Rust Engine, Parley Text Shaping, Vello Compositor  

---

## 1. Executive Summary & Problem Statement

DirectedType models layout as a late-bound Directed Acyclic Graph (DAG) of algebraic equations (`y: prev.bottom + gap`, `x: parent.left`). While this model provides mathematical determinism, zero layout thrashing, and high performance for block-level spatial geometry (`\VStack`, `\HStack`, `\Rect`), it currently lacks a formal **Inline Flow Model**.

Hypertext links (`\Link(url: "https://www.google.com"){google}`) are fundamentally **inline features**. They require:
1. Interspersing raw text runs with components in authoring markup.
2. Delegating spatial positioning and line breaking to a typographic shaping engine (Parley) rather than explicit topological equations.
3. Supporting multi-line line fragments (when a link wraps across line boundaries).
4. Range-level styling (colors, underlines, font weights) within a single text block.
5. Pointer cursor updates and URL navigation upon interaction.

This document details:
- **Part I:** A complete audit of everything currently broken or missing in DirectedType's plain text processing that prevents inline links.
- **Part II:** The architectural specification for Hypertext Links and the Inline Flow System.

---

## Part I: Audit of Current Plain Text Processing Flaws

Our code review of `src/parser/cursor.rs`, `src/compiler/expand.rs`, `src/compiler/text.rs`, `src/compiler/layout.rs`, `src/render/scene.rs`, and `src/render/viewer.rs` reveals seven critical limitations:

### 1. Inter-Element Whitespace Annihilation (`src/parser/cursor.rs`)
In `ParserCursor::parse_content_slot`:
```rust
let flush_text = |buf: &mut String, items: &mut Vec<ContentItem>, start: usize, end: usize, is_end: bool| {
    if buf.is_empty() { return; }
    let normalized = normalize_whitespace(buf);
    buf.clear();

    // BUG: Pure whitespace between elements or at boundaries is discarded
    if normalized.trim().is_empty() {
        return;
    }
    ...
};
```
* **The Bug:** If markup contains two inline elements separated by a space:
  ```dtml
  \Text { \Link(url: "a"){First} \Link(url: "b"){Second} }
  ```
  The text buffer between the two `\Link` invocations contains `" "`. Because `normalized.trim().is_empty()` evaluates to `true`, `flush_text` returns immediately and drops the space entirely. The AST receives `[Node(First), Node(Second)]` with no whitespace between them.
* **Boundary Trimming Instability:** The `items.is_empty()` and `is_end` heuristics prune leading or trailing spaces unevenly when interspersed with inline nodes, corrupting word boundaries around punctuation (e.g. `Click \Link(...){here} , now!`).

### 2. Destruction of Mixed Content in Primitive `\Text` (`src/compiler/expand.rs`)
In `expand_element` (lines 538–546):
```rust
if let Some(content_slot) = &elem.content {
    let mut text_parts = Vec::new();
    for item in &content_slot.items {
        if let ContentItem::Text(chunk) = item {
            text_parts.push(chunk.text.as_str());
        }
    }
    if !text_parts.is_empty() {
        expanded.text_content = Some(text_parts.join(" "));
    }
}
```
* **The Bug:** When extracting `text_content` for a `\Text` primitive, the compiler **only** iterates over `ContentItem::Text` and silently skips all `ContentItem::Node` items.
* **The Result:** If you write:
  ```dtml
  \Text { Visit \Link(url: "https://google.com"){Google} today }
  ```
  The parent `\Text`'s `text_content` becomes `"Visit today"`. The text inside the link (`"Google"`) is completely omitted from the rendered text layout!
* Furthermore, blindly joining text parts with `" "` corrupts spacing if chunks already contain natural whitespace or adjoining punctuation.

### 3. Synthetic Blockification in Containers (`src/compiler/expand.rs`)
When expanding user containers (`expand_component_body`, lines 888–916):
```rust
if let Some(slot) = &instance.content {
    for item in &slot.items {
        match item {
            ContentItem::Node(child_elem) => consumer_child_nodes.push(child_elem.clone()),
            ContentItem::Text(chunk) => {
                let synthetic_text = ElementNode {
                    name: Ident::new("Text", chunk.span),
                    ...
                };
                consumer_child_nodes.push(synthetic_text);
            }
        }
    }
}
```
* **The Bug:** If mixed text and inline nodes are placed inside any container component (like `\VStack`, `\HStack`, or a custom `\Paragraph` container):
  Each text chunk is wrapped into an isolated block `\Text` node.
* **The Result:** In a `\VStack`, the container's `\Children` rule (`y: prev ? prev.bottom + gap : y`) arranges the chunks vertically:
  1. Block 1: `"Visit "`
  2. Block 2: `\Link` (`"Google"`)
  3. Block 3: `" today"`
  They stack on three separate lines instead of flowing inline.

### 4. Primitive Children Are Spatially Unbound / Overlapped (`src/compiler/expand.rs`)
When child nodes appear inside a primitive like `\Text`, lines 1531–1556 expand them into `doc.nodes`. However:
* `\Text` does not declare a `\Children` directive.
* There are no equations to compute the child's `x`, `y`, `width`, or `height`.
* By default, the Base Spatial Trait assigns fallback ports: `x: parent.left`, `y: parent.top`.
* **The Result:** The child node floats at `(0, 0)` or the top-left corner of the paragraph, overlapping unrelated text.

### 5. Single Bounding Box (`Rect`) vs. Multi-Line Wrapped Links (`src/compiler/layout.rs`)
Every `ResolvedNode` in DirectedType is defined with exactly one spatial rectangle:
```rust
pub struct ResolvedNode {
    pub id: NodeId,
    pub rect: Rect, // { x, y, width, height }
    ...
}
```
* **The Bug:** An inline link can break across lines:
  ```
  Line 1: For further assistance, check out our comprehensive
  Line 2: documentation guide and tutorials.
  ```
  If `"comprehensive documentation guide"` is wrapped into `\Link`:
  - Part of the link is at the end of Line 1.
  - Part of the link is at the beginning of Line 2.
* An axis-aligned `Rect` enclosing both lines forms a massive rectangle that encompasses the entire width between Line 1 and Line 2.
* **The Result:** Clicking or hovering over unlinked text between Line 1 and Line 2 would falsely trigger the link. Inline nodes cannot be represented by a single `Rect`; they require **disjoint fragment rects**.

### 6. Homogeneous Flat String Measurement & Rendering (`src/compiler/text.rs` & `src/render/scene.rs`)
* In `src/compiler/text.rs`, `measure_text_bounds` and `measure_text_height` take a single `&str` and single `font_size`, `font_weight`, and `font_family`.
  If a link is styled with a different font weight (e.g. bold) or larger size, the layout engine calculates the wrong height for the text block, causing clipping.
* In `src/render/scene.rs`, text is painted via a single solid brush:
  ```rust
  scene.draw_glyphs(font)
      .brush(Brush::Solid(text_color))
      .draw(...);
  ```
  Parley's `ranged_builder` supports per-range brushes and styles, but DirectedType passes no range metadata.
* There is no text decoration rendering (no underlines or strikethroughs).

### 7. Coarse Hit Testing & Missing System Cursor Integration (`src/render/viewer.rs`)
* `layout.hit_test(point)` only tests primitive bounding boxes. It cannot resolve clicks to sub-ranges or character offsets within text.
* The viewer does not update the window cursor (`window.set_cursor(CursorIcon::Pointer)`) when hovering over interactive inline elements.
* There is no URL dispatch mechanism to open clicked links in the host operating system.

---

## Part II: Hypertext Links & Inline Flow Architecture

To resolve these issues while maintaining DirectedType's late-bound mathematical graph model, we introduce the **Rich Text Inline Flow Architecture**.

```
┌────────────────────────────────────────────────────────┐
│ 1. Markup Authoring                                    │
│ \Text(width: 480) {                                    │
│     Visit \Link(url: "https://google.com"){Google} now │
│ }                                                      │
└───────────────────────────┬────────────────────────────┘
                            │ (Parse & AST Lowering)
                            ▼
┌────────────────────────────────────────────────────────┐
│ 2. Text Run Flattening & Span Registration             │
│ • Full String: "Visit Google now"                      │
│ • Span 0 (0..6):   Default Style                       │
│ • Span 1 (6..12):  NodeId(Link), #1a73e8, Underline    │
│ • Span 2 (12..16): Default Style                       │
└───────────────────────────┬────────────────────────────┘
                            │ (Topological DAG Evaluation)
                            ▼
┌────────────────────────────────────────────────────────┐
│ 3. Parley Typographic Line Breaking                    │
│ • Breaks lines at \Text.width                          │
│ • Computes \Text.height                                │
│ • Projects Fragment Rects onto \Link Node:             │
│   Link.fragments = [ Rect(Line 1), Rect(Line 2) ]      │
│   Link.rect = BoundingUnion(fragments)                 │
└───────────────────────────┬────────────────────────────┘
                            │ (Compositor & Interaction)
                            ▼
┌────────────────────────────────────────────────────────┐
│ 4. Vello Rendering & Multi-Fragment Hit Testing        │
│ • Vello draws glyphs with range brushes & underlines   │
│ • Hover over any fragment -> CursorIcon::Pointer       │
│ • Click on any fragment -> Open URL / emit Click event │
└────────────────────────────────────────────────────────┘
```

---

## 2. Language Syntax & Authoring

### A. Inline Link Syntax
```dtml
\Text(width: 500, size: 16) {
    Welcome to DirectedType. For details, consult our 
    \Link(url: "https://directedtype.org"){documentation portal} 
    or reach out to support.
}
```

### B. Custom Styled Link
```dtml
\Link(
    url: "https://github.com",
    color: #2563eb,
    hover_color: #1d4ed8,
    underline: true
){GitHub Repository}
```

### C. Standalone Block Link
When instantiated outside a `\Text` context (e.g. directly in a `\VStack`), `\Link` wraps itself in an intrinsic `\Text` element:
```dtml
\VStack(gap: 12) {
    \Header { Helpful Resources }
    \Link(url: "https://docs.directedtype.org"){1. Developer Documentation}
    \Link(url: "https://crates.io"){2. Rust Crate Registry}
}
```

---

## 3. AST & Data Structures

### A. Preserving Inter-Element Whitespace in `ParserCursor`
Refactor `src/parser/cursor.rs` so that inter-element whitespace is preserved:
```rust
// Whitespace chunks between inline elements must NOT be discarded if they contain spaces.
// They should collapse to a single space " ".
if normalized.trim().is_empty() {
    if !items.is_empty() && !is_end {
        items.push(ContentItem::Text(TextChunk {
            text: " ".to_string(),
            span: Span::new(start, end),
        }));
    }
    return;
}
```

### B. Rich Text Spans on `ResolvedNode`
Extend `ResolvedNode` in `src/compiler/layout.rs`:
```rust
#[derive(Debug, Clone, PartialEq)]
pub struct TextSpan {
    /// Byte range in parent node's concatenated `text_content`
    pub range: std::ops::Range<usize>,
    /// Associated inline component (e.g. NodeId of \Link)
    pub node_id: Option<NodeId>,
    /// Typographic style overrides
    pub style: SpanStyle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpanStyle {
    pub color: Option<Color>,
    pub font_size: Option<f64>,
    pub font_weight: Option<f64>,
    pub font_family: Option<String>,
    pub underline: bool,
    pub url: Option<String>,
    pub cursor: Option<CursorKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorKind {
    Default,
    Pointer,
    Text,
}
```

And add to `ResolvedNode`:
```rust
pub struct ResolvedNode {
    ...
    pub text_content: Option<String>,
    pub text_spans: Vec<TextSpan>,
    /// Multi-line disjoint fragment bounding boxes for inline elements
    pub fragments: Vec<Rect>,
    ...
}
```

---

## 4. Compiler & Expansion Pipeline

### A. Mixed Content Flattening in `\Text`
When `expand_element` encounters `\Text`, it gathers both plain text chunks and inline child nodes:

```rust
if elem.name.as_str() == "Text" {
    let mut full_text = String::new();
    let mut spans = Vec::new();

    if let Some(slot) = &elem.content {
        for item in &slot.items {
            match item {
                ContentItem::Text(chunk) => {
                    let start = full_text.len();
                    full_text.push_str(&chunk.text);
                    let end = full_text.len();
                    spans.push(TextSpan {
                        range: start..end,
                        node_id: None,
                        style: SpanStyle::default(),
                    });
                }
                ContentItem::Node(child_elem) => {
                    // Expand child node into the document
                    let child_id = expand_element(child_elem, ...)?;
                    
                    // Extract link text
                    let link_text = extract_node_text(child_elem);
                    let start = full_text.len();
                    full_text.push_str(&link_text);
                    let end = full_text.len();

                    let link_style = SpanStyle {
                        color: child_ports.get("color").map(parse_color).or(Some(DEFAULT_LINK_COLOR)),
                        underline: child_ports.get("underline").unwrap_or(true),
                        url: child_ports.get("url").map(|v| v.as_str()),
                        cursor: Some(CursorKind::Pointer),
                        ..Default::default()
                    };

                    spans.push(TextSpan {
                        range: start..end,
                        node_id: Some(child_id),
                        style: link_style,
                    });
                }
                _ => {}
            }
        }
    }
    expanded.text_content = Some(full_text);
    expanded.text_spans = spans;
}
```

### B. Projecting Parley Geometry to Child Inline Nodes
During DAG evaluation:
1. `\Text.width` is evaluated.
2. The Parley layout engine shapes the text and breaks lines.
3. For each `TextSpan` referencing a `child_id`:
   - Query Parley's line layout for all line items / cluster bounds falling within `span.range`.
   - Merge contiguous glyph runs on each line into line fragments: `Vec<Rect>`.
   - Set `child_node.fragments = line_fragments`.
   - Set `child_node.rect = bounding_union(&line_fragments)`.
   - Expose public alias ports on the child:
     `child.x = child_node.rect.x`
     `child.y = child_node.rect.y`
     `child.width = child_node.rect.width`
     `child.height = child_node.rect.height`

---

## 5. Rendering & Compositing (Vello)

In `src/render/scene.rs`:
```rust
let mut builder = layout_cx.ranged_builder(font_cx, full_text, 1.0, true);
builder.push_default(StyleProperty::FontSize(base_font_size));
builder.push_default(StyleProperty::FontWeight(FontWeight::new(base_font_weight)));

// Apply ranged styles for inline links and spans
for span in &node.text_spans {
    if let Some(color) = span.style.color {
        builder.push(StyleProperty::Brush(Brush::Solid(color)), span.range.clone());
    }
    if span.style.underline {
        builder.push(StyleProperty::Underline(true), span.range.clone());
    }
    if let Some(weight) = span.style.font_weight {
        builder.push(StyleProperty::FontWeight(FontWeight::new(weight as f32)), span.range.clone());
    }
}

let mut layout = builder.build(full_text);
if node.rect.width > 0.0 {
    layout.break_all_lines(Some(node.rect.width as f32));
}

// Render glyphs using their active run brush (preserves individual link colors)
for line in layout.lines() {
    for item in line.items() {
        if let PositionedLayoutItem::GlyphRun(glyph_run) = item {
            let brush = glyph_run.style().brush.unwrap_or(default_brush);
            scene.draw_glyphs(glyph_run.run().font())
                .font_size(glyph_run.run().font_size())
                .transform(Affine::translate((node.rect.x, node.rect.y)))
                .brush(brush)
                .draw(Fill::NonZero, glyph_run.positioned_glyphs().map(...));
        }
    }
}
```

---

## 6. Interaction, Hit-Testing & Navigation

### A. Multi-Fragment Hit Testing
Update `hit_test` in `src/compiler/layout.rs`:
```rust
pub fn hit_test(&self, point: Point) -> Option<HitTestResult> {
    for node in self.render_order().into_iter().rev() {
        // Multi-line inline fragment test
        if !node.fragments.is_empty() {
            let hits_fragment = node.fragments.iter().any(|frag| {
                point.x >= frag.x && point.x <= frag.x + frag.width
                    && point.y >= frag.y && point.y <= frag.y + frag.height
            });
            if !hits_fragment {
                continue;
            }
        } else {
            // Standard single bounding box test
            if !rounded_rect_contains(&node.rect, radius, point) {
                continue;
            }
        }

        return Some(HitTestResult {
            target: node.id,
            global_point: point,
            local_point: Point::new(point.x - node.rect.x, point.y - node.rect.y),
            bubble_path: self.build_bubble_path(node.id),
        });
    }
    None
}
```

### B. Cursor Updates & Navigation in `src/render/viewer.rs`
1. **Cursor Update on Hover:**
   When `self.layout.hit_test(point)` returns a node whose properties include `cursor: Pointer` or `url`:
   ```rust
   window.set_cursor(winit::window::CursorIcon::Pointer);
   ```
   When hovering off, reset to `CursorIcon::Default`.

2. **Click Navigation:**
   Upon receiving `EventKind::Click`:
   ```rust
   if let Some(url) = target_node.properties.get("url").and_then(|v| v.as_str()) {
       println!("[Viewer] Navigating to URL: {}", url);
       #[cfg(not(target_arch = "wasm32"))]
       let _ = open::that(url);
       #[cfg(target_arch = "wasm32")]
       let _ = web_sys::window().and_then(|w| w.open_with_url_and_target(url, "_blank").ok());
   }
   ```

---

## 7. Migration & Implementation Milestones

| Phase | Scope | Key Deliverables |
| :--- | :--- | :--- |
| **Phase 1: Parser Whitespace** | `src/parser/cursor.rs` | Fix `flush_text` to preserve single space between inline elements (`\A{} \B{}`). |
| **Phase 2: Rich Text Data Model** | `src/ast.rs`, `src/compiler/layout.rs` | Add `TextSpan`, `SpanStyle`, and `fragments: Vec<Rect>` to `ResolvedNode`. |
| **Phase 3: Parley Ranged Styles** | `src/compiler/text.rs`, `src/render/scene.rs` | Support per-range font colors, weights, and underlines in Vello drawing. |
| **Phase 4: Fragment Projection** | `src/compiler/expand.rs`, `layout.rs` | Project line fragment boxes from Parley layout onto inline child nodes. |
| **Phase 5: Hit-Testing & Viewer** | `src/render/viewer.rs` | Multi-fragment hit testing, `CursorIcon::Pointer`, and host URL activation (`open::that`). |
| **Phase 6: Standard Component** | `components/Link.dt` | Deliver `\Link` component in standard library with hover states and styles. |
