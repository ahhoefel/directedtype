use directedtype::compiler::compiled::CompiledDocument;
use directedtype::component::ComponentRegistry;
use directedtype::parse;
use std::path::Path;

#[test]
fn test_center_fluid_in_rect() {
    let input = r#"
    \use "components/Center.dt"

    \Rect(x: 100px, y: 100px, width: 200px, height: 100px, color: #1e293b) {
        \Center {
            \Rect(width: 80px, height: 40px, color: #3b82f6)
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

    \Rect(x: 50px, y: 50px, width: 200px, height: 80px, color: #1e293b) {
        \Center {
            \Text(font: \Font(size: 20px, weight: 700)) {
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

    \Center(x: 50px, y: 50px, width: 300px, height: 200px) {
        \Rect(width: 100px, height: 60px, color: #3b82f6)
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

    \Rect(x: 20px, y: 40px, width: 500px, height: 400px, color: #0f172a) {
        \Center(width: 200px, height: 100px) {
            \Rect(width: 50px, height: 30px, color: #3b82f6)
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

#[test]
fn test_center_with_card() {
    let input = r#"
    \use "components/Center.dt"
    \use "components/Card.dt"
    \use "theme/default.dt"

    \Center {
        \Card(width: 400px, style: card_dark) {
            \Text { Hello }
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
    .expect("compile ok: Card inside Center must not produce cyclic dependency");

    let card_node = compiled.layout.nodes.iter().find(|n| n.name == "Card").expect("Card found");
    assert_eq!(card_node.rect.width, 400.0);
    // Card centered in 800x600:
    // x = (800 - 400) / 2 = 200
    assert_eq!(card_node.rect.x, 200.0);
    assert_eq!(card_node.rect.y, (600.0 - card_node.rect.height) / 2.0);
}

#[test]
fn test_center_clamps_when_child_exceeds_parent() {
    let input = r#"
    \use "components/Center.dt"

    \Center {
        \Rect(width: 500px, height: 800px, color: #3b82f6)
    }
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        300.0,
        400.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let center_node = compiled.layout.nodes.iter().find(|n| n.name == "Center").expect("Center found");
    let rect_node = compiled.layout.nodes.iter().find(|n| n.name == "Rect").expect("Rect found");

    assert_eq!(center_node.rect.width, 500.0);
    assert_eq!(center_node.rect.height, 800.0);
    assert_eq!(rect_node.rect.x, 0.0);
    assert_eq!(rect_node.rect.y, 0.0);
}

#[test]
fn test_center_no_clamp_allows_negative_offset() {
    let input = r#"
    \use "components/CenterNoClamp.dt"

    \CenterNoClamp {
        \Rect(width: 500px, height: 800px, color: #3b82f6)
    }
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        300.0,
        400.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let rect_node = compiled.layout.nodes.iter().find(|n| n.name == "Rect").expect("Rect found");

    // (300 - 500) / 2 = -100, (400 - 800) / 2 = -200
    assert_eq!(rect_node.rect.x, -100.0);
    assert_eq!(rect_node.rect.y, -200.0);
}


