use directedtype::compiler::compiled::CompiledDocument;
use directedtype::interaction::Point;
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

        \Component TableOfContents(target_scope: Node) {
            \Link(url: target_scope.append("setup")) {
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

