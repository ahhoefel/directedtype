use directedtype::compiler::compiled::CompiledDocument;
use directedtype::compiler::FsResolver;
use directedtype::component::ComponentRegistry;
use directedtype::parser::parse_document;
use std::fs;
use std::path::Path;

#[test]
fn test_all_examples_compile_and_evaluate() {
    let registry = ComponentRegistry::standard();
    let resolver = FsResolver;
    let examples_dir = Path::new("examples");

    let mut tested_count = 0;
    let mut failures = Vec::new();

    let mut entries: Vec<_> = fs::read_dir(examples_dir)
        .expect("Failed to read examples directory")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|ext| ext.to_str()) == Some("dt"))
        .collect();
    entries.sort();

    for path in entries {
        let display_name = path.display().to_string();
        let source = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{display_name}: Read error: {e}"));
                continue;
            }
        };

        let doc = match parse_document(&source) {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("{display_name}: Parse error: {e}"));
                continue;
            }
        };

        let base_dir = path.parent().unwrap_or(Path::new("."));
        match CompiledDocument::compile_with_registry(
            &doc,
            800.0,
            600.0,
            base_dir,
            &resolver,
            &registry,
        ) {
            Ok(compiled) => {
                assert!(
                    !compiled.layout().nodes.is_empty(),
                    "Layout for {display_name} should have resolved nodes"
                );
                tested_count += 1;
            }
            Err(e) => {
                failures.push(format!("{display_name}: Compilation error: {e}"));
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "Failed to compile {} example(s):\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    assert!(tested_count > 0, "Expected to test at least one example");
    println!("Successfully compiled and evaluated {tested_count} examples.");
}

#[test]
fn test_all_components_parse_and_resolve() {
    let resolver = FsResolver;
    let components_dir = Path::new("components");

    let mut tested_count = 0;
    let mut failures = Vec::new();

    let mut entries: Vec<_> = fs::read_dir(components_dir)
        .expect("Failed to read components directory")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|ext| ext.to_str()) == Some("dt"))
        .collect();
    entries.sort();

    for path in entries {
        let display_name = path.display().to_string();
        let source = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{display_name}: Read error: {e}"));
                continue;
            }
        };

        let doc = match parse_document(&source) {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("{display_name}: Parse error: {e}"));
                continue;
            }
        };

        let base_dir = Path::new(".");
        match directedtype::compiler::module::resolve_imports(&doc, base_dir, &resolver) {
            Ok(_) => {
                tested_count += 1;
            }
            Err(e) => {
                failures.push(format!("{display_name}: Resolution error: {e}"));
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "Failed to resolve {} component(s):\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    assert!(tested_count > 0, "Expected to test at least one component");
    println!("Successfully parsed and resolved {tested_count} standard library components.");
}

