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

    \VStack(x: 0, y: 0, width: 400, align: Align.Center, gap: 10) {
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
}

#[test]
fn test_vstack_right_alignment() {
    let input = r#"
    \use "components/VStack.dt"

    \VStack(x: 50, y: 0, width: 400, align: Align.Right) {
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

    \VStack(x: 0, y: 0, width: 500, align: Align.Stretch) {
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
    \use "theme/default.dt"

    \VStack(x: 20, y: 20, width: 350, align: Align.Stretch, gap: 14) {
        \Text(size: 20, weight: 700) { Header Title }
        \Button(style: button_primary, label: "Submit Order")
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

#[test]
fn test_vstack_standalone_without_width_rejected() {
    let input = r#"
    \use "components/VStack.dt"

    \VStack(gap: 12) {
        \Rect(width: 80, height: 30, color: #3b82f6)
        \Rect(width: 90, height: 40, color: #10b981)
    }
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let err = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect_err("Standalone VStack without width or shrink must fail compilation");

    match err {
        directedtype::compiler::CompileError::NoMatchingOverload(details) => {
            assert_eq!(details.name, "VStack");
            assert!(details.provided_ports.contains(&"gap".to_string()));
        }
        _ => panic!("Expected NoMatchingOverload for VStack, got {:?}", err),
    }
}

#[test]
fn test_vstack_shrink_wrap_width() {
    let input = r#"
    \use "components/VStack.dt"

    \VStack(shrink: true, x: 20, y: 30, gap: 10, align: Align.Center) {
        \Rect(width: 140, height: 40, color: #3b82f6)
        \Rect(width: 200, height: 50, color: #10b981)
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

    let vstack_node = compiled.layout.nodes.iter().find(|n| n.name == "VStack").expect("VStack found");
    // Width shrink-wraps to widest child (200)
    assert_eq!(vstack_node.rect.width, 200.0);
    assert_eq!(vstack_node.rect.x, 20.0);
    assert_eq!(vstack_node.rect.y, 30.0);

    let rects: Vec<_> = compiled.layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 3);

    // Centered within the shrink-wrapped 200 width:
    // Rect 1: width 140, centered -> 20 + (200 - 140) / 2 = 50
    assert_eq!(rects[0].rect.x, 50.0);
    // Rect 2: width 200, centered -> 20 + 0 = 20
    assert_eq!(rects[1].rect.x, 20.0);
    // Rect 3: width 80, centered -> 20 + (200 - 80) / 2 = 80
    assert_eq!(rects[2].rect.x, 80.0);
}

#[test]
fn test_vstack_fixed_height() {
    let input = r#"
    \use "components/VStack.dt"

    \VStack(height: 250, width: 180, x: 10, y: 15) {
        \Rect(width: 100, height: 40, color: #3b82f6)
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

    let vstack_node = compiled.layout.nodes.iter().find(|n| n.name == "VStack").expect("VStack found");
    assert_eq!(vstack_node.rect.width, 180.0);
    assert_eq!(vstack_node.rect.height, 250.0);
}

#[test]
fn test_vstack_inside_center_shrink_wrap_no_cycle() {
    let input = r#"
    \use "components/Center.dt"
    \use "components/VStack.dt"

    \Center {
        \VStack(shrink: true, gap: 10) {
            \Rect(width: 200, height: 50, color: #ef4444)
            \Rect(width: 150, height: 60, color: #22c55e)
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
    .expect("compile ok: VStack inside Center must not produce cyclic dependency");

    let vstack_node = compiled.layout.nodes.iter().find(|n| n.name == "VStack").expect("VStack found");
    // Height: 50 + 60 + max(0, 2 - 1) * 10 = 120
    assert_eq!(vstack_node.rect.height, 120.0);
    // Width: max(200, 150) = 200
    assert_eq!(vstack_node.rect.width, 200.0);
    // Centered in 800x600 window:
    // x = (800 - 200) / 2 = 300
    // y = (600 - 120) / 2 = 240
    assert_eq!(vstack_node.rect.x, 300.0);
    assert_eq!(vstack_node.rect.y, 240.0);

    let rects: Vec<_> = compiled.layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 2);
    assert_eq!(rects[0].rect.x, 300.0);
    assert_eq!(rects[0].rect.y, 240.0);
    assert_eq!(rects[1].rect.x, 300.0);
    assert_eq!(rects[1].rect.y, 300.0); // 240 + 50 + 10
}

#[test]
fn test_vstack_inside_center_explicit_width_no_cycle() {
    let input = r#"
    \use "components/Center.dt"
    \use "components/VStack.dt"

    \Center {
        \VStack(width: 400, gap: 20) {
            \Rect(width: 100, height: 30, color: #ef4444)
            \Rect(width: 120, height: 40, color: #22c55e)
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
    .expect("compile ok: VStack with explicit width inside Center must not produce cyclic dependency");

    let vstack_node = compiled.layout.nodes.iter().find(|n| n.name == "VStack").expect("VStack found");
    // Height: 30 + 40 + max(0, 2 - 1) * 20 = 90
    assert_eq!(vstack_node.rect.height, 90.0);
    assert_eq!(vstack_node.rect.width, 400.0);
    // Centered in 800x600 window:
    // x = (800 - 400) / 2 = 200
    // y = (600 - 90) / 2 = 255
    assert_eq!(vstack_node.rect.x, 200.0);
    assert_eq!(vstack_node.rect.y, 255.0);
}

