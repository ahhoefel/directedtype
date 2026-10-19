pub mod context;
pub mod event;
pub mod protocol;
pub mod value;

pub use context::Context;
pub use event::{Event, EventKind, Modifiers, MouseButton, Point};
pub use protocol::{Action, ComponentResponse, DispatchRequest, MountRequest, StateMutation};
pub use value::Value;

pub use directedtype_macros::guest_component as component;

use std::collections::HashMap;
use std::sync::Mutex;

/// Trait implemented by guest companion component structs.
pub trait Component: Default + 'static {
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

/// Error returned when event dispatch fails in guest logic.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DispatchError {
    #[error("Component '{component}' has no method '{method}'")]
    MethodNotFound {
        component: String,
        method: String,
    },
    #[error("Custom dispatch error: {0}")]
    Custom(String),
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

// ---------------------------------------------------------------------------
// C-ABI Memory & Entry Point Helpers
// ---------------------------------------------------------------------------

pub fn alloc_raw(size: usize) -> *mut u8 {
    let mut vec = Vec::with_capacity(size);
    let ptr = vec.as_mut_ptr();
    std::mem::forget(vec);
    ptr
}

pub unsafe fn dealloc_raw(ptr: *mut u8, size: usize) {
    if !ptr.is_null() && size > 0 {
        let _ = Vec::from_raw_parts(ptr, size, size);
    }
}

#[inline]
pub fn pack_ptr_len(ptr: *const u8, len: usize) -> u64 {
    ((ptr as u64) << 32) | (len as u64 & 0xFFFF_FFFF)
}

#[inline]
pub fn unpack_ptr_len(packed: u64) -> (u32, u32) {
    let ptr = (packed >> 32) as u32;
    let len = (packed & 0xFFFF_FFFF) as u32;
    (ptr, len)
}

pub fn serialize_response_raw(resp: &ComponentResponse) -> u64 {
    let bytes = match serde_json::to_vec(resp) {
        Ok(b) => b,
        Err(e) => {
            let err_resp = ComponentResponse {
                mutations: Vec::new(),
                actions: Vec::new(),
                error: Some(format!("Guest serialization error: {}", e)),
            };
            serde_json::to_vec(&err_resp).unwrap_or_default()
        }
    };
    let len = bytes.len();
    let ptr = bytes.as_ptr();
    std::mem::forget(bytes);
    pack_ptr_len(ptr, len)
}

pub fn mount_handler<T: Component>(
    instances: &Mutex<HashMap<u64, T>>,
    ptr: *const u8,
    len: usize,
) -> u64 {
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    let req: MountRequest = match serde_json::from_slice(slice) {
        Ok(r) => r,
        Err(e) => {
            return serialize_response_raw(&ComponentResponse {
                mutations: Vec::new(),
                actions: Vec::new(),
                error: Some(format!("Invalid MountRequest JSON: {}", e)),
            });
        }
    };

    let mut component = T::default();
    let mut ctx = Context::new(req.node_id, req.parent_id, &req.ports);
    component.on_mount(&mut ctx);

    let resp = ComponentResponse {
        mutations: ctx.take_mutations(),
        actions: ctx.take_actions(),
        error: None,
    };

    if let Ok(mut map) = instances.lock() {
        map.insert(req.node_id, component);
    }

    serialize_response_raw(&resp)
}

pub fn dispatch_handler<T: Component>(
    instances: &Mutex<HashMap<u64, T>>,
    ptr: *const u8,
    len: usize,
) -> u64 {
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    let req: DispatchRequest = match serde_json::from_slice(slice) {
        Ok(r) => r,
        Err(e) => {
            return serialize_response_raw(&ComponentResponse {
                mutations: Vec::new(),
                actions: Vec::new(),
                error: Some(format!("Invalid DispatchRequest JSON: {}", e)),
            });
        }
    };

    let mut instances_guard = match instances.lock() {
        Ok(g) => g,
        Err(e) => {
            return serialize_response_raw(&ComponentResponse {
                mutations: Vec::new(),
                actions: Vec::new(),
                error: Some(format!("Failed to lock instances: {}", e)),
            });
        }
    };

    let component = match instances_guard.get_mut(&req.node_id) {
        Some(c) => c,
        None => {
            return serialize_response_raw(&ComponentResponse {
                mutations: Vec::new(),
                actions: Vec::new(),
                error: Some(format!("Instance not found for node {}", req.node_id)),
            });
        }
    };

    let mut event = req.event.unwrap_or(Event {
        kind: EventKind::Click {
            button: MouseButton::Left,
        },
        global_point: Point::default(),
        local_point: Point::default(),
        modifiers: Modifiers::default(),
        target: req.node_id,
        current_target: req.node_id,
        payload: None,
        propagation_continued: false,
    });

    let mut ctx = Context::new(req.node_id, None, &req.ports);
    let result = component.dispatch(&req.method, &mut event, &mut ctx);

    let (mutations, actions, error) = match result {
        Ok(()) => (ctx.take_mutations(), ctx.take_actions(), None),
        Err(e) => (Vec::new(), Vec::new(), Some(e.to_string())),
    };

    serialize_response_raw(&ComponentResponse {
        mutations,
        actions,
        error,
    })
}

/// Macro to export a component as a WebAssembly guest module.
#[macro_export]
macro_rules! export_component {
    ($comp_ty:ty) => {
        static INSTANCES: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<u64, $comp_ty>>> =
            std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

        #[no_mangle]
        pub extern "C" fn __dt_alloc(size: usize) -> *mut u8 {
            $crate::alloc_raw(size)
        }

        #[no_mangle]
        pub unsafe extern "C" fn __dt_dealloc(ptr: *mut u8, size: usize) {
            $crate::dealloc_raw(ptr, size)
        }

        #[no_mangle]
        pub extern "C" fn __dt_mount(ptr: *const u8, len: usize) -> u64 {
            $crate::mount_handler::<$comp_ty>(&INSTANCES, ptr, len)
        }

        #[no_mangle]
        pub extern "C" fn __dt_dispatch(ptr: *const u8, len: usize) -> u64 {
            $crate::dispatch_handler::<$comp_ty>(&INSTANCES, ptr, len)
        }
    };
}

pub mod prelude {
    pub use crate::context::Context;
    pub use crate::event::{Event, EventKind, Modifiers, MouseButton, Point};
    pub use crate::value::Value;
    pub use crate::{export_component, Component, DispatchError, FromPayload};
    pub use directedtype_macros::guest_component as component;
}
