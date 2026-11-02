// DirectedType Material UI Component Library: Switch Companion
// Native companion struct providing boolean toggle state, hover/focus/pressed halos,
// disabled gating, controlled/uncontrolled support, and on_change(bool) callback emission.

use directedtype::prelude::*;

#[derive(Default, Debug, Clone)]
pub struct Switch {
    pub checked: bool,
    pub disabled: bool,
    pub controlled: bool,
    pub hovered: bool,
    pub focused: bool,
    pub pressed: bool,
    pub on_change: Option<Callback<bool>>,
}

impl Switch {
    pub fn new() -> Self {
        Self::default()
    }

    fn is_disabled(&self, ctx: &Context<'_>) -> bool {
        ctx.get_port_bool("disabled").unwrap_or(self.disabled)
    }
}

#[component]
impl Switch {
    pub fn on_mount(&mut self, ctx: &mut Context<'_>) {
        self.on_change = ctx.get_callback("on_change");
        if let Some(c) = ctx.get_port_bool("checked") {
            self.checked = c;
        }
        if let Some(d) = ctx.get_port_bool("disabled") {
            self.disabled = d;
        }
        if let Some(ctrl) = ctx.get_port_bool("controlled") {
            self.controlled = ctrl;
        }
    }

    pub fn click(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        if self.is_disabled(ctx) {
            return Ok(());
        }
        let current = ctx
            .get_port_bool("is_checked")
            .or_else(|| ctx.get_port_bool("checked"))
            .unwrap_or(self.checked);
        let next = !current;
        self.checked = next;

        let is_controlled = ctx.get_port_bool("controlled").unwrap_or(self.controlled);
        if !is_controlled {
            ctx.set_state("is_checked", next);
        }
        self.on_change.emit(ctx, next);
        Ok(())
    }

    pub fn focus(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        if self.is_disabled(ctx) {
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
        if self.is_disabled(ctx) {
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
        if self.is_disabled(ctx) {
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

