# GPU Acceleration & Scrolling Performance in DirectedType

## 1. Executive Summary

When scrolling a `\ScrollView` in DirectedType, the interface can feel sluggish and stuttery, especially on macOS trackpads.

### Are we taking advantage of the GPU to translate regions when scrolling?
**No.** DirectedType does **not** currently use the GPU to translate regions or composited layers when scrolling.
- **Hardware GPU Clipping is active**: The `ScrollView` boundary uses Vello's clip layer stack (`scene.push_clip_layer`), discarding out-of-bounds pixels in GPU compute shaders.
- **Translation is pure CPU reflow**: Scrolling mutates `scroll_y`, which DirectedType treats as an algebraic layout change. The CPU dependency graph evaluates new absolute `(x, y)` positions for every child, rebuilds the entire render scene from scratch, re-breaks text lines across all paragraphs, and submits brand-new geometry to the GPU every frame.

---

## 2. Root Cause Analysis: The Scroll Bottleneck Chain

Tracing a single scroll event through the engine reveals why performance degrades:

```
[ MouseWheel Event (Trackpad) ]
            │
            ▼
[ ScrollView.rs: on_scroll ]
  - ctx.set_state("scroll_y", new_y)
            │
            ▼
[ DirectedType DAG: invalidate_and_reevaluate ]
  - Marks scroll_y dirty
  - Downstream layout formulas recomputed: content_top, children y, thumb_y
  - update_resolved_layout updates absolute rect.x, rect.y on all descendant nodes
            │
            ▼
[ ViewerApp: Event Dispatch ]
  - self.layout = compiled.layout().clone()  <-- Full heap clone of all 99+ nodes!
  - window.request_redraw()
            │
            ▼
[ Redraw: build_scene ]
  - Allocates new Parley LayoutContext
  - For EVERY Text node (even off-screen):
      * parley::ranged_builder(...)
      * layout.break_all_lines(...) [Expensive bidi & line-wrapping algorithm]
      * layout.align(...)
      * Iterate glyphs and emit vello draw_glyphs at Affine::translate(node.rect.x, node.rect.y)
            │
            ▼
[ Vello GPU Pipeline ]
  - Reruns pathtag, bbox, coarse & fine tile rasterization shaders from scratch
```

### Key Bottlenecks Identified

1. **CPU Layout Reflow on Every Wheel Tick**:
   In `components/ScrollView.dt`:
   ```dt
   let content_top = y + padding - scroll_y;
   let content_left = x + padding - scroll_x;

   \Children {
       clip: clip,
       x: prev ? prev.left : content_left,
       y: prev ? prev.bottom : content_top
   }
   ```
   Because `scroll_y` is fed directly into child position equations, scrolling is modeled as a physical layout reflow rather than a visual translation.

2. **Expensive Text Re-shaping and Line Breaking**:
   In `src/render/scene.rs`, Parley's line breaking (`layout.break_all_lines`) and alignment passes run unconditionally for every paragraph of text in the document on every frame. During a scroll, text characters, font sizes, weights, and wrapping widths do not change—only their vertical translation changes.

3. **Full Layout Cloning (`self.layout.clone()`)**:
   In `src/render/viewer.rs:720`, mutating state triggers a full clone of `ResolvedLayout`. Every `ResolvedNode` contains multiple heap-allocated `HashMap`s (`properties`, `formulas`, `event_handlers`), `Vec`s, and `String`s. At 120Hz trackpad polling, this creates massive heap churn and GC-like allocator stalls.

4. **No Viewport / Spatial Culling**:
   `build_scene` loops over all nodes in the document. Text and cards that are hundreds of pixels below the visible `ScrollView` boundary are still measured, formatted, shaped, and pushed to Vello.

5. **Uncoalesced High-Frequency Input**:
   Precision trackpads on macOS generate hundreds of `PixelDelta` events per second. Handling each delta with a full DAG update and layout clone before presentation swamps the main thread.

---

## 3. Acceleration Roadmap

### Strategy A: GPU Layer Translation / Sub-Scene Caching (The True GPU Solution) [IMPLEMENTED]
- **Status:** **Completed.**
- **Architecture:**
  1. **DAG Layout Isolation:** In `components/ScrollView.dt`, `\Children` positions `content_top = y + padding` and `content_left = x + padding` are decoupled from `scroll_y` and `scroll_x`. When scrolling, the DAG completely skips child position re-evaluations and downstream layout invalidations.
  2. **Clip Layer Scroll Ports:** `\Clip` now accepts `scroll_x` and `scroll_y` ports, carrying active scroll translations in `ResolvedLayout`.
  3. **Sub-Scene Recording & Caching (`ClipSceneCache`):** In `src/render/scene.rs`, children within a clip are rendered into a sub-scene at static layout coordinates. The sub-scene is cached in `ClipSceneCache`.
  4. **Hardware GPU Translation:** During scene construction:
     ```rust
     parent_scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &clip_box);
     parent_scene.append(&cached_clip.scene, Some(Affine::translate((-scroll_x, -scroll_y))));
     parent_scene.pop_layer();
     ```
     On subsequent scroll events, `compute_clip_content_hash` confirms static child invariance (100% cache hit). The entire child sub-tree is skipped in $O(1)$ CPU time, and Vello applies the translation matrix directly in GPU rasterization shaders.
  5. **Hit-Testing Coordinates:** In `src/compiler/layout.rs`, `ResolvedLayout::clip_scroll_offset` sums cumulative clip scroll translations, and `hit_test` offsets cursor coordinates `test_point = Point::new(point.x + sx, point.y + sy)` against static primitive rects and fragments, preserving sub-pixel hit detection.

### Strategy B: Text Layout Caching & Memoization (Option 3: 1-Element `vello::Scene` Cache) [IMPLEMENTED]
- **Status:** **Completed.**
- **Design:** Implemented with `TextSceneCache` in `src/render/scene.rs`, maintaining a 1-element cache per text `NodeId`.
- **Inputs compared (`TextRenderKey`):** `text`, `width` (wrap boundary), `font_size`, `font_weight`, `font_family`, `color`, `align`, `spans` (rich text / link hover colors).
- **Position Invariance:** `node.rect.x` and `node.rect.y` are excluded from the cache key.
- **Cache Hit:** During scrolling, text inputs are 100% identical. The CPU skips Parley font shaping, OpenType analysis, line breaking (`break_all_lines`), and alignment entirely. It executes a single fast GPU scene command:
  ```rust
  scene.append(&cached.scene, Some(Affine::translate((node.rect.x, node.rect.y))));
  ```
- **Cache Miss:** First frame or when text/width/hover style changes. Renders into a local `vello::Scene` at `(0, 0)` and records the new entry.

### Strategy C: Viewport Frustum Culling
- In `build_scene`, test each node against the container's clip bounds:
  ```rust
  if node.rect.y + node.rect.height < clip_rect.y || node.rect.y > clip_rect.y + clip_rect.height {
      continue; // Skip offscreen elements
  }
  ```
- Reduces CPU formatting and Vello command submission from $O(N_{\text{total}})$ to $O(N_{\text{visible}})$.

### Strategy D: Event Coalescing and Zero-Copy State Dispatch [IMPLEMENTED]
- **Status:** **Completed.**
- **Zero-Copy Layout Borrowing:** Removed all `self.layout = compiled.layout().clone()` invocations across the viewer, scene construction, and test suites using Rust disjoint field borrowing (`current_layout(&self.compiled, &self.static_layout)`), eliminating redundant clone allocations on every single cursor move, click, and scroll tick.
- **Event Coalescing:**
  1. **Pending Scroll Accumulation (`PendingScroll`):** In `src/render/viewer.rs`, `ViewerApp` maintains `pending_scroll: Option<PendingScroll>` tracking accumulated `(delta_x, delta_y)` offsets.
  2. **Deferred Dispatch (`queue_scroll`):** High-frequency `WindowEvent::MouseWheel` events (emitted at 120Hz+ by macOS precision trackpads) are accumulated linearly without triggering hit-testing or DAG evaluations during input pumping.Redraw is scheduled via `window.request_redraw()`.
  3. **Synchronous Turn Flushing (`flush_pending_scroll`):** Before rendering a frame in `WindowEvent::RedrawRequested`, `about_to_wait`, or `render_frame()`, any pending scroll is flushed and dispatched as a single aggregated `EventKind::Scroll { delta_x, delta_y }`.
  4. **Causal Ordering Safety:** `flush_pending_scroll()` is also invoked prior to handling spatial input events (`CursorMoved`, `MouseInput`, `KeyboardInput`, `Resized`), guaranteeing that hover transitions, link clicks, and focus movements always hit-test against the up-to-date scrolled layout.
  5. **Result:** Completely eliminates CPU main-thread queue saturation on fast trackpad flicks while preserving 100% smooth momentum, sub-pixel precision, and 2D diagonal scrolling.

