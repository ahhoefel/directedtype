// DirectedType Standard Component Library: Card Companion
// Native companion struct providing focus/blur handling and reactive surface highlighting.

use directedtype::component::{Component, Context, DispatchError};
use directedtype::interaction::Event;

#[derive(Default, Debug, Clone)]
pub struct Card {
    pub focused: bool,
}

impl Card {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Component for Card {
    fn dispatch(
        &mut self,
        method: &str,
        _event: &mut Event,
        ctx: &mut Context<'_>,
    ) -> Result<(), DispatchError> {
        match method {
            "focus" => {
                self.focused = true;
                ctx.set_state("focused", true);
                Ok(())
            }
            "blur" => {
                self.focused = false;
                ctx.set_state("focused", false);
                Ok(())
            }
            _ => Err(DispatchError::MethodNotFound {
                component: "Card".into(),
                method: method.into(),
            }),
        }
    }
}
