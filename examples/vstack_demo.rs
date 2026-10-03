use directedtype::component::ComponentRegistry;
use directedtype::render::{run_viewer_with_file_and_registry, ViewerConfig};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let registry = ComponentRegistry::standard();

    let config = ViewerConfig {
        title: "DirectedType - Standard VStack Flow Component Demo".into(),
        width: 1000,
        height: 600,
        ..Default::default()
    };

    let dt_path = PathBuf::from("examples/vstack_demo.dt");
    println!("Launching DirectedType Standard VStack Demo...");
    println!("  • File: {}", dt_path.display());
    println!("  • Features: 4 cross-axis alignments (left, center, right, stretch), gap spacing, and padding");
    println!("  • Press 'd' to dump DOM tree to stdout, 'i' to toggle live inspector, 'q' or Esc to exit");

    run_viewer_with_file_and_registry(dt_path, config, registry)?;
    Ok(())
}
