use directedtype::ast::{ComponentKey, Expr};
use directedtype::compiler::error::CompileError;
use directedtype::compiler::graph::VarId;
use directedtype::compiler::{compile_document_with_window, NodeId};
use directedtype::dom::Dom;
use directedtype::parse;
use directedtype::Value;

#[test]
fn test_compiled_document_set_state_updates_downstream_dag() {
    let input = r#"
    \Component Counter(multiplier: Number: 10) {
        state count: 0;
        \Rect(x: 0, y: 0, width: count * multiplier, height: 30, color: #00FF00)
    }

    \Counter(multiplier: 10)
    "#;

    let doc = parse(input).expect("Failed to parse document");
    let mut compiled =
        compile_document_with_window(&doc, 800.0, 600.0).expect("Failed to compile document");

    // Initially count is 0, so rect width is 0
    let counter_id = compiled.layout.roots[0];
    let (rect_id, initial_width) = {
        let rect_node = compiled
            .layout
            .nodes
            .iter()
            .find(|n| n.name == "Rect")
            .expect("Should have Rect");
        (rect_node.id, rect_node.rect.width)
    };
    assert_eq!(initial_width, 0.0);

    // Mutate state: count = 7
    let changed = compiled
        .set_state(counter_id, "count", Value::Number(7.0))
        .expect("Failed to set state");

    // Verify changed set contains count and width
    assert!(changed.contains(&VarId::new(counter_id, "count")));
    assert!(changed.contains(&VarId::new(rect_id, "width")));

    // Verify layout was incrementally updated
    let updated_rect = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "Rect")
        .expect("Should have Rect");
    assert_eq!(updated_rect.rect.width, 70.0);
}

#[test]
fn test_compiled_document_not_a_state_variable_error() {
    let input = r#"
    \Component Counter(multiplier: Number: 10) {
        state count: 0;
        \Rect(x: 0, y: 0, width: count * multiplier, height: 30, color: #00FF00)
    }

    \Counter(multiplier: 10)
    "#;

    let doc = parse(input).expect("Failed to parse document");
    let mut compiled = compile_document_with_window(&doc, 800.0, 600.0).unwrap();
    let counter_id = compiled.layout.roots[0];

    // Attempting to mutate "multiplier" (which is an input port, not a state variable)
    let err = compiled
        .set_state(counter_id, "multiplier", Value::Number(20.0))
        .expect_err("Should reject mutating non-state variable");

    match err {
        CompileError::NotAStateVariable { node, var, .. } => {
            assert_eq!(node, "Counter");
            assert_eq!(var, "multiplier");
        }
        other => panic!("Expected NotAStateVariable, got {:?}", other),
    }
}

#[test]
fn test_compiled_document_type_mismatch_error() {
    let input = r#"
    \Component Counter {
        state count: Number: 0;
        \Rect(x: 0, y: 0, width: count * 10, height: 30, color: #00FF00)
    }

    \Counter()
    "#;

    let doc = parse(input).expect("Failed to parse document");
    let mut compiled = compile_document_with_window(&doc, 800.0, 600.0).unwrap();
    let counter_id = compiled.layout.roots[0];

    // Attempting to pass a String to a Number state variable
    let err = compiled
        .set_state(counter_id, "count", Value::String("ten".to_string()))
        .expect_err("Should reject type mismatch");

    match err {
        CompileError::TypeMismatch { expected, actual, .. } => {
            assert_eq!(expected, "Number");
            assert_eq!(actual, "String");
        }
        other => panic!("Expected TypeMismatch, got {:?}", other),
    }
}

#[test]
fn test_compiled_document_set_state_by_structured_key() {
    let input = r#"
    \Component Cell(is_selected: Boolean: false) {
        state active: false;
        \Rect(x: 0, y: 0, width: 50, height: 50, color: (active || is_selected) ? #00FF00 : #FF0000)
    }

    \Component Grid {
        \Cell(0, 0; is_selected: false)
        \Cell(0, 1; is_selected: false)
        \Cell(1, 0; is_selected: false)
    }

    \Grid()
    "#;

    let doc = parse(input).expect("Failed to parse document");
    let mut compiled = compile_document_with_window(&doc, 800.0, 600.0).unwrap();
    let grid_id = compiled.layout.roots[0];

    // Initially all cells are inactive / red (#FF0000)
    let key_1_0 = ComponentKey::tuple(&[Expr::number(1.0), Expr::number(0.0)]);
    let key_0_0 = ComponentKey::tuple(&[Expr::number(0.0), Expr::number(0.0)]);

    // Mutate state of cell (1, 0)
    compiled
        .set_state_by_key(Some(grid_id), &key_1_0, "active", Value::Bool(true))
        .expect("Should find and mutate cell (1, 0)");

    // Query node for cell (1, 0)
    let cell_1_0 = compiled
        .find_by_key(Some(grid_id), &key_1_0)
        .expect("Should find cell (1, 0)");
    let cell_1_0_id = cell_1_0.id;
    let rect_1_0 = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "Rect" && n.parent == Some(cell_1_0_id))
        .expect("Should find rect for cell (1, 0)");
    assert_eq!(
        rect_1_0.properties.get("color"),
        Some(&Value::Color("#00FF00".to_string()))
    );

    // Verify cell (0, 0) remains red (#FF0000)
    let cell_0_0 = compiled
        .find_by_key(Some(grid_id), &key_0_0)
        .expect("Should find cell (0, 0)");
    let cell_0_0_id = cell_0_0.id;
    let rect_0_0 = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.name == "Rect" && n.parent == Some(cell_0_0_id))
        .expect("Should find rect for cell (0, 0)");
    assert_eq!(
        rect_0_0.properties.get("color"),
        Some(&Value::Color("#FF0000".to_string()))
    );
}

#[test]
fn test_state_mutation_updates_dependent_component_key() {
    let input = r#"
    \Component Master {
        state cursor_idx: 0;
        \Rect(cursor_idx; x: cursor_idx * 100, y: 0, width: 80, height: 40, color: #0000FF)
    }

    \Master()
    "#;

    let doc = parse(input).expect("Failed to parse document");
    let mut compiled = compile_document_with_window(&doc, 800.0, 600.0).unwrap();
    let master_id = compiled.layout.roots[0];

    // Initial key for the rect is (0.0)
    let initial_key = ComponentKey::tuple(&[Expr::number(0.0)]);
    let rect = compiled
        .find_by_key(Some(master_id), &initial_key)
        .expect("Should find rect with key (0)");
    assert_eq!(rect.rect.x, 0.0);

    // Mutate state cursor_idx = 3
    compiled
        .set_state(master_id, "cursor_idx", Value::Number(3.0))
        .expect("Failed to set state");

    // Key has re-evaluated to (3.0) and rect x has updated to 300.0
    let updated_key = ComponentKey::tuple(&[Expr::number(3.0)]);
    let updated_rect = compiled
        .find_by_key(Some(master_id), &updated_key)
        .expect("Should find rect with key (3)");
    assert_eq!(updated_rect.rect.x, 300.0);
}

#[test]
fn test_multi_level_transitive_dependency_propagation() {
    let input = r#"
    \Component Pipeline {
        state base: 10;
        \Rect(x: 0, y: 0, width: base * 2, height: 20, color: #FFFFFF)
        \Rect(x: prev.right + 10, y: 0, width: prev.width + 5, height: 20, color: #FFFFFF)
    }

    \Pipeline()
    "#;

    let doc = parse(input).expect("Failed to parse document");
    let mut compiled = compile_document_with_window(&doc, 800.0, 600.0).unwrap();
    let pipe_id = compiled.layout.roots[0];

    // Initial: base = 10
    // Rect 1 width = 20
    // Rect 2 width = 25, x = 30
    let (rect_1_id, rect_2_id) = {
        let rects: Vec<_> = compiled.layout.nodes.iter().filter(|n| n.name == "Rect").collect();
        assert_eq!(rects[0].rect.width, 20.0);
        assert_eq!(rects[1].rect.width, 25.0);
        assert_eq!(rects[1].rect.x, 30.0);
        (rects[0].id, rects[1].id)
    };

    // Mutate base = 50:
    // Rect 1 width = 100
    // Rect 2 width = 105, x = 110
    let changed = compiled
        .set_state(pipe_id, "base", Value::Number(50.0))
        .expect("Failed to set state");

    assert!(changed.contains(&VarId::new(pipe_id, "base")));
    assert!(changed.contains(&VarId::new(rect_1_id, "width")));
    assert!(changed.contains(&VarId::new(rect_2_id, "width")));
    assert!(changed.contains(&VarId::new(rect_2_id, "x")));

    let updated_rects: Vec<_> = compiled.layout.nodes.iter().filter(|n| n.name == "Rect").collect();
    assert_eq!(updated_rects[0].rect.width, 100.0);
    assert_eq!(updated_rects[1].rect.width, 105.0);
    assert_eq!(updated_rects[1].rect.x, 110.0);
}

#[test]
fn test_dom_set_state_microsecond_incremental_update() {
    let input = r#"
    \Component Counter(multiplier: Number: 10) {
        state count: 0;
        \Rect(x: 0, y: 0, width: count * multiplier, height: 35, color: #0000FF)
    }

    \Counter(multiplier: 10)
    "#;

    let mut dom = Dom::from_source(input).expect("Failed to create DOM");
    dom.commit().expect("Initial commit failed");

    let counter_handle = dom.roots()[0];
    let counter_node = dom.layout().unwrap().get_node(NodeId(0)).unwrap();
    let rect_id = counter_node.children[0];

    // Initially width is 0
    let initial_rect = dom.layout().unwrap().get_node(rect_id).unwrap();
    assert_eq!(initial_rect.rect.width, 0.0);

    // Mutate state: count = 12
    let changed = dom
        .set_state(counter_handle, "count", Value::Number(12.0))
        .expect("Failed to set state in DOM");

    assert!(!changed.is_empty());
    // DOM dirty flag MUST be false (incremental update, no full recompile needed)
    assert!(!dom.is_dirty());

    // Spatial layout query immediately returns updated layout
    let updated_rect = dom.layout().unwrap().get_node(rect_id).unwrap();
    assert_eq!(updated_rect.rect.width, 120.0);
    assert_eq!(
        dom.layout().unwrap().get_value(rect_id, "width"),
        Some(&Value::Number(120.0))
    );
    assert_eq!(
        dom.get_state(counter_handle, "count"),
        Some(&Value::Number(12.0))
    );
}

#[test]
fn test_dom_set_state_by_key() {
    let input = r#"
    \Component Button(label: String: "OK") {
        state pressed: false;
        \Rect(x: 0, y: 0, width: pressed ? 120 : 100, height: 40, color: #222222)
    }

    \Flow() {
        \Button("save"; label: "Save")
        \Button("cancel"; label: "Cancel")
    }
    "#;

    let mut dom = Dom::from_source(input).unwrap();
    dom.commit().unwrap();
    let flow_handle = dom.roots()[0];

    let save_key = ComponentKey::string("save");
    let save_handle = dom
        .get_node_by_key(Some(flow_handle), &save_key)
        .expect("Should find save button handle");

    // Mutate state by key
    dom.set_state_by_key(
        Some(flow_handle),
        &save_key,
        "pressed",
        Value::Bool(true),
    )
    .expect("Failed to set state by key");

    assert_eq!(dom.get_state(save_handle, "pressed"), Some(&Value::Bool(true)));
    assert_eq!(dom.computed_value(save_handle, "pressed"), Some(&Value::Bool(true)));
}

#[test]
fn test_dom_structural_mutation_preserves_state() {
    let input = r#"
    \Component Counter {
        state count: 0;
        \Rect(x: 0, y: 0, width: count * 10, height: 30, color: #00FF00)
    }

    \Counter()
    "#;

    let mut dom = Dom::from_source(input).unwrap();
    dom.commit().unwrap();

    let counter_handle = dom.roots()[0];
    dom.set_state(counter_handle, "count", Value::Number(99.0)).unwrap();

    // Now perform a structural mutation: append an additional child rect to the document root
    let new_elem = dom.create_element(
        "Rect",
        vec![
            ("x".to_string(), Expr::number(0.0)),
            ("y".to_string(), Expr::number(100.0)),
            ("width".to_string(), Expr::number(50.0)),
            ("height".to_string(), Expr::number(50.0)),
            ("color".to_string(), Expr::color("#000")),
        ],
    );
    dom.append_root(new_elem).unwrap();

    // DOM is dirty due to structural mutation
    assert!(dom.is_dirty());

    // Re-commit
    dom.commit().unwrap();

    // State count = 99 should have been preserved and re-applied!
    let counter_node = dom.layout().unwrap().nodes.iter().find(|n| n.name == "Counter").unwrap();
    let rect_child_id = counter_node.children[0];
    let counter_rect = dom.layout().unwrap().get_node(rect_child_id).unwrap();
    assert_eq!(counter_rect.rect.width, 990.0);
}

#[test]
fn test_transaction_batching_state_mutations() {
    let input = r#"
    \Component DualCounter {
        state a: 10;
        state b: 20;
        \Rect(x: 0, y: 0, width: a + b, height: 40, color: #000)
    }

    \DualCounter()
    "#;

    let mut dom = Dom::from_source(input).unwrap();
    dom.commit().unwrap();
    let comp_handle = dom.roots()[0];

    // Begin transaction and mutate both state fields
    let mut tx = dom.begin_transaction();
    tx.set_state(comp_handle, "a", Value::Number(50.0)).unwrap();
    tx.set_state(comp_handle, "b", Value::Number(30.0)).unwrap();
    tx.commit().unwrap();

    // Width should now be 50 + 30 = 80
    let rect = dom.layout().unwrap().nodes.iter().find(|n| n.name == "Rect").unwrap();
    assert_eq!(rect.rect.width, 80.0);
}
