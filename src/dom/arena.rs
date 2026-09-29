use crate::dom::error::DomError;
use crate::dom::handle::NodeHandle;
use crate::dom::node::DomNode;

#[derive(Debug, Clone, PartialEq)]
enum Slot<T> {
    Occupied {
        value: T,
        generation: u32,
    },
    Vacant {
        next_free: Option<u32>,
        generation: u32,
    },
}

/// A generational arena providing $O(1)$ allocation, access, and deallocation
/// for Component DOM nodes, guarding against stale or dangling `NodeHandle`s.
#[derive(Debug, Clone, PartialEq)]
pub struct DomArena {
    slots: Vec<Slot<DomNode>>,
    first_free: Option<u32>,
    len: usize,
}

impl DomArena {
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            first_free: None,
            len: 0,
        }
    }

    /// Allocates a new node slot, passing the assigned `NodeHandle` into `f`.
    pub fn alloc(&mut self, f: impl FnOnce(NodeHandle) -> DomNode) -> NodeHandle {
        self.len += 1;
        if let Some(idx) = self.first_free {
            let slot = &mut self.slots[idx as usize];
            let gen = match slot {
                Slot::Vacant { next_free, generation } => {
                    self.first_free = *next_free;
                    *generation + 1
                }
                Slot::Occupied { .. } => unreachable!("first_free pointed to occupied slot"),
            };
            let handle = NodeHandle { index: idx, generation: gen };
            let node = f(handle);
            self.slots[idx as usize] = Slot::Occupied { value: node, generation: gen };
            handle
        } else {
            let idx = self.slots.len() as u32;
            let gen = 1;
            let handle = NodeHandle { index: idx, generation: gen };
            let node = f(handle);
            self.slots.push(Slot::Occupied { value: node, generation: gen });
            handle
        }
    }

    /// Frees a node slot. Fails if the handle is invalid or has a mismatched generation.
    pub fn free(&mut self, handle: NodeHandle) -> Result<DomNode, DomError> {
        let idx = handle.index as usize;
        if idx >= self.slots.len() {
            return Err(DomError::InvalidHandle(handle));
        }
        match &self.slots[idx] {
            Slot::Occupied { generation, .. } if *generation == handle.generation => {
                let next_free = self.first_free;
                let next_gen = generation.wrapping_add(1);
                let old_slot = std::mem::replace(
                    &mut self.slots[idx],
                    Slot::Vacant { next_free, generation: next_gen },
                );
                self.first_free = Some(handle.index);
                self.len -= 1;
                match old_slot {
                    Slot::Occupied { value, .. } => Ok(value),
                    _ => unreachable!(),
                }
            }
            _ => Err(DomError::InvalidHandle(handle)),
        }
    }

    /// Retrieves an immutable reference to a node by handle.
    pub fn get(&self, handle: NodeHandle) -> Result<&DomNode, DomError> {
        let idx = handle.index as usize;
        if idx >= self.slots.len() {
            return Err(DomError::InvalidHandle(handle));
        }
        match &self.slots[idx] {
            Slot::Occupied { value, generation } if *generation == handle.generation => Ok(value),
            _ => Err(DomError::InvalidHandle(handle)),
        }
    }

    /// Retrieves a mutable reference to a node by handle.
    pub fn get_mut(&mut self, handle: NodeHandle) -> Result<&mut DomNode, DomError> {
        let idx = handle.index as usize;
        if idx >= self.slots.len() {
            return Err(DomError::InvalidHandle(handle));
        }
        match &mut self.slots[idx] {
            Slot::Occupied { value, generation } if *generation == handle.generation => Ok(value),
            _ => Err(DomError::InvalidHandle(handle)),
        }
    }

    /// Checks whether a handle points to an active, valid node.
    pub fn contains(&self, handle: NodeHandle) -> bool {
        let idx = handle.index as usize;
        if idx >= self.slots.len() {
            return false;
        }
        match &self.slots[idx] {
            Slot::Occupied { generation, .. } => *generation == handle.generation,
            Slot::Vacant { .. } => false,
        }
    }

    /// The number of currently occupied nodes in the arena.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for DomArena {
    fn default() -> Self {
        Self::new()
    }
}
