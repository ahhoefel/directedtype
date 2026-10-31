use crate::ast::*;
use crate::compiler::expanded::{ExpandedNode, NodeId};
use crate::compiler::scope::ScopeId;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// A lexical binding in the component or document scope.
#[derive(Debug, Clone)]
pub enum LexicalBinding {
    Expr(Expr),
    State(Expr),
    Node(NodeId),
    Uninitialized,
}

/// An environmental entry in the active Lexical Scope Stack.
#[derive(Debug, Clone, PartialEq)]
pub enum EnvEntry {
    Bound(Expr),
    Tombstone,
}

/// Context used when rewriting expressions to bind variables to concrete node IDs.
#[derive(Debug, Clone)]
pub struct ScopeContext<'a> {
    pub current_node: NodeId,
    pub parent_node: Option<NodeId>,
    pub parent_name: Option<&'a str>,
    pub prev_sibling: Option<NodeId>,
    pub has_next_sibling: bool,
    pub child_ids: &'a [NodeId],
    pub num_children: usize,
    pub child_node: Option<NodeId>,
    pub nodes: &'a [ExpandedNode],
    pub shadowed_ports: Option<&'a HashSet<String>>,
    pub is_event_port: bool,
    pub parent_ports: &'a [String],
    pub lexical_scope: &'a HashMap<String, LexicalBinding>,
    pub env_scope: &'a HashMap<String, EnvEntry>,
    pub node_fonts: &'a HashMap<NodeId, NodeId>,
    pub enclosing_component: Option<NodeId>,
    pub comp_ports: Option<&'a HashMap<String, Expr>>,
    pub declared_state_names: Option<&'a HashSet<String>>,
    pub enums: &'a HashMap<String, EnumDef>,
}

impl<'a> ScopeContext<'a> {
    /// Creates a `ScopeContext` rooted at `NodeId::WINDOW`.
    pub fn for_window(
        nodes: &'a [ExpandedNode],
        lexical_scope: &'a HashMap<String, LexicalBinding>,
        env_scope: &'a HashMap<String, EnvEntry>,
        node_fonts: &'a HashMap<NodeId, NodeId>,
        enums: &'a HashMap<String, EnumDef>,
    ) -> Self {
        Self {
            current_node: NodeId::WINDOW,
            parent_node: None,
            parent_name: None,
            prev_sibling: None,
            has_next_sibling: false,
            child_ids: &[],
            num_children: 0,
            child_node: None,
            nodes,
            shadowed_ports: None,
            is_event_port: false,
            parent_ports: &[],
            lexical_scope,
            env_scope,
            node_fonts,
            enclosing_component: None,
            comp_ports: None,
            declared_state_names: None,
            enums,
        }
    }

    /// Creates a `ScopeContext` for `current_node` inheriting hierarchy and scopes from an `ElementContext`.
    pub(super) fn from_element(
        current_node: NodeId,
        ctx: &ElementContext<'a>,
        nodes: &'a [ExpandedNode],
        lexical_scope: &'a HashMap<String, LexicalBinding>,
        node_fonts: &'a HashMap<NodeId, NodeId>,
        child_ids: &'a [NodeId],
    ) -> Self {
        Self {
            current_node,
            parent_node: ctx.parent_id,
            parent_name: ctx.parent_name,
            prev_sibling: ctx.prev_sibling_id,
            has_next_sibling: ctx.has_next_sibling,
            child_ids,
            num_children: ctx.enclosing_num_children,
            child_node: None,
            nodes,
            shadowed_ports: ctx.shadowed_ports,
            is_event_port: false,
            parent_ports: ctx.parent_ports,
            lexical_scope,
            env_scope: ctx.env_scope,
            node_fonts,
            enclosing_component: ctx.enclosing_component_id,
            comp_ports: None,
            declared_state_names: None,
            enums: ctx.enums,
        }
    }

    /// Creates a `ScopeContext` for a component instance from an `InstanceContext`.
    pub(super) fn from_instance(
        ctx: &InstanceContext<'a>,
        nodes: &'a [ExpandedNode],
        lexical_scope: &'a HashMap<String, LexicalBinding>,
        env_scope: &'a HashMap<String, EnvEntry>,
        node_fonts: &'a HashMap<NodeId, NodeId>,
        child_ids: &'a [NodeId],
    ) -> Self {
        Self {
            current_node: ctx.comp_node_id,
            parent_node: ctx.parent_id,
            parent_name: ctx.parent_name,
            prev_sibling: ctx.prev_sibling_id,
            has_next_sibling: ctx.has_next_sibling,
            child_ids,
            num_children: ctx.enclosing_num_children,
            child_node: None,
            nodes,
            shadowed_ports: ctx.shadowed_ports,
            is_event_port: false,
            parent_ports: ctx.parent_ports,
            lexical_scope,
            env_scope,
            node_fonts,
            enclosing_component: ctx.enclosing_component_id,
            comp_ports: None,
            declared_state_names: None,
            enums: ctx.enums,
        }
    }

    pub fn with_children(mut self, child_ids: &'a [NodeId], num_children: usize) -> Self {
        self.child_ids = child_ids;
        self.num_children = num_children;
        self
    }

    pub fn with_child_node(mut self, child_node: Option<NodeId>) -> Self {
        self.child_node = child_node;
        self
    }

    pub fn with_parent(
        mut self,
        parent_node: Option<NodeId>,
        parent_name: Option<&'a str>,
        parent_ports: &'a [String],
    ) -> Self {
        self.parent_node = parent_node;
        self.parent_name = parent_name;
        self.parent_ports = parent_ports;
        self
    }

    pub fn with_siblings(mut self, prev_sibling: Option<NodeId>, has_next_sibling: bool) -> Self {
        self.prev_sibling = prev_sibling;
        self.has_next_sibling = has_next_sibling;
        self
    }

    pub fn with_shadowed_ports(mut self, shadowed_ports: Option<&'a HashSet<String>>) -> Self {
        self.shadowed_ports = shadowed_ports;
        self
    }

    pub fn with_enclosing_component(mut self, enclosing_component: Option<NodeId>) -> Self {
        self.enclosing_component = enclosing_component;
        self
    }

    pub fn with_comp_state(
        mut self,
        comp_ports: Option<&'a HashMap<String, Expr>>,
        declared_state_names: Option<&'a HashSet<String>>,
    ) -> Self {
        self.comp_ports = comp_ports;
        self.declared_state_names = declared_state_names;
        self
    }

    pub fn as_event_port(mut self, is_event_port: bool) -> Self {
        self.is_event_port = is_event_port;
        self
    }

    pub fn with_env_scope(mut self, env_scope: &'a HashMap<String, EnvEntry>) -> Self {
        self.env_scope = env_scope;
        self
    }
}

#[derive(Clone)]
pub(super) struct ForwardedChildrenDirective<'a> {
    pub ports: &'a [PortBinding],
    pub parent_id: NodeId,
    pub parent_name: &'a str,
    pub parent_ports: Vec<String>,
    pub lexical_scope: &'a RefCell<HashMap<String, LexicalBinding>>,
    pub env_scope: HashMap<String, EnvEntry>,
    pub enclosing_component_id: Option<NodeId>,
    pub enclosing_num_children: usize,
    pub enclosing_instantiated_children: Option<&'a RefCell<Vec<NodeId>>>,
    pub shadowed_ports: Option<&'a HashSet<String>>,
    pub comp_ports: Option<&'a HashMap<String, Expr>>,
    pub declared_state_names: Option<&'a HashSet<String>>,
}

#[derive(Clone)]
pub(super) struct ConsumerChildEntry<'a> {
    pub elem: ElementNode,
    pub var_name: Option<String>,
    pub raw_text: Option<TextChunk>,
    pub caller_lexical_scope: &'a RefCell<HashMap<String, LexicalBinding>>,
    pub caller_env_scope: HashMap<String, EnvEntry>,
    pub caller_enclosing_component_id: Option<NodeId>,
    pub caller_enclosing_num_children: usize,
    pub caller_enclosing_instantiated_children: Option<&'a RefCell<Vec<NodeId>>>,
    pub caller_shadowed_ports: Option<&'a HashSet<String>>,
    pub instantiated_ids: Vec<&'a RefCell<Vec<NodeId>>>,
    pub forwarded_directives: Vec<ForwardedChildrenDirective<'a>>,
}

pub(super) struct InstanceContext<'a> {
    pub comp_node_id: NodeId,
    pub parent_id: Option<NodeId>,
    pub parent_name: Option<&'a str>,
    pub prev_sibling_id: Option<NodeId>,
    pub has_next_sibling: bool,
    pub parent_ports: &'a [String],
    pub ambient_authored_ports: Option<&'a HashMap<String, Expr>>,
    pub enclosing_component_id: Option<NodeId>,
    pub enclosing_num_children: usize,
    pub enclosing_instantiated_children: Option<&'a RefCell<Vec<NodeId>>>,
    pub shadowed_ports: Option<&'a HashSet<String>>,
    pub enums: &'a HashMap<String, EnumDef>,
    pub active_scope_id: ScopeId,
    pub consumer_children: Option<&'a [ConsumerChildEntry<'a>]>,
}

pub(super) struct ElementContext<'a> {
    pub parent_id: Option<NodeId>,
    pub parent_name: Option<&'a str>,
    pub prev_sibling_id: Option<NodeId>,
    pub has_next_sibling: bool,
    pub parent_ports: &'a [String],
    pub lexical_scope: &'a RefCell<HashMap<String, LexicalBinding>>,
    pub env_scope: &'a HashMap<String, EnvEntry>,
    pub enclosing_component_id: Option<NodeId>,
    pub enclosing_num_children: usize,
    pub enclosing_instantiated_children: Option<&'a RefCell<Vec<NodeId>>>,
    pub shadowed_ports: Option<&'a HashSet<String>>,
    pub is_let: bool,
    pub ambient_authored_ports: Option<&'a HashMap<String, Expr>>,
    pub enums: &'a HashMap<String, EnumDef>,
    pub active_scope_id: ScopeId,
    pub consumer_children: Option<&'a [ConsumerChildEntry<'a>]>,
}

