use std::env;
use std::fs;
use std::path::PathBuf;

use directedtype::pdf::{compile_file_to_pdf, PdfOptions, PdfVersionOpt};

fn print_usage() {
    println!("DirectedType Vector PDF Compiler");
    println!("Usage:");
    println!("  dt2pdf <input.dt> [OPTIONS]");
    println!();
    println!("Options:");
    println!("  -o, --output <PATH>         Output PDF file path (default: <input>.pdf)");
    println!("      --pdf-version <1.7|2.0> Target PDF specification version (default: 1.7)");
    println!("      --compressed            Enable FlateDecode stream compression (default)");
    println!("      --uncompressed          Disable stream compression for inspection");
    println!("      --width <PT>            Fallback viewport/page width in PDF points");
    println!("      --height <PT>           Fallback viewport/page height in PDF points");
    println!("      --deterministic         Pin PDF metadata/IDs for reproducible bytes");
    println!("      --strict-links          Fail on unresolved internal #anchor links");
    println!("  -h, --help                  Display this help message");
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();

    let mut input_path: Option<PathBuf> = None;
    let mut output_path: Option<PathBuf> = None;
    let mut options = PdfOptions::default();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_usage();
                return Ok(());
            }
            "-o" | "--output" => {
                i += 1;
                if i >= args.len() {
                    return Err("Missing path value after --output".into());
                }
                output_path = Some(PathBuf::from(&args[i]));
            }
            "--pdf-version" => {
                i += 1;
                if i >= args.len() {
                    return Err("Missing version after --pdf-version (expected 1.7 or 2.0)".into());
                }
                options.pdf_version = match args[i].as_str() {
                    "1.7" => PdfVersionOpt::Pdf17,
                    "2.0" => PdfVersionOpt::Pdf20,
                    other => {
                        return Err(format!(
                            "Unsupported --pdf-version '{}' (expected 1.7 or 2.0)",
                            other
                        )
                        .into());
                    }
                };
            }
            "--compressed" => {
                options.compressed = true;
            }
            "--uncompressed" => {
                options.compressed = false;
            }
            "--width" => {
                i += 1;
                if i >= args.len() {
                    return Err("Missing point value after --width".into());
                }
                let w: f64 = args[i]
                    .parse()
                    .map_err(|_| format!("Invalid --width '{}': expected numeric pt", args[i]))?;
                if w <= 0.0 {
                    return Err("--width must be positive".into());
                }
                options.fallback_width = Some(w);
            }
            "--height" => {
                i += 1;
                if i >= args.len() {
                    return Err("Missing point value after --height".into());
                }
                let h: f64 = args[i]
                    .parse()
                    .map_err(|_| format!("Invalid --height '{}': expected numeric pt", args[i]))?;
                if h <= 0.0 {
                    return Err("--height must be positive".into());
                }
                options.fallback_height = Some(h);
            }
            "--deterministic" => {
                options.deterministic = true;
            }
            "--strict-links" => {
                options.strict_links = true;
            }
            flag if flag.starts_with('-') => {
                return Err(format!("Unknown option '{}'. Pass --help for usage.", flag).into());
            }
            pos => {
                if input_path.is_some() {
                    return Err(format!("Unexpected extra positional argument '{}'", pos).into());
                }
                input_path = Some(PathBuf::from(pos));
            }
        }
        i += 1;
    }

    let Some(input_path) = input_path else {
        print_usage();
        return Err("Missing required <input.dt> argument".into());
    };

    let out_path = output_path.unwrap_or_else(|| input_path.with_extension("pdf"));
    let pdf_bytes = compile_file_to_pdf(&input_path, &options)?;
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(&out_path, &pdf_bytes)?;
    println!(
        "Wrote {} ({} bytes)",
        out_path.display(),
        pdf_bytes.len()
    );
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("dt2pdf error: {}", err);
        std::process::exit(1);
    }
}
