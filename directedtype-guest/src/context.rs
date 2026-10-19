use crate::protocol::{Action, StateMutation};
use crate::value::Value;
use std::collections::HashMap;

/// Execution context passed to guest component lifecycle and event methods.
pub struct Context<'a> {
    node_id: u64,
    parent_id: Option<u64>,
    ports: &'a HashMap<String, Value>,
    mutations: Vec<StateMutation>,
    actions: Vec<Action>,
}

impl<'a> Context<'a> {
    pub fn new(node_id: u64, parent_id: Option<u64>, ports: &'a HashMap<String, Value>) -> Self {
        Self {
            node_id,
            parent_id,
            ports,
            mutations: Vec::new(),
            actions: Vec::new(),
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

