use directedtype::compiler::compiled::CompiledDocument;
use directedtype::component::ComponentRegistry;
use directedtype::parse;
use std::path::Path;

#[test]
fn test_card_with_direct_raw_text() {
    let input = r#"
    \use "components/Card.dt"
    \use "theme/default.dt"

    \Card(style: card_dark, x: 20px, y: 30px, width: 250px) {
        Hello Direct Text!
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

    // Direct raw text should be converted to synthetic Text node positioned inside card padding (20)
    let text_node = compiled.layout.nodes.iter().find(|n| n.name == "Text").expect("text node found");
    assert_eq!(text_node.text_content.as_deref(), Some("Hello Direct Text!"));
    assert_eq!(text_node.rect.x, 40.0); // 20 + 20
    assert_eq!(text_node.rect.y, 50.0); // 30 + 20
    let expected_cap = directedtype::compiler::text::measure_font_metrics(16.0, 400.0, None).cap_height;
    assert_eq!(text_node.rect.height, expected_cap);

    // Rect should wrap the text with padding_y on both sides: 20 + cap_height + 20
    let rect = compiled.layout.nodes.iter().find(|n| n.name == "Rect").expect("rect found");
    assert_eq!(rect.rect.x, 20.0);
    assert_eq!(rect.rect.y, 30.0);
    assert_eq!(rect.rect.width, 250.0);
    assert_eq!(rect.rect.height, 40.0 + expected_cap);
}

#[test]
fn test_card_multi_child_vertical_flow_gap() {
    let input = r#"
    \use "components/Card.dt"
    \use "theme/default.dt"

    \Card(style: card_dark, x: 0, y: 0, width: 300px, gap: 10px) {
        \Text(text: "Line 1")
        \Text(text: "Line 2")
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

    let text_nodes: Vec<_> = compiled.layout.nodes.iter().filter(|n| n.name == "Text").collect();
    assert_eq!(text_nodes.len(), 2);

    assert_eq!(text_nodes[0].rect.y, 20.0);
    // Second line should start after first line's bottom + gap
    assert_eq!(text_nodes[1].rect.y, text_nodes[0].rect.y + text_nodes[0].rect.height + 10.0);

    // Card should wrap both lines: 20 + cap_height + 10 + cap_height + 20
    let expected_cap = directedtype::compiler::text::measure_font_metrics(16.0, 400.0, None).cap_height;
    let card_node = compiled.layout.nodes.iter().find(|n| n.name == "Card").expect("card found");
    let rect = compiled.layout.nodes.iter().find(|n| n.name == "Rect" && n.parent == Some(card_node.id)).expect("rect found");
    assert_eq!(rect.rect.height, 50.0 + 2.0 * expected_cap);
}

#[test]
fn test_card_auto_height_wraps_children() {
    let input = r#"
    \use "components/Card.dt"
    \use "components/CardStyle.dt"

    \let custom_style = \CardStyle(
        bg: #1e293b,
        border_color: #334155,
        border_width: 1px,
        radius: 12px,
        padding_x: 16px,
        padding_y: 18px
    );

    \Card(style: custom_style, x: 20px, y: 30px, width: 300px, gap: 12px) {
        \Rect(width: 100px, height: 40px, color: #3b82f6)
        \Rect(width: 150px, height: 50px, color: #10b981)
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

    let card_node = compiled.layout.nodes.iter().find(|n| n.name == "Card").expect("card found");
    let card_rect = compiled.layout.nodes.iter().find(|n| n.name == "Rect" && n.parent == Some(card_node.id)).expect("card background rect found");

    // Children:
    // child 1: y = 30 + 18 = 48, height = 40, bottom = 88
    // child 2: y = 88 + 12 = 100, height = 50, bottom = 150
    // Card bottom: max(children.bottom) (150) + padding_y (18) = 168
    // Card height: bottom (168) - y (30) = 138
    assert_eq!(card_rect.rect.x, 20.0);
    assert_eq!(card_rect.rect.y, 30.0);
    assert_eq!(card_rect.rect.width, 300.0);
    assert_eq!(card_rect.rect.height, 138.0);
}

#[test]
fn test_card_rejects_external_height_specification() {
    let input = r#"
    \use "components/Card.dt"
    \use "theme/default.dt"

    \Card(style: card_dark, x: 0, y: 0, width: 200px, height: 80px) {
        \Rect(width: 100px, height: 40px)
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
    .expect_err("Card height is an immutable alias port and must not be specifiable from the outside");

    match err {
        directedtype::compiler::CompileError::ImmutableAliasPort { port, .. } => {
            assert_eq!(port, "height");
        }
        _ => panic!("Expected ImmutableAliasPort for height, got {:?}", err),
    }
}

#[test]
fn test_card_pushes_width_down_to_nested_vstack() {
    let input = r#"
    \use "components/Card.dt"
    \use "components/VStack.dt"
    \use "theme/default.dt"

    \Card(style: card_dark, x: 20px, y: 30px, width: 300px) {
        \VStack(gap: 12px) {
            \Rect(x: 40px, y: 50px, width: 100px, height: 40px, color: #3b82f6)
            \Rect(x: 40px, y: 102px, width: 120px, height: 40px, color: #10b981)
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

    let vstack = compiled.layout.nodes.iter().find(|n| n.name == "VStack").expect("VStack found");
    // Card width (300) - 2 * padding (20) = 260
    assert_eq!(vstack.rect.width, 260.0);
}
