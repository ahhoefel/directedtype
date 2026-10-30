use crate::compiler::error::CompileError;
use crate::compiler::expand::builtins::{BuiltinPrimitive, PrimitiveExpandState};

pub struct BoxPrimitive;
pub struct LinkPrimitive;

impl BuiltinPrimitive for BoxPrimitive {
    fn name(&self) -> &'static str {
        "Box"
    }

    fn finalize_ports(&self, _state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        Ok(())
    }
}

impl BuiltinPrimitive for LinkPrimitive {
    fn name(&self) -> &'static str {
        "Link"
    }

    fn finalize_ports(&self, _state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        Ok(())
    }
}

