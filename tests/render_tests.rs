use directedtype::compiler::evaluate_document;
use directedtype::parser::parse_document;
use directedtype::render::{HeadlessRenderer, SceneOptions, ViewerApp, ViewerConfig};
use vello::peniko::Color;

#[test]
fn test_headless_render_layout_to_image() {
    let source = r#"
\Component Flow(gap: Number: 16) {
    \Children {
        x: parent.left,
        y: prev ? prev.bottom + gap : parent.top
    }
}

\Flow(gap: 20) {
    \Rect(x: 20, y: 20, width: 200, height: 60, color: #ff0000)
    \Text(x: 20, y: 100, width: 200, size: 20, color: #0000ff) {
        Hello DirectedType Vello
    }
}
"#;

    let doc = parse_document(source).expect("Failed to parse document");
    let layout = evaluate_document(&doc).expect("Failed to evaluate layout");

    let mut renderer = HeadlessRenderer::new().expect("Failed to initialize HeadlessRenderer");

    let options = SceneOptions {
        background: Some(Color::WHITE),
        ..Default::default()
    };

    let img = renderer
        .render_layout(&layout, 400, 300, &options)
        .expect("Failed to render layout to image");

    assert_eq!(img.width(), 400);
    assert_eq!(img.height(), 300);

    // Pixel at (50, 40) is inside the red Rect at (20, 20, 200, 60)
    let pixel = img.get_pixel(50, 40);
    // Should be red (R high, G low, B low, A 255)
    assert!(pixel[0] > 200, "Expected red pixel, got: {:?}", pixel);
    assert!(pixel[1] < 50, "Expected low green, got: {:?}", pixel);
    assert!(pixel[2] < 50, "Expected low blue, got: {:?}", pixel);
    assert_eq!(pixel[3], 255);

    // Background pixel at (350, 250) should be white
    let bg_pixel = img.get_pixel(350, 250);
    assert!(bg_pixel[0] > 240, "Expected white bg, got: {:?}", bg_pixel);
    assert!(bg_pixel[1] > 240, "Expected white bg, got: {:?}", bg_pixel);
    assert!(bg_pixel[2] > 240, "Expected white bg, got: {:?}", bg_pixel);
}

#[test]
fn test_headless_render_to_png_file() {
    let source = r#"
\Component ShadedBox(bg_color: Color: #333333, width: max(children.width) + 32, height: sum(children.height) + 32) {
    \Rect(
        x: x,
        y: y,
        width: width,
        height: height,
        color: bg_color,
        radius: 8
    )
    \Children {
        x: x + 16,
        y: y + 16
    }
}

\ShadedBox(bg_color: #224488) {
    \Text(width: 250, size: 18, color: #ffffff) {
        DirectedType Headless PNG Rendering
    }
}
"#;

    let doc = parse_document(source).expect("Failed to parse document");
    let layout = evaluate_document(&doc).expect("Failed to evaluate layout");

    let mut renderer = HeadlessRenderer::new().expect("Failed to initialize HeadlessRenderer");

    let temp_dir = std::env::temp_dir();
    let png_path = temp_dir.join("directedtype_test_render.png");

    let options = SceneOptions {
        background: Some(Color::from_rgba8(240, 240, 240, 255)),
        ..Default::default()
    };

    renderer
        .render_layout_to_file(&layout, 500, 300, &options, &png_path)
        .expect("Failed to render to PNG file");

    assert!(png_path.exists(), "PNG file was not created");
    let metadata = std::fs::metadata(&png_path).expect("Failed to read PNG metadata");
    assert!(metadata.len() > 0, "PNG file is empty");

    // Clean up temporary file
    let _ = std::fs::remove_file(png_path);
}

#[test]
fn test_viewer_hot_reload_document() {
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join(format!("dt_hot_reload_test_{}.dt", std::process::id()));

    // 1. Initial version: Rect with width 100
    let v1 = r#"\Rect(x: 10, y: 10, width: 100, height: 50, color: #ff0000)"#;
    std::fs::write(&test_file, v1).expect("Failed to write v1 test file");

    let doc1 = parse_document(v1).expect("Failed to parse v1");
    let layout1 = evaluate_document(&doc1).expect("Failed to evaluate v1");

    let mut app = ViewerApp::new(layout1, ViewerConfig::default())
        .with_document(doc1)
        .with_watch_path(test_file.clone());

    assert_eq!(app.layout().nodes.len(), 1);
    assert_eq!(app.layout().nodes[0].rect.width, 100.0);

    // 2. Updated version: Rect with width 350
    let v2 = r#"\Rect(x: 10, y: 10, width: 350, height: 50, color: #00ff00)"#;
    std::fs::write(&test_file, v2).expect("Failed to write v2 test file");

    app.reload_document();

    assert_eq!(app.layout().nodes.len(), 1);
    assert_eq!(app.layout().nodes[0].rect.width, 350.0);

    // 3. Invalid syntax version: Must not crash, must retain last valid layout
    let v_invalid_syntax = r#"\Rect(x: 10, y: 10, width: { invalid syntax"#;
    std::fs::write(&test_file, v_invalid_syntax).expect("Failed to write invalid syntax");

    app.reload_document();

    // Still retains width 350.0
    assert_eq!(app.layout().nodes.len(), 1);
    assert_eq!(app.layout().nodes[0].rect.width, 350.0);

    // 4. Cyclic dependency version: Must not crash, must retain last valid layout
    let v_cycle = r#"
\Component ParadoxBox(width: max(children.width)) {
  \Children {
    x: 0,
    y: 0,
    width: parent.width
  }
}

\ParadoxBox {
  \Rect(height: 50, color: #000)
}
"#;
    std::fs::write(&test_file, v_cycle).expect("Failed to write cyclic layout");

    app.reload_document();

    // Still retains width 350.0
    assert_eq!(app.layout().nodes.len(), 1);
    assert_eq!(app.layout().nodes[0].rect.width, 350.0);

    // Clean up
    let _ = std::fs::remove_file(test_file);
}

#[test]
fn test_headless_render_clipping() {
    let source = r#"
\Component ScrollPane {
    let clip = \Clip(up: self.clip, box: \Box(x: 20, y: 20, width: 60, height: 60))
    \Rect(clip: clip, x: 20, y: 20, width: 200, height: 200, color: #ff0000)
}

\ScrollPane()
"#;

    let doc = parse_document(source).expect("Failed to parse document");
    let layout = evaluate_document(&doc).expect("Failed to evaluate layout");

    let mut renderer = HeadlessRenderer::new().expect("Failed to initialize HeadlessRenderer");

    let options = SceneOptions {
        background: Some(Color::WHITE),
        ..Default::default()
    };

    let img = renderer
        .render_layout(&layout, 300, 300, &options)
        .expect("Failed to render layout to image");

    // Pixel at (40, 40) is INSIDE the clip box (20..80, 20..80):
    // Should be red (R high, G low, B low)
    let inside_pixel = img.get_pixel(40, 40);
    assert!(inside_pixel[0] > 200, "Expected red pixel inside clip, got: {:?}", inside_pixel);
    assert!(inside_pixel[1] < 50, "Expected low green, got: {:?}", inside_pixel);
    assert!(inside_pixel[2] < 50, "Expected low blue, got: {:?}", inside_pixel);

    // Pixel at (100, 100) is OUTSIDE the clip box (20..80, 20..80),
    // but INSIDE the Rect's natural boundary (20..220, 20..220):
    // Because it is clipped, it must NOT be red! It must remain white background!
    let outside_pixel = img.get_pixel(100, 100);
    assert!(outside_pixel[0] > 240, "Expected white bg for clipped pixel, got: {:?}", outside_pixel);
    assert!(outside_pixel[1] > 240, "Expected white bg for clipped pixel, got: {:?}", outside_pixel);
    assert!(outside_pixel[2] > 240, "Expected white bg for clipped pixel, got: {:?}", outside_pixel);
}

#[test]
fn test_clipping_example_file() {
    let source = std::fs::read_to_string("examples/clipping.dt")
        .expect("Failed to read examples/clipping.dt");
    let doc = parse_document(&source).expect("Failed to parse examples/clipping.dt");
    let layout = evaluate_document(&doc).expect("Failed to evaluate examples/clipping.dt");

    let mut renderer = HeadlessRenderer::new().expect("Failed to initialize HeadlessRenderer");
    let options = SceneOptions {
        background: Some(Color::WHITE),
        ..Default::default()
    };

    let img = renderer
        .render_layout(&layout, 600, 600, &options)
        .expect("Failed to render examples/clipping.dt");

    assert_eq!(img.width(), 600);
    assert_eq!(img.height(), 600);

    let brain_dir = std::path::Path::new("/Users/hoefel/.gemini/antigravity-ide/brain/c2560240-cb95-4f78-9867-aa0ab7be2601");
    if brain_dir.exists() {
        let _ = img.save(brain_dir.join("clipping_render.png"));
    }
}

#[test]
fn test_env_example_file() {
    let source = std::fs::read_to_string("examples/env_demo.dt")
        .expect("Failed to read examples/env_demo.dt");
    let doc = parse_document(&source).expect("Failed to parse examples/env_demo.dt");
    let layout = evaluate_document(&doc).expect("Failed to evaluate examples/env_demo.dt");

    let mut renderer = HeadlessRenderer::new().expect("Failed to initialize HeadlessRenderer");
    let options = SceneOptions {
        background: Some(Color::WHITE),
        ..Default::default()
    };

    let img = renderer
        .render_layout(&layout, 750, 700, &options)
        .expect("Failed to render examples/env_demo.dt");

    assert_eq!(img.width(), 750);
    assert_eq!(img.height(), 700);

    let brain_dir = std::path::Path::new("/Users/hoefel/.gemini/antigravity-ide/brain/c2560240-cb95-4f78-9867-aa0ab7be2601");
    if brain_dir.exists() {
        let _ = img.save(brain_dir.join("env_demo_render.png"));
    }
}

#[test]
fn test_binding_example_file() {
    let source = std::fs::read_to_string("examples/binding.dt")
        .expect("Failed to read examples/binding.dt");
    let doc = parse_document(&source).expect("Failed to parse examples/binding.dt");
    let layout = evaluate_document(&doc).expect("Failed to evaluate examples/binding.dt");

    let mut renderer = HeadlessRenderer::new().expect("Failed to initialize HeadlessRenderer");
    let options = SceneOptions {
        background: Some(Color::WHITE),
        ..Default::default()
    };

    let img = renderer
        .render_layout(&layout, 400, 500, &options)
        .expect("Failed to render examples/binding.dt");

    assert_eq!(img.width(), 400);
    assert_eq!(img.height(), 500);

    let brain_dir = std::path::Path::new("/Users/hoefel/.gemini/antigravity-ide/brain/c2560240-cb95-4f78-9867-aa0ab7be2601");
    if brain_dir.exists() {
        let _ = img.save(brain_dir.join("binding_render.png"));
    }
}



