use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{exit, Command};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    let subcommand = args.first().map(|s| s.as_str()).unwrap_or("help");

    match subcommand {
        "build-wasm" => {
            let is_release = !args.iter().any(|arg| arg == "--debug");
            if let Err(e) = build_wasm(is_release) {
                eprintln!("[xtask] Error: {e}");
                exit(1);
            }
        }
        "-h" | "--help" | "help" => {
            print_help();
        }
        other => {
            eprintln!("[xtask] Unknown subcommand: '{other}'");
            print_help();
            exit(1);
        }
    }
}

fn print_help() {
    println!("DirectedType Workspace Automation (xtask)");
    println!();
    println!("Usage:");
    println!("  cargo xtask <SUBCOMMAND> [OPTIONS]");
    println!("  or: cargo <SUBCOMMAND>   (via .cargo/config.toml aliases)");
    println!();
    println!("Subcommands:");
    println!("  build-wasm [--debug]    Compile guest WebAssembly companion logic into .wasm");
    println!("                          and place artifacts adjacent to their .dt markup files.");
    println!("  help                    Display this help message");
}

fn build_wasm(is_release: bool) -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(
        env::var("CARGO_MANIFEST_DIR")
            .map_err(|_| "CARGO_MANIFEST_DIR not set (run via cargo xtask)")?,
    );
    let workspace_root = manifest_dir
        .parent()
        .ok_or("Failed to resolve workspace root from xtask manifest dir")?
        .to_path_buf();

    println!("[xtask] Workspace root: {}", workspace_root.display());

    // 1. Discover any guest components in examples/ and ensure they are declared in directedtype-guest/Cargo.toml
    let guest_cargo_toml_path = workspace_root.join("directedtype-guest").join("Cargo.toml");
    let mut guest_cargo_content = fs::read_to_string(&guest_cargo_toml_path)?;

    let examples_dir = workspace_root.join("examples");
    let mut guest_examples = Vec::new();

    if examples_dir.exists() {
        for entry in fs::read_dir(&examples_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                if let Ok(src) = fs::read_to_string(&path) {
                    if src.contains("directedtype_guest") || src.contains("export_component!") {
                        let stem = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .ok_or("Invalid file stem")?
                            .to_string();
                        guest_examples.push((stem.clone(), path.clone()));

                        let example_decl = format!("name = \"{}\"", stem);
                        if !guest_cargo_content.contains(&example_decl) {
                            println!("[xtask] Auto-registering new guest component example '{}' in directedtype-guest/Cargo.toml", stem);
                            guest_cargo_content.push_str(&format!(
                                "\n[[example]]\nname = \"{}\"\npath = \"../examples/{}.rs\"\ncrate-type = [\"cdylib\"]\n",
                                stem, stem
                            ));
                            fs::write(&guest_cargo_toml_path, &guest_cargo_content)?;
                        }
                    }
                }
            }
        }
    }

    if guest_examples.is_empty() {
        println!("[xtask] No guest components found to compile.");
        return Ok(());
    }

    let profile = if is_release { "release" } else { "debug" };
    println!(
        "[xtask] Building {} guest component(s) with target wasm32-unknown-unknown ({profile})...",
        guest_examples.len()
    );

    // 2. Run cargo build --package directedtype-guest --examples --target wasm32-unknown-unknown
    let mut cmd = configure_wasm_cargo_cmd(&workspace_root);
    cmd.arg("build");
    cmd.arg("--package").arg("directedtype-guest");
    cmd.arg("--examples");
    cmd.arg("--target").arg("wasm32-unknown-unknown");
    if is_release {
        cmd.arg("--release");
    }

    let status = cmd.status().map_err(|e| format!("Failed to execute cargo build: {e}"))?;
    if !status.success() {
        eprintln!();
        eprintln!("[xtask] Note: If the error mentions 'can't find crate for `std`', make sure the target is installed:");
        eprintln!("        rustup target add wasm32-unknown-unknown");
        eprintln!("        And ensure ~/.cargo/bin is in your PATH (e.g. export PATH=\"$HOME/.cargo/bin:$PATH\").");
        return Err(format!("cargo build exited with status: {status}").into());
    }

    // 3. Copy compiled .wasm artifacts next to their .rs / .dt source files
    let target_examples_dir = workspace_root
        .join("target")
        .join("wasm32-unknown-unknown")
        .join(profile)
        .join("examples");

    for (name, source_path) in guest_examples {
        let compiled_wasm = target_examples_dir.join(format!("{}.wasm", name));
        if !compiled_wasm.exists() {
            return Err(format!(
                "Expected compiled wasm at '{}' but file was not found",
                compiled_wasm.display()
            )
            .into());
        }

        let dest_wasm = source_path.with_extension("wasm");
        fs::copy(&compiled_wasm, &dest_wasm)?;

        let metadata = fs::metadata(&dest_wasm)?;
        let size_kb = metadata.len() as f64 / 1024.0;
        let rel_dest = dest_wasm.strip_prefix(&workspace_root).unwrap_or(&dest_wasm);
        println!(
            "[xtask] -> Copied {} ({:.1} KB)",
            rel_dest.display(),
            size_kb
        );
    }

    println!("[xtask] Guest WASM compilation completed successfully.");
    Ok(())
}

fn configure_wasm_cargo_cmd(workspace_root: &Path) -> Command {
    let mut cargo_path = PathBuf::from("cargo");
    let mut prepend_cargo_bin = None;

    if let Some(home) = env::var_os("HOME") {
        let cargo_bin_dir = PathBuf::from(&home).join(".cargo").join("bin");
        let cargo_bin = cargo_bin_dir.join("cargo");
        if cargo_bin.exists() {
            cargo_path = cargo_bin;
            prepend_cargo_bin = Some(cargo_bin_dir);
        }
    }

    if cargo_path == PathBuf::from("cargo") {
        if let Some(cargo_env) = env::var_os("CARGO") {
            cargo_path = PathBuf::from(cargo_env);
        }
    }

    let mut cmd = Command::new(&cargo_path);
    cmd.current_dir(workspace_root);

    if let Some(cargo_bin_dir) = prepend_cargo_bin {
        let current_path = env::var_os("PATH").unwrap_or_default();
        let mut new_path = std::ffi::OsString::new();
        new_path.push(&cargo_bin_dir);
        new_path.push(":");
        new_path.push(&current_path);
        cmd.env("PATH", new_path);
    }

    cmd
}
