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

### Strategy A: GPU Layer Translation / Sub-Scene Caching (The True GPU Solution)
Modern UI engines (Chromium, WebKit) achieve 120 FPS scrolling by keeping scrollable content in a composited GPU layer that translates via hardware matrix transforms without touching layout:
- **Vello Nested Scenes**: Vello allows scenes to be nested and transformed:
  ```rust
  // Child content scene recorded once at local (0, 0):
  scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &viewport_box);
  scene.append(&cached_content_scene, Some(Affine::translate((0.0, -scroll_y))));
  scene.pop_layer();
  ```
- **DAG Isolation**: Children inside `\ScrollView` maintain static local coordinates `(x, y)` in the DAG. Scrolling only mutates the GPU transform `Affine::translate(0.0, -scroll_y)` and the dynamic scrollbar thumb.
- **Hit-testing**: Spatial queries map cursor coordinates into the local container space by adding `(0.0, scroll_y)`.

### Strategy B: Text Layout Caching & Memoization (Immediate High-Impact CPU Win)
- Text line breaking is by far the most CPU-intensive step in `build_scene`.
- Cache the computed text layout (glyph runs, advances, and line heights). During scrolling, when only `y` changes, re-use the cached glyph runs and simply offset their draw transform by the new position.
- Can be implemented with a lightweight 1-item cache (previous call inputs/results per node or text instance).

### Strategy C: Viewport Frustum Culling
- In `build_scene`, test each node against the container's clip bounds:
  ```rust
  if node.rect.y + node.rect.height < clip_rect.y || node.rect.y > clip_rect.y + clip_rect.height {
      continue; // Skip offscreen elements
  }
  ```
- Reduces CPU formatting and Vello command submission from $O(N_{\text{total}})$ to $O(N_{\text{visible}})$.

### Strategy D: Event Coalescing and Zero-Copy State Dispatch
- Coalesce high-frequency `WindowEvent::MouseWheel` events during a frame turn, executing DAG evaluation once per VSync.
- Replace `self.layout = compiled.layout().clone()` with borrowed or shared references (`Arc<ResolvedLayout>`).
