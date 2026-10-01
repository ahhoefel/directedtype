use directedtype::ast::Expr;
use directedtype::dom::Dom;
use directedtype::inspector::{
    build_tree_items, standard_inspector_components, InspectOverlayComponent, InspectTargetInfo,
    InspectorState, InspectorTab, INSPECT_BADGE_DTML, INSPECT_CLIP_GUIDE_DTML,
    INSPECT_HIGHLIGHT_DTML, INSPECT_OVERLAY_DTML,
};

#[test]
fn test_inspect_target_info_from_dom() {
    let mut dom = Dom::new();

    let card = dom.create_element(
        "Rect",
        vec![
            ("id".to_string(), Expr::string("main_card")),
            ("x".to_string(), Expr::lit(50.0)),
            ("y".to_string(), Expr::lit(120.0)),
            ("width".to_string(), Expr::lit(380.0)),
            ("height".to_string(), Expr::lit(180.0)),
            ("color".to_string(), Expr::color("#6366f1")),
        ],
    );
    dom.append_root(card).unwrap();
    dom.commit().unwrap();

    let info = InspectTargetInfo::from_dom(&dom, card).expect("Target info must exist");

    assert_eq!(info.tag, "Rect");
    assert_eq!(info.id_name, Some("main_card".to_string()));
    assert_eq!(info.rect.x, 50.0);
    assert_eq!(info.rect.y, 120.0);
    assert_eq!(info.rect.width, 380.0);
    assert_eq!(info.rect.height, 180.0);
    assert_eq!(info.badge_label(), "Rect#main_card [380 × 180]");
}

#[test]
fn test_badge_rect_positioning_and_clamping() {
    let mut dom = Dom::new();

    // Node 1: near top of window (y = 8) -> badge must flip BELOW target
    let top_node = dom.create_element(
        "Rect",
        vec![
            ("x".to_string(), Expr::lit(100.0)),
            ("y".to_string(), Expr::lit(8.0)),
            ("width".to_string(), Expr::lit(120.0)),
            ("height".to_string(), Expr::lit(40.0)),
            ("color".to_string(), Expr::color("#000000")),
        ],
    );

    // Node 2: in normal middle space (y = 200) -> badge floats ABOVE target
    let mid_node = dom.create_element(
        "Rect",
        vec![
            ("x".to_string(), Expr::lit(100.0)),
            ("y".to_string(), Expr::lit(200.0)),
            ("width".to_string(), Expr::lit(120.0)),
            ("height".to_string(), Expr::lit(40.0)),
            ("color".to_string(), Expr::color("#000000")),
        ],
    );

    // Node 3: near right edge of 800px window (x = 750) -> badge clamps horizontally
    let right_node = dom.create_element(
        "Rect",
        vec![
            ("x".to_string(), Expr::lit(750.0)),
            ("y".to_string(), Expr::lit(200.0)),
            ("width".to_string(), Expr::lit(100.0)),
            ("height".to_string(), Expr::lit(40.0)),
            ("color".to_string(), Expr::color("#000000")),
        ],
    );

    dom.append_root(top_node).unwrap();
    dom.append_root(mid_node).unwrap();
    dom.append_root(right_node).unwrap();
    dom.commit().unwrap();

    let info_top = InspectTargetInfo::from_dom(&dom, top_node).unwrap();
    let badge_top = info_top.badge_rect(150.0, 20.0, 800.0, 600.0);
    // Flipped below: y >= top_node.y + top_node.height
    assert!(badge_top.y >= 8.0 + 40.0);

    let info_mid = InspectTargetInfo::from_dom(&dom, mid_node).unwrap();
    let badge_mid = info_mid.badge_rect(150.0, 20.0, 800.0, 600.0);
    // Floats above: y < mid_node.y
    assert!(badge_mid.y < 200.0);

    let info_right = InspectTargetInfo::from_dom(&dom, right_node).unwrap();
    let badge_right = info_right.badge_rect(150.0, 20.0, 800.0, 600.0);
    // Clamped inside window: x + width <= 800.0
    assert!(badge_right.x + badge_right.width <= 800.0);
}

#[test]
fn test_inspector_state_transitions() {
    let mut state = InspectorState::new();

    let h1 = directedtype::dom::NodeHandle::new(1, 1);
    let h2 = directedtype::dom::NodeHandle::new(2, 1);

    assert_eq!(state.active_tab, InspectorTab::Elements);
    assert!(!state.inspect_cursor_active);

    // Toggle cursor
    assert!(state.toggle_inspect_cursor());
    assert!(state.inspect_cursor_active);

    // Hover h1
    assert!(state.set_hovered(Some(h1)));
    assert_eq!(state.primary_highlight_target(), Some((h1, false)));

    // Select h2
    assert!(state.set_selected(Some(h2)));
    // Hover takes precedence while cursor is hovering
    assert_eq!(state.primary_highlight_target(), Some((h1, false)));

    // Unhover -> falls back to selected h2
    assert!(state.set_hovered(None));
    assert_eq!(state.primary_highlight_target(), Some((h2, true)));

    // Toggle tree expansion
    assert!(state.toggle_expanded(h1));
    assert!(state.is_expanded(h1));
    assert!(!state.toggle_expanded(h1));
    assert!(!state.is_expanded(h1));

    state.clear_selection();
    assert_eq!(state.primary_highlight_target(), None);
}

#[test]
fn test_dom_tree_item_building() {
    let mut dom = Dom::new();

    let root_box = dom.create_element(
        "Rect",
        vec![
            ("id".to_string(), Expr::string("main_container")),
            ("x".to_string(), Expr::lit(0.0)),
            ("y".to_string(), Expr::lit(0.0)),
            ("width".to_string(), Expr::lit(500.0)),
            ("height".to_string(), Expr::lit(500.0)),
            ("color".to_string(), Expr::color("#000000")),
        ],
    );
    let card1 = dom.create_element(
        "Rect",
        vec![
            ("id".to_string(), Expr::string("card1")),
            ("x".to_string(), Expr::lit(10.0)),
            ("y".to_string(), Expr::lit(10.0)),
            ("width".to_string(), Expr::lit(300.0)),
            ("height".to_string(), Expr::lit(100.0)),
            ("color".to_string(), Expr::color("#ff0000")),
        ],
    );
    let card2 = dom.create_element(
        "Rect",
        vec![
            ("id".to_string(), Expr::string("card2")),
            ("x".to_string(), Expr::lit(10.0)),
            ("y".to_string(), Expr::lit(120.0)),
            ("width".to_string(), Expr::lit(300.0)),
            ("height".to_string(), Expr::lit(100.0)),
            ("color".to_string(), Expr::color("#00ff00")),
        ],
    );

    dom.append_child(root_box, card1).unwrap();
    dom.append_child(root_box, card2).unwrap();
    dom.append_root(root_box).unwrap();
    dom.commit().unwrap();

    let mut state = InspectorState::new();
    state.set_selected(Some(card1));

    let tree_items = build_tree_items(&dom, &state);

    assert_eq!(tree_items.len(), 3);

    // Root container
    assert_eq!(tree_items[0].tag, "Rect");
    assert_eq!(tree_items[0].depth, 0);
    assert_eq!(tree_items[0].id_name, Some("main_container".to_string()));
    assert!(tree_items[0].has_children);

    // Child card1
    assert_eq!(tree_items[1].tag, "Rect");
    assert_eq!(tree_items[1].depth, 1);
    assert_eq!(tree_items[1].id_name, Some("card1".to_string()));
    assert!(tree_items[1].is_selected);

    // Child card2
    assert_eq!(tree_items[2].tag, "Rect");
    assert_eq!(tree_items[2].depth, 1);
    assert_eq!(tree_items[2].id_name, Some("card2".to_string()));
    assert!(!tree_items[2].is_selected);

    let display_str = tree_items[1].display_text();
    assert!(display_str.contains(r#"\Rect#card1"#));
}

#[test]
fn test_reusable_dtml_components_parse() {
    // Verifies that the reusable DTML inspector component definitions parse cleanly into AST
    directedtype::parse(INSPECT_HIGHLIGHT_DTML).expect("InspectHighlight component must parse");
    directedtype::parse(INSPECT_BADGE_DTML).expect("InspectBadge component must parse");
    directedtype::parse(INSPECT_CLIP_GUIDE_DTML).expect("InspectClipGuide component must parse");
    directedtype::parse(INSPECT_OVERLAY_DTML).expect("InspectOverlay component must parse");

    let combined = standard_inspector_components();
    let doc = directedtype::parse(&combined).expect("Combined standard inspector components must parse");
    assert_eq!(doc.items.len(), 4);
}

#[test]
fn test_overlay_component_build_dtml() {
    let mut dom = Dom::new();
    let target = dom.create_element(
        "Rect",
        vec![
            ("id".to_string(), Expr::string("test_box")),
            ("x".to_string(), Expr::lit(40.0)),
            ("y".to_string(), Expr::lit(60.0)),
            ("width".to_string(), Expr::lit(200.0)),
            ("height".to_string(), Expr::lit(100.0)),
            ("color".to_string(), Expr::color("#000000")),
        ],
    );
    dom.append_root(target).unwrap();
    dom.commit().unwrap();

    let info = InspectTargetInfo::from_dom(&dom, target).unwrap();
    let overlay = InspectOverlayComponent::default();
    let dtml_snippet = overlay.build_dtml(&info, true);

    assert!(dtml_snippet.contains(r#"\InspectOverlay"#));
    assert!(dtml_snippet.contains("target_x: 40"));
    assert!(dtml_snippet.contains("target_y: 60"));
    assert!(dtml_snippet.contains("target_w: 200"));
    assert!(dtml_snippet.contains("target_h: 100"));
    assert!(dtml_snippet.contains(r#"label: "Rect#test_box [200 × 100]""#));
    assert!(dtml_snippet.contains("is_selected: true"));
}

#[test]
fn test_overlay_render_to_scene_dpi_scaling() {
    let mut dom = Dom::new();
    let target = dom.create_element(
        "Rect",
        vec![
            ("id".to_string(), Expr::string("scaled_card")),
            ("x".to_string(), Expr::lit(100.0)),
            ("y".to_string(), Expr::lit(150.0)),
            ("width".to_string(), Expr::lit(200.0)),
            ("height".to_string(), Expr::lit(80.0)),
            ("color".to_string(), Expr::color("#000000")),
        ],
    );
    dom.append_root(target).unwrap();
    dom.commit().unwrap();

    let info = InspectTargetInfo::from_dom(&dom, target).unwrap();
    let overlay = InspectOverlayComponent::default();

    let mut font_cx = parley::FontContext::new();
    let mut layout_cx = parley::LayoutContext::new();

    // 1. Render in logical coordinates (Affine::IDENTITY)
    let mut scene_logical = vello::Scene::new();
    overlay.render_to_scene(
        &mut scene_logical,
        vello::kurbo::Affine::IDENTITY,
        &info,
        false,
        &mut font_cx,
        &mut layout_cx,
        800.0,
        600.0,
    );

    // 2. Render with 2.0x Retina scale factor
    let mut scene_scaled = vello::Scene::new();
    overlay.render_to_scene(
        &mut scene_scaled,
        vello::kurbo::Affine::scale(2.0),
        &info,
        false,
        &mut font_cx,
        &mut layout_cx,
        800.0,
        600.0,
    );
}

#[test]
fn test_build_tree_items_from_layout_and_ancestor_expansion() {
    let source = r#"
        \Component Card(id: String: "c1") {
            \Rect(id: id, x: 0, y: 0, width: 200, height: 100, color: #38bdf8) {
                \Text { "Card Title" }
            }
        }
        \Card(id: "main_card")
    "#;
    let doc = directedtype::parse(source).expect("Source must parse");
    let layout = directedtype::evaluate_document_with_window(&doc, 800.0, 600.0)
        .expect("Layout evaluation must succeed");

    let mut state = InspectorState::new();

    // 1. Initial state: all nodes expanded by default
    let tree_items = directedtype::inspector::build_tree_items_from_layout(&layout, &state);
    assert!(!tree_items.is_empty());

    // Find the Card root and the child Text
    let card_node = tree_items.iter().find(|it| it.tag == "Card").expect("Card must be in tree");
    let card_id = card_node.node_id.expect("Card must have node_id");

    let text_node = tree_items.iter().find(|it| it.tag == "Text").expect("Text must be in tree");
    let text_id = text_node.node_id.expect("Text must have node_id");

    // 2. Collapse the Card node
    assert!(!state.toggle_expanded_id(card_id)); // now collapsed
    assert!(!state.is_expanded_id(card_id));

    let tree_collapsed = directedtype::inspector::build_tree_items_from_layout(&layout, &state);
    // When Card is collapsed, its children (Rect and Text) must not appear in the flattened tree
    assert!(!tree_collapsed.iter().any(|it| it.tag == "Text"));

    // 3. Select child Text from canvas -> expand ancestors
    state.set_selected_id(Some(text_id));
    state.expand_ancestors(text_id, &layout);
    assert!(state.is_expanded_id(card_id)); // Card must be re-expanded!

    let tree_restored = directedtype::inspector::build_tree_items_from_layout(&layout, &state);
    assert!(tree_restored.iter().any(|it| it.tag == "Text"));
}

#[test]
fn test_inspect_panel_component_rendering_and_hit_testing() {
    let source = r#"
        \Rect(id: "hero_box", x: 0, y: 0, width: 400, height: 200, color: #0284c7) {
            \Text { "Hero Headline" }
        }
    "#;
    let doc = directedtype::parse(source).expect("Must parse");
    let layout = directedtype::evaluate_document_with_window(&doc, 1000.0, 700.0).expect("Layout ok");

    let state = InspectorState::new();
    let tree_items = directedtype::inspector::build_tree_items_from_layout(&layout, &state);
    assert!(tree_items.len() >= 2);

    let panel = directedtype::inspector::InspectPanelComponent::default();
    let win_w = 1000.0;
    let win_h = 700.0;
    let panel_x = win_w - panel.width; // 640.0

    // 1. Render test
    let mut scene = vello::Scene::new();
    let mut font_cx = parley::FontContext::new();
    let mut layout_cx = parley::LayoutContext::new();
    panel.render_to_scene(
        &mut scene,
        panel_x,
        win_h,
        &tree_items,
        &state,
        &layout,
        &mut font_cx,
        &mut layout_cx,
    );

    // 2. Hit testing: Header pick button [↖]
    let btn_click = panel.handle_click(
        panel_x + 12.0,
        15.0,
        panel_x,
        win_h,
        &tree_items,
        &state,
        Some(&layout),
    );
    assert_eq!(btn_click, directedtype::inspector::PanelHitResult::ToggleInspectCursor);

    // 3. Hit testing: Chevron click on first item (has children)
    let chevron_click = panel.handle_click(
        panel_x + 12.0,
        panel.header_height + 10.0, // in first row
        panel_x,
        win_h,
        &tree_items,
        &state,
        Some(&layout),
    );
    let root_id = tree_items[0].node_id.unwrap();
    assert_eq!(chevron_click, directedtype::inspector::PanelHitResult::ToggleExpand(root_id));

    // 4. Hit testing: Row selection click (to the right of chevron)
    let row_click = panel.handle_click(
        panel_x + 80.0,
        panel.header_height + 10.0,
        panel_x,
        win_h,
        &tree_items,
        &state,
        Some(&layout),
    );
    assert_eq!(row_click, directedtype::inspector::PanelHitResult::SelectNode(root_id));

    // 5. Mouse move / hover testing
    let hovered = panel.handle_mouse_move(
        panel_x + 80.0,
        panel.header_height + 10.0,
        panel_x,
        win_h,
        &tree_items,
        &state,
    );
    assert_eq!(hovered, Some(root_id));

    // Outside panel
    let hovered_outside = panel.handle_mouse_move(
        100.0,
        100.0,
        panel_x,
        win_h,
        &tree_items,
        &state,
    );
    assert_eq!(hovered_outside, None);
}

#[test]
fn test_inspect_cursor_toggle_and_selection_gating() {
    let source = r#"
        \Component Button {
            \Rect(id: "btn_rect", x: 20, y: 20, width: 120, height: 40, color: #2563eb)
        }
        \Button()
    "#;
    let doc = directedtype::parse(source).expect("Must parse");
    let layout = directedtype::evaluate_document_with_window(&doc, 800.0, 600.0).expect("Layout ok");

    let panel = directedtype::inspector::InspectPanelComponent::default();
    let win_w = 800.0;
    let win_h = 600.0;
    let panel_x = win_w - panel.width; // 440.0

    let mut state = directedtype::inspector::InspectorState::new();

    // 1. By default, inspect_cursor_active must be false (page interaction mode)
    assert!(!state.inspect_cursor_active);
    assert!(!state.inspect_cursor_hovered);

    // 2. Cursor icon button hover hit testing
    assert!(panel.is_cursor_btn_hovered(panel_x + 12.0, 15.0, panel_x));
    assert!(!panel.is_cursor_btn_hovered(panel_x + 60.0, 15.0, panel_x)); // outside button
    assert!(!panel.is_cursor_btn_hovered(100.0, 15.0, panel_x));          // canvas area

    // 3. Clicking cursor button toggles inspect_cursor_active
    let tree_items = directedtype::inspector::build_tree_items_from_layout(&layout, &state);
    let hit_action = panel.handle_click(
        panel_x + 12.0,
        15.0,
        panel_x,
        win_h,
        &tree_items,
        &state,
        Some(&layout),
    );
    assert_eq!(hit_action, directedtype::inspector::PanelHitResult::ToggleInspectCursor);

    assert!(state.toggle_inspect_cursor());
    assert!(state.inspect_cursor_active);

    assert!(!state.toggle_inspect_cursor());
    assert!(!state.inspect_cursor_active);

    state.set_inspect_cursor(true);
    assert!(state.inspect_cursor_active);
    state.set_inspect_cursor(false);
    assert!(!state.inspect_cursor_active);

    // 4. ViewerApp defaults: inspect_cursor_active is false even when inspect_mode is enabled
    let mut app = directedtype::render::ViewerApp::new(layout.clone(), directedtype::render::ViewerConfig::default());
    app.set_inspect_mode(true);

    assert!(app.inspect_mode());
    assert!(!app.inspect_cursor_active());
    assert_eq!(app.selected_node(), None);

    // Activating inspect cursor on app
    app.set_inspect_cursor_active(true);
    assert!(app.inspect_cursor_active());

    // Deactivating inspect cursor on app
    app.set_inspect_cursor_active(false);
    assert!(!app.inspect_cursor_active());
}

#[test]
fn test_inspector_bottom_panel_dimensions_and_formulas() {
    let source = r#"
        \Component Card(id: String: "card1") {
            \Rect(id: id, x: 20 + 5, y: 10 * 2, width: min(400, 300), height: 180, color: #6366f1) {
                \Text { "Card Content" }
            }
        }
        \Card(id: "my_card")
    "#;
    let doc = directedtype::parse(source).expect("Source must parse");
    let layout = directedtype::evaluate_document_with_window(&doc, 800.0, 600.0)
        .expect("Layout evaluation must succeed");

    let mut state = directedtype::inspector::InspectorState::new();

    // 1. Verify dimensions are NOT in the DOM tree items
    let tree_items = directedtype::inspector::build_tree_items_from_layout(&layout, &state);
    assert!(!tree_items.is_empty());
    for item in &tree_items {
        let display = item.display_text();
        assert!(
            !display.contains(" × "),
            "DOM tree item text '{}' must not contain dimensions ' × '",
            display
        );
        assert!(
            !display.contains('[') && !display.contains(']'),
            "DOM tree item text '{}' must not contain dimension brackets '[W × H]'",
            display
        );
    }

    // 2. Verify formulas were captured on ResolvedNode
    let rect_node = layout
        .nodes
        .iter()
        .find(|n| n.name == "Rect")
        .expect("Rect node must exist in layout");

    assert!(
        !rect_node.formulas.is_empty(),
        "ResolvedNode should contain preserved authored formulas"
    );
    // x had expression `20 + 5`
    if let Some(f_x) = rect_node.formulas.get("x") {
        assert!(f_x.contains("20") && f_x.contains('+') && f_x.contains('5'), "Formula for x: {}", f_x);
    }
    // width had expression `min(400, 300)`
    if let Some(f_w) = rect_node.formulas.get("width") {
        assert!(f_w.contains("min"), "Formula for width: {}", f_w);
    }

    // 3. Render bottom panel with selected component
    state.set_selected_id(Some(rect_node.id));

    let panel = directedtype::inspector::InspectPanelComponent::default();
    let win_h = 600.0;
    let win_w = 800.0;
    let panel_x = win_w - panel.width;

    let mut scene = vello::Scene::new();
    let mut font_cx = parley::FontContext::new();
    let mut layout_cx = parley::LayoutContext::new();

    // Render with selection
    panel.render_to_scene(
        &mut scene,
        panel_x,
        win_h,
        &tree_items,
        &state,
        &layout,
        &mut font_cx,
        &mut layout_cx,
    );

    // Verify detail scrolling and max_detail_scroll
    let max_detail = panel.max_detail_scroll(Some(rect_node.id), &layout, win_h);
    assert!(max_detail >= 0.0);

    state.scroll_detail_by(50.0, max_detail);
    assert!(state.detail_scroll_offset <= max_detail);
    state.scroll_detail_by(-100.0, max_detail);
    assert_eq!(state.detail_scroll_offset, 0.0);

    // Render empty state (no selection)
    state.clear_selection();
    let mut scene_empty = vello::Scene::new();
    panel.render_to_scene(
        &mut scene_empty,
        panel_x,
        win_h,
        &tree_items,
        &state,
        &layout,
        &mut font_cx,
        &mut layout_cx,
    );
}

#[test]
fn test_ambient_children_authored_formulas_preserved() {
    let source = r#"
        \Component Flow(margin: Number: 30, gap: Number: 20) {
            \Children {
                x: parent.left + parent.margin,
                y: prev ? prev.bottom + gap : parent.top,
                width: parent.width - (2 * parent.margin)
            }
        }

        \Flow(margin: 40) {
            \Text { "Environmental Scope Showcase" }
        }
    "#;
    let doc = directedtype::parse(source).expect("Source must parse");
    let layout = directedtype::evaluate_document_with_window(&doc, 800.0, 600.0)
        .expect("Layout evaluation must succeed");

    let text_node = layout
        .nodes
        .iter()
        .find(|n| n.name == "Text")
        .expect("Text node must exist in layout");

    assert_eq!(
        text_node.formulas.get("x").map(|s| s.as_str()),
        Some("parent.left + parent.margin"),
        "Child node should preserve authored ambient expression for x"
    );
    assert_eq!(
        text_node.formulas.get("y").map(|s| s.as_str()),
        Some("prev ? prev.bottom + gap : parent.top"),
        "Child node should preserve authored ambient expression for y"
    );
    assert!(
        text_node.formulas.get("width").unwrap().contains("parent.width"),
        "Child node should preserve authored ambient expression for width"
    );
}

#[test]
fn test_env_demo_components_authored_formulas() {
    let source = std::fs::read_to_string("examples/env_demo.dt").expect("read ok");
    let doc = directedtype::parse(&source).expect("parse ok");
    let layout = directedtype::evaluate_document_with_window(&doc, 800.0, 600.0).expect("layout ok");

    let mut failed = Vec::new();
    for node in &layout.nodes {
        for (port, formula) in &node.formulas {
            if formula.contains("__node_") {
                failed.push(format!("Node {} (id: {:?}): {} -> {}", node.name, node.id, port, formula));
            }
        }
    }
    assert!(
        failed.is_empty(),
        "Found formulas containing __node_:\n{}",
        failed.join("\n")
    );

    let theme_provider = layout.nodes.iter().find(|n| n.name == "ThemeProvider").unwrap();
    assert_eq!(
        theme_provider.formulas.get("x").map(|s| s.as_str()),
        Some("parent.left + parent.margin"),
        "ThemeProvider must have authored formula for x"
    );

    let ambient_card = layout.nodes.iter().find(|n| n.name == "AmbientCard").unwrap();
    assert_eq!(
        ambient_card.formulas.get("x").map(|s| s.as_str()),
        Some("parent.left"),
        "AmbientCard must have authored formula for x"
    );
}

#[test]
fn test_badge_text_typographic_centering() {
    use parley::style::{FontWeight, StyleProperty};
    use parley::{Alignment, FontContext, LayoutContext};

    let mut font_cx = FontContext::new();
    let mut layout_cx = LayoutContext::<()>::new();

    let font_size = 11.0f32;
    let badge_height = 20.0f64;
    let text = "Rect#main [380 × 180]";

    let mut builder = layout_cx.ranged_builder(&mut font_cx, text, 1.0, true);
    builder.push_default(StyleProperty::FontSize(font_size));
    builder.push_default(StyleProperty::FontWeight(FontWeight::BOLD));

    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, parley::layout::AlignmentOptions::default());

    let cap_height = (font_size * 0.71) as f64;
    let line = layout.lines().next().unwrap();
    let m = line.metrics();
    let baseline_offset = m.baseline as f64;
    let descent = m.descent as f64;

    let v_pad = ((badge_height - cap_height) / 2.0).max(descent);
    let target_baseline = v_pad + cap_height;
    let text_y_offset = target_baseline - baseline_offset;

    // Check 1: Top of capital letters has identical padding to space below baseline
    let cap_top = text_y_offset + baseline_offset - cap_height;
    let bottom_to_baseline = badge_height - (text_y_offset + baseline_offset);
    assert!((cap_top - v_pad).abs() < 1e-4, "Top padding should equal v_pad");
    assert!((bottom_to_baseline - v_pad).abs() < 1e-4, "Bottom space below baseline should equal v_pad");
    assert!((cap_top - bottom_to_baseline).abs() < 1e-4, "Capital letters must be vertically centered");

    // Check 2: Descenders must be enclosed inside the badge
    let descender_bottom = text_y_offset + baseline_offset + descent;
    assert!(descender_bottom < badge_height, "Descenders must not exceed badge bounds");
    assert!(badge_height - descender_bottom >= 2.0, "Descenders must have comfortable clearance");
}

#[test]
fn test_dom_tree_highlight_and_baseline_alignment() {
    let panel = directedtype::inspector::InspectPanelComponent::default();
    assert_eq!(panel.row_height, 18.0, "DOM tree row height should be 18px for tight modern DevTools aesthetics");

    // For 10.5pt Menlo in 18px row:
    let font_size = 10.5;
    let cap_height = font_size * 0.71;
    let v_pad = (panel.row_height - cap_height) / 2.0;
    let target_baseline = panel.row_height - v_pad;

    // Padding above capitals must equal padding below baseline
    let cap_top = target_baseline - cap_height;
    let bottom_space = panel.row_height - target_baseline;
    assert!((cap_top - bottom_space).abs() < 1e-4, "Highlight must be vertically symmetric around capital letters and baseline");

    // Descenders (~2.5px) must be comfortably enclosed
    let descent = 2.48;
    assert!(v_pad > descent, "Row padding must enclose descenders");
    assert!(panel.row_height - (target_baseline + descent) > 2.0, "Clearance below descenders must exist");
}

#[test]
fn test_property_bullet_vertical_centering() {
    // In property rows, text baseline is at cur_y + 11.5.
    // x-height for 10.5pt Menlo is ~5.73px (from cur_y + 5.77 to cur_y + 11.5).
    // The bullet center is at cur_y + 8.65, which is midway between x-top (cur_y + 5.77) and baseline (cur_y + 11.5).
    let baseline_y: f64 = 11.5;
    let x_height: f64 = 5.73;
    let x_top: f64 = baseline_y - x_height; // 5.77
    let bullet_cy: f64 = 8.65;
    let bullet_radius: f64 = 2.0;

    // Bullet center must be distinctly below the top of lowercase x
    assert!(bullet_cy > x_top + 1.0, "Bullet must be centered lower than the top of lowercase x");

    // Bullet center must be at the optical midpoint of lowercase letters (x-height center)
    let optical_center: f64 = baseline_y - (x_height / 2.0); // 8.635
    assert!((bullet_cy - optical_center).abs() < 0.1, "Bullet center must align with optical x-height midpoint");

    // Entire bullet circle must be inside the lowercase x-height vertical span
    assert!(bullet_cy - bullet_radius > x_top, "Top of bullet must not exceed top of lowercase letters");
    assert!(bullet_cy + bullet_radius < baseline_y, "Bottom of bullet must not cross baseline");
}

#[test]
fn test_let_bound_var_name_in_dom_tree() {
    let source = r#"
        let heading_font = \Font(size: 24, weight: 700, family: "Inter")
        \Text(font: heading_font, text: "Hello World")
    "#;
    let doc = directedtype::parse(source).expect("Parse ok");
    let layout = directedtype::evaluate_document_with_window(&doc, 800.0, 600.0).expect("Layout ok");

    let state = directedtype::inspector::InspectorState::new();
    let tree_items = directedtype::inspector::build_tree_items_from_layout(&layout, &state);

    // One of the tree items should be the let-bound Font node with var_name "heading_font"
    let font_item = tree_items.iter().find(|it| it.var_name.as_deref() == Some("heading_font"));
    assert!(font_item.is_some(), "heading_font must be present in DOM tree items");
    let item = font_item.unwrap();
    assert_eq!(item.display_text().trim(), "heading_font: \\Font");
}

#[test]
fn test_expandable_reference_properties() {
    let source = r#"
        let heading_font = \Font(size: 24, weight: 700, family: "Inter")
        \Text(id: "my_text", font: heading_font, text: "Hello World")
    "#;
    let doc = directedtype::parse(source).expect("Parse ok");
    let layout = directedtype::evaluate_document_with_window(&doc, 800.0, 600.0).expect("Layout ok");

    let text_node = layout.nodes.iter().find(|n| n.name == "Text").expect("Text node found");
    assert!(text_node.properties.contains_key("font"));
    assert!(matches!(text_node.properties.get("font"), Some(directedtype::compiler::Value::Node(_))));

    let mut state = directedtype::inspector::InspectorState::new();
    state.set_selected_id(Some(text_node.id));

    // Initially, property ref is not expanded
    assert!(!state.is_property_ref_expanded(text_node.id, "font"));

    // Toggle expansion
    state.toggle_property_ref_expanded(text_node.id, "font");
    assert!(state.is_property_ref_expanded(text_node.id, "font"));

    // Toggle again collapses it
    state.toggle_property_ref_expanded(text_node.id, "font");
    assert!(!state.is_property_ref_expanded(text_node.id, "font"));

    // Hit testing click in details panel toggles expansion
    let panel = directedtype::inspector::InspectPanelComponent::default();
    let win_w = 800.0;
    let win_h = 600.0;
    let panel_x = win_w - panel.width;
    let tree_items = directedtype::inspector::build_tree_items_from_layout(&layout, &state);

    let divider_y = panel.divider_y(win_h);
    let detail_y = divider_y + panel.divider_height;
    let section_b_start = detail_y + 134.0;
    let mut test_y = section_b_start;
    let mut prop_keys: Vec<String> = text_node.properties.keys().cloned().collect();
    prop_keys.retain(|k| k != "clip");
    for geom in ["x", "y", "width", "height", "z"] {
        if !prop_keys.contains(&geom.to_string()) {
            prop_keys.push(geom.to_string());
        }
    }
    prop_keys.sort_by(|a, b| {
        let rank = |k: &str| match k {
            "x" => 1,
            "y" => 2,
            "width" => 3,
            "height" => 4,
            "z" => 5,
            "color" | "bg_color" => 6,
            "border_color" | "border_width" => 7,
            "radius" | "corner_radius" => 8,
            "clip" => 9,
            _ => 10,
        };
        rank(a).cmp(&rank(b)).then_with(|| a.cmp(b))
    });
    for key in &prop_keys {
        let val = text_node.properties.get(key);
        let eval_str = if let Some(v) = val {
            match v {
                directedtype::compiler::Value::Node(ref_id) => {
                    if let Some(rn) = layout.get_node(*ref_id) {
                        if let Some(var) = &rn.var_name {
                            format!("{var} (\\{})", rn.name)
                        } else {
                            format!("\\{}", rn.name)
                        }
                    } else {
                        format!("{v}")
                    }
                }
                _ => format!("{v}"),
            }
        } else {
            String::new()
        };
        let formula_str = text_node.formulas.get(key).cloned().unwrap_or_default();
        let has_formula = !formula_str.is_empty() && formula_str != eval_str;
        let row_h = if has_formula { 36.0 } else { 22.0 };
        if key == "font" {
            test_y += 5.0; // Click inside this row!
            break;
        }
        test_y += row_h;
    }

    let click_res = panel.handle_click(
        panel_x + 50.0,
        test_y,
        panel_x,
        win_h,
        &tree_items,
        &state,
        Some(&layout),
    );
    assert_eq!(click_res, directedtype::inspector::PanelHitResult::TogglePropertyRef(text_node.id, "font".to_string()));
}


