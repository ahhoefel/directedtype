use std::env;
use std::fs;
use std::path::PathBuf;

use directedtype::compiler::evaluate_document_with_window;
use directedtype::parser::parse_document;
use directedtype::render::{
    run_viewer_with_file, HeadlessRenderer, SceneOptions, ViewerConfig,
};
use vello::peniko::Color;

fn print_usage() {
    println!("DirectedType Layout Engine & Vello Viewer");
    println!("Usage:");
    println!("  viewer [FILE] [OPTIONS]");
    println!();
    println!("Options:");
    println!("  -o, --output <PATH>   Render headless directly to PNG file");
    println!("  -w, --width <INT>     Viewport width in pixels (default: 800)");
    println!("  -h, --height <INT>    Viewport height in pixels (default: 600)");
    println!("      --help            Display this help message");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();

    let mut file_path: Option<PathBuf> = None;
    let mut output_path: Option<PathBuf> = None;
    let mut width: u32 = 800;
    let mut height: u32 = 600;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" => {
                print_usage();
                return Ok(());
            }
            "-o" | "--output" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: Missing value for --output");
                    std::process::exit(1);
                }
                output_path = Some(PathBuf::from(&args[i]));
            }
            "-w" | "--width" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: Missing value for --width");
                    std::process::exit(1);
                }
                width = args[i].parse().unwrap_or(800);
            }
            "-h" | "--height" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: Missing value for --height");
                    std::process::exit(1);
                }
                height = args[i].parse().unwrap_or(600);
            }
            arg if !arg.starts_with('-') && file_path.is_none() => {
                file_path = Some(PathBuf::from(arg));
            }
            unknown => {
                eprintln!("Warning: Unrecognized argument '{}'", unknown);
            }
        }
        i += 1;
    }

    let file_path = file_path.unwrap_or_else(|| PathBuf::from("examples/demo.dt"));
    println!("Loading DirectedType document: {}", file_path.display());

    let source = fs::read_to_string(&file_path)
        .map_err(|e| format!("Failed to read file '{}': {e}", file_path.display()))?;

    println!("Parsing document...");
    let doc = parse_document(&source)
        .map_err(|e| format!("Parse error in '{}': {e}", file_path.display()))?;

    println!("Evaluating DAG layout ({}x{})...", width, height);
    let layout = evaluate_document_with_window(&doc, width as f64, height as f64)
        .map_err(|e| format!("Layout evaluation error in '{}': {e}", file_path.display()))?;

    println!(
        "Successfully resolved {} layout nodes across topological schedule.",
        layout.nodes.len()
    );

    let scene_options = SceneOptions {
        background: Some(Color::from_rgba8(248, 250, 252, 255)),
        ..Default::default()
    };

    if let Some(out_path) = output_path {
        println!(
            "Rendering headless to {} ({}x{})...",
            out_path.display(),
            width,
            height
        );
        let mut renderer = HeadlessRenderer::new()?;
        renderer.render_layout_to_file(&layout, width, height, &scene_options, &out_path)?;
        println!("Headless render complete: {}", out_path.display());
    } else {
        println!("Launching interactive native window viewer ({}x{})...", width, height);
        let config = ViewerConfig {
            title: format!("DirectedType - {}", file_path.file_name().unwrap_or_default().to_string_lossy()),
            width,
            height,
            scene_options,
        };
        run_viewer_with_file(file_path, config)?;
    }

    Ok(())
}
