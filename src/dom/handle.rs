/// A stable, generational handle to an authoring node in the Component DOM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeHandle {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

impl NodeHandle {
    pub const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    /// The slot index in the DOM arena.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// The generational version counter to guard against stale references.
    pub fn generation(&self) -> u32 {
        self.generation
    }
}
