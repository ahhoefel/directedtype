use crate::ast::{
    BinaryExpr, BinaryOp, CallExpr, ComponentDef, DistanceUnit, EnumDef, Expr, Ident, Literal,
    MemberAccessExpr,
};
use crate::compiler::error::CompileError;
use crate::compiler::expand::builtins::{
    default_builtin_port_type, BuiltinChildren, BuiltinPrimitive, PrimitiveExpandState,
};
use crate::compiler::expand::typecheck::StaticType;
use crate::span::Span;
use std::collections::HashMap;

pub struct LinePrimitive;

impl BuiltinPrimitive for LinePrimitive {
    fn name(&self) -> &'static str {
        "Line"
    }

    fn output_aliases(&self) -> &'static [&'static str] {
        &["x", "y", "width", "height", "length"]
    }

    fn required_port_sets(&self) -> &'static [&'static [&'static str]] {
        &[&["x1", "y1", "x2", "y2"]]
    }

    fn children_cardinality(&self) -> BuiltinChildren {
        BuiltinChildren::None
    }

    fn accepted_ports(&self) -> &'static [&'static str] {
        &[
            "x1",
            "y1",
            "x2",
            "y2",
            "z",
            "thickness",
            "color",
            "cap",
            "clip",
            "pagination",
            "cursor",
            "opacity",
            "id",
        ]
    }

    fn accepts_event_handlers(&self) -> bool {
        true
    }

    fn register_types(
        &self,
        enums: &mut HashMap<String, EnumDef>,
        _registry: &mut HashMap<String, Vec<ComponentDef>>,
        span: Span,
    ) {
        enums.entry("LineCap".to_string()).or_insert_with(|| EnumDef {
            name: Ident::new("LineCap", span),
            variants: vec![
                Ident::new("Butt", span),
                Ident::new("Round", span),
                Ident::new("Square", span),
            ],
            span,
        });
    }

    fn port_type(&self, port_name: &str) -> Option<StaticType> {
        match port_name {
            "x1" | "y1" | "x2" | "y2" | "thickness" => Some(StaticType::Distance),
            "cap" => Some(StaticType::Enum("LineCap".to_string())),
            _ => default_builtin_port_type(port_name),
        }
    }

    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        let elem = state.elem;
        let node_id = state.node_id;

        state.ports.entry("thickness".to_string()).or_insert_with(|| {
            Expr::Literal(Literal::Distance(1.0, DistanceUnit::Px, elem.span))
        });

        state.ports.entry("color".to_string()).or_insert_with(|| {
            Expr::Literal(Literal::Color("#000000".to_string(), elem.span))
        });

        state.ports.entry("cap".to_string()).or_insert_with(|| {
            Expr::Literal(Literal::Enum(
                "LineCap".to_string(),
                "Butt".to_string(),
                elem.span,
            ))
        });

        let self_ident = Expr::Ident(Ident::from_node(node_id, elem.span));
        let x1_ref = Expr::MemberAccess(MemberAccessExpr {
            target: std::sync::Arc::new(self_ident.clone()),
            member: Ident::new("x1", elem.span),
            span: elem.span,
        });
        let y1_ref = Expr::MemberAccess(MemberAccessExpr {
            target: std::sync::Arc::new(self_ident.clone()),
            member: Ident::new("y1", elem.span),
            span: elem.span,
        });
        let x2_ref = Expr::MemberAccess(MemberAccessExpr {
            target: std::sync::Arc::new(self_ident.clone()),
            member: Ident::new("x2", elem.span),
            span: elem.span,
        });
        let y2_ref = Expr::MemberAccess(MemberAccessExpr {
            target: std::sync::Arc::new(self_ident),
            member: Ident::new("y2", elem.span),
            span: elem.span,
        });

        state.ports.entry("x".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("min", elem.span),
                args: vec![x1_ref.clone(), x2_ref.clone()],
                span: elem.span,
            })
        });
        state.ports.entry("y".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("min", elem.span),
                args: vec![y1_ref.clone(), y2_ref.clone()],
                span: elem.span,
            })
        });
        state.ports.entry("width".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("abs", elem.span),
                args: vec![Expr::Binary(BinaryExpr {
                    op: BinaryOp::Sub,
                    left: std::sync::Arc::new(x2_ref),
                    right: std::sync::Arc::new(x1_ref),
                    span: elem.span,
                })],
                span: elem.span,
            })
        });
        state.ports.entry("height".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("abs", elem.span),
                args: vec![Expr::Binary(BinaryExpr {
                    op: BinaryOp::Sub,
                    left: std::sync::Arc::new(y2_ref),
                    right: std::sync::Arc::new(y1_ref),
                    span: elem.span,
                })],
                span: elem.span,
            })
        });

        Ok(())
    }

    fn overload_params(&self, _state: &PrimitiveExpandState<'_, '_>) -> Vec<String> {
        vec![
            "x1".to_string(),
            "y1".to_string(),
            "x2".to_string(),
            "y2".to_string(),
        ]
    }
}

