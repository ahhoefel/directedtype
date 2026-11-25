use crate::ast::{
    BinaryExpr, BinaryOp, CallExpr, ComponentDef, ElementNode, Expr, Ident, MemberAccessExpr,
    Symbol,
};
use crate::compiler::error::{CompileError, NoMatchingOverloadDetails};
use crate::compiler::expand::builtins::{
    default_builtin_port_type, BuiltinPrimitive, PrimitiveExpandState,
};
use crate::compiler::expand::context::ElementContext;
use crate::compiler::expand::typecheck::StaticType;
use crate::compiler::expanded::{ExpandedDocument, NodeId};
use crate::compiler::scope::ScopeId;
use std::collections::HashMap;

pub struct ImagePrimitive;

impl BuiltinPrimitive for ImagePrimitive {
    fn name(&self) -> &'static str {
        "Image"
    }

    fn is_immutable_alias_port(&self, port_name: &str) -> bool {
        matches!(
            port_name,
            "left" | "top" | "right" | "bottom" | "aspect_ratio" | "file_dpi"
        )
    }

    fn accepted_ports(&self) -> &'static [&'static str] {
        &[
            "src",
            "x",
            "y",
            "width",
            "height",
            "dpi",
            "z",
            "border_color",
            "border_width",
            "radius",
            "radius_left",
            "radius_right",
            "radius_top",
            "radius_bottom",
            "radius_top_left",
            "radius_top_right",
            "radius_bottom_right",
            "radius_bottom_left",
            "shadow",
            "clip",
            "pagination",
            "cursor",
            "opacity",
            "alt",
            "name",
            "path",
            "id",
        ]
    }

    fn accepts_event_handlers(&self) -> bool {
        true
    }

    fn port_type(&self, port_name: &str) -> Option<StaticType> {
        match port_name {
            "src" => Some(StaticType::Path),
            "dpi" | "aspect_ratio" | "file_dpi" | "opacity" => Some(StaticType::Float),
            "alt" => Some(StaticType::String),
            "radius"
            | "radius_left"
            | "radius_right"
            | "radius_top"
            | "radius_bottom"
            | "radius_top_left"
            | "radius_top_right"
            | "radius_bottom_right"
            | "radius_bottom_left"
            | "border_width"
            | "shadow_offset_x"
            | "shadow_offset_y"
            | "shadow_blur"
            | "shadow_spread" => Some(StaticType::Distance),
            "border_color" | "shadow_color" => Some(StaticType::Color),
            "shadow" => Some(StaticType::Node("Shadow".to_string())),
            _ => default_builtin_port_type(port_name),
        }
    }

    fn scope_ports(&self) -> Vec<String> {
        vec![
            "src".to_string(),
            "x".to_string(),
            "y".to_string(),
            "width".to_string(),
            "height".to_string(),
            "z".to_string(),
            "left".to_string(),
            "top".to_string(),
            "right".to_string(),
            "bottom".to_string(),
            "aspect_ratio".to_string(),
            "file_dpi".to_string(),
            "clip".to_string(),
            "name".to_string(),
            "path".to_string(),
        ]
    }

    fn pre_expand_children(
        &self,
        _node_id: NodeId,
        elem: &ElementNode,
        ctx: &ElementContext<'_>,
        _doc: &mut ExpandedDocument,
        _registry: &HashMap<String, Vec<ComponentDef>>,
        _node_fonts: &HashMap<NodeId, NodeId>,
    ) -> Result<ScopeId, CompileError> {
        if elem.content.as_ref().is_some_and(|slot| !slot.items.is_empty()) {
            return Err(CompileError::Custom {
                message: "Node 'Image' does not accept child content inside '{ ... }'".to_string(),
                span: elem.span,
            });
        }
        Ok(ctx.active_scope_id)
    }

    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        let elem = state.elem;
        let span = elem.span;

        for req in ["src", "x", "y"] {
            if !state.ports.contains_key(req) {
                return Err(CompileError::MissingPort {
                    node: "Image".to_string(),
                    port: req.to_string(),
                    span,
                });
            }
        }

        let has_width = state.ports.contains_key("width");
        let has_height = state.ports.contains_key("height");
        let has_dpi = state.ports.contains_key("dpi");

        if (has_width && has_height) || (has_dpi && (has_width || has_height)) {
            let mut provided: Vec<String> = state
                .authored_ports
                .keys()
                .filter(|p| *p != "clip" && *p != "pagination" && !p.starts_with("on_"))
                .cloned()
                .collect();
            provided.sort();
            return Err(CompileError::NoMatchingOverload(Box::new(
                NoMatchingOverloadDetails {
                    name: "Image".to_string(),
                    provided_ports: provided,
                    available_signatures: vec![
                        vec![
                            "src".to_string(),
                            "x".to_string(),
                            "y".to_string(),
                            "dpi".to_string(),
                        ],
                        vec![
                            "src".to_string(),
                            "x".to_string(),
                            "y".to_string(),
                            "width".to_string(),
                        ],
                        vec![
                            "src".to_string(),
                            "x".to_string(),
                            "y".to_string(),
                            "height".to_string(),
                        ],
                    ],
                    span,
                },
            )));
        }

        let self_ident = Expr::Ident(Ident::from_node(state.node_id, span));
        let self_src = Expr::MemberAccess(MemberAccessExpr {
            target: std::sync::Arc::new(self_ident.clone()),
            member: Ident::new("src", span),
            span,
        });
        let win_dpi = Expr::MemberAccess(MemberAccessExpr {
            target: std::sync::Arc::new(Expr::Ident(Ident::from_symbol(Symbol::WINDOW, span))),
            member: Ident::from_symbol(Symbol::DPI, span),
            span,
        });

        let aspect_ratio_expr = Expr::Call(CallExpr {
            callee: Ident::new("__image_aspect_ratio", span),
            args: vec![self_src.clone()],
            span,
        });
        state
            .ports
            .insert("aspect_ratio".to_string(), aspect_ratio_expr.clone());
        state
            .authored_ports
            .insert("aspect_ratio".to_string(), aspect_ratio_expr);

        let file_dpi_expr = Expr::Call(CallExpr {
            callee: Ident::new("__image_file_dpi", span),
            args: vec![self_src.clone(), win_dpi.clone()],
            span,
        });
        state
            .ports
            .insert("file_dpi".to_string(), file_dpi_expr.clone());
        state
            .authored_ports
            .insert("file_dpi".to_string(), file_dpi_expr);

        if has_width {
            let self_width = Expr::MemberAccess(MemberAccessExpr {
                target: std::sync::Arc::new(self_ident.clone()),
                member: Ident::from_symbol(Symbol::WIDTH, span),
                span,
            });
            let self_ar = Expr::MemberAccess(MemberAccessExpr {
                target: std::sync::Arc::new(self_ident),
                member: Ident::new("aspect_ratio", span),
                span,
            });
            let height_expr = Expr::Binary(BinaryExpr {
                op: BinaryOp::Div,
                left: std::sync::Arc::new(self_width),
                right: std::sync::Arc::new(self_ar),
                span,
            });
            state.ports.insert("height".to_string(), height_expr);
            state.authored_ports.insert(
                "height".to_string(),
                Expr::Binary(BinaryExpr {
                    op: BinaryOp::Div,
                    left: std::sync::Arc::new(Expr::Ident(Ident::new("width", span))),
                    right: std::sync::Arc::new(Expr::Ident(Ident::new("aspect_ratio", span))),
                    span,
                }),
            );
        } else if has_height {
            let self_height = Expr::MemberAccess(MemberAccessExpr {
                target: std::sync::Arc::new(self_ident.clone()),
                member: Ident::from_symbol(Symbol::HEIGHT, span),
                span,
            });
            let self_ar = Expr::MemberAccess(MemberAccessExpr {
                target: std::sync::Arc::new(self_ident),
                member: Ident::new("aspect_ratio", span),
                span,
            });
            let width_expr = Expr::Binary(BinaryExpr {
                op: BinaryOp::Mul,
                left: std::sync::Arc::new(self_height),
                right: std::sync::Arc::new(self_ar),
                span,
            });
            state.ports.insert("width".to_string(), width_expr);
            state.authored_ports.insert(
                "width".to_string(),
                Expr::Binary(BinaryExpr {
                    op: BinaryOp::Mul,
                    left: std::sync::Arc::new(Expr::Ident(Ident::new("height", span))),
                    right: std::sync::Arc::new(Expr::Ident(Ident::new("aspect_ratio", span))),
                    span,
                }),
            );
        } else {
            if !has_dpi {
                state.ports.insert("dpi".to_string(), win_dpi.clone());
                state.authored_ports.insert(
                    "dpi".to_string(),
                    Expr::MemberAccess(MemberAccessExpr {
                        target: std::sync::Arc::new(Expr::Ident(Ident::new("window", span))),
                        member: Ident::new("dpi", span),
                        span,
                    }),
                );
            }
            let self_dpi = Expr::MemberAccess(MemberAccessExpr {
                target: std::sync::Arc::new(self_ident),
                member: Ident::new("dpi", span),
                span,
            });
            let width_expr = Expr::Call(CallExpr {
                callee: Ident::new("__image_intrinsic_width", span),
                args: vec![self_src.clone(), self_dpi.clone(), win_dpi.clone()],
                span,
            });
            let height_expr = Expr::Call(CallExpr {
                callee: Ident::new("__image_intrinsic_height", span),
                args: vec![self_src, self_dpi, win_dpi],
                span,
            });
            state.ports.insert("width".to_string(), width_expr.clone());
            state.authored_ports.insert("width".to_string(), width_expr);
            state
                .ports
                .insert("height".to_string(), height_expr.clone());
            state
                .authored_ports
                .insert("height".to_string(), height_expr);
        }

        if let Some(sh_target) = state.ports.get("shadow").cloned() {
            for field in ["offset_x", "offset_y", "blur", "spread", "color"] {
                let prop_name = format!("shadow_{field}");
                if !state.ports.contains_key(&prop_name) {
                    let member_expr = Expr::MemberAccess(MemberAccessExpr {
                        target: std::sync::Arc::new(sh_target.clone()),
                        member: Ident::new(field, span),
                        span,
                    });
                    state.ports.insert(prop_name.clone(), member_expr.clone());
                    state.authored_ports.insert(prop_name, member_expr);
                }
            }
        }

        let port_types = std::sync::Arc::make_mut(&mut state.doc.nodes[state.node_id.0].port_types);
        port_types.insert("src".to_string(), "Path".to_string());
        port_types.insert("width".to_string(), "Distance".to_string());
        port_types.insert("height".to_string(), "Distance".to_string());
        port_types.insert("aspect_ratio".to_string(), "Float".to_string());
        port_types.insert("file_dpi".to_string(), "Float".to_string());
        if state.ports.contains_key("dpi") {
            port_types.insert("dpi".to_string(), "Float".to_string());
        }

        Ok(())
    }

    fn overload_params(&self, state: &PrimitiveExpandState<'_, '_>) -> Vec<String> {
        if state.authored_ports.contains_key("width")
            && !matches!(
                state.authored_ports.get("width"),
                Some(Expr::Call(c)) if c.callee.as_str() == "__image_intrinsic_width"
            )
            && !matches!(state.authored_ports.get("width"), Some(Expr::Binary(_)) if state.authored_ports.contains_key("height") && !matches!(state.authored_ports.get("height"), Some(Expr::Binary(_))))
        {
            return vec![
                "src".to_string(),
                "x".to_string(),
                "y".to_string(),
                "width".to_string(),
            ];
        }
        if state.elem.ports.iter().any(|p| p.name.as_str() == "height") {
            return vec![
                "src".to_string(),
                "x".to_string(),
                "y".to_string(),
                "height".to_string(),
            ];
        }
        if state.elem.ports.iter().any(|p| p.name.as_str() == "width") {
            return vec![
                "src".to_string(),
                "x".to_string(),
                "y".to_string(),
                "width".to_string(),
            ];
        }
        vec![
            "src".to_string(),
            "x".to_string(),
            "y".to_string(),
            "dpi".to_string(),
        ]
    }
}
