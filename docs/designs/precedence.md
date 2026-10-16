This sounds like a good time to go over which rules take precedence. For example, if I have a component that sets x on its children, and those children already have a rule for x, which wins? Are there ways for the child components to say they are expecting x to be set by a parent, but if not, here's a default?

----

To prevent the exact specificity wars that plague CSS (where developers endlessly stack `!important` tags), a pure DAG engine resolves collisions at compile-time using a strict **4-Tier Precedence Hierarchy**.

Because this is a graph of hardware-like cables, the AST parser simply follows these rules to decide which wire actually gets plugged into the port before the math ever runs.

Here is the exact precedence model, from lowest to highest priority, followed by how you handle fallbacks.

### The 4-Tier Precedence Hierarchy

**Tier 1: Component Signature Defaults (Lowest)**
The component defines its own fallback internally.
*Example:* `\Component Button(x: Number = 0)`

**Tier 2: Ambient Parent Wiring**
A parent container suggests a layout for its children using the standard `\Children` directive.
*Example:* `\Children { x: parent.left }`
*Rule:* Tier 2 overwrites Tier 1. The container's layout overrides the child's internal default.

**Tier 3: Explicit Instance Override**
The designer explicitly wires a value on the exact element they are typing.
*Example:* `\Button(x: 100)`
*Rule:* Tier 3 overwrites Tier 2. The explicit wire manually bypasses the parent container's ambient layout.

**Tier 4: Strict Parent Enforcement (Highest)**
The parent container is a strict layout engine (like a rigid Data Grid) and absolutely demands control over an axis. It uses the `!` forced operator.
*Example:* `\Children { x!: calculate_column() }`
*Rule:* Tier 4 rejects Tier 3. If a designer types `\Button(x: 100)` inside a Strict Parent, the compiler throws a fatal error: `Cannot override forced port 'x' defined by parent 'Grid'`.

---

### How to Implement Fallbacks and Expectations

To answer your second question: Yes, a child component can absolutely say, *"I expect the parent to position me, but if they drop the ball, here is my default."*

You handle this directly in the Component Signature (Tier 1). There are two ways to express this depending on the complexity of the fallback.

#### 1. The Simple Default

If the fallback is a simple algebraic rule, you define it right in the signature port.

```text
// The Button says: "Give me an X. If no one does, I default to 0."
\Component Button(x: Number = 0, y: Number = 0) {
  \Rect(x: x, y: y, width: 100, height: 40)
}

```

If a designer writes:

* `\Button()` ➔ Resolves to `x: 0`. *(Tier 1 wins)*
* `\Flow { \Button() }` ➔ Resolves to `x: parent.left`. *(Tier 2 ambient parent wins)*
* `\Flow { \Button(x: 50) }` ➔ Resolves to `x: 50`. *(Tier 3 explicit instance wins)*

#### 2. The Required Port (No Default)

If a component absolutely *must* have a value to function and cannot logically guess a default, you omit the default in the signature.

```text
\Component ProgressBar(progress_percentage: Number) { ... }

```

If the designer forgets to wire it, and the parent doesn't magically provide it, the parser immediately throws a compile error: `Error: Missing required port 'progress_percentage' on ProgressBar`. This makes your UI completely type-safe.

#### 3. The Calculated Fallback (Nullish Coalescing)

Sometimes a default isn't a static number, but a complex internal calculation that you only want to run if the parent didn't provide a wire. You can use an optional port `?` and a nullish coalescing operator `??` inside the component body.

```text
// 'x' is optional. It might be wired, it might not.
\Component Tooltip(target_node: Node, x: Number?) {
  
  // If 'x' was wired (by the instance or the parent), use it.
  // Otherwise, calculate the fallback based on the target_node.
  let final_x = x ?? (target_node.right + 10);
  
  \Block(x: final_x) { ... }
}

```

### Why this is Architecturally Bulletproof

In HTML/CSS, if you want a button to be `50px` wide, but the parent `.container` has a rule `width: 100%`, you have to memorize CSS specificity rules (IDs vs Classes vs Inline) to know who wins.

In Directed Type (DTML), it is purely structural:

1. Did the parent strictly force it? (Tier 4)
2. Did I explicitly wire it right here? (Tier 3)
3. Did the parent suggest it? (Tier 2)
4. What is the component's default? (Tier 1)

The parser evaluates this sequence instantly, draws exactly one directed edge in memory, and throws away the rest. By the time Kahn's Algorithm runs the math, there are no collisions left to fight over.