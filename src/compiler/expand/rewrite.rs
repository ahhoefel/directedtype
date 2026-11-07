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
}

pub(in crate::compiler::expand) fn reset_rewrite_tracking() {
    PENDING_FWD_NODES.with(|c| c.borrow_mut().clear());
    PENDING_FWD_BINDINGS.with(|c| c.borrow_mut().clear());
    FWD_REF_COUNT.with(|c| c.set(0));
    UNFOLDED_TERNARY_COUNT.with(|c| c.set(0));
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
        Expr::Node(_) => false,
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

/// Recursively rewrites an expression into canonical node variable references and resolves derived aliases.
pub fn rewrite_expr(expr: &Expr, ctx: &ScopeContext<'_>) -> Result<Expr, CompileError> {
    match expr {
        Expr::Ident(id) => {
            if id.name == Symbol::ENV {
                return Err(CompileError::BareEnvUse { span: id.span });
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
                        && !target_ident.is_some_and(|id| id.name == Symbol::PARENT)
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
            }

            Ok(Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(resolved_target),
                member: m.member,
                span: m.span,
            }))
        }

        Expr::Call(call) => {
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
    }
}

