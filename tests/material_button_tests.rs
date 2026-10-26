use directedtype::ast::ComponentKey;
use directedtype::compiler::compiled::CompiledDocument;
use directedtype::compiler::value::Value;
use directedtype::component::{Component, ComponentRegistry, Context, DispatchError};
use directedtype::interaction::{Event, EventKind, Modifiers, MouseButton, Point};
use directedtype::parse;
use std::path::Path;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[test]
fn test_material_button_variants_colors_and_sizes() {
    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/Button.dt"

    // Default variant is Text, Medium (36px), Primary (#1976d2)
    \Button("default_text"; label: "Text", x: 10px, y: 10px)

    // Contained Primary (#1976d2 bg, #ffffff text)
    \Button("contained_primary"; label: "Contained", variant: ButtonVariant.Contained, x: 10px, y: 60px)

    // Outlined Secondary (#9c27b0 text, 1px border)
    \Button("outlined_sec"; label: "Outlined", variant: ButtonVariant.Outlined, color: MaterialColor.Secondary, x: 10px, y: 110px)

    // Sizes: Small (30px) and Large (42px)
    \Button("small_btn"; label: "Small", size: ButtonSize.Small, variant: ButtonVariant.Contained, color: MaterialColor.Success, x: 10px, y: 160px)
    \Button("large_btn"; label: "Large", size: ButtonSize.Large, variant: ButtonVariant.Contained, color: MaterialColor.Error, x: 10px, y: 210px)

    // Disabled Contained
    \Button("disabled_btn"; label: "Disabled", variant: ButtonVariant.Contained, disabled: true, x: 10px, y: 270px)
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

    // 1. Default Text Button
    let text_comp = compiled
        .find_by_key(None, &ComponentKey::string("default_text"))
        .unwrap();
    let text_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(text_comp.id) && n.name == "Rect")
        .unwrap();
    assert_eq!(text_rect.rect.height, 36.0);
    assert_eq!(
        text_rect.properties.get("color"),
        Some(&Value::Color("#00000000".to_string()))
    );

    // 2. Contained Primary Button
    let contained_comp = compiled
        .find_by_key(None, &ComponentKey::string("contained_primary"))
        .unwrap();
    let contained_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(contained_comp.id) && n.name == "Rect")
        .unwrap();
    assert_eq!(contained_rect.rect.height, 36.0);
    assert_eq!(
        contained_rect.properties.get("color"),
        Some(&Value::Color("#1976d2".to_string()))
    );
    assert_eq!(
        contained_rect.properties.get("radius"),
        Some(&Value::Distance(4.0))
    );

    // 3. Outlined Secondary Button
    let outlined_comp = compiled
        .find_by_key(None, &ComponentKey::string("outlined_sec"))
        .unwrap();
    let outlined_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(outlined_comp.id) && n.name == "Rect")
        .unwrap();
    assert_eq!(
        outlined_rect.properties.get("border_width"),
        Some(&Value::Distance(1.0))
    );
    assert_eq!(
        outlined_rect.properties.get("border_color"),
        Some(&Value::Color("#9c27b080".to_string()))
    );

    // 4. Small & Large Sizes
    let small_comp = compiled
        .find_by_key(None, &ComponentKey::string("small_btn"))
        .unwrap();
    let small_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(small_comp.id) && n.name == "Rect")
        .unwrap();
    assert_eq!(small_rect.rect.height, 30.0);
    assert_eq!(
        small_rect.properties.get("color"),
        Some(&Value::Color("#2e7d32".to_string()))
    );

    let large_comp = compiled
        .find_by_key(None, &ComponentKey::string("large_btn"))
        .unwrap();
    let large_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(large_comp.id) && n.name == "Rect")
        .unwrap();
    assert_eq!(large_rect.rect.height, 42.0);
    assert_eq!(
        large_rect.properties.get("color"),
        Some(&Value::Color("#d32f2f".to_string()))
    );

    // 5. Disabled Contained Button
    let disabled_comp = compiled
        .find_by_key(None, &ComponentKey::string("disabled_btn"))
        .unwrap();
    let disabled_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(disabled_comp.id) && n.name == "Rect")
        .unwrap();
    assert_eq!(
        disabled_rect.properties.get("color"),
        Some(&Value::Color("#0000001f".to_string()))
    );
}

#[test]
fn test_material_button_children_overloads() {
    let input = r#"
    \use "components/material/Button.dt"

    \Button("child_intrinsic"; variant: ButtonVariant.Contained, x: 10px, y: 10px) {
        Upload File
    }

    \Button("child_fixed"; width: 200px, variant: ButtonVariant.Outlined, x: 10px, y: 60px) {
        Fixed Width Child
    }
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

    let intrinsic_comp = compiled
        .find_by_key(None, &ComponentKey::string("child_intrinsic"))
        .unwrap();
    let intrinsic_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(intrinsic_comp.id) && n.name == "Rect")
        .unwrap();
    assert!(intrinsic_rect.rect.width >= 64.0);
    assert_eq!(intrinsic_rect.rect.height, 36.0);

    let fixed_comp = compiled
        .find_by_key(None, &ComponentKey::string("child_fixed"))
        .unwrap();
    let fixed_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(fixed_comp.id) && n.name == "Rect")
        .unwrap();
    assert_eq!(fixed_rect.rect.width, 200.0);
    assert_eq!(fixed_rect.rect.height, 36.0);
}

#[test]
fn test_material_button_hover_focus_and_click_callback() {
    #[derive(Clone)]
    struct HostApp {
        clicks: Arc<AtomicUsize>,
    }

    impl Component for HostApp {
        fn dispatch(
            &mut self,
            method: &str,
            _event: &mut Event,
            _ctx: &mut Context<'_>,
        ) -> Result<(), DispatchError> {
            if method == "handle_save" {
                self.clicks.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/Button.dt"

    \Component HostApp {
        \Button(
            "save_btn";
            variant: ButtonVariant.Contained,
            color: MaterialColor.Primary,
            x: 20px,
            y: 20px,
            on_click: handle_save
        ) {
            Save Changes
        }
        \Button(
            "disabled_btn";
            variant: ButtonVariant.Contained,
            color: MaterialColor.Primary,
            disabled: true,
            x: 20px,
            y: 80px,
            on_click: handle_save
        ) {
            Disabled Save
        }
    }

    \HostApp("app";)
    "#;

    let clicks = Arc::new(AtomicUsize::new(0));
    let clicks_clone = Arc::clone(&clicks);

    let doc = parse(input).expect("parse ok");
    let mut registry = ComponentRegistry::standard();
    registry.register("HostApp", move || {
        Box::new(HostApp {
            clicks: Arc::clone(&clicks_clone),
        })
    });

    let mut compiled = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let save_comp = compiled
        .find_by_key(None, &ComponentKey::string("save_btn"))
        .unwrap();
    let save_id = save_comp.id;
    let save_rect_id = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(save_id) && n.name == "Rect")
        .unwrap()
        .id;

    // Initial background is Primary main (#1976d2)
    let rect_before = compiled.layout.get_node(save_rect_id).unwrap();
    assert_eq!(
        rect_before.properties.get("color"),
        Some(&Value::Color("#1976d2".to_string()))
    );

    // Hover -> changes Contained background to Primary dark (#1565c0)
    let mut enter_ev = Event::new(
        EventKind::PointerEnter,
        Point::new(30.0, 30.0),
        Point::new(10.0, 10.0),
        Modifiers::default(),
        save_rect_id,
    );
    compiled.dispatch_event(&mut enter_ev).expect("enter ok");

    let rect_hovered = compiled.layout.get_node(save_rect_id).unwrap();
    assert_eq!(
        rect_hovered.properties.get("color"),
        Some(&Value::Color("#1565c0".to_string()))
    );

    // Leave -> restores Primary main (#1976d2)
    let mut leave_ev = Event::new(
        EventKind::PointerLeave,
        Point::new(30.0, 30.0),
        Point::new(10.0, 10.0),
        Modifiers::default(),
        save_rect_id,
    );
    compiled.dispatch_event(&mut leave_ev).expect("leave ok");

    let rect_after_leave = compiled.layout.get_node(save_rect_id).unwrap();
    assert_eq!(
        rect_after_leave.properties.get("color"),
        Some(&Value::Color("#1976d2".to_string()))
    );

    // Click enabled button -> invokes HostApp::handle_save callback
    let mut click_ev = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(30.0, 30.0),
        Point::new(10.0, 10.0),
        Modifiers::default(),
        save_rect_id,
    );
    compiled.dispatch_event(&mut click_ev).expect("click ok");
    assert_eq!(clicks.load(Ordering::SeqCst), 1);

    // Click disabled button -> callback is suppressed
    let disabled_comp = compiled
        .find_by_key(None, &ComponentKey::string("disabled_btn"))
        .unwrap();
    let disabled_rect_id = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(disabled_comp.id) && n.name == "Rect")
        .unwrap()
        .id;

    let mut click_disabled = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(30.0, 90.0),
        Point::new(10.0, 10.0),
        Modifiers::default(),
        disabled_rect_id,
    );
    compiled
        .dispatch_event(&mut click_disabled)
        .expect("click disabled ok");
    assert_eq!(clicks.load(Ordering::SeqCst), 1);
}

#[test]
fn test_material_gallery_cards_contain_contents_when_narrow() {
    let source = std::fs::read_to_string("examples/material_gallery.dt")
        .expect("read examples/material_gallery.dt");
    let doc = parse(&source).expect("parse material_gallery.dt");
    let registry = ComponentRegistry::standard();

    // Compile at a very narrow window width (400px)
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        400.0,
        600.0,
        Path::new("examples"),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile material_gallery.dt at 400px width");

    let card_keys = [
        "card_basic_variants",
        "card_contained_colors",
        "card_outlined_text_colors",
        "card_sizes",
        "card_widths",
        "card_paper_elevations",
        "card_paper_variants",
    ];

    for key in card_keys {
        let card_node = compiled
            .find_by_key(None, &ComponentKey::string(key))
            .unwrap_or_else(|| panic!("Paper '{}' found", key));
        let card_layout = compiled.layout.get_node(card_node.id).unwrap();
        let card_right = card_layout.rect.x + card_layout.rect.width;

        // Every direct child of the Paper must fit inside its horizontal padding (20px)
        for child_id in &card_node.children {
            let child_layout = compiled.layout.get_node(*child_id).unwrap();
            if child_layout.name == "Rect" || child_layout.name == "Shadow" {
                continue;
            }
            let child_right = child_layout.rect.x + child_layout.rect.width;
            assert!(
                card_right >= child_right + 19.5,
                "Paper '{}' right edge ({}) must contain child '{}' right edge ({}) + 20px padding",
                key,
                card_right,
                child_layout.name,
                child_right
            );
        }
    }

    // Content width must exceed the 400px window width so horizontal scrollbar is shown
    let content_w = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.clip.is_none())
        .fold(0.0f64, |acc, n| acc.max(n.rect.x + n.rect.width));
    assert!(
        content_w > 400.0,
        "Content width ({}) should exceed narrow window width (400.0) to trigger horizontal scrollbar",
        content_w
    );
}

#[test]
fn test_material_paper_variants_and_shadows() {
    let input = r#"
    \use "components/VStack.dt"
    \use "components/material/Paper.dt"

    // 1. Width pulled up from single child
    \Paper("pulled_up"; elevation: 4, padding_x: 16px, padding_y: 12px) {
        \VStack(gap: 8px) {
            \Rect(width: 120px, height: 30px, color: #1976d2)
            \Rect(width: 180px, height: 40px, color: #9c27b0)
        }
    }

    // 2. Width pushed down to single child
    \Paper("pushed_down"; width: 300px, y: 120px, variant: PaperVariant.Outlined, square: true, padding_x: 20px, padding_y: 10px) {
        \VStack(gap: 10px) {
            \Rect(width: parent.width, height: 50px, color: #2e7d32)
        }
    }
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
    .expect("compile Paper ok");

    // Verify pulled-up Paper dimensions: width = 180 + 2*16 = 212, height = (30 + 40 + 8) + 2*12 = 102
    let pulled_up = compiled
        .find_by_key(None, &ComponentKey::string("pulled_up"))
        .unwrap();
    let pulled_up_layout = compiled.layout.get_node(pulled_up.id).unwrap();
    assert_eq!(pulled_up_layout.rect.width, 212.0);
    assert_eq!(pulled_up_layout.rect.height, 102.0);

    let pulled_up_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(pulled_up.id) && n.name == "Rect")
        .unwrap();
    assert_eq!(
        pulled_up_rect.properties.get("radius").and_then(|v| v.as_f64()),
        Some(4.0)
    );
    assert_eq!(
        pulled_up_rect.properties.get("border_width").and_then(|v| v.as_f64()),
        Some(0.0)
    );
    assert!(
        pulled_up_rect
            .properties
            .get("shadow_blur")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            > 0.0
    );

    // Verify pushed-down Outlined + Square Paper: width = 300, inner VStack width = 300 - 2*20 = 260
    let pushed_down = compiled
        .find_by_key(None, &ComponentKey::string("pushed_down"))
        .unwrap();
    let pushed_down_layout = compiled.layout.get_node(pushed_down.id).unwrap();
    assert_eq!(pushed_down_layout.rect.width, 300.0);
    assert_eq!(pushed_down_layout.rect.height, 70.0);

    let pushed_down_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(pushed_down.id) && n.name == "Rect")
        .unwrap();
    assert_eq!(
        pushed_down_rect.properties.get("radius").and_then(|v| v.as_f64()),
        Some(0.0)
    );
    assert_eq!(
        pushed_down_rect.properties.get("border_width").and_then(|v| v.as_f64()),
        Some(1.0)
    );
    assert_eq!(
        pushed_down_rect.properties.get("shadow_blur").and_then(|v| v.as_f64()),
        Some(0.0)
    );

    // Verify scene builds cleanly with blurred rounded rect shadows
    let mut font_cx = parley::FontContext::new();
    let mut layout_cx = parley::LayoutContext::new();
    let _scene = directedtype::render::build_scene_without_cache(
        &compiled.layout,
        &mut font_cx,
        &mut layout_cx,
        &directedtype::render::SceneOptions::default(),
    );
}


