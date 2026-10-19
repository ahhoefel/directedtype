#![allow(non_snake_case)]

use directedtype_guest::prelude::*;

/// Object-Oriented Rust companion struct for the `AudioMixer` DTML component.
///
/// Encapsulates audio level state (`master`, `bass`, `treble`) and reactive setter callbacks,
/// communicating strictly via DirectedType's One-Way Pipeline (reading ports & emitting mutations).
#[derive(Default, Debug)]
pub struct AudioMixer {
    pub master: f64,
    pub bass: f64,
    pub treble: f64,
}

#[component]
impl AudioMixer {
    pub fn on_mount(&mut self, ctx: &mut Context<'_>) {
        if let Some(m) = ctx.get_state_number("master") {
            self.master = m;
        }
        if let Some(b) = ctx.get_state_number("bass") {
            self.bass = b;
        }
        if let Some(t) = ctx.get_state_number("treble") {
            self.treble = t;
        }
    }

    pub fn set_master(&mut self, val: f64, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        let rounded = val.round();
        self.master = rounded;
        ctx.set_state("master", rounded);
        Ok(())
    }

    pub fn set_bass(&mut self, val: f64, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        let rounded = val.round();
        self.bass = rounded;
        ctx.set_state("bass", rounded);
        Ok(())
    }

    pub fn set_treble(&mut self, val: f64, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        let rounded = val.round();
        self.treble = rounded;
        ctx.set_state("treble", rounded);
        Ok(())
    }
}

export_component!(AudioMixer);
