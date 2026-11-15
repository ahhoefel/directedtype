use crate::ast::*;
use crate::compiler::error::CompileError;
use crate::compiler::expanded::{ExpandedDocument, ExpandedNode, NodeId};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

thread_local! {
    static AGG_EMITTED_COUNTS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

pub(super) fn reset_agg_emitted_count(comp_id: NodeId) {
    AGG_EMITTED_COUNTS.with(|cell| {
        let mut counts = cell.borrow_mut();
        if comp_id.0 >= counts.len() {
            counts.resize(comp_id.0 + 1, 0);
        } else {
            counts[comp_id.0] = 0;
        }
    });
}

pub(super) fn record_agg_emitted(comp_id: NodeId) {
    AGG_EMITTED_COUNTS.with(|cell| {
        let mut counts = cell.borrow_mut();
        if comp_id.0 >= counts.len() {
            counts.resize(comp_id.0 + 1, 0);
        }
        counts[comp_id.0] += 1;
    });
}

pub(super) fn get_agg_emitted_count(comp_id: NodeId) -> u32 {
    AGG_EMITTED_COUNTS.with(|cell| cell.borrow().get(comp_id.0).copied().unwrap_or(0))
}

fn collect_agg_members_and_parent_refs(
    expr: &Expr,
    parent_id: NodeId,
    agg_members: &mut HashSet<String>,
    parent_refs: &mut HashSet<String>,
) {
    match expr {
        Expr::MemberAccess(m) => {
            if let Expr::Ident(id) = m.target.as_ref() {
                if id.agg_child_of_node().is_some_and(|(p, _)| p == parent_id) {
                    agg_members.insert(m.member.as_str().to_string());
                } else if id
                    .node_id()
                    .is_some_and(|n| n == parent_id || n.is_template_self())
                {
                    parent_refs.insert(m.member.as_str().to_string());
                }
            }
            collect_agg_members_and_parent_refs(&m.target, parent_id, agg_members, parent_refs);
        }
        Expr::Call(c) => {
            for a in &c.args {
                collect_agg_members_and_parent_refs(a, parent_id, agg_members, parent_refs);
            }
        }
        Expr::MethodCall(mc) => {
            collect_agg_members_and_parent_refs(&mc.target, parent_id, agg_members, parent_refs);
            for a in &mc.args {
                collect_agg_members_and_parent_refs(a, parent_id, agg_members, parent_refs);
            }
        }
        Expr::Binary(b) => {
            collect_agg_members_and_parent_refs(&b.left, parent_id, agg_members, parent_refs);
            collect_agg_members_and_parent_refs(&b.right, parent_id, agg_members, parent_refs);
        }
        Expr::Unary(u) => {
            collect_agg_members_and_parent_refs(&u.operand, parent_id, agg_members, parent_refs);
        }
        Expr::Ternary(t) => {
            collect_agg_members_and_parent_refs(&t.condition, parent_id, agg_members, parent_refs);
            collect_agg_members_and_parent_refs(&t.then_expr, parent_id, agg_members, parent_refs);
            collect_agg_members_and_parent_refs(&t.else_expr, parent_id, agg_members, parent_refs);
        }
        Expr::Paren(inner, _) => {
            collect_agg_members_and_parent_refs(inner, parent_id, agg_members, parent_refs);
        }
        Expr::Literal(_) | Expr::Ident(_) | Expr::Node(_) | Expr::ExprDef(_) | Expr::Match(_) => {}
    }
}

fn scan_expr_for_parent_or_local_deps<'a>(
    expr: &'a Expr,
    curr_id: NodeId,
    parent_id: NodeId,
    target_member: &str,
    parent_ports_for_member: &HashSet<String>,
    child_id: NodeId,
    stack: &mut Vec<(NodeId, &'a str)>,
) -> bool {
    match expr {
        Expr::MemberAccess(ma) => {
            if let Expr::Ident(id) = ma.target.as_ref() {
                if let Some(raw_node_id) = id.node_id() {
                    let target_node_id = if raw_node_id.is_template_self() {
                        curr_id
                    } else {
                        raw_node_id
                    };
                    if target_node_id == parent_id {
                        if parent_ports_for_member.contains(ma.member.as_str()) {
                            return true;
                        }
                    } else if target_node_id.0 >= child_id.0 {
                        stack.push((target_node_id, ma.member.as_str()));
                    }
                } else if id.agg_child_of_node().is_some_and(|(p, _)| p == parent_id)
                    && ma.member.as_str() == target_member
                {
                    return true;
                }
            }
            scan_expr_for_parent_or_local_deps(
                &ma.target,
                curr_id,
                parent_id,
                target_member,
                parent_ports_for_member,
                child_id,
                stack,
            )
        }
        Expr::Call(c) => c.args.iter().any(|a| {
            scan_expr_for_parent_or_local_deps(
                a,
                curr_id,
                parent_id,
                target_member,
                parent_ports_for_member,
                child_id,
                stack,
            )
        }),
        Expr::MethodCall(mc) => {
            scan_expr_for_parent_or_local_deps(
                &mc.target,
                curr_id,
                parent_id,
                target_member,
                parent_ports_for_member,
                child_id,
                stack,
            ) || mc.args.iter().any(|a| {
                scan_expr_for_parent_or_local_deps(
                    a,
                    curr_id,
                    parent_id,
                    target_member,
                    parent_ports_for_member,
                    child_id,
                    stack,
                )
            })
        }
        Expr::Binary(b) => {
            scan_expr_for_parent_or_local_deps(
                &b.left,
                curr_id,
                parent_id,
                target_member,
                parent_ports_for_member,
                child_id,
                stack,
            ) || scan_expr_for_parent_or_local_deps(
                &b.right,
                curr_id,
                parent_id,
                target_member,
                parent_ports_for_member,
                child_id,
                stack,
            )
        }
        Expr::Unary(u) => scan_expr_for_parent_or_local_deps(
            &u.operand,
            curr_id,
            parent_id,
            target_member,
            parent_ports_for_member,
            child_id,
            stack,
        ),
        Expr::Ternary(t) => {
            scan_expr_for_parent_or_local_deps(
                &t.condition,
                curr_id,
                parent_id,
                target_member,
                parent_ports_for_member,
                child_id,
                stack,
            ) || scan_expr_for_parent_or_local_deps(
                &t.then_expr,
                curr_id,
                parent_id,
                target_member,
                parent_ports_for_member,
                child_id,
                stack,
            ) || scan_expr_for_parent_or_local_deps(
                &t.else_expr,
                curr_id,
                parent_id,
                target_member,
                parent_ports_for_member,
                child_id,
                stack,
            )
        }
        Expr::Paren(inner, _) => scan_expr_for_parent_or_local_deps(
            inner,
            curr_id,
            parent_id,
            target_member,
            parent_ports_for_member,
            child_id,
            stack,
        ),
        Expr::Literal(_) | Expr::Ident(_) | Expr::Node(_) | Expr::ExprDef(_) | Expr::Match(_) => {
            false
        }
    }
}

fn child_port_depends_on_parent<'a>(
    child_id: NodeId,
    member: &'a str,
    parent_id: NodeId,
    parent_ports_for_member: &HashSet<String>,
    nodes: &'a [ExpandedNode],
) -> bool {
    let mut visited: HashSet<(NodeId, &'a str)> = HashSet::new();
    let mut stack: Vec<(NodeId, &'a str)> = Vec::new();
    stack.push((child_id, member));

    while let Some((curr_id, curr_port)) = stack.pop() {
        if !visited.insert((curr_id, curr_port)) {
            continue;
        }
        let Some(node) = nodes.get(curr_id.0) else {
            continue;
        };
        if node.aggregated_ports.contains(curr_port) {
            continue;
        }
        let mut exprs_to_scan: [Option<&Expr>; 2] = [None, None];
        if let Some(e) = node.ports.get(curr_port) {
            exprs_to_scan[0] = Some(e);
        } else if curr_port == "right" {
            exprs_to_scan[0] = node.ports.get("x");
            exprs_to_scan[1] = node.ports.get("width");
        } else if curr_port == "bottom" {
            exprs_to_scan[0] = node.ports.get("y");
            exprs_to_scan[1] = node.ports.get("height");
        }

        for expr in exprs_to_scan.into_iter().flatten() {
            if scan_expr_for_parent_or_local_deps(
                expr,
                curr_id,
                parent_id,
                member,
                parent_ports_for_member,
                child_id,
                &mut stack,
            ) {
                return true;
            }
        }
    }
    false
}

#[allow(clippy::too_many_arguments, clippy::only_used_in_recursion)]
fn resolve_and_filter_agg_expr(
    expr: &Expr,
    comp_id: NodeId,
    comp_name: &str,
    instantiated_children_ids: &[NodeId],
    parent_ports_by_member: &HashMap<String, HashSet<String>>,
    nodes: &[ExpandedNode],
    skipped_out: &mut Vec<crate::compiler::graph::VarId>,
) -> Result<(Expr, Expr), CompileError> {
    match expr {
        Expr::Call(call) => {
            let allow_filter = call.callee.name != Symbol::ONLY || instantiated_children_ids.len() == 1;
            let mut new_args = Vec::with_capacity(call.args.len());
            let mut unfiltered_args = Vec::with_capacity(call.args.len());
            let mut call_skipped = Vec::new();
            let mut had_filtered = false;

            for arg in &call.args {
                if let Expr::MemberAccess(m) = arg {
                    if let Expr::Ident(id) = m.target.as_ref() {
                        if let Some((agg_parent, idx)) = id.agg_child_of_node() {
                            if agg_parent == comp_id {
                                if let Some(&child_id) = instantiated_children_ids.get(idx) {
                                    let member = m.member.as_str();
                                    let resolved_arg = Expr::MemberAccess(MemberAccessExpr {
                                        target: std::sync::Arc::new(Expr::Ident(Ident::from_node(
                                            child_id,
                                            id.span,
                                        ))),
                                        member: m.member,
                                        span: m.span,
                                    });
                                    unfiltered_args.push(resolved_arg.clone());

                                    if allow_filter {
                                        if let Some(parent_ports) = parent_ports_by_member.get(member) {
                                            if !parent_ports.is_empty()
                                                && child_port_depends_on_parent(
                                                    child_id,
                                                    member,
                                                    comp_id,
                                                    parent_ports,
                                                    nodes,
                                                )
                                            {
                                                had_filtered = true;
                                                call_skipped.push(crate::compiler::graph::VarId::from_symbol(
                                                    child_id, m.member.name,
                                                ));
                                                continue;
                                            }
                                        }
                                    }

                                    new_args.push(resolved_arg);
                                    continue;
                                }
                            }
                        }
                    }
                }
                let (filt, unfilt) = resolve_and_filter_agg_expr(
                    arg,
                    comp_id,
                    comp_name,
                    instantiated_children_ids,
                    parent_ports_by_member,
                    nodes,
                    skipped_out,
                )?;
                unfiltered_args.push(unfilt);
                new_args.push(filt);
            }

            let final_args = if had_filtered && new_args.is_empty() {
                unfiltered_args.clone()
            } else {
                for v in call_skipped {
                    if !skipped_out.contains(&v) {
                        skipped_out.push(v);
                    }
                }
                new_args
            };

            Ok((
                Expr::Call(CallExpr {
                    callee: call.callee,
                    args: final_args,
                    span: call.span,
                }),
                Expr::Call(CallExpr {
                    callee: call.callee,
                    args: unfiltered_args,
                    span: call.span,
                }),
            ))
        }
        Expr::MemberAccess(m) => {
            let (new_target, unfilt_target) = if let Expr::Ident(id) = m.target.as_ref() {
                if let Some((agg_parent, idx)) = id.agg_child_of_node() {
                    if agg_parent == comp_id {
                        if let Some(&child_id) = instantiated_children_ids.get(idx) {
                            let resolved = Expr::Ident(Ident::from_node(child_id, id.span));
                            (resolved.clone(), resolved)
                        } else {
                            let orig = m.target.as_ref().clone();
                            (orig.clone(), orig)
                        }
                    } else {
                        let orig = m.target.as_ref().clone();
                        (orig.clone(), orig)
                    }
                } else {
                    let orig = m.target.as_ref().clone();
                    (orig.clone(), orig)
                }
            } else {
                resolve_and_filter_agg_expr(
                    &m.target,
                    comp_id,
                    comp_name,
                    instantiated_children_ids,
                    parent_ports_by_member,
                    nodes,
                    skipped_out,
                )?
            };
            Ok((
                Expr::MemberAccess(MemberAccessExpr {
                    target: std::sync::Arc::new(new_target),
                    member: m.member,
                    span: m.span,
                }),
                Expr::MemberAccess(MemberAccessExpr {
                    target: std::sync::Arc::new(unfilt_target),
                    member: m.member,
                    span: m.span,
                }),
            ))
        }
        Expr::MethodCall(mc) => {
            let (new_target, unfilt_target) = resolve_and_filter_agg_expr(
                &mc.target,
                comp_id,
                comp_name,
                instantiated_children_ids,
                parent_ports_by_member,
                nodes,
                skipped_out,
            )?;
            let mut new_args = Vec::with_capacity(mc.args.len());
            let mut unfilt_args = Vec::with_capacity(mc.args.len());
            for a in &mc.args {
                let (filt, unfilt) = resolve_and_filter_agg_expr(
                    a,
                    comp_id,
                    comp_name,
                    instantiated_children_ids,
                    parent_ports_by_member,
                    nodes,
                    skipped_out,
                )?;
                new_args.push(filt);
                unfilt_args.push(unfilt);
            }
            Ok((
                Expr::MethodCall(MethodCallExpr {
                    target: std::sync::Arc::new(new_target),
                    method: mc.method,
                    args: new_args,
                    span: mc.span,
                }),
                Expr::MethodCall(MethodCallExpr {
                    target: std::sync::Arc::new(unfilt_target),
                    method: mc.method,
                    args: unfilt_args,
                    span: mc.span,
                }),
            ))
        }
        Expr::Binary(b) => {
            let (l_filt, l_unfilt) = resolve_and_filter_agg_expr(
                &b.left,
                comp_id,
                comp_name,
                instantiated_children_ids,
                parent_ports_by_member,
                nodes,
                skipped_out,
            )?;
            let (r_filt, r_unfilt) = resolve_and_filter_agg_expr(
                &b.right,
                comp_id,
                comp_name,
                instantiated_children_ids,
                parent_ports_by_member,
                nodes,
                skipped_out,
            )?;
            Ok((
                Expr::Binary(BinaryExpr {
                    op: b.op,
                    left: std::sync::Arc::new(l_filt),
                    right: std::sync::Arc::new(r_filt),
                    span: b.span,
                }),
                Expr::Binary(BinaryExpr {
                    op: b.op,
                    left: std::sync::Arc::new(l_unfilt),
                    right: std::sync::Arc::new(r_unfilt),
                    span: b.span,
                }),
            ))
        }
        Expr::Unary(u) => {
            let (filt, unfilt) = resolve_and_filter_agg_expr(
                &u.operand,
                comp_id,
                comp_name,
                instantiated_children_ids,
                parent_ports_by_member,
                nodes,
                skipped_out,
            )?;
            Ok((
                Expr::Unary(UnaryExpr {
                    op: u.op,
                    operand: std::sync::Arc::new(filt),
                    span: u.span,
                }),
                Expr::Unary(UnaryExpr {
                    op: u.op,
                    operand: std::sync::Arc::new(unfilt),
                    span: u.span,
                }),
            ))
        }
        Expr::Ternary(t) => {
            let (c_filt, c_unfilt) = resolve_and_filter_agg_expr(
                &t.condition,
                comp_id,
                comp_name,
                instantiated_children_ids,
                parent_ports_by_member,
                nodes,
                skipped_out,
            )?;
            let (t_filt, t_unfilt) = resolve_and_filter_agg_expr(
                &t.then_expr,
                comp_id,
                comp_name,
                instantiated_children_ids,
                parent_ports_by_member,
                nodes,
                skipped_out,
            )?;
            let (e_filt, e_unfilt) = resolve_and_filter_agg_expr(
                &t.else_expr,
                comp_id,
                comp_name,
                instantiated_children_ids,
                parent_ports_by_member,
                nodes,
                skipped_out,
            )?;
            Ok((
                Expr::Ternary(TernaryExpr {
                    condition: std::sync::Arc::new(c_filt),
                    then_expr: std::sync::Arc::new(t_filt),
                    else_expr: std::sync::Arc::new(e_filt),
                    span: t.span,
                }),
                Expr::Ternary(TernaryExpr {
                    condition: std::sync::Arc::new(c_unfilt),
                    then_expr: std::sync::Arc::new(t_unfilt),
                    else_expr: std::sync::Arc::new(e_unfilt),
                    span: t.span,
                }),
            ))
        }
        Expr::Paren(inner, span) => {
            let (filt, unfilt) = resolve_and_filter_agg_expr(
                inner,
                comp_id,
                comp_name,
                instantiated_children_ids,
                parent_ports_by_member,
                nodes,
                skipped_out,
            )?;
            Ok((
                Expr::Paren(std::sync::Arc::new(filt), *span),
                Expr::Paren(std::sync::Arc::new(unfilt), *span),
            ))
        }
        Expr::Literal(_) | Expr::Ident(_) | Expr::Node(_) | Expr::ExprDef(_) | Expr::Match(_) => {
            Ok((expr.clone(), expr.clone()))
        }
    }
}

fn expr_contains_agg_child_of(expr: &Expr, comp_id: NodeId) -> bool {
    match expr {
        Expr::Ident(id) => id.agg_child_of_node().is_some_and(|(p, _)| p == comp_id),
        Expr::MemberAccess(m) => expr_contains_agg_child_of(&m.target, comp_id),
        Expr::Call(c) => c
            .args
            .iter()
            .any(|a| expr_contains_agg_child_of(a, comp_id)),
        Expr::MethodCall(mc) => {
            expr_contains_agg_child_of(&mc.target, comp_id)
                || mc
                    .args
                    .iter()
                    .any(|a| expr_contains_agg_child_of(a, comp_id))
        }
        Expr::Binary(b) => {
            expr_contains_agg_child_of(&b.left, comp_id)
                || expr_contains_agg_child_of(&b.right, comp_id)
        }
        Expr::Unary(u) => expr_contains_agg_child_of(&u.operand, comp_id),
        Expr::Ternary(t) => {
            expr_contains_agg_child_of(&t.condition, comp_id)
                || expr_contains_agg_child_of(&t.then_expr, comp_id)
                || expr_contains_agg_child_of(&t.else_expr, comp_id)
        }
        Expr::Paren(inner, _) => expr_contains_agg_child_of(inner, comp_id),
        Expr::Literal(_) | Expr::Node(_) | Expr::ExprDef(_) | Expr::Match(_) => false,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_and_filter_component_aggregations(
    comp_id: NodeId,
    comp_name: &str,
    instantiated_children_ids: &[NodeId],
    rewritten_ports: &mut HashMap<String, Expr>,
    doc: &mut ExpandedDocument,
    start_node_idx: usize,
    start_local_bindings_idx: usize,
    body_agg_count: u32,
    total_agg_count: u32,
) -> Result<(), CompileError> {
    if total_agg_count == 0 {
        return Ok(());
    }

    let mut parent_ports_by_member: HashMap<String, HashSet<String>> = HashMap::new();
    let mut port_to_parent_refs: Vec<(String, HashSet<String>)> = Vec::new();
    let mut agg_port_names: Vec<String> = Vec::new();

    for (port_name, expr) in rewritten_ports.iter() {
        let mut agg_members = HashSet::new();
        let mut parent_refs = HashSet::new();
        collect_agg_members_and_parent_refs(
            expr,
            comp_id,
            &mut agg_members,
            &mut parent_refs,
        );
        if !agg_members.is_empty() {
            agg_port_names.push(port_name.clone());
            for m in agg_members {
                let entry = parent_ports_by_member.entry(m).or_default();
                entry.insert(port_name.clone());
            }
        }
        if !parent_refs.is_empty() {
            port_to_parent_refs.push((port_name.clone(), parent_refs));
        }
    }

    let mut changed = true;
    while changed {
        changed = false;
        for (port_name, refs) in &port_to_parent_refs {
            for set in parent_ports_by_member.values_mut() {
                if refs.iter().any(|r| set.contains(r)) {
                    if set.insert(port_name.clone()) {
                        changed = true;
                    }
                    if port_name == "width" && set.insert("right".to_string()) {
                        changed = true;
                    }
                    if port_name == "height" && set.insert("bottom".to_string()) {
                        changed = true;
                    }
                }
            }
        }
    }

    let mut all_aggregated_ports = HashSet::new();
    for set in parent_ports_by_member.values() {
        for p in set {
            all_aggregated_ports.insert(p.clone());
        }
    }

    if let Some(w_set) = parent_ports_by_member.get_mut("width") {
        if w_set.contains("width") {
            w_set.insert("right".to_string());
        }
    }
    if let Some(h_set) = parent_ports_by_member.get_mut("height") {
        if h_set.contains("height") {
            h_set.insert("bottom".to_string());
        }
    }

    let mut comp_unfiltered_ports = HashMap::new();
    let mut comp_skipped_upstream = HashMap::new();
    for port_name in agg_port_names {
        let Some(expr) = rewritten_ports.get_mut(&port_name) else {
            continue;
        };
        let mut skipped_vars = Vec::new();
        let (filt, unfilt) = resolve_and_filter_agg_expr(
            expr,
            comp_id,
            comp_name,
            instantiated_children_ids,
            &parent_ports_by_member,
            &doc.nodes,
            &mut skipped_vars,
        )?;
        *expr = filt;
        if !skipped_vars.is_empty() {
            comp_unfiltered_ports.insert(port_name.clone(), unfilt);
            comp_skipped_upstream.insert(port_name, skipped_vars);
        }
    }

    if let Some(node) = doc.get_node_mut(comp_id) {
        node.aggregated_ports = all_aggregated_ports;
        node.unfiltered_ports = comp_unfiltered_ports;
        node.skipped_upstream = comp_skipped_upstream;
    }

    if body_agg_count > 0 {
        let mut node_updates = Vec::new();
        for node_idx in start_node_idx..doc.nodes.len() {
            for (port_name, expr) in &doc.nodes[node_idx].ports {
                if expr_contains_agg_child_of(expr, comp_id) {
                    let mut skipped_vars = Vec::new();
                    let (filt, unfilt) = resolve_and_filter_agg_expr(
                        expr,
                        comp_id,
                        comp_name,
                        instantiated_children_ids,
                        &parent_ports_by_member,
                        &doc.nodes,
                        &mut skipped_vars,
                    )?;
                    node_updates.push((node_idx, port_name.clone(), filt, unfilt, skipped_vars));
                }
            }
        }
        for (node_idx, port_name, filt, unfilt, skipped_vars) in node_updates {
            let node = &mut doc.nodes[node_idx];
            node.ports.insert(port_name.clone(), filt);
            if !skipped_vars.is_empty() {
                node.unfiltered_ports.insert(port_name.clone(), unfilt);
                node.skipped_upstream.insert(port_name, skipped_vars);
            }
        }

        for binding_idx in start_local_bindings_idx..doc.local_bindings.len() {
            if !expr_contains_agg_child_of(&doc.local_bindings[binding_idx].0, comp_id) {
                continue;
            }
            let mut skipped_vars = Vec::new();
            let (filt, _) = resolve_and_filter_agg_expr(
                &doc.local_bindings[binding_idx].0,
                comp_id,
                comp_name,
                instantiated_children_ids,
                &parent_ports_by_member,
                &doc.nodes,
                &mut skipped_vars,
            )?;
            doc.local_bindings[binding_idx].0 = filt;
        }
    }

    Ok(())
}

