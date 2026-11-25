use crate::ast::{ComponentDef, ComponentKey, ElementNode, EnumDef, Expr, Symbol, TextChunk};
use crate::compiler::expand::context::{EnvEntry, LexicalBinding};
use crate::compiler::expand::schema::{PortMask, PortTable};
use crate::compiler::expand::StaticType;
use crate::dom::NodeHandle;
use crate::span::Span;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock, RwLock};

pub(crate) fn empty_expr_map() -> Arc<HashMap<String, Expr>> {
    static EMPTY: OnceLock<Arc<HashMap<String, Expr>>> = OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| Arc::new(HashMap::new())))
}

pub(crate) fn empty_string_map() -> Arc<HashMap<String, String>> {
    static EMPTY: OnceLock<Arc<HashMap<String, String>>> = OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| Arc::new(HashMap::new())))
}

pub(crate) fn empty_string_set() -> Arc<HashSet<String>> {
    static EMPTY: OnceLock<Arc<HashSet<String>>> = OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| Arc::new(HashSet::new())))
}

pub(crate) fn empty_opt_string_map() -> Arc<HashMap<String, Option<String>>> {
    static EMPTY: OnceLock<Arc<HashMap<String, Option<String>>>> = OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| Arc::new(HashMap::new())))
}

pub(crate) fn empty_string_vec() -> Arc<Vec<String>> {
    static EMPTY: OnceLock<Arc<Vec<String>>> = OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| Arc::new(Vec::new())))
}

/// Layered map of authored port expressions, sharing a component's static `base` map via `Arc`
/// while storing only instance-specific `overrides`.
#[derive(Debug, Clone)]
pub struct AuthoredPorts {
    pub base: Option<Arc<HashMap<String, Expr>>>,
    pub overrides: Arc<HashMap<String, Expr>>,
}

impl Default for AuthoredPorts {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthoredPorts {
    #[inline]
    pub fn new() -> Self {
        Self {
            base: None,
            overrides: empty_expr_map(),
        }
    }

    #[inline]
    pub fn from_map(map: HashMap<String, Expr>) -> Self {
        if map.is_empty() {
            Self::new()
        } else {
            Self {
                base: None,
                overrides: Arc::new(map),
            }
        }
    }

    #[inline]
    pub fn from_arc(overrides: Arc<HashMap<String, Expr>>) -> Self {
        Self {
            base: None,
            overrides,
        }
    }

    #[inline]
    pub fn from_base_and_overrides(
        base: Arc<HashMap<String, Expr>>,
        overrides: HashMap<String, Expr>,
    ) -> Self {
        Self {
            base: Some(base),
            overrides: if overrides.is_empty() {
                empty_expr_map()
            } else {
                Arc::new(overrides)
            },
        }
    }

    #[inline]
    pub fn get(&self, k: &str) -> Option<&Expr> {
        self.overrides
            .get(k)
            .or_else(|| self.base.as_ref().and_then(|b| b.get(k)))
    }

    #[inline]
    pub fn contains_key(&self, k: &str) -> bool {
        self.overrides.contains_key(k)
            || self.base.as_ref().is_some_and(|b| b.contains_key(k))
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.overrides.is_empty() && self.base.as_ref().is_none_or(|b| b.is_empty())
    }

    #[inline]
    pub fn len(&self) -> usize {
        match &self.base {
            None => self.overrides.len(),
            Some(b) if self.overrides.is_empty() => b.len(),
            Some(b) => {
                let mut count = b.len();
                for k in self.overrides.keys() {
                    if !b.contains_key(k) {
                        count += 1;
                    }
                }
                count
            }
        }
    }
}

impl PartialEq for AuthoredPorts {
    fn eq(&self, other: &Self) -> bool {
        let base_ptr_eq = match (&self.base, &other.base) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        if base_ptr_eq && Arc::ptr_eq(&self.overrides, &other.overrides) {
            return true;
        }
        if self.len() != other.len() {
            return false;
        }
        if let Some(b) = &self.base {
            for (k, v) in b.iter() {
                if !self.overrides.contains_key(k) && other.get(k) != Some(v) {
                    return false;
                }
            }
        }
        for (k, v) in self.overrides.iter() {
            if other.get(k) != Some(v) {
                return false;
            }
        }
        true
    }
}

impl Eq for AuthoredPorts {}

static CANONICAL_NODE_NAMES: OnceLock<RwLock<Vec<&'static str>>> = OnceLock::new();

fn interned_canonical_node_name(idx: usize) -> &'static str {
    let lock = CANONICAL_NODE_NAMES.get_or_init(|| {
        let mut v = Vec::with_capacity(4096);
        for i in 0..4096 {
            v.push(&*Box::leak(format!("__node_{}", i).into_boxed_str()));
        }
        RwLock::new(v)
    });
    {
        let read = lock.read().unwrap();
        if let Some(&s) = read.get(idx) {
            return s;
        }
    }
    let mut write = lock.write().unwrap();
    while write.len() <= idx {
        let i = write.len();
        write.push(&*Box::leak(format!("__node_{}", i).into_boxed_str()));
    }
    write[idx]
}

/// A unique, zero-based index for an expanded node in the layout tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub usize);

impl NodeId {
    /// Sentinel NodeId representing the ambient window root container.
    pub const WINDOW: NodeId = NodeId(usize::MAX);
    /// Sentinel NodeId representing the component instance (`self`) in pre-compiled port templates.
    pub const TEMPLATE_SELF: NodeId = NodeId(usize::MAX - 1);
    /// Sentinel NodeId representing the ambient `page` context inside `\PagedFlow` templates.
    pub const PAGE: NodeId = NodeId(usize::MAX - 2);

    #[inline]
    pub const fn is_window(&self) -> bool {
        self.0 == usize::MAX
    }

    #[inline]
    pub const fn is_template_self(&self) -> bool {
        self.0 == usize::MAX - 1
    }

    #[inline]
    pub const fn is_page(&self) -> bool {
        self.0 == usize::MAX - 2
    }

    #[inline]
    pub fn canonical_str(&self) -> &'static str {
        if self.is_window() {
            "__window"
        } else if self.is_template_self() {
            "__template_self"
        } else if self.is_page() {
            "__page"
        } else {
            interned_canonical_node_name(self.0)
        }
    }

    #[inline]
    pub fn canonical_name(&self) -> String {
        self.canonical_str().to_string()
    }

    #[inline]
    pub fn matches_str(&self, name: &str) -> bool {
        if self.is_window() {
            name == "__window" || name == "window"
        } else if self.is_page() {
            name == "__page" || name == "page"
        } else {
            Self::from_canonical_name(name) == Some(*self)
        }
    }

    #[inline]
    pub fn from_canonical_name(name: &str) -> Option<Self> {
        let bytes = name.as_bytes();
        if bytes.first() == Some(&b'_') {
            if let Some(num_str) = name.strip_prefix("__node_") {
                return num_str.parse::<usize>().ok().map(NodeId);
            }
            if name == "__window" {
                return Some(NodeId::WINDOW);
            }
            if name == "__template_self" {
                return Some(NodeId::TEMPLATE_SELF);
            }
            if name == "__page" {
                return Some(NodeId::PAGE);
            }
            None
        } else if name == "window" {
            Some(NodeId::WINDOW)
        } else {
            None
        }
    }
}

use crate::compiler::text::{InlineBoxSpan, TextSpan};

/// Break target for `\Pagination(break_before: ...)` and `\PageBreak(to: ...)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageBreakTarget {
    #[default]
    Auto,
    Page,
    Recto,
    Verso,
}

/// Shallow per-box pagination configuration resolved from `pagination` port and `\let pagination`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PaginationSpec {
    pub breakable: bool,
    pub keep_with_next: bool,
    pub break_before: PageBreakTarget,
}

/// Owned snapshot of an unexpanded element and its lexical/environment capture
/// for deferred instantiation (such as `\PagedFlow(page: ...)` templates and `\Deposit` payloads).
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedConsumerChild {
    pub elem: ElementNode,
    pub var_name: Option<String>,
    pub raw_text: Option<TextChunk>,
    pub caller_lexical_scope: HashMap<Symbol, LexicalBinding>,
    pub caller_env_scope: HashMap<String, EnvEntry>,
    pub caller_enclosing_component_id: Option<NodeId>,
    pub caller_enclosing_num_children: usize,
    pub caller_shadowed_ports: HashSet<String>,
    pub outer_consumer_children: Vec<OwnedConsumerChild>,
}

/// Recorded `\PagedFlow` instance metadata for Phase 1–3 pagination.
#[derive(Debug, Clone, PartialEq)]
pub struct PagedFlowRecord {
    pub node_id: NodeId,
    pub page_template_elem: OwnedConsumerChild,
    pub probe_doc: ExpandedDocument,
    pub probe_base_node_len: usize,
    pub flow_body_node_id: NodeId,
    pub width_depends_on_window: bool,
    pub span: Span,
}

/// Recorded `\Deposit(into: bucket) { <SingleComponent> }` call site.
#[derive(Debug, Clone, PartialEq)]
pub struct DepositRecord {
    pub deposit_id: NodeId,
    pub bucket_id: NodeId,
    pub host_node_id: Option<NodeId>,
    pub inline_byte_index: Option<usize>,
    pub payload: OwnedConsumerChild,
    pub span: Span,
}

/// An expanded layout element with concrete node identity, hierarchy, and port equations.
use crate::compiler::scope::{ScopeId, ScopeTree};

#[derive(Debug, Clone, PartialEq)]
pub struct ExpandedNode {
    pub id: NodeId,
    pub name: String,
    pub key: Option<ComponentKey>,
    pub parent: Option<NodeId>,
    pub prev_sibling: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub ports: HashMap<String, Expr>,
    pub port_types: Arc<HashMap<String, String>>,
    pub preverified_port_types: Option<Arc<HashMap<Symbol, StaticType>>>,
    pub preverified_ports: PortMask,
    pub port_table: Option<Arc<PortTable>>,
    pub authored_ports: AuthoredPorts,
    pub private_ports: Arc<HashSet<String>>,
    pub state_vars: Arc<HashMap<String, Option<String>>>,
    pub env_vars: Arc<HashMap<String, Option<String>>>,
    pub event_handlers: HashMap<String, crate::component::EventHandlerBinding>,
    pub default_event_handlers: HashMap<String, crate::component::EventHandlerBinding>,
    pub text_content: Option<String>,
    pub text_spans: Vec<TextSpan>,
    pub inline_boxes: Vec<InlineBoxSpan>,
    pub anchor_name: Option<String>,
    pub scope_id: Option<ScopeId>,
    pub span: Span,
    pub handle: Option<NodeHandle>,
    pub font: Option<NodeId>,
    pub var_name: Option<String>,
    pub enclosing_component: Option<NodeId>,
    pub ambient_ports: HashSet<String>,
    pub overload_params: Arc<Vec<String>>,
    pub is_component: bool,
    pub is_span: bool,
    pub is_inline_box: bool,
    pub component_path: Option<String>,
    pub aggregated_ports: HashSet<String>,
    pub unfiltered_ports: HashMap<String, Expr>,
    pub skipped_upstream: HashMap<String, Vec<crate::compiler::graph::VarId>>,
    pub pagination: PaginationSpec,
    pub inline_deposits: Vec<(NodeId, usize)>,
}

impl ExpandedNode {
    pub fn new(id: NodeId, name: impl Into<String>, span: Span) -> Self {
        Self {
            id,
            name: name.into(),
            key: None,
            parent: None,
            prev_sibling: None,
            children: Vec::new(),
            ports: HashMap::new(),
            port_types: empty_string_map(),
            preverified_port_types: None,
            preverified_ports: PortMask::default(),
            port_table: None,
            authored_ports: AuthoredPorts::new(),
            private_ports: empty_string_set(),
            state_vars: empty_opt_string_map(),
            env_vars: empty_opt_string_map(),
            event_handlers: HashMap::new(),
            default_event_handlers: HashMap::new(),
            text_content: None,
            text_spans: Vec::new(),
            inline_boxes: Vec::new(),
            anchor_name: None,
            scope_id: None,
            span,
            handle: None,
            font: None,
            var_name: None,
            enclosing_component: None,
            ambient_ports: HashSet::new(),
            overload_params: empty_string_vec(),
            is_component: false,
            is_span: false,
            is_inline_box: false,
            component_path: None,
            aggregated_ports: HashSet::new(),
            unfiltered_ports: HashMap::new(),
            skipped_upstream: HashMap::new(),
            pagination: PaginationSpec::default(),
            inline_deposits: Vec::new(),
        }
    }

    #[inline]
    pub fn is_port_preverified(&self, port_name: &str) -> bool {
        if let Some(ref table) = self.port_table {
            self.preverified_ports.contains(table, port_name)
        } else {
            false
        }
    }

    #[inline]
    pub fn get_preverified_port_type(&self, port_sym: Symbol, port_name: &str) -> Option<&StaticType> {
        let table = self.port_table.as_ref()?;
        if !self.preverified_ports.contains_sym(table, port_sym, port_name) {
            return None;
        }
        self.preverified_port_types.as_ref()?.get(&port_sym)
    }

    pub fn is_paint_primitive(&self) -> bool {
        self.name == "Rect"
            || self.name == "Line"
            || self.name == "Text"
            || self.name == "Image"
            || self.text_content.is_some()
            || self.ports.contains_key("text")
    }
}

/// The collection of all expanded nodes in the document.
#[derive(Debug, Clone, PartialEq)]
pub struct ExpandedDocument {
    pub nodes: Vec<ExpandedNode>,
    pub roots: Vec<NodeId>,
    pub window_ports: HashMap<String, Expr>,
    pub window_port_types: HashMap<String, String>,
    pub window_state_vars: HashMap<String, Option<String>>,
    pub page_ports: HashMap<String, Expr>,
    pub page_template_component: Option<String>,
    pub page_template_private_ports: HashSet<String>,
    pub paged_flows: Vec<PagedFlowRecord>,
    pub deposits: Vec<DepositRecord>,
    pub paged_registry: Option<Arc<HashMap<String, Vec<ComponentDef>>>>,
    pub paged_enums: Option<Arc<HashMap<String, EnumDef>>>,
    pub active_page_deposits: HashMap<NodeId, Vec<usize>>,
    pub active_page_deposit_nodes: Vec<(usize, NodeId)>,
    pub dom_handles: HashMap<crate::dom::NodeHandle, NodeId>,
    pub local_bindings: Vec<(Expr, Option<String>, crate::span::Span)>,
    pub scope_tree: ScopeTree,
    pub default_dpi: f64,
    pub image_assets: Arc<crate::compiler::image::ImageAssetStore>,
}

impl ExpandedDocument {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            roots: Vec::new(),
            window_ports: HashMap::new(),
            window_port_types: HashMap::new(),
            window_state_vars: HashMap::new(),
            page_ports: HashMap::new(),
            page_template_component: None,
            page_template_private_ports: HashSet::new(),
            paged_flows: Vec::new(),
            deposits: Vec::new(),
            paged_registry: None,
            paged_enums: None,
            active_page_deposits: HashMap::new(),
            active_page_deposit_nodes: Vec::new(),
            dom_handles: HashMap::new(),
            local_bindings: Vec::new(),
            scope_tree: ScopeTree::new(),
            default_dpi: crate::compiler::expand::builtins::DEFAULT_WINDOW_DPI,
            image_assets: Arc::new(crate::compiler::image::ImageAssetStore::new()),
        }
    }

    pub fn get_node(&self, id: NodeId) -> Option<&ExpandedNode> {
        self.nodes.get(id.0)
    }

    pub fn get_node_mut(&mut self, id: NodeId) -> Option<&mut ExpandedNode> {
        self.nodes.get_mut(id.0)
    }

    #[inline]
    pub fn push_local_binding(&mut self, expr: Expr, ty: Option<String>, span: crate::span::Span) {
        let idx = self.local_bindings.len();
        crate::compiler::expand::rewrite::check_and_record_pending_fwd_binding(idx, &expr);
        self.local_bindings.push((expr, ty, span));
    }

    pub fn find_by_key(&self, parent: Option<NodeId>, key: &ComponentKey) -> Option<&ExpandedNode> {
        self.nodes.iter().find(|n| {
            if let Some(expected_parent) = parent {
                if n.parent != Some(expected_parent) {
                    return false;
                }
            }
            n.key.as_ref() == Some(key)
        })
    }
}

impl Default for ExpandedDocument {
    fn default() -> Self {
        Self::new()
    }
}
