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
    scope: &HashMap<String, LexicalBinding>,
) -> Expr {
    match expr {
        Expr::Ident(id) => {
            if let Some(binding) = scope.get(id.as_str()) {
                match binding {
                    LexicalBinding::Node(node_id) => {
                        Expr::Ident(Ident::new(node_id.canonical_name(), id.span))
                    }
                    LexicalBinding::Expr(e) | LexicalBinding::State(e) => e.clone(),
                    LexicalBinding::Uninitialized => expr.clone(),
                }
            } else {
                expr.clone()
            }
        }
        Expr::MemberAccess(m) => {
            let target_ident_name = match m.target.as_ref() {
                Expr::Ident(id) => Some(id.as_str()),
                _ => None,
            };
            if let Some(target_name) = target_ident_name {
                if let Some(LexicalBinding::Node(node_id)) = scope.get(target_name) {
                    return Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(
                            node_id.canonical_name(),
                            m.target.span(),
                        ))),
                        member: m.member.clone(),
                        span: m.span,
                    });
                }
            }
            Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(resolve_forward_scope_refs(&m.target, scope)),
                member: m.member.clone(),
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
        _ => expr.clone(),
    }
}

/// Resolves forward lexical scope references in node ports and local bindings starting from the given indices.
pub(super) fn resolve_forward_refs_in_range(
    doc: &mut crate::compiler::expanded::ExpandedDocument,
    start_node_idx: usize,
    start_binding_idx: usize,
    scope: &HashMap<String, LexicalBinding>,
) {
    for node in &mut doc.nodes[start_node_idx..] {
        for expr in node.ports.values_mut() {
            *expr = resolve_forward_scope_refs(expr, scope);
        }
    }
    for (expr, _, _) in &mut doc.local_bindings[start_binding_idx..] {
        *expr = resolve_forward_scope_refs(expr, scope);
    }
}

/// Applies optional `type_annotation` coercion to `rewritten_res` and updates `scope[name]`.
/// Returns `Some(rewritten)` if the binding changed.
pub(super) fn try_preresolve_expr_binding(
    scope: &mut HashMap<String, LexicalBinding>,
    name: &str,
    rewritten_res: Result<Expr, CompileError>,
    type_annotation: Option<&TypeRef>,
) -> Option<Expr> {
    let rewritten = rewritten_res.ok()?;
    let rewritten = super::typecheck::coerce_typed_binding_expr(rewritten, type_annotation);
    if let Some(LexicalBinding::Expr(existing)) = scope.get(name) {
        if existing == &rewritten {
            return None;
        }
    }
    scope.insert(name.to_string(), LexicalBinding::Expr(rewritten.clone()));
    Some(rewritten)
}

/// Evaluates sibling existence conditions (`prev`, `next`, `!prev`, `!next`, `&&`, `||`) at expansion time.
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
                target: Box::new(Expr::Ident(Ident::new(fid.canonical_name(), span))),
                member: Ident::new("size", span),
                span,
            }));
        }
    }
    match ctx.env_scope.get("font") {
        Some(EnvEntry::Bound(font_expr)) => {
            let rewritten_font = rewrite_expr(font_expr, ctx)?;
            Ok(Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(rewritten_font),
                member: Ident::new("size", span),
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
            if id.as_str() == "env" {
                return Err(CompileError::BareEnvUse { span: id.span });
            }
            if ctx.enums.contains_key(id.as_str()) {
                return Err(CompileError::BareEnumUse {
                    name: id.as_str().to_string(),
                    span: id.span,
                });
            }

            // Check lexical scope first (let bindings, component ports, named nodes)
            if id.as_str() != "self"
                && id.as_str() != "parent"
                && id.as_str() != "window"
                && id.as_str() != "prev"
                && id.as_str() != "next"
                && id.as_str() != "child"
            {
                if let Some(binding) = ctx.lexical_scope.get(id.as_str()) {
                    match binding {
                        LexicalBinding::Expr(e) | LexicalBinding::State(e) => return Ok(e.clone()),
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

            // In an event port (on_*), a bare method name that wasn't in lexical_scope
            // resolves to the enclosing component's method (or current_node if in signature default).
            if ctx.is_event_port
                && id.as_str() != "unset"
                && id.as_str() != "None"
                && id.as_str() != "self"
                && id.as_str() != "parent"
                && id.as_str() != "window"
                && id.as_str() != "prev"
                && id.as_str() != "next"
                && id.as_str() != "child"
                && NodeId::from_canonical_name(id.as_str()).is_none()
            {
                let target_id = ctx.enclosing_component.unwrap_or(ctx.current_node);
                return Ok(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(target_id.canonical_name(), id.span))),
                    member: id.clone(),
                    span: id.span,
                }));
            }

            // Check if it's a port on the current node (e.g. component parameter in default expression)
            if ctx.current_ports.iter().any(|p| p == id.as_str()) {
                return Ok(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(
                        ctx.current_node.canonical_name(),
                        id.span,
                    ))),
                    member: id.clone(),
                    span: id.span,
                }));
            }

            if id.as_str() == "window" {
                return Ok(Expr::Ident(Ident::new(
                    NodeId::WINDOW.canonical_name(),
                    id.span,
                )));
            }
            if id.as_str() == "self" {
                return Err(CompileError::Custom {
                    message: "Bare 'self' is not allowed; use 'self.<method>' in an event handler or 'self.<property>' to access a shadowed component property".to_string(),
                    span: id.span,
                });
            }
            if id.as_str() == "child" {
                if let Some(cid) = ctx.child_node {
                    return Ok(Expr::Ident(Ident::new(cid.canonical_name(), id.span)));
                } else {
                    return Err(CompileError::Custom {
                        message: "'child' can only be used inside a '\\Children(...)' block"
                            .to_string(),
                        span: id.span,
                    });
                }
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
            if id.as_str() == "next" {
                if ctx.has_next_sibling {
                    if let Some(cid) = ctx.child_node {
                        return Ok(Expr::Ident(Ident::new(
                            format!("__next_of_{}", cid.canonical_name()),
                            id.span,
                        )));
                    }
                }
            }
            if id.as_str() == "num_children" {
                return Err(CompileError::Custom {
                    message: "'num_children' is no longer supported; use 'children.len' instead"
                        .to_string(),
                    span: id.span,
                });
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
                            _ => {}
                        }
                    }
                }
                return Ok(chosen);
            }
            let rewritten_cond = rewrite_expr(&tern.condition, ctx)?;
            let mut const_env = HashMap::new();
            if let Some(comp_ports) = ctx.comp_ports {
                for (port_name, port_expr) in comp_ports.iter() {
                    if let Some(states) = ctx.declared_state_names {
                        if states.contains(port_name) {
                            continue;
                        }
                    }
                    if let Some(lit) = resolve_ident_or_member_to_literal(port_expr, ctx) {
                        let val = match lit {
                            Literal::Integer(i, _) => crate::compiler::value::Value::Integer(i),
                            Literal::Float(n, _) => crate::compiler::value::Value::Float(n),
                            Literal::Distance(d, _, _) => {
                                crate::compiler::value::Value::Distance(d)
                            }
                            Literal::Area(a, _, _) => crate::compiler::value::Value::Area(a),
                            Literal::String(s, _) => crate::compiler::value::Value::String(s),
                            Literal::Bool(b, _) => crate::compiler::value::Value::Bool(b),
                            Literal::Color(c, _) => crate::compiler::value::Value::Color(c),
                            Literal::Enum(e, v, _) => crate::compiler::value::Value::Enum {
                                enum_name: e,
                                variant: v,
                            },
                        };
                        const_env.insert(
                            crate::compiler::graph::VarId::new(ctx.current_node, port_name),
                            val.clone(),
                        );
                        if let Some(parent) = ctx.parent_node {
                            const_env.insert(
                                crate::compiler::graph::VarId::new(parent, port_name),
                                val,
                            );
                        }
                    } else if let Ok(val) =
                        crate::compiler::eval::eval_expr(port_expr, &HashMap::new())
                    {
                        const_env.insert(
                            crate::compiler::graph::VarId::new(ctx.current_node, port_name),
                            val.clone(),
                        );
                        if let Some(parent) = ctx.parent_node {
                            const_env.insert(
                                crate::compiler::graph::VarId::new(parent, port_name),
                                val,
                            );
                        }
                    }
                }
            }
            if let Ok(crate::compiler::value::Value::Bool(b)) =
                crate::compiler::eval::eval_expr(&rewritten_cond, &const_env)
            {
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
                            _ => {}
                        }
                    }
                }
                return Ok(chosen);
            }
            Ok(Expr::Ternary(TernaryExpr {
                condition: Box::new(rewritten_cond),
                then_expr: Box::new(rewrite_expr(&tern.then_expr, ctx)?),
                else_expr: Box::new(rewrite_expr(&tern.else_expr, ctx)?),
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

            let target_ident_name = match m.target.as_ref() {
                Expr::Ident(id) => Some(id.as_str().to_string()),
                _ => None,
            };

            // Enum variant access and children.len
            if let Some(target_name) = &target_ident_name {
                if target_name == "children" {
                    if m.member.as_str() == "len" {
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
                if let Some(enum_def) = ctx.enums.get(target_name) {
                    let variant_name = m.member.as_str();
                    if enum_def.has_variant(variant_name) {
                        return Ok(Expr::Literal(Literal::Enum(
                            target_name.clone(),
                            variant_name.to_string(),
                            m.span,
                        )));
                    } else {
                        return Err(CompileError::UnknownEnumVariant {
                            enum_name: target_name.clone(),
                            variant: variant_name.to_string(),
                            span: m.member.span,
                        });
                    }
                }
            }

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

            // Resolve target (parent, window, prev, next, self, child, or named node in lexical scope)
            let resolved_target = if let Some(target_name) = &target_ident_name {
                if target_name == "self" {
                    if ctx.is_event_port {
                        Expr::Ident(Ident::new(
                            ctx.current_node.canonical_name(),
                            m.target.span(),
                        ))
                    } else if let (Some(comp_id), Some(shadowed)) =
                        (ctx.enclosing_component, ctx.shadowed_ports)
                    {
                        if shadowed.contains(m.member.as_str()) {
                            Expr::Ident(Ident::new(comp_id.canonical_name(), m.target.span()))
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
                } else if target_name == "child" {
                    if let Some(cid) = ctx.child_node {
                        Expr::Ident(Ident::new(cid.canonical_name(), m.target.span()))
                    } else {
                        return Err(CompileError::Custom {
                            message: "'child' can only be used inside a '\\Children(...)' block"
                                .to_string(),
                            span: m.target.span(),
                        });
                    }
                } else if let Some(LexicalBinding::Node(node_id)) =
                    ctx.lexical_scope.get(target_name)
                {
                    Expr::Ident(Ident::new(node_id.canonical_name(), m.target.span()))
                } else if target_name == "parent" {
                    if let Some(parent) = ctx.parent_node {
                        if m.member.as_str() != "parent"
                            && !ctx.parent_ports.is_empty()
                            && !ctx.parent_ports.contains(&m.member.as_str().to_string())
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
                        Expr::Ident(Ident::new(parent.canonical_name(), m.target.span()))
                    } else {
                        rewrite_expr(&m.target, ctx)?
                    }
                } else if target_name == "window" {
                    Expr::Ident(Ident::new(
                        NodeId::WINDOW.canonical_name(),
                        m.target.span(),
                    ))
                } else if target_name == "prev" {
                    if let Some(prev) = ctx.prev_sibling {
                        Expr::Ident(Ident::new(prev.canonical_name(), m.target.span()))
                    } else {
                        rewrite_expr(&m.target, ctx)?
                    }
                } else if target_name == "next" {
                    if ctx.has_next_sibling {
                        if let Some(cid) = ctx.child_node {
                            Expr::Ident(Ident::new(
                                format!("__next_of_{}", cid.canonical_name()),
                                m.target.span(),
                            ))
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
                Expr::Ident(id) => NodeId::from_canonical_name(id.as_str()),
                _ => None,
            };

            // Chained parent access: e.g. parent.parent -> parent_id of target_node_id
            if m.member.as_str() == "parent" {
                if let Some(tid) = target_node_id {
                    let parent_id = if Some(tid) == ctx.child_node {
                        ctx.parent_node
                    } else {
                        ctx.nodes.get(tid.0).and_then(|n| n.parent)
                    };
                    if let Some(pid) = parent_id {
                        return Ok(Expr::Ident(Ident::new(pid.canonical_name(), m.span)));
                    } else {
                        return Err(CompileError::Custom {
                            message: "Node has no parent".to_string(),
                            span: m.span,
                        });
                    }
                }
            }

            // Font node reference access: e.g. label.font -> font_node_id
            if m.member.as_str() == "font" {
                if let Some(tid) = target_node_id {
                    if let Some(&fid) = ctx.node_fonts.get(&tid) {
                        return Ok(Expr::Ident(Ident::new(fid.canonical_name(), m.span)));
                    }
                }
            }

            if let Some(tid) = target_node_id {
                if let Some(target_node) = ctx.nodes.get(tid.0) {
                    if target_node.private_ports.contains(m.member.as_str())
                        && ctx.current_node != tid
                        && ctx.enclosing_component != Some(tid)
                        && target_ident_name.as_deref() != Some("parent")
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
                member: m.member.clone(),
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
                        if target_id.as_str() == "children" && m.member.as_str() != "len" {
                            had_children = true;
                            children_member = Some(m.member.as_str().to_string());
                            if let Some(comp_id) = ctx.enclosing_component {
                                for idx in 0..ctx.num_children {
                                    let target_name = format!(
                                        "__agg_child_of_{}_{}",
                                        comp_id.canonical_name(),
                                        idx
                                    );
                                    let child_access = Expr::MemberAccess(MemberAccessExpr {
                                        target: Box::new(Expr::Ident(Ident::new(
                                            target_name,
                                            target_id.span,
                                        ))),
                                        member: m.member.clone(),
                                        span: m.span,
                                    });
                                    rewritten_args.push(child_access);
                                }
                            } else if ctx.child_ids.len() == ctx.num_children {
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
                            }
                            continue;
                        }
                    }
                }
                rewritten_args.push(rewrite_expr(arg, ctx)?);
            }

            if had_children && rewritten_args.is_empty() && call.callee.as_str() != "only" {
                let is_int_member = matches!(
                    children_member.as_deref(),
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
                        target: Box::new(Expr::Ident(Ident::new(
                            NodeId::WINDOW.canonical_name(),
                            *span,
                        ))),
                        member: Ident::new("dpi", *span),
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
        Expr::Node(n) => Ok(Expr::Node(n.clone())),
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
            "Handler" => Expr::Ident(Ident::new("unset", state.span)),
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
    use crate::component::{EventHandlerBinding, EventHandlerTarget};
    match expr {
        Expr::MemberAccess(m) => {
            let target = match m.target.as_ref() {
                Expr::Ident(id) => {
                    if let Some(node_id) = NodeId::from_canonical_name(id.as_str()) {
                        EventHandlerTarget::Component(node_id)
                    } else if id.as_str().starts_with("env.") || id.as_str() == "env" {
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

