// DirectedType Material UI Component Library: Button Companion
// Native companion struct providing hover, focus, pressed states, disabled suppression, and on_click callback dispatch.

use directedtype::component::{Callback, Context, DispatchError, EmitCallback};
use directedtype_macros::component;

#[derive(Default, Debug, Clone)]
pub struct Button {
    pub disabled: bool,
    pub hovered: bool,
    pub focused: bool,
    pub pressed: bool,
    pub on_click: Option<Callback<()>>,
}

impl Button {
    pub fn new() -> Self {
        Self::default()
    }
}

#[component]
impl Button {
    pub fn on_mount(&mut self, ctx: &mut Context<'_>) {
        self.on_click = ctx.get_callback("on_click");
        if let Some(d) = ctx.get_port_bool("disabled") {
            self.disabled = d;
        }
    }

    pub fn click(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        let is_disabled = ctx.get_port_bool("disabled").unwrap_or(self.disabled);
        if is_disabled {
            return Ok(());
        }
        self.on_click.emit(ctx, ());
        Ok(())
    }

    pub fn focus(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        let is_disabled = ctx.get_port_bool("disabled").unwrap_or(self.disabled);
        if is_disabled {
            return Ok(());
        }
        self.focused = true;
        ctx.set_state("focused", true);
        Ok(())
    }

    pub fn blur(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        self.focused = false;
        self.pressed = false;
        ctx.set_state("focused", false);
        ctx.set_state("pressed", false);
        Ok(())
    }

    pub fn pointer_enter(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        let is_disabled = ctx.get_port_bool("disabled").unwrap_or(self.disabled);
        if is_disabled {
            return Ok(());
        }
        self.hovered = true;
        ctx.set_state("hovered", true);
        Ok(())
    }

    pub fn pointer_leave(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        self.hovered = false;
        self.pressed = false;
        ctx.set_state("hovered", false);
        ctx.set_state("pressed", false);
        Ok(())
    }

    pub fn pointer_down(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        let is_disabled = ctx.get_port_bool("disabled").unwrap_or(self.disabled);
        if is_disabled {
            return Ok(());
        }
        self.pressed = true;
        ctx.set_state("pressed", true);
        Ok(())
    }

    pub fn pointer_up(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        self.pressed = false;
        ctx.set_state("pressed", false);
        Ok(())
    }
}

