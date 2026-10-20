// Comprehensive integration tests for Slider component, pointer capture, and typed callbacks.

use directedtype::compiler::{compile_document_with_registry, FsResolver};
use directedtype::component::{ComponentRegistry, Context, DispatchError};
use directedtype::interaction::{MouseButton, Point};
use directedtype::parse;
use directedtype::render::{ViewerApp, ViewerConfig};
use directedtype::Value;
use directedtype_macros::component;
use pretty_assertions::assert_eq;
use winit::event::ElementState;

#[derive(Default, Debug)]
struct EqualizerCompanion {
    pub bass: f64,
    pub treble: f64,
}

#[component]
impl EqualizerCompanion {
    pub fn on_mount(&mut self, ctx: &mut Context<'_>) {
        if let Some(b) = ctx.get_state_number("bass") {
            self.bass = b;
        }
        if let Some(t) = ctx.get_state_number("treble") {
            self.treble = t;
        }
    }

    pub fn set_bass(&mut self, val: f64, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        self.bass = val;
        ctx.set_state("bass", val);
        Ok(())
    }

    pub fn set_treble(&mut self, val: f64, ctx: &mut Context<'_>) -> Result<(), DispatchError> {
        self.treble = val;
        ctx.set_state("treble", val);
        Ok(())
    }
}

#[test]
fn test_slider_component_drag_and_callback() {
    let input = r#"
    \use "components/Slider.dt"

    \Component Equalizer() {
        \state bass: Float: 20;
        \state treble: Float: 80;

        \Slider(
            x: 50px,
            y: 50px,
            width: 200px,
            height: 24px,
            min: 0,
            max: 100,
            value: 20,
            on_change: set_bass
        )

        \Slider(
            x: 50px,
            y: 100px,
            width: 200px,
            height: 24px,
            min: 0,
            max: 100,
            value: 80,
            on_change: set_treble
        )
    }

    \Equalizer()
    "#;

    let doc = parse(input).expect("Parse equalizer dtml");
    let mut registry = ComponentRegistry::standard();
    registry.register("Equalizer", || Box::new(EqualizerCompanion::default()));

    let compiled = compile_document_with_registry(
        &doc,
        800.0,
        600.0,
        std::path::Path::new("."),
        &FsResolver,
        &registry,
    )
    .expect("Compile document ok");

    let mut app = ViewerApp::new_with_compiled(compiled, ViewerConfig::default());

    let eq_node = app
        .layout()
        .nodes
        .iter()
        .find(|n| n.name == "Equalizer")
        .expect("Equalizer node found")
        .id;

    let slider_nodes: Vec<_> = app
        .layout()
        .nodes
        .iter()
        .filter(|n| n.name == "Slider")
        .map(|n| n.id)
        .collect();
    assert_eq!(slider_nodes.len(), 2, "Must instantiate both Bass and Treble sliders");
    let bass_slider = slider_nodes[0];
    let treble_slider = slider_nodes[1];

    // Initial state check
    assert_eq!(
        app.compiled().unwrap().get_state(eq_node, "bass"),
        Some(&Value::Float(20.0))
    );
    assert_eq!(
        app.compiled().unwrap().get_state(eq_node, "treble"),
        Some(&Value::Float(80.0))
    );
    assert_eq!(
        app.compiled().unwrap().get_state(bass_slider, "current_value"),
        Some(&Value::Float(20.0))
    );
    assert_eq!(
        app.compiled().unwrap().get_state(treble_slider, "current_value"),
        Some(&Value::Float(80.0))
    );

    // 1. Click Bass slider at (150, 62):
    // Slider x is 50, width is 200. Cursor at 150 is local_x = 100.
    // Ratio = 100 / 200 = 0.5 -> 50.0!
    app.dispatch_mouse_input(ElementState::Pressed, MouseButton::Left, Point::new(150.0, 62.0));

    // Pointer capture must be active on the bass slider's hit rect
    assert!(app.captured_node().is_some());

    // Both Bass slider current_value and Equalizer bass state should be 50.0
    assert_eq!(
        app.compiled().unwrap().get_state(bass_slider, "current_value"),
        Some(&Value::Float(50.0))
    );
    assert_eq!(
        app.compiled().unwrap().get_state(eq_node, "bass"),
        Some(&Value::Float(50.0))
    );

    // 2. Drag Bass slider to (200, 62) -> local_x = 150 -> ratio = 150 / 200 = 0.75 -> 75.0
    app.dispatch_cursor_moved(Point::new(200.0, 62.0));
    assert_eq!(
        app.compiled().unwrap().get_state(bass_slider, "current_value"),
        Some(&Value::Float(75.0))
    );
    assert_eq!(
        app.compiled().unwrap().get_state(eq_node, "bass"),
        Some(&Value::Float(75.0))
    );

    // 3. Drag far beyond right bounds to (400, 62) -> clamped to 100.0
    app.dispatch_cursor_moved(Point::new(400.0, 62.0));
    assert_eq!(
        app.compiled().unwrap().get_state(bass_slider, "current_value"),
        Some(&Value::Float(100.0))
    );
    assert_eq!(
        app.compiled().unwrap().get_state(eq_node, "bass"),
        Some(&Value::Float(100.0))
    );

    // 4. Release mouse button -> pointer capture released
    app.dispatch_mouse_input(ElementState::Released, MouseButton::Left, Point::new(400.0, 62.0));
    assert_eq!(app.captured_node(), None);

    // 5. Now interact with the Treble slider at (100, 112):
    // Slider x is 50, width is 200. Cursor at 100 is local_x = 50.
    // Ratio = 50 / 200 = 0.25 -> 25.0!
    app.dispatch_mouse_input(ElementState::Pressed, MouseButton::Left, Point::new(100.0, 112.0));
    assert!(app.captured_node().is_some());

    assert_eq!(
        app.compiled().unwrap().get_state(treble_slider, "current_value"),
        Some(&Value::Float(25.0))
    );
    assert_eq!(
        app.compiled().unwrap().get_state(eq_node, "treble"),
        Some(&Value::Float(25.0))
    );

    // Drag Treble to far left (-50, 112) -> clamped to 0.0
    app.dispatch_cursor_moved(Point::new(-50.0, 112.0));
    assert_eq!(
        app.compiled().unwrap().get_state(treble_slider, "current_value"),
        Some(&Value::Float(0.0))
    );
    assert_eq!(
        app.compiled().unwrap().get_state(eq_node, "treble"),
        Some(&Value::Float(0.0))
    );

    app.dispatch_mouse_input(ElementState::Released, MouseButton::Left, Point::new(-50.0, 112.0));
    assert_eq!(app.captured_node(), None);

    // Bass slider state remains unchanged at 100.0 while Treble was dragged
    assert_eq!(
        app.compiled().unwrap().get_state(eq_node, "bass"),
        Some(&Value::Float(100.0))
    );
}

#[derive(Default, Debug)]
struct AudioMixerTestCompanion {
    pub master: f64,
    pub bass: f64,
    pub treble: f64,
}

#[component]
impl AudioMixerTestCompanion {
    pub fn on_mount(&mut self, ctx: &mut Context<'_>) {
        if let Some(m) = ctx.get_state_number("master") {
            self.master = m;
        }
        if let Some(b) = ctx.get_state_number("bass") {
            self.bass = b;
        }
        if let Some(t) = ctx.get_state_number("treble") {
            self.treble = t;
        }
    }

    pub fn set_master(&mut self, val: f64, ctx: &mut Context<'_>) {
        let rounded = val.round();
        self.master = rounded;
        ctx.set_state("master", rounded);
    }

    pub fn set_bass(&mut self, val: f64, ctx: &mut Context<'_>) {
        let rounded = val.round();
        self.bass = rounded;
        ctx.set_state("bass", rounded);
    }

    pub fn set_treble(&mut self, val: f64, ctx: &mut Context<'_>) {
        let rounded = val.round();
        self.treble = rounded;
        ctx.set_state("treble", rounded);
    }
}

#[test]
fn test_slider_demo_visualizer_rectangle() {
    let input = std::fs::read_to_string("examples/slider_demo.dt").expect("read slider_demo.dt");
    let doc = parse(&input).expect("parse ok");
    let mut registry = ComponentRegistry::standard();
    registry.register("AudioMixer", || Box::new(AudioMixerTestCompanion::default()));

    let compiled = compile_document_with_registry(
        &doc,
        800.0,
        600.0,
        std::path::Path::new("examples"),
        &FsResolver,
        &registry,
    )
    .expect("compile ok");

    let mut app = ViewerApp::new_with_compiled(compiled, ViewerConfig::default());

    let mixer_node = app
        .layout()
        .nodes
        .iter()
        .find(|n| n.name == "AudioMixer")
        .expect("AudioMixer node found")
        .id;

    // Verify initial states
    assert_eq!(
        app.compiled().unwrap().get_state(mixer_node, "master"),
        Some(&Value::Float(75.0))
    );
    assert_eq!(
        app.compiled().unwrap().get_state(mixer_node, "bass"),
        Some(&Value::Float(40.0))
    );
    assert_eq!(
        app.compiled().unwrap().get_state(mixer_node, "treble"),
        Some(&Value::Float(60.0))
    );

    // Find the visualizer Rect node (it's the Rect inside AudioMixer that has radius 8px)
    let visualizer_rect = app
        .layout()
        .nodes
        .iter()
        .find(|n| {
            n.name == "Rect"
                && n.properties.get("radius") == Some(&Value::Distance(8.0))
        })
        .expect("Visualizer Rect node found")
        .clone();

    // Initial dimensions: width = 40 + 40 * 3.6 = 184, height = 20 + 60 * 0.8 = 68
    assert_eq!(visualizer_rect.rect.width, 184.0);
    assert_eq!(visualizer_rect.rect.height, 68.0);
    // Initial color: hsl(75 * 3.6, 85, 55) = hsl(270, 85, 55) -> purple
    if let Some(Value::Color(c)) = visualizer_rect.properties.get("color") {
        assert!(c.starts_with('#'), "color should be hex string: {}", c);
    } else {
        panic!("Visualizer Rect must have a color property");
    }

    // Now mutate states through dispatch_callback
    // Mutate bass to 100 -> width becomes 40 + 100 * 3.6 = 400.0!
    app.compiled_mut()
        .unwrap()
        .dispatch_callback(mixer_node, "set_bass", Value::Float(100.0))
        .expect("dispatch set_bass ok");

    let updated_rect = app
        .layout()
        .nodes
        .iter()
        .find(|n| n.id == visualizer_rect.id)
        .expect("Visualizer Rect node found");
    assert_eq!(updated_rect.rect.width, 400.0);

    // Mutate treble to 10 -> height becomes 20 + 10 * 0.8 = 28.0!
    app.compiled_mut()
        .unwrap()
        .dispatch_callback(mixer_node, "set_treble", Value::Float(10.0))
        .expect("dispatch set_treble ok");

    let updated_rect = app
        .layout()
        .nodes
        .iter()
        .find(|n| n.id == visualizer_rect.id)
        .expect("Visualizer Rect node found");
    assert_eq!(updated_rect.rect.height, 28.0);

    // Mutate master to 0 -> hue becomes hsl(0, 85, 55) -> red!
    app.compiled_mut()
        .unwrap()
        .dispatch_callback(mixer_node, "set_master", Value::Float(0.0))
        .expect("dispatch set_master ok");

    let updated_rect = app
        .layout()
        .nodes
        .iter()
        .find(|n| n.id == visualizer_rect.id)
        .expect("Visualizer Rect node found");
    let color_val = updated_rect.properties.get("color").unwrap();
    if let Value::Color(hex) = color_val {
        assert!(hex.starts_with("#e") || hex.starts_with("#f") || hex.starts_with("#d"), "Hue 0 should be red, got: {}", hex);
    }
}

#[test]
fn test_slider_content_shrink_auto_scrolls_up() {
    let source = std::fs::read_to_string("examples/slider_demo.dt").expect("read slider_demo.dt");
    let doc = parse(&source).expect("parse slider_demo.dt");
    let mut registry = ComponentRegistry::standard();
    registry.register("AudioMixer", || Box::new(AudioMixerTestCompanion::default()));

    let compiled = compile_document_with_registry(
        &doc,
        800.0,
        400.0, // Short window so content overflows and scrolls
        std::path::Path::new("examples"),
        &FsResolver,
        &registry,
    )
    .expect("compile slider_demo.dt ok");

    let mut app = ViewerApp::new_with_compiled(
        compiled,
        ViewerConfig {
            width: 800,
            height: 400,
            show_nav_bar: false,
            ..Default::default()
        },
    );

    let mixer_node = app
        .layout()
        .nodes
        .iter()
        .find(|n| n.name == "AudioMixer")
        .expect("AudioMixer node found")
        .id;

    let initial_content_h = app.content_height();
    assert!(initial_content_h > 400.0, "Content must exceed viewport height");

    // Scroll to the bottom:
    app.set_scroll_y(2000.0); // Clamps to max_scroll_y
    let scroll_at_bottom = app.scroll_y();
    assert_eq!(scroll_at_bottom, initial_content_h - 400.0);

    // Now mutate treble from 60 to 0 -> visualizer shrinks, card shrinks, background rect shrinks!
    app.compiled_mut()
        .unwrap()
        .dispatch_callback(mixer_node, "set_treble", Value::Float(0.0))
        .expect("dispatch set_treble ok");
    app.clamp_scroll();

    let new_content_h = app.content_height();
    assert!(new_content_h < initial_content_h, "Content height should have shrunk");

    // Viewport must automatically scroll up so viewport bottom does not exceed content bottom:
    assert_eq!(app.scroll_y(), new_content_h - 400.0);
    assert_eq!(app.scroll_y() + 400.0, new_content_h);
}


