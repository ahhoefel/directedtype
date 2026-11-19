pub mod aggregation;
pub mod builtins;
pub mod children;
pub mod component;
pub mod context;
pub mod rewrite;
pub mod schema;
pub mod text_flow;
pub mod typecheck;

pub use context::{EnvEntry, LexicalBinding, ScopeContext};
pub use rewrite::{default_expr_for_state, default_expr_for_state_with_enums, rewrite_expr};
pub use schema::{
    collect_all_component_aliases, collect_all_component_lets, collect_all_document_lets,
    validate_component_key, ComponentSchema,
};
pub use typecheck::{typecheck_expanded_document, StaticType};

use builtins::{expand_primitive_element, register_builtin_enums_and_components};
use component::expand_component_instance;
use context::{ElementContext, InstanceContext};
use rewrite::{
    resolve_forward_refs_in_range, resolve_forward_scope_refs, try_preresolve_expr_binding,
};
use schema::{select_component_overload, validate_registered_component_def};
use typecheck::{
    coerce_typed_binding_expr, validate_content_items_type_annotations,
    validate_node_type_annotation,
};

use crate::ast::*;
use crate::compiler::error::CompileError;
use crate::compiler::expanded::{ExpandedDocument, ExpandedNode, NodeId};
use crate::compiler::scope::ScopeId;
use std::cell::RefCell;
use std::collections::HashMap;

/// Expands a parsed AST `Document` into an `ExpandedDocument` using default current directory.
pub fn expand_document(doc: &Document) -> Result<ExpandedDocument, CompileError> {
    expand_document_with_resolver(
        doc,
        std::path::Path::new("."),
        &crate::compiler::module::FsResolver,
    )
}

/// Expands a parsed AST `Document` into an `ExpandedDocument` with a specific base directory.
pub fn expand_document_with_base_dir(
    doc: &Document,
    base_dir: &std::path::Path,
) -> Result<ExpandedDocument, CompileError> {
    expand_document_with_resolver(doc, base_dir, &crate::compiler::module::FsResolver)
}

/// Expands a parsed AST `Document` into an `ExpandedDocument` with a custom file resolver.
pub fn expand_document_with_resolver<R: crate::compiler::module::FileResolver>(
    doc: &Document,
    base_dir: &std::path::Path,
    resolver: &R,
) -> Result<ExpandedDocument, CompileError> {
    expand_document_with_resolver_and_dpi(doc, base_dir, resolver, 96.0)
}

/// Expands a parsed AST `Document` into an `ExpandedDocument` with explicit default DPI (`96.0` viewer, `72.0` PDF).
pub fn expand_document_with_resolver_and_dpi<R: crate::compiler::module::FileResolver>(
    doc: &Document,
    base_dir: &std::path::Path,
    resolver: &R,
    default_dpi: f64,
) -> Result<ExpandedDocument, CompileError> {
    rewrite::reset_rewrite_tracking();
    crate::compiler::text::prewarm_font_context();
    let (mut registry, imported_items, all_module_enums) =
        crate::compiler::module::resolve_imports_with_enums(doc, base_dir, resolver)?;
    let mut merged_items = imported_items;
    merged_items.extend(doc.items.clone());
    let merged_doc = Document {
        items: merged_items,
        span: doc.span,
    };
    let doc = &merged_doc;

    let mut enums: HashMap<String, EnumDef> = HashMap::new();
    for item in &doc.items {
        if let Item::Enum(e) = item {
            let name = e.name.as_str().to_string();
            if enums.contains_key(&name) {
                return Err(CompileError::DuplicateEnum {
                    name,
                    span: e.name.span,
                });
            }
            enums.insert(name, e.clone());
        }
    }

    register_builtin_enums_and_components(&mut enums, &mut registry, doc.span);

    let mut known_enums = all_module_enums;
    for (k, v) in &enums {
        known_enums.insert(k.clone(), v.clone());
    }

    validate_top_level_type_annotations(doc, &enums, &known_enums, &registry)?;

    let mut global_scope = HashMap::new();
    let mut global_env_scope: HashMap<String, EnvEntry> = HashMap::new();
    let mut node_fonts: HashMap<NodeId, NodeId> = HashMap::new();
    let mut expanded_doc = ExpandedDocument::new();
    expanded_doc.default_dpi = default_dpi;

    let window_scope_ports = builtins::window::init_window_and_top_level_declarations(
        doc,
        &enums,
        &mut expanded_doc,
        &mut global_scope,
        &mut global_env_scope,
    )?;

    for overloads in registry.values() {
        for comp_def in overloads {
            ComponentSchema::for_def(comp_def)?;
        }
    }

    preresolve_top_level_lets_and_envs(
        doc,
        &enums,
        &mut expanded_doc,
        &mut global_scope,
        &mut global_env_scope,
        &node_fonts,
    );

    let global_scope = RefCell::new(global_scope);
    expand_top_level_items(
        doc,
        &window_scope_ports,
        &enums,
        &registry,
        &global_scope,
        &mut global_env_scope,
        &mut expanded_doc,
        &mut node_fonts,
    )?;

    finalize_window_and_forward_refs(
        &mut expanded_doc,
        &global_scope.borrow(),
        &global_env_scope,
        &node_fonts,
        &enums,
    )?;

    if !expanded_doc.paged_flows.is_empty() || !expanded_doc.deposits.is_empty() {
        expanded_doc.paged_registry = Some(std::sync::Arc::new(registry.clone()));
        expanded_doc.paged_enums = Some(std::sync::Arc::new(known_enums.clone()));
    }

    typecheck_expanded_document(&mut expanded_doc, &known_enums, &registry)?;

    Ok(expanded_doc)
}

fn validate_top_level_type_annotations(
    doc: &Document,
    enums: &HashMap<String, EnumDef>,
    known_enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> Result<(), CompileError> {
    for overloads in registry.values() {
        for comp_def in overloads {
            validate_registered_component_def(comp_def, known_enums, registry)?;
        }
    }
    for item in &doc.items {
        match item {
            Item::Let(l) => {
                if let Some(ty) = &l.type_annotation {
                    StaticType::from_type_name(ty.name.as_str(), ty.span, enums, registry)?;
                }
                match &l.value {
                    Some(LetValue::Node(n)) => {
                        for port in &n.ports {
                            typecheck::validate_expr_type_annotations(&port.expr, enums, registry)?;
                        }
                        if let Some(slot) = &n.content {
                            validate_content_items_type_annotations(&slot.items, enums, registry)?;
                        }
                    }
                    Some(LetValue::Expr(e)) => {
                        typecheck::validate_expr_type_annotations(e, enums, registry)?;
                    }
                    None => {}
                }
            }
            Item::State(s) => {
                if let Some(ty) = &s.type_annotation {
                    StaticType::from_type_name(ty.name.as_str(), ty.span, enums, registry)?;
                }
                if let Some(d) = &s.default {
                    typecheck::validate_expr_type_annotations(d, enums, registry)?;
                }
            }
            Item::Env(e) => {
                if let Some(ty) = &e.type_annotation {
                    StaticType::from_type_name(ty.name.as_str(), ty.span, enums, registry)?;
                }
                if let Some(v) = &e.value {
                    typecheck::validate_expr_type_annotations(v, enums, registry)?;
                }
            }
            Item::Node(n) => {
                for port in &n.ports {
                    typecheck::validate_expr_type_annotations(&port.expr, enums, registry)?;
                }
                if let Some(slot) = &n.content {
                    validate_content_items_type_annotations(&slot.items, enums, registry)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

pub(super) fn is_value_node_name(name: &str) -> bool {
    matches!(name, "Timer" | "IndefiniteTimer")
}

pub(super) fn expr_contains_timer(expr: &Expr) -> bool {
    match expr {
        Expr::Node(elem) => is_value_node_name(elem.name.as_str()),
        Expr::Paren(inner, _) => expr_contains_timer(inner),
        Expr::Unary(u) => expr_contains_timer(&u.operand),
        Expr::Ternary(t) => {
            expr_contains_timer(&t.condition)
                || expr_contains_timer(&t.then_expr)
                || expr_contains_timer(&t.else_expr)
        }
        _ => false,
    }
}

fn preresolve_top_level_lets_and_envs(
    doc: &Document,
    enums: &HashMap<String, EnumDef>,
    expanded_doc: &mut ExpandedDocument,
    global_scope: &mut HashMap<Symbol, LexicalBinding>,
    global_env_scope: &mut HashMap<String, EnvEntry>,
    node_fonts: &HashMap<NodeId, NodeId>,
) {
    let mut changed = true;
    let mut passes = 0;
    while changed && passes < 20 {
        changed = false;
        passes += 1;
        for item in &doc.items {
            match item {
                Item::Let(let_binding) => {
                    if let_binding.value.is_none() {
                        global_scope.insert(
                            let_binding.name.name,
                            LexicalBinding::Uninitialized,
                        );
                    } else if let Some(LetValue::Expr(raw_expr)) = &let_binding.value {
                        if expanded_doc
                            .window_ports
                            .contains_key(let_binding.name.as_str())
                        {
                            continue;
                        }
                        let rewritten_res = {
                            let scope_ctx = ScopeContext::for_window(
                                &expanded_doc.nodes,
                                global_scope,
                                global_env_scope,
                                node_fonts,
                                enums,
                            );
                            rewrite_expr(raw_expr, &scope_ctx)
                        };
                        if try_preresolve_expr_binding(
                            global_scope,
                            let_binding.name.name,
                            rewritten_res,
                            let_binding.type_annotation.as_ref(),
                        )
                        .is_some()
                        {
                            changed = true;
                        }
                    }
                }
                Item::Env(env_binding) => {
                    if env_binding.value.is_none() {
                        global_scope.insert(
                            env_binding.name.name,
                            LexicalBinding::Uninitialized,
                        );
                        global_env_scope
                            .insert(env_binding.name.as_str().to_string(), EnvEntry::Tombstone);
                    } else if let Some(raw_expr) = &env_binding.value {
                        if !matches!(raw_expr, Expr::Node(elem) if !is_value_node_name(elem.name.as_str())) {
                            let rewritten_res = {
                                let scope_ctx = ScopeContext::for_window(
                                    &expanded_doc.nodes,
                                    global_scope,
                                    global_env_scope,
                                    node_fonts,
                                    enums,
                                );
                                rewrite_expr(raw_expr, &scope_ctx)
                            };
                            if let Some(rewritten) = try_preresolve_expr_binding(
                                global_scope,
                                env_binding.name.name,
                                rewritten_res,
                                env_binding.type_annotation.as_ref(),
                            ) {
                                global_env_scope.insert(
                                    env_binding.name.as_str().to_string(),
                                    EnvEntry::Bound(rewritten),
                                );
                                changed = true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let window_scope_ctx = ScopeContext::for_window(
        &expanded_doc.nodes,
        global_scope,
        global_env_scope,
        node_fonts,
        enums,
    );
    for (name, expr) in expanded_doc.window_ports.clone() {
        if let Ok(rewritten) = rewrite_expr(&expr, &window_scope_ctx) {
            expanded_doc.window_ports.insert(name, rewritten);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn expand_top_level_items(
    doc: &Document,
    window_scope_ports: &[String],
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
    global_scope: &RefCell<HashMap<Symbol, LexicalBinding>>,
    global_env_scope: &mut HashMap<String, EnvEntry>,
    expanded_doc: &mut ExpandedDocument,
    node_fonts: &mut HashMap<NodeId, NodeId>,
) -> Result<(), CompileError> {
    let mut last_root_id = None;

    for item in &doc.items {
        match item {
            Item::Env(env_binding) => {
                if let Some(Expr::Node(elem)) = &env_binding.value {
                    if !is_value_node_name(elem.name.as_str()) {
                        validate_node_type_annotation(
                            elem.name.as_str(),
                            elem.span,
                            env_binding.type_annotation.as_ref(),
                            enums,
                            registry,
                        )?;
                        let elem_ctx = ElementContext {
                            parent_id: Some(NodeId::WINDOW),
                            parent_name: Some("window"),
                            prev_sibling_id: last_root_id,
                            has_next_sibling: false,
                            parent_ports: window_scope_ports,
                            lexical_scope: global_scope,
                            env_scope: global_env_scope,
                            enclosing_component_id: None,
                            enclosing_num_children: 0,
                            enclosing_instantiated_children: None,
                            shadowed_ports: None,
                            is_let: true,
                            ambient_authored_ports: None,
                            enums,
                            active_scope_id: ScopeId::ROOT,
                            consumer_children: None,
                            in_paged_flow: false,
                            in_page_template: false,
                            in_unbreakable_flow_container: None,
                            enclosing_comp_ports: None,
                        };
                        let root_id =
                            expand_element(elem, &elem_ctx, registry, expanded_doc, node_fonts)?;
                        expanded_doc.get_node_mut(root_id).unwrap().var_name =
                            Some(env_binding.name.as_str().to_string());
                        expanded_doc.roots.push(root_id);
                        last_root_id = Some(root_id);
                        global_scope.borrow_mut().insert(
                            env_binding.name.name,
                            LexicalBinding::Node(root_id),
                        );
                        let bound_expr =
                            Expr::Ident(Ident::from_node(root_id, elem.span));
                        global_env_scope.insert(
                            env_binding.name.as_str().to_string(),
                            EnvEntry::Bound(bound_expr),
                        );
                        if elem.name.as_str() == "Font" {
                            node_fonts.insert(root_id, root_id);
                        }
                        continue;
                    }
                }
                if let Some(raw_expr) = &env_binding.value {
                    let rewritten = {
                        let scope_borrow = global_scope.borrow();
                        let scope_ctx = ScopeContext::for_window(
                            &expanded_doc.nodes,
                            &scope_borrow,
                            global_env_scope,
                            node_fonts,
                            enums,
                        );
                        rewrite_expr(raw_expr, &scope_ctx)?
                    };
                    let rewritten = coerce_typed_binding_expr(
                        rewritten,
                        env_binding.type_annotation.as_ref(),
                    );
                    expanded_doc.local_bindings.push((
                        rewritten.clone(),
                        env_binding
                            .type_annotation
                            .as_ref()
                            .map(|t| t.name.as_str().to_string()),
                        raw_expr.span(),
                    ));
                    global_scope.borrow_mut().insert(
                        env_binding.name.name,
                        LexicalBinding::Expr(rewritten.clone()),
                    );
                    global_env_scope.insert(
                        env_binding.name.as_str().to_string(),
                        EnvEntry::Bound(rewritten),
                    );
                }
            }
            Item::Node(node) => {
                if node.name.as_str() == "Window" {
                    continue;
                }
                let elem_ctx = ElementContext {
                    parent_id: Some(NodeId::WINDOW),
                    parent_name: Some("window"),
                    prev_sibling_id: last_root_id,
                    has_next_sibling: false,
                    parent_ports: window_scope_ports,
                    lexical_scope: global_scope,
                    env_scope: global_env_scope,
                    enclosing_component_id: None,
                    enclosing_num_children: 0,
                    enclosing_instantiated_children: None,
                    shadowed_ports: None,
                    is_let: false,
                    ambient_authored_ports: None,
                    enums,
                    active_scope_id: ScopeId::ROOT,
                    consumer_children: None,
                    in_paged_flow: false,
                    in_page_template: false,
                    in_unbreakable_flow_container: None,
                    enclosing_comp_ports: None,
                };
                let root_id = expand_element(node, &elem_ctx, registry, expanded_doc, node_fonts)?;
                expanded_doc.roots.push(root_id);
                last_root_id = Some(root_id);
            }
            Item::Let(let_binding) => match &let_binding.value {
                None => {}
                Some(LetValue::Expr(raw_expr)) => {
                    let rewritten = {
                        let scope_borrow = global_scope.borrow();
                        let scope_ctx = ScopeContext::for_window(
                            &expanded_doc.nodes,
                            &scope_borrow,
                            global_env_scope,
                            node_fonts,
                            enums,
                        );
                        rewrite_expr(raw_expr, &scope_ctx)?
                    };
                    let rewritten = coerce_typed_binding_expr(
                        rewritten,
                        let_binding.type_annotation.as_ref(),
                    );
                    if expanded_doc
                        .window_ports
                        .contains_key(let_binding.name.as_str())
                    {
                        expanded_doc
                            .window_ports
                            .insert(let_binding.name.as_str().to_string(), rewritten);
                    } else {
                        expanded_doc.push_local_binding(
                            rewritten.clone(),
                            let_binding
                                .type_annotation
                                .as_ref()
                                .map(|t| t.name.as_str().to_string()),
                            raw_expr.span(),
                        );
                        global_scope.borrow_mut().insert(
                            let_binding.name.name,
                            LexicalBinding::Expr(rewritten),
                        );
                    }
                }
                Some(LetValue::Node(elem)) => {
                    validate_node_type_annotation(
                        elem.name.as_str(),
                        elem.span,
                        let_binding.type_annotation.as_ref(),
                        enums,
                        registry,
                    )?;
                    let root_id = NodeId(expanded_doc.nodes.len());
                    global_scope.borrow_mut().insert(
                        let_binding.name.name,
                        LexicalBinding::Node(root_id),
                    );
                    let elem_ctx = ElementContext {
                        parent_id: Some(NodeId::WINDOW),
                        parent_name: Some("window"),
                        prev_sibling_id: last_root_id,
                        has_next_sibling: false,
                        parent_ports: window_scope_ports,
                        lexical_scope: global_scope,
                        env_scope: global_env_scope,
                        enclosing_component_id: None,
                        enclosing_num_children: 0,
                        enclosing_instantiated_children: None,
                        shadowed_ports: None,
                        is_let: true,
                        ambient_authored_ports: None,
                        enums,
                        active_scope_id: ScopeId::ROOT,
                        consumer_children: None,
                        in_paged_flow: false,
                        in_page_template: false,
                        in_unbreakable_flow_container: None,
                        enclosing_comp_ports: None,
                    };
                    let expanded_root_id =
                        expand_element(elem, &elem_ctx, registry, expanded_doc, node_fonts)?;
                    debug_assert_eq!(root_id, expanded_root_id);
                    expanded_doc.get_node_mut(root_id).unwrap().var_name =
                        Some(let_binding.name.as_str().to_string());
                    expanded_doc.roots.push(root_id);
                    last_root_id = Some(root_id);
                    if elem.name.as_str() == "Font" {
                        node_fonts.insert(root_id, root_id);
                    }
                }
            },
            Item::Component(_) | Item::State(_) | Item::Use(_) | Item::Enum(_) => {}
        }
    }
    Ok(())
}

fn finalize_window_and_forward_refs(
    expanded_doc: &mut ExpandedDocument,
    final_global_scope: &HashMap<Symbol, LexicalBinding>,
    global_env_scope: &HashMap<String, EnvEntry>,
    node_fonts: &HashMap<NodeId, NodeId>,
    enums: &HashMap<String, EnumDef>,
) -> Result<(), CompileError> {
    let final_window_scope_ctx = ScopeContext::for_window(
        &expanded_doc.nodes,
        final_global_scope,
        global_env_scope,
        node_fonts,
        enums,
    );
    for (name, expr) in expanded_doc.window_ports.clone() {
        let rewritten = rewrite_expr(&expr, &final_window_scope_ctx)?;
        expanded_doc.window_ports.insert(name, rewritten);
    }
    for expr in expanded_doc.window_ports.values_mut() {
        *expr = resolve_forward_scope_refs(expr, final_global_scope);
    }
    resolve_forward_refs_in_range(expanded_doc, 0, 0, final_global_scope);
    rewrite::flush_all_materialized_expr_ports(expanded_doc);
    lower_dom_node_refs_in_doc(expanded_doc, None)?;
    for flow_rec in &mut expanded_doc.paged_flows {
        let n = flow_rec
            .probe_base_node_len
            .min(expanded_doc.nodes.len())
            .min(flow_rec.probe_doc.nodes.len());
        for i in 0..n {
            flow_rec.probe_doc.nodes[i] = expanded_doc.nodes[i].clone();
        }
        flow_rec.probe_doc.window_ports = expanded_doc.window_ports.clone();
    }
    Ok(())
}

fn lower_dom_node_refs_in_doc(
    doc: &mut ExpandedDocument,
    fallback_handles: Option<&HashMap<crate::dom::NodeHandle, NodeId>>,
) -> Result<(), CompileError> {
    let mut handles = doc.dom_handles.clone();
    if let Some(fallback) = fallback_handles {
        for (&h, &nid) in fallback {
            handles.entry(h).or_insert(nid);
        }
    }
    if handles.is_empty()
        && !doc_has_any_dom_node_ref(doc)
    {
        return Ok(());
    }
    for node in &mut doc.nodes {
        for expr in node.ports.values_mut() {
            lower_dom_node_refs_in_expr(expr, &handles)?;
        }
        for expr in node.unfiltered_ports.values_mut() {
            lower_dom_node_refs_in_expr(expr, &handles)?;
        }
        if let Some(key) = &mut node.key {
            for part in &mut key.parts {
                lower_dom_node_refs_in_expr(part, &handles)?;
            }
        }
    }
    for expr in doc.window_ports.values_mut() {
        lower_dom_node_refs_in_expr(expr, &handles)?;
    }
    for (expr, _, _) in &mut doc.local_bindings {
        lower_dom_node_refs_in_expr(expr, &handles)?;
    }
    for dep in &mut doc.deposits {
        lower_dom_node_refs_in_owned_child(&mut dep.payload, &handles)?;
    }
    for flow_rec in &mut doc.paged_flows {
        lower_dom_node_refs_in_owned_child(&mut flow_rec.page_template_elem, &handles)?;
        lower_dom_node_refs_in_doc(&mut flow_rec.probe_doc, Some(&handles))?;
    }
    Ok(())
}

fn doc_has_any_dom_node_ref(doc: &ExpandedDocument) -> bool {
    let has_in_expr = |e: &Expr| expr_has_dom_node_ref(e);
    doc.nodes.iter().any(|n| {
        n.ports.values().any(has_in_expr)
            || n.unfiltered_ports.values().any(has_in_expr)
            || n.key.as_ref().is_some_and(|k| k.parts.iter().any(has_in_expr))
    }) || doc.window_ports.values().any(has_in_expr)
        || doc.local_bindings.iter().any(|(e, _, _)| has_in_expr(e))
        || doc.deposits.iter().any(|d| owned_child_has_dom_node_ref(&d.payload))
        || doc.paged_flows.iter().any(|f| {
            owned_child_has_dom_node_ref(&f.page_template_elem)
                || doc_has_any_dom_node_ref(&f.probe_doc)
        })
}

fn owned_child_has_dom_node_ref(child: &crate::compiler::expanded::OwnedConsumerChild) -> bool {
    elem_has_dom_node_ref(&child.elem)
        || child.caller_lexical_scope.values().any(|b| match b {
            LexicalBinding::Expr(e) | LexicalBinding::State(e) => expr_has_dom_node_ref(e),
            _ => false,
        })
        || child.outer_consumer_children.iter().any(owned_child_has_dom_node_ref)
}

fn elem_has_dom_node_ref(elem: &ElementNode) -> bool {
    elem.ports.iter().any(|p| expr_has_dom_node_ref(&p.expr))
        || elem
            .key
            .as_ref()
            .is_some_and(|k| k.parts.iter().any(expr_has_dom_node_ref))
        || elem.content.as_ref().is_some_and(|c| {
            c.items.iter().any(|item| match item {
                ContentItem::Node(e) => elem_has_dom_node_ref(e),
                ContentItem::Children(ch) => {
                    ch.ports.iter().any(|p| expr_has_dom_node_ref(&p.expr))
                }
                ContentItem::Let(l) => match &l.value {
                    Some(LetValue::Expr(e)) => expr_has_dom_node_ref(e),
                    Some(LetValue::Node(e)) => elem_has_dom_node_ref(e),
                    None => false,
                },
                ContentItem::Alias(a) => expr_has_dom_node_ref(&a.value),
                ContentItem::State(s) => s.default.as_ref().is_some_and(expr_has_dom_node_ref),
                ContentItem::Env(e) => e.value.as_ref().is_some_and(expr_has_dom_node_ref),
                ContentItem::Assert(a) => expr_has_dom_node_ref(&a.condition),
                ContentItem::Text(_) | ContentItem::ParagraphBreak(_) => false,
            })
        })
}

fn expr_has_dom_node_ref(expr: &Expr) -> bool {
    match expr {
        Expr::DomNodeRef(_, _) => true,
        Expr::Paren(inner, _) => expr_has_dom_node_ref(inner),
        Expr::MemberAccess(m) => expr_has_dom_node_ref(&m.target),
        Expr::Index(idx) => expr_has_dom_node_ref(&idx.target) || expr_has_dom_node_ref(&idx.index),
        Expr::Binary(b) => expr_has_dom_node_ref(&b.left) || expr_has_dom_node_ref(&b.right),
        Expr::Unary(u) => expr_has_dom_node_ref(&u.operand),
        Expr::Ternary(t) => {
            expr_has_dom_node_ref(&t.condition)
                || expr_has_dom_node_ref(&t.then_expr)
                || expr_has_dom_node_ref(&t.else_expr)
        }
        Expr::Call(c) => c.args.iter().any(expr_has_dom_node_ref),
        Expr::MethodCall(mc) => {
            expr_has_dom_node_ref(&mc.target) || mc.args.iter().any(expr_has_dom_node_ref)
        }
        Expr::Node(e) => elem_has_dom_node_ref(e),
        Expr::ExprDef(def) => {
            def.params
                .iter()
                .any(|p| p.default_edge.as_ref().is_some_and(expr_has_dom_node_ref))
                || expr_has_dom_node_ref(&def.body)
        }
        Expr::Match(m) => {
            m.scrutinee.as_deref().is_some_and(expr_has_dom_node_ref)
                || m.arms.iter().any(|arm| {
                    matches!(&arm.pattern, MatchPattern::Condition(c) if expr_has_dom_node_ref(c))
                        || expr_has_dom_node_ref(&arm.body)
                })
        }
        Expr::Literal(_) | Expr::Ident(_) => false,
    }
}

fn lower_dom_node_refs_in_owned_child(
    child: &mut crate::compiler::expanded::OwnedConsumerChild,
    handles: &HashMap<crate::dom::NodeHandle, NodeId>,
) -> Result<(), CompileError> {
    lower_dom_node_refs_in_element(&mut child.elem, handles)?;
    for binding in child.caller_lexical_scope.values_mut() {
        match binding {
            LexicalBinding::Expr(e) | LexicalBinding::State(e) => {
                lower_dom_node_refs_in_expr(e, handles)?;
            }
            _ => {}
        }
    }
    for outer in &mut child.outer_consumer_children {
        lower_dom_node_refs_in_owned_child(outer, handles)?;
    }
    Ok(())
}

fn lower_dom_node_refs_in_content_items(
    items: &mut [ContentItem],
    handles: &HashMap<crate::dom::NodeHandle, NodeId>,
) -> Result<(), CompileError> {
    for item in items {
        match item {
            ContentItem::Node(elem) => lower_dom_node_refs_in_element(elem, handles)?,
            ContentItem::Children(ch) => {
                for p in &mut ch.ports {
                    lower_dom_node_refs_in_expr(&mut p.expr, handles)?;
                }
            }
            ContentItem::Let(l) => match &mut l.value {
                Some(LetValue::Expr(e)) => lower_dom_node_refs_in_expr(e, handles)?,
                Some(LetValue::Node(e)) => lower_dom_node_refs_in_element(e, handles)?,
                None => {}
            },
            ContentItem::Alias(a) => lower_dom_node_refs_in_expr(&mut a.value, handles)?,
            ContentItem::State(s) => {
                if let Some(d) = &mut s.default {
                    lower_dom_node_refs_in_expr(d, handles)?;
                }
            }
            ContentItem::Env(e) => {
                if let Some(v) = &mut e.value {
                    lower_dom_node_refs_in_expr(v, handles)?;
                }
            }
            ContentItem::Assert(a) => lower_dom_node_refs_in_expr(&mut a.condition, handles)?,
            ContentItem::Text(_) | ContentItem::ParagraphBreak(_) => {}
        }
    }
    Ok(())
}

fn lower_dom_node_refs_in_element(
    elem: &mut ElementNode,
    handles: &HashMap<crate::dom::NodeHandle, NodeId>,
) -> Result<(), CompileError> {
    for port in &mut elem.ports {
        lower_dom_node_refs_in_expr(&mut port.expr, handles)?;
    }
    if let Some(key) = &mut elem.key {
        for part in &mut key.parts {
            lower_dom_node_refs_in_expr(part, handles)?;
        }
    }
    if let Some(content) = &mut elem.content {
        lower_dom_node_refs_in_content_items(&mut content.items, handles)?;
    }
    Ok(())
}

fn lower_dom_node_refs_in_expr(
    expr: &mut Expr,
    handles: &HashMap<crate::dom::NodeHandle, NodeId>,
) -> Result<(), CompileError> {
    use std::sync::Arc;
    match expr {
        Expr::DomNodeRef(handle, span) => {
            let nid = handles.get(handle).copied().ok_or_else(|| CompileError::Custom {
                message: format!(
                    "Unresolved DOM node handle reference #{}",
                    handle.to_u64()
                ),
                span: *span,
            })?;
            *expr = Expr::Ident(Ident::from_node(nid, *span));
            Ok(())
        }
        Expr::Paren(inner, _) => lower_dom_node_refs_in_expr(Arc::make_mut(inner), handles),
        Expr::MemberAccess(m) => lower_dom_node_refs_in_expr(Arc::make_mut(&mut m.target), handles),
        Expr::Index(idx) => {
            lower_dom_node_refs_in_expr(Arc::make_mut(&mut idx.target), handles)?;
            lower_dom_node_refs_in_expr(Arc::make_mut(&mut idx.index), handles)
        }
        Expr::Binary(b) => {
            lower_dom_node_refs_in_expr(Arc::make_mut(&mut b.left), handles)?;
            lower_dom_node_refs_in_expr(Arc::make_mut(&mut b.right), handles)
        }
        Expr::Unary(u) => lower_dom_node_refs_in_expr(Arc::make_mut(&mut u.operand), handles),
        Expr::Ternary(t) => {
            lower_dom_node_refs_in_expr(Arc::make_mut(&mut t.condition), handles)?;
            lower_dom_node_refs_in_expr(Arc::make_mut(&mut t.then_expr), handles)?;
            lower_dom_node_refs_in_expr(Arc::make_mut(&mut t.else_expr), handles)
        }
        Expr::Call(c) => {
            for arg in &mut c.args {
                lower_dom_node_refs_in_expr(arg, handles)?;
            }
            Ok(())
        }
        Expr::MethodCall(mc) => {
            lower_dom_node_refs_in_expr(Arc::make_mut(&mut mc.target), handles)?;
            for arg in &mut mc.args {
                lower_dom_node_refs_in_expr(arg, handles)?;
            }
            Ok(())
        }
        Expr::Node(elem) => lower_dom_node_refs_in_element(elem, handles),
        Expr::ExprDef(def) => {
            let def = Arc::make_mut(def);
            for p in &mut def.params {
                if let Some(d) = &mut p.default_edge {
                    lower_dom_node_refs_in_expr(d, handles)?;
                }
            }
            lower_dom_node_refs_in_expr(Arc::make_mut(&mut def.body), handles)
        }
        Expr::Match(m) => {
            let m = Arc::make_mut(m);
            if let Some(s) = &mut m.scrutinee {
                lower_dom_node_refs_in_expr(Arc::make_mut(s), handles)?;
            }
            for arm in &mut m.arms {
                if let MatchPattern::Condition(c) = &mut arm.pattern {
                    lower_dom_node_refs_in_expr(c, handles)?;
                }
                lower_dom_node_refs_in_expr(&mut arm.body, handles)?;
            }
            Ok(())
        }
        Expr::Literal(_) | Expr::Ident(_) => Ok(()),
    }
}

#[inline(never)]
fn init_and_push_expanded_node(
    elem: &ElementNode,
    ctx: &ElementContext<'_>,
    registry: &HashMap<String, Vec<ComponentDef>>,
    doc: &mut ExpandedDocument,
    node_fonts: &mut HashMap<NodeId, NodeId>,
) -> Result<NodeId, CompileError> {
    let node_id = NodeId(doc.nodes.len());
    let unmangled_elem_name = crate::compiler::graph::unmangle_component_name(elem.name.as_str());
    let mut expanded = ExpandedNode::new(node_id, unmangled_elem_name, elem.span);
    expanded.handle = elem.handle;
    if let Some(h) = elem.handle {
        doc.dom_handles.entry(h).or_insert(node_id);
    }
    expanded.parent = ctx.parent_id;
    expanded.prev_sibling = ctx.prev_sibling_id;
    expanded.scope_id = Some(ctx.active_scope_id);
    expanded.enclosing_component = ctx.enclosing_component_id;
    if let Some(ambients) = ctx.ambient_authored_ports {
        expanded.ambient_ports = ambients.keys().cloned().collect();
    }
    doc.scope_tree
        .node_to_scope
        .insert(node_id, ctx.active_scope_id);

    if let Some(key) = &elem.key {
        validate_component_key(elem.name.as_str(), key, &ctx.lexical_scope.borrow())?;
        let empty_children: [NodeId; 0] = [];
        let inst_borrow = ctx.enclosing_instantiated_children.map(|c| c.borrow());
        let child_ids_slice: &[NodeId] = match &inst_borrow {
            Some(b) => b.as_slice(),
            None => &empty_children,
        };
        let lexical_borrow = ctx.lexical_scope.borrow();
        let key_scope_ctx = ScopeContext::from_element(
            node_id,
            ctx,
            &doc.nodes,
            &lexical_borrow,
            node_fonts,
            child_ids_slice,
        );
        let mut rewritten_parts = Vec::new();
        for part in &key.parts {
            rewritten_parts.push(rewrite_expr(part, &key_scope_ctx)?);
        }
        expanded.key = Some(ComponentKey::new(rewritten_parts, key.span));
    }

    // Check if element has text content directly in content slot (only for primitives or \Span components,
    // never for block \Component or \InlineBox containers which expand text into child \Text nodes)
    let is_non_span_component = registry
        .get(elem.name.as_str())
        .is_some_and(|defs| defs.iter().all(|d| d.kind != crate::ast::ComponentKind::Span));
    if !is_non_span_component {
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
    }

    // Allocate placeholder in doc.nodes to reserve index
    doc.nodes.push(expanded);
    Ok(node_id)
}

/// Expands a single element node (either a component invocation or a primitive).
pub(crate) fn expand_element(
    elem: &ElementNode,
    ctx: &ElementContext<'_>,
    registry: &HashMap<String, Vec<ComponentDef>>,
    doc: &mut ExpandedDocument,
    node_fonts: &mut HashMap<NodeId, NodeId>,
) -> Result<NodeId, CompileError> {
    if elem.name.as_str() == "Window" {
        return Err(CompileError::NestedWindowDeclaration { span: elem.span });
    }

    for port in &elem.ports {
        if port.name.as_str() == "parent" {
            return Err(CompileError::ReservedPort {
                node: elem.name.as_str().to_string(),
                port: "parent".to_string(),
                span: port.name.span,
            });
        }
    }

    let node_id = init_and_push_expanded_node(elem, ctx, registry, doc, node_fonts)?;

    // If it's an invocation of a user-defined component:
    if let Some(overloads) = registry.get(elem.name.as_str()) {
        if overloads.len() > 1 {
            for port in &elem.ports {
                let name = port.name.as_str();
                let mut is_accepted_param = false;
                let mut is_declared_alias = matches!(name, "left" | "top" | "right" | "bottom");
                for o in overloads {
                    let schema = ComponentSchema::for_def(o)?;
                    if schema.param_names.contains(name) {
                        is_accepted_param = true;
                        break;
                    }
                    if schema.declared_aliases.contains_key(name) {
                        is_declared_alias = true;
                    }
                }
                if !is_accepted_param && is_declared_alias {
                    return Err(CompileError::ImmutableAliasPort {
                        node: elem.name.as_str().to_string(),
                        port: name.to_string(),
                        span: port.name.span,
                    });
                }
            }
        }

        let comp_def = if overloads.len() == 1 {
            &overloads[0]
        } else {
            select_component_overload(
                overloads,
                elem.name.as_str(),
                elem,
                ctx.env_scope,
                elem.span,
            )?
        };
        let inst_ctx = InstanceContext {
            comp_node_id: node_id,
            parent_id: ctx.parent_id,
            parent_name: ctx.parent_name,
            prev_sibling_id: ctx.prev_sibling_id,
            has_next_sibling: ctx.has_next_sibling,
            parent_ports: ctx.parent_ports,
            ambient_authored_ports: ctx.ambient_authored_ports,
            enclosing_component_id: ctx.enclosing_component_id,
            enclosing_num_children: ctx.enclosing_num_children,
            enclosing_instantiated_children: ctx.enclosing_instantiated_children,
            shadowed_ports: ctx.shadowed_ports,
            enums: ctx.enums,
            active_scope_id: ctx.active_scope_id,
            consumer_children: ctx.consumer_children,
            in_page_template: ctx.in_page_template,
            in_paged_flow: ctx.in_paged_flow,
            in_unbreakable_flow_container: ctx.in_unbreakable_flow_container,
        };
        expand_component_instance(
            &inst_ctx,
            elem,
            comp_def,
            registry,
            doc,
            ctx.lexical_scope,
            ctx.env_scope,
            node_fonts,
        )?;
    } else {
        expand_primitive_element(node_id, elem, ctx, registry, doc, node_fonts)?;
    }

    Ok(node_id)
}
