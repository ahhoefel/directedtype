// DirectedType Standard Component Library: Button Companion
// Native companion struct providing click tracking, disabled state, and event propagation control.

use directedtype::component::{Component, Context, DispatchError};
use directedtype::interaction::Event;

#[derive(Default, Debug, Clone)]
pub struct Button {
    pub disabled: bool,
    pub click_count: u64,
}

impl Button {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Component for Button {
    fn on_mount(&mut self, ctx: &mut Context<'_>) {
        if let Some(d) = ctx.get_port_bool("disabled") {
            self.disabled = d;
        }
    }

    fn dispatch(
        &mut self,
        method: &str,
        event: &mut Event,
        ctx: &mut Context<'_>,
    ) -> Result<(), DispatchError> {
        match method {
            "click" => {
                let is_disabled = ctx.get_port_bool("disabled").unwrap_or(self.disabled);
                if is_disabled {
                    // Suppress further event bubbling when button is disabled
                    event.stop_propagation();
                } else {
                    self.click_count += 1;
                    ctx.set_state("click_count", self.click_count as f64);
                }
                Ok(())
            }
            _ => Err(DispatchError::MethodNotFound {
                component: "Button".into(),
                method: method.into(),
            }),
        }
    }
}
