use crate::ast::{DistanceUnit, Expr, Literal};
use crate::compiler::error::CompileError;
use crate::compiler::expand::builtins::{BuiltinPrimitive, PrimitiveExpandState};

pub struct BoxPrimitive;
pub struct LinkPrimitive;

impl BuiltinPrimitive for BoxPrimitive {
    fn name(&self) -> &'static str {
        "Box"
    }

    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        for port in ["height", "width"] {
            if !state.ports.contains_key(port) {
                return Err(CompileError::MissingPort {
                    node: "Box".to_string(),
                    port: port.to_string(),
                    span: state.elem.span,
                });
            }
        }
        Ok(())
    }
}

impl BuiltinPrimitive for LinkPrimitive {
    fn name(&self) -> &'static str {
        "Link"
    }

    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        let span = state.elem.span;
        state.ports.entry("width".to_string()).or_insert_with(|| {
            Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span))
        });
        state.ports.entry("height".to_string()).or_insert_with(|| {
            Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span))
        });
        Ok(())
    }
}

