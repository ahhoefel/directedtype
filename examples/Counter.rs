use directedtype::component::{ComponentRegistry, Context, DispatchError};
use directedtype::render::{run_viewer_with_file_and_registry, ViewerConfig};
use directedtype_macros::component;
use std::path::PathBuf;

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


fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut registry = ComponentRegistry::standard();
    registry.register_companion("Counter", "examples/Counter.rs", || {
        Box::new(Counter::default())
    });

    let config = ViewerConfig {
        title: "DirectedType - Reactive Counter Component".into(),
        width: 800,
        height: 600,
        ..Default::default()
    };

    let dt_path = PathBuf::from("examples/Counter.dt");
    println!("Launching DirectedType Interactive Counter Example...");
    println!("  • File: {}", dt_path.display());
    println!("  • Click '+' to increment, '-' to decrement, and 'Reset' to reset");
    println!("  • Press 'd' to dump DOM tree to stdout, 'q' or Esc to exit");

    run_viewer_with_file_and_registry(dt_path, config, registry)?;
    Ok(())
}
