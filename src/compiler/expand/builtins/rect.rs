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
                | "border_color"
                | "border_width"
                | "radius"
                | "radius_left"
                | "radius_right"
                | "radius_top"
                | "radius_bottom"
                | "radius_top_left"
                | "radius_top_right"
                | "radius_bottom_right"
                | "radius_bottom_left"
                | "shadow"
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
            | "shadow_spread"
            | "scroll_x"
            | "scroll_y" => Some(StaticType::Distance),
            "border_color" | "shadow_color" => Some(StaticType::Color),
            "shadow" => Some(StaticType::Node("Shadow".to_string())),
            _ => default_builtin_port_type(port_name),
        }
    }

    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        let elem = state.elem;

        for port in ["x", "y", "width", "height", "color"] {
            if !state.ports.contains_key(port) {
                return Err(CompileError::MissingPort {
                    node: "Rect".to_string(),
                    port: port.to_string(),
                    span: elem.span,
                });
            }
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

    fn overload_params(&self, _state: &PrimitiveExpandState<'_, '_>) -> Vec<String> {
        vec![
            "x".to_string(),
            "y".to_string(),
            "width".to_string(),
            "height".to_string(),
            "color".to_string(),
        ]
    }
}

