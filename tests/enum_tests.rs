use directedtype::ast::*;
use directedtype::compiler::error::CompileError;
use directedtype::compiler::{compile_document_with_window, compile_to_graph};
use directedtype::parse;
use pretty_assertions::assert_eq;

#[test]
fn test_enum_declaration_and_variant_usage() {
    let input = r#"
    \Enum Align {
        Left,
        Center,
        Right,
    }

    \Component Box(align: Align: Align.Left, x: Number: 0, y: Number: 0) {
        let is_center = align == Align.Center;
        let is_right = align == Align.Right;
        alias offset = is_center ? 50 : (is_right ? 100 : 0);
        \Rect(x: x + offset, y: y, width: 100, height: 100, color: #ff0000)
    }

    \Box(align: Align.Center)
    \Box(align: Align.Right)
    \Box()
    "#;

    let doc = parse(input).expect("Failed to parse");
    let (expanded, _graph) = compile_to_graph(&doc).expect("Failed to compile");

    assert_eq!(expanded.roots.len(), 3);

    let box1_id = expanded.roots[0];
    let rect1_id = expanded.get_node(box1_id).unwrap().children[0];

    let box2_id = expanded.roots[1];
    let rect2_id = expanded.get_node(box2_id).unwrap().children[0];

    let box3_id = expanded.roots[2];
    let rect3_id = expanded.get_node(box3_id).unwrap().children[0];

    let compiled = compile_document_with_window(&doc, 800.0, 600.0).expect("Failed to compile layout");
    let r1 = compiled.layout.get_node(rect1_id).unwrap().rect;
    let r2 = compiled.layout.get_node(rect2_id).unwrap().rect;
    let r3 = compiled.layout.get_node(rect3_id).unwrap().rect;

    assert_eq!(r1.x, 50.0);
    assert_eq!(r2.x, 100.0);
    assert_eq!(r3.x, 0.0);
}

#[test]
fn test_enum_unknown_variant_error() {
    let input = r#"
    \Enum Align {
        Left,
        Center,
    }

    \Component Box(align: Align) {
        \Rect(x: 0, y: 0, width: 10, height: 10, color: #000000)
    }

    \Box(align: Align.Bottom)
    "#;

    let doc = parse(input).expect("Failed to parse");
    let err = compile_to_graph(&doc).expect_err("Expected compile error");
    match err {
        CompileError::UnknownEnumVariant { enum_name, variant, .. } => {
            assert_eq!(enum_name, "Align");
            assert_eq!(variant, "Bottom");
        }
        other => panic!("Expected UnknownEnumVariant, got {:?}", other),
    }
}

#[test]
fn test_enum_bare_use_error() {
    let input = r#"
    \Enum Align {
        Left,
        Center,
    }

    \Component Box(align: Align) {
        \Rect(x: 0, y: 0, width: 10, height: 10, color: #000000)
    }

    \Box(align: Align)
    "#;

    let doc = parse(input).expect("Failed to parse");
    let err = compile_to_graph(&doc).expect_err("Expected compile error");
    match err {
        CompileError::BareEnumUse { name, .. } => {
            assert_eq!(name, "Align");
        }
        other => panic!("Expected BareEnumUse, got {:?}", other),
    }
}

#[test]
fn test_enum_type_mismatch_error() {
    let input = r#"
    \Enum Align {
        Left,
        Center,
    }

    \Component Box(align: Align) {
        \Rect(x: 0, y: 0, width: 10, height: 10, color: #000000)
    }

    \Box(align: "left")
    "#;

    let doc = parse(input).expect("Failed to parse");
    let err = compile_to_graph(&doc).expect_err("Expected compile error");
    match err {
        CompileError::TypeMismatch { expected, actual, .. } => {
            assert_eq!(expected, "Align");
            assert_eq!(actual, "String");
        }
        other => panic!("Expected TypeMismatch, got {:?}", other),
    }
}

#[test]
fn test_enum_duplicate_declaration_error() {
    let input = r#"
    \Enum Align {
        Left,
        Center,
    }

    \Enum Align {
        Top,
        Bottom,
    }

    \Rect(x: 0, y: 0, width: 10, height: 10, color: #000000)
    "#;

    let doc = parse(input).expect("Failed to parse");
    let err = compile_to_graph(&doc).expect_err("Expected compile error");
    match err {
        CompileError::DuplicateEnum { name, .. } => {
            assert_eq!(name, "Align");
        }
        other => panic!("Expected DuplicateEnum, got {:?}", other),
    }
}

#[test]
fn test_enum_import_and_aliased_import() {
    use directedtype::compiler::module::FileResolver;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    struct MemoryResolver {
        files: HashMap<PathBuf, String>,
    }

    impl FileResolver for MemoryResolver {
        fn read(&self, path: &Path) -> Result<String, String> {
            self.files
                .get(path)
                .cloned()
                .ok_or_else(|| format!("file not found: {:?}", path))
        }
    }

    let mut files = HashMap::new();
    files.insert(
        PathBuf::from("types/align.dt"),
        r#"
        \Enum Align {
            Start,
            Middle,
            End,
        }
        "#
        .to_string(),
    );

    let main_src = r#"
    \use "types/align.dt"
    \use "types/align.dt" as Layout

    \Component Card(align: Align: Align.Start, sub_align: Layout: Layout.Middle) {
        let is_middle = sub_align == Layout.Middle;
        alias w = is_middle ? 200 : 100;
        \Rect(x: 0, y: 0, width: w, height: 50, color: #ffffff)
    }

    \Card(align: Align.End, sub_align: Layout.Middle)
    "#;

    let resolver = MemoryResolver { files };
    let doc = parse(main_src).expect("Failed to parse main");
    let expanded = directedtype::compiler::expand_document_with_resolver(
        &doc,
        Path::new("."),
        &resolver,
    )
    .expect("Failed to expand document with memory resolver");

    assert_eq!(expanded.roots.len(), 1);
    let card_id = expanded.roots[0];
    let card = expanded.get_node(card_id).unwrap();
    let rect_id = card.children[0];
    let _rect = expanded.get_node(rect_id).unwrap();

    let card_w = card.ports.get("w").unwrap();
    match card_w {
        Expr::Literal(Literal::Number(n, _)) => assert_eq!(*n, 200.0),
        other => panic!("Expected Literal 200, got {:?}", other),
    }
}

#[test]
fn test_text_ends_at_baseline_and_descender() {
    let input = r#"
    \Text(size: 32, ends_at: TextEnd.Baseline) {
        Typography & Baseline
    }
    \Text(size: 32, ends_at: TextEnd.Descender) {
        Typography & Baseline
    }
    \Text(size: 32) {
        Typography & Baseline
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let layout = directedtype::evaluate_document(&doc).expect("Layout evaluation failed");

    assert_eq!(layout.nodes.len(), 3);
    let t_baseline = &layout.nodes[0];
    let t_descender = &layout.nodes[1];
    let t_default = &layout.nodes[2];

    assert_eq!(t_baseline.name, "Text");
    assert_eq!(t_descender.name, "Text");
    assert_eq!(t_default.name, "Text");

    // Check ends_at properties
    assert_eq!(
        t_baseline.properties.get("ends_at").map(|v| v.as_str().unwrap()),
        Some("Baseline")
    );
    assert_eq!(
        t_descender.properties.get("ends_at").map(|v| v.as_str().unwrap()),
        Some("Descender")
    );
    assert_eq!(
        t_default.properties.get("ends_at").map(|v| v.as_str().unwrap()),
        Some("Baseline")
    );

    // Baseline height is strictly less than Descender height
    assert!(
        t_baseline.rect.height < t_descender.rect.height,
        "Baseline height ({}) must be less than Descender height ({})",
        t_baseline.rect.height,
        t_descender.rect.height
    );

    // Default without ends_at equals Baseline height
    assert_eq!(t_default.rect.height, t_baseline.rect.height);
}

#[test]
fn test_text_ends_at_via_component_use() {
    use directedtype::component::ComponentRegistry;
    use directedtype::compiler::compiled::CompiledDocument;

    let input = r#"
    \use "components/TextEnd.dt"

    \Text(size: 24, ends_at: TextEnd.Descender) {
        Descender text
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        std::path::Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let text_node = compiled.layout.nodes.iter().find(|n| n.name == "Text").expect("text node found");
    assert_eq!(
        text_node.properties.get("ends_at").map(|v| v.as_str().unwrap()),
        Some("Descender")
    );
}

#[test]
fn test_text_start_at_capital_and_ascender() {
    use directedtype::compiler::compiled::CompiledDocument;
    use directedtype::component::ComponentRegistry;

    let input = r#"
    \Text(size: 32, start_at: TextStart.Capital, ends_at: TextEnd.Baseline) {
        Capital baseline text
    }

    \Text(size: 32, start_at: TextStart.Ascender, ends_at: TextEnd.Baseline) {
        Ascender baseline text
    }

    \Text(size: 32) {
        Default text
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        std::path::Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let text_nodes: Vec<_> = compiled.layout.nodes.iter().filter(|n| n.name == "Text").collect();
    assert_eq!(text_nodes.len(), 3);

    let t_capital = text_nodes[0];
    let t_ascender = text_nodes[1];
    let t_default = text_nodes[2];

    assert_eq!(
        t_capital.properties.get("start_at").map(|v| v.as_str().unwrap()),
        Some("Capital")
    );
    assert_eq!(
        t_ascender.properties.get("start_at").map(|v| v.as_str().unwrap()),
        Some("Ascender")
    );
    assert_eq!(
        t_default.properties.get("start_at").map(|v| v.as_str().unwrap()),
        Some("Capital")
    );

    // Capital start height is shorter than Ascender start height
    assert!(
        t_capital.rect.height < t_ascender.rect.height,
        "Capital height ({}) should be less than Ascender height ({})",
        t_capital.rect.height,
        t_ascender.rect.height
    );

    // Default start_at is Capital
    assert_eq!(t_default.rect.height, t_capital.rect.height);
}

#[test]
fn test_text_start_at_via_component_use() {
    use directedtype::component::ComponentRegistry;
    use directedtype::compiler::compiled::CompiledDocument;

    let input = r#"
    \use "components/TextStart.dt"

    \Text(size: 24, start_at: TextStart.Capital) {
        Capital start text
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        800.0,
        600.0,
        std::path::Path::new("."),
        &directedtype::compiler::FsResolver,
        &registry,
    )
    .expect("compile ok");

    let text_node = compiled.layout.nodes.iter().find(|n| n.name == "Text").expect("text node found");
    assert_eq!(
        text_node.properties.get("start_at").map(|v| v.as_str().unwrap()),
        Some("Capital")
    );
}

#[test]
fn test_text_start_at_misspelled_acender_rejected() {
    let input = r#"
    \use "components/TextStart.dt"

    \Text(size: 24, start_at: TextStart.Acender) {
        Misspelled enum
    }
    "#;

    let doc = parse(input).expect("Failed to parse");
    let err = compile_to_graph(&doc).expect_err("Misspelled variant Acender must be rejected");
    match err {
        directedtype::compiler::CompileError::UnknownEnumVariant { enum_name, variant, .. } => {
            assert_eq!(enum_name, "TextStart");
            assert_eq!(variant, "Acender");
        }
        other => panic!("Expected UnknownEnumVariant, got {:?}", other),
    }
}

