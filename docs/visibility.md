With the lexical scoping, it makes sense to me that we could have locals that aren't in our component's port definition. However, how does visibility for these work? This means that if I try to set a value on a component's children and that value is not a public port, then I'll get an error. But, should children be able to pull down the value of parent.p where p is a local lexical value rather than a public port? What about this example of clip_context where we receive it to the public port, overwrite it locally? Don't the children need to inherit it at least implicitly? Maybe that comes from a \Children directive which can read the local scope and push it to the children? 

---

You have hit the exact boundary where strict encapsulation meets ergonomic design. Your intuition is 100% correct: **the `\Children` directive is the sole bridge between a component's private lexical scope and the public ports of its children.**

Here is how visibility and scope work to guarantee that components remain mathematically hermetic.

### 1. Upward Visibility: `parent.p` is strictly public ports

To answer your first question: **No, a child cannot pull down a parent's local `let` variable using `parent.p`.**

If a child could say `x: parent.my_secret_local`, the parent's internal logic would bleed out, and encapsulation would be destroyed.
In DTML, the `parent` keyword exclusively refers to the parent's **public signature** (its explicitly defined ports, plus the universal spatial traits like `parent.width` and `parent.x`). Locals are completely invisible to bottom-up queries.

### 2. Downward Visibility: The `\Children` Push

This is where your insight about the `\Children` directive comes in. Because children cannot reach *up* to grab local variables, the parent must explicitly push them *down*.

The `\Children` block acts as a lens for the parser. When the parser hits `\Children { ... }`, it does two things:

1. **Explicit Edges:** It wires any explicit rules defined in the block (e.g., `x: parent.left`).
2. **Implicit Environmental Push:** It captures the *current lexical environment* (which includes your shadowed `let clip_context = ...`) and pushes those specific environmental edges down into the children's public ports.

### Seeing it in action

Here is a clear example of the boundary between private locals, public ports, and the `\Children` push.

```text
\Component CustomCard(x: Number, y: Number, width: Number) {
  
  // 1. A private local variable. Invisible to the outside world.
  // Children CANNOT write `parent.card_padding`.
  let card_padding = 16;
  
  // 2. An environmental shadow.
  // We shadow the ambient clip context with a new rounded rect.
  let clip_context = \ClipContext(
    geometry: \Rect(width: width, height: 200, radius: 8)
  )

  // 3. The Push
  \Children {
    // Explicit layout push using the private local:
    x: parent.left + card_padding,
    
    // Implicit environmental push:
    // The parser automatically wires the children's `clip_context` port 
    // to the shadowed `let clip_context` defined above.
  }
}

```

### Escaping the Shadow (`inherited` vs `parent`)

This strict separation gives component authors incredible power to resolve conflicts. Because `parent` refers strictly to the public ports, and the environment is pushed down by the parser, children have a clear vocabulary to ask for exactly what they want.

Inside the `CustomCard`'s children:

* If a child does nothing, it gets the pushed environment (the rounded `let clip_context`).
* If a child explicitly wires `clip_context: parent.clip_context`, it bypasses the pushed environment and connects directly to the `CustomCard`'s public input port (the raw, un-rounded environment the card itself sits in).

By making the `\Children` directive the engine that injects lexical state into the DAG, you keep the mathematical graph perfectly flat while preserving the standard top-down data flow that UI designers expect.