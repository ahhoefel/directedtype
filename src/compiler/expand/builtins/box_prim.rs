use crate::ast::{DistanceUnit, Expr, Literal};
use crate::compiler::error::CompileError;
use crate::compiler::expand::builtins::{BuiltinChildren, BuiltinPrimitive, PrimitiveExpandState};

pub struct BoxPrimitive;
pub struct LinkPrimitive;

impl BuiltinPrimitive for BoxPrimitive {
    fn name(&self) -> &'static str {
        "Box"
    }

    fn accepted_ports(&self) -> &'static [&'static str] {
        &[
            "x",
            "y",
            "width",
            "height",
            "z",
            "radius",
            "clip",
            "pagination",
            "cursor",
            "opacity",
            "name",
            "path",
            "id",
        ]
    }

    fn accepts_event_handlers(&self) -> bool {
        true
    }

    fn children_cardinality(&self) -> BuiltinChildren {
        BuiltinChildren::Multiple
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

    fn accepted_ports(&self) -> &'static [&'static str] {
        &[
            "url",
            "color",
            "underline",
            "x",
            "y",
            "width",
            "height",
            "z",
            "font",
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

    fn children_cardinality(&self) -> BuiltinChildren {
        BuiltinChildren::Multiple
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

