pub mod arena;
pub mod error;
pub mod handle;
pub mod node;
pub mod transaction;

pub use arena::DomArena;
pub use error::DomError;
pub use handle::NodeHandle;
pub use node::{DomContentItem, DomNode};
pub use transaction::Transaction;

use std::collections::{HashMap, HashSet};

use crate::ast::{
    ComponentDef, ComponentKey, ContentItem, ContentSlot, Document, ElementNode, EnumDef, EnvBinding, Expr,
    Ident, Item, LetBinding, LetValue, PortBinding, StateBinding, TextChunk, UseDeclaration,
};
use crate::compiler::error::CompileError;
use crate::compiler::expanded::NodeId;
use crate::compiler::graph::VarId;
use crate::compiler::layout::{Rect, ResolvedLayout};
use crate::compiler::module::{normalize_path, FileResolver};
use crate::compiler::value::Value;
use crate::compiler::{compile_document_with_window, CompiledDocument};
use crate::component::{run_wasm_prepare, wasm_has_prepare, ComponentRegistry, PrepareHostState};
use crate::interaction::Point;
use crate::parser::cursor::ParserCursor;
use crate::parser::node::parse_element_node;
use crate::parser::parse_document;
use crate::span::Span;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
    state_bindings: Vec<StateBinding>,
    uses: Vec<UseDeclaration>,
    enums: Vec<EnumDef>,
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
    state_bindings: Vec<StateBinding>,
    pub uses: Vec<UseDeclaration>,
    pub enums: Vec<EnumDef>,
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
            state_bindings: Vec::new(),
            uses: Vec::new(),
            enums: Vec::new(),
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
                Item::Let(l) => match &l.value {
                    Some(LetValue::Node(node)) => {
                        let handle = dom.insert_ast_element(node)?;
                        dom.arena.get_mut(handle)?.root_let_binding =
                            Some((l.name.clone(), l.type_annotation.clone(), l.span));
                        dom.roots.push(handle);
                    }
                    _ => {
                        dom.let_bindings.push(l.clone());
                    }
                }
                Item::Env(e) => {
                    dom.env_bindings.push(e.clone());
                }
                Item::Node(node) => {
                    let handle = dom.insert_ast_element(node)?;
                    dom.roots.push(handle);
                }
                Item::Enum(e) => {
                    dom.enums.push(e.clone());
                }
                Item::State(s) => {
                    dom.state_bindings.push(s.clone());
                }
                Item::Use(u) => {
                    dom.uses.push(u.clone());
                }
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
            content_items: Vec::new(),
            root_let_binding: None,
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
        let text_owned = text.to_string();
        let handle = self.arena.alloc(|h| DomNode {
            handle: h,
            tag: "Text".to_string(),
            key: None,
            parent: None,
            children: Vec::new(),
            content_items: vec![DomContentItem::Text(text_owned.clone())],
            root_let_binding: None,
            ports: port_map,
            text_content: Some(text_owned),
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
        parent_node.content_items.push(DomContentItem::Child(child));

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
        if let Some(cpos) = parent_node
            .content_items
            .iter()
            .position(|it| matches!(it, DomContentItem::Child(h) if *h == before))
        {
            parent_node
                .content_items
                .insert(cpos, DomContentItem::Child(child));
        } else {
            parent_node.content_items.push(DomContentItem::Child(child));
        }

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
        if let Some(cpos) = parent_node
            .content_items
            .iter()
            .position(|it| matches!(it, DomContentItem::Child(h) if *h == old_child))
        {
            parent_node.content_items[cpos] = DomContentItem::Child(new_child);
        } else {
            parent_node
                .content_items
                .push(DomContentItem::Child(new_child));
        }

        let new_child_node = self.arena.get_mut(new_child)?;
        new_child_node.parent = Some(parent);

        let old_child_node = self.arena.get_mut(old_child)?;
        old_child_node.parent = None;

        self.dirty = true;
        Ok(())
    }

    /// Wraps `target` in place with `wrapper`, moving `target` inside `wrapper`'s children.
    pub fn wrap_node(&mut self, target: NodeHandle, wrapper: NodeHandle) -> Result<(), DomError> {
        if !self.arena.contains(target) {
            return Err(DomError::InvalidHandle(target));
        }
        if !self.arena.contains(wrapper) {
            return Err(DomError::InvalidHandle(wrapper));
        }
        if target == wrapper || self.is_descendant_of(wrapper, target) {
            return Err(DomError::HierarchyCycle(wrapper));
        }
        let parent_opt = self.arena.get(target)?.parent;
        if let Some(parent) = parent_opt {
            self.replace_child(parent, target, wrapper)?;
            self.append_child(wrapper, target)?;
        } else {
            self.detach(wrapper)?;
            let taken_let = self.arena.get_mut(target)?.root_let_binding.take();
            if let Some(pos) = self.roots.iter().position(|&h| h == target) {
                self.roots[pos] = wrapper;
            }
            self.arena.get_mut(wrapper)?.root_let_binding = taken_let;
            self.arena.get_mut(target)?.parent = None;
            self.append_child(wrapper, target)?;
        }
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
                parent
                    .content_items
                    .retain(|it| !matches!(it, DomContentItem::Child(h) if *h == node));
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

    /// Updates the text content of a `Text` (or `\Span`) node while keeping `content_items` synchronized.
    pub fn set_text(&mut self, node: NodeHandle, text: &str) -> Result<(), DomError> {
        let n = self.arena.get_mut(node)?;
        let text_str = text.to_string();
        n.text_content = Some(text_str.clone());
        if n.content_items
            .iter()
            .any(|it| matches!(it, DomContentItem::Text(_)))
        {
            let mut replaced = false;
            n.content_items.retain_mut(|it| {
                if let DomContentItem::Text(existing) = it {
                    if !replaced {
                        *existing = text_str.clone();
                        replaced = true;
                        true
                    } else {
                        false
                    }
                } else {
                    true
                }
            });
        } else {
            n.content_items
                .insert(0, DomContentItem::Text(text_str));
        }
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

    /// Recursively extracts concatenated authoring plain text across inline children/text chunks.
    pub fn get_subtree_text(&self, handle: NodeHandle) -> Result<String, DomError> {
        let n = self.arena.get(handle)?;
        if let Some(Expr::Literal(crate::ast::Literal::String(s, _))) = n.ports.get("text") {
            if n.content_items.is_empty() && n.children.is_empty() {
                return Ok(s.clone());
            }
        }
        let mut out = String::new();
        if !n.content_items.is_empty() {
            for item in &n.content_items {
                match item {
                    DomContentItem::Text(t) => out.push_str(t),
                    DomContentItem::ParagraphBreak => out.push('\n'),
                    DomContentItem::Child(cid) => {
                        out.push_str(&self.get_subtree_text(*cid)?);
                    }
                    DomContentItem::Directive(_) => {}
                }
            }
        } else {
            if let Some(ref t) = n.text_content {
                out.push_str(t);
            }
            for &cid in &n.children {
                out.push_str(&self.get_subtree_text(cid)?);
            }
        }
        Ok(out)
    }

    /// Pre-order query over `scope` (or all roots if `None`) returning nodes whose tag matches any entry in `tags`.
    pub fn query_by_tag(
        &self,
        scope: Option<NodeHandle>,
        tags: &[&str],
    ) -> Result<Vec<NodeHandle>, DomError> {
        let start: Vec<NodeHandle> = if let Some(s) = scope {
            let n = self.arena.get(s)?;
            n.children.clone()
        } else {
            self.roots.clone()
        };
        let mut out = Vec::new();
        let mut stack: Vec<NodeHandle> = start.into_iter().rev().collect();
        while let Some(h) = stack.pop() {
            if let Ok(n) = self.arena.get(h) {
                if tags.iter().any(|&t| t == n.tag) {
                    out.push(h);
                }
                for &cid in n.children.iter().rev() {
                    stack.push(cid);
                }
            }
        }
        Ok(out)
    }

    /// Returns ancestors walking upward `[parent, grandparent, ..., root]`.
    pub fn ancestors(&self, handle: NodeHandle) -> Result<Vec<NodeHandle>, DomError> {
        let mut out = Vec::new();
        let mut curr = self.arena.get(handle)?.parent;
        let mut visited = HashSet::new();
        while let Some(h) = curr {
            if !visited.insert(h) {
                break;
            }
            out.push(h);
            curr = self.arena.get(h).ok().and_then(|n| n.parent);
        }
        Ok(out)
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

    /// Executes pre-layout `on_prepare` hooks (WASM and native) in deterministic pre-order using `FsResolver`.
    pub fn prepare(&mut self) -> Result<(), DomError> {
        let resolver = crate::compiler::module::FsResolver;
        let std_reg = ComponentRegistry::standard();
        self.prepare_with_resolver(Path::new("."), &resolver, Some(&std_reg))
    }

    /// Executes pre-layout `on_prepare` hooks (WASM companions, `\use "*.wasm"` bundles, and native companions)
    /// in deterministic pre-order with bounded passes (`MAX_PREPARE_PASSES = 8`) and atomic snapshot rollback.
    pub fn prepare_with_resolver(
        &mut self,
        base_dir: &Path,
        resolver: &dyn FileResolver,
        extra_registry: Option<&ComponentRegistry>,
    ) -> Result<(), DomError> {
        let snapshot = self.clone_structural_snapshot();
        match self.prepare_with_resolver_inner(base_dir, resolver, extra_registry) {
            Ok(()) => Ok(()),
            Err(err) => {
                self.restore_structural_snapshot(snapshot);
                Err(err)
            }
        }
    }

    fn prepare_with_resolver_inner(
        &mut self,
        base_dir: &Path,
        resolver: &dyn FileResolver,
        extra_registry: Option<&ComponentRegistry>,
    ) -> Result<(), DomError> {
        const MAX_PREPARE_PASSES: usize = 8;

        let mut component_paths: HashMap<String, String> = HashMap::new();
        let mut wasm_companions: HashMap<String, Arc<[u8]>> = HashMap::new();
        let mut wasm_bundles: Vec<(String, Arc<[u8]>)> = Vec::new();

        for (p, bytes) in self.component_registry.wasm_bundles() {
            if !wasm_bundles.iter().any(|(ep, _)| ep == p) {
                wasm_bundles.push((p.clone(), Arc::clone(bytes)));
            }
        }
        if let Some(reg) = extra_registry {
            for (p, bytes) in reg.wasm_bundles() {
                if !wasm_bundles.iter().any(|(ep, _)| ep == p) {
                    wasm_bundles.push((p.clone(), Arc::clone(bytes)));
                }
            }
        }

        let mut visited_dt = HashSet::new();
        let mut use_queue: Vec<(String, PathBuf)> = self
            .uses
            .iter()
            .map(|u| (u.path.clone(), base_dir.to_path_buf()))
            .collect();

        while let Some((raw_str, cur_dir)) = use_queue.pop() {
            let resolved = resolve_prepare_path(&raw_str, &cur_dir, resolver);
            if raw_str.ends_with(".wasm") {
                if let Ok(bytes) = resolver
                    .read_bytes(&resolved)
                    .or_else(|_| resolver.read_bytes(Path::new(&raw_str)))
                {
                    let key = normalize_path(&resolved).display().to_string();
                    if !wasm_bundles.iter().any(|(ep, _)| ep == &key) {
                        wasm_bundles.push((key, Arc::from(bytes)));
                    }
                }
                continue;
            }

            let canon = resolver
                .canonicalize(&resolved)
                .unwrap_or_else(|_| normalize_path(&resolved));
            if !visited_dt.insert(canon.clone()) {
                continue;
            }
            let dt_str = resolved.display().to_string();
            if let Some(wasm_path) = resolver.find_companion_wasm(&resolved) {
                if let Ok(bytes) = resolver.read_bytes(&wasm_path) {
                    let arc: Arc<[u8]> = Arc::from(bytes);
                    wasm_companions.insert(dt_str.clone(), Arc::clone(&arc));
                    if let Some(stem) = wasm_path.file_stem().and_then(|s| s.to_str()) {
                        wasm_companions
                            .entry(stem.to_string())
                            .or_insert_with(|| Arc::clone(&arc));
                    }
                }
            }
            if let Ok(source) = resolver.read(&resolved) {
                if let Ok(parsed) = parse_document(&source) {
                    let mod_dir = resolved.parent().unwrap_or(&cur_dir).to_path_buf();
                    for item in &parsed.items {
                        match item {
                            Item::Component(c) => {
                                component_paths
                                    .entry(c.name.as_str().to_string())
                                    .or_insert_with(|| dt_str.clone());
                            }
                            Item::Use(sub_u) => {
                                use_queue.push((sub_u.path.clone(), mod_dir.clone()));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // Probe adjacent `<Tag>.wasm` files on demand for local/virtual components
        let mut probed_tag_wasm: HashSet<String> = HashSet::new();
        let mut has_prepare_cache: HashMap<usize, bool> = HashMap::new();
        let mut exports_cache: HashMap<usize, HashSet<String>> = HashMap::new();

        let mut prepared_handles: HashSet<NodeHandle> = HashSet::new();
        let mut ran_generic_bundles: HashSet<String> = HashSet::new();
        let mut total_created: usize = 0;
        let mut total_mutations: usize = 0;

        for pass in 0..MAX_PREPARE_PASSES {
            // 1. Run any document-level bundle generic `__dt_prepare` export once per document.
            for idx in 0..wasm_bundles.len() {
                let bundle_path = wasm_bundles[idx].0.clone();
                if ran_generic_bundles.contains(&bundle_path) {
                    continue;
                }
                let bundle_bytes = Arc::clone(&wasm_bundles[idx].1);
                let exports = cached_wasm_exports(&bundle_bytes, &mut exports_cache)?;
                let has_generic = exports.contains("__dt_prepare")
                    && !exports.iter().any(|e| e.starts_with("__dt_prepare_"));
                if !has_generic {
                    continue;
                }
                if let Some(&first_root) = self.roots.first() {
                    let should_run = cached_wasm_has_prepare(&bundle_bytes, &mut has_prepare_cache)?;
                    if should_run {
                        ran_generic_bundles.insert(bundle_path.clone());
                        let mut host_state = PrepareHostState::new(self, base_dir, resolver);
                        host_state.created_nodes = total_created;
                        host_state.mutations = total_mutations;
                        run_wasm_prepare(
                            &bundle_bytes,
                            "__dt_prepare",
                            &bundle_path,
                            first_root,
                            &mut host_state,
                        )?;
                        total_created = host_state.created_nodes;
                        total_mutations = host_state.mutations;
                    } else {
                        ran_generic_bundles.insert(bundle_path);
                    }
                }
            }

            // 2. Snapshot current attached pre-order handles and execute newly discovered handlers.
            let preorder = self.collect_attached_preorder();
            for h in preorder {
                if prepared_handles.contains(&h) || !self.is_attached_handle(h) {
                    continue;
                }
                let Ok(tag) = self.tag_name(h).map(|s| s.to_string()) else {
                    continue;
                };
                prepared_handles.insert(h);
                let comp_path = component_paths.get(&tag).map(|s| s.as_str());

                if !wasm_companions.contains_key(&tag)
                    && !probed_tag_wasm.contains(&tag)
                {
                    probed_tag_wasm.insert(tag.clone());
                    let cand = base_dir.join(format!("{}.wasm", tag));
                    let rel = PathBuf::from(format!("{}.wasm", tag));
                    if let Ok(bytes) = resolver
                        .read_bytes(&cand)
                        .or_else(|_| resolver.read_bytes(&rel))
                    {
                        wasm_companions.insert(tag.clone(), Arc::from(bytes));
                    }
                }

                // Native companion `on_prepare`
                let native_opt = extra_registry
                    .and_then(|r| r.create_instance_for_node(&tag, comp_path))
                    .or_else(|| self.component_registry.create_instance_for_node(&tag, comp_path))
                    .or_else(|| extra_registry.and_then(|r| r.create_instance(&tag)))
                    .or_else(|| self.component_registry.create_instance(&tag));

                if let Some(mut comp) = native_opt {
                    if comp.has_prepare() {
                        let mut host_state = PrepareHostState::new(self, base_dir, resolver);
                        host_state.created_nodes = total_created;
                        host_state.mutations = total_mutations;
                        let guest_h = crate::component::GuestNodeHandle(h.to_u64());
                        let res = directedtype_guest::dom::with_host_dom_bridge(
                            &mut host_state,
                            guest_h,
                            |ctx| comp.on_prepare(ctx),
                        );
                        total_created = host_state.created_nodes;
                        total_mutations = host_state.mutations;
                        if let Some(fatal) = host_state.fatal_error.take() {
                            return Err(fatal);
                        }
                        if let Err(err) = res {
                            return Err(DomError::PrepareError {
                                component: tag,
                                node: h,
                                message: err.to_string(),
                            });
                        }
                    }
                }

                // Side-by-side `.wasm` companion `on_prepare`
                let wasm_bytes_opt: Option<Arc<[u8]>> = comp_path
                    .and_then(|p| wasm_companions.get(p).cloned())
                    .or_else(|| wasm_companions.get(&tag).cloned())
                    .or_else(|| {
                        extra_registry
                            .and_then(|r| r.get_wasm_for_node(&tag, comp_path))
                            .map(Arc::from)
                    })
                    .or_else(|| {
                        self.component_registry
                            .get_wasm_for_node(&tag, comp_path)
                            .map(Arc::from)
                    });

                if let Some(wasm_bytes) = wasm_bytes_opt {
                    if cached_wasm_has_prepare(&wasm_bytes, &mut has_prepare_cache)? {
                        let mut host_state = PrepareHostState::new(self, base_dir, resolver);
                        host_state.created_nodes = total_created;
                        host_state.mutations = total_mutations;
                        run_wasm_prepare(
                            &wasm_bytes,
                            "__dt_prepare",
                            &tag,
                            h,
                            &mut host_state,
                        )?;
                        total_created = host_state.created_nodes;
                        total_mutations = host_state.mutations;
                    }
                }

                // Tag-targeted exports in explicit `\use "*.wasm"` bundles (`__dt_prepare_<Tag>`)
                let specific_export = format!("__dt_prepare_{}", tag);
                for idx in 0..wasm_bundles.len() {
                    let bundle_bytes = Arc::clone(&wasm_bundles[idx].1);
                    let exports = cached_wasm_exports(&bundle_bytes, &mut exports_cache)?;
                    if exports.contains(&specific_export) {
                        let mut host_state = PrepareHostState::new(self, base_dir, resolver);
                        host_state.created_nodes = total_created;
                        host_state.mutations = total_mutations;
                        run_wasm_prepare(
                            &bundle_bytes,
                            &specific_export,
                            &tag,
                            h,
                            &mut host_state,
                        )?;
                        total_created = host_state.created_nodes;
                        total_mutations = host_state.mutations;
                    }
                }
            }

            // 3. Check whether any newly inserted nodes in the tree still need `on_prepare`.
            let next_preorder = self.collect_attached_preorder();
            let mut pending_tag: Option<String> = None;
            for h in next_preorder {
                if prepared_handles.contains(&h) {
                    continue;
                }
                let Ok(tag) = self.tag_name(h).map(|s| s.to_string()) else {
                    continue;
                };
                let comp_path = component_paths.get(&tag).map(|s| s.as_str());
                let mut needs_prepare = false;

                if let Some(comp) = extra_registry
                    .and_then(|r| r.create_instance_for_node(&tag, comp_path))
                    .or_else(|| self.component_registry.create_instance_for_node(&tag, comp_path))
                    .or_else(|| extra_registry.and_then(|r| r.create_instance(&tag)))
                    .or_else(|| self.component_registry.create_instance(&tag))
                {
                    if comp.has_prepare() {
                        needs_prepare = true;
                    }
                }

                if !needs_prepare {
                    let wasm_bytes_opt: Option<Arc<[u8]>> = comp_path
                        .and_then(|p| wasm_companions.get(p).cloned())
                        .or_else(|| wasm_companions.get(&tag).cloned())
                        .or_else(|| {
                            extra_registry
                                .and_then(|r| r.get_wasm_for_node(&tag, comp_path))
                                .map(Arc::from)
                        })
                        .or_else(|| {
                            self.component_registry
                                .get_wasm_for_node(&tag, comp_path)
                                .map(Arc::from)
                        });
                    if let Some(wb) = wasm_bytes_opt {
                        if cached_wasm_has_prepare(&wb, &mut has_prepare_cache)? {
                            needs_prepare = true;
                        }
                    }
                }

                if !needs_prepare {
                    let specific_export = format!("__dt_prepare_{}", tag);
                    for (_, bundle_bytes) in &wasm_bundles {
                        let exports = cached_wasm_exports(bundle_bytes, &mut exports_cache)?;
                        if exports.contains(&specific_export) {
                            needs_prepare = true;
                            break;
                        }
                    }
                }

                if needs_prepare {
                    pending_tag = Some(tag);
                    break;
                }
            }

            if let Some(last_tag) = pending_tag {
                if pass + 1 >= MAX_PREPARE_PASSES {
                    return Err(DomError::PrepareCycle {
                        pass_limit: MAX_PREPARE_PASSES,
                        last_tag,
                    });
                }
            } else {
                return Ok(());
            }
        }

        Ok(())
    }

    fn collect_attached_preorder(&self) -> Vec<NodeHandle> {
        let mut out = Vec::new();
        let mut stack: Vec<NodeHandle> = self.roots.iter().copied().rev().collect();
        while let Some(h) = stack.pop() {
            if let Ok(n) = self.arena.get(h) {
                out.push(h);
                for &cid in n.children.iter().rev() {
                    stack.push(cid);
                }
            }
        }
        out
    }

    fn is_attached_handle(&self, handle: NodeHandle) -> bool {
        let Ok(n) = self.arena.get(handle) else {
            return false;
        };
        n.parent.is_some() || self.roots.contains(&handle)
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
            state_bindings: self.state_bindings.clone(),
            uses: self.uses.clone(),
            enums: self.enums.clone(),
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
        self.state_bindings = snapshot.state_bindings;
        self.uses = snapshot.uses;
        self.enums = snapshot.enums;
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

        enum PendingContent<'a> {
            Text(String),
            ParagraphBreak,
            Child(&'a ElementNode, Option<(Ident, Option<crate::ast::TypeRef>, Span)>),
            Directive(ContentItem),
        }

        let mut text_content = None;
        let mut pending: Vec<PendingContent<'_>> = Vec::new();
        let mut text_parts = Vec::new();

        if let Some(content) = &elem.content {
            for item in &content.items {
                match item {
                    ContentItem::Text(chunk) => {
                        text_parts.push(chunk.text.clone());
                        pending.push(PendingContent::Text(chunk.text.clone()));
                    }
                    ContentItem::ParagraphBreak(_) => {
                        pending.push(PendingContent::ParagraphBreak);
                    }
                    ContentItem::Node(child_node) => {
                        pending.push(PendingContent::Child(child_node, None));
                    }
                    ContentItem::Let(let_binding) => {
                        if let Some(LetValue::Node(child_node)) = &let_binding.value {
                            pending.push(PendingContent::Child(
                                child_node,
                                Some((
                                    let_binding.name.clone(),
                                    let_binding.type_annotation.clone(),
                                    let_binding.span,
                                )),
                            ));
                        } else {
                            pending.push(PendingContent::Directive(item.clone()));
                        }
                    }
                    ContentItem::Children(_)
                    | ContentItem::Alias(_)
                    | ContentItem::State(_)
                    | ContentItem::Env(_)
                    | ContentItem::Assert(_) => {
                        pending.push(PendingContent::Directive(item.clone()));
                    }
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
            content_items: Vec::new(),
            root_let_binding: None,
            ports,
            text_content,
            span,
        });

        let mut built_items = Vec::with_capacity(pending.len());
        let mut child_handles = Vec::new();
        for p in pending {
            match p {
                PendingContent::Text(t) => built_items.push(DomContentItem::Text(t)),
                PendingContent::ParagraphBreak => built_items.push(DomContentItem::ParagraphBreak),
                PendingContent::Directive(d) => built_items.push(DomContentItem::Directive(d)),
                PendingContent::Child(child_elem, let_info) => {
                    let child_handle = self.insert_ast_element(child_elem)?;
                    let child_node = self.arena.get_mut(child_handle)?;
                    child_node.parent = Some(handle);
                    child_node.root_let_binding = let_info;
                    child_handles.push(child_handle);
                    built_items.push(DomContentItem::Child(child_handle));
                }
            }
        }
        let parent_node = self.arena.get_mut(handle)?;
        parent_node.children = child_handles;
        parent_node.content_items = built_items;

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
        if !node.content_items.is_empty() {
            for item in &node.content_items {
                match item {
                    DomContentItem::Text(text) => {
                        content_items.push(ContentItem::Text(TextChunk {
                            text: text.clone(),
                            span: node.span,
                        }));
                    }
                    DomContentItem::ParagraphBreak => {
                        content_items.push(ContentItem::ParagraphBreak(node.span));
                    }
                    DomContentItem::Child(child_handle) => {
                        let child_node = self.arena.get(*child_handle)?;
                        let child_elem = self.to_element_node(*child_handle)?;
                        if let Some((name, type_annotation, let_span)) = &child_node.root_let_binding {
                            content_items.push(ContentItem::Let(LetBinding {
                                name: name.clone(),
                                type_annotation: type_annotation.clone(),
                                value: Some(LetValue::Node(child_elem)),
                                span: *let_span,
                            }));
                        } else {
                            content_items.push(ContentItem::Node(child_elem));
                        }
                    }
                    DomContentItem::Directive(dir) => {
                        content_items.push(dir.clone());
                    }
                }
            }
        } else {
            if let Some(text) = &node.text_content {
                content_items.push(ContentItem::Text(TextChunk {
                    text: text.clone(),
                    span: node.span,
                }));
            }
            for &child_handle in &node.children {
                content_items.push(ContentItem::Node(self.to_element_node(child_handle)?));
            }
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

        for u in &self.uses {
            items.push(Item::Use(u.clone()));
        }

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

        for s in &self.state_bindings {
            items.push(Item::State(s.clone()));
        }

        for e in &self.enums {
            items.push(Item::Enum(e.clone()));
        }

        for &root_handle in &self.roots {
            let root_node = self.arena.get(root_handle)?;
            let elem = self.to_element_node(root_handle)?;
            if let Some((name, type_annotation, let_span)) = &root_node.root_let_binding {
                items.push(Item::Let(LetBinding {
                    name: name.clone(),
                    type_annotation: type_annotation.clone(),
                    value: Some(LetValue::Node(elem)),
                    span: *let_span,
                }));
            } else {
                items.push(Item::Node(elem));
            }
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

fn resolve_prepare_path(raw: &str, base_dir: &Path, resolver: &dyn FileResolver) -> PathBuf {
    let raw_path = Path::new(raw);
    if raw_path.is_absolute() {
        return raw_path.to_path_buf();
    }
    let joined = base_dir.join(raw_path);
    if resolver.read(&joined).is_ok()
        || resolver.read_bytes(&joined).is_ok()
        || resolver.find_companion_wasm(&joined).is_some()
    {
        return joined;
    }
    if resolver.read(raw_path).is_ok()
        || resolver.read_bytes(raw_path).is_ok()
        || resolver.find_companion_wasm(raw_path).is_some()
    {
        return raw_path.to_path_buf();
    }
    joined
}

fn cached_wasm_has_prepare(
    bytes: &Arc<[u8]>,
    cache: &mut HashMap<usize, bool>,
) -> Result<bool, DomError> {
    let key = Arc::as_ptr(bytes) as *const u8 as usize;
    if let Some(&res) = cache.get(&key) {
        return Ok(res);
    }
    let res = wasm_has_prepare(bytes)?;
    cache.insert(key, res);
    Ok(res)
}

fn cached_wasm_exports<'a>(
    bytes: &Arc<[u8]>,
    cache: &'a mut HashMap<usize, HashSet<String>>,
) -> Result<&'a HashSet<String>, DomError> {
    let key = Arc::as_ptr(bytes) as *const u8 as usize;
    if !cache.contains_key(&key) {
        let exports = crate::component::wasm::wasm_prepare_exports(bytes)?;
        cache.insert(key, exports);
    }
    Ok(&cache[&key])
}
