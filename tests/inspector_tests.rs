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
            ("x".to_string(), Expr::distance_px(50.0)),
            ("y".to_string(), Expr::distance_px(120.0)),
            ("width".to_string(), Expr::distance_px(380.0)),
            ("height".to_string(), Expr::distance_px(180.0)),
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
            ("x".to_string(), Expr::distance_px(100.0)),
            ("y".to_string(), Expr::distance_px(8.0)),
            ("width".to_string(), Expr::distance_px(120.0)),
            ("height".to_string(), Expr::distance_px(40.0)),
            ("color".to_string(), Expr::color("#000000")),
        ],
    );

    // Node 2: in normal middle space (y = 200) -> badge floats ABOVE target
    let mid_node = dom.create_element(
        "Rect",
        vec![
            ("x".to_string(), Expr::distance_px(100.0)),
            ("y".to_string(), Expr::distance_px(200.0)),
            ("width".to_string(), Expr::distance_px(120.0)),
            ("height".to_string(), Expr::distance_px(40.0)),
            ("color".to_string(), Expr::color("#000000")),
        ],
    );

    // Node 3: near right edge of 800px window (x = 750) -> badge clamps horizontally
    let right_node = dom.create_element(
        "Rect",
        vec![
            ("x".to_string(), Expr::distance_px(750.0)),
            ("y".to_string(), Expr::distance_px(200.0)),
            ("width".to_string(), Expr::distance_px(100.0)),
            ("height".to_string(), Expr::distance_px(40.0)),
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
            ("x".to_string(), Expr::distance_px(0.0)),
            ("y".to_string(), Expr::distance_px(0.0)),
            ("width".to_string(), Expr::distance_px(500.0)),
            ("height".to_string(), Expr::distance_px(500.0)),
            ("color".to_string(), Expr::color("#000000")),
        ],
    );
    let card1 = dom.create_element(
        "Rect",
        vec![
            ("id".to_string(), Expr::string("card1")),
            ("x".to_string(), Expr::distance_px(10.0)),
            ("y".to_string(), Expr::distance_px(10.0)),
            ("width".to_string(), Expr::distance_px(300.0)),
            ("height".to_string(), Expr::distance_px(100.0)),
            ("color".to_string(), Expr::color("#ff0000")),
        ],
    );
    let card2 = dom.create_element(
        "Rect",
        vec![
            ("id".to_string(), Expr::string("card2")),
            ("x".to_string(), Expr::distance_px(10.0)),
            ("y".to_string(), Expr::distance_px(120.0)),
            ("width".to_string(), Expr::distance_px(300.0)),
            ("height".to_string(), Expr::distance_px(100.0)),
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
            ("x".to_string(), Expr::distance_px(40.0)),
            ("y".to_string(), Expr::distance_px(60.0)),
            ("width".to_string(), Expr::distance_px(200.0)),
            ("height".to_string(), Expr::distance_px(100.0)),
            ("color".to_string(), Expr::color("#000000")),
        ],
    );
    dom.append_root(target).unwrap();
    dom.commit().unwrap();

    let info = InspectTargetInfo::from_dom(&dom, target).unwrap();
    let overlay = InspectOverlayComponent::default();
    let dtml_snippet = overlay.build_dtml(&info, true);

    assert!(dtml_snippet.contains(r#"\InspectOverlay"#));
    assert!(dtml_snippet.contains("target_x: 40px"));
    assert!(dtml_snippet.contains("target_y: 60px"));
    assert!(dtml_snippet.contains("target_w: 200px"));
    assert!(dtml_snippet.contains("target_h: 100px"));
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
            ("x".to_string(), Expr::distance_px(100.0)),
            ("y".to_string(), Expr::distance_px(150.0)),
            ("width".to_string(), Expr::distance_px(200.0)),
            ("height".to_string(), Expr::distance_px(80.0)),
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
        \Component Card(id: String = "c1") {
            \Rect(id: id, x: 0, y: 0, width: 200px, height: 100px, color: #38bdf8) {
                \Text(font: \Font(size: 14px)) { "Card Title" }
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
        \Rect(id: "hero_box", x: 0, y: 0, width: 400px, height: 200px, color: #0284c7) {
            \Text(font: \Font(size: 24px)) { "Hero Headline" }
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

    // 2. Hit testing: Header pick button [↖] (placed to the right of "DOM ELEMENTS")
    let btn_click = panel.handle_click(
        panel_x + 100.0,
        13.0,
        panel_x,
        win_h,
        &tree_items,
        &state,
        Some(&layout),
    );
    assert_eq!(btn_click, directedtype::inspector::PanelHitResult::ToggleInspectCursor);

    // 2b. Hit testing: Header close button [×] (right-aligned)
    let close_click = panel.handle_click(
        panel_x + panel.width - 18.0,
        13.0,
        panel_x,
        win_h,
        &tree_items,
        &state,
        Some(&layout),
    );
    assert_eq!(close_click, directedtype::inspector::PanelHitResult::CloseInspector);

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
            \Rect(id: "btn_rect", x: 20px, y: 20px, width: 120px, height: 40px, color: #2563eb)
        }
        \Button()
    "#;
    let doc = directedtype::parse(source).expect("Must parse");
    let layout = directedtype::evaluate_document_with_window(&doc, 800.0, 600.0).expect("Layout ok");

    let panel = directedtype::inspector::InspectPanelComponent::default();
    let win_w = 800.0;
    let win_h = 600.0;
    let panel_x = win_w - panel.width; // 420.0

    let mut state = directedtype::inspector::InspectorState::new();

    // 1. By default, inspect_cursor_active must be false (page interaction mode)
    assert!(!state.inspect_cursor_active);
    assert!(!state.inspect_cursor_hovered);

    // 2. Cursor icon button hover hit testing
    assert!(panel.is_cursor_btn_hovered(panel_x + 100.0, 13.0, panel_x));
    assert!(!panel.is_cursor_btn_hovered(panel_x + 20.0, 13.0, panel_x));  // on title text, outside button
    assert!(!panel.is_cursor_btn_hovered(100.0, 13.0, panel_x));           // canvas area

    // 3. Clicking cursor button toggles inspect_cursor_active
    let tree_items = directedtype::inspector::build_tree_items_from_layout(&layout, &state);
    let hit_action = panel.handle_click(
        panel_x + 100.0,
        13.0,
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
        \Component Card(id: String = "card1") {
            \Rect(id: id, x: 20px + 5px, y: 10px * 2, width: min(400px, 300px), height: 180px, color: #6366f1) {
                \Text(font: \Font(size: 14px)) { "Card Content" }
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
    // x had expression `20px + 5px`
    if let Some(f_x) = rect_node.formulas.get("x") {
        assert!(f_x.contains("20") && f_x.contains('+') && f_x.contains('5'), "Formula for x: {}", f_x);
    }
    // width had expression `min(400px, 300px)`
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
        \Component Flow(x: Distance = 0, y: Distance = 0, width: Distance = parent.width, margin: Distance = 30px, gap: Distance = 20px) {
            \Children(
                x: left + margin,
                y: prev ? prev.bottom + gap : top,
                width: width - (2 * margin)
            )
        }

        \Flow(margin: 40px) {
            \Text(font: \Font(size: 16px)) { "Environmental Scope Showcase" }
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
        Some("left + margin"),
        "Child node should preserve authored ambient expression for x"
    );
    assert_eq!(
        text_node.formulas.get("y").map(|s| s.as_str()),
        Some("prev ? prev.bottom + gap : top"),
        "Child node should preserve authored ambient expression for y"
    );
    assert!(
        text_node.formulas.get("width").unwrap().contains("width"),
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
        Some("left + margin"),
        "ThemeProvider must have authored formula for x"
    );

    let ambient_card = layout.nodes.iter().find(|n| n.name == "AmbientCard").unwrap();
    assert_eq!(
        ambient_card.formulas.get("x").map(|s| s.as_str()),
        Some("left"),
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
        \let heading_font = \Font(size: 24px, weight: 700, family: "Arial")
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
        \let heading_font = \Font(size: 24px, weight: 700, family: "Arial")
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
    let section_b_start = detail_y + 10.0 + 18.0;
    let mut test_y = section_b_start + 10.0 + 13.0;
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
            "left" => 6,
            "top" => 7,
            "right" => 8,
            "bottom" => 9,
            "color" | "bg_color" => 10,
            "border_color" | "border_width" => 11,
            "radius" | "corner_radius" => 12,
            "clip" => 13,
            _ => 14,
        };
        rank(a).cmp(&rank(b)).then_with(|| a.cmp(b))
    });
    for key in &prop_keys {
        let row_h = 18.0;
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

#[test]
fn test_dom_tree_item_structured_keys() {
    let source = r#"
\Component Cell(w: Distance = 80px, h: Distance = 30px) {
    \Rect(x: 0, y: 0, width: w, height: h, color: #ffffff)
}

\Component Table {
    \Children
}

\Table("main";) {
    \Cell(0, 0; w: 80px, h: 30px)
    \Cell(0, 1; w: 80px, h: 30px)
}
"#;
    let doc = directedtype::parse(source).expect("parse ok");
    let layout = directedtype::compiler::evaluate_document(&doc).expect("eval layout ok");

    let state = directedtype::inspector::InspectorState::new();
    let tree_items = directedtype::inspector::build_tree_items_from_layout(&layout, &state);

    // Tree item for Table should have #main
    let table_item = tree_items.iter().find(|i| i.tag == "Table").expect("Table found");
    assert!(table_item.key.is_some());
    assert_eq!(table_item.key.as_ref().unwrap().format_key(), "main");
    let table_display = table_item.display_text();
    assert!(table_display.contains(r#"\Table#main"#), "got: {table_display}");

    // Tree items for Cells should have #(0, 0) and #(0, 1)
    let cell_0_0 = tree_items.iter().find(|i| i.tag == "Cell" && i.display_text().contains("#(0, 0)")).expect("Cell 0,0 found");
    assert_eq!(cell_0_0.key.as_ref().unwrap().format_key(), "(0, 0)");

    let cell_0_1 = tree_items.iter().find(|i| i.tag == "Cell" && i.display_text().contains("#(0, 1)")).expect("Cell 0,1 found");
    assert_eq!(cell_0_1.key.as_ref().unwrap().format_key(), "(0, 1)");

    // Target info badge label
    let table_info = InspectTargetInfo::from_layout(&layout, table_item.node_id.unwrap()).unwrap();
    assert!(table_info.badge_label().starts_with("Table#main ["));

    let cell_info = InspectTargetInfo::from_layout(&layout, cell_0_0.node_id.unwrap()).unwrap();
    assert!(cell_info.badge_label().starts_with("Cell#(0, 0) ["));
}

#[test]
fn test_inspector_target_info_state_variables() {
    let source = r#"
\Component Counter(initial: Integer = 7) {
    \state count: Integer = initial;
    \state is_active: Boolean = true;
    \Rect(x: 0, y: 0, width: 200px, height: 100px, color: #ffffff)
}

\Counter(initial: 42)
"#;
    let doc = directedtype::parse(source).expect("parse ok");
    let layout = directedtype::compiler::evaluate_document(&doc).expect("eval layout ok");

    let counter_node = layout.nodes.iter().find(|n| n.name == "Counter").expect("Counter found");
    let info = InspectTargetInfo::from_layout(&layout, counter_node.id).expect("target info ok");

    assert_eq!(info.state_vars.len(), 2);
    let count_state = info.state_vars.iter().find(|s| s.0 == "count").expect("count found");
    assert_eq!(count_state.1, Some("Integer".to_string()));
    assert_eq!(count_state.2, Some(directedtype::compiler::Value::Integer(42)));

    let active_state = info.state_vars.iter().find(|s| s.0 == "is_active").expect("is_active found");
    assert_eq!(active_state.1, Some("Boolean".to_string()));
    assert_eq!(active_state.2, Some(directedtype::compiler::Value::Bool(true)));

    // Test with stateful DOM mutation
    let mut dom = directedtype::dom::Dom::from_document(&doc).expect("dom from doc ok");
    dom.commit().expect("dom commit ok");
    let counter_handle = dom.roots()[0];
    let initial_info = InspectTargetInfo::from_dom(&dom, counter_handle).expect("dom info ok");
    assert_eq!(
        initial_info.state_vars.iter().find(|s| s.0 == "count").unwrap().2,
        Some(directedtype::compiler::Value::Integer(42))
    );

    // Mutate state in DOM
    dom.set_state(counter_handle, "count", directedtype::compiler::Value::Integer(99)).expect("set_state ok");
    let updated_info = InspectTargetInfo::from_dom(&dom, counter_handle).expect("updated dom info ok");
    assert_eq!(
        updated_info.state_vars.iter().find(|s| s.0 == "count").unwrap().2,
        Some(directedtype::compiler::Value::Integer(99))
    );
}

#[test]
fn test_inspector_panel_render_state_variables_and_structured_keys() {
    let source = r#"
\Component Switch {
    \state is_on: Boolean = false;
    \Rect(x: 0, y: 0, width: 100px, height: 40px, color: #112233)
}

\Switch("toggle_btn";)
"#;
    let doc = directedtype::parse(source).expect("parse ok");
    let layout = directedtype::compiler::evaluate_document(&doc).expect("eval layout ok");

    let switch_node = layout.nodes.iter().find(|n| n.name == "Switch").expect("Switch found");

    let mut state = directedtype::inspector::InspectorState::new();
    state.selected_id = Some(switch_node.id);

    let panel = directedtype::inspector::InspectPanelComponent::default();
    let tree_items = directedtype::inspector::build_tree_items_from_layout(&layout, &state);

    // Tree item should have #toggle_btn
    assert!(tree_items[0].display_text().contains(r#"\Switch#toggle_btn"#));

    // Verify detail scroll calculation accounts for state vars section
    let scroll_h = panel.max_detail_scroll_with_state(Some(switch_node.id), &layout, Some(&state), 400.0);
    // Since switch_node has 1 state variable (is_on), content_h includes state section (18.0 + 24.0 + 10.0 = 52.0px extra)
    assert!(scroll_h >= 0.0);

    // Render panel to Vello scene
    let mut scene = vello::Scene::new();
    let mut font_cx = parley::FontContext::new();
    let mut layout_cx = parley::LayoutContext::new();

    panel.render_to_scene(
        &mut scene,
        0.0,
        600.0,
        &tree_items,
        &state,
        &layout,
        &mut font_cx,
        &mut layout_cx,
    );

    let mut renderer = directedtype::render::HeadlessRenderer::new().expect("init renderer");
    let img = renderer.render_scene(&scene, 360, 600).expect("render scene ok");
    assert_eq!(img.width(), 360);
    assert_eq!(img.height(), 600);
}

#[test]
fn test_dag_property_trace_building() {
    let source = r#"
\Component Container(x: Distance = 0, y: Distance = 0, width: Distance = 400px) {
    \Rect("box"; x: x, y: y, width: width - 60px, height: 80px, color: #3b82f6)
}
\Container("container"; x: 0, y: 0, width: 400px)
"#;
    let doc = directedtype::parse(source).expect("parse ok");
    let layout = directedtype::compiler::evaluate_document(&doc).expect("eval layout ok");

    let box_node = layout.nodes.iter().find(|n| n.name == "Rect").expect("Rect found");
    let container_node = layout.nodes.iter().find(|n| n.name == "Container").expect("Container found");

    // Trace 'width' on box node (computed expression: width - 60px = 340px)
    let box_trace = directedtype::inspector::build_property_dag_trace(&layout, box_node.id, "width")
        .expect("box width trace ok");

    assert_eq!(box_trace.port_name, "width");
    assert_eq!(box_trace.evaluated_value_str, "340px");
    assert_eq!(box_trace.value_type, "Distance");
    assert_eq!(box_trace.origin_kind, directedtype::inspector::PortOriginKind::Expression);
    assert_eq!(box_trace.upstream_dependencies.len(), 1);
    assert_eq!(box_trace.upstream_dependencies[0].node_id, container_node.id);
    assert_eq!(box_trace.upstream_dependencies[0].port_name, "width");
    assert_eq!(box_trace.upstream_dependencies[0].value_str, "400px");

    // Trace 'width' on container node (authored literal: 400px)
    let container_trace = directedtype::inspector::build_property_dag_trace(&layout, container_node.id, "width")
        .expect("container width trace ok");

    assert_eq!(container_trace.port_name, "width");
    assert_eq!(container_trace.evaluated_value_str, "400px");
    assert_eq!(container_trace.origin_kind, directedtype::inspector::PortOriginKind::Literal);
    assert!(container_trace.upstream_dependencies.is_empty());
    // Box width depends downstream on container width
    assert!(container_trace.downstream_dependents.iter().any(|d| d.node_id == box_node.id && d.port_name == "width"));
}

#[test]
fn test_dag_property_trace_inferred_default() {
    let source = r#"
\let b = \Box(width: 100px, height: 40px)
"#;
    let doc = directedtype::parse(source).expect("parse ok");
    let layout = directedtype::compiler::evaluate_document(&doc).expect("eval layout ok");
    let box_node = layout.nodes.iter().find(|n| n.name == "Box").expect("Box found");

    // Coordinate 'x' was not explicitly authored on the Box primitive, so it gets the default spatial coordinate (0px)
    let trace = directedtype::inspector::build_property_dag_trace(&layout, box_node.id, "x")
        .expect("trace ok");

    assert_eq!(trace.port_name, "x");
    assert_eq!(trace.evaluated_value_str, "0px");
    assert_eq!(trace.origin_kind, directedtype::inspector::PortOriginKind::InferredDefault);
    assert!(trace.upstream_dependencies.is_empty());
    assert!(trace.origin_description.contains("Default spatial coordinate"));
}

#[test]
fn test_inspector_state_property_selection() {
    let mut state = directedtype::inspector::InspectorState::new();
    let node_a = directedtype::compiler::expanded::NodeId(1);
    let node_b = directedtype::compiler::expanded::NodeId(2);

    state.selected_id = Some(node_a);
    state.detail_scroll_offset = 50.0;

    state.select_property(node_a, "width");
    assert_eq!(state.selected_property, Some((node_a, "width".to_string())));
    assert_eq!(state.detail_scroll_offset, 0.0);

    state.clear_selected_property();
    assert_eq!(state.selected_property, None);

    state.select_property(node_a, "height");
    // Changing selected_id clears property selection
    state.set_selected_id(Some(node_b));
    assert_eq!(state.selected_property, None);
}

#[test]
fn test_panel_hit_testing_property_details_and_back() {
    let source = r#"
\Component Container(x: Distance = 0, y: Distance = 0, width: Distance = 400px) {
    \Rect("box"; x: x, y: y, width: width - 60px, height: 80px, color: #3b82f6)
}
\Container("container"; x: 0, y: 0, width: 400px)
"#;
    let doc = directedtype::parse(source).expect("parse ok");
    let layout = directedtype::compiler::evaluate_document(&doc).expect("eval layout ok");
    let box_node = layout.nodes.iter().find(|n| n.name == "Rect").expect("Rect found");
    let container_node = layout.nodes.iter().find(|n| n.name == "Container").expect("Container found");

    let panel = directedtype::inspector::InspectPanelComponent::default();
    let mut state = directedtype::inspector::InspectorState::new();
    state.selected_id = Some(box_node.id);
    let tree_items = directedtype::inspector::build_tree_items_from_layout(&layout, &state);

    let divider_y = panel.divider_y(600.0);
    let detail_y = divider_y + panel.divider_height;

    // 1. Click in component details on 'width' property row (3rd row after x, y, below 18px overload signature and PROPERTIES header)
    let width_row_y = detail_y + 10.0 + 18.0 + 10.0 + 13.0 + 2.0 * 18.0 + 5.0;
    let res = panel.handle_click(
        50.0,
        width_row_y,
        0.0,
        600.0,
        &tree_items,
        &state,
        Some(&layout),
    );
    assert_eq!(res, directedtype::inspector::PanelHitResult::SelectProperty(box_node.id, "width".to_string()));

    // 2. Set state to property details view
    state.select_property(box_node.id, "width");

    // 3. Click [←] button in the property title bar (at panel.width - 56.0 .. panel.width - 34.0)
    let back_btn_y = divider_y + panel.divider_height / 2.0;
    let back_res = panel.handle_click(
        panel.width - 45.0,
        back_btn_y,
        0.0,
        600.0,
        &tree_items,
        &state,
        Some(&layout),
    );
    assert_eq!(back_res, directedtype::inspector::PanelHitResult::BackToComponentDetails);

    // 4. Click upstream dependency row (container width)
    let upstream_row_y = detail_y + 10.0 + 48.0 + 14.0 + 18.0 + 8.0;
    let dep_res = panel.handle_click(
        50.0,
        upstream_row_y,
        0.0,
        600.0,
        &tree_items,
        &state,
        Some(&layout),
    );
    assert_eq!(
        dep_res,
        directedtype::inspector::PanelHitResult::PushProperty(
            container_node.id,
            "width".to_string(),
            directedtype::inspector::PropertyTraceDirection::Upstream,
        )
    );

    // 5. Click downstream dependent row (box right)
    let downstream_row_y = detail_y + 10.0 + 48.0 + 14.0 + 18.0 + 18.0 + 10.0 + 18.0 + 8.0;
    let down_res = panel.handle_click(
        50.0,
        downstream_row_y,
        0.0,
        600.0,
        &tree_items,
        &state,
        Some(&layout),
    );
    assert_eq!(
        down_res,
        directedtype::inspector::PanelHitResult::PushProperty(
            box_node.id,
            "right".to_string(),
            directedtype::inspector::PropertyTraceDirection::Downstream,
        )
    );
}

#[test]
fn test_property_stack_and_history_navigation() {
    use directedtype::inspector::{InspectPanelComponent, InspectorState, PanelHitResult, PropertyTraceDirection};

    let source = r#"
\Component Outer(x: Distance = 0, y: Distance = 0, w: Distance = 500px) {
    \Inner("inner"; x: x, y: y, width: w - 40px)
}
\Component Inner(x: Distance = 0, y: Distance = 0, width: Distance = 460px) {
    \Rect("leaf"; x: x, y: y, width: width - 20px, height: 60px, color: #3b82f6)
}
\Outer("outer"; x: 0, y: 0, w: 500px)
"#;
    let doc = directedtype::parse(source).expect("parse ok");
    let layout = directedtype::compiler::evaluate_document(&doc).expect("eval layout ok");
    let leaf_node = layout.nodes.iter().find(|n| n.name == "Rect").expect("Rect found");
    let inner_node = layout.nodes.iter().find(|n| n.name == "Inner").expect("Inner found");
    let outer_node = layout.nodes.iter().find(|n| n.name == "Outer").expect("Outer found");

    let panel = InspectPanelComponent::default();
    let mut state = InspectorState::new();
    state.selected_id = Some(leaf_node.id);
    let tree_items = directedtype::inspector::build_tree_items_from_layout(&layout, &state);

    let divider_y = panel.divider_y(600.0);
    let detail_y = divider_y + panel.divider_height;
    let btn_y = divider_y + panel.divider_height / 2.0;
    let back_x = panel.width - 45.0;
    let fwd_x = panel.width - 18.0;

    // 1. Start at property A (leaf.width)
    state.select_property(leaf_node.id, "width");
    assert_eq!(state.active_property_stack(), &[(leaf_node.id, "width".to_string())]);
    assert!(state.can_go_back());
    assert!(!state.can_go_forward());

    // 2. Click upstream property B (inner.width) -> pushes onto stack [A, B]
    let upstream_b_y = detail_y + 10.0 + (48.0 + 14.0) + 18.0 + 5.0;
    let hit_b = panel.handle_click(50.0, upstream_b_y, 0.0, 600.0, &tree_items, &state, Some(&layout));
    assert_eq!(hit_b, PanelHitResult::PushProperty(inner_node.id, "width".to_string(), PropertyTraceDirection::Upstream));
    state.push_property(inner_node.id, "width", PropertyTraceDirection::Upstream);

    assert_eq!(
        state.active_property_stack(),
        &[
            (leaf_node.id, "width".to_string()),
            (inner_node.id, "width".to_string()),
        ]
    );
    assert_eq!(state.property_trace_direction, Some(PropertyTraceDirection::Upstream));

    // 3. Now 2 cards are stacked (A and B), followed ONLY by UPSTREAM of B (outer.w)
    // Card A: 48 + 8 = 56; Card B: 48 + 14 = 62 -> total cards = 118.0
    let upstream_c_y = detail_y + 10.0 + (48.0 + 8.0) + (48.0 + 14.0) + 18.0 + 5.0;
    let hit_c = panel.handle_click(50.0, upstream_c_y, 0.0, 600.0, &tree_items, &state, Some(&layout));
    assert_eq!(hit_c, PanelHitResult::PushProperty(outer_node.id, "w".to_string(), PropertyTraceDirection::Upstream));
    state.push_property(outer_node.id, "w", PropertyTraceDirection::Upstream);

    assert_eq!(
        state.active_property_stack(),
        &[
            (leaf_node.id, "width".to_string()),
            (inner_node.id, "width".to_string()),
            (outer_node.id, "w".to_string()),
        ]
    );

    // 4. Navigate Back from [A, B, C] -> [A, B]
    let hit_back_1 = panel.handle_click(back_x, btn_y, 0.0, 600.0, &tree_items, &state, Some(&layout));
    assert_eq!(hit_back_1, PanelHitResult::BackToComponentDetails);
    assert!(state.navigate_property_back());
    assert_eq!(state.active_property_stack().len(), 2);
    assert!(state.can_go_forward());

    // 5. Navigate Back from [A, B] -> [A]
    assert!(state.navigate_property_back());
    assert_eq!(state.active_property_stack().len(), 1);
    assert!(state.can_go_forward());

    // 6. Navigate Back from [A] -> Component Details (stack empty, selected_property is None)
    assert!(state.navigate_property_back());
    assert_eq!(state.active_property_stack().len(), 0);
    assert_eq!(state.selected_property, None);
    assert!(!state.can_go_back());
    assert!(state.can_go_forward());

    // 7. In Component Details, Forward button [→] is active and returns to [A]
    let hit_fwd_1 = panel.handle_click(fwd_x, btn_y, 0.0, 600.0, &tree_items, &state, Some(&layout));
    assert_eq!(hit_fwd_1, PanelHitResult::ForwardPropertyHistory);
    assert!(state.navigate_property_forward());
    assert_eq!(state.active_property_stack(), &[(leaf_node.id, "width".to_string())]);
    assert_eq!(state.selected_property, Some((leaf_node.id, "width".to_string())));

    // 8. Subsequent Forwards push B and C back onto the stack
    let hit_fwd_2 = panel.handle_click(fwd_x, btn_y, 0.0, 600.0, &tree_items, &state, Some(&layout));
    assert_eq!(hit_fwd_2, PanelHitResult::ForwardPropertyHistory);
    assert!(state.navigate_property_forward());
    assert_eq!(state.active_property_stack().len(), 2);

    assert!(state.navigate_property_forward());
    assert_eq!(state.active_property_stack().len(), 3);
    assert!(!state.can_go_forward());
}

#[test]
fn test_render_property_details_headless() {
    let source = r#"
\Component Container(x: Distance = 0, y: Distance = 0, width: Distance = 400px) {
    \Rect(x: x, y: y, width: width - 50px, height: 40px, color: #10b981)
}
\Container(x: 0, y: 0, width: 400px)
"#;
    let doc = directedtype::parse(source).expect("parse ok");
    let layout = directedtype::compiler::evaluate_document(&doc).expect("eval layout ok");
    let rect_node = layout.nodes.iter().find(|n| n.name == "Rect").expect("Rect found");

    let panel = directedtype::inspector::InspectPanelComponent::default();
    let mut state = directedtype::inspector::InspectorState::new();
    state.selected_id = Some(rect_node.id);
    state.select_property(rect_node.id, "width");

    let tree_items = directedtype::inspector::build_tree_items_from_layout(&layout, &state);

    let mut scene = vello::Scene::new();
    let mut font_cx = parley::FontContext::new();
    let mut layout_cx = parley::LayoutContext::new();

    panel.render_to_scene(
        &mut scene,
        0.0,
        600.0,
        &tree_items,
        &state,
        &layout,
        &mut font_cx,
        &mut layout_cx,
    );

    let mut renderer = directedtype::render::HeadlessRenderer::new().expect("init renderer");
    let img = renderer.render_scene(&scene, 380, 600).expect("render scene ok");
    assert_eq!(img.width(), 380);
    assert_eq!(img.height(), 600);
}

#[test]
fn test_inspector_horizontal_scroll_and_bounds() {
    let source = r#"
\Component VeryLongNamedComponentWithManyParametersAndDeepHierarchy(x: Distance = 0, y: Distance = 0, extra_long_parameter_expression_name: Distance = 100px) {
    \Rect(x: x, y: y, width: 400px, height: 100px, color: #3b82f6)
}
\Component GrandChild(x: Distance = 0, y: Distance = 0) {
    \VeryLongNamedComponentWithManyParametersAndDeepHierarchy(x: 0, y: 0)
}
\Component InnerChild(x: Distance = 0, y: Distance = 0) {
    \GrandChild(x: 0, y: 0)
}
\Component DeepNestedContainer(x: Distance = 0, y: Distance = 0, width: Distance = 500px) {
    \InnerChild(x: 0, y: 0)
}
\DeepNestedContainer(x: 0, y: 0, width: 500px)
"#;
    let doc = directedtype::parse(source).expect("parse ok");
    let layout = directedtype::compiler::evaluate_document(&doc).expect("eval layout ok");

    let panel = directedtype::inspector::InspectPanelComponent::default();
    let mut state = directedtype::inspector::InspectorState::new();

    // Nodes are expanded by default; ensure none are collapsed
    state.collapsed_ids.clear();

    let tree_items = directedtype::inspector::build_tree_items_from_layout(&layout, &state);
    let total_tree_w = panel.total_tree_width(&tree_items);
    assert!(total_tree_w > panel.width, "Deep tree with long names should exceed default panel width (380px)");

    let max_tree_scroll_x = panel.max_tree_scroll_x(&tree_items);
    assert_eq!(max_tree_scroll_x, total_tree_w - panel.width);

    // Test horizontal scroll clamping
    state.scroll_x_by(50.0, max_tree_scroll_x);
    assert_eq!(state.scroll_offset_x, 50.0);

    state.scroll_x_by(1000.0, max_tree_scroll_x);
    assert_eq!(state.scroll_offset_x, max_tree_scroll_x);

    state.scroll_x_by(-5000.0, max_tree_scroll_x);
    assert_eq!(state.scroll_offset_x, 0.0);

    // Now test details panel horizontal scroll
    let rect_node = layout.nodes.iter().find(|n| n.name == "Rect").expect("Rect node found");
    state.set_selected_id(Some(rect_node.id));

    let total_detail_w = panel.total_detail_content_width(Some(rect_node.id), &layout, Some(&state));
    assert!(total_detail_w >= panel.width);

    let max_detail_scroll_x = panel.max_detail_scroll_x(Some(rect_node.id), &layout, Some(&state));
    assert_eq!(max_detail_scroll_x, (total_detail_w - panel.width).max(0.0));

    state.scroll_detail_x_by(40.0, 100.0);
    assert_eq!(state.detail_scroll_offset_x, 40.0);

    // Resetting selection should reset detail_scroll_offset_x to 0.0
    state.set_selected_id(None);
    assert_eq!(state.detail_scroll_offset_x, 0.0);
}

#[test]
fn test_inspector_scrollbar_hit_testing() {
    let source = r#"
\Component LongList(x: Distance = 0, y: Distance = 0, width: Distance = 500px) {
    \Rect(x: 0, y: 0, width: 600px, height: 20px, color: #111111)
    \Rect(x: 0, y: 25px, width: 600px, height: 20px, color: #222222)
    \Rect(x: 0, y: 50px, width: 600px, height: 20px, color: #333333)
    \Rect(x: 0, y: 75px, width: 600px, height: 20px, color: #444444)
    \Rect(x: 0, y: 100px, width: 600px, height: 20px, color: #555555)
    \Rect(x: 0, y: 125px, width: 600px, height: 20px, color: #666666)
    \Rect(x: 0, y: 150px, width: 600px, height: 20px, color: #777777)
    \Rect(x: 0, y: 175px, width: 600px, height: 20px, color: #888888)
    \Rect(x: 0, y: 200px, width: 600px, height: 20px, color: #999999)
    \Rect(x: 0, y: 225px, width: 600px, height: 20px, color: #aaaaaa)
    \Rect(x: 0, y: 250px, width: 600px, height: 20px, color: #bbbbbb)
    \Rect(x: 0, y: 275px, width: 600px, height: 20px, color: #cccccc)
    \Rect(x: 0, y: 300px, width: 600px, height: 20px, color: #dddddd)
    \Rect(x: 0, y: 325px, width: 600px, height: 20px, color: #eeeeee)
}
\LongList(x: 0, y: 0, width: 500px)
"#;
    let doc = directedtype::parse(source).expect("parse ok");
    let layout = directedtype::compiler::evaluate_document(&doc).expect("eval layout ok");

    let panel = directedtype::inspector::InspectPanelComponent::default();
    let mut state = directedtype::inspector::InspectorState::new();

    let win_h = 400.0;
    let panel_x = 200.0;
    let divider_y = panel.divider_y(win_h);

    state.collapsed_ids.clear();
    let tree_items = directedtype::inspector::build_tree_items_from_layout(&layout, &state);

    // 1. Hit-test tree vertical scrollbar (right 12px of tree area)
    let v_hit = panel.hit_test_scrollbar(
        panel_x + panel.width - 6.0,
        panel.header_height + 20.0,
        panel_x,
        win_h,
        &tree_items,
        &state,
        &layout,
    );
    assert_eq!(v_hit, Some(directedtype::inspector::InspectorScrollbarKind::TreeVertical));

    // 2. Normal tree row click (away from scrollbars) should return None for scrollbars
    let normal_hit = panel.hit_test_scrollbar(
        panel_x + 50.0,
        panel.header_height + 20.0,
        panel_x,
        win_h,
        &tree_items,
        &state,
        &layout,
    );
    assert_eq!(normal_hit, None);

    // 3. Select a node and check detail panel vertical scrollbar
    let first_rect = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    state.set_selected_id(Some(first_rect.id));

    let detail_y = divider_y + panel.divider_height;
    let detail_v_hit = panel.hit_test_scrollbar(
        panel_x + panel.width - 6.0,
        detail_y + 30.0,
        panel_x,
        win_h,
        &tree_items,
        &state,
        &layout,
    );
    assert_eq!(detail_v_hit, Some(directedtype::inspector::InspectorScrollbarKind::DetailVertical));
}

#[test]
fn test_render_inspector_with_horizontal_scrollbars() {
    let source = r#"
\Component InnerDeepChild(x: Distance = 0, y: Distance = 0, very_long_formula_and_variable_name_causing_overflow: Integer = 123456789) {
    \Rect(x: x, y: y, width: 500px, height: 60px, color: #06b6d4)
}
\Component LargeComponentWithWideTree(x: Distance = 0, y: Distance = 0, width: Distance = 600px) {
    \InnerDeepChild(x: 0, y: 0)
}
\LargeComponentWithWideTree(x: 0, y: 0, width: 600px)
"#;
    let doc = directedtype::parse(source).expect("parse ok");
    let layout = directedtype::compiler::evaluate_document(&doc).expect("eval layout ok");

    let panel = directedtype::inspector::InspectPanelComponent::default();
    let mut state = directedtype::inspector::InspectorState::new();

    state.collapsed_ids.clear();
    let rect_node = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    state.set_selected_id(Some(rect_node.id));
    state.scroll_offset_x = 45.0;
    state.detail_scroll_offset_x = 30.0;

    let tree_items = directedtype::inspector::build_tree_items_from_layout(&layout, &state);

    let mut scene = vello::Scene::new();
    let mut font_cx = parley::FontContext::new();
    let mut layout_cx = parley::LayoutContext::new();

    panel.render_to_scene(
        &mut scene,
        0.0,
        500.0,
        &tree_items,
        &state,
        &layout,
        &mut font_cx,
        &mut layout_cx,
    );

    let mut renderer = directedtype::render::HeadlessRenderer::new().expect("init renderer");
    let img = renderer.render_scene(&scene, 380, 500).expect("render scene ok");
    assert_eq!(img.width(), 380);
    assert_eq!(img.height(), 500);
}

#[test]
fn test_inspect_mode_resizes_main_panel_window_to_visible_area() {
    let source = r#"
\Rect("main_bg"; x: 0, y: 0, width: window.width, height: window.height, color: #f8fafc)
"#;
    let doc = directedtype::parse(source).expect("parse ok");
    let compiled = directedtype::compiler::CompiledDocument::compile_with_window(&doc, 1024.0, 768.0)
        .expect("compile ok");

    let mut app = directedtype::render::ViewerApp::new_with_compiled(
        compiled,
        directedtype::render::ViewerConfig {
            width: 1024,
            height: 768,
            show_nav_bar: false,
            ..Default::default()
        },
    );

    // Initially, inspect mode is off: viewport and window.width are full 1024px
    assert_eq!(app.viewport_size(), (1024.0, 768.0));
    assert_eq!(app.content_width(), 1024.0);
    assert_eq!(app.max_scroll_x(app.viewport_size().0), 0.0);

    // Enable inspect mode: main panel window.width should shrink to visible area (1024 - 380 = 644)
    app.set_inspect_mode(true);
    assert_eq!(app.viewport_size(), (644.0, 768.0));
    assert_eq!(app.content_width(), 644.0);
    assert_eq!(
        app.max_scroll_x(app.viewport_size().0),
        0.0,
        "Main panel sized to window.width should fit visible area with no horizontal overflow"
    );

    // Disable inspect mode: main panel window.width should expand back to 1024
    app.set_inspect_mode(false);
    assert_eq!(app.viewport_size(), (1024.0, 768.0));
    assert_eq!(app.content_width(), 1024.0);
    assert_eq!(app.max_scroll_x(app.viewport_size().0), 0.0);
}

#[test]
fn test_inspect_mode_allows_horizontal_scrolling_for_wide_content() {
    let source = r#"
\Rect("wide_box"; x: 0, y: 0, width: 900px, height: 400px, color: #3b82f6)
"#;
    let doc = directedtype::parse(source).expect("parse ok");
    let compiled = directedtype::compiler::CompiledDocument::compile_with_window(&doc, 1024.0, 768.0)
        .expect("compile ok");

    let mut app = directedtype::render::ViewerApp::new_with_compiled(
        compiled,
        directedtype::render::ViewerConfig {
            width: 1024,
            height: 768,
            show_nav_bar: false,
            ..Default::default()
        },
    );

    // With inspector closed (1024px viewport), 900px content fits without scrolling
    assert_eq!(app.max_scroll_x(app.viewport_size().0), 0.0);

    // With inspector open (644px visible viewport), 900px content overflows by 256px
    app.set_inspect_mode(true);
    let (vp_w, _) = app.viewport_size();
    assert_eq!(vp_w, 644.0);
    assert_eq!(app.max_scroll_x(vp_w), 256.0);

    // Horizontal scrolling should work and not be clamped back to 0
    app.dispatch_scroll(-100.0, 0.0);
    assert_eq!(app.scroll_x(), 100.0);
    app.clamp_scroll();
    assert_eq!(app.scroll_x(), 100.0);

    app.scroll_to_x(500.0);
    assert_eq!(app.scroll_x(), 256.0);

    // Closing the inspector restores 1024px viewport and clamps scroll_x back to 0
    app.set_inspect_mode(false);
    assert_eq!(app.scroll_x(), 0.0);
}

#[test]
fn test_inspector_panel_resize_border_hit_test_and_clamping() {
    let mut panel = directedtype::inspector::InspectPanelComponent::default();
    assert_eq!(panel.width, 380.0);

    let win_w = 1024.0;
    let win_h = 768.0;
    let panel_x = win_w - panel.width; // 644.0

    // Hit zone around panel_x = 644.0 (+/- 5.0px)
    assert!(panel.is_resize_border_hovered(644.0, 200.0, panel_x, win_h));
    assert!(panel.is_resize_border_hovered(639.5, 200.0, panel_x, win_h));
    assert!(panel.is_resize_border_hovered(648.5, 200.0, panel_x, win_h));
    assert!(!panel.is_resize_border_hovered(635.0, 200.0, panel_x, win_h));
    assert!(!panel.is_resize_border_hovered(655.0, 200.0, panel_x, win_h));
    assert!(!panel.is_resize_border_hovered(644.0, -5.0, panel_x, win_h));

    // Clamping to valid bounds [MIN_WIDTH..=(win_w - MIN_MAIN_PANEL_WIDTH)]
    assert!(panel.set_width_clamped(500.0, win_w));
    assert_eq!(panel.width, 500.0);

    // Below minimum width clamps to MIN_WIDTH (220.0)
    assert!(panel.set_width_clamped(100.0, win_w));
    assert_eq!(panel.width, directedtype::inspector::InspectPanelComponent::MIN_WIDTH);

    // Above maximum width clamps to win_w - 100.0 (924.0)
    assert!(panel.set_width_clamped(1000.0, win_w));
    assert_eq!(panel.width, 924.0);
}

#[test]
fn test_inspector_panel_resize_updates_main_panel_layout_and_scroll() {
    let source = r#"
\Rect("main_bg"; x: 0, y: 0, width: window.width, height: window.height, color: #f8fafc)
"#;
    let doc = directedtype::parse(source).expect("parse ok");
    let compiled = directedtype::compiler::CompiledDocument::compile_with_window(&doc, 1024.0, 768.0)
        .expect("compile ok");

    let mut app = directedtype::render::ViewerApp::new_with_compiled(
        compiled,
        directedtype::render::ViewerConfig {
            width: 1024,
            height: 768,
            show_nav_bar: false,
            ..Default::default()
        },
    );

    app.set_inspect_mode(true);
    assert_eq!(app.inspector_panel_width(), 380.0);
    assert_eq!(app.viewport_size(), (644.0, 768.0));
    assert_eq!(app.content_width(), 644.0);

    // Dragging the inspector left border to widen the panel to 524px shrinks the main panel to 500px
    app.set_inspector_panel_width(524.0);
    assert_eq!(app.inspector_panel_width(), 524.0);
    assert_eq!(app.viewport_size(), (500.0, 768.0));
    assert_eq!(app.content_width(), 500.0);

    // Dragging the inspector left border to narrow the panel to 250px widens the main panel to 774px
    app.set_inspector_panel_width(250.0);
    assert_eq!(app.inspector_panel_width(), 250.0);
    assert_eq!(app.viewport_size(), (774.0, 768.0));
    assert_eq!(app.content_width(), 774.0);
}

#[test]
fn test_component_overload_signature_in_resolved_node() {
    let source = r#"
\use "components/VStack.dt"

\VStack("fixed_both"; width: 300px, height: 200px) {
    \Rect(width: 100px, height: 40px, color: #3b82f6)
}
\VStack("fixed_width"; x: 0, y: 220px, width: 300px) {
    \Rect(width: 100px, height: 40px, color: #10b981)
}
\VStack("shrink_wrap"; x: 0, y: 300px, shrink: true) {
    \Rect(width: 100px, height: 40px, color: #f59e0b)
}
"#;
    let doc = directedtype::parse(source).expect("parse ok");
    let layout = directedtype::evaluate_document_with_window(&doc, 800.0, 600.0).expect("eval layout ok");

    let vstacks: Vec<_> = layout.nodes.iter().filter(|n| n.name == "VStack").collect();
    assert_eq!(vstacks.len(), 3);
    assert_eq!(vstacks[0].overload_signature(), r#"\VStack(width, height)"#);
    assert_eq!(vstacks[1].overload_signature(), r#"\VStack(width)"#);
    assert_eq!(vstacks[2].overload_signature(), r#"\VStack(shrink)"#);
}

#[test]
fn test_inspect_component_env_properties() {
    let source = r#"
\Enum Theme {
    Dark,
    Light
}

\Component ThemeProvider(theme: Theme = Theme.Dark, width: Distance = parent.width) {
    \env text_color = (theme == Theme.Dark) ? #f8fafc : #0f172a
    \env card_bg = (theme == Theme.Dark) ? #1e293b : #e2e8f0
    \env accent_color = #3b82f6
    \env content_width = width - 40px
    \Children
}

\ThemeProvider(theme: Theme.Dark, width: 400px) {
    \Rect(x: 0, y: 0, width: 100px, height: 100px, color: env.text_color)
}
"#;
    let doc = directedtype::parse(source).expect("parse ok");
    let layout = directedtype::compiler::evaluate_document(&doc).expect("eval layout ok");

    let theme_node = layout.nodes.iter().find(|n| n.name == "ThemeProvider").expect("ThemeProvider found");
    let info = InspectTargetInfo::from_layout(&layout, theme_node.id).expect("target info ok");

    assert_eq!(info.env_vars.len(), 4);
    let accent_env = info.env_vars.iter().find(|e| e.0 == "accent_color").expect("accent_color found");
    assert_eq!(accent_env.2, Some(directedtype::compiler::Value::Color("#3b82f6".to_string())));

    let card_bg_env = info.env_vars.iter().find(|e| e.0 == "card_bg").expect("card_bg found");
    assert_eq!(card_bg_env.2, Some(directedtype::compiler::Value::Color("#1e293b".to_string())));

    let text_col_env = info.env_vars.iter().find(|e| e.0 == "text_color").expect("text_color found");
    assert_eq!(text_col_env.2, Some(directedtype::compiler::Value::Color("#f8fafc".to_string())));

    let content_w_env = info.env_vars.iter().find(|e| e.0 == "content_width").expect("content_width found");
    assert_eq!(content_w_env.2, Some(directedtype::compiler::Value::Distance(360.0)));

    // Test DAG property trace
    let trace_text = directedtype::inspector::build_property_dag_trace(&layout, theme_node.id, "text_color")
        .expect("DAG trace for text_color exists");
    assert_eq!(trace_text.origin_kind, directedtype::inspector::PortOriginKind::EnvironmentVariable);
    assert_eq!(trace_text.evaluated_value_str, "#f8fafc");

    let trace_cw = directedtype::inspector::build_property_dag_trace(&layout, theme_node.id, "content_width")
        .expect("DAG trace for content_width exists");
    assert_eq!(trace_cw.origin_kind, directedtype::inspector::PortOriginKind::EnvironmentVariable);
    assert_eq!(trace_cw.evaluated_value_str, "360px");
    assert!(trace_cw.upstream_dependencies.iter().any(|d| d.node_id == theme_node.id && d.port_name == "width"));

    // Test DOM target info
    let mut dom = directedtype::dom::Dom::from_document(&doc).expect("dom ok");
    dom.commit().expect("dom commit ok");
    let theme_handle = dom.roots()[0];
    let dom_info = InspectTargetInfo::from_dom(&dom, theme_handle).expect("dom info ok");
    assert_eq!(dom_info.env_vars.len(), 4);

    // Test Inspector Panel hit testing on env property
    let mut state = directedtype::inspector::InspectorState::new();
    state.selected_id = Some(theme_node.id);
    let panel = directedtype::inspector::InspectPanelComponent::default();
    let tree_items = directedtype::inspector::build_tree_items_from_layout(&layout, &state);

    // Render panel to scene
    let mut scene = vello::Scene::new();
    let mut font_cx = parley::FontContext::new();
    let mut layout_cx = parley::LayoutContext::new();
    panel.render_to_scene(
        &mut scene,
        0.0,
        600.0,
        &tree_items,
        &state,
        &layout,
        &mut font_cx,
        &mut layout_cx,
    );

    // Hit-test on the env property area (detail_y ~ 290.88, header at 300.88, COMPONENT ENV title at 318.88, first env item at ~336.88)
    let hit = panel.handle_click(50.0, 345.0, 0.0, 600.0, &tree_items, &state, Some(&layout));
    match hit {
        directedtype::inspector::PanelHitResult::SelectProperty(nid, prop) => {
            assert_eq!(nid, theme_node.id);
            assert!(prop == "accent_color" || prop == "card_bg" || prop == "text_color");
        }
        other => panic!("Expected SelectProperty hit result, got {:?}", other),
    }
}




