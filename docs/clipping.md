Here is the architectural design specification for implementing **Chained Clipping Contexts** and **Environmental Edges** in the Directed Type (DTML) engine.

This document defines how the parser translates ergonomic, lexically nested markup into a strict, flat mathematical DAG, and how the renderer translates that DAG into optimal GPU instructions.

---

# DTML Specification: Clipping & Environmental Edges

## 1. The Core Philosophy: Lexical vs. Structural

In HTML/CSS, clipping is structural: if you are physically inside a `<div>` with `overflow: hidden`, you are clipped.
In DTML, clipping is a **hardware state**. The DAG completely decouples the *layout hierarchy* from the *clipping hierarchy*.

* **Clip Contexts are a Linked List:** A clip does not contain children; it points to a geometry, and optionally points to a parent clip.
* **Environmental Edges are Desugared:** To prevent designers from manually wiring `clip_context` on every single node, the parser uses lexical scoping to *implicitly author* explicit DAG edges. By the time the graph evaluator runs, there are no implicit properties—only hard wires.

---

## 2. The `\ClipContext` Primitive

The engine introduces a mathematical primitive for defining a clipping boundary. It does not draw pixels and it does not affect layout geometry.

**Signature:**
`\ClipContext(id: String, geometry: Shape, parent_clip: ClipEdge?)`

When instantiated inside a component, it registers a node in the Clip Tree. If it is instantiated lexically inside another active clip, the parser automatically wires its `parent_clip` port to the ambient clip, creating an intersection (a linked stack).

```text
\Component ScrollView(x: Number, y: Number, width: Number, height: Number) {
  
  // 1. Define the clipping geometry
  let mask = \Rect(x: x, y: y, width: width, height: height);
  
  // 2. Establish the context. 
  // (The parser auto-wires `parent_clip` to whatever clip the ScrollView sits inside).
  \ClipContext(id: "scroll_clip", geometry: mask) {
    
    // 3. Yield the children.
    \Children {
      x: parent.left,
      y: prev ? prev.bottom : parent.top
    }
  }
}

```

---

## 3. Environmental Edges: The Parser's Job

The magic of DTML's ergonomics happens entirely in the **Parse Phase**. The parser maintains an *Environmental Scope Stack* as it reads the text.

### The Auto-Wiring Algorithm

When the parser encounters a visual node (like `\Paragraph` or `\Button`), it performs the following steps:

1. Check the explicitly written ports. Did the designer explicitly wire `clip_context`?
2. If yes, wire that exact edge.
3. If no, look at the top of the parser's current Environmental Scope Stack. Draw a hard DAG edge from the active `\ClipContext` to the node's `clip_context` port.

### Example: What the Human Writes vs. What the DAG Sees

**Authored Markup:**

```text
\Modal {                     // Implicitly creates ClipContext A
  \ScrollView {              // Implicitly creates ClipContext B (parent: A)
    \Paragraph { "Hello" }   // Inherits Clip B
  }
}

```

**Desugared AST (The true mathematical graph):**

```text
Clip_A = \ClipContext(geometry: ModalMask, parent_clip: none)
Clip_B = \ClipContext(geometry: ScrollMask, parent_clip: Clip_A)

Paragraph_1 = \Text("Hello")
Paragraph_1.clip_context = Clip_B

```

*Note: The structural nesting is entirely gone. The DAG is flat, connected only by explicit property edges.*

---

## 4. Escaping the Chain (Collision Resolution)

Because `clip_context` is just a standard input port on every visual primitive, escaping a clipping boundary is mathematically trivial. You simply overwrite the ambient edge.

To prevent confusion between the *Layout Parent* and the *Clip Parent*, DTML uses the `inherited` keyword to refer to environmental edges passed down by the parser.

### Scenario: The Footnote Escape Hatch

A footnote inside a `ScrollView` inside a `Modal` needs to escape the `ScrollView`'s clipping box so its tooltip can overflow, but it *must still be clipped by the Modal's rounded corners*.

```text
\Component Footnote(text: String, body: Node) {
  
  // The marker implicitly accepts the ambient edge (Clip B: ScrollView)
  \Text { text }
  
  // The body explicitly overwrites the ambient edge.
  // It steps one level up the clip chain, pointing directly to Clip A (Modal).
  \Block(clip_context: inherited.clip_context.parent, z: inherited.z + 100) { 
    body 
  }
}

```

### Absolute Escape

If a dropdown menu needs to escape *everything* and render directly on the root glass of the screen, it simply nullifies the port:

```text
\Block(clip_context: none, z: 9999) { ... }

```

The parser sees the explicit `none` and skips the auto-wiring step.

---

## 5. Renderer Handoff (The State Machine)

Once Kahn's Algorithm has resolved all spatial math (X, Y, Width, Height) for the DAG, the backend Rust engine collects a completely flat array of visual primitives.

Each primitive carries its exact coordinates, its Z-index, and a pointer to a specific `ClipContext`.

```rust
// Flat evaluation output
[
  Primitive { type: Text("Marker"), z: 10, clip: Clip_B },
  Primitive { type: Text("Normal"), z: 10, clip: Clip_B },
  Primitive { type: Rect(Body),     z: 110, clip: Clip_A } // Escaped!
]

```

### The GPU Batching Loop

Modern hardware rendering pipelines (WGPU, Skia) rely on Stencil Buffers or scissor rects to clip, which are managed via `PushClip` and `PopClip` commands.

To guarantee zero render-thrashing, the Rust backend executes this pipeline:

1. **Topological Sort:** Sort the flat array primarily by `z` index (Painter's Algorithm).
2. **Clip Batching:** Sub-sort elements sharing the exact same `z` index by their `clip_context` ID.
3. **Draw Loop:**
* Compare the current item's `clip_context` against the GPU's active clip state.
* If the target clip is a parent of the active clip, emit `PopClip` until you reach it.
* If the target clip is a child, emit `PushClip` until you reach it.
* Draw the primitive.



Because the Clip Tree is a linked list, the engine perfectly translates `inherited.clip_context.parent` into a single, highly optimized `PopClip` hardware instruction.