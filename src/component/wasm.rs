use crate::ast::{ComponentKey, Expr, Literal};
use crate::component::{Component, Context, ContextAction, DispatchError, EventHandlerTarget};
use crate::compiler::expanded::NodeId;
use crate::compiler::module::{normalize_path, FileResolver};
use crate::compiler::value::Value;
use crate::dom::{Dom, DomError as HostDomError, NodeHandle as HostNodeHandle};
use crate::interaction::{Event, EventKind, MouseButton};
use crate::span::Span;
use directedtype_guest::dom::{
    CreateElementWire, CreateTextWire, DomError as GuestDomError, DomLinkInfo, DomNodeInfoWire,
    DomSiblingsWire, HostDomBridge, NodeHandle as GuestNodeHandle, QueryByTagWire,
    SetPortLiteralWire, WireResult,
};
use directedtype_guest::protocol::{
    Action as GuestAction, ComponentResponse, DispatchRequest, MountRequest,
};
use directedtype_guest::Value as GuestValue;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Component as PathComponent, Path, PathBuf};
use wasmi::{Caller, Config, Engine, Instance, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc};

const DEFAULT_MAX_CREATED_NODES: usize = 100_000;
const DEFAULT_MAX_MUTATIONS: usize = 500_000;
const DEFAULT_MAX_ASSET_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_WASM_FUEL: u64 = 10_000_000;
const DEFAULT_WASM_MEMORY_BYTES: usize = 64 * 1024 * 1024;

/// Host wrapper around a compiled WebAssembly guest component instance.
pub struct WasmComponent {
    _engine: Engine,
    store: Store<()>,
    _instance: Instance,
    memory: Option<Memory>,
    alloc_fn: Option<TypedFunc<u32, u32>>,
    dealloc_fn: Option<TypedFunc<(u32, u32), ()>>,
    mount_fn: Option<TypedFunc<(u32, u32), u64>>,
    dispatch_fn: Option<TypedFunc<(u32, u32), u64>>,
}

// wasmi Store is not Send by default unless wrapped or used per thread.
// Here Component requires Send + 'static.
unsafe impl Send for WasmComponent {}

impl WasmComponent {
    /// Loads a WebAssembly module from bytecode and instantiates it for runtime mount/dispatch.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DispatchError> {
        let engine = Engine::default();
        let module = Module::new(&engine, bytes).map_err(|e| {
            DispatchError::Custom(format!("Failed to compile WASM module: {}", e))
        })?;
        let mut linker = Linker::new(&engine);
        define_runtime_dt_dom_stubs(&mut linker)?;
        let mut store = Store::new(&engine, ());
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| {
                DispatchError::Custom(format!("Failed to instantiate WASM component: {}", e))
            })?;

        let memory = instance.get_memory(&store, "memory");
        let alloc_fn = instance.get_typed_func::<u32, u32>(&store, "__dt_alloc").ok();
        let dealloc_fn = instance
            .get_typed_func::<(u32, u32), ()>(&store, "__dt_dealloc")
            .ok();
        let mount_fn = instance
            .get_typed_func::<(u32, u32), u64>(&store, "__dt_mount")
            .ok();
        let dispatch_fn = instance
            .get_typed_func::<(u32, u32), u64>(&store, "__dt_dispatch")
            .ok();

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
        let Some(memory) = self.memory else {
            return Err(DispatchError::Custom(
                "WASM component missing 'memory' export".into(),
            ));
        };
        let Some(alloc_fn) = self.alloc_fn.clone() else {
            return Err(DispatchError::Custom(
                "WASM missing '__dt_alloc' export".into(),
            ));
        };
        let target_fn = if is_mount {
            let Some(ref f) = self.mount_fn else {
                return Ok(ComponentResponse {
                    mutations: Vec::new(),
                    actions: Vec::new(),
                    call_default: false,
                    propagation_continued: false,
                    error: None,
                });
            };
            f.clone()
        } else {
            let Some(ref f) = self.dispatch_fn else {
                return Err(DispatchError::Custom(
                    "WASM missing '__dt_dispatch' export".into(),
                ));
            };
            f.clone()
        };

        let len = json_bytes.len() as u32;
        let ptr = alloc_fn
            .call(&mut self.store, len)
            .map_err(|e| DispatchError::Custom(format!("WASM __dt_alloc failed: {}", e)))?;

        memory
            .write(&mut self.store, ptr as usize, json_bytes)
            .map_err(|e| DispatchError::Custom(format!("WASM memory write failed: {}", e)))?;

        let packed_res = target_fn
            .call(&mut self.store, (ptr, len))
            .map_err(|e| DispatchError::Custom(format!("WASM call failed: {}", e)))?;

        if let Some(ref dealloc_fn) = self.dealloc_fn {
            let _ = dealloc_fn.call(&mut self.store, (ptr, len));
        }

        let res_ptr = (packed_res >> 32) as u32;
        let res_len = (packed_res & 0xFFFF_FFFF) as u32;

        let mut res_bytes = vec![0u8; res_len as usize];
        memory
            .read(&self.store, res_ptr as usize, &mut res_bytes)
            .map_err(|e| DispatchError::Custom(format!("WASM memory read failed: {}", e)))?;

        if let Some(ref dealloc_fn) = self.dealloc_fn {
            let _ = dealloc_fn.call(&mut self.store, (res_ptr, res_len));
        }

        let resp: ComponentResponse = serde_json::from_slice(&res_bytes).map_err(|e| {
            DispatchError::Custom(format!("Failed to parse ComponentResponse from guest: {}", e))
        })?;

        if let Some(err) = resp.error {
            return Err(DispatchError::Custom(err));
        }

        Ok(resp)
    }

    fn extract_guest_callbacks(ctx: &Context<'_>) -> HashMap<String, (u64, String)> {
        let mut callbacks = HashMap::new();
        if let Some(node) = ctx.layout().get_node(ctx.node_id()) {
            for (port_name, binding) in &node.event_handlers {
                if let EventHandlerTarget::Component(target_id) = binding.target {
                    if target_id != ctx.node_id() {
                        callbacks.insert(
                            port_name.clone(),
                            (target_id.0 as u64, binding.method.clone()),
                        );
                    }
                }
            }
        }
        callbacks
    }

    fn apply_response(&self, resp: ComponentResponse, ctx: &mut Context<'_>) {
        if resp.call_default {
            ctx.call_default();
        }

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
            callbacks: Self::extract_guest_callbacks(ctx),
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
            callbacks: Self::extract_guest_callbacks(ctx),
        };

        let json_bytes = serde_json::to_vec(&req).map_err(|e| {
            DispatchError::Custom(format!("Failed to serialize DispatchRequest: {}", e))
        })?;

        let resp = self.call_wasm(false, &json_bytes)?;
        if resp.propagation_continued {
            event.continue_propagation();
        }
        self.apply_response(resp, ctx);
        Ok(())
    }
}

fn define_runtime_dt_dom_stubs(linker: &mut Linker<()>) -> Result<(), DispatchError> {
    let map_err =
        |e: wasmi::errors::LinkerError| DispatchError::Custom(format!("Failed to link dt_dom stub: {e}"));
    macro_rules! stub {
        ($name:literal, ($($param:ty),*), $ret:ty) => {
            linker
                .func_wrap(
                    "dt_dom",
                    $name,
                    |_caller: Caller<'_, ()>, $(_: $param),*| -> Result<$ret, wasmi::Error> {
                        Err(wasmi::Error::new(concat!(
                            "dt_dom host function '",
                            $name,
                            "' cannot be called outside on_prepare"
                        )))
                    },
                )
                .map_err(map_err)?;
        };
    }

    stub!("__dt_dom_roots", (), u64);
    stub!("__dt_dom_children", (u64), u64);
    stub!("__dt_dom_parent", (u64), u64);
    stub!("__dt_dom_siblings", (u64), u64);
    stub!("__dt_dom_node_info", (u64), u64);
    stub!("__dt_dom_get_port_literal", (u64, u32, u32), u64);
    stub!("__dt_dom_get_text_content", (u64, u32), u64);
    stub!("__dt_dom_query_by_tag", (u64, u32, u32), u64);
    stub!("__dt_dom_ancestors", (u64), u64);
    stub!("__dt_dom_document_links", (u64), u64);
    stub!("__dt_dom_create_element", (u32, u32), u64);
    stub!("__dt_dom_create_text", (u32, u32), u64);
    stub!("__dt_dom_parse_fragment", (u32, u32), u64);
    stub!("__dt_dom_append_child", (u64, u64), u64);
    stub!("__dt_dom_insert_before", (u64, u64, u64), u64);
    stub!("__dt_dom_remove_child", (u64, u64), u64);
    stub!("__dt_dom_replace_child", (u64, u64, u64), u64);
    stub!("__dt_dom_set_port_literal", (u64, u32, u32), u64);
    stub!("__dt_dom_set_port_node_ref", (u64, u64, u32, u32), u64);
    stub!("__dt_dom_remove_port", (u64, u32, u32), u64);
    stub!("__dt_dom_set_text", (u64, u32, u32), u64);
    stub!("__dt_host_read_asset", (u32, u32), u64);
    Ok(())
}

// ---------------------------------------------------------------------------
// Pre-Layout Prepare Host State & Bridge (`dt_dom` ABI)
// ---------------------------------------------------------------------------

/// Shared host session state backing `PrepareContext` (native and WASM) during `Dom::prepare`.
pub struct PrepareHostState<'a> {
    pub dom: &'a mut Dom,
    pub base_dir: &'a Path,
    pub resolver: &'a dyn FileResolver,
    pub created_nodes: usize,
    pub mutations: usize,
    pub max_created_nodes: usize,
    pub max_mutations: usize,
    pub max_asset_bytes: usize,
    pub fatal_error: Option<HostDomError>,
}

impl<'a> PrepareHostState<'a> {
    pub fn new(dom: &'a mut Dom, base_dir: &'a Path, resolver: &'a dyn FileResolver) -> Self {
        Self {
            dom,
            base_dir,
            resolver,
            created_nodes: 0,
            mutations: 0,
            max_created_nodes: DEFAULT_MAX_CREATED_NODES,
            max_mutations: DEFAULT_MAX_MUTATIONS,
            max_asset_bytes: DEFAULT_MAX_ASSET_BYTES,
            fatal_error: None,
        }
    }

    fn record_created(&mut self, count: usize) -> Result<(), GuestDomError> {
        self.created_nodes = self.created_nodes.saturating_add(count);
        if self.created_nodes > self.max_created_nodes {
            let msg = format!(
                "Created node count {} exceeded prepare limit of {}",
                self.created_nodes, self.max_created_nodes
            );
            self.fatal_error = Some(HostDomError::PrepareBudgetExceeded(msg.clone()));
            return Err(GuestDomError::BudgetExceeded(msg));
        }
        Ok(())
    }

    fn record_mutation(&mut self) -> Result<(), GuestDomError> {
        self.mutations = self.mutations.saturating_add(1);
        if self.mutations > self.max_mutations {
            let msg = format!(
                "Mutation count {} exceeded prepare limit of {}",
                self.mutations, self.max_mutations
            );
            self.fatal_error = Some(HostDomError::PrepareBudgetExceeded(msg.clone()));
            return Err(GuestDomError::BudgetExceeded(msg));
        }
        Ok(())
    }

    fn to_host_handle(&self, handle: GuestNodeHandle) -> Result<HostNodeHandle, GuestDomError> {
        let Some(h) = HostNodeHandle::from_u64(handle.0) else {
            return Err(GuestDomError::InvalidHandle);
        };
        if !self.dom.is_valid_handle(h) {
            return Err(GuestDomError::InvalidHandle);
        }
        Ok(h)
    }

    fn validate_asset_rel_path(&mut self, rel_path: &str) -> Result<PathBuf, GuestDomError> {
        if rel_path.is_empty() {
            let msg = "Empty asset path".to_string();
            self.fatal_error = Some(HostDomError::AssetAccessDenied(msg.clone()));
            return Err(GuestDomError::AssetAccessDenied(msg));
        }
        let raw = Path::new(rel_path);
        if raw.is_absolute() {
            let msg = format!("Absolute asset path '{}' is not permitted", rel_path);
            self.fatal_error = Some(HostDomError::AssetAccessDenied(msg.clone()));
            return Err(GuestDomError::AssetAccessDenied(msg));
        }
        let mut depth: usize = 0;
        for comp in raw.components() {
            match comp {
                PathComponent::RootDir | PathComponent::Prefix(_) => {
                    let msg = format!("Asset path '{}' must be relative to document directory", rel_path);
                    self.fatal_error = Some(HostDomError::AssetAccessDenied(msg.clone()));
                    return Err(GuestDomError::AssetAccessDenied(msg));
                }
                PathComponent::CurDir => {}
                PathComponent::ParentDir => {
                    if depth == 0 {
                        let msg = format!(
                            "Asset path '{}' attempts to escape document base directory",
                            rel_path
                        );
                        self.fatal_error = Some(HostDomError::AssetAccessDenied(msg.clone()));
                        return Err(GuestDomError::AssetAccessDenied(msg));
                    }
                    depth -= 1;
                }
                PathComponent::Normal(_) => depth += 1,
            }
        }

        let joined = self.base_dir.join(raw);
        let norm_base = normalize_path(self.base_dir);
        let norm_joined = normalize_path(&joined);
        if norm_base != Path::new(".")
            && !norm_base.as_os_str().is_empty()
            && !norm_joined.starts_with(&norm_base)
        {
            let msg = format!("Asset path '{}' escaped base directory", rel_path);
            self.fatal_error = Some(HostDomError::AssetAccessDenied(msg.clone()));
            return Err(GuestDomError::AssetAccessDenied(msg));
        }
        if let (Ok(canon_base), Ok(canon_target)) = (
            self.resolver.canonicalize(self.base_dir),
            self.resolver.canonicalize(&joined),
        ) {
            let outside = if canon_base == Path::new(".") || canon_base.as_os_str().is_empty() {
                canon_target.is_absolute()
                    || matches!(
                        canon_target.components().next(),
                        Some(PathComponent::ParentDir)
                    )
            } else {
                !canon_target.starts_with(&canon_base)
            };
            if outside {
                let msg = format!(
                    "Canonical asset path '{}' resolved outside sandbox base directory",
                    rel_path
                );
                self.fatal_error = Some(HostDomError::AssetAccessDenied(msg.clone()));
                return Err(GuestDomError::AssetAccessDenied(msg));
            }
        }

        Ok(joined)
    }

    fn collect_document_links_rec(
        &self,
        handle: HostNodeHandle,
        out: &mut Vec<DomLinkInfo>,
    ) -> Result<(), GuestDomError> {
        let tag = self.dom.tag_name(handle).map_err(host_to_guest_err)?.to_string();
        if matches!(tag.as_str(), "Link" | "Anchor" | "AnchorScope") {
            let ports = self.dom.declared_ports(handle).map_err(host_to_guest_err)?;
            let url_or_name = match tag.as_str() {
                "Anchor" | "AnchorScope" => ports.get("name").and_then(|e| match e {
                    Expr::Literal(Literal::String(s, _)) => Some(s.clone()),
                    _ => None,
                }),
                _ => ports.get("url").and_then(|e| match e {
                    Expr::Literal(Literal::String(s, _)) => Some(s.clone()),
                    _ => None,
                }),
            };
            let target_handle = ports.get("to").and_then(|e| match e {
                Expr::DomNodeRef(h, _) => Some(GuestNodeHandle(h.to_u64())),
                _ => None,
            });
            out.push(DomLinkInfo {
                handle: GuestNodeHandle(handle.to_u64()),
                tag,
                url_or_name,
                target_handle,
            });
        }
        let children = self.dom.children(handle).map_err(host_to_guest_err)?.to_vec();
        for ch in children {
            self.collect_document_links_rec(ch, out)?;
        }
        Ok(())
    }

    fn subtree_node_count(&self, root: HostNodeHandle) -> usize {
        let mut count = 1usize;
        if let Ok(children) = self.dom.children(root) {
            let copy = children.to_vec();
            for ch in copy {
                count = count.saturating_add(self.subtree_node_count(ch));
            }
        }
        count
    }
}

fn host_to_guest_err(err: HostDomError) -> GuestDomError {
    match err {
        HostDomError::InvalidHandle(_) => GuestDomError::InvalidHandle,
        HostDomError::HierarchyCycle(_) => GuestDomError::HierarchyCycle,
        HostDomError::NoSuchChild { .. } | HostDomError::NotARoot(_) => GuestDomError::NotAChild,
        HostDomError::Parse(e) => GuestDomError::ParseError(e.to_string()),
        HostDomError::AssetAccessDenied(s) => GuestDomError::AssetAccessDenied(s),
        HostDomError::PrepareBudgetExceeded(s) => GuestDomError::BudgetExceeded(s),
        other => GuestDomError::Custom(other.to_string()),
    }
}

fn guest_value_to_literal_expr(val: &GuestValue) -> Result<Expr, GuestDomError> {
    let sp = Span::default();
    let lit = match val {
        GuestValue::Integer(i) => Literal::Integer(*i, sp),
        GuestValue::Float(f) => Literal::Float(*f, sp),
        GuestValue::Distance(d) => Literal::Distance(*d, crate::ast::DistanceUnit::Px, sp),
        GuestValue::Duration(d) => Literal::Duration(*d, crate::ast::DurationUnit::Ms, sp),
        GuestValue::String(s) => Literal::String(s.clone(), sp),
        GuestValue::Bool(b) => Literal::Bool(*b, sp),
        GuestValue::Color(c) => Literal::Color(c.clone(), sp),
        GuestValue::Enum { enum_name, variant } => {
            Literal::Enum(enum_name.clone(), variant.clone(), sp)
        }
        GuestValue::Unset => {
            return Ok(Expr::Ident(crate::ast::Ident::new("unset", sp)));
        }
        GuestValue::Node(_) | GuestValue::Timer { .. } | GuestValue::IndefiniteTimer { .. } => {
            return Err(GuestDomError::Custom(
                "Only literal values are allowed in set_port_literal / create_element".into(),
            ));
        }
    };
    Ok(Expr::Literal(lit))
}

fn literal_to_guest_value(lit: &Literal) -> GuestValue {
    match lit {
        Literal::Integer(i, _) => GuestValue::Integer(*i),
        Literal::Float(f, _) => GuestValue::Float(*f),
        Literal::Distance(d, unit, _) | Literal::Area(d, unit, _) => {
            let pts = unit.points_per_unit().unwrap_or(1.0);
            GuestValue::Distance(*d * pts)
        }
        Literal::Duration(d, _, _) => GuestValue::Duration(*d),
        Literal::String(s, _) => GuestValue::String(s.clone()),
        Literal::Bool(b, _) => GuestValue::Bool(*b),
        Literal::Color(c, _) => GuestValue::Color(c.clone()),
        Literal::Enum(enum_name, variant, _) => GuestValue::Enum {
            enum_name: enum_name.clone(),
            variant: variant.clone(),
        },
    }
}

impl<'a> HostDomBridge for PrepareHostState<'a> {
    fn roots(&mut self) -> Result<Vec<GuestNodeHandle>, GuestDomError> {
        Ok(self
            .dom
            .roots()
            .iter()
            .map(|h| GuestNodeHandle(h.to_u64()))
            .collect())
    }

    fn children(&mut self, node: GuestNodeHandle) -> Result<Vec<GuestNodeHandle>, GuestDomError> {
        let h = self.to_host_handle(node)?;
        Ok(self
            .dom
            .children(h)
            .map_err(host_to_guest_err)?
            .iter()
            .map(|c| GuestNodeHandle(c.to_u64()))
            .collect())
    }

    fn parent(&mut self, node: GuestNodeHandle) -> Result<Option<GuestNodeHandle>, GuestDomError> {
        let h = self.to_host_handle(node)?;
        Ok(self
            .dom
            .parent(h)
            .map_err(host_to_guest_err)?
            .map(|p| GuestNodeHandle(p.to_u64())))
    }

    fn prev_sibling(
        &mut self,
        node: GuestNodeHandle,
    ) -> Result<Option<GuestNodeHandle>, GuestDomError> {
        let h = self.to_host_handle(node)?;
        Ok(self
            .dom
            .prev_sibling(h)
            .map_err(host_to_guest_err)?
            .map(|p| GuestNodeHandle(p.to_u64())))
    }

    fn next_sibling(
        &mut self,
        node: GuestNodeHandle,
    ) -> Result<Option<GuestNodeHandle>, GuestDomError> {
        let h = self.to_host_handle(node)?;
        Ok(self
            .dom
            .next_sibling(h)
            .map_err(host_to_guest_err)?
            .map(|n| GuestNodeHandle(n.to_u64())))
    }

    fn tag_name(&mut self, node: GuestNodeHandle) -> Result<String, GuestDomError> {
        let h = self.to_host_handle(node)?;
        Ok(self.dom.tag_name(h).map_err(host_to_guest_err)?.to_string())
    }

    fn node_key(&mut self, node: GuestNodeHandle) -> Result<Option<Vec<GuestValue>>, GuestDomError> {
        let h = self.to_host_handle(node)?;
        let Some(key) = self.dom.node_key(h) else {
            return Ok(None);
        };
        let mut vals = Vec::with_capacity(key.parts.len());
        for part in &key.parts {
            match part {
                Expr::Literal(lit) => vals.push(literal_to_guest_value(lit)),
                _ => return Ok(None),
            }
        }
        Ok(Some(vals))
    }

    fn get_port_literal(
        &mut self,
        node: GuestNodeHandle,
        port: &str,
    ) -> Result<Option<GuestValue>, GuestDomError> {
        let h = self.to_host_handle(node)?;
        let ports = self.dom.declared_ports(h).map_err(host_to_guest_err)?;
        Ok(match ports.get(port) {
            Some(Expr::Literal(lit)) => Some(literal_to_guest_value(lit)),
            _ => None,
        })
    }

    fn get_text_content(&mut self, node: GuestNodeHandle) -> Result<Option<String>, GuestDomError> {
        let h = self.to_host_handle(node)?;
        Ok(self
            .dom
            .text_content(h)
            .map_err(host_to_guest_err)?
            .map(|s| s.to_string()))
    }

    fn get_subtree_text(&mut self, node: GuestNodeHandle) -> Result<String, GuestDomError> {
        let h = self.to_host_handle(node)?;
        self.dom.get_subtree_text(h).map_err(host_to_guest_err)
    }

    fn query_by_tag(
        &mut self,
        scope: Option<GuestNodeHandle>,
        tags: &[&str],
    ) -> Result<Vec<GuestNodeHandle>, GuestDomError> {
        let scope_h = match scope {
            Some(s) if s.0 != 0 => Some(self.to_host_handle(s)?),
            _ => None,
        };
        Ok(self
            .dom
            .query_by_tag(scope_h, tags)
            .map_err(host_to_guest_err)?
            .into_iter()
            .map(|h| GuestNodeHandle(h.to_u64()))
            .collect())
    }

    fn ancestors(&mut self, node: GuestNodeHandle) -> Result<Vec<GuestNodeHandle>, GuestDomError> {
        let h = self.to_host_handle(node)?;
        Ok(self
            .dom
            .ancestors(h)
            .map_err(host_to_guest_err)?
            .into_iter()
            .map(|h| GuestNodeHandle(h.to_u64()))
            .collect())
    }

    fn document_links(
        &mut self,
        scope: Option<GuestNodeHandle>,
    ) -> Result<Vec<DomLinkInfo>, GuestDomError> {
        let mut out = Vec::new();
        match scope {
            Some(s) if s.0 != 0 => {
                let h = self.to_host_handle(s)?;
                let children = self.dom.children(h).map_err(host_to_guest_err)?.to_vec();
                for ch in children {
                    self.collect_document_links_rec(ch, &mut out)?;
                }
            }
            _ => {
                let roots = self.dom.roots().to_vec();
                for r in roots {
                    self.collect_document_links_rec(r, &mut out)?;
                }
            }
        }
        Ok(out)
    }

    fn read_asset(&mut self, rel_path: &str) -> Result<Vec<u8>, GuestDomError> {
        let target = self.validate_asset_rel_path(rel_path)?;
        let bytes = self
            .resolver
            .read_bytes(&target)
            .or_else(|_| self.resolver.read(&target).map(|s| s.into_bytes()))
            .or_else(|_| {
                let raw = Path::new(rel_path);
                self.resolver
                    .read_bytes(raw)
                    .or_else(|_| self.resolver.read(raw).map(|s| s.into_bytes()))
            })
            .map_err(|_| GuestDomError::AssetNotFound(rel_path.to_string()))?;

        if bytes.len() > self.max_asset_bytes {
            let msg = format!(
                "Asset '{}' ({} bytes) exceeds {} byte limit",
                rel_path,
                bytes.len(),
                self.max_asset_bytes
            );
            self.fatal_error = Some(HostDomError::PrepareBudgetExceeded(msg.clone()));
            return Err(GuestDomError::BudgetExceeded(msg));
        }
        Ok(bytes)
    }

    fn create_element(
        &mut self,
        tag: &str,
        key: Option<&[GuestValue]>,
        ports: &[(&str, GuestValue)],
    ) -> Result<GuestNodeHandle, GuestDomError> {
        self.record_created(1)?;
        self.record_mutation()?;
        let comp_key = if let Some(parts) = key {
            let mut expr_parts = Vec::with_capacity(parts.len());
            for p in parts {
                expr_parts.push(guest_value_to_literal_expr(p)?);
            }
            Some(ComponentKey::new(expr_parts, Span::default()))
        } else {
            None
        };
        let mut expr_ports = Vec::with_capacity(ports.len());
        for (k, v) in ports {
            expr_ports.push(((*k).to_string(), guest_value_to_literal_expr(v)?));
        }
        let h = self.dom.create_element_with_key(tag, comp_key, expr_ports);
        Ok(GuestNodeHandle(h.to_u64()))
    }

    fn create_text(
        &mut self,
        text: &str,
        ports: &[(&str, GuestValue)],
    ) -> Result<GuestNodeHandle, GuestDomError> {
        self.record_created(1)?;
        self.record_mutation()?;
        let mut expr_ports = Vec::with_capacity(ports.len());
        for (k, v) in ports {
            expr_ports.push(((*k).to_string(), guest_value_to_literal_expr(v)?));
        }
        let h = self.dom.create_text(text, expr_ports);
        Ok(GuestNodeHandle(h.to_u64()))
    }

    fn parse_fragment(&mut self, dt_snippet: &str) -> Result<GuestNodeHandle, GuestDomError> {
        let h = self.dom.parse_fragment(dt_snippet).map_err(host_to_guest_err)?;
        let count = self.subtree_node_count(h);
        self.record_created(count)?;
        self.record_mutation()?;
        Ok(GuestNodeHandle(h.to_u64()))
    }

    fn append_child(
        &mut self,
        parent: GuestNodeHandle,
        child: GuestNodeHandle,
    ) -> Result<(), GuestDomError> {
        self.record_mutation()?;
        let p = self.to_host_handle(parent)?;
        let c = self.to_host_handle(child)?;
        self.dom.append_child(p, c).map_err(host_to_guest_err)
    }

    fn insert_before(
        &mut self,
        parent: GuestNodeHandle,
        before: GuestNodeHandle,
        child: GuestNodeHandle,
    ) -> Result<(), GuestDomError> {
        self.record_mutation()?;
        let p = self.to_host_handle(parent)?;
        let b = self.to_host_handle(before)?;
        let c = self.to_host_handle(child)?;
        self.dom.insert_before(p, b, c).map_err(host_to_guest_err)
    }

    fn remove_child(
        &mut self,
        parent: GuestNodeHandle,
        child: GuestNodeHandle,
    ) -> Result<(), GuestDomError> {
        self.record_mutation()?;
        let p = self.to_host_handle(parent)?;
        let c = self.to_host_handle(child)?;
        self.dom.remove_child(p, c).map_err(host_to_guest_err)
    }

    fn replace_child(
        &mut self,
        parent: GuestNodeHandle,
        old_child: GuestNodeHandle,
        new_child: GuestNodeHandle,
    ) -> Result<(), GuestDomError> {
        self.record_mutation()?;
        let p = self.to_host_handle(parent)?;
        let old_c = self.to_host_handle(old_child)?;
        let new_c = self.to_host_handle(new_child)?;
        self.dom
            .replace_child(p, old_c, new_c)
            .map_err(host_to_guest_err)
    }

    fn set_port_literal(
        &mut self,
        node: GuestNodeHandle,
        port: &str,
        value: GuestValue,
    ) -> Result<(), GuestDomError> {
        self.record_mutation()?;
        let h = self.to_host_handle(node)?;
        let expr = guest_value_to_literal_expr(&value)?;
        self.dom.set_port(h, port, expr).map_err(host_to_guest_err)
    }

    fn set_port_node_ref(
        &mut self,
        node: GuestNodeHandle,
        port: &str,
        target: GuestNodeHandle,
    ) -> Result<(), GuestDomError> {
        self.record_mutation()?;
        let h = self.to_host_handle(node)?;
        let t = self.to_host_handle(target)?;
        self.dom
            .set_port(h, port, Expr::DomNodeRef(t, Span::default()))
            .map_err(host_to_guest_err)
    }

    fn remove_port(&mut self, node: GuestNodeHandle, port: &str) -> Result<(), GuestDomError> {
        self.record_mutation()?;
        let h = self.to_host_handle(node)?;
        let _ = self.dom.remove_port(h, port).map_err(host_to_guest_err)?;
        Ok(())
    }

    fn set_text(&mut self, node: GuestNodeHandle, text: &str) -> Result<(), GuestDomError> {
        self.record_mutation()?;
        let h = self.to_host_handle(node)?;
        self.dom.set_text(h, text).map_err(host_to_guest_err)
    }

    fn wrap_node(
        &mut self,
        node: GuestNodeHandle,
        wrapper: GuestNodeHandle,
    ) -> Result<(), GuestDomError> {
        self.record_mutation()?;
        let n = self.to_host_handle(node)?;
        let w = self.to_host_handle(wrapper)?;
        self.dom.wrap_node(n, w).map_err(host_to_guest_err)
    }
}

struct PrepareStoreData {
    host_ptr: std::ptr::NonNull<PrepareHostState<'static>>,
    limits: StoreLimits,
}

impl PrepareStoreData {
    #[inline]
    unsafe fn host_mut(&mut self) -> &mut PrepareHostState<'_> {
        &mut *self.host_ptr.as_ptr()
    }
}

/// Returns exported WASM prepare hook names present in `bytes` (accepts binary `.wasm` or `.wat`).
pub fn wasm_has_prepare(bytes: &[u8]) -> Result<bool, HostDomError> {
    let engine = Engine::default();
    let module = Module::new(&engine, bytes)
        .map_err(|e| HostDomError::PrepareTrap(format!("Failed to compile WASM module: {e}")))?;
    let has_generic = module
        .exports()
        .any(|e| e.name() == "__dt_prepare" || e.name().starts_with("__dt_prepare_"));
    if !has_generic {
        return Ok(false);
    }
    if module.exports().any(|e| e.name() == "__dt_has_prepare") {
        let mut linker = Linker::new(&engine);
        define_runtime_dt_dom_stubs(&mut linker)
            .map_err(|e| HostDomError::PrepareTrap(e.to_string()))?;
        let mut store = Store::new(&engine, ());
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| HostDomError::PrepareTrap(e.to_string()))?;
        if let Ok(f) = instance.get_typed_func::<(), u32>(&store, "__dt_has_prepare") {
            let v = f
                .call(&mut store, ())
                .map_err(|e| HostDomError::PrepareTrap(e.to_string()))?;
            return Ok(v != 0);
        }
    }
    Ok(true)
}

/// Lists `__dt_prepare*` exports declared by a compiled WASM module/bundle.
pub fn wasm_prepare_exports(bytes: &[u8]) -> Result<HashSet<String>, HostDomError> {
    let engine = Engine::default();
    let module = Module::new(&engine, bytes)
        .map_err(|e| HostDomError::PrepareTrap(format!("Failed to compile WASM module: {e}")))?;
    Ok(module
        .exports()
        .filter(|e| e.name() == "__dt_prepare" || e.name().starts_with("__dt_prepare_"))
        .map(|e| e.name().to_string())
        .collect())
}

fn read_guest_bytes(
    caller: &mut Caller<'_, PrepareStoreData>,
    ptr: u32,
    len: u32,
) -> Result<Vec<u8>, wasmi::Error> {
    let memory = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| wasmi::Error::new("WASM guest missing exported 'memory'"))?;
    let mut buf = vec![0u8; len as usize];
    memory
        .read(&*caller, ptr as usize, &mut buf)
        .map_err(|e| wasmi::Error::new(format!("WASM memory read out of bounds: {e}")))?;
    Ok(buf)
}

fn write_wire_result<T: Serialize>(
    caller: &mut Caller<'_, PrepareStoreData>,
    res: Result<T, GuestDomError>,
) -> Result<u64, wasmi::Error> {
    let wire = match res {
        Ok(v) => WireResult::Ok(v),
        Err(e) => WireResult::Err(e),
    };
    let alloc = caller
        .get_export("__dt_alloc")
        .and_then(|e| e.into_func())
        .and_then(|f| f.typed::<u32, u32>(&*caller).ok());
    let Some(alloc_fn) = alloc else {
        if let WireResult::Err(err) = wire {
            return Err(wasmi::Error::new(err.to_string()));
        }
        return Ok(0);
    };
    let json = serde_json::to_vec(&wire)
        .map_err(|e| wasmi::Error::new(format!("Failed to serialize dt_dom response: {e}")))?;
    let memory = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| wasmi::Error::new("WASM guest missing exported 'memory'"))?;
    let len = json.len() as u32;
    let out_ptr = alloc_fn.call(&mut *caller, len)?;
    memory
        .write(&mut *caller, out_ptr as usize, &json)
        .map_err(|e| wasmi::Error::new(format!("WASM memory write failed: {e}")))?;
    Ok(((out_ptr as u64) << 32) | (len as u64))
}

fn bind_prepare_dt_dom_imports(linker: &mut Linker<PrepareStoreData>) -> Result<(), HostDomError> {
    let map_err =
        |e: wasmi::errors::LinkerError| HostDomError::PrepareTrap(format!("Failed to bind dt_dom: {e}"));

    linker
        .func_wrap("__dt_dummy_ignore", "__ignore", |_c: Caller<'_, PrepareStoreData>| {})
        .ok();

    linker
        .func_wrap(
            "dt_dom",
            "__dt_dom_roots",
            |mut caller: Caller<'_, PrepareStoreData>| -> Result<u64, wasmi::Error> {
                let res = unsafe { caller.data_mut().host_mut().roots() };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    linker
        .func_wrap(
            "dt_dom",
            "__dt_dom_children",
            |mut caller: Caller<'_, PrepareStoreData>, node: u64| -> Result<u64, wasmi::Error> {
                let res = unsafe { caller.data_mut().host_mut().children(GuestNodeHandle(node)) };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    linker
        .func_wrap(
            "dt_dom",
            "__dt_dom_parent",
            |mut caller: Caller<'_, PrepareStoreData>, node: u64| -> Result<u64, wasmi::Error> {
                let res = unsafe { caller.data_mut().host_mut().parent(GuestNodeHandle(node)) };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    linker
        .func_wrap(
            "dt_dom",
            "__dt_dom_siblings",
            |mut caller: Caller<'_, PrepareStoreData>, node: u64| -> Result<u64, wasmi::Error> {
                let res = unsafe {
                    let h = caller.data_mut().host_mut();
                    let prev = h.prev_sibling(GuestNodeHandle(node));
                    let next = h.next_sibling(GuestNodeHandle(node));
                    match (prev, next) {
                        (Ok(p), Ok(n)) => Ok(DomSiblingsWire { prev: p, next: n }),
                        (Err(e), _) | (_, Err(e)) => Err(e),
                    }
                };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    linker
        .func_wrap(
            "dt_dom",
            "__dt_dom_node_info",
            |mut caller: Caller<'_, PrepareStoreData>, node: u64| -> Result<u64, wasmi::Error> {
                let res = unsafe {
                    let h = caller.data_mut().host_mut();
                    let gh = GuestNodeHandle(node);
                    match (h.tag_name(gh), h.node_key(gh), h.get_text_content(gh)) {
                        (Ok(tag), Ok(key), Ok(tc)) => Ok(DomNodeInfoWire {
                            tag,
                            key,
                            has_text: tc.is_some(),
                        }),
                        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => Err(e),
                    }
                };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    linker
        .func_wrap(
            "dt_dom",
            "__dt_dom_get_port_literal",
            |mut caller: Caller<'_, PrepareStoreData>,
             node: u64,
             ptr: u32,
             len: u32|
             -> Result<u64, wasmi::Error> {
                let bytes = read_guest_bytes(&mut caller, ptr, len)?;
                let res = match std::str::from_utf8(&bytes) {
                    Ok(port) => unsafe {
                        caller
                            .data_mut()
                            .host_mut()
                            .get_port_literal(GuestNodeHandle(node), port)
                    },
                    Err(e) => Err(GuestDomError::Custom(format!("Invalid UTF-8 port name: {e}"))),
                };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    linker
        .func_wrap(
            "dt_dom",
            "__dt_dom_get_text_content",
            |mut caller: Caller<'_, PrepareStoreData>,
             node: u64,
             recursive: u32|
             -> Result<u64, wasmi::Error> {
                let res = unsafe {
                    let h = caller.data_mut().host_mut();
                    if recursive != 0 {
                        h.get_subtree_text(GuestNodeHandle(node)).map(Some)
                    } else {
                        h.get_text_content(GuestNodeHandle(node))
                    }
                };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    linker
        .func_wrap(
            "dt_dom",
            "__dt_dom_query_by_tag",
            |mut caller: Caller<'_, PrepareStoreData>,
             scope: u64,
             ptr: u32,
             len: u32|
             -> Result<u64, wasmi::Error> {
                let bytes = read_guest_bytes(&mut caller, ptr, len)?;
                let res = match serde_json::from_slice::<QueryByTagWire>(&bytes) {
                    Ok(q) => {
                        let refs: Vec<&str> = q.tags.iter().map(|s| s.as_str()).collect();
                        let scope_opt = (scope != 0).then_some(GuestNodeHandle(scope));
                        unsafe { caller.data_mut().host_mut().query_by_tag(scope_opt, &refs) }
                    }
                    Err(e) => Err(GuestDomError::Custom(format!("Invalid QueryByTagWire: {e}"))),
                };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    linker
        .func_wrap(
            "dt_dom",
            "__dt_dom_ancestors",
            |mut caller: Caller<'_, PrepareStoreData>, node: u64| -> Result<u64, wasmi::Error> {
                let res = unsafe {
                    caller
                        .data_mut()
                        .host_mut()
                        .ancestors(GuestNodeHandle(node))
                };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    linker
        .func_wrap(
            "dt_dom",
            "__dt_dom_document_links",
            |mut caller: Caller<'_, PrepareStoreData>, scope: u64| -> Result<u64, wasmi::Error> {
                let scope_opt = (scope != 0).then_some(GuestNodeHandle(scope));
                let res = unsafe { caller.data_mut().host_mut().document_links(scope_opt) };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    linker
        .func_wrap(
            "dt_dom",
            "__dt_dom_create_element",
            |mut caller: Caller<'_, PrepareStoreData>,
             ptr: u32,
             len: u32|
             -> Result<u64, wasmi::Error> {
                let bytes = read_guest_bytes(&mut caller, ptr, len)?;
                let res = match serde_json::from_slice::<CreateElementWire>(&bytes) {
                    Ok(ce) => {
                        let ports: Vec<(&str, GuestValue)> =
                            ce.ports.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
                        unsafe {
                            caller.data_mut().host_mut().create_element(
                                &ce.tag,
                                ce.key.as_deref(),
                                &ports,
                            )
                        }
                    }
                    Err(e) => Err(GuestDomError::Custom(format!("Invalid CreateElementWire: {e}"))),
                };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    linker
        .func_wrap(
            "dt_dom",
            "__dt_dom_create_text",
            |mut caller: Caller<'_, PrepareStoreData>,
             ptr: u32,
             len: u32|
             -> Result<u64, wasmi::Error> {
                let bytes = read_guest_bytes(&mut caller, ptr, len)?;
                let res = match serde_json::from_slice::<CreateTextWire>(&bytes) {
                    Ok(ct) => {
                        let ports: Vec<(&str, GuestValue)> =
                            ct.ports.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
                        unsafe { caller.data_mut().host_mut().create_text(&ct.text, &ports) }
                    }
                    Err(e) => Err(GuestDomError::Custom(format!("Invalid CreateTextWire: {e}"))),
                };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    linker
        .func_wrap(
            "dt_dom",
            "__dt_dom_parse_fragment",
            |mut caller: Caller<'_, PrepareStoreData>,
             ptr: u32,
             len: u32|
             -> Result<u64, wasmi::Error> {
                let bytes = read_guest_bytes(&mut caller, ptr, len)?;
                let res = match std::str::from_utf8(&bytes) {
                    Ok(src) => unsafe { caller.data_mut().host_mut().parse_fragment(src) },
                    Err(e) => Err(GuestDomError::Custom(format!("Invalid UTF-8 DTML snippet: {e}"))),
                };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    linker
        .func_wrap(
            "dt_dom",
            "__dt_dom_append_child",
            |mut caller: Caller<'_, PrepareStoreData>,
             parent: u64,
             child: u64|
             -> Result<u64, wasmi::Error> {
                let res = unsafe {
                    caller
                        .data_mut()
                        .host_mut()
                        .append_child(GuestNodeHandle(parent), GuestNodeHandle(child))
                };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    linker
        .func_wrap(
            "dt_dom",
            "__dt_dom_insert_before",
            |mut caller: Caller<'_, PrepareStoreData>,
             parent: u64,
             before: u64,
             child: u64|
             -> Result<u64, wasmi::Error> {
                let res = unsafe {
                    caller.data_mut().host_mut().insert_before(
                        GuestNodeHandle(parent),
                        GuestNodeHandle(before),
                        GuestNodeHandle(child),
                    )
                };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    linker
        .func_wrap(
            "dt_dom",
            "__dt_dom_remove_child",
            |mut caller: Caller<'_, PrepareStoreData>,
             parent: u64,
             child: u64|
             -> Result<u64, wasmi::Error> {
                let res = unsafe {
                    caller
                        .data_mut()
                        .host_mut()
                        .remove_child(GuestNodeHandle(parent), GuestNodeHandle(child))
                };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    linker
        .func_wrap(
            "dt_dom",
            "__dt_dom_replace_child",
            |mut caller: Caller<'_, PrepareStoreData>,
             parent: u64,
             old_child: u64,
             new_child: u64|
             -> Result<u64, wasmi::Error> {
                let res = unsafe {
                    caller.data_mut().host_mut().replace_child(
                        GuestNodeHandle(parent),
                        GuestNodeHandle(old_child),
                        GuestNodeHandle(new_child),
                    )
                };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    linker
        .func_wrap(
            "dt_dom",
            "__dt_dom_set_port_literal",
            |mut caller: Caller<'_, PrepareStoreData>,
             node: u64,
             ptr: u32,
             len: u32|
             -> Result<u64, wasmi::Error> {
                let bytes = read_guest_bytes(&mut caller, ptr, len)?;
                let res = match serde_json::from_slice::<SetPortLiteralWire>(&bytes) {
                    Ok(sp) => unsafe {
                        caller.data_mut().host_mut().set_port_literal(
                            GuestNodeHandle(node),
                            &sp.port,
                            sp.value,
                        )
                    },
                    Err(e) => Err(GuestDomError::Custom(format!("Invalid SetPortLiteralWire: {e}"))),
                };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    linker
        .func_wrap(
            "dt_dom",
            "__dt_dom_set_port_node_ref",
            |mut caller: Caller<'_, PrepareStoreData>,
             node: u64,
             target: u64,
             ptr: u32,
             len: u32|
             -> Result<u64, wasmi::Error> {
                let bytes = read_guest_bytes(&mut caller, ptr, len)?;
                let res = match std::str::from_utf8(&bytes) {
                    Ok(port) => unsafe {
                        caller.data_mut().host_mut().set_port_node_ref(
                            GuestNodeHandle(node),
                            port,
                            GuestNodeHandle(target),
                        )
                    },
                    Err(e) => Err(GuestDomError::Custom(format!("Invalid UTF-8 port name: {e}"))),
                };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    linker
        .func_wrap(
            "dt_dom",
            "__dt_dom_remove_port",
            |mut caller: Caller<'_, PrepareStoreData>,
             node: u64,
             ptr: u32,
             len: u32|
             -> Result<u64, wasmi::Error> {
                let bytes = read_guest_bytes(&mut caller, ptr, len)?;
                let res = match std::str::from_utf8(&bytes) {
                    Ok(port) => unsafe {
                        caller
                            .data_mut()
                            .host_mut()
                            .remove_port(GuestNodeHandle(node), port)
                    },
                    Err(e) => Err(GuestDomError::Custom(format!("Invalid UTF-8 port name: {e}"))),
                };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    linker
        .func_wrap(
            "dt_dom",
            "__dt_dom_set_text",
            |mut caller: Caller<'_, PrepareStoreData>,
             node: u64,
             ptr: u32,
             len: u32|
             -> Result<u64, wasmi::Error> {
                let bytes = read_guest_bytes(&mut caller, ptr, len)?;
                let res = match std::str::from_utf8(&bytes) {
                    Ok(text) => unsafe {
                        caller
                            .data_mut()
                            .host_mut()
                            .set_text(GuestNodeHandle(node), text)
                    },
                    Err(e) => Err(GuestDomError::Custom(format!("Invalid UTF-8 text: {e}"))),
                };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    linker
        .func_wrap(
            "dt_dom",
            "__dt_host_read_asset",
            |mut caller: Caller<'_, PrepareStoreData>,
             ptr: u32,
             len: u32|
             -> Result<u64, wasmi::Error> {
                let bytes = read_guest_bytes(&mut caller, ptr, len)?;
                let res = match std::str::from_utf8(&bytes) {
                    Ok(rel_path) => unsafe { caller.data_mut().host_mut().read_asset(rel_path) },
                    Err(e) => Err(GuestDomError::AssetAccessDenied(format!(
                        "Invalid UTF-8 asset path: {e}"
                    ))),
                };
                write_wire_result(&mut caller, res)
            },
        )
        .map_err(map_err)?;

    Ok(())
}

/// Runs a sandboxed WASM pre-layout transform (`__dt_prepare` or `__dt_prepare_<Tag>`) against `host_state`.
pub fn run_wasm_prepare(
    wasm_bytes: &[u8],
    export_name: &str,
    component_tag: &str,
    self_handle: HostNodeHandle,
    host_state: &mut PrepareHostState<'_>,
) -> Result<(), HostDomError> {
    let mut cfg = Config::default();
    cfg.consume_fuel(true);
    let engine = Engine::new(&cfg);
    let module = Module::new(&engine, wasm_bytes)
        .map_err(|e| HostDomError::PrepareTrap(format!("Failed to compile WASM module: {e}")))?;

    let mut linker = Linker::new(&engine);
    bind_prepare_dt_dom_imports(&mut linker)?;

    let raw_ptr = unsafe {
        std::mem::transmute::<*mut PrepareHostState<'_>, *mut PrepareHostState<'static>>(
            host_state as *mut PrepareHostState<'_>,
        )
    };
    let limits = StoreLimitsBuilder::new()
        .memory_size(DEFAULT_WASM_MEMORY_BYTES)
        .build();
    let store_data = PrepareStoreData {
        host_ptr: std::ptr::NonNull::new(raw_ptr).expect("non-null host_state"),
        limits,
    };
    let mut store = Store::new(&engine, store_data);
    store.limiter(|data| &mut data.limits);
    store
        .set_fuel(DEFAULT_WASM_FUEL)
        .map_err(|e| HostDomError::PrepareTrap(format!("Failed to configure WASM fuel: {e}")))?;

    let instance = linker
        .instantiate_and_start(&mut store, &module)
        .map_err(|e| {
            if let Some(fatal) = host_state.fatal_error.take() {
                fatal
            } else {
                HostDomError::PrepareTrap(format!("WASM instantiation trapped: {e}"))
            }
        })?;

    let prepare_fn = instance
        .get_typed_func::<u64, u64>(&store, export_name)
        .map_err(|e| {
            HostDomError::PrepareTrap(format!("WASM missing export '{export_name}': {e}"))
        })?;

    let call_res = prepare_fn.call(&mut store, self_handle.to_u64());
    if let Some(fatal) = host_state.fatal_error.take() {
        return Err(fatal);
    }

    let packed = match call_res {
        Ok(p) => p,
        Err(trap) => {
            return Err(HostDomError::PrepareTrap(format!(
                "WASM component '{component_tag}' trapped during {export_name}: {trap}"
            )));
        }
    };

    if packed != 0 {
        let ptr = (packed >> 32) as u32;
        let len = (packed & 0xFFFF_FFFF) as u32;
        let message = if let Some(memory) = instance.get_memory(&store, "memory") {
            let mut buf = vec![0u8; len as usize];
            if memory.read(&store, ptr as usize, &mut buf).is_ok() {
                if let Ok(dealloc) = instance.get_typed_func::<(u32, u32), ()>(&store, "__dt_dealloc") {
                    let _ = dealloc.call(&mut store, (ptr, len));
                }
                String::from_utf8_lossy(&buf).into_owned()
            } else {
                "WASM on_prepare returned unreadable error pointer".to_string()
            }
        } else {
            "WASM on_prepare returned non-zero error status".to_string()
        };
        return Err(HostDomError::PrepareError {
            component: component_tag.to_string(),
            node: self_handle,
            message,
        });
    }

    Ok(())
}

fn host_to_guest_value(val: &Value) -> GuestValue {
    match val {
        Value::Integer(i) => GuestValue::Integer(*i),
        Value::Float(n) => GuestValue::Float(*n),
        Value::Distance(d) | Value::Area(d) => GuestValue::Distance(*d),
        Value::Duration(d) => GuestValue::Duration(*d),
        Value::String(s) => GuestValue::String(s.clone()),
        Value::Bool(b) => GuestValue::Bool(*b),
        Value::Color(c) => GuestValue::Color(c.clone()),
        Value::Node(id) => GuestValue::Node(id.0 as u64),
        Value::Enum { enum_name, variant } => GuestValue::Enum {
            enum_name: enum_name.clone(),
            variant: variant.clone(),
        },
        Value::Timer {
            time,
            duration,
            dt,
            frames,
        } => GuestValue::Timer {
            time: *time,
            duration: *duration,
            dt: *dt,
            frames: *frames,
        },
        Value::IndefiniteTimer { time, dt, frames } => GuestValue::IndefiniteTimer {
            time: *time,
            dt: *dt,
            frames: *frames,
        },
        Value::Unset => GuestValue::Unset,
    }
}

fn guest_to_host_value(val: GuestValue) -> Value {
    match val {
        GuestValue::Integer(i) => Value::Integer(i),
        GuestValue::Float(n) => Value::Float(n),
        GuestValue::Distance(d) => Value::Distance(d),
        GuestValue::Duration(d) => Value::Duration(d),
        GuestValue::String(s) => Value::String(s),
        GuestValue::Bool(b) => Value::Bool(b),
        GuestValue::Color(c) => Value::Color(c),
        GuestValue::Node(id) => Value::Node(NodeId(id as usize)),
        GuestValue::Enum { enum_name, variant } => Value::Enum { enum_name, variant },
        GuestValue::Timer {
            time,
            duration,
            dt,
            frames,
        } => Value::Timer {
            time,
            duration,
            dt,
            frames,
        },
        GuestValue::IndefiniteTimer { time, dt, frames } => {
            Value::IndefiniteTimer { time, dt, frames }
        }
        GuestValue::Unset => Value::Unset,
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
