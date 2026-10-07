// DirectedType Standard Component Library: Link Companion
// Native companion struct providing click navigation, internal anchor scrolling, external URL routing,
// and reactive on_focus / on_blur visual styling.

use directedtype::component::{Component, Context, DispatchError};
use directedtype::interaction::Event;

#[derive(Default, Debug, Clone)]
pub struct Link {
    pub focused: bool,
}

impl Link {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Component for Link {
    fn dispatch(
        &mut self,
        method: &str,
        event: &mut Event,
        ctx: &mut Context<'_>,
    ) -> Result<(), DispatchError> {
        match method {
            "click" => {
                if event.propagation_stopped {
                    return Ok(());
                }
                event.stop_propagation();

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
            "focus" => {
                self.focused = true;
                ctx.set_state("focused", true);
                event.stop_propagation();
                Ok(())
            }
            "blur" => {
                self.focused = false;
                ctx.set_state("focused", false);
                event.stop_propagation();
                Ok(())
            }
            _ => Err(DispatchError::MethodNotFound {
                component: "Link".into(),
                method: method.into(),
            }),
        }
    }
}
