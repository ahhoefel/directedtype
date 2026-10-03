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

A component may define multiple signatures under the same identifier. Each overload declares a distinct parameter signature, default values, and internal body wiring:

```dt
// ============================================================================
// Card Overload 1: Caller-Constrained Width (Leaf-Driven)
// ============================================================================
\Component Card(
    x: Number: 0,
    y: Number: 0,
    width: Number, // Explicit required width from caller
    color: Color: #1e293b,
    padding_x: Number: 20,
    padding_y: Number: 20,
    gap: Number: 12
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
    x: Number: 0,
    y: Number: 0,
    color: Color: #1e293b,
    padding_x: Number: 20,
    padding_y: Number: 20,
    gap: Number: 12
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
\Component Text(width: Number, text: String, size: Number: 14) {
    alias height = text_height_wrapped(text, size, width);
    ...
}

// Overload B: Natural intrinsic badge/label
\Component Text(text: String, size: Number: 14) {
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

## 5. Overload Resolution Semantics

Overload resolution is performed statically during AST expansion in `compiler::expand`. It does not exist at runtime and incurs zero overhead in the DAG evaluation loop.

### 5.1 Step 1: Input Port Collection
For an element invocation `\Foo(...)`:
1. Collect the set of **Provided Ports** ($P_{\text{provided}}$):
   - Explicit ports on the invocation (`\Foo(a: 10, b: "hi")` $\rightarrow \{a, b\}$).
   - Ambient ports applied by the parent `\Children` directive (Tier 3).
   - Environmental bindings in scope matching declared `env` parameters (Tier 2).

### 5.2 Step 2: Candidate Filtering
An overload definition $O$ with parameter set $\text{Params}(O)$ is a candidate if and only if:
1. **Required Port Satisfaction:** Every required parameter (parameters without a default expression) in $\text{Params}(O)$ is present in $P_{\text{provided}}$.
2. **Port Acceptance:** Every port in $P_{\text{provided}}$ corresponds to a valid parameter declared in $\text{Params}(O)$. If the invocation supplies a port that $O$ does not declare, $O$ is disqualified.

### 5.3 Step 3: Specificity Ranking
If multiple overloads satisfy the candidate criteria, select the most specific overload:
1. **Explicit Match Count:** The overload that matches the highest number of explicitly provided ports without falling back to default parameter values ranks higher.
2. **Exact Parameter Set Match:** An overload whose parameter set exactly matches $P_{\text{provided}}$ beats an overload that relies on optional defaults.
3. **Ambiguity Error:** If two or more candidates have identical specificity, compilation fails with `CompileError::AmbiguousOverload`:
   ```text
   error: Ambiguous overload for component 'Card'
     --> src/main.dt:14:5
      |
   14 |     \Card(padding_x: 10, padding_y: 10)
      |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
   note: Candidate 1: \Component Card(padding_x: Number, padding_y: Number)
   note: Candidate 2: \Component Card(padding_x: Number: 0, padding_y: Number: 0, width: Number: 0)
   help: Provide explicit disambiguating ports or consolidate overload definitions.
   ```

---

## 6. Architectural Changes

### 6.1 `ComponentRegistry`
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
    pub fn resolve_overload(
        &self,
        name: &str,
        provided_ports: &HashSet<String>,
        span: Span,
    ) -> Result<&ComponentDef, CompileError>;
}
```

### 6.2 Expansion Pipeline (`src/compiler/expand.rs`)
In `expand_element`:
1. When encountering a component invocation, query `registry.resolve_overload(comp_name, &provided_ports, span)`.
2. Expand the component using the returned `ComponentDef` template.
3. No sentinel checks or artificial expression pruners are required.

### 6.3 Diagnostic Spans
When an invocation matches no overloads, emit `CompileError::NoMatchingOverload` listing the signatures available in the registry and the ports provided by the caller.

---

## 7. Interaction with Hot-Reloading & State

1. **Topological Invariant Preservation:** Because each overload expands into a strictly verified DAG, switching an overload (e.g. adding `width: 250` during live editing) triggers standard recompilation of that element's subtree.
2. **Companion Component State:** Component state variables (`state count: Number: 0;`) operate within the component instance node. Overloads may declare the same state variables; state migration during live reload uses the existing structured component key system (`key: "button_0"`).

---

## 8. Summary Table: Sizing Workarounds vs. Overloading

| Dimension | Sentinel Ternaries (Current) | `Optional<T>` Runtime Type | Component Overloading (Proposed) |
| :--- | :--- | :--- | :--- |
| **Cycle Hazard** | High (phantom branches enter DAG unless pruned) | High (un-taken branches still present in AST) | **Zero** (un-taken branches do not exist in AST) |
| **Bidirectional Layout** | Impossible (creates static cycle) | Impossible | **Fully Supported** (different overloads for $W \to H$ vs $H \to W$) |
| **Compiler Complexity** | Complex heuristic branch folding in `expand.rs` | Complex runtime unwrap checking | Clean signature pattern matching in `expand.rs` |
| **API Clarity** | Magic numbers (`width: 0`) | Nested optionals | Clear, explicit component signatures |
| **Performance** | Dependency traversal overhead | Runtime conditional evaluation | Zero runtime cost (evaluated purely at compile time) |

---

## 9. Next Steps

1. **Spec & RFC Review:** Align with DirectedType roadmap goals (roadmap.md item 10).
2. **Phase 1 Implementation:** Extend `ComponentRegistry` to group multiple `ComponentDef`s by name.
3. **Phase 2 Implementation:** Implement candidate filtering and resolution algorithm in `src/compiler/expand.rs`.
4. **Standard Library Refactor:**
   - Refactor [`components/Card.dt`](file:///Users/hoefel/dev/directedtype/components/Card.dt) into explicit-width and fluid-width overloads.
   - Refactor [`components/VStack.dt`](file:///Users/hoefel/dev/directedtype/components/VStack.dt) and [`components/HStack.dt`](file:///Users/hoefel/dev/directedtype/components/HStack.dt).
   - Author multi-mode [`components/Text.dt`](file:///Users/hoefel/dev/directedtype/components/Text.dt).
