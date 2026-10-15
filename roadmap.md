# DirectedType Roadmap

## Active & Upcoming Priorities

### 1. DOM Inspector UI & Usability
* **Component Breadcrumbs & Selection Sync**: Add breadcrumb navigation and visual indicators linking the selected node in the visual tree to its entry in the property DAG panel.

### 2. Diagnostics & Language Ergonomics
* **Human-Readable Node & Error Diagnostics**: Replace raw numbered node IDs (`NodeId(1)`) with hierarchical, human-readable DOM/DAG component path references (e.g. `Window/VStack/Card#header/Text`). Add source spans and clear diagnostic explanations to all compiler and runtime errors.
* **Compile-Time DAG Flow Checks & Cycle Diagnostics**: Find ways to compile-time check DAG flow compatibility. For example, does wrapping a top-down sized component with `\PadOut` produce a clear error? Identify component combinations that conflict or need clearer diagnostics explaining why certain component pairs cannot compose.

### 3. Component Library & Primitives
* **Standard Component Library**: Expand standard components beyond `Button`, `Card`, `Link`, `HCenter`, `VCenter`, `PadIn`, `PadOut`, `VStack`, `VStackStretch`, `HStack`, `HStackStretch`, `ScrollView`, `Slider`, and `NavBar` to include `TextInput`, `Checkbox`, `Toggle`, `Tabs`, `Dialog/Modal`, and menus.
* **Graphical Primitives**: Add vector paths, strokes, complex borders, gradients, and custom canvas-like layout compositions.
* **Showcase Applications**: Build multi-pane documentation readers, split editors, and rich interactive applications demonstrating single-pass reactive DAG layout and event flows.

---

## Completed Milestones

* **Resizable Docked DOM Inspector & Side-by-Side Viewport Reflow**: Docked the DOM inspector alongside (rather than overlapping) the main document viewport so `window.width` and horizontal scrollbars reflect the visible canvas area, added incremental `CompiledDocument::resize_window` DAG re-evaluation, and implemented a draggable left border on the inspector panel with hover/active highlights (`CursorIcon::EwResize`), width clamping, and live main-panel layout reflow.
* **Scrollable Tree & Property Panels in DOM Inspector**: Full 2D scrollability (vertical and horizontal) in both the top DOM tree view and bottom component/property details panel. Deep trees, long formula strings, DAG traces, and expanded object references can be inspected without clipping. Supports wheel and trackpad scroll gestures (`delta_x` and `delta_y`), interactive draggable dual scrollbars with hit-testing and track guards preventing phantom row clicks, and automatic selection reset behavior.
* **Elimination of `auto` Sentinel (`\VStackStretch` / `\HStackStretch`)**: Completely eradicated `auto` keywords and sentinels from the grammar, compiler, and component library. Replaced `is_stretch ? width : auto` and compiler `is_auto_expr` with dedicated `\VStackStretch` and `\HStackStretch` components that unconditionally push cross-axis constraints to children.
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

