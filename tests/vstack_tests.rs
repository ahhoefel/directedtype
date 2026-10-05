use directedtype::compiler::compiled::CompiledDocument;
use directedtype::component::ComponentRegistry;
use directedtype::parse;
use std::path::Path;

#[test]
fn test_vstack_default_left_alignment_and_gap() {
    let input = r#"
    \use "components/VStack.dt"

    \VStack(x: 10, y: 20, width: 300, gap: 16) {
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

    // Left alignment: inner_left = x (10)
    assert_eq!(rects[0].rect.x, 10.0);
    assert_eq!(rects[1].rect.x, 10.0);
    assert_eq!(rects[2].rect.x, 10.0);

    // Vertical stacking with gap: 16
    // Item 1 y: y (20)
    assert_eq!(rects[0].rect.y, 20.0);
    assert_eq!(rects[0].rect.height, 40.0);

    // Item 2 y: rects[0].bottom (20 + 40 = 60) + gap (16) = 76
    assert_eq!(rects[1].rect.y, 76.0);
    assert_eq!(rects[1].rect.height, 50.0);

    // Item 3 y: rects[1].bottom (76 + 50 = 126) + gap (16) = 142
    assert_eq!(rects[2].rect.y, 142.0);
    assert_eq!(rects[2].rect.height, 30.0);

    // VStack total bottom alias: last rect bottom (142 + 30 = 172)
    let vstack_node = compiled.layout.nodes.iter().find(|n| n.name == "VStack").expect("VStack found");
    assert_eq!(vstack_node.rect.height, 152.0); // 172 - 20 (y)
}

#[test]
fn test_vstack_center_and_centered_alignment() {
    let input = r#"
    \use "components/VStack.dt"

    \VStack(x: 0, y: 0, width: 400, align: "center", gap: 10) {
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

    \VStack(x: 50, y: 0, width: 400, align: "right") {
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
    // resolved_width = 400, inner_left = 50
    // Right aligned: inner_left (50) + inner_width (400) - child.width (120) = 330
    assert_eq!(rect.rect.x, 330.0);
    // Right edge: 330 + 120 = 450 (which is 50 + 400)
    assert_eq!(rect.rect.x + rect.rect.width, 450.0);
}

#[test]
fn test_vstack_stretch_alignment() {
    let input = r#"
    \use "components/VStack.dt"

    \VStack(x: 0, y: 0, width: 500, align: "stretch") {
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

    // Stretched child without explicit width: width = 500
    assert_eq!(rects[0].rect.x, 0.0);
    assert_eq!(rects[0].rect.width, 500.0);

    // Child with explicit width override: Tier 4 overrides ambient rule!
    assert_eq!(rects[1].rect.x, 0.0);
    assert_eq!(rects[1].rect.width, 200.0);
}

#[test]
fn test_vstack_with_button_and_text_composition() {
    let input = r#"
    \use "components/VStack.dt"
    \use "components/Button.dt"

    \VStack(x: 20, y: 20, width: 350, align: "stretch", gap: 14) {
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
    assert_eq!(text_node.rect.x, 20.0);

    let btn_node = compiled.layout.nodes.iter().find(|n| n.name == "Button").expect("Button found");
    assert_eq!(btn_node.rect.x, 20.0);
    // Stretched button: width = 350
    let btn_rect = compiled.layout.nodes.iter().find(|n| n.name == "Rect" && n.parent == Some(btn_node.id)).expect("Button Rect found");
    assert_eq!(btn_rect.rect.width, 350.0);
}

#[test]
fn test_vstack_default_zero_gap() {
    let input = r#"
    \use "components/VStack.dt"

    \VStack(x: 10, y: 20, width: 300) {
        \Rect(width: 100, height: 40, color: #3b82f6)
        \Rect(width: 150, height: 50, color: #10b981)
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

    // Default gap is 0: rects[1].top == rects[0].bottom (20 + 40 = 60)
    assert_eq!(rects[0].rect.y, 20.0);
    assert_eq!(rects[1].rect.y, 60.0);
}
