use directedtype::ast::ComponentKey;
use directedtype::compiler::compiled::CompiledDocument;
use directedtype::compiler::value::Value;
use directedtype::component::{ComponentRegistry, Context, DispatchError};
use directedtype::interaction::{Event, EventKind, Modifiers, MouseButton, Point};
use directedtype::parse;
use directedtype_macros::component;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

#[test]
fn test_material_switch_geometry_sizes_colors_and_overloads() {
    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/Switch.dt"

    \Switch("sw_off"; x: 10px, y: 10px)
    \Switch("sw_on"; checked: true, color: MaterialColor.Primary, x: 80px, y: 10px)
    \Switch("sw_small_off"; size: SwitchSize.Small, x: 150px, y: 10px)
    \Switch("sw_small_on"; size: SwitchSize.Small, checked: true, color: MaterialColor.Secondary, x: 210px, y: 10px)
    \Switch("sw_label_prop"; label: "Wi-Fi", checked: true, x: 10px, y: 60px)
    \Switch("sw_child_text"; checked: true, color: MaterialColor.Success, x: 10px, y: 110px) { Bluetooth }
    \Switch("sw_disabled_on"; checked: true, disabled: true, x: 10px, y: 160px)
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

    // 1. Standalone Medium Switch (58x38) — unchecked vs checked
    let sw_off = compiled
        .find_by_key(None, &ComponentKey::string("sw_off"))
        .unwrap();
    assert_eq!(sw_off.rect.width, 58.0);
    assert_eq!(sw_off.rect.height, 38.0);

    let off_rects: Vec<_> = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(sw_off.id) && n.name == "Rect")
        .collect();
    // Child rects: [0] hit_area (58x38), [1] track (34x14), [2] halo (38x38), [3] thumb (20x20)
    assert_eq!(off_rects.len(), 4);
    assert_eq!(off_rects[1].rect.width, 34.0);
    assert_eq!(off_rects[1].rect.height, 14.0);
    assert_eq!(
        off_rects[1].properties.get("color"),
        Some(&Value::Color("#00000061".to_string()))
    );
    assert_eq!(off_rects[3].rect.x, 10.0 + 9.0);
    assert_eq!(off_rects[3].rect.y, 10.0 + 9.0);
    assert_eq!(off_rects[3].rect.width, 20.0);
    assert_eq!(
        off_rects[3].properties.get("color"),
        Some(&Value::Color("#ffffff".to_string()))
    );

    let sw_on = compiled
        .find_by_key(None, &ComponentKey::string("sw_on"))
        .unwrap();
    let on_rects: Vec<_> = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(sw_on.id) && n.name == "Rect")
        .collect();
    assert_eq!(
        on_rects[1].properties.get("color"),
        Some(&Value::Color("#1976d280".to_string()))
    );
    // Checked thumb shifts +20px horizontally (from x+9 to x+29)
    assert_eq!(on_rects[3].rect.x, 80.0 + 29.0);
    assert_eq!(
        on_rects[3].properties.get("color"),
        Some(&Value::Color("#1976d2".to_string()))
    );

    // 2. Small Switch (40x24) — unchecked vs checked
    let sw_small_off = compiled
        .find_by_key(None, &ComponentKey::string("sw_small_off"))
        .unwrap();
    assert_eq!(sw_small_off.rect.width, 40.0);
    assert_eq!(sw_small_off.rect.height, 24.0);

    let sw_small_on = compiled
        .find_by_key(None, &ComponentKey::string("sw_small_on"))
        .unwrap();
    assert_eq!(sw_small_on.rect.width, 40.0);
    assert_eq!(sw_small_on.rect.height, 24.0);
    let sm_on_rects: Vec<_> = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(sw_small_on.id) && n.name == "Rect")
        .collect();
    // Small checked thumb is 16x16 at x + 20px
    assert_eq!(sm_on_rects[3].rect.width, 16.0);
    assert_eq!(sm_on_rects[3].rect.x, 210.0 + 20.0);
    assert_eq!(
        sm_on_rects[3].properties.get("color"),
        Some(&Value::Color("#9c27b0".to_string()))
    );

    // 3. Labeled overloads (`label: String` and `{ ... }` child text) expand width beyond 58px
    let sw_label_prop = compiled
        .find_by_key(None, &ComponentKey::string("sw_label_prop"))
        .unwrap();
    assert!(sw_label_prop.rect.width > 58.0);

    let sw_child_text = compiled
        .find_by_key(None, &ComponentKey::string("sw_child_text"))
        .unwrap();
    assert!(sw_child_text.rect.width > 58.0);

    // 4. Disabled checked switch uses disabled track (#0000001f) and disabled thumb (#bdbdbd)
    let sw_disabled_on = compiled
        .find_by_key(None, &ComponentKey::string("sw_disabled_on"))
        .unwrap();
    let dis_rects: Vec<_> = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(sw_disabled_on.id) && n.name == "Rect")
        .collect();
    assert_eq!(
        dis_rects[1].properties.get("color"),
        Some(&Value::Color("#0000001f".to_string()))
    );
    assert_eq!(
        dis_rects[3].properties.get("color"),
        Some(&Value::Color("#bdbdbd".to_string()))
    );
}

#[test]
fn test_material_switch_interactive_toggle_halo_and_on_change_callback() {
    #[derive(Default)]
    struct SwitchHost {
        changes: Arc<AtomicUsize>,
        last_val: Arc<AtomicBool>,
    }

    #[component]
    impl SwitchHost {
        pub fn on_switch_change(
            &mut self,
            checked: bool,
            _ctx: &mut Context<'_>,
        ) -> Result<(), DispatchError> {
            self.changes.fetch_add(1, Ordering::SeqCst);
            self.last_val.store(checked, Ordering::SeqCst);
            Ok(())
        }
    }

    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/Switch.dt"

    \Component SwitchHost {
        \Switch("sw_interactive"; x: 10px, y: 10px, on_change: on_switch_change) { Notifications }
        \Switch("sw_disabled"; x: 10px, y: 60px, disabled: true, on_change: on_switch_change)
    }

    \SwitchHost("host";)
    "#;

    let changes = Arc::new(AtomicUsize::new(0));
    let last_val = Arc::new(AtomicBool::new(false));
    let c_clone = Arc::clone(&changes);
    let v_clone = Arc::clone(&last_val);

    let doc = parse(input).expect("parse ok");
    let mut registry = ComponentRegistry::standard();
    registry.register("SwitchHost", move || {
        Box::new(SwitchHost {
            changes: Arc::clone(&c_clone),
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

    let sw_id = compiled
        .find_by_key(None, &ComponentKey::string("sw_interactive"))
        .unwrap()
        .id;
    let sw_rects: Vec<_> = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(sw_id) && n.name == "Rect")
        .map(|n| n.id)
        .collect();
    let hit_rect_id = sw_rects[0];
    let halo_rect_id = sw_rects[2];
    let thumb_rect_id = sw_rects[3];

    // 1. Hover shows unchecked hover halo (#0000000a)
    let mut enter_ev = Event::new(
        EventKind::PointerEnter,
        Point::new(20.0, 20.0),
        Point::new(10.0, 10.0),
        Modifiers::default(),
        hit_rect_id,
    );
    compiled.dispatch_event(&mut enter_ev).unwrap();
    assert_eq!(
        compiled
            .layout
            .get_node(halo_rect_id)
            .unwrap()
            .properties
            .get("color"),
        Some(&Value::Color("#0000000a".to_string()))
    );

    // 2. First click toggles switch ON (thumb animates from x+9 to x+29 over 150ms, halo becomes primary hover_bg #1976d20a, callback receives true)
    let mut click_ev = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(20.0, 20.0),
        Point::new(10.0, 10.0),
        Modifiers::default(),
        hit_rect_id,
    );
    compiled.dispatch_event(&mut click_ev).unwrap();
    assert_eq!(changes.load(Ordering::SeqCst), 1);
    assert!(last_val.load(Ordering::SeqCst));
    assert_eq!(
        compiled.get_state(sw_id, "is_checked"),
        Some(&Value::Bool(true))
    );
    assert!(compiled.has_active_timers());
    // At t = 0ms, thumb is at start of transition (x + 9)
    assert_eq!(
        compiled.layout.get_node(thumb_rect_id).unwrap().rect.x,
        10.0 + 9.0
    );
    // At t = 75ms (midpoint), thumb is halfway (x + 19)
    assert!(!compiled.advance_timers(75_000).unwrap().is_empty());
    assert_eq!(
        compiled.layout.get_node(thumb_rect_id).unwrap().rect.x,
        10.0 + 19.0
    );
    // At t = 150ms (completion), timer unsets and thumb settles at x + 29
    assert!(!compiled.advance_timers(75_000).unwrap().is_empty());
    assert!(!compiled.has_active_timers());
    assert_eq!(compiled.get_state(sw_id, "anim"), Some(&Value::Unset));
    assert_eq!(
        compiled.layout.get_node(thumb_rect_id).unwrap().rect.x,
        10.0 + 29.0
    );
    assert_eq!(
        compiled
            .layout
            .get_node(halo_rect_id)
            .unwrap()
            .properties
            .get("color"),
        Some(&Value::Color("#1976d20a".to_string()))
    );

    // 3. Second click toggles switch OFF (thumb animates from x+29 back to x+9 over 150ms, callback receives false)
    let mut click_ev2 = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(20.0, 20.0),
        Point::new(10.0, 10.0),
        Modifiers::default(),
        hit_rect_id,
    );
    compiled.dispatch_event(&mut click_ev2).unwrap();
    assert_eq!(changes.load(Ordering::SeqCst), 2);
    assert!(!last_val.load(Ordering::SeqCst));
    assert!(compiled.has_active_timers());
    // At t = 0ms, thumb starts at x + 29
    assert_eq!(
        compiled.layout.get_node(thumb_rect_id).unwrap().rect.x,
        10.0 + 29.0
    );
    // At t = 75ms, thumb is halfway back at x + 19
    assert!(!compiled.advance_timers(75_000).unwrap().is_empty());
    assert_eq!(
        compiled.layout.get_node(thumb_rect_id).unwrap().rect.x,
        10.0 + 19.0
    );
    // At t = 150ms, timer unsets and thumb settles at x + 9
    assert!(!compiled.advance_timers(75_000).unwrap().is_empty());
    assert!(!compiled.has_active_timers());
    assert_eq!(
        compiled.layout.get_node(thumb_rect_id).unwrap().rect.x,
        10.0 + 9.0
    );

    // 4. Disabled switch ignores clicks
    let dis_id = compiled
        .find_by_key(None, &ComponentKey::string("sw_disabled"))
        .unwrap()
        .id;
    let dis_hit_id = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(dis_id) && n.name == "Rect")
        .unwrap()
        .id;
    let mut click_dis = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(20.0, 70.0),
        Point::new(10.0, 10.0),
        Modifiers::default(),
        dis_hit_id,
    );
    compiled.dispatch_event(&mut click_dis).unwrap();
    assert_eq!(changes.load(Ordering::SeqCst), 2);
}

#[test]
fn test_material_checkbox_states_indeterminate_and_toggle() {
    #[derive(Default)]
    struct CheckboxHost {
        changes: Arc<AtomicUsize>,
        last_val: Arc<AtomicBool>,
    }

    #[component]
    impl CheckboxHost {
        pub fn on_cb_change(
            &mut self,
            checked: bool,
            _ctx: &mut Context<'_>,
        ) -> Result<(), DispatchError> {
            self.changes.fetch_add(1, Ordering::SeqCst);
            self.last_val.store(checked, Ordering::SeqCst);
            Ok(())
        }
    }

    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/Checkbox.dt"

    \Component CheckboxHost {
        \Checkbox("cb_unchecked"; x: 10px, y: 10px, on_change: on_cb_change)
        \Checkbox("cb_checked"; x: 60px, y: 10px, checked: true, color: MaterialColor.Success)
        \Checkbox("cb_indeterminate"; x: 110px, y: 10px, indeterminate: true, on_change: on_cb_change)
        \Checkbox("cb_small"; x: 160px, y: 10px, size: CheckboxSize.Small, label: "Remember me")
    }

    \CheckboxHost("host";)
    "#;

    let changes = Arc::new(AtomicUsize::new(0));
    let last_val = Arc::new(AtomicBool::new(false));
    let c_clone = Arc::clone(&changes);
    let v_clone = Arc::clone(&last_val);

    let doc = parse(input).expect("parse ok");
    let mut registry = ComponentRegistry::standard();
    registry.register("CheckboxHost", move || {
        Box::new(CheckboxHost {
            changes: Arc::clone(&c_clone),
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

    // 1. Unchecked Medium Checkbox (42x42 outer, 18x18 box at +12,+12 with 2px border)
    let cb_unchecked = compiled
        .find_by_key(None, &ComponentKey::string("cb_unchecked"))
        .unwrap();
    assert_eq!(cb_unchecked.rect.width, 42.0);
    assert_eq!(cb_unchecked.rect.height, 42.0);
    let un_rects: Vec<_> = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(cb_unchecked.id) && n.name == "Rect")
        .collect();
    // [0] hit_area, [1] halo, [2] box_rect, [3] dash_rect
    assert_eq!(un_rects[2].rect.width, 18.0);
    assert_eq!(un_rects[2].rect.x, 10.0 + 12.0);
    assert_eq!(
        un_rects[2].properties.get("color"),
        Some(&Value::Color("#00000000".to_string()))
    );
    assert_eq!(
        un_rects[2].properties.get("border_width").and_then(|v| v.as_f64()),
        Some(2.0)
    );

    // 2. Checked Checkbox has filled box (#2e7d32) and visible checkmark lines (thickness 2px)
    let cb_checked = compiled
        .find_by_key(None, &ComponentKey::string("cb_checked"))
        .unwrap();
    let ch_lines: Vec<_> = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(cb_checked.id) && n.name == "Line")
        .collect();
    assert_eq!(ch_lines.len(), 2);
    assert_eq!(
        ch_lines[0].properties.get("thickness").and_then(|v| v.as_f64()),
        Some(2.0)
    );

    // 3. Indeterminate Checkbox shows dash_rect (#ffffff) and hides checkmark lines (0px)
    let cb_ind_id = compiled
        .find_by_key(None, &ComponentKey::string("cb_indeterminate"))
        .unwrap()
        .id;
    let ind_rects: Vec<_> = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(cb_ind_id) && n.name == "Rect")
        .map(|n| n.id)
        .collect();
    let ind_lines: Vec<_> = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(cb_ind_id) && n.name == "Line")
        .map(|n| n.id)
        .collect();
    assert_eq!(
        compiled
            .layout
            .get_node(ind_rects[3])
            .unwrap()
            .properties
            .get("color"),
        Some(&Value::Color("#ffffff".to_string()))
    );
    assert_eq!(
        compiled
            .layout
            .get_node(ind_lines[0])
            .unwrap()
            .properties
            .get("thickness")
            .and_then(|v| v.as_f64()),
        Some(0.0)
    );

    // Clicking the indeterminate checkbox clears `is_indeterminate` and sets `is_checked = true`
    let mut click_ind = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(125.0, 25.0),
        Point::new(15.0, 15.0),
        Modifiers::default(),
        ind_rects[0],
    );
    compiled.dispatch_event(&mut click_ind).unwrap();
    assert_eq!(changes.load(Ordering::SeqCst), 1);
    assert!(last_val.load(Ordering::SeqCst));
    assert_eq!(
        compiled.get_state(cb_ind_id, "is_indeterminate"),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        compiled.get_state(cb_ind_id, "is_checked"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        compiled
            .layout
            .get_node(ind_lines[0])
            .unwrap()
            .properties
            .get("thickness")
            .and_then(|v| v.as_f64()),
        Some(2.0)
    );

    // 4. Small labeled Checkbox has height >= 34px and width > 34px
    let cb_small = compiled
        .find_by_key(None, &ComponentKey::string("cb_small"))
        .unwrap();
    assert!(cb_small.rect.width > 34.0);
    assert_eq!(cb_small.rect.height, 34.0);
}

#[test]
fn test_material_radio_standalone_and_controlled_group() {
    #[derive(Default)]
    struct RadioGroupHost {
        selected: i64,
    }

    #[component]
    impl RadioGroupHost {
        pub fn select_1(&mut self, _checked: bool, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            self.selected = 1;
            ctx.set_state("selected", 1i64);
            Ok(())
        }

        pub fn select_2(&mut self, _checked: bool, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            self.selected = 2;
            ctx.set_state("selected", 2i64);
            Ok(())
        }
    }

    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/Radio.dt"

    \Component RadioGroupHost {
        \state selected: Integer = 1;

        \Radio("r1"; x: 10px, y: 10px, controlled: true, checked: selected == 1, on_change: select_1) { Option 1 }
        \Radio("r2"; x: 10px, y: 60px, controlled: true, checked: selected == 2, on_change: select_2) { Option 2 }
        \Radio("r_standalone"; x: 10px, y: 110px, size: RadioSize.Small)
    }

    \RadioGroupHost("group";)
    "#;

    let doc = parse(input).expect("parse ok");
    let mut registry = ComponentRegistry::standard();
    registry.register("RadioGroupHost", || Box::new(RadioGroupHost { selected: 1 }));

    let mut compiled = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let r1_id = compiled
        .find_by_key(None, &ComponentKey::string("r1"))
        .unwrap()
        .id;
    let r2_id = compiled
        .find_by_key(None, &ComponentKey::string("r2"))
        .unwrap()
        .id;
    let r_standalone = compiled
        .find_by_key(None, &ComponentKey::string("r_standalone"))
        .unwrap();
    assert_eq!(r_standalone.rect.width, 34.0);
    assert_eq!(r_standalone.rect.height, 34.0);

    let r1_rects: Vec<_> = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(r1_id) && n.name == "Rect")
        .map(|n| n.id)
        .collect();
    let r2_rects: Vec<_> = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(r2_id) && n.name == "Rect")
        .map(|n| n.id)
        .collect();
    // [0] hit_area, [1] halo, [2] outer_ring, [3] inner_dot
    let r1_dot = r1_rects[3];
    let r2_dot = r2_rects[3];

    // Initially r1 is checked (inner_dot = #1976d2) and r2 is unchecked (inner_dot = #00000000)
    assert_eq!(
        compiled.layout.get_node(r1_dot).unwrap().properties.get("color"),
        Some(&Value::Color("#1976d2".to_string()))
    );
    assert_eq!(
        compiled.layout.get_node(r2_dot).unwrap().properties.get("color"),
        Some(&Value::Color("#00000000".to_string()))
    );

    // Click r2 -> r2 becomes checked and r1 automatically unchecks!
    let mut click_r2 = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(25.0, 75.0),
        Point::new(15.0, 15.0),
        Modifiers::default(),
        r2_rects[0],
    );
    compiled.dispatch_event(&mut click_r2).unwrap();
    assert_eq!(
        compiled.layout.get_node(r1_dot).unwrap().properties.get("color"),
        Some(&Value::Color("#00000000".to_string()))
    );
    assert_eq!(
        compiled.layout.get_node(r2_dot).unwrap().properties.get("color"),
        Some(&Value::Color("#1976d2".to_string()))
    );

    // Click r1 -> r1 becomes checked again and r2 unchecks!
    let mut click_r1 = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(25.0, 25.0),
        Point::new(15.0, 15.0),
        Modifiers::default(),
        r1_rects[0],
    );
    compiled.dispatch_event(&mut click_r1).unwrap();
    assert_eq!(
        compiled.layout.get_node(r1_dot).unwrap().properties.get("color"),
        Some(&Value::Color("#1976d2".to_string()))
    );
    assert_eq!(
        compiled.layout.get_node(r2_dot).unwrap().properties.get("color"),
        Some(&Value::Color("#00000000".to_string()))
    );
}

#[test]
fn test_viewer_focus_and_label_click_on_selection_controls() {
    use directedtype::render::{ViewerApp, ViewerConfig};
    use winit::event::ElementState;

    let input = r#"
    \use "components/material/Switch.dt"
    \use "components/material/Checkbox.dt"
    \use "components/material/Radio.dt"

    \Switch("sw"; x: 20px, y: 20px) { Dark Mode }
    \Switch("sw_dis"; x: 20px, y: 70px, disabled: true) { Disabled }
    \Checkbox("cb"; x: 20px, y: 120px, label: "Auto-Save")
    \Radio("rad"; x: 20px, y: 170px) { High Quality }
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

    let sw_id = compiled.find_by_key(None, &ComponentKey::string("sw")).unwrap().id;
    let cb_id = compiled.find_by_key(None, &ComponentKey::string("cb")).unwrap().id;
    let rad_id = compiled.find_by_key(None, &ComponentKey::string("rad")).unwrap().id;

    let mut viewer = ViewerApp::new_with_compiled(compiled, ViewerConfig::default());

    // Only enabled selection controls are focusable
    assert_eq!(viewer.focusable_nodes(), vec![sw_id, cb_id, rad_id]);

    // Clicking on the right side of the Switch (over its label text) focuses and toggles the Switch
    let sw_rect = viewer.layout().get_node(sw_id).unwrap().rect;
    let label_pt = Point::new(sw_rect.x + sw_rect.width - 10.0, sw_rect.y + sw_rect.height / 2.0);
    viewer.dispatch_mouse_input(ElementState::Pressed, MouseButton::Left, label_pt);
    viewer.dispatch_mouse_input(ElementState::Released, MouseButton::Left, label_pt);
    assert_eq!(viewer.focused_node(), Some(sw_id));
    assert_eq!(
        viewer.compiled().unwrap().get_state(sw_id, "is_checked"),
        Some(&Value::Bool(true))
    );
}

#[test]
fn test_material_gallery_selection_controls_section_compiles() {
    let gallery_src = std::fs::read_to_string("examples/material_gallery.dt").unwrap();
    let doc = parse(&gallery_src).expect("gallery parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        1024.0,
        1200.0,
        Path::new("examples"),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("gallery compile ok");

    for key in [
        "card_selection_controls",
        "sw_unchecked",
        "sw_checked",
        "sw_small",
        "sw_disabled_on",
        "cb_unchecked",
        "cb_checked",
        "cb_indeterminate",
        "cb_small",
        "rad_unchecked",
        "rad_checked",
        "rad_small",
    ] {
        let node = compiled
            .find_by_key(None, &ComponentKey::string(key))
            .unwrap_or_else(|| panic!("expected gallery key '{}' to exist", key));
        assert!(node.rect.width > 0.0);
        assert!(node.rect.height > 0.0);
    }
}

#[test]
fn test_material_selection_controls_headless_render_pixels() {
    use directedtype::render::headless::HeadlessRenderer;
    use directedtype::render::scene::SceneOptions;

    let input = r#"
    \use "components/material/Switch.dt"
    \use "components/material/Checkbox.dt"
    \use "components/material/Radio.dt"

    \Rect(x: 0px, y: 0px, width: 300px, height: 100px, color: #ffffff)
    \Switch("sw"; x: 10px, y: 10px, checked: true)
    \Checkbox("cb"; x: 80px, y: 10px, checked: true)
    \Radio("rad"; x: 140px, y: 10px, checked: true)
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        300.0,
        100.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let mut renderer = HeadlessRenderer::new().expect("init headless renderer");
    let img = renderer
        .render_layout(&compiled.layout, 300, 100, &SceneOptions::default())
        .expect("render ok");

    // 1. Checked Switch thumb center at (10 + 29 + 10 = 49, 10 + 9 + 10 = 29) -> Primary blue (#1976d2 = rgb(25, 118, 210))
    let sw_thumb_px = img.get_pixel(49, 29).0;
    assert!(
        sw_thumb_px[0] < 40 && sw_thumb_px[1] > 100 && sw_thumb_px[2] > 190,
        "Expected primary blue pixel on checked Switch thumb at (49, 29), got {:?}",
        sw_thumb_px
    );

    // 2. Checked Checkbox box upper-left inside fill at (80 + 14 = 94, 10 + 14 = 24) -> Primary blue
    let cb_fill_px = img.get_pixel(94, 24).0;
    assert!(
        cb_fill_px[0] < 40 && cb_fill_px[1] > 100 && cb_fill_px[2] > 190,
        "Expected primary blue pixel on checked Checkbox box at (94, 24), got {:?}",
        cb_fill_px
    );

    // 3. Checked Radio inner dot center at (140 + 21 = 161, 10 + 21 = 31) -> Primary blue
    let rad_dot_px = img.get_pixel(161, 31).0;
    assert!(
        rad_dot_px[0] < 40 && rad_dot_px[1] > 100 && rad_dot_px[2] > 190,
        "Expected primary blue pixel on checked Radio inner dot at (161, 31), got {:?}",
        rad_dot_px
    );
}

