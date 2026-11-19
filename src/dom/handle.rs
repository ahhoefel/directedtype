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

    /// Packs this handle losslessly into a non-zero `u64` (`(generation << 32) | index`) for WASM FFI.
    #[inline]
    pub const fn to_u64(self) -> u64 {
        ((self.generation as u64) << 32) | (self.index as u64)
    }

    /// Unpacks a `NodeHandle` from a `u64` received across the WASM FFI boundary (`0` = `None`).
    #[inline]
    pub const fn from_u64(packed: u64) -> Option<Self> {
        let generation = (packed >> 32) as u32;
        if generation == 0 {
            None
        } else {
            Some(Self {
                index: packed as u32,
                generation,
            })
        }
    }
}
