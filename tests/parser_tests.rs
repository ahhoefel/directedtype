use directedtype::ast::*;
use directedtype::error::ParseError;
use directedtype::parse;
use pretty_assertions::assert_eq;

#[test]
fn test_spec_shaded_box_component() {
    let input = r#"
    \Component ShadedBox(bg_color: Color, width: max(children.width) + 32) {
      \Rect(
        x: x,
        y: y,
        width: width,
        height: height,
        color: bg_color
      )
      
      \Children {
        x: x + 16,
        y: y + 16
      }
    }
    "#;

    let doc = parse(input).expect("Failed to parse ShadedBox component");
    assert_eq!(doc.items.len(), 1);

    match &doc.items[0] {
        Item::Component(comp) => {
            assert_eq!(comp.name.as_str(), "ShadedBox");
            assert_eq!(comp.params.len(), 2);

            // Param 1: bg_color: Color
            assert_eq!(comp.params[0].name.as_str(), "bg_color");
            assert_eq!(
                comp.params[0].type_annotation.as_ref().map(|t| t.name.as_str()),
                Some("Color")
            );
            assert!(comp.params[0].default_edge.is_none());

            // Param 2: width: max(children.width) + 32
            assert_eq!(comp.params[1].name.as_str(), "width");
            assert!(comp.params[1].type_annotation.is_none());
            match &comp.params[1].default_edge {
                Some(Expr::Binary(bin)) => {
                    assert_eq!(bin.op, BinaryOp::Add);
                    match bin.left.as_ref() {
                        Expr::Call(call) => {
                            assert_eq!(call.callee.as_str(), "max");
                            assert_eq!(call.args.len(), 1);
                            match &call.args[0] {
                                Expr::MemberAccess(m) => {
                                    match m.target.as_ref() {
                                        Expr::Ident(id) => assert_eq!(id.as_str(), "children"),
                                        _ => panic!("Expected target ident"),
                                    }
                                    assert_eq!(m.member.as_str(), "width");
                                }
                                _ => panic!("Expected member access"),
                            }
                        }
                        _ => panic!("Expected call expr"),
                    }
                    match bin.right.as_ref() {
                        Expr::Literal(Literal::Number(n, _)) => assert_eq!(*n, 32.0),
                        _ => panic!("Expected literal number 32"),
                    }
                }
                _ => panic!("Expected binary expression for width default"),
            }

            // Body: \Rect and \Children
            assert_eq!(comp.body.len(), 2);
            match &comp.body[0] {
                ComponentBodyItem::Node(rect) => {
                    assert_eq!(rect.name.as_str(), "Rect");
                    assert_eq!(rect.ports.len(), 5);
                    assert_eq!(rect.ports[0].name.as_str(), "x");
                    assert_eq!(rect.ports[1].name.as_str(), "y");
                    assert_eq!(rect.ports[2].name.as_str(), "width");
                    assert_eq!(rect.ports[3].name.as_str(), "height");
                    assert_eq!(rect.ports[4].name.as_str(), "color");
                }
                _ => panic!("Expected Rect node"),
            }

            match &comp.body[1] {
                ComponentBodyItem::Children(children) => {
                    assert_eq!(children.ports.len(), 2);
                    assert_eq!(children.ports[0].name.as_str(), "x");
                    assert_eq!(children.ports[1].name.as_str(), "y");
                }
                _ => panic!("Expected Children directive"),
            }
        }
        _ => panic!("Expected component item"),
    }
}

#[test]
fn test_spec_flow_component() {
    let input = r#"
    \Component Flow(gap: Number: 16) {
      \Children {
        x: parent.left,
        y: prev ? prev.bottom + gap : parent.top
      }
    }
    "#;

    let doc = parse(input).expect("Failed to parse Flow component");
    assert_eq!(doc.items.len(), 1);

    match &doc.items[0] {
        Item::Component(comp) => {
            assert_eq!(comp.name.as_str(), "Flow");
            assert_eq!(comp.params.len(), 1);

            let param = &comp.params[0];
            assert_eq!(param.name.as_str(), "gap");
            assert_eq!(
                param.type_annotation.as_ref().map(|t| t.name.as_str()),
                Some("Number")
            );
            match &param.default_edge {
                Some(Expr::Literal(Literal::Number(n, _))) => assert_eq!(*n, 16.0),
                _ => panic!("Expected literal 16 default edge"),
            }

            assert_eq!(comp.body.len(), 1);
            match &comp.body[0] {
                ComponentBodyItem::Children(children) => {
                    assert_eq!(children.ports.len(), 2);

                    // x: parent.left
                    assert_eq!(children.ports[0].name.as_str(), "x");
                    match &children.ports[0].expr {
                        Expr::MemberAccess(m) => {
                            match m.target.as_ref() {
                                Expr::Ident(id) => assert_eq!(id.as_str(), "parent"),
                                _ => panic!("Expected parent ident"),
                            }
                            assert_eq!(m.member.as_str(), "left");
                        }
                        _ => panic!("Expected member access parent.left"),
                    }

                    // y: prev ? prev.bottom + gap : parent.top
                    assert_eq!(children.ports[1].name.as_str(), "y");
                    match &children.ports[1].expr {
                        Expr::Ternary(tern) => {
                            match tern.condition.as_ref() {
                                Expr::Ident(id) => assert_eq!(id.as_str(), "prev"),
                                _ => panic!("Expected prev in condition"),
                            }
                            match tern.then_expr.as_ref() {
                                Expr::Binary(bin) => {
                                    assert_eq!(bin.op, BinaryOp::Add);
                                    match bin.left.as_ref() {
                                        Expr::MemberAccess(m) => {
                                            assert_eq!(m.member.as_str(), "bottom");
                                        }
                                        _ => panic!("Expected prev.bottom"),
                                    }
                                }
                                _ => panic!("Expected binary then expr"),
                            }
                            match tern.else_expr.as_ref() {
                                Expr::MemberAccess(m) => {
                                    assert_eq!(m.member.as_str(), "top");
                                }
                                _ => panic!("Expected parent.top"),
                            }
                        }
                        _ => panic!("Expected ternary expression"),
                    }
                }
                _ => panic!("Expected Children directive"),
            }
        }
        _ => panic!("Expected component"),
    }
}

#[test]
fn test_spec_applying_flow_with_raw_text() {
    let input = r#"
    \Flow(gap: 24) {
      \Header(size: 32) { Welcome to DAG-UI }
      \Paragraph { This layout is mathematically provable. }
    }
    "#;

    let doc = parse(input).expect("Failed to parse applying flow");
    assert_eq!(doc.items.len(), 1);

    match &doc.items[0] {
        Item::Node(flow) => {
            assert_eq!(flow.name.as_str(), "Flow");
            assert_eq!(flow.ports.len(), 1);
            assert_eq!(flow.ports[0].name.as_str(), "gap");

            let content = flow.content.as_ref().expect("Expected Flow to have content");
            assert_eq!(content.items.len(), 2);

            // Child 1: \Header(size: 32) { Welcome to DAG-UI }
            match &content.items[0] {
                ContentItem::Node(header) => {
                    assert_eq!(header.name.as_str(), "Header");
                    assert_eq!(header.ports.len(), 1);
                    assert_eq!(header.ports[0].name.as_str(), "size");
                    let header_content = header.content.as_ref().expect("Header content");
                    assert_eq!(header_content.items.len(), 1);
                    match &header_content.items[0] {
                        ContentItem::Text(text) => {
                            assert_eq!(text.text, "Welcome to DAG-UI");
                        }
                        _ => panic!("Expected raw text in Header"),
                    }
                }
                _ => panic!("Expected Header node"),
            }

            // Child 2: \Paragraph { This layout is mathematically provable. }
            match &content.items[1] {
                ContentItem::Node(para) => {
                    assert_eq!(para.name.as_str(), "Paragraph");
                    assert_eq!(para.ports.len(), 0);
                    let para_content = para.content.as_ref().expect("Paragraph content");
                    assert_eq!(para_content.items.len(), 1);
                    match &para_content.items[0] {
                        ContentItem::Text(text) => {
                            assert_eq!(text.text, "This layout is mathematically provable.");
                        }
                        _ => panic!("Expected raw text in Paragraph"),
                    }
                }
                _ => panic!("Expected Paragraph node"),
            }
        }
        _ => panic!("Expected Node item"),
    }
}

#[test]
fn test_tex_escapes_and_whitespace_normalization() {
    let input = r#"
    \Text {
        Here is a backslash: \\
        and braces: \{ and \}
        with    multiple     spaces    and
        newlines.
    }
    "#;

    let doc = parse(input).expect("Failed to parse text with escapes");
    assert_eq!(doc.items.len(), 1);

    match &doc.items[0] {
        Item::Node(node) => {
            let slot = node.content.as_ref().unwrap();
            assert_eq!(slot.items.len(), 1);
            match &slot.items[0] {
                ContentItem::Text(chunk) => {
                    assert_eq!(
                        chunk.text,
                        "Here is a backslash: \\ and braces: { and } with multiple spaces and newlines."
                    );
                }
                _ => panic!("Expected text chunk"),
            }
        }
        _ => panic!("Expected node"),
    }
}

#[test]
fn test_inline_node_anchors_in_text() {
    let input = r#"
    \Paragraph {
      This is a complex sentence that will wrap across multiple lines.
      Right here \MarginNote(y: self.anchor.y) { This note tracks the word! } is where the note belongs.
    }
    "#;

    let doc = parse(input).expect("Failed to parse inline node in paragraph");
    assert_eq!(doc.items.len(), 1);

    match &doc.items[0] {
        Item::Node(para) => {
            let slot = para.content.as_ref().unwrap();
            assert_eq!(slot.items.len(), 3);

            // 1. Preceding text
            match &slot.items[0] {
                ContentItem::Text(chunk) => {
                    assert_eq!(
                        chunk.text,
                        "This is a complex sentence that will wrap across multiple lines. Right here "
                    );
                }
                _ => panic!("Expected preceding text"),
            }

            // 2. Inline MarginNote node
            match &slot.items[1] {
                ContentItem::Node(note) => {
                    assert_eq!(note.name.as_str(), "MarginNote");
                    assert_eq!(note.ports.len(), 1);
                    assert_eq!(note.ports[0].name.as_str(), "y");
                    // Expr is self.anchor.y
                    match &note.ports[0].expr {
                        Expr::MemberAccess(m1) => {
                            assert_eq!(m1.member.as_str(), "y");
                            match m1.target.as_ref() {
                                Expr::MemberAccess(m2) => {
                                    assert_eq!(m2.member.as_str(), "anchor");
                                    match m2.target.as_ref() {
                                        Expr::Ident(id) => assert_eq!(id.as_str(), "self"),
                                        _ => panic!("Expected self"),
                                    }
                                }
                                _ => panic!("Expected self.anchor"),
                            }
                        }
                        _ => panic!("Expected chained member access"),
                    }

                    // Content of note
                    let note_slot = note.content.as_ref().unwrap();
                    assert_eq!(note_slot.items.len(), 1);
                    match &note_slot.items[0] {
                        ContentItem::Text(t) => {
                            assert_eq!(t.text, "This note tracks the word!");
                        }
                        _ => panic!("Expected text inside note"),
                    }
                }
                _ => panic!("Expected inline node"),
            }

            // 3. Trailing text
            match &slot.items[2] {
                ContentItem::Text(chunk) => {
                    assert_eq!(chunk.text, " is where the note belongs.");
                }
                _ => panic!("Expected trailing text"),
            }
        }
        _ => panic!("Expected node"),
    }
}

#[test]
fn test_complex_algebraic_expressions() {
    let input = r#"
    \Rect(
      x: (parent.width / 2) - (self.width / 2),
      y: top + 10 * 2,
      z: parent.z - 1,
      visible: width > 100 && height > 50,
      color: #ffaa00
    )
    "#;

    let doc = parse(input).expect("Failed to parse algebraic expressions");
    match &doc.items[0] {
        Item::Node(rect) => {
            assert_eq!(rect.ports.len(), 5);

            // x: (parent.width / 2) - (self.width / 2)
            match &rect.ports[0].expr {
                Expr::Binary(bin) => {
                    assert_eq!(bin.op, BinaryOp::Sub);
                    match bin.left.as_ref() {
                        Expr::Paren(inner, _) => match inner.as_ref() {
                            Expr::Binary(b) => assert_eq!(b.op, BinaryOp::Div),
                            _ => panic!("Expected Div in left paren"),
                        },
                        _ => panic!("Expected Paren"),
                    }
                }
                _ => panic!("Expected Binary"),
            }

            // y: top + 10 * 2 (multiplication has higher precedence than addition)
            match &rect.ports[1].expr {
                Expr::Binary(bin) => {
                    assert_eq!(bin.op, BinaryOp::Add);
                    match bin.right.as_ref() {
                        Expr::Binary(mul) => assert_eq!(mul.op, BinaryOp::Mul),
                        _ => panic!("Expected Mul on right of Add"),
                    }
                }
                _ => panic!("Expected Binary Add"),
            }

            // z: parent.z - 1
            match &rect.ports[2].expr {
                Expr::Binary(bin) => {
                    assert_eq!(bin.op, BinaryOp::Sub);
                }
                _ => panic!("Expected Binary Sub"),
            }

            // visible: width > 100 && height > 50
            match &rect.ports[3].expr {
                Expr::Binary(bin) => {
                    assert_eq!(bin.op, BinaryOp::And);
                    match bin.left.as_ref() {
                        Expr::Binary(b) => assert_eq!(b.op, BinaryOp::Gt),
                        _ => panic!("Expected Gt"),
                    }
                }
                _ => panic!("Expected Binary And"),
            }

            // color: #ffaa00
            match &rect.ports[4].expr {
                Expr::Literal(Literal::Color(c, _)) => assert_eq!(c, "#ffaa00"),
                _ => panic!("Expected Color literal"),
            }
        }
        _ => panic!("Expected node"),
    }
}

#[test]
fn test_unclosed_content_brace_error() {
    let input = r#"\Paragraph { Unclosed text..."#;
    let err = parse(input).expect_err("Expected error for unclosed brace");
    match err {
        ParseError::UnclosedDelimiter { delimiter, .. } => {
            assert_eq!(delimiter, '}');
        }
        _ => panic!("Expected UnclosedDelimiter error, got {:?}", err),
    }
}

#[test]
fn test_unescaped_brace_in_text_error() {
    let input = r#"\Paragraph { hello { world } }"#;
    let err = parse(input).expect_err("Expected error for unescaped brace in text");
    match err {
        ParseError::Custom { message, .. } => {
            assert!(message.contains("Unescaped '{' in content text"));
        }
        _ => panic!("Expected Custom unescaped brace error, got {:?}", err),
    }
}

#[test]
fn test_full_document_with_components_and_nodes() {
    let input = r#"
    // Component definition
    \Component Flow(gap: Number: 16) {
      \Children {
        x: parent.left,
        y: prev ? prev.bottom + gap : parent.top
      }
    }

    /* Node invocation applying the flow */
    \Flow(gap: 24) {
      \Header(size: 32) { Welcome to DAG-UI }
      \Paragraph { This layout is mathematically provable. }
    }
    "#;

    let doc = parse(input).expect("Failed to parse combined document");
    assert_eq!(doc.items.len(), 2);

    match &doc.items[0] {
        Item::Component(c) => assert_eq!(c.name.as_str(), "Flow"),
        _ => panic!("Expected Component"),
    }

    match &doc.items[1] {
        Item::Node(n) => {
            assert_eq!(n.name.as_str(), "Flow");
            let slot = n.content.as_ref().unwrap();
            assert_eq!(slot.items.len(), 2);
        }
        _ => panic!("Expected Node"),
    }
}

#[test]
fn test_string_literals_in_ports() {
    let input = r#"\Text(font: "Inter", size: 18, color: #333333) { Hello World }"#;
    let doc = parse(input).expect("Failed to parse Text primitive with string port");
    match &doc.items[0] {
        Item::Node(text_node) => {
            assert_eq!(text_node.name.as_str(), "Text");
            assert_eq!(text_node.ports.len(), 3);
            match &text_node.ports[0].expr {
                Expr::Literal(Literal::String(s, _)) => assert_eq!(s, "Inter"),
                _ => panic!("Expected string literal for font"),
            }
            match &text_node.ports[1].expr {
                Expr::Literal(Literal::Number(n, _)) => assert_eq!(*n, 18.0),
                _ => panic!("Expected number literal for size"),
            }
            match &text_node.ports[2].expr {
                Expr::Literal(Literal::Color(c, _)) => assert_eq!(c, "#333333"),
                _ => panic!("Expected color literal"),
            }
            let slot = text_node.content.as_ref().unwrap();
            match &slot.items[0] {
                ContentItem::Text(t) => assert_eq!(t.text, "Hello World"),
                _ => panic!("Expected text content"),
            }
        }
        _ => panic!("Expected node"),
    }
}

#[test]
fn test_formatted_parse_error_location() {
    let input = r#"
    \Component Test {
      invalid_syntax
    }
    "#;
    let err = parse(input).expect_err("Expected parse error for invalid syntax");
    let formatted = err.display_with_source(input).to_string();
    assert!(formatted.contains("Parse error at line 3, column"));
}

#[test]
fn test_parse_let_bindings() {
    let input = r#"
    let global_gap = 24;

    \Component Card(padding: Number = 16) {
      let inset: Number = padding * 2;
      let mask = \Rect(width: 100, height: 50)
      \Rect(x: mask.right + inset)
    }
    "#;

    let doc = parse(input).expect("Failed to parse document with let bindings");
    assert_eq!(doc.items.len(), 2);

    // Item 0: Top-level let
    match &doc.items[0] {
        Item::Let(l) => {
            assert_eq!(l.name.as_str(), "global_gap");
            match &l.value {
                Some(LetValue::Expr(Expr::Literal(Literal::Number(n, _)))) => assert_eq!(*n, 24.0),
                _ => panic!("Expected number literal for global_gap"),
            }
        }
        _ => panic!("Expected Item::Let"),
    }

    // Item 1: Component Card
    match &doc.items[1] {
        Item::Component(c) => {
            assert_eq!(c.name.as_str(), "Card");
            assert_eq!(c.body.len(), 3);
            match &c.body[0] {
                ComponentBodyItem::Let(l) => {
                    assert_eq!(l.name.as_str(), "inset");
                    assert_eq!(l.type_annotation.as_ref().unwrap().name.as_str(), "Number");
                }
                _ => panic!("Expected ComponentBodyItem::Let"),
            }
            match &c.body[1] {
                ComponentBodyItem::Let(l) => {
                    assert_eq!(l.name.as_str(), "mask");
                    match &l.value {
                        Some(LetValue::Node(n)) => assert_eq!(n.name.as_str(), "Rect"),
                        _ => panic!("Expected LetValue::Node for mask"),
                    }
                }
                _ => panic!("Expected ComponentBodyItem::Let for mask"),
            }
            match &c.body[2] {
                ComponentBodyItem::Node(n) => assert_eq!(n.name.as_str(), "Rect"),
                _ => panic!("Expected ComponentBodyItem::Node"),
            }
        }
        _ => panic!("Expected Item::Component"),
    }
}

#[test]
fn test_component_definition_rejects_plain_text() {
    let input = r#"
    \Component Card {
      some invalid plain text
      \Rect(width: 100)
    }
    "#;
    let err = parse(input).expect_err("Component definition should reject plain text");
    let err_str = err.to_string();
    assert!(
        err_str.contains("Expected") || err_str.contains("expected"),
        "Expected unexpected token error, got: {}",
        err_str
    );
}

#[test]
fn test_component_literal_treats_let_as_plain_text() {
    let input = r#"
    \Paragraph {
      let x = 10;
    }
    "#;
    let doc = parse(input).expect("Failed to parse component literal with text containing let");
    assert_eq!(doc.items.len(), 1);

    match &doc.items[0] {
        Item::Node(node) => {
            assert_eq!(node.name.as_str(), "Paragraph");
            let content = node.content.as_ref().expect("Expected content");
            assert_eq!(content.items.len(), 1);
            match &content.items[0] {
                ContentItem::Text(text) => {
                    assert_eq!(text.text, "let x = 10;");
                }
                _ => panic!("Expected ContentItem::Text"),
            }
        }
        _ => panic!("Expected Item::Node"),
    }
}

#[test]
fn test_component_literal_supports_mixed_text_and_nodes() {
    let input = r#"
    \Container {
      Leading text
      \Button(width: 80) { Click }
      Trailing text
    }
    "#;
    let doc = parse(input).expect("Failed to parse mixed content");
    assert_eq!(doc.items.len(), 1);

    match &doc.items[0] {
        Item::Node(container) => {
            assert_eq!(container.name.as_str(), "Container");
            let content = container.content.as_ref().expect("Expected content");
            assert_eq!(content.items.len(), 3);
            match &content.items[0] {
                ContentItem::Text(t) => assert_eq!(t.text.trim(), "Leading text"),
                _ => panic!("Expected Text"),
            }
            match &content.items[1] {
                ContentItem::Node(b) => assert_eq!(b.name.as_str(), "Button"),
                _ => panic!("Expected Button node"),
            }
            match &content.items[2] {
                ContentItem::Text(t) => assert_eq!(t.text.trim(), "Trailing text"),
                _ => panic!("Expected Text"),
            }
        }
        _ => panic!("Expected Container node"),
    }
}

#[test]
fn test_parse_env_declarations_and_uninitialized_tombstones() {
    let input = r#"
    env theme = "dark";
    let uninit_let;
    env uninit_env;

    \Component Card(env padding: Number: 16, border: Number: 1) {
        let private_state;
        env local_env = #ffffff;
        env hole_env;

        \Rect()
        \Children
    }
    "#;

    let doc = parse(input).expect("Failed to parse env declarations and tombstones");
    assert_eq!(doc.items.len(), 4);

    match &doc.items[0] {
        Item::Env(e) => {
            assert_eq!(e.name.as_str(), "theme");
            assert!(e.value.is_some());
        }
        _ => panic!("Expected Item::Env"),
    }

    match &doc.items[1] {
        Item::Let(l) => {
            assert_eq!(l.name.as_str(), "uninit_let");
            assert!(l.value.is_none());
        }
        _ => panic!("Expected Item::Let"),
    }

    match &doc.items[2] {
        Item::Env(e) => {
            assert_eq!(e.name.as_str(), "uninit_env");
            assert!(e.value.is_none());
        }
        _ => panic!("Expected Item::Env"),
    }

    match &doc.items[3] {
        Item::Component(c) => {
            assert_eq!(c.name.as_str(), "Card");
            assert!(c.params[0].is_env);
            assert_eq!(c.params[0].name.as_str(), "padding");
            assert!(!c.params[1].is_env);
            assert_eq!(c.params[1].name.as_str(), "border");

            match &c.body[0] {
                ComponentBodyItem::Let(l) => {
                    assert_eq!(l.name.as_str(), "private_state");
                    assert!(l.value.is_none());
                }
                _ => panic!("Expected uninit let"),
            }
            match &c.body[1] {
                ComponentBodyItem::Env(e) => {
                    assert_eq!(e.name.as_str(), "local_env");
                    assert!(e.value.is_some());
                }
                _ => panic!("Expected local env"),
            }
            match &c.body[2] {
                ComponentBodyItem::Env(e) => {
                    assert_eq!(e.name.as_str(), "hole_env");
                    assert!(e.value.is_none());
                }
                _ => panic!("Expected hole env"),
            }
            match &c.body[4] {
                ComponentBodyItem::Children(ch) => {
                    assert!(ch.ports.is_empty());
                }
                _ => panic!("Expected bare Children directive"),
            }
        }
        _ => panic!("Expected Item::Component"),
    }
}

#[test]
fn test_parse_env_member_access() {
    let input = r#"
    \Foo(color: env.color) {
        \Children {
            bg: env.card_bg
        }
    }

    let is_dark = env.theme == "dark" ? true : false;
    "#;

    let doc = parse(input).expect("Failed to parse env member access");
    assert_eq!(doc.items.len(), 2);

    match &doc.items[0] {
        Item::Node(elem) => {
            assert_eq!(elem.name.as_str(), "Foo");
            assert_eq!(elem.ports.len(), 1);
            assert_eq!(elem.ports[0].name.as_str(), "color");
            match &elem.ports[0].expr {
                Expr::MemberAccess(m) => {
                    match m.target.as_ref() {
                        Expr::Ident(id) => assert_eq!(id.as_str(), "env"),
                        _ => panic!("Expected target to be ident 'env'"),
                    }
                    assert_eq!(m.member.as_str(), "color");
                }
                _ => panic!("Expected Expr::MemberAccess"),
            }
        }
        _ => panic!("Expected Item::Node"),
    }

    match &doc.items[1] {
        Item::Let(l) => {
            assert_eq!(l.name.as_str(), "is_dark");
            match &l.value {
                Some(LetValue::Expr(Expr::Ternary(tern))) => {
                    match tern.condition.as_ref() {
                        Expr::Binary(bin) => {
                            match bin.left.as_ref() {
                                Expr::MemberAccess(m) => {
                                    match m.target.as_ref() {
                                        Expr::Ident(id) => assert_eq!(id.as_str(), "env"),
                                        _ => panic!("Expected target 'env'"),
                                    }
                                    assert_eq!(m.member.as_str(), "theme");
                                }
                                _ => panic!("Expected member access"),
                            }
                        }
                        _ => panic!("Expected binary condition"),
                    }
                }
                _ => panic!("Expected ternary expr"),
            }
        }
        _ => panic!("Expected Item::Let"),
    }
}
