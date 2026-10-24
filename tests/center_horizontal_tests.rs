use directedtype::compiler::compiled::CompiledDocument;
use directedtype::component::ComponentRegistry;
use directedtype::parse;
use std::path::Path;

#[test]
fn test_center_horizontal_fluid_in_window() {
    let input = r#"
    \use "components/HCenter.dt"

    \HCenter {
        \Rect(width: 400px, height: 100px, color: #3b82f6)
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
fn test_center_horizontal_inside_padding() {
    let input = r#"
    \use "components/PadIn.dt"
    \use "components/PadOut.dt"
    \use "components/HCenterUp.dt"

    \PadIn(padding_x: 40px) {
        \HCenterUp {
            \PadOut(padding_y: 50px) {
                \Rect(width: 480px, height: 200px, color: #3b82f6)
            }
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

    // PadIn inner_width = 800 - 80 = 720
    // HCenter centers: 40 + (720 - 480) / 2 = 40 + 120 = 160
    assert_eq!(rect.rect.x, 160.0);
    // Fixed to top padding: y = 50
    assert_eq!(rect.rect.y, 50.0);

    let padout_node = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "PadOut")
        .expect("padout node found");

    // Height = rect.height (200) + 2 * padding_y (100) = 300
    assert_eq!(padout_node.rect.height, 300.0);
    assert_eq!(padout_node.rect.y, 0.0);
    // Bottom = 300 (rect bottom 250 + 50 bottom padding)
    assert_eq!(padout_node.rect.y + padout_node.rect.height, 300.0);
}

#[test]
fn test_center_horizontal_clamps_when_narrower_than_child() {
    let input = r#"
    \use "components/PadOut.dt"
    \use "components/HCenter.dt"

    \HCenter {
        \PadOut(padding_x: 40px, padding_y: 40px) {
            \Rect(width: 480px, height: 100px, color: #3b82f6)
        }
    }
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    // Window width is 400 (smaller than 480 + 80 = 560)
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        400.0,
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

    // Left padding must not squish down into negative/zero: clamped to parent.left (40.0)
    assert_eq!(rect.rect.x, 40.0);
    assert_eq!(rect.rect.y, 40.0);
}

#[test]
fn test_center_horizontal_multiple_children_fails_assertion() {
    let input = r#"
    \use "components/HCenter.dt"

    \HCenter {
        \Rect(width: 100px, height: 50px, color: #ff0000)
        \Rect(width: 100px, height: 50px, color: #00ff00)
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
        Err(directedtype::compiler::error::CompileError::AssertionFailed { message, .. }) => {
            assert_eq!(message, "Should have only one child.");
        }
        other => panic!("expected CompileError::AssertionFailed, got {:?}", other),
    }
}

#[test]
fn test_center_horizontal_zero_children_fails_assertion() {
    let input = r#"
    \use "components/HCenter.dt"

    \HCenter {}
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
        Err(directedtype::compiler::error::CompileError::AssertionFailed { message, .. }) => {
            assert_eq!(message, "Should have only one child.");
        }
        other => panic!("expected CompileError::AssertionFailed, got {:?}", other),
    }
}

#[test]
fn test_assert_directive_in_custom_component() {
    let input = r#"
    \Component MustBeSquare(width: Distance, height: Distance) {
        \assert width == height as "Must be a square!";
        \Children
    }

    \MustBeSquare(width: 100px, height: 100px) {
        \Rect(x: 0, y: 0, width: 100px, height: 100px, color: #3b82f6)
    }
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    ).expect("compilation of valid MustBeSquare should succeed");

    let fail_input = r#"
    \Component MustBeSquare(width: Distance, height: Distance) {
        \assert width == height as "Must be a square!";
        \Children
    }

    \MustBeSquare(width: 100px, height: 200px) {
        \Rect(x: 0, y: 0, width: 100px, height: 200px, color: #3b82f6)
    }
    "#;

    let fail_doc = parse(fail_input).expect("parse ok");
    let result = CompiledDocument::compile_with_registry(
        &fail_doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    );

    match result {
        Err(directedtype::compiler::error::CompileError::AssertionFailed { message, .. }) => {
            assert_eq!(message, "Must be a square!");
        }
        other => panic!("expected CompileError::AssertionFailed, got {:?}", other),
    }
}

