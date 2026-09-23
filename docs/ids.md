# DirectedType (DTML) Specification: Component Identity & Node Addressing

In DirectedType (DTML), nodes and components participate in both an authoring hierarchy and a mathematical Directed Acyclic Graph (DAG). To support developer tooling, inspector selection, scripting, and cross-node algebraic referencing, DirectedType defines a dual identification model: **Authored Identifiers** and **Runtime Node Handles**.

---

## 1. Authored Identifiers (`id`)

Any element or component can declare an optional `id` port in its instantiation or parameter signature:

```dtml
\Flow(id: main_flow, gap: 20) {
    \ScrollView(id: viewport, height: 200) {
        \Rect(id: accent_bar, width: 500, height: 44, color: #6366f1)
        \Text(id: label) { "Clipped Content" }
    }
}
```

### Semantics & Guarantees
1. **Document-Wide Uniqueness:** An authored `id` is unique across the document namespace. Defining duplicate IDs in the same document produces a compile-time or parse-time diagnostic error:
   ```text
   CompileError: Duplicate node identifier 'viewport' (first defined at line 2:17)
   ```
2. **Direct Graph Referencing:** In addition to standard structural navigation (`parent`, `prev`), authored IDs can be referenced directly in algebraic layout equations across the document:
   ```dtml
   \Rect(
       x: viewport.right + 20,
       y: viewport.top,
       width: 100,
       height: viewport.height
   )
   ```
   During expansion, `viewport.right` resolves to the concrete output port of the node bound to `viewport`.
3. **DOM Querying:** Scripts and developer tools can look up nodes in $O(1)$ time via `dom.get_element_by_id("viewport")`.

---

## 2. Runtime Node Handles (`NodeId` / `NodeHandle`)

Not every element will have an explicit authored `id`. For internal execution, DevTools inspection, and dynamic manipulation, every node receives a concrete runtime identity.

### `NodeId` (Internal Compiler Identity)
* A zero-based index assigned during template expansion: `NodeId(usize)`.
* Generates canonical variable prefixes in the layout graph: `__node_0`, `__node_1`, etc.
* `NodeId::WINDOW` (`usize::MAX` / `__window`) serves as the root container sentinel.

### `NodeHandle` (Public API Identity)
* A stable opaque token handed to external consumers (e.g. DevTools, host embedding APIs, WASM guest scripts).
* Encapsulates the node's pointer or generation index so mutations remain memory-safe even as the tree re-expands.

---

## 3. Spatial Hit-Testing & Selection

For interactive tools (the Dev Console inspector, mouse events, and click targets), nodes are identified by their resolved spatial geometry:

```rust
dom.hit_test(x: f64, y: f64) -> Option<NodeHandle>
```

### Hit-Testing Algorithm
1. **Painter's Order (Top-to-Bottom):** Evaluates nodes in reverse render order (`z` descending, followed by declaration order descending).
2. **Bounding Box Intersection:** Checks if the point `(x, y)` lies within the node's resolved `Rect { x, y, width, height }`.
3. **Clip Mask Verification:** If the candidate node has an active clip context (`clip: Some(clip_id)`), verifies that `(x, y)` is also inside the bounding geometry of every ancestor clip node up to `window.clip`.
4. **First Hit Wins:** The uppermost unclipped visual element at `(x, y)` is returned as the targeted `NodeHandle`.

---

## 4. Querying & Traversal

The DOM API provides standard structural query methods using `NodeHandle`:

```rust
// Lookup
dom.get_element_by_id(id: &str) -> Option<NodeHandle>;

// Hierarchy traversal
dom.parent(node: NodeHandle) -> Option<NodeHandle>;
dom.children(node: NodeHandle) -> &[NodeHandle];
dom.first_child(node: NodeHandle) -> Option<NodeHandle>;
dom.last_child(node: NodeHandle) -> Option<NodeHandle>;
dom.prev_sibling(node: NodeHandle) -> Option<NodeHandle>;
dom.next_sibling(node: NodeHandle) -> Option<NodeHandle>;
```
