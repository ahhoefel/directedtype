use crate::ast::*;
use crate::compiler::expanded::{ExpandedNode, NodeId};
use crate::compiler::scope::ScopeId;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// A lexical binding in the component or document scope.
#[derive(Debug, Clone, PartialEq)]
pub enum LexicalBinding {
    Expr(Expr),
    State(Expr),
    /// Zero-allocation member access binding (`<node_id>.<member>`) avoiding a boxed `Expr` per port in `local_scope`.
    Member {
        node_id: NodeId,
        member: Ident,
        is_state: bool,
    },
    Node(NodeId),
    Uninitialized,
}

impl LexicalBinding {
    /// Materializes this binding as an `Expr` if it is an `Expr`, `State`, or `Member`.
    #[inline]
    pub fn to_expr(&self) -> Option<Expr> {
        match self {
            LexicalBinding::Expr(e) | LexicalBinding::State(e) => Some(e.clone()),
            LexicalBinding::Member {
                node_id, member, ..
            } => Some(Expr::MemberAccess(MemberAccessExpr {
                target: std::sync::Arc::new(Expr::Ident(Ident::from_node(*node_id, member.span))),
                member: *member,
                span: member.span,
            })),
            LexicalBinding::Node(_) | LexicalBinding::Uninitialized => None,
        }
    }
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
    pub lexical_scope: &'a HashMap<Symbol, LexicalBinding>,
    pub env_scope: &'a HashMap<String, EnvEntry>,
    pub node_fonts: &'a HashMap<NodeId, NodeId>,
    pub enclosing_component: Option<NodeId>,
    pub comp_ports: Option<&'a HashMap<String, Expr>>,
    pub declared_state_names: Option<&'a HashSet<String>>,
    pub enums: &'a HashMap<String, EnumDef>,
    pub in_page_template: bool,
}

impl<'a> ScopeContext<'a> {
    /// Creates a `ScopeContext` rooted at `NodeId::WINDOW`.
    pub fn for_window(
        nodes: &'a [ExpandedNode],
        lexical_scope: &'a HashMap<Symbol, LexicalBinding>,
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
            in_page_template: false,
        }
    }

    /// Creates a `ScopeContext` for `current_node` inheriting hierarchy and scopes from an `ElementContext`.
    pub(super) fn from_element(
        current_node: NodeId,
        ctx: &ElementContext<'a>,
        nodes: &'a [ExpandedNode],
        lexical_scope: &'a HashMap<Symbol, LexicalBinding>,
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
            in_page_template: ctx.in_page_template,
        }
    }

    /// Creates a `ScopeContext` for a component instance from an `InstanceContext`.
    pub(super) fn from_instance(
        ctx: &InstanceContext<'a>,
        nodes: &'a [ExpandedNode],
        lexical_scope: &'a HashMap<Symbol, LexicalBinding>,
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
            in_page_template: ctx.in_page_template,
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

    pub fn with_page_template(mut self, in_page_template: bool) -> Self {
        self.in_page_template = in_page_template;
        self
    }
}

#[derive(Clone)]
pub(crate) struct ForwardedChildrenDirective<'a> {
    pub ports: &'a [PortBinding],
    pub parent_id: NodeId,
    pub parent_name: &'a str,
    pub parent_ports: Vec<String>,
    pub lexical_scope: &'a RefCell<HashMap<Symbol, LexicalBinding>>,
    pub env_scope: HashMap<String, EnvEntry>,
    pub enclosing_component_id: Option<NodeId>,
    pub enclosing_num_children: usize,
    pub enclosing_instantiated_children: Option<&'a RefCell<Vec<NodeId>>>,
    pub shadowed_ports: Option<&'a HashSet<String>>,
    pub comp_ports: Option<&'a HashMap<String, Expr>>,
    pub declared_state_names: Option<&'a HashSet<String>>,
}

#[derive(Clone)]
pub(crate) struct ConsumerChildEntry<'a> {
    pub elem: ElementNode,
    pub var_name: Option<String>,
    pub raw_text: Option<TextChunk>,
    pub caller_lexical_scope: &'a RefCell<HashMap<Symbol, LexicalBinding>>,
    pub owned_caller_lexical_scope: Option<std::rc::Rc<RefCell<HashMap<Symbol, LexicalBinding>>>>,
    pub caller_env_scope: HashMap<String, EnvEntry>,
    pub caller_enclosing_component_id: Option<NodeId>,
    pub caller_enclosing_num_children: usize,
    pub caller_enclosing_instantiated_children: Option<&'a RefCell<Vec<NodeId>>>,
    pub caller_shadowed_ports: Option<&'a HashSet<String>>,
    pub owned_shadowed_ports: Option<HashSet<String>>,
    pub instantiated_ids: Vec<&'a RefCell<Vec<NodeId>>>,
    pub forwarded_directives: Vec<ForwardedChildrenDirective<'a>>,
    pub deposit_payload_idx: Option<usize>,
    pub owned_children: Vec<crate::compiler::expanded::OwnedConsumerChild>,
}

impl<'a> ConsumerChildEntry<'a> {
    #[inline]
    pub fn effective_lexical_scope(&self) -> &RefCell<HashMap<Symbol, LexicalBinding>> {
        self.owned_caller_lexical_scope
            .as_deref()
            .unwrap_or(self.caller_lexical_scope)
    }

    #[inline]
    pub fn effective_shadowed_ports(&self) -> Option<&HashSet<String>> {
        self.owned_shadowed_ports
            .as_ref()
            .or(self.caller_shadowed_ports)
    }
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
    pub in_page_template: bool,
    pub in_paged_flow: bool,
    pub in_unbreakable_flow_container: Option<&'a str>,
}

pub(crate) struct ElementContext<'a> {
    pub parent_id: Option<NodeId>,
    pub parent_name: Option<&'a str>,
    pub prev_sibling_id: Option<NodeId>,
    pub has_next_sibling: bool,
    pub parent_ports: &'a [String],
    pub lexical_scope: &'a RefCell<HashMap<Symbol, LexicalBinding>>,
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
    pub in_paged_flow: bool,
    pub in_page_template: bool,
    pub in_unbreakable_flow_container: Option<&'a str>,
    pub enclosing_comp_ports: Option<&'a HashMap<String, Expr>>,
}

