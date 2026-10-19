#![allow(non_snake_case)]

use directedtype_guest::prelude::*;

/// Object-Oriented Rust companion struct for the `Counter` DTML component.
///
/// Encapsulates private mutable state (`count`) and event handlers (`increment`, `decrement`, `reset`),
/// communicating strictly via DirectedType's One-Way Pipeline (reading ports & emitting mutations).
#[derive(Default, Debug)]
pub struct Counter {
    pub count: i32,
}

#[component]
impl Counter {
    pub fn on_mount(&mut self, ctx: &mut Context<'_>) {
        if let Some(initial) = ctx.get_port_number("initial") {
            self.count = initial as i32;
        }
    }

    pub fn increment(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        if self.count < 10 {
            self.count += 1;
            ctx.set_state("count", self.count as f64);
        }
        Ok(())
    }

    pub fn decrement(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        if self.count > 0 {
            self.count -= 1;
            ctx.set_state("count", self.count as f64);
        }
        Ok(())
    }

    pub fn reset(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        self.count = 0;
        ctx.set_state("count", 0.0);
        Ok(())
    }
}

export_component!(Counter);
