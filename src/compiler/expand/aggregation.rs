use crate::ast::*;
use crate::compiler::error::CompileError;
use crate::compiler::eval::eval_expr;
use crate::compiler::expanded::{ExpandedDocument, ExpandedNode, NodeId};
use std::collections::{HashMap, HashSet};

fn collect_agg_members_and_parent_refs(
    expr: &Expr,
    agg_prefix: &str,
    parent_canon: &str,
    agg_members: &mut HashSet<String>,
    parent_refs: &mut HashSet<String>,
) {
    match expr {
        Expr::MemberAccess(m) => {
            if let Expr::Ident(id) = m.target.as_ref() {
                if id.as_str().starts_with(agg_prefix) {
                    agg_members.insert(m.member.as_str().to_string());
                } else if id.as_str() == parent_canon {
                    parent_refs.insert(m.member.as_str().to_string());
                }
            }
            collect_agg_members_and_parent_refs(&m.target, agg_prefix, parent_canon, agg_members, parent_refs);
        }
        Expr::Call(c) => {
            for a in &c.args {
                collect_agg_members_and_parent_refs(a, agg_prefix, parent_canon, agg_members, parent_refs);
            }
        }
        Expr::MethodCall(mc) => {
            collect_agg_members_and_parent_refs(&mc.target, agg_prefix, parent_canon, agg_members, parent_refs);
            for a in &mc.args {
                collect_agg_members_and_parent_refs(a, agg_prefix, parent_canon, agg_members, parent_refs);
            }
        }
        Expr::Binary(b) => {
            collect_agg_members_and_parent_refs(&b.left, agg_prefix, parent_canon, agg_members, parent_refs);
            collect_agg_members_and_parent_refs(&b.right, agg_prefix, parent_canon, agg_members, parent_refs);
        }
        Expr::Unary(u) => {
            collect_agg_members_and_parent_refs(&u.operand, agg_prefix, parent_canon, agg_members, parent_refs);
        }
        Expr::Ternary(t) => {
            collect_agg_members_and_parent_refs(&t.condition, agg_prefix, parent_canon, agg_members, parent_refs);
            collect_agg_members_and_parent_refs(&t.then_expr, agg_prefix, parent_canon, agg_members, parent_refs);
            collect_agg_members_and_parent_refs(&t.else_expr, agg_prefix, parent_canon, agg_members, parent_refs);
        }
        Expr::Paren(inner, _) => {
            collect_agg_members_and_parent_refs(inner, agg_prefix, parent_canon, agg_members, parent_refs);
        }
        Expr::Literal(_) | Expr::Ident(_) | Expr::Node(_) => {}
    }
}

fn scan_expr_for_parent_or_local_deps(
    expr: &Expr,
    parent_canon: &str,
    agg_prefix: &str,
    target_member: &str,
    parent_ports_for_member: &HashSet<String>,
    child_id: NodeId,
    stack: &mut Vec<(NodeId, String)>,
) -> bool {
    match expr {
        Expr::MemberAccess(ma) => {
            if let Expr::Ident(id) = ma.target.as_ref() {
                if id.as_str() == parent_canon {
                    if parent_ports_for_member.contains(ma.member.as_str()) {
                        return true;
                    }
                } else if id.as_str().starts_with(agg_prefix) && ma.member.as_str() == target_member {
                    return true;
                } else if let Some(target_node_id) = NodeId::from_canonical_name(id.as_str()) {
                    if target_node_id.0 >= child_id.0 {
                        stack.push((target_node_id, ma.member.as_str().to_string()));
                    }
                }
            }
            scan_expr_for_parent_or_local_deps(
                &ma.target,
                parent_canon,
                agg_prefix,
                target_member,
                parent_ports_for_member,
                child_id,
                stack,
            )
        }
        Expr::Call(c) => c.args.iter().any(|a| {
            scan_expr_for_parent_or_local_deps(
                a,
                parent_canon,
                agg_prefix,
                target_member,
                parent_ports_for_member,
                child_id,
                stack,
            )
        }),
        Expr::MethodCall(mc) => {
            scan_expr_for_parent_or_local_deps(
                &mc.target,
                parent_canon,
                agg_prefix,
                target_member,
                parent_ports_for_member,
                child_id,
                stack,
            ) || mc.args.iter().any(|a| {
                scan_expr_for_parent_or_local_deps(
                    a,
                    parent_canon,
                    agg_prefix,
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
                parent_canon,
                agg_prefix,
                target_member,
                parent_ports_for_member,
                child_id,
                stack,
            ) || scan_expr_for_parent_or_local_deps(
                &b.right,
                parent_canon,
                agg_prefix,
                target_member,
                parent_ports_for_member,
                child_id,
                stack,
            )
        }
        Expr::Unary(u) => scan_expr_for_parent_or_local_deps(
            &u.operand,
            parent_canon,
            agg_prefix,
            target_member,
            parent_ports_for_member,
            child_id,
            stack,
        ),
        Expr::Ternary(t) => {
            scan_expr_for_parent_or_local_deps(
                &t.condition,
                parent_canon,
                agg_prefix,
                target_member,
                parent_ports_for_member,
                child_id,
                stack,
            ) || scan_expr_for_parent_or_local_deps(
                &t.then_expr,
                parent_canon,
                agg_prefix,
                target_member,
                parent_ports_for_member,
                child_id,
                stack,
            ) || scan_expr_for_parent_or_local_deps(
                &t.else_expr,
                parent_canon,
                agg_prefix,
                target_member,
                parent_ports_for_member,
                child_id,
                stack,
            )
        }
        Expr::Paren(inner, _) => scan_expr_for_parent_or_local_deps(
            inner,
            parent_canon,
            agg_prefix,
            target_member,
            parent_ports_for_member,
            child_id,
            stack,
        ),
        Expr::Literal(_) | Expr::Ident(_) | Expr::Node(_) => false,
    }
}

fn child_port_depends_on_parent(
    child_id: NodeId,
    member: &str,
    parent_id: NodeId,
    agg_prefix: &str,
    parent_ports_for_member: &HashSet<String>,
    nodes: &[ExpandedNode],
) -> bool {
    let parent_canon = parent_id.canonical_name();
    let mut visited: HashSet<(NodeId, String)> = HashSet::new();
    let mut stack: Vec<(NodeId, String)> = Vec::new();
    stack.push((child_id, member.to_string()));

    while let Some((curr_id, curr_port)) = stack.pop() {
        if !visited.insert((curr_id, curr_port.clone())) {
            continue;
        }
        let Some(node) = nodes.get(curr_id.0) else {
            continue;
        };
        if node.aggregated_ports.contains(&curr_port) {
            continue;
        }
        let mut exprs_to_scan = Vec::new();
        if let Some(e) = node.ports.get(&curr_port) {
            exprs_to_scan.push(e);
        } else if curr_port == "right" {
            if let Some(e) = node.ports.get("x") {
                exprs_to_scan.push(e);
            }
            if let Some(e) = node.ports.get("width") {
                exprs_to_scan.push(e);
            }
        } else if curr_port == "bottom" {
            if let Some(e) = node.ports.get("y") {
                exprs_to_scan.push(e);
            }
            if let Some(e) = node.ports.get("height") {
                exprs_to_scan.push(e);
            }
        }

        for expr in exprs_to_scan {
            if scan_expr_for_parent_or_local_deps(
                expr,
                &parent_canon,
                agg_prefix,
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
    agg_prefix: &str,
    instantiated_children_ids: &[NodeId],
    parent_ports_by_member: &HashMap<String, HashSet<String>>,
    nodes: &[ExpandedNode],
    skipped_out: &mut Vec<crate::compiler::graph::VarId>,
) -> Result<(Expr, Expr), CompileError> {
    match expr {
        Expr::Call(call) => {
            let allow_filter = call.callee.as_str() != "only" || instantiated_children_ids.len() == 1;
            let mut new_args = Vec::with_capacity(call.args.len());
            let mut unfiltered_args = Vec::with_capacity(call.args.len());
            let mut call_skipped = Vec::new();
            let mut had_filtered = false;

            for arg in &call.args {
                if let Expr::MemberAccess(m) = arg {
                    if let Expr::Ident(id) = m.target.as_ref() {
                        if let Some(idx_str) = id.as_str().strip_prefix(agg_prefix) {
                            if let Ok(idx) = idx_str.parse::<usize>() {
                                if let Some(&child_id) = instantiated_children_ids.get(idx) {
                                    let member = m.member.as_str();
                                    let resolved_arg = Expr::MemberAccess(MemberAccessExpr {
                                        target: Box::new(Expr::Ident(Ident::new(
                                            child_id.canonical_name(),
                                            id.span,
                                        ))),
                                        member: m.member.clone(),
                                        span: m.span,
                                    });
                                    unfiltered_args.push(resolved_arg.clone());

                                    if allow_filter {
                                        if let Some(base_parent_ports) = parent_ports_by_member.get(member) {
                                            if !base_parent_ports.is_empty() {
                                                let mut parent_ports = base_parent_ports.clone();
                                                if member == "width" && parent_ports.contains("width") {
                                                    parent_ports.insert("right".to_string());
                                                } else if member == "height" && parent_ports.contains("height") {
                                                    parent_ports.insert("bottom".to_string());
                                                }

                                                if child_port_depends_on_parent(
                                                    child_id,
                                                    member,
                                                    comp_id,
                                                    agg_prefix,
                                                    &parent_ports,
                                                    nodes,
                                                ) {
                                                    had_filtered = true;
                                                    call_skipped.push(crate::compiler::graph::VarId::new(
                                                        child_id, member,
                                                    ));
                                                    continue;
                                                }
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
                    agg_prefix,
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
                    callee: call.callee.clone(),
                    args: final_args,
                    span: call.span,
                }),
                Expr::Call(CallExpr {
                    callee: call.callee.clone(),
                    args: unfiltered_args,
                    span: call.span,
                }),
            ))
        }
        Expr::MemberAccess(m) => {
            let (new_target, unfilt_target) = if let Expr::Ident(id) = m.target.as_ref() {
                if let Some(idx_str) = id.as_str().strip_prefix(agg_prefix) {
                    if let Ok(idx) = idx_str.parse::<usize>() {
                        if let Some(&child_id) = instantiated_children_ids.get(idx) {
                            let resolved = Expr::Ident(Ident::new(child_id.canonical_name(), id.span));
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
                    agg_prefix,
                    instantiated_children_ids,
                    parent_ports_by_member,
                    nodes,
                    skipped_out,
                )?
            };
            Ok((
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(new_target),
                    member: m.member.clone(),
                    span: m.span,
                }),
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(unfilt_target),
                    member: m.member.clone(),
                    span: m.span,
                }),
            ))
        }
        Expr::MethodCall(mc) => {
            let (new_target, unfilt_target) = resolve_and_filter_agg_expr(
                &mc.target,
                comp_id,
                comp_name,
                agg_prefix,
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
                    agg_prefix,
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
                    target: Box::new(new_target),
                    method: mc.method.clone(),
                    args: new_args,
                    span: mc.span,
                }),
                Expr::MethodCall(MethodCallExpr {
                    target: Box::new(unfilt_target),
                    method: mc.method.clone(),
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
                agg_prefix,
                instantiated_children_ids,
                parent_ports_by_member,
                nodes,
                skipped_out,
            )?;
            let (r_filt, r_unfilt) = resolve_and_filter_agg_expr(
                &b.right,
                comp_id,
                comp_name,
                agg_prefix,
                instantiated_children_ids,
                parent_ports_by_member,
                nodes,
                skipped_out,
            )?;
            Ok((
                Expr::Binary(BinaryExpr {
                    op: b.op,
                    left: Box::new(l_filt),
                    right: Box::new(r_filt),
                    span: b.span,
                }),
                Expr::Binary(BinaryExpr {
                    op: b.op,
                    left: Box::new(l_unfilt),
                    right: Box::new(r_unfilt),
                    span: b.span,
                }),
            ))
        }
        Expr::Unary(u) => {
            let (filt, unfilt) = resolve_and_filter_agg_expr(
                &u.operand,
                comp_id,
                comp_name,
                agg_prefix,
                instantiated_children_ids,
                parent_ports_by_member,
                nodes,
                skipped_out,
            )?;
            Ok((
                Expr::Unary(UnaryExpr {
                    op: u.op,
                    operand: Box::new(filt),
                    span: u.span,
                }),
                Expr::Unary(UnaryExpr {
                    op: u.op,
                    operand: Box::new(unfilt),
                    span: u.span,
                }),
            ))
        }
        Expr::Ternary(t) => {
            let (c_filt, c_unfilt) = resolve_and_filter_agg_expr(
                &t.condition,
                comp_id,
                comp_name,
                agg_prefix,
                instantiated_children_ids,
                parent_ports_by_member,
                nodes,
                skipped_out,
            )?;
            let (t_filt, t_unfilt) = resolve_and_filter_agg_expr(
                &t.then_expr,
                comp_id,
                comp_name,
                agg_prefix,
                instantiated_children_ids,
                parent_ports_by_member,
                nodes,
                skipped_out,
            )?;
            let (e_filt, e_unfilt) = resolve_and_filter_agg_expr(
                &t.else_expr,
                comp_id,
                comp_name,
                agg_prefix,
                instantiated_children_ids,
                parent_ports_by_member,
                nodes,
                skipped_out,
            )?;
            Ok((
                Expr::Ternary(TernaryExpr {
                    condition: Box::new(c_filt),
                    then_expr: Box::new(t_filt),
                    else_expr: Box::new(e_filt),
                    span: t.span,
                }),
                Expr::Ternary(TernaryExpr {
                    condition: Box::new(c_unfilt),
                    then_expr: Box::new(t_unfilt),
                    else_expr: Box::new(e_unfilt),
                    span: t.span,
                }),
            ))
        }
        Expr::Paren(inner, span) => {
            let (filt, unfilt) = resolve_and_filter_agg_expr(
                inner,
                comp_id,
                comp_name,
                agg_prefix,
                instantiated_children_ids,
                parent_ports_by_member,
                nodes,
                skipped_out,
            )?;
            Ok((
                Expr::Paren(Box::new(filt), *span),
                Expr::Paren(Box::new(unfilt), *span),
            ))
        }
        Expr::Literal(_) | Expr::Ident(_) | Expr::Node(_) => Ok((expr.clone(), expr.clone())),
    }
}

fn fold_const_ternaries(
    expr: &Expr,
    const_env: &HashMap<crate::compiler::graph::VarId, crate::compiler::value::Value>,
) -> Expr {
    match expr {
        Expr::Ternary(t) => {
            let cond = fold_const_ternaries(&t.condition, const_env);
            let then_e = fold_const_ternaries(&t.then_expr, const_env);
            let else_e = fold_const_ternaries(&t.else_expr, const_env);
            if let Ok(val) = eval_expr(&cond, const_env) {
                if val.is_truthy() {
                    return then_e;
                } else {
                    return else_e;
                }
            }
            Expr::Ternary(TernaryExpr {
                condition: Box::new(cond),
                then_expr: Box::new(then_e),
                else_expr: Box::new(else_e),
                span: t.span,
            })
        }
        Expr::Binary(b) => Expr::Binary(BinaryExpr {
            op: b.op,
            left: Box::new(fold_const_ternaries(&b.left, const_env)),
            right: Box::new(fold_const_ternaries(&b.right, const_env)),
            span: b.span,
        }),
        Expr::Unary(u) => Expr::Unary(UnaryExpr {
            op: u.op,
            operand: Box::new(fold_const_ternaries(&u.operand, const_env)),
            span: u.span,
        }),
        Expr::Call(c) => Expr::Call(CallExpr {
            callee: c.callee.clone(),
            args: c.args.iter().map(|a| fold_const_ternaries(a, const_env)).collect(),
            span: c.span,
        }),
        Expr::MethodCall(mc) => Expr::MethodCall(MethodCallExpr {
            target: Box::new(fold_const_ternaries(&mc.target, const_env)),
            method: mc.method.clone(),
            args: mc.args.iter().map(|a| fold_const_ternaries(a, const_env)).collect(),
            span: mc.span,
        }),
        Expr::MemberAccess(m) => Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(fold_const_ternaries(&m.target, const_env)),
            member: m.member.clone(),
            span: m.span,
        }),
        Expr::Paren(inner, span) => {
            Expr::Paren(Box::new(fold_const_ternaries(inner, const_env)), *span)
        }
        Expr::Literal(_) | Expr::Ident(_) | Expr::Node(_) => expr.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_and_filter_component_aggregations(
    comp_id: NodeId,
    comp_name: &str,
    instantiated_children_ids: &[NodeId],
    declared_state_names: &HashSet<String>,
    rewritten_ports: &mut HashMap<String, Expr>,
    doc: &mut ExpandedDocument,
    start_node_idx: usize,
    start_local_bindings_idx: usize,
) -> Result<(), CompileError> {
    let parent_canon = comp_id.canonical_name();
    let agg_prefix = format!("__agg_child_of_{}_", parent_canon);

    let mut const_env: HashMap<crate::compiler::graph::VarId, crate::compiler::value::Value> =
        HashMap::new();
    for _ in 0..3 {
        for (port_name, port_expr) in rewritten_ports.iter() {
            if declared_state_names.contains(port_name) {
                continue;
            }
            if let Ok(val) = eval_expr(port_expr, &const_env) {
                const_env.insert(crate::compiler::graph::VarId::new(comp_id, port_name), val);
            }
        }
        for expr in rewritten_ports.values_mut() {
            *expr = fold_const_ternaries(expr, &const_env);
        }
    }

    let mut parent_ports_by_member: HashMap<String, HashSet<String>> = HashMap::new();
    let mut port_to_parent_refs: Vec<(String, HashSet<String>)> = Vec::new();

    for (port_name, expr) in rewritten_ports.iter() {
        let mut agg_members = HashSet::new();
        let mut parent_refs = HashSet::new();
        collect_agg_members_and_parent_refs(
            expr,
            &agg_prefix,
            &parent_canon,
            &mut agg_members,
            &mut parent_refs,
        );
        for m in agg_members {
            let entry = parent_ports_by_member.entry(m).or_default();
            entry.insert(port_name.clone());
        }
        port_to_parent_refs.push((port_name.clone(), parent_refs));
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

    let mut comp_unfiltered_ports = HashMap::new();
    let mut comp_skipped_upstream = HashMap::new();
    for (port_name, expr) in rewritten_ports.iter_mut() {
        let mut skipped_vars = Vec::new();
        let (filt, unfilt) = resolve_and_filter_agg_expr(
            expr,
            comp_id,
            comp_name,
            &agg_prefix,
            instantiated_children_ids,
            &parent_ports_by_member,
            &doc.nodes,
            &mut skipped_vars,
        )?;
        *expr = filt;
        if !skipped_vars.is_empty() {
            comp_unfiltered_ports.insert(port_name.clone(), unfilt);
            comp_skipped_upstream.insert(port_name.clone(), skipped_vars);
        }
    }

    if let Some(node) = doc.get_node_mut(comp_id) {
        node.aggregated_ports = all_aggregated_ports;
        node.unfiltered_ports = comp_unfiltered_ports;
        node.skipped_upstream = comp_skipped_upstream;
    }

    let nodes_snapshot = doc.nodes.clone();
    for node_idx in start_node_idx..doc.nodes.len() {
        let node = &mut doc.nodes[node_idx];
        let mut node_unfilt = HashMap::new();
        let mut node_skipped = HashMap::new();
        for (port_name, expr) in node.ports.iter_mut() {
            let mut skipped_vars = Vec::new();
            let (filt, unfilt) = resolve_and_filter_agg_expr(
                expr,
                comp_id,
                comp_name,
                &agg_prefix,
                instantiated_children_ids,
                &parent_ports_by_member,
                &nodes_snapshot,
                &mut skipped_vars,
            )?;
            *expr = filt;
            if !skipped_vars.is_empty() {
                node_unfilt.insert(port_name.clone(), unfilt);
                node_skipped.insert(port_name.clone(), skipped_vars);
            }
        }
        node.unfiltered_ports.extend(node_unfilt);
        node.skipped_upstream.extend(node_skipped);
    }

    for binding_idx in start_local_bindings_idx..doc.local_bindings.len() {
        let (expr, _, _) = &mut doc.local_bindings[binding_idx];
        let mut skipped_vars = Vec::new();
        let (filt, _) = resolve_and_filter_agg_expr(
            expr,
            comp_id,
            comp_name,
            &agg_prefix,
            instantiated_children_ids,
            &parent_ports_by_member,
            &doc.nodes,
            &mut skipped_vars,
        )?;
        *expr = filt;
    }

    Ok(())
}

