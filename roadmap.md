
* Build dom inspector (Completed)
* Build interactivity (events) & State ([docs/state_and_logic.md](file:///Users/hoefel/dev/directedtype/docs/state_and_logic.md))
* Come up with a programming model ([WASM.md](file:///Users/hoefel/dev/directedtype/docs/WASM.md), [docs/state_and_logic.md](file:///Users/hoefel/dev/directedtype/docs/state_and_logic.md))
* Build more primitives (shapes, paths, strokes, borders, layouts, etc.)
* Add optional scrollbars
* Build a library of standard components.
* Build more examples
* Dom inspector needs to show source of rules. For example, using a VStack sets the width of children nodes, but the formula shows "inner_width" which is a variable on the vstack and not on the child, so it's not in the right context. We should show it in the context of the parent node, not the child node. It should also indicate that properties are inherited from parents, and which parent.
* Support overloaded Component constructors with different selections of ports ([docs/overloading.md](docs/overloading.md)). For example, a text component could take a width value and determine its own height, or take its height and determine its width. For another example, you might want variants with padding, padding_x & padding_y, or padding_{top,left,bottom,right}.
* Understand what auto is and whether we want to keep it in the design.
* Named Symbol Imports: Explicitly name symbols pulled in from module imports (e.g. `\use { card_dark, button_primary } from "theme/default.dt"`) rather than implicitly pulling all top-level symbols into scope.

1. Live Hot-Reloading in the Viewer (Completed)
2. Declarative Reactive State & Object-Oriented Rust Components ([docs/state_and_logic.md](file:///Users/hoefel/dev/directedtype/docs/state_and_logic.md))
3. Spatial Clipping (scene.push_layer) (Completed)
4. Hit Testing & Reverse-Painter's Dispatch (Completed)





