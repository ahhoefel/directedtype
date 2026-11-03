// DirectedType Material UI Component Library: Slider Companion
// Native companion struct for \Slider providing pointer capture & drag tracking,
// continuous and discrete step/mark snapping, hover/focus/pressed thumb halo states,
// disabled gating, controlled/uncontrolled mode, and on_change(f64) callback emission.

use directedtype::prelude::*;

#[derive(Debug, Clone)]
pub struct Slider {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub value: f64,
    pub marks: bool,
    pub disabled: bool,
    pub controlled: bool,
    pub hovered: bool,
    pub focused: bool,
    pub pressed: bool,
    pub is_dragging: bool,
    pub on_change: Option<Callback<f64>>,
}

impl Default for Slider {
    fn default() -> Self {
        Self::new()
    }
}

impl Slider {
    pub fn new() -> Self {
        Self {
            min: 0.0,
            max: 100.0,
            step: 0.0,
            value: 0.0,
            marks: false,
            disabled: false,
            controlled: false,
            hovered: false,
            focused: false,
            pressed: false,
            is_dragging: false,
            on_change: None,
        }
    }

    fn is_disabled(&self, ctx: &Context<'_>) -> bool {
        ctx.get_port_bool("disabled").unwrap_or(self.disabled)
    }

    fn resolve_local_x(&self, event: &Event, ctx: &Context<'_>) -> f64 {
        if event.global_point.x != 0.0 || event.global_point.y != 0.0 {
            let slider_x = ctx
                .layout()
                .get_node(ctx.node_id())
                .map(|n| n.rect.x)
                .or_else(|| ctx.get_port_distance("x"))
                .unwrap_or(0.0);
            event.global_point.x - slider_x
        } else {
            event.local_point.x
        }
    }

    fn snap_value(&self, raw_value: f64, min: f64, max: f64, step: f64, marks: bool, ctx: &Context<'_>) -> f64 {
        if step > 0.0 {
            let steps = ((raw_value - min) / step).round();
            (min + steps * step).clamp(min, max)
        } else if marks {
            let slider_id = ctx.node_id();
            let mark_values: Vec<f64> = ctx
                .layout()
                .nodes
                .iter()
                .filter(|n| n.parent == Some(slider_id) && n.name == "SliderMark")
                .filter_map(|n| n.properties.get("value").and_then(|v| v.as_f64()))
                .collect();
            if !mark_values.is_empty() {
                mark_values
                    .into_iter()
                    .min_by(|a, b| {
                        (a - raw_value)
                            .abs()
                            .partial_cmp(&(b - raw_value).abs())
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .unwrap_or(raw_value)
                    .clamp(min, max)
            } else {
                raw_value.clamp(min, max)
            }
        } else {
            raw_value.clamp(min, max)
        }
    }

    fn update_value_from_event(&mut self, event: &Event, ctx: &mut Context<'_>) {
        let min = ctx.get_port_float("min").unwrap_or(self.min);
        let max = ctx.get_port_float("max").unwrap_or(self.max);
        let step = ctx.get_port_float("step").unwrap_or(self.step);
        let marks = ctx.get_port_bool("marks").unwrap_or(self.marks);
        let width = ctx
            .layout()
            .get_node(ctx.node_id())
            .map(|n| n.rect.width)
            .or_else(|| ctx.get_port_distance("width"))
            .unwrap_or(200.0);

        let local_x = self.resolve_local_x(event, ctx);
        let ratio = if width > 0.0 {
            (local_x / width).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let raw_value = min + ratio * (max - min);
        let new_value = self.snap_value(raw_value, min, max, step, marks, ctx);

        self.min = min;
        self.max = max;
        self.step = step;
        self.marks = marks;
        self.value = new_value;

        let is_controlled = ctx.get_port_bool("controlled").unwrap_or(self.controlled);
        if !is_controlled {
            ctx.set_state("current_value", new_value);
        }
        self.on_change.emit(ctx, new_value);
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
        if let Some(step) = ctx.get_port_float("step") {
            self.step = step;
        }
        if let Some(marks) = ctx.get_port_bool("marks") {
            self.marks = marks;
        }
        if let Some(d) = ctx.get_port_bool("disabled") {
            self.disabled = d;
        }
        if let Some(ctrl) = ctx.get_port_bool("controlled") {
            self.controlled = ctrl;
        }
        if let Some(val) = ctx.get_port_float("value") {
            let initial = self.snap_value(val, self.min, self.max, self.step, self.marks, ctx);
            self.value = initial;
            if !self.controlled {
                ctx.set_state("current_value", initial);
            }
        }
    }

    pub fn click(&mut self, _ctx: &mut Context<'_>) -> Result<(), DispatchError> {
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
        self.is_dragging = false;
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
        ctx.set_state("hovered", false);
        if !self.is_dragging {
            self.pressed = false;
            ctx.set_state("pressed", false);
        }
        Ok(())
    }

    pub fn pointer_down(&mut self, event: &mut Event, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        if self.is_disabled(ctx) {
            return Ok(());
        }
        self.pressed = true;
        self.is_dragging = true;
        ctx.set_state("pressed", true);
        ctx.capture_pointer();
        self.update_value_from_event(event, ctx);
        Ok(())
    }

    pub fn pointer_move(&mut self, event: &mut Event, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        if self.is_disabled(ctx) || !self.is_dragging {
            return Ok(());
        }
        self.update_value_from_event(event, ctx);
        Ok(())
    }

    pub fn pointer_up(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        self.pressed = false;
        self.is_dragging = false;
        ctx.set_state("pressed", false);
        ctx.release_pointer();
        Ok(())
    }
}

