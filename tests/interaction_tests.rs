use directedtype::compiler::{compile_document_with_registry, evaluate_document_with_window, FsResolver};
use directedtype::component::{Callback, ComponentRegistry, Context, DispatchError, EmitCallback};
use directedtype::interaction::{Event, EventKind, Modifiers, MouseButton, Point};
use directedtype::parse;
use directedtype::render::{ViewerApp, ViewerConfig};
use directedtype_macros::component;
use pretty_assertions::assert_eq;
use std::cell::RefCell;
use std::rc::Rc;
use winit::event::ElementState;

#[test]
fn test_hit_test_basic_rect() {
    let input = r#"
    \Rect(x: 10px, y: 20px, width: 100px, height: 50px, color: #ff0000)
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
    \Rect(x: 0, y: 0, width: 100px, height: 100px, z: 10, color: #00ff00)
    \Rect(x: 0, y: 0, width: 100px, height: 100px, z: 0, color: #ff0000)
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
    \Rect(x: 0, y: 0, width: 100px, height: 100px, color: #ff0000)
    \Rect(x: 0, y: 0, width: 100px, height: 100px, color: #0000ff)
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
    \let clip_box = \Box(x: 0, y: 0, width: 100px, height: 100px)
    \let my_clip = \Clip(box: clip_box)

    \Rect(clip: my_clip, x: 50px, y: 50px, width: 200px, height: 200px, color: #ff0000)
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
    \Rect(x: 0, y: 0, width: 100px, height: 100px, radius: 20px, color: #3b82f6)
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
        \Rect(x: 10px, y: 10px, width: 100px, height: 100px, color: #ffffff)
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
        \Rect(x: 20px, y: 20px, width: 100px, height: 40px, color: #2563eb)
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
    \Text(x: 20px, y: 30px, width: 500px, size: 16px) {
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
    \Text(x: 20px, y: 30px, width: 140px, size: 16px) {
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

#[test]
fn test_pointer_capture_explicit_routing() {
    let input = r#"
    \Rect(x: 10px, y: 10px, width: 100px, height: 50px, color: #ff0000)
    \Rect(x: 200px, y: 200px, width: 100px, height: 50px, color: #0000ff)
    "#;
    let doc = parse(input).expect("Parse error");
    let layout = evaluate_document_with_window(&doc, 800.0, 600.0).expect("Layout error");
    let rect_a = layout.nodes[0].clone();
    let rect_b = layout.nodes[1].clone();

    let mut app = ViewerApp::new(layout.clone(), ViewerConfig::default());
    let events = Rc::new(RefCell::new(Vec::new()));
    let events_clone = Rc::clone(&events);
    app.set_event_handler(move |event, _| {
        events_clone
            .borrow_mut()
            .push((event.kind.clone(), event.target, event.local_point));
    });

    // 1. Move to (250, 225) without capture -> hits Rect B
    app.dispatch_cursor_moved(Point::new(250.0, 225.0));
    {
        let evs = events.borrow();
        assert!(!evs.is_empty());
        let last = evs.last().unwrap();
        assert_eq!(last.1, rect_b.id);
    }

    // 2. Explicitly capture pointer to Rect A
    app.set_captured_node(Some(rect_a.id));
    assert_eq!(app.captured_node(), Some(rect_a.id));

    // 3. Move to (250, 225) (over Rect B) with capture -> MUST route to Rect A!
    events.borrow_mut().clear();
    app.dispatch_cursor_moved(Point::new(250.0, 225.0));
    {
        let evs = events.borrow();
        assert_eq!(evs.len(), 1);
        let (kind, target, local_pt) = &evs[0];
        assert_eq!(*kind, EventKind::PointerMove);
        assert_eq!(*target, rect_a.id);
        // local_point relative to rect_a (10, 10): 250 - 10 = 240, 225 - 10 = 215
        assert_eq!(*local_pt, Point::new(240.0, 215.0));
    }

    // 4. Move far to the left of Rect A: (-20, -15)
    events.borrow_mut().clear();
    app.dispatch_cursor_moved(Point::new(-20.0, -15.0));
    {
        let evs = events.borrow();
        assert_eq!(evs.len(), 1);
        let (kind, target, local_pt) = &evs[0];
        assert_eq!(*kind, EventKind::PointerMove);
        assert_eq!(*target, rect_a.id);
        // Clean negative offset for clamping: -20 - 10 = -30, -15 - 10 = -25
        assert_eq!(*local_pt, Point::new(-30.0, -25.0));
    }

    // 5. Release mouse button -> Dispatches PointerUp to Rect A and automatically releases capture!
    events.borrow_mut().clear();
    app.dispatch_mouse_input(ElementState::Released, MouseButton::Left, Point::new(250.0, 225.0));
    {
        let evs = events.borrow();
        assert_eq!(evs.len(), 1);
        let (kind, target, _) = &evs[0];
        assert_eq!(*kind, EventKind::PointerUp { button: MouseButton::Left });
        assert_eq!(*target, rect_a.id);
    }
    assert_eq!(app.captured_node(), None, "Capture must automatically release on PointerUp");

    // 6. Next move over Rect B hits Rect B normally
    events.borrow_mut().clear();
    app.dispatch_cursor_moved(Point::new(250.0, 225.0));
    {
        let evs = events.borrow();
        assert!(!evs.is_empty());
        let last = evs.last().unwrap();
        assert_eq!(last.1, rect_b.id);
    }
}

#[derive(Default, Debug, Clone)]
struct DragTargetCompanion {
    pub down_called: bool,
    pub drag_moves: Vec<Point>,
    pub up_called: bool,
}

#[component]
impl DragTargetCompanion {
    pub fn on_down(&mut self, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        self.down_called = true;
        ctx.capture_pointer();
        Ok(())
    }

    pub fn on_move(&mut self, event: &mut Event) -> Result<(), DispatchError> {
        self.drag_moves.push(event.local_point);
        Ok(())
    }

    pub fn on_up(&mut self) -> Result<(), DispatchError> {
        self.up_called = true;
        Ok(())
    }
}

#[test]
fn test_pointer_capture_via_context_action_and_companion() {
    let input = r#"
    \Component Draggable(x: Distance = 50px, y: Distance = 50px, width: Distance = 100px, height: Distance = 50px) {
        \Rect(
            x: x,
            y: y,
            width: width,
            height: height,
            color: #3b82f6,
            on_pointer_down: on_down,
            on_pointer_move: on_move,
            on_pointer_up: on_up
        )
    }

    \Draggable(x: 50px, y: 50px)
    \Rect(x: 300px, y: 300px, width: 100px, height: 100px, color: #10b981)
    "#;
    let doc = parse(input).expect("Parse error");
    let mut registry = ComponentRegistry::standard();
    registry.register_companion("Draggable", "Draggable.rs", || {
        Box::new(DragTargetCompanion::default())
    });

    let compiled = compile_document_with_registry(
        &doc,
        800.0,
        600.0,
        std::path::Path::new("."),
        &FsResolver,
        &registry,
    )
    .expect("compile ok");

    let mut app = ViewerApp::new_with_compiled(compiled, ViewerConfig::default());
    let drag_node = app
        .layout()
        .nodes
        .iter()
        .find(|n| n.name == "Rect" && n.rect.x == 50.0)
        .expect("Draggable rect found")
        .id;

    assert_eq!(app.captured_node(), None);

    // 1. Mouse down inside Draggable rect at (60, 60)
    app.dispatch_mouse_input(ElementState::Pressed, MouseButton::Left, Point::new(60.0, 60.0));

    // Captured node should now be set to the interactive Rect (which received pointer down and called ctx.capture_pointer())!
    assert_eq!(app.captured_node(), Some(drag_node), "Draggable must acquire pointer capture on down");

    // 2. Drag cursor far across the screen to (400, 400) (over the green Rect at 300, 300)
    app.dispatch_cursor_moved(Point::new(400.0, 400.0));

    // 3. Move cursor to negative coordinates (-50, -50)
    app.dispatch_cursor_moved(Point::new(-50.0, -50.0));

    // 4. Release mouse button
    app.dispatch_mouse_input(ElementState::Released, MouseButton::Left, Point::new(400.0, 400.0));

    // Capture must now be released!
    assert_eq!(app.captured_node(), None, "Capture must release after PointerUp");
}

#[derive(Default)]
struct TestSlider {
    pub value: f64,
    pub on_change: Option<Callback<f64>>,
}

#[component]
impl TestSlider {
    pub fn on_mount(&mut self, ctx: &mut Context<'_>) {
        self.on_change = ctx.get_callback("on_change");
        if let Some(v) = ctx.get_state_number("value") {
            self.value = v;
        }
    }

    pub fn on_pointer_down(&mut self, event: &mut Event, ctx: &mut Context<'_>) {
        ctx.capture_pointer();
        self.handle_drag(event.local_point.x, ctx);
    }

    pub fn on_pointer_move(&mut self, event: &mut Event, ctx: &mut Context<'_>) {
        self.handle_drag(event.local_point.x, ctx);
    }

    pub fn on_pointer_up(&mut self, ctx: &mut Context<'_>) {
        ctx.release_pointer();
    }

    fn handle_drag(&mut self, local_x: f64, ctx: &mut Context<'_>) {
        let new_val = (local_x / 100.0 * 100.0).clamp(0.0, 100.0);
        self.value = new_val;
        ctx.set_state("value", new_val);
        self.on_change.emit(ctx, new_val);
    }
}

#[derive(Default)]
struct TestAudioPanel {
    pub current_volume: f64,
}

#[component]
impl TestAudioPanel {
    pub fn on_mount(&mut self, ctx: &mut Context<'_>) {
        if let Some(v) = ctx.get_state_number("volume") {
            self.current_volume = v;
        }
    }

    pub fn set_volume(&mut self, val: f64, ctx: &mut Context<'_>) {
        self.current_volume = val;
        ctx.set_state("volume", val);
    }
}

#[test]
fn test_component_typed_callback_wiring_and_execution() {
    let input = r#"
    \Component TestSlider(
        x: Distance: 0,
        y: Distance: 0,
        width: Distance: 100px,
        height: Distance: 20px
    ) {
        \state value: Float: 0;
        \Rect(
            x: x,
            y: y,
            width: width,
            height: height,
            color: #333333,
            on_pointer_down: on_pointer_down,
            on_pointer_move: on_pointer_move,
            on_pointer_up: on_pointer_up
        )
    }

    \Component TestAudioPanel() {
        \state volume: Float: 25;
        \TestSlider(
            x: 10px,
            y: 10px,
            width: 100px,
            height: 20px,
            on_change: set_volume
        )
    }

    \TestAudioPanel()
    "#;

    let doc = parse(input).expect("parse ok");
    let mut registry = ComponentRegistry::new();
    registry.register("TestSlider", || Box::new(TestSlider::default()));
    registry.register("TestAudioPanel", || Box::new(TestAudioPanel::default()));

    let compiled = compile_document_with_registry(
        &doc,
        800.0,
        600.0,
        std::path::Path::new("."),
        &FsResolver,
        &registry,
    )
    .expect("compile ok");

    let mut app = ViewerApp::new_with_compiled(compiled, ViewerConfig::default());

    let slider_node = app
        .layout()
        .nodes
        .iter()
        .find(|n| n.name == "TestSlider")
        .expect("TestSlider node found")
        .id;

    let panel_node = app
        .layout()
        .nodes
        .iter()
        .find(|n| n.name == "TestAudioPanel")
        .expect("TestAudioPanel node found")
        .id;

    // Initial state check
    assert_eq!(
        app.compiled().unwrap().get_state(panel_node, "volume"),
        Some(&directedtype::Value::Float(25.0))
    );
    assert_eq!(
        app.compiled().unwrap().get_state(slider_node, "value"),
        Some(&directedtype::Value::Float(0.0))
    );

    let rect_node = app
        .layout()
        .nodes
        .iter()
        .find(|n| n.name == "Rect" && n.parent == Some(slider_node))
        .expect("Slider Rect found")
        .id;

    // 1. Mouse down at (60, 15), which is local_x = 50 in the slider (x: 10 to 110)
    app.dispatch_mouse_input(ElementState::Pressed, MouseButton::Left, Point::new(60.0, 15.0));

    // Pointer capture must be active on the slider's interactive Rect!
    assert_eq!(app.captured_node(), Some(rect_node));

    // Both Slider's value and AudioPanel's volume must have been updated in one batch!
    assert_eq!(
        app.compiled().unwrap().get_state(slider_node, "value"),
        Some(&directedtype::Value::Float(50.0))
    );
    assert_eq!(
        app.compiled().unwrap().get_state(panel_node, "volume"),
        Some(&directedtype::Value::Float(50.0))
    );

    // 2. Drag to (85, 15) -> local_x = 75
    app.dispatch_cursor_moved(Point::new(85.0, 15.0));
    assert_eq!(
        app.compiled().unwrap().get_state(slider_node, "value"),
        Some(&directedtype::Value::Float(75.0))
    );
    assert_eq!(
        app.compiled().unwrap().get_state(panel_node, "volume"),
        Some(&directedtype::Value::Float(75.0))
    );

    // 3. Drag far outside window to (300, 15) -> clamped to 100
    app.dispatch_cursor_moved(Point::new(300.0, 15.0));
    assert_eq!(
        app.compiled().unwrap().get_state(slider_node, "value"),
        Some(&directedtype::Value::Float(100.0))
    );
    assert_eq!(
        app.compiled().unwrap().get_state(panel_node, "volume"),
        Some(&directedtype::Value::Float(100.0))
    );

    // 4. Release mouse button -> capture is cleared
    app.dispatch_mouse_input(ElementState::Released, MouseButton::Left, Point::new(300.0, 15.0));
    assert_eq!(app.captured_node(), None);
}


