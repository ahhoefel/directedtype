use crate::value::Value;
use serde::{Deserialize, Serialize};

/// A 2D point in logical pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Mouse buttons for pointer events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u16),
}

/// Keyboard modifier keys active during an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

/// Specific type of pointer/interaction event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    Callback,
}

/// A high-level DirectedType interaction event passed to guest methods.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub kind: EventKind,
    pub global_point: Point,
    pub local_point: Point,
    pub modifiers: Modifiers,
    pub target: u64,
    pub current_target: u64,
    pub payload: Option<Value>,
    pub propagation_continued: bool,
}

impl Event {
    pub fn continue_propagation(&mut self) {
        self.propagation_continued = true;
    }
}

