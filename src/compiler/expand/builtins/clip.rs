use crate::ast::{Expr, Ident, MemberAccessExpr};
use crate::compiler::error::CompileError;
use crate::compiler::expand::builtins::{
    default_builtin_port_type, BuiltinPrimitive, PrimitiveExpandState,
};
use crate::compiler::expand::context::EnvEntry;
use crate::compiler::expand::typecheck::StaticType;
use crate::compiler::expanded::NodeId;
use crate::span::Span;
use std::collections::HashMap;

pub struct ClipPrimitive;

/// Resolves the inherited `clip` expression from `env_scope` (`\env clip`) or `parent_id.clip` (falling back to `window.clip`).
pub(in crate::compiler::expand) fn default_inherited_clip_expr(
    current_node: NodeId,
    parent_id: Option<NodeId>,
    env_scope: &HashMap<String, EnvEntry>,
    span: Span,
) -> Expr {
    if let Some(EnvEntry::Bound(clip_expr)) = env_scope.get("clip") {
        if crate::compiler::expand::rewrite::has_any_pending_fwd_refs()
            && crate::compiler::expand::rewrite::expr_has_unresolved_ident(clip_expr)
        {
            crate::compiler::expand::rewrite::record_pending_fwd_node(current_node);
        }
        clip_expr.clone()
    } else if let Some(parent) = parent_id {
        let target_node = if parent.is_window() {
            NodeId::WINDOW
        } else {
            parent
        };
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(Expr::Ident(Ident::from_node(target_node, span))),
            member: Ident::new("clip", span),
            span,
        })
    } else {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(Expr::Ident(Ident::from_node(
                NodeId::WINDOW,
                span,
            ))),
            member: Ident::new("clip", span),
            span,
        })
    }
}

impl BuiltinPrimitive for ClipPrimitive {
    fn name(&self) -> &'static str {
        "Clip"
    }

    fn accepts_port(&self, port_name: &str) -> bool {
        matches!(
            port_name,
            "box" | "up" | "clip" | "x" | "y" | "width" | "height" | "z" | "scroll_x" | "scroll_y" | "id"
        )
    }

    fn port_type(&self, port_name: &str) -> Option<StaticType> {
        match port_name {
            "up" | "clip" => Some(StaticType::Node("Clip".to_string())),
            _ => default_builtin_port_type(port_name),
        }
    }

    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        let elem = state.elem;
        let node_id = state.node_id;
        let ctx = state.ctx;

        if !state.ports.contains_key("box") {
            return Err(CompileError::MissingPort {
                node: "Clip".to_string(),
                port: "box".to_string(),
                span: elem.span,
            });
        }
        if !state.ports.contains_key("up") {
            let default_up =
                default_inherited_clip_expr(node_id, ctx.parent_id, ctx.env_scope, elem.span);
            state.ports.insert("up".to_string(), default_up);
        }
        state.ports.insert(
            "clip".to_string(),
            Expr::Ident(Ident::from_node(node_id, elem.span)),
        );
        if let Some(box_expr) = state.ports.get("box").cloned() {
            state.ports.entry("x".to_string()).or_insert_with(|| {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(box_expr.clone()),
                    member: Ident::new("x", elem.span),
                    span: elem.span,
                })
            });
            state.ports.entry("y".to_string()).or_insert_with(|| {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(box_expr.clone()),
                    member: Ident::new("y", elem.span),
                    span: elem.span,
                })
            });
            state.ports.entry("width".to_string()).or_insert_with(|| {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(box_expr.clone()),
                    member: Ident::new("width", elem.span),
                    span: elem.span,
                })
            });
            state.ports.entry("height".to_string()).or_insert_with(|| {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(box_expr),
                    member: Ident::new("height", elem.span),
                    span: elem.span,
                })
            });
        }
        Ok(())
    }

    fn overload_params(&self, _state: &PrimitiveExpandState<'_, '_>) -> Vec<String> {
        vec!["box".to_string()]
    }
}

