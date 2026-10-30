use crate::ast::{
    ComponentDef, ComponentKind, DistanceUnit, EnumDef, Expr, Ident, Literal, MemberAccessExpr,
    ParamDef, TypeRef,
};
use crate::compiler::error::CompileError;
use crate::compiler::expand::builtins::{
    default_builtin_port_type, BuiltinPrimitive, PrimitiveExpandState,
};
use crate::compiler::expand::typecheck::StaticType;
use crate::span::Span;
use std::collections::HashMap;

pub struct RectPrimitive;

impl BuiltinPrimitive for RectPrimitive {
    fn name(&self) -> &'static str {
        "Rect"
    }

    fn accepts_port(&self, port_name: &str) -> bool {
        matches!(
            port_name,
            "x" | "y"
                | "width"
                | "height"
                | "z"
                | "color"
                | "bg_color"
                | "border_color"
                | "border_width"
                | "radius"
                | "corner_radius"
                | "radius_left"
                | "radius_right"
                | "radius_top"
                | "radius_bottom"
                | "left_radius"
                | "right_radius"
                | "top_radius"
                | "bottom_radius"
                | "radius_top_left"
                | "radius_top_right"
                | "radius_bottom_right"
                | "radius_bottom_left"
                | "top_left_radius"
                | "top_right_radius"
                | "bottom_right_radius"
                | "bottom_left_radius"
                | "shadow"
                | "shadow_offset_x"
                | "shadow_offset_y"
                | "shadow_blur"
                | "shadow_spread"
                | "shadow_color"
                | "scroll_x"
                | "scroll_y"
                | "clip"
                | "cursor"
                | "opacity"
                | "name"
                | "path"
                | "id"
        ) || port_name.starts_with("on_")
    }

    fn register_types(
        &self,
        _enums: &mut HashMap<String, EnumDef>,
        registry: &mut HashMap<String, Vec<ComponentDef>>,
        span: Span,
    ) {
        registry.entry("Shadow".to_string()).or_insert_with(|| {
            vec![ComponentDef {
                kind: ComponentKind::Component,
                name: Ident::new("Shadow", span),
                params: vec![
                    ParamDef {
                        name: Ident::new("offset_x", span),
                        type_annotation: Some(TypeRef {
                            name: Ident::new("Distance", span),
                            span,
                        }),
                        default_edge: Some(Expr::Literal(Literal::Distance(
                            0.0,
                            DistanceUnit::Px,
                            span,
                        ))),
                        is_env: false,
                        span,
                    },
                    ParamDef {
                        name: Ident::new("offset_y", span),
                        type_annotation: Some(TypeRef {
                            name: Ident::new("Distance", span),
                            span,
                        }),
                        default_edge: Some(Expr::Literal(Literal::Distance(
                            0.0,
                            DistanceUnit::Px,
                            span,
                        ))),
                        is_env: false,
                        span,
                    },
                    ParamDef {
                        name: Ident::new("blur", span),
                        type_annotation: Some(TypeRef {
                            name: Ident::new("Distance", span),
                            span,
                        }),
                        default_edge: Some(Expr::Literal(Literal::Distance(
                            0.0,
                            DistanceUnit::Px,
                            span,
                        ))),
                        is_env: false,
                        span,
                    },
                    ParamDef {
                        name: Ident::new("spread", span),
                        type_annotation: Some(TypeRef {
                            name: Ident::new("Distance", span),
                            span,
                        }),
                        default_edge: Some(Expr::Literal(Literal::Distance(
                            0.0,
                            DistanceUnit::Px,
                            span,
                        ))),
                        is_env: false,
                        span,
                    },
                    ParamDef {
                        name: Ident::new("color", span),
                        type_annotation: Some(TypeRef {
                            name: Ident::new("Color", span),
                            span,
                        }),
                        default_edge: Some(Expr::Literal(Literal::Color(
                            "#00000000".to_string(),
                            span,
                        ))),
                        is_env: false,
                        span,
                    },
                ],
                body: Vec::new(),
                span,
            }]
        });
    }

    fn port_type(&self, port_name: &str) -> Option<StaticType> {
        match port_name {
            "radius"
            | "corner_radius"
            | "radius_left"
            | "radius_right"
            | "radius_top"
            | "radius_bottom"
            | "left_radius"
            | "right_radius"
            | "top_radius"
            | "bottom_radius"
            | "radius_top_left"
            | "radius_top_right"
            | "radius_bottom_right"
            | "radius_bottom_left"
            | "top_left_radius"
            | "top_right_radius"
            | "bottom_right_radius"
            | "bottom_left_radius"
            | "border_width"
            | "shadow_offset_x"
            | "shadow_offset_y"
            | "shadow_blur"
            | "shadow_spread"
            | "scroll_x"
            | "scroll_y" => Some(StaticType::Distance),
            "bg_color" | "border_color" | "shadow_color" => Some(StaticType::Color),
            "shadow" => Some(StaticType::Node("Shadow".to_string())),
            _ => default_builtin_port_type(port_name),
        }
    }

    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        let elem = state.elem;

        for port in ["x", "y", "width", "height"] {
            if !state.ports.contains_key(port) {
                return Err(CompileError::MissingPort {
                    node: "Rect".to_string(),
                    port: port.to_string(),
                    span: elem.span,
                });
            }
        }
        if !state.ports.contains_key("color") && !state.ports.contains_key("bg_color") {
            return Err(CompileError::MissingPort {
                node: "Rect".to_string(),
                port: "color".to_string(),
                span: elem.span,
            });
        }

        if let Some(sh_target) = state.ports.get("shadow").cloned() {
            for field in ["offset_x", "offset_y", "blur", "spread", "color"] {
                let prop_name = format!("shadow_{field}");
                if !state.ports.contains_key(&prop_name) {
                    let member_expr = Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(sh_target.clone()),
                        member: Ident::new(field, elem.span),
                        span: elem.span,
                    });
                    state.ports.insert(prop_name.clone(), member_expr.clone());
                    state.authored_ports.insert(prop_name, member_expr);
                }
            }
        }

        Ok(())
    }

    fn overload_params(&self, state: &PrimitiveExpandState<'_, '_>) -> Vec<String> {
        let color_port =
            if state.ports.contains_key("bg_color") && !state.ports.contains_key("color") {
                "bg_color"
            } else {
                "color"
            };
        vec![
            "x".to_string(),
            "y".to_string(),
            "width".to_string(),
            "height".to_string(),
            color_port.to_string(),
        ]
    }
}

