use crate::ast::ComponentKey;
use crate::compiler::expanded::NodeId;
use crate::compiler::layout::ResolvedLayout;
use crate::compiler::value::Value;
use crate::interaction::Event;
use std::collections::HashMap;

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
#[derive(Debug, Clone, PartialEq, Eq)]
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
}

/// Execution context passed to component lifecycle and event methods.
///
/// Gives components access to read their input ports, query children by structured key,
/// record state mutations for the one-way pipeline, and queue high-level context actions.
pub struct Context<'a> {
    node_id: NodeId,
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
        Self {
            node_id,
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

    /// Reads an input port as a floating-point number.
    pub fn get_port_number(&self, name: &str) -> Option<f64> {
        match self.get_port(name)? {
            Value::Number(n) => Some(*n),
            _ => None,
        }
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

    /// Scrolls to bring `target` into view. Returns `true` if `target` exists in the layout, `false` otherwise.
    pub fn scroll_to_node(&mut self, target: NodeId) -> bool {
        self.scroll_to_node_in_container(target, None)
    }

    /// Scrolls to bring `target` into view within an optional container (e.g. ScrollPane).
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

    /// Resolves an anchor link and queues a scroll action within a specific container (e.g. ScrollPane).
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

/// Registry mapping component names to their typed Rust constructors and companion files.
#[derive(Default)]
pub struct ComponentRegistry {
    factories: HashMap<String, ComponentFactory>,
    companion_files: HashMap<String, String>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a component constructor by name.
    pub fn register<F>(&mut self, name: impl Into<String>, factory: F)
    where
        F: Fn() -> Box<dyn Component> + Send + Sync + 'static,
    {
        self.factories.insert(name.into(), Box::new(factory));
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
        self.companion_files
            .insert(name_str.clone(), companion_path.into());
        self.factories.insert(name_str, Box::new(factory));
    }

    /// Instantiates a fresh `Component` instance for the given component name.
    pub fn create_instance(&self, name: &str) -> Option<Box<dyn Component>> {
        self.factories.get(name).map(|f| f())
    }

    /// Checks if a component constructor is registered for the given name.
    pub fn has_component(&self, name: &str) -> bool {
        self.factories.contains_key(name)
    }

    /// Returns the associated companion file path, if registered.
    pub fn get_companion_path(&self, name: &str) -> Option<&str> {
        self.companion_files.get(name).map(|s| s.as_str())
    }

    /// Returns a pre-configured ComponentRegistry containing standard library components.
    pub fn standard() -> Self {
        let mut registry = Self::new();
        registry.register_companion("Button", "components/Button.rs", || {
            Box::new(std_components::Button::default())
        });
        registry.register_companion("Link", "components/Link.rs", || {
            Box::new(std_components::Link::default())
        });
        registry.register_companion("Card", "components/Card.rs", || {
            Box::new(std_components::Card::default())
        });
        registry
    }
}

pub mod std_components {
    use super::{Component, Context, DispatchError};
    use crate::interaction::Event;

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

    impl Component for Button {
        fn on_mount(&mut self, ctx: &mut Context<'_>) {
            if let Some(d) = ctx.get_port_bool("disabled") {
                self.disabled = d;
            }
        }

        fn dispatch(
            &mut self,
            method: &str,
            event: &mut Event,
            ctx: &mut Context<'_>,
        ) -> Result<(), DispatchError> {
            match method {
                "click" => {
                    let is_disabled = ctx.get_port_bool("disabled").unwrap_or(self.disabled);
                    if is_disabled {
                        event.stop_propagation();
                    }
                    Ok(())
                }
                _ => Err(DispatchError::MethodNotFound {
                    component: "Button".into(),
                    method: method.into(),
                }),
            }
        }
    }

    /// Standard hypertext Link companion component.
    #[derive(Default, Debug, Clone)]
    pub struct Link {
        pub focused: bool,
    }

    impl Link {
        pub fn new() -> Self {
            Self::default()
        }
    }

    impl Component for Link {
        fn dispatch(
            &mut self,
            method: &str,
            event: &mut Event,
            ctx: &mut Context<'_>,
        ) -> Result<(), DispatchError> {
            match method {
                "click" => {
                    if event.propagation_stopped {
                        return Ok(());
                    }
                    event.stop_propagation();

                    if let Some(url) = ctx.get_port_string("url") {
                        let url = url.to_string();
                        let pane_container = ctx.get_port("pane").and_then(|v| v.as_node());

                        if url.starts_with('#') {
                            if !ctx.scroll_to_anchor_in_container(&url, pane_container) {
                                eprintln!("[Link] In-page anchor not found: {}", url);
                            }
                        } else if ctx.scroll_to_anchor_in_container(&url, pane_container) {
                            // Scrolled to relative/scoped anchor path without '#'
                        } else {
                            ctx.open_url(url);
                        }
                    }
                    Ok(())
                }
                "focus" => {
                    self.focused = true;
                    ctx.set_state("focused", true);
                    event.stop_propagation();
                    Ok(())
                }
                "blur" => {
                    self.focused = false;
                    ctx.set_state("focused", false);
                    event.stop_propagation();
                    Ok(())
                }
                _ => Err(DispatchError::MethodNotFound {
                    component: "Link".into(),
                    method: method.into(),
                }),
            }
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

    impl Component for Card {
        fn dispatch(
            &mut self,
            method: &str,
            _event: &mut Event,
            ctx: &mut Context<'_>,
        ) -> Result<(), DispatchError> {
            match method {
                "focus" => {
                    self.focused = true;
                    ctx.set_state("focused", true);
                    Ok(())
                }
                "blur" => {
                    self.focused = false;
                    ctx.set_state("focused", false);
                    Ok(())
                }
                _ => Err(DispatchError::MethodNotFound {
                    component: "Card".into(),
                    method: method.into(),
                }),
            }
        }
    }
}

pub use std_components::{Button, Card, Link};


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
