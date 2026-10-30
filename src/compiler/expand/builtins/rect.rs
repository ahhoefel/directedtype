use crate::ast::{Expr, Ident, MemberAccessExpr};
use crate::compiler::error::CompileError;
use crate::compiler::expand::builtins::{BuiltinPrimitive, PrimitiveExpandState};

pub struct RectPrimitive;

impl BuiltinPrimitive for RectPrimitive {
    fn name(&self) -> &'static str {
        "Rect"
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

