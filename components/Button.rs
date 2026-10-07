// DirectedType Standard Component Library: Button Companion
// Native companion struct providing click tracking, disabled state, and event propagation control.

use directedtype::component::{Context, DispatchError};
use directedtype::interaction::Event;
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

    pub fn click(&mut self, event: &mut Event, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        let is_disabled = ctx.get_port_bool("disabled").unwrap_or(self.disabled);
        if is_disabled {
            // Suppress further event bubbling when button is disabled
            event.stop_propagation();
        }
        Ok(())
    }
}

