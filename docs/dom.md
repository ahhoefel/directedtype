# DirectedType (DTML) Specification: DOM Manipulation & Runtime Mutation Model

In DirectedType, layout is a compiled Directed Acyclic Graph (DAG) of late-bound algebraic equations. To support dynamic applications, scripting environments (such as WASM guest modules), and self-hosted developer tools, DirectedType exposes a stateful **DOM Manipulation Interface** (`DocumentController` / `Dom`).

---

## 1. Operating Abstraction Level: Component DOM

DirectedType maintains a strict separation between two layers of the document:
1. **The Component DOM (Authoring Layer):** High-level component trees (`\ScrollView`, `\Flow`) and consumer child nodes.
2. **The Expanded Layout DAG (Engine Layer):** The flattened collection of visual and spatial primitives (`\Rect`, `\Clip`, `\Box`, `\Text`) with resolved topological edges.

### Why Mutations Target the Component DOM
When a script or developer tool mutates the tree:
```rust
dom.append_child(scroll_view_handle, new_paragraph_node);
```
The mutation operates at the **Component DOM layer**. This ensures that:
* The container's internal `\Children` directive rules (`x: parent.left + 24`, `y: prev ? prev.bottom + 12 : parent.top + 24`, `clip: clip`) automatically bind to the newly added child.
* Recurrence relations (`prev`) remain mathematically consistent.
* Component encapsulation is preserved; the caller does not need to know or manage internal primitives (`\Clip`, `\Box`, background `\Rect`) that implement the component.

---

## 2. Core Operations

### A. Element Creation & Parsing
* `create_element(tag: &str, ports: Vec<(String, Expr)>) -> NodeHandle`: Instantiates an unattached node.
* `create_text(text: &str, ports: Vec<(String, Expr)>) -> NodeHandle`: Instantiates an unattached text node.
* `parse_fragment(dtml_string: &str) -> NodeHandle`: Parses a DTML snippet into an unattached element subtree.

### B. Tree Hierarchy Mutations
* `append_child(parent: NodeHandle, child: NodeHandle)`: Appends `child` as the last child in `parent`'s content slot.
* `insert_before(parent: NodeHandle, before: NodeHandle, child: NodeHandle)`: Inserts `child` immediately before `before`.
* `remove_child(parent: NodeHandle, child: NodeHandle)`: Detaches `child` from `parent`.
* `replace_child(parent: NodeHandle, old_child: NodeHandle, new_child: NodeHandle)`: Replaces `old_child` with `new_child`.

### C. Port & Text Mutations
* `set_port(node: NodeHandle, port_name: &str, expr: Expr)`: Sets or overrides a port equation (e.g. `width: 320` or `y: prev.bottom + 12`).
* `remove_port(node: NodeHandle, port_name: &str)`: Removes an explicit port, falling back to ambient or component defaults.
* `set_text(node: NodeHandle, text: &str)`: Updates the text content of a `\Text` node.

### D. Reflection & Inspection (Reading the DAG)
* `computed_rect(node: NodeHandle) -> Rect`: Returns the resolved bounding box `{ x, y, width, height }`.
* `computed_value(node: NodeHandle, port: &str) -> Option<&Value>`: Returns the evaluated value of any public port (e.g., `color`, `z`, `clip`).
* `clip_context(node: NodeHandle) -> Option<NodeHandle>`: Returns the active clip node bounding this element.
* `declared_ports(node: NodeHandle) -> HashMap<String, Expr>`: Returns the authored port equations.

---

## 3. Transaction Model & Batched Commit

To prevent layout thrashing and maintain 120 FPS performance during high-frequency updates, mutations follow a **Transaction / Commit** lifecycle:

```
[Mutations: set_port, append_child, ...]
                   │
                   ▼
       dom.commit() / Frame End
                   │
                   ▼
┌───────────────────────────────────────┐
│ 1. Incremental AST / Tree Expansion   │
│ 2. Variable Graph Edge Updates        │
│ 3. Topological Sort (Kahn's / DFS)    │
│ 4. Algebraic Math Evaluation          │
│ 5. GPU Scene Generation (Vello)       │
└───────────────────────────────────────┘
```

1. **Mutation Queue:** Programmatic changes mutate the AST data structures and mark affected nodes dirty without immediately invoking the graph compiler.
2. **Commit Phase:** At the end of an event turn or when `dom.commit()` is called, the compiler re-expands dirty subtrees, updates the variable dependency graph, evaluates the topological schedule, and presents the new layout.

---

## 4. The Dual Viewport Architecture (Option B)

For host viewer features like a self-hosted **Dev Console**, the document DAG and the Dev Console DAG remain strictly isolated:

```
┌────────────────────────────────────────────────────────┐
│ Native OS Window (Root Compositor)                     │
│                                                        │
│  ┌──────────────────────────────────────────────────┐  │
│  │ Viewport A: User Document DAG                    │  │
│  │ (Sandboxed user markup; cycle errors contained)  │  │
│  └──────────────────────────────────────────────────┘  │
│                                                        │
│  ┌──────────────────────────────────────────────────┐  │
│  │ Viewport B: Dev Console DAG (Written in DTML)    │  │
│  │ (Has read/write DOM handle to Viewport A)        │  │
│  └──────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────┘
```

### Architectural Benefits
1. **Error Isolation:** If the user document enters a cyclic dependency error (e.g., `__node_9.width -> __node_9.width`), the Dev Console DAG remains fully operational to render error diagnostics and inspector states.
2. **Pure DTML DevTools:** The Dev Console is authored entirely in DTML, leveraging the engine's native text shaping, rounded rectangles, clipping, and responsive layout.
3. **Clean GPU Composition:** Both viewports are composited onto the same Vello GPU surface, rendering the user document first and the developer tools overlay second.
