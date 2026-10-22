use directedtype::ast::*;
use directedtype::compiler::compile_to_graph;
use directedtype::evaluate_document;
use directedtype::parse;
use pretty_assertions::assert_eq;

#[test]
fn test_nested_let_node_in_dom_and_scope() {
    let input = r#"
    \Component Container() {
      \Children
    }

    \Component Card() {
      \Container {
        \let header = \Rect(x: 10px, y: 10px, width: 200px, height: 50px, color: #111)
        \let footer = \Rect(x: 10px, y: header.bottom + 10px, width: 200px, height: 30px, color: #222)
      }
    }

    \Card
    "#;

    let doc = parse(input).expect("Failed to parse document");
    let (expanded, _graph) = compile_to_graph(&doc).expect("Failed to compile to graph");

    // Check DOM tree structure
    // Root should be Card (__node_0)
    assert_eq!(expanded.roots.len(), 1);
    let card_id = expanded.roots[0];
    let card_node = expanded.get_node(card_id).unwrap();
    assert_eq!(card_node.name, "Card");

    // Inside Card is Container (__node_1)
    assert_eq!(card_node.children.len(), 1);
    let container_id = card_node.children[0];
    let container_node = expanded.get_node(container_id).unwrap();
    assert_eq!(container_node.name, "Container");

    // Inside Container are header (__node_2) and footer (__node_3)
    assert_eq!(container_node.children.len(), 2);
    let header_id = container_node.children[0];
    let footer_id = container_node.children[1];

    let header_node = expanded.get_node(header_id).unwrap();
    assert_eq!(header_node.name, "Rect");
    assert_eq!(header_node.var_name.as_deref(), Some("header"));

    let footer_node = expanded.get_node(footer_id).unwrap();
    assert_eq!(footer_node.name, "Rect");
    assert_eq!(footer_node.var_name.as_deref(), Some("footer"));

    // Check port connection: footer.y should point to header_id.bottom + 10
    match footer_node.ports.get("y").unwrap() {
        Expr::Binary(bin) => {
            assert_eq!(bin.op, BinaryOp::Add);
            match bin.left.as_ref() {
                Expr::MemberAccess(m) => {
                    assert_eq!(m.member.as_str(), "bottom");
                    match m.target.as_ref() {
                        Expr::Ident(id) => assert_eq!(id.as_str(), &format!("__node_{}", header_id.0)),
                        other => panic!("Expected node ident target, got {:?}", other),
                    }
                }
                other => panic!("Expected member access, got {:?}", other),
            }
        }
        other => panic!("Expected binary expr, got {:?}", other),
    }

    // Verify layout evaluation succeeds without errors or cycles
    let layout = evaluate_document(&doc).expect("Document evaluation should succeed");
    let header_node = layout.get_node(header_id).unwrap();
    let footer_node = layout.get_node(footer_id).unwrap();

    assert_eq!(header_node.rect.x, 10.0);
    assert_eq!(header_node.rect.y, 10.0);
    assert_eq!(header_node.rect.height, 50.0);
    // 10 + 50 + 10 = 70
    assert_eq!(footer_node.rect.y, 70.0);
}

#[test]
fn test_nested_let_scalar_expr() {
    let input = r#"
    \Component Box() {
      \Children
    }

    \Component Card() {
      \Box {
        \let margin = 25px
        \Rect(x: margin, y: margin, width: 100px, height: 50px, color: #333)
      }
    }

    \Card
    "#;

    let doc = parse(input).expect("Failed to parse document");
    let (expanded, _graph) = compile_to_graph(&doc).expect("Failed to compile to graph");

    let card_id = expanded.roots[0];
    let card_node = expanded.get_node(card_id).unwrap();
    let box_id = card_node.children[0];
    let box_node = expanded.get_node(box_id).unwrap();

    // Scalar let binding does not create a node in the DOM
    assert_eq!(box_node.children.len(), 1);
    let rect_id = box_node.children[0];
    let rect_node = expanded.get_node(rect_id).unwrap();
    assert_eq!(rect_node.name, "Rect");

    // margin should be compiled as a private port on the component instance (__node_0.margin)
    match rect_node.ports.get("x").unwrap() {
        Expr::MemberAccess(m) => {
            assert_eq!(m.member.as_str(), "margin");
            match m.target.as_ref() {
                Expr::Ident(id) => assert_eq!(id.as_str(), &format!("__node_{}", card_id.0)),
                other => panic!("Expected node ident target, got {:?}", other),
            }
        }
        other => panic!("Expected MemberAccess, got {:?}", other),
    }

    let layout = evaluate_document(&doc).expect("Document evaluation should succeed");
    let rect_resolved = layout.get_node(rect_id).unwrap();
    assert_eq!(rect_resolved.rect.x, 25.0);
    assert_eq!(rect_resolved.rect.y, 25.0);
}

#[test]
fn test_declarative_forward_reference_in_component() {
    let input = r#"
    \Component Container() {
      \Children
    }

    \Component Layout() {
      // First rect references 'banner' which is defined LATER in a nested slot
      \Rect(x: 0, y: 0, width: banner.width + 20px, height: banner.height, color: #444)
      \Container {
        \let banner = \Rect(x: 5px, y: 5px, width: 150px, height: 40px, color: #555)
      }
    }

    \Layout
    "#;

    let doc = parse(input).expect("Failed to parse document");
    let (expanded, _graph) = compile_to_graph(&doc).expect("Failed to compile to graph");

    let layout_id = expanded.roots[0];
    let layout_node = expanded.get_node(layout_id).unwrap();

    // Layout has 2 children: Rect and Container
    assert_eq!(layout_node.children.len(), 2);
    let first_rect_id = layout_node.children[0];
    let container_id = layout_node.children[1];

    let container_node = expanded.get_node(container_id).unwrap();
    let banner_id = container_node.children[0];

    let first_rect = expanded.get_node(first_rect_id).unwrap();
    // Verify first_rect width references banner_id.width + 20
    match first_rect.ports.get("width").unwrap() {
        Expr::Binary(bin) => {
            match bin.left.as_ref() {
                Expr::MemberAccess(m) => {
                    assert_eq!(m.member.as_str(), "width");
                    match m.target.as_ref() {
                        Expr::Ident(id) => assert_eq!(id.as_str(), &format!("__node_{}", banner_id.0)),
                        other => panic!("Expected node ident target, got {:?}", other),
                    }
                }
                other => panic!("Expected member access, got {:?}", other),
            }
        }
        other => panic!("Expected binary expr, got {:?}", other),
    }

    let layout = evaluate_document(&doc).expect("Document evaluation should succeed");
    let banner_resolved = layout.get_node(banner_id).unwrap();
    let first_rect_resolved = layout.get_node(first_rect_id).unwrap();
    assert_eq!(banner_resolved.rect.width, 150.0);
    assert_eq!(first_rect_resolved.rect.width, 170.0);
}

#[test]
fn test_nested_alias_exposes_public_port() {
    let input = r#"
    \Component Wrapper() {
      \Children
    }

    \Component Modal() {
      \Wrapper {
        \let body = \Rect(x: 20px, y: 20px, width: 300px, height: 200px, color: #666)
        \alias body_width = body.width
        \alias body_height = body.height
      }
    }

    \Component App() {
      \let modal = \Modal()
      \Rect(x: 0, y: 0, width: modal.body_width, height: modal.body_height, color: #777)
    }

    \App
    "#;

    let doc = parse(input).expect("Failed to parse document");
    let (expanded, _graph) = compile_to_graph(&doc).expect("Failed to compile to graph");

    let app_id = expanded.roots[0];
    let app_node = expanded.get_node(app_id).unwrap();

    let modal_id = app_node.children[0];
    let follower_rect_id = app_node.children[1];

    let modal_node = expanded.get_node(modal_id).unwrap();
    let wrapper_id = modal_node.children[0];
    let wrapper_node = expanded.get_node(wrapper_id).unwrap();
    let body_id = wrapper_node.children[0];

    // modal_node's public body_width should point to body_id.width
    match modal_node.ports.get("body_width").unwrap() {
        Expr::MemberAccess(m) => {
            assert_eq!(m.member.as_str(), "width");
            match m.target.as_ref() {
                Expr::Ident(id) => assert_eq!(id.as_str(), &format!("__node_{}", body_id.0)),
                other => panic!("Expected node ident target, got {:?}", other),
            }
        }
        other => panic!("Expected member access, got {:?}", other),
    }

    // follower_rect's width should point to modal_id.body_width
    let follower_rect = expanded.get_node(follower_rect_id).unwrap();
    match follower_rect.ports.get("width").unwrap() {
        Expr::MemberAccess(m) => {
            assert_eq!(m.member.as_str(), "body_width");
            match m.target.as_ref() {
                Expr::Ident(id) => assert_eq!(id.as_str(), &format!("__node_{}", modal_id.0)),
                other => panic!("Expected node ident target, got {:?}", other),
            }
        }
        other => panic!("Expected member access, got {:?}", other),
    }

    let layout = evaluate_document(&doc).expect("Document evaluation should succeed");
    let body_resolved = layout.get_node(body_id).unwrap();
    let follower_resolved = layout.get_node(follower_rect_id).unwrap();

    assert_eq!(body_resolved.rect.width, 300.0);
    assert_eq!(body_resolved.rect.height, 200.0);
    assert_eq!(follower_resolved.rect.width, 300.0);
    assert_eq!(follower_resolved.rect.height, 200.0);
}

#[test]
fn test_nested_alias_forward_reference() {
    let input = r#"
    \Component Card() {
      // Alias defined at top level referring to a let node defined inside a nested slot
      \alias content_h = content.height

      \Rect(x: 0, y: 0, width: 100px, height: 100px, color: #888) {
        \let content = \Rect(x: 5px, y: 5px, width: 90px, height: 45px, color: #999)
      }
    }

    \Component App() {
      \let card = \Card()
      \Rect(x: 0, y: 0, width: 50px, height: card.content_h, color: #aaa)
    }

    \App
    "#;

    let doc = parse(input).expect("Failed to parse document");
    let (_expanded, _graph) = compile_to_graph(&doc).expect("Failed to compile to graph");

    let layout = evaluate_document(&doc).expect("Document evaluation should succeed");
    let app_id = layout.roots[0];
    let app_node = layout.get_node(app_id).unwrap();
    let rect_id = app_node.children[1];

    let rect_resolved = layout.get_node(rect_id).unwrap();
    assert_eq!(rect_resolved.rect.height, 45.0);
}

#[test]
fn test_nested_state_rejected_by_parser() {
    let input = r#"
    \Component Box() {
      \Children
    }

    \Component Card() {
      \Box {
        \state count = 0
      }
    }
    "#;

    let err = parse(input).expect_err("Should fail to parse nested \\state");
    let err_str = format!("{}", err);
    assert!(
        err_str.contains("\\state is only allowed at the top level of a component definition"),
        "Unexpected error: {}",
        err_str
    );
}

#[test]
fn test_nested_env_rejected_by_parser() {
    let input = r#"
    \Component Box() {
      \Children
    }

    \Component Card() {
      \Box {
        \env theme: Theme
      }
    }
    "#;

    let err = parse(input).expect_err("Should fail to parse nested \\env");
    let err_str = format!("{}", err);
    assert!(
        err_str.contains("\\env is only allowed at the top level of a component definition"),
        "Unexpected error: {}",
        err_str
    );
}

#[test]
fn test_shorthand_property_names_in_component_and_children() {
    let input = r#"
    \Component Wrapper(gap: Distance, font_size: Distance) {
      \Children(
        gap,
        font_size
      )
    }

    \Component Consumer(gap: Distance, font_size: Distance) {
      \Rect(x: 0, y: 0, width: gap, height: font_size, color: #123)
    }

    \Component App() {
      \let gap = 42px;
      \let font_size = 18px;

      \Wrapper(gap, font_size) {
        \Consumer()
      }
    }

    \App
    "#;

    let doc = parse(input).expect("Failed to parse document with shorthand properties");
    let layout = evaluate_document(&doc).expect("Document evaluation should succeed");

    let rect_node = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    assert_eq!(rect_node.rect.width, 42.0);
    assert_eq!(rect_node.rect.height, 18.0);
}
