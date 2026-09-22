use directedtype::ast::*;
use directedtype::compiler::{compile_to_graph, VarId};
use directedtype::parse;
use directedtype::span::Span;
use pretty_assertions::assert_eq;

#[test]
fn test_expand_flow_recurrence_and_graph() {
    let input = r#"
    \Component Flow(gap: Number: 16) {
      \Children {
        x: parent.left,
        y: prev ? prev.bottom + gap : parent.top
      }
    }

    \Flow(gap: 24) {
      \Header(size: 32) { Welcome to DAG-UI }
      \Paragraph { This layout is mathematically provable. }
    }
    "#;

    let doc = parse(input).expect("Failed to parse document");
    let (expanded, graph) = compile_to_graph(&doc).expect("Failed to compile to graph");

    // We have 3 expanded nodes:
    // Node 0: \Flow container
    // Node 1: \Header
    // Node 2: \Paragraph
    assert_eq!(expanded.roots.len(), 1);
    let flow_id = expanded.roots[0];
    assert_eq!(flow_id.0, 0);

    let flow_node = expanded.get_node(flow_id).unwrap();
    assert_eq!(flow_node.name, "Flow");
    assert_eq!(flow_node.children.len(), 2);

    let header_id = flow_node.children[0];
    let para_id = flow_node.children[1];
    assert_eq!(header_id.0, 1);
    assert_eq!(para_id.0, 2);

    let header_node = expanded.get_node(header_id).unwrap();
    assert_eq!(header_node.name, "Header");
    assert_eq!(header_node.parent, Some(flow_id));
    assert_eq!(header_node.prev_sibling, None);

    let para_node = expanded.get_node(para_id).unwrap();
    assert_eq!(para_node.name, "Paragraph");
    assert_eq!(para_node.parent, Some(flow_id));
    assert_eq!(para_node.prev_sibling, Some(header_id));

    // 1. Verify Header (Child 0) Base-Case Expansion:
    // x: parent.left -> __node_0.x
    match header_node.ports.get("x").unwrap() {
        Expr::MemberAccess(m) => {
            assert_eq!(m.member.as_str(), "x");
            match m.target.as_ref() {
                Expr::Ident(id) => assert_eq!(id.as_str(), "__node_0"),
                _ => panic!("Expected target ident"),
            }
        }
        _ => panic!("Expected MemberAccess for Header.x"),
    }

    // y: parent.top -> __node_0.y (since prev is None, base case was chosen)
    match header_node.ports.get("y").unwrap() {
        Expr::MemberAccess(m) => {
            assert_eq!(m.member.as_str(), "y");
            match m.target.as_ref() {
                Expr::Ident(id) => assert_eq!(id.as_str(), "__node_0"),
                _ => panic!("Expected target ident"),
            }
        }
        _ => panic!("Expected MemberAccess for Header.y"),
    }

    // 2. Verify Paragraph (Child 1) Recurrence Step Expansion:
    // x: parent.left -> __node_0.x
    // y: prev.bottom + gap -> (__node_1.y + __node_1.height) + __node_0.gap
    let para_y = para_node.ports.get("y").unwrap();
    match para_y {
        Expr::Binary(bin) => {
            assert_eq!(bin.op, BinaryOp::Add);
            // Right is __node_0.gap
            match bin.right.as_ref() {
                Expr::MemberAccess(m) => {
                    assert_eq!(m.member.as_str(), "gap");
                    match m.target.as_ref() {
                        Expr::Ident(id) => assert_eq!(id.as_str(), "__node_0"),
                        _ => panic!("Expected target __node_0"),
                    }
                }
                _ => panic!("Expected gap member access"),
            }
            // Left is (__node_1.y + __node_1.height)
            match bin.left.as_ref() {
                Expr::Binary(inner_add) => {
                    assert_eq!(inner_add.op, BinaryOp::Add);
                    match inner_add.left.as_ref() {
                        Expr::MemberAccess(m) => assert_eq!(m.member.as_str(), "y"),
                        _ => panic!("Expected y"),
                    }
                    match inner_add.right.as_ref() {
                        Expr::MemberAccess(m) => assert_eq!(m.member.as_str(), "height"),
                        _ => panic!("Expected height"),
                    }
                }
                _ => panic!("Expected inner add for bottom"),
            }
        }
        _ => panic!("Expected Binary expression for Paragraph.y"),
    }

    // 3. Verify Graph Dependencies:
    let var_flow_gap = VarId::new(flow_id, "gap");
    let var_flow_x = VarId::new(flow_id, "x");
    let var_flow_y = VarId::new(flow_id, "y");
    let var_header_x = VarId::new(header_id, "x");
    let var_header_y = VarId::new(header_id, "y");
    let var_header_h = VarId::new(header_id, "height");
    let var_para_x = VarId::new(para_id, "x");
    let var_para_y = VarId::new(para_id, "y");

    // Header.x depends on Flow.x
    assert_eq!(graph.upstream.get(&var_header_x).unwrap(), &vec![var_flow_x.clone()]);
    // Header.y depends on Flow.y
    assert_eq!(graph.upstream.get(&var_header_y).unwrap(), &vec![var_flow_y.clone()]);

    // Paragraph.x depends on Flow.x
    assert_eq!(graph.upstream.get(&var_para_x).unwrap(), &vec![var_flow_x.clone()]);

    // Paragraph.y depends on Flow.gap, Header.height, and Header.y
    let para_y_upstream = graph.upstream.get(&var_para_y).unwrap();
    assert!(para_y_upstream.contains(&var_flow_gap));
    assert!(para_y_upstream.contains(&var_header_h));
    assert!(para_y_upstream.contains(&var_header_y));

    // Downstream check: Flow.gap must have Paragraph.y in its downstream list!
    let flow_gap_downstream = graph.downstream.get(&var_flow_gap).unwrap();
    assert!(flow_gap_downstream.contains(&var_para_y));
}

#[test]
fn test_precedence_explicit_overrides_ambient() {
    let input = r#"
    \Component Flow(gap: Number: 16) {
      \Children {
        x: parent.left
      }
    }

    \Flow {
      \Paragraph(x: 100) { Explicit Position }
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let (expanded, _) = compile_to_graph(&doc).expect("Failed to compile");

    let para_id = expanded.nodes[0].children[0];
    let para = expanded.get_node(para_id).unwrap();

    // The explicit x: 100 should override ambient x: parent.left
    match para.ports.get("x").unwrap() {
        Expr::Literal(Literal::Number(n, _)) => assert_eq!(*n, 100.0),
        other => panic!("Expected Literal 100 for x, got {:?}", other),
    }
}

#[test]
fn test_expand_shaded_box_intrinsic_children_width() {
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

    \ShadedBox(bg_color: #333333) {
      \Button(width: 120) { Cancel }
      \Button(width: 250) { Confirm }
    }
    "#;

    let doc = parse(input).expect("Failed to parse ShadedBox");
    let (expanded, graph) = compile_to_graph(&doc).expect("Failed to compile");

    let box_id = expanded.roots[0];
    let box_node = expanded.get_node(box_id).unwrap();

    // Children of ShadedBox should be the 1 rect (declared first) and 2 buttons
    // Rect: Node 1
    // Button 1: Node 2
    // Button 2: Node 3
    assert_eq!(box_node.children.len(), 3);
    let rect_id = box_node.children[0];
    let btn1_id = box_node.children[1];
    let btn2_id = box_node.children[2];

    assert_eq!(expanded.get_node(rect_id).unwrap().name, "Rect");
    assert_eq!(expanded.get_node(btn1_id).unwrap().name, "Button");
    assert_eq!(expanded.get_node(btn2_id).unwrap().name, "Button");

    // ShadedBox width should be max(__node_1.width, __node_2.width) + 32
    let box_width = box_node.ports.get("width").unwrap();
    match box_width {
        Expr::Binary(bin) => {
            assert_eq!(bin.op, BinaryOp::Add);
            match bin.left.as_ref() {
                Expr::Call(call) => {
                    assert_eq!(call.callee.as_str(), "max");
                    assert_eq!(call.args.len(), 2);
                }
                _ => panic!("Expected Call to max"),
            }
        }
        _ => panic!("Expected Binary expression for ShadedBox width"),
    }

    // Graph checks:
    let var_box_width = VarId::new(box_id, "width");
    let var_btn1_width = VarId::new(btn1_id, "width");
    let var_btn2_width = VarId::new(btn2_id, "width");
    let var_rect_width = VarId::new(rect_id, "width");

    // ShadedBox.width depends on Button 1 width and Button 2 width
    let box_w_upstream = graph.upstream.get(&var_box_width).unwrap();
    assert!(box_w_upstream.contains(&var_btn1_width));
    assert!(box_w_upstream.contains(&var_btn2_width));

    // Rect.width depends on ShadedBox.width
    let rect_w_upstream = graph.upstream.get(&var_rect_width).unwrap();
    assert!(rect_w_upstream.contains(&var_box_width));
}

#[test]
fn test_text_wrapping_default_height() {
    let input = r#"
    \Text(width: 200) { This is a long sentence for text wrapping test. }
    "#;

    let doc = parse(input).expect("Failed to parse Text");
    let (expanded, graph) = compile_to_graph(&doc).expect("Failed to compile");

    let text_node = expanded.get_node(expanded.roots[0]).unwrap();
    assert!(text_node.ports.contains_key("height"));

    let var_w = VarId::new(text_node.id, "width");
    let var_h = VarId::new(text_node.id, "height");

    // height depends on width
    let h_upstream = graph.upstream.get(&var_h).unwrap();
    assert!(h_upstream.contains(&var_w));

    let w_downstream = graph.downstream.get(&var_w).unwrap();
    assert!(w_downstream.contains(&var_h));
}

#[test]
fn test_topological_sort_acyclic() {
    let input = r#"
    \Component Flow(gap: Number: 16) {
      \Children {
        x: parent.left,
        y: prev ? prev.bottom + gap : parent.top
      }
    }

    \Flow(gap: 24) {
      \Header(size: 32) { Welcome to DAG-UI }
      \Paragraph { This layout is mathematically provable. }
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let (_, graph, schedule) = directedtype::compiler::compile_and_sort(&doc)
        .expect("Topological sort should succeed for acyclic graph");

    // Total variables scheduled must equal total variables in graph
    assert_eq!(schedule.len(), graph.variables.len());

    // Map each variable to its index in the topological execution order
    let mut order_map = std::collections::HashMap::new();
    for (idx, var_id) in schedule.order.iter().enumerate() {
        order_map.insert(var_id.clone(), idx);
    }

    // Mathematical verification: For EVERY variable in the schedule,
    // all of its upstream dependencies MUST appear strictly BEFORE it!
    for var_id in &schedule.order {
        let var_idx = order_map[var_id];
        if let Some(upstream_deps) = graph.upstream.get(var_id) {
            for dep in upstream_deps {
                let dep_idx = order_map[dep];
                assert!(
                    dep_idx < var_idx,
                    "Variable {var_id} at index {var_idx} scheduled before its dependency {dep} at index {dep_idx}!"
                );
            }
        }
    }
}

#[test]
fn test_cycle_detection_fit_content_paradox() {
    // The "fit-content paradox" from SPEC.md and PDF:
    // Container width depends on child width, while child width depends on container width.
    let input = r#"
    \Component ParadoxBox(width: max(children.width)) {
      \Children {
        width: parent.width
      }
    }

    \ParadoxBox {
      \Button { Click Me }
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let err = directedtype::compiler::compile_and_sort(&doc)
        .expect_err("Expected cyclic dependency error for fit-content paradox");

    match &err {
        directedtype::compiler::CompileError::CyclicDependency { cycle, .. } => {
            assert!(
                cycle.len() >= 2,
                "Cycle must contain at least 2 nodes, got: {:?}",
                cycle
            );
            // Cycle starts and ends on the same variable
            assert_eq!(cycle.first(), cycle.last());

            let err_msg = err.to_string();
            assert!(
                err_msg.contains("Cyclic dependency detected"),
                "Error message should mention cyclic dependency: {}",
                err_msg
            );
            assert!(
                err_msg.contains("width"),
                "Error message should mention the cyclic width variable: {}",
                err_msg
            );
        }
        other => panic!("Expected CyclicDependency error, got: {:?}", other),
    }
}

#[test]
fn test_multi_node_cycle_detection() {
    let mut graph = directedtype::compiler::VariableGraph::new();

    let node_a = directedtype::compiler::NodeId(0);
    let node_b = directedtype::compiler::NodeId(1);
    let node_c = directedtype::compiler::NodeId(2);

    let var_a = directedtype::compiler::VarId::new(node_a, "x");
    let var_b = directedtype::compiler::VarId::new(node_b, "x");
    let var_c = directedtype::compiler::VarId::new(node_c, "x");

    // A.x depends on C.x
    let eq_a = Expr::MemberAccess(MemberAccessExpr {
        target: Box::new(Expr::Ident(Ident::new("__node_2", Span::default()))),
        member: Ident::new("x", Span::default()),
        span: Span::default(),
    });
    // B.x depends on A.x
    let eq_b = Expr::MemberAccess(MemberAccessExpr {
        target: Box::new(Expr::Ident(Ident::new("__node_0", Span::default()))),
        member: Ident::new("x", Span::default()),
        span: Span::default(),
    });
    // C.x depends on B.x
    let eq_c = Expr::MemberAccess(MemberAccessExpr {
        target: Box::new(Expr::Ident(Ident::new("__node_1", Span::default()))),
        member: Ident::new("x", Span::default()),
        span: Span::default(),
    });

    graph.add_variable(var_a, eq_a, Span::default());
    graph.add_variable(var_b, eq_b, Span::default());
    graph.add_variable(var_c, eq_c, Span::default());

    let err = directedtype::compiler::sort_graph(&graph)
        .expect_err("3-node cycle must be detected");

    match err {
        directedtype::compiler::CompileError::CyclicDependency { cycle, .. } => {
            assert!(cycle.len() >= 3);
            assert_eq!(cycle.first(), cycle.last());
        }
        other => panic!("Expected CyclicDependency error, got: {:?}", other),
    }
}

#[test]
fn test_end_to_end_flow_math_evaluation() {
    let input = r#"
    \Component Flow(gap: Number: 16) {
      \Children {
        x: parent.left,
        y: prev ? prev.bottom + gap : parent.top
      }
    }

    \Flow(gap: 24) {
      \Header(size: 32) { Welcome to DAG-UI }
      \Paragraph { This layout is mathematically provable. }
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Failed to evaluate layout");

    assert_eq!(layout.nodes.len(), 3);

    // Node 0: Flow
    let flow = &layout.nodes[0];
    assert_eq!(flow.name, "Flow");
    assert_eq!(flow.rect.x, 0.0);
    assert_eq!(flow.rect.y, 0.0);

    // Node 1: Header
    let header = &layout.nodes[1];
    assert_eq!(header.name, "Header");
    assert_eq!(header.rect.x, 0.0);
    assert_eq!(header.rect.y, 0.0);
    assert_eq!(header.rect.height, 32.0);
    assert_eq!(header.text_content.as_deref(), Some("Welcome to DAG-UI"));

    // Node 2: Paragraph
    // Paragraph y = Header.bottom + gap = (0.0 + 32.0) + 24.0 = 56.0!
    let para = &layout.nodes[2];
    assert_eq!(para.name, "Paragraph");
    assert_eq!(para.rect.x, 0.0);
    assert_eq!(para.rect.y, 56.0);
    assert_eq!(
        para.text_content.as_deref(),
        Some("This layout is mathematically provable.")
    );
}

#[test]
fn test_end_to_end_shaded_box_bottom_up_evaluation() {
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

    \ShadedBox(bg_color: #333333) {
      \Button(width: 120) { Cancel }
      \Button(width: 250) { Confirm }
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Failed to evaluate layout");

    // Nodes:
    // 0: ShadedBox
    // 1: Rect (background, declared first)
    // 2: Button 1 (width 120)
    // 3: Button 2 (width 250)
    let shaded_box = &layout.nodes[0];
    let rect = &layout.nodes[1];
    let btn1 = &layout.nodes[2];
    let btn2 = &layout.nodes[3];

    // ShadedBox width = max(120, 250) + 32 = 250 + 32 = 282!
    assert_eq!(shaded_box.rect.width, 282.0);

    // Rect width matches ShadedBox width!
    assert_eq!(rect.rect.width, 282.0);
    assert_eq!(rect.rect.x, 0.0);
    assert_eq!(
        rect.properties.get("color"),
        Some(&directedtype::Value::Color("#333333".to_string()))
    );

    // Buttons are padded by 16px
    assert_eq!(btn1.rect.x, 16.0);
    assert_eq!(btn1.rect.width, 120.0);

    assert_eq!(btn2.rect.x, 16.0);
    assert_eq!(btn2.rect.width, 250.0);
}

#[test]
fn test_painters_algorithm_and_z_ordering() {
    let input = r#"
    \Rect(z: 0)
    \Rect(z: 0)
    \Rect(z: -10)
    \Rect(z: 100)
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Failed to evaluate layout");

    let render_order = layout.render_order();
    assert_eq!(render_order.len(), 4);

    // Node 2 has z: -10 -> rendered first
    assert_eq!(render_order[0].id.0, 2);
    // Node 0 has z: 0 (index 0) -> rendered second
    assert_eq!(render_order[1].id.0, 0);
    // Node 1 has z: 0 (index 1) -> rendered third
    assert_eq!(render_order[2].id.0, 1);
    // Node 3 has z: 100 -> rendered last (on top of everything)
    assert_eq!(render_order[3].id.0, 3);
}

#[test]
fn test_global_window_dimensions_access() {
    let input = r#"
    \Component Container {
        \Children {
            width: window.width / 2,
            height: window.height - 100
        }
    }

    \Container {
        \Rect(color: #3b82f6)
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document_with_window(&doc, 1024.0, 768.0)
        .expect("Layout evaluation should succeed");

    assert_eq!(layout.nodes.len(), 2);
    let rect = &layout.nodes[1];
    assert_eq!(rect.name, "Rect");
    assert_eq!(rect.rect.width, 512.0); // 1024 / 2
    assert_eq!(rect.rect.height, 668.0); // 768 - 100
}

#[test]
fn test_top_level_parent_resolves_to_window() {
    let input = r#"
    \Rect(
        x: parent.left + 50,
        y: parent.top + 30,
        width: parent.width - 100,
        height: parent.height - 60
    )
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document_with_window(&doc, 1440.0, 900.0)
        .expect("Layout evaluation should succeed");

    assert_eq!(layout.nodes.len(), 1);
    let rect = &layout.nodes[0];
    assert_eq!(rect.rect.x, 50.0);
    assert_eq!(rect.rect.y, 30.0);
    assert_eq!(rect.rect.width, 1340.0); // 1440 - 100
    assert_eq!(rect.rect.height, 840.0); // 900 - 60
}

#[test]
fn test_responsive_flow_clamped_formula() {
    let input = r#"
    \Component Flow {
        \Children {
            width: min(max(window.width, 400), 700)
        }
    }

    \Flow {
        \Text { Responsive DirectedType }
    }
    "#;

    let doc = parse(input).expect("Failed to parse");

    // Case 1: Below minimum 400 -> clamped to 400
    let layout_narrow = directedtype::evaluate_document_with_window(&doc, 300.0, 600.0)
        .expect("Evaluation should succeed");
    assert_eq!(layout_narrow.nodes[1].rect.width, 400.0);

    // Case 2: Intermediate width -> responsive
    let layout_mid = directedtype::evaluate_document_with_window(&doc, 550.0, 600.0)
        .expect("Evaluation should succeed");
    assert_eq!(layout_mid.nodes[1].rect.width, 550.0);

    // Case 3: Above maximum 700 -> clamped to 700
    let layout_wide = directedtype::evaluate_document_with_window(&doc, 1200.0, 600.0)
        .expect("Evaluation should succeed");
    assert_eq!(layout_wide.nodes[1].rect.width, 700.0);
}
