use directedtype::ast::ComponentKey;
use directedtype::compiler::compiled::CompiledDocument;
use directedtype::compiler::value::Value;
use directedtype::component::ComponentRegistry;
use directedtype::parse;
use std::path::Path;

fn find_node<'a>(
    compiled: &'a CompiledDocument,
    id: directedtype::compiler::expanded::NodeId,
) -> Option<&'a directedtype::compiler::layout::ResolvedNode> {
    compiled.layout.nodes.iter().find(|n| n.id == id)
}

fn find_descendant_text<'a>(
    compiled: &'a CompiledDocument,
    ancestor_id: directedtype::compiler::expanded::NodeId,
) -> &'a directedtype::compiler::layout::ResolvedNode {
    compiled
        .layout
        .nodes
        .iter()
        .find(|n| {
            n.name == "Text" && {
                let mut cur = n.parent;
                let mut is_desc = false;
                while let Some(pid) = cur {
                    if pid == ancestor_id {
                        is_desc = true;
                        break;
                    }
                    cur = find_node(compiled, pid).and_then(|p| p.parent);
                }
                is_desc
            }
        })
        .expect("expected descendant Text node")
}

#[test]
fn test_material_typography_all_13_variants() {
    let input = r#"
    \use "components/VStack.dt"
    \use "components/material/Typography.dt"

    \VStack(x: 0px, y: 0px, gap: 8px) {
        \Typography("v_h1"; variant: TypographyVariant.H1) { Heading 1 }
        \Typography("v_h2"; variant: TypographyVariant.H2) { Heading 2 }
        \Typography("v_h3"; variant: TypographyVariant.H3) { Heading 3 }
        \Typography("v_h4"; variant: TypographyVariant.H4) { Heading 4 }
        \Typography("v_h5"; variant: TypographyVariant.H5) { Heading 5 }
        \Typography("v_h6"; variant: TypographyVariant.H6) { Heading 6 }
        \Typography("v_sub1"; variant: TypographyVariant.Subtitle1) { Subtitle 1 }
        \Typography("v_sub2"; variant: TypographyVariant.Subtitle2) { Subtitle 2 }
        \Typography("v_body1";) { Body 1 Default }
        \Typography("v_body2"; variant: TypographyVariant.Body2) { Body 2 }
        \Typography("v_button"; variant: TypographyVariant.Button) { BUTTON TEXT }
        \Typography("v_caption"; variant: TypographyVariant.Caption) { Caption text }
        \Typography("v_overline"; variant: TypographyVariant.Overline) { OVERLINE TEXT }
    }
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        1200.0,
        1000.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let expected = [
        ("v_h1", 96.0, 300, 112.0),
        ("v_h2", 60.0, 300, 72.0),
        ("v_h3", 48.0, 400, 56.0),
        ("v_h4", 34.0, 400, 42.0),
        ("v_h5", 24.0, 400, 32.0),
        ("v_h6", 20.0, 500, 32.0),
        ("v_sub1", 16.0, 400, 28.0),
        ("v_sub2", 14.0, 500, 22.0),
        ("v_body1", 16.0, 400, 24.0),
        ("v_body2", 14.0, 400, 20.0),
        ("v_button", 14.0, 500, 24.0),
        ("v_caption", 12.0, 400, 20.0),
        ("v_overline", 12.0, 400, 32.0),
    ];

    for (key, size, weight, line_height) in expected {
        let comp = compiled
            .find_by_key(None, &ComponentKey::string(key))
            .unwrap_or_else(|| panic!("missing key {key}"));
        let text_node = find_descendant_text(&compiled, comp.id);
        assert_eq!(
            text_node.properties.get("color"),
            Some(&Value::Color("#000000de".to_string())),
            "default color for {key}"
        );
        let font_id = text_node.font.expect("font node attached");
        let font_node = find_node(&compiled, font_id).unwrap();
        assert_eq!(
            font_node.properties.get("size"),
            Some(&Value::Distance(size)),
            "size mismatch for {key}"
        );
        assert_eq!(
            font_node.properties.get("weight"),
            Some(&Value::Integer(weight)),
            "weight mismatch for {key}"
        );
        assert_eq!(
            font_node.properties.get("line_height"),
            Some(&Value::Distance(line_height)),
            "line_height mismatch for {key}"
        );
    }
}

#[test]
fn test_material_typography_all_12_shorthands_and_tokens() {
    let input = r#"
    \use "components/VStack.dt"
    \use "components/material/Palette.dt"
    \use "components/material/Typography.dt"

    \VStack(x: 10px, y: 10px, gap: 6px) {
        \H1("s_h1";) { H1 Shorthand }
        \H2("s_h2";) { H2 Shorthand }
        \H3("s_h3";) { H3 Shorthand }
        \H4("s_h4";) { H4 Shorthand }
        \H5("s_h5";) { H5 Shorthand }
        \H6("s_h6";) { H6 Shorthand }
        \Subtitle1("s_sub1"; color: material_text_secondary) { Subtitle 1 }
        \Subtitle2("s_sub2"; color: material_text_disabled) { Subtitle 2 }
        \Body1("s_body1"; color: material_text_primary) { Body 1 }
        \Body2("s_body2";) { Body 2 }
        \Caption("s_caption";) { Caption }
        \Overline("s_overline";) { Overline }
        \Text("token_btn"; font: material_font_button, color: material_text_primary) { Token Button Font }
    }
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        1200.0,
        1000.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let expected = [
        ("s_h1", 96.0, 300, 112.0, "#000000de"),
        ("s_h2", 60.0, 300, 72.0, "#000000de"),
        ("s_h3", 48.0, 400, 56.0, "#000000de"),
        ("s_h4", 34.0, 400, 42.0, "#000000de"),
        ("s_h5", 24.0, 400, 32.0, "#000000de"),
        ("s_h6", 20.0, 500, 32.0, "#000000de"),
        ("s_sub1", 16.0, 400, 28.0, "#00000099"),
        ("s_sub2", 14.0, 500, 22.0, "#00000061"),
        ("s_body1", 16.0, 400, 24.0, "#000000de"),
        ("s_body2", 14.0, 400, 20.0, "#000000de"),
        ("s_caption", 12.0, 400, 20.0, "#000000de"),
        ("s_overline", 12.0, 400, 32.0, "#000000de"),
    ];

    for (key, size, weight, line_height, color) in expected {
        let comp = compiled
            .find_by_key(None, &ComponentKey::string(key))
            .unwrap_or_else(|| panic!("missing key {key}"));
        assert!(comp.rect.width > 0.0, "{key} should have intrinsic width > 0");
        assert!(comp.rect.height > 0.0, "{key} should have height > 0");
        let text_node = find_descendant_text(&compiled, comp.id);
        assert_eq!(
            text_node.properties.get("color"),
            Some(&Value::Color(color.to_string())),
            "color mismatch for {key}"
        );
        let font_id = text_node.font.expect("font node attached");
        let font_node = find_node(&compiled, font_id).unwrap();
        assert_eq!(
            font_node.properties.get("size"),
            Some(&Value::Distance(size)),
            "size mismatch for {key}"
        );
        assert_eq!(
            font_node.properties.get("weight"),
            Some(&Value::Integer(weight)),
            "weight mismatch for {key}"
        );
        assert_eq!(
            font_node.properties.get("line_height"),
            Some(&Value::Distance(line_height)),
            "line_height mismatch for {key}"
        );
    }

    let token_btn = compiled
        .find_by_key(None, &ComponentKey::string("token_btn"))
        .unwrap();
    let token_font_id = token_btn.font.expect("token_btn font node");
    let token_font = find_node(&compiled, token_font_id).unwrap();
    assert_eq!(
        token_font.properties.get("size"),
        Some(&Value::Distance(14.0))
    );
    assert_eq!(
        token_font.properties.get("weight"),
        Some(&Value::Integer(500))
    );
    assert_eq!(
        token_font.properties.get("line_height"),
        Some(&Value::Distance(24.0))
    );
}

#[test]
fn test_material_typography_wrapping_alignment_and_inline_spans() {
    let input = r#"
    \use "components/Align.dt"
    \use "components/Bold.dt"
    \use "components/Italic.dt"
    \use "components/Code.dt"
    \use "components/VStackStretch.dt"
    \use "components/material/Typography.dt"

    \env font = \Font(family: "Georgia", size: 16px);

    \VStackStretch(x: 20px, y: 20px, width: 240px, gap: 12px) {
        \H4("centered_h4"; align: Align.Center) { Centered Heading }
        \Body1("wrapped_body";) {
            This paragraph uses \Bold { bold text }, \Italic { italics }, and \Code { monospace } inside a wrapped Body1 block that spans multiple lines.
        }
        \Caption("right_caption"; align: Align.Right) { Right-aligned caption }
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

    // 1. Centered H4 receives width=240 from VStackStretch and inherits family="Georgia"
    let h4_comp = compiled
        .find_by_key(None, &ComponentKey::string("centered_h4"))
        .unwrap();
    assert_eq!(h4_comp.rect.width, 240.0);
    let h4_text = find_descendant_text(&compiled, h4_comp.id);
    assert_eq!(h4_text.rect.width, 240.0);
    assert_eq!(
        h4_text.properties.get("align"),
        Some(&Value::Enum {
            enum_name: "Align".to_string(),
            variant: "Center".to_string(),
        })
    );
    let h4_font = find_node(&compiled, h4_text.font.unwrap()).unwrap();
    assert_eq!(
        h4_font.properties.get("family"),
        Some(&Value::String("Georgia".to_string()))
    );
    assert_eq!(
        h4_font.properties.get("size"),
        Some(&Value::Distance(34.0))
    );

    // 2. Wrapped Body1 with inline \Bold, \Italic, and \Code spans
    let body_comp = compiled
        .find_by_key(None, &ComponentKey::string("wrapped_body"))
        .unwrap();
    assert_eq!(body_comp.rect.width, 240.0);
    let body_text = find_descendant_text(&compiled, body_comp.id);
    assert!(
        body_text.rect.height > 24.0,
        "Expected wrapped Body1 to span multiple lines (height = {})",
        body_text.rect.height
    );
    // Verify inline spans inherited Body1's 16px size while applying their own overrides
    let bold_span = body_text
        .text_spans
        .iter()
        .find(|s| s.style.font_weight == Some(700.0))
        .expect("expected bold span in Body1");
    assert_eq!(bold_span.style.font_size, Some(16.0));
    assert_eq!(bold_span.style.font_family.as_deref(), Some("Georgia"));

    let code_span = body_text
        .text_spans
        .iter()
        .find(|s| s.style.font_family.as_deref() == Some("monospace"))
        .expect("expected monospace code span in Body1");
    assert!((code_span.style.font_size.unwrap() - 16.0 * 0.9).abs() < 0.01);

    // 3. Right-aligned Caption
    let cap_comp = compiled
        .find_by_key(None, &ComponentKey::string("right_caption"))
        .unwrap();
    let cap_text = find_descendant_text(&compiled, cap_comp.id);
    assert_eq!(
        cap_text.properties.get("align"),
        Some(&Value::Enum {
            enum_name: "Align".to_string(),
            variant: "Right".to_string(),
        })
    );
}

#[test]
fn test_material_button_inherits_env_font_and_button_metrics() {
    let input = r#"
    \use "components/HStack.dt"
    \use "components/material/Button.dt"

    \env font = \Font(family: "Georgia", size: 16px);

    \HStack(x: 10px, y: 10px, gap: 12px) {
        \Button("btn_label"; label: "Label Button")
        \Button("btn_child"; size: ButtonSize.Large) { Child Button }
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

    for (key, expected_size) in [("btn_label", 14.0), ("btn_child", 15.0)] {
        let btn_comp = compiled
            .find_by_key(None, &ComponentKey::string(key))
            .unwrap();
        let btn_text = find_descendant_text(&compiled, btn_comp.id);
        let btn_font = find_node(&compiled, btn_text.font.unwrap()).unwrap();
        assert_eq!(
            btn_font.properties.get("family"),
            Some(&Value::String("Georgia".to_string()))
        );
        assert_eq!(
            btn_font.properties.get("size"),
            Some(&Value::Distance(expected_size))
        );
        assert_eq!(btn_font.properties.get("weight"), Some(&Value::Integer(500)));
        assert_eq!(
            btn_font.properties.get("line_height"),
            Some(&Value::Distance(24.0))
        );
    }
}

#[test]
fn test_material_typography_gutter_bottom() {
    let input = r#"
    \use "components/VStack.dt"
    \use "components/material/Typography.dt"

    \VStack(x: 0px, y: 0px, gap: 0px) {
        \H4("h4_no_gutter"; gutter_bottom: false) { Section Heading }
        \H4("h4_with_gutter"; gutter_bottom: true) { Section Heading }
        \Body1("following_body";) { Following paragraph }
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

    let no_gutter = compiled
        .find_by_key(None, &ComponentKey::string("h4_no_gutter"))
        .unwrap();
    let with_gutter = compiled
        .find_by_key(None, &ComponentKey::string("h4_with_gutter"))
        .unwrap();
    let following_body = compiled
        .find_by_key(None, &ComponentKey::string("following_body"))
        .unwrap();

    // H4 has size 34px, so gutter_bottom adds 34.0 * 0.35 = 11.9px to height/bottom
    let expected_gutter = 34.0 * 0.35;
    assert!(
        (with_gutter.rect.height - (no_gutter.rect.height + expected_gutter)).abs() < 1e-6,
        "Expected with_gutter.height ({}) == no_gutter.height ({}) + {}",
        with_gutter.rect.height,
        no_gutter.rect.height,
        expected_gutter
    );
    // Inner \Text height is unchanged
    let inner_text = find_descendant_text(&compiled, with_gutter.id);
    assert!((inner_text.rect.height - no_gutter.rect.height).abs() < 1e-6);

    // Following sibling starts at with_gutter.bottom
    assert!(
        (following_body.rect.y - (with_gutter.rect.y + with_gutter.rect.height)).abs() < 1e-6
    );
}


