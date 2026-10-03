use directedtype::compiler::compiled::CompiledDocument;
use directedtype::component::ComponentRegistry;
use directedtype::parse;
use std::path::Path;

#[test]
fn test_vstack_default_left_alignment_and_gap() {
    let input = r#"
    \use "components/VStack.dt"

    \VStack(x: 10, y: 20, width: 300, gap: 16, padding_x: 20, padding_y: 24) {
        \Rect(width: 100, height: 40, color: #3b82f6)
        \Rect(width: 150, height: 50, color: #10b981)
        \Rect(width: 80, height: 30, color: #f59e0b)
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

    // Left alignment: inner_left = x (10) + padding_x (20) = 30
    assert_eq!(rects[0].rect.x, 30.0);
    assert_eq!(rects[1].rect.x, 30.0);
    assert_eq!(rects[2].rect.x, 30.0);

    // Vertical stacking with gap: 16
    // Item 1 y: y (20) + padding_y (24) = 44
    assert_eq!(rects[0].rect.y, 44.0);
    assert_eq!(rects[0].rect.height, 40.0);

    // Item 2 y: rects[0].bottom (44 + 40 = 84) + gap (16) = 100
    assert_eq!(rects[1].rect.y, 100.0);
    assert_eq!(rects[1].rect.height, 50.0);

    // Item 3 y: rects[1].bottom (100 + 50 = 150) + gap (16) = 166
    assert_eq!(rects[2].rect.y, 166.0);
    assert_eq!(rects[2].rect.height, 30.0);

    // VStack total bottom alias: last rect bottom (166 + 30 = 196) + padding_y (24) = 220
    let vstack_node = compiled.layout.nodes.iter().find(|n| n.name == "VStack").expect("VStack found");
    assert_eq!(vstack_node.rect.height, 200.0); // 220 - 20 (y)
}

#[test]
fn test_vstack_center_and_centered_alignment() {
    let input = r#"
    \use "components/VStack.dt"

    \VStack(x: 0, y: 0, width: 400, align: "center", gap: 10, padding_x: 0, padding_y: 10) {
        \Rect(width: 100, height: 40, color: #3b82f6)
        \Rect(width: 250, height: 50, color: #10b981)
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

    // Centered: (400 - child.width) / 2
    assert_eq!(rects[0].rect.x, 150.0); // (400 - 100) / 2
    assert_eq!(rects[1].rect.x, 75.0);  // (400 - 250) / 2

    // Also test "centered" spelling
    let input2 = r#"
    \use "components/VStack.dt"

    \VStack(x: 0, y: 0, width: 400, align: "centered") {
        \Rect(width: 120, height: 40, color: #3b82f6)
    }
    "#;
    let doc2 = parse(input2).expect("parse ok");
    let compiled2 = CompiledDocument::compile_with_registry(
        &doc2,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");
    let rect2 = compiled2.layout.nodes.iter().find(|n| n.name == "Rect").expect("Rect found");
    assert_eq!(rect2.rect.x, 140.0); // (400 - 120) / 2
}

#[test]
fn test_vstack_right_alignment() {
    let input = r#"
    \use "components/VStack.dt"

    \VStack(x: 50, y: 0, width: 400, align: "right", padding_x: 20) {
        \Rect(width: 120, height: 40, color: #ef4444)
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
    // resolved_width = 400, padding_x = 20, inner_width = 360, inner_left = 70
    // Right aligned: inner_left (70) + inner_width (360) - child.width (120) = 310
    assert_eq!(rect.rect.x, 310.0);
    // Right edge: 310 + 120 = 430 (which is 50 + 400 - 20)
    assert_eq!(rect.rect.x + rect.rect.width, 430.0);
}

#[test]
fn test_vstack_stretch_alignment() {
    let input = r#"
    \use "components/VStack.dt"

    \VStack(x: 0, y: 0, width: 500, align: "stretch", padding_x: 25) {
        \Rect(height: 50, color: #8b5cf6)
        \Rect(width: 200, height: 40, color: #ec4899)
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

    // Stretched child without explicit width: width = 500 - 2 * 25 = 450
    assert_eq!(rects[0].rect.x, 25.0);
    assert_eq!(rects[0].rect.width, 450.0);

    // Child with explicit width override: Tier 4 overrides ambient rule!
    assert_eq!(rects[1].rect.x, 25.0);
    assert_eq!(rects[1].rect.width, 200.0);
}

#[test]
fn test_vstack_with_button_and_text_composition() {
    let input = r#"
    \use "components/VStack.dt"
    \use "components/Button.dt"

    \VStack(x: 20, y: 20, width: 350, align: "stretch", gap: 14, padding_x: 16, padding_y: 16) {
        \Text(size: 20, weight: 700) { Header Title }
        \Button(label: "Submit Order", variant: "primary")
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

    let text_node = compiled.layout.nodes.iter().find(|n| n.name == "Text" && n.text_content.as_deref() == Some("Header Title")).expect("Text found");
    assert_eq!(text_node.rect.x, 36.0); // 20 + 16

    let btn_node = compiled.layout.nodes.iter().find(|n| n.name == "Button").expect("Button found");
    assert_eq!(btn_node.rect.x, 36.0); // 20 + 16
    // Stretched button: width = 350 - 2 * 16 = 318
    let btn_rect = compiled.layout.nodes.iter().find(|n| n.name == "Rect" && n.parent == Some(btn_node.id)).expect("Button Rect found");
    assert_eq!(btn_rect.rect.width, 318.0);
}
