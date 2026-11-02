use directedtype::ast::ComponentKey;
use directedtype::compiler::compiled::CompiledDocument;
use directedtype::compiler::value::Value;
use directedtype::component::{ComponentRegistry, Context, DispatchError};
use directedtype::interaction::{Event, EventKind, Modifiers, MouseButton, Point};
use directedtype::parse;
use directedtype_macros::component;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[test]
fn test_material_list_geometry_single_and_two_line_dense_inset_and_avatars() {
    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/Avatar.dt"
    \use "components/material/Divider.dt"
    \use "components/material/List.dt"

    // 1. Standard Explicit-Width List (360px) with Subheader, 1-line, 2-line + Avatar, Inset Divider, and Inset Item
    \List("std_list"; x: 20px, y: 20px, width: 360px, subheader: "Folders") {
        \ListItem("item_1line";) {
            \ListItemText("text_1line"; primary: "Single-line item")
        }
        \ListItem("item_2line_av";) {
            \ListItemAvatar("av_wrap";) {
                \Avatar("av_inner"; bg: material_primary.main) { P }
            }
            \ListItemText("text_2line"; primary: "Photos", secondary: "Jan 9, 2026")
        }
        \Divider("div_inset"; variant: DividerVariant.Inset)
        \ListItem("item_inset"; divider: true) {
            \ListItemText("text_inset"; inset: true) { Starred (Inset) }
        }
    }

    // 2. Dense List (propagating `dense: true` via env to ListItem and ListItemText)
    \List("dense_list"; x: 420px, y: 20px, width: 300px, dense: true, disable_padding: true, subheader: "Dense Header") {
        \ListItem("dense_1line";) {
            \ListItemText("dense_text_1"; primary: "Dense Single")
        }
        \ListItem("dense_2line";) {
            \ListItemAvatar("av_shorthand"; label: "D", bg: material_secondary.main)
            \ListItemText("dense_text_2"; primary: "Dense Two-Line", secondary: "Compact subtext")
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

    // 1. Standard List dimensions:
    // top/bottom padding = 2 * 8 = 16px
    // subheader = 48px
    // item_1line = 32px (ListItemText: 24 + 8) + 16px (ListItem: 2 * 8) = 48px
    // item_2line_av = 56px (ListItemText: 24 + 20 + 12) + 16px (ListItem: 2 * 8) = 72px
    // div_inset = 1px
    // item_inset = 48px
    let std_list = compiled
        .find_by_key(None, &ComponentKey::string("std_list"))
        .unwrap();
    let item_1line = compiled
        .find_by_key(None, &ComponentKey::string("item_1line"))
        .unwrap();
    let item_2line_av = compiled
        .find_by_key(None, &ComponentKey::string("item_2line_av"))
        .unwrap();
    let av_wrap = compiled
        .find_by_key(None, &ComponentKey::string("av_wrap"))
        .unwrap();
    let text_2line = compiled
        .find_by_key(None, &ComponentKey::string("text_2line"))
        .unwrap();
    let item_inset = compiled
        .find_by_key(None, &ComponentKey::string("item_inset"))
        .unwrap();
    let text_inset = compiled
        .find_by_key(None, &ComponentKey::string("text_inset"))
        .unwrap();

    assert_eq!(std_list.rect.width, 360.0);
    assert_eq!(item_1line.rect.width, 360.0);
    assert_eq!(item_1line.rect.height, 48.0);
    assert_eq!(item_1line.rect.y, 20.0 + 8.0 + 48.0); // y + pad_y(8) + subheader(48)

    assert_eq!(item_2line_av.rect.width, 360.0);
    assert_eq!(item_2line_av.rect.height, 72.0);

    // ListItemAvatar has 56px min_width and 40px height, placing ListItemText at x + 16 + 56 = x + 72px
    assert_eq!(av_wrap.rect.x, 20.0 + 16.0);
    assert_eq!(av_wrap.rect.width, 56.0);
    assert_eq!(av_wrap.rect.height, 40.0);
    assert_eq!(text_2line.rect.x, 20.0 + 72.0);
    assert_eq!(text_2line.rect.height, 56.0);

    // Inset ListItemText (without avatar) shifts its text node by +56px so it aligns at x + 72px
    assert_eq!(item_inset.rect.height, 48.0);
    assert_eq!(text_inset.rect.x, 20.0 + 16.0);
    let inset_text_node = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(text_inset.id) && n.name == "Text")
        .unwrap();
    assert_eq!(inset_text_node.rect.x, 20.0 + 72.0);

    // Total std_list height = 16 + 48 + 48 + 72 + 1 + 48 = 233px
    assert_eq!(std_list.rect.height, 233.0);

    // 2. Dense List (disable_padding: true -> pad_y = 0, subheader = 36px):
    // dense_1line = 28px (ListItemText: 20 + 8) + 8px (ListItem: 2 * 4) = 36px
    // dense_2line = 52px (ListItemText: 20 + 20 + 12) + 8px (ListItem: 2 * 4) = 60px
    let dense_list = compiled
        .find_by_key(None, &ComponentKey::string("dense_list"))
        .unwrap();
    let dense_1line = compiled
        .find_by_key(None, &ComponentKey::string("dense_1line"))
        .unwrap();
    let dense_2line = compiled
        .find_by_key(None, &ComponentKey::string("dense_2line"))
        .unwrap();
    let av_shorthand = compiled
        .find_by_key(None, &ComponentKey::string("av_shorthand"))
        .unwrap();

    assert_eq!(dense_1line.rect.y, 20.0 + 36.0);
    assert_eq!(dense_1line.rect.height, 36.0);
    assert_eq!(dense_2line.rect.height, 60.0);
    assert_eq!(av_shorthand.rect.width, 56.0);
    assert_eq!(av_shorthand.rect.height, 40.0);
    assert_eq!(dense_list.rect.height, 36.0 + 36.0 + 60.0);
}

#[test]
fn test_material_list_item_trailing_actions_prop_overloads_and_intrinsic_sizing() {
    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/Switch.dt"
    \use "components/material/Checkbox.dt"
    \use "components/material/Button.dt"
    \use "components/material/List.dt"

    // 1. Explicit-width List with trailing controls (3-child, 2-child secondary_action, and primary prop overload)
    \List("settings_list"; x: 10px, y: 10px, width: 340px) {
        \ListItem("row_3child";) {
            \ListItemAvatar(label: "W", bg: material_primary.main)
            \ListItemText(primary: "Wi-Fi", secondary: "Connected")
            \Switch("sw_trailing"; checked: true)
        }
        \ListItem("row_2child_action"; secondary_action: true) {
            \ListItemText(primary: "Bluetooth")
            \Checkbox("cb_trailing"; checked: true)
        }
        \ListItem("row_prop_overload"; primary: "Direct Prop Item", secondary: "Subtext", avatar: "P", avatar_bg: material_success.main) {
            \Button("btn_trailing"; size: ButtonSize.Small, variant: ButtonVariant.Outlined) { Edit }
        }
        \ListItem("row_wrapped_btn"; disable_padding: true) {
            \ListItemButton("inner_btn";) {
                \ListItemAvatar(label: "B", bg: material_info.main)
                \ListItemText(primary: "Wrapped ListItemButton")
            }
        }
    }

    // 2. Intrinsic-width List
    \List("intrinsic_list"; x: 400px, y: 10px) {
        \ListItem("intr_item1"; primary: "Short")
        \ListItem("intr_item2";) {
            \ListItemAvatar(label: "L")
            \ListItemText(primary: "Longer intrinsic list item row")
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

    // 1. Trailing controls in 340px List are right-aligned at x + 340 - 16 - control.width
    let sw_trailing = compiled
        .find_by_key(None, &ComponentKey::string("sw_trailing"))
        .unwrap();
    assert!(
        ((sw_trailing.rect.x + sw_trailing.rect.width) - (10.0 + 340.0 - 16.0)).abs() < 1e-4,
        "3-child trailing Switch right edge should be at 334.0, got {}",
        sw_trailing.rect.x + sw_trailing.rect.width
    );

    let cb_trailing = compiled
        .find_by_key(None, &ComponentKey::string("cb_trailing"))
        .unwrap();
    assert!(
        ((cb_trailing.rect.x + cb_trailing.rect.width) - (10.0 + 340.0 - 16.0)).abs() < 1e-4,
        "2-child secondary_action trailing Checkbox right edge should be at 334.0, got {}",
        cb_trailing.rect.x + cb_trailing.rect.width
    );

    let row_prop = compiled
        .find_by_key(None, &ComponentKey::string("row_prop_overload"))
        .unwrap();
    let btn_trailing = compiled
        .find_by_key(None, &ComponentKey::string("btn_trailing"))
        .unwrap();
    assert_eq!(row_prop.rect.height, 72.0);
    assert!(
        ((btn_trailing.rect.x + btn_trailing.rect.width) - (10.0 + 340.0 - 16.0)).abs() < 1e-4
    );

    // 2. ListItem(disable_padding: true) wrapping ListItemButton stretches ListItemButton to 340px via env list_item_inner_width
    let row_wrapped = compiled
        .find_by_key(None, &ComponentKey::string("row_wrapped_btn"))
        .unwrap();
    let inner_btn = compiled
        .find_by_key(None, &ComponentKey::string("inner_btn"))
        .unwrap();
    assert_eq!(row_wrapped.rect.width, 340.0);
    assert_eq!(row_wrapped.rect.height, 56.0); // 40px avatar + 16px ListItemButton padding
    assert_eq!(inner_btn.rect.width, 340.0);
    assert_eq!(inner_btn.rect.height, 56.0);

    // 3. Intrinsic List pulls width bottom-up from widest item
    let intrinsic_list = compiled
        .find_by_key(None, &ComponentKey::string("intrinsic_list"))
        .unwrap();
    let intr_item1 = compiled
        .find_by_key(None, &ComponentKey::string("intr_item1"))
        .unwrap();
    let intr_item2 = compiled
        .find_by_key(None, &ComponentKey::string("intr_item2"))
        .unwrap();
    assert!(intr_item2.rect.width > intr_item1.rect.width);
    assert!((intrinsic_list.rect.width - intr_item2.rect.width).abs() < 1e-4);
}

#[test]
fn test_material_list_item_button_interactive_states_selection_and_disabled() {
    #[derive(Default)]
    struct ListHost {
        clicks: Arc<AtomicUsize>,
    }

    #[component]
    impl ListHost {
        pub fn on_item_click(&mut self, _ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            self.clicks.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/List.dt"

    \Component ListHost {
        \List("nav"; x: 10px, y: 10px, width: 280px) {
            \ListItemButton("btn_normal"; on_click: on_item_click) {
                \ListItemText("txt_normal"; primary: "Inbox")
            }
            \ListItemButton("btn_selected"; selected: true, color: MaterialColor.Secondary, on_click: on_item_click) {
                \ListItemText("txt_selected"; primary: "Drafts")
            }
            \ListItemButton("btn_disabled"; disabled: true, on_click: on_item_click) {
                \ListItemText("txt_disabled"; primary: "Archived")
            }
        }
    }

    \ListHost("host";)
    "#;

    let clicks = Arc::new(AtomicUsize::new(0));
    let clicks_clone = Arc::clone(&clicks);

    let doc = parse(input).expect("parse ok");
    let mut registry = ComponentRegistry::standard();
    registry.register("ListHost", move || {
        Box::new(ListHost {
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

    // Verify only ListItemButton (and ListHost) have component instances — List, ListItemText do not
    let nav_id = compiled
        .find_by_key(None, &ComponentKey::string("nav"))
        .unwrap()
        .id;
    assert!(compiled.instances.get(nav_id).is_none());

    let btn_normal_id = compiled
        .find_by_key(None, &ComponentKey::string("btn_normal"))
        .unwrap()
        .id;
    let btn_selected_id = compiled
        .find_by_key(None, &ComponentKey::string("btn_selected"))
        .unwrap()
        .id;
    let btn_disabled_id = compiled
        .find_by_key(None, &ComponentKey::string("btn_disabled"))
        .unwrap()
        .id;
    let txt_disabled_id = compiled
        .find_by_key(None, &ComponentKey::string("txt_disabled"))
        .unwrap()
        .id;

    let normal_bg_id = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(btn_normal_id) && n.name == "Rect")
        .unwrap()
        .id;
    let selected_bg_id = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(btn_selected_id) && n.name == "Rect")
        .unwrap()
        .id;
    let disabled_bg_id = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(btn_disabled_id) && n.name == "Rect")
        .unwrap()
        .id;

    // 1. Resting colors: normal = #00000000, selected (Secondary) = #9c27b014
    assert_eq!(
        compiled
            .layout
            .get_node(normal_bg_id)
            .unwrap()
            .properties
            .get("color"),
        Some(&Value::Color("#00000000".to_string()))
    );
    assert_eq!(
        compiled
            .layout
            .get_node(selected_bg_id)
            .unwrap()
            .properties
            .get("color"),
        Some(&Value::Color("#9c27b014".to_string()))
    );

    // 2. Hover + Down + Up + Click on normal ListItemButton
    let mut enter_ev = Event::new(
        EventKind::PointerEnter,
        Point::new(30.0, 30.0),
        Point::new(20.0, 12.0),
        Modifiers::default(),
        normal_bg_id,
    );
    compiled.dispatch_event(&mut enter_ev).unwrap();
    assert_eq!(
        compiled
            .layout
            .get_node(normal_bg_id)
            .unwrap()
            .properties
            .get("color"),
        Some(&Value::Color("#0000000a".to_string()))
    );

    let mut down_ev = Event::new(
        EventKind::PointerDown {
            button: MouseButton::Left,
        },
        Point::new(30.0, 30.0),
        Point::new(20.0, 12.0),
        Modifiers::default(),
        normal_bg_id,
    );
    compiled.dispatch_event(&mut down_ev).unwrap();
    assert_eq!(
        compiled
            .layout
            .get_node(normal_bg_id)
            .unwrap()
            .properties
            .get("color"),
        Some(&Value::Color("#0000001f".to_string()))
    );

    let mut click_ev = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(30.0, 30.0),
        Point::new(20.0, 12.0),
        Modifiers::default(),
        normal_bg_id,
    );
    compiled.dispatch_event(&mut click_ev).unwrap();
    assert_eq!(clicks.load(Ordering::SeqCst), 1);

    // 3. Hover on selected ListItemButton transitions to ch_focus_bg (#9c27b01f)
    let mut enter_sel = Event::new(
        EventKind::PointerEnter,
        Point::new(30.0, 80.0),
        Point::new(20.0, 12.0),
        Modifiers::default(),
        selected_bg_id,
    );
    compiled.dispatch_event(&mut enter_sel).unwrap();
    assert_eq!(
        compiled
            .layout
            .get_node(selected_bg_id)
            .unwrap()
            .properties
            .get("color"),
        Some(&Value::Color("#9c27b01f".to_string()))
    );

    // 4. Disabled ListItemButton propagates disabled text color (#00000061) to child ListItemText and suppresses click
    let dis_text_node = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.parent == Some(txt_disabled_id) && n.name == "Text")
        .unwrap();
    assert_eq!(
        dis_text_node.properties.get("color"),
        Some(&Value::Color("#00000061".to_string()))
    );

    let mut click_dis = Event::new(
        EventKind::Click {
            button: MouseButton::Left,
        },
        Point::new(30.0, 130.0),
        Point::new(20.0, 12.0),
        Modifiers::default(),
        disabled_bg_id,
    );
    compiled.dispatch_event(&mut click_dis).unwrap();
    assert_eq!(clicks.load(Ordering::SeqCst), 1);
}

#[test]
fn test_viewer_focus_and_nested_control_clicks_in_list() {
    use directedtype::render::{ViewerApp, ViewerConfig};
    use winit::event::ElementState;

    #[derive(Default)]
    struct NavApp {
        row_clicks: Arc<AtomicUsize>,
    }

    #[component]
    impl NavApp {
        pub fn on_row_click(&mut self, _ctx: &mut Context<'_>) -> Result<(), DispatchError> {
            self.row_clicks.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    let input = r#"
    \use "components/material/Switch.dt"
    \use "components/material/List.dt"

    \Component NavApp {
        \List(x: 20px, y: 20px, width: 320px) {
            \ListItemButton("btn_row"; on_click: on_row_click, secondary_action: true) {
                \ListItemText("row_txt"; primary: "Wi-Fi Settings", secondary: "Tap row or toggle switch")
                \Switch("row_sw";)
            }
            \ListItemButton("btn_dis"; disabled: true, on_click: on_row_click) {
                \ListItemText(primary: "Disabled Row")
            }
        }
    }

    \NavApp("app";)
    "#;

    let row_clicks = Arc::new(AtomicUsize::new(0));
    let rc_clone = Arc::clone(&row_clicks);

    let doc = parse(input).expect("parse ok");
    let mut registry = ComponentRegistry::standard();
    registry.register("NavApp", move || {
        Box::new(NavApp {
            row_clicks: Arc::clone(&rc_clone),
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

    let btn_row_id = compiled
        .find_by_key(None, &ComponentKey::string("btn_row"))
        .unwrap()
        .id;
    let row_txt_id = compiled
        .find_by_key(None, &ComponentKey::string("row_txt"))
        .unwrap()
        .id;
    let row_sw_id = compiled
        .find_by_key(None, &ComponentKey::string("row_sw"))
        .unwrap()
        .id;

    let mut viewer = ViewerApp::new_with_compiled(compiled, ViewerConfig::default());

    // 1. Focusable nodes include enabled ListItemButton and trailing Switch, excluding disabled ListItemButton
    assert_eq!(viewer.focusable_nodes(), vec![btn_row_id, row_sw_id]);

    // 2. Clicking on the ListItemText inside ListItemButton focuses ListItemButton and invokes on_row_click
    let txt_rect = viewer.layout().get_node(row_txt_id).unwrap().rect;
    let txt_pt = Point::new(txt_rect.x + 10.0, txt_rect.y + 10.0);
    viewer.dispatch_mouse_input(ElementState::Pressed, MouseButton::Left, txt_pt);
    viewer.dispatch_mouse_input(ElementState::Released, MouseButton::Left, txt_pt);
    assert_eq!(viewer.focused_node(), Some(btn_row_id));
    assert_eq!(row_clicks.load(Ordering::SeqCst), 1);

    // 3. Clicking on the trailing Switch toggles the Switch without firing ListItemButton's on_row_click
    let sw_rect = viewer.layout().get_node(row_sw_id).unwrap().rect;
    let sw_pt = Point::new(sw_rect.x + sw_rect.width / 2.0, sw_rect.y + sw_rect.height / 2.0);
    viewer.dispatch_mouse_input(ElementState::Pressed, MouseButton::Left, sw_pt);
    viewer.dispatch_mouse_input(ElementState::Released, MouseButton::Left, sw_pt);
    assert_eq!(viewer.focused_node(), Some(row_sw_id));
    assert_eq!(
        viewer.compiled().unwrap().get_state(row_sw_id, "is_checked"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        row_clicks.load(Ordering::SeqCst),
        1,
        "Clicking nested Switch must stop propagation and not fire enclosing ListItemButton on_click"
    );
}

#[test]
fn test_material_gallery_lists_section_compiles() {
    let gallery_src = std::fs::read_to_string("examples/material_gallery.dt").unwrap();
    let doc = parse(&gallery_src).expect("gallery parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        1024.0,
        1600.0,
        Path::new("examples"),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("gallery compile ok");

    for key in [
        "card_lists",
        "list_nav",
        "list_btn_inbox",
        "list_btn_drafts",
        "list_btn_starred",
        "list_btn_trash",
        "list_folders",
        "list_item_photos",
        "list_item_work",
        "list_item_vacation",
        "list_settings",
        "list_item_wifi",
        "list_sw_wifi",
        "list_item_sync",
        "list_cb_sync",
    ] {
        let node = compiled
            .find_by_key(None, &ComponentKey::string(key))
            .unwrap_or_else(|| panic!("expected gallery key '{}' to exist", key));
        assert!(node.rect.width > 0.0);
        assert!(node.rect.height > 0.0);
    }
}

#[test]
fn test_material_list_headless_render_pixels() {
    use directedtype::render::headless::HeadlessRenderer;
    use directedtype::render::scene::SceneOptions;

    let input = r#"
    \use "components/material/Palette.dt"
    \use "components/material/List.dt"

    \Rect(x: 0px, y: 0px, width: 320px, height: 160px, color: #ffffff)
    \List("list"; x: 10px, y: 10px, width: 300px, disable_padding: true) {
        \ListItemButton("sel_btn"; selected: true) {
            \ListItemAvatar(label: "A", bg: material_primary.main)
            \ListItemText(primary: "Primary Item", secondary: "Secondary line")
        }
    }
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        320.0,
        160.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let mut renderer = HeadlessRenderer::new().expect("init headless renderer");
    let img = renderer
        .render_layout(&compiled.layout, 320, 160, &SceneOptions::default())
        .expect("render ok");

    // 1. Avatar circle center at (10 + 16 + 20 = 46, 10 + 36 = 46) -> Primary blue (#1976d2)
    // Wait: center of Avatar (46, 46) has white letter "A", so sample upper-left of Avatar circle at (36, 36)
    let av_px = img.get_pixel(36, 36).0;
    assert!(
        av_px[0] < 45 && av_px[1] > 100 && av_px[2] > 190,
        "Expected primary blue pixel on ListItemAvatar at (36, 36), got {:?}",
        av_px
    );

    // 2. Selected ListItemButton background tint at empty right area (290, 46) -> tinted blue (#1976d214 over #ffffff -> B > R)
    let bg_px = img.get_pixel(290, 46).0;
    assert!(
        bg_px[2] > bg_px[0] && bg_px[0] < 252,
        "Expected selected blue tint on ListItemButton background at (290, 46), got {:?}",
        bg_px
    );
}

