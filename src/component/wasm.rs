use crate::component::{Component, Context, ContextAction, DispatchError};
use crate::compiler::expanded::NodeId;
use crate::compiler::value::Value;
use crate::interaction::{Event, EventKind, MouseButton};
use directedtype_guest::protocol::{
    Action as GuestAction, ComponentResponse, DispatchRequest, MountRequest,
};
use directedtype_guest::Value as GuestValue;
use std::collections::HashMap;
use wasmi::{Engine, Instance, Linker, Memory, Module, Store, TypedFunc};

/// Host wrapper around a compiled WebAssembly guest component instance.
pub struct WasmComponent {
    _engine: Engine,
    store: Store<()>,
    _instance: Instance,
    memory: Memory,
    alloc_fn: TypedFunc<u32, u32>,
    dealloc_fn: TypedFunc<(u32, u32), ()>,
    mount_fn: TypedFunc<(u32, u32), u64>,
    dispatch_fn: TypedFunc<(u32, u32), u64>,
}

// wasmi Store is not Send by default unless wrapped or used per thread.
// Here Component requires Send + 'static.
unsafe impl Send for WasmComponent {}

impl WasmComponent {
    /// Loads a WebAssembly module from bytecode and instantiates it.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DispatchError> {
        let engine = Engine::default();
        let module = Module::new(&engine, bytes).map_err(|e| {
            DispatchError::Custom(format!("Failed to compile WASM module: {}", e))
        })?;
        let linker = Linker::new(&engine);
        let mut store = Store::new(&engine, ());
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| {
                DispatchError::Custom(format!("Failed to instantiate WASM component: {}", e))
            })?;

        let memory = instance
            .get_memory(&store, "memory")
            .ok_or_else(|| DispatchError::Custom("WASM component missing 'memory' export".into()))?;

        let alloc_fn = instance
            .get_typed_func::<u32, u32>(&store, "__dt_alloc")
            .map_err(|e| {
                DispatchError::Custom(format!("WASM missing '__dt_alloc' export: {}", e))
            })?;

        let dealloc_fn = instance
            .get_typed_func::<(u32, u32), ()>(&store, "__dt_dealloc")
            .map_err(|e| {
                DispatchError::Custom(format!("WASM missing '__dt_dealloc' export: {}", e))
            })?;

        let mount_fn = instance
            .get_typed_func::<(u32, u32), u64>(&store, "__dt_mount")
            .map_err(|e| {
                DispatchError::Custom(format!("WASM missing '__dt_mount' export: {}", e))
            })?;

        let dispatch_fn = instance
            .get_typed_func::<(u32, u32), u64>(&store, "__dt_dispatch")
            .map_err(|e| {
                DispatchError::Custom(format!("WASM missing '__dt_dispatch' export: {}", e))
            })?;

        Ok(Self {
            _engine: engine,
            store,
            _instance: instance,
            memory,
            alloc_fn,
            dealloc_fn,
            mount_fn,
            dispatch_fn,
        })
    }

    fn call_wasm(&mut self, is_mount: bool, json_bytes: &[u8]) -> Result<ComponentResponse, DispatchError> {
        let len = json_bytes.len() as u32;
        let ptr = self
            .alloc_fn
            .call(&mut self.store, len)
            .map_err(|e| DispatchError::Custom(format!("WASM __dt_alloc failed: {}", e)))?;

        self.memory
            .write(&mut self.store, ptr as usize, json_bytes)
            .map_err(|e| DispatchError::Custom(format!("WASM memory write failed: {}", e)))?;

        let packed_res = if is_mount {
            self.mount_fn
                .call(&mut self.store, (ptr, len))
                .map_err(|e| DispatchError::Custom(format!("WASM __dt_mount call failed: {}", e)))?
        } else {
            self.dispatch_fn
                .call(&mut self.store, (ptr, len))
                .map_err(|e| DispatchError::Custom(format!("WASM __dt_dispatch call failed: {}", e)))?
        };

        // Deallocate input buffer
        let _ = self.dealloc_fn.call(&mut self.store, (ptr, len));

        // Unpack response (ptr: high 32 bits, len: low 32 bits)
        let res_ptr = (packed_res >> 32) as u32;
        let res_len = (packed_res & 0xFFFF_FFFF) as u32;

        let mut res_bytes = vec![0u8; res_len as usize];
        self.memory
            .read(&self.store, res_ptr as usize, &mut res_bytes)
            .map_err(|e| DispatchError::Custom(format!("WASM memory read failed: {}", e)))?;

        // Deallocate response buffer in guest
        let _ = self.dealloc_fn.call(&mut self.store, (res_ptr, res_len));

        let resp: ComponentResponse = serde_json::from_slice(&res_bytes).map_err(|e| {
            DispatchError::Custom(format!("Failed to parse ComponentResponse from guest: {}", e))
        })?;

        if let Some(err) = resp.error {
            return Err(DispatchError::Custom(err));
        }

        Ok(resp)
    }

    fn apply_response(&self, resp: ComponentResponse, ctx: &mut Context<'_>) {
        for mutation in resp.mutations {
            ctx.set_state(mutation.name, guest_to_host_value(mutation.value));
        }

        for action in resp.actions {
            match action {
                GuestAction::ScrollToNode { target, container } => {
                    ctx.push_action(ContextAction::ScrollToNode {
                        target: NodeId(target as usize),
                        container: container.map(|c| NodeId(c as usize)),
                    });
                }
                GuestAction::OpenUrl { url } => {
                    ctx.push_action(ContextAction::OpenUrl { url });
                }
                GuestAction::SetFocus { target } => {
                    ctx.push_action(ContextAction::SetFocus {
                        target: target.map(|t| NodeId(t as usize)),
                    });
                }
                GuestAction::CapturePointer(t) => {
                    ctx.push_action(ContextAction::CapturePointer(NodeId(t as usize)));
                }
                GuestAction::ReleasePointer => {
                    ctx.push_action(ContextAction::ReleasePointer);
                }
                GuestAction::InvokeCallback {
                    target,
                    method,
                    payload,
                } => {
                    ctx.push_action(ContextAction::InvokeCallback {
                        target: NodeId(target as usize),
                        method,
                        payload: guest_to_host_value(payload),
                    });
                }
            }
        }
    }
}

impl Component for WasmComponent {
    fn on_mount(&mut self, ctx: &mut Context<'_>) {
        let mut guest_ports = HashMap::new();
        for (k, v) in ctx.ports() {
            guest_ports.insert(k.clone(), host_to_guest_value(v));
        }

        let req = MountRequest {
            node_id: ctx.node_id().0 as u64,
            parent_id: ctx.parent_id().map(|p| p.0 as u64),
            ports: guest_ports,
        };

        if let Ok(json_bytes) = serde_json::to_vec(&req) {
            if let Ok(resp) = self.call_wasm(true, &json_bytes) {
                self.apply_response(resp, ctx);
            }
        }
    }

    fn dispatch(
        &mut self,
        method: &str,
        event: &mut Event,
        ctx: &mut Context<'_>,
    ) -> Result<(), DispatchError> {
        let mut guest_ports = HashMap::new();
        for (k, v) in ctx.ports() {
            guest_ports.insert(k.clone(), host_to_guest_value(v));
        }

        let guest_event = host_to_guest_event(event);

        let req = DispatchRequest {
            node_id: ctx.node_id().0 as u64,
            method: method.to_string(),
            event: Some(guest_event),
            ports: guest_ports,
        };

        let json_bytes = serde_json::to_vec(&req).map_err(|e| {
            DispatchError::Custom(format!("Failed to serialize DispatchRequest: {}", e))
        })?;

        let resp = self.call_wasm(false, &json_bytes)?;
        self.apply_response(resp, ctx);
        Ok(())
    }
}

fn host_to_guest_value(val: &Value) -> GuestValue {
    match val {
        Value::Number(n) => GuestValue::Number(*n),
        Value::String(s) => GuestValue::String(s.clone()),
        Value::Bool(b) => GuestValue::Bool(*b),
        Value::Color(c) => GuestValue::Color(c.clone()),
        Value::Node(id) => GuestValue::Node(id.0 as u64),
        Value::Enum { enum_name, variant } => GuestValue::Enum {
            enum_name: enum_name.clone(),
            variant: variant.clone(),
        },
    }
}

fn guest_to_host_value(val: GuestValue) -> Value {
    match val {
        GuestValue::Number(n) => Value::Number(n),
        GuestValue::String(s) => Value::String(s),
        GuestValue::Bool(b) => Value::Bool(b),
        GuestValue::Color(c) => Value::Color(c),
        GuestValue::Node(id) => Value::Node(NodeId(id as usize)),
        GuestValue::Enum { enum_name, variant } => Value::Enum { enum_name, variant },
    }
}

fn host_to_guest_event(e: &Event) -> directedtype_guest::Event {
    let kind = match e.kind {
        EventKind::PointerDown { button } => directedtype_guest::EventKind::PointerDown {
            button: host_to_guest_mouse_button(button),
        },
        EventKind::PointerUp { button } => directedtype_guest::EventKind::PointerUp {
            button: host_to_guest_mouse_button(button),
        },
        EventKind::PointerMove => directedtype_guest::EventKind::PointerMove,
        EventKind::Click { button } => directedtype_guest::EventKind::Click {
            button: host_to_guest_mouse_button(button),
        },
        EventKind::PointerEnter => directedtype_guest::EventKind::PointerEnter,
        EventKind::PointerLeave => directedtype_guest::EventKind::PointerLeave,
        EventKind::Scroll { delta_x, delta_y } => {
            directedtype_guest::EventKind::Scroll { delta_x, delta_y }
        }
        EventKind::Focus => directedtype_guest::EventKind::Focus,
        EventKind::Blur => directedtype_guest::EventKind::Blur,
        EventKind::Callback => directedtype_guest::EventKind::Callback,
    };

    directedtype_guest::Event {
        kind,
        global_point: directedtype_guest::Point::new(e.global_point.x, e.global_point.y),
        local_point: directedtype_guest::Point::new(e.local_point.x, e.local_point.y),
        modifiers: directedtype_guest::Modifiers {
            shift: e.modifiers.shift,
            ctrl: e.modifiers.ctrl,
            alt: e.modifiers.alt,
            meta: e.modifiers.meta,
        },
        target: e.target.0 as u64,
        current_target: e.current_target.0 as u64,
        payload: e.payload.as_ref().map(host_to_guest_value),
        propagation_continued: e.propagation_continued,
    }
}

fn host_to_guest_mouse_button(b: MouseButton) -> directedtype_guest::MouseButton {
    match b {
        MouseButton::Left => directedtype_guest::MouseButton::Left,
        MouseButton::Right => directedtype_guest::MouseButton::Right,
        MouseButton::Middle => directedtype_guest::MouseButton::Middle,
        MouseButton::Other(o) => directedtype_guest::MouseButton::Other(o),
    }
}
