use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::time::Duration;

use directedtype::docgen::{compile_markdown_file, default_output_md_path};
use notify::{EventKind, RecursiveMode, Watcher};

fn print_usage() {
    println!("DirectedType Documentation & Tutorial Compiler (docgen)");
    println!();
    println!("Compiles .dt.md Markdown templates containing ```dt render code blocks");
    println!("into standard .md files and rendered PNG images (in images/<doc>/).");
    println!();
    println!("Usage:");
    println!("  cargo docgen [FILES_OR_DIRS...] [OPTIONS]");
    println!();
    println!("Arguments:");
    println!("  [FILES_OR_DIRS...]    One or more .dt.md files or directories to scan");
    println!("                        (defaults to scanning 'docs/' if omitted)");
    println!();
    println!("Options:");
    println!("  -o, --output <PATH>   Explicit output .md path (when compiling a single file)");
    println!("  -w, --watch           Watch input .dt.md file(s) and recompile automatically on save");
    println!("  -h, --help            Display this help message");
    println!();
    println!("Code Fence Syntax in .dt.md:");
    println!("  ```dt render [width=N] [height=N] [padding=N] [bg=#rrggbb|transparent]");
    println!("               [side-by-side|hide-code] [name=slug] [alt=\"...\"]");
    println!("  ```dt compiles                  // Verifies the block compiles without rendering an image");
    println!("  ```dt compile-error             // Verifies the block fails to compile and displays the error");
    println!("  # \\use \"components/VStack.dt\"   // '# ' lines are compiled but hidden from output");
    println!("  \\Rect(x: 20px, y: 20px, width: 120px, height: 60px, color: #3b82f6)");
    println!("  ```");
}

fn discover_dt_md_files(path: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if path.is_file() {
        out.push(path.to_path_buf());
        return Ok(());
    }
    if !path.exists() {
        return Err(format!("Path does not exist: '{}'", path.display()));
    }
    let entries = fs::read_dir(path)
        .map_err(|e| format!("Failed to read directory '{}': {e}", path.display()))?;
    let mut subpaths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    subpaths.sort();

    for p in subpaths {
        if p.is_dir() {
            discover_dt_md_files(&p, out)?;
        } else if p
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.ends_with(".dt.md"))
            .unwrap_or(false)
        {
            out.push(p);
        }
    }
    Ok(())
}

fn compile_targets(targets: &[PathBuf], explicit_output: Option<&Path>) -> bool {
    let mut all_ok = true;
    for input in targets {
        let out_path = explicit_output
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| default_output_md_path(input));
        match compile_markdown_file(input, explicit_output) {
            Ok(res) => {
                println!(
                    "[docgen] Compiled {} -> {} ({} image{})",
                    input.display(),
                    out_path.display(),
                    res.rendered_images.len(),
                    if res.rendered_images.len() == 1 { "" } else { "s" }
                );
            }
            Err(e) => {
                eprintln!("[docgen] Error: {e}");
                all_ok = false;
            }
        }
    }
    all_ok
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut inputs: Vec<PathBuf> = Vec::new();
    let mut output_path: Option<PathBuf> = None;
    let mut watch = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_usage();
                return;
            }
            "-w" | "--watch" => {
                watch = true;
            }
            "-o" | "--output" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("[docgen] Error: Missing value for --output");
                    std::process::exit(1);
                }
                output_path = Some(PathBuf::from(&args[i]));
            }
            arg if arg.starts_with('-') => {
                eprintln!("[docgen] Error: Unrecognized option '{arg}'");
                print_usage();
                std::process::exit(1);
            }
            arg => {
                inputs.push(PathBuf::from(arg));
            }
        }
        i += 1;
    }

    if inputs.is_empty() {
        let default_docs = PathBuf::from("docs");
        if default_docs.exists() {
            inputs.push(default_docs);
        } else {
            inputs.push(PathBuf::from("."));
        }
    }

    let mut targets = Vec::new();
    for input in &inputs {
        if let Err(e) = discover_dt_md_files(input, &mut targets) {
            eprintln!("[docgen] Error: {e}");
            std::process::exit(1);
        }
    }

    if targets.is_empty() {
        println!(
            "[docgen] No *.dt.md files found in {}.",
            inputs
                .iter()
                .map(|p| format!("'{}'", p.display()))
                .collect::<Vec<_>>()
                .join(", ")
        );
        return;
    }

    if output_path.is_some() && targets.len() > 1 {
        eprintln!("[docgen] Error: --output can only be used when compiling a single .dt.md file.");
        std::process::exit(1);
    }

    let ok = compile_targets(&targets, output_path.as_deref());
    if !ok && !watch {
        std::process::exit(1);
    }

    if watch {
        println!("[docgen] Watching for changes... (Press Ctrl+C to stop)");
        let (tx, rx) = channel();
        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })
        .expect("Failed to create filesystem watcher");

        for input in &inputs {
            let watch_target = if input.is_file() {
                input.parent().unwrap_or(Path::new(".")).to_path_buf()
            } else {
                input.clone()
            };
            watcher
                .watch(&watch_target, RecursiveMode::Recursive)
                .expect("Failed to watch path");
        }

        while let Ok(Ok(event)) = rx.recv() {
            if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                let is_dt_md = event.paths.iter().any(|p| {
                    p.file_name()
                        .and_then(|s| s.to_str())
                        .map(|s| s.ends_with(".dt.md"))
                        .unwrap_or(false)
                });
                if is_dt_md {
                    // Drain any rapid consecutive events
                    std::thread::sleep(Duration::from_millis(60));
                    while rx.try_recv().is_ok() {}
                    let mut refreshed_targets = Vec::new();
                    for input in &inputs {
                        let _ = discover_dt_md_files(input, &mut refreshed_targets);
                    }
                    compile_targets(&refreshed_targets, output_path.as_deref());
                }
            }
        }
    }
}

