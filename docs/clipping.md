Here is the architectural design specification for **Chained Clipping Contexts** and **Clip Edges** in the DirectedType engine.

This document defines how the compiler translates ergonomic lexical markup into a strict, flat mathematical DAG, and how the renderer translates that DAG into optimal GPU instructions.

---

# DirectedType Specification: Clipping & Environmental Clip Contexts

## 1. The Core Philosophy: Lexical vs. Structural

In HTML/CSS, clipping is structural: if you are physically inside a `<div>` with `overflow: hidden`, you are clipped.
In DirectedType, clipping is a **hardware state**. The DAG completely decouples the *layout hierarchy* from the *clipping hierarchy*.

* **Clip Nodes Form a Directed Chain:** A `\Clip` node does not wrap or contain children. Instead, it points to a spatial geometry (`box: \Box(...)`), and optionally points to an upstream clip (`up: self.clip` or `up: window.clip`).
* **Environmental Inheritance via Parent Ports:** Every visual node (`\Rect`, text nodes) has a universal `clip` port defaulting to `parent.clip` (or `window.clip` at root).
* **Escape Hatches via Member Access:** Elements can step up the clip chain via `clip: clip.up` (or `self.clip.up`), or unclip entirely to the root viewport via `clip: window.clip`.
* **Zero Magic / Hard Wires:** By the time the graph evaluator runs, all clip contexts are explicit, strongly typed `NodeId` references in the DAG.

---

## 2. Geometry & Clip Primitives: `\Rect`, `\Box`, and `\Clip`

### `\Rect` (Visual Paint Primitive)
`\Rect` is a visual drawing element. It supports spatial layout (`x`, `y`, `width`, `height`, `z`), styling (`color`, `bg_color`, `border_width`, `border_color`, `radius`), and visual clipping (`clip: ...`).

### `\Box` (Mathematical Spatial Primitive)
`\Box` is a pure mathematical, non-drawing spatial rectangle (`x`, `y`, `width`, `height`, `radius`). It emits **zero draw commands** and is used for non-visual layout regions, bounding calculations, and clip boundaries.

### `\Clip` (Clip Context Primitive)
`\Clip` is a non-drawing node that establishes a clipping context. It takes:
* `box`: (Required) The bounding spatial geometry (`\Box` or `\Rect`). Can be passed inline or via a `let` binding.
* `up`: (Optional) The upstream clip node. Defaults to `parent.clip` (or `window.clip` at the root container).

```text
\Component ScrollView(x: Number, y: Number, width: Number, height: Number) {
  // 1. Establish the clip context with bounding Box
  let clip = \Clip(up: self.clip, box: \Box(x: self.left, y: self.top, width: self.width, height: self.height))

  // 2. Yield children with ambient clip
  \Children {
    clip: clip,
    x: parent.left,
    y: prev ? prev.bottom : parent.top
  }
}
```

---

## 3. Escaping the Chain & Multi-Level Clipping

Because `clip` is a standard port on every visual primitive:

### Stepping Up One Level (`clip.up`)
A tooltip or overflow element inside a nested clip can step up to the outer container's clip context:

```text
\Component Footnote(text: String) {
  \Text { text }

  // Overwrites ambient clip to step one level up the chain
  \Rect(clip: self.clip.up, z: self.z + 100, width: 200, height: 80)
}
```

### Unclipped Root Viewport (`window.clip`)
To escape all clips and render directly in window coordinate space (e.g. for a global modal or full-screen overlay):

```text
\Rect(clip: window.clip, z: 9999, width: 400, height: 300, color: #ffffff)
```

---

## 4. Reserved Port Validation

`parent` is strictly reserved as an implicit, immutable tree hierarchy navigation keyword. Any attempt to author `parent: ...` on an element, in a component parameter list, or in a `\Children` directive will produce a compile error:

```text
CompileError::ReservedPort { node: "Rect", port: "parent", span: ... }
```

---

## 5. Renderer Execution (Vello GPU Layer Management)

During scene compilation (`build_scene` in `src/render/scene.rs`):

1. **Painter's Order Traversal:** Non-paint primitives (`\Box`, `\Clip`) are skipped; only visual elements (`\Rect`, text) are rendered.
2. **Lowest Common Ancestor (LCA) Stack Management:**
   * The renderer tracks the GPU's `active_clip_stack: Vec<NodeId>`.
   * For each node, it computes the target clip chain from root to leaf.
   * Layers beyond the common prefix (LCA) are popped via `scene.pop_layer()`.
   * New layers are pushed via `scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &shape)`.
   * The shape is automatically instantiated as a `vello::kurbo::Rect` or `vello::kurbo::RoundedRect` based on the `\Box` coordinates and `radius`.
3. **Clean Teardown:** When all visual elements have been drawn, any remaining open layers on `active_clip_stack` are popped to return the GPU stack to neutral.