# DirectedType Roadmap

## Active & Upcoming Priorities

### 1. Diagnostics & Language Ergonomics
* **Human-Readable Node & Error Diagnostics**: Replace raw numbered node IDs (`NodeId(1)`) with hierarchical, human-readable DOM/DAG component path references (e.g. `Window/VStack/Card#header/Text`). Add source spans and clear diagnostic explanations to all compiler and runtime errors.
* **Compile-Time DAG Flow Checks & Cycle Diagnostics**: Find ways to compile-time check DAG flow compatibility. For example, does wrapping a top-down sized component with `\PadOut` produce a clear error? Identify component combinations that conflict or need clearer diagnostics explaining why certain component pairs cannot compose.
* **Ternary Compile-time Specialization**: Is this necessary? What does it allow? Could we do without it?
* Remove duplicate : and = default assignment style. Just pick one.

### 2. Documentation
* **Language & Component Reference**: Public-facing documentation of the language ([docs/language.md](file:///Users/hoefel/dev/directedtype/docs/language.md))—capturing scope variables, implicit functions, and native primitives like `\Text`—and documentation of the individual standard library components (potentially using generated documentation components).
* **Typography Guide**: Write a typography guide mirroring https://webtypography.net/ and following Bringhurst's book. This should be a good test of the language and serve as a tutorial.

### 3. Text Selection & Copying
* **Overview**: Enable native-feeling single-node and cross-node text selection, double/triple-click word/line selection, visual selection highlights, keyboard shortcuts (`Cmd+C` / `Ctrl+C`, `Cmd+A` / `Ctrl+A`, `Escape`), and system clipboard integration across both the document canvas and the DOM Inspector.
* **Phase 1: Retaining Shaped Parley Layouts & Character Hit-Testing (`src/render/scene.rs`)**
  * Store the shaped `parley::Layout<()>` (alongside `y_offset`) inside `CachedTextScene` in `TextSceneCache` so glyph cluster positions, line metrics, and byte ranges are available for hit-testing and geometry queries without re-shaping on mouse move.
  * Expose a helper `hit_test_text_point(layout: &parley::Layout<()>, local_x: f64, local_y: f64) -> Cursor/ByteIndex` using `parley::layout::Cursor::from_point` to map document-space pointer coordinates `(doc_x - node.rect.x, doc_y - (node.rect.y + y_offset))` to a UTF-8 byte offset and grapheme cluster boundary within a `\Text` node.
* **Phase 2: Document Selection Model & Multi-Node Ordering (`src/render/viewer.rs` / `src/selection.rs`)**
  * Define a `TextSelection` state struct holding `anchor: TextPosition { node_id: NodeId, byte_offset: usize }` and `focus: TextPosition { node_id: NodeId, byte_offset: usize }`, plus an `is_dragging: bool` flag.
  * Define canonical document reading order over selectable `\Text` nodes (pre-order DOM traversal or visual `(y, x)` topological order) so dragging across multiple `\Text` nodes selects:
    * `anchor.byte_offset..text.len()` on the start node,
    * `0..text.len()` on all intermediate `\Text` nodes in reading order (respecting clip/scroll container boundaries), and
    * `0..focus.byte_offset` on the end node.
  * Support `selectable: Boolean = true` port on `\Text` (and opt-out on interactive controls like `\Button` labels where clicks should activate rather than select text).
* **Phase 3: Pointer Gestures & Cursor Feedback (`src/render/viewer.rs`)**
  * **I-Beam Cursor**: Switch the window cursor to `CursorIcon::Text` when hovering over a selectable `\Text` node (unless overridden by an ancestor link/pointer cursor or active inspector pick mode).
  * **Click & Drag Selection**: On left `MouseDown` over selectable text (when no interactive ancestor consumes `PointerDown`), set `anchor = focus = hit_pos` and begin selection drag; on `CursorMoved`, update `focus` (clamping to the nearest text node when dragging into whitespace) and auto-scroll the viewport or active `\ScrollView` when dragging near edges.
  * **Multi-Click Granularity**: Track click timestamps and positions to support double-click (word boundary expansion via Unicode word segmentation / whitespace-punctuation boundaries) and triple-click (full line or paragraph `\Text` node selection).
* **Phase 4: Visual Highlight Rendering (`src/render/scene.rs`)**
  * For each `\Text` node intersecting the active `TextSelection`, compute its selected byte range `start_byte..end_byte`.
  * Query `parley::layout::Selection` (or iterate over `layout.lines()` and runs/clusters overlapping `start_byte..end_byte`) to obtain tight per-line bounding rectangles `(x0, y0, x1, y1)`.
  * Render translucent selection highlight rectangles (e.g. `rgba(59, 130, 246, 0.32)`) behind the glyph scene (or as a translucent overlay layer respecting the node's clip stack and scroll offsets) without invalidating the cached glyph `vello::Scene`.
* **Phase 5: Clipboard Integration & Keyboard Shortcuts (`src/render/viewer.rs`)**
  * Add a cross-platform clipboard crate (e.g. `arboard`) to `Cargo.toml`.
  * Handle `Cmd+C` (macOS) / `Ctrl+C` (Linux/Windows): extract the selected UTF-8 slices across all selected `\Text` nodes in reading order (joining distinct block/line `\Text` nodes with `\n`) and write the string to the OS clipboard.
  * Handle `Cmd+A` / `Ctrl+A` (select all selectable text in document), `Escape` (clear active selection), and `Shift + ArrowLeft/ArrowRight` (extend selection by grapheme cluster).
  * Extend selection/copy support to the DOM Inspector details panel so formulas, evaluated values, and component signatures can also be copied directly.

### 4. Component Library & Primitives
* **Standard Component Library**: Expand standard components beyond `Button`, `Card`, `Link`, `HCenter`, `VCenter`, `PadIn`, `PadOut`, `VStack`, `VStackStretch`, `HStack`, `HStackStretch`, `ScrollView`, `Slider`, and `NavBar` to include `TextInput`, `Checkbox`, `Toggle`, `Tabs`, `Dialog/Modal`, and menus.
* **Graphical Primitives**: Add vector paths, strokes, complex borders, gradients, and custom canvas-like layout compositions.
* **Showcase Applications**: Build multi-pane documentation readers, split editors, and rich interactive applications demonstrating single-pass reactive DAG layout and event flows.

---

## Completed Milestones

* **DOM Inspector UI Refinement & Stacked Property DAG Navigation**: Streamlined the DOM Elements, Component Details, and Property panels with unified title bars (`DOM ELEMENTS`, `COMPONENT DETAILS`, `PROPERTY`), right-aligned DOM node indices, constructor overload signatures (e.g. `\VStack(width, height)`), directional `↑`/`↓` dependency indicators, stacked property trace cards when traversing `UPSTREAM` or `DOWNSTREAM` chains, and full Back (`←`) / Forward (`→`) history navigation across Component and Property views.
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
* **In-Page Anchors & Hypertext Links**: `\Link`, `\Anchor`, `\AnchorScope`, hierarchical path resolution (`#/scope/target`), keyboard focus traversal (Tab / Shift+Tab), focus/blur styling, and target highlight pulses ([docs/designs/anchors_and_internal_navigation.md](file:///Users/hoefel/dev/directedtype/docs/designs/anchors_and_internal_navigation.md)).
* **Component Constructor Overloading**: Port-signature-based overload selection at compile time, eliminating phantom layout cycles ([docs/designs/overloading.md](file:///Users/hoefel/dev/directedtype/docs/designs/overloading.md)).
* **Named Symbol Imports**: Explicit selective symbol imports (`\use { symbol [as alias], ... } from "path.dt";`), lexical and component scoping, internal dependency mangling to prevent scope pollution, and informative error handling.
* **Live Hot-Reloading in the Viewer**: Instant layout and visual updates on file change.
* **Spatial Clipping**: `scene.push_layer` rectangular and rounded-corner clipping boundaries.
* **Hit Testing & Reverse-Painter's Dispatch**: Accurate hit testing respecting z-index, paint order, corner radius, and clip boundaries.
* **Declarative Reactive State & Logic**: Component state declarations (`state counter = 0`), single-pass topological DAG re-evaluation, and transaction batching ([docs/designs/state_and_logic.md](file:///Users/hoefel/dev/directedtype/docs/designs/state_and_logic.md)).
* **`#[component]` Procedural Macro Programming Model**: Eliminated manual string-matching dispatch with compile-time reflection, generating typed method dispatch for companion structs with direct access to `Context`, ports, and state.
* **Event Handling & Consumption Conventions**: Events stop propagation by default on the first node with a matching handler; bubbling is opt-in via `event.continue_propagation()`. Completely removed DOM-legacy `propagation_stopped`.
* **DOM Inspector Base Implementation**: Interactive inspect mode, element picking, bounding box overlays, and property/formula visualization panel.

