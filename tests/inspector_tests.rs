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


