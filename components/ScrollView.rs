// DirectedType Standard Component Library: ScrollView Companion
// Native companion struct providing mousewheel/trackpad scrolling and programmatic navigation.

use directedtype::component::{Context, DispatchError};
use directedtype::interaction::{Event, EventKind};
use directedtype_macros::component;

#[derive(Default, Debug, Clone)]
pub struct ScrollView {
    pub scroll_y: f64,
    pub scroll_x: f64,
}

impl ScrollView {
    pub fn new() -> Self {
        Self::default()
    }
}

#[component]
impl ScrollView {
    pub fn on_mount(&mut self, ctx: &mut Context<'_>) {
        if let Some(sy) = ctx.get_state_number("scroll_y") {
            self.scroll_y = sy;
        }
        if let Some(sx) = ctx.get_state_number("scroll_x") {
            self.scroll_x = sx;
        }
    }

    pub fn on_scroll(&mut self, event: &mut Event, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        if let EventKind::Scroll { delta_x, delta_y } = event.kind {
            let container = match ctx.layout().get_node(ctx.node_id()) {
                Some(n) => n,
                None => return Ok(()),
            };
            let viewport_h = container.rect.height;
            let viewport_w = container.rect.width;

            let clip_node = ctx.layout().nodes.iter().find(|n| n.parent == Some(ctx.node_id()) && n.name == "Clip");
            let (max_scroll_y, max_scroll_x) = if let Some(clip) = clip_node {
                let mut max_bottom = container.rect.y;
                let mut max_right = container.rect.x;
                for n in &ctx.layout().nodes {
                    if n.clip == Some(clip.id) {
                        max_bottom = max_bottom.max(n.rect.y + n.rect.height);
                        max_right = max_right.max(n.rect.x + n.rect.width);
                    }
                }
                let padding = ctx.get_port_number("padding").unwrap_or(0.0);
                let content_h = (max_bottom + self.scroll_y + padding - container.rect.y).max(0.0);
                let content_w = (max_right + self.scroll_x + padding - container.rect.x).max(0.0);
                ((content_h - viewport_h).max(0.0), (content_w - viewport_w).max(0.0))
            } else {
                (f64::INFINITY, f64::INFINITY)
            };

            self.scroll_y = (self.scroll_y - delta_y).clamp(0.0, max_scroll_y);
            self.scroll_x = (self.scroll_x - delta_x).clamp(0.0, max_scroll_x);
            ctx.set_state("scroll_y", self.scroll_y);
            ctx.set_state("scroll_x", self.scroll_x);
        }
        Ok(())
    }
}
