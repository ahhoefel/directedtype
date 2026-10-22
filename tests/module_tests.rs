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
        \Component Button(width: Distance = 120px, height: Distance = 40px) {
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
        \Component Button(width: Distance = 100px, height: Distance = 30px) {
            \Rect(x: 0, y: 0, width: width, height: height, color: #2563EB)
        }
        "#,
    );

    let main_src = r#"
    \use "./widgets/Button.dt" as PrimaryButton;

    \PrimaryButton(width: 250px, height: 60px)
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
        \Component Icon(size: Distance = 16px) {
            \Rect(x: 0, y: 0, width: size, height: size, color: #E2E8F0)
        }
        "#,
    );
    resolver.insert(
        "ui/Button.dt",
        r#"
        \use "./Icon.dt";

        \Component Button() {
            \Icon(size: 24px)
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
        \Component Icon(size: Distance = 16px) {
            \Rect(x: 0, y: 0, width: size, height: size, color: #E2E8F0)
        }
        "#,
    );
    resolver.insert(
        "ui/Button.dt",
        r#"
        \use "./Icon.dt";

        \Component Button() {
            \Icon(size: 32px)
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
        \Component BaseBox(dim: Distance = 50px) {
            \Rect(x: 0, y: 0, width: dim, height: dim, color: #000)
        }
        "#,
    );
    resolver.insert(
        "core/Left.dt",
        r#"
        \use "./Base.dt";
        \Component LeftComp() {
            \BaseBox(dim: 60px)
        }
        "#,
    );
    resolver.insert(
        "core/Right.dt",
        r#"
        \use "./Base.dt";
        \Component RightComp() {
            \BaseBox(dim: 70px)
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
        \Component AComp() { \Rect(x: 0, y: 0, width: 10px, height: 10px, color: #000) }
        "#,
    );
    resolver.insert(
        "B.dt",
        r#"
        \use "./A.dt";
        \Component BComp() { \Rect(x: 0, y: 0, width: 10px, height: 10px, color: #000) }
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
        \Component Foo() { \Rect(x: 0, y: 0, width: 10px, height: 10px, color: #000) }
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
    \Rect(x: 0, y: 0, width: 10px, height: 10px, color: #000)
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
    \Rect(x: 0, y: 0, width: 10px, height: 10px, color: #000)
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

    match &err {
        CompileError::ImportError { path, message, .. } => {
            assert_eq!(path, "./Broken.dt");
            assert!(message.contains("Parse error"));
            assert!(
                message.contains("(Broken.dt:2:29)"),
                "Expected imported file location in error message, got: {message}"
            );
            let full = err.to_string();
            assert!(
                full.contains("Import failed for './Broken.dt' (2:5):"),
                "Expected caller import location in full error, got: {full}"
            );
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
        \Component Counter(multiplier: Distance = 10px) {
            \state count: Integer = 42;
            \Rect(x: 0, y: 0, width: count * multiplier, height: 30px, color: #0F0)
        }
        "#,
    );

    let main_src = r#"
    \use "./Counter.dt";

    \Counter(1, "main"; multiplier: 5px)
    "#;

    let doc = parse(main_src).expect("Failed to parse main");
    let expanded = expand_document_with_resolver(&doc, Path::new("."), &resolver)
        .expect("Failed to expand document");

    // Verify structured key lookup
    let expected_key = ComponentKey::tuple(&[Expr::integer(1), Expr::string("main")]);
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
    // count (42) * multiplier (5px) = 210
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
        \Component DiskWidget(w: Distance = 180px) {
            \Rect(x: 0, y: 0, width: w, height: 45px, color: #3B82F6)
        }
        "#,
    )
    .expect("Failed to write component file");

    let main_src = r#"
    \use "./DiskWidget.dt";

    \DiskWidget(w: 320px)
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

#[test]
fn test_module_import_overloads() {
    let mut resolver = VirtualResolver::new();
    resolver.insert(
        "components/Card.dt",
        r#"
        \Component Card(width: Distance) {
            \Rect(x: 0, y: 0, width: width, height: 40px, color: #1E293B)
        }
        \Component Card(height: Distance) {
            \Rect(x: 0, y: 0, width: 80px, height: height, color: #334155)
        }
        "#,
    );

    let main_src = r#"
    \use "./components/Card.dt";

    \Card(width: 150px)
    \Card(height: 90px)
    "#;

    let doc = parse(main_src).expect("Failed to parse main");
    let layout = evaluate_document_with_resolver(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &resolver,
    )
    .expect("Failed to evaluate layout with overloaded module imports");

    assert_eq!(layout.roots.len(), 2);
    let rects: Vec<_> = layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 2);
    assert_eq!(rects[0].rect.width, 150.0);
    assert_eq!(rects[0].rect.height, 40.0);
    assert_eq!(rects[1].rect.width, 80.0);
    assert_eq!(rects[1].rect.height, 90.0);
}

#[test]
fn test_aliased_module_import_overloads() {
    let mut resolver = VirtualResolver::new();
    resolver.insert(
        "components/Card.dt",
        r#"
        \Component Card(width: Distance) {
            \Rect(x: 0, y: 0, width: width, height: 40px, color: #1E293B)
        }
        \Component Card(height: Distance) {
            \Rect(x: 0, y: 0, width: 80px, height: height, color: #334155)
        }
        "#,
    );

    let main_src = r#"
    \use "./components/Card.dt" as PrimaryCard;

    \PrimaryCard(width: 220px)
    \PrimaryCard(height: 110px)
    "#;

    let doc = parse(main_src).expect("Failed to parse main");
    let layout = evaluate_document_with_resolver(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &resolver,
    )
    .expect("Failed to evaluate layout with aliased overloaded module");

    assert_eq!(layout.roots.len(), 2);
    let rects: Vec<_> = layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 2);
    assert_eq!(rects[0].rect.width, 220.0);
    assert_eq!(rects[0].rect.height, 40.0);
    assert_eq!(rects[1].rect.width, 80.0);
    assert_eq!(rects[1].rect.height, 110.0);
}

#[test]
fn test_diamond_dependency_with_overloads() {
    let mut resolver = VirtualResolver::new();
    resolver.insert(
        "core/Base.dt",
        r#"
        \Component BaseBox(width: Distance) {
            \Rect(x: 0, y: 0, width: width, height: 50px, color: #000)
        }
        \Component BaseBox(height: Distance) {
            \Rect(x: 0, y: 0, width: 50px, height: height, color: #111)
        }
        "#,
    );
    resolver.insert(
        "core/Left.dt",
        r#"
        \use "./Base.dt";
        \Component LeftComp() {
            \BaseBox(width: 75px)
        }
        "#,
    );
    resolver.insert(
        "core/Right.dt",
        r#"
        \use "./Base.dt";
        \Component RightComp() {
            \BaseBox(height: 85px)
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
    .expect("Diamond dependency with overloads should resolve idempotently");

    assert_eq!(layout.roots.len(), 2);
    let rects: Vec<_> = layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 2);
    assert_eq!(rects[0].rect.width, 75.0);
    assert_eq!(rects[0].rect.height, 50.0);
    assert_eq!(rects[1].rect.width, 50.0);
    assert_eq!(rects[1].rect.height, 85.0);
}

#[test]
fn test_named_import_components_and_layout() {
    let mut resolver = VirtualResolver::new();
    resolver.insert(
        "ui/Widgets.dt",
        r#"
        \Component PrimaryButton(width: Distance = 100px) {
            \Rect(x: 0, y: 0, width: width, height: 40px, color: #2563EB)
        }
        \Component SecondaryButton(width: Distance = 80px) {
            \Rect(x: 0, y: 0, width: width, height: 35px, color: #64748B)
        }
        \Component SecretWidget() {
            \Rect(x: 0, y: 0, width: 999px, height: 999px, color: #FF0000)
        }
        "#,
    );

    let main_src = r#"
    \use { PrimaryButton, SecondaryButton as SecBtn } from "./ui/Widgets.dt";

    \PrimaryButton(width: 150px)
    \SecBtn(width: 90px)
    "#;

    let doc = parse(main_src).expect("Failed to parse main");
    let layout = evaluate_document_with_resolver(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &resolver,
    )
    .expect("Failed to evaluate named import layout");

    assert_eq!(layout.roots.len(), 2);
    let rects: Vec<_> = layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 2);
    assert_eq!(rects[0].rect.width, 150.0);
    assert_eq!(rects[1].rect.width, 90.0);
}

#[test]
fn test_named_import_unimported_component_rejected() {
    let mut resolver = VirtualResolver::new();
    resolver.insert(
        "ui/Widgets.dt",
        r#"
        \Component PublicWidget() {
            \Rect(x: 0, y: 0, width: 100px, height: 50px, color: #000)
        }
        \Component SecretWidget() {
            \Rect(x: 0, y: 0, width: 999px, height: 999px, color: #FF0000)
        }
        "#,
    );

    let main_src = r#"
    \use { PublicWidget } from "./ui/Widgets.dt";

    \SecretWidget()
    "#;

    let doc = parse(main_src).expect("Failed to parse main");
    let err = evaluate_document_with_resolver(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &resolver,
    )
    .expect_err("Unimported component should not be in scope");

    match err {
        CompileError::UndefinedComponent { name, .. } => {
            assert_eq!(name, "SecretWidget");
        }
        other => panic!("Expected UndefinedComponent error on SecretWidget, got {:?}", other),
    }
}

#[test]
fn test_named_import_unknown_symbol_error() {
    let mut resolver = VirtualResolver::new();
    resolver.insert(
        "ui/Widgets.dt",
        r#"
        \Component PublicWidget() {
            \Rect(x: 0, y: 0, width: 100px, height: 50px, color: #000)
        }
        "#,
    );

    let main_src = r#"
    \use { NonExistent } from "./ui/Widgets.dt";

    \PublicWidget()
    "#;

    let doc = parse(main_src).expect("Failed to parse main");
    let err = evaluate_document_with_resolver(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &resolver,
    )
    .expect_err("Importing non-existent symbol should fail");

    match err {
        CompileError::ImportError { message, .. } => {
            assert!(message.contains("Cannot import 'NonExistent'"));
            assert!(message.contains("symbol not found"));
        }
        other => panic!("Expected ImportError, got {:?}", other),
    }
}

#[test]
fn test_named_import_helper_scoping_and_independence() {
    let mut resolver = VirtualResolver::new();
    resolver.insert(
        "ui/Card.dt",
        r#"
        \Component Helper(w: Distance) {
            \Rect(x: 0, y: 0, width: w, height: 20px, color: #111)
        }

        \Component Card(width: Distance) {
            \Helper(w: width)
            \Rect(x: 0, y: 20px, width: width, height: 100px, color: #222)
        }
        "#,
    );

    // Main imports Card, but ALSO defines its own Helper component with completely different signature/behavior!
    let main_src = r#"
    \use { Card } from "./ui/Card.dt";

    \Component Helper(extra_h: Distance) {
        \Rect(x: 0, y: 150px, width: 80px, height: extra_h, color: #333)
    }

    \Card(width: 200px)
    \Helper(extra_h: 45px)
    "#;

    let doc = parse(main_src).expect("Failed to parse main");
    let layout = evaluate_document_with_resolver(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &resolver,
    )
    .expect("Caller should be able to define own Helper without colliding with module internal helper");

    assert_eq!(layout.roots.len(), 2);
    // Card produced Rect(width: 200, height: 20) from its internal Helper and Rect(width: 200, height: 100)
    // Caller's Helper produced Rect(width: 80, height: 45)
    let rects: Vec<_> = layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 3);
    assert_eq!(rects[0].rect.width, 200.0);
    assert_eq!(rects[0].rect.height, 20.0);
    assert_eq!(rects[1].rect.width, 200.0);
    assert_eq!(rects[1].rect.height, 100.0);
    assert_eq!(rects[2].rect.width, 80.0);
    assert_eq!(rects[2].rect.height, 45.0);
}

#[test]
fn test_named_import_let_and_enum() {
    let mut resolver = VirtualResolver::new();
    resolver.insert(
        "theme/DesignTokens.dt",
        r#"
        \let primary_color = #2563EB;
        \let card_height = 80px;
        \let secret_padding = 999px;

        \Enum ThemeVariant { Dark, Light }
        \Enum SecretEnum { A, B }
        "#,
    );

    let main_src = r#"
    \use { primary_color as brand_color, card_height, ThemeVariant } from "./theme/DesignTokens.dt";

    \Component TokenBox(variant: ThemeVariant) {
        \Rect(x: 0, y: 0, width: 120px, height: card_height, color: brand_color)
    }

    \TokenBox(variant: ThemeVariant.Dark)
    "#;

    let doc = parse(main_src).expect("Failed to parse main");
    let layout = evaluate_document_with_resolver(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &resolver,
    )
    .expect("Named imports of let and enum should evaluate cleanly");

    assert_eq!(layout.roots.len(), 1);
    let rect = layout.nodes.iter().find(|n| n.name == "Rect").expect("Should have Rect");
    assert_eq!(rect.rect.width, 120.0);
    assert_eq!(rect.rect.height, 80.0);
    assert_eq!(
        rect.properties
            .get("color")
            .and_then(|v| v.as_str())
            .map(|s| s.to_lowercase()),
        Some("#2563eb".to_string())
    );
}

#[test]
fn test_transitive_enum_not_automatically_imported() {
    let mut resolver = VirtualResolver::new();
    resolver.insert(
        "components/Align.dt",
        r#"
        \Enum Align { Left, Center, Right }
        "#,
    );
    resolver.insert(
        "components/Card.dt",
        r#"
        \use { Align } from "./Align.dt";

        \Component Card(align: Align = Align.Center) {
            \Rect(x: 0, y: 0, width: 100px, height: 50px, color: #000)
        }
        "#,
    );

    // Main imports Card, but does NOT import Align. Using Align.Center in Main should fail.
    let main_src = r#"
    \use { Card } from "./components/Card.dt";

    \Card(align: Align.Center)
    "#;

    let doc = parse(main_src).expect("Failed to parse main");
    let err = evaluate_document_with_resolver(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &resolver,
    )
    .expect_err("Unimported enum Align should fail compilation");

    match err {
        CompileError::UnresolvedIdentifier { name, .. } => {
            assert_eq!(name, "Align");
        }
        other => panic!("Expected UnresolvedIdentifier, got {:?}", other),
    }

    // Now explicitly import Align from Align.dt into Main; it must succeed.
    let valid_main_src = r#"
    \use { Card } from "./components/Card.dt";
    \use { Align } from "./components/Align.dt";

    \Card(align: Align.Center)
    "#;

    let valid_doc = parse(valid_main_src).expect("Failed to parse valid main");
    let layout = evaluate_document_with_resolver(
        &valid_doc,
        800.0,
        600.0,
        Path::new("."),
        &resolver,
    )
    .expect("Explicitly importing Align should succeed");

    assert_eq!(layout.roots.len(), 1);
}

#[test]
fn test_cannot_import_transitive_symbol_from_intermediary() {
    let mut resolver = VirtualResolver::new();
    resolver.insert(
        "components/Align.dt",
        r#"
        \Enum Align { Left, Center, Right }
        "#,
    );
    resolver.insert(
        "components/Card.dt",
        r#"
        \use { Align } from "./Align.dt";

        \Component Card() {
            \Rect(x: 0, y: 0, width: 100px, height: 50px, color: #000)
        }
        "#,
    );

    // Trying to import Align from Card.dt (which only imported it, didn't define it) must be rejected
    let main_src = r#"
    \use { Align } from "./components/Card.dt";

    \Rect(x: 0, y: 0, width: 50px, height: 50px, color: #000)
    "#;

    let doc = parse(main_src).expect("Failed to parse main");
    let err = evaluate_document_with_resolver(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &resolver,
    )
    .expect_err("Importing symbol from intermediary module should fail");

    match err {
        CompileError::ImportError { message, .. } => {
            assert!(message.contains("Cannot import 'Align'"));
            assert!(message.contains("symbol not found in './components/Card.dt'"));
        }
        other => panic!("Expected ImportError, got {:?}", other),
    }
}



