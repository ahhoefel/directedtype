# DTML Architecture Request for Comments (RFC): Environmental Scope (`env`) & Lexical Propagation

## 1. Core Philosophy: Lexical vs. Structural Propagation

In traditional DOM/CSS models, property inheritance is **structural** and evaluated at runtime. A node inherits `color` based on its physical location in the final element tree, which routinely violates component encapsulation (e.g., CSS bleed).

In Directed Type (DTML), the UI is a strict Directed Acyclic Graph (DAG) evaluated at compile-time. To maintain component hermeticity without sacrificing ergonomics (forcing designers to manually wire every property), DTML strictly divides properties into two propagation models:

1. **Layout Traits (`x`, `y`, `width`, `height`, `z`):** Strictly **Structural**. They map explicitly from Parent to Immediate Child via the `\Children` directive. They never pierce through a component into its grandchildren.
2. **Environmental Traits (`color`, `clip`, `transform`):** Strictly **Lexical**. They map from the parser's *Lexical Scope Stack* to every node physically authored within that block that declares or accepts the property.

At the end of the Parse Phase, all implicit environmental inheritances are resolved into hard, explicit mathematical DAG edges.

---

## 2. Port Signatures and The `env` Keyword

For a public input port to auto-propagate lexically to child elements, it must be explicitly marked with the `env` keyword in the component's signature:

```dtml
// 'width' must be wired explicitly. 'theme' will auto-propagate lexically to children.
\Component ThemedBox(width: Number, env theme: String) {
    \Children
}
```

At the call site, the caller provides arguments normally without writing `env`:
```dtml
\ThemedBox(width: 300, theme: "dark") {
    // Child elements in this lexical scope automatically inherit theme: "dark"
    \ChildComponent()
}
```

### The Universal Base Trait

To ensure fundamental spatial realities (like hardware clipping and 2D transforms) work universally without boilerplate, the compiler implicitly injects a **Universal Base Trait** into the signature of *every* component:
* `env clip: NodeId`
* `env transform: Affine`

This guarantees that `env clip` acts as a transparent bridge, penetrating through custom layout wrappers and `\Children` blocks down to the raw paint primitives (`\Rect`, `\Text`) that issue GPU instructions, without requiring manual `clip: self.clip` plumbing on every component.

---

## 3. External Lexical Scoping & Encapsulation

Environmental traits flow down to any node authored *lexically* within their block that accepts the property, while strictly respecting component boundaries.

### The Call Site

```dtml
\Theme(color: "dark") {
    // \Row is a layout component. It does NOT have an `env color` port.
    \Row {
        // \Button explicitly accepts environmental color via `env color`.
        \Button { "Submit" }
    }
}
```

### Parser Execution & Encapsulation Boundary
1. The parser evaluates `\Theme` and pushes `color = "dark"` onto the active Lexical Scope Stack.
2. It evaluates `\Row`, sees no `env color` parameter, and does not pass `color` to `\Row` (preserving `\Row`'s internal encapsulation).
3. It evaluates `\Button`, sees that `\Button` explicitly exposes an `env color` parameter in its signature (`\Component Button(env color: Color)`), and draws a direct DAG edge: `Button.color = Theme.color`.

> [!IMPORTANT]
> **Strict Opt-In via `env`:**
> Environmental variables **only** bind to parameters explicitly marked with the `env` keyword. Standard parameters (e.g. `\Component Bar(color)`) do **not** bind to ambient environmental variables. This prevents accidental parameter name collisions and unintended action-at-a-distance. If a non-env parameter has no default and is not supplied by the caller, compilation fails with `CompileError::MissingPort`.

### Components Are Sealed Black Boxes
Environmental traits **do not penetrate** into the private internal implementation of a component unless that component explicitly exposes that port in its public signature as an `env` parameter.

For example:
```dtml
\Component MyCard {
    \Rect(color: #ffffff)
    \Text { "Card title" }
}

\Theme(color: "purple") {
    \MyCard()
}
```
`Theme`'s `color` does **not** mutate `MyCard`'s internal `\Text` or `\Rect` because `MyCard` did not declare `env color` as an input port. `MyCard` remains completely hermetic.

---

## 4. Local Environmental Variables (`env name = ...`) & Shadowing

Components and content blocks can define **local environmental variables**. These are not declared as public input ports that callers set, but are created locally and propagate down into nested lexical scopes.

The syntax uses `env name = expr`:

```dtml
\ScrollView {
    env text_color = #000000
    env clip = \Clip(up: self.clip, box: \Box(x: self.left, y: self.top, width: self.width, height: self.height))

    \Row {
        \Button() // Automatically inherits text_color and clip
    }
}
```

### Shadowing & Promotion Rules (The 2x2 State & Transport Matrix)

DirectedType completely decouples **State (the value)** from **Transport (how it flows down the tree)**. All four combinations of parameter and local declarations are permitted:

1. **Non-env Parameter + Non-env Local (`let foo = ...` - Standard Local Math):**
   Internal component calculation. Neither is environmental; descendants in `\Children` inherit nothing automatically.

2. **Env Parameter + Env Local (`env foo = ...` - The Interceptor / Middleware):**
   Intercepts an incoming environmental parameter, modifies or replaces it, and propagates the new value to all descendants in `\Children`:
   ```dtml
   \Component Surface(env color: Color) {
       \Rect(color: self.color) // Uses incoming ambient color
       env color = lighten(self.color, 15%) // Shadows and transforms for children
       \Children // Descendants receive lightened color
   }
   ```

3. **Non-env Parameter + Env Local (`env foo = ...` - The Translator / Provider):**
   Converts an explicit configuration property into an ambient environmental broadcast for descendants:
   ```dtml
   \Component ThemeProvider(theme_name: String) {
       env color = (self.theme_name == "dark") ? #000 : #FFF
       \Children // Descendants inherit ambient color
   }
   ```

4. **Env Parameter + Non-env Local (`let foo = ...` - The Shield / Swallower):**
   Consumes an incoming ambient environment locally, but intentionally **stops** it from cascading to children. Overwriting the name with a non-environmental `let` binding drops `foo` from the active environment stack for descendants:
   ```dtml
   \Component AlertBadge(env color: Color) {
       \Rect(fill: self.color)
       let color = #000 // Overwrites name as non-env; drops color from env stack
       \Children // Ambient env color has been safely swallowed
   }
   ```

### Uninitialized Declarations & Lexical Tombstones (`let foo;` and `env foo;`)

When the only goal is to swallow an ambient environment or create a firewall without allocating a dummy value in the DAG, DirectedType supports **Lexical Tombstones**:

* **`let color;` (The Firewall Tombstone):**
  Pushes a non-env `Tombstone` onto the scope stack.
  1. **Halts Propagation:** When descendants in `\Children` request ambient `env color`, lookup hits the `Tombstone` and halts. Descendants fall back to Tier 1 default or error if required.
  2. **Errors on Explicit Use:** If an expression inside the component attempts to read `color`, the compiler emits:
     ```text
     CompileError: Attempted to use explicitly uninitialized variable 'color'.
     ```

* **`env color;` (The Environmental Hole):**
  Pushes an `env Tombstone` onto the scope stack. It errors if used locally, and propagates downstream to mask any higher ambient environment for all descendants, forcing them to fallback to Tier 1 defaults.

---

## 5. The 4-Tier Precedence Hierarchy

In DirectedType, port collisions are resolved at compile-time using a deterministic **4-Tier Precedence Hierarchy**:

```
Tier 4: Explicit Instance Override  (Highest priority)
   ▲
   │  overrides
   │
Tier 3: Structural Container Wiring (\Children)
   ▲
   │  overrides
   │
Tier 2: Lexical Environment (env)
   ▲
   │  overrides
   │
Tier 1: Component Signature Default (Lowest priority)
```

### Tier Breakdown

1. **Tier 4: Explicit Instance Override (Highest)**
   Direct wiring at the instantiation site always wins:
   ```dtml
   \Button(color: "green")
   ```

2. **Tier 3: Structural Container Wiring (`\Children`)**
   A parent container's explicit structural rule takes precedence over ambient environmental values:
   ```dtml
   \Component AlertRow() {
       \Children {
           color: "red" // Explicit structural container rule
       }
   }
   ```
   If called inside `\Theme(color: "blue")`:
   ```dtml
   \Theme(color: "blue") {
       \AlertRow {
           \Button() // Receives "red" from AlertRow, NOT "blue" from Theme
       }
   }
   ```
   The `\Children` directive acts as a shield, ensuring layout and container contracts are never subverted by distant lexical ancestors.

3. **Tier 2: Lexical Environment (`env`)**
   If no explicit instance argument (Tier 4) and no structural container rule (Tier 3) were provided, the port inherits from the closest active `env` variable on the Lexical Scope Stack:
   ```dtml
   \Theme(color: "blue") {
       \Button() // Receives "blue" from Theme
   }
   ```

4. **Tier 1: Component Signature Default (Lowest)**
   If no higher tier provides a value, the component's parameter default expression is used:
   ```dtml
   \Component Button(color: Color: "gray")
   ```

---

## 6. The AST Handoff (Finalizing the Graph)

By the time the DTML compiler completes template expansion, all implicit scopes, shadows, and environments have been resolved into hard, explicit mathematical DAG edges:

* `Button_1.clip = Clip_A`
* `Button_1.color = Surface_Lighten_B`

Because the DAG treats clipping, colors, and layout traits as pure data edges, the Rust/Vello rendering backend evaluates them deterministically, sorts visual primitives by `z` and `clip`, and pushes draw commands directly to GPU layers with zero runtime cascading overhead.