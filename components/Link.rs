// DirectedType Standard Component Library: Link Companion
// Native companion struct providing click navigation, internal anchor scrolling, external URL routing,
// and reactive hover/focus visual styling.

use directedtype::component::{Context, DispatchError};
use directedtype::interaction::Event;
use directedtype_macros::component;

#[derive(Default, Debug, Clone)]
pub struct Link {
    pub focused: bool,
    pub hovered: bool,
}

impl Link {
    pub fn new() -> Self {
        Self::default()
    }
}

#[component]
impl Link {
    pub fn click(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        if let Some(url) = ctx.get_port_string("url") {
            let url = url.to_string();
            let pane_container = ctx.get_port("pane").and_then(|v| v.as_node());

            if url.starts_with('#') {
                if !ctx.scroll_to_anchor_in_container(&url, pane_container) {
                    eprintln!("[Link] In-page anchor not found: {}", url);
                }
            } else if ctx.scroll_to_anchor_in_container(&url, pane_container) {
                // Scrolled to relative/scoped anchor path without '#'
            } else {
                ctx.open_url(url);
            }
        }
        Ok(())
    }

    pub fn focus(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        self.focused = true;
        ctx.set_state("focused", true);
        Ok(())
    }

    pub fn blur(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        self.focused = false;
        ctx.set_state("focused", false);
        Ok(())
    }

    pub fn pointer_enter(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        self.hovered = true;
        ctx.set_state("hovered", true);
        Ok(())
    }

    pub fn pointer_leave(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        self.hovered = false;
        ctx.set_state("hovered", false);
        Ok(())
    }
}

