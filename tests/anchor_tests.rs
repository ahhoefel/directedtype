use directedtype::compiler::compiled::CompiledDocument;
use directedtype::parser::parse_document;

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
