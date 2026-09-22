# DirectedType (DTML) Specification: Ports & Precedence

In DirectedType (DTML), user interfaces are structured not as arbitrary CSS cascades, but as a mathematical **Directed Acyclic Graph (DAG)**. 

**Ports** are the typed input and output sockets—the "hardware pins"—of components and visual primitives. Connecting an expression to a port establishes a directed edge in the layout graph, evaluated in deterministic topological order.

---

## 1. Defining Component Ports

Component parameter signatures define the inputs accepted by a component. Each parameter can be **required** or **optional**, and can optionally include a **type annotation**.

### Syntax Variants

| Syntax Pattern | Kind | Type Annotation? | Default Value? | Example |
| :--- | :--- | :--- | :--- | :--- |
| `name: Type` | **Required** | Yes | No | `\Component Card(title: String)` |
| `name` | **Required** | No | No | `\Component Card(title)` |
| `name: Type: default` | **Optional** | Yes | Yes | `\Component Flow(gap: Number: 16)` |
| `name: default` | **Optional** | No | Yes | `\Component Flow(gap: 16)` |
| `name: expr` (computed) | **Optional** | No | Yes (Dynamic) | `\Component Box(width: max(children.width) + 32)` |

---

### Type Annotations are Optional

In DirectedType, type annotations (e.g. `: Number`, `: Color`, `: String`) are **optional**. 
What determines whether a port is **required** or **optional** is solely whether a **default expression** is provided:

```dtml
// Required: type annotation present, but no default
\Component ProgressBar(progress_percentage: Number) {
  \Rect(width: progress_percentage, height: 20)
}

// Also Required: no type annotation and no default
\Component SimpleBar(progress_percentage) {
  \Rect(width: progress_percentage, height: 20)
}

// Optional: type annotation present, default is 16
\Component Flow(gap: Number: 16) { ... }

// Also Optional: no type annotation, default is 16
\Component Flow(gap: 16) { ... }
```

---

## 2. Required vs. Optional Ports

### What it Means to be Required

A port is **required** when its component definition does not supply a default fallback value (`default_edge: None`).

When a component is instantiated:
1. The compiler checks whether a value was provided by:
   - An **explicit instance argument** (e.g. `\ProgressBar(progress_percentage: 75)`), OR
   - An **ambient parent container** rule (e.g. `\Children { progress_percentage: 50 }`).
2. If **neither** provides a value, the DirectedType compiler rejects the layout at compile-time with a fatal diagnostic error:
   ```text
   CompileError: Node 'ProgressBar' is missing required port 'progress_percentage'
   ```
3. **Guarantees:** A required port will never evaluate to `null`, `undefined`, or `NaN`. The layout cannot compile if any required pin is left unplugged.

---

### What it Means to be Optional

A port is **optional** when its component definition supplies a default expression (`default_edge: Some(...)`).

Optional ports can be:
- **Static literals**: `gap: Number: 16`, `bg_color: Color: #1e222d`
- **Expressions referencing sibling ports**: `height: width * 0.5`
- **Dynamic intrinsic aggregations**: `width: max(children.width) + 40`

If the consumer of the component does not wire this port, the compiler connects the default expression automatically.

---

## 3. The 3-Tier Precedence Hierarchy

When a port can be provided in multiple places, collisions are resolved deterministically before layout evaluation begins using a strict **3-Tier Precedence Hierarchy**:

```
Tier 3: Explicit Instance Override  (Highest priority)
   ▲
   │  overrides
   │
Tier 2: Ambient Parent Wiring (\Children)
   ▲
   │  overrides
   │
Tier 1: Component Signature Default (Lowest priority)
```

### Tier Breakdown

1. **Tier 1: Component Signature Default (Lowest)**
   The internal default defined in the component's parameter signature.
   *Example:* `\Component Button(width: Number: 100)`

2. **Tier 2: Ambient Parent Wiring**
   A parent container suggests layout rules for all of its injected children using the `\Children` directive.
   *Example:* `\Children { width: parent.width / 2 }`
   *Rule:* **Tier 2 overwrites Tier 1.** The container's ambient layout takes precedence over the child's internal default.

3. **Tier 3: Explicit Instance Override (Highest)**
   The author explicitly wires a port directly on the element tag.
   *Example:* `\Button(width: 300)`
   *Rule:* **Tier 3 overwrites Tier 2 and Tier 1.** The explicit wire manually bypasses both the ambient parent suggestion and the component's internal default.

---

### Step-by-Step Code Example

Consider a `Button` with default `width: 100` and a `Container` suggesting `width: 200`:

```dtml
\Component Button(width: Number: 100) {
  \Rect(width: width, height: 40)
}

\Component Container {
  \Children {
    width: 200
  }
}

// 1. Tier 1 Wins (Component Default):
// Neither a parent nor an explicit argument is present.
// width -> 100
\Button()

\Container {
  // 2. Tier 2 Wins (Ambient Parent):
  // The Container's \Children rule overrides the Button's default 100.
  // width -> 200
  \Button()

  // 3. Tier 3 Wins (Explicit Instance):
  // The explicit argument 300 overrides the Container's 200 and the Button's default 100.
  // width -> 300
  \Button(width: 300)
}
```

---

### Precedence Resolution Matrix

| Component Has Default? | Ambient Parent Specifies Port? | Explicit Instance Specifies Port? | Resolved Value Source |
| :---: | :---: | :---: | :--- |
| **No** (Required) | No | No | ❌ **Compile Error:** `MissingPort` |
| **No** (Required) | Yes (`42`) | No | ✅ **Tier 2:** `42` (Parent satisfied required port) |
| **No** (Required) | Yes (`42`) | Yes (`88`) | ✅ **Tier 3:** `88` (Explicit instance override) |
| **No** (Required) | No | Yes (`75`) | ✅ **Tier 3:** `75` (Explicit instance provided) |
| **Yes** (`100`) | No | No | ✅ **Tier 1:** `100` (Signature default fallback) |
| **Yes** (`100`) | Yes (`200`) | No | ✅ **Tier 2:** `200` (Ambient parent overrides default) |
| **Yes** (`100`) | Yes (`200`) | Yes (`300`) | ✅ **Tier 3:** `300` (Explicit instance overrides all) |

---

## 4. Built-in Spatial Trait Ports on Primitives

All layout elements (e.g. `\Rect`, `\Text`, `\Header`, `\Paragraph`, and component roots) inherit the **Base Spatial Trait**, which exposes 5 standard spatial ports:

- `x`: Left coordinate
- `y`: Top coordinate
- `width`: Horizontal span
- `height`: Vertical span
- `z`: Stacking layer order (Painter's algorithm)

If not explicitly wired or supplied by a parent, primitive elements receive standard ambient defaults:
- **Root elements**: `x: 0`, `y: 0`, `z: 0`, `width: window.width`, `height: 24`
- **Child elements**: `x: parent.left`, `y: parent.top`, `z: parent.z`, `width: parent.width`
- **Text elements (`\Text`)**: `width` measures text intrinsics, and `height` dynamically wraps text across multi-line boundaries via the Parley font engine.
