use crate::ast::ComponentKey;
use crate::compiler::expanded::NodeId;
use crate::compiler::layout::ResolvedLayout;
use crate::compiler::value::Value;
use crate::interaction::Event;
use std::collections::HashMap;

pub mod wasm;
pub use wasm::WasmComponent;

/// The target entity receiving an event dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventHandlerTarget {
    /// A component instance identified by its expanded `NodeId`.
    Component(NodeId),
    /// An environmental service identified by name.
    Env(String),
}

/// An event handler binding attached to a visual element or component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventHandlerBinding {
    /// The event port name (e.g. "on_click", "on_pointer_down", "on_toggle").
    pub name: String,
    /// The target component or service receiving the event.
    pub target: EventHandlerTarget,
    /// The method name to invoke on the target component.
    pub method: String,
}

/// Error returned when event dispatch fails.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DispatchError {
    #[error("Component '{component}' has no method '{method}'")]
    MethodNotFound {
        component: String,
        method: String,
    },
    #[error("Component instance for node {0:?} not found")]
    InstanceNotFound(NodeId),
    #[error("Target '{0}' not supported for event dispatch")]
    UnsupportedTarget(String),
    #[error("Custom dispatch error: {0}")]
    Custom(String),
}

/// An action emitted by a component method (e.g. navigation, scrolling, external URL opening).
#[derive(Debug, Clone, PartialEq)]
pub enum ContextAction {
    /// Scroll to bring a target node into view.
    /// `container: None` scrolls the window/viewport.
    /// `container: Some(pane_id)` will scroll a specific ScrollPane node.
    ScrollToNode {
        target: NodeId,
        container: Option<NodeId>,
    },
    /// Request the host application/viewer to open an external URL.
    OpenUrl {
        url: String,
    },
    /// Request focus shift to a target node (or clear focus if None).
    SetFocus {
        target: Option<NodeId>,
    },
    /// Capture all subsequent pointer events (moves and release) to the specified node until released.
    CapturePointer(NodeId),
    /// Release any active pointer capture.
    ReleasePointer,
    /// Invokes an event handler method on a target component instance with an explicit payload.
    InvokeCallback {
        target: NodeId,
        method: String,
        payload: Value,
    },
}

/// A strongly-typed callable handle to an event handler method on a target component instance.
///
/// Encapsulates the target component's identity (`NodeId`) and method name, guaranteeing
/// compile-time type safety for payloads without requiring direct mutable references.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Callback<T = ()> {
    pub target: NodeId,
    pub method: String,
    _marker: std::marker::PhantomData<fn(T)>,
}

impl<T> Callback<T> {
    pub fn new(target: NodeId, method: impl Into<String>) -> Self {
        Self {
            target,
            method: method.into(),
            _marker: std::marker::PhantomData,
        }
    }

    pub fn target(&self) -> NodeId {
        self.target
    }

    pub fn method(&self) -> &str {
        &self.method
    }
}

impl<T: Into<Value>> Callback<T> {
    /// Invokes the callback on the target component instance with the given typed payload.
    pub fn call(&self, ctx: &mut Context<'_>, payload: T) {
        ctx.actions.push(ContextAction::InvokeCallback {
            target: self.target,
            method: self.method.clone(),
            payload: payload.into(),
        });
    }
}

/// Convenience trait for invoking optional callbacks with a single method call.
pub trait EmitCallback<T> {
    fn emit(&self, ctx: &mut Context<'_>, payload: T);
}

impl<T: Into<Value> + Clone> EmitCallback<T> for Option<Callback<T>> {
    fn emit(&self, ctx: &mut Context<'_>, payload: T) {
        if let Some(cb) = self {
            cb.call(ctx, payload);
        }
    }
}

/// Trait for deserializing or converting an event payload into a strongly-typed Rust method argument.
pub trait FromPayload: Sized {
    fn from_payload(payload: Option<&Value>) -> Option<Self>;
}

impl FromPayload for f64 {
    fn from_payload(payload: Option<&Value>) -> Option<Self> {
        payload.and_then(|v| v.as_f64())
    }
}

impl FromPayload for f32 {
    fn from_payload(payload: Option<&Value>) -> Option<Self> {
        payload.and_then(|v| v.as_f64()).map(|n| n as f32)
    }
}

impl FromPayload for i32 {
    fn from_payload(payload: Option<&Value>) -> Option<Self> {
        payload
            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|n| n as i64)))
            .map(|i| i as i32)
    }
}

impl FromPayload for i64 {
    fn from_payload(payload: Option<&Value>) -> Option<Self> {
        payload.and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|n| n as i64)))
    }
}

impl FromPayload for usize {
    fn from_payload(payload: Option<&Value>) -> Option<Self> {
        payload
            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|n| n as i64)))
            .map(|i| i as usize)
    }
}

impl FromPayload for bool {
    fn from_payload(payload: Option<&Value>) -> Option<Self> {
        match payload {
            Some(Value::Bool(b)) => Some(*b),
            _ => None,
        }
    }
}

impl FromPayload for String {
    fn from_payload(payload: Option<&Value>) -> Option<Self> {
        match payload {
            Some(Value::String(s)) | Some(Value::Color(s)) => Some(s.clone()),
            Some(Value::Enum { variant, .. }) => Some(variant.clone()),
            _ => None,
        }
    }
}

impl FromPayload for Value {
    fn from_payload(payload: Option<&Value>) -> Option<Self> {
        payload.cloned()
    }
}

impl FromPayload for () {
    fn from_payload(_payload: Option<&Value>) -> Option<Self> {
        Some(())
    }
}

impl<T: FromPayload> FromPayload for Option<T> {
    fn from_payload(payload: Option<&Value>) -> Option<Self> {
        Some(T::from_payload(payload))
    }
}

/// A strongly typed handle to a scrollable viewport (either the outer window or a specific `ScrollView`).
pub struct ViewHandle<'b> {
    pub target: NodeId,
    actions: &'b mut Vec<ContextAction>,
}

impl<'b> ViewHandle<'b> {
    pub fn new(target: NodeId, actions: &'b mut Vec<ContextAction>) -> Self {
        Self { target, actions }
    }

    /// Whether this handle represents the main window viewport.
    pub fn is_window(&self) -> bool {
        self.target.is_window()
    }

    /// The target `NodeId` for this view (either `NodeId::WINDOW` or a `ScrollView` node).
    pub fn target(&self) -> NodeId {
        self.target
    }

    /// Scrolls to bring `target_node` into view within this view/pane.
    pub fn scroll_to(&mut self, target_node: NodeId) {
        let container = if self.is_window() {
            None
        } else {
            Some(self.target)
        };
        self.actions.push(ContextAction::ScrollToNode {
            target: target_node,
            container,
        });
    }
}

/// Execution context passed to component lifecycle and event methods.
///
/// Gives components access to read their input ports, query children by structured key,
/// record state mutations for the one-way pipeline, and queue high-level context actions.
pub struct Context<'a> {
    node_id: NodeId,
    event_target: Option<NodeId>,
    parent_id: Option<NodeId>,
    key: Option<&'a ComponentKey>,
    ports: &'a HashMap<String, Value>,
    layout: &'a ResolvedLayout,
    mutations: Vec<(String, Value)>,
    actions: Vec<ContextAction>,
}

impl<'a> Context<'a> {
    pub fn new(
        node_id: NodeId,
        parent_id: Option<NodeId>,
        key: Option<&'a ComponentKey>,
        ports: &'a HashMap<String, Value>,
        layout: &'a ResolvedLayout,
    ) -> Self {
        Self::with_target(node_id, None, parent_id, key, ports, layout)
    }

    pub fn with_target(
        node_id: NodeId,
        event_target: Option<NodeId>,
        parent_id: Option<NodeId>,
        key: Option<&'a ComponentKey>,
        ports: &'a HashMap<String, Value>,
        layout: &'a ResolvedLayout,
    ) -> Self {
        Self {
            node_id,
            event_target,
            parent_id,
            key,
            ports,
            layout,
            mutations: Vec::new(),
            actions: Vec::new(),
        }
    }

    /// The component instance's expanded `NodeId`.
    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    /// The component instance's parent `NodeId`, if any.
    pub fn parent_id(&self) -> Option<NodeId> {
        self.parent_id
    }

    /// The component's structured key, if declared.
    pub fn key(&self) -> Option<&ComponentKey> {
        self.key
    }

    /// Reference to the active computed layout.
    pub fn layout(&self) -> &ResolvedLayout {
        self.layout
    }

    /// Reads an input port value passed to this component instance.
    pub fn get_port(&self, name: &str) -> Option<&Value> {
        self.ports.get(name)
    }

    /// Returns a reference to all input ports passed to this component instance.
    pub fn ports(&self) -> &HashMap<String, Value> {
        self.ports
    }

    /// Queues a context action.
    pub fn push_action(&mut self, action: ContextAction) {
        self.actions.push(action);
    }

    /// Reads an input port as an integer.
    pub fn get_port_integer(&self, name: &str) -> Option<i64> {
        self.get_port(name).and_then(|v| v.as_i64())
    }

    /// Reads an input port as a floating-point number.
    pub fn get_port_float(&self, name: &str) -> Option<f64> {
        self.get_port(name).and_then(|v| v.as_f64())
    }

    /// Reads an input port as a distance in logical pixels.
    pub fn get_port_distance(&self, name: &str) -> Option<f64> {
        self.get_port(name).and_then(|v| v.as_distance())
    }

    /// Reads an input port as a floating-point number.
    pub fn get_port_number(&self, name: &str) -> Option<f64> {
        self.get_port(name).and_then(|v| v.as_f64())
    }

    /// Reads an input port as a string slice.
    pub fn get_port_string(&self, name: &str) -> Option<&str> {
        match self.get_port(name)? {
            Value::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Reads an input port as a boolean.
    pub fn get_port_bool(&self, name: &str) -> Option<bool> {
        match self.get_port(name)? {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Looks up a direct child component by its structured key in $O(1)$ relative time.
    pub fn get_child_by_key(&self, key: &ComponentKey) -> Option<NodeId> {
        self.layout
            .nodes
            .iter()
            .find(|n| n.parent == Some(self.node_id) && n.key.as_ref() == Some(key))
            .map(|n| n.id)
    }

    /// Records a reactive state mutation to be applied to the layout DAG at method return.
    pub fn set_state(&mut self, state_name: impl Into<String>, value: impl Into<Value>) {
        self.mutations.push((state_name.into(), value.into()));
    }

    /// Drains and returns all recorded state mutations.
    pub fn take_mutations(&mut self) -> Vec<(String, Value)> {
        std::mem::take(&mut self.mutations)
    }

    /// Returns a typed handle to the window viewport.
    pub fn window(&mut self) -> ViewHandle<'_> {
        ViewHandle::new(NodeId::WINDOW, &mut self.actions)
    }

    /// Reads an input port as a scrollable view handle (e.g. `view = window` or `view: my_scroll_view`).
    /// Returns `None` if the port is not present or does not contain a Node reference.
    pub fn get_view(&mut self, port_name: &str) -> Option<ViewHandle<'_>> {
        let node_id = match self.get_port(port_name)? {
            Value::Node(id) => *id,
            _ => return None,
        };
        Some(ViewHandle::new(node_id, &mut self.actions))
    }

    /// Returns the scrollable view passed to `port_name`, defaulting to `self.window()` if omitted or not a node.
    pub fn view_or_window(&mut self, port_name: &str) -> ViewHandle<'_> {
        if let Some(port_val) = self.get_port(port_name) {
            if let Value::Node(id) = port_val {
                return ViewHandle::new(*id, &mut self.actions);
            }
        }
        self.window()
    }

    /// Resolves an anchor link (relative or absolute) from the current component's position
    /// and returns the target NodeId if found.
    pub fn resolve_anchor(&self, url: &str) -> Option<NodeId> {
        self.layout.resolve_anchor(self.node_id, url).map(|(target, _)| target)
    }

    /// Reads a state variable's active value.
    pub fn get_state(&self, name: &str) -> Option<&Value> {
        self.layout.get_value(self.node_id, name)
    }

    /// Reads a state variable as a floating-point number.
    pub fn get_state_number(&self, name: &str) -> Option<f64> {
        self.get_state(name).and_then(|v| v.as_f64())
    }

    /// Scrolls to bring `target` into view. Returns `true` if `target` exists in the layout, `false` otherwise.
    pub fn scroll_to_node(&mut self, target: NodeId) -> bool {
        self.scroll_to_node_in_container(target, None)
    }

    /// Scrolls to bring `target` into view within an optional container (e.g. ScrollView).
    pub fn scroll_to_node_in_container(&mut self, target: NodeId, container: Option<NodeId>) -> bool {
        if self.layout.get_node(target).is_some() {
            self.actions.push(ContextAction::ScrollToNode { target, container });
            true
        } else {
            false
        }
    }

    /// Resolves an anchor link (relative or absolute) from the current component's position
    /// and queues a scroll action if found. Returns `true` if the anchor was resolved, `false` otherwise.
    pub fn scroll_to_anchor(&mut self, url: &str) -> bool {
        self.scroll_to_anchor_in_container(url, None)
    }

    /// Resolves an anchor link and queues a scroll action within a specific container (e.g. ScrollView).
    pub fn scroll_to_anchor_in_container(&mut self, url: &str, container: Option<NodeId>) -> bool {
        if let Some((target_id, _scope_id)) = self.layout.resolve_anchor(self.node_id, url) {
            self.scroll_to_node_in_container(target_id, container)
        } else {
            false
        }
    }

    /// Queues an action to open an external URL.
    pub fn open_url(&mut self, url: impl Into<String>) {
        self.actions.push(ContextAction::OpenUrl { url: url.into() });
    }

    /// Queues an action to update focus to a target node.
    pub fn set_focus(&mut self, target: Option<NodeId>) {
        self.actions.push(ContextAction::SetFocus { target });
    }

    /// Requests pointer capture for the currently interacting element (or this component if not in an event).
    pub fn capture_pointer(&mut self) {
        let target = self.event_target.unwrap_or(self.node_id);
        self.actions.push(ContextAction::CapturePointer(target));
    }

    /// Requests pointer capture specifically for a target node (e.g. a child element).
    pub fn capture_pointer_for(&mut self, target: NodeId) {
        self.actions.push(ContextAction::CapturePointer(target));
    }

    /// Releases any active pointer capture.
    pub fn release_pointer(&mut self) {
        self.actions.push(ContextAction::ReleasePointer);
    }

    /// Looks up an event handler or callback binding configured on this component's node
    /// and returns a strongly-typed `Callback<T>`.
    pub fn get_callback<T>(&self, port_name: &str) -> Option<Callback<T>> {
        let node = self.layout.get_node(self.node_id)?;
        let binding = node
            .event_handlers
            .get(port_name)
            .or_else(|| {
                if !port_name.starts_with("on_") {
                    let prefixed = format!("on_{}", port_name);
                    node.event_handlers.get(&prefixed)
                } else {
                    node.event_handlers.get(port_name.strip_prefix("on_").unwrap_or(port_name))
                }
            })?;
        match &binding.target {
            EventHandlerTarget::Component(id) => Some(Callback::new(*id, binding.method.clone())),
            EventHandlerTarget::Env(_) => None,
        }
    }

    /// Drains and returns all recorded context actions.
    pub fn take_actions(&mut self) -> Vec<ContextAction> {
        std::mem::take(&mut self.actions)
    }
}

/// Trait implemented by typed Rust companion component structs.
///
/// Encapsulates private state and behavioral methods, communicating with layout
/// strictly through the One-Way Pipeline (reading ports & emitting state mutations).
pub trait Component: Send + 'static {
    /// Lifecycle hook invoked when the component is mounted into the document layout.
    fn on_mount(&mut self, _ctx: &mut Context<'_>) {}

    /// Dispatches an event method call (e.g. `on_click: self.increment`).
    fn dispatch(
        &mut self,
        method: &str,
        event: &mut Event,
        ctx: &mut Context<'_>,
    ) -> Result<(), DispatchError>;
}

/// Constructor function for creating fresh component instances.
pub type ComponentFactory = Box<dyn Fn() -> Box<dyn Component> + Send + Sync>;

fn canonical_companion_stem(path: &str) -> String {
    let norm = crate::compiler::module::normalize_path(std::path::Path::new(path));
    let without_ext = match norm.extension().and_then(|e| e.to_str()) {
        Some("dt" | "rs" | "wasm") => norm.with_extension(""),
        _ => norm,
    };
    without_ext.to_string_lossy().replace('\\', "/")
}

fn stems_match(stem_a: &str, stem_b: &str) -> bool {
    if stem_a == stem_b {
        return true;
    }
    let a_abs = stem_a.starts_with('/');
    let b_abs = stem_b.starts_with('/');
    if a_abs != b_abs {
        let (abs, rel) = if a_abs {
            (stem_a, stem_b)
        } else {
            (stem_b, stem_a)
        };
        if let Some(prefix) = abs.strip_suffix(rel) {
            return prefix.ends_with('/');
        }
    }
    false
}

/// Registry mapping component paths (and fallback names) to their typed Rust constructors and companion files.
#[derive(Default)]
pub struct ComponentRegistry {
    factories: HashMap<String, ComponentFactory>,
    unqualified_factories: HashMap<String, ComponentFactory>,
    path_factories: HashMap<String, ComponentFactory>,
    companion_files: HashMap<String, String>,
    path_companion_files: HashMap<String, String>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an unqualified component constructor by name (used for inline/test components).
    pub fn register<F>(&mut self, name: impl Into<String>, factory: F)
    where
        F: Fn() -> Box<dyn Component> + Send + Sync + 'static,
    {
        let name_str = name.into();
        let shared = std::sync::Arc::new(factory);
        let f1 = {
            let shared = std::sync::Arc::clone(&shared);
            Box::new(move || shared())
        };
        let f2 = Box::new(move || shared());
        self.factories.insert(name_str.clone(), f1);
        self.unqualified_factories.insert(name_str, f2);
    }

    /// Registers a component constructor along with its associated companion file path.
    pub fn register_companion<F>(
        &mut self,
        name: impl Into<String>,
        companion_path: impl Into<String>,
        factory: F,
    ) where
        F: Fn() -> Box<dyn Component> + Send + Sync + 'static,
    {
        let name_str = name.into();
        let path_str = companion_path.into();
        let stem = canonical_companion_stem(&path_str);
        let qualified = format!("{}::{}", stem, name_str);

        let shared = std::sync::Arc::new(factory);
        let f_name = {
            let shared = std::sync::Arc::clone(&shared);
            Box::new(move || shared())
        };
        let f_stem = {
            let shared = std::sync::Arc::clone(&shared);
            Box::new(move || shared())
        };
        let f_qual = Box::new(move || shared());

        self.companion_files.entry(name_str.clone()).or_insert_with(|| path_str.clone());
        self.path_companion_files.insert(stem.clone(), path_str.clone());
        self.path_companion_files.insert(qualified.clone(), path_str);

        self.factories.entry(name_str).or_insert(f_name);
        self.path_factories.insert(stem, f_stem);
        self.path_factories.insert(qualified, f_qual);
    }

    /// Registers a WebAssembly companion component by reading its `.wasm` bytes.
    pub fn register_wasm(&mut self, name: impl Into<String>, wasm_bytes: Vec<u8>) {
        let name_str = name.into();
        self.register(name_str, move || {
            match WasmComponent::from_bytes(&wasm_bytes) {
                Ok(c) => Box::new(c),
                Err(e) => panic!("Failed to instantiate WASM component: {}", e),
            }
        });
    }

    /// Registers a WebAssembly companion component along with its associated companion file path.
    pub fn register_wasm_companion(
        &mut self,
        name: impl Into<String>,
        companion_path: impl Into<String>,
        wasm_bytes: Vec<u8>,
    ) {
        let name_str = name.into();
        let path_str = companion_path.into();
        self.register_companion(name_str, path_str, move || {
            match WasmComponent::from_bytes(&wasm_bytes) {
                Ok(c) => Box::new(c),
                Err(e) => panic!("Failed to instantiate WASM component: {}", e),
            }
        });
    }

    /// Instantiates a fresh `Component` instance for the given component name or path.
    pub fn create_instance(&self, name_or_path: &str) -> Option<Box<dyn Component>> {
        let stem = canonical_companion_stem(name_or_path);
        if let Some(f) = self.path_factories.get(&stem) {
            return Some(f());
        }
        for (k, f) in &self.path_factories {
            if !k.contains("::") && stems_match(k, &stem) {
                return Some(f());
            }
        }
        self.factories.get(name_or_path).map(|f| f())
    }

    /// Instantiates a fresh `Component` instance resolved by its source `.dt` path and component name.
    pub fn create_instance_for_node(
        &self,
        name: &str,
        component_path: Option<&str>,
    ) -> Option<Box<dyn Component>> {
        if let Some(f) = self.unqualified_factories.get(name) {
            return Some(f());
        }
        if let Some(comp_path) = component_path {
            let stem = canonical_companion_stem(comp_path);
            let qualified = format!("{}::{}", stem, name);
            if let Some(f) = self.path_factories.get(&qualified) {
                return Some(f());
            }
            if let Some(f) = self.path_factories.get(&stem) {
                return Some(f());
            }
            for (k, f) in &self.path_factories {
                if let Some((k_stem, k_name)) = k.split_once("::") {
                    if k_name == name && stems_match(k_stem, &stem) {
                        return Some(f());
                    }
                }
            }
            for (k, f) in &self.path_factories {
                if !k.contains("::") && stems_match(k, &stem) {
                    return Some(f());
                }
            }
            return None;
        }
        self.factories.get(name).map(|f| f())
    }

    /// Checks if a companion is registered for the given `.dt` / `.rs` / `.wasm` path.
    pub fn has_companion_path(&self, path: &std::path::Path) -> bool {
        let stem = canonical_companion_stem(&path.to_string_lossy());
        if self.path_factories.contains_key(&stem) {
            return true;
        }
        self.path_factories
            .keys()
            .any(|k| !k.contains("::") && stems_match(k, &stem))
    }

    /// Checks if a component constructor is registered for the given name or path.
    pub fn has_component(&self, name_or_path: &str) -> bool {
        let stem = canonical_companion_stem(name_or_path);
        self.path_factories.contains_key(&stem)
            || self
                .path_factories
                .keys()
                .any(|k| !k.contains("::") && stems_match(k, &stem))
            || self.factories.contains_key(name_or_path)
    }

    /// Returns the associated companion file path for a component name or path, if registered.
    pub fn get_companion_path(&self, name_or_path: &str) -> Option<&str> {
        let stem = canonical_companion_stem(name_or_path);
        if let Some(p) = self.path_companion_files.get(&stem) {
            return Some(p.as_str());
        }
        for (k, p) in &self.path_companion_files {
            if !k.contains("::") && stems_match(k, &stem) {
                return Some(p.as_str());
            }
        }
        self.companion_files.get(name_or_path).map(|s| s.as_str())
    }

    /// Discovers and registers any direct `.wasm` companion file adjacent to the given `.dt` path.
    pub fn auto_discover_wasm<R: crate::compiler::FileResolver>(
        &mut self,
        dt_path: &std::path::Path,
        resolver: &R,
    ) {
        if let Some(wasm_path) = resolver.find_companion_wasm(dt_path) {
            if let Some(stem) = wasm_path.file_stem().and_then(|s| s.to_str()) {
                if !self.has_companion_path(&wasm_path) {
                    if let Ok(bytes) = resolver.read_bytes(&wasm_path) {
                        self.register_wasm_companion(stem, wasm_path.display().to_string(), bytes);
                    }
                }
            }
        }
    }

    /// Discovers and registers any `.wasm` companion files for components declared or imported in a document.
    pub fn auto_discover_wasm_for_document<R: crate::compiler::FileResolver>(
        &mut self,
        doc: &crate::ast::Document,
        doc_or_dir_path: &std::path::Path,
        resolver: &R,
    ) {
        let (base_dir, doc_wasm_path) = if doc_or_dir_path.is_dir() {
            (doc_or_dir_path, None)
        } else {
            (
                doc_or_dir_path.parent().unwrap_or_else(|| std::path::Path::new(".")),
                resolver.find_companion_wasm(doc_or_dir_path),
            )
        };

        for item in &doc.items {
            match item {
                crate::ast::Item::Component(comp) => {
                    let name = comp.name.as_str();
                    let candidate = base_dir.join(format!("{}.wasm", name));
                    if !self.has_companion_path(&candidate) {
                        if let Ok(bytes) = resolver.read_bytes(&candidate) {
                            self.register_wasm_companion(name, candidate.display().to_string(), bytes);
                            continue;
                        }
                    }
                    if let Some(ref doc_wasm) = doc_wasm_path {
                        let doc_stem = canonical_companion_stem(&doc_wasm.to_string_lossy());
                        let qual_key = format!("{}::{}", doc_stem, name);
                        if !self.path_factories.contains_key(&qual_key) {
                            if let Ok(bytes) = resolver.read_bytes(doc_wasm) {
                                self.register_wasm_companion(name, doc_wasm.display().to_string(), bytes);
                            }
                        }
                    }
                }
                crate::ast::Item::Use(u) => {
                    let raw_path = std::path::Path::new(&u.path);
                    let resolved = if raw_path.is_absolute() {
                        raw_path.to_path_buf()
                    } else {
                        base_dir.join(raw_path)
                    };
                    if let Some(wasm_path) = resolver.find_companion_wasm(&resolved) {
                        if let Some(stem) = wasm_path.file_stem().and_then(|s| s.to_str()) {
                            if !self.has_companion_path(&wasm_path) {
                                if let Ok(bytes) = resolver.read_bytes(&wasm_path) {
                                    self.register_wasm_companion(stem, wasm_path.display().to_string(), bytes);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Returns a pre-configured ComponentRegistry containing standard library components.
    pub fn standard() -> Self {
        let mut registry = Self::new();
        registry.register_companion("Button", "components/Button.rs", || {
            Box::new(std_components::Button::default())
        });
        registry.register_companion("Button", "components/material/Button.rs", || {
            Box::new(material_components::Button::default())
        });
        registry.register_companion("Link", "components/Link.rs", || {
            Box::new(std_components::Link::default())
        });
        registry.register_companion("Card", "components/Card.rs", || {
            Box::new(std_components::Card::default())
        });
        registry.register_companion("ScrollView", "components/ScrollView.rs", || {
            Box::new(std_components::ScrollView::default())
        });
        registry.register_companion("Slider", "components/Slider.rs", || {
            Box::new(std_components::Slider::default())
        });
        registry
    }
}

pub mod material_components {
    use super::{Callback, Context, DispatchError, EmitCallback};
    use directedtype_macros::component;

    /// Material UI Button companion component (`components/material/Button.rs`).
    #[derive(Default, Debug, Clone)]
    pub struct Button {
        pub disabled: bool,
        pub hovered: bool,
        pub focused: bool,
        pub pressed: bool,
        pub on_click: Option<Callback<()>>,
    }

    impl Button {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[component]
    impl Button {
        pub fn on_mount(&mut self, ctx: &mut Context<'_>) {
            self.on_click = ctx.get_callback("on_click");
            if let Some(d) = ctx.get_port_bool("disabled") {
                self.disabled = d;
            }
        }

        pub fn click(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            let is_disabled = ctx.get_port_bool("disabled").unwrap_or(self.disabled);
            if is_disabled {
                return Ok(());
            }
            self.on_click.emit(ctx, ());
            Ok(())
        }

        pub fn focus(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            let is_disabled = ctx.get_port_bool("disabled").unwrap_or(self.disabled);
            if is_disabled {
                return Ok(());
            }
            self.focused = true;
            ctx.set_state("focused", true);
            Ok(())
        }

        pub fn blur(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            self.focused = false;
            self.pressed = false;
            ctx.set_state("focused", false);
            ctx.set_state("pressed", false);
            Ok(())
        }

        pub fn pointer_enter(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            let is_disabled = ctx.get_port_bool("disabled").unwrap_or(self.disabled);
            if is_disabled {
                return Ok(());
            }
            self.hovered = true;
            ctx.set_state("hovered", true);
            Ok(())
        }

        pub fn pointer_leave(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            self.hovered = false;
            self.pressed = false;
            ctx.set_state("hovered", false);
            ctx.set_state("pressed", false);
            Ok(())
        }

        pub fn pointer_down(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            let is_disabled = ctx.get_port_bool("disabled").unwrap_or(self.disabled);
            if is_disabled {
                return Ok(());
            }
            self.pressed = true;
            ctx.set_state("pressed", true);
            Ok(())
        }

        pub fn pointer_up(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            self.pressed = false;
            ctx.set_state("pressed", false);
            Ok(())
        }
    }
}

pub mod std_components {
    use super::{Context, DispatchError};
    use directedtype_macros::component;

    /// Standard interactive Button companion component.
    #[derive(Default, Debug, Clone)]
    pub struct Button {
        pub disabled: bool,
    }

    impl Button {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[component]
    impl Button {
        pub fn on_mount(&mut self, ctx: &mut Context<'_>) {
            if let Some(d) = ctx.get_port_bool("disabled") {
                self.disabled = d;
            }
        }

        pub fn click(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            let is_disabled = ctx.get_port_bool("disabled").unwrap_or(self.disabled);
            if is_disabled {
                // Disabled button does nothing; click propagation is consumed by default
            }
            Ok(())
        }
    }

    /// Standard hypertext Link companion component.
    #[derive(Default, Debug, Clone)]
    pub struct Link {
        pub focused: bool,
        pub hovered: bool,
    }

    impl Link {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[component]
    impl Link {
        pub fn click(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            if let Some(url) = ctx.get_port_string("url") {
                let url = url.to_string();
                if let Some(target_id) = ctx.resolve_anchor(&url) {
                    let mut view = ctx.view_or_window("view");
                    view.scroll_to(target_id);
                } else if url.starts_with('#') {
                    eprintln!("[Link] In-page anchor not found: {}", url);
                } else {
                    ctx.open_url(url);
                }
            }
            Ok(())
        }

        pub fn focus(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            self.focused = true;
            ctx.set_state("focused", true);
            Ok(())
        }

        pub fn blur(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            self.focused = false;
            ctx.set_state("focused", false);
            Ok(())
        }

        pub fn pointer_enter(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            self.hovered = true;
            ctx.set_state("hovered", true);
            Ok(())
        }

        pub fn pointer_leave(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            self.hovered = false;
            ctx.set_state("hovered", false);
            Ok(())
        }
    }

    /// Standard surface container Card companion component.
    #[derive(Default, Debug, Clone)]
    pub struct Card {
        pub focused: bool,
    }

    impl Card {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[component]
    impl Card {
        pub fn focus(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            self.focused = true;
            ctx.set_state("focused", true);
            Ok(())
        }

        pub fn blur(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            self.focused = false;
            ctx.set_state("focused", false);
            Ok(())
        }
    }

    /// Standard scroll container ScrollView companion component.
    #[derive(Default, Debug, Clone)]
    pub struct ScrollView {
        pub scroll_y: f64,
        pub scroll_x: f64,
    }

    impl ScrollView {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[component]
    impl ScrollView {
        pub fn on_mount(&mut self, ctx: &mut Context<'_>) {
            if let Some(sy) = ctx.get_state_number("scroll_y") {
                self.scroll_y = sy;
            }
            if let Some(sx) = ctx.get_state_number("scroll_x") {
                self.scroll_x = sx;
            }
        }

        pub fn on_scroll(&mut self, event: &mut crate::interaction::Event, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            if let crate::interaction::EventKind::Scroll { delta_x, delta_y } = event.kind {
                let container = match ctx.layout().get_node(ctx.node_id()) {
                    Some(n) => n,
                    None => return Ok(()),
                };
                let viewport_h = container.rect.height;
                let viewport_w = container.rect.width;

                let clip_node = ctx.layout().nodes.iter().find(|n| n.parent == Some(ctx.node_id()) && n.name == "Clip");
                let (max_scroll_y, max_scroll_x) = if let Some(clip) = clip_node {
                    let mut max_bottom = container.rect.y;
                    let mut max_right = container.rect.x;
                    for n in &ctx.layout().nodes {
                        if n.clip == Some(clip.id) {
                            max_bottom = max_bottom.max(n.rect.y + n.rect.height);
                            max_right = max_right.max(n.rect.x + n.rect.width);
                        }
                    }
                    let padding = ctx.get_port_number("padding").unwrap_or(0.0);
                    let content_h = (max_bottom + padding - container.rect.y).max(0.0);
                    let content_w = (max_right + padding - container.rect.x).max(0.0);
                    ((content_h - viewport_h).max(0.0), (content_w - viewport_w).max(0.0))
                } else {
                    (f64::INFINITY, f64::INFINITY)
                };

                self.scroll_y = (self.scroll_y - delta_y).clamp(0.0, max_scroll_y);
                self.scroll_x = (self.scroll_x - delta_x).clamp(0.0, max_scroll_x);
                ctx.set_state("scroll_y", self.scroll_y);
                ctx.set_state("scroll_x", self.scroll_x);
            }
            Ok(())
        }
    }

    /// Standard interactive Slider companion component.
    #[derive(Default, Debug, Clone)]
    pub struct Slider {
        pub min: f64,
        pub max: f64,
        pub value: f64,
        pub is_dragging: bool,
        pub on_change: Option<super::Callback<f64>>,
    }

    impl Slider {
        pub fn new() -> Self {
            Self {
                min: 0.0,
                max: 100.0,
                value: 0.0,
                is_dragging: false,
                on_change: None,
            }
        }
    }

    #[component]
    impl Slider {
        pub fn on_mount(&mut self, ctx: &mut Context<'_>) {
            self.on_change = ctx.get_callback("on_change");
            if let Some(min) = ctx.get_port_number("min") {
                self.min = min;
            }
            if let Some(max) = ctx.get_port_number("max") {
                self.max = max;
            }
            if let Some(val) = ctx.get_port_number("value") {
                self.value = val.clamp(self.min, self.max);
                ctx.set_state("current_value", self.value);
            }
        }

        pub fn on_pointer_down(
            &mut self,
            event: &mut crate::interaction::Event,
            ctx: &mut Context<'_>,
        ) -> Result<(), DispatchError> {
            self.is_dragging = true;
            ctx.capture_pointer();
            self.handle_drag(event.local_point.x, ctx);
            Ok(())
        }

        pub fn on_pointer_move(
            &mut self,
            event: &mut crate::interaction::Event,
            ctx: &mut Context<'_>,
        ) -> Result<(), DispatchError> {
            if self.is_dragging {
                self.handle_drag(event.local_point.x, ctx);
            }
            Ok(())
        }

        pub fn on_pointer_up(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            self.is_dragging = false;
            ctx.release_pointer();
            Ok(())
        }

        fn handle_drag(&mut self, local_x: f64, ctx: &mut Context<'_>) {
            use super::EmitCallback;
            let width = ctx.get_port_number("width").unwrap_or(200.0);
            let ratio = if width > 0.0 {
                (local_x / width).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let new_value = self.min + ratio * (self.max - self.min);

            self.value = new_value;
            ctx.set_state("current_value", new_value);
            self.on_change.emit(ctx, new_value);
        }
    }
}

pub use std_components::{Button, Card, Link, ScrollView, Slider};


/// Container managing live `Component` instances keyed by their `NodeId`.
#[derive(Default)]
pub struct InstanceManager {
    instances: HashMap<NodeId, Box<dyn Component>>,
}

impl InstanceManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, node_id: NodeId, instance: Box<dyn Component>) {
        self.instances.insert(node_id, instance);
    }

    pub fn get(&self, node_id: NodeId) -> Option<&dyn Component> {
        self.instances.get(&node_id).map(|b| b.as_ref())
    }

    pub fn get_mut(&mut self, node_id: NodeId) -> Option<&mut Box<dyn Component>> {
        self.instances.get_mut(&node_id)
    }

    pub fn contains(&self, node_id: NodeId) -> bool {
        self.instances.contains_key(&node_id)
    }

    pub fn remove(&mut self, node_id: NodeId) -> Option<Box<dyn Component>> {
        self.instances.remove(&node_id)
    }

    pub fn len(&self) -> usize {
        self.instances.len()
    }

    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&NodeId, &mut Box<dyn Component>)> {
        self.instances.iter_mut()
    }
}

impl std::fmt::Debug for InstanceManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstanceManager")
            .field("instance_count", &self.instances.len())
            .finish()
    }
}
