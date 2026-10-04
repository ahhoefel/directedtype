# DirectedType Design Document: Component Constructor Overloading

- **Status:** Proposed
- **Author:** Antigravity / DirectedType Core Team
- **Date:** October 2026
- **Target Area:** Compiler (`compiler::expand`, `component::registry`), Language Grammar (`syntax::ast`)

---

## 1. Executive Summary

In DirectedType (DTML), user interfaces are evaluated as a mathematical **Directed Acyclic Graph (DAG)** of layout variables sorted topologically via Kahn's algorithm. Because layout edges represent strict dependencies between variables, a component's sizing mode (intrinsic/content-driven vs. extrinsic/parent-driven) dictates the direction of dependency edges in the graph.

Currently, DirectedType enforces a **single constructor signature** per component. When a component needs to support multiple sizing behaviors (for example, a `Card` that can either have an explicit fixed width or automatically fill its parent container), authors are forced to use sentinel values (`width: 0`) and runtime ternary expressions (`width > 0 ? width : parent.width - 2 * x`). Because dependency analysis examines all branches of an expression, the unused branch introduces **phantom dependency edges**, causing false cycle detection errors when composing intrinsic and extrinsic containers (such as placing a `Card` with fixed `width` inside an `HStack`).

This document proposes **Component Constructor Overloading based on Port Signatures**. Overloading enables components to declare distinct templates for distinct layout contracts. By selecting the matching overload at compile time during AST expansion, the resulting DAG contains only the exact edges for the chosen layout mode, completely eliminating phantom cycle hazards and enabling bidirectional layout patterns (such as `Text` taking `width` to calculate `height`, or taking `height` to calculate `width`).

Furthermore, this design sets an explicit architectural goal: **moving away from default parameter values**. If overloading renders parameter defaults obsolete, overload candidate filtering collapses from a complex heuristic ranking of partial subsets into **strict, exact set matching on port names**, radically simplifying the compiler and language semantics.

---

## 2. Motivation & The Problem Today

### 2.1 The DAG Layout Paradigm & The "Fit-Content Paradox"

In HTML/CSS, layout systems frequently oscillate or run multi-pass negotiation loops (such as Flexbox or CSS Grid measuring intrinsic min/max content before applying stretched constraints). 

In DirectedType, layout is **not** multi-pass negotiation; layout is **pure algebraic math**:
- Every spatial variable (`x`, `y`, `width`, `height`, `z`, and derived aliases like `left`, `right`, `top`, `bottom`) is a node in a `VariableGraph`.
- Directed edges represent variable dependencies.
- A single topological sort evaluates the entire UI deterministically.

This architecture enforces strict one-way flow. If container width depends on child width, child width **must not** depend on container width:
$$\text{Container.width} = \sum \text{Child.width} \implies \text{Child.width} \not\leftarrow \text{Container.width}$$

Violating this rule produces the classic **"Fit-Content Paradox"** and triggers a fatal cycle diagnostic (`CompileError::CyclicDependency`).

---

### 2.2 Single-Signature Workarounds and Phantom Cycles

Because a component can currently only declare one signature, component authors combine multiple mutually exclusive sizing behaviors into a single definition:

```dt
// components/Card.dt
\Component Card(
    x: Number: 0,
    y: Number: 0,
    width: Number: 0, // Sentinel value: 0 means "fill parent"
    ...
) {
    let resolved_width = width > 0 ? width : (parent.width - (2 * x));
    alias right = x + resolved_width;
    ...
}
```

Now, consider placing a fixed-width `Card` inside an `HStack`:

```dt
\HStack(x: 40, y: 115, gap: 25) {
    \Card(width: 210) { ... }
}
```

- `HStack` is intrinsically sized along the horizontal axis:
  $$\text{HStack.right} = \max(\text{children.right}) + \text{padding\_x}$$
  $$\text{HStack.width} = \text{right} - x$$
- `Card.right` depends on `Card.resolved_width`.
- When the graph compiler builds dependency edges with `collect_dependencies`, it walks the expression AST:
  ```rust
  Expr::Ternary(t) => {
      collect_dependencies(&t.condition, out);
      collect_dependencies(&t.then_expr, out);
      collect_dependencies(&t.else_expr, out);
  }
  ```
  It inspects **both** branches. Even though `width: 210` was provided and the `else` branch would never execute at runtime, the compiler registers a dependency from `Card.resolved_width` to `parent.width` (`HStack.width`).

This creates a fatal **phantom cycle** in the DAG:
$$\text{HStack.width} \longrightarrow \text{Card.right} \longrightarrow \text{Card.resolved_width} \longrightarrow \text{HStack.width}$$

The developer explicitly passed `width: 210` to avoid relying on `parent.width`, but the presence of the un-taken fallback branch in the single AST template infected the graph.

---

### 2.3 The Stopgap: Static Constant Condition Folding

To resolve this issue for `HStack`, we implemented static condition folding in `src/compiler/expand.rs`:
- When expanding a component, if a ternary condition in a `let` binding depends only on constant arguments supplied to the component (e.g. `width: 210` where `210 > 0` is `true`), the compiler evaluates the condition and replaces the ternary with the active branch (`width`).
- This prunes `parent.width` before `VariableGraph` construction, eliminating the phantom cycle.

#### Limitations of Static Branch Pruning
1. **Dynamic Expressions:** If `width` is driven by a runtime state variable or companion calculation, the condition cannot be folded at compile time, and the false cycle reappears.
2. **State Leakage Risk:** Component state variables (`state count: Number: 0`) must be carefully tracked and excluded from constant folding to avoid accidentally pruning reactive UI branches at expansion time.
3. **Leaked Implementation Details:** The component's internal fallback logic leaks into the caller's cycle detection diagnostics.
4. **Inability to Support Bidirectional Contracts:** It does not solve components whose inputs and outputs swap roles based on usage.

---

## 3. Alternative Considered: `Optional<T>`

We considered adding an explicit `Optional<Number>` type to DirectedType:
```dt
\Component Card(width: Optional<Number>: None, ...) {
    let resolved_width = width.is_some() ? width.unwrap() : (parent.width - 2 * x);
}
```

### Analysis
- **Runtime `Optional` does not solve graph cycles:** If `Optional` is simply a runtime value variant (like `Value::Null` or `Option<f64>`), the expression AST still contains `parent.width`. The graph compiler still sees `parent.width` as an upstream dependency of `resolved_width`, and the phantom cycle persists.
- **Requires compile-time dispatch anyway:** To avoid the cycle, `Optional` would need compile-time pattern matching or specialization that strips the unused branch before graph generation.
- **Port presence is inherently structural:** A port is either wired to an edge or it is not. Sizing modes are not merely differences in values; they represent fundamentally different **topological wiring diagrams**.

Therefore, treating port presence as a **compile-time signature matching problem** is mathematically and architecturally aligned with DirectedType's DAG execution model.

---

## 4. Proposed Solution: Component Constructor Overloading

### 4.1 Syntax

A component may define multiple signatures under the same identifier. Each overload declares a distinct parameter signature and internal body wiring:

```dt
// ============================================================================
// Card Overload 1: Caller-Constrained Width (Leaf-Driven)
// ============================================================================
\Component Card(
    x: Number,
    y: Number,
    width: Number,
    color: Color,
    padding_x: Number,
    padding_y: Number,
    gap: Number
) {
    alias right = x + width;
    alias bottom = max(children.bottom) > 0 ? max(children.bottom) + padding_y : y + (2 * padding_y);
    alias height = bottom - y;

    \Rect(x: x, y: y, width: width, height: height, color: color)
    \Children {
        x: parent.left + padding_x,
        y: prev ? prev.bottom + gap : parent.top + padding_y
    }
}

// ============================================================================
// Card Overload 2: Parent-Constrained Width (Extrinsic / Stretched)
// ============================================================================
\Component Card(
    x: Number,
    y: Number,
    color: Color,
    padding_x: Number,
    padding_y: Number,
    gap: Number
) {
    let resolved_width = parent.width - (2 * x);
    alias right = x + resolved_width;
    alias bottom = max(children.bottom) > 0 ? max(children.bottom) + padding_y : y + (2 * padding_y);
    alias height = bottom - y;

    \Rect(x: x, y: y, width: resolved_width, height: height, color: color)
    \Children {
        x: parent.left + padding_x,
        y: prev ? prev.bottom + gap : parent.top + padding_y
    }
}
```

When a user writes:
```dt
\HStack {
    \Card(width: 210) { ... }
}
```
The compiler selects **Overload 1**. Overload 1 contains **zero references** to `parent.width`. The resulting DAG has no edge to `HStack.width`, and the layout compiles acyclic and cleanly.

---

### 4.2 Canonical Use Cases

#### 1. Bidirectional Typographic Layout (`Text`)
Currently, text layout in UI engines is notoriously prone to circular constraints:
- **Case A (Paragraph Wrapping):** The container provides `width`. The text engine calculates text layout wrapped at that width and determines its resulting `height`.
- **Case B (Fixed-Height Single Line / Font Scaling):** The container provides `height`. The text engine calculates the required `width` or scales font metrics to fit.
- **Case C (Intrinsic Label):** Neither `width` nor `height` is provided. The text engine measures the natural bounding box (`text_width`, `text_height`).

In a single component template, declaring formulas for both `width` and `height` creates an immediate cycle because `width` and `height` depend on each other. With overloading, each contract is isolated:

```dt
// Overload A: Width-constrained paragraph
\Component Text(width: Number, text: String, size: Number) {
    alias height = text_height_wrapped(text, size, width);
    ...
}

// Overload B: Natural intrinsic badge/label
\Component Text(text: String, size: Number) {
    alias width = text_width(text, size);
    alias height = text_height(size);
    ...
}
```

#### 2. Padding and Spacing Variants
Instead of requiring complex fallback ladders for padding:
```dt
// Variant 1: Uniform padding
\Component Surface(padding: Number) {
    let pad_top = padding;
    let pad_left = padding;
    let pad_bottom = padding;
    let pad_right = padding;
    ...
}

// Variant 2: Axis padding
\Component Surface(padding_x: Number, padding_y: Number) {
    let pad_top = padding_y;
    let pad_left = padding_x;
    let pad_bottom = padding_y;
    let pad_right = padding_x;
    ...
}

// Variant 3: Directional quad padding
\Component Surface(padding_top: Number, padding_left: Number, padding_bottom: Number, padding_right: Number) {
    ...
}
```

#### 3. Container Stacking (`HStack` / `VStack`)
- **Intrinsic Sizing:** Omit container `width` $\rightarrow$ container sizes to fit `max(children.right)`.
- **Extrinsic Sizing:** Provide container `width` $\rightarrow$ container children stretch or distribute across the allocated space.

---

## 5. Architectural Goal: Moving Away from Default Parameter Values

A central architectural goal of this design is to **eliminate default parameter values** once overloading is available.

### 5.1 Why Default Parameter Values Add Accidental Complexity

Default parameter values (e.g. `gap: Number: 12`, `color: Color: #1e293b`) were originally introduced because single-signature components had no other way to accommodate callers who omitted arguments. However, in a reactive DAG system, default values introduce significant friction:

1. **Obscured Graph Topologies:** A parameter default like `width: Number: 0` is not just a fallback number; it is an invisible topological edge injected into the graph. When combined with sentinel checks (`width > 0 ? ...`), it generates phantom cycles.
2. **Competing Sourcing Mechanisms:** DirectedType already features a first-class **Environmental System** (`env color: Color`, `env font`, etc.) specifically designed to propagate ambient defaults across subtrees. Having both parameter defaults and environmental defaults creates unnecessary precedence competition (Tier 1 default vs. Tier 2 environment vs. Tier 3 ambient children vs. Tier 4 explicit argument).
3. **Combinatorial Signature Bloat:** If a component has 5 optional parameters with defaults, it technically represents $2^5 = 32$ possible invocation shapes, many of which may make no mathematical sense in the layout DAG.

---

### 5.2 Definition-Time Ambiguity Verification vs. Strict Exact-Set Matching

Rather than allowing ambiguous overloads to exist at definition time and relying on complex call-site heuristics or ranking algorithms to disambiguate them, DirectedType enforces a **strict definition-time model**:

> **Definition-Time Ambiguity Rule:**
> If any two overloads could *possibly* be ambiguous for any valid caller invocation, compilation fails immediately at component definition/registration time (`CompileError::PotentiallyAmbiguousOverloads`).

#### The Construction Principle & Mathematical Overlap Condition
When a caller constructs a component, they **must** supply all required ports for that signature, and they **may** supply any optional ports:
$$\text{Req}(O) \subseteq P \subseteq \text{All}(O)$$

If the inputs required to construct overload $O_1$ also form a valid input for overload $O_2$, then supplying those ports is a source of ambiguity.

Formally, an input $P$ valid for both $O_1$ and $O_2$ exists if and only if:
$$\exists P \quad \text{such that} \quad (\text{Req}(O_1) \cup \text{Req}(O_2)) \subseteq P \subseteq (\text{All}(O_1) \cap \text{All}(O_2))$$

This condition holds **if and only if**:
$$\text{Req}(O_1) \subseteq \text{All}(O_2) \quad \text{AND} \quad \text{Req}(O_2) \subseteq \text{All}(O_1)$$

#### Concrete Examples:
Consider two overloads: `\Card(text: String, width: Number)` and `\Card(text: String, height: Number)`.
1. **Ambiguous when differentiating ports (`width` or `height`) have defaults:**
   - $O_1$: `\Card(text: String, width: Number: 0)` ($\text{Req} = \{\text{text}\}$, $\text{All} = \{\text{text}, \text{width}\}$)
   - $O_2$: `\Card(text: String, height: Number: 0)` ($\text{Req} = \{\text{text}\}$, $\text{All} = \{\text{text}, \text{height}\}$)
   - Here, $\text{Req}(O_1) = \{\text{text}\} \subseteq \text{All}(O_2)$ and $\text{Req}(O_2) = \{\text{text}\} \subseteq \text{All}(O_1)$.
   - An invocation `\Card(text: "Hello")` satisfies both!
   - **Result: COMPILE ERROR at component definition time.**
2. **Ambiguous when one overload's required ports are accepted by another:**
   - $O_1$: `\Card(text: String, width: Number: 0)` ($\text{Req} = \{\text{text}\}$, $\text{All} = \{\text{text}, \text{width}\}$)
   - $O_2$: `\Card(text: String)` ($\text{Req} = \{\text{text}\}$, $\text{All} = \{\text{text}\}$)
   - The required ports of $O_2$ ($\text{text}$) are accepted by $O_1$, and $O_1$'s required ports are satisfied by $O_2$.
   - **Result: COMPILE ERROR at component definition time.**
3. **Unambiguous when common ports (`text`) have defaults, but differentiating ports are required:**
   - $O_1$: `\Card(text: String: "Default", width: Number)` ($\text{Req} = \{\text{width}\}$, $\text{All} = \{\text{text}, \text{width}\}$)
   - $O_2$: `\Card(text: String: "Default", height: Number)` ($\text{Req} = \{\text{height}\}$, $\text{All} = \{\text{text}, \text{height}\}$)
   - Here:
     - $\text{Req}(O_1) = \{\text{width}\} \not\subseteq \text{All}(O_2)$ (because $O_2$ does not declare `width`)
     - $\text{Req}(O_2) = \{\text{height}\} \not\subseteq \text{All}(O_1)$ (because $O_1$ does not declare `height`)
   - Any valid invocation of $O_1$ must supply `width`, which $O_2$ rejects. Any valid invocation of $O_2$ must supply `height`, which $O_1$ rejects.
   - **Result: VALID.** There is zero overlap in their valid input spaces.

#### How Eliminating Defaults Collapses the Math to Exact Set Equality
When parameter defaults are eliminated, every parameter in an overload signature is required:
$$\text{Req}(O) = \text{All}(O) = \text{Params}(O)$$
The definition-time ambiguity condition collapses from subset analysis into **exact set equality**:
$$\text{Params}(O_1) \subseteq \text{Params}(O_2) \land \text{Params}(O_2) \subseteq \text{Params}(O_1) \iff \text{Params}(O_1) == \text{Params}(O_2)$$

This guarantees:
1. **Definition Time:** Two overloads are ambiguous if and only if they declare the exact same set of parameter names.
2. **Call Site:** Because all overloads have mutually disjoint parameter sets, call-site resolution is a trivial $O(1)$ set lookup ($\text{Params}(O) == P_{\text{provided}}$) with zero heuristic ranking or ambiguity possible at runtime.

---

### 5.3 How Component Defaults Are Expressed Without Parameter Defaults

Moving away from parameter defaults does not mean component users must specify every single property manually. Instead, defaults are handled through three clearer, more principled mechanisms:

#### 1. Internal `let` Constants Inside Specific Overloads
Common boilerplate configurations are defined as concise overloads that set internal constants:

```dt
// Zero-argument convenience overload: uses standard default styling
\Component Card() {
    let x = 0;
    let y = 0;
    let color = #1e293b;
    let padding_x = 20;
    let padding_y = 20;
    let gap = 12;

    let resolved_width = parent.width - (2 * x);
    alias right = x + resolved_width;
    alias bottom = max(children.bottom) > 0 ? max(children.bottom) + padding_y : y + (2 * padding_y);
    alias height = bottom - y;

    \Rect(x: x, y: y, width: resolved_width, height: height, color: color)
    \Children {
        x: parent.left + padding_x,
        y: prev ? prev.bottom + gap : parent.top + padding_y
    }
}

// Explicit dimensions overload:
\Component Card(width: Number, height: Number) {
    let x = 0;
    let y = 0;
    let color = #1e293b;
    ...
}
```

#### 2. Constructor Delegation / Composition
A simpler overload can delegate to a richer overload or primitive, avoiding code duplication without needing parameter defaults:

```dt
\Component Card(width: Number) {
    \Card(x: 0, y: 0, width: width)
}
```

#### 3. Environmental Cascade (`env`)
Theme properties (colors, corner radii, fonts) naturally belong in the environment:
```dt
\Component Button(label: String) {
    env color: Color;     // Inherited from ambient ThemeProvider
    env font: Font;       // Inherited from ambient Font declaration
    \Rect(color: color) {
        \Text(text: label, font: font)
    }
}
```

---

## 6. Overload Resolution Semantics

Overload resolution is performed statically during AST expansion in `compiler::expand`. It does not exist at runtime and incurs zero overhead in the DAG evaluation loop.

### 6.1 Step 1: Input Port Collection
For an element invocation `\Foo(...)`:
1. Collect the set of **Provided Ports** ($P_{\text{provided}}$):
   - Explicit ports on the invocation (`\Foo(a: 10, b: "hi")` $\rightarrow \{a, b\}$).
   - Ambient ports applied by the parent `\Children` directive (Tier 3).
   - Environmental bindings in scope matching declared `env` parameters (Tier 2).

### 6.2 Step 2: Signature Matching (Exact-Match Model)
With default parameters removed, an overload $O$ matches if and only if:
$$\text{Params}(O) == P_{\text{provided}}$$

If no overload matches $P_{\text{provided}}$, compilation fails immediately with `CompileError::NoMatchingOverload`:
```text
error: No matching overload for component 'Card' with ports {width, padding_x}
  --> src/main.dt:14:5
   |
14 |     \Card(width: 200, padding_x: 16)
   |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
note: Available overloads for 'Card':
      \Component Card()
      \Component Card(width: Number)
      \Component Card(width: Number, height: Number)
      \Component Card(x: Number, y: Number, width: Number, color: Color)
help: Did you mean to use '\Card(width: 200)'?
```

*(Note: During the transition phase, if default parameters are still supported, Model A subset ranking is applied as described in Section 5.2).*

---

## 7. Architectural Changes

### 7.1 `ComponentRegistry`
Currently, `ComponentRegistry` stores a single definition per name:
```rust
// Current
pub struct ComponentRegistry {
    components: HashMap<String, ComponentDef>,
}
```

Update to support overload sets:
```rust
// Proposed
pub struct ComponentRegistry {
    components: HashMap<String, Vec<ComponentDef>>,
}

impl ComponentRegistry {
    /// Registers an overload. Returns an error if an identical signature already exists.
    pub fn register(&mut self, comp: ComponentDef) -> Result<(), CompileError>;

    /// Resolves the exact overload matching the provided port keys.
    pub fn resolve_overload(
        &self,
        name: &str,
        provided_ports: &HashSet<String>,
        span: Span,
    ) -> Result<&ComponentDef, CompileError>;
}
```

### 7.2 Expansion Pipeline (`src/compiler/expand.rs`)
In `expand_element`:
1. Collect provided port names from explicit ports, ambient rules, and env bindings.
2. Query `registry.resolve_overload(comp_name, &provided_ports, span)`.
3. Expand the component using the matching `ComponentDef` template.
4. No sentinel checks, fallback ternaries, or artificial expression pruners are required.

---

## 8. Interaction with Hot-Reloading & State

1. **Topological Invariant Preservation:** Because each overload expands into a strictly verified DAG, switching an overload during live editing (e.g. adding `width: 250`) cleanly re-expands the element's subtree with the newly matched template.
2. **Companion Component State:** Component state variables (`state count: Number: 0;`) operate within the component instance node. Overloads declaring companion state preserve state across live reloads via structured component keys (`key: "button_0"`).

---

## 9. Summary Table: Evolution of Component Sizing & Parameters

| Dimension | Single-Signature with Defaults (Current) | Overloading WITH Parameter Defaults | Overloading WITHOUT Parameter Defaults (Target Goal) |
| :--- | :--- | :--- | :--- |
| **Cycle Hazard** | High (sentinel branches enter DAG unless pruned) | Zero (un-taken branches omitted from AST) | **Zero** (exact topological graphs per overload) |
| **Bidirectional Layout** | Impossible (creates static cycle) | Fully Supported | **Fully Supported** |
| **Candidate Filtering** | N/A (single candidate) | Complex heuristic subset ranking | **Trivial exact set equality ($O(1)$ lookup)** |
| **Default Handling** | Magic numbers (`width: 0`) and parameter defaults | Mixed parameter & env defaults | **Explicit internal `let` constants & `env` cascade** |
| **API Clarity** | Monolithic parameter lists | Multiple signatures with default fallbacks | **Clear, distinct signatures with exact contracts** |
| **Diagnostic Quality** | Cycles surface deep inside component internal formulas | Ambiguous overload errors if defaults overlap | **Immediate, precise `NoMatchingOverload` diagnostics** |

---

## 10. Next Steps & Implementation Roadmap

1. **Phase 1: Multi-Signature Registry & AST Support**
   - Update `ComponentRegistry` to store `Vec<ComponentDef>` per component name.
   - Disallow duplicate parameter sets for the same component name at registration time.
2. **Phase 2: Overload Resolution in Compiler Expansion**
   - Implement port collection and signature matching in `src/compiler/expand.rs`.
   - Add `CompileError::NoMatchingOverload` and `CompileError::DuplicateOverloadSignature`.
3. **Phase 3: Standard Library Transition & Testing**
   - Refactor [`components/Card.dt`](file:///Users/hoefel/dev/directedtype/components/Card.dt) into explicit-width and fluid-width overloads.
   - Refactor [`components/HStack.dt`](file:///Users/hoefel/dev/directedtype/components/HStack.dt) and [`components/VStack.dt`](file:///Users/hoefel/dev/directedtype/components/VStack.dt).
   - Implement multi-mode [`components/Text.dt`](file:///Users/hoefel/dev/directedtype/components/Text.dt) ($W \to H$, $H \to W$, and natural bounds).
4. **Phase 4: Deprecation of Parameter Defaults**
   - Assess whether any standard components genuinely require parameter defaults after overloading is available.
   - Deprecate default parameter syntax in component headers in favor of internal `let` bindings and `env` declarations.
   - Transition candidate filtering to pure exact-set matching.
