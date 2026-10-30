use super::context::{
    ConsumerChildEntry, ElementContext, EnvEntry, ForwardedChildrenDirective, InstanceContext,
    LexicalBinding, ScopeContext,
};
use super::expand_element;
use super::rewrite::rewrite_expr;
use super::text_flow::{
    is_allowed_in_single_text, is_inline_box_element, is_inline_element, is_span_element,
};
use super::typecheck::validate_node_type_annotation;
use crate::ast::*;
use crate::compiler::error::CompileError;
use crate::compiler::expanded::{ExpandedDocument, ExpandedNode, NodeId};
use crate::compiler::text::{InlineBoxSpan, TextSpan};
use crate::span::Span;
use std::cell::RefCell;
use std::collections::HashMap;

pub(super) fn count_consumer_children(
    slot: Option<&ContentSlot>,
    is_inline_context: bool,
    outer_children_len: usize,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> usize {
    let Some(slot) = slot else {
        return 0;
    };
    if is_inline_context {
        return slot
            .items
            .iter()
            .map(|item| match item {
                ContentItem::Node(_) => 1,
                ContentItem::Text(chunk) if !chunk.text.is_empty() => 1,
                ContentItem::Children(_) => outer_children_len,
                ContentItem::Let(l) if matches!(l.value, Some(LetValue::Node(_))) => 1,
                _ => 0,
            })
            .sum();
    }
    let mut count = 0;
    let mut pending_has_text_or_span = false;
    let mut pending_inline_boxes = 0usize;
    let flush = |count: &mut usize,
                 pending_has_text_or_span: &mut bool,
                 pending_inline_boxes: &mut usize| {
        if *pending_has_text_or_span {
            *count += 1;
        } else {
            *count += *pending_inline_boxes;
        }
        *pending_has_text_or_span = false;
        *pending_inline_boxes = 0;
    };
    for item in &slot.items {
        match item {
            ContentItem::Text(chunk) => {
                if !chunk.text.trim().is_empty() {
                    pending_has_text_or_span = true;
                }
            }
            ContentItem::Node(child_elem) => {
                if is_span_element(child_elem, registry) {
                    pending_has_text_or_span = true;
                } else if is_inline_box_element(child_elem, registry) {
                    pending_inline_boxes += 1;
                } else {
                    flush(
                        &mut count,
                        &mut pending_has_text_or_span,
                        &mut pending_inline_boxes,
                    );
                    count += 1;
                }
            }
            ContentItem::Let(l) => {
                if let Some(LetValue::Node(child_elem)) = &l.value {
                    if is_span_element(child_elem, registry) {
                        pending_has_text_or_span = true;
                    } else if is_inline_box_element(child_elem, registry) {
                        pending_inline_boxes += 1;
                    } else {
                        flush(
                            &mut count,
                            &mut pending_has_text_or_span,
                            &mut pending_inline_boxes,
                        );
                        count += 1;
                    }
                }
            }
            ContentItem::ParagraphBreak(_) => {
                flush(
                    &mut count,
                    &mut pending_has_text_or_span,
                    &mut pending_inline_boxes,
                );
            }
            ContentItem::Children(_) => {
                flush(
                    &mut count,
                    &mut pending_has_text_or_span,
                    &mut pending_inline_boxes,
                );
                count += outer_children_len;
            }
            ContentItem::Alias(_)
            | ContentItem::State(_)
            | ContentItem::Env(_)
            | ContentItem::Assert(_) => {}
        }
    }
    flush(
        &mut count,
        &mut pending_has_text_or_span,
        &mut pending_inline_boxes,
    );
    count
}

/// Collects and normalizes consumer children passed to a component invocation,
/// grouping inline text/spans into synthetic `\Text` paragraphs when in block context.
#[allow(clippy::too_many_arguments)]
pub(super) fn collect_consumer_children<'a>(
    ctx: &InstanceContext<'a>,
    instance: &'a ElementNode,
    comp_def: &'a ComponentDef,
    comp_scope_ports: &[String],
    is_span_def: bool,
    is_single_text_context: bool,
    lexical_scope: &'a RefCell<HashMap<String, LexicalBinding>>,
    caller_env_scope: &HashMap<String, EnvEntry>,
    instantiated_children_ids: &'a RefCell<Vec<NodeId>>,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> Result<Vec<ConsumerChildEntry<'a>>, CompileError> {
    let mut consumer_child_entries: Vec<ConsumerChildEntry<'a>> = Vec::new();
    let Some(slot) = &instance.content else {
        return Ok(consumer_child_entries);
    };

    if is_span_def || is_single_text_context {
        for item in &slot.items {
            match item {
                ContentItem::Node(child_elem) => {
                    if is_single_text_context && !is_allowed_in_single_text(child_elem, registry) {
                        return Err(CompileError::Custom {
                            message: format!(
                                "Block component '{}' cannot be placed inside single-text component '{}'; only plain text, \\Span, and \\InlineBox are allowed",
                                child_elem.name.as_str(),
                                comp_def.name.as_str()
                            ),
                            span: child_elem.span,
                        });
                    }
                    consumer_child_entries.push(ConsumerChildEntry {
                        elem: child_elem.clone(),
                        var_name: None,
                        raw_text: None,
                        caller_lexical_scope: lexical_scope,
                        caller_env_scope: caller_env_scope.clone(),
                        caller_enclosing_component_id: ctx.enclosing_component_id,
                        caller_enclosing_num_children: ctx.enclosing_num_children,
                        caller_enclosing_instantiated_children: ctx
                            .enclosing_instantiated_children,
                        caller_shadowed_ports: ctx.shadowed_ports,
                        instantiated_ids: vec![instantiated_children_ids],
                        forwarded_directives: Vec::new(),
                    });
                }
                ContentItem::Text(chunk) => {
                    if !chunk.text.is_empty() {
                        let synthetic_text = ElementNode {
                            name: Ident::new("Text", chunk.span),
                            key: None,
                            ports: Vec::new(),
                            content: Some(ContentSlot {
                                items: vec![ContentItem::Text(chunk.clone())],
                                span: chunk.span,
                            }),
                            span: chunk.span,
                            handle: None,
                        };
                        consumer_child_entries.push(ConsumerChildEntry {
                            elem: synthetic_text,
                            var_name: None,
                            raw_text: Some(chunk.clone()),
                            caller_lexical_scope: lexical_scope,
                            caller_env_scope: caller_env_scope.clone(),
                            caller_enclosing_component_id: ctx.enclosing_component_id,
                            caller_enclosing_num_children: ctx.enclosing_num_children,
                            caller_enclosing_instantiated_children: ctx
                                .enclosing_instantiated_children,
                            caller_shadowed_ports: ctx.shadowed_ports,
                            instantiated_ids: vec![instantiated_children_ids],
                            forwarded_directives: Vec::new(),
                        });
                    }
                }
                ContentItem::Children(forward_dir) => {
                    for port in &forward_dir.ports {
                        if port.name.as_str() == "parent" {
                            return Err(CompileError::ReservedPort {
                                node: "Children".to_string(),
                                port: "parent".to_string(),
                                span: port.name.span,
                            });
                        }
                    }
                    if let Some(outer_entries) = ctx.consumer_children {
                        for outer_entry in outer_entries {
                            let mut entry = outer_entry.clone();
                            entry.instantiated_ids.push(instantiated_children_ids);
                            if !forward_dir.ports.is_empty() {
                                entry.forwarded_directives.push(ForwardedChildrenDirective {
                                    ports: &forward_dir.ports,
                                    parent_id: ctx.comp_node_id,
                                    parent_name: comp_def.name.as_str(),
                                    parent_ports: comp_scope_ports.to_vec(),
                                    lexical_scope,
                                    env_scope: caller_env_scope.clone(),
                                    enclosing_component_id: ctx.enclosing_component_id,
                                    enclosing_num_children: ctx.enclosing_num_children,
                                    enclosing_instantiated_children: ctx
                                        .enclosing_instantiated_children,
                                    shadowed_ports: ctx.shadowed_ports,
                                    comp_ports: None,
                                    declared_state_names: None,
                                });
                            }
                            consumer_child_entries.push(entry);
                        }
                    }
                }
                ContentItem::Let(l) => {
                    if let Some(LetValue::Node(child_elem)) = &l.value {
                        validate_node_type_annotation(
                            child_elem.name.as_str(),
                            child_elem.span,
                            l.type_annotation.as_ref(),
                            ctx.enums,
                            registry,
                        )?;
                        if is_single_text_context && !is_allowed_in_single_text(child_elem, registry) {
                            return Err(CompileError::Custom {
                                message: format!(
                                    "Block component '{}' cannot be placed inside single-text component '{}'; only plain text, \\Span, and \\InlineBox are allowed",
                                    child_elem.name.as_str(),
                                    comp_def.name.as_str()
                                ),
                                span: child_elem.span,
                            });
                        }
                        consumer_child_entries.push(ConsumerChildEntry {
                            elem: child_elem.clone(),
                            var_name: Some(l.name.as_str().to_string()),
                            raw_text: None,
                            caller_lexical_scope: lexical_scope,
                            caller_env_scope: caller_env_scope.clone(),
                            caller_enclosing_component_id: ctx.enclosing_component_id,
                            caller_enclosing_num_children: ctx.enclosing_num_children,
                            caller_enclosing_instantiated_children: ctx
                                .enclosing_instantiated_children,
                            caller_shadowed_ports: ctx.shadowed_ports,
                            instantiated_ids: vec![instantiated_children_ids],
                            forwarded_directives: Vec::new(),
                        });
                    }
                }
                ContentItem::ParagraphBreak(span) => {
                    let kind_label = if is_span_def {
                        "inline \\Span"
                    } else {
                        "single-text component"
                    };
                    return Err(CompileError::Custom {
                        message: format!(
                            "Paragraph break (blank line) is not permitted inside {} '{}'",
                            kind_label,
                            comp_def.name.as_str()
                        ),
                        span: *span,
                    });
                }
                ContentItem::Alias(_)
                | ContentItem::State(_)
                | ContentItem::Env(_)
                | ContentItem::Assert(_) => {}
            }
        }
    } else {
        let mut pending_inline_items: Vec<ContentItem> = Vec::new();
        let drain_flushed_inline_items =
            |pending_inline_items: &mut Vec<ContentItem>| -> Vec<(ElementNode, Option<String>, Option<TextChunk>)> {
                if pending_inline_items.is_empty() {
                    return Vec::new();
                }
                let has_text_or_span = pending_inline_items.iter().any(|it| match it {
                    ContentItem::Text(t) => !t.text.trim().is_empty(),
                    ContentItem::Node(e) => is_span_element(e, registry),
                    ContentItem::Let(l) => l.value.as_ref().is_some_and(
                        |v| matches!(v, LetValue::Node(e) if is_span_element(e, registry)),
                    ),
                    _ => false,
                });

                if has_text_or_span {
                    while let Some(ContentItem::Text(first_chunk)) =
                        pending_inline_items.first_mut()
                    {
                        let trimmed = first_chunk.text.trim_start();
                        if trimmed.is_empty() {
                            pending_inline_items.remove(0);
                        } else {
                            if trimmed.len() != first_chunk.text.len() {
                                first_chunk.text = trimmed.to_string();
                            }
                            break;
                        }
                    }
                    while let Some(ContentItem::Text(last_chunk)) =
                        pending_inline_items.last_mut()
                    {
                        let trimmed = last_chunk.text.trim_end();
                        if trimmed.is_empty() {
                            pending_inline_items.pop();
                        } else {
                            if trimmed.len() != last_chunk.text.len() {
                                last_chunk.text = trimmed.to_string();
                            }
                            break;
                        }
                    }
                    if pending_inline_items.is_empty() {
                        return Vec::new();
                    }
                    let first_span = pending_inline_items.first().unwrap().span();
                    let last_span = pending_inline_items.last().unwrap().span();
                    let para_span = Span::new(first_span.start, last_span.end);
                    let raw_text = if pending_inline_items.len() == 1 {
                        if let ContentItem::Text(chunk) = &pending_inline_items[0] {
                            Some(chunk.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    let items = std::mem::take(pending_inline_items);
                    let synthetic_text = ElementNode {
                        name: Ident::new("Text", para_span),
                        key: None,
                        ports: Vec::new(),
                        content: Some(ContentSlot {
                            items,
                            span: para_span,
                        }),
                        span: para_span,
                        handle: None,
                    };
                    vec![(synthetic_text, None, raw_text)]
                } else {
                    let mut out = Vec::new();
                    for item in pending_inline_items.drain(..) {
                        match item {
                            ContentItem::Node(child_elem) => {
                                out.push((child_elem, None, None));
                            }
                            ContentItem::Let(l) => {
                                if let Some(LetValue::Node(child_elem)) = l.value {
                                    out.push((
                                        child_elem,
                                        Some(l.name.as_str().to_string()),
                                        None,
                                    ));
                                }
                            }
                            _ => {}
                        }
                    }
                    out
                }
            };

        for item in &slot.items {
            match item {
                ContentItem::Text(chunk) => {
                    pending_inline_items.push(ContentItem::Text(chunk.clone()));
                }
                ContentItem::Node(child_elem) => {
                    if is_inline_element(child_elem, registry) {
                        pending_inline_items.push(ContentItem::Node(child_elem.clone()));
                    } else {
                        for (elem, var_name, raw_text) in
                            drain_flushed_inline_items(&mut pending_inline_items)
                        {
                            consumer_child_entries.push(ConsumerChildEntry {
                                elem,
                                var_name,
                                raw_text,
                                caller_lexical_scope: lexical_scope,
                                caller_env_scope: caller_env_scope.clone(),
                                caller_enclosing_component_id: ctx.enclosing_component_id,
                                caller_enclosing_num_children: ctx.enclosing_num_children,
                                caller_enclosing_instantiated_children: ctx
                                    .enclosing_instantiated_children,
                                caller_shadowed_ports: ctx.shadowed_ports,
                                instantiated_ids: vec![instantiated_children_ids],
                                forwarded_directives: Vec::new(),
                            });
                        }
                        consumer_child_entries.push(ConsumerChildEntry {
                            elem: child_elem.clone(),
                            var_name: None,
                            raw_text: None,
                            caller_lexical_scope: lexical_scope,
                            caller_env_scope: caller_env_scope.clone(),
                            caller_enclosing_component_id: ctx.enclosing_component_id,
                            caller_enclosing_num_children: ctx.enclosing_num_children,
                            caller_enclosing_instantiated_children: ctx
                                .enclosing_instantiated_children,
                            caller_shadowed_ports: ctx.shadowed_ports,
                            instantiated_ids: vec![instantiated_children_ids],
                            forwarded_directives: Vec::new(),
                        });
                    }
                }
                ContentItem::Let(l) => {
                    if let Some(LetValue::Node(child_elem)) = &l.value {
                        validate_node_type_annotation(
                            child_elem.name.as_str(),
                            child_elem.span,
                            l.type_annotation.as_ref(),
                            ctx.enums,
                            registry,
                        )?;
                        if is_inline_element(child_elem, registry) {
                            pending_inline_items.push(ContentItem::Let(l.clone()));
                        } else {
                            for (elem, var_name, raw_text) in
                                drain_flushed_inline_items(&mut pending_inline_items)
                            {
                                consumer_child_entries.push(ConsumerChildEntry {
                                    elem,
                                    var_name,
                                    raw_text,
                                    caller_lexical_scope: lexical_scope,
                                    caller_env_scope: caller_env_scope.clone(),
                                    caller_enclosing_component_id: ctx.enclosing_component_id,
                                    caller_enclosing_num_children: ctx.enclosing_num_children,
                                    caller_enclosing_instantiated_children: ctx
                                        .enclosing_instantiated_children,
                                    caller_shadowed_ports: ctx.shadowed_ports,
                                    instantiated_ids: vec![instantiated_children_ids],
                                    forwarded_directives: Vec::new(),
                                });
                            }
                            consumer_child_entries.push(ConsumerChildEntry {
                                elem: child_elem.clone(),
                                var_name: Some(l.name.as_str().to_string()),
                                raw_text: None,
                                caller_lexical_scope: lexical_scope,
                                caller_env_scope: caller_env_scope.clone(),
                                caller_enclosing_component_id: ctx.enclosing_component_id,
                                caller_enclosing_num_children: ctx.enclosing_num_children,
                                caller_enclosing_instantiated_children: ctx
                                    .enclosing_instantiated_children,
                                caller_shadowed_ports: ctx.shadowed_ports,
                                instantiated_ids: vec![instantiated_children_ids],
                                forwarded_directives: Vec::new(),
                            });
                        }
                    }
                }
                ContentItem::ParagraphBreak(_) => {
                    for (elem, var_name, raw_text) in
                        drain_flushed_inline_items(&mut pending_inline_items)
                    {
                        consumer_child_entries.push(ConsumerChildEntry {
                            elem,
                            var_name,
                            raw_text,
                            caller_lexical_scope: lexical_scope,
                            caller_env_scope: caller_env_scope.clone(),
                            caller_enclosing_component_id: ctx.enclosing_component_id,
                            caller_enclosing_num_children: ctx.enclosing_num_children,
                            caller_enclosing_instantiated_children: ctx
                                .enclosing_instantiated_children,
                            caller_shadowed_ports: ctx.shadowed_ports,
                            instantiated_ids: vec![instantiated_children_ids],
                            forwarded_directives: Vec::new(),
                        });
                    }
                }
                ContentItem::Children(forward_dir) => {
                    for (elem, var_name, raw_text) in
                        drain_flushed_inline_items(&mut pending_inline_items)
                    {
                        consumer_child_entries.push(ConsumerChildEntry {
                            elem,
                            var_name,
                            raw_text,
                            caller_lexical_scope: lexical_scope,
                            caller_env_scope: caller_env_scope.clone(),
                            caller_enclosing_component_id: ctx.enclosing_component_id,
                            caller_enclosing_num_children: ctx.enclosing_num_children,
                            caller_enclosing_instantiated_children: ctx
                                .enclosing_instantiated_children,
                            caller_shadowed_ports: ctx.shadowed_ports,
                            instantiated_ids: vec![instantiated_children_ids],
                            forwarded_directives: Vec::new(),
                        });
                    }
                    for port in &forward_dir.ports {
                        if port.name.as_str() == "parent" {
                            return Err(CompileError::ReservedPort {
                                node: "Children".to_string(),
                                port: "parent".to_string(),
                                span: port.name.span,
                            });
                        }
                    }
                    if let Some(outer_entries) = ctx.consumer_children {
                        for outer_entry in outer_entries {
                            let mut entry = outer_entry.clone();
                            entry.instantiated_ids.push(instantiated_children_ids);
                            if !forward_dir.ports.is_empty() {
                                entry.forwarded_directives.push(ForwardedChildrenDirective {
                                    ports: &forward_dir.ports,
                                    parent_id: ctx.comp_node_id,
                                    parent_name: comp_def.name.as_str(),
                                    parent_ports: comp_scope_ports.to_vec(),
                                    lexical_scope,
                                    env_scope: caller_env_scope.clone(),
                                    enclosing_component_id: ctx.enclosing_component_id,
                                    enclosing_num_children: ctx.enclosing_num_children,
                                    enclosing_instantiated_children: ctx
                                        .enclosing_instantiated_children,
                                    shadowed_ports: ctx.shadowed_ports,
                                    comp_ports: None,
                                    declared_state_names: None,
                                });
                            }
                            consumer_child_entries.push(entry);
                        }
                    }
                }
                ContentItem::Alias(_)
                | ContentItem::State(_)
                | ContentItem::Env(_)
                | ContentItem::Assert(_) => {}
            }
        }
        for (elem, var_name, raw_text) in
            drain_flushed_inline_items(&mut pending_inline_items)
        {
            consumer_child_entries.push(ConsumerChildEntry {
                elem,
                var_name,
                raw_text,
                caller_lexical_scope: lexical_scope,
                caller_env_scope: caller_env_scope.clone(),
                caller_enclosing_component_id: ctx.enclosing_component_id,
                caller_enclosing_num_children: ctx.enclosing_num_children,
                caller_enclosing_instantiated_children: ctx
                    .enclosing_instantiated_children,
                caller_shadowed_ports: ctx.shadowed_ports,
                instantiated_ids: vec![instantiated_children_ids],
                forwarded_directives: Vec::new(),
            });
        }
    }

    Ok(consumer_child_entries)
}

/// Applies forwarded `\Children(...)` ambient port directives from outer component wrappers to a consumer child.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_forwarded_directives(
    forwarded_directives: &[ForwardedChildrenDirective<'_>],
    child_node_id: NodeId,
    prev_sibling: Option<NodeId>,
    has_next_sibling: bool,
    nodes: &[ExpandedNode],
    node_fonts: &HashMap<NodeId, NodeId>,
    enums: &HashMap<String, EnumDef>,
    merged_ports: &mut HashMap<String, Expr>,
    authored_ambient_ports: &mut HashMap<String, Expr>,
) -> Result<(), CompileError> {
    let empty_children: [NodeId; 0] = [];
    let empty_ports: [String; 0] = [];
    for fwd in forwarded_directives {
        let fwd_lexical_borrow = fwd.lexical_scope.borrow();
        let fwd_inst_borrow = fwd.enclosing_instantiated_children.map(|c| c.borrow());
        let fwd_child_ids: &[NodeId] = match &fwd_inst_borrow {
            Some(b) => b.as_slice(),
            None => &empty_children,
        };
        let mut fwd_scope_ctx = ScopeContext {
            current_node: child_node_id,
            parent_node: Some(fwd.parent_id),
            parent_name: Some(fwd.parent_name),
            prev_sibling,
            has_next_sibling,
            child_ids: fwd_child_ids,
            num_children: fwd.enclosing_num_children,
            child_node: Some(child_node_id),
            nodes,
            shadowed_ports: fwd.shadowed_ports,
            is_event_port: false,
            parent_ports: &fwd.parent_ports,
            current_ports: &empty_ports,
            lexical_scope: &*fwd_lexical_borrow,
            env_scope: &fwd.env_scope,
            node_fonts,
            enclosing_component: fwd.enclosing_component_id,
            comp_ports: fwd.comp_ports,
            declared_state_names: fwd.declared_state_names,
            enums,
        };
        for ambient in fwd.ports {
            fwd_scope_ctx.is_event_port = ambient.name.as_str().starts_with("on_");
            let rewritten = rewrite_expr(&ambient.expr, &fwd_scope_ctx)?;
            merged_ports.insert(ambient.name.as_str().to_string(), rewritten);
            authored_ambient_ports
                .insert(ambient.name.as_str().to_string(), ambient.expr.clone());
        }
    }
    Ok(())
}

/// Expands consumer children inside a `\Span` component definition and accumulates its text, spans, and inline boxes.
#[allow(clippy::too_many_arguments)]
pub(super) fn expand_span_consumer_children(
    ctx: &InstanceContext<'_>,
    comp_def: &ComponentDef,
    comp_scope_ports: &[String],
    consumer_child_entries: &[ConsumerChildEntry<'_>],
    children_env_scope: &HashMap<String, EnvEntry>,
    registry: &HashMap<String, Vec<ComponentDef>>,
    doc: &mut ExpandedDocument,
    node_fonts: &mut HashMap<NodeId, NodeId>,
    all_children_ids: &mut Vec<NodeId>,
) -> Result<(String, Vec<TextSpan>, Vec<InlineBoxSpan>), CompileError> {
    let num_consumer_children = consumer_child_entries.len();
    let mut full_text = String::new();
    let mut span_text_spans: Vec<TextSpan> = Vec::new();
    let mut span_inline_boxes: Vec<InlineBoxSpan> = Vec::new();
    let mut last_consumer_child_id = None;

    for (child_idx, entry) in consumer_child_entries.iter().enumerate() {
        if let Some(chunk) = &entry.raw_text {
            let start = full_text.len();
            full_text.push_str(&chunk.text);
            let end = full_text.len();
            if start < end {
                span_text_spans.push(TextSpan::new(start..end).with_node(ctx.comp_node_id));
            }
            continue;
        }
        let has_next_consumer_child = child_idx + 1 < num_consumer_children;
        let child_elem = &entry.elem;
        let var_name = &entry.var_name;
        let is_child_span = registry
            .get(child_elem.name.as_str())
            .is_some_and(|defs| defs.iter().any(|d| d.kind == ComponentKind::Span))
            || child_elem.name.as_str() == "Link";
        let is_child_inline_box = registry
            .get(child_elem.name.as_str())
            .is_some_and(|defs| defs.iter().any(|d| d.kind == ComponentKind::InlineBox));
        if !is_child_span && !is_child_inline_box && child_elem.name.as_str() != "Font" {
            return Err(CompileError::Custom {
                message: format!(
                    "Cannot place non-span component '{}' inside inline \\Span '{}'; only plain text, nested \\Span, and \\InlineBox components are allowed",
                    child_elem.name.as_str(),
                    comp_def.name.as_str()
                ),
                span: child_elem.span,
            });
        }
        let mut entry_env_scope = entry.caller_env_scope.clone();
        for (k, v) in children_env_scope {
            entry_env_scope.insert(k.clone(), v.clone());
        }
        let elem_ctx = ElementContext {
            parent_id: Some(ctx.comp_node_id),
            parent_name: Some(comp_def.name.as_str()),
            prev_sibling_id: last_consumer_child_id,
            has_next_sibling: has_next_consumer_child,
            parent_ports: comp_scope_ports,
            lexical_scope: entry.caller_lexical_scope,
            env_scope: &entry_env_scope,
            enclosing_component_id: entry.caller_enclosing_component_id,
            enclosing_num_children: entry.caller_enclosing_num_children,
            enclosing_instantiated_children: entry.caller_enclosing_instantiated_children,
            shadowed_ports: entry.caller_shadowed_ports,
            is_let: var_name.is_some(),
            ambient_authored_ports: None,
            enums: ctx.enums,
            active_scope_id: ctx.active_scope_id,
            consumer_children: None,
        };
        let child_id = expand_element(child_elem, &elem_ctx, registry, doc, node_fonts)?;
        if let Some(v_name) = var_name {
            doc.get_node_mut(child_id).unwrap().var_name = Some(v_name.clone());
            entry
                .caller_lexical_scope
                .borrow_mut()
                .insert(v_name.clone(), LexicalBinding::Node(child_id));
        }
        if child_elem.name.as_str() == "Font" {
            node_fonts.insert(child_id, child_id);
            all_children_ids.push(child_id);
            last_consumer_child_id = Some(child_id);
            continue;
        }
        for inst_list in &entry.instantiated_ids {
            inst_list.borrow_mut().push(child_id);
        }
        all_children_ids.push(child_id);
        last_consumer_child_id = Some(child_id);

        if doc.nodes[child_id.0].is_inline_box {
            span_inline_boxes.push(InlineBoxSpan {
                node_id: child_id,
                index: full_text.len(),
                width: 0.0,
                height: 0.0,
                baseline_offset: 0.0,
            });
            continue;
        }

        let child_text = doc.nodes[child_id.0]
            .text_content
            .clone()
            .unwrap_or_default();
        let start = full_text.len();
        full_text.push_str(&child_text);
        let end = full_text.len();

        for mut sub_ib in doc.nodes[child_id.0].inline_boxes.clone() {
            sub_ib.index += start;
            span_inline_boxes.push(sub_ib);
        }

        if !doc.nodes[child_id.0].text_spans.is_empty() {
            let sub_spans = doc.nodes[child_id.0].text_spans.clone();
            for mut sub_span in sub_spans {
                sub_span.range = (start + sub_span.range.start)..(start + sub_span.range.end);
                sub_span.push_parent_node(ctx.comp_node_id);
                span_text_spans.push(sub_span);
            }
        } else if start < end {
            let mut sub_span = TextSpan::new(start..end).with_node(child_id);
            sub_span.push_parent_node(ctx.comp_node_id);
            span_text_spans.push(sub_span);
        }
    }

    Ok((full_text, span_text_spans, span_inline_boxes))
}
