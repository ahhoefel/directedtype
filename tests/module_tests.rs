use directedtype::ast::{ComponentKey, Expr};
use directedtype::compiler::error::CompileError;
use directedtype::compiler::module::{normalize_path, VirtualResolver};
use directedtype::compiler::{
    evaluate_document_with_base_dir, evaluate_document_with_resolver,
    expand_document_with_resolver,
};
use directedtype::parse;
use std::path::{Path, PathBuf};

#[test]
fn test_path_normalization() {
    assert_eq!(
        normalize_path(Path::new("a/b/../c")),
        PathBuf::from("a/c")
    );
    assert_eq!(
        normalize_path(Path::new("./a/./b")),
        PathBuf::from("a/b")
    );
    assert_eq!(
        normalize_path(Path::new("../primitives/Button.dt")),
        PathBuf::from("../primitives/Button.dt")
    );
    assert_eq!(
        normalize_path(Path::new("/root/dir/../other")),
        PathBuf::from("/root/other")
    );
    assert_eq!(normalize_path(Path::new(".")), PathBuf::from("."));
}

#[test]
fn test_basic_module_import_and_layout() {
    let mut resolver = VirtualResolver::new();
    resolver.insert(
        "components/Button.dt",
        r#"
        \Component Button(width: Number: 120, height: Number: 40) {
            \Rect(x: 0, y: 0, width: width, height: height, color: #1E293B)
        }
        "#,
    );

    let main_src = r#"
    \use "./components/Button.dt";

    \Button()
    "#;

    let doc = parse(main_src).expect("Failed to parse main");
    let layout = evaluate_document_with_resolver(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &resolver,
    )
    .expect("Failed to evaluate layout with module resolution");

    assert_eq!(layout.roots.len(), 1);
    let rect_node = layout.nodes.iter().find(|n| n.name == "Rect").expect("Should have Rect node");
    assert_eq!(rect_node.rect.width, 120.0);
    assert_eq!(rect_node.rect.height, 40.0);
}

#[test]
fn test_aliased_module_import() {
    let mut resolver = VirtualResolver::new();
    resolver.insert(
        "widgets/Button.dt",
        r#"
        \Component Button(width: Number: 100, height: Number: 30) {
            \Rect(x: 0, y: 0, width: width, height: height, color: #2563EB)
        }
        "#,
    );

    let main_src = r#"
    \use "./widgets/Button.dt" as PrimaryButton;

    \PrimaryButton(width: 250, height: 60)
    "#;

    let doc = parse(main_src).expect("Failed to parse main");
    let layout = evaluate_document_with_resolver(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &resolver,
    )
    .expect("Failed to evaluate aliased import");

    assert_eq!(layout.roots.len(), 1);
    let root_node = &layout.nodes[layout.roots[0].0];
    assert_eq!(root_node.name, "PrimaryButton");
    let rect_node = layout.nodes.iter().find(|n| n.name == "Rect").expect("Should have Rect node");
    assert_eq!(rect_node.rect.width, 250.0);
    assert_eq!(rect_node.rect.height, 60.0);
}

#[test]
fn test_transitive_imports() {
    let mut resolver = VirtualResolver::new();
    resolver.insert(
        "ui/Icon.dt",
        r#"
        \Component Icon(size: Number: 16) {
            \Rect(x: 0, y: 0, width: size, height: size, color: #E2E8F0)
        }
        "#,
    );
    resolver.insert(
        "ui/Button.dt",
        r#"
        \use "./Icon.dt";

        \Component Button() {
            \Icon(size: 24)
        }
        "#,
    );

    let main_src = r#"
    \use "./ui/Button.dt";

    \Button()
    "#;

    let doc = parse(main_src).expect("Failed to parse main");
    let layout = evaluate_document_with_resolver(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &resolver,
    )
    .expect("Failed to evaluate transitive imports");

    assert_eq!(layout.roots.len(), 1);
    let rect_node = layout.nodes.iter().find(|n| n.name == "Rect").expect("Should have Rect node");
    assert_eq!(rect_node.rect.width, 24.0);
    assert_eq!(rect_node.rect.height, 24.0);
}

#[test]
fn test_transitive_imports_with_alias() {
    let mut resolver = VirtualResolver::new();
    resolver.insert(
        "ui/Icon.dt",
        r#"
        \Component Icon(size: Number: 16) {
            \Rect(x: 0, y: 0, width: size, height: size, color: #E2E8F0)
        }
        "#,
    );
    resolver.insert(
        "ui/Button.dt",
        r#"
        \use "./Icon.dt";

        \Component Button() {
            \Icon(size: 32)
        }
        "#,
    );

    let main_src = r#"
    \use "./ui/Button.dt" as CoolButton;

    \CoolButton()
    "#;

    let doc = parse(main_src).expect("Failed to parse main");
    let layout = evaluate_document_with_resolver(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &resolver,
    )
    .expect("Failed to evaluate transitive imports with alias");

    assert_eq!(layout.roots.len(), 1);
    let root_node = &layout.nodes[layout.roots[0].0];
    assert_eq!(root_node.name, "CoolButton");
    let rect_node = layout.nodes.iter().find(|n| n.name == "Rect").expect("Should have Rect node");
    assert_eq!(rect_node.rect.width, 32.0);
    assert_eq!(rect_node.rect.height, 32.0);
}

#[test]
fn test_diamond_dependency_resolution() {
    let mut resolver = VirtualResolver::new();
    resolver.insert(
        "core/Base.dt",
        r#"
        \Component BaseBox(dim: Number: 50) {
            \Rect(x: 0, y: 0, width: dim, height: dim, color: #000)
        }
        "#,
    );
    resolver.insert(
        "core/Left.dt",
        r#"
        \use "./Base.dt";
        \Component LeftComp() {
            \BaseBox(dim: 60)
        }
        "#,
    );
    resolver.insert(
        "core/Right.dt",
        r#"
        \use "./Base.dt";
        \Component RightComp() {
            \BaseBox(dim: 70)
        }
        "#,
    );

    let main_src = r#"
    \use "./core/Left.dt";
    \use "./core/Right.dt";

    \LeftComp()
    \RightComp()
    "#;

    let doc = parse(main_src).expect("Failed to parse main");
    let layout = evaluate_document_with_resolver(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &resolver,
    )
    .expect("Diamond dependency imports should resolve cleanly");

    assert_eq!(layout.roots.len(), 2);
    let rect_nodes: Vec<_> = layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rect_nodes.len(), 2);
    assert_eq!(rect_nodes[0].rect.width, 60.0);
    assert_eq!(rect_nodes[1].rect.width, 70.0);
}

#[test]
fn test_cyclic_import_detection() {
    let mut resolver = VirtualResolver::new();
    resolver.insert(
        "A.dt",
        r#"
        \use "./B.dt";
        \Component AComp() { \Rect(x: 0, y: 0, width: 10, height: 10, color: #000) }
        "#,
    );
    resolver.insert(
        "B.dt",
        r#"
        \use "./A.dt";
        \Component BComp() { \Rect(x: 0, y: 0, width: 10, height: 10, color: #000) }
        "#,
    );

    let main_src = r#"
    \use "./A.dt";
    \AComp()
    "#;

    let doc = parse(main_src).expect("Failed to parse main");
    let err = evaluate_document_with_resolver(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &resolver,
    )
    .expect_err("Cyclic imports should be rejected with CompileError::CyclicImport");

    match err {
        CompileError::CyclicImport { path, .. } => {
            assert!(path.contains("A.dt") || path.contains("B.dt"));
        }
        other => panic!("Expected CyclicImport, got {:?}", other),
    }
}

#[test]
fn test_self_cyclic_import_detection() {
    let mut resolver = VirtualResolver::new();
    resolver.insert(
        "SelfCycle.dt",
        r#"
        \use "./SelfCycle.dt";
        \Component Foo() { \Rect(x: 0, y: 0, width: 10, height: 10, color: #000) }
        "#,
    );

    let main_src = r#"
    \use "./SelfCycle.dt";
    \Foo()
    "#;

    let doc = parse(main_src).expect("Failed to parse main");
    let err = evaluate_document_with_resolver(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &resolver,
    )
    .expect_err("Self cyclic import should be rejected");

    match err {
        CompileError::CyclicImport { .. } => {}
        other => panic!("Expected CyclicImport, got {:?}", other),
    }
}

#[test]
fn test_missing_file_import_error() {
    let resolver = VirtualResolver::new();
    let main_src = r#"
    \use "./NonExistent.dt";
    \Rect(x: 0, y: 0, width: 10, height: 10, color: #000)
    "#;

    let doc = parse(main_src).expect("Failed to parse main");
    let err = evaluate_document_with_resolver(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &resolver,
    )
    .expect_err("Missing file should produce ImportError");

    match err {
        CompileError::ImportError { path, .. } => {
            assert_eq!(path, "./NonExistent.dt");
        }
        other => panic!("Expected ImportError, got {:?}", other),
    }
}

#[test]
fn test_syntax_error_in_imported_file() {
    let mut resolver = VirtualResolver::new();
    resolver.insert(
        "Broken.dt",
        r#"
        \Component Unclosed {
        "#,
    );

    let main_src = r#"
    \use "./Broken.dt";
    \Rect(x: 0, y: 0, width: 10, height: 10, color: #000)
    "#;

    let doc = parse(main_src).expect("Failed to parse main");
    let err = evaluate_document_with_resolver(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &resolver,
    )
    .expect_err("Syntax error in imported file should produce ImportError");

    match err {
        CompileError::ImportError { path, message, .. } => {
            assert_eq!(path, "./Broken.dt");
            assert!(message.contains("Parse error"));
        }
        other => panic!("Expected ImportError, got {:?}", other),
    }
}

#[test]
fn test_imported_component_with_state_and_structured_key() {
    let mut resolver = VirtualResolver::new();
    resolver.insert(
        "Counter.dt",
        r#"
        \Component Counter(multiplier: Number: 10) {
            state count: 42;
            \Rect(x: 0, y: 0, width: count * multiplier, height: 30, color: #0F0)
        }
        "#,
    );

    let main_src = r#"
    \use "./Counter.dt";

    \Counter(1, "main"; multiplier: 5)
    "#;

    let doc = parse(main_src).expect("Failed to parse main");
    let expanded = expand_document_with_resolver(&doc, Path::new("."), &resolver)
        .expect("Failed to expand document");

    // Verify structured key lookup
    let expected_key = ComponentKey::tuple(&[Expr::number(1.0), Expr::string("main")]);
    let counter_node = expanded
        .find_by_key(None, &expected_key)
        .expect("Should find Counter with key [1, 'main']");
    assert_eq!(counter_node.name, "Counter");

    let layout = evaluate_document_with_resolver(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &resolver,
    )
    .expect("Failed to evaluate layout");

    assert_eq!(layout.roots.len(), 1);
    let rect_node = layout.nodes.iter().find(|n| n.name == "Rect").expect("Should have Rect node");
    // count (42) * multiplier (5) = 210
    assert_eq!(rect_node.rect.width, 210.0);
    assert_eq!(rect_node.rect.height, 30.0);
}

#[test]
fn test_fs_resolver_disk_import() {
    let temp_dir = std::env::temp_dir().join(format!("dt_test_{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).expect("Failed to create temp dir");

    let comp_file = temp_dir.join("DiskWidget.dt");
    std::fs::write(
        &comp_file,
        r#"
        \Component DiskWidget(w: Number: 180) {
            \Rect(x: 0, y: 0, width: w, height: 45, color: #3B82F6)
        }
        "#,
    )
    .expect("Failed to write component file");

    let main_src = r#"
    \use "./DiskWidget.dt";

    \DiskWidget(w: 320)
    "#;

    let doc = parse(main_src).expect("Failed to parse main");
    let layout_res = evaluate_document_with_base_dir(&doc, &temp_dir);

    // Clean up
    let _ = std::fs::remove_file(&comp_file);
    let _ = std::fs::remove_dir(&temp_dir);

    let layout = layout_res.expect("Failed to evaluate disk import");
    assert_eq!(layout.roots.len(), 1);
    let rect_node = layout.nodes.iter().find(|n| n.name == "Rect").expect("Should have Rect node");
    assert_eq!(rect_node.rect.width, 320.0);
    assert_eq!(rect_node.rect.height, 45.0);
}
