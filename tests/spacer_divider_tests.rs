use directedtype::compiler::compiled::CompiledDocument;
use directedtype::component::ComponentRegistry;
use directedtype::parse;
use std::path::Path;

#[test]
fn test_vertical_spacer_in_vstack() {
    let input = r#"
    \use "components/VStack.dt"
    \use "components/Spacer.dt"

    \VStack(x: 20px, y: 20px, width: 200px, gap: 10px) {
        \Rect(width: 100px, height: 40px, color: #3b82f6)
        \Spacer(height: 35px)
        \Rect(width: 100px, height: 40px, color: #10b981)
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

    let rects: Vec<_> = compiled.layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 2);

    // Rect 1: y = 20, height = 40 -> bottom = 60
    assert_eq!(rects[0].rect.y, 20.0);
    assert_eq!(rects[0].rect.height, 40.0);

    // Spacer: y = 60 + gap(10) = 70, height = 35 -> bottom = 105
    // Rect 2: y = 105 + gap(10) = 115
    assert_eq!(rects[1].rect.y, 115.0);
    assert_eq!(rects[1].rect.height, 40.0);
}

#[test]
fn test_horizontal_spacer_in_hstack() {
    let input = r#"
    \use "components/HStack.dt"
    \use "components/Spacer.dt"

    \HStack(x: 10px, y: 10px, gap: 12px) {
        \Rect(width: 50px, height: 30px, color: #3b82f6)
        \Spacer(width: 48px)
        \Rect(width: 50px, height: 30px, color: #10b981)
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

    let rects: Vec<_> = compiled.layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 2);

    // Rect 1: x = 10, width = 50 -> right = 60
    assert_eq!(rects[0].rect.x, 10.0);
    assert_eq!(rects[0].rect.width, 50.0);

    // Spacer: x = 60 + gap(12) = 72, width = 48 -> right = 120
    // Rect 2: x = 120 + gap(12) = 132
    assert_eq!(rects[1].rect.x, 132.0);
    assert_eq!(rects[1].rect.width, 50.0);
}

#[test]
fn test_divider_in_vstack() {
    let input = r#"
    \use "components/VStack.dt"
    \use "components/Divider.dt"

    \VStack(x: 0, y: 0, width: 300px, gap: 8px) {
        \Rect(width: 100px, height: 20px, color: #3b82f6)
        \Divider()
        \Rect(width: 100px, height: 20px, color: #10b981)
        \Divider(width: 120px, color: #ef4444, thickness: 2px)
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

    let rects: Vec<_> = compiled.layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    // 2 content rects + 2 divider rects = 4 rects
    assert_eq!(rects.len(), 4);

    // Content Rect 1
    assert_eq!(rects[0].rect.y, 0.0);
    assert_eq!(rects[0].rect.height, 20.0);

    // Fluid Divider 1: width matches parent (300), height = 1
    assert_eq!(rects[1].rect.y, 28.0); // 20 + 8
    assert_eq!(rects[1].rect.height, 1.0);
    assert_eq!(rects[1].rect.width, 300.0);

    // Content Rect 2
    assert_eq!(rects[2].rect.y, 37.0); // 29 + 8

    // Fixed Divider 2: width = 120, thickness = 2
    assert_eq!(rects[3].rect.y, 65.0); // 37 + 20 + 8
    assert_eq!(rects[3].rect.height, 2.0);
    assert_eq!(rects[3].rect.width, 120.0);
}

#[test]
fn test_line_primitive_and_caps() {
    let input = r#"
    \Line(x1: 100px, y1: 50px, x2: 20px, y2: 110px, thickness: 3px, color: #3b82f6, cap: LineCap.Round)
    \Line(x1: 10px, y1: 10px, x2: 50px, y2: 10px, thickness: 2px, cap: LineCap.Square)
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

    let lines: Vec<_> = compiled.layout.nodes.iter().filter(|n| n.name == "Line").collect();
    assert_eq!(lines.len(), 2);

    assert_eq!(lines[0].rect.x, 20.0);
    assert_eq!(lines[0].rect.y, 50.0);
    assert_eq!(lines[0].rect.width, 80.0);
    assert_eq!(lines[0].rect.height, 60.0);

    assert_eq!(lines[1].rect.x, 10.0);
    assert_eq!(lines[1].rect.y, 10.0);
    assert_eq!(lines[1].rect.width, 40.0);
    assert_eq!(lines[1].rect.height, 0.0);
}

#[test]
fn test_arrow_component() {
    let input = r#"
    \use "components/Arrow.dt"

    \let a1 = \Arrow(
        x1: 100px,
        y1: 40px,
        x2: 100px,
        y2: 120px,
        thickness: 2px,
        color: #1e293b
    );

    \let a2 = \Arrow(
        x1: 50px,
        y1: 50px,
        x2: 150px,
        y2: 100px,
        thickness: 3px,
        head_length: 12px,
        head_width: 10px,
        cap: LineCap.Butt
    );
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

    let lines: Vec<_> = compiled.layout.nodes.iter().filter(|n| n.name == "Line").collect();
    // Each Arrow expands to 3 Line primitives (1 shaft + 2 arrowhead wings)
    assert_eq!(lines.len(), 6);

    // First arrow is vertical from (100, 40) to (100, 120) with head_length = max(8, 2*4) = 8, head_width = 8
    // Wing 1: (96, 112) -> (100, 120), Wing 2: (104, 112) -> (100, 120)
    assert_eq!(lines[0].properties.get("x1").and_then(|v| v.as_f64()), Some(100.0));
    assert_eq!(lines[0].properties.get("y1").and_then(|v| v.as_f64()), Some(40.0));
    assert_eq!(lines[0].properties.get("x2").and_then(|v| v.as_f64()), Some(100.0));
    assert_eq!(lines[0].properties.get("y2").and_then(|v| v.as_f64()), Some(120.0));

    assert_eq!(lines[1].properties.get("x1").and_then(|v| v.as_f64()), Some(96.0));
    assert_eq!(lines[1].properties.get("y1").and_then(|v| v.as_f64()), Some(112.0));
    assert_eq!(lines[2].properties.get("x1").and_then(|v| v.as_f64()), Some(104.0));
    assert_eq!(lines[2].properties.get("y1").and_then(|v| v.as_f64()), Some(112.0));
}
