use directedtype::ast::ComponentKey;
use directedtype::compiler::compiled::CompiledDocument;
use directedtype::compiler::value::Value;
use directedtype::component::{Component, ComponentRegistry, Context, DispatchError};
use directedtype::interaction::{Event, EventKind, Modifiers, MouseButton, Point};
use directedtype::parse;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[test]
fn test_material_avatar_variants_sizes_and_overloads() {
    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/Avatar.dt"

    \Avatar("av_default"; x: 10px, y: 10px) { H }
    \Avatar("av_label"; label: "N", bg: material_primary.main, x: 60px, y: 10px)
    \Avatar("av_rounded"; variant: AvatarVariant.Rounded, radius: 6px, bg: material_success.main, x: 110px, y: 10px) { R }
    \Avatar("av_square"; variant: AvatarVariant.Square, bg: material_warning.main, x: 160px, y: 10px) { SQ }
    \Avatar("av_small"; size: 24px, bg: material_info.main, x: 210px, y: 10px) { S }
    \Avatar("av_large"; size: 56px, bg: material_error.main, x: 250px, y: 10px) { L }
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

    // 1. Default Circular Avatar (40x40, radius = 20, bg = #bdbdbd)
    let av_default = compiled
        .find_by_key(None, &ComponentKey::string("av_default"))
        .unwrap();
    assert_eq!(av_default.rect.width, 40.0);
    assert_eq!(av_default.rect.height, 40.0);
    let default_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(av_default.id) && n.name == "Rect")
        .unwrap();
    assert_eq!(
        default_rect.properties.get("radius").and_then(|v| v.as_f64()),
        Some(20.0)
    );
    assert_eq!(
        default_rect.properties.get("color"),
        Some(&Value::Color("#bdbdbd".to_string()))
    );

    // 2. Label prop overload with primary bg
    let av_label = compiled
        .find_by_key(None, &ComponentKey::string("av_label"))
        .unwrap();
    assert_eq!(av_label.rect.width, 40.0);
    assert_eq!(av_label.rect.height, 40.0);
    let label_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(av_label.id) && n.name == "Rect")
        .unwrap();
    assert_eq!(
        label_rect.properties.get("color"),
        Some(&Value::Color("#1976d2".to_string()))
    );

    // 3. Rounded variant (radius = 6px) and Square variant (radius = 0px)
    let av_rounded = compiled
        .find_by_key(None, &ComponentKey::string("av_rounded"))
        .unwrap();
    let rounded_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(av_rounded.id) && n.name == "Rect")
        .unwrap();
    assert_eq!(
        rounded_rect.properties.get("radius").and_then(|v| v.as_f64()),
        Some(6.0)
    );

    let av_square = compiled
        .find_by_key(None, &ComponentKey::string("av_square"))
        .unwrap();
    let square_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(av_square.id) && n.name == "Rect")
        .unwrap();
    assert_eq!(
        square_rect.properties.get("radius").and_then(|v| v.as_f64()),
        Some(0.0)
    );

    // 4. Custom sizes (24px -> radius 12px, 56px -> radius 28px)
    let av_small = compiled
        .find_by_key(None, &ComponentKey::string("av_small"))
        .unwrap();
    assert_eq!(av_small.rect.width, 24.0);
    assert_eq!(av_small.rect.height, 24.0);
    let small_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(av_small.id) && n.name == "Rect")
        .unwrap();
    assert_eq!(
        small_rect.properties.get("radius").and_then(|v| v.as_f64()),
        Some(12.0)
    );

    let av_large = compiled
        .find_by_key(None, &ComponentKey::string("av_large"))
        .unwrap();
    assert_eq!(av_large.rect.width, 56.0);
    assert_eq!(av_large.rect.height, 56.0);
    let large_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(av_large.id) && n.name == "Rect")
        .unwrap();
    assert_eq!(
        large_rect.properties.get("radius").and_then(|v| v.as_f64()),
        Some(28.0)
    );
}

#[test]
fn test_material_chip_variants_sizes_colors_and_avatars() {
    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/Chip.dt"

    \Chip("chip_default"; x: 10px, y: 10px) { Default Filled }
    \Chip("chip_outlined"; variant: ChipVariant.Outlined, x: 10px, y: 50px) { Default Outlined }
    \Chip("chip_primary"; color: MaterialColor.Primary, label: "Primary Label", x: 10px, y: 90px)
    \Chip("chip_small_avatar"; size: ChipSize.Small, avatar: "S", color: MaterialColor.Secondary, x: 10px, y: 130px) { Small Avatar }
    \Chip("chip_medium_avatar"; avatar: "M", color: MaterialColor.Primary, x: 10px, y: 170px) { Medium Avatar }
    \Chip("chip_explicit_width"; width: 180px, variant: ChipVariant.Outlined, color: MaterialColor.Success, x: 10px, y: 210px) { Fixed Width }
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

    // 1. Default Filled Chip: height = 32px, radius = 16px, bg = #00000014
    let chip_default = compiled
        .find_by_key(None, &ComponentKey::string("chip_default"))
        .unwrap();
    assert_eq!(chip_default.rect.height, 32.0);
    assert!(chip_default.rect.width > 24.0);
    let default_bg_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(chip_default.id) && n.name == "Rect")
        .unwrap();
    assert_eq!(
        default_bg_rect.properties.get("radius").and_then(|v| v.as_f64()),
        Some(16.0)
    );
    assert_eq!(
        default_bg_rect.properties.get("color"),
        Some(&Value::Color("#00000014".to_string()))
    );
    assert_eq!(
        default_bg_rect.properties.get("border_width").and_then(|v| v.as_f64()),
        Some(0.0)
    );

    // 2. Default Outlined Chip: bg = #00000000, border_width = 1.0, border_color = #bdbdbd
    let chip_outlined = compiled
        .find_by_key(None, &ComponentKey::string("chip_outlined"))
        .unwrap();
    let outlined_bg_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(chip_outlined.id) && n.name == "Rect")
        .unwrap();
    assert_eq!(
        outlined_bg_rect.properties.get("color"),
        Some(&Value::Color("#00000000".to_string()))
    );
    assert_eq!(
        outlined_bg_rect.properties.get("border_width").and_then(|v| v.as_f64()),
        Some(1.0)
    );
    assert_eq!(
        outlined_bg_rect.properties.get("border_color"),
        Some(&Value::Color("#bdbdbd".to_string()))
    );

    // 3. Primary Filled Chip with label prop: bg = #1976d2
    let chip_primary = compiled
        .find_by_key(None, &ComponentKey::string("chip_primary"))
        .unwrap();
    let primary_bg_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(chip_primary.id) && n.name == "Rect")
        .unwrap();
    assert_eq!(
        primary_bg_rect.properties.get("color"),
        Some(&Value::Color("#1976d2".to_string()))
    );

    // 4. Small Chip with Avatar: height = 24px, radius = 12px, avatar = 18x18 at x + 4px
    let chip_small = compiled
        .find_by_key(None, &ComponentKey::string("chip_small_avatar"))
        .unwrap();
    assert_eq!(chip_small.rect.height, 24.0);
    let small_rects: Vec<_> = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(chip_small.id) && n.name == "Rect")
        .collect();
    assert_eq!(small_rects.len(), 2);
    assert_eq!(
        small_rects[0].properties.get("radius").and_then(|v| v.as_f64()),
        Some(12.0)
    );
    assert_eq!(small_rects[1].rect.width, 18.0);
    assert_eq!(small_rects[1].rect.height, 18.0);
    assert_eq!(small_rects[1].rect.x, chip_small.rect.x + 4.0);

    // 5. Medium Chip with Avatar: height = 32px, avatar = 24x24 at x + 5px, dark channel bg (#1565c0)
    let chip_med_av = compiled
        .find_by_key(None, &ComponentKey::string("chip_medium_avatar"))
        .unwrap();
    let med_rects: Vec<_> = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(chip_med_av.id) && n.name == "Rect")
        .collect();
    assert_eq!(med_rects[1].rect.width, 24.0);
    assert_eq!(med_rects[1].rect.height, 24.0);
    assert_eq!(med_rects[1].rect.x, chip_med_av.rect.x + 5.0);
    assert_eq!(
        med_rects[1].properties.get("color"),
        Some(&Value::Color("#1565c0".to_string()))
    );

    // 6. Explicit Width Chip: width = 180px
    let chip_explicit = compiled
        .find_by_key(None, &ComponentKey::string("chip_explicit_width"))
        .unwrap();
    assert_eq!(chip_explicit.rect.width, 180.0);
    assert_eq!(chip_explicit.rect.height, 32.0);
}

#[test]
fn test_material_chip_clickable_vs_static_and_disabled_interactions() {
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
            if method == "on_chip_activate" {
                self.clicks.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/Chip.dt"

    \Component HostApp {
        \Chip("static_chip"; color: MaterialColor.Primary, x: 10px, y: 10px) {
            Static Chip
        }
        \Chip(
            "clickable_chip";
            color: MaterialColor.Primary,
            x: 10px,
            y: 60px,
            on_click: on_chip_activate
        ) {
            Interactive Chip
        }
        \Chip(
            "disabled_chip";
            color: MaterialColor.Primary,
            disabled: true,
            x: 10px,
            y: 110px,
            on_click: on_chip_activate
        ) {
            Disabled Chip
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

    // 1. Static chip (clickable == false, no on_click) does NOT darken on hover
    let static_comp = compiled
        .find_by_key(None, &ComponentKey::string("static_chip"))
        .unwrap();
    let static_rect_id = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(static_comp.id) && n.name == "Rect")
        .unwrap()
        .id;

    let mut enter_static = Event::new(
        EventKind::PointerEnter,
        Point::new(20.0, 20.0),
        Point::new(10.0, 10.0),
        Modifiers::default(),
        static_rect_id,
    );
    compiled.dispatch_event(&mut enter_static).unwrap();
    let static_rect = compiled.layout.get_node(static_rect_id).unwrap();
    assert_eq!(
        static_rect.properties.get("color"),
        Some(&Value::Color("#1976d2".to_string())),
        "Non-clickable chip should remain at primary.main (#1976d2) when hovered"
    );

    // 2. Clickable chip (has on_click) darkens to primary.dark (#1565c0) on hover and emits on_click
    let clickable_comp_id = compiled
        .find_by_key(None, &ComponentKey::string("clickable_chip"))
        .unwrap()
        .id;
    let clickable_rect_id = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(clickable_comp_id) && n.name == "Rect")
        .unwrap()
        .id;

    let mut enter_clickable = Event::new(
        EventKind::PointerEnter,
        Point::new(20.0, 70.0),
        Point::new(10.0, 10.0),
        Modifiers::default(),
        clickable_rect_id,
    );
    compiled.dispatch_event(&mut enter_clickable).unwrap();
    let clickable_rect = compiled.layout.get_node(clickable_rect_id).unwrap();
    assert_eq!(
        clickable_rect.properties.get("color"),
        Some(&Value::Color("#1565c0".to_string())),
        "Clickable chip should transition to primary.dark (#1565c0) on hover"
    );

    let mut click_ev = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(20.0, 70.0),
        Point::new(10.0, 10.0),
        Modifiers::default(),
        clickable_rect_id,
    );
    compiled.dispatch_event(&mut click_ev).unwrap();
    assert_eq!(clicks.load(Ordering::SeqCst), 1);

    // 3. Disabled chip suppresses callback
    let disabled_comp = compiled
        .find_by_key(None, &ComponentKey::string("disabled_chip"))
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
        Point::new(20.0, 120.0),
        Point::new(10.0, 10.0),
        Modifiers::default(),
        disabled_rect_id,
    );
    compiled.dispatch_event(&mut click_disabled).unwrap();
    assert_eq!(clicks.load(Ordering::SeqCst), 1);
}

#[test]
fn test_material_gallery_avatars_and_chips_section_compiles() {
    let gallery_src = std::fs::read_to_string("examples/material_gallery.dt").unwrap();
    let doc = parse(&gallery_src).expect("gallery parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        1024.0,
        900.0,
        Path::new("examples"),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("gallery compile ok");

    for key in [
        "card_avatars_and_chips",
        "av_default",
        "av_primary",
        "av_rounded",
        "av_square",
        "chip_filled",
        "chip_outlined",
        "chip_avatar_filled",
        "chip_c_primary",
        "chip_o_primary",
        "chip_sm_filled",
    ] {
        let node = compiled
            .find_by_key(None, &ComponentKey::string(key))
            .unwrap_or_else(|| panic!("expected gallery key '{}' to exist", key));
        assert!(node.rect.width > 0.0);
        assert!(node.rect.height > 0.0);
    }
}

#[test]
fn test_component_level_focus_and_click_on_buttons_and_chips() {
    use directedtype::render::{ViewerApp, ViewerConfig};
    use winit::event::ElementState;

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
            if method == "on_activate" {
                self.clicks.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/Button.dt"
    \use "components/material/Chip.dt"

    \Component HostApp {
        \Button("btn_enabled"; x: 20px, y: 20px, on_click: on_activate) { Enabled Button }
        \Button("btn_disabled"; x: 20px, y: 70px, disabled: true, on_click: on_activate) { Disabled Button }
        \Button("btn_unfocused"; x: 20px, y: 120px, on_focus: unset, on_click: on_activate) { Unfocused Button }
        \Chip("chip_static"; x: 20px, y: 170px) { Static Chip }
        \Chip("chip_clickable"; x: 20px, y: 210px, clickable: true) { Clickable Chip }
        \Chip("chip_callback"; x: 20px, y: 250px, on_click: on_activate) { Callback Chip }
        \Chip("chip_disabled"; x: 20px, y: 290px, clickable: true, disabled: true) { Disabled Chip }
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

    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let btn_enabled_id = compiled
        .find_by_key(None, &ComponentKey::string("btn_enabled"))
        .unwrap()
        .id;
    let btn_unfocused_id = compiled
        .find_by_key(None, &ComponentKey::string("btn_unfocused"))
        .unwrap()
        .id;
    let chip_clickable_id = compiled
        .find_by_key(None, &ComponentKey::string("chip_clickable"))
        .unwrap()
        .id;
    let chip_callback_id = compiled
        .find_by_key(None, &ComponentKey::string("chip_callback"))
        .unwrap()
        .id;

    let mut viewer = ViewerApp::new_with_compiled(compiled, ViewerConfig::default());

    // 1. Focusable nodes include only nodes with an active `on_focus` handler
    let focusable = viewer.focusable_nodes();
    assert_eq!(
        focusable,
        vec![btn_enabled_id, chip_clickable_id, chip_callback_id],
        "Only enabled Button and interactive Chips should be focusable; disabled, static, and on_focus: unset nodes must be excluded"
    );

    // 2. Clicking on the center of `btn_enabled` (which hits the sibling `\Text` node)
    // focuses `btn_enabled` and invokes `HostApp::on_activate`.
    let btn_rect = viewer.layout().get_node(btn_enabled_id).unwrap().rect;
    let btn_center = Point::new(
        btn_rect.x + btn_rect.width / 2.0,
        btn_rect.y + btn_rect.height / 2.0,
    );
    let hit = viewer.layout().hit_test(btn_center).unwrap();
    assert_eq!(
        viewer.layout().get_node(hit.target).unwrap().name,
        "Text",
        "Center hit-test on Button should hit its sibling Text node"
    );

    viewer.dispatch_mouse_input(ElementState::Pressed, MouseButton::Left, btn_center);
    viewer.dispatch_mouse_input(ElementState::Released, MouseButton::Left, btn_center);
    assert_eq!(viewer.focused_node(), Some(btn_enabled_id));
    assert_eq!(clicks.load(Ordering::SeqCst), 1);

    // 3. Clicking on `btn_unfocused` (`on_focus: unset`) still invokes `on_click`, but does not focus it
    let unfocused_rect = viewer.layout().get_node(btn_unfocused_id).unwrap().rect;
    let unfocused_center = Point::new(
        unfocused_rect.x + unfocused_rect.width / 2.0,
        unfocused_rect.y + unfocused_rect.height / 2.0,
    );
    viewer.dispatch_mouse_input(ElementState::Pressed, MouseButton::Left, unfocused_center);
    viewer.dispatch_mouse_input(ElementState::Released, MouseButton::Left, unfocused_center);
    assert_eq!(viewer.focused_node(), None);
    assert_eq!(clicks.load(Ordering::SeqCst), 2);

    // 4. Clicking on the center of `chip_callback` (hitting its sibling `\Text` node)
    // focuses `chip_callback` and invokes `HostApp::on_activate`.
    let chip_rect = viewer.layout().get_node(chip_callback_id).unwrap().rect;
    let chip_center = Point::new(
        chip_rect.x + chip_rect.width / 2.0,
        chip_rect.y + chip_rect.height / 2.0,
    );
    viewer.dispatch_mouse_input(ElementState::Pressed, MouseButton::Left, chip_center);
    viewer.dispatch_mouse_input(ElementState::Released, MouseButton::Left, chip_center);
    assert_eq!(viewer.focused_node(), Some(chip_callback_id));
    assert_eq!(clicks.load(Ordering::SeqCst), 3);
}

