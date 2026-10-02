use crate::ast::*;
use crate::compiler::error::CompileError;
use crate::compiler::expanded::{ExpandedDocument, ExpandedNode, NodeId};
use std::collections::HashMap;

/// A lexical binding in the component or document scope.
#[derive(Debug, Clone)]
pub enum LexicalBinding {
    Expr(Expr),
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
    pub prev_sibling: Option<NodeId>,
    pub child_ids: &'a [NodeId],
    pub parent_ports: &'a [String],
    pub current_ports: &'a [String],
    pub lexical_scope: &'a HashMap<String, LexicalBinding>,
    pub env_scope: &'a HashMap<String, EnvEntry>,
    pub node_fonts: &'a HashMap<NodeId, NodeId>,
}

/// Expands a parsed AST `Document` into an `ExpandedDocument`.
pub fn expand_document(doc: &Document) -> Result<ExpandedDocument, CompileError> {
    let mut registry = HashMap::new();
    let mut global_scope = HashMap::new();
    let mut global_env_scope: HashMap<String, EnvEntry> = HashMap::new();
    let mut node_fonts: HashMap<NodeId, NodeId> = HashMap::new();

    // Default universal clip: window.clip
    global_env_scope.insert(
        "clip".to_string(),
        EnvEntry::Bound(Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(Expr::Ident(Ident::new(NodeId::WINDOW.canonical_name(), doc.span))),
            member: Ident::new("clip", doc.span),
            span: doc.span,
        })),
    );

    // 1. Index all component definitions
    for item in &doc.items {
        if let Item::Component(comp) = item {
            registry.insert(comp.name.as_str().to_string(), comp.clone());
        }
    }

    let mut expanded_doc = ExpandedDocument::new();

    let window_scope_ports = vec![
        "x".to_string(),
        "y".to_string(),
        "width".to_string(),
        "height".to_string(),
        "z".to_string(),
        "clip".to_string(),
    ];

    // Pre-register uninitialized let and env items
    for item in &doc.items {
        match item {
            Item::Let(l) if l.value.is_none() => {
                global_scope.insert(l.name.as_str().to_string(), LexicalBinding::Uninitialized);
                global_env_scope.insert(l.name.as_str().to_string(), EnvEntry::Tombstone);
            }
            Item::Env(e) if e.value.is_none() => {
                global_scope.insert(e.name.as_str().to_string(), LexicalBinding::Uninitialized);
                global_env_scope.insert(e.name.as_str().to_string(), EnvEntry::Tombstone);
            }
            _ => {}
        }
    }

    // 2. Pre-resolve top-level let and env expressions
    let empty_ports: [String; 0] = [];
    let empty_children: [NodeId; 0] = [];
    let mut changed = true;
    let mut passes = 0;
    while changed && passes < 20 {
        changed = false;
        passes += 1;
        for item in &doc.items {
            match item {
                Item::Let(let_binding) => {
                    if let_binding.value.is_none() {
                        global_scope.insert(let_binding.name.as_str().to_string(), LexicalBinding::Uninitialized);
                    } else if let Some(LetValue::Expr(raw_expr)) = &let_binding.value {
                        let scope_ctx = ScopeContext {
                            current_node: NodeId::WINDOW,
                            parent_node: None,
                            prev_sibling: None,
                            child_ids: &empty_children,
                            parent_ports: &empty_ports,
                            current_ports: &window_scope_ports,
                            lexical_scope: &global_scope,
                            env_scope: &global_env_scope,
                            node_fonts: &node_fonts,
                        };
                        let rewritten = rewrite_expr(raw_expr, &scope_ctx)?;
                        if let Some(LexicalBinding::Expr(existing)) = global_scope.get(let_binding.name.as_str()) {
                            if existing != &rewritten {
                                global_scope.insert(let_binding.name.as_str().to_string(), LexicalBinding::Expr(rewritten));
                                changed = true;
                            }
                        } else {
                            global_scope.insert(let_binding.name.as_str().to_string(), LexicalBinding::Expr(rewritten));
                            changed = true;
                        }
                    }
                }
                Item::Env(env_binding) => {
                    if env_binding.value.is_none() {
                        global_scope.insert(env_binding.name.as_str().to_string(), LexicalBinding::Uninitialized);
                        global_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Tombstone);
                    } else if let Some(raw_expr) = &env_binding.value {
                        if !matches!(raw_expr, Expr::Node(_)) {
                            let scope_ctx = ScopeContext {
                                current_node: NodeId::WINDOW,
                                parent_node: None,
                                prev_sibling: None,
                                child_ids: &empty_children,
                                parent_ports: &empty_ports,
                                current_ports: &window_scope_ports,
                                lexical_scope: &global_scope,
                                env_scope: &global_env_scope,
                                node_fonts: &node_fonts,
                            };
                            let rewritten = rewrite_expr(raw_expr, &scope_ctx)?;
                            if let Some(LexicalBinding::Expr(existing)) = global_scope.get(env_binding.name.as_str()) {
                                if existing != &rewritten {
                                    global_scope.insert(env_binding.name.as_str().to_string(), LexicalBinding::Expr(rewritten.clone()));
                                    global_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Bound(rewritten));
                                    changed = true;
                                }
                            } else {
                                global_scope.insert(env_binding.name.as_str().to_string(), LexicalBinding::Expr(rewritten.clone()));
                                global_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Bound(rewritten));
                                changed = true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // 3. Expand root elements and top-level node bindings in document order
    let mut last_root_id = None;

    for item in &doc.items {
        match item {
            Item::Component(_) => {}
            Item::Env(env_binding) => {
                if let Some(Expr::Node(elem)) = &env_binding.value {
                    let elem_ctx = ElementContext {
                        parent_id: Some(NodeId::WINDOW),
                        prev_sibling_id: last_root_id,
                        parent_ports: &window_scope_ports,
                        lexical_scope: &global_scope,
                        env_scope: &global_env_scope,
                        enclosing_component_id: None,
                        is_let: true,
                        ambient_authored_ports: None,
                    };
                    let root_id = expand_element(elem, &elem_ctx, &registry, &mut expanded_doc, &mut node_fonts)?;
                    expanded_doc.get_node_mut(root_id).unwrap().var_name = Some(env_binding.name.as_str().to_string());
                    expanded_doc.roots.push(root_id);
                    last_root_id = Some(root_id);
                    global_scope.insert(
                        env_binding.name.as_str().to_string(),
                        LexicalBinding::Node(root_id),
                    );
                    let bound_expr = Expr::Ident(Ident::new(root_id.canonical_name(), elem.span));
                    global_env_scope.insert(
                        env_binding.name.as_str().to_string(),
                        EnvEntry::Bound(bound_expr),
                    );
                    if elem.name.as_str() == "Font" {
                        node_fonts.insert(root_id, root_id);
                    }
                } else if let Some(raw_expr) = &env_binding.value {
                    let scope_ctx = ScopeContext {
                        current_node: NodeId::WINDOW,
                        parent_node: None,
                        prev_sibling: None,
                        child_ids: &empty_children,
                        parent_ports: &empty_ports,
                        current_ports: &window_scope_ports,
                        lexical_scope: &global_scope,
                        env_scope: &global_env_scope,
                        node_fonts: &node_fonts,
                    };
                    if let Ok(rewritten) = rewrite_expr(raw_expr, &scope_ctx) {
                        global_scope.insert(
                            env_binding.name.as_str().to_string(),
                            LexicalBinding::Expr(rewritten.clone()),
                        );
                        global_env_scope.insert(
                            env_binding.name.as_str().to_string(),
                            EnvEntry::Bound(rewritten),
                        );
                    }
                }
            }
            Item::Node(node) => {
                let elem_ctx = ElementContext {
                    parent_id: Some(NodeId::WINDOW),
                    prev_sibling_id: last_root_id,
                    parent_ports: &window_scope_ports,
                    lexical_scope: &global_scope,
                    env_scope: &global_env_scope,
                    enclosing_component_id: None,
                    is_let: false,
                    ambient_authored_ports: None,
                };
                let root_id = expand_element(node, &elem_ctx, &registry, &mut expanded_doc, &mut node_fonts)?;
                expanded_doc.roots.push(root_id);
                last_root_id = Some(root_id);
            }
            Item::Let(let_binding) => match &let_binding.value {
                Some(LetValue::Expr(_)) | None => {} // Already resolved in pre-pass
                Some(LetValue::Node(elem)) => {
                    let elem_ctx = ElementContext {
                        parent_id: Some(NodeId::WINDOW),
                        prev_sibling_id: last_root_id,
                        parent_ports: &window_scope_ports,
                        lexical_scope: &global_scope,
                        env_scope: &global_env_scope,
                        enclosing_component_id: None,
                        is_let: true,
                        ambient_authored_ports: None,
                    };
                    let root_id = expand_element(elem, &elem_ctx, &registry, &mut expanded_doc, &mut node_fonts)?;
                    expanded_doc.get_node_mut(root_id).unwrap().var_name = Some(let_binding.name.as_str().to_string());
                    expanded_doc.roots.push(root_id);
                    last_root_id = Some(root_id);
                    global_scope.insert(
                        let_binding.name.as_str().to_string(),
                        LexicalBinding::Node(root_id),
                    );
                    if elem.name.as_str() == "Font" {
                        node_fonts.insert(root_id, root_id);
                    }
                }
            }
            Item::State(_) => {}
        }
    }

    Ok(expanded_doc)
}

struct InstanceContext<'a> {
    pub comp_node_id: NodeId,
    pub parent_id: Option<NodeId>,
    pub prev_sibling_id: Option<NodeId>,
    pub parent_ports: &'a [String],
    pub ambient_authored_ports: Option<&'a HashMap<String, Expr>>,
}

struct ElementContext<'a> {
    pub parent_id: Option<NodeId>,
    pub prev_sibling_id: Option<NodeId>,
    pub parent_ports: &'a [String],
    pub lexical_scope: &'a HashMap<String, LexicalBinding>,
    pub env_scope: &'a HashMap<String, EnvEntry>,
    pub enclosing_component_id: Option<NodeId>,
    pub is_let: bool,
    pub ambient_authored_ports: Option<&'a HashMap<String, Expr>>,
}

/// Expands a single element node (either a component invocation or a primitive).
fn expand_element(
    elem: &ElementNode,
    ctx: &ElementContext<'_>,
    registry: &HashMap<String, ComponentDef>,
    doc: &mut ExpandedDocument,
    node_fonts: &mut HashMap<NodeId, NodeId>,
) -> Result<NodeId, CompileError> {
    for port in &elem.ports {
        if port.name.as_str() == "parent" {
            return Err(CompileError::ReservedPort {
                node: elem.name.as_str().to_string(),
                port: "parent".to_string(),
                span: port.name.span,
            });
        }
    }

    let node_id = NodeId(doc.nodes.len());
    let mut expanded = ExpandedNode::new(node_id, elem.name.as_str(), elem.span);
    expanded.handle = elem.handle;
    expanded.parent = ctx.parent_id;
    expanded.prev_sibling = ctx.prev_sibling_id;

    // Check if element has text content directly in content slot
    if let Some(content_slot) = &elem.content {
        let mut text_parts = Vec::new();
        for item in &content_slot.items {
            if let ContentItem::Text(chunk) = item {
                text_parts.push(chunk.text.as_str());
            }
        }
        if !text_parts.is_empty() {
            expanded.text_content = Some(text_parts.join(" "));
        }
    }

    // Allocate placeholder in doc.nodes to reserve index
    doc.nodes.push(expanded);

    // If it's an invocation of a user-defined component:
    if let Some(comp_def) = registry.get(elem.name.as_str()).cloned() {
        let inst_ctx = InstanceContext {
            comp_node_id: node_id,
            parent_id: ctx.parent_id,
            prev_sibling_id: ctx.prev_sibling_id,
            parent_ports: ctx.parent_ports,
            ambient_authored_ports: ctx.ambient_authored_ports,
        };
        expand_component_instance(
            &inst_ctx,
            elem,
            &comp_def,
            registry,
            doc,
            ctx.lexical_scope,
            ctx.env_scope,
            node_fonts,
        )?;
    } else {
        // It's a primitive element (e.g. \Rect, \Text, \Header, etc.)
        expand_primitive_element(
            node_id,
            elem,
            ctx,
            registry,
            doc,
            node_fonts,
        )?;
    }

    Ok(node_id)
}

/// Expands a component instance, wiring parameters, component body, and consumer children.
fn expand_component_instance(
    ctx: &InstanceContext<'_>,
    instance: &ElementNode,
    comp_def: &ComponentDef,
    registry: &HashMap<String, ComponentDef>,
    doc: &mut ExpandedDocument,
    lexical_scope: &HashMap<String, LexicalBinding>,
    caller_env_scope: &HashMap<String, EnvEntry>,
    node_fonts: &mut HashMap<NodeId, NodeId>,
) -> Result<(), CompileError> {
    // 1. Gather parameter definitions and map consumer arguments
    let mut comp_scope_ports = vec![
        "x".to_string(),
        "y".to_string(),
        "width".to_string(),
        "height".to_string(),
        "z".to_string(),
        "clip".to_string(),
        "left".to_string(),
        "top".to_string(),
        "right".to_string(),
        "bottom".to_string(),
    ];
    let mut declared_aliases: HashMap<String, AliasBinding> = HashMap::new();
    for item in &comp_def.body {
        if let ComponentBodyItem::Alias(a) = item {
            let alias_name = a.name.as_str().to_string();
            if alias_name == "parent" {
                return Err(CompileError::ReservedPort {
                    node: comp_def.name.as_str().to_string(),
                    port: "parent".to_string(),
                    span: a.name.span,
                });
            }
            if comp_def.params.iter().any(|p| p.name.as_str() == alias_name) {
                return Err(CompileError::DuplicatePort {
                    node: comp_def.name.as_str().to_string(),
                    port: alias_name,
                    span: a.name.span,
                });
            }
            if declared_aliases.contains_key(&alias_name) {
                return Err(CompileError::DuplicatePort {
                    node: comp_def.name.as_str().to_string(),
                    port: alias_name,
                    span: a.name.span,
                });
            }
            if !comp_scope_ports.contains(&alias_name) {
                comp_scope_ports.push(alias_name.clone());
            }
            declared_aliases.insert(alias_name, a.clone());
        }
    }

    // Validate that caller does not attempt to override immutable alias ports
    for port in &instance.ports {
        let name = port.name.as_str();
        if declared_aliases.contains_key(name) || matches!(name, "left" | "top" | "right" | "bottom") {
            return Err(CompileError::ImmutableAliasPort {
                node: comp_def.name.as_str().to_string(),
                port: name.to_string(),
                span: port.name.span,
            });
        }
    }

    let mut comp_ports = HashMap::new();

    // Map explicit arguments passed to the component (Tier 4)
    let mut explicit_ports = HashMap::new();
    let empty_ports: [String; 0] = [];
    let empty_children: [NodeId; 0] = [];
    for port in &instance.ports {
        let name = port.name.as_str().to_string();
        let expr = match &port.expr {
            Expr::Node(inline_elem) => {
                let child_ctx = ElementContext {
                    parent_id: Some(ctx.comp_node_id),
                    prev_sibling_id: ctx.prev_sibling_id,
                    parent_ports: ctx.parent_ports,
                    lexical_scope,
                    env_scope: caller_env_scope,
                    enclosing_component_id: None,
                    is_let: false,
                    ambient_authored_ports: None,
                };
                let child_id = expand_element(inline_elem, &child_ctx, registry, doc, node_fonts)?;
                Expr::Ident(Ident::new(child_id.canonical_name(), inline_elem.span))
            }
            other => {
                let caller_scope_ctx = ScopeContext {
                    current_node: ctx.comp_node_id,
                    parent_node: ctx.parent_id,
                    prev_sibling: ctx.prev_sibling_id,
                    child_ids: &empty_children,
                    parent_ports: ctx.parent_ports,
                    current_ports: &empty_ports,
                    lexical_scope,
                    env_scope: caller_env_scope,
                    node_fonts,
                };
                rewrite_expr(other, &caller_scope_ctx)?
            }
        };
        explicit_ports.insert(name, expr);
    }

    // Resolve parameters according to 4-Tier Precedence
    for param in &comp_def.params {
        if param.name.as_str() == "parent" {
            return Err(CompileError::ReservedPort {
                node: comp_def.name.as_str().to_string(),
                port: "parent".to_string(),
                span: param.name.span,
            });
        }
        let name = param.name.as_str().to_string();
        if !comp_scope_ports.contains(&name) {
            comp_scope_ports.push(name.clone());
        }

        // Tier 4: Explicit caller argument
        if let Some(explicit_expr) = explicit_ports.remove(&name) {
            comp_ports.insert(name, explicit_expr);
        }
        // Tier 2: Lexical Environment (caller_env_scope) - only for env parameters
        else if param.is_env {
            if let Some(env_entry) = caller_env_scope.get(&name) {
                match env_entry {
                    EnvEntry::Bound(env_expr) => {
                        comp_ports.insert(name, env_expr.clone());
                    }
                    EnvEntry::Tombstone => {
                        // Tombstone halts lookup! Fall back to Tier 1 default
                        if let Some(default_expr) = &param.default_edge {
                            comp_ports.insert(name, default_expr.clone());
                        }
                    }
                }
            } else if let Some(default_expr) = &param.default_edge {
                comp_ports.insert(name, default_expr.clone());
            }
        }
        // Tier 1: Component Signature Default (for non-env parameters)
        else if let Some(default_expr) = &param.default_edge {
            comp_ports.insert(name, default_expr.clone());
        }
    }

    // Any remaining explicit ports not defined in signature (e.g. ad-hoc ports)
    for (name, expr) in explicit_ports {
        if !comp_scope_ports.contains(&name) {
            comp_scope_ports.push(name.clone());
        }
        comp_ports.insert(name, expr);
    }

    // Record component font if resolved
    if let Some(font_expr) = comp_ports.get("font") {
        if let Expr::Ident(id) = font_expr {
            if let Some(fid) = NodeId::from_canonical_name(id.as_str()) {
                node_fonts.insert(ctx.comp_node_id, fid);
                doc.get_node_mut(ctx.comp_node_id).unwrap().font = Some(fid);
            }
        }
    }

    // Default clip port if not explicitly declared
    if !comp_ports.contains_key("clip") {
        if let Some(EnvEntry::Bound(clip_expr)) = caller_env_scope.get("clip") {
            comp_ports.insert("clip".to_string(), clip_expr.clone());
        } else {
            let default_clip_expr = if let Some(parent) = ctx.parent_id {
                if parent.is_window() {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(NodeId::WINDOW.canonical_name(), instance.span))),
                        member: Ident::new("clip", instance.span),
                        span: instance.span,
                    })
                } else {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(parent.canonical_name(), instance.span))),
                        member: Ident::new("clip", instance.span),
                        span: instance.span,
                    })
                }
            } else {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(NodeId::WINDOW.canonical_name(), instance.span))),
                    member: Ident::new("clip", instance.span),
                    span: instance.span,
                })
            };
            comp_ports.insert("clip".to_string(), default_clip_expr);
        }
    }

    // Validate that all required parameters (parameters without defaults) have been supplied
    for param in &comp_def.params {
        let name = param.name.as_str();
        if param.default_edge.is_none() && !comp_ports.contains_key(name) {
            return Err(CompileError::MissingPort {
                node: comp_def.name.as_str().to_string(),
                port: name.to_string(),
                span: instance.span,
            });
        }
    }

    // 2. Expand consumer children passed to this component instance
    let mut consumer_child_nodes = Vec::new();
    if let Some(slot) = &instance.content {
        for item in &slot.items {
            match item {
                ContentItem::Node(child_elem) => {
                    consumer_child_nodes.push(child_elem.clone());
                }
                ContentItem::Text(_) | ContentItem::Children(_) => {}
            }
        }
    }

    // 3. Expand component body items in declaration order (Painter's Algorithm)
    let mut all_children_ids = Vec::new();
    let mut instantiated_children_ids = Vec::new();
    let mut last_child_id: Option<NodeId> = None;
    let mut local_scope: HashMap<String, LexicalBinding> = lexical_scope.clone();

    // Setup internal_body_env_scope (sealed black box for internal elements)
    let mut internal_body_env_scope: HashMap<String, EnvEntry> = HashMap::new();
    internal_body_env_scope.insert(
        "clip".to_string(),
        EnvEntry::Bound(Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(Expr::Ident(Ident::new(ctx.comp_node_id.canonical_name(), instance.span))),
            member: Ident::new("clip", instance.span),
            span: instance.span,
        })),
    );
    for param in &comp_def.params {
        if param.is_env {
            internal_body_env_scope.insert(
                param.name.as_str().to_string(),
                EnvEntry::Bound(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(ctx.comp_node_id.canonical_name(), instance.span))),
                    member: param.name.clone(),
                    span: instance.span,
                })),
            );
        }
    }

    // Setup children_env_scope (for \Children)
    let mut children_env_scope = caller_env_scope.clone();
    children_env_scope.insert(
        "clip".to_string(),
        EnvEntry::Bound(Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(Expr::Ident(Ident::new(ctx.comp_node_id.canonical_name(), instance.span))),
            member: Ident::new("clip", instance.span),
            span: instance.span,
        })),
    );
    for param in &comp_def.params {
        if param.is_env {
            children_env_scope.insert(
                param.name.as_str().to_string(),
                EnvEntry::Bound(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(ctx.comp_node_id.canonical_name(), instance.span))),
                    member: param.name.clone(),
                    span: instance.span,
                })),
            );
        }
    }

    // Pre-register uninitialized let and env in component body
    for item in &comp_def.body {
        match item {
            ComponentBodyItem::Let(l) => {
                let is_env_param_shadow = comp_def
                    .params
                    .iter()
                    .any(|p| p.is_env && p.name.as_str() == l.name.as_str());
                if l.value.is_none() || is_env_param_shadow {
                    children_env_scope.insert(l.name.as_str().to_string(), EnvEntry::Tombstone);
                    internal_body_env_scope.insert(l.name.as_str().to_string(), EnvEntry::Tombstone);
                }
                if l.value.is_none() {
                    local_scope.insert(l.name.as_str().to_string(), LexicalBinding::Uninitialized);
                }
            }
            ComponentBodyItem::Env(e) => {
                if e.value.is_none() {
                    local_scope.insert(e.name.as_str().to_string(), LexicalBinding::Uninitialized);
                    children_env_scope.insert(e.name.as_str().to_string(), EnvEntry::Tombstone);
                    internal_body_env_scope.insert(e.name.as_str().to_string(), EnvEntry::Tombstone);
                }
            }
            _ => {}
        }
    }

    // Pre-resolve let and env expressions in component body so their declaration order does not matter
    let empty_children: [NodeId; 0] = [];
    let mut changed = true;
    let mut passes = 0;
    while changed && passes < 20 {
        changed = false;
        passes += 1;
        for item in &comp_def.body {
            match item {
                ComponentBodyItem::Let(let_binding) => {
                    if let_binding.value.is_none() {
                        local_scope.insert(let_binding.name.as_str().to_string(), LexicalBinding::Uninitialized);
                    } else if let Some(LetValue::Expr(raw_expr)) = &let_binding.value {
                        let scope_ctx = ScopeContext {
                            current_node: ctx.comp_node_id,
                            parent_node: ctx.parent_id,
                            prev_sibling: ctx.prev_sibling_id,
                            child_ids: &empty_children,
                            parent_ports: ctx.parent_ports,
                            current_ports: &comp_scope_ports,
                            lexical_scope: &local_scope,
                            env_scope: &internal_body_env_scope,
                            node_fonts: &*node_fonts,
                        };
                        let rewritten = rewrite_expr(raw_expr, &scope_ctx)?;
                        if let Some(LexicalBinding::Expr(existing)) = local_scope.get(let_binding.name.as_str()) {
                            if existing != &rewritten {
                                local_scope.insert(let_binding.name.as_str().to_string(), LexicalBinding::Expr(rewritten));
                                changed = true;
                            }
                        } else {
                            local_scope.insert(let_binding.name.as_str().to_string(), LexicalBinding::Expr(rewritten));
                            changed = true;
                        }
                    }
                }
                ComponentBodyItem::Env(env_binding) => {
                    if env_binding.value.is_none() {
                        local_scope.insert(env_binding.name.as_str().to_string(), LexicalBinding::Uninitialized);
                    } else if let Some(raw_expr) = &env_binding.value {
                        if !matches!(raw_expr, Expr::Node(_)) {
                            let scope_ctx = ScopeContext {
                                current_node: ctx.comp_node_id,
                                parent_node: ctx.parent_id,
                                prev_sibling: ctx.prev_sibling_id,
                                child_ids: &empty_children,
                                parent_ports: ctx.parent_ports,
                                current_ports: &comp_scope_ports,
                                lexical_scope: &local_scope,
                                env_scope: &internal_body_env_scope,
                                node_fonts: &*node_fonts,
                            };
                            let rewritten = rewrite_expr(raw_expr, &scope_ctx)?;
                            if let Some(LexicalBinding::Expr(existing)) = local_scope.get(env_binding.name.as_str()) {
                                if existing != &rewritten {
                                    local_scope.insert(env_binding.name.as_str().to_string(), LexicalBinding::Expr(rewritten));
                                    changed = true;
                                }
                            } else {
                                local_scope.insert(env_binding.name.as_str().to_string(), LexicalBinding::Expr(rewritten));
                                changed = true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Process body items in declaration order
    for item in &comp_def.body {
        match item {
            ComponentBodyItem::Let(let_binding) => {
                let is_env_param_shadow = comp_def
                    .params
                    .iter()
                    .any(|p| p.is_env && p.name.as_str() == let_binding.name.as_str());

                if let_binding.value.is_none() || is_env_param_shadow {
                    // Shield/Swallower pattern or Firewall Tombstone
                    children_env_scope.insert(let_binding.name.as_str().to_string(), EnvEntry::Tombstone);
                    internal_body_env_scope.insert(let_binding.name.as_str().to_string(), EnvEntry::Tombstone);
                }

                if let Some(LetValue::Node(elem)) = &let_binding.value {
                    let elem_ctx = ElementContext {
                        parent_id: Some(ctx.comp_node_id),
                        prev_sibling_id: None,
                        parent_ports: &comp_scope_ports,
                        lexical_scope: &local_scope,
                        env_scope: &internal_body_env_scope,
                        enclosing_component_id: Some(ctx.comp_node_id),
                        is_let: true,
                        ambient_authored_ports: None,
                    };
                    let node_id = expand_element(
                        elem,
                        &elem_ctx,
                        registry,
                        doc,
                        node_fonts,
                    )?;
                    doc.get_node_mut(node_id).unwrap().var_name = Some(let_binding.name.as_str().to_string());
                    all_children_ids.push(node_id);
                    local_scope.insert(
                        let_binding.name.as_str().to_string(),
                        LexicalBinding::Node(node_id),
                    );
                    if elem.name.as_str() == "Font" {
                        node_fonts.insert(node_id, node_id);
                    }
                }
            }
            ComponentBodyItem::Env(env_binding) => {
                if let Some(raw_expr) = &env_binding.value {
                    match raw_expr {
                        Expr::Node(elem) => {
                            let elem_ctx = ElementContext {
                                parent_id: Some(ctx.comp_node_id),
                                prev_sibling_id: None,
                                parent_ports: &comp_scope_ports,
                                lexical_scope: &local_scope,
                                env_scope: &internal_body_env_scope,
                                enclosing_component_id: Some(ctx.comp_node_id),
                                is_let: true,
                                ambient_authored_ports: None,
                            };
                            let node_id = expand_element(
                                elem,
                                &elem_ctx,
                                registry,
                                doc,
                                node_fonts,
                            )?;
                            doc.get_node_mut(node_id).unwrap().var_name = Some(env_binding.name.as_str().to_string());
                            all_children_ids.push(node_id);
                            local_scope.insert(
                                env_binding.name.as_str().to_string(),
                                LexicalBinding::Node(node_id),
                            );
                            let bound_expr = Expr::Ident(Ident::new(node_id.canonical_name(), elem.span));
                            children_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Bound(bound_expr.clone()));
                            internal_body_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Bound(bound_expr));
                            if elem.name.as_str() == "Font" {
                                node_fonts.insert(node_id, node_id);
                            }
                        }
                        _ => {
                            let scope_ctx = ScopeContext {
                                current_node: ctx.comp_node_id,
                                parent_node: ctx.parent_id,
                                prev_sibling: ctx.prev_sibling_id,
                                child_ids: &empty_children,
                                parent_ports: ctx.parent_ports,
                                current_ports: &comp_scope_ports,
                                lexical_scope: &local_scope,
                                env_scope: &internal_body_env_scope,
                                node_fonts: &*node_fonts,
                            };
                            let rewritten = rewrite_expr(raw_expr, &scope_ctx)?;
                            local_scope.insert(
                                env_binding.name.as_str().to_string(),
                                LexicalBinding::Expr(rewritten.clone()),
                            );
                            children_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Bound(rewritten.clone()));
                            internal_body_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Bound(rewritten));
                        }
                    }
                } else {
                    // Environmental Hole Tombstone
                    local_scope.insert(env_binding.name.as_str().to_string(), LexicalBinding::Uninitialized);
                    children_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Tombstone);
                    internal_body_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Tombstone);
                }
            }
            ComponentBodyItem::Node(body_node) => {
                let elem_ctx = ElementContext {
                    parent_id: Some(ctx.comp_node_id),
                    prev_sibling_id: last_child_id,
                    parent_ports: &comp_scope_ports,
                    lexical_scope: &local_scope,
                    env_scope: &internal_body_env_scope,
                    enclosing_component_id: Some(ctx.comp_node_id),
                    is_let: false,
                    ambient_authored_ports: None,
                };
                let body_id = expand_element(
                    body_node,
                    &elem_ctx,
                    registry,
                    doc,
                    node_fonts,
                )?;
                all_children_ids.push(body_id);
                last_child_id = Some(body_id);
            }
            ComponentBodyItem::Children(dir) => {
                for port in &dir.ports {
                    if port.name.as_str() == "parent" {
                        return Err(CompileError::ReservedPort {
                            node: "Children".to_string(),
                            port: "parent".to_string(),
                            span: port.name.span,
                        });
                    }
                }
                let mut last_consumer_child_id = None;
                for child_elem in &consumer_child_nodes {
                    let mut merged_ports = HashMap::new();
                    let mut authored_ambient_ports = HashMap::new();

                    // Ambient rules from \Children are authored inside the component (Tier 3)
                    let empty_children: [NodeId; 0] = [];
                    let ambient_scope_ctx = ScopeContext {
                        current_node: ctx.comp_node_id,
                        parent_node: Some(ctx.comp_node_id),
                        prev_sibling: last_consumer_child_id,
                        child_ids: &empty_children,
                        parent_ports: &comp_scope_ports,
                        current_ports: &comp_scope_ports,
                        lexical_scope: &local_scope,
                        env_scope: &children_env_scope,
                        node_fonts: &*node_fonts,
                    };

                    for ambient in &dir.ports {
                        let rewritten = rewrite_expr(&ambient.expr, &ambient_scope_ctx)?;
                        merged_ports.insert(ambient.name.as_str().to_string(), rewritten);
                        authored_ambient_ports.insert(ambient.name.as_str().to_string(), ambient.expr.clone());
                    }

                    // Explicit child ports override ambient rules (Tier 4 > Tier 3)
                    for explicit in &child_elem.ports {
                        merged_ports.insert(explicit.name.as_str().to_string(), explicit.expr.clone());
                        authored_ambient_ports.remove(explicit.name.as_str());
                    }

                    let mut wired_elem = child_elem.clone();
                    wired_elem.ports = merged_ports
                        .into_iter()
                        .map(|(name, expr)| PortBinding {
                            name: Ident::new(name, child_elem.span),
                            expr,
                            span: child_elem.span,
                        })
                        .collect();

                    let elem_ctx = ElementContext {
                        parent_id: Some(ctx.comp_node_id),
                        prev_sibling_id: last_consumer_child_id,
                        parent_ports: &comp_scope_ports,
                        lexical_scope, // Pass caller's lexical scope to preserve encapsulation
                        env_scope: &children_env_scope,
                        enclosing_component_id: Some(ctx.comp_node_id),
                        is_let: false,
                        ambient_authored_ports: Some(&authored_ambient_ports),
                    };
                    let child_id = expand_element(
                        &wired_elem,
                        &elem_ctx,
                        registry,
                        doc,
                        node_fonts,
                    )?;

                    instantiated_children_ids.push(child_id);
                    all_children_ids.push(child_id);
                    last_consumer_child_id = Some(child_id);
                }
            }
            ComponentBodyItem::Alias(_) => {}
            ComponentBodyItem::State(_) => {}
        }
    }

    // 6. Rewrite component ports in scope
    let empty_ports: [String; 0] = [];
    let scope_ctx = ScopeContext {
        current_node: ctx.comp_node_id,
        parent_node: ctx.parent_id,
        prev_sibling: ctx.prev_sibling_id,
        child_ids: &instantiated_children_ids,
        parent_ports: ctx.parent_ports,
        current_ports: &empty_ports,
        lexical_scope: &local_scope,
        env_scope: &internal_body_env_scope,
        node_fonts: &*node_fonts,
    };

    let mut rewritten_ports = HashMap::new();
    for (port_name, port_expr) in comp_ports {
        rewritten_ports.insert(port_name, rewrite_expr(&port_expr, &scope_ctx)?);
    }

    let mut comp_authored_ports = HashMap::new();
    if let Some(ambient_map) = ctx.ambient_authored_ports {
        for (k, v) in ambient_map {
            comp_authored_ports.insert(k.clone(), v.clone());
        }
    }
    for param in &comp_def.params {
        let name = param.name.as_str().to_string();
        if let Some(default_expr) = &param.default_edge {
            comp_authored_ports.entry(name).or_insert_with(|| default_expr.clone());
        }
    }
    for port in &instance.ports {
        if let Some(ambient_expr) = ctx.ambient_authored_ports.and_then(|m| m.get(port.name.as_str())) {
            comp_authored_ports.insert(port.name.as_str().to_string(), ambient_expr.clone());
        } else {
            comp_authored_ports.insert(port.name.as_str().to_string(), port.expr.clone());
        }
    }

    // 7. Process explicitly declared component aliases
    let alias_scope_ctx = ScopeContext {
        current_node: ctx.comp_node_id,
        parent_node: ctx.parent_id,
        prev_sibling: ctx.prev_sibling_id,
        child_ids: &instantiated_children_ids,
        parent_ports: ctx.parent_ports,
        current_ports: &comp_scope_ports,
        lexical_scope: &local_scope,
        env_scope: &internal_body_env_scope,
        node_fonts: &*node_fonts,
    };
    for (alias_name, alias_binding) in &declared_aliases {
        let rewritten_alias = rewrite_expr(&alias_binding.value, &alias_scope_ctx)?;
        rewritten_ports.insert(alias_name.clone(), rewritten_alias);
        comp_authored_ports.insert(alias_name.clone(), alias_binding.value.clone());
    }

    // 8. Default spatial aliases if not explicitly declared
    let self_ident = Expr::Ident(Ident::new(ctx.comp_node_id.canonical_name(), instance.span));
    let self_x = Expr::MemberAccess(MemberAccessExpr {
        target: Box::new(self_ident.clone()),
        member: Ident::new("x", instance.span),
        span: instance.span,
    });
    let self_y = Expr::MemberAccess(MemberAccessExpr {
        target: Box::new(self_ident.clone()),
        member: Ident::new("y", instance.span),
        span: instance.span,
    });
    let self_width = Expr::MemberAccess(MemberAccessExpr {
        target: Box::new(self_ident.clone()),
        member: Ident::new("width", instance.span),
        span: instance.span,
    });
    let self_height = Expr::MemberAccess(MemberAccessExpr {
        target: Box::new(self_ident.clone()),
        member: Ident::new("height", instance.span),
        span: instance.span,
    });

    if !rewritten_ports.contains_key("left") {
        rewritten_ports.insert("left".to_string(), self_x.clone());
        comp_authored_ports.insert("left".to_string(), Expr::Ident(Ident::new("x", instance.span)));
    }
    if !rewritten_ports.contains_key("top") {
        rewritten_ports.insert("top".to_string(), self_y.clone());
        comp_authored_ports.insert("top".to_string(), Expr::Ident(Ident::new("y", instance.span)));
    }
    if !rewritten_ports.contains_key("right") {
        let right_expr = Expr::Binary(BinaryExpr {
            op: BinaryOp::Add,
            left: Box::new(self_x),
            right: Box::new(self_width),
            span: instance.span,
        });
        rewritten_ports.insert("right".to_string(), right_expr);
        comp_authored_ports.insert(
            "right".to_string(),
            Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(Expr::Ident(Ident::new("x", instance.span))),
                right: Box::new(Expr::Ident(Ident::new("width", instance.span))),
                span: instance.span,
            }),
        );
    }
    if !rewritten_ports.contains_key("bottom") {
        let bottom_expr = Expr::Binary(BinaryExpr {
            op: BinaryOp::Add,
            left: Box::new(self_y),
            right: Box::new(self_height),
            span: instance.span,
        });
        rewritten_ports.insert("bottom".to_string(), bottom_expr);
        comp_authored_ports.insert(
            "bottom".to_string(),
            Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(Expr::Ident(Ident::new("y", instance.span))),
                right: Box::new(Expr::Ident(Ident::new("height", instance.span))),
                span: instance.span,
            }),
        );
    }

    let node = doc.get_node_mut(ctx.comp_node_id).unwrap();
    node.children = all_children_ids;
    node.ports = rewritten_ports;
    node.authored_ports = comp_authored_ports;

    Ok(())
}

/// Expands a primitive element (e.g. \Rect, \Text, \Font, \Header, etc.)
fn expand_primitive_element(
    node_id: NodeId,
    elem: &ElementNode,
    ctx: &ElementContext<'_>,
    registry: &HashMap<String, ComponentDef>,
    doc: &mut ExpandedDocument,
    node_fonts: &mut HashMap<NodeId, NodeId>,
) -> Result<(), CompileError> {
    // Validate that caller does not attempt to override immutable spatial alias ports
    for port in &elem.ports {
        if matches!(port.name.as_str(), "left" | "top" | "right" | "bottom") {
            return Err(CompileError::ImmutableAliasPort {
                node: elem.name.as_str().to_string(),
                port: port.name.as_str().to_string(),
                span: port.name.span,
            });
        }
    }

    // 1. Expand nested child nodes in content slot
    let mut child_ids = Vec::new();
    let mut last_child_id = None;

    if let Some(slot) = &elem.content {
        for item in &slot.items {
            if let ContentItem::Node(child_elem) = item {
                let child_ctx = ElementContext {
                    parent_id: Some(node_id),
                    prev_sibling_id: last_child_id,
                    parent_ports: ctx.parent_ports,
                    lexical_scope: ctx.lexical_scope,
                    env_scope: ctx.env_scope,
                    enclosing_component_id: ctx.enclosing_component_id,
                    is_let: false,
                    ambient_authored_ports: None,
                };
                let child_id = expand_element(
                    child_elem,
                    &child_ctx,
                    registry,
                    doc,
                    node_fonts,
                )?;
                child_ids.push(child_id);
                last_child_id = Some(child_id);
            }
        }
    }

    // 1b. Expand any inline node element expressions in ports
    let mut resolved_port_exprs = Vec::with_capacity(elem.ports.len());
    for port in &elem.ports {
        match &port.expr {
            Expr::Node(inline_elem) => {
                let parent_id = if ctx.is_let {
                    ctx.parent_id
                } else {
                    Some(node_id)
                };
                let child_ctx = ElementContext {
                    parent_id,
                    prev_sibling_id: last_child_id,
                    parent_ports: ctx.parent_ports,
                    lexical_scope: ctx.lexical_scope,
                    env_scope: ctx.env_scope,
                    enclosing_component_id: ctx.enclosing_component_id,
                    is_let: ctx.is_let,
                    ambient_authored_ports: None,
                };
                let child_id = expand_element(inline_elem, &child_ctx, registry, doc, node_fonts)?;
                child_ids.push(child_id);
                last_child_id = Some(child_id);
                resolved_port_exprs.push((
                    port.name.as_str().to_string(),
                    Expr::Ident(Ident::new(child_id.canonical_name(), inline_elem.span)),
                ));
            }
            other => {
                resolved_port_exprs.push((port.name.as_str().to_string(), other.clone()));
            }
        }
    }

    // 2. Rewrite element ports
    let empty_ports: [String; 0] = [];
    let current_node = if ctx.is_let {
        ctx.enclosing_component_id.unwrap_or(node_id)
    } else {
        node_id
    };
    let parent_node = if ctx.is_let {
        if let Some(comp_id) = ctx.enclosing_component_id {
            doc.get_node(comp_id).and_then(|n| n.parent)
        } else {
            ctx.parent_id
        }
    } else {
        ctx.parent_id
    };
    let current_ports = if ctx.is_let {
        ctx.parent_ports
    } else {
        &empty_ports
    };

    // First, determine if this element has a font node (explicitly or ambiently)
    let mut font_node_id = None;
    if let Some((_, raw_font_expr)) = resolved_port_exprs.iter().find(|(name, _)| name == "font") {
        let temp_scope_ctx = ScopeContext {
            current_node,
            parent_node,
            prev_sibling: ctx.prev_sibling_id,
            child_ids: &child_ids,
            parent_ports: ctx.parent_ports,
            current_ports,
            lexical_scope: ctx.lexical_scope,
            env_scope: ctx.env_scope,
            node_fonts: &*node_fonts,
        };
        if let Ok(rewritten) = rewrite_expr(raw_font_expr, &temp_scope_ctx) {
            if let Expr::Ident(id) = &rewritten {
                if let Some(fid) = NodeId::from_canonical_name(id.as_str()) {
                    font_node_id = Some(fid);
                }
            }
        }
    } else if let Some(EnvEntry::Bound(env_expr)) = ctx.env_scope.get("font") {
        let temp_scope_ctx = ScopeContext {
            current_node,
            parent_node,
            prev_sibling: ctx.prev_sibling_id,
            child_ids: &child_ids,
            parent_ports: ctx.parent_ports,
            current_ports,
            lexical_scope: ctx.lexical_scope,
            env_scope: ctx.env_scope,
            node_fonts: &*node_fonts,
        };
        if let Ok(rewritten) = rewrite_expr(env_expr, &temp_scope_ctx) {
            if let Expr::Ident(id) = &rewritten {
                if let Some(fid) = NodeId::from_canonical_name(id.as_str()) {
                    font_node_id = Some(fid);
                    resolved_port_exprs.push(("font".to_string(), Expr::Ident(Ident::new(fid.canonical_name(), elem.span))));
                }
            } else if matches!(rewritten, Expr::Literal(Literal::String(_, _))) {
                resolved_port_exprs.push(("font".to_string(), rewritten));
            }
        }
    }

    if let Some(fid) = font_node_id {
        node_fonts.insert(node_id, fid);
        doc.get_node_mut(node_id).unwrap().font = Some(fid);
    } else if elem.name.as_str() == "Font" {
        node_fonts.insert(node_id, node_id);
        doc.get_node_mut(node_id).unwrap().font = Some(node_id);
    }

    let scope_ctx = ScopeContext {
        current_node,
        parent_node,
        prev_sibling: ctx.prev_sibling_id,
        child_ids: &child_ids,
        parent_ports: ctx.parent_ports,
        current_ports,
        lexical_scope: ctx.lexical_scope,
        env_scope: ctx.env_scope,
        node_fonts: &*node_fonts,
    };

    let mut ports = HashMap::new();
    let mut authored_ports = HashMap::new();
    for (port_name, expr) in resolved_port_exprs {
        if let Some(ambient_expr) = ctx.ambient_authored_ports.and_then(|m| m.get(&port_name)) {
            authored_ports.insert(port_name.clone(), ambient_expr.clone());
        } else {
            authored_ports.insert(port_name.clone(), expr.clone());
        }
        let rewritten = rewrite_expr(&expr, &scope_ctx)?;
        ports.insert(port_name, rewritten);
    }

    if elem.name.as_str() == "Font" {
        // Default size to 16.0 if not specified
        if !ports.contains_key("size") {
            ports.insert(
                "size".to_string(),
                Expr::Literal(Literal::Number(16.0, elem.span)),
            );
        }
        // Default weight to 400.0 if not specified
        if !ports.contains_key("weight") {
            ports.insert(
                "weight".to_string(),
                Expr::Literal(Literal::Number(400.0, elem.span)),
            );
        }
        // Default family to empty string if not specified
        if !ports.contains_key("family") {
            if let Some(font_val) = ports.get("font").cloned() {
                ports.insert("family".to_string(), font_val);
            } else {
                ports.insert(
                    "family".to_string(),
                    Expr::Literal(Literal::String(String::new(), elem.span)),
                );
            }
        }
        if !ports.contains_key("font") {
            ports.insert(
                "font".to_string(),
                ports.get("family").cloned().unwrap(),
            );
        }

        let self_ident = Expr::Ident(Ident::new(node_id.canonical_name(), elem.span));
        let size_ref = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("size", elem.span),
            span: elem.span,
        });
        let weight_ref = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("weight", elem.span),
            span: elem.span,
        });
        let family_ref = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("family", elem.span),
            span: elem.span,
        });

        // Add typographic metric ports: cap_height, x_height, descent, ascent, line_height
        ports.entry("cap_height".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("font_cap_height", elem.span),
                args: vec![size_ref.clone(), weight_ref.clone(), family_ref.clone()],
                span: elem.span,
            })
        });
        ports.entry("x_height".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("font_x_height", elem.span),
                args: vec![size_ref.clone(), weight_ref.clone(), family_ref.clone()],
                span: elem.span,
            })
        });
        ports.entry("descent".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("font_descent", elem.span),
                args: vec![size_ref.clone(), weight_ref.clone(), family_ref.clone()],
                span: elem.span,
            })
        });
        ports.entry("ascent".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("font_ascent", elem.span),
                args: vec![size_ref.clone(), weight_ref.clone(), family_ref.clone()],
                span: elem.span,
            })
        });
        ports.entry("line_height".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("font_line_height", elem.span),
                args: vec![size_ref, weight_ref, family_ref],
                span: elem.span,
            })
        });

        // Dummy spatial ports
        ports.entry("x".to_string()).or_insert_with(|| Expr::Literal(Literal::Number(0.0, elem.span)));
        ports.entry("y".to_string()).or_insert_with(|| Expr::Literal(Literal::Number(0.0, elem.span)));
        ports.entry("width".to_string()).or_insert_with(|| Expr::Literal(Literal::Number(0.0, elem.span)));
        ports.entry("height".to_string()).or_insert_with(|| Expr::Literal(Literal::Number(0.0, elem.span)));
        ports.entry("z".to_string()).or_insert_with(|| Expr::Literal(Literal::Number(0.0, elem.span)));

        let node = doc.get_node_mut(node_id).unwrap();
        node.children = child_ids;
        node.ports = ports;
        node.authored_ports = authored_ports;
        node.font = Some(node_id);
        node_fonts.insert(node_id, node_id);
        return Ok(());
    }

    if elem.name.as_str() == "Clip" {
        if !ports.contains_key("box") {
            return Err(CompileError::MissingPort {
                node: "Clip".to_string(),
                port: "box".to_string(),
                span: elem.span,
            });
        }
        if !ports.contains_key("up") {
            let default_up = if let Some(EnvEntry::Bound(clip_expr)) = ctx.env_scope.get("clip") {
                clip_expr.clone()
            } else if let Some(parent) = ctx.parent_id {
                if parent.is_window() {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(NodeId::WINDOW.canonical_name(), elem.span))),
                        member: Ident::new("clip", elem.span),
                        span: elem.span,
                    })
                } else {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(parent.canonical_name(), elem.span))),
                        member: Ident::new("clip", elem.span),
                        span: elem.span,
                    })
                }
            } else {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(NodeId::WINDOW.canonical_name(), elem.span))),
                    member: Ident::new("clip", elem.span),
                    span: elem.span,
                })
            };
            ports.insert("up".to_string(), default_up);
        }
        ports.insert(
            "clip".to_string(),
            Expr::Ident(Ident::new(node_id.canonical_name(), elem.span)),
        );
    } else if !ports.contains_key("clip") {
        let default_clip = if let Some(EnvEntry::Bound(clip_expr)) = ctx.env_scope.get("clip") {
            clip_expr.clone()
        } else if let Some(parent) = ctx.parent_id {
            if parent.is_window() {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(NodeId::WINDOW.canonical_name(), elem.span))),
                    member: Ident::new("clip", elem.span),
                    span: elem.span,
                })
            } else {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(parent.canonical_name(), elem.span))),
                    member: Ident::new("clip", elem.span),
                    span: elem.span,
                })
            }
        } else {
            Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(Expr::Ident(Ident::new(NodeId::WINDOW.canonical_name(), elem.span))),
                member: Ident::new("clip", elem.span),
                span: elem.span,
            })
        };
        ports.insert("clip".to_string(), default_clip);
    }

    if elem.name.as_str() == "Rect" {
        for port in ["x", "y", "width", "height"] {
            if !ports.contains_key(port) {
                return Err(CompileError::MissingPort {
                    node: "Rect".to_string(),
                    port: port.to_string(),
                    span: elem.span,
                });
            }
        }
        if !ports.contains_key("color") && !ports.contains_key("bg_color") {
            return Err(CompileError::MissingPort {
                node: "Rect".to_string(),
                port: "color".to_string(),
                span: elem.span,
            });
        }
    }

    // Base Spatial Trait defaults for height and width
    let text_content = doc
        .get_node(node_id)
        .and_then(|n| n.text_content.clone());
    let has_text = text_content.as_ref().is_some_and(|t| !t.is_empty());

    if let Some(fid) = font_node_id {
        let font_ident = Expr::Ident(Ident::new(fid.canonical_name(), elem.span));
        if !ports.contains_key("size") && !ports.contains_key("font_size") {
            ports.insert(
                "size".to_string(),
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(font_ident.clone()),
                    member: Ident::new("size", elem.span),
                    span: elem.span,
                }),
            );
        }
        if !ports.contains_key("weight") && !ports.contains_key("font_weight") {
            ports.insert(
                "weight".to_string(),
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(font_ident.clone()),
                    member: Ident::new("weight", elem.span),
                    span: elem.span,
                }),
            );
        }
        if !ports.contains_key("family") && !ports.contains_key("font_family") {
            ports.insert(
                "family".to_string(),
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(font_ident.clone()),
                    member: Ident::new("family", elem.span),
                    span: elem.span,
                }),
            );
        }
    }

    let self_ident = Expr::Ident(Ident::new(node_id.canonical_name(), elem.span));
    let size_expr = if ports.contains_key("size") {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("size", elem.span),
            span: elem.span,
        })
    } else if ports.contains_key("font_size") {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("font_size", elem.span),
            span: elem.span,
        })
    } else {
        Expr::Literal(Literal::Number(16.0, elem.span))
    };

    let weight_expr = if ports.contains_key("weight") {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("weight", elem.span),
            span: elem.span,
        })
    } else if ports.contains_key("font_weight") {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("font_weight", elem.span),
            span: elem.span,
        })
    } else {
        Expr::Literal(Literal::Number(400.0, elem.span))
    };

    let font_expr = if ports.contains_key("family") {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("family", elem.span),
            span: elem.span,
        })
    } else if ports.contains_key("font_family") {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("font_family", elem.span),
            span: elem.span,
        })
    } else if ports.contains_key("font") {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("font", elem.span),
            span: elem.span,
        })
    } else {
        Expr::Literal(Literal::String(String::new(), elem.span))
    };

    // Height defaults
    if !ports.contains_key("height") {
        if (elem.name.as_str() == "Text" || ports.contains_key("width")) && has_text {
            // Text wrapping with Parley: height depends on width, size, weight, font
            let width_expr = Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(self_ident.clone()),
                member: Ident::new("width", elem.span),
                span: elem.span,
            });
            let text_str = text_content.clone().unwrap_or_default();
            let height_call = Expr::Call(CallExpr {
                callee: Ident::new("text_height", elem.span),
                args: vec![
                    Expr::Literal(Literal::String(text_str, elem.span)),
                    size_expr.clone(),
                    weight_expr.clone(),
                    font_expr.clone(),
                    width_expr,
                ],
                span: elem.span,
            });
            ports.insert("height".to_string(), height_call);
        } else if ports.contains_key("size") {
            // E.g. \Header(size: 32) -> height: self.size
            ports.insert(
                "height".to_string(),
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("size", elem.span),
                    span: elem.span,
                }),
            );
        } else {
            ports.insert(
                "height".to_string(),
                Expr::Literal(Literal::Number(24.0, elem.span)),
            );
        }
    }

    // Width defaults
    if !ports.contains_key("width") {
        if elem.name.as_str() == "Text" && has_text {
            let text_str = text_content.clone().unwrap_or_default();
            let width_call = Expr::Call(CallExpr {
                callee: Ident::new("text_width", elem.span),
                args: vec![
                    Expr::Literal(Literal::String(text_str, elem.span)),
                    size_expr,
                    weight_expr,
                    font_expr,
                ],
                span: elem.span,
            });
            ports.insert("width".to_string(), width_call);
        } else {
            let default_w = if has_text {
                (text_content.as_ref().unwrap().len() as f64 * 8.0).max(100.0)
            } else {
                100.0
            };
            ports.insert(
                "width".to_string(),
                Expr::Literal(Literal::Number(default_w, elem.span)),
            );
        }
    }

    // Default spatial alias ports for primitives
    let self_ident = Expr::Ident(Ident::new(node_id.canonical_name(), elem.span));
    if !ports.contains_key("left") {
        ports.insert(
            "left".to_string(),
            Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(self_ident.clone()),
                member: Ident::new("x", elem.span),
                span: elem.span,
            }),
        );
        authored_ports.insert("left".to_string(), Expr::Ident(Ident::new("x", elem.span)));
    }
    if !ports.contains_key("top") {
        ports.insert(
            "top".to_string(),
            Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(self_ident.clone()),
                member: Ident::new("y", elem.span),
                span: elem.span,
            }),
        );
        authored_ports.insert("top".to_string(), Expr::Ident(Ident::new("y", elem.span)));
    }
    if !ports.contains_key("right") {
        ports.insert(
            "right".to_string(),
            Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("x", elem.span),
                    span: elem.span,
                })),
                right: Box::new(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("width", elem.span),
                    span: elem.span,
                })),
                span: elem.span,
            }),
        );
        authored_ports.insert(
            "right".to_string(),
            Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(Expr::Ident(Ident::new("x", elem.span))),
                right: Box::new(Expr::Ident(Ident::new("width", elem.span))),
                span: elem.span,
            }),
        );
    }
    if !ports.contains_key("bottom") {
        ports.insert(
            "bottom".to_string(),
            Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("y", elem.span),
                    span: elem.span,
                })),
                right: Box::new(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident),
                    member: Ident::new("height", elem.span),
                    span: elem.span,
                })),
                span: elem.span,
            }),
        );
        authored_ports.insert(
            "bottom".to_string(),
            Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(Expr::Ident(Ident::new("y", elem.span))),
                right: Box::new(Expr::Ident(Ident::new("height", elem.span))),
                span: elem.span,
            }),
        );
    }

    let node = doc.get_node_mut(node_id).unwrap();
    node.children = child_ids;
    node.ports = ports;
    node.authored_ports = authored_ports;

    Ok(())
}

/// Recursively rewrites an expression into canonical node variable references and resolves derived aliases.
pub fn rewrite_expr(expr: &Expr, ctx: &ScopeContext<'_>) -> Result<Expr, CompileError> {
    match expr {
        Expr::Ident(id) => {
            if id.as_str() == "env" {
                return Err(CompileError::BareEnvUse { span: id.span });
            }

            // Check lexical scope first (let bindings shadow ambient parent ports)
            if id.as_str() != "self"
                && id.as_str() != "parent"
                && id.as_str() != "window"
                && id.as_str() != "prev"
            {
                if let Some(binding) = ctx.lexical_scope.get(id.as_str()) {
                    match binding {
                        LexicalBinding::Expr(e) => return Ok(e.clone()),
                        LexicalBinding::Node(node_id) => {
                            return Ok(Expr::Ident(Ident::new(node_id.canonical_name(), id.span)));
                        }
                        LexicalBinding::Uninitialized => {
                            return Err(CompileError::UninitializedVariableUse {
                                name: id.as_str().to_string(),
                                span: id.span,
                            });
                        }
                    }
                }
            }

            // Check if it's a port on the current node (e.g. component parameter in let expression)
            if ctx.current_ports.iter().any(|p| p == id.as_str()) {
                return Ok(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(ctx.current_node.canonical_name(), id.span))),
                    member: id.clone(),
                    span: id.span,
                }));
            }

            // Check if it's a port on the parent container in scope
            if ctx.parent_ports.iter().any(|p| p == id.as_str()) {
                if let Some(parent) = ctx.parent_node {
                    return Ok(Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(parent.canonical_name(), id.span))),
                        member: id.clone(),
                        span: id.span,
                    }));
                }
            }
            if id.as_str() == "window" {
                return Ok(Expr::Ident(Ident::new(NodeId::WINDOW.canonical_name(), id.span)));
            }
            if id.as_str() == "self" {
                return Ok(Expr::Ident(Ident::new(ctx.current_node.canonical_name(), id.span)));
            }
            if id.as_str() == "parent" {
                if let Some(parent) = ctx.parent_node {
                    return Ok(Expr::Ident(Ident::new(parent.canonical_name(), id.span)));
                }
            }
            if id.as_str() == "prev" {
                if let Some(prev) = ctx.prev_sibling {
                    return Ok(Expr::Ident(Ident::new(prev.canonical_name(), id.span)));
                }
            }
            Ok(expr.clone())
        }

        Expr::Ternary(tern) => {
            // Recurrence resolution for `prev ? then_expr : else_expr`
            if let Expr::Ident(id) = tern.condition.as_ref() {
                if id.as_str() == "prev" {
                    if ctx.prev_sibling.is_none() {
                        // Base case: prev does not exist -> else_expr
                        return rewrite_expr(&tern.else_expr, ctx);
                    } else {
                        // Recurrence step: prev exists -> then_expr
                        return rewrite_expr(&tern.then_expr, ctx);
                    }
                }
            }
            Ok(Expr::Ternary(TernaryExpr {
                condition: Box::new(rewrite_expr(&tern.condition, ctx)?),
                then_expr: Box::new(rewrite_expr(&tern.then_expr, ctx)?),
                else_expr: Box::new(rewrite_expr(&tern.else_expr, ctx)?),
                span: tern.span,
            }))
        }

        Expr::MemberAccess(m) => {
            let target_ident_name = match m.target.as_ref() {
                Expr::Ident(id) => Some(id.as_str().to_string()),
                _ => None,
            };

            if target_ident_name.as_deref() == Some("env") {
                let member_name = m.member.as_str();
                return match ctx.env_scope.get(member_name) {
                    Some(EnvEntry::Bound(expr)) => Ok(expr.clone()),
                    Some(EnvEntry::Tombstone) => Err(CompileError::BlockedEnvVariable {
                        name: member_name.to_string(),
                        span: m.span,
                    }),
                    None => Err(CompileError::UndefinedEnvVariable {
                        name: member_name.to_string(),
                        span: m.span,
                    }),
                };
            }

            // Resolve target (parent, window, prev, self, or named node in lexical scope)
            let resolved_target = if let Some(target_name) = &target_ident_name {
                if target_name == "self" {
                    Expr::Ident(Ident::new(ctx.current_node.canonical_name(), m.target.span()))
                } else if let Some(LexicalBinding::Node(node_id)) = ctx.lexical_scope.get(target_name) {
                    Expr::Ident(Ident::new(node_id.canonical_name(), m.target.span()))
                } else if target_name == "parent" {
                    if let Some(parent) = ctx.parent_node {
                        Expr::Ident(Ident::new(parent.canonical_name(), m.target.span()))
                    } else {
                        rewrite_expr(&m.target, ctx)?
                    }
                } else if target_name == "window" {
                    Expr::Ident(Ident::new(NodeId::WINDOW.canonical_name(), m.target.span()))
                } else if target_name == "prev" {
                    if let Some(prev) = ctx.prev_sibling {
                        Expr::Ident(Ident::new(prev.canonical_name(), m.target.span()))
                    } else {
                        rewrite_expr(&m.target, ctx)?
                    }
                } else {
                    rewrite_expr(&m.target, ctx)?
                }
            } else {
                rewrite_expr(&m.target, ctx)?
            };

            let target_node_id = match &resolved_target {
                Expr::Ident(id) => NodeId::from_canonical_name(id.as_str()),
                _ => None,
            };

            // Font node reference access: e.g. label.font -> font_node_id
            if m.member.as_str() == "font" {
                if let Some(tid) = target_node_id {
                    if let Some(&fid) = ctx.node_fonts.get(&tid) {
                        return Ok(Expr::Ident(Ident::new(fid.canonical_name(), m.span)));
                    }
                }
            }

            // Typographic metric access: e.g. label.cap_height -> fid.cap_height
            if ["cap_height", "x_height", "descent", "ascent", "line_height"].contains(&m.member.as_str()) {
                if let Some(tid) = target_node_id {
                    if let Some(&fid) = ctx.node_fonts.get(&tid) {
                        return Ok(Expr::MemberAccess(MemberAccessExpr {
                            target: Box::new(Expr::Ident(Ident::new(fid.canonical_name(), m.span))),
                            member: m.member.clone(),
                            span: m.span,
                        }));
                    }
                }
            }

            Ok(Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(resolved_target),
                member: m.member.clone(),
                span: m.span,
            }))
        }

        Expr::Call(call) => {
            let mut rewritten_args = Vec::new();
            for arg in &call.args {
                if let Expr::MemberAccess(m) = arg {
                    if let Expr::Ident(target_id) = m.target.as_ref() {
                        if target_id.as_str() == "children" {
                            for &child_id in ctx.child_ids {
                                let child_access = Expr::MemberAccess(MemberAccessExpr {
                                    target: Box::new(Expr::Ident(Ident::new(
                                        child_id.canonical_name(),
                                        target_id.span,
                                    ))),
                                    member: m.member.clone(),
                                    span: m.span,
                                });
                                rewritten_args.push(child_access);
                            }
                            continue;
                        }
                    }
                }
                rewritten_args.push(rewrite_expr(arg, ctx)?);
            }

            if rewritten_args.is_empty() && !ctx.child_ids.is_empty() {
                return Ok(Expr::Literal(Literal::Number(0.0, call.span)));
            }

            Ok(Expr::Call(CallExpr {
                callee: call.callee.clone(),
                args: rewritten_args,
                span: call.span,
            }))
        }

        Expr::Binary(bin) => Ok(Expr::Binary(BinaryExpr {
            op: bin.op,
            left: Box::new(rewrite_expr(&bin.left, ctx)?),
            right: Box::new(rewrite_expr(&bin.right, ctx)?),
            span: bin.span,
        })),

        Expr::Unary(u) => Ok(Expr::Unary(UnaryExpr {
            op: u.op,
            operand: Box::new(rewrite_expr(&u.operand, ctx)?),
            span: u.span,
        })),

        Expr::Paren(inner, span) => {
            Ok(Expr::Paren(Box::new(rewrite_expr(inner, ctx)?), *span))
        }

        Expr::Literal(_) => Ok(expr.clone()),
        Expr::Node(n) => Ok(Expr::Node(n.clone())),
    }
}
