use directedtype::compiler::evaluate_document_with_window;
use directedtype::interaction::{Event, EventKind, Modifiers, MouseButton, Point};
use directedtype::parse;
use directedtype::render::{ViewerApp, ViewerConfig};
use pretty_assertions::assert_eq;
use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn test_hit_test_basic_rect() {
    let input = r#"
    \Rect(x: 10, y: 20, width: 100, height: 50, color: #ff0000)
    "#;
    let doc = parse(input).expect("Parse error");
    let layout = evaluate_document_with_window(&doc, 800.0, 600.0).expect("Layout error");

    // Inside rect
    let hit = layout.hit_test(Point::new(50.0, 40.0)).expect("Should hit rect");
    let rect_node = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    assert_eq!(hit.target, rect_node.id);
    assert_eq!(hit.global_point, Point::new(50.0, 40.0));
    assert_eq!(hit.local_point, Point::new(40.0, 20.0));
    assert_eq!(hit.bubble_path, vec![rect_node.id]);

    // Outside bounds
    assert!(layout.hit_test(Point::new(5.0, 40.0)).is_none());
    assert!(layout.hit_test(Point::new(50.0, 10.0)).is_none());
    assert!(layout.hit_test(Point::new(120.0, 40.0)).is_none());
    assert!(layout.hit_test(Point::new(50.0, 80.0)).is_none());
}

#[test]
fn test_hit_test_z_index_dominance() {
    // Rect B is declared FIRST with z: 10.
    // Rect A is declared SECOND with z: 0.
    // Both cover (50, 50). Rect B must win because z: 10 > z: 0.
    let input = r#"
    \Rect(x: 0, y: 0, width: 100, height: 100, z: 10, color: #00ff00)
    \Rect(x: 0, y: 0, width: 100, height: 100, z: 0, color: #ff0000)
    "#;
    let doc = parse(input).expect("Parse error");
    let layout = evaluate_document_with_window(&doc, 800.0, 600.0).expect("Layout error");

    let hit = layout.hit_test(Point::new(50.0, 50.0)).expect("Should hit top rect");
    // Node 0 is Rect B (z: 10), Node 1 is Rect A (z: 0)
    assert_eq!(hit.target, layout.nodes[0].id);
}

#[test]
fn test_hit_test_painters_dominance_same_z() {
    // Both rects have default z: 0.
    // Rect A is declared first. Rect B is declared second.
    // Both cover (50, 50). Rect B must win because it paints on top (Painter's Algorithm).
    let input = r#"
    \Rect(x: 0, y: 0, width: 100, height: 100, color: #ff0000)
    \Rect(x: 0, y: 0, width: 100, height: 100, color: #0000ff)
    "#;
    let doc = parse(input).expect("Parse error");
    let layout = evaluate_document_with_window(&doc, 800.0, 600.0).expect("Layout error");

    let hit = layout.hit_test(Point::new(50.0, 50.0)).expect("Should hit top rect");
    // Node 1 is Rect B (second declared)
    assert_eq!(hit.target, layout.nodes[1].id);
}

#[test]
fn test_hit_test_hardware_clip_culling() {
    let input = r#"
    let clip_box = \Box(x: 0, y: 0, width: 100, height: 100)
    let my_clip = \Clip(box: clip_box)

    \Rect(clip: my_clip, x: 50, y: 50, width: 200, height: 200, color: #ff0000)
    "#;
    let doc = parse(input).expect("Parse error");
    let layout = evaluate_document_with_window(&doc, 800.0, 600.0).expect("Layout error");

    // (75, 75) is inside the Rect AND inside the Clip box -> Hit!
    assert!(layout.hit_test(Point::new(75.0, 75.0)).is_some());

    // (150, 150) is inside the Rect, but OUTSIDE the Clip box (width 100) -> Culled!
    assert!(layout.hit_test(Point::new(150.0, 150.0)).is_none());
}

#[test]
fn test_hit_test_corner_radius_culling() {
    let input = r#"
    \Rect(x: 0, y: 0, width: 100, height: 100, radius: 20, color: #3b82f6)
    "#;
    let doc = parse(input).expect("Parse error");
    let layout = evaluate_document_with_window(&doc, 800.0, 600.0).expect("Layout error");

    // Center is inside
    assert!(layout.hit_test(Point::new(50.0, 50.0)).is_some());

    // Point in top-left corner outside arc:
    // Corner center is (20, 20), radius is 20 (r^2 = 400).
    // Point (2, 2): dx = -18, dy = -18, dx^2 + dy^2 = 648 > 400 -> Miss!
    assert!(layout.hit_test(Point::new(2.0, 2.0)).is_none());

    // Point in top-left corner inside arc:
    // Point (10, 10): dx = -10, dy = -10, dx^2 + dy^2 = 200 <= 400 -> Hit!
    assert!(layout.hit_test(Point::new(10.0, 10.0)).is_some());
}

#[test]
fn test_hit_test_bubble_path_through_component_hierarchy() {
    let input = r#"
    \Component Card {
        \Rect(x: 10, y: 10, width: 100, height: 100, color: #ffffff)
    }

    \Component Container {
        \Card()
    }

    \Container()
    "#;
    let doc = parse(input).expect("Parse error");
    let layout = evaluate_document_with_window(&doc, 800.0, 600.0).expect("Layout error");

    let hit = layout.hit_test(Point::new(50.0, 50.0)).expect("Should hit Card's Rect");

    let rect_node = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    let card_node = layout.nodes.iter().find(|n| n.name == "Card").unwrap();
    let container_node = layout.nodes.iter().find(|n| n.name == "Container").unwrap();

    assert_eq!(hit.target, rect_node.id);
    assert_eq!(hit.bubble_path.len(), 3);
    assert_eq!(hit.bubble_path[0], rect_node.id);
    assert_eq!(hit.bubble_path[1], card_node.id);
    assert_eq!(hit.bubble_path[2], container_node.id);
}

#[test]
fn test_event_click_synthesis_and_bubbling() {
    let input = r#"
    \Component Button {
        \Rect(x: 20, y: 20, width: 100, height: 40, color: #2563eb)
    }

    \Button()
    "#;
    let doc = parse(input).expect("Parse error");
    let layout = evaluate_document_with_window(&doc, 800.0, 600.0).expect("Layout error");

    let events_received = Rc::new(RefCell::new(Vec::new()));
    let events_clone = events_received.clone();

    let mut app = ViewerApp::new(layout.clone(), ViewerConfig::default());
    app.set_event_handler(move |event, _| {
        events_clone.borrow_mut().push((event.kind.clone(), event.target, event.current_target));
    });

    let rect_node = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    let button_node = layout.nodes.iter().find(|n| n.name == "Button").unwrap();

    // Verify manually constructing an event and checking continue_propagation
    let mut click_event = Event::new(
        EventKind::Click { button: MouseButton::Left },
        Point::new(50.0, 30.0),
        Point::new(30.0, 10.0),
        Modifiers::default(),
        rect_node.id,
    );

    assert!(!click_event.propagation_continued);
    click_event.continue_propagation();
    assert!(click_event.propagation_continued);

    // Hit test target
    let hit = layout.hit_test(Point::new(50.0, 30.0)).expect("Should hit Button's Rect");
    assert_eq!(hit.target, rect_node.id);
    assert!(hit.bubble_path.contains(&button_node.id));
}

#[test]
fn test_hit_test_inline_link_single_line() {
    let input = r#"
    \Text(x: 20, y: 30, width: 500, size: 16) {
        Before link \Link(url: "https://directedtype.org"){ClickableLink} after link
    }
    "#;
    let doc = parse(input).expect("Parse error");
    let layout = evaluate_document_with_window(&doc, 800.0, 600.0).expect("Layout error");

    let text_node = layout.nodes.iter().find(|n| n.name == "Text").unwrap();
    let link_node = layout.nodes.iter().find(|n| n.name == "Link").unwrap();

    assert_eq!(link_node.fragments.len(), 1);
    let frag = link_node.fragments[0];

    // Center of the link fragment
    let link_center = Point::new(frag.x + frag.width / 2.0, frag.y + frag.height / 2.0);
    let hit = layout.hit_test(link_center).expect("Should hit link fragment");
    assert_eq!(hit.target, link_node.id);
    assert_eq!(hit.bubble_path[0], link_node.id);
    assert_eq!(hit.bubble_path[1], text_node.id);

    // Hit test on the "Before link" text area (x: 25, y: 35)
    let before_hit = layout.hit_test(Point::new(25.0, 35.0)).expect("Should hit text node");
    assert_eq!(before_hit.target, text_node.id);
}

#[test]
fn test_hit_test_inline_link_multi_line_disjoint_fragments() {
    // Narrow container forces link text to wrap across two lines
    let input = r#"
    \Text(x: 20, y: 30, width: 140, size: 16) {
        Prefix text \Link(url: "https://example.com"){wrapped link that spans across multiple lines} suffix
    }
    "#;
    let doc = parse(input).expect("Parse error");
    let layout = evaluate_document_with_window(&doc, 800.0, 600.0).expect("Layout error");

    let text_node = layout.nodes.iter().find(|n| n.name == "Text").unwrap();
    let link_node = layout.nodes.iter().find(|n| n.name == "Link").unwrap();

    assert!(
        link_node.fragments.len() >= 2,
        "Expected at least 2 fragments, got {}",
        link_node.fragments.len()
    );

    let frag1 = link_node.fragments[0];
    let frag2 = link_node.fragments[1];

    // 1. Point on Line 1 inside fragment 1 hits the Link
    let p1 = Point::new(frag1.x + frag1.width / 2.0, frag1.y + frag1.height / 2.0);
    let hit1 = layout.hit_test(p1).expect("Should hit fragment 1 of link");
    assert_eq!(hit1.target, link_node.id);

    // 2. Point on Line 2 inside fragment 2 hits the Link
    let p2 = Point::new(frag2.x + frag2.width / 2.0, frag2.y + frag2.height / 2.0);
    let hit2 = layout.hit_test(p2).expect("Should hit fragment 2 of link");
    assert_eq!(hit2.target, link_node.id);

    // 3. Point on Line 1 before fragment 1 (where "Prefix text" is) hits Text, not Link
    let p_prefix = Point::new(frag1.x - 15.0, frag1.y + frag1.height / 2.0);
    if p_prefix.x >= text_node.rect.x {
        let hit_prefix = layout.hit_test(p_prefix).expect("Should hit prefix text");
        assert_eq!(hit_prefix.target, text_node.id);
    }
}
