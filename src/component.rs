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

/// Execution context passed to component lifecycle and event methods.
///
/// Gives components access to read their input ports, query children by structured key,
/// and record state mutations for the one-way pipeline.
pub struct Context<'a> {
    node_id: NodeId,
    parent_id: Option<NodeId>,
    key: Option<&'a ComponentKey>,
    ports: &'a HashMap<String, Value>,
    layout: &'a ResolvedLayout,
    mutations: Vec<(String, Value)>,
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
        event: &Event,
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
}

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
