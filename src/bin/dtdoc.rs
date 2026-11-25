use std::env;
use std::fs;
use std::path::PathBuf;

use directedtype::compdoc::{generate_docs_pdf, DocGenOptions};
use directedtype::pdf::{PdfOptions, PdfVersionOpt};

fn print_usage() {
    println!("DirectedType Automatic Component Documentation Generator (dtdoc)");
    println!();
    println!("Scans DirectedType .dt component definitions, extracts public components,");
    println!("overloads, parameters, aliases, enums, provided environment bindings,");
    println!("and top-level bindings, and compiles an internally linked PDF with a");
    println!("Table of Contents and PDF outlines.");
    println!();
    println!("Usage:");
    println!("  cargo dtdoc [FILES_OR_DIRS...] [OPTIONS]");
    println!();
    println!("Arguments:");
    println!("  [FILES_OR_DIRS...]          One or more .dt files or directories to document");
    println!("                              (defaults to 'components/' if omitted)");
    println!();
    println!("Options:");
    println!("  -o, --output <PATH>         Output PDF path (default: docs/components.pdf)");
    println!("      --emit-dt <PATH>        Also write the generated .dt source file");
    println!("      --title <TITLE>         Document title (default: \"DirectedType Component Reference\")");
    println!("      --subtitle <TEXT>       Document subtitle");
    println!("      --root <DIR>            Base directory for relative paths & imports (default: .)");
    println!("      --pdf-version <1.7|2.0> Target PDF specification version (default: 1.7)");
    println!("      --uncompressed          Disable PDF stream compression for inspection");
    println!("      --deterministic         Pin PDF metadata/IDs for reproducible bytes");
    println!("      --no-strict-links       Allow unresolved internal #anchor links (strict by default)");
    println!("  -h, --help                  Display this help message");
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();

    let mut inputs: Vec<PathBuf> = Vec::new();
    let mut output_path = PathBuf::from("docs/components.pdf");
    let mut emit_dt_path: Option<PathBuf> = None;
    let mut doc_opts = DocGenOptions::default();
    let mut pdf_opts = PdfOptions {
        strict_links: true,
        ..Default::default()
    };

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
                output_path = PathBuf::from(&args[i]);
            }
            "--emit-dt" => {
                i += 1;
                if i >= args.len() {
                    return Err("Missing path value after --emit-dt".into());
                }
                emit_dt_path = Some(PathBuf::from(&args[i]));
            }
            "--title" => {
                i += 1;
                if i >= args.len() {
                    return Err("Missing value after --title".into());
                }
                doc_opts.title = args[i].clone();
            }
            "--subtitle" => {
                i += 1;
                if i >= args.len() {
                    return Err("Missing value after --subtitle".into());
                }
                doc_opts.subtitle = Some(args[i].clone());
            }
            "--root" => {
                i += 1;
                if i >= args.len() {
                    return Err("Missing directory after --root".into());
                }
                doc_opts.root_dir = PathBuf::from(&args[i]);
            }
            "--pdf-version" => {
                i += 1;
                if i >= args.len() {
                    return Err("Missing version after --pdf-version (expected 1.7 or 2.0)".into());
                }
                pdf_opts.pdf_version = match args[i].as_str() {
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
            "--uncompressed" => {
                pdf_opts.compressed = false;
            }
            "--deterministic" => {
                pdf_opts.deterministic = true;
            }
            "--no-strict-links" => {
                pdf_opts.strict_links = false;
            }
            flag if flag.starts_with('-') => {
                return Err(format!("Unknown option '{}'. Pass --help for usage.", flag).into());
            }
            pos => {
                inputs.push(PathBuf::from(pos));
            }
        }
        i += 1;
    }

    if inputs.is_empty() {
        inputs.push(PathBuf::from("components"));
    }

    let (catalog, dt_source, pdf_bytes) = generate_docs_pdf(&inputs, &doc_opts, &pdf_opts)?;

    if let Some(dt_path) = emit_dt_path {
        if let Some(parent) = dt_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(&dt_path, &dt_source)?;
        println!(
            "[dtdoc] Wrote {} ({} bytes)",
            dt_path.display(),
            dt_source.len()
        );
    }

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(&output_path, &pdf_bytes)?;
    println!(
        "[dtdoc] Documented {} package(s), {} module(s), {} component(s) ({} overload(s)), {} enum(s), {} public binding(s)",
        catalog.packages.len(),
        catalog.module_count(),
        catalog.component_count(),
        catalog.overload_count(),
        catalog.enum_count(),
        catalog.binding_count()
    );
    println!(
        "[dtdoc] Wrote {} ({} bytes)",
        output_path.display(),
        pdf_bytes.len()
    );
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("dtdoc error: {err}");
        std::process::exit(1);
    }
}
