use directedtype::compiler::evaluate_document;
use directedtype::parser::parse_document;
use directedtype::render::{HeadlessRenderer, SceneOptions};
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
