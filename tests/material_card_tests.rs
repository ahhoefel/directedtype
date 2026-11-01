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

#[test]
fn test_material_card_explicit_width_with_header_content_and_actions() {
    let input = r#"
    \use "components/Align.dt"
    \use "components/material/Palette.dt"
    \use "components/material/Button.dt"
    \use "components/material/Divider.dt"
    \use "components/material/Typography.dt"
    \use "components/material/Card.dt"

    \Card("recipe_card"; x: 20px, y: 20px, width: 345px) {
        \CardHeader(
            "hdr";
            avatar: "R",
            avatar_bg: material_error.main,
            title: "Shrimp and Chorizo Paella",
            subheader: "September 14, 2016"
        ) {
            \Button("hdr_action"; size: ButtonSize.Small) { Follow }
        }
        \Divider("hdr_div";)
        \CardContent("content";) {
            \Overline("card_overline"; color: material_text_secondary) { WORD OF THE DAY }
            \H5("card_title"; gutter_bottom: true) { Benevolent }
            \Body2("card_body"; color: material_text_secondary) {
                This impressive paella is a perfect party dish and a fun meal to cook together with your guests. Add 1 cup of frozen peas along with the mussels, if you like.
            }
        }
        \CardActions("actions";) {
            \Button("btn_share"; size: ButtonSize.Small) { Share }
            \Button("btn_learn"; size: ButtonSize.Small) { Learn More }
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

    let card = compiled
        .find_by_key(None, &ComponentKey::string("recipe_card"))
        .unwrap();
    let hdr = compiled
        .find_by_key(None, &ComponentKey::string("hdr"))
        .unwrap();
    let hdr_action = compiled
        .find_by_key(None, &ComponentKey::string("hdr_action"))
        .unwrap();
    let hdr_div = compiled
        .find_by_key(None, &ComponentKey::string("hdr_div"))
        .unwrap();
    let content = compiled
        .find_by_key(None, &ComponentKey::string("content"))
        .unwrap();
    let card_body = compiled
        .find_by_key(None, &ComponentKey::string("card_body"))
        .unwrap();
    let actions = compiled
        .find_by_key(None, &ComponentKey::string("actions"))
        .unwrap();
    let btn_share = compiled
        .find_by_key(None, &ComponentKey::string("btn_share"))
        .unwrap();
    let btn_learn = compiled
        .find_by_key(None, &ComponentKey::string("btn_learn"))
        .unwrap();

    // 1. Card dimensions & vertical stacking of sections
    assert_eq!(card.rect.x, 20.0);
    assert_eq!(card.rect.y, 20.0);
    assert_eq!(card.rect.width, 345.0);

    let expected_card_h =
        hdr.rect.height + hdr_div.rect.height + content.rect.height + actions.rect.height;
    assert!(
        (card.rect.height - expected_card_h).abs() < 1e-4,
        "Card height ({}) should equal sum of sections ({})",
        card.rect.height,
        expected_card_h
    );

    // 2. CardHeader width, avatar, and right-aligned action button
    assert_eq!(hdr.rect.width, 345.0);
    assert!(hdr.rect.height >= 72.0); // 40px avatar + 2 * 16px padding_y = 72px minimum
    assert!(
        (hdr_action.rect.x - (hdr.rect.x + 345.0 - 16.0 - hdr_action.rect.width)).abs() < 1e-4,
        "Header action button should be right-aligned with 16px padding"
    );

    // 3. Divider stretches full 345px width of the Card
    assert_eq!(hdr_div.rect.width, 345.0);
    assert_eq!(hdr_div.rect.height, 1.0);

    // 4. CardContent pushes inner_width (345 - 32 = 313px) to Typography children, wrapping Body2
    assert_eq!(content.rect.width, 345.0);
    assert_eq!(card_body.rect.width, 313.0);
    assert!(
        card_body.rect.height > 20.0,
        "Wrapped Body2 inside 313px CardContent should span multiple lines (got {})",
        card_body.rect.height
    );

    // 5. CardActions horizontal spacing (8px padding_x, 8px gap, 8px padding_y)
    assert_eq!(actions.rect.width, 345.0);
    assert_eq!(actions.rect.height, 30.0 + 16.0); // Small button (30px) + 2*8px padding_y = 46px
    assert_eq!(btn_share.rect.x, actions.rect.x + 8.0);
    assert!(
        (btn_learn.rect.x - (btn_share.rect.x + btn_share.rect.width + 8.0)).abs() < 1e-4
    );
}

#[test]
fn test_material_card_variants_raised_and_square() {
    let input = r#"
    \use "components/material/Typography.dt"
    \use "components/material/Card.dt"

    \Card("default_card"; x: 10px, y: 10px, width: 200px) {
        \CardContent { \Body2 { Default Elevation (1) } }
    }

    \Card("raised_card"; x: 10px, y: 100px, width: 200px, raised: true) {
        \CardContent { \Body2 { Raised Elevation (8) } }
    }

    \Card("outlined_square_card"; x: 10px, y: 190px, width: 200px, variant: CardVariant.Outlined, square: true) {
        \CardContent { \Body2 { Outlined Square } }
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

    // 1. Default Card: elevation = 1 -> blur = 3 + 1.8 = 4.8, offset_y = 1 + 0.75 = 1.75, radius = 4
    let default_card = compiled
        .find_by_key(None, &ComponentKey::string("default_card"))
        .unwrap();
    let default_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(default_card.id) && n.name == "Rect")
        .unwrap();
    assert_eq!(
        default_rect.properties.get("radius").and_then(|v| v.as_f64()),
        Some(4.0)
    );
    assert_eq!(
        default_rect.properties.get("border_width").and_then(|v| v.as_f64()),
        Some(0.0)
    );
    assert!(
        (default_rect.properties.get("shadow_blur").and_then(|v| v.as_f64()).unwrap() - 4.8).abs()
            < 1e-4
    );
    assert!(
        (default_rect.properties.get("shadow_offset_y").and_then(|v| v.as_f64()).unwrap() - 1.75)
            .abs()
            < 1e-4
    );

    // 2. Raised Card: elevation = 8 -> blur = 3 + 8*1.8 = 17.4, offset_y = 1 + 8*0.75 = 7.0
    let raised_card = compiled
        .find_by_key(None, &ComponentKey::string("raised_card"))
        .unwrap();
    let raised_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(raised_card.id) && n.name == "Rect")
        .unwrap();
    assert!(
        (raised_rect.properties.get("shadow_blur").and_then(|v| v.as_f64()).unwrap() - 17.4).abs()
            < 1e-4
    );
    assert!(
        (raised_rect.properties.get("shadow_offset_y").and_then(|v| v.as_f64()).unwrap() - 7.0)
            .abs()
            < 1e-4
    );

    // 3. Outlined & Square Card: border_width = 1, radius = 0, shadow_blur = 0
    let outlined_card = compiled
        .find_by_key(None, &ComponentKey::string("outlined_square_card"))
        .unwrap();
    let outlined_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(outlined_card.id) && n.name == "Rect")
        .unwrap();
    assert_eq!(
        outlined_rect.properties.get("radius").and_then(|v| v.as_f64()),
        Some(0.0)
    );
    assert_eq!(
        outlined_rect.properties.get("border_width").and_then(|v| v.as_f64()),
        Some(1.0)
    );
    assert_eq!(
        outlined_rect.properties.get("shadow_blur").and_then(|v| v.as_f64()),
        Some(0.0)
    );
}

#[test]
fn test_material_card_intrinsic_bottom_up_sizing() {
    let input = r#"
    \use "components/Align.dt"
    \use "components/material/Button.dt"
    \use "components/material/Divider.dt"
    \use "components/material/Typography.dt"
    \use "components/material/Card.dt"

    \Card("intrinsic_card"; x: 16px, y: 16px) {
        \CardHeader("i_hdr"; title: "Compact Header", subheader: "Secondary")
        \Divider("i_div";)
        \CardContent("i_content";) {
            \Body1 { This intrinsic body line is wider than the header above it. }
        }
        \CardActions("i_actions"; width: parent.width, align: Align.Right) {
            \Button("i_cancel"; size: ButtonSize.Small) { Cancel }
            \Button("i_ok"; size: ButtonSize.Small, variant: ButtonVariant.Contained) { Confirm }
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

    let card = compiled
        .find_by_key(None, &ComponentKey::string("intrinsic_card"))
        .unwrap();
    let hdr = compiled
        .find_by_key(None, &ComponentKey::string("i_hdr"))
        .unwrap();
    let div = compiled
        .find_by_key(None, &ComponentKey::string("i_div"))
        .unwrap();
    let content = compiled
        .find_by_key(None, &ComponentKey::string("i_content"))
        .unwrap();
    let actions = compiled
        .find_by_key(None, &ComponentKey::string("i_actions"))
        .unwrap();
    let ok_btn = compiled
        .find_by_key(None, &ComponentKey::string("i_ok"))
        .unwrap();

    // Card width is pulled up from max(hdr.width, content.width)
    assert!(content.rect.width > hdr.rect.width);
    assert!((card.rect.width - content.rect.width).abs() < 1e-4);

    // Divider and right-aligned CardActions(width: parent.width) stretch to Card.width
    assert!((div.rect.width - card.rect.width).abs() < 1e-4);
    assert!((actions.rect.width - card.rect.width).abs() < 1e-4);

    // Right-aligned Confirm button sits 8px from right edge of Card
    assert!(
        ((ok_btn.rect.x + ok_btn.rect.width) - (card.rect.x + card.rect.width - 8.0)).abs() < 1e-4
    );
}

#[test]
fn test_material_card_header_typography_and_custom_children_overloads() {
    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/Typography.dt"
    \use "components/material/Card.dt"

    // 1. Header without avatar -> title uses 20px / 500 / 28px
    \CardHeader("hdr_no_avatar"; x: 10px, y: 10px, title: "Title Only")

    // 2. Custom children overload with explicit width and avatar
    \CardHeader("hdr_custom_fixed"; x: 10px, y: 100px, width: 300px, avatar: "A", avatar_bg: material_primary.main) {
        \Subtitle1("custom_title";) { Custom Block Title }
        \Caption("custom_cap"; color: material_text_secondary) { Updated 5m ago }
    }

    // 3. Custom children overload with intrinsic width
    \CardHeader("hdr_custom_intrinsic"; x: 10px, y: 200px, avatar: "B") {
        \Subtitle2 { Intrinsic Custom Header }
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

    // 1. Check title_node font when no avatar is present (20px, weight 500, line_height 28px)
    let hdr_no_avatar = compiled
        .find_by_key(None, &ComponentKey::string("hdr_no_avatar"))
        .unwrap();
    let title_text = compiled
        .layout
        .nodes
        .iter()
        .find(|n| {
            n.parent == Some(hdr_no_avatar.id)
                && n.name == "Text"
                && n.properties.get("text") == Some(&Value::String("Title Only".to_string()))
        })
        .expect("title text node");
    let font_node = find_node(&compiled, title_text.font.unwrap()).unwrap();
    assert_eq!(font_node.properties.get("size"), Some(&Value::Distance(20.0)));
    assert_eq!(font_node.properties.get("weight"), Some(&Value::Integer(500)));
    assert_eq!(
        font_node.properties.get("line_height"),
        Some(&Value::Distance(28.0))
    );
    assert!(
        (title_text.rect.height - 28.0).abs() < 1e-4,
        "CardHeader title without avatar should have 28px line-box height, got {}",
        title_text.rect.height
    );

    // 2. Check custom children overload with explicit width (300px) and avatar (40px + 16px gap + 2*16px pad = 88px)
    let hdr_custom_fixed = compiled
        .find_by_key(None, &ComponentKey::string("hdr_custom_fixed"))
        .unwrap();
    let custom_title = compiled
        .find_by_key(None, &ComponentKey::string("custom_title"))
        .unwrap();
    assert_eq!(hdr_custom_fixed.rect.width, 300.0);
    assert_eq!(custom_title.rect.width, 300.0 - 32.0 - 40.0 - 16.0);

    // 3. Check custom children overload with intrinsic width
    let hdr_custom_intrinsic = compiled
        .find_by_key(None, &ComponentKey::string("hdr_custom_intrinsic"))
        .unwrap();
    assert!(hdr_custom_intrinsic.rect.width > 88.0);
    assert_eq!(hdr_custom_intrinsic.rect.height, 40.0 + 32.0); // 40px avatar + 2*16px padding_y
}

#[test]
fn test_material_card_actions_alignment_and_disable_spacing() {
    let input = r#"
    \use "components/Align.dt"
    \use "components/material/Button.dt"
    \use "components/material/Card.dt"

    \CardActions("act_center"; x: 0px, y: 0px, width: 300px, align: Align.Center) {
        \Button("c_b1"; width: 80px) { One }
        \Button("c_b2"; width: 80px) { Two }
    }

    \CardActions("act_no_space"; x: 0px, y: 60px, disable_spacing: true) {
        \Button("ns_b1"; width: 70px) { First }
        \Button("ns_b2"; width: 70px) { Second }
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

    // 1. Centered CardActions (width=300, padding_x=8 -> inner=284; 2 buttons of 80 + 8 gap = 168; start_x = 8 + (284 - 168)/2 = 66)
    let c_b1 = compiled
        .find_by_key(None, &ComponentKey::string("c_b1"))
        .unwrap();
    let c_b2 = compiled
        .find_by_key(None, &ComponentKey::string("c_b2"))
        .unwrap();
    assert!((c_b1.rect.x - 66.0).abs() < 1e-4);
    assert!((c_b2.rect.x - (66.0 + 80.0 + 8.0)).abs() < 1e-4);

    // 2. disable_spacing: true (gap = 0px, intrinsic width = 70 + 70 + 16 = 156)
    let act_no_space = compiled
        .find_by_key(None, &ComponentKey::string("act_no_space"))
        .unwrap();
    let ns_b1 = compiled
        .find_by_key(None, &ComponentKey::string("ns_b1"))
        .unwrap();
    let ns_b2 = compiled
        .find_by_key(None, &ComponentKey::string("ns_b2"))
        .unwrap();
    assert_eq!(act_no_space.rect.width, 156.0);
    assert_eq!(ns_b1.rect.x, 8.0);
    assert_eq!(ns_b2.rect.x, 78.0);
}
