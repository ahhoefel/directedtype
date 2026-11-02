// DirectedType Material UI Component Library: Tabs & Tab Companion
// Native companion structs for \Tabs and \Tab providing tab selection state,
// active indicator synchronization, hover/focus/pressed interactive states,
// disabled tab gating, controlled/uncontrolled mode, and on_change(index) callback dispatch.

use directedtype::prelude::*;

#[derive(Default, Debug, Clone)]
pub struct Tabs {
    pub selected_index: i64,
    pub controlled: bool,
    pub on_change: Option<Callback<i64>>,
}

impl Tabs {
    pub fn new() -> Self {
        Self::default()
    }
}

#[component]
impl Tabs {
    pub fn on_mount(&mut self, ctx: &mut Context<'_>) {
        self.on_change = ctx.get_callback("on_change");
        if let Some(v) = ctx.get_port_integer("value") {
            self.selected_index = v;
        }
        if let Some(ctrl) = ctx.get_port_bool("controlled") {
            self.controlled = ctrl;
        }
    }

    pub fn click(&mut self, event: &mut Event, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        let tabs_id = ctx.node_id();
        let layout = ctx.layout();

        // 1. Locate the clicked direct child \Tab node from the event's bubble path or target chain
        let mut clicked_tab_id = event.bubble_path.iter().copied().find(|&nid| {
            layout
                .get_node(nid)
                .map(|n| n.parent == Some(tabs_id) && n.name == "Tab")
                .unwrap_or(false)
        });

        if clicked_tab_id.is_none() {
            let mut curr = Some(event.target);
            while let Some(nid) = curr {
                if let Some(node) = layout.get_node(nid) {
                    if node.parent == Some(tabs_id) && node.name == "Tab" {
                        clicked_tab_id = Some(nid);
                        break;
                    }
                    curr = node.parent;
                } else {
                    break;
                }
            }
        }

        // 2. Fallback: spatial hit test against direct child \Tab rectangles
        if clicked_tab_id.is_none() {
            clicked_tab_id = layout
                .nodes
                .iter()
                .find(|n| {
                    n.parent == Some(tabs_id)
                        && n.name == "Tab"
                        && n.rect.contains_point(event.global_point)
                })
                .map(|n| n.id);
        }

        let Some(tab_id) = clicked_tab_id else {
            return Ok(());
        };
        let Some(tab_node) = layout.get_node(tab_id) else {
            return Ok(());
        };

        // 3. Ignore clicks on disabled tabs
        let is_disabled = tab_node
            .properties
            .get("disabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if is_disabled {
            return Ok(());
        }

        // 4. Resolve the tab's 0-based index (from `tab_index` property or child order)
        let clicked_index = tab_node
            .properties
            .get("tab_index")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| {
                layout
                    .nodes
                    .iter()
                    .filter(|n| n.parent == Some(tabs_id) && n.name == "Tab")
                    .position(|n| n.id == tab_id)
                    .unwrap_or(0) as i64
            });

        let is_controlled = ctx.get_port_bool("controlled").unwrap_or(self.controlled);
        self.selected_index = clicked_index;
        if !is_controlled {
            ctx.set_state("selected_index", clicked_index);
        }
        self.on_change.emit(ctx, clicked_index);

        Ok(())
    }
}

#[derive(Default, Debug, Clone)]
pub struct Tab {
    pub selected: bool,
    pub disabled: bool,
    pub controlled: bool,
    pub hovered: bool,
    pub focused: bool,
    pub pressed: bool,
    pub on_change: Option<Callback<i64>>,
}

impl Tab {
    pub fn new() -> Self {
        Self::default()
    }

    fn is_disabled(&self, ctx: &Context<'_>) -> bool {
        ctx.get_port_bool("disabled").unwrap_or(self.disabled)
    }
}

#[component]
impl Tab {
    pub fn on_mount(&mut self, ctx: &mut Context<'_>) {
        self.on_change = ctx.get_callback("on_change");
        if let Some(s) = ctx.get_port_bool("selected") {
            self.selected = s;
        }
        if let Some(d) = ctx.get_port_bool("disabled") {
            self.disabled = d;
        }
        if let Some(ctrl) = ctx.get_port_bool("controlled") {
            self.controlled = ctrl;
        }
    }

    pub fn click(&mut self, event: &mut Event, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        if self.is_disabled(ctx) {
            return Ok(());
        }

        let has_tabs_context = ctx.get_port_bool("tabs_has_context").unwrap_or(false);
        let is_controlled = ctx.get_port_bool("controlled").unwrap_or(self.controlled);
        let tab_index = ctx.get_port_integer("tab_index").unwrap_or(0);

        self.selected = true;
        if !has_tabs_context && !is_controlled {
            ctx.set_state("self_selected", true);
        }

        self.on_change.emit(ctx, tab_index);

        // Allow click event to bubble up to parent \Tabs container
        event.continue_propagation();
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

