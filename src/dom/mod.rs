pub mod arena;
pub mod error;
pub mod handle;
pub mod node;
pub mod transaction;

pub use arena::DomArena;
pub use error::DomError;
pub use handle::NodeHandle;
pub use node::DomNode;
pub use transaction::Transaction;

use std::collections::{HashMap, HashSet};

use crate::ast::{
    ComponentDef, ComponentKey, ContentItem, ContentSlot, Document, ElementNode, EnvBinding, Expr,
    Ident, Item, LetBinding, PortBinding, TextChunk,
};
use crate::compiler::error::CompileError;
use crate::compiler::expanded::NodeId;
use crate::compiler::graph::VarId;
use crate::compiler::layout::{Rect, ResolvedLayout};
use crate::compiler::value::Value;
use crate::compiler::{compile_document_with_window, CompiledDocument};
use crate::interaction::Point;
use crate::parser::cursor::ParserCursor;
use crate::parser::node::parse_element_node;
use crate::parser::parse_document;
use crate::span::Span;

/// Result of a spatial hit test on the Component DOM.
#[derive(Debug, Clone, PartialEq)]
pub struct DomHitTestResult {
    /// The primary authoring-level node handle that was hit.
    pub target: NodeHandle,
    /// Hit point in window coordinates.
    pub global_point: Point,
    /// Hit point translated into target node's local coordinate space.
    pub local_point: Point,
    /// Bubble path from hit node up to root in terms of Component DOM handles.
    pub bubble_path: Vec<NodeHandle>,
}

#[derive(Clone)]
struct DomStructuralSnapshot {
    arena: DomArena,
    roots: Vec<NodeHandle>,
    components: HashMap<String, Vec<ComponentDef>>,
    let_bindings: Vec<LetBinding>,
    env_bindings: Vec<EnvBinding>,
    window_width: f64,
    window_height: f64,
    dirty: bool,
}

/// The stateful Component DOM controller for DirectedType.
///
/// Maintains the live authoring-level component hierarchy, tracks modifications,
/// and executes transactional layout compilations and spatial queries.
pub struct Dom {
    arena: DomArena,
    roots: Vec<NodeHandle>,
    components: HashMap<String, Vec<ComponentDef>>,
    let_bindings: Vec<LetBinding>,
    env_bindings: Vec<EnvBinding>,
    window_width: f64,
    window_height: f64,
    dirty: bool,
    compiled: Option<CompiledDocument>,
    state_overrides: HashMap<VarId, Value>,
    component_registry: crate::component::ComponentRegistry,
    handle_to_node_id: HashMap<NodeHandle, NodeId>,
    node_id_to_handle: HashMap<NodeId, NodeHandle>,
}

impl Dom {
    /// Creates a new empty DOM with default window dimensions (800x600).
    pub fn new() -> Self {
        Self::with_window(800.0, 600.0)
    }

    /// Creates a new empty DOM with specified window dimensions.
    pub fn with_window(width: f64, height: f64) -> Self {
        Self {
            arena: DomArena::new(),
            roots: Vec::new(),
            components: HashMap::new(),
            let_bindings: Vec::new(),
            env_bindings: Vec::new(),
            window_width: width,
            window_height: height,
            dirty: true,
            compiled: None,
            state_overrides: HashMap::new(),
            component_registry: crate::component::ComponentRegistry::new(),
            handle_to_node_id: HashMap::new(),
            node_id_to_handle: HashMap::new(),
        }
    }

    /// Initializes a DOM from an existing parsed AST `Document` with default window dimensions.
    pub fn from_document(doc: &Document) -> Result<Self, DomError> {
        Self::from_document_with_window(doc, 800.0, 600.0)
    }

    /// Initializes a DOM from an existing parsed AST `Document` with specified window dimensions.
    pub fn from_document_with_window(
        doc: &Document,
        width: f64,
        height: f64,
    ) -> Result<Self, DomError> {
        let mut dom = Self::with_window(width, height);

        for item in &doc.items {
            match item {
                Item::Component(comp) => {
                    crate::compiler::module::register_component_overload(
                        &mut dom.components,
                        comp.clone(),
                    )?;
                }
                Item::Let(l) => {
                    dom.let_bindings.push(l.clone());
                }
                Item::Env(e) => {
                    dom.env_bindings.push(e.clone());
                }
                Item::Node(node) => {
                    let handle = dom.insert_ast_element(node)?;
                    dom.roots.push(handle);
                }
                Item::State(_) => {}
                Item::Use(_) => {}
            }
        }

        dom.dirty = true;
        Ok(dom)
    }

    /// Parses a DTML source string and initializes a DOM.
    pub fn from_source(source: &str) -> Result<Self, DomError> {
        Self::from_source_with_window(source, 800.0, 600.0)
    }

    /// Parses a DTML source string and initializes a DOM with specified window dimensions.
    pub fn from_source_with_window(
        source: &str,
        width: f64,
        height: f64,
    ) -> Result<Self, DomError> {
        let doc = parse_document(source)?;
        Self::from_document_with_window(&doc, width, height)
    }

    // -------------------------------------------------------------------------
    // Element Creation & Parsing
    // -------------------------------------------------------------------------

    /// Instantiates an unattached element node.
    pub fn create_element(&mut self, tag: &str, ports: Vec<(String, Expr)>) -> NodeHandle {
        self.create_element_with_key(tag, None, ports)
    }

    /// Instantiates an unattached element node with a structured key.
    pub fn create_element_with_key(
        &mut self,
        tag: &str,
        key: Option<ComponentKey>,
        ports: Vec<(String, Expr)>,
    ) -> NodeHandle {
        let mut port_map = HashMap::new();
        for (k, v) in ports {
            port_map.insert(k, v);
        }
        let handle = self.arena.alloc(|h| DomNode {
            handle: h,
            tag: tag.to_string(),
            key,
            parent: None,
            children: Vec::new(),
            ports: port_map,
            text_content: None,
            span: Span::default(),
        });
        self.dirty = true;
        handle
    }

    /// Instantiates an unattached text node.
    pub fn create_text(&mut self, text: &str, ports: Vec<(String, Expr)>) -> NodeHandle {
        let mut port_map = HashMap::new();
        for (k, v) in ports {
            port_map.insert(k, v);
        }
        let handle = self.arena.alloc(|h| DomNode {
            handle: h,
            tag: "Text".to_string(),
            key: None,
            parent: None,
            children: Vec::new(),
            ports: port_map,
            text_content: Some(text.to_string()),
            span: Span::default(),
        });
        self.dirty = true;
        handle
    }

    /// Parses a DTML snippet into an unattached element subtree and returns the root `NodeHandle`.
    pub fn parse_fragment(&mut self, dtml: &str) -> Result<NodeHandle, DomError> {
        let trimmed = dtml.trim();
        let mut cursor = ParserCursor::new(trimmed);
        let element = parse_element_node(&mut cursor)?;
        let handle = self.insert_ast_element(&element)?;
        self.dirty = true;
        Ok(handle)
    }

    // -------------------------------------------------------------------------
    // Hierarchy Mutations
    // -------------------------------------------------------------------------

    /// Appends `child` as the last child in `parent`'s content slot.
    ///
    /// If `child` was attached elsewhere, it is automatically detached first.
    pub fn append_child(&mut self, parent: NodeHandle, child: NodeHandle) -> Result<(), DomError> {
        if !self.arena.contains(parent) {
            return Err(DomError::InvalidHandle(parent));
        }
        if !self.arena.contains(child) {
            return Err(DomError::InvalidHandle(child));
        }
        if parent == child || self.is_descendant_of(parent, child) {
            return Err(DomError::HierarchyCycle(child));
        }

        self.detach(child)?;

        let parent_node = self.arena.get_mut(parent)?;
        parent_node.children.push(child);

        let child_node = self.arena.get_mut(child)?;
        child_node.parent = Some(parent);

        self.dirty = true;
        Ok(())
    }

    /// Inserts `child` immediately before `before` in `parent`'s content slot.
    pub fn insert_before(
        &mut self,
        parent: NodeHandle,
        before: NodeHandle,
        child: NodeHandle,
    ) -> Result<(), DomError> {
        if !self.arena.contains(parent) {
            return Err(DomError::InvalidHandle(parent));
        }
        if !self.arena.contains(child) {
            return Err(DomError::InvalidHandle(child));
        }
        if !self.arena.contains(before) {
            return Err(DomError::InvalidHandle(before));
        }
        if parent == child || self.is_descendant_of(parent, child) {
            return Err(DomError::HierarchyCycle(child));
        }

        let parent_node = self.arena.get(parent)?;
        if !parent_node.children.contains(&before) {
            return Err(DomError::BeforeNodeNotFound { parent, before });
        }

        self.detach(child)?;

        let parent_node = self.arena.get_mut(parent)?;
        let before_idx = parent_node
            .children
            .iter()
            .position(|&h| h == before)
            .ok_or(DomError::BeforeNodeNotFound { parent, before })?;

        parent_node.children.insert(before_idx, child);

        let child_node = self.arena.get_mut(child)?;
        child_node.parent = Some(parent);

        self.dirty = true;
        Ok(())
    }

    /// Detaches `child` from `parent`. Fails if `child` is not a child of `parent`.
    pub fn remove_child(&mut self, parent: NodeHandle, child: NodeHandle) -> Result<(), DomError> {
        if !self.arena.contains(parent) {
            return Err(DomError::InvalidHandle(parent));
        }
        if !self.arena.contains(child) {
            return Err(DomError::InvalidHandle(child));
        }

        let child_node = self.arena.get(child)?;
        if child_node.parent != Some(parent) {
            return Err(DomError::NoSuchChild { parent, child });
        }

        self.detach(child)
    }

    /// Replaces `old_child` with `new_child` in `parent`'s content slot.
    pub fn replace_child(
        &mut self,
        parent: NodeHandle,
        old_child: NodeHandle,
        new_child: NodeHandle,
    ) -> Result<(), DomError> {
        if !self.arena.contains(parent) {
            return Err(DomError::InvalidHandle(parent));
        }
        if !self.arena.contains(old_child) {
            return Err(DomError::InvalidHandle(old_child));
        }
        if !self.arena.contains(new_child) {
            return Err(DomError::InvalidHandle(new_child));
        }

        let parent_node = self.arena.get(parent)?;
        if !parent_node.children.contains(&old_child) {
            return Err(DomError::NoSuchChild {
                parent,
                child: old_child,
            });
        }

        if old_child == new_child {
            return Ok(());
        }

        if parent == new_child || self.is_descendant_of(parent, new_child) {
            return Err(DomError::HierarchyCycle(new_child));
        }

        self.detach(new_child)?;

        let parent_node = self.arena.get_mut(parent)?;
        let idx = parent_node
            .children
            .iter()
            .position(|&h| h == old_child)
            .ok_or(DomError::NoSuchChild {
                parent,
                child: old_child,
            })?;

        parent_node.children[idx] = new_child;

        let new_child_node = self.arena.get_mut(new_child)?;
        new_child_node.parent = Some(parent);

        let old_child_node = self.arena.get_mut(old_child)?;
        old_child_node.parent = None;

        self.dirty = true;
        Ok(())
    }

    /// Detaches `node` from its parent or from the document roots list.
    pub fn detach(&mut self, node: NodeHandle) -> Result<(), DomError> {
        let n = self.arena.get_mut(node)?;
        let old_parent = n.parent.take();

        if let Some(parent_handle) = old_parent {
            if let Ok(parent) = self.arena.get_mut(parent_handle) {
                if let Some(idx) = parent.children.iter().position(|&h| h == node) {
                    parent.children.remove(idx);
                }
            }
            self.dirty = true;
        }

        if let Some(idx) = self.roots.iter().position(|&h| h == node) {
            self.roots.remove(idx);
            self.dirty = true;
        }

        Ok(())
    }

    /// Detaches `node` and recursively frees it and all its descendants from the arena.
    pub fn destroy_node(&mut self, node: NodeHandle) -> Result<(), DomError> {
        self.detach(node)?;
        self.destroy_subtree_recursive(node)
    }

    fn destroy_subtree_recursive(&mut self, node: NodeHandle) -> Result<(), DomError> {
        let n = self.arena.free(node)?;
        for child in n.children {
            self.destroy_subtree_recursive(child)?;
        }
        self.handle_to_node_id.remove(&node);
        self.dirty = true;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Document Roots
    // -------------------------------------------------------------------------

    /// Appends a node as a top-level document root.
    pub fn append_root(&mut self, node: NodeHandle) -> Result<(), DomError> {
        if !self.arena.contains(node) {
            return Err(DomError::InvalidHandle(node));
        }
        self.detach(node)?;
        self.roots.push(node);
        self.dirty = true;
        Ok(())
    }

    /// Inserts a node before an existing root.
    pub fn insert_root_before(
        &mut self,
        before: NodeHandle,
        node: NodeHandle,
    ) -> Result<(), DomError> {
        if !self.arena.contains(node) {
            return Err(DomError::InvalidHandle(node));
        }
        if !self.arena.contains(before) {
            return Err(DomError::InvalidHandle(before));
        }

        let _ = self
            .roots
            .iter()
            .position(|&h| h == before)
            .ok_or(DomError::BeforeRootNotFound(before))?;

        self.detach(node)?;

        let before_idx = self
            .roots
            .iter()
            .position(|&h| h == before)
            .ok_or(DomError::BeforeRootNotFound(before))?;

        self.roots.insert(before_idx, node);
        self.dirty = true;
        Ok(())
    }

    /// Removes a node from the document roots list.
    pub fn remove_root(&mut self, node: NodeHandle) -> Result<(), DomError> {
        if !self.arena.contains(node) {
            return Err(DomError::InvalidHandle(node));
        }
        let idx = self
            .roots
            .iter()
            .position(|&h| h == node)
            .ok_or(DomError::NotARoot(node))?;
        self.roots.remove(idx);
        self.dirty = true;
        Ok(())
    }

    /// The list of top-level document root node handles.
    pub fn roots(&self) -> &[NodeHandle] {
        &self.roots
    }

    // -------------------------------------------------------------------------
    // Port & Text Mutations
    // -------------------------------------------------------------------------

    /// Sets or overrides a declared port equation on a node.
    pub fn set_port(
        &mut self,
        node: NodeHandle,
        port_name: &str,
        expr: Expr,
    ) -> Result<(), DomError> {
        let n = self.arena.get_mut(node)?;
        n.ports.insert(port_name.to_string(), expr);
        self.dirty = true;
        Ok(())
    }

    /// Removes a declared port equation from a node.
    pub fn remove_port(
        &mut self,
        node: NodeHandle,
        port_name: &str,
    ) -> Result<Option<Expr>, DomError> {
        let n = self.arena.get_mut(node)?;
        let old = n.ports.remove(port_name);
        if old.is_some() {
            self.dirty = true;
        }
        Ok(old)
    }

    /// Updates the text content of a `Text` node.
    pub fn set_text(&mut self, node: NodeHandle, text: &str) -> Result<(), DomError> {
        let n = self.arena.get_mut(node)?;
        n.text_content = Some(text.to_string());
        self.dirty = true;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Traversal & Inspection (Synchronous Component DOM)
    // -------------------------------------------------------------------------

    pub fn is_valid_handle(&self, handle: NodeHandle) -> bool {
        self.arena.contains(handle)
    }

    pub fn node_count(&self) -> usize {
        self.arena.len()
    }

    pub fn tag_name(&self, handle: NodeHandle) -> Result<&str, DomError> {
        let n = self.arena.get(handle)?;
        Ok(&n.tag)
    }

    pub fn text_content(&self, handle: NodeHandle) -> Result<Option<&str>, DomError> {
        let n = self.arena.get(handle)?;
        Ok(n.text_content.as_deref())
    }

    pub fn declared_ports(&self, handle: NodeHandle) -> Result<&HashMap<String, Expr>, DomError> {
        let n = self.arena.get(handle)?;
        Ok(&n.ports)
    }

    pub fn parent(&self, handle: NodeHandle) -> Result<Option<NodeHandle>, DomError> {
        let n = self.arena.get(handle)?;
        Ok(n.parent)
    }

    pub fn children(&self, handle: NodeHandle) -> Result<&[NodeHandle], DomError> {
        let n = self.arena.get(handle)?;
        Ok(&n.children)
    }

    pub fn first_child(&self, handle: NodeHandle) -> Result<Option<NodeHandle>, DomError> {
        let n = self.arena.get(handle)?;
        Ok(n.children.first().copied())
    }

    pub fn last_child(&self, handle: NodeHandle) -> Result<Option<NodeHandle>, DomError> {
        let n = self.arena.get(handle)?;
        Ok(n.children.last().copied())
    }

    pub fn prev_sibling(&self, handle: NodeHandle) -> Result<Option<NodeHandle>, DomError> {
        let node = self.arena.get(handle)?;
        if let Some(parent_handle) = node.parent {
            let parent = self.arena.get(parent_handle)?;
            if let Some(idx) = parent.children.iter().position(|&h| h == handle) {
                if idx > 0 {
                    return Ok(Some(parent.children[idx - 1]));
                }
            }
            Ok(None)
        } else {
            if let Some(idx) = self.roots.iter().position(|&h| h == handle) {
                if idx > 0 {
                    return Ok(Some(self.roots[idx - 1]));
                }
            }
            Ok(None)
        }
    }

    pub fn next_sibling(&self, handle: NodeHandle) -> Result<Option<NodeHandle>, DomError> {
        let node = self.arena.get(handle)?;
        if let Some(parent_handle) = node.parent {
            let parent = self.arena.get(parent_handle)?;
            if let Some(idx) = parent.children.iter().position(|&h| h == handle) {
                if idx + 1 < parent.children.len() {
                    return Ok(Some(parent.children[idx + 1]));
                }
            }
            Ok(None)
        } else {
            if let Some(idx) = self.roots.iter().position(|&h| h == handle) {
                if idx + 1 < self.roots.len() {
                    return Ok(Some(self.roots[idx + 1]));
                }
            }
            Ok(None)
        }
    }

    // -------------------------------------------------------------------------
    // Layout Evaluation & Inspection
    // -------------------------------------------------------------------------

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn window_size(&self) -> (f64, f64) {
        (self.window_width, self.window_height)
    }

    pub fn set_window_size(&mut self, width: f64, height: f64) {
        if (self.window_width - width).abs() > f64::EPSILON
            || (self.window_height - height).abs() > f64::EPSILON
        {
            self.window_width = width;
            self.window_height = height;
            self.dirty = true;
        }
    }

    /// Compiles and evaluates the Component DOM layout graph, presenting the latest `ResolvedLayout`.
    pub fn commit(&mut self) -> Result<&ResolvedLayout, DomError> {
        self.commit_with_window(self.window_width, self.window_height)
    }

    /// Compiles and evaluates the Component DOM layout graph with explicit window dimensions.
    pub fn commit_with_window(
        &mut self,
        width: f64,
        height: f64,
    ) -> Result<&ResolvedLayout, DomError> {
        self.window_width = width;
        self.window_height = height;

        if !self.dirty {
            if let Some(ref compiled) = self.compiled {
                return Ok(compiled.layout());
            }
        }

        let doc = self.to_document()?;
        let mut compiled = compile_document_with_window(&doc, width, height)?;
        compiled.attach_registry(&self.component_registry)?;

        // Re-apply any active runtime state overrides into the newly compiled document
        if !self.state_overrides.is_empty() {
            for (var_id, val) in &self.state_overrides {
                let _ = compiled.set_state(var_id.node, &var_id.port, val.clone());
            }
        }

        self.handle_to_node_id.clear();
        self.node_id_to_handle.clear();

        for resolved_node in &compiled.layout.nodes {
            if let Some(handle) = resolved_node.handle {
                self.handle_to_node_id.insert(handle, resolved_node.id);
                self.node_id_to_handle.insert(resolved_node.id, handle);
            }
        }

        self.compiled = Some(compiled);
        self.dirty = false;
        Ok(self.compiled.as_ref().unwrap().layout())
    }

    /// Returns the most recently committed `ResolvedLayout`, if one exists.
    pub fn layout(&self) -> Option<&ResolvedLayout> {
        self.compiled.as_ref().map(|c| c.layout())
    }

    /// Returns the active `CompiledDocument`, if one exists.
    pub fn compiled(&self) -> Option<&CompiledDocument> {
        self.compiled.as_ref()
    }

    /// Returns a mutable reference to the active `CompiledDocument`, if one exists.
    pub fn compiled_mut(&mut self) -> Option<&mut CompiledDocument> {
        self.compiled.as_mut()
    }

    /// Registers a typed companion component implementation on the DOM.
    pub fn register_component<F>(&mut self, name: impl Into<String>, factory: F)
    where
        F: Fn() -> Box<dyn crate::component::Component> + Send + Sync + 'static,
    {
        self.component_registry.register(name, factory);
        self.dirty = true;
    }

    /// Registers a typed companion component implementation with its companion file path.
    pub fn register_companion<F>(
        &mut self,
        name: impl Into<String>,
        companion_path: impl Into<String>,
        factory: F,
    ) where
        F: Fn() -> Box<dyn crate::component::Component> + Send + Sync + 'static,
    {
        self.component_registry.register_companion(name, companion_path, factory);
        self.dirty = true;
    }

    /// Returns a reference to the DOM's component registry.
    pub fn component_registry(&self) -> &crate::component::ComponentRegistry {
        &self.component_registry
    }

    /// Returns a mutable reference to the DOM's component registry.
    pub fn component_registry_mut(&mut self) -> &mut crate::component::ComponentRegistry {
        self.dirty = true;
        &mut self.component_registry
    }

    /// Dispatches an interaction event through the DOM's compiled component hierarchy.
    pub fn dispatch_event(
        &mut self,
        event: &mut crate::interaction::Event,
    ) -> Result<HashSet<VarId>, crate::component::DispatchError> {
        if self.dirty || self.compiled.is_none() {
            let _ = self.commit();
        }

        let compiled = self.compiled.as_mut().ok_or_else(|| {
            crate::component::DispatchError::Custom("DOM layout could not be compiled".into())
        })?;

        let changed = compiled.dispatch_event(event)?;
        for var_id in &changed {
            if let Some(val) = compiled.state_overrides.get(var_id) {
                self.state_overrides.insert(var_id.clone(), val.clone());
            }
        }
        Ok(changed)
    }

    /// Mutates a declared reactive state variable on a component instance, triggering an
    /// incremental topological re-evaluation of all downstream dependent variables in microseconds.
    pub fn set_state(
        &mut self,
        node: NodeHandle,
        state_name: &str,
        value: Value,
    ) -> Result<HashSet<VarId>, DomError> {
        if self.dirty || self.compiled.is_none() {
            self.commit()?;
        }

        let node_id = self
            .handle_to_node_id
            .get(&node)
            .copied()
            .ok_or(DomError::InvalidHandle(node))?;

        let compiled = self.compiled.as_mut().unwrap();
        let changed = compiled.set_state(node_id, state_name, value.clone())?;
        let var_id = VarId::new(node_id, state_name);
        self.state_overrides.insert(var_id, value);

        Ok(changed)
    }

    /// Mutates a reactive state variable on a child component identified by its structured identity key.
    pub fn set_state_by_key(
        &mut self,
        parent: Option<NodeHandle>,
        key: &ComponentKey,
        state_name: &str,
        value: Value,
    ) -> Result<HashSet<VarId>, DomError> {
        if self.dirty || self.compiled.is_none() {
            self.commit()?;
        }

        let parent_id = match parent {
            Some(h) => Some(
                self.handle_to_node_id
                    .get(&h)
                    .copied()
                    .ok_or(DomError::InvalidHandle(h))?,
            ),
            None => None,
        };

        let target_node_id = self
            .compiled
            .as_ref()
            .unwrap()
            .find_by_key(parent_id, key)
            .map(|n| n.id)
            .ok_or_else(|| CompileError::Custom {
                message: format!("Component with structured key '{}' not found", key),
                span: key.span,
            })?;

        let compiled = self.compiled.as_mut().unwrap();
        let changed = compiled.set_state(target_node_id, state_name, value.clone())?;
        let var_id = VarId::new(target_node_id, state_name);
        self.state_overrides.insert(var_id, value);

        Ok(changed)
    }

    /// Returns the active runtime value for a state variable on `node`.
    pub fn get_state(&self, node: NodeHandle, state_name: &str) -> Option<&Value> {
        let node_id = self.handle_to_node_id.get(&node)?;
        self.compiled.as_ref()?.get_state(*node_id, state_name)
    }

    /// Finds a node handle matching a structured component key.
    pub fn get_node_by_key(&self, parent: Option<NodeHandle>, key: &ComponentKey) -> Option<NodeHandle> {
        let parent_id = parent.and_then(|h| self.handle_to_node_id.get(&h).copied());
        let resolved = self.compiled.as_ref()?.find_by_key(parent_id, key)?;
        self.node_id_to_handle.get(&resolved.id).copied()
    }

    /// Returns the structured component key for `node` if one was declared.
    pub fn node_key(&self, node: NodeHandle) -> Option<&ComponentKey> {
        self.arena
            .get(node)
            .ok()
            .and_then(|n| n.key.as_ref())
            .or_else(|| {
                let node_id = self.handle_to_node_id.get(&node)?;
                self.layout()?.get_node(*node_id)?.key.as_ref()
            })
    }

    /// Returns the active map of all runtime state overrides.
    pub fn state_overrides(&self) -> &HashMap<VarId, Value> {
        &self.state_overrides
    }

    /// Returns the resolved bounding rectangle for `node` from the most recent committed layout.
    pub fn computed_rect(&self, node: NodeHandle) -> Option<Rect> {
        let layout = self.layout()?;
        let node_id = self.handle_to_node_id.get(&node)?;
        layout.get_node(*node_id).map(|n| n.rect)
    }

    /// Returns the computed value of a port on `node` from the most recent committed layout.
    pub fn computed_value(&self, node: NodeHandle, port: &str) -> Option<&Value> {
        let layout = self.layout()?;
        let node_id = self.handle_to_node_id.get(&node)?;
        layout.get_value(*node_id, port)
    }

    /// Returns the active clip node bounding `node` from the most recent committed layout.
    pub fn clip_context(&self, node: NodeHandle) -> Option<NodeHandle> {
        let layout = self.layout()?;
        let node_id = self.handle_to_node_id.get(&node)?;
        let resolved = layout.get_node(*node_id)?;
        let clip_node_id = resolved.clip?;
        self.node_id_to_handle.get(&clip_node_id).copied()
    }

    /// Queries the topmost visual element handle at the given coordinate.
    pub fn hit_test(&self, point: Point) -> Option<NodeHandle> {
        let layout = self.layout()?;
        let hit = layout.hit_test(point)?;
        if let Some(&handle) = self.node_id_to_handle.get(&hit.target) {
            return Some(handle);
        }
        for &ancestor_id in &hit.bubble_path {
            if let Some(&handle) = self.node_id_to_handle.get(&ancestor_id) {
                return Some(handle);
            }
        }
        None
    }

    /// Queries the full hit test result translated into Component DOM `NodeHandle`s.
    pub fn hit_test_full(&self, point: Point) -> Option<DomHitTestResult> {
        let layout = self.layout()?;
        let hit = layout.hit_test(point)?;
        let target = self
            .node_id_to_handle
            .get(&hit.target)
            .copied()
            .or_else(|| {
                hit.bubble_path
                    .iter()
                    .find_map(|id| self.node_id_to_handle.get(id).copied())
            })?;

        let bubble_path = hit
            .bubble_path
            .iter()
            .filter_map(|id| self.node_id_to_handle.get(id).copied())
            .collect();

        Some(DomHitTestResult {
            target,
            global_point: hit.global_point,
            local_point: hit.local_point,
            bubble_path,
        })
    }

    pub fn node_handle_to_id(&self, handle: NodeHandle) -> Option<NodeId> {
        self.handle_to_node_id.get(&handle).copied()
    }

    pub fn node_id_to_handle(&self, id: NodeId) -> Option<NodeHandle> {
        self.node_id_to_handle.get(&id).copied()
    }

    // -------------------------------------------------------------------------
    // Transaction Model & Batched Commit
    // -------------------------------------------------------------------------

    /// Executes a batch of mutations inside an atomic transaction.
    ///
    /// Changes mutate the live Component DOM immediately. When `f` returns `Ok(res)`,
    /// `commit()` is automatically invoked. If `f` returns `Err` or if layout evaluation
    /// fails (e.g., cyclic dependency error), the structural state is rolled back.
    pub fn transaction<F, R>(&mut self, f: F) -> Result<R, DomError>
    where
        F: FnOnce(&mut Transaction<'_>) -> Result<R, DomError>,
    {
        let snapshot = self.clone_structural_snapshot();

        let mut tx = Transaction::new(self);
        match f(&mut tx) {
            Ok(result) => {
                if !tx.committed {
                    if let Err(err) = tx.dom.commit() {
                        tx.dom.restore_structural_snapshot(snapshot);
                        return Err(err);
                    }
                }
                Ok(result)
            }
            Err(err) => {
                tx.dom.restore_structural_snapshot(snapshot);
                Err(err)
            }
        }
    }

    /// Begins a manual transaction handle wrapping `&mut Dom`.
    pub fn begin_transaction(&mut self) -> Transaction<'_> {
        Transaction::new(self)
    }

    fn clone_structural_snapshot(&self) -> DomStructuralSnapshot {
        DomStructuralSnapshot {
            arena: self.arena.clone(),
            roots: self.roots.clone(),
            components: self.components.clone(),
            let_bindings: self.let_bindings.clone(),
            env_bindings: self.env_bindings.clone(),
            window_width: self.window_width,
            window_height: self.window_height,
            dirty: self.dirty,
        }
    }

    fn restore_structural_snapshot(&mut self, snapshot: DomStructuralSnapshot) {
        self.arena = snapshot.arena;
        self.roots = snapshot.roots;
        self.components = snapshot.components;
        self.let_bindings = snapshot.let_bindings;
        self.env_bindings = snapshot.env_bindings;
        self.window_width = snapshot.window_width;
        self.window_height = snapshot.window_height;
        self.dirty = snapshot.dirty;
    }

    // -------------------------------------------------------------------------
    // Internal AST Conversion Helpers
    // -------------------------------------------------------------------------

    fn is_descendant_of(&self, maybe_descendant: NodeHandle, maybe_ancestor: NodeHandle) -> bool {
        let mut curr = Some(maybe_descendant);
        let mut visited = std::collections::HashSet::new();
        while let Some(h) = curr {
            if h == maybe_ancestor {
                return true;
            }
            if !visited.insert(h) {
                break;
            }
            curr = self.arena.get(h).ok().and_then(|n| n.parent);
        }
        false
    }

    fn insert_ast_element(&mut self, elem: &ElementNode) -> Result<NodeHandle, DomError> {
        let span = elem.span;
        let tag = elem.name.as_str().to_string();
        let mut ports = HashMap::new();
        for p in &elem.ports {
            ports.insert(p.name.as_str().to_string(), p.expr.clone());
        }

        let mut text_content = None;
        let mut child_elements = Vec::new();

        if let Some(content) = &elem.content {
            let mut text_parts = Vec::new();
            for item in &content.items {
                match item {
                    ContentItem::Text(chunk) => {
                        text_parts.push(chunk.text.clone());
                    }
                    ContentItem::Node(child_node) => {
                        child_elements.push(child_node);
                    }
                    ContentItem::Children(_) => {}
                }
            }
            if !text_parts.is_empty() {
                text_content = Some(text_parts.join(" "));
            }
        }

        let key = elem.key.clone();
        let handle = self.arena.alloc(|h| DomNode {
            handle: h,
            tag,
            key,
            parent: None,
            children: Vec::new(),
            ports,
            text_content,
            span,
        });

        for child_elem in child_elements {
            let child_handle = self.insert_ast_element(child_elem)?;
            let child_node = self.arena.get_mut(child_handle)?;
            child_node.parent = Some(handle);
            let parent_node = self.arena.get_mut(handle)?;
            parent_node.children.push(child_handle);
        }

        Ok(handle)
    }

    /// Converts a node subtree in the Component DOM into an AST `ElementNode`.
    pub fn to_element_node(&self, handle: NodeHandle) -> Result<ElementNode, DomError> {
        let node = self.arena.get(handle)?;
        let mut ports = Vec::with_capacity(node.ports.len());
        let mut sorted_ports: Vec<_> = node.ports.iter().collect();
        sorted_ports.sort_by_key(|(k, _)| *k);
        for (k, v) in sorted_ports {
            ports.push(PortBinding {
                name: Ident::new(k.clone(), node.span),
                expr: v.clone(),
                span: node.span,
            });
        }

        let mut content_items = Vec::new();
        if let Some(text) = &node.text_content {
            content_items.push(ContentItem::Text(TextChunk {
                text: text.clone(),
                span: node.span,
            }));
        }
        for &child_handle in &node.children {
            content_items.push(ContentItem::Node(self.to_element_node(child_handle)?));
        }

        let content = if !content_items.is_empty() {
            Some(ContentSlot {
                items: content_items,
                span: node.span,
            })
        } else {
            None
        };

        Ok(ElementNode {
            name: Ident::new(node.tag.clone(), node.span),
            key: node.key.clone(),
            ports,
            content,
            span: node.span,
            handle: Some(handle),
        })
    }

    /// Converts the entire Component DOM into an AST `Document`.
    pub fn to_document(&self) -> Result<Document, DomError> {
        let mut items = Vec::new();

        let mut sorted_comps: Vec<_> = self.components.values().flatten().collect();
        sorted_comps.sort_by_key(|c| (c.name.as_str(), c.span.start));
        for comp in sorted_comps {
            items.push(Item::Component(comp.clone()));
        }

        for l in &self.let_bindings {
            items.push(Item::Let(l.clone()));
        }

        for e in &self.env_bindings {
            items.push(Item::Env(e.clone()));
        }

        for &root_handle in &self.roots {
            let elem = self.to_element_node(root_handle)?;
            items.push(Item::Node(elem));
        }

        Ok(Document {
            items,
            span: Span::default(),
        })
    }
}

impl Default for Dom {
    fn default() -> Self {
        Self::new()
    }
}
