use crate::protocol::{Action, StateMutation};
use crate::value::Value;
use std::collections::HashMap;
use std::marker::PhantomData;

/// Strongly-typed handle to a parent or peer component callback method.
#[derive(Debug, Clone, PartialEq)]
pub struct Callback<T = ()> {
    pub target: u64,
    pub method: String,
    _phantom: PhantomData<T>,
}

impl<T> Callback<T> {
    pub fn new(target: u64, method: impl Into<String>) -> Self {
        Self {
            target,
            method: method.into(),
            _phantom: PhantomData,
        }
    }
}

impl<T: Into<Value>> Callback<T> {
    pub fn emit(&self, ctx: &mut Context<'_>, payload: T) {
        ctx.actions.push(Action::InvokeCallback {
            target: self.target,
            method: self.method.clone(),
            payload: payload.into(),
        });
    }
}

/// Extension trait allowing `Option<Callback<T>>` to emit directly without unwrapping.
pub trait EmitCallback<T> {
    fn emit(&self, ctx: &mut Context<'_>, payload: T);
}

impl<T: Into<Value>> EmitCallback<T> for Option<Callback<T>> {
    fn emit(&self, ctx: &mut Context<'_>, payload: T) {
        if let Some(cb) = self {
            cb.emit(ctx, payload);
        }
    }
}

/// Execution context passed to guest component lifecycle and event methods.
pub struct Context<'a> {
    node_id: u64,
    parent_id: Option<u64>,
    ports: &'a HashMap<String, Value>,
    callbacks: &'a HashMap<String, (u64, String)>,
    mutations: Vec<StateMutation>,
    actions: Vec<Action>,
    call_default_requested: bool,
}

impl<'a> Context<'a> {
    pub fn new(node_id: u64, parent_id: Option<u64>, ports: &'a HashMap<String, Value>) -> Self {
        static EMPTY_CALLBACKS: std::sync::LazyLock<HashMap<String, (u64, String)>> =
            std::sync::LazyLock::new(HashMap::new);
        Self {
            node_id,
            parent_id,
            ports,
            callbacks: &EMPTY_CALLBACKS,
            mutations: Vec::new(),
            actions: Vec::new(),
            call_default_requested: false,
        }
    }

    pub fn with_callbacks(
        node_id: u64,
        parent_id: Option<u64>,
        ports: &'a HashMap<String, Value>,
        callbacks: &'a HashMap<String, (u64, String)>,
    ) -> Self {
        Self {
            node_id,
            parent_id,
            ports,
            callbacks,
            mutations: Vec::new(),
            actions: Vec::new(),
            call_default_requested: false,
        }
    }

    /// The component instance's node ID.
    pub fn node_id(&self) -> u64 {
        self.node_id
    }

    /// The component instance's parent node ID, if any.
    pub fn parent_id(&self) -> Option<u64> {
        self.parent_id
    }

    /// Reads an input port value passed to this component instance.
    pub fn get_port(&self, name: &str) -> Option<&Value> {
        self.ports.get(name)
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
        self.get_port(name).and_then(|v| v.as_str())
    }

    /// Reads an input port as a boolean.
    pub fn get_port_bool(&self, name: &str) -> Option<bool> {
        self.get_port(name).and_then(|v| v.as_bool())
    }

    /// Reads a state variable or property value.
    pub fn get_state(&self, name: &str) -> Option<&Value> {
        self.get_port(name)
    }

    /// Reads a state variable or property value as an integer.
    pub fn get_state_integer(&self, name: &str) -> Option<i64> {
        self.get_port_integer(name)
    }

    /// Reads a state variable or property value as a floating-point number.
    pub fn get_state_float(&self, name: &str) -> Option<f64> {
        self.get_port_float(name)
    }

    /// Reads a state variable or property value as a distance in logical pixels.
    pub fn get_state_distance(&self, name: &str) -> Option<f64> {
        self.get_port_distance(name)
    }

    /// Reads a state variable or property value as a floating-point number.
    pub fn get_state_number(&self, name: &str) -> Option<f64> {
        self.get_port_number(name)
    }

    /// Reads a state variable or property value as a string slice.
    pub fn get_state_string(&self, name: &str) -> Option<&str> {
        self.get_port_string(name)
    }

    /// Reads a state variable or property value as a boolean.
    pub fn get_state_bool(&self, name: &str) -> Option<bool> {
        self.get_port_bool(name)
    }

    /// Records a reactive state mutation to be applied to the layout DAG at method return.
    pub fn set_state(&mut self, state_name: impl Into<String>, value: impl Into<Value>) {
        self.mutations.push(StateMutation {
            name: state_name.into(),
            value: value.into(),
        });
    }

    /// Drains and returns all recorded state mutations.
    pub fn take_mutations(&mut self) -> Vec<StateMutation> {
        std::mem::take(&mut self.mutations)
    }

    /// Drains and returns all recorded actions.
    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    /// Requests that the component's default event handler for the current event be executed.
    pub fn call_default(&mut self) {
        self.call_default_requested = true;
    }

    /// Returns whether `call_default()` was requested during this dispatch.
    pub fn take_call_default(&mut self) -> bool {
        std::mem::take(&mut self.call_default_requested)
    }

    /// Looks up a callback binding configured on this component's node and returns a strongly-typed `Callback<T>`.
    pub fn get_callback<T>(&self, port_name: &str) -> Option<Callback<T>> {
        let alt_name = if !port_name.starts_with("on_") {
            format!("on_{}", port_name)
        } else {
            port_name.strip_prefix("on_").unwrap_or(port_name).to_string()
        };
        let (target, method) = self
            .callbacks
            .get(port_name)
            .or_else(|| self.callbacks.get(&alt_name))?;
        Some(Callback::new(*target, method.clone()))
    }

    /// Requests opening an external URL.
    pub fn open_url(&mut self, url: impl Into<String>) {
        self.actions.push(Action::OpenUrl { url: url.into() });
    }

    /// Requests setting focus to a target node (or clearing focus if None).
    pub fn set_focus(&mut self, target: Option<u64>) {
        self.actions.push(Action::SetFocus { target });
    }

    /// Requests pointer capture for this component.
    pub fn capture_pointer(&mut self) {
        self.actions.push(Action::CapturePointer(self.node_id));
    }

    /// Releases any active pointer capture.
    pub fn release_pointer(&mut self) {
        self.actions.push(Action::ReleasePointer);
    }

    /// Scrolls to bring `target` into view.
    pub fn scroll_to_node(&mut self, target: u64) {
        self.actions.push(Action::ScrollToNode {
            target,
            container: None,
        });
    }

    /// Scrolls to bring `target` into view within a specific container (e.g. ScrollView).
    pub fn scroll_to_node_in_container(&mut self, target: u64, container: Option<u64>) {
        self.actions.push(Action::ScrollToNode { target, container });
    }
}

