Here is a comprehensive architectural and semantic specification document. You can save this as `SPEC.md` or `ARCHITECTURE.md` and feed it directly into Antigravity (or any AI coding agent/workspace) to give it the exact constraints, mental models, and syntax required to start building the parser and graph compiler.

This document is written explicitly to prevent an AI from falling back into HTML/CSS or React DOM mentalities.

---

```markdown
# DAG-UI: A Pure Functional Reactive Layout Engine
**Version:** Draft 1.0
**Target Backend:** Rust (Graph Compiler), Vello (GPU Compositor), Parley (Text Shaping)

## 1. Core Philosophy
DAG-UI replaces the HTML/CSS document-flow model with a strict **Directed Acyclic Graph (DAG)** of layout variables. 
* **Layout is Math:** X, Y, Width, Height, and Z-index are not implicit rules; they are explicit, late-bound algebraic equations.
* **Layout is not Painting:** Grouping elements does not draw pixels. Visuals (like backgrounds) are strictly separate from layout boundaries.
* **Two-Phase Execution:** 
    1. **Phase 1 (Template/Scripting):** A standard scripting language constructs the AST and wires the declarative equations.
    2. **Phase 2 (Graph Evaluation):** A Rust engine topologically sorts the equations (Kahn's Algorithm), resolves the math, and pushes a flat list of draw commands to the GPU.

## 2. Execution Model & The Graph
There is no "DOM Tree" at runtime. The runtime is a flat graph of variables.

### Late-Bound Edges
The `=` operator does **not** perform imperative assignment. It establishes a directed edge in the graph. 
`width = parent.width / 2` means: "Draw an edge from `parent.width` to this node's `width` port." The math is evaluated only after the entire AST is parsed and topologically sorted.

### Cycle Detection
Because layout relies on a topological sort, bidirectional dependencies (e.g., Parent width relies on Child width AND Child width relies on Parent width) will fail to sort. The compiler must throw a fatal error on cycle detection, enforcing strict one-way data flow (Intrinsic vs. Extrinsic sizing).

## 3. Language Semantics

### 3.1 Components & Implicit Scope
A `\Component` is a black box. It acts as a pure function and an implicit mathematical namespace.
* Every component implicitly inherits the **Base Spatial Trait** (`x`, `y`, `width`, `height`, `z`).
* These spatial variables are natively available in the component's internal scope.
* Component ports are strictly typed.

```text
\Component ShadedBox(bg_color: Color, width: max(children.width) + 32) {
  
  // Paint Primitive: Uses the component's implicit spatial variables
  \Rect(
    x: x, 
    y: y, 
    width: width, 
    height: height,
    color: bg_color
  )
  
  // Content Integration
  \Children {
    x: x + 16,
    y: y + 16
  }
}

```

### 3.2 The `\Children` Directive

`\Children` is not a node; it is a structural spread operator evaluated at compile time. It takes the nested nodes provided by the consumer and explicitly applies a set of mathematical edges to them.

### 3.3 The `prev` Keyword

To define sequences without procedural loops, DAG-UI relies on recurrence relations. The `prev` keyword allows a node injected via `\Children` to reference the output ports of the node immediately preceding it in the AST array.

* Example: `y: prev ? prev.bottom + gap : top`

### 3.4 Layering (The Painter's Algorithm)

Z-index wars are eliminated by separating grouping from painting.

* **Default:** Execution array order defines layering. Primitives declared later in a component paint on top of earlier ones.
* **Override:** Any node can explicitly declare a `z` edge (e.g., `z: parent.z + 100`) to mathematically break out of the Painter's Algorithm.

## 4. Syntax & Grammar Reference

**Instantiation:** Nodes begin with `\` followed by their Component or Primitive name.
**Input Ports (Edges):** Defined in parentheses `( )`. Expressions here are algebraic edges, not strings (unless quoted).
**Content Slot:** Defined in braces `{ }`.

```text
// Usage Example
\Component Flow(gap: Number = 16) {
  \Children {
    x: parent.left,
    y: prev ? prev.bottom + gap : parent.top
  }
}

// Applying the Flow
\Flow(gap: 24) {
  \Header(size: 32) { "Welcome to DAG-UI" }
  \Paragraph { "This layout is mathematically provable." }
}

```

## 5. Primitive Reference (The Engine ABI)

The language eventually compiles down to a flat list of these primitives for the Rust/Vello engine:

* `\Block`: A purely spatial construct. Evaluates layout math but passes zero draw commands to Vello.
* `\Rect`, `\Circle`, `\Path`: Paint primitives. They take spatial coordinates and color/stroke data and output Vello paths.
* `\Text`: The bridge to Parley.
* **Input:** `string`, `max_width`, `font`, `size`.
* **Output:** Passes data to Parley's line-breaker. Parley yields the `height` and exact `glyph` coordinates back to the graph.



## 6. Collision Resolution (Precedence)

If multiple edges attempt to drive the same input port, the compiler resolves them statically:

1. **Explicit Instance Wiring Wins:** `\Paragraph(x: 100)` overrides ambient layouts.
2. **Ambient Container Wiring:** `\Children { x: parent.left }` applies if the instance did not explicitly set `x`.
3. **Forced Container Wiring:** Using `x!: parent.left` in a `\Children` block throws a compiler error if a child attempts to override it.

```

***

### How to direct your agent

When you pass this into your workspace/agent, give it a prompt like this to structure its work:

> *"I am providing a specification document for a new functional reactive UI language called DAG-UI. We are building the engine for this from scratch in Rust.*
> 
> *Your first task is to write the parser using `nom` or `logos`/`chumsky`. The parser must read this syntax and output an un-evaluated AST. Do not write any HTML or CSS generation code. Treat the `=` in port definitions as late-bound algebraic edges, not immediate variable assignments. Provide the AST Rust structs first so I can review them."* 

This will force the agent to skip all the traditional web-framework boilerplate and start directly on your mathematical graph architecture.

```