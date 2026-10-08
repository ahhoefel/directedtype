use directedtype::compiler::{CompiledDocument, FsResolver, NodeId, Value};
use directedtype::component::{ComponentRegistry, ContextAction};
use directedtype::interaction::{Event, EventKind, MouseButton, Point};
use directedtype::parser::parse_document;
use std::path::Path;

#[test]
fn test_ctx_window_scroll_to() {
    let source = r##"
        \Component TestComp {
            \Rect(x: 0, y: 0, width: 100, height: 100, color: #1e293b, on_click: self.click)
        }
        \TestComp
    "##;

    struct TestComp;
    impl directedtype::component::Component for TestComp {
        fn dispatch(
            &mut self,
            method: &str,
            _event: &mut Event,
            ctx: &mut directedtype::component::Context<'_>,
        ) -> Result<(), directedtype::component::DispatchError> {
            if method == "click" {
                ctx.window().scroll_to(NodeId(42));
            }
            Ok(())
        }
    }

    let mut registry = ComponentRegistry::new();
    registry.register("TestComp", || Box::new(TestComp));

    let ast = parse_document(source).expect("parse ok");
    let mut compiled = CompiledDocument::compile_with_registry(
        &ast,
        800.0,
        600.0,
        Path::new("."),
        &FsResolver,
        &registry,
    )
    .expect("compile ok");

    let rect_node = compiled.layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    let mut click_event = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(10.0, 10.0),
        Point::new(5.0, 5.0),
        Default::default(),
        rect_node.id,
    )
    .with_bubble_path(vec![rect_node.id]);

    compiled.dispatch_event(&mut click_event).expect("dispatch ok");
    let actions = compiled.take_actions();
    assert_eq!(
        actions,
        vec![ContextAction::ScrollToNode {
            target: NodeId(42),
            container: None,
        }],
        "ctx.window().scroll_to must queue a ScrollToNode targeting None (the window)"
    );
}

#[test]
fn test_link_default_view_is_window() {
    let source = r##"
        \use "components/Link.dt";
        \use "theme/default.dt";

        \VStack(gap: 20) {
            \Link(url: "#target_header", link_style: link_default) {
                Jump to Target
            }
            \Anchor("target_header") {
                \Rect(width: 200, height: 50, color: #3b82f6)
            }
        }
    "##;

    let ast = parse_document(source).expect("parse ok");
    let mut compiled = CompiledDocument::compile_with_registry(
        &ast,
        800.0,
        600.0,
        Path::new("."),
        &FsResolver,
        &ComponentRegistry::standard(),
    )
    .expect("compile ok");

    let link_node = compiled.layout.nodes.iter().find(|n| n.name == "Link").unwrap();
    let link_id = link_node.id;
    let link_point = Point::new(link_node.rect.x + 5.0, link_node.rect.y + 5.0);
    let target_anchor_id = compiled.layout.nodes.iter().find(|n| n.name == "Anchor").unwrap().id;

    let mut click = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        link_point,
        Point::default(),
        Default::default(),
        link_id,
    )
    .with_bubble_path(vec![link_id]);

    compiled.dispatch_event(&mut click).expect("dispatch click ok");
    let actions = compiled.take_actions();

    assert_eq!(
        actions,
        vec![ContextAction::ScrollToNode {
            target: target_anchor_id,
            container: None,
        }],
        "By default, Link view is window (container: None)"
    );
}

#[test]
fn test_link_custom_view_is_scroll_view() {
    let source = r##"
        \use "components/ScrollView.dt";
        \use "components/Link.dt";
        \use "theme/default.dt";

        let pane = \ScrollView(width: 400, height: 250) {
            \Anchor("inside_pane") {
                \Rect(width: 100, height: 40, color: #10b981)
            }
        };

        \Link(url: "#inside_pane", link_style: link_default, view: pane) {
            Scroll pane to anchor
        }
    "##;

    let ast = parse_document(source).expect("parse ok");
    let mut compiled = CompiledDocument::compile_with_registry(
        &ast,
        800.0,
        600.0,
        Path::new("."),
        &FsResolver,
        &ComponentRegistry::standard(),
    )
    .expect("compile ok");

    let pane_id = compiled.layout.nodes.iter().find(|n| n.name == "ScrollView").unwrap().id;
    let link_node = compiled.layout.nodes.iter().find(|n| n.name == "Link").unwrap();
    let link_id = link_node.id;
    let link_point = Point::new(link_node.rect.x + 5.0, link_node.rect.y + 5.0);
    let target_anchor_id = compiled.layout.nodes.iter().find(|n| n.name == "Anchor").unwrap().id;

    let mut click = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        link_point,
        Point::default(),
        Default::default(),
        link_id,
    )
    .with_bubble_path(vec![link_id]);

    compiled.dispatch_event(&mut click).expect("dispatch click ok");
    let actions = compiled.take_actions();

    assert_eq!(
        actions,
        vec![ContextAction::ScrollToNode {
            target: target_anchor_id,
            container: Some(pane_id),
        }],
        "Passing view: pane must target that specific ScrollView container"
    );
}

#[test]
fn test_scroll_view_hardware_clip_boundary() {
    let source = r##"
        \use "components/ScrollView.dt";

        \ScrollView(width: 300, height: 200, radius: 12) {
            \Rect(width: 400, height: 500, color: #3b82f6)
        }
    "##;

    let ast = parse_document(source).expect("parse ok");
    let compiled = CompiledDocument::compile_with_registry(
        &ast,
        800.0,
        600.0,
        Path::new("."),
        &FsResolver,
        &ComponentRegistry::standard(),
    )
    .expect("compile ok");

    let scroll_view = compiled.layout.nodes.iter().find(|n| n.name == "ScrollView").unwrap();
    let child_rect = compiled.layout.nodes.iter().find(|n| n.parent == Some(scroll_view.id) && n.name == "Rect" && n.rect.width == 400.0).unwrap();

    // Child must be clipped to the ScrollView's hardware clip node
    assert!(child_rect.clip.is_some(), "Child inside ScrollView must inherit clip");
    let clip_node = compiled.layout.get_node(child_rect.clip.unwrap()).unwrap();
    assert_eq!(clip_node.name, "Clip");
}

#[test]
fn test_scroll_view_on_scroll_event() {
    let source = r##"
        \use "components/ScrollView.dt";

        \ScrollView(width: 300, height: 200) {
            \Rect(width: 200, height: 600, color: #3b82f6)
        }
    "##;

    let ast = parse_document(source).expect("parse ok");
    let mut compiled = CompiledDocument::compile_with_registry(
        &ast,
        800.0,
        600.0,
        Path::new("."),
        &FsResolver,
        &ComponentRegistry::standard(),
    )
    .expect("compile ok");

    let scroll_view = compiled.layout.nodes.iter().find(|n| n.name == "ScrollView").unwrap();
    let scroll_view_id = scroll_view.id;

    let bg_rect = compiled.layout.nodes.iter().find(|n| n.parent == Some(scroll_view_id) && n.name == "Rect" && n.rect.width == 300.0).unwrap();

    // Dispatch scroll event: wheel down (delta_y = -35.0)
    let mut scroll_event = Event::new(
        EventKind::Scroll {
            delta_x: 0.0,
            delta_y: -35.0,
        },
        Point::new(50.0, 50.0),
        Point::new(50.0, 50.0),
        Default::default(),
        bg_rect.id,
    )
    .with_bubble_path(vec![bg_rect.id, scroll_view_id]);

    let changed = compiled.dispatch_event(&mut scroll_event).expect("dispatch ok");
    assert!(!changed.is_empty(), "Dispatching on_scroll should trigger DAG re-evaluation");

    assert_eq!(
        compiled.get_state(scroll_view_id, "scroll_y"),
        Some(&Value::Number(35.0)),
        "ScrollView scroll_y should be updated to 35.0"
    );

    // Child rect retains static layout coordinates (DAG isolation), while clip provides GPU translation offset
    let child_rect = compiled.layout.nodes.iter().find(|n| n.parent == Some(scroll_view_id) && n.name == "Rect" && n.rect.width == 200.0).unwrap();
    assert_eq!(child_rect.rect.y, 0.0, "Child rect layout position must remain static in DAG");
    let (sx, sy) = compiled.layout.clip_scroll_offset(child_rect.clip);
    assert_eq!((sx, sy), (0.0, 35.0), "Clip must carry GPU scroll translation offset (0.0, 35.0)");
    assert_eq!(child_rect.rect.y - sy, -35.0, "Visual position rendered on GPU is shifted by -35.0");

    // Hit-testing inside the scrolled clip maps screen coords to static node coords
    let hit = compiled.layout.hit_test(Point::new(50.0, 15.0)).expect("hit test ok");
    assert_eq!(hit.target, child_rect.id, "Hit test must correctly target the scrolled child");
}

#[test]
fn test_scroll_view_scroll_container_to_node() {
    let source = r##"
        \use "components/ScrollView.dt";

        \ScrollView(width: 300, height: 200) {
            \Rect(width: 200, height: 100, color: #ef4444)
            \Anchor("middle") {
                \Rect(width: 200, height: 100, color: #3b82f6)
            }
            \Rect(width: 200, height: 100, color: #10b981)
        }
    "##;

    let ast = parse_document(source).expect("parse ok");
    let mut compiled = CompiledDocument::compile_with_registry(
        &ast,
        800.0,
        600.0,
        Path::new("."),
        &FsResolver,
        &ComponentRegistry::standard(),
    )
    .expect("compile ok");

    let scroll_view = compiled.layout.nodes.iter().find(|n| n.name == "ScrollView").unwrap();
    let scroll_view_id = scroll_view.id;

    let anchor = compiled.layout.nodes.iter().find(|n| n.name == "Anchor").unwrap();
    let anchor_id = anchor.id;

    // Anchor is initially at y = 100.0
    assert_eq!(anchor.rect.y, 100.0);

    // Scroll container to the anchor
    let changed = compiled.scroll_container_to_node(scroll_view_id, anchor_id).expect("scroll ok");
    assert!(!changed.is_empty());

    // scroll_y should now be 100.0
    assert_eq!(
        compiled.get_state(scroll_view_id, "scroll_y"),
        Some(&Value::Number(100.0))
    );

    // Target anchor layout position remains static, while clip scroll offset brings it visually to y = 0.0
    let updated_anchor = compiled.layout.get_node(anchor_id).unwrap();
    assert_eq!(updated_anchor.rect.y, 100.0, "Anchor layout position remains static");
    let (sx, sy) = compiled.layout.clip_scroll_offset(updated_anchor.clip);
    assert_eq!((sx, sy), (0.0, 100.0));
    assert_eq!(updated_anchor.rect.y - sy, 0.0, "Anchor visual position on GPU is brought to top of ScrollView (y = 0.0)");
}

#[test]
fn test_end_to_end_link_scrolls_scroll_view() {
    let source = r##"
        \use "components/ScrollView.dt";
        \use "components/Link.dt";
        \use "theme/default.dt";

        let my_scroll = \ScrollView(width: 300, height: 150) {
            \Rect(width: 200, height: 120, color: #ef4444)
            \Anchor("deep_section") {
                \Rect(width: 200, height: 100, color: #3b82f6)
            }
            \Rect(width: 200, height: 150, color: #10b981)
        };

        \Link(url: "#deep_section", link_style: link_default, view: my_scroll) {
            Jump To Deep Section
        }
    "##;

    let ast = parse_document(source).expect("parse ok");
    let mut compiled = CompiledDocument::compile_with_registry(
        &ast,
        800.0,
        600.0,
        Path::new("."),
        &FsResolver,
        &ComponentRegistry::standard(),
    )
    .expect("compile ok");

    let link_node = compiled.layout.nodes.iter().find(|n| n.name == "Link").unwrap();
    let scroll_view_node = compiled.layout.nodes.iter().find(|n| n.name == "ScrollView").unwrap();
    let scroll_view_id = scroll_view_node.id;

    let anchor = compiled.layout.nodes.iter().find(|n| n.name == "Anchor").unwrap();
    let anchor_id = anchor.id;
    assert_eq!(anchor.rect.y, 120.0);

    // 1. Click link
    let mut click = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(link_node.rect.x + 5.0, link_node.rect.y + 5.0),
        Point::default(),
        Default::default(),
        link_node.id,
    )
    .with_bubble_path(vec![link_node.id]);

    compiled.dispatch_event(&mut click).expect("dispatch click ok");
    let actions = compiled.take_actions();
    assert_eq!(actions.len(), 1);

    // 2. Execute the action through compiled document pipeline
    let action = actions.into_iter().next().unwrap();
    let changed = compiled.execute_action(action).expect("execute ok");
    assert!(!changed.is_empty());

    // 3. ScrollView state is updated to 120.0, bringing anchor visually to y = 0.0
    assert_eq!(
        compiled.get_state(scroll_view_id, "scroll_y"),
        Some(&Value::Number(120.0))
    );
    let updated_anchor = compiled.layout.get_node(anchor_id).unwrap();
    assert_eq!(updated_anchor.rect.y, 120.0, "Anchor layout coordinate remains static");
    let (sx, sy) = compiled.layout.clip_scroll_offset(updated_anchor.clip);
    assert_eq!((sx, sy), (0.0, 120.0));
    assert_eq!(updated_anchor.rect.y - sy, 0.0, "Anchor visual position on GPU is at y = 0.0");
}
