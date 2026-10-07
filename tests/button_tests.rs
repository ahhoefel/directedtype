use directedtype::ast::ComponentKey;
use directedtype::compiler::compiled::CompiledDocument;
use directedtype::compiler::value::Value;
use directedtype::component::{Component, ComponentRegistry, Context, DispatchError};
use directedtype::interaction::{Event, EventKind, Modifiers, MouseButton, Point};
use directedtype::parse;
use std::path::Path;

#[test]
fn test_button_intrinsic_width_calculation() {
    let input = r#"
    \use "components/Button.dt"
    \use "theme/default.dt"

    \Button("auto"; style: button_primary, label: "OK", x: 10, y: 10)
    \Button("wide"; style: button_primary, label: "Submit Application Form", x: 10, y: 60)
    \Button("fixed"; style: button_primary, label: "Fixed", width: 180, x: 10, y: 110)
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    // Retrieve auto button rect
    let auto_node = compiled.find_by_key(None, &ComponentKey::string("auto")).expect("auto button found");
    let auto_rect = compiled.layout.nodes.iter().find(|n| n.parent == Some(auto_node.id) && n.name == "Rect").expect("auto rect");

    let wide_node = compiled.find_by_key(None, &ComponentKey::string("wide")).expect("wide button found");
    let wide_rect = compiled.layout.nodes.iter().find(|n| n.parent == Some(wide_node.id) && n.name == "Rect").expect("wide rect");

    let fixed_node = compiled.find_by_key(None, &ComponentKey::string("fixed")).expect("fixed button found");
    let fixed_rect = compiled.layout.nodes.iter().find(|n| n.parent == Some(fixed_node.id) && n.name == "Rect").expect("fixed rect");

    // "OK" text is short, but minimum width is clamped to 64
    assert!(auto_rect.rect.width >= 64.0);

    // "Submit Application Form" is much wider than "OK"
    assert!(wide_rect.rect.width > auto_rect.rect.width);
    assert!(wide_rect.rect.width > 120.0);

    // Fixed width explicitly overrides intrinsic sizing
    assert_eq!(fixed_rect.rect.width, 180.0);
}

#[test]
fn test_button_variants_visual_ports() {
    let input = r#"
    \use "components/Button.dt"
    \use "theme/default.dt"

    \Button("primary"; style: button_primary, x: 10, y: 10, label: "Primary")
    \Button("outline"; style: button_outline, x: 10, y: 60, label: "Outline")
    \Button("danger"; style: button_danger, x: 10, y: 110, label: "Danger")
    \Button("disabled"; style: button_primary, x: 10, y: 160, label: "Disabled", disabled: true)
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let primary_comp = compiled.find_by_key(None, &ComponentKey::string("primary")).unwrap();
    let primary_rect = compiled.layout.nodes.iter().find(|n| n.parent == Some(primary_comp.id) && n.name == "Rect").unwrap();
    assert_eq!(primary_rect.properties.get("color"), Some(&Value::Color("#2563eb".to_string())));

    let outline_comp = compiled.find_by_key(None, &ComponentKey::string("outline")).unwrap();
    let outline_rect = compiled.layout.nodes.iter().find(|n| n.parent == Some(outline_comp.id) && n.name == "Rect").unwrap();
    assert_eq!(outline_rect.properties.get("color"), Some(&Value::Color("#0f172a".to_string())));
    assert_eq!(outline_rect.properties.get("border_width"), Some(&Value::Number(1.5)));

    let danger_comp = compiled.find_by_key(None, &ComponentKey::string("danger")).unwrap();
    let danger_rect = compiled.layout.nodes.iter().find(|n| n.parent == Some(danger_comp.id) && n.name == "Rect").unwrap();
    assert_eq!(danger_rect.properties.get("color"), Some(&Value::Color("#dc2626".to_string())));

    let disabled_comp = compiled.find_by_key(None, &ComponentKey::string("disabled")).unwrap();
    let disabled_rect = compiled.layout.nodes.iter().find(|n| n.parent == Some(disabled_comp.id) && n.name == "Rect").unwrap();
    assert_eq!(disabled_rect.properties.get("color"), Some(&Value::Color("#33415580".to_string())));
}

#[test]
fn test_disabled_button_suppresses_event_bubbling() {
    #[derive(Default, Debug)]
    struct ParentContainer {
        parent_received_click: bool,
    }

    impl Component for ParentContainer {
        fn dispatch(
            &mut self,
            method: &str,
            _event: &mut Event,
            _ctx: &mut Context<'_>,
        ) -> Result<(), DispatchError> {
            if method == "on_parent_click" {
                self.parent_received_click = true;
            }
            Ok(())
        }
    }

    let input = r#"
    \use "components/Button.dt"
    \use "theme/default.dt"

    \Component Card {
        \Rect(x: 0, y: 0, width: 300, height: 200, color: #1e293b, on_click: self.on_parent_click) {
            \Button("active_btn"; style: button_primary, x: 20, y: 20, width: 100, label: "Active", disabled: false)
            \Button("disabled_btn"; style: button_primary, x: 20, y: 80, width: 100, label: "Disabled", disabled: true)
        }
    }

    \Card("main_card";)
    "#;

    let doc = parse(input).expect("parse ok");
    let mut registry = ComponentRegistry::standard();
    registry.register("Card", || Box::new(ParentContainer::default()));

    let mut compiled = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let disabled_comp = compiled.find_by_key(None, &ComponentKey::string("disabled_btn")).unwrap();
    let disabled_id = disabled_comp.id;
    let disabled_rect_id = compiled.layout.nodes.iter().find(|n| n.parent == Some(disabled_id) && n.name == "Rect").unwrap().id;

    // Click disabled button rect (which bubbles to Card if not stopped)
    let mut click_disabled = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(30.0, 90.0),
        Point::new(10.0, 10.0),
        Modifiers::default(),
        disabled_rect_id,
    );

    compiled.dispatch_event(&mut click_disabled).expect("dispatch ok");

    // Propagation stops by default on the first handler
    assert!(!click_disabled.propagation_continued);

    // Now click the active button rect
    let active_comp = compiled.find_by_key(None, &ComponentKey::string("active_btn")).unwrap();
    let active_id = active_comp.id;
    let active_rect_id = compiled.layout.nodes.iter().find(|n| n.parent == Some(active_id) && n.name == "Rect").unwrap().id;

    let mut click_active = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(30.0, 30.0),
        Point::new(10.0, 10.0),
        Modifiers::default(),
        active_rect_id,
    );

    compiled.dispatch_event(&mut click_active).expect("dispatch ok");

    // Under the new convention, event propagation stops on the first node with a handler by default
    assert!(!click_active.propagation_continued);
}

#[test]
fn test_button_spatial_alias_flow() {
    let input = r#"
    \use "components/Button.dt"
    \use "theme/default.dt"

    \Button("first"; style: button_primary, x: 20, y: 20, label: "First Button")
    \Button("second"; style: button_primary, x: prev.right + 16, y: 20, label: "Second Button")
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let first_comp = compiled.find_by_key(None, &ComponentKey::string("first")).unwrap();
    let first_rect = compiled.layout.nodes.iter().find(|n| n.parent == Some(first_comp.id) && n.name == "Rect").unwrap();

    let second_comp = compiled.find_by_key(None, &ComponentKey::string("second")).unwrap();
    let second_rect = compiled.layout.nodes.iter().find(|n| n.parent == Some(second_comp.id) && n.name == "Rect").unwrap();

    assert_eq!(second_rect.rect.x, first_rect.rect.x + first_rect.rect.width + 16.0);
}
