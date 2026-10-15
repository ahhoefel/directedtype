use directedtype::compiler::compiled::CompiledDocument;
use directedtype::component::ComponentRegistry;
use directedtype::parse;
use std::path::Path;

#[test]
fn test_center_horizontal_fluid_in_window() {
    let input = r#"
    \use "components/CenterHorizontal.dt"

    \CenterHorizontal {
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
fn test_center_horizontal_inside_padding() {
    let input = r#"
    \use "components/Padding.dt"
    \use "components/CenterHorizontal.dt"

    \Padding(padding_x: 40, padding_y: 50, x: 0, y: 0) {
        \CenterHorizontal {
            \Rect(width: 480, height: 200, color: #3b82f6)
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

    // Padding inner_width = 800 - 80 = 720
    // CenterHorizontal centers: 40 + (720 - 480) / 2 = 40 + 120 = 160
    assert_eq!(rect.rect.x, 160.0);
    // Fixed to top padding: y = 50
    assert_eq!(rect.rect.y, 50.0);

    let padding_node = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "Padding")
        .expect("padding node found");

    // Height = rect.height (200) + 2 * padding_y (100) = 300
    assert_eq!(padding_node.rect.height, 300.0);
    assert_eq!(padding_node.rect.y, 0.0);
    // Bottom = 300 (rect bottom 250 + 50 bottom padding)
    assert_eq!(padding_node.rect.y + padding_node.rect.height, 300.0);
}

#[test]
fn test_center_horizontal_clamps_when_narrower_than_child() {
    let input = r#"
    \use "components/Padding.dt"
    \use "components/CenterHorizontal.dt"

    \Padding(padding_x: 40, padding_y: 40, x: 0, y: 0) {
        \CenterHorizontal {
            \Rect(width: 480, height: 100, color: #3b82f6)
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
    \use "components/CenterHorizontal.dt"

    \CenterHorizontal {
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
        Err(directedtype::compiler::error::CompileError::AssertionFailed { message, .. }) => {
            assert_eq!(message, "Should have only one child.");
        }
        other => panic!("expected CompileError::AssertionFailed, got {:?}", other),
    }
}

#[test]
fn test_center_horizontal_zero_children_fails_assertion() {
    let input = r#"
    \use "components/CenterHorizontal.dt"

    \CenterHorizontal {}
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
    \Component MustBeSquare(width: Number, height: Number) {
        \assert width == height as "Must be a square!";
        \Children
    }

    \MustBeSquare(width: 100, height: 100) {
        \Rect(x: 0, y: 0, width: 100, height: 100, color: #3b82f6)
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
    \Component MustBeSquare(width: Number, height: Number) {
        \assert width == height as "Must be a square!";
        \Children
    }

    \MustBeSquare(width: 100, height: 200) {
        \Rect(x: 0, y: 0, width: 100, height: 200, color: #3b82f6)
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

