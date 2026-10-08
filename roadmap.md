# DirectedType Roadmap

## Active & Upcoming Priorities

### 1. DOM Inspector Provenance & Rule Context
* **Source of Rules Context**: Show formulas in the context of the defining node. For example, when a `VStack` sets the width of children nodes, display the formula with its source variable on the `VStack` rather than appearing as an unbound local on the child.
* **Inheritance Attribution**: Clearly indicate inherited properties and which ancestor established each constraint or rule.
* **Scrollable Tree & Property Panels**: Integrate `ScrollView` into the DOM inspector sidebar and property panels so deep trees and long formula lists can be scrolled cleanly.

### 2. Language & Module Features
* **Named Symbol Imports**: Support explicit imports (e.g. `\use { card_dark, button_primary } from "theme/default.dt"`) rather than implicitly pulling all top-level symbols into scope.
* **Sizing Semantics & `auto`**: Clarify the role of `auto` vs. explicit intrinsic and extrinsic sizing contracts in the algebraic DAG.

### 3. Component Library & Primitives
* **Standard Component Library**: Expand beyond `Button`, `Card`, `Link`, `Center`, and `ScrollView` to include `TextInput`, `Checkbox`, `Slider`, `Toggle`, `Tabs`, `Dialog/Modal`, and navigation bars.
* **Scrollbars**: Add draggable thumb interaction and hover states to the `ScrollView` scrollbars.
* **Graphical Primitives**: Add vector paths, strokes, complex borders, and advanced layout compositions.
* **Showcase Examples**: Build multi-pane documentation readers, split editors, and rich interactive applications.

---

## Completed Milestones

* **Live Hot-Reloading in the Viewer**: Instant layout and visual updates on file change.
* **Spatial Clipping**: `scene.push_layer` rectangular and rounded-corner clipping boundaries.
* **Hit Testing & Reverse-Painter's Dispatch**: Accurate hit testing respecting z-index, paint order, corner radius, and clip boundaries.
* **Declarative Reactive State & Logic**: Component state declarations (`state counter = 0`), single-pass topological DAG re-evaluation, and transaction batching ([docs/state_and_logic.md](file:///Users/hoefel/dev/directedtype/docs/state_and_logic.md)).
* **`#[component]` Procedural Macro Programming Model**: Eliminated manual string-matching dispatch with compile-time reflection, generating typed method dispatch for companion structs with direct access to `Context`, ports, and state.
* **Event Handling & Consumption Conventions**: Events stop propagation by default on the first node with a matching handler; bubbling is opt-in via `event.continue_propagation()`. Completely removed DOM-legacy `propagation_stopped`.
* **Component Constructor Overloading**: Port-signature-based overload selection at compile time, eliminating phantom layout cycles ([docs/overloading.md](file:///Users/hoefel/dev/directedtype/docs/overloading.md)).
* **In-Page Anchors & Hypertext Links**: `\Link`, `\Anchor`, `\AnchorScope`, hierarchical path resolution (`#/scope/target`), keyboard focus traversal (Tab / Shift+Tab), focus/blur styling, and target highlight pulses ([docs/anchors_and_internal_navigation.md](file:///Users/hoefel/dev/directedtype/docs/anchors_and_internal_navigation.md)).
* **`\ScrollView` Component & Typed `ViewHandle` Navigation**: Hardware GPU-clipped scroll container (`components/ScrollView.dt`, `components/ScrollView.rs`), strongly typed `ViewHandle` (`ctx.window().scroll_to(node)` and `view.scroll_to(node)`), `view = window` default port on `\Link`, container-targeted scrolling (`ContextAction::ScrollToNode { target, container }`), and mousewheel event bubbling priority.
* **DOM Inspector Base Implementation**: Interactive inspect mode, element picking, bounding box overlays, and property/formula visualization panel.
