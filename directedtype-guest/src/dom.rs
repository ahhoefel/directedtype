use crate::value::Value;
#[cfg(target_arch = "wasm32")]
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Opaque Component DOM handle packed losslessly as `(generation << 32) | index`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeHandle(pub u64);

/// Errors returned by Component DOM queries and mutations during `on_prepare`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
pub enum DomError {
    #[error("Invalid or stale DOM node handle")]
    InvalidHandle,
    #[error("DOM hierarchy cycle detected")]
    HierarchyCycle,
    #[error("Target node is not a child of the specified parent")]
    NotAChild,
    #[error("Failed to parse DTML fragment: {0}")]
    ParseError(String),
    #[error("Asset not found: {0}")]
    AssetNotFound(String),
    #[error("Asset access denied outside sandbox: {0}")]
    AssetAccessDenied(String),
    #[error("Pre-layout transform budget exceeded: {0}")]
    BudgetExceeded(String),
    #[error("DOM error: {0}")]
    Custom(String),
}

impl From<DomError> for crate::DispatchError {
    fn from(err: DomError) -> Self {
        crate::DispatchError::Custom(err.to_string())
    }
}

/// Summary metadata for a hyperlink or anchor node returned by `PrepareContext::document_links`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomLinkInfo {
    pub handle: NodeHandle,
    pub tag: String,
    pub url_or_name: Option<String>,
    pub target_handle: Option<NodeHandle>,
}

/// Wire envelope shared across the WASM `dt_dom` ABI.
#[doc(hidden)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WireResult<T> {
    Ok(T),
    Err(DomError),
}

#[doc(hidden)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomSiblingsWire {
    pub prev: Option<NodeHandle>,
    pub next: Option<NodeHandle>,
}

#[doc(hidden)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomNodeInfoWire {
    pub tag: String,
    pub key: Option<Vec<Value>>,
    pub has_text: bool,
}

#[doc(hidden)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateElementWire {
    pub tag: String,
    pub key: Option<Vec<Value>>,
    pub ports: Vec<(String, Value)>,
}

#[doc(hidden)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTextWire {
    pub text: String,
    pub ports: Vec<(String, Value)>,
}

#[doc(hidden)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetPortLiteralWire {
    pub port: String,
    pub value: Value,
}

#[doc(hidden)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryByTagWire {
    pub tags: Vec<String>,
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "dt_dom")]
extern "C" {
    fn __dt_dom_roots() -> u64;
    fn __dt_dom_children(node: u64) -> u64;
    fn __dt_dom_parent(node: u64) -> u64;
    fn __dt_dom_siblings(node: u64) -> u64;
    fn __dt_dom_node_info(node: u64) -> u64;
    fn __dt_dom_get_port_literal(node: u64, ptr: u32, len: u32) -> u64;
    fn __dt_dom_get_text_content(node: u64, recursive: u32) -> u64;
    fn __dt_dom_query_by_tag(scope: u64, ptr: u32, len: u32) -> u64;
    fn __dt_dom_ancestors(node: u64) -> u64;
    fn __dt_dom_document_links(scope: u64) -> u64;
    fn __dt_dom_create_element(ptr: u32, len: u32) -> u64;
    fn __dt_dom_create_text(ptr: u32, len: u32) -> u64;
    fn __dt_dom_parse_fragment(ptr: u32, len: u32) -> u64;
    fn __dt_dom_append_child(parent: u64, child: u64) -> u64;
    fn __dt_dom_insert_before(parent: u64, before: u64, child: u64) -> u64;
    fn __dt_dom_remove_child(parent: u64, child: u64) -> u64;
    fn __dt_dom_replace_child(parent: u64, old: u64, new: u64) -> u64;
    fn __dt_dom_set_port_literal(node: u64, ptr: u32, len: u32) -> u64;
    fn __dt_dom_set_port_node_ref(node: u64, target: u64, ptr: u32, len: u32) -> u64;
    fn __dt_dom_remove_port(node: u64, ptr: u32, len: u32) -> u64;
    fn __dt_dom_set_text(node: u64, ptr: u32, len: u32) -> u64;
    fn __dt_host_read_asset(ptr: u32, len: u32) -> u64;
}

#[cfg(target_arch = "wasm32")]
fn decode_packed_wire<T: DeserializeOwned>(packed: u64) -> Result<T, DomError> {
    if packed == 0 {
        return Err(DomError::Custom("Null host DOM response pointer".into()));
    }
    let (ptr, len) = crate::unpack_ptr_len(packed);
    let bytes = unsafe { Vec::from_raw_parts(ptr as *mut u8, len as usize, len as usize) };
    let wire: WireResult<T> = serde_json::from_slice(&bytes)
        .map_err(|e| DomError::Custom(format!("Failed to decode host DOM response: {e}")))?;
    match wire {
        WireResult::Ok(val) => Ok(val),
        WireResult::Err(err) => Err(err),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub trait HostDomBridge {
    fn roots(&mut self) -> Result<Vec<NodeHandle>, DomError>;
    fn children(&mut self, node: NodeHandle) -> Result<Vec<NodeHandle>, DomError>;
    fn parent(&mut self, node: NodeHandle) -> Result<Option<NodeHandle>, DomError>;
    fn prev_sibling(&mut self, node: NodeHandle) -> Result<Option<NodeHandle>, DomError>;
    fn next_sibling(&mut self, node: NodeHandle) -> Result<Option<NodeHandle>, DomError>;
    fn tag_name(&mut self, node: NodeHandle) -> Result<String, DomError>;
    fn node_key(&mut self, node: NodeHandle) -> Result<Option<Vec<Value>>, DomError>;
    fn get_port_literal(&mut self, node: NodeHandle, port: &str) -> Result<Option<Value>, DomError>;
    fn get_text_content(&mut self, node: NodeHandle) -> Result<Option<String>, DomError>;
    fn get_subtree_text(&mut self, node: NodeHandle) -> Result<String, DomError>;
    fn query_by_tag(&mut self, scope: Option<NodeHandle>, tags: &[&str]) -> Result<Vec<NodeHandle>, DomError>;
    fn ancestors(&mut self, node: NodeHandle) -> Result<Vec<NodeHandle>, DomError>;
    fn document_links(&mut self, scope: Option<NodeHandle>) -> Result<Vec<DomLinkInfo>, DomError>;
    fn read_asset(&mut self, rel_path: &str) -> Result<Vec<u8>, DomError>;

    fn create_element(
        &mut self,
        tag: &str,
        key: Option<&[Value]>,
        ports: &[(&str, Value)],
    ) -> Result<NodeHandle, DomError>;
    fn create_text(&mut self, text: &str, ports: &[(&str, Value)]) -> Result<NodeHandle, DomError>;
    fn parse_fragment(&mut self, dt_snippet: &str) -> Result<NodeHandle, DomError>;
    fn append_child(&mut self, parent: NodeHandle, child: NodeHandle) -> Result<(), DomError>;
    fn insert_before(
        &mut self,
        parent: NodeHandle,
        before: NodeHandle,
        child: NodeHandle,
    ) -> Result<(), DomError>;
    fn remove_child(&mut self, parent: NodeHandle, child: NodeHandle) -> Result<(), DomError>;
    fn replace_child(
        &mut self,
        parent: NodeHandle,
        old_child: NodeHandle,
        new_child: NodeHandle,
    ) -> Result<(), DomError>;
    fn set_port_literal(
        &mut self,
        node: NodeHandle,
        port: &str,
        value: Value,
    ) -> Result<(), DomError>;
    fn set_port_node_ref(
        &mut self,
        node: NodeHandle,
        port: &str,
        target: NodeHandle,
    ) -> Result<(), DomError>;
    fn remove_port(&mut self, node: NodeHandle, port: &str) -> Result<(), DomError>;
    fn set_text(&mut self, node: NodeHandle, text: &str) -> Result<(), DomError>;
    fn wrap_node(&mut self, node: NodeHandle, wrapper: NodeHandle) -> Result<(), DomError>;
}

#[cfg(not(target_arch = "wasm32"))]
std::thread_local! {
    static ACTIVE_HOST_BRIDGE: std::cell::Cell<Option<std::ptr::NonNull<dyn HostDomBridge>>> =
        const { std::cell::Cell::new(None) };
}

/// Runs `f` with a scoped native `PrepareContext` backed by `bridge`.
#[cfg(not(target_arch = "wasm32"))]
pub fn with_host_dom_bridge<R>(
    bridge: &mut dyn HostDomBridge,
    self_handle: NodeHandle,
    f: impl FnOnce(&mut PrepareContext<'_>) -> R,
) -> R {
    struct BridgeGuard(Option<std::ptr::NonNull<dyn HostDomBridge>>);
    impl Drop for BridgeGuard {
        fn drop(&mut self) {
            ACTIVE_HOST_BRIDGE.with(|cell| cell.set(self.0));
        }
    }

    let raw: *mut (dyn HostDomBridge + '_) = bridge;
    let raw_static: *mut (dyn HostDomBridge + 'static) = unsafe { std::mem::transmute(raw) };
    let ptr = std::ptr::NonNull::new(raw_static);
    let prev = ACTIVE_HOST_BRIDGE.with(|cell| cell.replace(ptr));
    let _guard = BridgeGuard(prev);
    let mut ctx = PrepareContext::new(self_handle);
    f(&mut ctx)
}

#[cfg(not(target_arch = "wasm32"))]
fn with_active_bridge<R>(f: impl FnOnce(&mut dyn HostDomBridge) -> Result<R, DomError>) -> Result<R, DomError> {
    let ptr = ACTIVE_HOST_BRIDGE
        .with(|cell| cell.get())
        .ok_or_else(|| DomError::Custom("PrepareContext used outside active host DOM prepare session".into()))?;
    // SAFETY: `with_host_dom_bridge` keeps the underlying `&mut dyn HostDomBridge` alive and exclusive
    // on the current thread for the entire dynamic extent of `f`.
    let bridge = unsafe { &mut *ptr.as_ptr() };
    f(bridge)
}

/// Execution context passed to `Component::on_prepare` prior to AST expansion and layout solving.
pub struct PrepareContext<'a> {
    self_handle: NodeHandle,
    _marker: std::marker::PhantomData<&'a mut ()>,
}

impl<'a> PrepareContext<'a> {
    pub fn new(self_handle: NodeHandle) -> Self {
        Self {
            self_handle,
            _marker: std::marker::PhantomData,
        }
    }

    /// The Component DOM handle of the node whose `on_prepare` hook is running.
    pub fn self_handle(&self) -> NodeHandle {
        self.self_handle
    }

    // -------------------------------------------------------------------------
    // Read / Query Methods
    // -------------------------------------------------------------------------

    pub fn root_nodes(&self) -> Result<Vec<NodeHandle>, DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            decode_packed_wire(unsafe { __dt_dom_roots() })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.roots())
        }
    }

    pub fn children(&self, node: NodeHandle) -> Result<Vec<NodeHandle>, DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            decode_packed_wire(unsafe { __dt_dom_children(node.0) })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.children(node))
        }
    }

    pub fn parent(&self, node: NodeHandle) -> Result<Option<NodeHandle>, DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            decode_packed_wire(unsafe { __dt_dom_parent(node.0) })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.parent(node))
        }
    }

    pub fn prev_sibling(&self, node: NodeHandle) -> Result<Option<NodeHandle>, DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            let sibs: DomSiblingsWire = decode_packed_wire(unsafe { __dt_dom_siblings(node.0) })?;
            Ok(sibs.prev)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.prev_sibling(node))
        }
    }

    pub fn next_sibling(&self, node: NodeHandle) -> Result<Option<NodeHandle>, DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            let sibs: DomSiblingsWire = decode_packed_wire(unsafe { __dt_dom_siblings(node.0) })?;
            Ok(sibs.next)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.next_sibling(node))
        }
    }

    pub fn tag_name(&self, node: NodeHandle) -> Result<String, DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            let info: DomNodeInfoWire = decode_packed_wire(unsafe { __dt_dom_node_info(node.0) })?;
            Ok(info.tag)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.tag_name(node))
        }
    }

    pub fn node_key(&self, node: NodeHandle) -> Result<Option<Vec<Value>>, DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            let info: DomNodeInfoWire = decode_packed_wire(unsafe { __dt_dom_node_info(node.0) })?;
            Ok(info.key)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.node_key(node))
        }
    }

    pub fn get_port_literal(&self, node: NodeHandle, port: &str) -> Result<Option<Value>, DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            let bytes = port.as_bytes();
            decode_packed_wire(unsafe {
                __dt_dom_get_port_literal(node.0, bytes.as_ptr() as u32, bytes.len() as u32)
            })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.get_port_literal(node, port))
        }
    }

    pub fn get_text_content(&self, node: NodeHandle) -> Result<Option<String>, DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            decode_packed_wire(unsafe { __dt_dom_get_text_content(node.0, 0) })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.get_text_content(node))
        }
    }

    pub fn get_subtree_text(&self, node: NodeHandle) -> Result<String, DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            let opt: Option<String> =
                decode_packed_wire(unsafe { __dt_dom_get_text_content(node.0, 1) })?;
            Ok(opt.unwrap_or_default())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.get_subtree_text(node))
        }
    }

    pub fn query_by_tag(
        &self,
        scope: Option<NodeHandle>,
        tags: &[&str],
    ) -> Result<Vec<NodeHandle>, DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            let wire = QueryByTagWire {
                tags: tags.iter().map(|s| (*s).to_string()).collect(),
            };
            let bytes = serde_json::to_vec(&wire)
                .map_err(|e| DomError::Custom(format!("Failed to encode query_by_tag: {e}")))?;
            decode_packed_wire(unsafe {
                __dt_dom_query_by_tag(
                    scope.map_or(0, |h| h.0),
                    bytes.as_ptr() as u32,
                    bytes.len() as u32,
                )
            })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.query_by_tag(scope, tags))
        }
    }

    pub fn ancestors(&self, node: NodeHandle) -> Result<Vec<NodeHandle>, DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            decode_packed_wire(unsafe { __dt_dom_ancestors(node.0) })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.ancestors(node))
        }
    }

    pub fn document_links(&self, scope: Option<NodeHandle>) -> Result<Vec<DomLinkInfo>, DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            decode_packed_wire(unsafe { __dt_dom_document_links(scope.map_or(0, |h| h.0)) })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.document_links(scope))
        }
    }

    pub fn read_asset(&self, rel_path: &str) -> Result<Vec<u8>, DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            let bytes = rel_path.as_bytes();
            decode_packed_wire(unsafe {
                __dt_host_read_asset(bytes.as_ptr() as u32, bytes.len() as u32)
            })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.read_asset(rel_path))
        }
    }

    // -------------------------------------------------------------------------
    // Structural Mutation Methods
    // -------------------------------------------------------------------------

    pub fn create_element(
        &mut self,
        tag: &str,
        key: Option<&[Value]>,
        ports: &[(&str, Value)],
    ) -> Result<NodeHandle, DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            let wire = CreateElementWire {
                tag: tag.to_string(),
                key: key.map(|k| k.to_vec()),
                ports: ports.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect(),
            };
            let bytes = serde_json::to_vec(&wire)
                .map_err(|e| DomError::Custom(format!("Failed to encode create_element: {e}")))?;
            decode_packed_wire(unsafe {
                __dt_dom_create_element(bytes.as_ptr() as u32, bytes.len() as u32)
            })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.create_element(tag, key, ports))
        }
    }

    pub fn create_text(&mut self, text: &str, ports: &[(&str, Value)]) -> Result<NodeHandle, DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            let wire = CreateTextWire {
                text: text.to_string(),
                ports: ports.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect(),
            };
            let bytes = serde_json::to_vec(&wire)
                .map_err(|e| DomError::Custom(format!("Failed to encode create_text: {e}")))?;
            decode_packed_wire(unsafe {
                __dt_dom_create_text(bytes.as_ptr() as u32, bytes.len() as u32)
            })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.create_text(text, ports))
        }
    }

    pub fn parse_fragment(&mut self, dt_snippet: &str) -> Result<NodeHandle, DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            let bytes = dt_snippet.as_bytes();
            decode_packed_wire(unsafe {
                __dt_dom_parse_fragment(bytes.as_ptr() as u32, bytes.len() as u32)
            })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.parse_fragment(dt_snippet))
        }
    }

    pub fn append_child(&mut self, parent: NodeHandle, child: NodeHandle) -> Result<(), DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            decode_packed_wire(unsafe { __dt_dom_append_child(parent.0, child.0) })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.append_child(parent, child))
        }
    }

    pub fn insert_before(
        &mut self,
        parent: NodeHandle,
        before: NodeHandle,
        child: NodeHandle,
    ) -> Result<(), DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            decode_packed_wire(unsafe { __dt_dom_insert_before(parent.0, before.0, child.0) })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.insert_before(parent, before, child))
        }
    }

    pub fn remove_child(&mut self, parent: NodeHandle, child: NodeHandle) -> Result<(), DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            decode_packed_wire(unsafe { __dt_dom_remove_child(parent.0, child.0) })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.remove_child(parent, child))
        }
    }

    pub fn replace_child(
        &mut self,
        parent: NodeHandle,
        old_child: NodeHandle,
        new_child: NodeHandle,
    ) -> Result<(), DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            decode_packed_wire(unsafe { __dt_dom_replace_child(parent.0, old_child.0, new_child.0) })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.replace_child(parent, old_child, new_child))
        }
    }

    pub fn set_port_literal(
        &mut self,
        node: NodeHandle,
        port: &str,
        value: Value,
    ) -> Result<(), DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            let wire = SetPortLiteralWire {
                port: port.to_string(),
                value,
            };
            let bytes = serde_json::to_vec(&wire)
                .map_err(|e| DomError::Custom(format!("Failed to encode set_port_literal: {e}")))?;
            decode_packed_wire(unsafe {
                __dt_dom_set_port_literal(node.0, bytes.as_ptr() as u32, bytes.len() as u32)
            })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.set_port_literal(node, port, value))
        }
    }

    pub fn set_port_node_ref(
        &mut self,
        node: NodeHandle,
        port: &str,
        target: NodeHandle,
    ) -> Result<(), DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            let bytes = port.as_bytes();
            decode_packed_wire(unsafe {
                __dt_dom_set_port_node_ref(
                    node.0,
                    target.0,
                    bytes.as_ptr() as u32,
                    bytes.len() as u32,
                )
            })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.set_port_node_ref(node, port, target))
        }
    }

    pub fn remove_port(&mut self, node: NodeHandle, port: &str) -> Result<(), DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            let bytes = port.as_bytes();
            decode_packed_wire(unsafe {
                __dt_dom_remove_port(node.0, bytes.as_ptr() as u32, bytes.len() as u32)
            })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.remove_port(node, port))
        }
    }

    pub fn set_text(&mut self, node: NodeHandle, text: &str) -> Result<(), DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            let bytes = text.as_bytes();
            decode_packed_wire(unsafe {
                __dt_dom_set_text(node.0, bytes.as_ptr() as u32, bytes.len() as u32)
            })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.set_text(node, text))
        }
    }

    /// Replaces `node` in its parent (or roots) with `wrapper` and moves `node` inside `wrapper`.
    pub fn wrap_node(&mut self, node: NodeHandle, wrapper: NodeHandle) -> Result<(), DomError> {
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(parent) = self.parent(node)? {
                self.replace_child(parent, node, wrapper)?;
                self.append_child(wrapper, node)?;
                Ok(())
            } else {
                Err(DomError::Custom(
                    "Cannot wrap root node without parent across legacy wasm bridge".into(),
                ))
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            with_active_bridge(|b| b.wrap_node(node, wrapper))
        }
    }
}
