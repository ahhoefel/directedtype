# DirectedType DevTools: DOM Inspection Tool Specification

## 1. Overview & Dual-Viewport Split

The DirectedType Inspector adopts a dual-pane layout inspired by Chrome DevTools, designed specifically for late-bound algebraic layout trees:

```
┌──────────────────────────────────────┬───────────────────────────────────────┐
│ Viewport A: Document Surface (55%)   │ Viewport B: DirectedType Inspector    │
│                                      │                                       │
│  ┌────────────────────────────────┐  │  [↖] [Search DOM...] [Elements] [DAG] │
│  │                                │  │  ───────────────────────────────────  │
│  │    Active Inspected Element    │  │  ▼ \Flow                              │
│  │   ┌────────────────────────┐   │  │    ▼ \ScrollView#main                 │
│  │   │ Rect#card [380 × 180]  │   │  │      ▶ \Rect#card  ◄── [Selected]    │
│  │   │ (Cyan overlay + badge) │   │  │          \Text "Hello"                │
│  │   └────────────────────────┘   │  │                                       │
│  │                                │  │  ───────────────────────────────────  │
│  │                                │  │  Computed Box Model  │ Declared Ports │
│  │                                │  │  ┌─────────────────┐ │ width: p.w - 64│
│  │                                │  │  │  x: 48   y: 120 │ │  ↳ = 380px     │
│  │                                │  │  │  w: 380  h: 180 │ │ y: prev.b + 16 │
│  │                                │  │  └─────────────────┘ │  ↳ = 120px     │
│  └────────────────────────────────┘  │                                       │
└──────────────────────────────────────┴───────────────────────────────────────┘
```

---

## 2. Key Panels & Features

### A. Viewport A: Live Document Surface & Spatial Inspection
1. **Interactive Selection Cursor (`[↖]`):** Clicking the inspect tool allows hovering over visual elements on screen. As the cursor moves, `dom.hit_test(Point)` queries the leaf element and its ancestor bubble path.
2. **Bounding Box Overlay:** A translucent cyan overlay (`#38bdf820` with a `1.5px` border) highlights the active node.
3. **Floating Badge:** Displays the node's tag, ID (if authored), and resolved dimensions: `Rect#card [380 × 180]`.
4. **Clip Context Visualization:** If the element is clipped, dashed purple guidelines indicate the bounding rectangle of its active clip node.

### B. Viewport B (Top): Component DOM Tree View
1. **Expandable Hierarchy:** Displays the authored Component DOM nodes with toggle chevrons (`▼` / `▶`).
2. **Component vs. Primitive Distinctions:**
   * High-level components (`\ScrollView`, `\Flow`) are distinguished from primitives (`\Rect`, `\Text`).
   * Component encapsulation is preserved: the developer can inspect the component as an authoring unit or expand it to view internal primitive nodes.
3. **Bi-directional Selection Sync:**
   * Selecting a node in the tree highlights it on the page surface.
   * Clicking an element on the page surface jumps to and selects its node in the tree.

### C. Viewport B (Bottom Left): Computed Box Model
* **Resolved Spatial Values:** Displays evaluated physical pixel boundaries `{ x, y, width, height }`.
* **Z-Index & Stacking:** Displays the computed `z` ordering.
* **Clip Node:** Displays the active clip container (e.g. `clip: NodeHandle(1)` or `clip: window`).

### D. Viewport B (Bottom Right): Declared Port Equations (DirectedType Specialty)
Unlike standard web DevTools where styles are static CSS properties, DirectedType ports are **algebraic formulas**:
* **Authored Equation:** Shows the formula authored in markup:
  `width: parent.width - 64`
* **Evaluated Value:** Shows the solved topological result:
  `↳ 380.0px`
* **Live In-Place Editing:** Clicking an equation opens an inline input to edit formulas live (e.g. changing `width: parent.width - 64` to `width: parent.width / 2`), triggering a live `dom.set_port` and batched `commit()`.
* **Dependency Highlighting:** Hovering over an equation symbol (like `parent.width` or `prev.bottom`) highlights the referenced node in the viewport.

---

## 3. The Pure DTML Architecture

As specified in [dom.md](file:///Users/hoefel/dev/directedtype/docs/dom.md#L76-L100), the DevTools inspector itself can be **authored entirely in DTML**!
* Both viewports run in the same native OS window.
* Viewport A renders the user document.
* Viewport B renders the DevTools UI (authored in DTML using standard `\Flow`, `\ScrollView`, `\Text`, and `\Rect` components).
* Viewport B holds a `Dom` handle to Viewport A, executing queries and mutations through the `Dom` API.
