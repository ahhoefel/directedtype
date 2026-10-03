use directedtype::component::ComponentRegistry;
use directedtype::render::{run_viewer_with_file_and_registry, ViewerConfig};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let registry = ComponentRegistry::standard();

    let config = ViewerConfig {
        title: "DirectedType - Standard Button Component Demo".into(),
        width: 800,
        height: 600,
        ..Default::default()
    };

    let dt_path = PathBuf::from("examples/button_demo.dt");
    println!("Launching DirectedType Standard Button Demo...");
    println!("  • File: {}", dt_path.display());
    println!("  • Features: 6 visual variants, intrinsic typography auto-sizing, and event suppression");
    println!("  • Press 'd' to dump DOM tree to stdout, 'i' to toggle live inspector, 'q' or Esc to exit");

    run_viewer_with_file_and_registry(dt_path, config, registry)?;
    Ok(())
}
