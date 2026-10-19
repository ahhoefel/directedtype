// DirectedType Standard Component Library: Slider Companion
// Native companion struct providing drag tracking, pointer capture, and strongly-typed callbacks.

use directedtype::component::{Callback, Context, DispatchError, EmitCallback};
use directedtype::interaction::Event;
use directedtype_macros::component;

#[derive(Default, Debug, Clone)]
pub struct Slider {
    pub min: f64,
    pub max: f64,
    pub value: f64,
    pub is_dragging: bool,
    pub on_change: Option<Callback<f64>>,
}

impl Slider {
    pub fn new() -> Self {
        Self {
            min: 0.0,
            max: 100.0,
            value: 0.0,
            is_dragging: false,
            on_change: None,
        }
    }
}

#[component]
impl Slider {
    pub fn on_mount(&mut self, ctx: &mut Context<'_>) {
        self.on_change = ctx.get_callback("on_change");
        if let Some(min) = ctx.get_port_float("min") {
            self.min = min;
        }
        if let Some(max) = ctx.get_port_float("max") {
            self.max = max;
        }
        if let Some(val) = ctx.get_port_float("value") {
            self.value = val.clamp(self.min, self.max);
            ctx.set_state("current_value", self.value);
        }
    }

    pub fn on_pointer_down(&mut self, event: &mut Event, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        self.is_dragging = true;
        ctx.capture_pointer();
        self.handle_drag(event.local_point.x, ctx);
        Ok(())
    }

    pub fn on_pointer_move(&mut self, event: &mut Event, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        if self.is_dragging {
            self.handle_drag(event.local_point.x, ctx);
        }
        Ok(())
    }

    pub fn on_pointer_up(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        self.is_dragging = false;
        ctx.release_pointer();
        Ok(())
    }

    fn handle_drag(&mut self, local_x: f64, ctx: &mut Context<'_>) {
        let width = ctx.get_port_distance("width").unwrap_or(200.0);
        let ratio = if width > 0.0 {
            (local_x / width).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let new_value = self.min + ratio * (self.max - self.min);

        self.value = new_value;
        ctx.set_state("current_value", new_value);
        self.on_change.emit(ctx, new_value);
    }
}
