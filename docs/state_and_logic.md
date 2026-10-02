# DirectedType (DTML) Specification: State, Application Logic, & Object-Oriented Component Model

This document outlines the architectural specification for **Declarative Reactive State**, **Application Logic**, and the **Object-Oriented Rust/WASM Component Model** in the DirectedType layout engine.

---

## 1. The Core Philosophy: The One-Way Information Pipeline

DirectedType replaces the traditional web document flow with a strict **Directed Acyclic Graph (DAG)** of late-bound algebraic layout equations. To preserve the mathematical integrity of the DAG and prevent the bugs and performance pitfalls of web browsers (such as layout thrashing and forced synchronous reflows), DirectedType enforces a strict **One-Way Information Pipeline**:

```
┌────────────────────────────────────────────────────────┐
│ Phase 1: Application Logic (Rust / WASM)               │
│ • State Transitions: State_t ──► State_{t+1}           │
│ • Reads: Event payloads, App Services, Local State     │
│ • Writes: Mutates local `state` fields                 │
└───────────────────────────┬────────────────────────────┘
                            │ (One-Way State Injection)
                            ▼
┌────────────────────────────────────────────────────────┐
│ Phase 2: Topological Layout Evaluation (DAG)           │
│ • Math: `width: is_open ? 300 : 0`                     │
│ • Reads: State cells, Ports, Formulas, Text metrics    │
│ • Computes: Flat array of physical pixel coordinates   │
└───────────────────────────┬────────────────────────────┘
                            │ (One-Way Draw List)
                            ▼
┌────────────────────────────────────────────────────────┐
│ Phase 3: GPU Composition & Painting (Vello)            │
│ • 120 FPS rasterization to the native window surface   │
└────────────────────────────────────────────────────────┘
```

### The Cardinal Invariant
**There is NEVER an edge or query from `Layout Math (DAG)` backwards into `Application Logic (Rust)`.**

Application code cannot query evaluated spatial layout values (`node.x`, `node.y`, `node.width`, `node.height`, `node.clip`) to make runtime state decisions. By eliminating back-edges from layout to logic, **feedback loops and layout thrashing are mathematically impossible.**

---

## 2. The Unified Variable Matrix

In DirectedType, layout math consists of two fundamental classes of variables:
1. **Independent Source Nodes (Indegree = 0):** Values supplied from outside the layout engine's math (parameters from callers, ambient context, or state mutated by application logic).
2. **Dependent Formula Nodes (Indegree > 0):** Values calculated by algebraic equations: $f(v_1, v_2, \dots)$.

Every variable in the language is defined across two orthogonal axes: **Mathematical Role** and **Visibility Scope**:

| Scope / Visibility | Independent Source Node<br>*(Set from Outside)* | Dependent Formula Node<br>*(Evaluated Topologically)* |
| :--- | :--- | :--- |
| **Public API**<br>*(Caller / Siblings)* | **Public Input Port**<br>`\Component Card(width: Number: 100)`<br>*Configured by caller; defaults if omitted.* | **Alias Port**<br>`alias right = x + width;`<br>*Public, read-only output for siblings/parent.* |
| **Ambient Context**<br>*(Cascaded down)* | **Environmental Port**<br>`env font: Font`<br>*Pushed down by ancestor context.* | **Environmental Derived Formula**<br>`env theme = dark ? Dark : Light;`<br>*Computes ambient value for descendants.* |
| **Private Internal**<br>*(Component Body)* | **State Variable**<br>`state count: Number = 0;`<br>*Owned and mutated by Rust application logic.* | **Local Helper (`let`)**<br>`let half_width = width / 2;`<br>*Pure internal layout shorthand.* |

---

## 3. State Variables (`state`) vs. Local Variables (`let`)

### Why `state` Must Be Distinct From `let`
In pure declarative layout, `let` defines an **immutable mathematical binding**:
```dtml
let card_width = parent.width - 32;
let padding = 16;
```
If application logic could arbitrarily overwrite `card_width` at runtime:
1. The declarative contract would be broken—an engineer reading the code could no longer trust that `card_width` is always 32px narrower than its parent.
2. It would require bidirectional constraint solving (Cassowary/Simplex), which DirectedType explicitly rejects in favor of deterministic, one-way topological sorting.

Therefore:
* **`let` (Formulas):** Owned by the layout engine. Immutable equations. Application logic cannot read or write them.
* **`state` (Data Cells):** Owned by the backing Rust struct. Mutable by application logic. Act as source nodes in the layout DAG.

### The Fallback Multiplexer Model (Static Graph Topology)
When a `state` variable declares an initial expression, setting that state does **not** sever edges or alter the shape of the DAG:
```dtml
state bio: String: "Loading bio for ID #" + user_id + "...";
```

Mathematically, a `state` variable is modeled as a **Fallback Multiplexer Node**:
```
┌──────────────────────────────────────────────┐
│ State Node: `state bio = "Loading..."`       │
│                                              │
│  [ Initial Expression ] ───┐                 │
│                            ▼                 │
│                     ┌─────────────┐          │
│  [ Runtime Value Slot ]──►│ MUX (Cell)  ├───► Output to Downstream Nodes
│  (None | Some("Alice"))  └─────────────┘     │
└──────────────────────────────────────────────┘
```

1. **Before Application Write:** The runtime slot is `None`. The cell evaluates its initial algebraic expression (`"Loading bio for ID #42..."`).
2. **After Application Write:** Rust sets `self.bio = "Alice".into()`. The runtime slot becomes `Some("Alice")`. The cell evaluates to the stored value and bypasses the fallback expression.
3. **Topology Remains Static:** The DAG schedule never changes shape. Setting state simply updates the value in the cell's memory slot and flags downstream nodes dirty for a single-pass topological re-evaluation.

---

## 4. The Object-Oriented Rust Component Model

Each DTML component can be backed by a **typed Rust struct**:
* **The Rust Struct** owns **State & Behavior** (fields, business logic, event handlers).
* **The DTML Markup** owns **Layout & Geometry** (algebraic equations, visual primitives, child structure).

### Authoring Syntax: Side-by-Side or Inline

Components can be authored with companion `.rs` files or with inline `rust { ... }` blocks inside `.dt` files:

```dtml
// Counter.dt
\Component Counter(initial: Number: 0) {
    // 1. Declare state variables backed by Rust struct fields
    state count: Number: initial;
    state is_max: Boolean = count >= 10;

    // 2. Algebraic layout consuming state
    \Rect(
        width: 280,
        height: 80,
        color: is_max ? #ef4444 : #1e293b,
        radius: 12
    ) {
        \Button(on_click: self.decrement) { "-" }

        \Text(size: 20, color: #ffffff) { 
            "Count: " + count 
        }

        \Button(on_click: self.increment) { "+" }
    }
}
```

```rust
// Counter.rs
use directedtype_wasm::prelude::*;

#[derive(Default, Component)]
pub struct Counter {
    pub count: i32,
}

impl Counter {
    // Lifecycle: called once when the component mounts into the DOM
    pub fn on_mount(&mut self, ctx: &mut Context) {
        if let Some(initial) = ctx.get_port_number("initial") {
            self.count = initial as i32;
        }
    }

    // Strongly typed method bound to `on_click: self.increment`
    pub fn increment(&mut self, e: &ClickEvent, ctx: &mut Context) {
        if self.count < 10 {
            self.count += 1;
        }
    }

    pub fn decrement(&mut self, e: &ClickEvent, ctx: &mut Context) {
        if self.count > 0 {
            self.count -= 1;
        }
    }
}
```

### Key Properties:
1. **Encapsulated Instances:** Every instantiated `<Counter>` in the DOM has its own dedicated `Counter` struct instance in WASM memory.
2. **Full Rust Expressiveness:** Methods have access to Rust's pattern matching, borrow checker, standard collections (`Vec`, `HashMap`), and third-party crates.
3. **Static Method Verification:** If DTML writes `on_click: self.incremnet` (a typo), the compiler rejects it at build time:
   ```text
   CompileError: Component 'Counter' has no method 'incremnet'. Did you mean 'increment'?
   ```
4. **Automatic Snapshot Reflection:** At method return, the WASM runtime automatically detects mutated state fields and emits batched updates into shared memory. Zero manual notification boilerplate is required.

---

## 5. Strict Visibility & Isolation Guarantees

To enforce the One-Way Pipeline, the visibility rules for Rust component code are strictly defined:

```
┌────────────────────────────────────────────────────────┐
│ Rust Component Struct (Self)                           │
│                                                        │
│  CAN ACCESS:                                           │
│  ✅ Private state fields (`self.count`, `self.is_open`) │
│  ✅ Context API (`ctx.app_state()`, `ctx.spawn_async`)  │
│  ✅ Event payload (`e.button`, `e.local_x`, `e.key`)    │
│  ✅ Configuration props at mount (`ctx.get_port(...)`)  │
│                                                        │
│  CANNOT ACCESS:                                        │
│  ❌ Local layout formulas (`let` bindings in DTML)     │
│  ❌ Computed spatial layout values (`x`, `y`, `w`, `h`)│
│  ❌ Active clip chains or GPU render stacks             │
└────────────────────────────────────────────────────────┘
```

### Event Payloads as Snapshots
If application logic needs spatial information (e.g., where a user clicked inside a canvas), that information is delivered as an **immutable input snapshot** on the event object:
```rust
pub fn on_pointer_down(&mut self, e: &PointerEvent, ctx: &mut Context) {
    let click_x = e.local_pos.x;
    let click_y = e.local_pos.y;
    // Logic consumes snapshot data, does not query the live DAG!
}
```

---

## 6. Cross-Component Communication & State Sharing

Real applications require components to communicate across hierarchies. DirectedType provides three clean patterns:

### Pattern A: State Lifting via Ports (Parent-to-Child)
When two sibling components must stay synchronized, their state is owned by their common parent and passed down as input ports:

```dtml
\Component PageLayout {
    state sidebar_open: Boolean: true;

    \NavBar(
        is_sidebar_open: sidebar_open,
        on_toggle: self.toggle_sidebar
    )
    \Sidebar(open: sidebar_open)
}
```
* **For `\Sidebar`:** `open` is a standard public input port. It does not know or care whether the value came from state, an expression, or a literal.
* **Topological Flow:** Flipping `sidebar_open` updates both components in a single topological evaluation pass.

---

### Pattern B: Lexical Component Handles (Actor Message-Passing)
When one component needs to trigger an action on a sibling without passing callbacks through intermediate parents, it uses a **lexical component handle**:

```dtml
\Component PageLayout {
    // 1. Declare target component with lexical handle:
    let drawer = \Drawer(width: 300);

    // 2. Button dispatches directly to the drawer's public method:
    \Button(on_click: drawer.toggle) { "Menu" }
}
```

```rust
// Drawer.rs
#[derive(Default, Component)]
pub struct Drawer {
    pub is_open: bool,
}

impl Drawer {
    pub fn toggle(&mut self, _e: &ClickEvent, _ctx: &mut Context) {
        self.is_open = !self.is_open;
    }
}
```

#### Why This Does Not Violate the One-Way Boundary:
* `drawer` is an **Instance Address** (an actor handle), NOT a DAG layout value.
* Writing `on_click: drawer.toggle` emits a message:
  ```rust
  EventMessage { target: InstanceId(42), method: "toggle" }
  ```
* Inside `Drawer::toggle`, the method only mutates its own private `self.is_open`. The calling button never inspects or reads the drawer's layout properties.

---

### Pattern C: Environmental State Services (`env`)
For application-wide domain models (shopping carts, user authentication, global themes) that are needed deep within the component tree, passing ports through dozens of layers ("prop drilling") is avoided using **Environmental Variables (`env`)**:

```dtml
\Component AppRoot {
    // Inject the shared application service into the ambient environment:
    env cart: CartModel = app.cart;

    \MainView
}
```

Any descendant anywhere in the subtree can directly read from or dispatch actions to the ambient service:

```dtml
\Component ProductCard(product: Product) {
    \Button(on_click: env.cart.add_item(product.id)) {
        "Add to Cart (" + env.cart.item_count + ")"
    }
}
```

---

## 7. Inputs to Application Logic & Asynchronous Work

Application logic does not only run in response to user clicks; it also processes background tasks (database operations, network requests, timers, and WebSocket messages).

Every component method receives a mutable reference to the **Application Context (`ctx: &mut Context<AppState>`)**.

### Context Capabilities:
1. `ctx.app_state()`: Access shared application services and domain stores.
2. `ctx.spawn_async(...)`: Spawn non-blocking asynchronous futures.
3. `ctx.node_handle()`: Access the component's identity handle.

### Asynchronous Workflow:
```rust
// UserProfileView.rs
use directedtype_wasm::prelude::*;

#[derive(Default, Component)]
pub struct UserProfileView {
    pub user_name: String,
    pub avatar_url: String,
    pub is_loading: bool,
}

impl UserProfileView {
    pub fn on_mount(&mut self, ctx: &mut Context<MyAppState>) {
        self.is_loading = true;
        let user_id = ctx.get_port_number("user_id") as u64;
        let db_service = ctx.app_state().db.clone();

        // 1. Spawn non-blocking background async work:
        ctx.spawn_async(async move {
            let profile = db_service.fetch_profile(user_id).await;

            // 2. Post result safely back to this specific component instance:
            AsyncResult::update(move |component: &mut UserProfileView, _ctx| {
                component.user_name = profile.name;
                component.avatar_url = profile.avatar;
                component.is_loading = false;
                // Method return automatically syncs to the DAG and repaints!
            })
        });
    }

    pub fn on_save(&mut self, _e: &ClickEvent, ctx: &mut Context<MyAppState>) {
        let db = ctx.app_state().db.clone();
        let name_to_save = self.user_name.clone();

        ctx.spawn_async(async move {
            db.save_user_name(name_to_save).await;
        });
    }
}
```

---

## 8. Structured Component Identifiers & Programmatic Addressing

For application logic to act programmatically on components (e.g. focusing a text input, scrolling to a specific table cell, or highlighting elements), components require reliable identities.

Rather than relying on flat, fragile string concatenation (like `"cell_3_5"`), DirectedType supports **Structured Composite Identifiers** (tuples of integers, strings, or atoms).

### 1. Syntax Ergonomics: `\Component(identity; ports...)`

DirectedType uses a semicolon (`;`) inside the component instantiation parentheses to cleanly separate **Identity** from **Layout Ports**:

```dtml
// Anonymous component (no structured ID; engine assigns internal NodeId)
\Rect(width: 200, height: 100, color: #3b82f6)

// Single-part identifier
\Button("submit_btn"; width: 120, height: 40)

// Multi-part Composite ID (Row, Column)
\Cell(row, col; width: 80, height: 32, color: #ffffff)

// Semantic Domain ID (Entity type + Entity ID)
\UserCard("user", user.id; width: 300)
```

Everything **before** the semicolon is the `ComponentKey` tuple. Everything **after** the semicolon is the list of public input ports.

---

### 2. Why IDs Cannot Be Part of the Layout DAG

A component’s identifier **MUST NOT** be a node in the layout DAG:
* **The Circular Paradox:** Application logic needs to query, address, and update components during Phase 1 (before layout math runs). If an ID were computed from layout math (e.g. `node.x` or `node.width`), the component's identity would not exist until Phase 2 completes, creating a fatal feedback loop.
* **The State / Static Purity Invariant:** All expressions in the identity header (before `;`) must be statically evaluatable, loop iteration variables, or derived from application `state` variables.
* **Compiler Enforcement:** If the compiler detects an AST reference to a layout port (`x`, `y`, `width`, `height`, `clip`) or a local layout `let` binding before the `;`, it throws a compile-time error:
  ```text
  CompileError: Component ID cannot depend on layout port 'width'. 
  IDs must be constant literals, loop iterators, or state variables.
  ```

---

### 3. The Four Rules of Structured Component IDs

#### Rule 1: Scoped & Hierarchical Namespaces (The "Two Tables" Problem)
If two separate `\Table` components are rendered on the same page, both will have a cell at `(0, 0)`. If IDs were global strings, they would collide.

In DirectedType, structured IDs are **lexically scoped to their parent component**:
* Inside `table_a`, cell `(0, 0)` is unique.
* Inside `table_b`, cell `(0, 0)` is unique.
* The full document path is hierarchical: `table_a / (0, 0)`.
* In Rust logic:
  - Inside `Table`, code queries locally: `ctx.get_child_by_key(&(0, 0))`.
  - From the root, code queries hierarchically: `ctx.get_by_path(&["table_a", (0, 0)])`.

#### Rule 2: Identity Governs Lifecycle (Key Change = Re-mount)
In DirectedType, **Identity IS the lifecycle**:
* If an item’s key changes (for instance, in a dynamic list where an ID flips from `101` to `102`), the engine unmounts the old component instance and mounts a fresh one.
* Ephemeral local state from the old component is safely destroyed, preventing state leakage across different data items.

#### Rule 3: Support Both Declarative Binding and Programmatic $O(1)$ Lookup
Structured IDs empower both paradigms of GUI architecture:

1. **Declarative Selection (Standard UI):**
   ```dtml
   \Cell(r, c; 
       highlighted: selected_cell == (r, c),
       on_click: self.select_cell(r, c)
   )
   ```
   State updates, and the DAG evaluates `highlighted: true` for the matching cell in microseconds.

2. **Programmatic Lookup (Dynamic Commands):**
   Application logic can directly query child components in $O(1)$ time using the structured key:
   ```rust
   let target_key = ComponentKey::tuple(&[row, col]);
   if let Some(cell_handle) = ctx.get_child_by_key(&target_key) {
       ctx.set_focus(cell_handle);
   }
   ```

---

### 4. Concrete Example: Table Navigation with Structured IDs

```dtml
// TableView.dt
\Component TableView {
    state cursor_row: Number: 0;
    state cursor_col: Number: 0;

    \Flow(gap: 2) {
        \For(r in 0..10) {
            \Row(gap: 2) {
                \For(c in 0..5) {
                    // Identity is (r, c); public ports follow the semicolon
                    \Cell(r, c; 
                        is_active: (r == cursor_row && c == cursor_col),
                        on_click: self.on_cell_clicked(r, c)
                    )
                }
            }
        }
    }
}
```

```rust
// TableView.rs
#[derive(Default, Component)]
pub struct TableView {
    pub cursor_row: usize,
    pub cursor_col: usize,
}

impl TableView {
    // Arrow key navigation from a key event:
    pub fn on_key_down(&mut self, e: &KeyEvent, ctx: &mut Context) {
        match e.key_code {
            KeyCode::ArrowDown => self.cursor_row = (self.cursor_row + 1).min(9),
            KeyCode::ArrowUp => self.cursor_row = self.cursor_row.saturating_sub(1),
            KeyCode::ArrowRight => self.cursor_col = (self.cursor_col + 1).min(4),
            KeyCode::ArrowLeft => self.cursor_col = self.cursor_col.saturating_sub(1),
            _ => return,
        }

        // Programmatic hook: focus or inspect the target cell by its structured key
        let target_key = ComponentKey::tuple(&[self.cursor_row, self.cursor_col]);
        if let Some(target_handle) = ctx.get_child_by_key(&target_key) {
            ctx.set_focus(target_handle);
        }
    }

    pub fn on_cell_clicked(&mut self, r: usize, c: usize, _e: &ClickEvent, _ctx: &mut Context) {
        self.cursor_row = r;
        self.cursor_col = c;
    }
}
```

---

## 8.5. Component Imports & Multi-File Architecture

Components are organized across modular `.dt` files and imported using the `\use` directive:

```dtml
// Main.dt
\use "./widgets/Button.dt";
\use "./cards/ProductCard.dt" as Card;

\Card(id: 101) {
    \Button(label: "Buy Now")
}
```

### Module Resolution Rules:
1. **Explicit Tag-Prefixed Syntax:** The `\use` keyword requires a leading backslash, preventing parsing conflicts with plain document prose starting with the word *"Use"*.
2. **Path Normalization:** Relative paths (`./`, `../`) resolve relative to the enclosing file's directory.
3. **Aliasing Support:** An optional `as AliasName` aliases the imported component, enabling conflict-free namespacing and semantic renaming.
4. **Cycle Detection & Memoization:** The compiler maintains an active recursion stack detecting circular dependencies (`A -> B -> A`) and throwing `CompileError::CyclicImport`, while safely memoizing diamond dependency graphs (`A -> B, A -> C, B -> D, C -> D`).
5. **Transitive Dependency Propagation:** When a component is imported (either directly or via an alias), any child components it internally depends upon are safely made available to the component's expansion scope.

---

## 9. Summary of Guarantees

1. **Deterministic Layout Math:** Formulas (`let`, `alias`, and spatial ports) are strictly algebraic and evaluated topologically. Application logic cannot overwrite equations or break formulas.
2. **Actor-Based State Isolation:** Components own their state in strongly typed Rust structs. They communicate via one-way ports (downward), actor handles (horizontal messaging), and environmental services (ambient).
3. **Structured Non-DAG Identity:** Identifiers are defined before a semicolon (`\Cell(row, col; ...)`), determined strictly by literals, loop variables, or state, guaranteeing $O(1)$ programmatic lookup without layout feedback cycles.
4. **Zero Layout Thrashing:** Because Rust logic has no read access to computed layout values, feedback cycles between logic and layout are mathematically impossible.
5. **Minimal Boundary Crossing:** Events cross from the native viewer into WASM once per event turn; all resulting state mutations are batched and applied to the DAG simultaneously, triggering a single-pass GPU redraw.


