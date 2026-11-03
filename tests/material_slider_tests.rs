use directedtype::ast::ComponentKey;
use directedtype::compiler::compiled::CompiledDocument;
use directedtype::compiler::value::Value;
use directedtype::component::{ComponentRegistry, Context, DispatchError};
use directedtype::interaction::{Event, EventKind, Modifiers, MouseButton, Point};
use directedtype::parse;
use directedtype::render::{ViewerApp, ViewerConfig};
use directedtype_macros::component;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use winit::event::ElementState;

#[test]
fn test_material_slider_sizes_colors_track_modes_and_disabled() {
    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/Slider.dt"

    \Slider("sl_md_primary"; x: 20px, y: 20px, width: 200px, value: 50)
    \Slider("sl_sm_secondary"; x: 20px, y: 70px, width: 200px, size: SliderSize.Small, value: 25, color: MaterialColor.Secondary)
    \Slider("sl_success"; x: 20px, y: 110px, width: 200px, value: 75, color: MaterialColor.Success)
    \Slider("sl_error_inverted"; x: 20px, y: 160px, width: 200px, value: 40, color: MaterialColor.Error, track: SliderTrack.Inverted)
    \Slider("sl_hidden_track"; x: 20px, y: 210px, width: 200px, value: 60, track: SliderTrack.Hidden)
    \Slider("sl_disabled"; x: 20px, y: 260px, width: 200px, value: 50, disabled: true)
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

    // 1. Medium Primary Slider (200x30, rail 4px, thumb 20x20 at 50% -> x = 20 + 0.5*(200-20) = 110)
    let sl_md = compiled
        .find_by_key(None, &ComponentKey::string("sl_md_primary"))
        .unwrap();
    assert_eq!(sl_md.rect.width, 200.0);
    assert_eq!(sl_md.rect.height, 30.0);

    let md_rects: Vec<_> = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(sl_md.id) && n.name == "Rect")
        .collect();
    // Direct Rect children: [0] rail, [1] active_track, [2] halo, [3] thumb, [4] hit_area
    assert_eq!(md_rects.len(), 5);
    // Rail: 200x4, #1976d261
    assert_eq!(md_rects[0].rect.width, 200.0);
    assert_eq!(md_rects[0].rect.height, 4.0);
    assert_eq!(
        md_rects[0].properties.get("color"),
        Some(&Value::Color("#1976d261".to_string()))
    );
    // Active track at 50%: width = 100, #1976d2
    assert_eq!(md_rects[1].rect.x, 20.0);
    assert_eq!(md_rects[1].rect.width, 100.0);
    assert_eq!(md_rects[1].rect.height, 4.0);
    assert_eq!(
        md_rects[1].properties.get("color"),
        Some(&Value::Color("#1976d2".to_string()))
    );
    // Halo: 36x36, transparent when idle
    assert_eq!(md_rects[2].rect.width, 36.0);
    assert_eq!(md_rects[2].rect.height, 36.0);
    assert_eq!(
        md_rects[2].properties.get("color"),
        Some(&Value::Color("#00000000".to_string()))
    );
    // Thumb: 20x20 at x = 20 + 0.5*(200 - 20) = 110, y = 20 + (30 - 20)/2 = 25
    assert_eq!(md_rects[3].rect.width, 20.0);
    assert_eq!(md_rects[3].rect.height, 20.0);
    assert_eq!(md_rects[3].rect.x, 110.0);
    assert_eq!(md_rects[3].rect.y, 25.0);
    assert_eq!(
        md_rects[3].properties.get("color"),
        Some(&Value::Color("#1976d2".to_string()))
    );

    // 2. Small Secondary Slider (200x24, rail 2px, thumb 12x12, #9c27b0)
    let sl_sm = compiled
        .find_by_key(None, &ComponentKey::string("sl_sm_secondary"))
        .unwrap();
    assert_eq!(sl_sm.rect.width, 200.0);
    assert_eq!(sl_sm.rect.height, 24.0);
    let sm_rects: Vec<_> = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(sl_sm.id) && n.name == "Rect")
        .collect();
    assert_eq!(sm_rects[0].rect.height, 2.0);
    assert_eq!(
        sm_rects[0].properties.get("color"),
        Some(&Value::Color("#9c27b061".to_string()))
    );
    assert_eq!(sm_rects[1].rect.width, 50.0);
    assert_eq!(
        sm_rects[1].properties.get("color"),
        Some(&Value::Color("#9c27b0".to_string()))
    );
    assert_eq!(sm_rects[2].rect.width, 28.0);
    assert_eq!(sm_rects[3].rect.width, 12.0);
    assert_eq!(sm_rects[3].rect.height, 12.0);
    assert_eq!(
        sm_rects[3].properties.get("color"),
        Some(&Value::Color("#9c27b0".to_string()))
    );

    // 3. Inverted Track (value = 40 -> active track starts at 20 + 0.4*200 = 100, width = 0.6*200 = 120)
    let sl_inv = compiled
        .find_by_key(None, &ComponentKey::string("sl_error_inverted"))
        .unwrap();
    let inv_rects: Vec<_> = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(sl_inv.id) && n.name == "Rect")
        .collect();
    assert_eq!(inv_rects[1].rect.x, 100.0);
    assert_eq!(inv_rects[1].rect.width, 120.0);
    assert_eq!(
        inv_rects[1].properties.get("color"),
        Some(&Value::Color("#d32f2f".to_string()))
    );

    // 4. Hidden Track (active_track width = 0, color = #00000000)
    let sl_hid = compiled
        .find_by_key(None, &ComponentKey::string("sl_hidden_track"))
        .unwrap();
    let hid_rects: Vec<_> = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(sl_hid.id) && n.name == "Rect")
        .collect();
    assert_eq!(hid_rects[1].rect.width, 0.0);
    assert_eq!(
        hid_rects[1].properties.get("color"),
        Some(&Value::Color("#00000000".to_string()))
    );

    // 5. Disabled Slider (rail #0000001f, active track #bdbdbd, thumb #bdbdbd, not focusable)
    let sl_dis = compiled
        .find_by_key(None, &ComponentKey::string("sl_disabled"))
        .unwrap();
    assert!(!sl_dis.event_handlers.contains_key("on_focus"));
    let dis_rects: Vec<_> = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(sl_dis.id) && n.name == "Rect")
        .collect();
    assert_eq!(
        dis_rects[0].properties.get("color"),
        Some(&Value::Color("#0000001f".to_string()))
    );
    assert_eq!(
        dis_rects[1].properties.get("color"),
        Some(&Value::Color("#bdbdbd".to_string()))
    );
    assert_eq!(
        dis_rects[3].properties.get("color"),
        Some(&Value::Color("#bdbdbd".to_string()))
    );
}

#[test]
fn test_material_slider_halos_drag_capture_and_callback() {
    #[derive(Default)]
    struct SliderHost {
        changes: Arc<AtomicUsize>,
        last_val: Arc<Mutex<f64>>,
    }

    #[component]
    impl SliderHost {
        pub fn on_slider_change(
            &mut self,
            val: f64,
            _ctx: &mut Context<'_>,
        ) -> Result<(), DispatchError> {
            self.changes.fetch_add(1, Ordering::SeqCst);
            *self.last_val.lock().unwrap() = val;
            Ok(())
        }
    }

    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/Slider.dt"

    \Component SliderHost {
        \Slider(
            "sl_interactive";
            x: 50px,
            y: 40px,
            width: 200px,
            value: 20,
            color: MaterialColor.Primary,
            value_label_display: SliderValueLabelDisplay.Auto,
            on_change: on_slider_change
        )
        \Slider(
            "sl_disabled";
            x: 50px,
            y: 120px,
            width: 200px,
            value: 30,
            disabled: true,
            on_change: on_slider_change
        )
    }

    \SliderHost("host";)
    "#;

    let changes = Arc::new(AtomicUsize::new(0));
    let last_val = Arc::new(Mutex::new(20.0));
    let c_clone = Arc::clone(&changes);
    let v_clone = Arc::clone(&last_val);

    let doc = parse(input).expect("parse ok");
    let mut registry = ComponentRegistry::standard();
    registry.register("SliderHost", move || {
        Box::new(SliderHost {
            changes: Arc::clone(&c_clone),
            last_val: Arc::clone(&v_clone),
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

    let mut app = ViewerApp::new_with_compiled(compiled, ViewerConfig::default());

    let sl_id = app
        .compiled()
        .unwrap()
        .find_by_key(None, &ComponentKey::string("sl_interactive"))
        .unwrap()
        .id;

    let sl_rects: Vec<_> = app
        .layout()
        .nodes
        .iter()
        .filter(|n| n.parent == Some(sl_id) && n.name == "Rect")
        .map(|n| n.id)
        .collect();
    let halo_id = sl_rects[2];
    let thumb_id = sl_rects[3];

    let val_label_id = app
        .layout()
        .nodes
        .iter()
        .find(|n| n.parent == Some(sl_id) && n.name == "SliderValueLabel")
        .unwrap()
        .id;

    // Initially idle: halo is transparent (36px), Auto value label is hidden (height 0)
    assert_eq!(
        app.layout().get_node(halo_id).unwrap().properties.get("color"),
        Some(&Value::Color("#00000000".to_string()))
    );
    assert_eq!(app.layout().get_node(halo_id).unwrap().rect.width, 36.0);
    assert_eq!(app.layout().get_node(val_label_id).unwrap().rect.height, 0.0);

    // 1. Move cursor over slider (150, 55) -> hover halo (#1976d20a) and Auto value label visible (24px)
    app.dispatch_cursor_moved(Point::new(150.0, 55.0));
    assert_eq!(
        app.layout().get_node(halo_id).unwrap().properties.get("color"),
        Some(&Value::Color("#1976d20a".to_string()))
    );
    assert_eq!(app.layout().get_node(val_label_id).unwrap().rect.height, 24.0);

    // 2. Press mouse at (150, 55): local_x = 150 - 50 = 100 -> 50% -> value = 50.0
    // Halo switches to active/pressed (#1976d229, size 42x42), pointer capture active
    app.dispatch_mouse_input(ElementState::Pressed, MouseButton::Left, Point::new(150.0, 55.0));
    assert!(app.captured_node().is_some());
    assert_eq!(changes.load(Ordering::SeqCst), 1);
    assert_eq!(*last_val.lock().unwrap(), 50.0);
    assert_eq!(
        app.compiled().unwrap().get_state(sl_id, "current_value"),
        Some(&Value::Float(50.0))
    );
    assert_eq!(
        app.layout().get_node(halo_id).unwrap().properties.get("color"),
        Some(&Value::Color("#1976d229".to_string()))
    );
    assert_eq!(app.layout().get_node(halo_id).unwrap().rect.width, 42.0);
    // Thumb at 50%: x = 50 + 0.5*(200 - 20) = 140.0
    assert_eq!(app.layout().get_node(thumb_id).unwrap().rect.x, 140.0);

    // 3. Drag cursor to (200, 55): local_x = 150 -> 75% -> value = 75.0
    app.dispatch_cursor_moved(Point::new(200.0, 55.0));
    assert_eq!(changes.load(Ordering::SeqCst), 2);
    assert_eq!(*last_val.lock().unwrap(), 75.0);
    assert_eq!(
        app.compiled().unwrap().get_state(sl_id, "current_value"),
        Some(&Value::Float(75.0))
    );
    // Thumb at 75%: x = 50 + 0.75*(200 - 20) = 185.0
    assert_eq!(app.layout().get_node(thumb_id).unwrap().rect.x, 185.0);

    // 4. Release mouse -> pointer capture cleared, pressed = false, focus halo (#1976d21f, 36x36) remains since slider was focused on click
    app.dispatch_mouse_input(ElementState::Released, MouseButton::Left, Point::new(200.0, 55.0));
    assert_eq!(app.captured_node(), None);
    assert_eq!(app.layout().get_node(halo_id).unwrap().rect.width, 36.0);
    assert_eq!(
        app.layout().get_node(halo_id).unwrap().properties.get("color"),
        Some(&Value::Color("#1976d21f".to_string()))
    );

    // 5. Clicking disabled slider does not change value or fire callback
    let dis_id = app
        .compiled()
        .unwrap()
        .find_by_key(None, &ComponentKey::string("sl_disabled"))
        .unwrap()
        .id;
    app.dispatch_mouse_input(ElementState::Pressed, MouseButton::Left, Point::new(150.0, 135.0));
    app.dispatch_mouse_input(ElementState::Released, MouseButton::Left, Point::new(150.0, 135.0));
    assert_eq!(changes.load(Ordering::SeqCst), 2);
    assert_eq!(
        app.compiled().unwrap().get_state(dis_id, "current_value"),
        Some(&Value::Float(30.0))
    );
}

#[test]
fn test_material_slider_discrete_steps_marks_and_custom_marks() {
    #[derive(Default)]
    struct DiscreteHost {
        last_val: Arc<Mutex<f64>>,
    }

    #[component]
    impl DiscreteHost {
        pub fn on_change(&mut self, val: f64, _ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            *self.last_val.lock().unwrap() = val;
            Ok(())
        }
    }

    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/Slider.dt"

    \Component DiscreteHost {
        \Slider(
            "sl_step_marks";
            x: 40px,
            y: 20px,
            width: 200px,
            min: 0,
            max: 100,
            step: 20,
            marks: true,
            value: 40,
            on_change: on_change
        )

        \Slider(
            "sl_custom_marks";
            x: 40px,
            y: 80px,
            width: 200px,
            min: 0,
            max: 100,
            step: 0,
            marks: true,
            value: 20,
            on_change: on_change
        ) {
            \SliderMark("mark_0"; value: 0, label: "0°C")
            \SliderMark("mark_20"; value: 20, label: "20°C")
            \SliderMark("mark_37"; value: 37) { 37°C }
            \SliderMark("mark_100"; value: 100, label: "100°C")
        }
    }

    \DiscreteHost("host";)
    "#;

    let last_val = Arc::new(Mutex::new(0.0));
    let v_clone = Arc::clone(&last_val);

    let doc = parse(input).expect("parse ok");
    let mut registry = ComponentRegistry::standard();
    registry.register("DiscreteHost", move || {
        Box::new(DiscreteHost {
            last_val: Arc::clone(&v_clone),
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

    // 1. Verify automatic discrete step marks (min=0, max=100, step=20 -> 6 visible ticks: 0, 20, 40, 60, 80, 100)
    let sl_step = compiled
        .find_by_key(None, &ComponentKey::string("sl_step_marks"))
        .unwrap();
    let step_marks_container = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(sl_step.id) && n.name == "SliderStepMarks")
        .unwrap();
    let tick_nodes: Vec<_> = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(step_marks_container.id) && n.name == "SliderStepTick")
        .collect();
    let visible_ticks: Vec<_> = tick_nodes
        .iter()
        .filter(|n| n.rect.width > 0.0)
        .collect();
    assert_eq!(visible_ticks.len(), 6);

    // First 3 ticks (0, 20, 40 <= 40) have active mark color #ffffffcc; last 3 (60, 80, 100) have #1976d2
    for (idx, tick) in visible_ticks.iter().enumerate() {
        let tick_rect = compiled
            .layout
            .nodes
            .iter()
            .find(|n| n.parent == Some(tick.id) && n.name == "Rect")
            .unwrap();
        let expected_color = if idx <= 2 { "#ffffffcc" } else { "#1976d2" };
        assert_eq!(
            tick_rect.properties.get("color"),
            Some(&Value::Color(expected_color.to_string()))
        );
    }

    // 2. Dragging/clicking sl_step_marks at local_x = 136 (68% -> raw = 68.0) snaps to nearest step (60.0)!
    let sl_step_id = sl_step.id;
    let mut down_ev = Event::new(
        EventKind::PointerDown {
            button: MouseButton::Left,
        },
        Point::new(40.0 + 136.0, 35.0),
        Point::new(136.0, 15.0),
        Modifiers::default(),
        sl_step_id,
    );
    compiled.dispatch_event(&mut down_ev).unwrap();
    assert_eq!(*last_val.lock().unwrap(), 60.0);
    assert_eq!(
        compiled.get_state(sl_step_id, "current_value"),
        Some(&Value::Float(60.0))
    );

    // 3. Verify custom labeled SliderMarks expand slider height beyond 30px and snap when step=0, marks=true
    let sl_custom = compiled
        .find_by_key(None, &ComponentKey::string("sl_custom_marks"))
        .unwrap();
    assert!(
        sl_custom.rect.height > 30.0,
        "Labeled SliderMarks must expand Slider height to accommodate labels below the rail"
    );
    let sl_custom_id = sl_custom.id;

    // Click near 34% (local_x = 68 -> raw = 34.0) -> snaps to custom mark 37.0!
    let mut down_custom = Event::new(
        EventKind::PointerDown {
            button: MouseButton::Left,
        },
        Point::new(40.0 + 68.0, 95.0),
        Point::new(68.0, 15.0),
        Modifiers::default(),
        sl_custom_id,
    );
    compiled.dispatch_event(&mut down_custom).unwrap();
    assert_eq!(*last_val.lock().unwrap(), 37.0);
    assert_eq!(
        compiled.get_state(sl_custom_id, "current_value"),
        Some(&Value::Float(37.0))
    );
}

#[test]
fn test_material_gallery_with_sliders_compiles_cleanly() {
    let source =
        std::fs::read_to_string("examples/material_gallery.dt").expect("read material_gallery.dt");
    let doc = parse(&source).expect("parse material_gallery.dt");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        1024.0,
        900.0,
        Path::new("examples"),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile material_gallery.dt ok");

    for key in [
        "sl_small",
        "sl_default",
        "sl_disabled",
        "sl_val_on",
        "sl_secondary",
        "sl_success",
        "sl_warning",
        "sl_error",
        "sl_info",
        "sl_discrete",
        "sl_custom_marks",
        "sl_inverted",
    ] {
        let node = compiled
            .find_by_key(None, &ComponentKey::string(key))
            .unwrap_or_else(|| panic!("Expected slider '{}' in material_gallery.dt", key));
        assert!(node.rect.width > 0.0);
        assert!(node.rect.height >= 24.0);
    }
}

