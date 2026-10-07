# DirectedType (DTML) Specification: In-Page Anchors, Hierarchical Scopes, & Focus Management

**Status:** Proposed Architecture  
**Topic:** Anchors, Hierarchical Scopes, In-Page Navigation, & Focus Management  
**Target Backend:** Rust Compiler, Component DOM, Parley / Vello Viewer  

---

## 1. Executive Summary & Problem Statement

DirectedType recently introduced inline hypertext links (`\Link(url: "https://...")`). While external URLs dispatch to the host operating system, in-page navigation (deep-linking, table of contents sidebars, footnotes, and cross-references) requires an **Internal Navigation and Anchor System**.

### The Failure Modes of Flat Global Anchor Registries

Standard HTML models anchors as a flat global namespace (`<a id="section-1">` or `<a name="section-1">`). While simple in static HTML pages, a flat global registry breaks down in modular component architectures:

1. **Component Encapsulation Leaks:** If internal component IDs are exposed globally as anchors, consumers must know the private naming of internal sub-elements, making refactoring brittle.
2. **Multi-Pane / Side-by-Side Collisions:** In split-screen editors, side-by-side diff viewers, or multi-document comparison panes, two identical documents will have identical anchor names (e.g. `#overview`, `#setup`). A flat global registry produces immediate collision errors, or causes a link in Pane 1 to unexpectedly scroll Pane 2.
3. **Confused Scroll Domains:** An anchor jump is not an abstract coordinate jump; it is an instruction to scroll a specific viewport. In a multi-pane layout, each pane has its own independent `\ScrollView`.
4. **Lifecycle & Pruning Burden:** A global registry requires tedious delta-tracking to avoid stale/dangling anchors when subtrees unmount or mutate.

### Core Architectural Principles

To resolve these issues while maintaining DirectedType's mathematical clarity:

1. **Separation of Scoped IDs vs. Semantic Anchors:**
   - **`id` (Local / Component-Scoped):** Used for layout equations (`prev.bottom`), DOM queries, and state dispatch. Cannot be targeted by `#` links.
   - **`\Anchor` (Navigation Target):** Dedicated wrapper or inline point for in-page navigation, deep linking, and focus.
2. **The Wrapper Pattern (`\Anchor("name") { ... }`):**
   - The anchor wraps the target subject. Its bounding box transparently matches the child.
   - Avoids polluting other component signatures with an implicit `anchor:` port.
   - Eliminates layout gap anomalies in flex/stack containers like `\VStack(gap: 16)`.
3. **Hierarchical Scopes (`\AnchorScope("name") { ... }`):**
   - Scopes form isolated navigation realms (e.g., matching a `\ScrollView` or document pane).
   - Anchors are unique *per scope*, eliminating multi-document collisions.
   - Dropping a scope cleanly drops all its nested anchors in $O(1)$ time.
4. **Path-Based Addressing (Unix-Style Absolute vs. Relative):**
   - **Absolute (`#/scope/target`):** Starts with a leading `/` from the root window scope.
   - **Relative (`#target` or `#sub/target`):** No leading `/`. Resolves within the current enclosing scope, bubbling upward.
5. **First-Class `AnchorScope` as a Port:**
   - An `AnchorScope` can be captured into a variable and passed into sibling navigation components (e.g. `\TableOfContents(target: doc_scope)`).
   - Built-in operations like `scope.append("target")` generate canonical absolute URLs (`"#/doc/target"`).

---

## 2. Language Syntax & Authoring

### A. Wrapping Block Elements
`\Anchor` wraps any visual component or container. It inherits the child's spatial dimensions:

```dtml
\VStack(gap: 20) {
    \Anchor("installation") {
        \Heading { Installation Guide }
    }

    \Paragraph {
        To get started with DirectedType, run the compiler suite.
    }

    \Anchor("pricing") {
        \Card(style: card_style) {
            \Heading { Tier Options }
        }
    }
}
```

### B. Inline Text Bookmarks
Inside paragraphs, `\Anchor` with an empty body acts as a zero-width inline bookmark at that character offset:

```dtml
\Text {
    For further context, see our methodology\Anchor("footnote-1") outlined below.
}
```

### C. Defining Hierarchical Scopes (`\AnchorScope`)
Scopes create isolated namespaces, typically aligned with scrollable panes:

```dtml
\HStack(gap: 24) {
    // Pane 1: Document A
    \AnchorScope("doc-left") {
        \ScrollView {
            \Anchor("intro") { \Heading { Document A Overview } }
        }
    }

    // Pane 2: Document B (Identical anchor names do NOT collide!)
    \AnchorScope("doc-right") {
        \ScrollView {
            \Anchor("intro") { \Heading { Document B Overview } }
        }
    }
}
```

### D. Absolute vs. Relative Links

```dtml
// Inside "doc-left":
\Link(url: "#intro"){Jump to local intro} // Relative: resolves to /doc-left/intro and scrolls left pane

// From a shared Header/Toolbar outside both panes:
\Link(url: "#/doc-left/intro"){Left Doc Intro}   // Absolute: scrolls left pane
\Link(url: "#/doc-right/intro"){Right Doc Intro} // Absolute: scrolls right pane
```

### E. Passing `AnchorScope` as a Component Port
A navigation component (e.g. a Table of Contents) can accept a target scope and build links dynamically:

```dtml
\Component TableOfContents(target_scope: AnchorScope) {
    \VStack(gap: 10) {
        \Text(weight: 700) { Table of Contents }
        \Link(url: target_scope.append("intro")){1. Introduction}
        \Link(url: target_scope.append("setup")){2. Setup}
        \Link(url: target_scope.append("pricing")){3. Pricing}
    }
}

// In the parent view:
\Component DocumentView {
    let main_scope = \AnchorScope("main") {
        \ScrollView {
            \Anchor("intro") { \Heading { Introduction } }
            \Anchor("setup") { \Heading { Setup } }
            \Anchor("pricing") { \Heading { Pricing } }
        }
    };

    \HStack {
        \TableOfContents(target_scope: main_scope)
        main_scope
    }
}
```

---

## 3. Data Model & Architecture

### A. AST & Expanded Representation

```rust
// AST representation
pub struct AnchorNode {
    pub name: String,
    pub content: Option<ContentSlot>,
    pub span: Span,
}

pub struct AnchorScopeNode {
    pub name: String,
    pub content: ContentSlot,
    pub span: Span,
}
```

### B. Layout DAG Geometry of `\Anchor`
In `src/compiler/layout.rs`:
- If `\Anchor` wraps a child node, its layout variables are strictly pass-through:
  ```text
  anchor.x = child.x
  anchor.y = child.y
  anchor.width = child.width
  anchor.height = child.height
  ```
- If `\Anchor` has no children (inline text bookmark), Parley shapes it as a zero-width inline span with coordinates `(x, y)` at the glyph cluster boundary.

### C. The Scope Tree Hierarchy
The compiler and runtime maintain a `ScopeTree`:

```rust
#[derive(Debug, Clone)]
pub struct NavigationScope {
    pub id: ScopeId,
    pub name: String,
    pub parent: Option<ScopeId>,
    pub children: HashMap<String, ScopeId>,
    /// Map from anchor slug to target NodeId
    pub anchors: HashMap<String, NodeId>,
    /// The nearest enclosing scroll container for this scope
    pub scroll_container: Option<NodeId>,
}

#[derive(Debug, Clone)]
pub struct ScopeTree {
    pub root: ScopeId,
    pub scopes: HashMap<ScopeId, NavigationScope>,
}
```

### D. Canonical Paths & Operations on `AnchorScope`
Every `AnchorScope` has a canonical path:
- Root scope: `/`
- Scope `"doc-left"` under Root: `/doc-left`
- Scope `"ch1"` under `"book"`: `/book/ch1`

Methods exposed to DTML runtime:
- `scope.path`: Returns `"/doc-left"`
- `scope.name`: Returns `"doc-left"`
- `scope.append(target)`: Formats and returns `"#/doc-left/" + target`

---

## 4. Path Resolution Algorithm

When a link with `url` (e.g. `"#..."`) is activated:

```
                      Is URL an anchor (starts with '#')?
                                    │
                                    ├─── No ───> External URL (open in OS browser)
                                    ▼ Yes
                         Strip '#' -> Path string
                                    │
                     Does path start with leading '/'?
                                    │
                  ┌─────────────────┴─────────────────┐
                  ▼ Yes                               ▼ No
         [ Absolute Path ]                   [ Relative Path ]
  Start lookup at Root Scope           Start lookup at Link's Enclosing Scope
          │                                   │
  Split path segments:                        ├── 1. Check local scope anchors
  "/pane-left/installation"                   │      Found? -> Return target NodeId
          │                                   ├── 2. Check child scopes: "sub/step1"
  Traverse child scopes -> target             │      Found? -> Return target NodeId
          │                                   └── 3. Bubble up to parent scope
  Return target NodeId                               Found? -> Return target NodeId
```

### Ambiguity & Collision Guarantees
- Anchors within the **same scope** must be unique. Duplicate anchors in the same scope generate a compile-time error.
- Anchors across **different scopes** can share names without collision (e.g. `/pane-left/intro` and `/pane-right/intro`).
- Unqualified links (`#intro`) search outward hierarchically (lexical bubbling).

---

## 5. Interaction, Scroll Execution & Focus Management

### A. Scroll-To Execution
Once the target `NodeId` is resolved:
1. **Find Target Geometry:** Look up `target_node.rect` in `ResolvedLayout`.
2. **Identify Scroll Viewport:** Walk up the target's ancestor chain to find the nearest scrollable container (e.g. `\ScrollView` or the root `\Window`).
3. **Compute Target Offset:**
   ```rust
   let target_scroll_y = (target_rect.y - container_rect.y + current_scroll_y).max(0.0);
   ```
4. **Apply Scroll Mutation:**
   - Smoothly animate or directly set `container.scroll_offset = target_scroll_y`.
   - Issue a layout update for the viewport.

### B. Focus Management (`focused_node`)
1. **Active Focus Tracking:**
   `CompiledDocument` and `ViewerState` track:
   ```rust
   pub focused_node: Option<NodeId>,
   ```
2. **Focus Events:**
   - If `old_focused != new_focused`:
     - Dispatch `EventKind::Blur` to `old_focused`.
     - Dispatch `EventKind::Focus` to `new_focused`.
3. **Target Highlighting (`:target`):**
   - The target `\Anchor` wrapper receives a visual highlight pulse or focus ring:
     ```rust
     if Some(node.id) == self.focused_node {
         render_focus_ring(scene, &node.rect, theme.focus_color);
     }
     ```
4. **Keyboard Traversal (Tab / Shift+Tab):**
   - Scopes define a sequential tab order across interactive elements (`Link`, `Button`, `Anchor`).

---

## 6. DOM Lifecycle, Mutations & $O(1)$ Pruning

When using the live Component DOM (`Dom`):
1. **Scope Ownership:** Each `DomNode` representing an `\AnchorScope` owns its local scope table.
2. **Subtree Detachment & Destruction:**
   When `dom.destroy_node(handle)` is called on a pane or container:
   - The arena recursively frees all nodes in the subtree.
   - The associated `NavigationScope` is detached and dropped as a single unit ($O(1)$).
   - There are **no global maps to sweep** and no risk of orphaned anchor entries.

---

## 7. Implementation Milestones

| Phase | Milestone | Deliverables | Target Files |
| :--- | :--- | :--- | :--- |
| **Phase 1** | **`\Anchor` Wrapper & Data Model** | AST nodes for `\Anchor`, pass-through layout equations, inline text anchor bookmark support. | `src/ast.rs`<br>`src/parser/`<br>`src/compiler/expand.rs`<br>`src/compiler/layout.rs` |
| **Phase 2** | **Hierarchical Scopes (`\AnchorScope`)** | `ScopeTree` data structure, scope registration, absolute (`/`) vs. relative path resolution algorithm. | `src/compiler/scope.rs`<br>`src/compiler/layout.rs`<br>`src/compiler/compiled.rs` |
| **Phase 3** | **First-Class Port Passing & `.append()`** | Support passing `AnchorScope` as a port value, evaluate `scope.append("target")` to produce canonical URL strings. | `src/compiler/value.rs`<br>`src/compiler/eval.rs`<br>`src/compiler/expand.rs` |
| **Phase 4** | **Viewer In-Page Scroll Navigation** | Intercept `#` URLs in viewer, resolve target node, calculate scroll delta, update `ScrollView` offset. | `src/render/viewer.rs`<br>`src/interaction.rs` |
| **Phase 5** | **Focus Management & Target Highlighting** | `focused_node` state, focus/blur event dispatch, target highlight pulse/focus ring rendering. | `src/render/viewer.rs`<br>`src/render/scene.rs`<br>`src/compiler/compiled.rs` |
| **Phase 6** | **Standard Components & Split-Pane Demo** | Author `components/Anchor.dt`, `components/AnchorScope.dt`, and `examples/split_pane_anchor_demo.dt`. | `components/Anchor.dt`<br>`components/AnchorScope.dt`<br>`examples/` |
