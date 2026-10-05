use directedtype::compiler::compiled::CompiledDocument;
use directedtype::component::ComponentRegistry;
use directedtype::parse;
use std::path::Path;

#[test]
fn test_card_with_direct_raw_text() {
    let input = r#"
    \use "components/Card.dt"
    \use "theme/default.dt"

    \Card(style: card_dark, x: 20, y: 30, width: 250) {
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
    assert_eq!(text_node.rect.height, 16.0);

    // Rect should wrap the text with padding_y on both sides: 20 + 16 + 20 = 56
    let rect = compiled.layout.nodes.iter().find(|n| n.name == "Rect").expect("rect found");
    assert_eq!(rect.rect.x, 20.0);
    assert_eq!(rect.rect.y, 30.0);
    assert_eq!(rect.rect.width, 250.0);
    assert_eq!(rect.rect.height, 56.0);
}

#[test]
fn test_card_multi_child_vertical_flow_gap() {
    let input = r#"
    \use "components/Card.dt"
    \use "theme/default.dt"

    \Card(style: card_dark, x: 0, y: 0, width: 300, gap: 10) {
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

    // Card should wrap both lines: 20 + 16 + 10 + 16 + 20 = 82
    let card_node = compiled.layout.nodes.iter().find(|n| n.name == "Card").expect("card found");
    let rect = compiled.layout.nodes.iter().find(|n| n.name == "Rect" && n.parent == Some(card_node.id)).expect("rect found");
    assert_eq!(rect.rect.height, 82.0);
}

#[test]
fn test_card_auto_height_wraps_children() {
    let input = r#"
    \use "components/Card.dt"
    \use "components/CardStyle.dt"

    let custom_style = \CardStyle(
        bg: #1e293b,
        border_color: #334155,
        border_width: 1,
        radius: 12,
        padding_x: 16,
        padding_y: 18
    );

    \Card(style: custom_style, x: 20, y: 30, width: 300, gap: 12) {
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

    \Card(style: card_dark, x: 0, y: 0, width: 200, height: 80) {
        \Rect(width: 100, height: 40)
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
