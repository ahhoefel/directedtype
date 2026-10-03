use directedtype::ast::ComponentKey;
use directedtype::component::{
    Component, ComponentRegistry, Context, DispatchError,
};
use directedtype::compiler::module::{FileResolver, VirtualResolver};
use directedtype::compiler::{compile_document_with_registry, CompiledDocument};
use directedtype::dom::Dom;
use directedtype::interaction::{Event, EventKind, Modifiers, MouseButton, Point};
use directedtype::parse;
use pretty_assertions::assert_eq;
use std::path::{Path, PathBuf};

// Companion component: Counter
#[derive(Default, Debug)]
struct CounterComponent {
    pub count: f64,
}

impl Component for CounterComponent {
    fn on_mount(&mut self, ctx: &mut Context<'_>) {
        if let Some(initial) = ctx.get_port_number("initial") {
            self.count = initial;
            ctx.set_state("count", self.count);
        }
    }

    fn dispatch(
        &mut self,
        method: &str,
        _event: &Event,
        ctx: &mut Context<'_>,
    ) -> Result<(), DispatchError> {
        match method {
            "increment" => {
                self.count += 1.0;
                ctx.set_state("count", self.count);
                Ok(())
            }
            "decrement" => {
                if self.count > 0.0 {
                    self.count -= 1.0;
                    ctx.set_state("count", self.count);
                }
                Ok(())
            }
            _ => Err(DispatchError::MethodNotFound {
                component: "Counter".into(),
                method: method.into(),
            }),
        }
    }
}

// Companion component: Drawer
#[derive(Default, Debug)]
struct DrawerComponent {
    pub is_open: bool,
}

impl Component for DrawerComponent {
    fn on_mount(&mut self, _ctx: &mut Context<'_>) {}

    fn dispatch(
        &mut self,
        method: &str,
        _event: &Event,
        ctx: &mut Context<'_>,
    ) -> Result<(), DispatchError> {
        match method {
            "toggle" => {
                self.is_open = !self.is_open;
                ctx.set_state("is_open", self.is_open);
                Ok(())
            }
            _ => Err(DispatchError::MethodNotFound {
                component: "Drawer".into(),
                method: method.into(),
            }),
        }
    }
}

#[test]
fn test_companion_file_discovery_via_resolver() {
    let mut resolver = VirtualResolver::new();
    resolver.insert("components/Counter.dt", "\\Component Counter {}");
    resolver.insert("components/Counter.rs", "pub struct Counter;");
    resolver.insert("components/Button.dt", "\\Component Button {}");

    // Counter has Counter.rs -> Some
    let counter_path = Path::new("components/Counter.dt");
    assert_eq!(
        resolver.find_companion_rs(counter_path),
        Some(PathBuf::from("components/Counter.rs"))
    );

    // Button does not have Button.rs -> None
    let button_path = Path::new("components/Button.dt");
    assert_eq!(resolver.find_companion_rs(button_path), None);
}

#[test]
fn test_companion_component_on_mount_lifecycle() {
    let input = r#"
    \Component Counter(initial: Number: 0) {
        state count: Number = 0;
        \Rect(x: 0, y: 0, width: count * 10, height: 40, color: #3b82f6)
    }

    \Counter(initial: 7)
    "#;

    let doc = parse(input).expect("parse ok");
    let mut registry = ComponentRegistry::new();
    registry.register("Counter", || Box::new(CounterComponent::default()));

    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let rect = compiled.layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    // on_mount initialized count to 7 -> width should evaluate to 70.0
    assert_eq!(rect.rect.width, 70.0);
}

#[test]
fn test_event_dispatch_self_increment_and_decrement() {
    let input = r#"
    \Component Counter {
        state count: Number: 0;
        \Rect(x: 10, y: 10, width: 100 + count * 20, height: 40, color: #10b981, on_click: self.increment)
    }

    \Counter
    "#;

    let doc = parse(input).expect("parse ok");
    let mut registry = ComponentRegistry::new();
    registry.register("Counter", || Box::new(CounterComponent::default()));

    let mut compiled = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    // Initially count is 0 -> width 100.0
    let rect_initial = compiled.layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    assert_eq!(rect_initial.rect.width, 100.0);

    let rect_id = rect_initial.id;

    // Simulate clicking the rect
    let mut click_event = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(15.0, 15.0),
        Point::new(5.0, 5.0),
        Modifiers::default(),
        rect_id,
    );

    let changed = compiled.dispatch_event(&mut click_event).expect("dispatch ok");
    assert!(!changed.is_empty(), "Should update downstream variables");

    // Layout should update in-place immediately: count is now 1 -> width 120.0
    let rect_after_1 = compiled.layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    assert_eq!(rect_after_1.rect.width, 120.0);

    // Second click -> count becomes 2 -> width 140.0
    let mut click_event_2 = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(15.0, 15.0),
        Point::new(5.0, 5.0),
        Modifiers::default(),
        rect_id,
    );
    compiled.dispatch_event(&mut click_event_2).expect("dispatch ok");

    let rect_after_2 = compiled.layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    assert_eq!(rect_after_2.rect.width, 140.0);
}

#[test]
fn test_sibling_actor_messaging_drawer_toggle() {
    let input = r#"
    \Component PageLayout {
        let drawer = \Drawer(width: 300);
        \Rect(x: 0, y: 0, width: 80, height: 30, color: #2563eb, on_click: drawer.toggle)
    }

    \Component Drawer(width: Number: 200) {
        state is_open: Boolean: false;
        \Rect(x: 100, y: 0, width: is_open ? width : 0, height: 200, color: #1e293b)
    }

    \PageLayout
    "#;

    let doc = parse(input).expect("parse ok");
    let mut registry = ComponentRegistry::new();
    registry.register("Drawer", || Box::new(DrawerComponent::default()));

    let mut compiled = compile_document_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    // Drawer rect initially has width 0 (is_open: false)
    let drawer_rect = compiled.layout.nodes.iter().find(|n| n.rect.x == 100.0).unwrap();
    assert_eq!(drawer_rect.rect.width, 0.0);

    // Find the toggle button rect ID
    let button_rect_id = compiled.layout.nodes.iter().find(|n| n.rect.x == 0.0).unwrap().id;
    let mut click_event = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(10.0, 10.0),
        Point::new(10.0, 10.0),
        Modifiers::default(),
        button_rect_id,
    );

    // Clicking button dispatches to `drawer.toggle`
    compiled.dispatch_event(&mut click_event).expect("dispatch ok");

    // Drawer is now open -> width should be 300.0!
    let updated_drawer_rect = compiled.layout.nodes.iter().find(|n| n.rect.x == 100.0).unwrap();
    assert_eq!(updated_drawer_rect.rect.width, 300.0);

    // Second click closes drawer -> width back to 0.0
    let mut click_event_close = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(10.0, 10.0),
        Point::new(10.0, 10.0),
        Modifiers::default(),
        button_rect_id,
    );
    compiled.dispatch_event(&mut click_event_close).expect("dispatch ok");

    let closed_drawer_rect = compiled.layout.nodes.iter().find(|n| n.rect.x == 100.0).unwrap();
    assert_eq!(closed_drawer_rect.rect.width, 0.0);
}

#[test]
fn test_structured_key_lookup_from_companion_context() {
    #[derive(Default, Debug)]
    struct GridParent;

    impl Component for GridParent {
        fn on_mount(&mut self, _ctx: &mut Context<'_>) {}

        fn dispatch(
            &mut self,
            _method: &str,
            _event: &Event,
            _ctx: &mut Context<'_>,
        ) -> Result<(), DispatchError> {
            Ok(())
        }
    }

    let input = r#"
    \Component Grid {
        \Cell(0, 0; x: 0, y: 0, width: 50, height: 50)
        \Cell(1, 2; x: 60, y: 0, width: 50, height: 50)
    }

    \Component Cell(x: Number: 0, y: Number: 0, width: Number: 50, height: Number: 50) {
        \Rect(x: x, y: y, width: width, height: height, color: #3b82f6)
    }

    \Grid
    "#;

    let doc = parse(input).expect("parse ok");
    let mut registry = ComponentRegistry::new();
    registry.register("Grid", || Box::new(GridParent));

    let compiled = compile_document_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let grid_node = compiled.layout.nodes.iter().find(|n| n.name == "Grid").unwrap();
    let cell_1_2 = compiled
        .layout
        .find_by_key(
            Some(grid_node.id),
            &ComponentKey::tuple(&[
                directedtype::ast::Expr::number(1.0),
                directedtype::ast::Expr::number(2.0),
            ]),
        )
        .expect("Cell (1, 2) should exist");

    // Verify instance is attached
    assert!(compiled.instances().contains(grid_node.id));
    assert_eq!(cell_1_2.name, "Cell");
}

#[test]
fn test_dom_integration_register_and_dispatch() {
    let input = r#"
    \Component Counter {
        state count: Number: 0;
        \Rect(x: 20, y: 20, width: 100 + count * 50, height: 30, color: #ff0000, on_click: self.increment)
    }

    \Counter
    "#;

    let mut dom = Dom::from_source(input).expect("from_source ok");
    dom.register_component("Counter", || Box::new(CounterComponent::default()));

    // Commit layout
    let layout = dom.commit().expect("commit ok");
    let rect_node = layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    assert_eq!(rect_node.rect.width, 100.0);
    let rect_id = rect_node.id;

    // Dispatch click via DOM
    let mut click_event = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(25.0, 25.0),
        Point::new(5.0, 5.0),
        Modifiers::default(),
        rect_id,
    );

    let changed = dom.dispatch_event(&mut click_event).expect("dispatch ok");
    assert!(!changed.is_empty());

    // DOM layout should reflect width = 150.0 immediately
    let updated_layout = dom.layout().unwrap();
    let updated_rect = updated_layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    assert_eq!(updated_rect.rect.width, 150.0);
}

#[test]
fn test_event_propagation_stopping() {
    #[derive(Default, Debug)]
    struct BubbleTestComponent {
        pub inner_clicked: bool,
        pub outer_clicked: bool,
    }

    impl Component for BubbleTestComponent {
        fn dispatch(
            &mut self,
            method: &str,
            _event: &Event,
            _ctx: &mut Context<'_>,
        ) -> Result<(), DispatchError> {
            match method {
                "on_inner" => {
                    self.inner_clicked = true;
                    Ok(())
                }
                "on_outer" => {
                    self.outer_clicked = true;
                    Ok(())
                }
                _ => Ok(()),
            }
        }
    }

    let input = r#"
    \Component Parent {
        \Rect(x: 0, y: 0, width: 200, height: 200, color: #1e293b, on_click: self.on_outer) {
            \Rect(x: 10, y: 10, width: 50, height: 50, color: #3b82f6, on_click: self.on_inner)
        }
    }

    \Parent
    "#;

    let doc = parse(input).expect("parse ok");
    let mut registry = ComponentRegistry::new();
    registry.register("Parent", || Box::new(BubbleTestComponent::default()));

    let mut compiled = compile_document_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let inner_rect = compiled.layout.nodes.iter().find(|n| n.rect.width == 50.0).unwrap();
    let mut click_event = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(15.0, 15.0),
        Point::new(5.0, 5.0),
        Modifiers::default(),
        inner_rect.id,
    );

    // Set stop_propagation on event
    click_event.stop_propagation();

    compiled.dispatch_event(&mut click_event).expect("dispatch ok");
}

#[test]
fn test_dispatch_error_method_not_found() {
    let input = r#"
    \Component Broken {
        \Rect(x: 0, y: 0, width: 100, height: 100, color: #ff0000, on_click: self.nonexistent)
    }

    \Broken
    "#;

    let doc = parse(input).expect("parse ok");
    let mut registry = ComponentRegistry::new();
    registry.register("Broken", || Box::new(CounterComponent::default()));

    let mut compiled = compile_document_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let rect = compiled.layout.nodes.iter().find(|n| n.name == "Rect").unwrap();
    let mut click_event = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(10.0, 10.0),
        Point::new(10.0, 10.0),
        Modifiers::default(),
        rect.id,
    );

    let err = compiled.dispatch_event(&mut click_event).unwrap_err();
    assert_eq!(
        err,
        DispatchError::MethodNotFound {
            component: "Counter".into(),
            method: "nonexistent".into(),
        }
    );
}

#[test]
fn test_examples_counter_dt_and_rs_integration() {
    let dt_source = std::fs::read_to_string("examples/Counter.dt").expect("Counter.dt must exist");
    let doc = parse(&dt_source).expect("Counter.dt must parse cleanly");

    // Match Counter implementation from examples/Counter.rs
    #[derive(Default, Debug)]
    struct ExampleCounter {
        pub count: i32,
    }

    impl Component for ExampleCounter {
        fn on_mount(&mut self, ctx: &mut Context<'_>) {
            if let Some(initial) = ctx.get_port_number("initial") {
                self.count = initial as i32;
            }
        }

        fn dispatch(
            &mut self,
            method: &str,
            _event: &Event,
            ctx: &mut Context<'_>,
        ) -> Result<(), DispatchError> {
            match method {
                "increment" => {
                    if self.count < 10 {
                        self.count += 1;
                        ctx.set_state("count", self.count as f64);
                    }
                    Ok(())
                }
                "decrement" => {
                    if self.count > 0 {
                        self.count -= 1;
                        ctx.set_state("count", self.count as f64);
                    }
                    Ok(())
                }
                "reset" => {
                    self.count = 0;
                    ctx.set_state("count", 0.0);
                    Ok(())
                }
                _ => Err(DispatchError::MethodNotFound {
                    component: "Counter".into(),
                    method: method.into(),
                }),
            }
        }
    }

    let mut registry = ComponentRegistry::new();
    registry.register_companion("Counter", "examples/Counter.rs", || {
        Box::new(ExampleCounter::default())
    });

    let mut compiled = compile_document_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("examples"),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile Counter.dt ok");

    // Initial count is 3
    let counter_node = compiled.layout.nodes.iter().find(|n| n.name == "Counter").unwrap();
    let counter_id = counter_node.id;
    assert_eq!(
        compiled.get_state(counter_id, "count"),
        Some(&directedtype::compiler::Value::Number(3.0))
    );

    // Find the "+" increment button (has on_click: self.increment)
    let inc_btn = compiled
        .layout
        .nodes
        .iter()
        .find(|n| {
            n.event_handlers
                .get("on_click")
                .map(|b| b.method == "increment")
                .unwrap_or(false)
        })
        .expect("increment button found");

    let mut click_inc = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(inc_btn.rect.x + 5.0, inc_btn.rect.y + 5.0),
        Point::new(5.0, 5.0),
        Modifiers::default(),
        inc_btn.id,
    )
    .with_bubble_path(vec![inc_btn.id, counter_id]);

    let changed = compiled.dispatch_event(&mut click_inc).expect("dispatch click ok");
    assert!(!changed.is_empty(), "State mutations must update downstream DAG");
    assert_eq!(
        compiled.get_state(counter_id, "count"),
        Some(&directedtype::compiler::Value::Number(4.0))
    );

    // Find the "-" decrement button (has on_click: self.decrement)
    let dec_btn = compiled
        .layout
        .nodes
        .iter()
        .find(|n| {
            n.event_handlers
                .get("on_click")
                .map(|b| b.method == "decrement")
                .unwrap_or(false)
        })
        .expect("decrement button found");

    let mut click_dec = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(dec_btn.rect.x + 5.0, dec_btn.rect.y + 5.0),
        Point::new(5.0, 5.0),
        Modifiers::default(),
        dec_btn.id,
    )
    .with_bubble_path(vec![dec_btn.id, counter_id]);

    compiled.dispatch_event(&mut click_dec).expect("dispatch click ok");
    assert_eq!(
        compiled.get_state(counter_id, "count"),
        Some(&directedtype::compiler::Value::Number(3.0))
    );

    // Find the "Reset" button (has on_click: self.reset)
    let reset_btn = compiled
        .layout
        .nodes
        .iter()
        .find(|n| {
            n.event_handlers
                .get("on_click")
                .map(|b| b.method == "reset")
                .unwrap_or(false)
        })
        .expect("reset button found");

    let mut click_reset = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(reset_btn.rect.x + 5.0, reset_btn.rect.y + 5.0),
        Point::new(5.0, 5.0),
        Modifiers::default(),
        reset_btn.id,
    )
    .with_bubble_path(vec![reset_btn.id, counter_id]);

    compiled.dispatch_event(&mut click_reset).expect("dispatch click ok");
    assert_eq!(
        compiled.get_state(counter_id, "count"),
        Some(&directedtype::compiler::Value::Number(0.0))
    );

    // Headless render verification
    let mut renderer = directedtype::render::HeadlessRenderer::new().expect("renderer ok");
    let options = directedtype::render::SceneOptions::default();
    let img = renderer
        .render_layout(&compiled.layout, 800, 600, &options)
        .expect("render layout ok");
    assert_eq!(img.width(), 800);
    assert_eq!(img.height(), 600);
}

