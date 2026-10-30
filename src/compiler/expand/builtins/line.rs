use crate::ast::{
    BinaryExpr, BinaryOp, CallExpr, DistanceUnit, Expr, Ident, Literal, MemberAccessExpr,
};
use crate::compiler::error::CompileError;
use crate::compiler::expand::builtins::{BuiltinPrimitive, PrimitiveExpandState};

pub struct LinePrimitive;

impl BuiltinPrimitive for LinePrimitive {
    fn name(&self) -> &'static str {
        "Line"
    }

    fn ignores_ambient_box_ports(&self) -> bool {
        true
    }

    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        let elem = state.elem;
        let node_id = state.node_id;

        for (alias, canonical) in [
            ("start_x", "x1"),
            ("start_y", "y1"),
            ("end_x", "x2"),
            ("end_y", "y2"),
        ] {
            if !state.ports.contains_key(canonical) {
                if let Some(val) = state.ports.get(alias).cloned() {
                    state.ports.insert(canonical.to_string(), val);
                }
                if let Some(authored) = state.authored_ports.get(alias).cloned() {
                    state.authored_ports.insert(canonical.to_string(), authored);
                }
            }
        }

        for port in ["x1", "y1", "x2", "y2"] {
            if !state.ports.contains_key(port) {
                return Err(CompileError::MissingPort {
                    node: "Line".to_string(),
                    port: port.to_string(),
                    span: elem.span,
                });
            }
        }

        if !state.ports.contains_key("thickness") {
            if let Some(sw) = state
                .ports
                .get("stroke_width")
                .or_else(|| state.ports.get("border_width"))
                .cloned()
            {
                state.ports.insert("thickness".to_string(), sw);
            } else {
                state.ports.insert(
                    "thickness".to_string(),
                    Expr::Literal(Literal::Distance(1.0, DistanceUnit::Px, elem.span)),
                );
            }
        }

        if !state.ports.contains_key("color") {
            if let Some(sc) = state
                .ports
                .get("stroke_color")
                .or_else(|| state.ports.get("border_color"))
                .cloned()
            {
                state.ports.insert("color".to_string(), sc);
            } else {
                state.ports.insert(
                    "color".to_string(),
                    Expr::Literal(Literal::Color("#000000".to_string(), elem.span)),
                );
            }
        }

        if !state.ports.contains_key("cap") {
            if let Some(lc) = state.ports.get("line_cap").cloned() {
                state.ports.insert("cap".to_string(), lc);
            } else {
                state.ports.insert(
                    "cap".to_string(),
                    Expr::Literal(Literal::Enum(
                        "LineCap".to_string(),
                        "Butt".to_string(),
                        elem.span,
                    )),
                );
            }
        }

        let self_ident = Expr::Ident(Ident::new(node_id.canonical_name(), elem.span));
        let x1_ref = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("x1", elem.span),
            span: elem.span,
        });
        let y1_ref = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("y1", elem.span),
            span: elem.span,
        });
        let x2_ref = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("x2", elem.span),
            span: elem.span,
        });
        let y2_ref = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident),
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
                    left: Box::new(x2_ref),
                    right: Box::new(x1_ref),
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
                    left: Box::new(y2_ref),
                    right: Box::new(y1_ref),
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

