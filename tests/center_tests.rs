use directedtype::compiler::compiled::CompiledDocument;
use directedtype::component::ComponentRegistry;
use directedtype::parse;
use std::path::Path;

#[test]
fn test_center_fluid_in_rect() {
    let input = r#"
    \use "components/Center.dt"

    \Rect(x: 100, y: 100, width: 200, height: 100, color: #1e293b) {
        \Center {
            \Rect(width: 80, height: 40, color: #3b82f6)
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

    let inner_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "Rect" && n.rect.width == 80.0)
        .expect("inner rect found");

    assert_eq!(inner_rect.rect.x, 160.0); // 100 + (200 - 80) / 2
    assert_eq!(inner_rect.rect.y, 130.0); // 100 + (100 - 40) / 2
}

#[test]
fn test_center_with_intrinsic_text() {
    let input = r#"
    \use "components/Center.dt"

    \Rect(x: 50, y: 50, width: 200, height: 80, color: #1e293b) {
        \Center {
            \Text(size: 20, weight: 700) {
                Centered
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

    let text_node = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "Text")
        .expect("text node found");

    // Text width is calculated by Parley/font metrics
    assert!(text_node.rect.width > 0.0);
    let expected_x = 50.0 + (200.0 - text_node.rect.width) / 2.0;
    let expected_y = 50.0 + (80.0 - text_node.rect.height) / 2.0;

    assert_eq!(text_node.rect.x, expected_x);
    assert_eq!(text_node.rect.y, expected_y);
}

#[test]
fn test_center_explicit_spatial_bounds() {
    let input = r#"
    \use "components/Center.dt"

    \Center(x: 50, y: 50, width: 300, height: 200) {
        \Rect(width: 100, height: 60, color: #3b82f6)
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

    let rect_node = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "Rect")
        .expect("rect found");

    assert_eq!(rect_node.rect.x, 150.0); // 50 + (300 - 100) / 2
    assert_eq!(rect_node.rect.y, 120.0); // 50 + (200 - 60) / 2
}

#[test]
fn test_center_explicit_dimensions_in_parent() {
    let input = r#"
    \use "components/Center.dt"

    \Rect(x: 20, y: 40, width: 500, height: 400, color: #0f172a) {
        \Center(width: 200, height: 100) {
            \Rect(width: 50, height: 30, color: #3b82f6)
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

    let inner_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "Rect" && n.rect.width == 50.0)
        .expect("inner rect found");

    assert_eq!(inner_rect.rect.x, 95.0); // 20 + (200 - 50) / 2
    assert_eq!(inner_rect.rect.y, 75.0); // 40 + (100 - 30) / 2
}
