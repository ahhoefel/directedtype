use directedtype::compiler::compiled::CompiledDocument;
use directedtype::component::ComponentRegistry;
use directedtype::parse;
use std::path::Path;

#[test]
fn test_vertical_spacer_in_vstack() {
    let input = r#"
    \use "components/VStack.dt"
    \use "components/Spacer.dt"

    \VStack(x: 20, y: 20, width: 200, gap: 10) {
        \Rect(width: 100, height: 40, color: #3b82f6)
        \Spacer(height: 35)
        \Rect(width: 100, height: 40, color: #10b981)
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

    // Rect 1: y = 20, height = 40 -> bottom = 60
    assert_eq!(rects[0].rect.y, 20.0);
    assert_eq!(rects[0].rect.height, 40.0);

    // Spacer: y = 60 + gap(10) = 70, height = 35 -> bottom = 105
    // Rect 2: y = 105 + gap(10) = 115
    assert_eq!(rects[1].rect.y, 115.0);
    assert_eq!(rects[1].rect.height, 40.0);
}

#[test]
fn test_horizontal_spacer_in_hstack() {
    let input = r#"
    \use "components/HStack.dt"
    \use "components/Spacer.dt"

    \HStack(x: 10, y: 10, gap: 12) {
        \Rect(width: 50, height: 30, color: #3b82f6)
        \Spacer(width: 48)
        \Rect(width: 50, height: 30, color: #10b981)
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

    // Rect 1: x = 10, width = 50 -> right = 60
    assert_eq!(rects[0].rect.x, 10.0);
    assert_eq!(rects[0].rect.width, 50.0);

    // Spacer: x = 60 + gap(12) = 72, width = 48 -> right = 120
    // Rect 2: x = 120 + gap(12) = 132
    assert_eq!(rects[1].rect.x, 132.0);
    assert_eq!(rects[1].rect.width, 50.0);
}

#[test]
fn test_divider_in_vstack() {
    let input = r#"
    \use "components/VStack.dt"
    \use "components/Divider.dt"

    \VStack(x: 0, y: 0, width: 300, gap: 8) {
        \Rect(width: 100, height: 20, color: #3b82f6)
        \Divider()
        \Rect(width: 100, height: 20, color: #10b981)
        \Divider(width: 120, color: #ef4444, thickness: 2)
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
    // 2 content rects + 2 divider rects = 4 rects
    assert_eq!(rects.len(), 4);

    // Content Rect 1
    assert_eq!(rects[0].rect.y, 0.0);
    assert_eq!(rects[0].rect.height, 20.0);

    // Fluid Divider 1: width matches parent (300), height = 1
    assert_eq!(rects[1].rect.y, 28.0); // 20 + 8
    assert_eq!(rects[1].rect.height, 1.0);
    assert_eq!(rects[1].rect.width, 300.0);

    // Content Rect 2
    assert_eq!(rects[2].rect.y, 37.0); // 29 + 8

    // Fixed Divider 2: width = 120, thickness = 2
    assert_eq!(rects[3].rect.y, 65.0); // 37 + 20 + 8
    assert_eq!(rects[3].rect.height, 2.0);
    assert_eq!(rects[3].rect.width, 120.0);
}
