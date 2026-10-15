# DirectedType Roadmap

## Active & Upcoming Priorities

### 1. DOM Inspector UI & Usability
* **Scrollable Tree & Property Panels**: Integrate `ScrollView` into the DOM inspector sidebar and property panels so deep trees and long formula lists can be navigated without clipping.
* **Component Breadcrumbs & Selection Sync**: Add breadcrumb navigation and visual indicators linking the selected node in the visual tree to its entry in the property DAG panel.

### 2. Diagnostics & Language Ergonomics
* **Human-Readable Node & Error Diagnostics**: Replace raw numbered node IDs (`NodeId(1)`) with hierarchical, human-readable DOM/DAG component path references (e.g. `Window/VStack/Card#header/Text`). Add source spans and clear diagnostic explanations to all compiler and runtime errors.
* **Eliminate `auto` Sentinel**: Replace the remaining `is_stretch ? width : auto` compiler sentinel in `VStack`/`HStack` and `is_auto_expr` check in `expand.rs` with formal conditional port emission in `\Children` or dedicated stretch overloads.
* Find ways to compile time check the dag flow. For instance, does putting a \PadOut around a component that is sized from top-down in the dom give an error? Are there component combinations that should not be allowed but aren't detectable through DAG cycles? Do DAG cycles detect everything, but do we need better error messages about why certain component pairs are incompatible?

### 3. Component Library & Primitives
* **Standard Component Library**: Expand standard components beyond `Button`, `Card`, `Link`, `HCenter`, `VCenter`, `PadIn`, `PadOut`, `ScrollView`, `Slider`, and `NavBar` to include `TextInput`, `Checkbox`, `Toggle`, `Tabs`, `Dialog/Modal`, and menus.
* **Graphical Primitives**: Add vector paths, strokes, complex borders, gradients, and custom canvas-like layout compositions.
* **Showcase Applications**: Build multi-pane documentation readers, split editors, and rich interactive applications demonstrating single-pass reactive DAG layout and event flows.

---

## Completed Milestones

* **Top-Level `\Window` Declaration & Dual Scrollbars**: Declarative `\Window(title, vscroll, hscroll)` configuration, bidirectional scrollbar rendering, draggable thumb interaction, hit-testing, automatic content clamping, and arrow-key scrolling in the Viewer.
* **Directional Padding Primitives (`PadIn` / `PadOut`)**: Replaced ambiguous `\Padding` and implicit "auto" sizing with pure directional primitives: `\PadIn` for top-down/inward parent sizing and `\PadOut` for bottom-up/outward child sizing, with 1D (`padding_x`, `padding_y`) and 2D overloads and single-child assertions.
* **Built-in `only()` Aggregate Function & Content Capping**: Added `only()` child aggregate function with cardinality assertions. Added lower-end content capping (`max(available, only(children))`) to `HCenter` and `VCenter` to prevent content clipping in narrow viewports.
* **Component-Level `\assert` Directives**: Built-in support for `\assert(condition, "error message")` inside component declarations, validating layout constraints and port bounds at compile and evaluation time.
* **String & Number Formatting Primitives**: Added built-in formatting methods on numbers (`to_fixed(digits)` and `to_precision(digits)`), facilitating reactive numeric labels and metric displays.
* **`\Slider` Component & Strongly Typed Callbacks**: Pointer capture engine (`ctx.capture_pointer()`, direct hardware event routing during active capture sessions), point-to-point typed pin callbacks (`Callback<T>`, `EmitCallback<T>`, `self.on_change.emit(ctx, val)`), `FromPayload` reflection in `#[component]` macros for receiver methods, and single-pass atomic state mutation batching across components.
* **Navigation History & Browser `NavBar`**: Browser-style navigation stack and URL/history state synchronization with forward/back navigation and address bar integration.
* **DOM Inspector Provenance & Rule Context**: Authored ambient expressions (`\Children`) are preserved on child nodes with origin attribution (`parent.left + margin`); `build_property_dag_trace` tracks upstream dependency chains and port origin kinds.
* **`\ScrollView` Component & Typed `ViewHandle` Navigation**: Hardware GPU-clipped scroll container (`components/ScrollView.dt`, `components/ScrollView.rs`), strongly typed `ViewHandle` (`ctx.window().scroll_to(node)` and `view.scroll_to(node)`), `view = window` default port on `\Link`, container-targeted scrolling (`ContextAction::ScrollToNode { target, container }`), and mousewheel event bubbling priority.
* **In-Page Anchors & Hypertext Links**: `\Link`, `\Anchor`, `\AnchorScope`, hierarchical path resolution (`#/scope/target`), keyboard focus traversal (Tab / Shift+Tab), focus/blur styling, and target highlight pulses ([docs/anchors_and_internal_navigation.md](file:///Users/hoefel/dev/directedtype/docs/anchors_and_internal_navigation.md)).
* **Component Constructor Overloading**: Port-signature-based overload selection at compile time, eliminating phantom layout cycles ([docs/overloading.md](file:///Users/hoefel/dev/directedtype/docs/overloading.md)).
* **Named Symbol Imports**: Explicit selective symbol imports (`\use { symbol [as alias], ... } from "path.dt";`), lexical and component scoping, internal dependency mangling to prevent scope pollution, and informative error handling.
* **Live Hot-Reloading in the Viewer**: Instant layout and visual updates on file change.
* **Spatial Clipping**: `scene.push_layer` rectangular and rounded-corner clipping boundaries.
* **Hit Testing & Reverse-Painter's Dispatch**: Accurate hit testing respecting z-index, paint order, corner radius, and clip boundaries.
* **Declarative Reactive State & Logic**: Component state declarations (`state counter = 0`), single-pass topological DAG re-evaluation, and transaction batching ([docs/state_and_logic.md](file:///Users/hoefel/dev/directedtype/docs/state_and_logic.md)).
* **`#[component]` Procedural Macro Programming Model**: Eliminated manual string-matching dispatch with compile-time reflection, generating typed method dispatch for companion structs with direct access to `Context`, ports, and state.
* **Event Handling & Consumption Conventions**: Events stop propagation by default on the first node with a matching handler; bubbling is opt-in via `event.continue_propagation()`. Completely removed DOM-legacy `propagation_stopped`.
* **DOM Inspector Base Implementation**: Interactive inspect mode, element picking, bounding box overlays, and property/formula visualization panel.

