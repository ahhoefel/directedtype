// DirectedType Standard Component Library: Card Companion
// Native companion struct providing focus/blur handling and reactive surface highlighting.

use directedtype::component::{Context, DispatchError};
use directedtype_macros::component;

#[derive(Default, Debug, Clone)]
pub struct Card {
    pub focused: bool,
}

impl Card {
    pub fn new() -> Self {
        Self::default()
    }
}

#[component]
impl Card {
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
}

