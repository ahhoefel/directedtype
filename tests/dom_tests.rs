use directedtype::ast::{Expr, Ident};
use directedtype::dom::{Dom, DomError};
use directedtype::interaction::Point;
use directedtype::span::Span;

#[test]
fn test_dom_create_and_append() {
    let mut dom = Dom::new();

    let root = dom.create_element(
        "Rect",
        vec![
            ("x".to_string(), Expr::lit(10.0)),
            ("y".to_string(), Expr::lit(20.0)),
            ("width".to_string(), Expr::lit(200.0)),
            ("height".to_string(), Expr::lit(100.0)),
            ("color".to_string(), Expr::color("#ff0000")),
        ],
    );

    let child = dom.create_text(
        "Hello DirectedType",
        vec![
            ("x".to_string(), Expr::lit(15.0)),
            ("y".to_string(), Expr::lit(25.0)),
            ("color".to_string(), Expr::color("#ffffff")),
        ],
    );

    dom.append_child(root, child).unwrap();
    dom.append_root(root).unwrap();

    assert_eq!(dom.roots(), &[root]);
    assert_eq!(dom.parent(child).unwrap(), Some(root));
    assert_eq!(dom.children(root).unwrap(), &[child]);
    assert_eq!(dom.first_child(root).unwrap(), Some(child));
    assert_eq!(dom.last_child(root).unwrap(), Some(child));

    // Verify commit resolves layout correctly
    dom.commit().unwrap();

    let root_rect = dom.computed_rect(root).unwrap();
    assert_eq!(root_rect.x, 10.0);
    assert_eq!(root_rect.y, 20.0);
    assert_eq!(root_rect.width, 200.0);
    assert_eq!(root_rect.height, 100.0);

    let child_rect = dom.computed_rect(child).unwrap();
    assert_eq!(child_rect.x, 15.0);
    assert_eq!(child_rect.y, 25.0);
}

#[test]
fn test_dom_insert_before_staged() {
    // Verifies that operations like insert_before immediately operate on the
    // staged in-memory Component DOM structure before commit.
    let mut dom = Dom::new();

    let container = dom.create_element(
        "Rect",
        vec![
            ("x".to_string(), Expr::lit(0.0)),
            ("y".to_string(), Expr::lit(0.0)),
            ("width".to_string(), Expr::lit(500.0)),
            ("height".to_string(), Expr::lit(500.0)),
            ("color".to_string(), Expr::color("#222222")),
        ],
    );
    dom.append_root(container).unwrap();

    let child_a = dom.create_element(
        "Rect",
        vec![
            ("x".to_string(), Expr::lit(10.0)),
            ("y".to_string(), Expr::lit(10.0)),
            ("width".to_string(), Expr::lit(50.0)),
            ("height".to_string(), Expr::lit(50.0)),
            ("color".to_string(), Expr::color("#ff0000")),
        ],
    );

    let child_c = dom.create_element(
        "Rect",
        vec![
            ("x".to_string(), Expr::lit(10.0)),
            ("y".to_string(), Expr::lit(150.0)),
            ("width".to_string(), Expr::lit(50.0)),
            ("height".to_string(), Expr::lit(50.0)),
            ("color".to_string(), Expr::color("#0000ff")),
        ],
    );

    // Staged mutations: append A then C
    dom.append_child(container, child_a).unwrap();
    dom.append_child(container, child_c).unwrap();

    assert_eq!(dom.children(container).unwrap(), &[child_a, child_c]);

    // Now insert B before C
    let child_b = dom.create_element(
        "Rect",
        vec![
            ("x".to_string(), Expr::lit(10.0)),
            ("y".to_string(), Expr::lit(80.0)),
            ("width".to_string(), Expr::lit(50.0)),
            ("height".to_string(), Expr::lit(50.0)),
            ("color".to_string(), Expr::color("#00ff00")),
        ],
    );

    dom.insert_before(container, child_c, child_b).unwrap();

    // Verify immediately reflected in synchronous tree
    assert_eq!(
        dom.children(container).unwrap(),
        &[child_a, child_b, child_c]
    );
    assert_eq!(dom.prev_sibling(child_b).unwrap(), Some(child_a));
    assert_eq!(dom.next_sibling(child_b).unwrap(), Some(child_c));
    assert_eq!(dom.prev_sibling(child_c).unwrap(), Some(child_b));

    // Commit and verify layout computes
    dom.commit().unwrap();
    assert_eq!(dom.computed_rect(child_b).unwrap().y, 80.0);
    assert_eq!(dom.computed_rect(child_c).unwrap().y, 150.0);
}

#[test]
fn test_dom_replace_child() {
    let mut dom = Dom::new();

    let parent = dom.create_element(
        "Rect",
        vec![
            ("x".to_string(), Expr::lit(0.0)),
            ("y".to_string(), Expr::lit(0.0)),
            ("width".to_string(), Expr::lit(100.0)),
            ("height".to_string(), Expr::lit(100.0)),
            ("color".to_string(), Expr::color("#000000")),
        ],
    );
    dom.append_root(parent).unwrap();

    let old_child = dom.create_element(
        "Rect",
        vec![
            ("x".to_string(), Expr::lit(10.0)),
            ("y".to_string(), Expr::lit(10.0)),
            ("width".to_string(), Expr::lit(20.0)),
            ("height".to_string(), Expr::lit(20.0)),
            ("color".to_string(), Expr::color("#ff0000")),
        ],
    );
    dom.append_child(parent, old_child).unwrap();

    let new_child = dom.create_element(
        "Rect",
        vec![
            ("x".to_string(), Expr::lit(30.0)),
            ("y".to_string(), Expr::lit(30.0)),
            ("width".to_string(), Expr::lit(40.0)),
            ("height".to_string(), Expr::lit(40.0)),
            ("color".to_string(), Expr::color("#00ff00")),
        ],
    );

    dom.replace_child(parent, old_child, new_child).unwrap();

    assert_eq!(dom.children(parent).unwrap(), &[new_child]);
    assert_eq!(dom.parent(new_child).unwrap(), Some(parent));
    assert_eq!(dom.parent(old_child).unwrap(), None);

    dom.commit().unwrap();
    assert_eq!(dom.computed_rect(new_child).unwrap().x, 30.0);
    assert_eq!(dom.computed_rect(old_child), None); // Not in active layout
}

#[test]
fn test_dom_remove_child() {
    let mut dom = Dom::new();

    let parent = dom.create_element(
        "Rect",
        vec![
            ("x".to_string(), Expr::lit(0.0)),
            ("y".to_string(), Expr::lit(0.0)),
            ("width".to_string(), Expr::lit(100.0)),
            ("height".to_string(), Expr::lit(100.0)),
            ("color".to_string(), Expr::color("#000000")),
        ],
    );
    dom.append_root(parent).unwrap();

    let child = dom.create_element(
        "Rect",
        vec![
            ("x".to_string(), Expr::lit(10.0)),
            ("y".to_string(), Expr::lit(10.0)),
            ("width".to_string(), Expr::lit(20.0)),
            ("height".to_string(), Expr::lit(20.0)),
            ("color".to_string(), Expr::color("#ff0000")),
        ],
    );
    dom.append_child(parent, child).unwrap();

    dom.remove_child(parent, child).unwrap();
    assert_eq!(dom.children(parent).unwrap(), &[]);
    assert_eq!(dom.parent(child).unwrap(), None);
}

#[test]
fn test_dom_ports_and_text_mutation() {
    let mut dom = Dom::new();

    let text_node = dom.create_text(
        "Initial Text",
        vec![
            ("x".to_string(), Expr::lit(0.0)),
            ("y".to_string(), Expr::lit(0.0)),
            ("color".to_string(), Expr::color("#000000")),
        ],
    );
    dom.append_root(text_node).unwrap();

    dom.commit().unwrap();
    assert_eq!(dom.text_content(text_node).unwrap(), Some("Initial Text"));

    // Mutate text and port
    dom.set_text(text_node, "Updated Text").unwrap();
    dom.set_port(text_node, "x", Expr::lit(45.0)).unwrap();

    assert!(dom.is_dirty());
    dom.commit().unwrap();
    assert!(!dom.is_dirty());

    assert_eq!(dom.text_content(text_node).unwrap(), Some("Updated Text"));
    assert_eq!(dom.computed_rect(text_node).unwrap().x, 45.0);

    // Remove port
    let old_port = dom.remove_port(text_node, "x").unwrap();
    assert!(old_port.is_some());
    assert!(dom.is_dirty());
}

#[test]
fn test_dom_parse_fragment() {
    let mut dom = Dom::new();

    let fragment_handle = dom
        .parse_fragment(r#"\Rect(x: 25, y: 35, width: 220, height: 110, color: #6366f1)"#)
        .unwrap();

    dom.append_root(fragment_handle).unwrap();
    dom.commit().unwrap();

    let rect = dom.computed_rect(fragment_handle).unwrap();
    assert_eq!(rect.x, 25.0);
    assert_eq!(rect.y, 35.0);
    assert_eq!(rect.width, 220.0);
    assert_eq!(rect.height, 110.0);
}

#[test]
fn test_dom_transaction_auto_commit() {
    let mut dom = Dom::new();

    let (r1, r2) = dom
        .transaction(|tx| {
            let n1 = tx.create_element(
                "Rect",
                vec![
                    ("x".to_string(), Expr::lit(0.0)),
                    ("y".to_string(), Expr::lit(0.0)),
                    ("width".to_string(), Expr::lit(100.0)),
                    ("height".to_string(), Expr::lit(50.0)),
                    ("color".to_string(), Expr::color("#ff0000")),
                ],
            );
            let n2 = tx.create_element(
                "Rect",
                vec![
                    ("x".to_string(), Expr::lit(100.0)),
                    ("y".to_string(), Expr::lit(0.0)),
                    ("width".to_string(), Expr::lit(100.0)),
                    ("height".to_string(), Expr::lit(50.0)),
                    ("color".to_string(), Expr::color("#00ff00")),
                ],
            );
            tx.append_root(n1)?;
            tx.append_root(n2)?;
            Ok((n1, n2))
        })
        .unwrap();

    // Verify auto-commit ran: layout is resolved
    assert!(!dom.is_dirty());
    assert_eq!(dom.computed_rect(r1).unwrap().width, 100.0);
    assert_eq!(dom.computed_rect(r2).unwrap().x, 100.0);
}

#[test]
fn test_dom_transaction_rollback_on_user_error() {
    let mut dom = Dom::new();

    let root = dom.create_element(
        "Rect",
        vec![
            ("x".to_string(), Expr::lit(0.0)),
            ("y".to_string(), Expr::lit(0.0)),
            ("width".to_string(), Expr::lit(100.0)),
            ("height".to_string(), Expr::lit(100.0)),
            ("color".to_string(), Expr::color("#000000")),
        ],
    );
    dom.append_root(root).unwrap();
    dom.commit().unwrap();

    // Transaction that aborts intentionally
    let res: Result<(), DomError> = dom.transaction(|tx| {
        let child = tx.create_element("Rect", vec![]);
        tx.append_child(root, child)?;
        // Staged child exists right now
        assert_eq!(tx.children(root)?, &[child]);

        // Fail transaction
        Err(DomError::TransactionError("aborted intentionally".into()))
    });

    assert!(res.is_err());

    // Verify rollback: root has 0 children
    assert_eq!(dom.children(root).unwrap(), &[]);
    assert_eq!(dom.roots(), &[root]);
}

#[test]
fn test_dom_transaction_rollback_on_compile_cycle() {
    let mut dom = Dom::new();

    let root = dom.create_element(
        "Rect",
        vec![
            ("x".to_string(), Expr::lit(10.0)),
            ("y".to_string(), Expr::lit(10.0)),
            ("width".to_string(), Expr::lit(100.0)),
            ("height".to_string(), Expr::lit(100.0)),
            ("color".to_string(), Expr::color("#000000")),
        ],
    );
    dom.append_root(root).unwrap();
    dom.commit().unwrap();

    assert_eq!(dom.computed_rect(root).unwrap().width, 100.0);

    // Introduce an algebraic cyclic dependency: __node_0.width -> __node_0.width
    let cycle_res = dom.transaction(|tx| {
        let self_cycle = Expr::MemberAccess(directedtype::ast::MemberAccessExpr {
            target: Box::new(Expr::Ident(Ident::new("__node_0", Span::default()))),
            member: Ident::new("width", Span::default()),
            span: Span::default(),
        });
        tx.set_port(root, "width", self_cycle)?;
        Ok(())
    });

    assert!(cycle_res.is_err());

    // Verify the previous valid layout is still intact!
    assert_eq!(dom.computed_rect(root).unwrap().width, 100.0);
}

#[test]
fn test_dom_hierarchy_cycle_prevention() {
    let mut dom = Dom::new();

    let a = dom.create_element("Rect", vec![]);
    let b = dom.create_element("Rect", vec![]);
    let c = dom.create_element("Rect", vec![]);

    dom.append_child(a, b).unwrap();
    dom.append_child(b, c).unwrap();

    // Try to append `a` to `c` (cycle: a -> b -> c -> a)
    let res = dom.append_child(c, a);
    assert!(matches!(res, Err(DomError::HierarchyCycle(_))));

    // Try to append `a` to `a`
    let res_self = dom.append_child(a, a);
    assert!(matches!(res_self, Err(DomError::HierarchyCycle(_))));
}

#[test]
fn test_dom_spatial_hit_test() {
    let mut dom = Dom::new();

    let bg = dom.create_element(
        "Rect",
        vec![
            ("x".to_string(), Expr::lit(0.0)),
            ("y".to_string(), Expr::lit(0.0)),
            ("width".to_string(), Expr::lit(500.0)),
            ("height".to_string(), Expr::lit(500.0)),
            ("z".to_string(), Expr::lit(1.0)),
            ("color".to_string(), Expr::color("#000000")),
        ],
    );

    let btn = dom.create_element(
        "Rect",
        vec![
            ("x".to_string(), Expr::lit(50.0)),
            ("y".to_string(), Expr::lit(50.0)),
            ("width".to_string(), Expr::lit(100.0)),
            ("height".to_string(), Expr::lit(40.0)),
            ("z".to_string(), Expr::lit(2.0)),
            ("color".to_string(), Expr::color("#ff0000")),
        ],
    );

    dom.append_root(bg).unwrap();
    dom.append_root(btn).unwrap();
    dom.commit().unwrap();

    // Hit test inside button (z=2 dominates z=1)
    let hit_btn = dom.hit_test(Point::new(75.0, 60.0));
    assert_eq!(hit_btn, Some(btn));

    // Hit test inside background outside button
    let hit_bg = dom.hit_test(Point::new(10.0, 10.0));
    assert_eq!(hit_bg, Some(bg));

    // Hit test outside everything
    let hit_none = dom.hit_test(Point::new(600.0, 600.0));
    assert_eq!(hit_none, None);
}

#[test]
fn test_dom_destroy_node_frees_subtree() {
    let mut dom = Dom::new();

    let root = dom.create_element("Rect", vec![]);
    let child1 = dom.create_element("Rect", vec![]);
    let child2 = dom.create_element("Rect", vec![]);

    dom.append_root(root).unwrap();
    dom.append_child(root, child1).unwrap();
    dom.append_child(child1, child2).unwrap();

    assert_eq!(dom.node_count(), 3);
    assert!(dom.is_valid_handle(child1));
    assert!(dom.is_valid_handle(child2));

    dom.destroy_node(child1).unwrap();

    assert_eq!(dom.node_count(), 1);
    assert_eq!(dom.children(root).unwrap(), &[]);
    assert!(!dom.is_valid_handle(child1));
    assert!(!dom.is_valid_handle(child2));

    // Calling methods on destroyed handles returns InvalidHandle
    assert!(matches!(
        dom.children(child1),
        Err(DomError::InvalidHandle(_))
    ));
}
