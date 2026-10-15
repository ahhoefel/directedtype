use directedtype::compiler::compiled::CompiledDocument;
use directedtype::component::ComponentRegistry;
use directedtype::parse;
use std::path::Path;

#[test]
fn test_padout_uniform_wrapping_rect() {
    let input = r#"
    \use "components/PadOut.dt"

    \PadOut(padding: 20, x: 10, y: 15) {
        \Rect(width: 100, height: 60, color: #3b82f6)
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

    let rect = compiled.layout.nodes.iter().find(|n| n.name == "Rect").expect("Rect found");
    assert_eq!(rect.rect.x, 30.0); // 10 + 20
    assert_eq!(rect.rect.y, 35.0); // 15 + 20
    assert_eq!(rect.rect.width, 100.0);
    assert_eq!(rect.rect.height, 60.0);

    let padout_node = compiled.layout.nodes.iter().find(|n| n.name == "PadOut").expect("PadOut found");
    assert_eq!(padout_node.rect.width, 140.0); // 100 + 2 * 20
    assert_eq!(padout_node.rect.height, 100.0); // 60 + 2 * 20
}

#[test]
fn test_padin_uniform_insetting_child() {
    let input = r#"
    \use "components/PadIn.dt"

    \PadIn(padding: 20, x: 10, y: 15) {
        \Rect(color: #3b82f6)
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

    let rect = compiled.layout.nodes.iter().find(|n| n.name == "Rect").expect("Rect found");
    assert_eq!(rect.rect.x, 30.0); // 10 + 20
    assert_eq!(rect.rect.y, 35.0); // 15 + 20
    // available_width = 800 - 2 * 10 = 780; inner_width = 780 - 40 = 740
    assert_eq!(rect.rect.width, 740.0);
    // available_height = 600 - 2 * 15 = 570; inner_height = 570 - 40 = 530
    assert_eq!(rect.rect.height, 530.0);

    let padin_node = compiled.layout.nodes.iter().find(|n| n.name == "PadIn").expect("PadIn found");
    assert_eq!(padin_node.rect.width, 780.0);
    assert_eq!(padin_node.rect.height, 570.0);
}

#[test]
fn test_padout_asymmetric_wrapping_hstack() {
    let input = r#"
    \use "components/PadOut.dt"
    \use "components/HStack.dt"

    \PadOut(padding_x: 20, padding_y: 24, x: 10, y: 20) {
        \HStack(gap: 16) {
            \Rect(width: 100, height: 40, color: #3b82f6)
            \Rect(width: 150, height: 50, color: #10b981)
            \Rect(width: 80, height: 30, color: #f59e0b)
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

    let rects: Vec<_> = compiled.layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 3);

    assert_eq!(rects[0].rect.x, 30.0);
    assert_eq!(rects[0].rect.y, 44.0);

    // rect 1: 30 + 100 + 16 = 146
    assert_eq!(rects[1].rect.x, 146.0);
    assert_eq!(rects[1].rect.y, 44.0);

    // rect 2: 146 + 150 + 16 = 312
    assert_eq!(rects[2].rect.x, 312.0);
    assert_eq!(rects[2].rect.y, 44.0);

    let padout_node = compiled.layout.nodes.iter().find(|n| n.name == "PadOut").expect("PadOut found");
    // HStack width = 100 + 150 + 80 + 32 = 362. PadOut width = 362 + 40 = 402.
    assert_eq!(padout_node.rect.width, 402.0);
    // HStack height = 50. PadOut height = 50 + 48 = 98.
    assert_eq!(padout_node.rect.height, 98.0);
}

#[test]
fn test_padin_asymmetric_insetting_vstack() {
    let input = r#"
    \use "components/PadIn.dt"
    \use "components/VStack.dt"

    \PadIn(padding_x: 20, padding_y: 24, x: 10, y: 20) {
        \VStackStretch(gap: 16) {
            \Rect(height: 40, color: #3b82f6)
            \Rect(height: 50, color: #10b981)
            \Rect(height: 30, color: #f59e0b)
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

    let rects: Vec<_> = compiled.layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(rects.len(), 3);

    // Inner width: 800 - 20 (2*x) - 40 (2*padding_x) = 740
    assert_eq!(rects[0].rect.x, 30.0);
    assert_eq!(rects[0].rect.y, 44.0);

    // rect 1: 44 + 40 + 16 = 100
    assert_eq!(rects[1].rect.x, 30.0);
    assert_eq!(rects[1].rect.y, 100.0);

    // rect 2: 100 + 50 + 16 = 166
    assert_eq!(rects[2].rect.x, 30.0);
    assert_eq!(rects[2].rect.y, 166.0);

    let padin_node = compiled.layout.nodes.iter().find(|n| n.name == "PadIn").expect("PadIn found");
    assert_eq!(padin_node.rect.width, 780.0);
    assert_eq!(padin_node.rect.height, 560.0);
}

#[test]
fn test_padout_assertion_rejects_multiple_children() {
    let input = r#"
    \use "components/PadOut.dt"

    \PadOut(padding: 20) {
        \Rect(width: 100, height: 50, color: #ff0000)
        \Rect(width: 100, height: 50, color: #00ff00)
    }
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let result = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    );

    match result {
        Err(directedtype::compiler::error::CompileError::AssertionFailed { message, .. }) => {
            assert!(
                message.contains("Should have only one child.") || message.contains("only() expected exactly one child"),
                "unexpected assertion message: {}", message
            );
        }
        other => panic!("expected CompileError::AssertionFailed, got {:?}", other),
    }
}

#[test]
fn test_padin_assertion_rejects_multiple_children() {
    let input = r#"
    \use "components/PadIn.dt"

    \PadIn(padding: 20) {
        \Rect(width: 100, height: 50, color: #ff0000)
        \Rect(width: 100, height: 50, color: #00ff00)
    }
    "#;

    let doc = parse(input).expect("parse ok");
    let registry = ComponentRegistry::standard();
    let result = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    );

    match result {
        Err(directedtype::compiler::error::CompileError::AssertionFailed { message, .. }) => {
            assert!(
                message.contains("Should have only one child.") || message.contains("only() expected exactly one child"),
                "unexpected assertion message: {}", message
            );
        }
        other => panic!("expected CompileError::AssertionFailed, got {:?}", other),
    }
}
