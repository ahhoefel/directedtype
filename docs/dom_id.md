# DirectedType (DTML) Specification: Advanced Node Addressing & Identity Models

This document expands on [ids.md](file:///Users/hoefel/dev/directedtype/docs/ids.md) and [dom.md](file:///Users/hoefel/dev/directedtype/docs/dom.md), outlining four robust strategies for identifying and looking up nodes across the Component DOM without relying on fragile global string IDs.

---

## 1. Component-Scoped / Hierarchical Keys (Paths)

Instead of a single flat document-wide string namespace where names like `submit_btn` or `header` collide when components are instantiated multiple times, IDs can be **lexically scoped** to their enclosing component:

```dtml
\Dialog(id: login_dialog) {
    \TextInput(id: username)
    \Button(id: submit_btn) { "Log In" }
}
```

### Mechanisms
* **Scoped Lookup API:** `dom.get_element_by_id(dialog_handle, "submit_btn")` searches only within the scope of `dialog_handle`.
* **Path-based Navigation:** `dom.get_element("login_dialog/submit_btn")` allows unambiguous deep querying from the root.
* **Component Encapsulation:** Component authors can safely assign local IDs (`header`, `content`, `close_btn`) internally without worrying about collisions with other components or sibling instances.

---

## 2. AST Lexical Bindings (`let` / Named Elements)

DirectedType already includes first-class lexical element bindings in the AST (`Item::Let` and `ComponentBodyItem::Let`):

```dtml
let sidebar = \ScrollView(width: 240) { ... };
```

### Mechanisms
* **Symbol Table Mapping:** Rather than requiring an explicit `id: ...` port in markup, the AST parser and template expander naturally associate `NodeHandle` with the lexical binding's identifier name (`Ident`).
* **Document and Component Scopes:** Variables defined at the document level are directly queryable in the document symbol table, and variables defined inside component bodies are queryable in that component instance's local scope table.

---

## 3. Fragment Parsing with Handle Bindings (Refs)

When instantiating dynamic DTML fragments (e.g., from host scripting, WASM modules, or DevTools), looking up nodes by querying a global registry after creation is error-prone. Instead, fragment parsing can return a typed map of captured references:

```rust
let (root_handle, refs) = dom.parse_fragment_with_refs(r#"
    \Dialog {
        \TextInput(ref: username_input)
        \Button(ref: submit_btn) { "OK" }
    }
"#)?;

let btn: NodeHandle = refs["submit_btn"];
```

### Mechanisms
* **Direct Handle Capture:** In DTML markup, a `ref: <ident>` directive captures the newly created `NodeHandle` directly into an output map returned by the parser.
* **No Global Registry Pollution:** Refs only exist during the fragment construction turn, preventing memory leaks and naming collisions in the main DOM index.

---

## 4. Typed Keys / Interned Atoms

For host-side Rust APIs, WASM guest bindings, or compile-time checked component schemas, string comparisons add runtime overhead and risk silent typos.

### Mechanisms
* **Interned String Atoms:** Use an atom pool (`Atom` / `Symbol`) so identifier comparisons are single integer equality checks ($O(1)$) rather than heap string comparisons.
* **Code-Generated Enums / Keys:** For static layouts or host-driven applications, generate Rust/TypeScript typed enums for known IDs:
  ```rust
  #[derive(Copy, Clone, PartialEq, Eq, Hash)]
  pub enum AppNodeKey {
      MainFlow,
      Sidebar,
      EditorPane,
  }

  dom.get_element_by_key(AppNodeKey::Sidebar) -> Option<NodeHandle>;
  ```
