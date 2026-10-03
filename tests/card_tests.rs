use directedtype::compiler::compiled::CompiledDocument;
use directedtype::compiler::value::Value;
use directedtype::component::ComponentRegistry;
use directedtype::parse;
use std::path::Path;

#[test]
fn test_card_with_direct_raw_text() {
    let input = r#"
    \use "components/Card.dt"

    \Card(x: 20, y: 30, width: 250, height: 80, padding_x: 16, padding_y: 14) {
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

    // Rect should be positioned at (20, 30) with width 250, height 80
    let rect = compiled.layout.nodes.iter().find(|n| n.name == "Rect").expect("rect found");
    assert_eq!(rect.rect.x, 20.0);
    assert_eq!(rect.rect.y, 30.0);
    assert_eq!(rect.rect.width, 250.0);
    assert_eq!(rect.rect.height, 80.0);

    // Direct raw text should be converted to synthetic Text node positioned inside card padding
    let text_node = compiled.layout.nodes.iter().find(|n| n.name == "Text").expect("text node found");
    assert_eq!(text_node.text_content.as_deref(), Some("Hello Direct Text!"));
    assert_eq!(text_node.rect.x, 36.0); // 20 + 16
    assert_eq!(text_node.rect.y, 44.0); // 30 + 14

    // Ambient text color inherited from Card
    assert_eq!(text_node.properties.get("color"), Some(&Value::Color("#f8fafc".into())));
}

#[test]
fn test_card_optical_vertical_centering() {
    let input = r#"
    \use "components/Card.dt"

    \Card(x: 10, y: 10, width: 200, height: 60, center_y: true) {
        Optically Centered
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

    let text_node = compiled.layout.nodes.iter().find(|n| n.name == "Text").expect("text node found");
    // Expected y: parent.top (10) + (60 - 16)/2 - 1 = 10 + 22 - 1 = 31.0
    assert_eq!(text_node.rect.y, 31.0);
}

#[test]
fn test_card_multi_child_vertical_flow_gap() {
    let input = r#"
    \use "components/Card.dt"

    \Card(x: 0, y: 0, width: 300, height: 160, padding_x: 20, padding_y: 20, gap: 10) {
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
}
