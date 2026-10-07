// DirectedType Standard Component Library: Button Companion
// Native companion struct providing click tracking, disabled state, and event propagation control.

use directedtype::component::{Context, DispatchError};
use directedtype_macros::component;

#[derive(Default, Debug, Clone)]
pub struct Button {
    pub disabled: bool,
}

impl Button {
    pub fn new() -> Self {
        Self::default()
    }
}

#[component]
impl Button {
    pub fn on_mount(&mut self, ctx: &mut Context<'_>) {
        if let Some(d) = ctx.get_port_bool("disabled") {
            self.disabled = d;
        }
    }

    pub fn click(&mut self) -> Result<(), DispatchError> {
        Ok(())
    }
}

