use directedtype::compiler::compiled::CompiledDocument;
use directedtype::compiler::error::CompileError;
use directedtype::component::ComponentRegistry;
use directedtype::parse;
use std::path::Path;

#[test]
fn test_hcenter_fluid_in_window() {
    let input = r#"
    \use "components/HCenter.dt"

    \HCenter {
        \Rect(width: 400, height: 100, color: #3b82f6)
    }
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "Rect")
        .expect("rect found");

    // Centered in 800 width: x = (800 - 400) / 2 = 200
    assert_eq!(rect.rect.x, 200.0);
    // Fixed to top: y = 0
    assert_eq!(rect.rect.y, 0.0);
    assert_eq!(rect.rect.width, 400.0);
    assert_eq!(rect.rect.height, 100.0);
}

#[test]
fn test_hcenter_explicit_parameters() {
    let input = r#"
    \use "components/HCenter.dt"

    \HCenter(width: 500, x: 50, y: 30) {
        \Rect(width: 300, height: 80, color: #3b82f6)
    }
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "Rect")
        .expect("rect found");

    // Centered in 500 width at x=50: 50 + (500 - 300) / 2 = 150
    assert_eq!(rect.rect.x, 150.0);
    assert_eq!(rect.rect.y, 30.0);
}

#[test]
fn test_hcenter_assertion_rejects_multiple_children() {
    let input = r#"
    \use "components/HCenter.dt"

    \HCenter {
        \Rect(width: 100, height: 50, color: #ff0000)
        \Rect(width: 100, height: 50, color: #00ff00)
    }
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let result = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    );

    match result {
        Err(CompileError::AssertionFailed { message, .. }) => {
            assert_eq!(message, "Should have only one child.");
        }
        other => panic!("expected CompileError::AssertionFailed, got {:?}", other),
    }
}

#[test]
fn test_vcenter_fluid_in_window() {
    let input = r#"
    \use "components/VCenter.dt"

    \VCenter {
        \Rect(width: 200, height: 100, color: #10b981)
    }
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "Rect")
        .expect("rect found");

    // Centered vertically in 600 height: y = (600 - 100) / 2 = 250
    assert_eq!(rect.rect.y, 250.0);
    // Fixed to left: x = 0
    assert_eq!(rect.rect.x, 0.0);
    assert_eq!(rect.rect.width, 200.0);
    assert_eq!(rect.rect.height, 100.0);
}

#[test]
fn test_vcenter_explicit_parameters() {
    let input = r#"
    \use "components/VCenter.dt"

    \VCenter(height: 400, x: 40, y: 50) {
        \Rect(width: 150, height: 100, color: #10b981)
    }
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "Rect")
        .expect("rect found");

    // Centered vertically in 400 height at y=50: 50 + (400 - 100) / 2 = 200
    assert_eq!(rect.rect.y, 200.0);
    assert_eq!(rect.rect.x, 40.0);
}

#[test]
fn test_vcenter_assertion_rejects_multiple_children() {
    let input = r#"
    \use "components/VCenter.dt"

    \VCenter {
        \Rect(width: 100, height: 50, color: #ff0000)
        \Rect(width: 100, height: 50, color: #00ff00)
    }
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let result = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    );

    match result {
        Err(CompileError::AssertionFailed { message, .. }) => {
            assert_eq!(message, "Should have only one child.");
        }
        other => panic!("expected CompileError::AssertionFailed, got {:?}", other),
    }
}

#[test]
fn test_vcenter_assertion_rejects_zero_children() {
    let input = r#"
    \use "components/VCenter.dt"

    \VCenter {}
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let result = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    );

    match result {
        Err(CompileError::AssertionFailed { message, .. }) => {
            assert_eq!(message, "Should have only one child.");
        }
        other => panic!("expected CompileError::AssertionFailed, got {:?}", other),
    }
}

#[test]
fn test_nesting_vcenter_and_hcenter() {
    let input = r#"
    \use "components/VCenter.dt"
    \use "components/HCenter.dt"

    \VCenter {
        \HCenter {
            \Rect(width: 300, height: 200, color: #a855f7)
        }
    }
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "Rect")
        .expect("rect found");

    // Centered both horizontally and vertically in 800x600:
    // x = (800 - 300) / 2 = 250
    // y = (600 - 200) / 2 = 200
    assert_eq!(rect.rect.x, 250.0);
    assert_eq!(rect.rect.y, 200.0);
    assert_eq!(rect.rect.width, 300.0);
    assert_eq!(rect.rect.height, 200.0);
}
