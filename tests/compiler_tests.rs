use directedtype::ast::*;
use directedtype::compiler::error::CompileError;
use directedtype::compiler::{compile_to_graph, VarId};
use directedtype::parse;
use directedtype::span::Span;
use directedtype::Value;
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
    // x: parent.left -> __node_0.left
    match header_node.ports.get("x").unwrap() {
        Expr::MemberAccess(m) => {
            assert_eq!(m.member.as_str(), "left");
            match m.target.as_ref() {
                Expr::Ident(id) => assert_eq!(id.as_str(), "__node_0"),
                _ => panic!("Expected target ident"),
            }
        }
        _ => panic!("Expected MemberAccess for Header.x"),
    }

    // y: parent.top -> __node_0.top (since prev is None, base case was chosen)
    match header_node.ports.get("y").unwrap() {
        Expr::MemberAccess(m) => {
            assert_eq!(m.member.as_str(), "top");
            match m.target.as_ref() {
                Expr::Ident(id) => assert_eq!(id.as_str(), "__node_0"),
                _ => panic!("Expected target ident"),
            }
        }
        _ => panic!("Expected MemberAccess for Header.y"),
    }

    // 2. Verify Paragraph (Child 1) Recurrence Step Expansion:
    // x: parent.left -> __node_0.left
    // y: prev.bottom + gap -> __node_1.bottom + __node_0.gap
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
            // Left is __node_1.bottom
            match bin.left.as_ref() {
                Expr::MemberAccess(m) => {
                    assert_eq!(m.member.as_str(), "bottom");
                    match m.target.as_ref() {
                        Expr::Ident(id) => assert_eq!(id.as_str(), "__node_1"),
                        _ => panic!("Expected target __node_1"),
                    }
                }
                _ => panic!("Expected member access for bottom"),
            }
        }
        _ => panic!("Expected Binary expression for Paragraph.y"),
    }

    // 3. Verify Graph Dependencies:
    let var_flow_gap = VarId::new(flow_id, "gap");
    let var_flow_left = VarId::new(flow_id, "left");
    let var_flow_top = VarId::new(flow_id, "top");
    let var_header_x = VarId::new(header_id, "x");
    let var_header_y = VarId::new(header_id, "y");
    let var_header_bottom = VarId::new(header_id, "bottom");
    let var_header_h = VarId::new(header_id, "height");
    let var_para_x = VarId::new(para_id, "x");
    let var_para_y = VarId::new(para_id, "y");

    // Header.x depends on Flow.left
    assert_eq!(graph.upstream.get(&var_header_x).unwrap(), &vec![var_flow_left.clone()]);
    // Header.y depends on Flow.top
    assert_eq!(graph.upstream.get(&var_header_y).unwrap(), &vec![var_flow_top.clone()]);

    // Paragraph.x depends on Flow.left
    assert_eq!(graph.upstream.get(&var_para_x).unwrap(), &vec![var_flow_left.clone()]);

    // Paragraph.y depends on Flow.gap and Header.bottom
    let para_y_upstream = graph.upstream.get(&var_para_y).unwrap();
    assert!(para_y_upstream.contains(&var_flow_gap));
    assert!(para_y_upstream.contains(&var_header_bottom));

    // Header.bottom depends on Header.y and Header.height
    let header_bottom_upstream = graph.upstream.get(&var_header_bottom).unwrap();
    assert!(header_bottom_upstream.contains(&var_header_y));
    assert!(header_bottom_upstream.contains(&var_header_h));

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
    \Text(width: 200, size: 16) { This is a long sentence for text wrapping test. }
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
    assert_eq!(header.rect.height, 22.72);
    assert_eq!(header.text_content.as_deref(), Some("Welcome to DAG-UI"));

    // Node 2: Paragraph
    // Paragraph y = Header.bottom + gap = (0.0 + 22.72) + 24.0 = 46.72!
    let para = &layout.nodes[2];
    assert_eq!(para.name, "Paragraph");
    assert_eq!(para.rect.x, 0.0);
    assert_eq!(para.rect.y, 46.72);
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
    \Rect(x: 0, y: 0, width: 100, height: 100, z: 0, color: #000)
    \Rect(x: 0, y: 0, width: 100, height: 100, z: 0, color: #000)
    \Rect(x: 0, y: 0, width: 100, height: 100, z: -10, color: #000)
    \Rect(x: 0, y: 0, width: 100, height: 100, z: 100, color: #000)
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
            x: 0,
            y: 0,
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
        height: parent.height - 60,
        color: #000
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
        \Text(size: 16) { Responsive DirectedType }
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

#[test]
fn test_parley_text_height_wrapping_multiline() {
    let input = r#"
    \Component Flow(gap: Number: 24) {
        \Children {
            x: parent.left,
            y: prev ? prev.bottom + gap : parent.top,
            width: 320
        }
    }

    \Flow {
        \Text(size: 36, weight: 700) {
            DirectedType Native Layout Engine
        }
        \Text(size: 18) {
            A pure functional reactive layout engine
        }
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation failed");

    assert_eq!(layout.nodes.len(), 3);
    let title = &layout.nodes[1];
    let subtitle = &layout.nodes[2];

    assert_eq!(title.name, "Text");
    assert_eq!(title.rect.x, 0.0);
    assert_eq!(title.rect.y, 0.0);
    assert_eq!(title.rect.width, 320.0);

    // Parley shapes 3 lines at size 36 bold: height is ~97.56px (not 67px for 2 lines)
    assert!(
        title.rect.height > 90.0,
        "Expected height > 90px for 3 wrapped lines of size 36, got: {}",
        title.rect.height
    );

    // Subtitle must be positioned strictly below the title + gap (y >= title.bottom + 24)
    assert!(
        subtitle.rect.y >= title.rect.bottom() + 23.9,
        "Subtitle y ({}) must be >= title.bottom + 24 ({})",
        subtitle.rect.y,
        title.rect.bottom() + 24.0
    );
}

#[test]
fn test_parley_intrinsic_text_width_unconstrained() {
    let input = r#"
    \Text(size: 20) { Hello World }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation failed");

    assert_eq!(layout.nodes.len(), 1);
    let text_node = &layout.nodes[0];

    // Unconstrained width must be measured via Parley (> 50px, single line)
    assert!(
        text_node.rect.width > 50.0 && text_node.rect.width < 250.0,
        "Expected realistic intrinsic text width, got: {}",
        text_node.rect.width
    );
    // Height must be single line baseline height (~15px)
    assert!(
        text_node.rect.height > 14.0 && text_node.rect.height < 40.0,
        "Expected single line height, got: {}",
        text_node.rect.height
    );
}

#[test]
fn test_required_port_missing_error() {
    let input = r#"
    \Component ProgressBar(progress_percentage: Number) {
      \Rect(width: progress_percentage, height: 20, color: #000)
    }

    \ProgressBar()
    "#;

    let doc = parse(input).expect("Failed to parse");
    let err = directedtype::compiler::compile_to_graph(&doc)
        .expect_err("Expected compile error for missing required port");

    match err {
        directedtype::compiler::CompileError::MissingPort { node, port, .. } => {
            assert_eq!(node, "ProgressBar");
            assert_eq!(port, "progress_percentage");
        }
        other => panic!("Expected MissingPort error, got: {:?}", other),
    }
}

#[test]
fn test_required_port_provided_by_instance() {
    let input = r#"
    \Component ProgressBar(progress_percentage: Number) {
      \Rect(x: 0, y: 0, width: progress_percentage, height: 20, color: #000)
    }

    \ProgressBar(progress_percentage: 75)
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation failed");

    // Node 0: ProgressBar container; Node 1: Inner Rect
    let rect_node = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    assert_eq!(rect_node.rect.width, 75.0);
}

#[test]
fn test_required_port_provided_by_ambient_parent() {
    let input = r#"
    \Component ProgressBar(progress_percentage: Number) {
      \Rect(x: 0, y: 0, width: progress_percentage, height: 20, color: #000)
    }

    \Component TaskList {
      \Children {
        progress_percentage: 42
      }
    }

    \TaskList {
      \ProgressBar()
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation failed");

    let rect_node = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    assert_eq!(rect_node.rect.width, 42.0);
}

#[test]
fn test_required_port_ambient_overridden_by_instance() {
    let input = r#"
    \Component ProgressBar(progress_percentage: Number) {
      \Rect(x: 0, y: 0, width: progress_percentage, height: 20, color: #000)
    }

    \Component TaskList {
      \Children {
        progress_percentage: 42
      }
    }

    \TaskList {
      \ProgressBar(progress_percentage: 88)
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation failed");

    // Explicit instance port (88) overrides ambient parent (42)
    let rect_node = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    assert_eq!(rect_node.rect.width, 88.0);
}

#[test]
fn test_precedence_3_tier_hierarchy() {
    // Tier 1: Component signature default (width: 100)
    // Tier 2: Ambient parent (\Children { width: 200 })
    // Tier 3: Explicit instance override (width: 300)
    let input = r#"
    \Component Button(width: Number: 100) {
      \Rect(x: 0, y: 0, width: width, height: 40, color: #000)
    }

    \Component Container {
      \Children {
        width: 200
      }
    }

    // Instance 1: Uses Tier 1 (Component default = 100)
    \Button()

    \Container {
      // Instance 2: Uses Tier 2 (Ambient parent = 200 overrides default 100)
      \Button()
      // Instance 3: Uses Tier 3 (Explicit instance = 300 overrides ambient 200 and default 100)
      \Button(width: 300)
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation failed");

    let rects: Vec<_> = layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 3);
    assert_eq!(rects[0].rect.width, 100.0); // Tier 1 wins
    assert_eq!(rects[1].rect.width, 200.0); // Tier 2 wins
    assert_eq!(rects[2].rect.width, 300.0); // Tier 3 wins
}

#[test]
fn test_lexical_scope_expression_let() {
    let input = r#"
    \Component Card(padding: Number: 16) {
      let inset = padding * 2
      \Rect(x: 0, y: 0, width: 400 - inset, height: 50, color: #000)
    }

    \Card(padding: 20)
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation failed");

    let rect = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    // 400 - (20 * 2) = 360
    assert_eq!(rect.rect.width, 360.0);
}

#[test]
fn test_lexical_scope_node_let() {
    let input = r#"
    \Component DualBox {
      let primary = \Rect(x: 10, y: 15, width: 100, height: 40, color: #000)
      \Rect(x: primary.right + 20, y: primary.top, width: 80, height: 40, color: #000)
    }

    \DualBox()
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation failed");

    let rects: Vec<_> = layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 2);
    // Primary: x: 10, y: 15, width: 100
    assert_eq!(rects[0].rect.x, 10.0);
    assert_eq!(rects[0].rect.y, 15.0);
    assert_eq!(rects[0].rect.width, 100.0);

    // Secondary: x: primary.right (10 + 100) + 20 = 130, y: primary.top (15)
    assert_eq!(rects[1].rect.x, 130.0);
    assert_eq!(rects[1].rect.y, 15.0);
    assert_eq!(rects[1].rect.width, 80.0);
}

#[test]
fn test_lexical_scope_shadowing() {
    let input = r#"
    \Component ShadowBox(x: Number: 10) {
      let x = self.x + 40
      \Rect(x: x, y: parent.x, width: 100, height: 40, color: #000)
    }

    \ShadowBox()
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation failed");

    let rect = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    // `x: x` resolves to shadowed `let x = 10 + 40 = 50`
    assert_eq!(rect.rect.x, 50.0);
    // `y: parent.x` resolves to component port `ShadowBox.x = 10`
    assert_eq!(rect.rect.y, 10.0);
}

#[test]
fn test_lexical_scope_in_children_directive() {
    let input = r#"
    \Component FlowWithMargin {
      let margin = 35
      \Children {
        x: parent.left + margin,
        y: 0
      }
    }

    \FlowWithMargin {
      \Rect(width: 50, height: 50, color: #000)
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation failed");

    let rect = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    // parent.left (0) + margin (35) = 35
    assert_eq!(rect.rect.x, 35.0);
}

#[test]
fn test_top_level_let_binding() {
    let input = r#"
    let global_pad = 45;

    \Rect(x: global_pad, y: global_pad, width: 100, height: 100, color: #000)
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation failed");

    let rect = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    assert_eq!(rect.rect.x, 45.0);
    assert_eq!(rect.rect.y, 45.0);
}

#[test]
fn test_top_level_node_let_binding() {
    let input = r#"
    let sidebar = \Rect(x: 10, y: 10, width: 200, height: 500, color: #000)
    \Rect(x: sidebar.right + 20, y: sidebar.top, width: 600, height: 500, color: #000)
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation failed");

    let rects: Vec<_> = layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 2);
    assert_eq!(rects[0].rect.x, 10.0);
    assert_eq!(rects[0].rect.width, 200.0);

    // sidebar.right (10 + 200) + 20 = 230
    assert_eq!(rects[1].rect.x, 230.0);
    assert_eq!(rects[1].rect.y, 10.0);
}

#[test]
fn test_visibility_child_cannot_access_parent_private_local_via_parent_dot() {
    let input = r#"
    \Component CustomCard {
      let card_padding = 16
      \Children {}
    }

    \CustomCard {
      \Rect(x: parent.card_padding, y: 0, width: 100, height: 100, color: #000)
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let result = directedtype::evaluate_document(&doc);
    assert!(result.is_err(), "Expected error when child accesses parent's private local via parent.p");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("has no public port 'card_padding'"),
        "Unexpected error message: {}",
        err_msg
    );
}

#[test]
fn test_visibility_child_cannot_access_parent_private_local_via_bare_ident() {
    let input = r#"
    \Component CustomCard {
      let card_padding = 16
      \Children {}
    }

    \CustomCard {
      \Rect(x: card_padding, y: 0, width: 100, height: 100, color: #000)
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let result = directedtype::evaluate_document(&doc);
    assert!(result.is_err(), "Expected error when child accesses parent's private local as bare ident");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("card_padding"),
        "Unexpected error message: {}",
        err_msg
    );
}

#[test]
fn test_visibility_parent_explicit_push_via_children_directive() {
    let input = r#"
    \Component CustomCard {
      let card_padding = 16
      \Children {
        x: parent.left + card_padding,
        y: 0
      }
    }

    \CustomCard {
      \Rect(width: 100, height: 50, color: #000)
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation failed");

    let rect = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    // parent.left (0) + card_padding (16) = 16
    assert_eq!(rect.rect.x, 16.0);
}

#[test]
fn test_visibility_child_retains_caller_lexical_scope() {
    let input = r#"
    \Component CustomCard {
      let internal_padding = 10
      \Children {
        x: parent.left + internal_padding,
        y: 0
      }
    }

    let caller_width = 180;

    \CustomCard {
      \Rect(width: caller_width, height: 50, color: #000)
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation failed");

    let rect = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    // ambient x from parent: 10
    assert_eq!(rect.rect.x, 10.0);
    // caller width: 180
    assert_eq!(rect.rect.width, 180.0);
}

#[test]
fn test_visibility_parent_private_local_does_not_shadow_caller_local() {
    let input = r#"
    \Component CustomCard {
      let my_size = 999
      \Children {
        x: parent.left,
        y: 0
      }
    }

    let my_size = 42;

    \CustomCard {
      \Rect(width: my_size, height: 50, color: #000)
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation failed");

    let rect = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    // width should be 42 (from caller's scope), NOT 999 (from parent's private scope)
    assert_eq!(rect.rect.width, 42.0);
}

#[test]
fn test_visibility_child_explicit_parent_port_bypasses_internal_shadow() {
    let input = r#"
    \Component CustomCard(clip_offset: Number = 5) {
      let clip_offset = 100 // internal shadow
      \Children {
        x: parent.left,
        y: 0
      }
    }

    \CustomCard(clip_offset: 25) {
      // Child explicitly wires to parent's public port, bypassing internal shadow
      \Rect(width: parent.clip_offset, height: 50, color: #000)
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation failed");

    let rect = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    // rect.width = parent.clip_offset (which was passed as 25 to CustomCard), NOT 100
    assert_eq!(rect.rect.width, 25.0);
}

#[test]
fn test_component_let_order_independence_element_before_let() {
    let input = r#"
    \Component Card {
      // Element declared BEFORE the let definition it references
      \Rect(x: 0, y: 0, width: card_width, height: 40, color: #000)
      let card_width = 320
    }

    \Card()
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation failed");

    let rect = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    assert_eq!(rect.rect.width, 320.0);
}

#[test]
fn test_component_let_order_independence_chained_out_of_order() {
    let input = r#"
    \Component ChainedCard {
      // 'a' references 'b' declared below it
      let a = b + 15
      let b = 100
      \Rect(x: 0, y: 0, width: a, height: 50, color: #000)
    }

    \ChainedCard()
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation failed");

    let rect = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    // a = 100 + 15 = 115
    assert_eq!(rect.rect.width, 115.0);
}

#[test]
fn test_component_literals_preserve_declaration_order_with_interspersed_lets() {
    let input = r#"
    \Component LayeredCard {
      let bg_pad = 10
      \Rect(x: 0, y: 0, width: 400, height: 100, z: 0, color: #000) // First node: Background (index 1)
      let inner_pad = bg_pad * 2
      \Rect(x: 0, y: 0, width: 200, height: 100, z: 0, color: #000) // Second node: Foreground (index 2)
    }

    \LayeredCard()
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation failed");

    let render_order = layout.render_order();
    // Nodes in render order: LayeredCard (0), first Rect (1), second Rect (2)
    assert_eq!(render_order.len(), 3);
    assert_eq!(render_order[0].id.0, 0);
    assert_eq!(render_order[1].id.0, 1);
    assert_eq!(render_order[1].rect.width, 400.0);
    assert_eq!(render_order[2].id.0, 2);
    assert_eq!(render_order[2].rect.width, 200.0);
}

#[test]
fn test_forbidden_parent_port_on_element() {
    let input = r#"
    \Rect(parent: 10, width: 100, height: 100)
    "#;

    let doc = parse(input).expect("Failed to parse");
    let err = directedtype::evaluate_document(&doc).expect_err("Should reject reserved parent port");
    match err {
        directedtype::compiler::error::CompileError::ReservedPort { node, port, .. } => {
            assert_eq!(node, "Rect");
            assert_eq!(port, "parent");
        }
        other => panic!("Expected ReservedPort error, got {:?}", other),
    }
}

#[test]
fn test_forbidden_parent_port_on_component_param() {
    let input = r#"
    \Component Container(parent: Number, width: 100) {
        \Rect(width: self.width)
    }
    \Container()
    "#;

    let doc = parse(input).expect("Failed to parse");
    let err = directedtype::evaluate_document(&doc).expect_err("Should reject reserved parent param");
    match err {
        directedtype::compiler::error::CompileError::ReservedPort { node, port, .. } => {
            assert_eq!(node, "Container");
            assert_eq!(port, "parent");
        }
        other => panic!("Expected ReservedPort error, got {:?}", other),
    }
}

#[test]
fn test_forbidden_parent_port_on_children_directive() {
    let input = r#"
    \Component Container {
        \Children {
            parent: 50
        }
    }
    \Container {
        \Rect()
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let err = directedtype::evaluate_document(&doc).expect_err("Should reject reserved parent port on Children");
    match err {
        directedtype::compiler::error::CompileError::ReservedPort { node, port, .. } => {
            assert_eq!(node, "Children");
            assert_eq!(port, "parent");
        }
        other => panic!("Expected ReservedPort error, got {:?}", other),
    }
}

#[test]
fn test_clip_missing_box_port_error() {
    let input = r#"
    let clip = \Clip()
    \Rect(clip: clip)
    "#;

    let doc = parse(input).expect("Failed to parse");
    let err = directedtype::evaluate_document(&doc).expect_err("Should fail when Clip is missing box");
    match err {
        directedtype::compiler::error::CompileError::MissingPort { node, port, .. } => {
            assert_eq!(node, "Clip");
            assert_eq!(port, "box");
        }
        other => panic!("Expected MissingPort error for box, got {:?}", other),
    }
}

#[test]
fn test_box_and_clip_non_drawing_primitives() {
    let input = r#"
    let viewport = \Box(x: 10, y: 10, width: 200, height: 100)
    let clip = \Clip(box: viewport)
    \Rect(clip: clip, x: 0, y: 0, width: 50, height: 50, color: #000)
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation should succeed");

    let box_node = layout.nodes.iter().find(|n| n.name == "Box").unwrap();
    let clip_node = layout.nodes.iter().find(|n| n.name == "Clip").unwrap();
    let rect_node = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();

    assert!(!box_node.is_paint_primitive(), "Box should not be a paint primitive");
    assert!(!clip_node.is_paint_primitive(), "Clip should not be a paint primitive");
    assert!(rect_node.is_paint_primitive(), "Rect should be a paint primitive");
}

#[test]
fn test_clip_with_box_inline_expansion() {
    let input = r##"
    \Component ScrollView {
        let clip = \Clip(up: self.clip, box: \Box(x: self.left + 5, y: self.top + 5, width: self.width - 10, height: self.height - 10))
        \Rect(clip: clip, x: 0, y: 0, width: 300, height: 300, color: #ff0000)
    }

    \ScrollView(width: 200, height: 150)
    "##;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation should succeed");

    let rect = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    assert!(rect.clip.is_some(), "Rect should have an assigned clip node");

    let clip_id = rect.clip.unwrap();
    let clip_node = layout.get_node(clip_id).unwrap();
    assert_eq!(clip_node.name, "Clip");

    // Check that Clip's box node was expanded and evaluated with correct dimensions
    let box_node_id = layout.get_value(clip_id, "box").and_then(|v| v.as_node()).unwrap();
    let box_node = layout.get_node(box_node_id).unwrap();
    assert_eq!(box_node.name, "Box");
    assert_eq!(box_node.rect.x, 5.0);
    assert_eq!(box_node.rect.y, 5.0);
    assert_eq!(box_node.rect.width, 190.0); // 200 - 10
    assert_eq!(box_node.rect.height, 140.0); // 150 - 10
}

#[test]
fn test_clip_unclip_with_window_clip() {
    let input = r##"
    \Component Modal {
        let clip = \Clip(box: \Box(width: 100, height: 100))
        \Rect(clip: clip, x: 0, y: 0, width: 100, height: 100, color: #aaaaaa)
        \Rect(clip: window.clip, x: 100, y: 0, width: 100, height: 100, color: #ffffff)
    }

    \Modal()
    "##;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation should succeed");

    let clipped_rect = layout.nodes.iter().find(|n| {
        n.name == "Rect" && n.properties.get("color").and_then(|v| v.as_str()) == Some("#aaaaaa")
    }).unwrap();
    let unclipped_rect = layout.nodes.iter().find(|n| {
        n.name == "Rect" && n.properties.get("color").and_then(|v| v.as_str()) == Some("#ffffff")
    }).unwrap();

    assert!(clipped_rect.clip.is_some(), "Clipped rect should have a clip NodeId");
    assert!(unclipped_rect.clip.is_none(), "Unclipped rect with window.clip should have None clip");
}

#[test]
fn test_clip_stepping_up_chain() {
    let input = r##"
    \Component MultiLevel {
        let outer_clip = \Clip(box: \Box(width: 300, height: 300))
        let inner_clip = \Clip(up: outer_clip, box: \Box(width: 150, height: 150))
        \Rect(clip: inner_clip, x: 0, y: 0, width: 50, height: 50, color: #111111)
        \Rect(clip: inner_clip.up, x: 50, y: 0, width: 50, height: 50, color: #222222) // stepped up to outer_clip!
    }

    \MultiLevel()
    "##;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation should succeed");

    let inner_rect = layout.nodes.iter().find(|n| {
        n.name == "Rect" && n.properties.get("color").and_then(|v| v.as_str()) == Some("#111111")
    }).unwrap();
    let outer_rect = layout.nodes.iter().find(|n| {
        n.name == "Rect" && n.properties.get("color").and_then(|v| v.as_str()) == Some("#222222")
    }).unwrap();

    let inner_clip_id = inner_rect.clip.unwrap();
    let outer_clip_id = outer_rect.clip.unwrap();

    assert_ne!(inner_clip_id, outer_clip_id);
    let inner_up = layout.get_value(inner_clip_id, "up").and_then(|v| v.as_node()).unwrap();
    assert_eq!(inner_up, outer_clip_id);
}

#[test]
fn test_children_directive_ambient_clip() {
    let input = r##"
    \Component ScrollContainer {
        let clip = \Clip(box: \Box(width: 150, height: 150))
        \Children {
            clip: clip,
            x: 0,
            y: 0,
            width: 100,
            height: 100
        }
    }

    \ScrollContainer {
        \Rect(color: #333333) // ambient clip
        \Rect(clip: window.clip, color: #444444) // consumer overrides to window.clip
    }
    "##;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation should succeed");

    let ambient_rect = layout.nodes.iter().find(|n| {
        n.name == "Rect" && n.properties.get("color").and_then(|v| v.as_str()) == Some("#333333")
    }).unwrap();
    let override_rect = layout.nodes.iter().find(|n| {
        n.name == "Rect" && n.properties.get("color").and_then(|v| v.as_str()) == Some("#444444")
    }).unwrap();

    assert!(ambient_rect.clip.is_some(), "Ambient rect should inherit clip from \\Children");
    assert!(override_rect.clip.is_none(), "Overridden rect should be unclipped (window.clip)");
}

#[test]
fn test_dom_tree_formatting() {
    let input = r#"
    \Component Card(bg: Color: #1e293b, width: 300, height: 150) {
        \Rect(x: parent.left, y: parent.top, width: parent.width, height: parent.height, color: parent.bg)
        \Children {
            x: parent.left + 16,
            y: prev ? prev.bottom + 8 : parent.top + 16
        }
    }

    \Card {
        \Text(size: 18) { Hello DOM }
        \Rect(width: 100, height: 20, color: #3b82f6)
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation should succeed");

    let dom_str = layout.format_dom();

    assert!(dom_str.contains("\\Card("));
    assert!(dom_str.contains("{\n"));
    assert!(dom_str.contains("\\Rect("));
    assert!(dom_str.contains("\\Text("));
    assert!(dom_str.contains("{ Hello DOM }"));
    assert!(dom_str.contains("color: #1e293b"));
}

#[test]
fn test_parse_env_syntax_and_tombstones() {
    let input = r#"
    env global_theme: String = "dark";
    let uninit_local;
    env uninit_env;

    \Component Box(width: Number, env theme: String, color: Color: #ffffff) {
        let private_gap;
        let shadow_val = 10;
        env local_env = #ff0000;
        env hole_env;

        \Rect(color: self.color)
        \Children
    }
    "#;
    let doc = parse(input).expect("Failed to parse env syntax and tombstones");
    assert_eq!(doc.items.len(), 4);
    match &doc.items[0] {
        Item::Env(e) => {
            assert_eq!(e.name.as_str(), "global_theme");
            assert!(e.value.is_some());
        }
        _ => panic!("Expected Item::Env"),
    }
    match &doc.items[1] {
        Item::Let(l) => {
            assert_eq!(l.name.as_str(), "uninit_local");
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
            assert_eq!(c.name.as_str(), "Box");
            assert_eq!(c.params[0].name.as_str(), "width");
            assert!(!c.params[0].is_env);
            assert_eq!(c.params[1].name.as_str(), "theme");
            assert!(c.params[1].is_env);
            assert_eq!(c.params[2].name.as_str(), "color");
            assert!(!c.params[2].is_env);

            match &c.body[0] {
                ComponentBodyItem::Let(l) => {
                    assert_eq!(l.name.as_str(), "private_gap");
                    assert!(l.value.is_none());
                }
                _ => panic!("Expected uninit let"),
            }
            match &c.body[2] {
                ComponentBodyItem::Env(e) => {
                    assert_eq!(e.name.as_str(), "local_env");
                    assert!(e.value.is_some());
                }
                _ => panic!("Expected initialized env"),
            }
            match &c.body[3] {
                ComponentBodyItem::Env(e) => {
                    assert_eq!(e.name.as_str(), "hole_env");
                    assert!(e.value.is_none());
                }
                _ => panic!("Expected uninit env hole"),
            }
        }
        _ => panic!("Expected Item::Component"),
    }
}

#[test]
fn test_env_auto_propagation_bypassing_middleman() {
    let input = r#"
    \Component Theme(color: Color) {
        env color = self.color
        \Children
    }

    \Component Row() {
        \Children
    }

    \Component Button(env color: Color: #000000) {
        \Rect(x: 0, y: 0, width: 100, height: 100, color: self.color)
    }

    \Theme(color: #123456) {
        \Row {
            \Button()
        }
    }
    "#;
    let doc = parse(input).expect("Parse error");
    let (expanded, _) = compile_to_graph(&doc).expect("Compilation error");
    let btn_node = expanded.nodes.iter().find(|n| n.name == "Button").expect("Button not found");
    let btn_color = btn_node.ports.get("color").expect("Button missing color port");
    match btn_color {
        Expr::MemberAccess(m) => {
            assert_eq!(m.member.as_str(), "color");
            match m.target.as_ref() {
                Expr::Ident(id) => assert_eq!(id.as_str(), "__node_0"),
                _ => panic!("Expected target ident"),
            }
        }
        _ => panic!("Expected member access for Button.color, got {:?}", btn_color),
    }
}

#[test]
fn test_env_component_encapsulation_sealed_black_box() {
    let input = r#"
    \Component Theme(color: Color) {
        env color = self.color
        \Children
    }

    \Component CustomCard() {
        \Rect(x: 0, y: 0, width: 100, height: 100, color: #999999)
    }

    \Theme(color: #123456) {
        \CustomCard()
    }
    "#;
    let doc = parse(input).expect("Parse error");
    let (expanded, _) = compile_to_graph(&doc).expect("Compilation error");
    let rect_node = expanded.nodes.iter().find(|n| n.name == "Rect").expect("Rect not found");
    let rect_color = rect_node.ports.get("color").expect("Rect missing color port");
    match rect_color {
        Expr::Literal(Literal::Color(c, _)) => assert_eq!(c, "#999999"),
        _ => panic!("Encapsulation violated: internal Rect received {:?}", rect_color),
    }
}

#[test]
fn test_env_4_tier_precedence_order() {
    let input = r#"
    \Component Theme(color: Color) {
        env color = self.color
        \Children
    }

    \Component Container() {
        \Children {
            color: #333333
        }
    }

    \Component Button(env color: Color: #111111) {
        \Rect(x: 0, y: 0, width: 100, height: 100, color: self.color)
    }

    \Theme(color: #222222) {
        // Case A: Tier 4 beats 3, 2, 1
        \Container {
            \Button(color: #444444)
        }
        // Case B: Tier 3 beats 2, 1
        \Container {
            \Button()
        }
        // Case C: Tier 2 beats 1
        \Button()
    }

    // Case D: Tier 1 alone
    \Button()
    "#;
    let doc = parse(input).expect("Parse error");
    let (expanded, _) = compile_to_graph(&doc).expect("Compilation error");
    let buttons: Vec<_> = expanded.nodes.iter().filter(|n| n.name == "Button").collect();
    assert_eq!(buttons.len(), 4);

    // Case A: Button 0 has explicit color #444444
    assert!(matches!(buttons[0].ports.get("color").unwrap(), Expr::Literal(Literal::Color(c, _)) if c == "#444444"));

    // Case B: Button 1 has Tier 3 container color #333333
    assert!(matches!(buttons[1].ports.get("color").unwrap(), Expr::Literal(Literal::Color(c, _)) if c == "#333333"));

    // Case C: Button 2 inherits Tier 2 Theme color
    match buttons[2].ports.get("color").unwrap() {
        Expr::MemberAccess(m) => {
            assert_eq!(m.member.as_str(), "color");
            match m.target.as_ref() {
                Expr::Ident(id) => assert_eq!(id.as_str(), "__node_0"),
                _ => panic!("Expected target ident"),
            }
        }
        _ => panic!("Expected Theme.color member access"),
    }

    // Case D: Button 3 has Tier 1 default #111111
    assert!(matches!(buttons[3].ports.get("color").unwrap(), Expr::Literal(Literal::Color(c, _)) if c == "#111111"));
}

#[test]
fn test_env_interceptor() {
    let input = r#"
    \Component Surface(env color: Color) {
        env color = #abcdef
        \Children
    }

    \Component Button(env color: Color: #000000) {
        \Rect(x: 0, y: 0, width: 100, height: 100, color: self.color)
    }

    \Surface(color: #123456) {
        \Button()
    }
    "#;
    let doc = parse(input).expect("Parse error");
    let (expanded, _) = compile_to_graph(&doc).expect("Compilation error");
    let btn_node = expanded.nodes.iter().find(|n| n.name == "Button").expect("Button not found");
    let btn_color = btn_node.ports.get("color").unwrap();
    assert!(matches!(btn_color, Expr::Literal(Literal::Color(c, _)) if c == "#abcdef"));
}

#[test]
fn test_env_translator_provider() {
    let input = r#"
    \Component ThemeProvider(theme: String) {
        env color = (self.theme == "dark") ? #000000 : #ffffff
        \Children
    }

    \Component Button(env color: Color: #888888) {
        \Rect(x: 0, y: 0, width: 100, height: 100, color: self.color)
    }

    \ThemeProvider(theme: "dark") {
        \Button()
    }
    "#;
    let doc = parse(input).expect("Parse error");
    let (expanded, _) = compile_to_graph(&doc).expect("Compilation error");
    let btn_node = expanded.nodes.iter().find(|n| n.name == "Button").expect("Button not found");
    let btn_color = btn_node.ports.get("color").unwrap();
    assert!(matches!(btn_color, Expr::Ternary(_)));
}

#[test]
fn test_env_shield_swallower() {
    let input = r#"
    \Component AlertBadge(env color: Color) {
        \Rect(x: 0, y: 0, width: 100, height: 100, color: self.color)
        let color = #000000;
        \Children
    }

    \Component Button(env color: Color: #888888) {
        \Rect(x: 0, y: 0, width: 100, height: 100, color: self.color)
    }

    \AlertBadge(color: #ff0000) {
        \Button()
    }
    "#;
    let doc = parse(input).expect("Parse error");
    let (expanded, _) = compile_to_graph(&doc).expect("Compilation error");
    let btn_node = expanded.nodes.iter().find(|n| n.name == "Button").expect("Button not found");
    let btn_color = btn_node.ports.get("color").unwrap();
    assert!(matches!(btn_color, Expr::Literal(Literal::Color(c, _)) if c == "#888888"));
}

#[test]
fn test_let_tombstone_firewall() {
    let input = r#"
    \Component Shield(env color: Color) {
        let color;
        \Children
    }

    \Component Button(env color: Color: #888888) {
        \Rect(x: 0, y: 0, width: 100, height: 100, color: self.color)
    }

    \Shield(color: #ff0000) {
        \Button()
    }
    "#;
    let doc = parse(input).expect("Parse error");
    let (expanded, _) = compile_to_graph(&doc).expect("Compilation error");
    let btn_node = expanded.nodes.iter().find(|n| n.name == "Button").expect("Button not found");
    let btn_color = btn_node.ports.get("color").unwrap();
    assert!(matches!(btn_color, Expr::Literal(Literal::Color(c, _)) if c == "#888888"));
}

#[test]
fn test_env_tombstone_hole() {
    let input = r#"
    \Component Theme(color: Color) {
        env color = self.color
        \Children
    }

    \Component Hole() {
        env color;
        \Children
    }

    \Component Button(env color: Color: #888888) {
        \Rect(x: 0, y: 0, width: 100, height: 100, color: self.color)
    }

    \Theme(color: #123456) {
        \Hole {
            \Button()
        }
    }
    "#;
    let doc = parse(input).expect("Parse error");
    let (expanded, _) = compile_to_graph(&doc).expect("Compilation error");
    let btn_node = expanded.nodes.iter().find(|n| n.name == "Button").expect("Button not found");
    let btn_color = btn_node.ports.get("color").unwrap();
    assert!(matches!(btn_color, Expr::Literal(Literal::Color(c, _)) if c == "#888888"));
}

#[test]
fn test_uninitialized_variable_use_error() {
    let input = r#"
    \Component Bad() {
        let color;
        \Rect(x: 0, y: 0, width: 100, height: 100, color: color)
    }

    \Bad()
    "#;
    let doc = parse(input).expect("Parse error");
    let res = compile_to_graph(&doc);
    assert!(res.is_err());
    match res.unwrap_err() {
        directedtype::compiler::error::CompileError::UninitializedVariableUse { name, .. } => {
            assert_eq!(name, "color");
        }
        other => panic!("Expected UninitializedVariableUse, got {:?}", other),
    }
}

#[test]
fn test_env_clip_universal_base_trait() {
    let input = r#"
    \Component ScrollView() {
        env clip = \Clip(box: \Box(width: 200, height: 200))
        \Children
    }

    \ScrollView {
        \Rect(x: 0, y: 0, width: 50, height: 50, color: #ff0000)
    }
    "#;
    let doc = parse(input).expect("Parse error");
    let (expanded, _) = compile_to_graph(&doc).expect("Compilation error");
    let rect_node = expanded.nodes.iter().find(|n| n.name == "Rect").expect("Rect not found");
    let rect_clip = rect_node.ports.get("clip").expect("Rect missing clip");
    match rect_clip {
        Expr::Ident(id) => assert!(id.as_str().starts_with("__node_")),
        _ => panic!("Expected clip ident, got {:?}", rect_clip),
    }
}

#[test]
fn test_env_does_not_bind_to_non_env_parameter_missing_port_error() {
    let input = r#"
    \Component Foo(env color) {
        \Children
    }

    \Component Bar(color) {
        \Children {
            color: color
        }
    }

    \Foo(color: #F00) {
        \Bar() {
            \Rect(x: 100, y: 100, width: 100, height: 100)
        }
    }
    "#;
    let doc = parse(input).expect("Parse error");
    let err = compile_to_graph(&doc).expect_err("Should fail with missing port for Bar.color");
    match err {
        directedtype::compiler::error::CompileError::MissingPort { node, port, .. } => {
            assert_eq!(node, "Bar");
            assert_eq!(port, "color");
        }
        other => panic!("Expected MissingPort, got {:?}", other),
    }
}

#[test]
fn test_rect_missing_color_port_error() {
    let input = r#"
    \Rect(x: 10, y: 10, width: 100, height: 100)
    "#;
    let doc = parse(input).expect("Parse error");
    let err = directedtype::evaluate_document(&doc).expect_err("Should fail without color port on Rect");
    match err {
        directedtype::compiler::error::CompileError::MissingPort { node, port, .. } => {
            assert_eq!(node, "Rect");
            assert_eq!(port, "color");
        }
        other => panic!("Expected MissingPort for Rect color, got {:?}", other),
    }
}

#[test]
fn test_rect_missing_x_port_error() {
    let input = r#"
    \Rect(y: 10, width: 100, height: 100, color: #000)
    "#;
    let doc = parse(input).expect("Parse error");
    let err = directedtype::evaluate_document(&doc).expect_err("Should fail without x port on Rect");
    match err {
        directedtype::compiler::error::CompileError::MissingPort { node, port, .. } => {
            assert_eq!(node, "Rect");
            assert_eq!(port, "x");
        }
        other => panic!("Expected MissingPort for Rect x, got {:?}", other),
    }
}

#[test]
fn test_rect_missing_y_port_error() {
    let input = r#"
    \Rect(x: 10, width: 100, height: 100, color: #000)
    "#;
    let doc = parse(input).expect("Parse error");
    let err = directedtype::evaluate_document(&doc).expect_err("Should fail without y port on Rect");
    match err {
        directedtype::compiler::error::CompileError::MissingPort { node, port, .. } => {
            assert_eq!(node, "Rect");
            assert_eq!(port, "y");
        }
        other => panic!("Expected MissingPort for Rect y, got {:?}", other),
    }
}

#[test]
fn test_rect_missing_width_port_error() {
    let input = r#"
    \Rect(x: 10, y: 10, height: 100, color: #000)
    "#;
    let doc = parse(input).expect("Parse error");
    let err = directedtype::evaluate_document(&doc).expect_err("Should fail without width port on Rect");
    match err {
        directedtype::compiler::error::CompileError::MissingPort { node, port, .. } => {
            assert_eq!(node, "Rect");
            assert_eq!(port, "width");
        }
        other => panic!("Expected MissingPort for Rect width, got {:?}", other),
    }
}

#[test]
fn test_rect_missing_height_port_error() {
    let input = r#"
    \Rect(x: 10, y: 10, width: 100, color: #000)
    "#;
    let doc = parse(input).expect("Parse error");
    let err = directedtype::evaluate_document(&doc).expect_err("Should fail without height port on Rect");
    match err {
        directedtype::compiler::error::CompileError::MissingPort { node, port, .. } => {
            assert_eq!(node, "Rect");
            assert_eq!(port, "height");
        }
        other => panic!("Expected MissingPort for Rect height, got {:?}", other),
    }
}

#[test]
fn test_env_manual_wiring_to_component_param() {
    let input = r#"
    \Component Theme(env color: Color) {
        \Children
    }

    \Component Card(bg: Color) {
        \Rect(x: 0, y: 0, width: 100, height: 100, color: bg)
    }

    \Theme(color: #ff0000) {
        \Card(bg: env.color)
    }
    "#;
    let doc = parse(input).expect("Parse error");
    let (expanded, _) = compile_to_graph(&doc).expect("Compilation error");
    let card = expanded.nodes.iter().find(|n| n.name == "Card").expect("Card not found");
    match card.ports.get("bg").unwrap() {
        Expr::MemberAccess(m) => {
            assert_eq!(m.member.as_str(), "color");
        }
        _ => panic!("Expected member access to Theme.color"),
    }
}

#[test]
fn test_env_manual_wiring_to_primitive_rect() {
    let input = r#"
    \Component Theme(env color: Color) {
        \Children
    }

    \Theme(color: #00ff00) {
        \Rect(x: 10, y: 20, width: 200, height: 100, color: env.color)
    }
    "#;
    let doc = parse(input).expect("Parse error");
    let (expanded, _) = compile_to_graph(&doc).expect("Compilation error");
    let rect = expanded.nodes.iter().find(|n| n.name == "Rect").expect("Rect not found");
    match rect.ports.get("color").unwrap() {
        Expr::MemberAccess(m) => {
            assert_eq!(m.member.as_str(), "color");
        }
        _ => panic!("Expected member access to Theme.color"),
    }
}

#[test]
fn test_env_manual_wiring_in_children_directive() {
    let input = r#"
    \Component Theme(env accent: Color) {
        \Children
    }

    \Component Container {
        \Children {
            border_color: env.accent
        }
    }

    \Component Box(border_color: Color) {
        \Rect(x: 0, y: 0, width: 50, height: 50, color: border_color)
    }

    \Theme(accent: #123456) {
        \Container {
            \Box()
        }
    }
    "#;
    let doc = parse(input).expect("Parse error");
    let (expanded, _) = compile_to_graph(&doc).expect("Compilation error");
    let box_node = expanded.nodes.iter().find(|n| n.name == "Box").expect("Box not found");
    match box_node.ports.get("border_color").unwrap() {
        Expr::MemberAccess(m) => {
            assert_eq!(m.member.as_str(), "accent");
        }
        _ => panic!("Expected member access to Theme.accent"),
    }
}

#[test]
fn test_env_blocked_by_shield_firewall_error() {
    let input = r#"
    \Component Theme(env color: Color) {
        \Children
    }

    \Component Shield(env color: Color) {
        let color;
        \Children
    }

    \Component Card(bg: Color) {
        \Rect(x: 0, y: 0, width: 100, height: 100, color: bg)
    }

    \Theme(color: #ff0000) {
        \Shield {
            \Card(bg: env.color)
        }
    }
    "#;
    let doc = parse(input).expect("Parse error");
    let err = directedtype::evaluate_document(&doc).expect_err("Should fail due to firewalled env variable");
    match err {
        directedtype::compiler::error::CompileError::BlockedEnvVariable { name, .. } => {
            assert_eq!(name, "color");
        }
        other => panic!("Expected BlockedEnvVariable, got {:?}", other),
    }
}

#[test]
fn test_env_undefined_variable_error() {
    let input = r#"
    \Component Card(bg: Color) {
        \Rect(x: 0, y: 0, width: 100, height: 100, color: bg)
    }

    \Card(bg: env.missing_var)
    "#;
    let doc = parse(input).expect("Parse error");
    let err = directedtype::evaluate_document(&doc).expect_err("Should fail due to undefined env variable");
    match err {
        directedtype::compiler::error::CompileError::UndefinedEnvVariable { name, .. } => {
            assert_eq!(name, "missing_var");
        }
        other => panic!("Expected UndefinedEnvVariable, got {:?}", other),
    }
}

#[test]
fn test_env_component_internal_body_hermeticity_error() {
    let input = r#"
    \Component Theme(env color: Color) {
        \Children
    }

    \Component SneakyCard {
        \Rect(x: 0, y: 0, width: 100, height: 100, color: env.color)
    }

    \Theme(color: #ff0000) {
        \SneakyCard()
    }
    "#;
    let doc = parse(input).expect("Parse error");
    let err = directedtype::evaluate_document(&doc).expect_err("Should fail: component body cannot access undeclared env");
    match err {
        directedtype::compiler::error::CompileError::UndefinedEnvVariable { name, .. } => {
            assert_eq!(name, "color");
        }
        other => panic!("Expected UndefinedEnvVariable, got {:?}", other),
    }
}

#[test]
fn test_bare_env_expression_error() {
    let input = r#"
    let x = env;
    "#;
    let doc = parse(input).expect("Parse error");
    let err = directedtype::evaluate_document(&doc).expect_err("Should fail on bare env");
    match err {
        directedtype::compiler::error::CompileError::BareEnvUse { .. } => {}
        other => panic!("Expected BareEnvUse, got {:?}", other),
    }
}

#[test]
fn test_font_primitive_metrics_and_sharing() {
    let input = r#"
    let f = \Font(size: 20, weight: 700);
    let t1 = \Text(font: f) { First };
    let t2 = \Text(font: f) { Second };
    let r1 = \Rect(x: 0, y: 0, width: 100, height: t1.font.cap_height + 10, color: #000000);
    let r2 = \Rect(x: 0, y: 50, width: 100, height: t2.cap_height + 10, color: #000000);
    "#;
    let doc = parse(input).expect("Failed to parse");
    let (expanded, _graph) = compile_to_graph(&doc).expect("Failed to compile to graph");

    // Verify DAG minimization: t1 and t2 do NOT have metric ports in expanded node.ports!
    let t1_node = expanded.get_node(expanded.roots[1]).unwrap();
    assert_eq!(t1_node.name, "Text");
    assert!(!t1_node.ports.contains_key("cap_height"), "Text node should NOT duplicate cap_height port in DAG");
    assert!(!t1_node.ports.contains_key("x_height"), "Text node should NOT duplicate x_height port in DAG");
    assert!(!t1_node.ports.contains_key("descent"), "Text node should NOT duplicate descent port in DAG");

    // The shared \Font node holds the metric ports
    let f_node = expanded.get_node(expanded.roots[0]).unwrap();
    assert_eq!(f_node.name, "Font");
    assert!(f_node.ports.contains_key("cap_height"));
    assert!(f_node.ports.contains_key("x_height"));
    assert!(f_node.ports.contains_key("descent"));
    assert!(f_node.ports.contains_key("ascent"));
    assert!(f_node.ports.contains_key("line_height"));

    // Evaluate layout
    let layout = directedtype::evaluate_document(&doc).expect("Failed to evaluate layout");
    let expected_cap = 20.0 * 0.71;
    let expected_rect_h = expected_cap + 10.0;

    let r1_h = layout.get_value(expanded.roots[3], "height").and_then(|v| v.as_f64()).unwrap();
    let r2_h = layout.get_value(expanded.roots[4], "height").and_then(|v| v.as_f64()).unwrap();

    assert!((r1_h - expected_rect_h).abs() < 1e-4, "r1.height should match t1.font.cap_height + 10");
    assert!((r2_h - expected_rect_h).abs() < 1e-4, "r2.height should match t2.cap_height + 10 (forwarded to font node)");
}

#[test]
fn test_text_ambient_font_inheritance() {
    let input = r#"
    let my_font = \Font(size: 24, weight: 600);
    env font = my_font;
    let label = \Text { Ambient text };
    let box = \Rect(x: 0, y: 0, width: 200, height: label.font.cap_height + 16, color: #111111);
    "#;
    let doc = parse(input).expect("Failed to parse");
    let (expanded, _graph) = compile_to_graph(&doc).expect("Failed to compile to graph");

    let font_id = expanded.roots[0];
    let label_id = expanded.roots[1];
    let box_id = expanded.roots[2];

    let label_node = expanded.get_node(label_id).unwrap();
    assert_eq!(label_node.font, Some(font_id), "Label should inherit ambient font node");

    let layout = directedtype::evaluate_document(&doc).expect("Failed to evaluate layout");
    let expected_cap = 24.0 * 0.71;
    let expected_box_h = expected_cap + 16.0;

    let box_h = layout.get_value(box_id, "height").and_then(|v| v.as_f64()).unwrap();
    assert!((box_h - expected_box_h).abs() < 1e-4, "box.height should match label.font.cap_height + 16");

    let label_size = layout.get_value(label_id, "size").and_then(|v| v.as_f64()).unwrap();
    assert_eq!(label_size, 24.0, "Label should inherit font size 24 from ambient font node");
}

#[test]
fn test_font_backward_compatible_string_family() {
    let input = r#"
    let t = \Text(font: "Menlo", size: 18) { Code };
    "#;
    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Failed to evaluate layout");
    let (expanded, _) = compile_to_graph(&doc).expect("Graph compilation");

    let t_id = expanded.roots[0];
    let t_node = expanded.get_node(t_id).unwrap();
    assert_eq!(t_node.font, None, "String font should not create a font node reference");

    let font_val = layout.get_value(t_id, "font").and_then(|v| v.as_str()).unwrap();
    assert_eq!(font_val, "Menlo");
}

#[test]
fn test_font_metrics_algebraic_calculations() {
    let input = r#"
    let f = \Font(size: 20);
    let t = \Text(font: f) { Metrics Test };
    let r = \Rect(x: 0, y: 0, width: f.line_height, height: t.font.ascent + t.font.descent, color: #ffffff);
    "#;
    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation");
    let (expanded, _) = compile_to_graph(&doc).expect("Graph compilation");

    let r_id = expanded.roots[2];
    let r_w = layout.get_value(r_id, "width").and_then(|v| v.as_f64()).unwrap();
    let r_h = layout.get_value(r_id, "height").and_then(|v| v.as_f64()).unwrap();

    let f_node = expanded.roots[0];
    let f_lh = layout.get_value(f_node, "line_height").and_then(|v| v.as_f64()).unwrap();
    let f_ascent = layout.get_value(f_node, "ascent").and_then(|v| v.as_f64()).unwrap();
    let f_descent = layout.get_value(f_node, "descent").and_then(|v| v.as_f64()).unwrap();

    assert!((r_w - f_lh).abs() < 1e-4);
    assert!((r_h - (f_ascent + f_descent)).abs() < 1e-4);
}

#[test]
fn test_component_custom_alias_declaration_and_evaluation() {
    let input = r#"
    \Component Card(x: 10, y: 20, width: 100, height: 50) {
        alias right = x + width;
        alias bottom = y + height;
        \Rect(x: x, y: y, width: width, height: height, color: #ff0000)
    }

    \Card(x: 30, width: 200)
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation");
    let (expanded, graph) = compile_to_graph(&doc).expect("Graph compilation");

    let card_id = expanded.roots[0];
    let card_node = expanded.get_node(card_id).unwrap();

    // Check authored expressions
    assert_eq!(
        card_node.authored_ports.get("right").map(|e| e.to_string()),
        Some("x + width".to_string())
    );
    assert_eq!(
        card_node.authored_ports.get("bottom").map(|e| e.to_string()),
        Some("y + height".to_string())
    );

    // Check evaluated layout values
    let right_val = layout.get_value(card_id, "right").and_then(|v| v.as_f64()).unwrap();
    let bottom_val = layout.get_value(card_id, "bottom").and_then(|v| v.as_f64()).unwrap();
    assert_eq!(right_val, 230.0); // 30 + 200
    assert_eq!(bottom_val, 70.0);  // 20 + 50

    // Check resolved node in layout
    let resolved_card = layout.get_node(card_id).unwrap();
    assert_eq!(resolved_card.formulas.get("right").map(String::as_str), Some("x + width"));
    assert_eq!(resolved_card.formulas.get("bottom").map(String::as_str), Some("y + height"));
    assert_eq!(resolved_card.properties.get("right"), Some(&directedtype::compiler::Value::Number(230.0)));
    assert_eq!(resolved_card.properties.get("bottom"), Some(&directedtype::compiler::Value::Number(70.0)));

    // Check graph dependencies
    let var_right = VarId::new(card_id, "right");
    let var_x = VarId::new(card_id, "x");
    let var_width = VarId::new(card_id, "width");
    assert_eq!(graph.upstream.get(&var_right).unwrap(), &vec![var_width, var_x]);
}

#[test]
fn test_component_immutable_alias_error() {
    let input = r#"
    \Component Card(width: 100, height: 50) {
        alias right = x + width;
        \Rect(x: x, y: y, width: width, height: height, color: #ff0000)
    }

    \Card(right: 300)
    "#;

    let doc = parse(input).expect("Failed to parse");
    let err = compile_to_graph(&doc).expect_err("Should fail when passing alias port");
    match err {
        directedtype::compiler::CompileError::ImmutableAliasPort { node, port, .. } => {
            assert_eq!(node, "Card");
            assert_eq!(port, "right");
        }
        _ => panic!("Expected ImmutableAliasPort error, got {:?}", err),
    }
}

#[test]
fn test_primitive_immutable_alias_error() {
    let input = r#"
    \Rect(x: 10, y: 10, width: 100, height: 50, right: 110, color: #ff0000)
    "#;

    let doc = parse(input).expect("Failed to parse");
    let err = compile_to_graph(&doc).expect_err("Should fail when passing spatial alias to primitive");
    match err {
        directedtype::compiler::CompileError::ImmutableAliasPort { node, port, .. } => {
            assert_eq!(node, "Rect");
            assert_eq!(port, "right");
        }
        _ => panic!("Expected ImmutableAliasPort error, got {:?}", err),
    }
}

#[test]
fn test_alias_interdependency_and_downstream_dependency() {
    let input = r#"
    \Component Card(x: 10, width: 100) {
        alias right = x + width;
        alias center_x = (x + right) / 2;
        \Rect(x: x, y: 0, width: width, height: 50, color: #00ff00)
    }

    let c = \Card(x: 20, width: 80);
    \Rect(x: c.center_x, y: 100, width: 50, height: 50, color: #0000ff)
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation");

    let card_node = layout.nodes.iter().find(|n| n.name == "Card").expect("Card node");
    let target_rect = layout.nodes.iter().find(|n| n.rect.y == 100.0).expect("Rect node");

    let right = card_node.properties.get("right").and_then(|v| v.as_f64()).unwrap();
    let center_x = card_node.properties.get("center_x").and_then(|v| v.as_f64()).unwrap();

    assert_eq!(right, 100.0); // 20 + 80
    assert_eq!(center_x, 60.0); // (20 + 100) / 2
    assert_eq!(target_rect.rect.x, 60.0);
}

#[test]
fn test_duplicate_alias_error() {
    // Alias shadows parameter
    let input1 = r#"
    \Component Card(width: 100) {
        alias width = 200;
        \Rect(x: 0, y: 0, width: width, height: 50, color: #ff0000)
    }
    \Card
    "#;
    let doc1 = parse(input1).expect("parse ok");
    let err1 = compile_to_graph(&doc1).expect_err("Should error on alias shadowing parameter");
    match err1 {
        directedtype::compiler::CompileError::DuplicatePort { port, .. } => assert_eq!(port, "width"),
        _ => panic!("Expected DuplicatePort, got {:?}", err1),
    }

    // Duplicate alias declarations
    let input2 = r#"
    \Component Card(w: 100) {
        alias right = w;
        alias right = w + 10;
        \Rect(x: 0, y: 0, width: w, height: 50, color: #ff0000)
    }
    \Card
    "#;
    let doc2 = parse(input2).expect("parse ok");
    let err2 = compile_to_graph(&doc2).expect_err("Should error on duplicate alias");
    match err2 {
        directedtype::compiler::CompileError::DuplicatePort { port, .. } => assert_eq!(port, "right"),
        _ => panic!("Expected DuplicatePort, got {:?}", err2),
    }
}

#[test]
fn test_reserved_parent_alias_error() {
    let input = r#"
    \Component Card() {
        alias parent = 10;
        \Rect(x: 0, y: 0, width: 10, height: 10, color: #ff0000)
    }
    \Card
    "#;
    let doc = parse(input).expect("parse ok");
    let err = compile_to_graph(&doc).expect_err("Should error on reserved alias 'parent'");
    match err {
        directedtype::compiler::CompileError::ReservedPort { port, .. } => assert_eq!(port, "parent"),
        _ => panic!("Expected ReservedPort, got {:?}", err),
    }
}

#[test]
fn test_component_key_purity_rejects_spatial_port_error() {
    let input = r#"
    \Rect(width / 2; x: 0, y: 0, width: 100, height: 50, color: #ff0000)
    "#;
    let doc = parse(input).expect("parse ok");
    let err = directedtype::evaluate_document(&doc).expect_err("Should reject spatial port in key");
    match err {
        directedtype::compiler::CompileError::InvalidComponentKeyDependency { node, name, .. } => {
            assert_eq!(node, "Rect");
            assert_eq!(name, "width");
        }
        _ => panic!("Expected InvalidComponentKeyDependency, got {:?}", err),
    }

    let input_member = r#"
    \Rect(self.height; x: 0, y: 0, width: 100, height: 50, color: #ff0000)
    "#;
    let doc_member = parse(input_member).expect("parse ok");
    let err_member = directedtype::evaluate_document(&doc_member).expect_err("Should reject self.height in key");
    match err_member {
        directedtype::compiler::CompileError::InvalidComponentKeyDependency { node, name, .. } => {
            assert_eq!(node, "Rect");
            assert_eq!(name, "height");
        }
        _ => panic!("Expected InvalidComponentKeyDependency, got {:?}", err_member),
    }

    let input_parent = r#"
    \Component Container {
        \Rect(parent.x; x: 0, y: 0, width: 50, height: 50, color: #000000)
    }
    \Container
    "#;
    let doc_parent = parse(input_parent).expect("parse ok");
    let err_parent = directedtype::evaluate_document(&doc_parent).expect_err("Should reject parent.x in key");
    match err_parent {
        directedtype::compiler::CompileError::InvalidComponentKeyDependency { node, name, .. } => {
            assert_eq!(node, "Rect");
            assert_eq!(name, "x");
        }
        _ => panic!("Expected InvalidComponentKeyDependency, got {:?}", err_parent),
    }
}

#[test]
fn test_component_key_purity_rejects_lexical_let_formula_error() {
    let input = r#"
    \Component Grid {
        let cell_width = 50;
        \Rect(cell_width; x: 0, y: 0, width: 50, height: 50, color: #ffffff)
    }
    \Grid
    "#;
    let doc = parse(input).expect("parse ok");
    let err = directedtype::evaluate_document(&doc).expect_err("Should reject let variable in key");
    match err {
        directedtype::compiler::CompileError::InvalidComponentKeyDependency { node, name, .. } => {
            assert_eq!(node, "Rect");
            assert_eq!(name, "cell_width");
        }
        _ => panic!("Expected InvalidComponentKeyDependency, got {:?}", err),
    }
}

#[test]
fn test_component_private_state_port_rejected() {
    let input = r#"
    \Component Counter {
        state count: Number: 0;
        \Rect(x: 0, y: 0, width: 100, height: 40, color: #000000)
    }
    \Counter(count: 10)
    "#;
    let doc = parse(input).expect("parse ok");
    let err = directedtype::evaluate_document(&doc).expect_err("Should reject caller passing private state port");
    match err {
        directedtype::compiler::CompileError::PrivateStatePort { node, port, .. } => {
            assert_eq!(node, "Counter");
            assert_eq!(port, "count");
        }
        _ => panic!("Expected PrivateStatePort, got {:?}", err),
    }
}

#[test]
fn test_component_state_evaluates_initial_defaults_in_layout_dag() {
    let input = r#"
    \Component Counter {
        state count: Number: 5;
        state is_active: Boolean: true;
        \Rect(
            x: 0,
            y: 0,
            width: count * 20,
            height: 40,
            color: is_active ? #3b82f6 : #1e293b
        )
    }
    \Counter
    "#;
    let doc = parse(input).expect("parse ok");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation should succeed");

    assert_eq!(layout.nodes.len(), 2);
    let counter_node = &layout.nodes[0];
    let rect_node = &layout.nodes[1];

    assert_eq!(counter_node.name, "Counter");
    assert_eq!(rect_node.name, "Rect");

    // Rect width should be count (5) * 20 = 100.0
    assert_eq!(rect_node.rect.width, 100.0);
    // Rect color should be "#3b82f6"
    assert_eq!(
        rect_node.properties.get("color"),
        Some(&Value::Color("#3b82f6".to_string()))
    );

    // Counter state values in layout values
    assert_eq!(
        layout.get_value(counter_node.id, "count"),
        Some(&Value::Number(5.0))
    );
    assert_eq!(
        layout.get_value(counter_node.id, "is_active"),
        Some(&Value::Bool(true))
    );
}

#[test]
fn test_top_level_state_evaluates_in_window() {
    let input = r#"
    state global_margin: Number: 30;
    \Rect(x: global_margin, y: global_margin, width: 200, height: 100, color: #ff0000)
    "#;
    let doc = parse(input).expect("parse ok");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation should succeed");

    assert_eq!(layout.nodes.len(), 1);
    let rect_node = &layout.nodes[0];
    assert_eq!(rect_node.rect.x, 30.0);
    assert_eq!(rect_node.rect.y, 30.0);

    assert_eq!(
        layout.get_value(directedtype::compiler::NodeId::WINDOW, "global_margin"),
        Some(&Value::Number(30.0))
    );
}

#[test]
fn test_structured_key_lookup_and_evaluation() {
    let input = r#"
    \Component TableView {
        state selected_row: Number: 1;
        \Rect("header"; x: 0, y: 0, width: 100, height: 30, color: #ffffff)
        \Rect(0, 0; x: 0, y: 30, width: 50, height: 30, color: #ffffff)
        \Rect(selected_row, 0; x: 0, y: 60, width: 50, height: 30, color: #ffffff)
    }
    \TableView
    "#;
    let doc = parse(input).expect("parse ok");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation should succeed");

    let table_id = layout.roots[0];

    // Find header by string key
    let header_node = layout.find_by_key(Some(table_id), &ComponentKey::string("header"));
    assert!(header_node.is_some());
    assert_eq!(header_node.unwrap().rect.y, 0.0);

    // Find cell (0, 0) by tuple key
    let cell_0_0 = layout.find_by_key(
        Some(table_id),
        &ComponentKey::tuple(&[Expr::number(0.0), Expr::number(0.0)]),
    );
    assert!(cell_0_0.is_some());
    assert_eq!(cell_0_0.unwrap().rect.y, 30.0);

    // Find cell (selected_row, 0) which evaluated to (1.0, 0.0)
    let cell_1_0 = layout.find_by_key(
        Some(table_id),
        &ComponentKey::tuple(&[Expr::number(1.0), Expr::number(0.0)]),
    );
    assert!(cell_1_0.is_some());
    assert_eq!(cell_1_0.unwrap().rect.y, 60.0);
}

#[test]
fn test_state_reserved_parent_and_duplicate_error() {
    let input_reserved = r#"
    \Component TestComp {
        state parent: Number: 0;
        \Rect(x: 0, y: 0, width: 10, height: 10, color: #ff0000)
    }
    \TestComp
    "#;
    let doc_res = parse(input_reserved).expect("parse ok");
    let err_res = directedtype::evaluate_document(&doc_res).expect_err("Should reject reserved state 'parent'");
    match err_res {
        directedtype::compiler::CompileError::ReservedPort { port, .. } => assert_eq!(port, "parent"),
        _ => panic!("Expected ReservedPort, got {:?}", err_res),
    }

    let input_dup = r#"
    \Component TestComp {
        state count: Number: 0;
        state count: Number: 1;
        \Rect(x: 0, y: 0, width: 10, height: 10, color: #ff0000)
    }
    \TestComp
    "#;
    let doc_dup = parse(input_dup).expect("parse ok");
    let err_dup = directedtype::evaluate_document(&doc_dup).expect_err("Should reject duplicate state");
    match err_dup {
        directedtype::compiler::CompileError::DuplicatePort { port, .. } => assert_eq!(port, "count"),
        _ => panic!("Expected DuplicatePort, got {:?}", err_dup),
    }
}

#[test]
fn test_raw_text_passed_directly_to_component_children() {
    let input = r#"
    env font = \Font(size: 16);

    \Component Card(padding_x: Number: 24, padding_y: Number: 18) {
        \Rect(x: 10, y: 10, width: 300, height: 100, color: #1e293b)
        \Children {
            x: parent.left + padding_x,
            y: parent.top + padding_y
        }
    }

    \Card { Hello world! }
    "#;

    let doc = parse(input).expect("parse ok");
    let layout = directedtype::evaluate_document(&doc).expect("evaluate ok");

    let text_node = layout.nodes.iter().find(|n| n.name == "Text").expect("Text node found");
    assert_eq!(text_node.text_content.as_deref(), Some("Hello world!"));
    assert_eq!(text_node.rect.x, 24.0); // parent.left (0) + 24
    assert_eq!(text_node.rect.y, 18.0); // parent.top (0) + 18
}

#[test]
fn test_component_def_param_helpers() {
    let input = r#"
    \Component TestComp(a: String, b: Number: 10, c: Color) {
        \Rect(width: 100, height: 100)
    }
    "#;
    let doc = parse(input).expect("parse ok");
    let comp = match &doc.items[0] {
        directedtype::ast::Item::Component(c) => c,
        _ => panic!("Expected component"),
    };

    let all_params = comp.param_names();
    assert_eq!(all_params.len(), 3);
    assert!(all_params.contains("a"));
    assert!(all_params.contains("b"));
    assert!(all_params.contains("c"));

    let req_params = comp.required_param_names();
    assert_eq!(req_params.len(), 2);
    assert!(req_params.contains("a"));
    assert!(req_params.contains("c"));
    assert!(!req_params.contains("b"));

    let opt_params = comp.optional_param_names();
    assert_eq!(opt_params.len(), 1);
    assert!(opt_params.contains("b"));

    assert!(comp.has_defaults());
}

#[test]
fn test_overload_error_formatting() {
    use directedtype::compiler::error::{AmbiguousOverloadDetails, CompileError, NoMatchingOverloadDetails};
    use directedtype::span::Span;

    let err1 = CompileError::PotentiallyAmbiguousOverloads(Box::new(AmbiguousOverloadDetails {
        name: "Card".to_string(),
        signature_a: vec!["text".to_string(), "width".to_string()],
        signature_b: vec!["text".to_string(), "height".to_string()],
        witness_overlap: vec!["text".to_string()],
        span: Span::new(10, 20),
        second_span: Span::new(30, 40),
    }));
    assert_eq!(err1.span(), Span::new(10, 20));
    assert!(err1.to_string().contains("Potentially ambiguous overloads for component 'Card'"));

    let err2 = CompileError::NoMatchingOverload(Box::new(NoMatchingOverloadDetails {
        name: "Card".to_string(),
        provided_ports: vec!["width".to_string()],
        available_signatures: vec![vec!["height".to_string()]],
        span: Span::new(5, 15),
    }));
    assert_eq!(err2.span(), Span::new(5, 15));
    assert!(err2.to_string().contains("No matching overload for component 'Card'"));

    let err3 = CompileError::DuplicateOverloadSignature {
        name: "Card".to_string(),
        signature: vec!["width".to_string()],
        span: Span::new(1, 10),
        second_span: Span::new(20, 30),
    };
    assert_eq!(err3.span(), Span::new(1, 10));
    assert!(err3.to_string().contains("Duplicate overload signature for component 'Card'"));
}

#[test]
fn test_overload_definition_time_ambiguity_rejected() {
    let input = r#"
    \Component Card(text: String, width: Number: 0) {
        \Rect(width: width, height: 100)
    }
    \Component Card(text: String, height: Number: 0) {
        \Rect(width: 100, height: height)
    }
    \Card(text: "Hello")
    "#;
    let doc = parse(input).expect("Failed to parse");
    let err = directedtype::evaluate_document(&doc).expect_err("Ambiguous overloads should fail at definition time");
    match err {
        CompileError::PotentiallyAmbiguousOverloads(details) => {
            assert_eq!(details.name, "Card");
            assert_eq!(details.witness_overlap, vec!["text"]);
        }
        other => panic!("Expected PotentiallyAmbiguousOverloads, got {:?}", other),
    }
}

#[test]
fn test_overload_subset_ambiguity_rejected() {
    let input = r#"
    \Component Box(width: Number: 100) {
        \Rect(width: width, height: 100)
    }
    \Component Box() {
        \Rect(width: 50, height: 50)
    }
    \Box()
    "#;
    let doc = parse(input).expect("Failed to parse");
    let err = directedtype::evaluate_document(&doc).expect_err("Subset overlap should fail at definition time");
    match err {
        CompileError::PotentiallyAmbiguousOverloads(details) => {
            assert_eq!(details.name, "Box");
            assert!(details.witness_overlap.is_empty());
        }
        other => panic!("Expected PotentiallyAmbiguousOverloads, got {:?}", other),
    }
}

#[test]
fn test_overload_duplicate_signature_rejected() {
    let input = r#"
    \Component Foo(x: Number) {
        \Rect(width: x, height: 10)
    }
    \Component Foo(x: Number) {
        \Rect(width: x, height: 20)
    }
    \Foo(x: 10)
    "#;
    let doc = parse(input).expect("Failed to parse");
    let err = directedtype::evaluate_document(&doc).expect_err("Duplicate signature should fail at definition time");
    match err {
        CompileError::DuplicateOverloadSignature { name, signature, .. } => {
            assert_eq!(name, "Foo");
            assert_eq!(signature, vec!["x"]);
        }
        other => panic!("Expected DuplicateOverloadSignature, got {:?}", other),
    }
}

#[test]
fn test_overload_non_overlapping_defaults_allowed() {
    let input = r#"
    \Component Card(text: String: "Default", width: Number) {
        \Rect(x: 0, y: 0, width: width, height: 50, color: #111111)
    }
    \Component Card(text: String: "Default", height: Number) {
        \Rect(x: 0, y: 0, width: 50, height: height, color: #222222)
    }

    \Card(width: 200)
    \Card(height: 120)
    "#;
    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Disjoint overloads should compile and evaluate");
    assert_eq!(layout.roots.len(), 2);

    let rects: Vec<_> = layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 2);
    assert_eq!(rects[0].rect.width, 200.0);
    assert_eq!(rects[0].rect.height, 50.0);
    assert_eq!(rects[1].rect.width, 50.0);
    assert_eq!(rects[1].rect.height, 120.0);
}

#[test]
fn test_overload_no_matching_overload_diagnostic() {
    let input = r#"
    \Component Card(width: Number) {
        \Rect(width: width, height: 50)
    }
    \Component Card(height: Number) {
        \Rect(width: 50, height: height)
    }

    \Card(color: #ffffff)
    "#;
    let doc = parse(input).expect("Failed to parse");
    let err = directedtype::evaluate_document(&doc).expect_err("Missing matching overload should return NoMatchingOverload");
    match err {
        CompileError::NoMatchingOverload(details) => {
            assert_eq!(details.name, "Card");
            assert_eq!(details.provided_ports, vec!["color"]);
            assert_eq!(details.available_signatures.len(), 2);
        }
        other => panic!("Expected NoMatchingOverload, got {:?}", other),
    }
}

#[test]
fn test_overload_dom_integration() {
    use directedtype::dom::Dom;

    let valid_input = r#"
    \Component Card(width: Number) { \Rect(width: width, height: 50) }
    \Component Card(height: Number) { \Rect(width: 50, height: height) }
    \Card(width: 100)
    "#;
    let doc = parse(valid_input).expect("Failed to parse");
    let dom = Dom::from_document(&doc).expect("Dom should register disjoint overloads");
    let roundtrip_doc = dom.to_document().expect("Dom to_document roundtrip");

    let comp_count = roundtrip_doc.items.iter().filter(|i| matches!(i, directedtype::ast::Item::Component(_))).count();
    assert_eq!(comp_count, 2, "Roundtripped document should preserve both overloads");

    let invalid_input = r#"
    \Component Card(width: Number: 10) { \Rect(width: width, height: 50) }
    \Component Card(height: Number: 20) { \Rect(width: 50, height: height) }
    "#;
    let doc_bad = parse(invalid_input).expect("Failed to parse");
    assert!(Dom::from_document(&doc_bad).is_err(), "Dom should reject ambiguous overloads");
}

#[test]
fn test_overload_exact_selection_at_call_site() {
    let input = r#"
    \Component Badge(text: String) {
        \Rect(x: 0, y: 0, width: 100, height: 20, color: #111111)
    }
    \Component Badge(text: String, count: Number) {
        \Rect(x: 0, y: 0, width: 150, height: count, color: #222222)
    }
    \Component Badge(icon: String, count: Number) {
        \Rect(x: 0, y: 0, width: 200, height: count, color: #333333)
    }

    \Badge(text: "Simple")
    \Badge(text: "Counted", count: 35)
    \Badge(icon: "star", count: 45)
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Overload selection should succeed");
    assert_eq!(layout.roots.len(), 3);

    let rects: Vec<_> = layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 3);
    assert_eq!(rects[0].rect.width, 100.0);
    assert_eq!(rects[0].rect.height, 20.0);
    assert_eq!(rects[1].rect.width, 150.0);
    assert_eq!(rects[1].rect.height, 35.0);
    assert_eq!(rects[2].rect.width, 200.0);
    assert_eq!(rects[2].rect.height, 45.0);
}

#[test]
fn test_overload_env_parameter_satisfaction() {
    let input = r#"
    let theme_val = "dark";
    env theme = theme_val;

    \Component ThemedBox(env theme: String, width: Number) {
        \Rect(x: 0, y: 0, width: width, height: 40, color: #101010)
    }
    \Component ThemedBox(width: Number, height: Number) {
        \Rect(x: 0, y: 0, width: width, height: height, color: #202020)
    }

    \ThemedBox(width: 80)
    \ThemedBox(width: 80, height: 60)
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Env-satisfied overload should resolve cleanly");
    assert_eq!(layout.roots.len(), 2);

    let rects: Vec<_> = layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 2);
    // Overload 1 (env theme)
    assert_eq!(rects[0].rect.width, 80.0);
    assert_eq!(rects[0].rect.height, 40.0);
    // Overload 2 (width + height)
    assert_eq!(rects[1].rect.width, 80.0);
    assert_eq!(rects[1].rect.height, 60.0);
}

#[test]
fn test_overload_card_in_hstack_no_cycle() {
    let input = r#"
    \Component HBox() {
        alias width = max(children.right);
        alias right = x + width;
        \Rect(x: 0, y: 0, width: width, height: 60, color: #000000)
        \Children {
            x: prev ? prev.right + 10 : parent.left
        }
    }

    \Component Card(width: Number) {
        alias right = x + width;
        \Rect(x: x, y: y, width: width, height: 50, color: #111111)
    }

    \Component Card() {
        let resolved_width = parent.width;
        alias right = x + resolved_width;
        \Rect(x: x, y: y, width: resolved_width, height: 50, color: #222222)
    }

    \HBox() {
        \Card(width: 150)
        \Card(width: 200)
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc)
        .expect("Fixed Card overload in HBox should not produce cyclic dependency");
    assert_eq!(layout.roots.len(), 1);

    let hbox_node = &layout.nodes[layout.roots[0].0];
    assert_eq!(hbox_node.name, "HBox");
    // HBox width should be 150 + 10 + 200 = 360
    assert_eq!(hbox_node.rect.width, 360.0);
}

#[test]
fn test_text_with_inline_link_spans() {
    let input = r#"
    \Text(width: 400, size: 16) {
        Visit \Link(url: "https://www.google.com"){Google} today
    }
    "#;
    let doc = parse(input).expect("Failed to parse Text with Link");
    let layout = directedtype::evaluate_document(&doc).expect("Failed to evaluate layout");
    assert_eq!(layout.roots.len(), 1);

    let text_node = &layout.nodes[layout.roots[0].0];
    assert_eq!(text_node.name, "Text");
    assert_eq!(text_node.text_content.as_deref(), Some("Visit Google today"));

    // Verify text spans
    assert_eq!(text_node.text_spans.len(), 3);

    // Span 0: "Visit "
    assert_eq!(text_node.text_spans[0].range, 0..6);
    assert_eq!(text_node.text_spans[0].node_id, None);
    assert_eq!(text_node.text_spans[0].style.url, None);

    // Span 1: "Google" (associated with Link node)
    assert_eq!(text_node.text_spans[1].range, 6..12);
    let link_node_id = text_node.text_spans[1].node_id.expect("Expected Link node ID on span");
    assert_eq!(text_node.text_spans[1].style.url.as_deref(), Some("https://www.google.com"));
    assert_eq!(text_node.text_spans[1].style.color.as_deref(), Some("#1a73e8"));
    assert!(text_node.text_spans[1].style.underline);
    assert_eq!(text_node.text_spans[1].style.cursor, Some(directedtype::compiler::CursorKind::Pointer));

    // Span 2: " today"
    assert_eq!(text_node.text_spans[2].range, 12..18);
    assert_eq!(text_node.text_spans[2].node_id, None);

    // Verify Link child node in hierarchy
    assert!(text_node.children.contains(&link_node_id));
    let link_node = &layout.nodes[link_node_id.0];
    assert_eq!(link_node.name, "Link");
    assert_eq!(link_node.text_content.as_deref(), Some("Google"));

    // Verify span lookup helper
    assert_eq!(text_node.span_for_node(link_node_id), Some(&text_node.text_spans[1]));
}

#[test]
fn test_text_with_custom_styled_link_span() {
    let input = r#"
    \Text(size: 16) {
        Check \Link(url: "https://github.com", color: #2563eb, underline: false){GitHub}
    }
    "#;
    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Failed to evaluate");

    let text_node = &layout.nodes[layout.roots[0].0];
    assert_eq!(text_node.text_content.as_deref(), Some("Check GitHub"));
    assert_eq!(text_node.text_spans.len(), 2);

    let link_span = &text_node.text_spans[1];
    assert_eq!(link_span.range, 6..12);
    assert_eq!(link_span.style.url.as_deref(), Some("https://github.com"));
    assert_eq!(link_span.style.color.as_deref(), Some("#2563eb"));
    assert!(!link_span.style.underline);
    assert_eq!(link_span.style.cursor, Some(directedtype::compiler::CursorKind::Pointer));
}

#[test]
fn test_plain_text_has_default_span() {
    let input = r#"\Text(size: 16) { Plain text here }"#;
    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Failed to evaluate");

    let text_node = &layout.nodes[layout.roots[0].0];
    assert_eq!(text_node.text_content.as_deref(), Some("Plain text here"));
    assert_eq!(text_node.text_spans.len(), 1);
    assert_eq!(text_node.text_spans[0].range, 0..15);
    assert_eq!(text_node.text_spans[0].node_id, None);
    assert_eq!(text_node.text_spans[0].style, directedtype::compiler::SpanStyle::default());
}

#[test]
fn test_rect_bounding_union() {
    use directedtype::compiler::Rect;

    assert_eq!(Rect::bounding_union(&[]), None);

    let r1 = Rect::new(10.0, 5.0, 100.0, 20.0);
    assert_eq!(Rect::bounding_union(&[r1]), Some(r1));

    let r2 = Rect::new(0.0, 30.0, 80.0, 25.0);
    // Combined: min_x = 0, min_y = 5, max_x = 110, max_y = 55
    // width = 110, height = 50
    let union = Rect::bounding_union(&[r1, r2]).expect("Expected union");
    assert_eq!(union.x, 0.0);
    assert_eq!(union.y, 5.0);
    assert_eq!(union.width, 110.0);
    assert_eq!(union.height, 50.0);
}

#[test]
fn test_single_line_link_fragments_projection() {
    let input = r#"\Text(width: 500, size: 16) { Visit \Link(url: "https://google.com"){Google} now }"#;
    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Failed to evaluate");

    let text_node = &layout.nodes[layout.roots[0].0];
    assert_eq!(text_node.text_spans.len(), 3);

    let link_child_id = text_node.text_spans[1].node_id.expect("Expected child node id");
    let link_node = layout.get_node(link_child_id).expect("Link node should exist");

    assert_eq!(link_node.fragments.len(), 1);
    let frag = link_node.fragments[0];
    assert!(frag.x > 0.0, "Expected positive x, got {}", frag.x);
    assert!(frag.width > 20.0, "Expected positive width, got {}", frag.width);
    assert!(frag.height > 10.0, "Expected positive height, got {}", frag.height);

    assert_eq!(link_node.rect, frag);
    assert_eq!(link_node.properties.get("url").and_then(|v| v.as_str()), Some("https://google.com"));
    assert_eq!(link_node.properties.get("cursor").and_then(|v| v.as_str()), Some("Pointer"));
}

#[test]
fn test_multiline_wrapped_link_fragments_projection() {
    // Narrow width forces the link phrase to wrap across line boundaries
    let input = r#"\Text(width: 160, size: 16) { Welcome and visit our \Link(url: "https://example.com"){comprehensive guide to documentation} today }"#;
    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Failed to evaluate");

    let text_node = &layout.nodes[layout.roots[0].0];
    let link_span = text_node.text_spans.iter().find(|s| s.node_id.is_some()).expect("Expected link span");
    let link_node = layout.get_node(link_span.node_id.unwrap()).expect("Link node should exist");

    assert!(
        link_node.fragments.len() >= 2,
        "Expected multi-line link to produce at least 2 fragments, got {}",
        link_node.fragments.len()
    );

    let frag1 = link_node.fragments[0];
    let frag2 = link_node.fragments[1];

    // Frag 1 is on line 1, Frag 2 is on line 2 (strictly below)
    assert!(frag1.y < frag2.y, "Frag 1 y ({}) should be < Frag 2 y ({})", frag1.y, frag2.y);
    assert!(frag1.width > 0.0);
    assert!(frag2.width > 0.0);

    // Bounding union contains both fragments
    assert_eq!(link_node.rect, directedtype::compiler::Rect::bounding_union(&link_node.fragments).unwrap());
    assert!(link_node.rect.height >= frag1.height + frag2.height);
}

#[test]
fn test_missing_size_on_text_yields_compile_error() {
    let input = r#"\Text { Hello missing size }"#;
    let doc = parse(input).expect("parse ok");
    let err = compile_to_graph(&doc).unwrap_err();
    match err {
        directedtype::compiler::CompileError::MissingPort { node, port, .. } => {
            assert_eq!(node, "Text");
            assert_eq!(port, "size");
        }
        other => panic!("Expected MissingPort on Text size, got: {:?}", other),
    }
}

#[test]
fn test_missing_size_on_font_yields_compile_error() {
    let input = r#"let f = \Font(weight: 500);"#;
    let doc = parse(input).expect("parse ok");
    let err = compile_to_graph(&doc).unwrap_err();
    match err {
        directedtype::compiler::CompileError::MissingPort { node, port, .. } => {
            assert_eq!(node, "Font");
            assert_eq!(port, "size");
        }
        other => panic!("Expected MissingPort on Font size, got: {:?}", other),
    }
}

#[test]
fn test_font_rejects_name_port_error() {
    let input = r#"let f = \Font(name: "Times New Roman", size: 16);"#;
    let doc = parse(input).expect("parse ok");
    let err = compile_to_graph(&doc).expect_err("Expected compile error for name on Font");
    match err {
        directedtype::compiler::CompileError::NoMatchingOverload(details) => {
            assert_eq!(details.name, "Font");
            assert!(details.provided_ports.contains(&"name".to_string()));
            assert!(details.available_signatures.iter().any(|sig| sig.contains(&"family".to_string())));
        }
        other => panic!("Expected NoMatchingOverload CompileError, got: {:?}", other),
    }
}

#[test]
fn test_env_font_rejects_name_port_error() {
    let input = r#"env font = \Font(name: "Arial", size: 12);"#;
    let doc = parse(input).expect("parse ok");
    let err = compile_to_graph(&doc).expect_err("Expected compile error for name on env font");
    match err {
        directedtype::compiler::CompileError::NoMatchingOverload(details) => {
            assert_eq!(details.name, "Font");
            assert!(details.provided_ports.contains(&"name".to_string()));
        }
        other => panic!("Expected NoMatchingOverload CompileError, got: {:?}", other),
    }
}

#[test]
fn test_font_rejects_unknown_ports() {
    let input = r#"let f = \Font(size: 16, color: #ff0000);"#;
    let doc = parse(input).expect("parse ok");
    let err = compile_to_graph(&doc).expect_err("Expected compile error for color on Font");
    match err {
        directedtype::compiler::CompileError::NoMatchingOverload(details) => {
            assert_eq!(details.name, "Font");
            assert!(details.provided_ports.contains(&"color".to_string()));
        }
        other => panic!("Expected NoMatchingOverload CompileError, got: {:?}", other),
    }
}

#[test]
fn test_font_accepts_line_height_port() {
    let input = r#"
    let f = \Font(family: "Arial", size: 32, line_height: 48);
    let r = \Rect(x: 0, y: 0, width: 100, height: f.line_height, color: #ffffff);
    "#;
    let doc = parse(input).expect("parse ok");
    let layout = directedtype::evaluate_document(&doc).expect("layout ok");
    let r_node = layout.nodes.iter().find(|n| n.name == "Rect").expect("rect found");
    assert_eq!(r_node.rect.height, 48.0);
}

#[test]
fn test_font_line_height_forwarded_from_text() {
    let input = r#"
    env font = \Font(family: "Arial", size: 32, line_height: 52);
    let t = \Text { Hello };
    let r = \Rect(x: 0, y: 0, width: 100, height: t.font.line_height, color: #ffffff);
    let r2 = \Rect(x: 0, y: 0, width: 100, height: t.line_height, color: #ffffff);
    "#;
    let doc = parse(input).expect("parse ok");
    let layout = directedtype::evaluate_document(&doc).expect("layout ok");
    let rects: Vec<_> = layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 2);
    assert_eq!(rects[0].rect.height, 52.0);
    assert_eq!(rects[1].rect.height, 52.0);
}


#[test]
fn test_missing_dimensions_on_child_rect_yields_compile_error() {
    let input = r#"
    \Rect(x: 0, y: 0, width: 400, height: 300, color: #ffffff) {
        \Rect(x: 0, y: 0, color: #ff0000)
    }
    "#;
    let doc = parse(input).expect("parse ok");
    let err = compile_to_graph(&doc).unwrap_err();
    match err {
        directedtype::compiler::CompileError::MissingPort { node, port, .. } => {
            assert_eq!(node, "Rect");
            assert!(port == "width" || port == "height");
        }
        other => panic!("Expected MissingPort on Rect width/height, got: {:?}", other),
    }
}

#[test]
fn test_text_baseline_and_text_font_baseline() {
    let input = r#"
    env font = \Font(size: 32, family: "Arial");
    let t = \Text(x: 10, y: 50, start_at: TextStart.Ascender, end_at: TextEnd.Descender) {
        Typography
    };
    let guide_direct = \Rect(x: 0, y: t.baseline, width: 100, height: 1, color: #ff0000);
    let guide_via_font = \Rect(x: 0, y: t.font.baseline, width: 100, height: 1, color: #00ff00);
    let cap_y = \Rect(x: 0, y: t.font.baseline - t.font.cap_height, width: 100, height: 1, color: #0000ff);
    "#;
    let doc = parse(input).expect("parse ok");
    let layout = directedtype::evaluate_document(&doc).expect("layout ok");
    let rects: Vec<_> = layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 3);
    // Both t.baseline and t.font.baseline evaluate to the exact same y coordinate
    assert_eq!(rects[0].rect.y, rects[1].rect.y);
    assert!(rects[0].rect.y > 50.0);
    // Cap height guide is above the baseline
    assert!(rects[2].rect.y < rects[0].rect.y);
    assert_eq!(rects[2].rect.y, rects[0].rect.y - (32.0 * 0.71));
}

#[test]
fn test_font_family_not_found_in_font_static() {
    let input = r#"
    \Font(family: "NonExistentFontXYZ", size: 16)
    "#;
    let doc = parse(input).expect("parse ok");
    let err = directedtype::evaluate_document(&doc).unwrap_err();
    match err {
        CompileError::FontFamilyNotFound { family, .. } => {
            assert_eq!(family, "NonExistentFontXYZ");
        }
        other => panic!("Expected FontFamilyNotFound, got: {:?}", other),
    }
}

#[test]
fn test_font_family_not_found_in_text_family_static() {
    let input = r#"
    \Text(family: "NonExistentFontXYZ", size: 16) { Hello }
    "#;
    let doc = parse(input).expect("parse ok");
    let err = directedtype::evaluate_document(&doc).unwrap_err();
    match err {
        CompileError::FontFamilyNotFound { family, .. } => {
            assert_eq!(family, "NonExistentFontXYZ");
        }
        other => panic!("Expected FontFamilyNotFound, got: {:?}", other),
    }
}

#[test]
fn test_font_family_not_found_in_text_font_static() {
    let input = r#"
    \Text(font: "NonExistentFontXYZ", size: 16) { Hello }
    "#;
    let doc = parse(input).expect("parse ok");
    let err = directedtype::evaluate_document(&doc).unwrap_err();
    match err {
        CompileError::FontFamilyNotFound { family, .. } => {
            assert_eq!(family, "NonExistentFontXYZ");
        }
        other => panic!("Expected FontFamilyNotFound, got: {:?}", other),
    }
}

#[test]
fn test_font_family_not_found_dynamic() {
    let input = r#"
    let fam = "NonExistentFontXYZ";
    \Font(family: fam, size: 16)
    "#;
    let doc = parse(input).expect("parse ok");
    let err = directedtype::evaluate_document(&doc).unwrap_err();
    match err {
        CompileError::FontFamilyNotFound { family, .. } => {
            assert_eq!(family, "NonExistentFontXYZ");
        }
        other => panic!("Expected FontFamilyNotFound, got: {:?}", other),
    }
}

#[test]
fn test_font_family_generic_and_valid_fonts_succeed() {
    let input = r#"
    \Font(family: "sans-serif", size: 16)
    \Font(family: "serif", size: 16)
    \Font(family: "monospace", size: 16)
    \Font(family: "Arial", size: 16)
    \Font(size: 16)
    \Text(font: "sans-serif", size: 16) { Test }
    "#;
    let doc = parse(input).expect("parse ok");
    assert!(directedtype::evaluate_document(&doc).is_ok());
}







