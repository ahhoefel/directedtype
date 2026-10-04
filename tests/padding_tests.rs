use directedtype::compiler::compiled::CompiledDocument;
use directedtype::component::ComponentRegistry;
use directedtype::parse;
use std::path::Path;

#[test]
fn test_fluid_uniform_padding() {
    let input = r#"
    \use "components/Padding.dt"

    \Padding(padding: 20, x: 10, y: 15) {
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

    let rect = compiled.layout.nodes.iter().find(|n| n.name == "Rect").expect("Rect found");
    assert_eq!(rect.rect.x, 30.0); // 10 + 20
    assert_eq!(rect.rect.y, 35.0); // 15 + 20
    assert_eq!(rect.rect.width, 100.0);
    assert_eq!(rect.rect.height, 60.0);

    let padding_node = compiled.layout.nodes.iter().find(|n| n.name == "Padding").expect("Padding found");
    // Fluid width: parent.width (800) - 2 * x (20) = 780
    assert_eq!(padding_node.rect.width, 780.0);
    // Height wraps children + padding: 60 + 2 * 20 = 100
    assert_eq!(padding_node.rect.height, 100.0);
}

#[test]
fn test_explicit_width_uniform_padding() {
    let input = r#"
    \use "components/Padding.dt"

    \Padding(width: 140, padding: 20, x: 10, y: 15) {
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

    let rect = compiled.layout.nodes.iter().find(|n| n.name == "Rect").expect("Rect found");
    assert_eq!(rect.rect.x, 30.0);
    assert_eq!(rect.rect.y, 35.0);

    let padding_node = compiled.layout.nodes.iter().find(|n| n.name == "Padding").expect("Padding found");
    assert_eq!(padding_node.rect.width, 140.0);
    assert_eq!(padding_node.rect.height, 100.0);
}

#[test]
fn test_fluid_asymmetric_padding_wrapping_hstack() {
    let input = r#"
    \use "components/Padding.dt"
    \use "components/HStack.dt"

    \Padding(padding_x: 20, padding_y: 24, x: 10, y: 20) {
        \HStack(gap: 16) {
            \Rect(width: 100, height: 40, color: #3b82f6)
            \Rect(width: 150, height: 50, color: #10b981)
            \Rect(width: 80, height: 30, color: #f59e0b)
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

    let rects: Vec<_> = compiled.layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 3);

    // Inset by padding_x (20) and padding_y (24):
    assert_eq!(rects[0].rect.x, 30.0);
    assert_eq!(rects[0].rect.y, 44.0);

    // rect 1: 30 + 100 + 16 = 146
    assert_eq!(rects[1].rect.x, 146.0);
    assert_eq!(rects[1].rect.y, 44.0);

    // rect 2: 146 + 150 + 16 = 312
    assert_eq!(rects[2].rect.x, 312.0);
    assert_eq!(rects[2].rect.y, 44.0);

    let padding_node = compiled.layout.nodes.iter().find(|n| n.name == "Padding").expect("Padding found");
    // Fluid width: parent.width (800) - 2 * 10 = 780
    assert_eq!(padding_node.rect.width, 780.0);
    // HStack bottom is 44 + 50 = 94. Padding bottom is 94 + 24 = 118.
    // Padding height is 118 - 20 = 98.
    assert_eq!(padding_node.rect.height, 98.0);
}

#[test]
fn test_explicit_width_padding_wrapping_vstack() {
    let input = r#"
    \use "components/Padding.dt"
    \use "components/VStack.dt"

    \Padding(width: 300, padding: 20, x: 10, y: 20) {
        \VStack(gap: 16) {
            \Rect(width: 100, height: 40, color: #3b82f6)
            \Rect(width: 150, height: 50, color: #10b981)
            \Rect(width: 80, height: 30, color: #f59e0b)
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

    let rects: Vec<_> = compiled.layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 3);

    assert_eq!(rects[0].rect.x, 30.0);
    assert_eq!(rects[0].rect.y, 40.0);

    // rect 1: 40 + 40 + 16 = 96
    assert_eq!(rects[1].rect.x, 30.0);
    assert_eq!(rects[1].rect.y, 96.0);

    // rect 2: 96 + 50 + 16 = 162
    assert_eq!(rects[2].rect.x, 30.0);
    assert_eq!(rects[2].rect.y, 162.0);

    let padding_node = compiled.layout.nodes.iter().find(|n| n.name == "Padding").expect("Padding found");
    assert_eq!(padding_node.rect.width, 300.0);
    // VStack bottom is 162 + 30 = 192. Padding bottom is 192 + 20 = 212.
    // Padding height is 212 - 20 = 192.
    assert_eq!(padding_node.rect.height, 192.0);
}

#[test]
fn test_directional_4_sided_padding() {
    let input = r#"
    \use "components/Padding.dt"

    \Padding(width: 145, padding_top: 10, padding_right: 30, padding_bottom: 20, padding_left: 15, x: 50, y: 50) {
        \Rect(width: 100, height: 50, color: #ef4444)
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

    let rect = compiled.layout.nodes.iter().find(|n| n.name == "Rect").expect("Rect found");
    assert_eq!(rect.rect.x, 65.0); // 50 + 15
    assert_eq!(rect.rect.y, 60.0); // 50 + 10

    let padding_node = compiled.layout.nodes.iter().find(|n| n.name == "Padding").expect("Padding found");
    assert_eq!(padding_node.rect.width, 145.0); // 100 + 15 + 30
    assert_eq!(padding_node.rect.height, 80.0); // 50 + 10 + 20
}

#[test]
fn test_stretch_alignment_inside_explicit_width_padding() {
    let input = r#"
    \use "components/Padding.dt"
    \use "components/VStack.dt"

    \Padding(width: 400, padding_x: 25, padding_y: 20, x: 0, y: 0) {
        \VStack(align: "stretch", gap: 10) {
            \Rect(height: 40, color: #8b5cf6)
            \Rect(width: 150, height: 40, color: #ec4899)
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

    let rects: Vec<_> = compiled.layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 2);

    // Inner width = 400 - 2 * 25 = 350
    assert_eq!(rects[0].rect.x, 25.0);
    assert_eq!(rects[0].rect.width, 350.0);

    // Explicit override
    assert_eq!(rects[1].rect.x, 25.0);
    assert_eq!(rects[1].rect.width, 150.0);
}
