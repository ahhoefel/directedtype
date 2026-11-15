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

#[test]
#[ignore]
fn bench_compiler_phases() {
    use directedtype::compiler::{
        build_variable_graph_with_window, evaluate_graph_with_state, expand_document_with_resolver,
        resolve_layout, sort_graph,
    };
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    let registry = ComponentRegistry::standard();
    let resolver = FsResolver;

    let gallery_path = Path::new("examples/material_gallery.dt");
    let gallery_src = fs::read_to_string(gallery_path).unwrap();
    let gallery_doc = parse_document(&gallery_src).unwrap();
    let base_dir = gallery_path.parent().unwrap();

    // Warmup (loads fonts, caches component schemas/templates)
    for _ in 0..5 {
        let _ = CompiledDocument::compile_with_registry(
            &gallery_doc,
            1200.0,
            900.0,
            base_dir,
            &resolver,
            &registry,
        )
        .unwrap();
    }

    let iters = 50;
    let mut expand_total = Duration::ZERO;
    let mut graph_total = Duration::ZERO;
    let mut topo_total = Duration::ZERO;
    let mut eval_total = Duration::ZERO;
    let mut layout_total = Duration::ZERO;
    let mut full_total = Duration::ZERO;
    let mut var_count = 0;
    let mut node_count = 0;

    for _ in 0..iters {
        let t0 = Instant::now();
        let expanded = expand_document_with_resolver(&gallery_doc, base_dir, &resolver).unwrap();
        let t1 = Instant::now();
        let graph = build_variable_graph_with_window(&expanded, 1200.0, 900.0).unwrap();
        let t2 = Instant::now();
        let schedule = sort_graph(&graph).unwrap();
        let t3 = Instant::now();
        let values = evaluate_graph_with_state(&graph, &schedule, &HashMap::new()).unwrap();
        let t4 = Instant::now();
        let layout = resolve_layout(&expanded, values);
        let t5 = Instant::now();

        var_count = graph.var_nodes.len();
        node_count = layout.nodes.len();
        expand_total += t1 - t0;
        graph_total += t2 - t1;
        topo_total += t3 - t2;
        eval_total += t4 - t3;
        layout_total += t5 - t4;

        let f0 = Instant::now();
        let _ = CompiledDocument::compile_with_registry(
            &gallery_doc,
            1200.0,
            900.0,
            base_dir,
            &resolver,
            &registry,
        )
        .unwrap();
        full_total += f0.elapsed();
    }

    println!("\n=== material_gallery.dt ({iters} iterations, warm cache) ===");
    println!("Nodes: {node_count}, DAG Variables: {var_count}");
    println!(
        "  Expand AST:     {:>8.3} ms/iter",
        expand_total.as_secs_f64() * 1000.0 / iters as f64
    );
    println!(
        "  Build Graph:    {:>8.3} ms/iter",
        graph_total.as_secs_f64() * 1000.0 / iters as f64
    );
    println!(
        "  Topo Sort:      {:>8.3} ms/iter",
        topo_total.as_secs_f64() * 1000.0 / iters as f64
    );
    println!(
        "  Eval DAG:       {:>8.3} ms/iter",
        eval_total.as_secs_f64() * 1000.0 / iters as f64
    );
    println!(
        "  Resolve Layout: {:>8.3} ms/iter",
        layout_total.as_secs_f64() * 1000.0 / iters as f64
    );
    println!(
        "  Sum of phases:  {:>8.3} ms/iter",
        (expand_total + graph_total + topo_total + eval_total + layout_total).as_secs_f64()
            * 1000.0
            / iters as f64
    );
    println!(
        "  compile_with_registry: {:>8.3} ms/iter",
        full_total.as_secs_f64() * 1000.0 / iters as f64
    );
}

