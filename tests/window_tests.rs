use directedtype::compiler::error::CompileError;
use directedtype::compiler::expanded::NodeId;
use directedtype::compiler::graph::VarId;
use directedtype::compiler::{
    compile_document_with_registry, evaluate_document_with_window, expand_document, FsResolver, Value,
};
use directedtype::component::ComponentRegistry;
use directedtype::parse;
use directedtype::render::{ViewerApp, ViewerConfig};
use pretty_assertions::assert_eq;

#[test]
fn test_window_declaration_sets_ports_and_evaluates() {
    let input = r#"
    \Window(vscroll: false, hscroll: false, title: "Custom Title")

    \Rect(
        x: window.vscroll ? 100 : 20,
        y: 10,
        width: 100,
        height: 50,
        color: #ff0000
    )
    "#;
    let doc = parse(input).expect("parse ok");
    let expanded = expand_document(&doc).expect("expand ok");

    // Window must NOT be inserted as a node into the DOM
    assert_eq!(expanded.roots.len(), 1);
    assert_eq!(expanded.nodes.len(), 1);
    assert_eq!(expanded.nodes[0].name, "Rect");

    let layout = evaluate_document_with_window(&doc, 800.0, 600.0).expect("eval ok");
    assert_eq!(
        layout.values.get(&VarId::new(NodeId::WINDOW, "vscroll")),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        layout.values.get(&VarId::new(NodeId::WINDOW, "hscroll")),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        layout.values.get(&VarId::new(NodeId::WINDOW, "title")),
        Some(&Value::String("Custom Title".to_string()))
    );

    let rect = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    assert_eq!(rect.rect.x, 20.0);
}

#[test]
fn test_window_order_independence() {
    // Window declared at the top
    let input1 = r#"
    \Window(vscroll: false, hscroll: false, title: "Order Test")

    \let w = 250;
    \Rect(x: window.vscroll ? 50 : 20, y: 0, width: w, height: 100, color: #ff0000)
    "#;

    // Window declared at the bottom
    let input2 = r#"
    \let w = 250;
    \Rect(x: window.vscroll ? 50 : 20, y: 0, width: w, height: 100, color: #ff0000)

    \Window(vscroll: false, hscroll: false, title: "Order Test")
    "#;

    let doc1 = parse(input1).expect("parse 1 ok");
    let doc2 = parse(input2).expect("parse 2 ok");

    let layout1 = evaluate_document_with_window(&doc1, 800.0, 600.0).expect("eval 1 ok");
    let layout2 = evaluate_document_with_window(&doc2, 800.0, 600.0).expect("eval 2 ok");

    assert_eq!(layout1.roots.len(), layout2.roots.len());
    assert_eq!(layout1.nodes.len(), layout2.nodes.len());
    assert_eq!(layout1.nodes[0].rect, layout2.nodes[0].rect);
    assert_eq!(
        layout1.values.get(&VarId::new(NodeId::WINDOW, "vscroll")),
        layout2.values.get(&VarId::new(NodeId::WINDOW, "vscroll"))
    );
    assert_eq!(
        layout1.values.get(&VarId::new(NodeId::WINDOW, "hscroll")),
        layout2.values.get(&VarId::new(NodeId::WINDOW, "hscroll"))
    );
    assert_eq!(
        layout1.values.get(&VarId::new(NodeId::WINDOW, "title")),
        layout2.values.get(&VarId::new(NodeId::WINDOW, "title"))
    );
}

#[test]
fn test_window_disallows_spatial_geometry_ports() {
    let spatial_ports = ["x", "y", "z", "width", "height", "left", "top", "right", "bottom", "clip"];
    for port in spatial_ports {
        let input = format!(r#"\Window({port}: 100)"#);
        let doc = parse(&input).expect("parse ok");
        let err = expand_document(&doc).unwrap_err();
        match err {
            CompileError::DisallowedWindowPort { port: p, .. } => {
                assert_eq!(p, port);
            }
            other => panic!("Expected DisallowedWindowPort for '{port}', got: {other:?}"),
        }
    }
}

#[test]
fn test_window_disallows_arbitrary_custom_ports() {
    let custom_ports = ["pad", "gutter", "breakpoint", "theme", "foo", "my_var"];
    for port in custom_ports {
        let input = format!(r#"\Window({port}: 100)"#);
        let doc = parse(&input).expect("parse ok");
        let err = expand_document(&doc).unwrap_err();
        match err {
            CompileError::DisallowedWindowPort { port: p, .. } => {
                assert_eq!(p, port);
            }
            other => panic!("Expected DisallowedWindowPort for '{port}', got: {other:?}"),
        }
    }
}

#[test]
fn test_window_disallows_duplicate_declaration() {
    let input = r#"
    \Window(vscroll: false)
    \Window(hscroll: false)
    "#;
    let doc = parse(input).expect("parse ok");
    let err = expand_document(&doc).unwrap_err();
    match err {
        CompileError::DuplicateWindowDeclaration { .. } => {}
        other => panic!("Expected DuplicateWindowDeclaration, got: {other:?}"),
    }
}

#[test]
fn test_window_disallows_children_content_block() {
    let input = r#"
    \Window(vscroll: false) {
        \Rect(width: 100, height: 100)
    }
    "#;
    let doc = parse(input).expect("parse ok");
    let err = expand_document(&doc).unwrap_err();
    match err {
        CompileError::WindowContentNotAllowed { .. } => {}
        other => panic!("Expected WindowContentNotAllowed, got: {other:?}"),
    }
}

#[test]
fn test_window_disallows_key() {
    let input = r#"
    \Window("my_key"; vscroll: false)
    "#;
    let doc = parse(input).expect("parse ok");
    let err = expand_document(&doc).unwrap_err();
    match err {
        CompileError::Custom { message, .. } => {
            assert!(message.contains("key"));
        }
        other => panic!("Expected Custom error about key, got: {other:?}"),
    }
}

#[test]
fn test_window_disallows_nesting_in_component() {
    let input = r#"
    \Component MyCard() {
        \Window(vscroll: false)
    }

    \MyCard()
    "#;
    let doc = parse(input).expect("parse ok");
    let err = expand_document(&doc).unwrap_err();
    match err {
        CompileError::NestedWindowDeclaration { .. } => {}
        other => panic!("Expected NestedWindowDeclaration, got: {other:?}"),
    }
}

#[test]
fn test_window_disallows_nesting_in_element() {
    let input = r#"
    \Rect(width: 100, height: 100, color: #000) {
        \Window(vscroll: false)
    }
    "#;
    let doc = parse(input).expect("parse ok");
    let err = expand_document(&doc).unwrap_err();
    match err {
        CompileError::NestedWindowDeclaration { .. } => {}
        other => panic!("Expected NestedWindowDeclaration, got: {other:?}"),
    }
}

#[test]
fn test_window_disallows_let_binding() {
    let input = r#"
    \let w = \Window(vscroll: false);
    "#;
    let doc = parse(input).expect("parse ok");
    let err = expand_document(&doc).unwrap_err();
    match err {
        CompileError::NestedWindowDeclaration { .. } => {}
        other => panic!("Expected NestedWindowDeclaration, got: {other:?}"),
    }
}

#[test]
fn test_window_disallows_defining_component_named_window() {
    let input = r#"
    \Component Window() {
        \Rect(width: 100, height: 100)
    }
    "#;
    let doc = parse(input).expect("parse ok");
    let err = expand_document(&doc).unwrap_err();
    match err {
        CompileError::Custom { message, .. } => {
            assert!(message.contains("reserved"));
        }
        other => panic!("Expected Custom error about reserved component name, got: {other:?}"),
    }
}

#[test]
fn test_window_disallows_duplicate_or_reserved_ports() {
    // Duplicate port
    let input_dup = r#"\Window(vscroll: false, vscroll: true)"#;
    let doc_dup = parse(input_dup).expect("parse ok");
    assert!(matches!(
        expand_document(&doc_dup).unwrap_err(),
        CompileError::DuplicatePort { .. }
    ));

    // Conflict with state
    let input_state = r#"
    \state vscroll = false;
    \Window(vscroll: true)
    "#;
    let doc_state = parse(input_state).expect("parse ok");
    assert!(matches!(
        expand_document(&doc_state).unwrap_err(),
        CompileError::DuplicatePort { .. }
    ));
}

#[test]
fn test_window_vscroll_disables_scrolling_in_viewer() {
    let input = r#"
    \Window(vscroll: false)

    \Rect(x: 0, y: 0, width: 400, height: 2000, color: #112233)
    "#;
    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = compile_document_with_registry(
        &doc,
        800.0,
        600.0,
        std::path::Path::new("."),
        &FsResolver,
        &registry,
    )
    .expect("compile ok");

    let mut app = ViewerApp::new_with_compiled(
        compiled,
        ViewerConfig {
            width: 800,
            height: 600,
            show_nav_bar: false,
            ..Default::default()
        },
    );

    assert!(!app.is_vscroll_enabled());
    assert_eq!(app.scroll_y(), 0.0);

    // Attempting to scroll must have no effect:
    app.dispatch_scroll(0.0, -100.0);
    assert_eq!(app.scroll_y(), 0.0);

    app.set_scroll_y(500.0);
    assert_eq!(app.scroll_y(), 0.0);

    app.scroll_to_y(300.0);
    assert_eq!(app.scroll_y(), 0.0);
}

#[test]
fn test_window_vscroll_true_allows_scrolling_in_viewer() {
    let input = r#"
    \Window(vscroll: true)

    \Rect(x: 0, y: 0, width: 400, height: 2000, color: #112233)
    "#;
    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = compile_document_with_registry(
        &doc,
        800.0,
        600.0,
        std::path::Path::new("."),
        &FsResolver,
        &registry,
    )
    .expect("compile ok");

    let mut app = ViewerApp::new_with_compiled(
        compiled,
        ViewerConfig {
            width: 800,
            height: 600,
            show_nav_bar: false,
            ..Default::default()
        },
    );

    assert!(app.is_vscroll_enabled());
    assert_eq!(app.scroll_y(), 0.0);

    // Scrolling down by 100
    app.dispatch_scroll(0.0, -100.0);
    assert_eq!(app.scroll_y(), 100.0);

    app.set_scroll_y(300.0);
    assert_eq!(app.scroll_y(), 300.0);
}

#[test]
fn test_window_hscroll_disables_scrolling_in_viewer() {
    let input = r#"
    \Window(hscroll: false)

    \Rect(x: 0, y: 0, width: 2500, height: 400, color: #112233)
    "#;
    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = compile_document_with_registry(
        &doc,
        800.0,
        600.0,
        std::path::Path::new("."),
        &FsResolver,
        &registry,
    )
    .expect("compile ok");

    let mut app = ViewerApp::new_with_compiled(
        compiled,
        ViewerConfig {
            width: 800,
            height: 600,
            show_nav_bar: false,
            ..Default::default()
        },
    );

    assert!(!app.is_hscroll_enabled());
    assert_eq!(app.scroll_x(), 0.0);
    assert!(!app.is_dragging_h_scrollbar());

    // Attempting to scroll horizontally must have no effect:
    app.dispatch_scroll(-100.0, 0.0);
    assert_eq!(app.scroll_x(), 0.0);

    app.set_scroll_x(500.0);
    assert_eq!(app.scroll_x(), 0.0);

    app.scroll_to_x(300.0);
    assert_eq!(app.scroll_x(), 0.0);
}

#[test]
fn test_window_hscroll_true_allows_scrolling_in_viewer() {
    let input = r#"
    \Window(hscroll: true)

    \Rect(x: 0, y: 0, width: 2000, height: 400, color: #112233)
    "#;
    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = compile_document_with_registry(
        &doc,
        800.0,
        600.0,
        std::path::Path::new("."),
        &FsResolver,
        &registry,
    )
    .expect("compile ok");

    let mut app = ViewerApp::new_with_compiled(
        compiled,
        ViewerConfig {
            width: 800,
            height: 600,
            show_nav_bar: false,
            ..Default::default()
        },
    );

    assert!(app.is_hscroll_enabled());
    assert_eq!(app.scroll_x(), 0.0);
    assert!(!app.is_dragging_h_scrollbar());

    // Content is 2000, viewport is 800 -> max_scroll_x is 1200
    assert_eq!(app.max_scroll_x(800.0), 1200.0);

    // Scroll right by 150
    app.dispatch_scroll(-150.0, 0.0);
    assert_eq!(app.scroll_x(), 150.0);

    app.set_scroll_x(500.0);
    assert_eq!(app.scroll_x(), 500.0);

    app.scroll_to_x(800.0);
    assert_eq!(app.scroll_x(), 800.0);

    // Clamps to max_scroll_x
    app.scroll_to_x(5000.0);
    assert_eq!(app.scroll_x(), 1200.0);

    // Clamps to 0
    app.scroll_to_x(-500.0);
    assert_eq!(app.scroll_x(), 0.0);
}

#[test]
fn test_window_both_vscroll_and_hscroll_supported() {
    let input = r#"
    \Window(vscroll: true, hscroll: true)

    \Rect(x: 0, y: 0, width: 2000, height: 3000, color: #112233)
    "#;
    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = compile_document_with_registry(
        &doc,
        800.0,
        600.0,
        std::path::Path::new("."),
        &FsResolver,
        &registry,
    )
    .expect("compile ok");

    let mut app = ViewerApp::new_with_compiled(
        compiled,
        ViewerConfig {
            width: 800,
            height: 600,
            show_nav_bar: false,
            ..Default::default()
        },
    );

    assert!(app.is_vscroll_enabled());
    assert!(app.is_hscroll_enabled());

    app.set_scroll_offset(300.0, 500.0);
    assert_eq!(app.scroll_offset(), (300.0, 500.0));

    assert_eq!(app.max_scroll_x(800.0), 1200.0);
    assert_eq!(app.max_scroll_y(600.0), 2400.0);
}
