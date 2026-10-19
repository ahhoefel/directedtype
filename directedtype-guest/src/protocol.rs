use crate::event::Event;
use crate::value::Value;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Host-to-guest request for mounting a component instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountRequest {
    pub node_id: u64,
    pub parent_id: Option<u64>,
    pub ports: HashMap<String, Value>,
}

/// Host-to-guest request for dispatching an event method on a component instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchRequest {
    pub node_id: u64,
    pub method: String,
    pub event: Option<Event>,
    pub ports: HashMap<String, Value>,
}

/// A reactive state mutation emitted by a guest component.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StateMutation {
    pub name: String,
    pub value: Value,
}

/// A high-level context action emitted by a guest component method.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Action {
    ScrollToNode {
        target: u64,
        container: Option<u64>,
    },
    OpenUrl {
        url: String,
    },
    SetFocus {
        target: Option<u64>,
    },
    CapturePointer(u64),
    ReleasePointer,
    InvokeCallback {
        target: u64,
        method: String,
        payload: Value,
    },
}

/// Guest-to-host response containing mutations, actions, and any error message.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ComponentResponse {
    pub mutations: Vec<StateMutation>,
    pub actions: Vec<Action>,
    pub error: Option<String>,
}

