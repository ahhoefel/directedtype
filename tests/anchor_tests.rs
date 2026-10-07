use directedtype::compiler::compiled::CompiledDocument;
use directedtype::component::{Component, ComponentRegistry, Context, ContextAction, DispatchError};
use directedtype::interaction::{Event, EventKind, Modifiers, MouseButton, Point};
use directedtype::parser::parse_document;
use directedtype::render::{HeadlessRenderer, SceneOptions, ViewerApp, ViewerConfig};
use vello::peniko::Color;

#[test]
fn test_block_anchor_geometry_passthrough_in_vstack() {
    let source = r##"
        \use "components/VStack.dt";

        \VStack(gap: 16) {
            \Anchor("first") {
                \Rect(width: 200, height: 50, color: "#112233")
            }
            \Anchor("second") {
                \Rect(width: 300, height: 80, color: "#445566")
            }
        }
    "##;

    let ast = parse_document(source).expect("Failed to parse document");
    let compiled = CompiledDocument::compile(&ast).expect("Compilation failed");

    let first_anchor = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.anchor_name.as_deref() == Some("first"))
        .expect("First anchor not found");

    let second_anchor = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.anchor_name.as_deref() == Some("second"))
        .expect("Second anchor not found");

    assert_eq!(first_anchor.rect.width, 200.0);
    assert_eq!(first_anchor.rect.height, 50.0);

    assert_eq!(second_anchor.rect.width, 300.0);
    assert_eq!(second_anchor.rect.height, 80.0);

    // Second anchor should be positioned exactly after first anchor + gap (16.0)
    assert_eq!(second_anchor.rect.y, first_anchor.rect.y + first_anchor.rect.height + 16.0);

    // And first anchor's child Rect should have exact matching origin and dimensions
    let first_child = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.id == first_anchor.children[0])
        .expect("Child rect not found");

    assert_eq!(first_child.rect.x, first_anchor.rect.x);
    assert_eq!(first_child.rect.y, first_anchor.rect.y);
    assert_eq!(first_child.rect.width, 200.0);
    assert_eq!(first_child.rect.height, 50.0);
}

#[test]
fn test_inline_anchor_bookmark_in_text() {
    let source = r##"
        \Text {
            Hello \Anchor("bookmark") World
        }
    "##;

    let ast = parse_document(source).expect("Failed to parse document");
    let compiled = CompiledDocument::compile(&ast).expect("Compilation failed");

    let bookmark = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.anchor_name.as_deref() == Some("bookmark"))
        .expect("Bookmark anchor not found");

    assert_eq!(bookmark.anchor_name.as_deref(), Some("bookmark"));
    assert!(bookmark.rect.x >= 0.0);
    assert!(bookmark.rect.y >= 0.0);
}

#[test]
fn test_hierarchical_scopes_and_path_resolution() {
    let source = r##"
        \use "components/HStack.dt";

        \HStack(gap: 20) {
            \AnchorScope("pane-left") {
                \Anchor("intro") {
                    \Rect(width: 100, height: 40, color: "#111111")
                }
            }
            \AnchorScope("pane-right") {
                \Anchor("intro") {
                    \Rect(width: 150, height: 60, color: "#222222")
                }
            }
        }
    "##;

    let ast = parse_document(source).expect("Failed to parse document");
    let compiled = CompiledDocument::compile(&ast).expect("Compilation failed");

    let left_intro = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.anchor_name.as_deref() == Some("intro") && n.rect.width == 100.0)
        .expect("Left intro anchor not found");

    let right_intro = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.anchor_name.as_deref() == Some("intro") && n.rect.width == 150.0)
        .expect("Right intro anchor not found");

    // 1. Relative resolution from inside pane-left
    let (resolved_left, _) = compiled
        .resolve_anchor(left_intro.id, "#intro")
        .expect("Relative #intro from left pane should resolve");
    assert_eq!(resolved_left, left_intro.id);

    // 2. Relative resolution from inside pane-right
    let (resolved_right, _) = compiled
        .resolve_anchor(right_intro.id, "#intro")
        .expect("Relative #intro from right pane should resolve");
    assert_eq!(resolved_right, right_intro.id);

    // 3. Absolute resolution from anywhere
    let (abs_left, _) = compiled
        .resolve_anchor(right_intro.id, "#/pane-left/intro")
        .expect("Absolute #/pane-left/intro should resolve from right pane");
    assert_eq!(abs_left, left_intro.id);

    let (abs_right, _) = compiled
        .resolve_anchor(left_intro.id, "#/pane-right/intro")
        .expect("Absolute #/pane-right/intro should resolve from left pane");
    assert_eq!(abs_right, right_intro.id);
}

#[test]
fn test_duplicate_anchor_in_same_scope_rejected() {
    let source = r##"
        \AnchorScope("doc") {
            \Anchor("section") {
                \Rect(width: 100, height: 40, color: "#111111")
            }
            \Anchor("section") {
                \Rect(width: 100, height: 40, color: "#222222")
            }
        }
    "##;

    let ast = parse_document(source).expect("Failed to parse document");
    let err = CompiledDocument::compile(&ast).expect_err("Should reject duplicate anchor in same scope");
    let msg = format!("{:?}", err);
    assert!(msg.contains("Duplicate anchor"));
}

#[test]
fn test_anchor_scope_first_class_port_and_append_method() {
    let source = r##"
        \use "components/Link.dt";
        \use "theme/default.dt";

        \Component TableOfContents(target_scope: Node) {
            \Link(url: target_scope.append("setup"), link_style: link_default) {
                Setup Link
            }
        }

        let main_scope = \AnchorScope("main") {
            \Anchor("setup") {
                \Rect(width: 200, height: 50, color: "#abcdef")
            }
        };

        \TableOfContents(target_scope: main_scope)
    "##;

    let ast = parse_document(source).expect("Failed to parse document");
    let compiled = CompiledDocument::compile(&ast).expect("Compilation failed");

    // Verify main_scope has path port evaluated to "/main"
    let scope_node = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "AnchorScope")
        .expect("AnchorScope node not found");

    let scope_path = compiled
        .layout
        .get_value(scope_node.id, "path")
        .and_then(|v| v.as_str());
    assert_eq!(scope_path, Some("/main"));

    // Verify the Link url property evaluated to "#/main/setup"
    let link_node = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.properties.contains_key("url"))
        .expect("Link node with url property not found");

    let url_val = compiled
        .layout
        .get_value(link_node.id, "url")
        .and_then(|v| v.as_str());
    assert_eq!(url_val, Some("#/main/setup"));

    // Verify link resolves to the setup anchor node
    let setup_anchor = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.anchor_name.as_deref() == Some("setup"))
        .expect("Setup anchor node not found");

    let (target_id, _) = compiled
        .resolve_anchor(link_node.id, url_val.unwrap())
        .expect("Should resolve anchor from generated URL");
    assert_eq!(target_id, setup_anchor.id);
}

#[test]
fn test_viewer_window_scrolling_to_anchors() {
    let source = r##"
        \use "components/VStack.dt";

        \VStack(gap: 50) {
            \Anchor("top") {
                \Rect(width: 400, height: 100, color: "#111111")
            }
            \Rect(width: 400, height: 1000, color: "#222222")
            \Anchor("middle") {
                \Rect(width: 400, height: 100, color: "#333333")
            }
            \Rect(width: 400, height: 1000, color: "#444444")
            \Anchor("bottom") {
                \Rect(width: 400, height: 100, color: "#555555")
            }
        }
    "##;

    let ast = parse_document(source).expect("Failed to parse document");
    let compiled = CompiledDocument::compile(&ast).expect("Compilation failed");
    let mut viewer = ViewerApp::new(
        compiled.layout().clone(),
        ViewerConfig {
            width: 800,
            height: 600,
            ..Default::default()
        },
    );

    assert_eq!(viewer.scroll_y(), 0.0);
    assert!(viewer.content_height() > 2200.0);
    assert!(viewer.max_scroll_y(600.0) > 1600.0);

    // Scroll to middle anchor
    let scrolled = viewer.scroll_to_anchor(None, "#middle");
    assert!(scrolled);
    let middle_node = viewer
        .layout()
        .nodes
        .iter()
        .find(|n| n.anchor_name.as_deref() == Some("middle"))
        .unwrap();
    assert_eq!(viewer.scroll_y(), middle_node.rect.y);

    // Scroll to bottom anchor
    let scrolled = viewer.scroll_to_anchor(None, "#bottom");
    assert!(scrolled);
    let bottom_node = viewer
        .layout()
        .nodes
        .iter()
        .find(|n| n.anchor_name.as_deref() == Some("bottom"))
        .unwrap();
    let expected_bottom_scroll = bottom_node.rect.y.min(viewer.max_scroll_y(600.0));
    assert_eq!(viewer.scroll_y(), expected_bottom_scroll);

    // Scroll back to top
    let scrolled = viewer.scroll_to_anchor(None, "#top");
    assert!(scrolled);
    assert_eq!(viewer.scroll_y(), 0.0);
}

#[test]
fn test_headless_render_with_scroll_offset() {
    let source = r##"
        \use "components/VStack.dt";

        \VStack {
            \Rect(width: 200, height: 100, color: "#ff0000")
            \Rect(width: 200, height: 100, color: "#0000ff")
        }
    "##;

    let ast = parse_document(source).expect("Failed to parse document");
    let compiled = CompiledDocument::compile(&ast).expect("Compilation failed");

    let mut renderer = HeadlessRenderer::new().expect("Failed to init renderer");

    // Unscrolled: top 100px is red
    let img_unscrolled = renderer
        .render_layout(
            compiled.layout(),
            200,
            200,
            &SceneOptions {
                background: Some(Color::WHITE),
                scale_factor: 1.0,
                scroll_offset: (0.0, 0.0),
                ..Default::default()
            },
        )
        .expect("Render unscrolled");

    let pixel_top = img_unscrolled.get_pixel(50, 50);
    assert!(pixel_top[0] > 200, "Expected red pixel at (50, 50), got: {:?}", pixel_top);
    assert!(pixel_top[2] < 50);

    // Scrolled by 100px: the blue rect (originally at y=100) is now at y=0!
    let img_scrolled = renderer
        .render_layout(
            compiled.layout(),
            200,
            200,
            &SceneOptions {
                background: Some(Color::WHITE),
                scale_factor: 1.0,
                scroll_offset: (0.0, 100.0),
                ..Default::default()
            },
        )
        .expect("Render scrolled");

    let pixel_scrolled = img_scrolled.get_pixel(50, 50);
    assert!(pixel_scrolled[2] > 200, "Expected blue pixel at (50, 50) when scrolled, got: {:?}", pixel_scrolled);
    assert!(pixel_scrolled[0] < 50);
}

#[test]
fn test_scrolled_coordinate_hit_testing() {
    let source = r##"
        \use "components/VStack.dt";

        \VStack {
            \Rect(width: 300, height: 500, color: "#111111")
            \Anchor("target") {
                \Rect(width: 300, height: 200, color: "#222222")
            }
            \Rect(width: 300, height: 800, color: "#333333")
        }
    "##;

    let ast = parse_document(source).expect("Failed to parse document");
    let compiled = CompiledDocument::compile(&ast).expect("Compilation failed");
    let mut viewer = ViewerApp::new(
        compiled.layout().clone(),
        ViewerConfig {
            width: 800,
            height: 600,
            ..Default::default()
        },
    );

    // Scroll to target anchor (which is at y = 500)
    let scrolled = viewer.scroll_to_anchor(None, "#target");
    assert!(scrolled);
    assert_eq!(viewer.scroll_y(), 500.0);

    // A click at window logical point (50.0, 20.0) corresponds to doc point (50.0, 520.0)
    let window_point = Point::new(50.0, 20.0);
    let doc_point = Point::new(window_point.x + viewer.scroll_x(), window_point.y + viewer.scroll_y());

    let hit = viewer.layout().hit_test(doc_point).expect("Should hit target node");
    let target_anchor = viewer
        .layout()
        .nodes
        .iter()
        .find(|n| n.anchor_name.as_deref() == Some("target"))
        .unwrap();

    assert!(hit.bubble_path.contains(&target_anchor.id));
}

#[derive(Default, Debug)]
struct FocusTrackerComponent;

impl Component for FocusTrackerComponent {
    fn dispatch(
        &mut self,
        method: &str,
        _event: &mut Event,
        ctx: &mut Context<'_>,
    ) -> Result<(), DispatchError> {
        match method {
            "handle_focus" => {
                ctx.set_state("focused", true);
                Ok(())
            }
            "handle_blur" => {
                ctx.set_state("focused", false);
                Ok(())
            }
            _ => Err(DispatchError::MethodNotFound {
                component: "FocusTracker".into(),
                method: method.into(),
            }),
        }
    }
}

#[test]
fn test_focus_and_blur_event_dispatch_and_bubbling() {
    let source = r##"
        \Component FocusTracker {
            state focused: false;
            \Rect(x: 0, y: 0, width: 120, height: 40, color: "#111111", on_focus: self.handle_focus, on_blur: self.handle_blur)
        }
        \FocusTracker
    "##;

    let ast = parse_document(source).expect("Failed to parse document");
    let mut registry = ComponentRegistry::new();
    registry.register("FocusTracker", || Box::new(FocusTrackerComponent::default()));

    let mut compiled = CompiledDocument::compile_with_registry(
        &ast,
        800.0,
        600.0,
        std::path::Path::new("."),
        &directedtype::compiler::module::FsResolver,
        &registry,
    )
    .expect("Compilation failed");

    let tracker_node = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "FocusTracker")
        .expect("FocusTracker node not found");
    let tracker_id = tracker_node.id;

    let rect_node = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "Rect")
        .expect("Rect node not found");
    let rect_id = rect_node.id;

    assert_eq!(compiled.focused_node(), None);
    assert_eq!(
        compiled.get_state(tracker_id, "focused"),
        Some(&directedtype::compiler::value::Value::Bool(false))
    );

    // 1. Focus on rect -> on_focus bubbles to FocusTracker -> focused becomes true
    let changed = compiled
        .set_focused_node(Some(rect_id))
        .expect("Focus dispatch failed");
    assert!(!changed.is_empty());
    assert_eq!(compiled.focused_node(), Some(rect_id));
    assert_eq!(
        compiled.get_state(tracker_id, "focused"),
        Some(&directedtype::compiler::value::Value::Bool(true))
    );

    // 2. Blur rect -> on_blur bubbles to FocusTracker -> focused becomes false
    let changed = compiled
        .set_focused_node(None)
        .expect("Blur dispatch failed");
    assert!(!changed.is_empty());
    assert_eq!(compiled.focused_node(), None);
    assert_eq!(
        compiled.get_state(tracker_id, "focused"),
        Some(&directedtype::compiler::value::Value::Bool(false))
    );
}

#[test]
fn test_keyboard_focus_traversal_reading_order() {
    let source = r##"
        \use "components/VStack.dt";
        \use "components/HStack.dt";
        \use "components/Link.dt";
        \use "theme/default.dt";

        env link_style = link_default;

        \VStack(gap: 20) {
            \Link(url: "#sec1") { Link 1 }
            \HStack(gap: 10) {
                \Link(url: "#sec2") { Link 2 }
                \Link(url: "#sec3") { Link 3 }
            }
            \Link(url: "#sec4") { Link 4 }
        }
    "##;

    let ast = parse_document(source).expect("Failed to parse document");
    let compiled = CompiledDocument::compile(&ast).expect("Compilation failed");
    let mut viewer = ViewerApp::new(compiled.layout().clone(), ViewerConfig::default())
        .with_compiled(compiled);

    let focusable = viewer.focusable_nodes();
    assert_eq!(focusable.len(), 4);

    // Top-to-bottom, left-to-right reading order:
    let link_urls: Vec<String> = focusable
        .iter()
        .map(|id| {
            viewer
                .layout()
                .get_node(*id)
                .unwrap()
                .properties
                .get("url")
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();

    assert_eq!(link_urls, vec!["#sec1", "#sec2", "#sec3", "#sec4"]);

    // Test Tab / focus_next cycling:
    assert_eq!(viewer.focused_node(), None);
    assert_eq!(viewer.focus_next(), Some(focusable[0]));
    assert_eq!(viewer.focused_node(), Some(focusable[0]));

    assert_eq!(viewer.focus_next(), Some(focusable[1]));
    assert_eq!(viewer.focused_node(), Some(focusable[1]));

    assert_eq!(viewer.focus_next(), Some(focusable[2]));
    assert_eq!(viewer.focus_next(), Some(focusable[3]));

    // Wraparound to start
    assert_eq!(viewer.focus_next(), Some(focusable[0]));

    // Test Shift+Tab / focus_previous:
    assert_eq!(viewer.focus_previous(), Some(focusable[3]));
    assert_eq!(viewer.focus_previous(), Some(focusable[2]));
    assert_eq!(viewer.focus_previous(), Some(focusable[1]));
    assert_eq!(viewer.focus_previous(), Some(focusable[0]));
    assert_eq!(viewer.focus_previous(), Some(focusable[3]));
}

#[test]
fn test_keyboard_focus_scrolls_into_view() {
    let source = r##"
        \use "components/VStack.dt";
        \use "components/Link.dt";
        \use "theme/default.dt";

        env link_style = link_default;

        \VStack(gap: 40) {
            \Link(url: "#top") { Top Link }
            \Rect(width: 300, height: 1200, color: "#111111")
            \Link(url: "#bottom") { Bottom Link }
        }
    "##;

    let ast = parse_document(source).expect("Failed to parse document");
    let compiled = CompiledDocument::compile(&ast).expect("Compilation failed");
    let mut viewer = ViewerApp::new(
        compiled.layout().clone(),
        ViewerConfig {
            width: 800,
            height: 600,
            ..Default::default()
        },
    )
    .with_compiled(compiled);

    assert_eq!(viewer.scroll_y(), 0.0);

    // Focus first link (at top) -> no scrolling needed
    viewer.focus_next();
    assert_eq!(viewer.scroll_y(), 0.0);

    // Focus second link (far below fold) -> scrolls into view!
    viewer.focus_next();
    assert!(
        viewer.scroll_y() > 600.0,
        "Expected scroll_y to advance past 600px to reveal bottom link, got: {}",
        viewer.scroll_y()
    );

    // Focus wrap back to top link -> scrolls back up!
    viewer.focus_next();
    assert_eq!(viewer.scroll_y(), 0.0);
}

#[test]
fn test_keyboard_activation_triggers_anchor_navigation() {
    let source = r##"
        \use "components/VStack.dt";
        \use "components/Link.dt";
        \use "theme/default.dt";

        env link_style = link_default;

        \VStack(gap: 30) {
            \Link(url: "#deep") { Jump to Deep Anchor }
            \Rect(width: 300, height: 1000, color: "#222222")
            \Anchor("deep") {
                \Rect(width: 300, height: 100, color: "#333333")
            }
        }
    "##;

    let ast = parse_document(source).expect("Failed to parse document");
    let compiled = CompiledDocument::compile(&ast).expect("Compilation failed");
    let mut viewer = ViewerApp::new(
        compiled.layout().clone(),
        ViewerConfig {
            width: 800,
            height: 600,
            ..Default::default()
        },
    )
    .with_compiled(compiled);

    // Tab to jump link
    viewer.focus_next();
    assert_eq!(viewer.scroll_y(), 0.0);

    // Activate focused link (simulating Enter or Space key press)
    let activated = viewer.activate_focused_node();
    assert!(activated);

    let deep_anchor = viewer
        .layout()
        .nodes
        .iter()
        .find(|n| n.anchor_name.as_deref() == Some("deep"))
        .expect("Deep anchor not found");

    // Window scrolled to target anchor (clamped to max scroll)
    assert_eq!(viewer.scroll_y(), deep_anchor.rect.y.min(viewer.max_scroll_y(600.0)));

    // Both target_node and focused_node are set to the anchor target!
    assert_eq!(viewer.target_node(), Some(deep_anchor.id));
    assert_eq!(viewer.focused_node(), Some(deep_anchor.id));
}

#[test]
fn test_designer_driven_focus_and_blur_target_styling() {
    // Page designers control target and focus visuals via on_focus and on_blur handlers,
    // rather than the engine hardcoding arbitrary colors or overlays.
    let source = r##"
        \Component SectionTarget {
            state focused: Boolean = false;
            let bg_color = focused ? #3b82f6 : #ffffff;

            \Anchor(name: "styled-target", on_focus: self.handle_focus, on_blur: self.handle_blur) {
                \Rect(width: 200, height: 80, color: bg_color)
            }
        }

        \SectionTarget
    "##;

    struct TargetCompanion;
    impl Component for TargetCompanion {
        fn dispatch(
            &mut self,
            method: &str,
            _event: &mut Event,
            ctx: &mut Context<'_>,
        ) -> Result<(), DispatchError> {
            match method {
                "handle_focus" => {
                    ctx.set_state("focused", true);
                    Ok(())
                }
                "handle_blur" => {
                    ctx.set_state("focused", false);
                    Ok(())
                }
                _ => Err(DispatchError::MethodNotFound {
                    component: "SectionTarget".into(),
                    method: method.into(),
                }),
            }
        }
    }

    let mut registry = ComponentRegistry::standard();
    registry.register("SectionTarget", || Box::new(TargetCompanion));

    let ast = parse_document(source).expect("Failed to parse document");
    let mut compiled = CompiledDocument::compile_with_registry(
        &ast,
        400.0,
        300.0,
        std::path::Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("Compilation failed");

    let rect_node = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "Rect")
        .expect("Rect node not found");

    // Initially unfocused: white (#ffffff)
    assert_eq!(
        rect_node.properties.get("color").and_then(|v| v.as_str()),
        Some("#ffffff")
    );

    let target_anchor_id = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.anchor_name.as_deref() == Some("styled-target"))
        .expect("Anchor not found")
        .id;

    // Execute scroll_to action targeting the anchor -> shifts focus to target!
    let scroll_act = ContextAction::ScrollToNode {
        target: target_anchor_id,
        container: None,
    };
    compiled
        .execute_action(scroll_act)
        .expect("Execute action ok");

    // After focus shifted to target: on_focus fired, updating color to blue (#3b82f6)
    let rect_after_focus = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "Rect")
        .expect("Rect node not found");
    assert_eq!(
        rect_after_focus.properties.get("color").and_then(|v| v.as_str()),
        Some("#3b82f6")
    );

    // Blurring target (focusing None) -> on_blur fired, restoring color to white
    compiled
        .execute_action(ContextAction::SetFocus { target: None })
        .expect("Blur action ok");
    let rect_after_blur = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "Rect")
        .expect("Rect node not found");
    assert_eq!(
        rect_after_blur.properties.get("color").and_then(|v| v.as_str()),
        Some("#ffffff")
    );
}

#[test]
fn test_link_component_click_handler_and_action_queue() {
    let source = r##"
        \use "components/VStack.dt";
        \use "components/Link.dt";
        \use "theme/default.dt";

        env link_style = link_default;

        \VStack(gap: 20) {
            \Link(url: "#dest") { Go to Destination }
            \Link(url: "#nonexistent") { Broken Link }
            \Link(url: "https://example.com") { External Web }
            \Anchor("dest") {
                \Rect(width: 200, height: 50, color: "#cccccc")
            }
        }
    "##;

    let ast = parse_document(source).expect("Failed to parse document");
    let registry = ComponentRegistry::standard();
    let mut compiled = CompiledDocument::compile_with_registry(
        &ast,
        800.0,
        600.0,
        std::path::Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("Compilation failed");

    let dest_anchor_id = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.anchor_name.as_deref() == Some("dest"))
        .expect("Anchor 'dest' not found")
        .id;

    let link_nodes: Vec<(directedtype::compiler::expanded::NodeId, f64, f64)> = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.name == "Link")
        .map(|n| (n.id, n.rect.x, n.rect.y))
        .collect();
    assert_eq!(link_nodes.len(), 3);

    // 1. Click valid in-page anchor link -> emits ScrollToNode
    let mut click_event_1 = Event::new(
        EventKind::Click { button: MouseButton::Left },
        Point::new(link_nodes[0].1 + 10.0, link_nodes[0].2 + 10.0),
        Point::default(),
        Modifiers::default(),
        link_nodes[0].0,
    );
    click_event_1.bubble_path = compiled.layout.bubble_path_for_node(link_nodes[0].0);
    compiled.dispatch_event(&mut click_event_1).expect("dispatch click 1 ok");

    let actions = compiled.take_actions();
    assert_eq!(
        actions,
        vec![ContextAction::ScrollToNode {
            target: dest_anchor_id,
            container: None,
        }]
    );

    // 2. Click broken anchor link -> anchor not found, returns false, queues no scroll action
    let mut click_event_2 = Event::new(
        EventKind::Click { button: MouseButton::Left },
        Point::new(link_nodes[1].1 + 10.0, link_nodes[1].2 + 10.0),
        Point::default(),
        Modifiers::default(),
        link_nodes[1].0,
    );
    click_event_2.bubble_path = compiled.layout.bubble_path_for_node(link_nodes[1].0);
    compiled.dispatch_event(&mut click_event_2).expect("dispatch click 2 ok");

    let actions2 = compiled.take_actions();
    assert!(actions2.is_empty(), "Broken anchor link should not queue actions");

    // 3. Click external URL link -> queues OpenUrl action
    let mut click_event_3 = Event::new(
        EventKind::Click { button: MouseButton::Left },
        Point::new(link_nodes[2].1 + 10.0, link_nodes[2].2 + 10.0),
        Point::default(),
        Modifiers::default(),
        link_nodes[2].0,
    );
    click_event_3.bubble_path = compiled.layout.bubble_path_for_node(link_nodes[2].0);
    compiled.dispatch_event(&mut click_event_3).expect("dispatch click 3 ok");

    let actions3 = compiled.take_actions();
    assert_eq!(
        actions3,
        vec![ContextAction::OpenUrl {
            url: "https://example.com".to_string(),
        }]
    );
}

#[test]
fn test_link_component_custom_on_click_override() {
    let source = r##"
        \use "components/Link.dt";
        \use "theme/default.dt";

        \Component CustomPage {
            state custom_clicked: Boolean = false;

            \Link(url: "#ignored", link_style: link_default, on_click: self.handle_custom_click) {
                Custom Action Link
            }
        }

        \CustomPage
    "##;

    struct PageCompanion;
    impl Component for PageCompanion {
        fn dispatch(
            &mut self,
            method: &str,
            event: &mut Event,
            ctx: &mut Context<'_>,
        ) -> Result<(), DispatchError> {
            match method {
                "handle_custom_click" => {
                    event.stop_propagation();
                    ctx.set_state("custom_clicked", true);
                    Ok(())
                }
                _ => Err(DispatchError::MethodNotFound {
                    component: "CustomPage".into(),
                    method: method.into(),
                }),
            }
        }
    }

    let mut registry = ComponentRegistry::standard();
    registry.register("CustomPage", || Box::new(PageCompanion));

    let ast = parse_document(source).expect("Failed to parse document");
    let mut compiled = CompiledDocument::compile_with_registry(
        &ast,
        400.0,
        300.0,
        std::path::Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("Compilation failed");

    let link_node = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "Link")
        .expect("Link not found");

    let mut click_event = Event::new(
        EventKind::Click { button: MouseButton::Left },
        Point::new(link_node.rect.x + 5.0, link_node.rect.y + 5.0),
        Point::default(),
        Modifiers::default(),
        link_node.id,
    );
    click_event.bubble_path = compiled.layout.bubble_path_for_node(link_node.id);
    compiled.dispatch_event(&mut click_event).expect("dispatch custom click ok");

    // The custom on_click handler was called instead of standard Link::click!
    let page_node = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "CustomPage")
        .expect("CustomPage node not found");
    assert_eq!(
        compiled.get_state(page_node.id, "custom_clicked"),
        Some(&directedtype::compiler::Value::Bool(true))
    );

    // No ScrollToNode action was queued by standard Link::click because it was overridden!
    assert!(compiled.take_actions().is_empty());
}

#[test]
fn test_link_demo_focus_and_blur_target_highlight_transitions() {
    let source = std::fs::read_to_string("examples/link_demo.dt").expect("read link_demo.dt");
    let doc = parse_document(&source).expect("parse link_demo.dt ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        std::path::Path::new("examples"),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile link_demo.dt ok");

    let mut viewer = ViewerApp::new(
        compiled.layout().clone(),
        ViewerConfig {
            width: 800,
            height: 600,
            ..Default::default()
        },
    )
    .with_document(doc.clone())
    .with_registry(ComponentRegistry::standard())
    .with_compiled(compiled);

    // Identify anchors
    let a1_id = viewer
        .layout()
        .nodes
        .iter()
        .find(|n| n.anchor_name.as_deref() == Some("inline-links"))
        .expect("a1 found")
        .id;
    let a2_id = viewer
        .layout()
        .nodes
        .iter()
        .find(|n| n.anchor_name.as_deref() == Some("custom-styling"))
        .expect("a2 found")
        .id;
    let a4_id = viewer
        .layout()
        .nodes
        .iter()
        .find(|n| n.anchor_name.as_deref() == Some("deep-section"))
        .expect("a4 found")
        .id;
    let atop_id = viewer
        .layout()
        .nodes
        .iter()
        .find(|n| n.anchor_name.as_deref() == Some("top"))
        .expect("atop found")
        .id;

    // Helper to get Card parent of an anchor
    let card1_id = viewer.layout().get_node(a1_id).unwrap().parent.unwrap();
    let card2_id = viewer.layout().get_node(a2_id).unwrap().parent.unwrap();
    let card4_id = viewer.layout().get_node(a4_id).unwrap().parent.unwrap();

    // 1. Initial state: all cards unfocused
    assert_eq!(
        viewer.compiled().unwrap().get_state(card1_id, "focused"),
        Some(&directedtype::compiler::Value::Bool(false))
    );
    assert_eq!(
        viewer.compiled().unwrap().get_state(card2_id, "focused"),
        Some(&directedtype::compiler::Value::Bool(false))
    );
    assert_eq!(
        viewer.compiled().unwrap().get_state(card4_id, "focused"),
        Some(&directedtype::compiler::Value::Bool(false))
    );

    // 2. Navigate to #inline-links -> fires on_focus on Card 1
    viewer.execute_action(ContextAction::ScrollToNode {
        target: a1_id,
        container: None,
    });
    assert_eq!(
        viewer.compiled().unwrap().get_state(card1_id, "focused"),
        Some(&directedtype::compiler::Value::Bool(true))
    );
    assert_eq!(
        viewer.compiled().unwrap().get_state(card2_id, "focused"),
        Some(&directedtype::compiler::Value::Bool(false))
    );

    // 3. Navigate to #custom-styling -> Card 1 gets on_blur (false), Card 2 gets on_focus (true)
    viewer.execute_action(ContextAction::ScrollToNode {
        target: a2_id,
        container: None,
    });
    assert_eq!(
        viewer.compiled().unwrap().get_state(card1_id, "focused"),
        Some(&directedtype::compiler::Value::Bool(false)),
        "Card 1 must be reset to unfocused via on_blur"
    );
    assert_eq!(
        viewer.compiled().unwrap().get_state(card2_id, "focused"),
        Some(&directedtype::compiler::Value::Bool(true)),
        "Card 2 must be highlighted via on_focus"
    );

    // 4. Navigate to #deep-section -> Card 2 gets on_blur (false), Card 4 gets on_focus (true)
    viewer.execute_action(ContextAction::ScrollToNode {
        target: a4_id,
        container: None,
    });
    assert_eq!(
        viewer.compiled().unwrap().get_state(card2_id, "focused"),
        Some(&directedtype::compiler::Value::Bool(false)),
        "Card 2 must be reset to unfocused via on_blur"
    );
    assert_eq!(
        viewer.compiled().unwrap().get_state(card4_id, "focused"),
        Some(&directedtype::compiler::Value::Bool(true)),
        "Card 4 must be highlighted via on_focus"
    );

    // 5. Navigate to #top -> Card 4 gets on_blur (false), no card focused
    viewer.execute_action(ContextAction::ScrollToNode {
        target: atop_id,
        container: None,
    });
    assert_eq!(
        viewer.compiled().unwrap().get_state(card4_id, "focused"),
        Some(&directedtype::compiler::Value::Bool(false)),
        "Card 4 must be reset to unfocused via on_blur"
    );
}

#[test]
fn test_link_component_focus_and_blur_transitions() {
    let source = r##"
        \use "components/LinkStyle.dt";
        \use "components/Link.dt";
        \use "components/VStack.dt";

        let custom_style = \LinkStyle(
            color: #1a73e8,
            underline: false,
            bg: #00000000,
            hover_color: #2563eb,
            hover_underline: true,
            hover_bg: #dbeafe80,
            focused_color: #d97706,
            focused_underline: true,
            focused_bg: #fef3c7
        );

        \VStack {
            \Link(url: "https://example.com", link_style: custom_style) {
                Visit Example
            }
        }
    "##;

    let doc = parse_document(source).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let mut compiled = CompiledDocument::compile_with_registry(
        &doc,
        400.0,
        300.0,
        std::path::Path::new("examples"),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let link_node = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "Link")
        .expect("Link node found");
    let link_id = link_node.id;

    // 1. Initially unfocused & unhovered -> base style
    assert_eq!(
        compiled.get_state(link_id, "focused"),
        Some(&directedtype::compiler::Value::Bool(false))
    );
    assert_eq!(
        compiled.get_state(link_id, "hovered"),
        Some(&directedtype::compiler::Value::Bool(false))
    );
    assert_eq!(
        compiled.layout.nodes.iter().find(|n| n.id == link_id).unwrap().properties.get("current_color").and_then(|v| v.as_str()),
        Some("#1a73e8")
    );
    assert_eq!(
        compiled.layout.nodes.iter().find(|n| n.id == link_id).unwrap().properties.get("current_underline").and_then(|v| v.as_bool()),
        Some(false)
    );

    // 2. Hover over link (PointerEnter) -> hover style
    let mut enter_event = Event::new(
        EventKind::PointerEnter,
        Point::default(),
        Point::default(),
        Modifiers::default(),
        link_id,
    );
    enter_event.bubble_path = compiled.layout.bubble_path_for_node(link_id);
    let changed = compiled.dispatch_event(&mut enter_event).expect("pointer_enter ok");
    assert!(!changed.is_empty());
    assert_eq!(
        compiled.get_state(link_id, "hovered"),
        Some(&directedtype::compiler::Value::Bool(true))
    );
    assert_eq!(
        compiled.layout.nodes.iter().find(|n| n.id == link_id).unwrap().properties.get("current_color").and_then(|v| v.as_str()),
        Some("#2563eb")
    );
    assert_eq!(
        compiled.layout.nodes.iter().find(|n| n.id == link_id).unwrap().properties.get("current_underline").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        compiled.layout.nodes.iter().find(|n| n.id == link_id).unwrap().properties.get("current_bg").and_then(|v| v.as_str()),
        Some("#dbeafe80")
    );

    // 3. Focus link while hovered -> focus style takes precedence
    let changed = compiled.set_focused_node(Some(link_id)).expect("set focus ok");
    assert!(!changed.is_empty());
    assert_eq!(
        compiled.get_state(link_id, "focused"),
        Some(&directedtype::compiler::Value::Bool(true))
    );
    assert_eq!(
        compiled.layout.nodes.iter().find(|n| n.id == link_id).unwrap().properties.get("current_color").and_then(|v| v.as_str()),
        Some("#d97706")
    );
    assert_eq!(
        compiled.layout.nodes.iter().find(|n| n.id == link_id).unwrap().properties.get("current_underline").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        compiled.layout.nodes.iter().find(|n| n.id == link_id).unwrap().properties.get("current_bg").and_then(|v| v.as_str()),
        Some("#fef3c7")
    );

    // 4. Blur link while still hovered -> falls back to hover style
    let changed = compiled.set_focused_node(None).expect("blur ok");
    assert!(!changed.is_empty());
    assert_eq!(
        compiled.get_state(link_id, "focused"),
        Some(&directedtype::compiler::Value::Bool(false))
    );
    assert_eq!(
        compiled.get_state(link_id, "hovered"),
        Some(&directedtype::compiler::Value::Bool(true))
    );
    assert_eq!(
        compiled.layout.nodes.iter().find(|n| n.id == link_id).unwrap().properties.get("current_color").and_then(|v| v.as_str()),
        Some("#2563eb")
    );
    assert_eq!(
        compiled.layout.nodes.iter().find(|n| n.id == link_id).unwrap().properties.get("current_underline").and_then(|v| v.as_bool()),
        Some(true)
    );

    // 5. Unhover link (PointerLeave) -> restored to base normal style
    let mut leave_event = Event::new(
        EventKind::PointerLeave,
        Point::default(),
        Point::default(),
        Modifiers::default(),
        link_id,
    );
    leave_event.bubble_path = compiled.layout.bubble_path_for_node(link_id);
    let changed = compiled.dispatch_event(&mut leave_event).expect("pointer_leave ok");
    assert!(!changed.is_empty());
    assert_eq!(
        compiled.get_state(link_id, "hovered"),
        Some(&directedtype::compiler::Value::Bool(false))
    );
    assert_eq!(
        compiled.layout.nodes.iter().find(|n| n.id == link_id).unwrap().properties.get("current_color").and_then(|v| v.as_str()),
        Some("#1a73e8")
    );
    assert_eq!(
        compiled.layout.nodes.iter().find(|n| n.id == link_id).unwrap().properties.get("current_underline").and_then(|v| v.as_bool()),
        Some(false)
    );
}

#[test]
fn test_link_styling_required_no_default() {
    // Omitting style when no ambient theme is in scope must fail compilation with MissingPort
    let source = r##"
        \use "components/Link.dt";

        \Link(url: "https://example.com") {
            Unstyled Link
        }
    "##;

    let doc = parse_document(source).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let result = CompiledDocument::compile_with_registry(
        &doc,
        400.0,
        300.0,
        std::path::Path::new("examples"),
        &directedtype::compiler::FsResolver,
        &registry,
    );

    assert!(result.is_err(), "Expected compilation failure due to missing required style");
    let err = result.unwrap_err();
    match err {
        directedtype::compiler::CompileError::MissingPort { node, port, .. } => {
            assert_eq!(node, "Link");
            assert_eq!(port, "link_style");
        }
        other => panic!("Expected MissingPort, got: {:?}", other),
    }
}

#[test]
fn test_link_styling_environmental_from_theme() {
    // Environmental link_style provides ambient styling to Link components
    let source = r##"
        \use "components/Link.dt";
        \use "theme/default.dt";

        env link_style = link_default;

        \Link(url: "https://example.com") {
            Ambiently Styled Link
        }
    "##;

    let doc = parse_document(source).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        400.0,
        300.0,
        std::path::Path::new("examples"),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok with ambient env link_style");

    let link_node = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "Link")
        .expect("Link node found");

    // Inherited link_default styling: color #2563eb, underline true
    assert_eq!(
        link_node.properties.get("current_color").and_then(|v| v.as_str()),
        Some("#2563eb")
    );
    assert_eq!(
        link_node.properties.get("current_underline").and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[test]
fn test_link_demo_keyboard_tabbing_and_link_focus_order() {
    let source = std::fs::read_to_string("examples/link_demo.dt").expect("read link_demo.dt");
    let doc = parse_document(&source).expect("parse link_demo.dt ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        std::path::Path::new("examples"),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile link_demo.dt ok");

    let mut viewer = ViewerApp::new(
        compiled.layout().clone(),
        ViewerConfig {
            width: 800,
            height: 600,
            ..Default::default()
        },
    )
    .with_document(doc)
    .with_registry(ComponentRegistry::standard())
    .with_compiled(compiled);

    let focusable = viewer.focusable_nodes();
    assert!(!focusable.is_empty(), "Must have focusable links");

    // 1. Verify no Cards, Rects, or Anchors are in the keyboard sequential tab order
    for &id in &focusable {
        let node = viewer.layout().get_node(id).unwrap();
        assert_ne!(node.name, "Card", "Card container must not be in Tab navigation");
        assert_ne!(node.name, "Rect", "Rect shape must not be in Tab navigation");
        assert_ne!(node.name, "Anchor", "Anchor target must not be in Tab navigation");
        assert_eq!(node.name, "Link", "Every sequentially focusable node in link demo must be a Link");
    }

    // Identify Section 1 Card and Section 4 Card
    let a1_id = viewer
        .layout()
        .nodes
        .iter()
        .find(|n| n.anchor_name.as_deref() == Some("inline-links"))
        .unwrap()
        .id;
    let card1_id = viewer.layout().get_node(a1_id).unwrap().parent.unwrap();

    let a4_id = viewer
        .layout()
        .nodes
        .iter()
        .find(|n| n.anchor_name.as_deref() == Some("deep-section"))
        .unwrap()
        .id;
    let card4_id = viewer.layout().get_node(a4_id).unwrap().parent.unwrap();

    // 2. Sequential tabbing through TOC links
    assert_eq!(viewer.focused_node(), None);

    let l1_id = viewer.focus_next().expect("focus link 1");
    assert_eq!(viewer.focused_node(), Some(l1_id));
    assert_eq!(
        viewer.compiled().unwrap().get_state(l1_id, "focused"),
        Some(&directedtype::compiler::Value::Bool(true))
    );

    let l2_id = viewer.focus_next().expect("focus link 2");
    assert_eq!(viewer.focused_node(), Some(l2_id));
    assert_eq!(
        viewer.compiled().unwrap().get_state(l1_id, "focused"),
        Some(&directedtype::compiler::Value::Bool(false))
    );
    assert_eq!(
        viewer.compiled().unwrap().get_state(l2_id, "focused"),
        Some(&directedtype::compiler::Value::Bool(true))
    );

    let _l3_id = viewer.focus_next().expect("focus link 3");
    let _l4_id = viewer.focus_next().expect("focus link 4");

    // 3. Tab into Section 1 link (directedtype.org)
    let s1_link_id = viewer.focus_next().expect("focus section 1 link");
    assert_eq!(viewer.focused_node(), Some(s1_link_id));
    assert_eq!(
        viewer.compiled().unwrap().get_state(s1_link_id, "focused"),
        Some(&directedtype::compiler::Value::Bool(true))
    );
    // Link focus must NOT bubble up to Card!
    assert_eq!(
        viewer.compiled().unwrap().get_state(card1_id, "focused"),
        Some(&directedtype::compiler::Value::Bool(false)),
        "Tabbing to an inner link must not highlight the outer Card"
    );

    // 4. In-page anchor jump to #deep-section -> Card 4 highlights
    viewer.execute_action(ContextAction::ScrollToNode {
        target: a4_id,
        container: None,
    });
    assert_eq!(
        viewer.compiled().unwrap().get_state(card4_id, "focused"),
        Some(&directedtype::compiler::Value::Bool(true)),
        "Anchor navigation must highlight target Card"
    );

    // 5. Pressing Tab after anchor jump advances to the link after the anchor (Section 4 back to top link)
    let s4_link_id = viewer.focus_next().expect("focus link after deep anchor");
    assert_eq!(viewer.focused_node(), Some(s4_link_id));
    let s4_link_url = viewer.layout().get_node(s4_link_id).unwrap().properties.get("url").unwrap().as_str().unwrap();
    assert_eq!(s4_link_url, "#top", "Must advance to the link inside Section 4");
    assert_eq!(
        viewer.compiled().unwrap().get_state(card4_id, "focused"),
        Some(&directedtype::compiler::Value::Bool(false)),
        "Card 4 must blur when focus moves to a link"
    );
    assert_eq!(
        viewer.compiled().unwrap().get_state(s4_link_id, "focused"),
        Some(&directedtype::compiler::Value::Bool(true))
    );
}


