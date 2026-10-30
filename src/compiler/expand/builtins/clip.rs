use crate::ast::{Expr, Ident, MemberAccessExpr};
use crate::compiler::error::CompileError;
use crate::compiler::expand::builtins::{BuiltinPrimitive, PrimitiveExpandState};
use crate::compiler::expand::context::EnvEntry;
use crate::compiler::expanded::NodeId;

pub struct ClipPrimitive;

impl BuiltinPrimitive for ClipPrimitive {
    fn name(&self) -> &'static str {
        "Clip"
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
            let default_up = if let Some(EnvEntry::Bound(clip_expr)) = ctx.env_scope.get("clip") {
                clip_expr.clone()
            } else if let Some(parent) = ctx.parent_id {
                if parent.is_window() {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(
                            NodeId::WINDOW.canonical_name(),
                            elem.span,
                        ))),
                        member: Ident::new("clip", elem.span),
                        span: elem.span,
                    })
                } else {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(
                            parent.canonical_name(),
                            elem.span,
                        ))),
                        member: Ident::new("clip", elem.span),
                        span: elem.span,
                    })
                }
            } else {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(
                        NodeId::WINDOW.canonical_name(),
                        elem.span,
                    ))),
                    member: Ident::new("clip", elem.span),
                    span: elem.span,
                })
            };
            state.ports.insert("up".to_string(), default_up);
        }
        state.ports.insert(
            "clip".to_string(),
            Expr::Ident(Ident::new(node_id.canonical_name(), elem.span)),
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

