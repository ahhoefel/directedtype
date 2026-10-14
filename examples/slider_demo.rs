use directedtype::component::{ComponentRegistry, Context};
use directedtype::render::{run_viewer_with_file_and_registry, ViewerConfig};
use directedtype_macros::component;
use std::path::PathBuf;

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

    pub fn set_master(&mut self, val: f64, ctx: &mut Context<'_>) {
        let rounded = val.round();
        self.master = rounded;
        ctx.set_state("master", rounded);
    }

    pub fn set_bass(&mut self, val: f64, ctx: &mut Context<'_>) {
        let rounded = val.round();
        self.bass = rounded;
        ctx.set_state("bass", rounded);
    }

    pub fn set_treble(&mut self, val: f64, ctx: &mut Context<'_>) {
        let rounded = val.round();
        self.treble = rounded;
        ctx.set_state("treble", rounded);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut registry = ComponentRegistry::standard();
    registry.register("AudioMixer", || Box::new(AudioMixer::default()));

    let config = ViewerConfig {
        title: "DirectedType - Standard Slider Component & Callbacks Demo".into(),
        width: 800,
        height: 600,
        ..Default::default()
    };

    let dt_path = PathBuf::from("examples/slider_demo.dt");
    println!("Launching DirectedType Standard Slider Demo...");
    println!("  • File: {}", dt_path.display());
    println!("  • Features: hardware pointer capture, smooth dragging, and typed callbacks");
    println!("  • Press 'd' to dump DOM tree, 'i' to toggle live inspector, 'q' or Esc to exit");

    run_viewer_with_file_and_registry(dt_path, config, registry)?;
    Ok(())
}
