use crate::compiler::expanded::NodeId;

/// A 2D point in logical pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// The result of a spatial hit test on a `ResolvedLayout`.
#[derive(Debug, Clone, PartialEq)]
pub struct HitTestResult {
    /// The leaf visual paint primitive directly under the cursor.
    pub target: NodeId,

    /// The hit point in global window coordinates.
    pub global_point: Point,

    /// The hit point translated into the hit node's local coordinate space: (x - node.x, y - node.y).
    pub local_point: Point,

    /// The chain of ancestor nodes from the hit node up to the root, representing the bubble path.
    /// `bubble_path[0]` is the leaf primitive, `bubble_path[1]` is its enclosing component, etc.
    pub bubble_path: Vec<NodeId>,
}

/// Mouse buttons for pointer events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u16),
}

/// Keyboard modifier keys active during an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

/// Specific type of pointer/interaction event.
#[derive(Debug, Clone, PartialEq)]
pub enum EventKind {
    PointerDown { button: MouseButton },
    PointerUp { button: MouseButton },
    PointerMove,
    Click { button: MouseButton },
    PointerEnter,
    PointerLeave,
    Scroll { delta_x: f64, delta_y: f64 },
    Focus,
    Blur,
}

/// A high-level DirectedType interaction event.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub kind: EventKind,
    pub global_point: Point,
    pub local_point: Point,
    pub modifiers: Modifiers,

    /// The original leaf node that was hit.
    pub target: NodeId,

    /// The node currently receiving the event during bubbling.
    pub current_target: NodeId,

    /// The chain of ancestor nodes from the hit node up to the root.
    pub bubble_path: Vec<NodeId>,

    /// Flag requesting event propagation to continue to the next ancestor node.
    pub propagation_continued: bool,
}

impl Event {
    pub fn new(
        kind: EventKind,
        global_point: Point,
        local_point: Point,
        modifiers: Modifiers,
        target: NodeId,
    ) -> Self {
        Self {
            kind,
            global_point,
            local_point,
            modifiers,
            target,
            current_target: target,
            bubble_path: Vec::new(),
            propagation_continued: false,
        }
    }

    /// Sets the bubble path on this event.
    pub fn with_bubble_path(mut self, path: Vec<NodeId>) -> Self {
        self.bubble_path = path;
        self
    }

    /// Requests that event propagation continue to the next ancestor node with a matching handler.
    /// By default, event propagation stops on the first node with a matching handler.
    pub fn continue_propagation(&mut self) {
        self.propagation_continued = true;
    }
}
