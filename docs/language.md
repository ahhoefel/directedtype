# DirectedType Language Reference (`.dt`)

DirectedType (`.dt`) is a declarative UI and typography layout language. Rather than executing complicated multi-pass layout algorithms (such as CSS Flexbox or Grid), a DirectedType document compiles into a **Directed Acyclic Graph (DAG)** of mathematical equations that is solved in a single pass.

---

## Table of Contents

1. [Core Concepts & Execution Model](#1-core-concepts--execution-model)
2. [Syntax, Types & Operators](#2-syntax-types--operators)
3. [Scopes & Implicit Variables Reference](#3-scopes--implicit-variables-reference)
4. [Implicit & Built-in Functions Reference](#4-implicit--built-in-functions-reference)
5. [Native Primitives Reference](#5-native-primitives-reference)
   - [`\Window` & Ambient `window`](#51-window--ambient-window)
   - [`\Text` (Typography & Inline Spans)](#52-text-typography--inline-spans)
   - [`\Font` (First-Class Font & Metric Node)](#53-font-first-class-font--metric-node)
   - [`\Rect` (Painted Rectangle)](#54-rect-painted-rectangle)
   - [`\Clip` & `\Box` (Hardware Clipping & Scroll Layers)](#55-clip--box-hardware-clipping--scroll-layers)
   - [`\Anchor` & `\AnchorScope` (Internal Navigation)](#56-anchor--anchorscope-internal-navigation)
6. [Components, Overloading & Directives](#6-components-overloading--directives)
   - [Module Imports (`\use`)](#61-module-imports-use)
   - [Enumerations (`\Enum`)](#62-enumerations-enum)
   - [Component Declarations (`\Component`)](#63-component-declarations-component)
   - [Constructor Overloading](#64-constructor-overloading)
   - [Lexical Bindings (`\let`) vs. Exported Aliases (`\alias`)](#65-lexical-bindings-let-vs-exported-aliases-alias)
   - [Ambient Child Defaults (`\Children`)](#66-ambient-child-defaults-children)
   - [Environmental Scoping (`\env`)](#67-environmental-scoping-env)
   - [Assertions (`\assert`)](#68-assertions-assert)
   - [Reactive State (`\state`), Keys & Events](#69-reactive-state-state-keys--events)

---

## 1. Core Concepts & Execution Model

### Every Property an Equation
Every element in a DirectedType file (`.dt`) is a node in the document tree, and every property on a node (`x`, `y`, `width`, `height`, `color`, custom parameters, etc.) is a symbolic expression in terms of other properties and values.

```dt
\let header = \Text(x: 24, y: 20, size: 28) { "Chapter 1" };
\Text(x: header.left, y: header.bottom + 12, size: 16) {
    "Layout is expressed directly as relationships between properties."
}
```

### Single-Pass Evaluation
1. **Expansion**: Component macros (`\Component`) are expanded into a hierarchy of nodes, and lexical/environmental references (`parent`, `prev`, `self`, `children`, `env`) are rewritten into canonical node-property identifiers (`VarId(NodeId, "property")`).
2. **Dependency Graph & Cycle Detection**: The compiler builds a directed dependency graph across all node ports. If any circular dependency exists (e.g., a parent's `width` depends on `children.width` while a child's `width` simultaneously reads `parent.width`), compilation fails immediately with a cycle diagnostic.
3. **Topological Sort**: All port equations are evaluated in topological order in a single linear pass. When reactive state (`\state`) or `window` dimensions change at runtime, only downstream dependent ports are re-evaluated.

### Coordinate System & Painter's Order
* **Origin**: Top-left `(x: 0, y: 0)`, with `x` increasing to the right and `y` increasing downward, measured in logical pixels.
* **Spatial Box Ports**: Every spatial node exposes `x`, `y`, `width`, and `height`, along with four spatial edge aliases:
  * `left = x`
  * `top = y`
  * `right = x + width`
  * `bottom = y + height`
* **Rendering Order**: Paint primitives (`\Rect`, `\Text`) are rendered using the **Painter's Algorithm**, sorted primarily by their `z` port (ascending, default `0`) and secondarily by their declaration order in the document.

---

## 2. Syntax, Types & Operators

### Comments
```dt
// Single-line comment
/* Multi-line
   block comment */
```

### Data Types & Literals

| Type | Description | Literal Examples |
| :--- | :--- | :--- |
| `Number` | 64-bit IEEE 754 floating-point scalar | `0`, `42`, `3.14159`, `-12.5` |
| `String` | UTF-8 text string | `"Hello"`, `"Inter"` |
| `Boolean` (or `Bool`) | Logical truth value | `true`, `false` |
| `Color` | Hex color literal (`#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`) or color function | `#1e293b`, `#3b82f680`, `rgba(59, 130, 246, 0.5)` |
| `Enum` | Strongly-typed variant of a declared `\Enum` | `Align.Center`, `TextStart.Capital`, `TextEnd.Baseline` |
| `Node` (or Component type) | Reference to an instantiated DOM node (e.g. `\Font`, `\Clip`, `\Box`, or any component) | `header`, `parent`, `prev`, `self`, `window` |

### Operators (Highest to Lowest Precedence)

| Precedence | Operator(s) | Description | Associativity |
| :---: | :--- | :--- | :---: |
| 1 | `( expr )` | Grouping parentheses | — |
| 2 | `target.port`, `target.method(...)` | Port member access, enum variant access, method call | Left-to-right |
| 3 | `-expr`, `!expr` | Unary negation (`Number`), logical NOT (`Boolean`) | Right-to-left |
| 4 | `*`, `/`, `%` | Multiplication, division, remainder (`0.0` on divide-by-zero) | Left-to-right |
| 5 | `+`, `-` | Addition (or string concatenation if either operand is `String`), subtraction | Left-to-right |
| 6 | `<`, `<=`, `>`, `>=` | Numeric relational comparisons (yields `Boolean`) | Left-to-right |
| 7 | `==`, `!=` | Equality and inequality across `Number`, `String`, `Boolean`, `Color`, `Enum` | Left-to-right |
| 8 | `&&` | Logical AND | Left-to-right |
| 9 | `\|\|` | Logical OR | Left-to-right |
| 10 | `cond ? then_expr : else_expr` | Conditional ternary expression | Right-to-left |

> [!TIP]
> **Compile-Time Ternary Specialization**: When a ternary condition `cond` can be statically resolved during expansion (for example, `prev ? prev.bottom + gap : y`, or checking a component parameter like `align == Align.Center`), the compiler prunes the dead branch before building the dependency graph. This prevents unused branches from introducing false dependency cycles.

---

## 3. Scopes & Implicit Variables Reference

Understanding which implicit variables are in scope at each point in a `.dt` file is essential. DirectedType has **four distinct scopes**:

1. **Top-Level Document Scope**: Outside any `\Component`, at the root of a `.dt` file.
2. **Component Body Scope**: Inside `\Component Name(...) { ... }`.
3. **Ambient `\Children` Scope**: Inside a `\Children { ... }` block within a `\Component`.
4. **Element Instance Scope**: Inside the port list `\Elem(port: expr, ...)` or body of an instantiated element.

### 3.1 Summary of Implicit Variables by Scope

| Variable / Keyword | Top-Level Document | `\Component` Signature & Directives (`\let`, `\alias`, `\env`, `\assert`) | `\Children { ... }` Block | Internal Elements in `\Component` | Caller Element Instance `\Elem(...)` | Description |
| :--- | :---: | :---: | :---: | :---: | :---: | :--- |
| `window` | Yes | Yes | Yes | Yes | Yes | The root viewport container node (`NodeId::WINDOW`). |
| `parent` | Resolves to `window` | Enclosing caller parent | **Error** (use bare component properties) | **Error** (use bare component properties) | Immediate parent container | The enclosing parent container in the expanded hierarchy. |
| `self.<port>` | **Error** | Shadowed port or `on_*` method only | Shadowed port or `on_*` method only | Shadowed port on component, or `on_*` method on element | `on_*` method on element only | Uncovers a component port shadowed by a local `\let` or `\env`, or binds an `on_*` event to the target element's own method (`self.act`) rather than the enclosing component's method (`act`). |
| `child` | **Error** | **Error** | **Each child** receiving the default | **Error** | **Error** | Only valid in `\Children { ... }`, where it refers to the specific child node being configured (e.g., `child.width`). |
| `prev` | Previous top-level sibling | — | **Previous sibling** of each child | Previous sibling in component body | Previous sibling in the same block | Refers to the immediately preceding sibling node (or `None` for the first child). |
| `children` | — | Direct children passed by caller | Direct children passed by caller | — | Direct children inside `{ ... }` | Collection pseudo-variable used in aggregates (`sum(children.height)`) and `children.len`. |
| `children.len` | `0` | Number of caller children | Number of caller children | Number of element's children | Number of direct children | Compile-time `Integer` literal equal to the count of direct children. |
| `<param_name>` | — | Component parameter / port | Component parameter / port | Component parameter / port | — | Unqualified parameter and port names (`x`, `y`, `width`, `height`, `left`, `top`, `right`, `bottom`, `clip`, etc.) resolve directly on the enclosing `\Component`. |
| `<let_name>` | Top-level `\let` / `\state` | Component `\let` / `\alias` / `\state` | Component `\let` / `\alias` / `\state` | Component `\let` / `\alias` / `\state` | Enclosing lexical `\let` / `\state` bindings | Lexical variable, reactive state, or named node in scope. |
| `env.<name>` | Top-level `\env` | Bound `\env` in scope | Bound `\env` in scope | Bound `\env` in scope | Bound `\env` in scope | Accesses an environmental variable down the call tree. |

### 3.2 Identifier Resolution & `self` Rules

When you write an unqualified identifier `foo` in an expression, the compiler resolves it in the following strict order without ever implicitly falling back to `parent.foo` or `window.foo`:

1. **Reserved Keywords**: `window`, `parent`, `self`, `child`, `prev`, `children`, `env` (and `true` / `false` / `unset`).
2. **Lexical Scope (`\let`, `\alias`, `\state`, `\env`)**: Any local binding declared in the current component or block (shadowing component parameters and outer lexical scopes).
3. **Enclosing Component Ports & Synthesized Boundaries**: Inside a `\Component` (including signature defaults, `\let`, `\alias`, `\env`, `\assert`, `\Children`, and internal elements), any declared parameter or synthesized port (`left`, `top`, `right`, `bottom`, `clip`) of that component.
4. **Enclosing Component Method (in `on_*` event ports only)**: In an `on_*` callback port (such as `on_click: act`), an unqualified identifier that does not match a local variable or port resolves to the enclosing component's companion method (`Outer.act`).

#### When to Use `self`
Because component ports and parameters resolve directly as bare identifiers everywhere inside a `\Component`, `self` is **disallowed** except in two specific disambiguation cases:
1. **Calling a Contained Component's Own Method in `on_*`**: Inside `\Component Foo() { \Bar(on_click: self.act) }`, bare `act` refers to `Foo.act`, whereas `self.act` refers to `Bar.act`.
2. **Uncovering a Shadowed Port**: When a component port `P` (including `clip`, `left`, `top`, `right`, `bottom`, or a declared parameter) is shadowed by a local `\let P` or `\env P`, `self.P` accesses the component's public port `P` (e.g., `\let clip = \Clip(up: self.clip, ...)` or `\env color = self.color`). Using `self.P` when `P` is **not** shadowed is a compile error.

### 3.3 The `prev` Sibling Recurrence Pattern

Because layout is a static DAG rather than an imperative loop, sequential flow layouts (like vertical or horizontal stacks) use the `prev` keyword inside a ternary expression:

```dt
\Children {
    x: left,
    y: prev ? prev.bottom + gap : top
}
```
* For the **first child** (`index == 0`), `prev` does not exist, so `prev ? ... : ...` statically expands to the `else` branch (`top`).
* For **subsequent children** (`index > 0`), `prev` resolves to the `NodeId` of child `index - 1`, so the expression expands to `child_{i-1}.bottom + gap`.

### 3.4 Spatial Ports & Automatic Boundary Aliases

Native spatial primitives (`\Rect`, `\Text`, `\Circle`, `\Box`, `\Clip`, `\Anchor`, `\AnchorScope`) expose `x`, `y`, `width`, `height`, `z`, `clip`, and the boundary aliases `left`, `top`, `right`, and `bottom` (whereas `\Font` is a non-spatial typographic resource node).

Custom `\Component` definitions do **not** automatically receive `x`, `y`, `width`, `height`, or `z` ports unless declared as parameters or `\alias` bindings—allowing pure data/style structs (such as `\CardStyle` or `\ButtonStyle`) to remain free of spatial ports. When a component **does** declare spatial input ports (and does not already define a matching `\alias`), the compiler automatically synthesizes the corresponding boundary alias ports:

| Synthesized Port | Type | Default Equation | Condition for Synthesis on `\Component` |
| :--- | :--- | :--- | :--- |
| `left` | `Distance` | `x` | `x` is present and `left` is not already defined. |
| `top` | `Distance` | `y` | `y` is present and `top` is not already defined. |
| `right` | `Distance` | `x + width` (or `left + width`) | (`x` or `left`) and `width` are present, and `right` is not already defined. |
| `bottom` | `Distance` | `y + height` (or `top + height`) | (`y` or `top`) and `height` are present, and `bottom` is not already defined. |
| `clip` | `Node` (`\Clip`) | `env.clip` or `parent.clip` or `window.clip` | Present on all spatial primitives and components for hardware clip propagation. |

---

## 4. Implicit & Built-in Functions Reference

DirectedType provides built-in functions and methods that can be called inside any port equation or `\let` / `\alias` expression without importing any module.

### 4.1 Child Collection Aggregate Functions & `children.len`

Inside a container or `\Component`, `children.<port>` expands to the list of `<port>` values across all direct child nodes, and `children.len` evaluates to the integer count of direct children.

| Expression / Function | Return Type | Empty Children (`0` children) | Description |
| :--- | :--- | :--- | :--- |
| `children.len` | `Integer` | `0` | Compile-time count of direct children passed to the component or element. |
| `sum(children.<port>)` | `Number` / `Distance` | `0` | Sums the specified numeric port across all direct children. |
| `max(children.<port>)` | `Number` / `Distance` | `0` | Returns the maximum value of `<port>` across all direct children. |
| `min(children.<port>)` | `Number` / `Distance` | `0` | Returns the minimum value of `<port>` across all direct children. |
| `only(children.<port>)` | `Any` | **Compile/Eval Error** | Asserts that the node has **exactly 1 child** and returns that single child's `<port>`. Fails if `children.len != 1`. |

> [!NOTE]
> `sum(...)`, `max(...)`, and `min(...)` can also be called with arbitrary comma-separated scalar arguments—or a mix of scalars and `children.<port>`:
> ```dt
> \alias width = max(min_width, max(children.width) + 2 * padding);
> ```

### 4.2 Scalar Math Functions

| Function Signature | Return Type | Description |
| :--- | :--- | :--- |
| `min(a, b, ...)` | `Number` | Returns the smallest of one or more numeric arguments. |
| `max(a, b, ...)` | `Number` | Returns the largest of one or more numeric arguments. |
| `sum(a, b, ...)` | `Number` | Returns the sum of all numeric arguments. |
| `clamp(val, min_val, max_val)` | `Number` | Clamps `val` to the inclusive range `[min_val, max_val]`. |
| `round(val)` | `Number` | Rounds `val` to the nearest integer (`f64::round`). |
| `floor(val)` | `Number` | Rounds `val` down toward $-\infty$ (`f64::floor`). |
| `ceil(val)` | `Number` | Rounds `val` up toward $+\infty$ (`f64::ceil`). |
| `abs(val)` | `Number` | Returns the absolute value $\|val\|$ (`f64::abs`). |

### 4.3 Number Formatting Functions & Methods

Formatting helpers convert numbers into formatted `String` values for dynamic labels (e.g., slider readouts or metrics) and can be invoked either as **methods** on a `Number` or as **standalone functions**:

| Method Call | Function Call | Return Type | Description |
| :--- | :--- | :--- | :--- |
| `val.to_fixed(decimals)` | `to_fixed(val, decimals)` | `String` | Formats `val` with fixed decimal places (`0`–`20`), e.g. `(3.14159).to_fixed(2)` $\rightarrow$ `"3.14"`. |
| `val.to_precision(sig_figs)` | `to_precision(val, sig_figs)` | `String` | Formats `val` to `sig_figs` (`1`–`21`) significant digits, e.g. `(123.456).to_precision(4)` $\rightarrow$ `"123.5"`. |

### 4.4 Color Constructor Functions

| Function Signature | Return Type | Parameter Ranges | Description |
| :--- | :--- | :--- | :--- |
| `rgb(r, g, b)` | `Color` | `r, g, b`: `0`–`255` | Constructs an opaque RGB color. |
| `rgba(r, g, b, a)` | `Color` | `r, g, b`: `0`–`255`, `a`: `0.0`–`1.0` | Constructs an RGBA color with alpha transparency. |
| `hsl(h, s, l)` | `Color` | `h`: `0`–`360`, `s, l`: `0`–`100` (or `0.0`–`1.0`) | Constructs an opaque HSL color. |
| `hsla(h, s, l, a)` | `Color` | `h`: `0`–`360`, `s, l`: `0`–`100` (or `0.0`–`1.0`), `a`: `0.0`–`1.0` | Constructs an HSLA color with alpha transparency. |

### 4.5 Typography & Font Measurement Functions

While `\Text` and `\Font` nodes automatically wire these functions into their default port equations, you can also call them directly in custom layout calculations:

| Function Signature | Return Type | Description |
| :--- | :--- | :--- |
| `text_width(text, size, weight, family)` | `Number` | Measures the single-line unwrapped advance width of `text` shaped with Parley. |
| `text_height(text, size, weight, family, max_width, ends_at, start_at, line_height)` | `Number` | Measures the multi-line wrapped height of `text` constrained to `max_width`, cropped at `start_at` (`TextStart`) and `ends_at` (`TextEnd`). Pass `0` for `line_height` to use the font's natural line height. |
| `text_baseline_offset(size, weight, family, start_at)` | `Number` | Distance from the top of the text box (`y`, determined by `start_at`) to the first typographic baseline. |
| `font_cap_height(size, weight, family)` | `Number` | Exact height of capital flat-topped letters (e.g. `'H'`) for the font. |
| `font_x_height(size, weight, family)` | `Number` | Exact height of lowercase `'x'` for the font. |
| `font_ascent(size, weight, family)` | `Number` | Font ascender metric above the baseline. |
| `font_descent(size, weight, family)` | `Number` | Font descender metric below the baseline (positive number). |
| `font_line_height(size, weight, family)` | `Number` | Default single-line height (`ascent + descent + leading`). |

### 4.6 Scope Path Methods

| Method Signature | Return Type | Description |
| :--- | :--- | :--- |
| `path.append(segment)` | `String` | Appends a relative anchor scope path segment to an existing scope path string (joining with `"/"`). |

---

## 5. Native Primitives Reference

Native primitives are built directly into the DirectedType compiler and renderer. Unlike user components defined with `\Component`, primitives are the leaf building blocks of the scene graph.

---

### 5.1 `\Window` & Ambient `window`

Every `.dt` document has an implicit root viewport node named `window`. You can optionally configure window-level metadata and scrollbars at the top level of a document using `\Window(...)`:

```dt
\Window(
    title: "Typography Specimen",
    vscroll: true,
    hscroll: false
)
```

#### Ports on `window`

| Port | Kind | Type | Default | Description |
| :--- | :--- | :--- | :--- | :--- |
| `window.x` / `window.left` | Output | `Number` | `0` | Always `0`. |
| `window.y` / `window.top` | Output | `Number` | `0` | Always `0`. |
| `window.width` | Output | `Number` | Viewport width | Current logical width of the document viewport (updates live on window or inspector resize). |
| `window.height` | Output | `Number` | Viewport height | Current logical height of the document viewport. |
| `window.right` | Output | `Number` | `window.width` | Right edge of the viewport. |
| `window.bottom` | Output | `Number` | `window.height` | Bottom edge of the viewport. |
| `window.title` | Input (`\Window`) | `String` | `""` | Native OS window title. |
| `window.vscroll` | Input (`\Window`) | `Boolean` | `true` | Enables or disables root vertical scrolling and scrollbar. |
| `window.hscroll` | Input (`\Window`) | `Boolean` | `true` | Enables or disables root horizontal scrolling and scrollbar. |
| `window.clip` | Output | `Node` | `NodeId::WINDOW` | Root unclipped viewport clip reference. |

---

### 5.2 `\Text` (Typography & Inline Spans)

`\Text` shapes, wraps, and renders OpenType text using Parley and Vello. It provides first-class typographic box trimming (`start_at` and `ends_at`) and baseline-grid alignment (`baseline`).

```dt
\let body_font = \Font(family: "Georgia", size: 18, weight: 400, line_height: 27);

\Text(
    x: 40,
    y: 60,
    width: 480,
    font: body_font,
    color: #1e293b,
    start_at: TextStart.Capital,
    ends_at: TextEnd.Baseline
) {
    "Typography is the craft of endowing human language with a durable visual form."
}
```

#### Input & Output Ports on `\Text`

| Port | Kind | Type | Default Equation / Value | Description |
| :--- | :--- | :--- | :--- | :--- |
| `x` | Required Input | `Number` | `parent.left` (if provided via `\Children`) | Left coordinate of the text box. |
| `y` | Input / Computed | `Number` | `baseline - text_baseline_offset(...)` (if `baseline` is set) | Top coordinate of the text box. Either `y` or `baseline` must be provided. |
| `baseline` | Input / Computed | `Number` | `y + text_baseline_offset(size, weight, family, start_at)` | Y-coordinate of the **first line's typographic baseline**. Supplying `baseline` instead of `y` positions the text by its baseline! |
| `font` | Input (or via `\env font`) | `\Font` or `String` | Ambient `\env font` (if bound) | Reference to a `\Font` node (or a font family name string). |
| `size` | Required Input (if no `font`) | `Number` | `font.size` (or parent `\Text.size` for inline spans) | Font size in logical pixels. |
| `weight` | Optional Input | `Number` | `font.weight` or `400` | OpenType font weight (`100`–`900`, `400` = normal, `700` = bold). |
| `family` | Optional Input | `String` | `font.family` or `""` (system default) | Font family name (validated against available system/registered fonts). |
| `line_height` | Optional Input | `Number` | `font.line_height` (if authored on `\Font`) | Explicit absolute line height in logical pixels. |
| `width` | Optional Input / Computed | `Number` | `text_width(text, size, weight, family)` | If omitted, shrink-wraps to single-line advance width. If provided, wraps lines to `width`. |
| `height` | Optional Input / Computed | `Number` | `text_height(text, size, weight, family, width, ends_at, start_at, line_height)` | Automatically computed from wrapped line count and vertical trim settings (`start_at` / `ends_at`). |
| `text` (or `content`) | Optional Input | `String` / `Number` / `Bool` | Body `{ "..." }` text content | Dynamic text expression (overrides static `{ "..." }` body when set). |
| `color` (or `text_color`) | Optional Input | `Color` | `#000000` | Fill color of the rendered glyphs. |
| `align` (or `text_align`) | Optional Input | `String` / `Align` | `"left"` (`Alignment::Start`) | Horizontal alignment within `width`: `"left"`, `"center"`, `"right"`, or `"justify"`. |
| `underline` | Optional Input | `Boolean` | `false` | Draws an underline across the text run. |
| `text_bounds` | `env` Input | `\TextBounds` | `\TextBounds(start_at: TextStart.Capital, end_at: TextEnd.Baseline)` | Controls vertical metric trimming (`start_at` and `end_at`) of the text bounding box; resolved via lexical `\env text_bounds` or explicit port. |
| `start_at` | Output (delegated) | `TextStart` | `text_bounds.start_at` | Top edge (`y`) metric anchor of the text bounding box. |
| `end_at` | Output (delegated) | `TextEnd` | `text_bounds.end_at` | Bottom edge (`bottom`) metric anchor of the text bounding box. |
| `font.cap_height`, `cap_height` | Output (delegated) | `Number` | `font_cap_height(...)` | Cap height of the text node's active font. |
| `font.x_height`, `x_height` | Output (delegated) | `Number` | `font_x_height(...)` | Lowercase x-height of the text node's active font. |
| `font.ascent`, `ascent` | Output (delegated) | `Number` | `font_ascent(...)` | Ascender height of the text node's active font. |
| `font.descent`, `descent` | Output (delegated) | `Number` | `font_descent(...)` | Descender depth of the text node's active font. |

#### Vertical Metric Trimming (`\TextBounds`, `TextStart`, and `TextEnd`)

By default, `\Text` boxes in DirectedType are tightly trimmed to the **cap height** at the top and the **baseline** at the bottom (`\TextBounds(start_at: TextStart.Capital, end_at: TextEnd.Baseline)`), eliminating the arbitrary half-leading whitespace that plagues CSS typography.

You can override vertical metric trimming for an entire subtree using `\env text_bounds = \TextBounds(...)`, or per-node via `text_bounds: \TextBounds(...)`:

* **`TextStart` Enum** (controls where `y` / `top` sits on the first line via `start_at`):
  * `TextStart.Capital` *(default)*: `top` aligns with the flat top of capital letters (`cap_height`).
  * `TextStart.Ascender`: `top` aligns with the font's ascender line (`ascent`).
  * `TextStart.EmTop`: `top` aligns with the top of the em-square.
  * `TextStart.GapTop`: `top` includes the upper half of the line gap.
* **`TextEnd` Enum** (controls where `bottom` sits on the last line via `end_at`):
  * `TextEnd.Baseline` *(default)*: `bottom` aligns with the last line's typographic baseline.
  * `TextEnd.Descender`: `bottom` includes the last line's descender depth (`descent`).
  * `TextEnd.EmBottom`: `bottom` aligns with the bottom of the em-square.
  * `TextEnd.GapBottom`: `bottom` includes the lower half of the line gap.

#### Inline Rich Text Spans
A `\Text` block can mix string literals and inline child `\Text` or `\Link` elements inside its `{ ... }` content block:

```dt
\Text(x: 20, y: 20, width: 400, size: 16) {
    "Read the "
    \Link(url: "#/guide/intro") { "Introduction" }
    " or highlight "
    \Text(color: #dc2626, underline: true) { "important terms" }
    " inline."
}
```
Inline child nodes inherit `size` and `font` from their parent `\Text` node and are shaped as continuous inline glyph runs by Parley.

---

### 5.3 `\Font` (First-Class Font & Metric Node)

`\Font` is a non-rendering node that encapsulates font selection and exposes exact OpenType typographic metrics as reactive ports in the DAG.

```dt
\let heading_font = \Font(
    family: "Helvetica",
    size: 32,
    weight: 700,
    line_height: 40
);

// Use font metrics directly in layout equations!
\Rect(
    x: 20,
    y: 20,
    width: 6,
    height: heading_font.cap_height,
    color: #3b82f6
)
\Text(x: 34, y: 20, font: heading_font) { "Aligned to Cap Height" }
```

#### Ports on `\Font`

| Port | Kind | Type | Default Equation / Value | Description |
| :--- | :--- | :--- | :--- | :--- |
| `size` | **Required Input** | `Number` | — | Font size in logical pixels. |
| `weight` | Optional Input | `Number` | `400` | Font weight (`100`–`900`). |
| `family` | Optional Input | `String` | `""` | Font family name. |
| `line_height` | Optional Input / Output | `Number` | `font_line_height(size, weight, family)` | Explicit line height if authored, or natural font line height. |
| `cap_height` | Output | `Number` | `font_cap_height(size, weight, family)` | Capital letter height above baseline. |
| `x_height` | Output | `Number` | `font_x_height(size, weight, family)` | Lowercase `'x'` height above baseline. |
| `ascent` | Output | `Number` | `font_ascent(size, weight, family)` | Ascender metric above baseline. |
| `descent` | Output | `Number` | `font_descent(size, weight, family)` | Descender metric below baseline. |

---

### 5.4 `\Rect` (Painted Rectangle)

`\Rect` renders a filled and/or stroked rectangle (with optional rounded corners).

```dt
\Rect(
    x: 20,
    y: 20,
    width: 240,
    height: 120,
    color: #f8fafc,
    radius: 8,
    border_width: 1,
    border_color: #cbd5e1
)
```

#### Ports on `\Rect`

| Port | Kind | Type | Default | Description |
| :--- | :--- | :--- | :--- | :--- |
| `x` | **Required Input** | `Number` | — | Left coordinate. |
| `y` | **Required Input** | `Number` | — | Top coordinate. |
| `width` | **Required Input** | `Number` | — | Width of the rectangle. |
| `height` | **Required Input** | `Number` | — | Height of the rectangle. |
| `color` (or `bg_color`) | **Required Input** | `Color` | — | Fill color (use `#00000000` for transparent fill). |
| `radius` (or `corner_radius`) | Optional Input | `Number` | `0` | Corner rounding radius in logical pixels. |
| `border_width` (or `stroke_width`) | Optional Input | `Number` | `0` | Border stroke thickness in logical pixels. |
| `border_color` (or `stroke_color`) | Optional Input | `Color` | `#000000` | Border stroke color. |
| `z` | Optional Input | `Number` | `0` | Stacking order. |
| `clip` | Optional Input | `Node` | Inherited `clip` | Clipping node reference. |

---

### 5.5 `\Clip` & `\Box` (Hardware Clipping & Scroll Layers)

`\Clip` defines a GPU clipping layer (rectangular or rounded-rectangle) and an optional hardware scroll translation `(scroll_x, scroll_y)`.

> [!IMPORTANT]
> Because `scroll_x` and `scroll_y` live on the `\Clip` node rather than in the `x`/`y` equations of child nodes, scrolling a `\ScrollView` translates the cached Vello sub-scene directly on the GPU without re-evaluating child layout equations or re-shaping text!

```dt
\let viewport_clip = \Clip(
    up: self.clip,
    box: \Box(x: x, y: y, width: width, height: height, radius: 8),
    scroll_x: 0,
    scroll_y: scroll_y
);
```

#### Ports on `\Clip`

| Port | Kind | Type | Default | Description |
| :--- | :--- | :--- | :--- | :--- |
| `box` | **Required Input** | `Node` (e.g. `\Box`) | — | Node whose `x`, `y`, `width`, `height`, and optional `radius` define the clip boundary. |
| `up` | Optional Input | `Node` (`\Clip`) | `env.clip` or `parent.clip` or `window.clip` | Parent clip layer in the clip hierarchy, enabling nested scroll views and clips. |
| `scroll_x` | Optional Input | `Number` | `0` | Horizontal scroll offset applied to all nodes referencing this clip. |
| `scroll_y` | Optional Input | `Number` | `0` | Vertical scroll offset applied to all nodes referencing this clip. |
| `x`, `y`, `width`, `height` | Output | `Number` | Delegated to `box.<port>` | Bounding box of the clip region. |

#### `\Box` (Geometric Helper Node)
Any non-painting element name (conventionally `\Box(x: ..., y: ..., width: ..., height: ...)`) acts as a pure geometric node in the DAG without emitting paint commands.

---

### 5.6 `\Anchor` & `\AnchorScope` (Internal Navigation)

`\Anchor` marks a named scroll destination in the document, and `\AnchorScope` creates a hierarchical namespace for anchors (similar to filesystem directories), enabling reusable components to define local anchors without global name collisions.

```dt
\AnchorScope(name: "chapter1") {
    \VStack(width: 600, gap: 16) {
        \Anchor(name: "intro") {
            \Text(size: 24) { "1.1 Introduction" }
        }
        // Navigate via relative ("intro") or absolute ("#/chapter1/intro") URL:
        \Link(url: "#/chapter1/intro") { "Permalink to 1.1" }
    }
}
```

#### Ports on `\Anchor` & `\AnchorScope`

| Port | Kind | Type | Default | Description |
| :--- | :--- | :--- | :--- | :--- |
| `name` | **Required Input** | `String` | — | Local identifier for the anchor or scope segment. |
| `path` | Output (`\AnchorScope` only) | `String` | Canonical `/`-joined scope path | Full resolved path of the scope in the document's `ScopeTree`. |
| `x`, `y` | Optional Input / Forwarded | `Number` | Forwarded to child | Child's `(x, y)` defaults to the anchor's `(x, y)`. |
| `width`, `height` | Optional Input / Computed | `Number` | `child.width`, `child.height` | Inherited from the wrapped child if not explicitly set. |

---

## 6. Components, Overloading & Directives

### 6.1 Module Imports (`\use`)

Files can import components, enums, and top-level bindings from other `.dt` files using relative paths from the workspace root:

```dt
// 1. Wildcard import (brings all public symbols into scope)
\use "components/VStack.dt"

// 2. Namespaced wildcard import (access as Layout.VStack)
\use "components/VStack.dt" as Layout

// 3. Selective named import with optional aliasing
\use { VStack, VStackStretch as StretchStack } from "components/VStack.dt";
```

---

### 6.2 Enumerations (`\Enum`)

Declare strongly-typed enumerations at the top level of a `.dt` file:

```dt
\Enum Align {
    Left,
    Center,
    Right
}
```
Variants are accessed with dot syntax (`Align.Center`) and compared using `==` and `!=`.

---

### 6.3 Component Declarations (`\Component`)

Components encapsulate reusable layout topologies, state, and subtrees.

```dt
\Component CardHeader(
    width: Number,                  // Required parameter (no default)
    x: Number = parent.left,        // Optional parameter (with default)
    y: Number = parent.top,         // Note: `: default` and `= default` are both valid
    padding: Number = 16,
    bg: Color = #ffffff
) {
    \let inner_width = width - 2 * padding;

    \Rect(x: x, y: y, width: width, height: height, color: bg, radius: 8)

    \Children {
        x: x + padding,
        y: y + padding,
        width: inner_width
    }

    \alias height = only(children.height) + 2 * padding;
    \alias right = x + width;
    \alias bottom = y + height;
}
```

* **Required Parameters**: Parameters without a default expression (e.g., `width: Number`) must be supplied by the caller (or inherited via `\Children` from the caller's parent).
* **Optional Parameters**: Parameters with a default expression (`= expr` or `: expr`) use that default if omitted by the caller.

---

### 6.4 Constructor Overloading

In a single-pass DAG layout engine, a component whose `width` flows **top-down** from its parent (`width: Number`) has the opposite dependency direction from one whose `width` flows **bottom-up** from its children (`\alias width = max(children.width)`).

DirectedType solves this cleanly through **compile-time constructor overloading**. You can define multiple `\Component` declarations with the same name as long as their sets of **required parameters** are distinct:

```dt
// Overload 1: Fixed-width & Fixed-height VStack -> \VStack(width, height)
\Component VStack(
    width: Number,
    height: Number,
    x: Number = parent.left,
    y: Number = parent.top,
    gap: Number = 0
) { ... }

// Overload 2: Fixed-width, intrinsic bottom-up height -> \VStack(width)
\Component VStack(
    width: Number,
    x: Number = parent.left,
    y: Number = parent.top,
    gap: Number = 0
) {
    \alias height = sum(children.height) + max(0, children.len - 1) * gap;
    ...
}

// Overload 3: Intrinsic shrink-wrapped width & height -> \VStack(shrink)
\Component VStack(
    shrink: Boolean,
    x: Number = parent.left,
    y: Number = parent.top,
    gap: Number = 0
) {
    \alias width = max(children.width);
    \alias height = sum(children.height) + max(0, children.len - 1) * gap;
    ...
}
```

When a caller instantiates `\VStack(...)`, the compiler inspects the provided ports (plus any ambient ports supplied by the parent's `\Children` block and active `\env` bindings) and selects the unique matching overload. The selected overload signature (e.g. `\VStack(width)`) is displayed at the top of the Component Details panel in the DOM Inspector.

---

### 6.5 Lexical Bindings (`\let`) vs. Exported Aliases (`\alias`)

Inside a `\Component`, there is a strict distinction between private internal variables and public output ports:

| Directive | Visibility outside Component | Can Bind | Purpose |
| :--- | :--- | :--- | :--- |
| `\let name = expr;` | **Private** (internal to lexical scope) | Scalar expression **or** a DOM node (`\let bg = \Rect(...)`) | Intermediate helper formulas or named internal child nodes. |
| `\alias name = expr;` | **Public Output Port** (accessible as `comp.name` and `parent.name`) | Scalar or node reference expression | Exposes a computed property (such as intrinsic `width`, `height`, `right`, `bottom`) as a public port on the component instance. |

---

### 6.6 Ambient Child Defaults (`\Children`)

A component places caller-provided children into its hierarchy using `\Children`. If `\Children` includes a `{ port: expr, ... }` block, those port equations are injected as **default ports** onto every direct child node that does not explicitly override them:

```dt
\Children {
    x: left,
    y: prev ? prev.bottom + gap : top
}
```

Inside a `\Children { ... }` block:
* The enclosing component's parameters, `\let`s, `\alias`es, `\state`s, and synthesized ports (`left`, `top`, `right`, `bottom`, `clip`) are referenced directly as **bare identifiers** (`parent` is not allowed inside `\Children`, and `self.<port>` is only allowed when `<port>` is shadowed by a local `\let` or `\env`).
* `child` refers to the individual child receiving the port (allowing alignment formulas like `left + (width - child.width) / 2`). `child` is **only** valid inside `\Children { ... }`.
* `prev` refers to the child's immediately preceding sibling.

---

### 6.7 Environmental Scoping (`\env`)

Environmental variables (`\env`) pass contextual styling or configuration (such as `\Font`, `ButtonStyle`, `CardStyle`, or `LinkStyle`) down the component tree without manual prop-drilling through intermediate layout containers.

1. **Providing an `\env` binding** (at top level or inside a component):
   ```dt
   \env font = \Font(family: "Inter", size: 16, weight: 400);
   \env card_style = \CardStyle();
   ```
2. **Consuming an `\env` binding in a component signature**:
   ```dt
   \Component Label(
       \env font: Font,
       x: Number = parent.left,
       y: Number = parent.top
   ) {
       \Text(x: x, y: y, font: font) { \Children }
   }
   ```
   A caller can still override an `\env` parameter explicitly at the call site: `\Label(font: custom_font) { "Custom" }`.
3. **Blocking/Unsetting an `\env` binding**:
   ```dt
   \env font = unset; // Prevents descendants from implicitly inheriting `font`
   ```

---

### 6.8 Assertions (`\assert`)

Components can enforce layout invariants and cardinality constraints using `\assert`:

```dt
// Function-style syntax:
\assert(children.len == 1, "PadIn requires exactly one child element.");

// Statement-style syntax:
\assert padding >= 0 as "Padding must be non-negative.";
```
If the boolean condition evaluates to `false` during expansion or DAG evaluation, compilation halts with an `AssertionFailed` diagnostic pointing to the assertion's source span.

---

### 6.9 Reactive State (`\state`), Keys & Events

#### Declaring Reactive State (`\state`)
Components can declare mutable reactive state variables with `\state`:

```dt
\state hovered: Boolean = false;
\state value: Number = 0.5;
```
State variables act as source nodes in the DAG. When mutated by an event handler at runtime, the engine re-evaluates only the downstream ports connected to that state variable.

#### Component Identity Keys (`key;`)
To preserve state across dynamic re-compilations or list reordering, an element instantiation can specify an identity key before a semicolon `;` at the start of its port list:

```dt
\Button("save-btn"; x: 20, y: 40) { "Save" }
```
> [!WARNING]
> Identity key expressions must be pure identity values (literals, non-spatial parameters, or `\state` variables) and **cannot** depend on spatial layout ports (`x`, `y`, `width`, `height`, `left`, `top`, `right`, `bottom`, `z`, `clip`) or `\let` layout formulas.

#### Event Handler Ports & Companion Rust Structs (`#[component]`)
Interactive behavior is implemented in a companion Rust struct annotated with `#[component]` (e.g., `components/Button.rs` alongside `components/Button.dt`). In `.dt`, event ports (`on_click`, `on_scroll`, `on_pointer_down`, `on_pointer_move`, `on_pointer_up`, `on_pointer_enter`, `on_pointer_leave`, `on_focus`, `on_blur`) bind DOM events to companion methods:
* **Unqualified method name (`on_click: act`)**: Binds to the `act` method on the **enclosing `\Component`** being defined.
* **Qualified `self.<method>` (`on_click: self.act`)**: Binds to the `act` method on the **target component instance** itself (for example, when `\Component Foo() { \Bar(on_click: self.act) }` invokes `Bar.act` instead of `Foo.act`, or in a component's own signature default).

```dt
\Component Slider(
    width: Number,
    min: Number = 0,
    max: Number = 100,
    initial: Number = 50,
    on_change = unset,
    on_pointer_down = self.pointer_down,
    on_pointer_move = self.pointer_move,
    on_pointer_up = self.pointer_up
) {
    \state value: Number = initial;
    ...
}
```


