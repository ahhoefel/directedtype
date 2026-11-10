use crate::ast::*;
use crate::compiler::error::CompileError;
use crate::compiler::expand::context::{EnvEntry, LexicalBinding, ScopeContext};
use crate::compiler::expand::typecheck::{
    infer_expr_literal_for_typecheck, resolve_ident_or_member_to_literal,
};
use crate::compiler::expanded::NodeId;
use crate::span::Span;
use std::collections::HashMap;

pub(super) fn resolve_forward_scope_refs(
    expr: &Expr,
    scope: &HashMap<Symbol, LexicalBinding>,
) -> Expr {
    match expr {
        Expr::Ident(id) => {
            if let Some(binding) = scope.get(&id.name) {
                match binding {
                    LexicalBinding::Node(node_id) => {
                        Expr::Ident(Ident::from_node(*node_id, id.span))
                    }
                    LexicalBinding::Expr(e) | LexicalBinding::State(e) => e.clone(),
                    LexicalBinding::Member {
                        node_id, member, ..
                    } => Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::from_node(*node_id, id.span))),
                        member: Ident::from_symbol(member.name, id.span),
                        span: id.span,
                    }),
                    LexicalBinding::Uninitialized => expr.clone(),
                }
            } else {
                expr.clone()
            }
        }
        Expr::MemberAccess(m) => {
            let target_sym = match m.target.as_ref() {
                Expr::Ident(id) => Some(id.name),
                _ => None,
            };
            if let Some(target_name) = target_sym {
                if let Some(LexicalBinding::Node(node_id)) = scope.get(&target_name) {
                    return Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::from_node(
                            *node_id,
                            m.target.span(),
                        ))),
                        member: m.member,
                        span: m.span,
                    });
                }
            }
            Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(resolve_forward_scope_refs(&m.target, scope)),
                member: m.member,
                span: m.span,
            })
        }
        Expr::Binary(b) => Expr::Binary(BinaryExpr {
            op: b.op,
            left: Box::new(resolve_forward_scope_refs(&b.left, scope)),
            right: Box::new(resolve_forward_scope_refs(&b.right, scope)),
            span: b.span,
        }),
        Expr::Unary(u) => Expr::Unary(UnaryExpr {
            op: u.op,
            operand: Box::new(resolve_forward_scope_refs(&u.operand, scope)),
            span: u.span,
        }),
        Expr::Ternary(t) => Expr::Ternary(TernaryExpr {
            condition: Box::new(resolve_forward_scope_refs(&t.condition, scope)),
            then_expr: Box::new(resolve_forward_scope_refs(&t.then_expr, scope)),
            else_expr: Box::new(resolve_forward_scope_refs(&t.else_expr, scope)),
            span: t.span,
        }),
        Expr::Paren(inner, span) => {
            Expr::Paren(Box::new(resolve_forward_scope_refs(inner, scope)), *span)
        }
        Expr::Call(c) => Expr::Call(CallExpr {
            callee: c.callee.clone(),
            args: c
                .args
                .iter()
                .map(|a| resolve_forward_scope_refs(a, scope))
                .collect(),
            span: c.span,
        }),
        Expr::MethodCall(m) => Expr::MethodCall(MethodCallExpr {
            target: Box::new(resolve_forward_scope_refs(&m.target, scope)),
            method: m.method.clone(),
            args: m
                .args
                .iter()
                .map(|a| resolve_forward_scope_refs(a, scope))
                .collect(),
            span: m.span,
        }),
        Expr::Node(n) => {
            let mut cloned = n.clone();
            for port in &mut cloned.ports {
                port.expr = resolve_forward_scope_refs(&port.expr, scope);
            }
            Expr::Node(cloned)
        }
        _ => expr.clone(),
    }
}

pub(super) fn expr_has_scope_ref(expr: &Expr, scope: &HashMap<Symbol, LexicalBinding>) -> bool {
    match expr {
        Expr::Ident(id) => {
            !id.is_canonical_node_ref() && id.name != Symbol::WINDOW && scope.contains_key(&id.name)
        }
        Expr::MemberAccess(m) => expr_has_scope_ref(&m.target, scope),
        Expr::Binary(b) => expr_has_scope_ref(&b.left, scope) || expr_has_scope_ref(&b.right, scope),
        Expr::Unary(u) => expr_has_scope_ref(&u.operand, scope),
        Expr::Ternary(t) => {
            expr_has_scope_ref(&t.condition, scope)
                || expr_has_scope_ref(&t.then_expr, scope)
                || expr_has_scope_ref(&t.else_expr, scope)
        }
        Expr::Paren(inner, _) => expr_has_scope_ref(inner, scope),
        Expr::Call(c) => c.args.iter().any(|a| expr_has_scope_ref(a, scope)),
        Expr::MethodCall(m) => {
            expr_has_scope_ref(&m.target, scope)
                || m.args.iter().any(|a| expr_has_scope_ref(a, scope))
        }
        Expr::Node(n) => n.ports.iter().any(|p| expr_has_scope_ref(&p.expr, scope)),
        _ => false,
    }
}

/// Returns `true` if `expr` contains any non-canonical identifier (`TAG_NEXT_OF` or unresolved `TAG_STR`)
/// that could potentially be resolved by a later `resolve_forward_refs_in_range` pass.
pub(in crate::compiler::expand) fn expr_has_unresolved_ident(expr: &Expr) -> bool {
    match expr {
        Expr::Ident(id) => {
            !id.is_canonical_node_ref()
                && id.name.agg_child_of_node().is_none()
                && id.name != Symbol::WINDOW
                && id.name != Symbol::UNSET
                && id.name != Symbol::NONE
        }
        Expr::MemberAccess(m) => expr_has_unresolved_ident(&m.target),
        Expr::Binary(b) => {
            expr_has_unresolved_ident(&b.left) || expr_has_unresolved_ident(&b.right)
        }
        Expr::Unary(u) => expr_has_unresolved_ident(&u.operand),
        Expr::Ternary(t) => {
            expr_has_unresolved_ident(&t.condition)
                || expr_has_unresolved_ident(&t.then_expr)
                || expr_has_unresolved_ident(&t.else_expr)
        }
        Expr::Paren(inner, _) => expr_has_unresolved_ident(inner),
        Expr::Call(c) => c.args.iter().any(expr_has_unresolved_ident),
        Expr::MethodCall(m) => {
            expr_has_unresolved_ident(&m.target) || m.args.iter().any(expr_has_unresolved_ident)
        }
        Expr::Node(n) => n.ports.iter().any(|p| expr_has_unresolved_ident(&p.expr)),
        _ => false,
    }
}

/// Returns `true` if `expr` contains any `Expr::Ternary` sub-expression.
pub(in crate::compiler::expand) fn expr_has_ternary(expr: &Expr) -> bool {
    match expr {
        Expr::Ternary(_) => true,
        Expr::MemberAccess(m) => expr_has_ternary(&m.target),
        Expr::Binary(b) => expr_has_ternary(&b.left) || expr_has_ternary(&b.right),
        Expr::Unary(u) => expr_has_ternary(&u.operand),
        Expr::Paren(inner, _) => expr_has_ternary(inner),
        Expr::Call(c) => c.args.iter().any(expr_has_ternary),
        Expr::MethodCall(m) => {
            expr_has_ternary(&m.target) || m.args.iter().any(expr_has_ternary)
        }
        Expr::Node(n) => n.ports.iter().any(|p| expr_has_ternary(&p.expr)),
        _ => false,
    }
}

thread_local! {
    static PENDING_FWD_NODES: std::cell::RefCell<Vec<NodeId>> = const { std::cell::RefCell::new(Vec::new()) };
    static PENDING_FWD_BINDINGS: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
    static FWD_REF_COUNT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static UNFOLDED_TERNARY_COUNT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static MATERIALIZED_EXPR_PORTS: std::cell::RefCell<HashMap<NodeId, Vec<(String, Expr)>>> = std::cell::RefCell::new(HashMap::new());
    static PENDING_EXPR_TYPECHECKS: std::cell::RefCell<Vec<(Expr, Option<String>, Span)>> = const { std::cell::RefCell::new(Vec::new()) };
    static ACTIVE_EXPR_EVAL_STACK: std::cell::RefCell<Vec<Span>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub(in crate::compiler::expand) fn reset_rewrite_tracking() {
    PENDING_FWD_NODES.with(|c| c.borrow_mut().clear());
    PENDING_FWD_BINDINGS.with(|c| c.borrow_mut().clear());
    FWD_REF_COUNT.with(|c| c.set(0));
    UNFOLDED_TERNARY_COUNT.with(|c| c.set(0));
    MATERIALIZED_EXPR_PORTS.with(|c| c.borrow_mut().clear());
    PENDING_EXPR_TYPECHECKS.with(|c| c.borrow_mut().clear());
    ACTIVE_EXPR_EVAL_STACK.with(|c| c.borrow_mut().clear());
}

/// Materializes an evaluated `\Expr` body as a synthetic DAG port (`__expr_N`) on `owner_node`,
/// deduplicating structurally identical expressions on the same node.
pub(in crate::compiler::expand) fn materialize_expr_instance(
    owner_node: NodeId,
    expanded_body: Expr,
    call_span: Span,
) -> Expr {
    if owner_node.is_template_self() {
        return expanded_body;
    }
    let port_name = MATERIALIZED_EXPR_PORTS.with(|cell| {
        let mut map = cell.borrow_mut();
        let list = map.entry(owner_node).or_default();
        if let Some((existing_name, _)) = list
            .iter()
            .find(|(_, existing_expr)| expr_eq_ignore_span(existing_expr, &expanded_body))
        {
            return existing_name.clone();
        }
        let name = format!("__expr_{}", list.len());
        list.push((name.clone(), expanded_body));
        name
    });
    Expr::MemberAccess(MemberAccessExpr {
        target: Box::new(Expr::Ident(Ident::from_node(owner_node, call_span))),
        member: Ident::new(port_name, call_span),
        span: call_span,
    })
}

pub(in crate::compiler::expand) fn get_materialized_expr_port(
    node_id: NodeId,
    port_name: &str,
) -> Option<Expr> {
    MATERIALIZED_EXPR_PORTS.with(|cell| {
        let map = cell.borrow();
        map.get(&node_id)?
            .iter()
            .find(|(k, _)| k == port_name)
            .map(|(_, v)| v.clone())
    })
}

pub(in crate::compiler::expand) fn has_materialized_expr_port(
    node_id: NodeId,
    port_name: &str,
) -> bool {
    MATERIALIZED_EXPR_PORTS.with(|cell| {
        let map = cell.borrow();
        map.get(&node_id)
            .is_some_and(|list| list.iter().any(|(k, _)| k == port_name))
    })
}

pub(in crate::compiler::expand) fn lookup_materialized_expr_port(
    node_id: NodeId,
    port_name: &str,
    nodes: &[crate::compiler::expanded::ExpandedNode],
) -> Option<Expr> {
    if !node_id.is_window() && !node_id.is_template_self() {
        if let Some(expr) = nodes.get(node_id.0).and_then(|n| n.ports.get(port_name)) {
            return Some(expr.clone());
        }
    }
    get_materialized_expr_port(node_id, port_name)
}

pub(in crate::compiler::expand) fn sync_materialized_expr_ports_for_node(
    node_id: NodeId,
    ports: &mut HashMap<String, Expr>,
) {
    MATERIALIZED_EXPR_PORTS.with(|cell| {
        let map = cell.borrow();
        if let Some(list) = map.get(&node_id) {
            for (k, v) in list {
                ports.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
    });
}

pub(in crate::compiler::expand) fn flush_all_materialized_expr_ports(
    doc: &mut crate::compiler::expanded::ExpandedDocument,
) {
    MATERIALIZED_EXPR_PORTS.with(|cell| {
        let map = cell.borrow();
        for (&node_id, list) in map.iter() {
            if node_id.is_window() {
                for (k, v) in list {
                    doc.window_ports
                        .entry(k.clone())
                        .or_insert_with(|| v.clone());
                }
            } else if !node_id.is_template_self() {
                if let Some(node) = doc.nodes.get_mut(node_id.0) {
                    for (k, v) in list {
                        node.ports.entry(k.clone()).or_insert_with(|| v.clone());
                    }
                }
            }
        }
    });
    PENDING_EXPR_TYPECHECKS.with(|cell| {
        let mut checks = cell.borrow_mut();
        doc.local_bindings.append(&mut checks);
    });
}

#[inline]
pub(in crate::compiler::expand) fn has_any_pending_fwd_refs() -> bool {
    FWD_REF_COUNT.with(|c| c.get() > 0)
}

#[inline]
pub(in crate::compiler::expand) fn get_unfolded_ternary_count() -> u32 {
    UNFOLDED_TERNARY_COUNT.with(|c| c.get())
}

#[inline]
pub(in crate::compiler::expand) fn record_unfolded_ternary_emitted() {
    UNFOLDED_TERNARY_COUNT.with(|c| c.set(c.get().wrapping_add(1)));
}

#[inline]
pub(in crate::compiler::expand) fn record_pending_fwd_node(node_id: NodeId) {
    FWD_REF_COUNT.with(|c| c.set(c.get().wrapping_add(1)));
    if node_id.is_window() || node_id.is_template_self() {
        return;
    }
    PENDING_FWD_NODES.with(|cell| {
        let mut nodes = cell.borrow_mut();
        if nodes.last() != Some(&node_id) {
            nodes.push(node_id);
        }
    });
}

#[inline]
pub(crate) fn check_and_record_pending_fwd_binding(idx: usize, expr: &Expr) {
    if has_any_pending_fwd_refs() && expr_has_unresolved_ident(expr) {
        PENDING_FWD_BINDINGS.with(|cell| {
            let mut list = cell.borrow_mut();
            if list.last() != Some(&idx) {
                list.push(idx);
            }
        });
    }
}

/// Resolves forward lexical scope references in node ports and local bindings starting from the given indices.
pub(super) fn resolve_forward_refs_in_range(
    doc: &mut crate::compiler::expanded::ExpandedDocument,
    start_node_idx: usize,
    start_binding_idx: usize,
    scope: &HashMap<Symbol, LexicalBinding>,
) {
    if scope.is_empty() || !has_any_pending_fwd_refs() {
        return;
    }
    PENDING_FWD_NODES.with(|cell| {
        let mut pending = cell.borrow_mut();
        if pending.is_empty() {
            return;
        }
        pending.retain(|&node_id| {
            if node_id.0 < start_node_idx {
                return true;
            }
            let Some(node) = doc.nodes.get_mut(node_id.0) else {
                return false;
            };
            if node.ports.is_empty() {
                return true;
            }
            let mut still_has_unresolved = false;
            for expr in node.ports.values_mut() {
                if expr_has_scope_ref(expr, scope) {
                    *expr = resolve_forward_scope_refs(expr, scope);
                }
                if !still_has_unresolved && expr_has_unresolved_ident(expr) {
                    still_has_unresolved = true;
                }
            }
            still_has_unresolved
        });
    });
    MATERIALIZED_EXPR_PORTS.with(|cell| {
        let mut map = cell.borrow_mut();
        for (&node_id, list) in map.iter_mut() {
            if !node_id.is_window() && node_id.0 < start_node_idx {
                continue;
            }
            for (_, expr) in list.iter_mut() {
                if expr_has_scope_ref(expr, scope) {
                    *expr = resolve_forward_scope_refs(expr, scope);
                }
            }
        }
    });
    PENDING_FWD_BINDINGS.with(|cell| {
        let mut pending = cell.borrow_mut();
        if pending.is_empty() {
            return;
        }
        pending.retain(|&binding_idx| {
            if binding_idx < start_binding_idx {
                return true;
            }
            let Some((expr, _, _)) = doc.local_bindings.get_mut(binding_idx) else {
                return false;
            };
            if expr_has_scope_ref(expr, scope) {
                *expr = resolve_forward_scope_refs(expr, scope);
            }
            expr_has_unresolved_ident(expr)
        });
    });
}

/// Applies optional `type_annotation` coercion to `rewritten_res` and updates `scope[name]`.
/// Returns `Some(rewritten)` if the binding changed.
pub(super) fn try_preresolve_expr_binding(
    scope: &mut HashMap<Symbol, LexicalBinding>,
    name: impl Into<Symbol>,
    rewritten_res: Result<Expr, CompileError>,
    type_annotation: Option<&TypeRef>,
) -> Option<Expr> {
    let name_sym = name.into();
    let rewritten = rewritten_res.ok()?;
    let rewritten = super::typecheck::coerce_typed_binding_expr(rewritten, type_annotation);
    if let Some(LexicalBinding::Expr(existing)) = scope.get(&name_sym) {
        if existing == &rewritten {
            return None;
        }
    }
    scope.insert(name_sym, LexicalBinding::Expr(rewritten.clone()));
    Some(rewritten)
}

/// Checks whether `expr` only references variables present in `env` (or no variables at all),
/// avoiding heap string formatting inside `eval_expr` when variables are missing.
pub(super) fn expr_vars_in_env(
    expr: &Expr,
    env: &HashMap<crate::compiler::graph::VarId, crate::compiler::value::Value>,
) -> bool {
    match expr {
        Expr::Literal(_) => true,
        Expr::Ident(id) => id.name == Symbol::TRUE || id.name == Symbol::FALSE,
        Expr::MemberAccess(m) => {
            if let Expr::Ident(target_id) = m.target.as_ref() {
                if let Some(nid) = target_id.node_id() {
                    return env.contains_key(&crate::compiler::graph::VarId::from_symbol(
                        nid,
                        m.member.name,
                    ));
                }
            }
            false
        }
        Expr::Binary(b) => expr_vars_in_env(&b.left, env) && expr_vars_in_env(&b.right, env),
        Expr::Unary(u) => expr_vars_in_env(&u.operand, env),
        Expr::Paren(inner, _) => expr_vars_in_env(inner, env),
        Expr::Ternary(t) => {
            expr_vars_in_env(&t.condition, env)
                && expr_vars_in_env(&t.then_expr, env)
                && expr_vars_in_env(&t.else_expr, env)
        }
        Expr::Call(c) => c.args.iter().all(|a| expr_vars_in_env(a, env)),
        Expr::MethodCall(m) => {
            expr_vars_in_env(&m.target, env) && m.args.iter().all(|a| expr_vars_in_env(a, env))
        }
        Expr::Node(_) | Expr::ExprDef(_) => false,
    }
}

/// Evaluates sibling existence conditions (`prev`, `next`, `!prev`, `!next`, `&&`, `||`) at expansion time.
pub(super) fn populate_const_env_for_cond(
    expr: &Expr,
    cond_ctx: &ScopeContext<'_>,
    const_env: &mut HashMap<crate::compiler::graph::VarId, crate::compiler::value::Value>,
) {
    let Some(comp_ports) = cond_ctx.comp_ports else {
        return;
    };
    match expr {
        Expr::MemberAccess(m) => {
            if let Expr::Ident(target_id) = m.target.as_ref() {
                if let Some(nid) = target_id.node_id() {
                    if nid == cond_ctx.current_node || Some(nid) == cond_ctx.parent_node {
                        let port_name = m.member.as_str();
                        let var_id = crate::compiler::graph::VarId::from_symbol(
                            cond_ctx.current_node,
                            m.member.name,
                        );
                        if !const_env.contains_key(&var_id)
                            && !cond_ctx
                                .declared_state_names
                                .is_some_and(|s| s.contains(port_name))
                        {
                            if let Some(port_expr) = comp_ports.get(port_name) {
                                let empty_env = HashMap::new();
                                let resolved_val = if let Some(lit) =
                                    resolve_ident_or_member_to_literal(port_expr, cond_ctx)
                                {
                                    Some(match lit {
                                        Literal::Integer(i, _) => {
                                            crate::compiler::value::Value::Integer(i)
                                        }
                                        Literal::Float(n, _) => {
                                            crate::compiler::value::Value::Float(n)
                                        }
                                        Literal::Distance(d, _, _) => {
                                            crate::compiler::value::Value::Distance(d)
                                        }
                                        Literal::Area(a, _, _) => {
                                            crate::compiler::value::Value::Area(a)
                                        }
                                        Literal::Duration(d, _, _) => {
                                            crate::compiler::value::Value::Duration(d)
                                        }
                                        Literal::String(s, _) => {
                                            crate::compiler::value::Value::String(s)
                                        }
                                        Literal::Bool(b, _) => {
                                            crate::compiler::value::Value::Bool(b)
                                        }
                                        Literal::Color(c, _) => {
                                            crate::compiler::value::Value::Color(c)
                                        }
                                        Literal::Enum(e, v, _) => {
                                            crate::compiler::value::Value::Enum {
                                                enum_name: e,
                                                variant: v,
                                            }
                                        }
                                    })
                                } else if expr_vars_in_env(port_expr, &empty_env) {
                                    if let Ok(val) =
                                        crate::compiler::eval::eval_expr(port_expr, &empty_env)
                                    {
                                        if !matches!(
                                            val,
                                            crate::compiler::value::Value::Timer { .. }
                                                | crate::compiler::value::Value::IndefiniteTimer { .. }
                                                | crate::compiler::value::Value::Unset
                                        ) {
                                            Some(val)
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                };
                                if let Some(val) = resolved_val {
                                    const_env.insert(var_id, val.clone());
                                    if let Some(parent) = cond_ctx.parent_node {
                                        const_env.insert(
                                            crate::compiler::graph::VarId::from_symbol(
                                                parent,
                                                m.member.name,
                                            ),
                                            val,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            populate_const_env_for_cond(&m.target, cond_ctx, const_env);
        }
        Expr::Binary(b) => {
            populate_const_env_for_cond(&b.left, cond_ctx, const_env);
            populate_const_env_for_cond(&b.right, cond_ctx, const_env);
        }
        Expr::Unary(u) => populate_const_env_for_cond(&u.operand, cond_ctx, const_env),
        Expr::Paren(inner, _) => populate_const_env_for_cond(inner, cond_ctx, const_env),
        Expr::Ternary(t) => {
            populate_const_env_for_cond(&t.condition, cond_ctx, const_env);
            populate_const_env_for_cond(&t.then_expr, cond_ctx, const_env);
            populate_const_env_for_cond(&t.else_expr, cond_ctx, const_env);
        }
        Expr::Call(c) => {
            for arg in &c.args {
                populate_const_env_for_cond(arg, cond_ctx, const_env);
            }
        }
        Expr::MethodCall(m) => {
            populate_const_env_for_cond(&m.target, cond_ctx, const_env);
            for arg in &m.args {
                populate_const_env_for_cond(arg, cond_ctx, const_env);
            }
        }
        _ => {}
    }
}

fn eval_sibling_condition(expr: &Expr, ctx: &ScopeContext<'_>) -> Option<bool> {
    match expr {
        Expr::Paren(inner, _) => eval_sibling_condition(inner, ctx),
        Expr::Ident(id) => match id.as_str() {
            "prev" => Some(ctx.prev_sibling.is_some()),
            "next" => Some(ctx.has_next_sibling),
            _ => None,
        },
        Expr::Unary(u) if u.op == UnaryOp::Not => {
            eval_sibling_condition(&u.operand, ctx).map(|b| !b)
        }
        Expr::Binary(bin) => match bin.op {
            BinaryOp::And => {
                let l = eval_sibling_condition(&bin.left, ctx)?;
                let r = eval_sibling_condition(&bin.right, ctx)?;
                Some(l && r)
            }
            BinaryOp::Or => {
                let l = eval_sibling_condition(&bin.left, ctx)?;
                let r = eval_sibling_condition(&bin.right, ctx)?;
                Some(l || r)
            }
            _ => None,
        },
        _ => None,
    }
}

/// Resolves the active font's `.size` expression in the current scope for `em` units and `distance_to_float(d, em)`.
fn resolve_active_font_size_expr(
    ctx: &ScopeContext<'_>,
    span: Span,
) -> Result<Expr, CompileError> {
    if let Some(&fid) = ctx.node_fonts.get(&ctx.current_node) {
        if fid != ctx.current_node {
            return Ok(Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(Expr::Ident(Ident::from_node(fid, span))),
                member: Ident::from_symbol(Symbol::SIZE, span),
                span,
            }));
        }
    }
    match ctx.env_scope.get("font") {
        Some(EnvEntry::Bound(font_expr)) => {
            let rewritten_font = rewrite_expr(font_expr, ctx)?;
            Ok(Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(rewritten_font),
                member: Ident::from_symbol(Symbol::SIZE, span),
                span,
            }))
        }
        Some(EnvEntry::Tombstone) => Err(CompileError::BlockedEnvVariable {
            name: "font".to_string(),
            span,
        }),
        None => Err(CompileError::UndefinedEnvVariable {
            name: "font".to_string(),
            span,
        }),
    }
}

fn split_outer_member_chain(expr: &Expr) -> Option<(usize, Vec<Ident>)> {
    let mut members = Vec::new();
    let mut cur = expr;
    while let Expr::MemberAccess(m) = cur {
        members.push(m.member);
        cur = m.target.as_ref();
    }
    if let Expr::Ident(root_id) = cur {
        if root_id.name == Symbol::OUTER {
            members.push(*root_id);
            members.reverse();
            let k = members
                .iter()
                .take_while(|id| id.name == Symbol::OUTER)
                .count();
            return Some((k, members));
        }
    }
    None
}

fn build_member_chain(chain: &[Ident], full_span: Span) -> Expr {
    let mut expr = Expr::Ident(chain[0]);
    for (idx, member) in chain[1..].iter().enumerate() {
        let span = if idx + 2 == chain.len() {
            full_span
        } else {
            member.span
        };
        expr = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(expr),
            member: *member,
            span,
        });
    }
    expr
}

fn first_unresolved_lexical_ident(expr: &Expr) -> Option<Ident> {
    match expr {
        Expr::Ident(id) => {
            if !id.is_canonical_node_ref()
                && id.name.agg_child_of_node().is_none()
                && id.name.next_of_node().is_none()
                && id.name != Symbol::WINDOW
                && id.name != Symbol::UNSET
                && id.name != Symbol::NONE
            {
                Some(*id)
            } else {
                None
            }
        }
        Expr::MemberAccess(m) => first_unresolved_lexical_ident(&m.target),
        Expr::Binary(b) => first_unresolved_lexical_ident(&b.left)
            .or_else(|| first_unresolved_lexical_ident(&b.right)),
        Expr::Unary(u) => first_unresolved_lexical_ident(&u.operand),
        Expr::Ternary(t) => first_unresolved_lexical_ident(&t.condition)
            .or_else(|| first_unresolved_lexical_ident(&t.then_expr))
            .or_else(|| first_unresolved_lexical_ident(&t.else_expr)),
        Expr::Paren(inner, _) => first_unresolved_lexical_ident(inner),
        Expr::Call(c) => c.args.iter().find_map(first_unresolved_lexical_ident),
        Expr::MethodCall(mc) => first_unresolved_lexical_ident(&mc.target)
            .or_else(|| mc.args.iter().find_map(first_unresolved_lexical_ident)),
        Expr::Node(_) | Expr::ExprDef(_) | Expr::Literal(_) => None,
    }
}

fn resolve_outer_chain(
    chain: &[Ident],
    k: usize,
    full_span: Span,
    creation_ctx: &ScopeContext<'_>,
) -> Result<Expr, CompileError> {
    let mut stripped_chain: Vec<Ident> = chain[k..].to_vec();
    if stripped_chain[0].name == Symbol::SELF && stripped_chain.len() > 1 {
        if let Some(comp_id) = creation_ctx.enclosing_component {
            if !creation_ctx.is_event_port
                && !creation_ctx
                    .shadowed_ports
                    .is_some_and(|s| s.contains(stripped_chain[1].as_str()))
            {
                stripped_chain[0] = Ident::from_node(comp_id, stripped_chain[0].span);
            }
        }
    }
    let stripped_expr = build_member_chain(&stripped_chain, full_span);
    let rewritten = rewrite_expr(&stripped_expr, creation_ctx)?;
    if let Some(bad_id) = first_unresolved_lexical_ident(&rewritten) {
        return Err(CompileError::Custom {
            message: format!(
                "Undefined variable or property '{}' in 'outer'",
                bad_id.as_str()
            ),
            span: bad_id.span,
        });
    }
    Ok(rewritten)
}

fn rewrite_outer_in_expr(
    expr: &Expr,
    depth: usize,
    creation_ctx: &ScopeContext<'_>,
) -> Result<Expr, CompileError> {
    match expr {
        Expr::Ident(id) => {
            if id.name == Symbol::OUTER {
                return Err(CompileError::Custom {
                    message: "Bare 'outer' is not allowed; use 'outer.<property>' to access a property from the outer scope".to_string(),
                    span: id.span,
                });
            }
            Ok(expr.clone())
        }
        Expr::MemberAccess(m) => {
            if let Some((k, chain)) = split_outer_member_chain(expr) {
                if k == chain.len() {
                    return Err(CompileError::Custom {
                        message: "Bare 'outer' is not allowed; use 'outer.<property>' to access a property from the outer scope".to_string(),
                        span: expr.span(),
                    });
                }
                if k > depth + 1 {
                    return Err(CompileError::Custom {
                        message: "Too many 'outer.' prefixes for the current '\\Expr' nesting depth"
                            .to_string(),
                        span: expr.span(),
                    });
                }
                if k == depth + 1 {
                    return resolve_outer_chain(&chain, k, expr.span(), creation_ctx);
                }
                return Ok(expr.clone());
            }
            Ok(Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(rewrite_outer_in_expr(&m.target, depth, creation_ctx)?),
                member: m.member,
                span: m.span,
            }))
        }
        Expr::Call(call) => {
            let mut rewritten_args = Vec::with_capacity(call.args.len());
            for arg in &call.args {
                if let Expr::MemberAccess(m) = arg {
                    if let Some((k, chain)) = split_outer_member_chain(arg) {
                        if k == depth + 1
                            && chain.len() == k + 2
                            && chain[k].name == Symbol::CHILDREN
                            && chain[k + 1].name != Symbol::LEN
                        {
                            if let Some(comp_id) = creation_ctx.enclosing_component {
                                if creation_ctx.num_children > 0 {
                                    super::aggregation::record_agg_emitted(comp_id);
                                }
                                for idx in 0..creation_ctx.num_children {
                                    rewritten_args.push(Expr::MemberAccess(MemberAccessExpr {
                                        target: Box::new(Expr::Ident(Ident::agg_child_of(
                                            comp_id,
                                            idx,
                                            chain[k].span,
                                        ))),
                                        member: chain[k + 1],
                                        span: m.span,
                                    }));
                                }
                                continue;
                            } else if creation_ctx.child_ids.len() == creation_ctx.num_children {
                                for &child_id in creation_ctx.child_ids {
                                    rewritten_args.push(Expr::MemberAccess(MemberAccessExpr {
                                        target: Box::new(Expr::Ident(Ident::from_node(
                                            child_id,
                                            chain[k].span,
                                        ))),
                                        member: chain[k + 1],
                                        span: m.span,
                                    }));
                                }
                                continue;
                            }
                        }
                    }
                }
                rewritten_args.push(rewrite_outer_in_expr(arg, depth, creation_ctx)?);
            }
            Ok(Expr::Call(CallExpr {
                callee: call.callee,
                args: rewritten_args,
                span: call.span,
            }))
        }
        Expr::MethodCall(mc) => {
            let mut rewritten_args = Vec::with_capacity(mc.args.len());
            for arg in &mc.args {
                rewritten_args.push(rewrite_outer_in_expr(arg, depth, creation_ctx)?);
            }
            let target_outer = match mc.target.as_ref() {
                Expr::Ident(id) if id.name == Symbol::OUTER => Some((1, vec![*id])),
                _ => split_outer_member_chain(&mc.target),
            };
            if let Some((k, target_chain)) = target_outer {
                if k == target_chain.len() {
                    if k > depth + 1 {
                        return Err(CompileError::Custom {
                            message:
                                "Too many 'outer.' prefixes for the current '\\Expr' nesting depth"
                                    .to_string(),
                            span: mc.span,
                        });
                    }
                    if k == depth + 1 {
                        let mut full_chain = target_chain;
                        full_chain.push(mc.method);
                        let rewritten_callee =
                            resolve_outer_chain(&full_chain, k, mc.span, creation_ctx)?;
                        return Ok(Expr::MethodCall(MethodCallExpr {
                            target: Box::new(rewritten_callee),
                            method: Ident::from_symbol(Symbol::EMPTY, mc.method.span),
                            args: rewritten_args,
                            span: mc.span,
                        }));
                    }
                    return Ok(Expr::MethodCall(MethodCallExpr {
                        target: mc.target.clone(),
                        method: mc.method,
                        args: rewritten_args,
                        span: mc.span,
                    }));
                }
            }
            Ok(Expr::MethodCall(MethodCallExpr {
                target: Box::new(rewrite_outer_in_expr(&mc.target, depth, creation_ctx)?),
                method: mc.method,
                args: rewritten_args,
                span: mc.span,
            }))
        }
        Expr::Binary(b) => Ok(Expr::Binary(BinaryExpr {
            op: b.op,
            left: Box::new(rewrite_outer_in_expr(&b.left, depth, creation_ctx)?),
            right: Box::new(rewrite_outer_in_expr(&b.right, depth, creation_ctx)?),
            span: b.span,
        })),
        Expr::Unary(u) => Ok(Expr::Unary(UnaryExpr {
            op: u.op,
            operand: Box::new(rewrite_outer_in_expr(&u.operand, depth, creation_ctx)?),
            span: u.span,
        })),
        Expr::Ternary(t) => Ok(Expr::Ternary(TernaryExpr {
            condition: Box::new(rewrite_outer_in_expr(&t.condition, depth, creation_ctx)?),
            then_expr: Box::new(rewrite_outer_in_expr(&t.then_expr, depth, creation_ctx)?),
            else_expr: Box::new(rewrite_outer_in_expr(&t.else_expr, depth, creation_ctx)?),
            span: t.span,
        })),
        Expr::Paren(inner, span) => Ok(Expr::Paren(
            Box::new(rewrite_outer_in_expr(inner, depth, creation_ctx)?),
            *span,
        )),
        Expr::Node(n) => {
            let mut cloned = n.clone();
            for port in &mut cloned.ports {
                port.expr = rewrite_outer_in_expr(&port.expr, depth, creation_ctx)?;
            }
            Ok(Expr::Node(cloned))
        }
        Expr::ExprDef(nested) => {
            let mut params = Vec::with_capacity(nested.params.len());
            for param in &nested.params {
                let default_edge = match &param.default_edge {
                    Some(d) => Some(rewrite_outer_in_expr(d, depth + 1, creation_ctx)?),
                    None => None,
                };
                params.push(ParamDef {
                    name: param.name,
                    type_annotation: param.type_annotation.clone(),
                    default_edge,
                    is_env: param.is_env,
                    span: param.span,
                });
            }
            let body = rewrite_outer_in_expr(&nested.body, depth + 1, creation_ctx)?;
            Ok(Expr::ExprDef(Box::new(ExprDef {
                params,
                body: Box::new(body),
                span: nested.span,
            })))
        }
        Expr::Literal(_) => Ok(expr.clone()),
    }
}

pub(super) fn rewrite_expr_def_at_creation(
    def: &ExprDef,
    creation_ctx: &ScopeContext<'_>,
) -> Result<Expr, CompileError> {
    let mut seen_params = std::collections::HashSet::with_capacity(def.params.len());
    let mut params = Vec::with_capacity(def.params.len());
    for param in &def.params {
        if !seen_params.insert(param.name.name) {
            return Err(CompileError::Custom {
                message: format!("Duplicate parameter '{}' in '\\Expr'", param.name),
                span: param.name.span,
            });
        }
        let default_edge = match &param.default_edge {
            Some(d) => Some(rewrite_outer_in_expr(d, 0, creation_ctx)?),
            None => None,
        };
        params.push(ParamDef {
            name: param.name,
            type_annotation: param.type_annotation.clone(),
            default_edge,
            is_env: param.is_env,
            span: param.span,
        });
    }
    let body = rewrite_outer_in_expr(&def.body, 0, creation_ctx)?;
    Ok(Expr::ExprDef(Box::new(ExprDef {
        params,
        body: Box::new(body),
        span: def.span,
    })))
}

fn lookup_expr_def_on_node(
    node_id: NodeId,
    member_name: &str,
    ctx: &ScopeContext<'_>,
) -> Option<Box<ExprDef>> {
    if Some(node_id) == ctx.enclosing_component || node_id == ctx.current_node {
        if let Some(comp_ports) = ctx.comp_ports {
            if let Some(Expr::ExprDef(def)) = comp_ports.get(member_name) {
                return Some(def.clone());
            }
        }
    }
    if !node_id.is_window() && !node_id.is_template_self() {
        if let Some(node) = ctx.nodes.get(node_id.0) {
            if let Some(Expr::ExprDef(def)) = node.ports.get(member_name) {
                return Some(def.clone());
            }
        }
    }
    None
}

fn eval_expr_def_call(
    def: &ExprDef,
    raw_args: &[Expr],
    call_span: Span,
    ctx: &ScopeContext<'_>,
) -> Result<Expr, CompileError> {
    let is_recursive = ACTIVE_EXPR_EVAL_STACK.with(|cell| {
        let stack = cell.borrow();
        stack.len() >= 64 || stack.contains(&def.span)
    });
    if is_recursive {
        return Err(CompileError::Custom {
            message: "Recursive '\\Expr' evaluation is not allowed".to_string(),
            span: call_span,
        });
    }

    let max_args = def.params.len();
    let min_args = def
        .params
        .iter()
        .filter(|p| p.default_edge.is_none())
        .count();
    let missing_required = raw_args.len() > max_args
        || def
            .params
            .iter()
            .skip(raw_args.len())
            .any(|p| p.default_edge.is_none());
    if missing_required {
        let expected_str = if min_args == max_args {
            format!("{}", max_args)
        } else {
            format!("{}..={}", min_args, max_args)
        };
        return Err(CompileError::Custom {
            message: format!(
                "'\\Expr' expects {} argument(s), got {}",
                expected_str,
                raw_args.len()
            ),
            span: call_span,
        });
    }

    let mut call_lexical_scope = ctx.lexical_scope.clone();
    let mut shadowed_ports_owned: Option<std::collections::HashSet<String>> = None;
    if !def.params.is_empty() {
        if let Some(existing_shadowed) = ctx.shadowed_ports {
            let mut s = existing_shadowed.clone();
            for p in &def.params {
                s.insert(p.name.as_str().to_string());
            }
            shadowed_ports_owned = Some(s);
        }
    }

    for (idx, param) in def.params.iter().enumerate() {
        let arg_span;
        let mut rewritten_arg = if idx < raw_args.len() {
            let raw_arg = &raw_args[idx];
            arg_span = raw_arg.span();
            if let Expr::Ident(id) = raw_arg {
                if id.name == Symbol::SELF {
                    if let Some(comp_id) = ctx.enclosing_component {
                        Expr::Ident(Ident::from_node(comp_id, id.span))
                    } else {
                        rewrite_expr(raw_arg, ctx)?
                    }
                } else {
                    rewrite_expr(raw_arg, ctx)?
                }
            } else {
                rewrite_expr(raw_arg, ctx)?
            }
        } else {
            let default_expr = param.default_edge.as_ref().unwrap();
            arg_span = default_expr.span();
            let mut default_ctx = ctx.clone();
            default_ctx.lexical_scope = &call_lexical_scope;
            if let Some(ref s) = shadowed_ports_owned {
                default_ctx.shadowed_ports = Some(s);
            }
            rewrite_expr(default_expr, &default_ctx)?
        };

        if let Some(type_ref) = &param.type_annotation {
            let expected_type = type_ref.name.as_str();
            rewritten_arg =
                super::typecheck::coerce_typed_binding_expr(rewritten_arg, Some(type_ref));
            if matches!(expected_type, "Expr" | "Expression") {
                if !matches!(rewritten_arg, Expr::ExprDef(_)) {
                    let actual_str =
                        if let Some(lit) = infer_expr_literal_for_typecheck(&rewritten_arg, ctx) {
                            lit.type_name().to_string()
                        } else {
                            "non-Expr".to_string()
                        };
                    return Err(CompileError::TypeMismatch {
                        expected: expected_type.to_string(),
                        actual: actual_str,
                        span: arg_span,
                    });
                }
            } else if matches!(rewritten_arg, Expr::ExprDef(_)) {
                return Err(CompileError::TypeMismatch {
                    expected: expected_type.to_string(),
                    actual: "Expr".to_string(),
                    span: arg_span,
                });
            } else if let Some(lit) = infer_expr_literal_for_typecheck(&rewritten_arg, ctx) {
                if !lit.matches_type_name(expected_type) {
                    return Err(CompileError::TypeMismatch {
                        expected: expected_type.to_string(),
                        actual: lit.type_name().to_string(),
                        span: arg_span,
                    });
                }
            } else {
                PENDING_EXPR_TYPECHECKS.with(|cell| {
                    cell.borrow_mut().push((
                        rewritten_arg.clone(),
                        Some(expected_type.to_string()),
                        type_ref.span,
                    ));
                });
            }
        }

        let binding = match &rewritten_arg {
            Expr::Ident(id) if id.node_id().is_some() => {
                LexicalBinding::Node(id.node_id().unwrap())
            }
            _ => LexicalBinding::Expr(rewritten_arg),
        };
        call_lexical_scope.insert(param.name.name, binding);
    }

    ACTIVE_EXPR_EVAL_STACK.with(|cell| cell.borrow_mut().push(def.span));
    let mut body_ctx = ctx.clone();
    body_ctx.lexical_scope = &call_lexical_scope;
    if let Some(ref s) = shadowed_ports_owned {
        body_ctx.shadowed_ports = Some(s);
    }
    let body_res = rewrite_expr(&def.body, &body_ctx);
    ACTIVE_EXPR_EVAL_STACK.with(|cell| {
        cell.borrow_mut().pop();
    });
    let expanded_body = body_res?;

    if ctx.is_event_port
        || matches!(expanded_body, Expr::ExprDef(_) | Expr::Node(_))
        || matches!(
            &expanded_body,
            Expr::Ident(id)
                if id.is_canonical_node_ref()
                    || id.name == Symbol::WINDOW
                    || id.name == Symbol::UNSET
                    || id.name == Symbol::NONE
        )
    {
        return Ok(expanded_body);
    }

    Ok(materialize_expr_instance(
        ctx.current_node,
        expanded_body,
        call_span,
    ))
}

/// Recursively rewrites an expression into canonical node variable references and resolves derived aliases.
pub fn rewrite_expr(expr: &Expr, ctx: &ScopeContext<'_>) -> Result<Expr, CompileError> {
    match expr {
        Expr::Ident(id) => {
            if id.name == Symbol::ENV {
                return Err(CompileError::BareEnvUse { span: id.span });
            }
            if id.name == Symbol::OUTER {
                return Err(CompileError::Custom {
                    message: "'outer' can only be used inside an '\\Expr' literal".to_string(),
                    span: id.span,
                });
            }
            if !id.is_canonical_node_ref() && ctx.enums.contains_key(id.as_str()) {
                return Err(CompileError::BareEnumUse {
                    name: id.as_str().to_string(),
                    span: id.span,
                });
            }

            // Check lexical scope first (let bindings, component ports, named nodes)
            if id.name != Symbol::SELF
                && id.name != Symbol::PARENT
                && id.name != Symbol::WINDOW
                && id.name != Symbol::PREV
                && id.name != Symbol::NEXT
                && id.name != Symbol::CHILD
            {
                if let Some(binding) = ctx.lexical_scope.get(&id.name) {
                    match binding {
                        LexicalBinding::Expr(e) | LexicalBinding::State(e) => {
                            if has_any_pending_fwd_refs() && expr_has_unresolved_ident(e) {
                                record_pending_fwd_node(ctx.current_node);
                            }
                            if expr_has_ternary(e) {
                                record_unfolded_ternary_emitted();
                            }
                            return Ok(e.clone());
                        }
                        LexicalBinding::Member {
                            node_id, member, ..
                        } => {
                            if let Some(def) =
                                lookup_expr_def_on_node(*node_id, member.as_str(), ctx)
                            {
                                return Ok(Expr::ExprDef(def));
                            }
                            return Ok(Expr::MemberAccess(MemberAccessExpr {
                                target: Box::new(Expr::Ident(Ident::from_node(*node_id, id.span))),
                                member: Ident::from_symbol(member.name, id.span),
                                span: id.span,
                            }));
                        }
                        LexicalBinding::Node(node_id) => {
                            return Ok(Expr::Ident(Ident::from_node(*node_id, id.span)));
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

            // In an event port (on_*), a bare method name that wasn't in lexical_scope
            // resolves to the enclosing component's method (or current_node if in signature default).
            if ctx.is_event_port
                && id.name != Symbol::UNSET
                && id.name != Symbol::NONE
                && id.name != Symbol::SELF
                && id.name != Symbol::PARENT
                && id.name != Symbol::WINDOW
                && id.name != Symbol::PREV
                && id.name != Symbol::NEXT
                && id.name != Symbol::CHILD
                && id.node_id().is_none()
            {
                let target_id = ctx.enclosing_component.unwrap_or(ctx.current_node);
                return Ok(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::from_node(target_id, id.span))),
                    member: *id,
                    span: id.span,
                }));
            }

            if id.name == Symbol::WINDOW {
                return Ok(Expr::Ident(Ident::from_node(NodeId::WINDOW, id.span)));
            }
            if id.name == Symbol::SELF {
                return Err(CompileError::Custom {
                    message: "Bare 'self' is not allowed; use 'self.<method>' in an event handler or 'self.<property>' to access a shadowed component property".to_string(),
                    span: id.span,
                });
            }
            if id.name == Symbol::CHILD {
                if let Some(cid) = ctx.child_node {
                    return Ok(Expr::Ident(Ident::from_node(cid, id.span)));
                } else {
                    return Err(CompileError::Custom {
                        message: "'child' can only be used inside a '\\Children(...)' block"
                            .to_string(),
                        span: id.span,
                    });
                }
            }
            if id.name == Symbol::PARENT {
                if let Some(parent) = ctx.parent_node {
                    return Ok(Expr::Ident(Ident::from_node(parent, id.span)));
                }
            }
            if id.name == Symbol::PREV {
                if let Some(prev) = ctx.prev_sibling {
                    return Ok(Expr::Ident(Ident::from_node(prev, id.span)));
                }
            }
            if id.name == Symbol::NEXT {
                if ctx.has_next_sibling {
                    if let Some(cid) = ctx.child_node {
                        record_pending_fwd_node(ctx.current_node);
                        return Ok(Expr::Ident(Ident::next_of(cid, id.span)));
                    }
                }
            }
            if id.name == Symbol::NUM_CHILDREN {
                return Err(CompileError::Custom {
                    message: "'num_children' is no longer supported; use 'children.len' instead"
                        .to_string(),
                    span: id.span,
                });
            }
            if !id.is_canonical_node_ref()
                && id.name.agg_child_of_node().is_none()
                && id.name != Symbol::WINDOW
                && id.name != Symbol::UNSET
                && id.name != Symbol::NONE
            {
                record_pending_fwd_node(ctx.current_node);
            }
            Ok(expr.clone())
        }

        Expr::Ternary(tern) => {
            // Recurrence resolution for `prev` / `next` conditions
            if let Some(cond_val) = eval_sibling_condition(&tern.condition, ctx) {
                let (chosen_raw, other_raw) = if cond_val {
                    (&tern.then_expr, &tern.else_expr)
                } else {
                    (&tern.else_expr, &tern.then_expr)
                };
                let mut chosen = rewrite_expr(chosen_raw, ctx)?;
                if let Expr::Literal(Literal::Integer(0, span)) = chosen {
                    if let Ok(other_rewritten) = rewrite_expr(other_raw, ctx) {
                        match infer_expr_literal_for_typecheck(&other_rewritten, ctx) {
                            Some(Literal::Distance(_, _, _)) => {
                                chosen =
                                    Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span));
                            }
                            Some(Literal::Area(_, _, _)) => {
                                chosen = Expr::Literal(Literal::Area(0.0, DistanceUnit::Px, span));
                            }
                            Some(Literal::Duration(_, _, _)) => {
                                chosen =
                                    Expr::Literal(Literal::Duration(0, DurationUnit::S, span));
                            }
                            _ => {}
                        }
                    }
                }
                return Ok(chosen);
            }
            let mut cond_ctx = ctx.clone();
            cond_ctx.is_event_port = false;
            let rewritten_cond = rewrite_expr(&tern.condition, &cond_ctx)?;
            let mut const_env = HashMap::new();
            populate_const_env_for_cond(&rewritten_cond, &cond_ctx, &mut const_env);
            let cond_bool = if expr_vars_in_env(&rewritten_cond, &const_env) {
                match crate::compiler::eval::eval_expr(&rewritten_cond, &const_env) {
                    Ok(crate::compiler::value::Value::Bool(b)) => Some(b),
                    _ if ctx.is_event_port => {
                        match resolve_ident_or_member_to_literal(&tern.condition, &cond_ctx) {
                            Some(Literal::Bool(b, _)) => Some(b),
                            _ => None,
                        }
                    }
                    _ => None,
                }
            } else if ctx.is_event_port {
                match resolve_ident_or_member_to_literal(&tern.condition, &cond_ctx) {
                    Some(Literal::Bool(b, _)) => Some(b),
                    _ => None,
                }
            } else {
                None
            };
            if let Some(b) = cond_bool {
                let (chosen_raw, other_raw) = if b {
                    (&tern.then_expr, &tern.else_expr)
                } else {
                    (&tern.else_expr, &tern.then_expr)
                };
                let mut chosen = rewrite_expr(chosen_raw, ctx)?;
                if let Expr::Literal(Literal::Integer(0, span)) = chosen {
                    if let Ok(other_rewritten) = rewrite_expr(other_raw, ctx) {
                        match infer_expr_literal_for_typecheck(&other_rewritten, ctx) {
                            Some(Literal::Distance(_, _, _)) => {
                                chosen =
                                    Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span));
                            }
                            Some(Literal::Area(_, _, _)) => {
                                chosen = Expr::Literal(Literal::Area(0.0, DistanceUnit::Px, span));
                            }
                            Some(Literal::Duration(_, _, _)) => {
                                chosen =
                                    Expr::Literal(Literal::Duration(0, DurationUnit::S, span));
                            }
                            _ => {}
                        }
                    }
                }
                return Ok(chosen);
            }
            let mut then_rewritten = rewrite_expr(&tern.then_expr, ctx)?;
            let mut else_rewritten = rewrite_expr(&tern.else_expr, ctx)?;
            if let Expr::Literal(Literal::Integer(0, span)) = then_rewritten {
                match infer_expr_literal_for_typecheck(&else_rewritten, ctx) {
                    Some(Literal::Distance(_, _, _)) => {
                        then_rewritten =
                            Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span));
                    }
                    Some(Literal::Area(_, _, _)) => {
                        then_rewritten = Expr::Literal(Literal::Area(0.0, DistanceUnit::Px, span));
                    }
                    Some(Literal::Duration(_, _, _)) => {
                        then_rewritten =
                            Expr::Literal(Literal::Duration(0, DurationUnit::S, span));
                    }
                    _ => {}
                }
            }
            if let Expr::Literal(Literal::Integer(0, span)) = else_rewritten {
                match infer_expr_literal_for_typecheck(&then_rewritten, ctx) {
                    Some(Literal::Distance(_, _, _)) => {
                        else_rewritten =
                            Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span));
                    }
                    Some(Literal::Area(_, _, _)) => {
                        else_rewritten = Expr::Literal(Literal::Area(0.0, DistanceUnit::Px, span));
                    }
                    Some(Literal::Duration(_, _, _)) => {
                        else_rewritten =
                            Expr::Literal(Literal::Duration(0, DurationUnit::S, span));
                    }
                    _ => {}
                }
            }
            record_unfolded_ternary_emitted();
            Ok(Expr::Ternary(TernaryExpr {
                condition: Box::new(rewritten_cond),
                then_expr: Box::new(then_rewritten),
                else_expr: Box::new(else_rewritten),
                span: tern.span,
            }))
        }

        Expr::MemberAccess(m) => {
            if m.member.as_str() == "baseline" {
                let mut inner_target = m.target.as_ref();
                while let Expr::Paren(p, _) = inner_target {
                    inner_target = p.as_ref();
                }
                if let Expr::MemberAccess(inner) = inner_target {
                    if inner.member.as_str() == "font" {
                        let text_target = rewrite_expr(&inner.target, ctx)?;
                        return Ok(Expr::MemberAccess(MemberAccessExpr {
                            target: Box::new(text_target),
                            member: Ident::new("baseline", m.span),
                            span: m.span,
                        }));
                    }
                }
            }

            let target_ident = match m.target.as_ref() {
                Expr::Ident(id) => Some(*id),
                _ => None,
            };

            // Enum variant access and children.len
            if let Some(target_id) = target_ident {
                if target_id.name == Symbol::CHILDREN {
                    if m.member.name == Symbol::LEN {
                        return Ok(Expr::Literal(Literal::Integer(
                            ctx.num_children as i64,
                            m.span,
                        )));
                    } else {
                        return Err(CompileError::Custom {
                            message: format!(
                                "Cannot access 'children.{}' outside of an aggregation function; use 'children.len' for the child count",
                                m.member
                            ),
                            span: m.span,
                        });
                    }
                }
                if !target_id.is_canonical_node_ref() {
                    if let Some(enum_def) = ctx.enums.get(target_id.as_str()) {
                        let variant_name = m.member.as_str();
                        if enum_def.has_variant(variant_name) {
                            return Ok(Expr::Literal(Literal::Enum(
                                target_id.as_str().to_string(),
                                variant_name.to_string(),
                                m.span,
                            )));
                        } else {
                            return Err(CompileError::UnknownEnumVariant {
                                enum_name: target_id.as_str().to_string(),
                                variant: variant_name.to_string(),
                                span: m.member.span,
                            });
                        }
                    }
                }
            }

            if target_ident.is_some_and(|id| id.name == Symbol::ENV) {
                let member_name = m.member.as_str();
                return match ctx.env_scope.get(member_name) {
                    Some(EnvEntry::Bound(expr)) => {
                        if has_any_pending_fwd_refs() && expr_has_unresolved_ident(expr) {
                            record_pending_fwd_node(ctx.current_node);
                        }
                        if expr_has_ternary(expr) {
                            record_unfolded_ternary_emitted();
                        }
                        Ok(expr.clone())
                    }
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

            // Resolve target (parent, window, prev, next, self, child, or named node in lexical scope)
            let resolved_target = if let Some(target_id) = target_ident {
                let target_sym = target_id.name;
                if target_sym == Symbol::SELF {
                    if ctx.is_event_port {
                        if let Some(shadowed) = ctx.shadowed_ports {
                            if shadowed.contains(m.member.as_str()) {
                                if let Some(comp_ports) = ctx.comp_ports {
                                    if let Some(port_expr) = comp_ports.get(m.member.as_str()) {
                                        if expr_has_unresolved_ident(port_expr) {
                                            record_pending_fwd_node(ctx.current_node);
                                        }
                                        if expr_has_ternary(port_expr) {
                                            record_unfolded_ternary_emitted();
                                        }
                                        return Ok(port_expr.clone());
                                    }
                                }
                            }
                        }
                        let target_node_id = ctx.enclosing_component.unwrap_or(ctx.current_node);
                        Expr::Ident(Ident::from_node(target_node_id, m.target.span()))
                    } else if let (Some(comp_id), Some(shadowed)) =
                        (ctx.enclosing_component, ctx.shadowed_ports)
                    {
                        if shadowed.contains(m.member.as_str()) {
                            Expr::Ident(Ident::from_node(comp_id, m.target.span()))
                        } else {
                            return Err(CompileError::Custom {
                                message: format!(
                                    "Cannot use 'self.{}' because property '{}' is not shadowed by a local '\\let' or '\\env' binding; use '{}' directly",
                                    m.member, m.member, m.member
                                ),
                                span: m.span,
                            });
                        }
                    } else {
                        return Err(CompileError::Custom {
                            message: format!(
                                "Cannot use 'self.{}' outside of a component; 'self' is only allowed in event handlers or to access a shadowed component property",
                                m.member
                            ),
                            span: m.span,
                        });
                    }
                } else if target_sym == Symbol::CHILD {
                    if let Some(cid) = ctx.child_node {
                        Expr::Ident(Ident::from_node(cid, m.target.span()))
                    } else {
                        return Err(CompileError::Custom {
                            message: "'child' can only be used inside a '\\Children(...)' block"
                                .to_string(),
                            span: m.target.span(),
                        });
                    }
                } else if let Some(LexicalBinding::Node(node_id)) =
                    ctx.lexical_scope.get(&target_sym)
                {
                    Expr::Ident(Ident::from_node(*node_id, m.target.span()))
                } else if target_sym == Symbol::PARENT {
                    if let Some(parent) = ctx.parent_node {
                        if m.member.name != Symbol::PARENT
                            && !ctx.parent_ports.is_empty()
                            && !ctx.parent_ports.iter().any(|p| p == m.member.as_str())
                        {
                            let parent_name = ctx.parent_name.unwrap_or("unknown");
                            return Err(CompileError::Custom {
                                message: format!(
                                    "Node '{}' has no public property '{}'",
                                    parent_name, m.member
                                ),
                                span: m.span,
                            });
                        }
                        Expr::Ident(Ident::from_node(parent, m.target.span()))
                    } else {
                        rewrite_expr(&m.target, ctx)?
                    }
                } else if target_sym == Symbol::WINDOW {
                    Expr::Ident(Ident::from_node(NodeId::WINDOW, m.target.span()))
                } else if target_sym == Symbol::PREV {
                    if let Some(prev) = ctx.prev_sibling {
                        Expr::Ident(Ident::from_node(prev, m.target.span()))
                    } else {
                        rewrite_expr(&m.target, ctx)?
                    }
                } else if target_sym == Symbol::NEXT {
                    if ctx.has_next_sibling {
                        if let Some(cid) = ctx.child_node {
                            record_pending_fwd_node(ctx.current_node);
                            Expr::Ident(Ident::next_of(cid, m.target.span()))
                        } else {
                            rewrite_expr(&m.target, ctx)?
                        }
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
                Expr::Ident(id) => id.node_id(),
                _ => None,
            };

            // Chained parent access: e.g. parent.parent -> parent_id of target_node_id
            if m.member.name == Symbol::PARENT {
                if let Some(tid) = target_node_id {
                    let parent_id = if Some(tid) == ctx.child_node {
                        ctx.parent_node
                    } else {
                        ctx.nodes.get(tid.0).and_then(|n| n.parent)
                    };
                    if let Some(pid) = parent_id {
                        return Ok(Expr::Ident(Ident::from_node(pid, m.span)));
                    } else {
                        return Err(CompileError::Custom {
                            message: "Node has no parent".to_string(),
                            span: m.span,
                        });
                    }
                }
            }

            // Font node reference access: e.g. label.font -> font_node_id
            if m.member.name == Symbol::FONT {
                if let Some(tid) = target_node_id {
                    if let Some(&fid) = ctx.node_fonts.get(&tid) {
                        return Ok(Expr::Ident(Ident::from_node(fid, m.span)));
                    }
                }
            }

            if let Some(tid) = target_node_id {
                if let Some(target_node) = ctx.nodes.get(tid.0) {
                    if target_node.private_ports.contains(m.member.as_str())
                        && ctx.current_node != tid
                        && ctx.enclosing_component != Some(tid)
                        && !target_ident.is_some_and(|id| {
                            id.name == Symbol::PARENT || id.is_canonical_node_ref()
                        })
                    {
                        return Err(CompileError::Custom {
                            message: format!(
                                "Node '{}' has no public property '{}'",
                                target_node.name, m.member
                            ),
                            span: m.span,
                        });
                    }
                }
                if let Some(def) = lookup_expr_def_on_node(tid, m.member.as_str(), ctx) {
                    return Ok(Expr::ExprDef(def));
                }
            }

            Ok(Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(resolved_target),
                member: m.member,
                span: m.span,
            }))
        }

        Expr::Call(call) => {
            let is_builtin_fn = matches!(
                call.callee.as_str(),
                "max"
                    | "min"
                    | "sum"
                    | "only"
                    | "clamp"
                    | "round"
                    | "floor"
                    | "ceil"
                    | "abs"
                    | "hypot"
                    | "sqrt"
                    | "sin"
                    | "cos"
                    | "tan"
                    | "atan2"
                    | "hsl"
                    | "hsla"
                    | "rgb"
                    | "rgba"
                    | "distance_to_float"
                    | "to_fixed"
                    | "to_precision"
                    | "to_px"
                    | "to_pt"
                    | "to_float"
                    | "to_number"
                    | "to_integer"
                    | "text_width"
                    | "text_height"
                    | "text_baseline_offset"
                    | "span_metric"
                    | "font_cap_height"
                    | "font_x_height"
                    | "font_ascent"
                    | "font_descent"
                    | "font_line_gap"
                    | "font_line_height"
            );
            if call.callee.name != Symbol::SELF
                && call.callee.name != Symbol::PARENT
                && call.callee.name != Symbol::WINDOW
                && call.callee.name != Symbol::PREV
                && call.callee.name != Symbol::NEXT
                && call.callee.name != Symbol::CHILD
            {
                if let Some(binding) = ctx.lexical_scope.get(&call.callee.name) {
                    match binding {
                        LexicalBinding::Expr(Expr::ExprDef(def)) => {
                            let def_cloned = def.clone();
                            return eval_expr_def_call(&def_cloned, &call.args, call.span, ctx);
                        }
                        LexicalBinding::Member {
                            node_id, member, ..
                        } => {
                            if let Some(def) =
                                lookup_expr_def_on_node(*node_id, member.as_str(), ctx)
                            {
                                return eval_expr_def_call(&def, &call.args, call.span, ctx);
                            }
                            if !is_builtin_fn {
                                return Err(CompileError::Custom {
                                    message: format!(
                                        "'{}' is not an '\\Expr' and cannot be called",
                                        call.callee
                                    ),
                                    span: call.span,
                                });
                            }
                        }
                        LexicalBinding::Uninitialized if !is_builtin_fn => {
                            return Err(CompileError::UninitializedVariableUse {
                                name: call.callee.as_str().to_string(),
                                span: call.callee.span,
                            });
                        }
                        LexicalBinding::Expr(_)
                        | LexicalBinding::State(_)
                        | LexicalBinding::Node(_)
                        | LexicalBinding::Uninitialized => {
                            if !is_builtin_fn {
                                return Err(CompileError::Custom {
                                    message: format!(
                                        "'{}' is not an '\\Expr' and cannot be called",
                                        call.callee
                                    ),
                                    span: call.span,
                                });
                            }
                        }
                    }
                }
            }
            if let Some(comp_id) = ctx.enclosing_component {
                if let Some(def) = lookup_expr_def_on_node(comp_id, call.callee.as_str(), ctx) {
                    return eval_expr_def_call(&def, &call.args, call.span, ctx);
                }
            }

            if call.callee.as_str() == "distance_to_float" {
                if call.args.len() != 2 {
                    return Err(CompileError::Custom {
                        message: format!(
                            "distance_to_float() expects 2 arguments (distance, unit), got {}",
                            call.args.len()
                        ),
                        span: call.span,
                    });
                }
                let dist_expr = rewrite_expr(&call.args[0], ctx)?;
                let unit_expr = match &call.args[1] {
                    Expr::Ident(unit_id) => {
                        let Some(unit) = DistanceUnit::from_str(unit_id.as_str()) else {
                            return Err(CompileError::Custom {
                                message: format!(
                                    "Second argument to distance_to_float() must be a unit ('px', 'pt', or 'em'), got '{}'",
                                    unit_id.as_str()
                                ),
                                span: unit_id.span,
                            });
                        };
                        rewrite_expr(
                            &Expr::Literal(Literal::Distance(1.0, unit, unit_id.span)),
                            ctx,
                        )?
                    }
                    already_lowered @ (Expr::Literal(Literal::Distance(_, DistanceUnit::Px, _))
                    | Expr::Binary(_)) => rewrite_expr(already_lowered, ctx)?,
                    other => {
                        return Err(CompileError::Custom {
                            message: "Second argument to distance_to_float() must be a unit ('px', 'pt', or 'em')".to_string(),
                            span: other.span(),
                        });
                    }
                };
                return Ok(Expr::Call(CallExpr {
                    callee: call.callee.clone(),
                    args: vec![dist_expr, unit_expr],
                    span: call.span,
                }));
            }

            let mut rewritten_args = Vec::new();
            let mut had_children = false;
            let mut children_member = None;
            for arg in &call.args {
                if let Expr::MemberAccess(m) = arg {
                    if let Expr::Ident(target_id) = m.target.as_ref() {
                        if target_id.name == Symbol::CHILDREN && m.member.name != Symbol::LEN {
                            had_children = true;
                            children_member = Some(m.member.as_str());
                            if let Some(comp_id) = ctx.enclosing_component {
                                if ctx.num_children > 0 {
                                    super::aggregation::record_agg_emitted(comp_id);
                                }
                                for idx in 0..ctx.num_children {
                                    let child_access = Expr::MemberAccess(MemberAccessExpr {
                                        target: Box::new(Expr::Ident(Ident::agg_child_of(
                                            comp_id,
                                            idx,
                                            target_id.span,
                                        ))),
                                        member: m.member,
                                        span: m.span,
                                    });
                                    rewritten_args.push(child_access);
                                }
                            } else if ctx.child_ids.len() == ctx.num_children {
                                for &child_id in ctx.child_ids {
                                    let child_access = Expr::MemberAccess(MemberAccessExpr {
                                        target: Box::new(Expr::Ident(Ident::from_node(
                                            child_id,
                                            target_id.span,
                                        ))),
                                        member: m.member,
                                        span: m.span,
                                    });
                                    rewritten_args.push(child_access);
                                }
                            }
                            continue;
                        }
                    }
                }
                rewritten_args.push(rewrite_expr(arg, ctx)?);
            }

            if had_children && rewritten_args.is_empty() && call.callee.name != Symbol::ONLY {
                let is_int_member = matches!(
                    children_member,
                    Some("z" | "weight" | "index" | "len")
                );
                if is_int_member {
                    return Ok(Expr::Literal(Literal::Integer(0, call.span)));
                } else {
                    return Ok(Expr::Literal(Literal::Distance(
                        0.0,
                        DistanceUnit::Px,
                        call.span,
                    )));
                }
            }

            Ok(Expr::Call(CallExpr {
                callee: call.callee.clone(),
                args: rewritten_args,
                span: call.span,
            }))
        }

        Expr::MethodCall(mc) => {
            if mc.method.name == Symbol::EMPTY {
                let callee = match mc.target.as_ref() {
                    Expr::ExprDef(def) => Expr::ExprDef(def.clone()),
                    other => rewrite_expr(other, ctx)?,
                };
                match callee {
                    Expr::ExprDef(def) => {
                        return eval_expr_def_call(&def, &mc.args, mc.span, ctx);
                    }
                    Expr::MemberAccess(m) => {
                        if let Expr::Ident(tid) = m.target.as_ref() {
                            if let Some(nid) = tid.node_id() {
                                if let Some(def) =
                                    lookup_expr_def_on_node(nid, m.member.as_str(), ctx)
                                {
                                    return eval_expr_def_call(&def, &mc.args, mc.span, ctx);
                                }
                            }
                        }
                        return Err(CompileError::Custom {
                            message: format!(
                                "Property '{}' is not an '\\Expr' and cannot be called",
                                m.member
                            ),
                            span: mc.span,
                        });
                    }
                    _ => {
                        return Err(CompileError::Custom {
                            message: "Target is not an '\\Expr' and cannot be called".to_string(),
                            span: mc.span,
                        });
                    }
                }
            }

            let is_builtin_method = matches!(
                mc.method.as_str(),
                "append"
                    | "to_fixed"
                    | "to_precision"
                    | "to_px"
                    | "to_pt"
                    | "to_float"
                    | "to_number"
                    | "to_seconds"
                    | "to_ms"
                    | "to_integer"
            );
            if !ctx.is_event_port && !is_builtin_method {
                let member_expr = Expr::MemberAccess(MemberAccessExpr {
                    target: mc.target.clone(),
                    member: mc.method,
                    span: mc.span,
                });
                let resolved_member = rewrite_expr(&member_expr, ctx)?;
                match resolved_member {
                    Expr::ExprDef(def) => {
                        return eval_expr_def_call(&def, &mc.args, mc.span, ctx);
                    }
                    Expr::MemberAccess(m) => {
                        if let Expr::Ident(tid) = m.target.as_ref() {
                            if let Some(nid) = tid.node_id() {
                                if let Some(def) =
                                    lookup_expr_def_on_node(nid, m.member.as_str(), ctx)
                                {
                                    return eval_expr_def_call(&def, &mc.args, mc.span, ctx);
                                }
                            }
                        }
                        return Err(CompileError::Custom {
                            message: format!(
                                "Property '{}' is not an '\\Expr' and cannot be called",
                                mc.method
                            ),
                            span: mc.span,
                        });
                    }
                    _ => {
                        return Err(CompileError::Custom {
                            message: format!(
                                "Property '{}' is not an '\\Expr' and cannot be called",
                                mc.method
                            ),
                            span: mc.span,
                        });
                    }
                }
            }

            let mut rewritten_args = Vec::new();
            for arg in &mc.args {
                rewritten_args.push(rewrite_expr(arg, ctx)?);
            }
            Ok(Expr::MethodCall(MethodCallExpr {
                target: Box::new(rewrite_expr(&mc.target, ctx)?),
                method: mc.method.clone(),
                args: rewritten_args,
                span: mc.span,
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

        Expr::Paren(inner, span) => Ok(Expr::Paren(Box::new(rewrite_expr(inner, ctx)?), *span)),

        Expr::Literal(Literal::Distance(val, unit, span)) => match unit {
            DistanceUnit::Px => Ok(expr.clone()),
            DistanceUnit::Pt => Ok(Expr::Binary(BinaryExpr {
                op: BinaryOp::Mul,
                left: Box::new(Expr::Literal(Literal::Distance(
                    *val,
                    DistanceUnit::Px,
                    *span,
                ))),
                right: Box::new(Expr::Binary(BinaryExpr {
                    op: BinaryOp::Div,
                    left: Box::new(Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::from_node(NodeId::WINDOW, *span))),
                        member: Ident::from_symbol(Symbol::DPI, *span),
                        span: *span,
                    })),
                    right: Box::new(Expr::Literal(Literal::Float(72.0, *span))),
                    span: *span,
                })),
                span: *span,
            })),
            DistanceUnit::Em => {
                let font_size_expr = resolve_active_font_size_expr(ctx, *span)?;
                Ok(Expr::Binary(BinaryExpr {
                    op: BinaryOp::Mul,
                    left: Box::new(Expr::Literal(Literal::Float(*val, *span))),
                    right: Box::new(font_size_expr),
                    span: *span,
                }))
            }
        },
        Expr::Literal(_) => Ok(expr.clone()),
        Expr::Node(n) => {
            let mut cloned = n.clone();
            for port in &mut cloned.ports {
                let mut rewritten_port = rewrite_expr(&port.expr, ctx)?;
                if matches!(n.name.as_str(), "Timer" | "IndefiniteTimer")
                    && port.name.as_str() == "duration"
                {
                    if let Expr::Literal(Literal::Integer(0, span)) = rewritten_port {
                        rewritten_port =
                            Expr::Literal(Literal::Duration(0, DurationUnit::S, span));
                    }
                }
                port.expr = rewritten_port;
            }
            Ok(Expr::Node(cloned))
        }
        Expr::ExprDef(def) => rewrite_expr_def_at_creation(def, ctx),
    }
}

/// Returns the initial default expression for a state binding.
pub fn default_expr_for_state(state: &StateBinding) -> Expr {
    default_expr_for_state_with_enums(state, &HashMap::new())
}

/// Returns the initial default expression for a state binding, resolving enum defaults if needed.
pub fn default_expr_for_state_with_enums(
    state: &StateBinding,
    enums: &HashMap<String, EnumDef>,
) -> Expr {
    let ty_name = state.type_annotation.as_ref().map(|t| t.name.as_str());
    if let Some(def) = &state.default {
        if let Expr::Literal(Literal::Integer(0, span)) = def {
            if ty_name == Some("Distance") {
                return Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, *span));
            } else if ty_name == Some("Area") {
                return Expr::Literal(Literal::Area(0.0, DistanceUnit::Px, *span));
            } else if ty_name == Some("Duration") {
                return Expr::Literal(Literal::Duration(0, DurationUnit::S, *span));
            } else if ty_name == Some("Float") {
                return Expr::Literal(Literal::Float(0.0, *span));
            }
        } else if let Expr::Literal(Literal::Integer(i, span)) = def {
            if ty_name == Some("Float") {
                return Expr::Literal(Literal::Float(*i as f64, *span));
            }
        }
        def.clone()
    } else if let Some(ty) = ty_name {
        match ty {
            "Boolean" => Expr::Literal(Literal::Bool(false, state.span)),
            "String" => Expr::Literal(Literal::String(String::new(), state.span)),
            "Color" => Expr::Literal(Literal::Color("#000000".to_string(), state.span)),
            "Float" => Expr::Literal(Literal::Float(0.0, state.span)),
            "Distance" => Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, state.span)),
            "Area" => Expr::Literal(Literal::Area(0.0, DistanceUnit::Px, state.span)),
            "Duration" => Expr::Literal(Literal::Duration(0, DurationUnit::S, state.span)),
            "Handler" | "Timer" | "IndefiniteTimer" => {
                Expr::Ident(Ident::new("unset", state.span))
            }
            other => {
                if let Some(enum_def) = enums.get(other) {
                    if let Some(first_var) = enum_def.variants.first() {
                        return Expr::Literal(Literal::Enum(
                            other.to_string(),
                            first_var.as_str().to_string(),
                            state.span,
                        ));
                    }
                }
                Expr::Literal(Literal::Integer(0, state.span))
            }
        }
    } else {
        Expr::Literal(Literal::Integer(0, state.span))
    }
}

/// Helper to parse an event handler binding expression: `target.method` or `method`.
pub(super) fn extract_event_binding(
    port_name: &str,
    expr: &Expr,
) -> Option<crate::component::EventHandlerBinding> {
    extract_event_binding_with_values(port_name, expr, None)
}

/// Evaluates an event handler expression (including runtime `Ternary` expressions when `values` is provided)
/// into an `EventHandlerBinding`.
pub(crate) fn extract_event_binding_with_values(
    port_name: &str,
    expr: &Expr,
    values: Option<&HashMap<crate::compiler::graph::VarId, crate::compiler::value::Value>>,
) -> Option<crate::component::EventHandlerBinding> {
    use crate::component::{EventHandlerBinding, EventHandlerTarget};
    match expr {
        Expr::Paren(inner, _) => extract_event_binding_with_values(port_name, inner, values),
        Expr::Ternary(tern) => {
            let vals = values?;
            let cond_val = crate::compiler::eval::eval_expr(&tern.condition, vals).ok()?;
            let b = cond_val.as_bool()?;
            let branch = if b { &tern.then_expr } else { &tern.else_expr };
            extract_event_binding_with_values(port_name, branch, values)
        }
        Expr::MemberAccess(m) => {
            let target = match m.target.as_ref() {
                Expr::Ident(id) => {
                    if let Some(node_id) = id.node_id() {
                        EventHandlerTarget::Component(node_id)
                    } else if id.as_str().starts_with("env.") || id.name == Symbol::ENV {
                        EventHandlerTarget::Env(m.member.as_str().to_string())
                    } else {
                        return None;
                    }
                }
                Expr::MemberAccess(inner_m) => {
                    if let Expr::Ident(root_id) = inner_m.target.as_ref() {
                        if root_id.as_str() == "env" {
                            EventHandlerTarget::Env(inner_m.member.as_str().to_string())
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                _ => return None,
            };
            Some(EventHandlerBinding {
                name: port_name.to_string(),
                target,
                method: m.member.as_str().to_string(),
            })
        }
        _ => None,
    }
}

/// Evaluates a template expression where `NodeId::TEMPLATE_SELF.<port>` looks up `slot_consts[slot]`.
/// Returns `None` without any heap allocation if any referenced port is not a known constant.
pub(super) fn eval_template_expr(
    expr: &Expr,
    port_table: &super::schema::PortTable,
    slot_consts: &[Option<crate::compiler::value::Value>],
) -> Option<crate::compiler::value::Value> {
    use crate::compiler::value::Value;
    match expr {
        Expr::Literal(_) => super::schema::expr_to_const_value(expr),
        Expr::Paren(inner, _) => eval_template_expr(inner, port_table, slot_consts),
        Expr::MemberAccess(m) => {
            if let Expr::Ident(id) = m.target.as_ref() {
                if id.node_id().is_some_and(|n| n.is_template_self()) {
                    let slot = port_table.slot_of_sym(m.member.name)?;
                    return slot_consts.get(slot.0 as usize)?.clone();
                }
            }
            None
        }
        Expr::Unary(u) => {
            let v = eval_template_expr(&u.operand, port_table, slot_consts)?;
            match u.op {
                UnaryOp::Not => match v {
                    Value::Bool(b) => Some(Value::Bool(!b)),
                    _ => None,
                },
                UnaryOp::Neg => match v {
                    Value::Integer(i) => Some(Value::Integer(-i)),
                    Value::Float(f) => Some(Value::Float(-f)),
                    Value::Distance(d) => Some(Value::Distance(-d)),
                    Value::Area(a) => Some(Value::Area(-a)),
                    Value::Duration(d) => Some(Value::Duration(-d)),
                    _ => None,
                },
            }
        }
        Expr::Binary(b) => match b.op {
            BinaryOp::And => {
                let Value::Bool(l) = eval_template_expr(&b.left, port_table, slot_consts)? else {
                    return None;
                };
                if !l {
                    return Some(Value::Bool(false));
                }
                let Value::Bool(r) = eval_template_expr(&b.right, port_table, slot_consts)? else {
                    return None;
                };
                Some(Value::Bool(r))
            }
            BinaryOp::Or => {
                let Value::Bool(l) = eval_template_expr(&b.left, port_table, slot_consts)? else {
                    return None;
                };
                if l {
                    return Some(Value::Bool(true));
                }
                let Value::Bool(r) = eval_template_expr(&b.right, port_table, slot_consts)? else {
                    return None;
                };
                Some(Value::Bool(r))
            }
            BinaryOp::Eq | BinaryOp::Ne => {
                let l = eval_template_expr(&b.left, port_table, slot_consts)?;
                let r = eval_template_expr(&b.right, port_table, slot_consts)?;
                let eq = match (&l, &r) {
                    (Value::Integer(a), Value::Integer(c)) => a == c,
                    (Value::Float(a), Value::Float(c)) => a == c,
                    (Value::Integer(a), Value::Float(c)) => (*a as f64) == *c,
                    (Value::Float(a), Value::Integer(c)) => *a == (*c as f64),
                    (Value::Distance(a), Value::Distance(c)) => a == c,
                    (Value::Distance(a), Value::Integer(0))
                    | (Value::Integer(0), Value::Distance(a)) => *a == 0.0,
                    (Value::Area(a), Value::Area(c)) => a == c,
                    (Value::Duration(a), Value::Duration(c)) => a == c,
                    (Value::String(a), Value::String(c)) => a == c,
                    (Value::Bool(a), Value::Bool(c)) => a == c,
                    (Value::Color(a), Value::Color(c)) => a == c,
                    (
                        Value::Enum {
                            enum_name: e1,
                            variant: v1,
                        },
                        Value::Enum {
                            enum_name: e2,
                            variant: v2,
                        },
                    ) => e1 == e2 && v1 == v2,
                    _ => return None,
                };
                Some(Value::Bool(if b.op == BinaryOp::Eq { eq } else { !eq }))
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                let l = eval_template_expr(&b.left, port_table, slot_consts)?;
                let r = eval_template_expr(&b.right, port_table, slot_consts)?;
                let (lv, rv) = match (&l, &r) {
                    (Value::Integer(a), Value::Integer(c)) => (*a as f64, *c as f64),
                    (Value::Float(a), Value::Float(c)) => (*a, *c),
                    (Value::Integer(a), Value::Float(c)) => (*a as f64, *c),
                    (Value::Float(a), Value::Integer(c)) => (*a, *c as f64),
                    (Value::Distance(a), Value::Distance(c)) => (*a, *c),
                    (Value::Distance(a), Value::Integer(0)) => (*a, 0.0),
                    (Value::Integer(0), Value::Distance(c)) => (0.0, *c),
                    _ => return None,
                };
                let res = match b.op {
                    BinaryOp::Lt => lv < rv,
                    BinaryOp::Le => lv <= rv,
                    BinaryOp::Gt => lv > rv,
                    BinaryOp::Ge => lv >= rv,
                    _ => unreachable!(),
                };
                Some(Value::Bool(res))
            }
            _ => None,
        },
        Expr::Ternary(t) => {
            let Value::Bool(c) = eval_template_expr(&t.condition, port_table, slot_consts)? else {
                return None;
            };
            if c {
                eval_template_expr(&t.then_expr, port_table, slot_consts)
            } else {
                eval_template_expr(&t.else_expr, port_table, slot_consts)
            }
        }
        _ => None,
    }
}

/// Instantiates a pre-compiled `NodeId::TEMPLATE_SELF` expression for `comp_node_id`,
/// folding constant ternaries on the fly without cloning unchosen branches.
pub(super) fn instantiate_template_expr(
    expr: &Expr,
    comp_node_id: NodeId,
    port_table: &super::schema::PortTable,
    slot_consts: &[Option<crate::compiler::value::Value>],
) -> Expr {
    match expr {
        Expr::Literal(lit) => Expr::Literal(lit.clone()),
        Expr::Ident(id) => {
            if id.node_id().is_some_and(|n| n.is_template_self()) {
                Expr::Ident(Ident::from_node(comp_node_id, id.span))
            } else {
                Expr::Ident(*id)
            }
        }
        Expr::MemberAccess(m) => {
            if let Expr::Ident(id) = m.target.as_ref() {
                if id.node_id().is_some_and(|n| n.is_template_self()) {
                    return Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::from_node(comp_node_id, id.span))),
                        member: m.member,
                        span: m.span,
                    });
                }
            }
            Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(instantiate_template_expr(
                    &m.target,
                    comp_node_id,
                    port_table,
                    slot_consts,
                )),
                member: m.member,
                span: m.span,
            })
        }
        Expr::Ternary(t) => {
            if let Some(crate::compiler::value::Value::Bool(b)) =
                eval_template_expr(&t.condition, port_table, slot_consts)
            {
                let chosen = if b { &t.then_expr } else { &t.else_expr };
                return instantiate_template_expr(chosen, comp_node_id, port_table, slot_consts);
            }
            record_unfolded_ternary_emitted();
            Expr::Ternary(TernaryExpr {
                condition: Box::new(instantiate_template_expr(
                    &t.condition,
                    comp_node_id,
                    port_table,
                    slot_consts,
                )),
                then_expr: Box::new(instantiate_template_expr(
                    &t.then_expr,
                    comp_node_id,
                    port_table,
                    slot_consts,
                )),
                else_expr: Box::new(instantiate_template_expr(
                    &t.else_expr,
                    comp_node_id,
                    port_table,
                    slot_consts,
                )),
                span: t.span,
            })
        }
        Expr::Binary(b) => Expr::Binary(BinaryExpr {
            op: b.op,
            left: Box::new(instantiate_template_expr(
                &b.left,
                comp_node_id,
                port_table,
                slot_consts,
            )),
            right: Box::new(instantiate_template_expr(
                &b.right,
                comp_node_id,
                port_table,
                slot_consts,
            )),
            span: b.span,
        }),
        Expr::Unary(u) => Expr::Unary(UnaryExpr {
            op: u.op,
            operand: Box::new(instantiate_template_expr(
                &u.operand,
                comp_node_id,
                port_table,
                slot_consts,
            )),
            span: u.span,
        }),
        Expr::Paren(inner, span) => Expr::Paren(
            Box::new(instantiate_template_expr(
                inner,
                comp_node_id,
                port_table,
                slot_consts,
            )),
            *span,
        ),
        Expr::Call(c) => {
            let mut args = Vec::with_capacity(c.args.len());
            for arg in &c.args {
                args.push(instantiate_template_expr(
                    arg,
                    comp_node_id,
                    port_table,
                    slot_consts,
                ));
            }
            Expr::Call(CallExpr {
                callee: c.callee,
                args,
                span: c.span,
            })
        }
        Expr::MethodCall(mc) => {
            let mut args = Vec::with_capacity(mc.args.len());
            for arg in &mc.args {
                args.push(instantiate_template_expr(
                    arg,
                    comp_node_id,
                    port_table,
                    slot_consts,
                ));
            }
            Expr::MethodCall(MethodCallExpr {
                target: Box::new(instantiate_template_expr(
                    &mc.target,
                    comp_node_id,
                    port_table,
                    slot_consts,
                )),
                method: mc.method,
                args,
                span: mc.span,
            })
        }
        Expr::Node(n) => Expr::Node(n.clone()),
        Expr::ExprDef(def) => Expr::ExprDef(def.clone()),
    }
}

