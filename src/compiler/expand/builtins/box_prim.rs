use crate::ast::{DistanceUnit, Expr, Literal};
use crate::compiler::error::CompileError;
use crate::compiler::expand::builtins::{
    BuiltinChildren, BuiltinDefault, BuiltinPrimitive, PrimitiveExpandState, DEFAULT_OPACITY,
};

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

    fn required_port_sets(&self) -> &'static [&'static [&'static str]] {
        &[&["width", "height"]]
    }

    fn default_port_sets(&self) -> &'static [&'static [(&'static str, BuiltinDefault)]] {
        &[&[
            ("x", BuiltinDefault::Px(0.0)),
            ("y", BuiltinDefault::Px(0.0)),
            ("z", BuiltinDefault::Integer(0)),
            ("radius", BuiltinDefault::Px(0.0)),
            ("pagination", BuiltinDefault::Expr("\\Pagination()")),
            ("cursor", BuiltinDefault::String("Default")),
            ("opacity", BuiltinDefault::Float(DEFAULT_OPACITY)),
            ("name", BuiltinDefault::String("")),
            ("path", BuiltinDefault::String("")),
            ("id", BuiltinDefault::String("")),
        ]]
    }

    fn children_cardinality(&self) -> BuiltinChildren {
        BuiltinChildren::Multiple
    }

    fn finalize_ports(&self, _state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
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

    fn required_port_sets(&self) -> &'static [&'static [&'static str]] {
        &[&[]]
    }

    fn default_port_sets(&self) -> &'static [&'static [(&'static str, BuiltinDefault)]] {
        &[&[
            ("url", BuiltinDefault::String("")),
            ("color", BuiltinDefault::Unset),
            ("underline", BuiltinDefault::Bool(false)),
            ("x", BuiltinDefault::Px(0.0)),
            ("y", BuiltinDefault::Px(0.0)),
            ("width", BuiltinDefault::Px(0.0)),
            ("height", BuiltinDefault::Px(0.0)),
            ("z", BuiltinDefault::Integer(0)),
            ("pagination", BuiltinDefault::Expr("\\Pagination()")),
            ("cursor", BuiltinDefault::String("Default")),
            ("opacity", BuiltinDefault::Float(DEFAULT_OPACITY)),
            ("id", BuiltinDefault::String("")),
        ]]
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

