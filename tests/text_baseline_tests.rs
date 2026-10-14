use directedtype::compiler::CompiledDocument;
use directedtype::parser::parse_document;

#[test]
fn test_text_overload_with_baseline_port() {
    let source = r#"
\let icon = \Rect(x: 10, y: 20, width: 14, height: 14, color: #3b82f6)
\let label = \Text(x: 30, baseline: icon.bottom, size: 14) {
    "Document.dt"
}
"#;

    let doc = parse_document(source).expect("parse ok");
    let compiled = CompiledDocument::compile(&doc).expect("compile ok");
    let layout = compiled.layout();

    let icon_node = layout.nodes.iter().find(|n| n.properties.contains_key("color")).expect("icon found");
    let label_node = layout.nodes.iter().find(|n| n.name == "Text" && n.text_content.is_some()).expect("label found");

    let icon_bottom = icon_node.rect.y + icon_node.rect.height;
    assert_eq!(icon_bottom, 34.0);

    let baseline = label_node.properties.get("baseline").and_then(|v| v.as_f64()).expect("baseline property present");
    assert_eq!(baseline, 34.0);

    // label y should be baseline - offset, meaning y < baseline and y + offset == baseline
    assert!(label_node.rect.y < baseline);
    assert!(label_node.rect.y > 20.0);
}

#[test]
fn test_two_text_sizes_sharing_same_baseline() {
    let source = r#"
\let title = \Text(x: 10, baseline: 80, size: 24) {
    "Large Title"
}
\let subtitle = \Text(x: 200, baseline: 80, size: 12) {
    "Small Subtitle"
}
"#;

    let doc = parse_document(source).expect("parse ok");
    let compiled = CompiledDocument::compile(&doc).expect("compile ok");
    let layout = compiled.layout();

    let title_node = layout.nodes.iter().find(|n| n.text_content.as_deref().unwrap_or("").contains("Large")).expect("title found");
    let sub_node = layout.nodes.iter().find(|n| n.text_content.as_deref().unwrap_or("").contains("Small")).expect("subtitle found");

    let title_baseline = title_node.properties.get("baseline").and_then(|v| v.as_f64()).unwrap();
    let sub_baseline = sub_node.properties.get("baseline").and_then(|v| v.as_f64()).unwrap();

    assert_eq!(title_baseline, 80.0);
    assert_eq!(sub_baseline, 80.0);

    // The larger font has a higher top (smaller y) because cap-height is larger,
    // but they align on the exact same baseline:
    assert!(title_node.rect.y < sub_node.rect.y);
}

#[test]
fn test_traditional_y_still_generates_baseline() {
    let source = r#"
\let t = \Text(x: 10, y: 40, size: 16) {
    "Traditional"
}
"#;

    let doc = parse_document(source).expect("parse ok");
    let compiled = CompiledDocument::compile(&doc).expect("compile ok");
    let layout = compiled.layout();

    let node = layout.nodes.iter().find(|n| n.name == "Text" && n.text_content.is_some()).unwrap();
    assert_eq!(node.rect.y, 40.0);

    let baseline = node.properties.get("baseline").and_then(|v| v.as_f64()).unwrap();
    assert!(baseline > 40.0);
}
