use directedtype::compiler::compiled::CompiledDocument;
use directedtype::component::ComponentRegistry;
use directedtype::parse;
use std::path::Path;

#[test]
fn test_hstack_default_top_alignment_and_gap() {
    let input = r#"
    \use "components/HStack.dt"

    \HStack(x: 10, y: 20, gap: 16, padding_x: 20, padding_y: 24) {
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

    // Top alignment (default): inner_top = y (20) + padding_y (24) = 44
    assert_eq!(rects[0].rect.y, 44.0);
    assert_eq!(rects[1].rect.y, 44.0);
    assert_eq!(rects[2].rect.y, 44.0);

    // Horizontal stacking with gap: 16
    // Item 1 x: x (10) + padding_x (20) = 30
    assert_eq!(rects[0].rect.x, 30.0);
    assert_eq!(rects[0].rect.width, 100.0);

    // Item 2 x: rects[0].right (30 + 100 = 130) + gap (16) = 146
    assert_eq!(rects[1].rect.x, 146.0);
    assert_eq!(rects[1].rect.width, 150.0);

    // Item 3 x: rects[1].right (146 + 150 = 296) + gap (16) = 312
    assert_eq!(rects[2].rect.x, 312.0);
    assert_eq!(rects[2].rect.width, 80.0);

    // HStack dimensions:
    // right: last rect right (312 + 80 = 392) + padding_x (20) = 412
    // bottom: max rect bottom (44 + 50 = 94) + padding_y (24) = 118
    let hstack_node = compiled.layout.nodes.iter().find(|n| n.name == "HStack").expect("HStack found");
    assert_eq!(hstack_node.rect.width, 402.0); // 412 - 10 (x)
    assert_eq!(hstack_node.rect.height, 98.0); // 118 - 20 (y)
}

#[test]
fn test_hstack_center_and_centered_alignment() {
    let input = r#"
    \use "components/HStack.dt"

    \HStack(x: 0, y: 10, align: "center", gap: 10, padding_x: 0, padding_y: 10) {
        \Rect(width: 100, height: 60, color: #3b82f6)
        \Rect(width: 120, height: 40, color: #10b981)
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

    // max_h = 60
    // inner_top = 10 + 10 = 20
    // rect 0 (h=60): 20 + (60 - 60) / 2 = 20
    assert_eq!(rects[0].rect.y, 20.0);
    // rect 1 (h=40): 20 + (60 - 40) / 2 = 30
    assert_eq!(rects[1].rect.y, 30.0);
}

#[test]
fn test_hstack_bottom_alignment() {
    let input = r#"
    \use "components/HStack.dt"

    \HStack(x: 0, y: 10, align: "bottom", gap: 10, padding_y: 15) {
        \Rect(width: 80, height: 70, color: #ef4444)
        \Rect(width: 80, height: 30, color: #3b82f6)
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

    // max_h = 70
    // inner_top = 10 + 15 = 25
    // rect 0 (h=70): bottom edge = 25 + 70 = 95
    assert_eq!(rects[0].rect.y + rects[0].rect.height, 95.0);
    // rect 1 (h=30): bottom edge = 25 + 70 = 95 -> y = 65
    assert_eq!(rects[1].rect.y, 65.0);
    assert_eq!(rects[1].rect.y + rects[1].rect.height, 95.0);
}

#[test]
fn test_hstack_with_button_and_card_composition() {
    let input = r#"
    \use "components/Card.dt"
    \use "components/HStack.dt"
    \use "components/Button.dt"

    \Card(padding_x: 20, padding_y: 16) {
        \HStack(gap: 12) {
            \Button(label: "First", variant: "primary")
            \Button(label: "Second", variant: "secondary")
            \Button(label: "Third", variant: "outline")
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

    let buttons: Vec<_> = compiled.layout.nodes.iter().filter(|n| n.name == "Button").collect();
    assert_eq!(buttons.len(), 3);

    let btn0_rect = compiled.layout.nodes.iter().find(|n| n.name == "Rect" && n.parent == Some(buttons[0].id)).expect("btn 0 rect");
    let btn1_rect = compiled.layout.nodes.iter().find(|n| n.name == "Rect" && n.parent == Some(buttons[1].id)).expect("btn 1 rect");

    // Button 0 starts at card padding: 20
    assert_eq!(buttons[0].rect.x, 20.0);
    // Button 1 starts after button 0 rect width (64) + 12 = 96
    assert_eq!(buttons[1].rect.x, buttons[0].rect.x + btn0_rect.rect.width + 12.0);
    // Button 2 starts after button 1 rect width + 12
    assert_eq!(buttons[2].rect.x, buttons[1].rect.x + btn1_rect.rect.width + 12.0);
}

#[test]
fn test_hstack_nested_in_vstack() {
    let input = r#"
    \use "components/VStack.dt"
    \use "components/HStack.dt"

    \VStack(x: 10, y: 10, width: 300, gap: 20) {
        \HStack(gap: 15) {
            \Rect(width: 60, height: 30, color: #3b82f6)
            \Rect(width: 80, height: 30, color: #10b981)
        }
        \Rect(width: 200, height: 50, color: #f59e0b)
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

    // HStack Rect 0
    assert_eq!(rects[0].rect.x, 10.0);
    assert_eq!(rects[0].rect.y, 10.0);
    assert_eq!(rects[0].rect.width, 60.0);

    // HStack Rect 1 (x: 10 + 60 + 15 = 85)
    assert_eq!(rects[1].rect.x, 85.0);
    assert_eq!(rects[1].rect.y, 10.0);
    assert_eq!(rects[1].rect.width, 80.0);

    // HStack dimensions:
    let hstack_node = compiled.layout.nodes.iter().find(|n| n.name == "HStack").expect("HStack found");
    assert_eq!(hstack_node.rect.width, 155.0); // 165 - 10
    assert_eq!(hstack_node.rect.height, 30.0);

    // Sibling Rect 2 in VStack: placed after HStack bottom (40) + gap (20) = 60
    assert_eq!(rects[2].rect.y, 60.0);
    assert_eq!(rects[2].rect.height, 50.0);
}

