use directedtype::ast::ComponentKey;
use directedtype::compiler::compiled::CompiledDocument;
use directedtype::compiler::value::Value;
use directedtype::component::{ComponentRegistry, Context};
use directedtype::interaction::{Event, EventKind, Modifiers, MouseButton, Point};
use directedtype::parse;
use directedtype_macros::component;
use std::path::Path;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;

#[test]
fn test_material_appbar_and_toolbar_geometry_variants_colors_and_actions() {
    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/Button.dt"
    \use "components/material/Typography.dt"
    \use "components/material/AppBar.dt"

    // 1. Primary Regular AppBar (64px height, square: true -> radius 0, elevation 4)
    \AppBar("ab_primary"; x: 20px, y: 20px, width: 600px, color: MaterialColor.Primary) {
        \Toolbar("tb_regular"; nav_icon: "☰", title: "News") {
            \Button("btn_login"; color: MaterialColor.Inherit) { Login }
        }
    }

    // 2. Secondary Dense AppBar (48px height, square: false -> radius 4px)
    \AppBar("ab_secondary"; x: 20px, y: 100px, width: 600px, color: MaterialColor.Secondary, square: false) {
        \Toolbar("tb_dense"; variant: ToolbarVariant.Dense, avatar: "M", title: "Workspace", subtitle: "Sub") {
            \Button("btn_share"; color: MaterialColor.Inherit, size: ButtonSize.Small) { Share }
            \Button("btn_export"; variant: ButtonVariant.Outlined, color: MaterialColor.Inherit, size: ButtonSize.Small) { Export }
        }
    }

    // 3. Inherit Outlined AppBar with disable_gutters and child composition (secondary_action: true)
    \AppBar("ab_outlined"; x: 20px, y: 170px, width: 600px, color: MaterialColor.Inherit, variant: AppBarVariant.Outlined) {
        \Toolbar("tb_nogutters"; disable_gutters: true, secondary_action: true) {
            \H6("tb_heading";) { Brand }
            \Button("btn_action"; color: MaterialColor.Inherit) { Action }
        }
    }

    // 4. Intrinsic-width AppBar & Toolbar
    \AppBar("ab_intrinsic"; x: 20px, y: 260px, color: MaterialColor.Success) {
        \Toolbar("tb_intrinsic"; title: "Intrinsic Header") {
            \Button("btn_intr"; color: MaterialColor.Inherit) { Go }
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
    .expect("compile ok");

    // 1. Primary Regular AppBar & Toolbar
    let ab_primary = compiled
        .find_by_key(None, &ComponentKey::string("ab_primary"))
        .unwrap();
    let tb_regular = compiled
        .find_by_key(None, &ComponentKey::string("tb_regular"))
        .unwrap();
    let btn_login = compiled
        .find_by_key(None, &ComponentKey::string("btn_login"))
        .unwrap();

    assert_eq!(ab_primary.rect.width, 600.0);
    assert_eq!(ab_primary.rect.height, 64.0);
    assert_eq!(tb_regular.rect.width, 600.0);
    assert_eq!(tb_regular.rect.height, 64.0);

    // Trailing button in 600px Toolbar with 24px gutters is pinned at right edge: 20 + 600 - 24 - btn.width
    assert!(
        (btn_login.rect.x + btn_login.rect.width - (20.0 + 600.0 - 24.0)).abs() < 0.01,
        "Expected trailing button right edge at {}, got {}",
        20.0 + 600.0 - 24.0,
        btn_login.rect.x + btn_login.rect.width
    );

    // Verify Button(color: MaterialColor.Inherit) inside dark Primary AppBar receives white (#ffffff) text color
    let login_text_node = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(btn_login.id) && n.name == "Text")
        .unwrap();
    assert_eq!(
        login_text_node.properties.get("color"),
        Some(&Value::Color("#ffffff".to_string()))
    );

    // 2. Secondary Dense AppBar (48px height, square: false -> radius 4px)
    let ab_secondary = compiled
        .find_by_key(None, &ComponentKey::string("ab_secondary"))
        .unwrap();
    let tb_dense = compiled
        .find_by_key(None, &ComponentKey::string("tb_dense"))
        .unwrap();
    let btn_export = compiled
        .find_by_key(None, &ComponentKey::string("btn_export"))
        .unwrap();

    assert_eq!(ab_secondary.rect.height, 48.0);
    assert_eq!(tb_dense.rect.height, 48.0);

    // Outlined Inherit button inside dark Secondary AppBar has white translucent border (#ffffff80)
    let export_surface = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(btn_export.id) && n.name == "Rect")
        .unwrap();
    assert_eq!(
        export_surface.properties.get("border_color"),
        Some(&Value::Color("#ffffff80".to_string()))
    );

    // 3. Inherit Outlined AppBar with disable_gutters (0px horizontal padding) and secondary_action
    let ab_outlined = compiled
        .find_by_key(None, &ComponentKey::string("ab_outlined"))
        .unwrap();
    let tb_heading = compiled
        .find_by_key(None, &ComponentKey::string("tb_heading"))
        .unwrap();
    let btn_action = compiled
        .find_by_key(None, &ComponentKey::string("btn_action"))
        .unwrap();

    assert_eq!(ab_outlined.rect.height, 64.0);
    assert_eq!(tb_heading.rect.x, 20.0); // disable_gutters -> 0px left padding
    assert!((btn_action.rect.x + btn_action.rect.width - (20.0 + 600.0)).abs() < 0.01);

    // Inside Inherit (light #f5f5f5) AppBar, Inherit Button receives AppBar's dark contrast text (#000000de)
    let action_text_node = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(btn_action.id) && n.name == "Text")
        .unwrap();
    assert_eq!(
        action_text_node.properties.get("color"),
        Some(&Value::Color("#000000de".to_string()))
    );

    // 4. Intrinsic AppBar & Toolbar
    let ab_intrinsic = compiled
        .find_by_key(None, &ComponentKey::string("ab_intrinsic"))
        .unwrap();
    let tb_intrinsic = compiled
        .find_by_key(None, &ComponentKey::string("tb_intrinsic"))
        .unwrap();
    assert!(ab_intrinsic.rect.width > 120.0);
    assert_eq!(ab_intrinsic.rect.width, tb_intrinsic.rect.width);
    assert_eq!(ab_intrinsic.rect.height, 64.0);
}

#[test]
fn test_material_tabs_and_tab_geometry_variants_orientations_and_icons() {
    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/Tabs.dt"

    // 1. Standard Horizontal Tabs (400px width, value: 0)
    \Tabs("tabs_std"; x: 10px, y: 10px, width: 400px, value: 0, divider: true) {
        \Tab("std_0"; label: "ONE")
        \Tab("std_1"; label: "TWO")
        \Tab("std_2"; label: "THREE")
    }

    // 2. Centered Horizontal Tabs (400px width, value: 1)
    \Tabs("tabs_centered"; x: 10px, y: 80px, width: 400px, value: 1, centered: true) {
        \Tab("cnt_0"; label: "ONE")
        \Tab("cnt_1"; label: "TWO")
    }

    // 3. FullWidth Horizontal Tabs (360px width, 3 tabs -> 120px each)
    \Tabs("tabs_fw"; x: 10px, y: 150px, width: 360px, value: 2, variant: TabsVariant.FullWidth) {
        \Tab("fw_0"; label: "A")
        \Tab("fw_1"; label: "B")
        \Tab("fw_2"; label: "C")
    }

    // 4. Icon Tabs: Top (72px), Bottom (72px), Start (48px), End (48px), and Icon-Only (48px)
    \Tabs("tabs_icons_stacked"; x: 10px, y: 220px, width: 360px, variant: TabsVariant.FullWidth) {
        \Tab("ic_top"; icon: "☎", icon_position: TabIconPosition.Top, label: "CALLS")
        \Tab("ic_bottom"; icon: "♥", icon_position: TabIconPosition.Bottom, label: "LIKES")
    }
    \Tabs("tabs_icons_inline"; x: 10px, y: 310px, width: 360px, variant: TabsVariant.FullWidth) {
        \Tab("ic_start"; icon: "✉", icon_position: TabIconPosition.Start, label: "MAIL")
        \Tab("ic_end"; icon: "★", icon_position: TabIconPosition.End, label: "STAR")
        \Tab("ic_only"; icon: "⚙")
    }

    // 5. Vertical Tabs (width: 200px, 3 tabs -> 3 * 48 = 144px height, right-edge indicator)
    \Tabs("tabs_vert"; x: 450px, y: 10px, width: 200px, value: 1, orientation: TabsOrientation.Vertical, divider: true) {
        \Tab("vert_0"; label: "PROFILE")
        \Tab("vert_1"; label: "SECURITY")
        \Tab("vert_2";) { NOTIFICATIONS }
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

    // 1. Standard Tabs
    let tabs_std = compiled
        .find_by_key(None, &ComponentKey::string("tabs_std"))
        .unwrap();
    let std_0 = compiled
        .find_by_key(None, &ComponentKey::string("std_0"))
        .unwrap();
    let std_1 = compiled
        .find_by_key(None, &ComponentKey::string("std_1"))
        .unwrap();
    assert_eq!(tabs_std.rect.width, 400.0);
    assert_eq!(tabs_std.rect.height, 48.0);
    assert_eq!(std_0.rect.x, 10.0);
    assert!(std_0.rect.width >= 90.0); // min_width: 90px
    assert_eq!(std_0.rect.height, 48.0);
    assert!((std_1.rect.x - (std_0.rect.x + std_0.rect.width)).abs() < 0.01);

    // Active indicator on tabs_std (last child Rect of tabs_std) matches std_0
    let std_rects: Vec<_> = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(tabs_std.id) && n.name == "Rect")
        .collect();
    let std_ind = std_rects.last().unwrap();
    assert_eq!(std_ind.rect.x, std_0.rect.x);
    assert_eq!(std_ind.rect.width, std_0.rect.width);
    assert_eq!(std_ind.rect.height, 2.0);
    assert_eq!(std_ind.rect.y, 10.0 + 48.0 - 2.0);

    // 2. Centered Tabs: cnt_0 starts at 10 + (400 - (cnt_0.w + cnt_1.w)) / 2
    let cnt_0 = compiled
        .find_by_key(None, &ComponentKey::string("cnt_0"))
        .unwrap();
    let cnt_1 = compiled
        .find_by_key(None, &ComponentKey::string("cnt_1"))
        .unwrap();
    let expected_cnt_x = 10.0 + (400.0 - (cnt_0.rect.width + cnt_1.rect.width)) / 2.0;
    assert!((cnt_0.rect.x - expected_cnt_x).abs() < 0.01);

    // 3. FullWidth Tabs: 360px / 3 = 120px per tab; value: 2 -> indicator at fw_2
    let tabs_fw = compiled
        .find_by_key(None, &ComponentKey::string("tabs_fw"))
        .unwrap();
    let fw_0 = compiled
        .find_by_key(None, &ComponentKey::string("fw_0"))
        .unwrap();
    let fw_1 = compiled
        .find_by_key(None, &ComponentKey::string("fw_1"))
        .unwrap();
    let fw_2 = compiled
        .find_by_key(None, &ComponentKey::string("fw_2"))
        .unwrap();
    assert!((fw_0.rect.width - 120.0).abs() < 0.01);
    assert!((fw_1.rect.width - 120.0).abs() < 0.01);
    assert!((fw_2.rect.width - 120.0).abs() < 0.01);
    assert!((fw_2.rect.x - (10.0 + 240.0)).abs() < 0.01);

    let fw_ind = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(tabs_fw.id) && n.name == "Rect")
        .last()
        .unwrap();
    assert!((fw_ind.rect.x - fw_2.rect.x).abs() < 0.01);
    assert!((fw_ind.rect.width - 120.0).abs() < 0.01);

    // 4. Stacked Icon Tabs (Top/Bottom -> 72px height) vs Inline/Icon-Only Tabs (Start/End/Icon-Only -> 48px height)
    let tabs_icons_stacked = compiled
        .find_by_key(None, &ComponentKey::string("tabs_icons_stacked"))
        .unwrap();
    let ic_top = compiled
        .find_by_key(None, &ComponentKey::string("ic_top"))
        .unwrap();
    let ic_bottom = compiled
        .find_by_key(None, &ComponentKey::string("ic_bottom"))
        .unwrap();
    let ic_start = compiled
        .find_by_key(None, &ComponentKey::string("ic_start"))
        .unwrap();
    let ic_end = compiled
        .find_by_key(None, &ComponentKey::string("ic_end"))
        .unwrap();
    let ic_only = compiled
        .find_by_key(None, &ComponentKey::string("ic_only"))
        .unwrap();

    assert_eq!(tabs_icons_stacked.rect.height, 72.0);
    assert_eq!(ic_top.rect.height, 72.0);
    assert_eq!(ic_bottom.rect.height, 72.0);
    assert_eq!(ic_start.rect.height, 48.0);
    assert_eq!(ic_end.rect.height, 48.0);
    assert_eq!(ic_only.rect.height, 48.0);

    // 5. Vertical Tabs: 3 tabs stacked vertically, each 200px wide and 48px tall; active indicator on right edge of vert_1
    let tabs_vert = compiled
        .find_by_key(None, &ComponentKey::string("tabs_vert"))
        .unwrap();
    let vert_0 = compiled
        .find_by_key(None, &ComponentKey::string("vert_0"))
        .unwrap();
    let vert_1 = compiled
        .find_by_key(None, &ComponentKey::string("vert_1"))
        .unwrap();
    let vert_2 = compiled
        .find_by_key(None, &ComponentKey::string("vert_2"))
        .unwrap();

    assert_eq!(tabs_vert.rect.width, 200.0);
    assert_eq!(tabs_vert.rect.height, 144.0);
    assert_eq!(vert_0.rect.width, 200.0);
    assert_eq!(vert_0.rect.y, 10.0);
    assert_eq!(vert_1.rect.y, 10.0 + 48.0);
    assert_eq!(vert_2.rect.y, 10.0 + 96.0);

    let vert_ind = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(tabs_vert.id) && n.name == "Rect")
        .last()
        .unwrap();
    assert_eq!(vert_ind.rect.x, 450.0 + 200.0 - 2.0);
    assert_eq!(vert_ind.rect.y, vert_1.rect.y);
    assert_eq!(vert_ind.rect.width, 2.0);
    assert_eq!(vert_ind.rect.height, 48.0);
}

#[test]
fn test_material_tabs_interactive_selection_callbacks_controlled_and_disabled() {
    #[derive(Default)]
    struct TestHost {
        change_count: Arc<AtomicUsize>,
        last_tab_index: Arc<AtomicI64>,
    }

    #[component]
    impl TestHost {
        pub fn on_tab_change(&mut self, index: i64, _ctx: &mut Context<'_>) {
            self.change_count.fetch_add(1, Ordering::SeqCst);
            self.last_tab_index.store(index, Ordering::SeqCst);
        }
    }

    let input = r#"
    \use "components/material/Tabs.dt"

    \Component TestHost {
        \Tabs("tabs_uncontrolled"; x: 10px, y: 10px, width: 400px, value: 0, on_change: self.on_tab_change) {
            \Tab("tab_0"; label: "FIRST")
            \Tab("tab_1"; label: "SECOND")
            \Tab("tab_2_dis"; label: "DISABLED", disabled: true)
        }

        \Tabs("tabs_controlled"; x: 10px, y: 100px, width: 400px, value: 0, controlled: true, on_change: self.on_tab_change) {
            \Tab("ctrl_0"; label: "CTRL A")
            \Tab("ctrl_1"; label: "CTRL B")
        }
    }

    \TestHost("host";)
    "#;

    let change_count = Arc::new(AtomicUsize::new(0));
    let last_tab_index = Arc::new(AtomicI64::new(-1));
    let cc_clone = Arc::clone(&change_count);
    let lti_clone = Arc::clone(&last_tab_index);

    let doc = parse(input).expect("parse ok");
    let mut registry = ComponentRegistry::standard();
    registry.register("TestHost", move || {
        Box::new(TestHost {
            change_count: Arc::clone(&cc_clone),
            last_tab_index: Arc::clone(&lti_clone),
        })
    });

    let mut compiled = CompiledDocument::compile_with_registry(
        &doc,
        600.0,
        300.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let tabs_uncontrolled_id = compiled
        .find_by_key(None, &ComponentKey::string("tabs_uncontrolled"))
        .unwrap()
        .id;
    let tab_0_id = compiled
        .find_by_key(None, &ComponentKey::string("tab_0"))
        .unwrap()
        .id;
    let tab_1_id = compiled
        .find_by_key(None, &ComponentKey::string("tab_1"))
        .unwrap()
        .id;
    let tab_2_dis_id = compiled
        .find_by_key(None, &ComponentKey::string("tab_2_dis"))
        .unwrap()
        .id;

    // 1. Initial state: tab_0 is selected (#1976d2), tab_1 is unselected (#00000099), tab_2_dis is disabled (#00000061)
    let tab_0_label = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(tab_0_id) && n.name == "Text")
        .last()
        .unwrap();
    let tab_1_label = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(tab_1_id) && n.name == "Text")
        .last()
        .unwrap();
    let tab_2_label = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(tab_2_dis_id) && n.name == "Text")
        .last()
        .unwrap();

    assert_eq!(
        tab_0_label.properties.get("color"),
        Some(&Value::Color("#1976d2".to_string()))
    );
    assert_eq!(
        tab_1_label.properties.get("color"),
        Some(&Value::Color("#00000099".to_string()))
    );
    assert_eq!(
        tab_2_label.properties.get("color"),
        Some(&Value::Color("#00000061".to_string()))
    );

    // 2. Hover & Press on tab_1
    let tab_1_rect = compiled.layout.get_node(tab_1_id).unwrap().rect;
    let tab_1_pt = Point::new(tab_1_rect.x + 20.0, tab_1_rect.y + 20.0);

    let mut enter_ev = Event::new(
        EventKind::PointerEnter,
        tab_1_pt,
        Point::new(20.0, 20.0),
        Modifiers::default(),
        tab_1_id,
    );
    compiled.dispatch_event(&mut enter_ev).unwrap();
    assert_eq!(
        compiled.get_state(tab_1_id, "hovered"),
        Some(&Value::Bool(true))
    );

    let mut down_ev = Event::new(
        EventKind::PointerDown {
            button: MouseButton::Left,
        },
        tab_1_pt,
        Point::new(20.0, 20.0),
        Modifiers::default(),
        tab_1_id,
    );
    compiled.dispatch_event(&mut down_ev).unwrap();
    assert_eq!(
        compiled.get_state(tab_1_id, "pressed"),
        Some(&Value::Bool(true))
    );

    let mut up_ev = Event::new(
        EventKind::PointerUp {
            button: MouseButton::Left,
        },
        tab_1_pt,
        Point::new(20.0, 20.0),
        Modifiers::default(),
        tab_1_id,
    );
    compiled.dispatch_event(&mut up_ev).unwrap();
    assert_eq!(
        compiled.get_state(tab_1_id, "pressed"),
        Some(&Value::Bool(false))
    );

    // 3. Click on tab_1 -> selects index 1, moves active indicator to tab_1, and fires on_change(1)
    let mut click_tab_1 = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        tab_1_pt,
        Point::new(20.0, 20.0),
        Modifiers::default(),
        tab_1_id,
    );
    compiled.dispatch_event(&mut click_tab_1).unwrap();

    assert_eq!(
        compiled.get_state(tabs_uncontrolled_id, "selected_index"),
        Some(&Value::Integer(1))
    );
    assert_eq!(change_count.load(Ordering::SeqCst), 1);
    assert_eq!(last_tab_index.load(Ordering::SeqCst), 1);

    let ind_rect = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(tabs_uncontrolled_id) && n.name == "Rect")
        .last()
        .unwrap()
        .rect;
    assert!((ind_rect.x - tab_1_rect.x).abs() < 0.01);
    assert!((ind_rect.width - tab_1_rect.width).abs() < 0.01);

    let tab_0_label_after = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(tab_0_id) && n.name == "Text")
        .last()
        .unwrap();
    let tab_1_label_after = compiled
        .layout
        .nodes
        .iter()
        .filter(|n| n.parent == Some(tab_1_id) && n.name == "Text")
        .last()
        .unwrap();
    assert_eq!(
        tab_0_label_after.properties.get("color"),
        Some(&Value::Color("#00000099".to_string()))
    );
    assert_eq!(
        tab_1_label_after.properties.get("color"),
        Some(&Value::Color("#1976d2".to_string()))
    );

    // 4. Click on disabled tab_2_dis -> ignored (selection stays at 1, callback count stays 1)
    let tab_2_rect = compiled.layout.get_node(tab_2_dis_id).unwrap().rect;
    let tab_2_pt = Point::new(tab_2_rect.x + 20.0, tab_2_rect.y + 20.0);
    let mut click_tab_2 = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        tab_2_pt,
        Point::new(20.0, 20.0),
        Modifiers::default(),
        tab_2_dis_id,
    );
    compiled.dispatch_event(&mut click_tab_2).unwrap();
    assert_eq!(
        compiled.get_state(tabs_uncontrolled_id, "selected_index"),
        Some(&Value::Integer(1))
    );
    assert_eq!(change_count.load(Ordering::SeqCst), 1);

    // 5. Controlled Tabs: clicking ctrl_1 emits on_change(1) without mutating selected_index state
    let tabs_controlled_id = compiled
        .find_by_key(None, &ComponentKey::string("tabs_controlled"))
        .unwrap()
        .id;
    let ctrl_1_id = compiled
        .find_by_key(None, &ComponentKey::string("ctrl_1"))
        .unwrap()
        .id;
    let ctrl_1_rect = compiled.layout.get_node(ctrl_1_id).unwrap().rect;
    let ctrl_1_pt = Point::new(ctrl_1_rect.x + 20.0, ctrl_1_rect.y + 20.0);

    let mut click_ctrl_1 = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        ctrl_1_pt,
        Point::new(20.0, 20.0),
        Modifiers::default(),
        ctrl_1_id,
    );
    compiled.dispatch_event(&mut click_ctrl_1).unwrap();
    assert_eq!(change_count.load(Ordering::SeqCst), 2);
    assert_eq!(last_tab_index.load(Ordering::SeqCst), 1);
    assert_eq!(
        compiled.get_state(tabs_controlled_id, "selected_index"),
        Some(&Value::Integer(0))
    );
}

#[test]
fn test_viewer_focus_and_keyboard_activation_on_tabs_and_appbar() {
    use directedtype::render::{ViewerApp, ViewerConfig};

    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/Button.dt"
    \use "components/material/AppBar.dt"
    \use "components/material/Tabs.dt"

    \AppBar("appbar"; x: 10px, y: 10px, width: 500px, color: MaterialColor.Primary) {
        \Toolbar("toolbar"; variant: ToolbarVariant.Dense, title: "Dashboard") {
            \Button("btn_help"; color: MaterialColor.Inherit) { Help }
        }
        \Tabs("tabs"; value: 0, text_color: MaterialColor.Inherit, indicator_color: MaterialColor.Inherit) {
            \Tab("tab_0"; label: "OVERVIEW")
            \Tab("tab_1"; label: "METRICS")
            \Tab("tab_dis"; label: "LOCKED", disabled: true)
        }
    }
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        600.0,
        300.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let btn_help_id = compiled
        .find_by_key(None, &ComponentKey::string("btn_help"))
        .unwrap()
        .id;
    let tabs_id = compiled
        .find_by_key(None, &ComponentKey::string("tabs"))
        .unwrap()
        .id;
    let tab_0_id = compiled
        .find_by_key(None, &ComponentKey::string("tab_0"))
        .unwrap()
        .id;
    let tab_1_id = compiled
        .find_by_key(None, &ComponentKey::string("tab_1"))
        .unwrap()
        .id;

    let mut viewer = ViewerApp::new_with_compiled(compiled, ViewerConfig::default());

    // 1. Focusable nodes include btn_help, tab_0, and tab_1, excluding disabled tab_dis
    assert_eq!(
        viewer.focusable_nodes(),
        vec![btn_help_id, tab_0_id, tab_1_id]
    );

    // 2. Traverse focus with Tab key to tab_1 and activate via keyboard (Enter/Space)
    viewer.focus_next(); // btn_help
    assert_eq!(viewer.focused_node(), Some(btn_help_id));
    viewer.focus_next(); // tab_0
    assert_eq!(viewer.focused_node(), Some(tab_0_id));
    viewer.focus_next(); // tab_1
    assert_eq!(viewer.focused_node(), Some(tab_1_id));

    assert!(viewer.activate_focused_node());
    assert_eq!(
        viewer
            .compiled()
            .unwrap()
            .get_state(tabs_id, "selected_index"),
        Some(&Value::Integer(1))
    );
}

#[test]
fn test_material_gallery_appbar_and_tabs_sections_compile() {
    let gallery_src = std::fs::read_to_string("examples/material_gallery.dt").unwrap();
    let doc = parse(&gallery_src).expect("gallery parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        1024.0,
        2400.0,
        Path::new("examples"),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("gallery compile ok");

    for key in [
        "card_appbar_toolbar",
        "appbar_primary",
        "toolbar_primary",
        "appbar_btn_login",
        "appbar_secondary",
        "toolbar_dense",
        "appbar_inherit",
        "toolbar_inherit",
        "appbar_with_tabs",
        "appbar_tabs",
        "appbar_tab_0",
        "appbar_tab_1",
        "card_tabs",
        "tabs_standard",
        "tab_std_0",
        "tab_std_1",
        "tab_std_disabled",
        "tabs_centered",
        "tabs_fullwidth",
        "tabs_icon_top",
        "tabs_icon_start",
        "tabs_vertical",
    ] {
        let node = compiled
            .find_by_key(None, &ComponentKey::string(key))
            .unwrap_or_else(|| panic!("expected gallery key '{}' to exist", key));
        assert!(node.rect.width > 0.0);
        assert!(node.rect.height > 0.0);
    }
}

#[test]
fn test_material_appbar_and_tabs_headless_render_pixels() {
    use directedtype::render::headless::HeadlessRenderer;
    use directedtype::render::scene::SceneOptions;

    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/AppBar.dt"
    \use "components/material/Tabs.dt"

    \Rect(x: 0px, y: 0px, width: 320px, height: 180px, color: #ffffff)

    \AppBar("ab"; x: 10px, y: 10px, width: 300px, color: MaterialColor.Primary) {
        \Toolbar(title: "Header")
    }

    \Tabs("tabs"; x: 10px, y: 100px, width: 300px, value: 0, variant: TabsVariant.FullWidth) {
        \Tab(label: "ONE")
        \Tab(label: "TWO")
    }
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        320.0,
        180.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let mut renderer = HeadlessRenderer::new().expect("init headless renderer");
    let img = renderer
        .render_layout(&compiled.layout, 320, 180, &SceneOptions::default())
        .expect("render ok");

    // 1. Primary AppBar surface pixel at (260, 40) -> #1976d2 (R=25, G=118, B=210)
    let ab_px = img.get_pixel(260, 40).0;
    assert!(
        ab_px[0] < 45 && ab_px[1] > 100 && ab_px[2] > 190,
        "Expected Primary blue pixel on AppBar surface at (260, 40), got {:?}",
        ab_px
    );

    // 2. Active tab indicator bar on Tab 0 at (80, 147) -> #1976d2, while Tab 1 bottom at (230, 147) is white (#ffffff)
    let ind_px = img.get_pixel(80, 147).0;
    assert!(
        ind_px[0] < 45 && ind_px[1] > 100 && ind_px[2] > 190,
        "Expected Primary blue active indicator pixel at (80, 147), got {:?}",
        ind_px
    );
    let inactive_bottom_px = img.get_pixel(230, 147).0;
    assert!(
        inactive_bottom_px[0] > 245 && inactive_bottom_px[1] > 245 && inactive_bottom_px[2] > 245,
        "Expected white background under inactive tab 1 at (230, 147), got {:?}",
        inactive_bottom_px
    );
}
