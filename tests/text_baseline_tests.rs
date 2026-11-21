use directedtype::compiler::CompiledDocument;
use directedtype::parser::parse_document;

#[test]
fn test_text_overload_with_baseline_port() {
    let source = r#"
\let icon = \Rect(x: 10px, y: 20px, width: 14px, height: 14px, color: #3b82f6)
\let label = \Text(x: 30px, baseline: icon.bottom, font: \Font(size: 14px)) {
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
\let title = \Text(x: 10px, baseline: 80px, font: \Font(size: 24px)) {
    "Large Title"
}
\let subtitle = \Text(x: 200px, baseline: 80px, font: \Font(size: 12px)) {
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
\let t = \Text(x: 10px, y: 40px, font: \Font(size: 16px)) {
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

#[test]
fn test_font_letter_spacing_widens_text_and_inherits() {
    let source = r#"
\let f_plain = \Font(size: 16px)
\let f_spaced = \Font(font: f_plain, letter_spacing: 2px)
\let f_inherited = \Font(font: f_spaced, weight: 700)
\let t_plain = \Text(x: 0px, y: 0px, font: f_plain, text: "CHAPTER IV.")
\let t_spaced = \Text(x: 0px, y: 30px, font: f_spaced, text: "CHAPTER IV.")
\let t_inherited = \Text(x: 0px, y: 60px, font: f_inherited, text: "CHAPTER IV.")
"#;

    let doc = parse_document(source).expect("parse ok");
    let compiled = CompiledDocument::compile(&doc).expect("compile ok");
    let layout = compiled.layout();

    let texts: Vec<_> = layout
        .nodes
        .iter()
        .filter(|n| n.name == "Text" && n.properties.contains_key("text"))
        .collect();
    assert_eq!(texts.len(), 3);
    assert!(
        texts[1].rect.width > texts[0].rect.width + 15.0,
        "2px letter_spacing across 11 chars should widen text significantly ({} vs {})",
        texts[1].rect.width,
        texts[0].rect.width
    );
    assert!(
        texts[2].rect.width > texts[0].rect.width + 15.0,
        "inherited letter_spacing should also widen text ({} vs {})",
        texts[2].rect.width,
        texts[0].rect.width
    );
}

#[test]
fn test_forced_justified_line_break_escape() {
    let source = r#"
\Enum Align {
    Left,
    Center,
    Right,
    Justify,
    JustifyAll,
}

\InlineBox DashBox(
    x: Distance = parent.left,
    y: Distance = parent.top,
    env font: Font
) {
    \alias width: Distance = 24px;
    \alias height: Distance = 4px;
}

\Span Mark(env font: Font) {
    \Children
}

\let t = \Text(x: 0px, y: 0px, width: 300px, font: \Font(size: 16px), paragraph_style: \ParagraphStyle(align: Align.Justify)) {
    Alice was \DashBox{}\Mark{point\-\|
    ing} at \Mark{sing}
}
"#;

    let doc = parse_document(source).expect("parse ok");
    let compiled = CompiledDocument::compile(&doc).expect("compile ok");
    let layout = compiled.layout();

    let t_node = layout
        .nodes
        .iter()
        .find(|n| n.name == "Text" && n.text_content.is_some())
        .expect("Text node");
    assert_eq!(
        t_node.text_content.as_deref(),
        Some("Alice was point-\u{2060}\ning at sing")
    );

    let dash = layout
        .nodes
        .iter()
        .find(|n| n.name == "DashBox")
        .expect("DashBox");
    let marks: Vec<_> = layout
        .nodes
        .iter()
        .filter(|n| n.name == "Mark")
        .collect();
    assert_eq!(marks.len(), 2);

    // First line (`Alice was [DashBox]point-`) is justified to right edge ~300px:
    // `point-` fragment on line 0 should touch ~300px, and `DashBox` should sit immediately before `point-`.
    let line0_frag = &marks[0].fragments[0];
    assert!(
        (line0_frag.x + line0_frag.width - 300.0).abs() < 2.0,
        "Expected line 0 trailing word 'point-' to reach justified right edge 300px, got {}",
        line0_frag.x + line0_frag.width
    );
    assert!(
        (dash.rect.x + dash.rect.width - line0_frag.x).abs() < 1.5,
        "Expected DashBox right edge ({}) to align with justified 'point-' start ({})",
        dash.rect.x + dash.rect.width,
        line0_frag.x
    );

    // Final line (`ing at sing`) must remain ragged (well short of 300px).
    let last_frag = &marks[1].fragments[0];
    assert!(
        last_frag.x + last_frag.width < 180.0,
        "Expected final line of Align.Justify to remain ragged, got right edge {}",
        last_frag.x + last_frag.width
    );
}

