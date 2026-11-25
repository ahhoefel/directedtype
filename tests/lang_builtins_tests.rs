use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use directedtype::ast::{
    ComponentDef, ComponentKind, ContentItem, EnumDef, Expr, Ident, Item, MemberAccessExpr,
};
use directedtype::compdoc::{extract_catalog, ChildrenCardinality, DocGenOptions};
use directedtype::compiler::error::CompileError;
use directedtype::compiler::eval::eval_expr;
use directedtype::compiler::expand::builtins::{
    builtin_primitive_accepted_ports, builtin_primitive_accepts_event_handlers,
    builtin_primitive_accepts_port, builtin_primitive_children_cardinality,
    builtin_primitive_is_immutable_alias, builtin_primitive_output_aliases,
    builtin_primitive_port_type_name, pagination::populated_page_port_names,
    public_builtin_primitive_names, register_builtin_enums_and_components,
    timer::{INDEFINITE_TIMER_SPEC, TIMER_SPEC},
    BuiltinChildren, ALL_SPECIAL_BUILTINS,
};
use directedtype::compiler::expand::expand_document;
use directedtype::compiler::expanded::NodeId;
use directedtype::compiler::graph::VarId;
use directedtype::compiler::module::{resolve_imports, FsResolver, VirtualResolver};
use directedtype::compiler::value::Value;
use directedtype::parser::{parse_document, parse_document_with_file};
use directedtype::span::Span;

const STANDARD_EVENT_PORTS: &[&str] = &[
    "on_click",
    "on_pointer_down",
    "on_pointer_up",
    "on_pointer_move",
    "on_pointer_enter",
    "on_pointer_leave",
    "on_scroll",
    "on_focus",
    "on_blur",
];

fn load_lang_declarations() -> (HashMap<String, Vec<ComponentDef>>, HashMap<String, EnumDef>) {
    let lang_dir = Path::new("components/lang");
    let mut entries: Vec<PathBuf> = fs::read_dir(lang_dir)
        .expect("Failed to read components/lang/")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("dt"))
        .collect();
    entries.sort();
    assert!(!entries.is_empty(), "components/lang/ must contain .dt files");

    let mut comps: HashMap<String, Vec<ComponentDef>> = HashMap::new();
    let mut enums: HashMap<String, EnumDef> = HashMap::new();

    for path in entries {
        let src = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()));
        let doc = parse_document_with_file(&src, &path)
            .unwrap_or_else(|e| panic!("Failed to parse {}: {e}", path.display()));
        for item in doc.items {
            match item {
                Item::Component(c) => {
                    assert_eq!(
                        c.kind,
                        ComponentKind::BuiltIn,
                        "All component declarations in components/lang/ ({}) must use \\BuiltIn",
                        path.display()
                    );
                    comps
                        .entry(c.name.as_str().to_string())
                        .or_default()
                        .push(c);
                }
                Item::Enum(e) => {
                    enums.insert(e.name.as_str().to_string(), e);
                }
                other => panic!(
                    "Unexpected top-level item in {}: {:?}",
                    path.display(),
                    other
                ),
            }
        }
    }

    (comps, enums)
}

#[test]
fn test_builtin_keyword_disallowed_outside_components_lang() {
    let src = r#"
\BuiltIn Fake(x: Distance) {
    \alias y: Distance;
}
"#;
    // Anonymous source (no file path) must be rejected.
    let err = parse_document(src).expect_err("\\BuiltIn without file path must fail to parse");
    assert!(
        err.to_string().contains("components/lang"),
        "Unexpected error: {err}"
    );

    // Non-lang path must be rejected.
    let err = parse_document_with_file(src, Path::new("components/layout/Fake.dt"))
        .expect_err("\\BuiltIn in components/layout/ must fail to parse");
    assert!(
        err.to_string().contains("components/lang"),
        "Unexpected error: {err}"
    );

    // Path inside components/lang/ must succeed.
    let doc = parse_document_with_file(src, Path::new("components/lang/Fake.dt"))
        .expect("\\BuiltIn inside components/lang/ must parse");
    assert_eq!(doc.items.len(), 1);
}

#[test]
fn test_bodyless_alias_only_allowed_inside_builtin() {
    let normal_comp = r#"
\Component Foo(x: Distance) {
    \alias bar: Distance;
}
"#;
    parse_document(normal_comp)
        .expect_err("Bodyless \\alias must be rejected inside \\Component");
    parse_document_with_file(normal_comp, Path::new("components/lang/Foo.dt"))
        .expect_err("Bodyless \\alias must be rejected inside \\Component even in components/lang/");

    let normal_span = r#"
\Span Foo() {
    \alias bar: Distance;
}
"#;
    parse_document(normal_span).expect_err("Bodyless \\alias must be rejected inside \\Span");

    let normal_inline = r#"
\InlineBox Foo() {
    \alias bar: Distance;
}
"#;
    parse_document(normal_inline)
        .expect_err("Bodyless \\alias must be rejected inside \\InlineBox");

    // Bodyless alias without `: Type` inside \BuiltIn must also be rejected.
    let untyped_builtin_alias = r#"
\BuiltIn Foo() {
    \alias bar;
}
"#;
    parse_document_with_file(untyped_builtin_alias, Path::new("components/lang/Foo.dt"))
        .expect_err("Bodyless \\alias without explicit type must fail to parse");

    // Typed bodyless alias inside \BuiltIn in components/lang/ must succeed.
    let typed_builtin_alias = r#"
\BuiltIn Foo() {
    \alias bar: Distance;
}
"#;
    let doc = parse_document_with_file(typed_builtin_alias, Path::new("components/lang/Foo.dt"))
        .expect("Typed bodyless \\alias inside \\BuiltIn must parse");
    match &doc.items[0] {
        Item::Component(c) => match &c.body[0] {
            ContentItem::Alias(a) => {
                assert_eq!(a.name.as_str(), "bar");
                assert_eq!(
                    a.type_annotation.as_ref().map(|t| t.name.as_str()),
                    Some("Distance")
                );
                assert!(a.value.is_none());
            }
            other => panic!("Expected Alias, got {other:?}"),
        },
        other => panic!("Expected Component, got {other:?}"),
    }
}

#[test]
fn test_explicit_import_of_components_lang_is_blocked() {
    let cases = [
        r#"\use "components/lang/Rect.dt""#,
        r#"\use "./components/lang/Font.dt""#,
        r#"\use { Rect } from "components/lang/Rect.dt";"#,
        r#"\use "components/lang/Shadow.dt" as MyShadow"#,
    ];

    for src in cases {
        let doc = parse_document(src).expect("Import syntax should parse");
        let err = resolve_imports(&doc, Path::new("."), &FsResolver)
            .expect_err(&format!("Explicit import must be blocked: {src}"));
        match err {
            CompileError::ImportError { message, .. } => {
                assert!(
                    message.contains("components/lang"),
                    "Expected components/lang message, got: {message}"
                );
            }
            other => panic!("Expected ImportError for '{src}', got {other:?}"),
        }
    }

    // Relative import from a subdirectory (e.g. `components/layout/`) into `../lang/Rect.dt` must also be blocked.
    let rel_src = r#"\use "../lang/Rect.dt""#;
    let rel_doc = parse_document(rel_src).unwrap();
    let mut vfs = VirtualResolver::new();
    vfs.insert("components/lang/Rect.dt", r#"\BuiltIn Rect(x: Distance) {}"#);
    let err = resolve_imports(&rel_doc, Path::new("components/layout"), &vfs)
        .expect_err("Relative import into components/lang/ must be blocked");
    assert!(
        matches!(err, CompileError::ImportError { .. }),
        "Expected ImportError, got {err:?}"
    );
}

#[test]
fn test_compiling_builtin_file_directly_is_rejected() {
    let rect_path = Path::new("components/lang/Rect.dt");
    let src = fs::read_to_string(rect_path).expect("Read Rect.dt");
    let doc = parse_document_with_file(&src, rect_path).expect("Parse Rect.dt");
    let err = expand_document(&doc).expect_err("Compiling components/lang/Rect.dt must fail");
    assert!(
        err.to_string().contains("\\BuiltIn"),
        "Expected \\BuiltIn compilation error, got: {err}"
    );
}

#[test]
fn test_components_lang_matches_compiler_builtins_without_drift() {
    let (lang_comps, lang_enums) = load_lang_declarations();

    // 1. Every public BuiltinPrimitive must have a \BuiltIn declaration in components/lang/.
    for prim_name in public_builtin_primitive_names() {
        assert!(
            lang_comps.contains_key(prim_name),
            "Compiler built-in primitive '{prim_name}' is missing from components/lang/"
        );
    }

    // Extract compdoc catalog for components/lang/ so we can also verify exact children cardinality.
    let lang_catalog =
        extract_catalog(&[PathBuf::from("components/lang")], &DocGenOptions::default())
            .expect("Failed to extract compdoc catalog for components/lang");
    let catalog_comps: HashMap<&str, &directedtype::compdoc::ComponentDoc> = lang_catalog
        .packages
        .iter()
        .flat_map(|p| &p.modules)
        .flat_map(|m| &m.components)
        .map(|c| (c.name.as_str(), c))
        .collect();

    let to_doc_cardinality = |c: BuiltinChildren| match c {
        BuiltinChildren::None => ChildrenCardinality::None,
        BuiltinChildren::Single => ChildrenCardinality::Single,
        BuiltinChildren::Multiple => ChildrenCardinality::Multiple,
    };

    // 2. Special non-primitive built-ins (Window, Page, Timer, IndefiniteTimer):
    //    Check exact bidirectional parity between each SpecialBuiltinSpec and components/lang/<Name>.dt.
    const IMPLICIT_SPATIAL_ALIASES: &[&str] = &["left", "top", "right", "bottom"];
    for spec in ALL_SPECIAL_BUILTINS {
        let overloads = lang_comps.get(spec.name).unwrap_or_else(|| {
            panic!(
                "Special built-in '{}' is missing from components/lang/",
                spec.name
            )
        });
        assert_eq!(
            overloads.len(),
            1,
            "Expected single overload for special built-in '{}'",
            spec.name
        );
        let ov = &overloads[0];

        // Children cardinality parity for special built-ins
        let doc_comp = catalog_comps[spec.name];
        for (oi, doc_ov) in doc_comp.overloads.iter().enumerate() {
            assert_eq!(
                doc_ov.children_cardinality,
                to_doc_cardinality(spec.children),
                "Children cardinality drifted for special built-in '{}'[{}]",
                spec.name,
                oi
            );
        }

        // Accepted input ports: exact 1-to-1 match (name + type)
        let compiler_params: HashMap<&str, &str> = spec.accepted_ports.iter().copied().collect();
        let mut doc_params: HashMap<&str, &str> = HashMap::new();
        for p in &ov.params {
            let ty = p
                .type_annotation
                .as_ref()
                .map(|t| t.name.as_str())
                .unwrap_or_else(|| {
                    panic!(
                        "components/lang/{}.dt param '{}' missing type annotation",
                        spec.name,
                        p.name.as_str()
                    )
                });
            doc_params.insert(p.name.as_str(), ty);
        }
        assert_eq!(
            doc_params, compiler_params,
            "Accepted parameters drifted for special built-in '{}'",
            spec.name
        );

        // Read-only output ports: exact 1-to-1 match against \alias declarations
        // (excluding implicit spatial aliases left/top/right/bottom and ports already in accepted_ports).
        let compiler_outputs: HashMap<&str, &str> = spec
            .output_ports
            .iter()
            .copied()
            .filter(|(p, _)| !IMPLICIT_SPATIAL_ALIASES.contains(p) && !compiler_params.contains_key(p))
            .collect();
        let mut doc_aliases: HashMap<&str, &str> = HashMap::new();
        for item in &ov.body {
            if let ContentItem::Alias(a) = item {
                let ty = a
                    .type_annotation
                    .as_ref()
                    .map(|t| t.name.as_str())
                    .unwrap_or_else(|| {
                        panic!(
                            "components/lang/{}.dt alias '{}' missing type annotation",
                            spec.name,
                            a.name.as_str()
                        )
                    });
                doc_aliases.insert(a.name.as_str(), ty);
            }
        }
        assert_eq!(
            doc_aliases, compiler_outputs,
            "Output aliases drifted for special built-in '{}'",
            spec.name
        );
    }

    // Verify `populate_page_ports` populates the exact 1-to-1 key set declared in `PAGE_SPEC.output_ports`.
    let mut spec_page_ports: Vec<String> = ALL_SPECIAL_BUILTINS
        .iter()
        .find(|s| s.name == "Page")
        .unwrap()
        .output_ports
        .iter()
        .map(|(p, _)| (*p).to_string())
        .collect();
    spec_page_ports.sort();
    assert_eq!(
        populated_page_port_names(),
        spec_page_ports,
        "populate_page_ports() keys drifted from PAGE_SPEC.output_ports"
    );

    // 3. Every compiler-registered enum and struct-like component (LineCap, LineBreak, Break,
    //    TextStart, TextEnd, Shadow, TextBounds) must match components/lang/ exactly.
    let mut compiler_enums = HashMap::new();
    let mut compiler_registry = HashMap::new();
    register_builtin_enums_and_components(
        &mut compiler_enums,
        &mut compiler_registry,
        Span::default(),
    );

    for (enum_name, comp_enum) in &compiler_enums {
        let lang_enum = lang_enums.get(enum_name).unwrap_or_else(|| {
            panic!("Compiler built-in enum '{enum_name}' is missing from components/lang/")
        });
        let comp_variants: Vec<&str> = comp_enum.variants.iter().map(|v| v.as_str()).collect();
        let lang_variants: Vec<&str> = lang_enum.variants.iter().map(|v| v.as_str()).collect();
        assert_eq!(
            lang_variants, comp_variants,
            "Enum variants drifted for '{enum_name}'"
        );
    }

    for (comp_name, overloads) in &compiler_registry {
        let lang_overloads = lang_comps.get(comp_name).unwrap_or_else(|| {
            panic!("Compiler built-in component '{comp_name}' is missing from components/lang/")
        });
        assert_eq!(
            lang_overloads.len(),
            overloads.len(),
            "Overload count drifted for built-in struct '{comp_name}'"
        );
        for (c_ov, l_ov) in overloads.iter().zip(lang_overloads.iter()) {
            let c_params: Vec<(&str, Option<&str>)> = c_ov
                .params
                .iter()
                .map(|p| (p.name.as_str(), p.type_annotation.as_ref().map(|t| t.name.as_str())))
                .collect();
            let l_params: Vec<(&str, Option<&str>)> = l_ov
                .params
                .iter()
                .map(|p| (p.name.as_str(), p.type_annotation.as_ref().map(|t| t.name.as_str())))
                .collect();
            assert_eq!(
                l_params, c_params,
                "Parameters drifted for built-in struct '{comp_name}'"
            );
        }
    }

    // 4. For every BuiltinPrimitive:
    //    - No primitive may accept arbitrary unknown ports.
    //    - Every parameter declared in components/lang/ must be accepted by the compiler primitive
    //      and match its compiler static type.
    //    - Every port in the compiler's own `accepted_ports()` slice must be documented in
    //      components/lang/ (zero hardcoded port list — adding ANY new port to a primitive
    //      without updating its .dt file immediately fails this test).
    //    - Event handler acceptance (`accepts_event_handlers()`) must match `on_*` params in .dt.
    //    - Every declared alias on a primitive must match its compiler port_type (when known).
    for prim_name in public_builtin_primitive_names() {
        assert_eq!(
            builtin_primitive_accepts_port(prim_name, "__arbitrary_unknown_port__"),
            Some(false),
            "Compiler primitive '{prim_name}' must not accept arbitrary unknown ports"
        );

        let compiler_ports = builtin_primitive_accepted_ports(prim_name)
            .unwrap_or_else(|| panic!("Missing accepted_ports for '{prim_name}'"));
        let mut seen_compiler_ports = HashSet::new();
        for &p in compiler_ports {
            assert!(
                seen_compiler_ports.insert(p),
                "Duplicate port '{p}' in {prim_name}::accepted_ports()"
            );
        }

        let compiler_aliases = builtin_primitive_output_aliases(prim_name)
            .unwrap_or_else(|| panic!("Missing output_aliases for '{prim_name}'"));
        let mut seen_compiler_aliases = HashSet::new();
        for &a in compiler_aliases {
            assert!(
                seen_compiler_aliases.insert(a),
                "Duplicate alias '{a}' in {prim_name}::output_aliases()"
            );
            assert!(
                !seen_compiler_ports.contains(a),
                "Port '{a}' cannot be in both accepted_ports() and output_aliases() on '{prim_name}'"
            );
        }

        // Children cardinality parity: every overload in components/lang/<Prim>.dt must match
        // the compiler's `children_cardinality()`.
        let expected_cardinality = builtin_primitive_children_cardinality(prim_name)
            .unwrap_or_else(|| panic!("Missing children_cardinality for '{prim_name}'"));
        let doc_comp = catalog_comps[prim_name];
        for (oi, doc_ov) in doc_comp.overloads.iter().enumerate() {
            assert_eq!(
                doc_ov.children_cardinality,
                to_doc_cardinality(expected_cardinality),
                "Children cardinality drifted for '{prim_name}' overload {oi}: .dt has {:?}, compiler has {:?}",
                doc_ov.children_cardinality,
                expected_cardinality
            );
        }

        let overloads = &lang_comps[prim_name];
        let mut documented_params: HashSet<String> = HashSet::new();

        for (oi, ov) in overloads.iter().enumerate() {
            let mut ov_params: HashSet<&str> = HashSet::new();
            let mut ov_aliases: HashSet<&str> = HashSet::new();

            for param in &ov.params {
                let pname = param.name.as_str();
                ov_params.insert(pname);
                documented_params.insert(pname.to_string());

                assert_eq!(
                    builtin_primitive_accepts_port(prim_name, pname),
                    Some(true),
                    "components/lang/{prim_name}.dt declares parameter '{pname}', but compiler primitive '{prim_name}' does not accept it"
                );
                assert_ne!(
                    builtin_primitive_is_immutable_alias(prim_name, pname),
                    Some(true),
                    "components/lang/{prim_name}.dt declares '{pname}' as a parameter, but compiler treats it as an immutable alias"
                );

                let doc_ty = param
                    .type_annotation
                    .as_ref()
                    .map(|t| t.name.as_str())
                    .unwrap_or_else(|| {
                        panic!("components/lang/{prim_name}.dt param '{pname}' missing type annotation")
                    });
                if let Some(compiler_ty) = builtin_primitive_port_type_name(prim_name, pname) {
                    assert_eq!(
                        doc_ty, compiler_ty,
                        "Type mismatch for {prim_name}.{pname}: lang stub has '{doc_ty}', compiler has '{compiler_ty}'"
                    );
                }
            }

            for item in &ov.body {
                if let ContentItem::Alias(a) = item {
                    let aname = a.name.as_str();
                    ov_aliases.insert(aname);
                    let doc_ty = a
                        .type_annotation
                        .as_ref()
                        .map(|t| t.name.as_str())
                        .unwrap_or_else(|| {
                            panic!("components/lang/{prim_name}.dt alias '{aname}' missing type annotation")
                        });
                    if let Some(compiler_ty) = builtin_primitive_port_type_name(prim_name, aname) {
                        assert_eq!(
                            doc_ty, compiler_ty,
                            "Type mismatch for alias {prim_name}.{aname}: lang stub has '{doc_ty}', compiler has '{compiler_ty}'"
                        );
                    }
                    // Every .dt \alias must either be in `output_aliases()` OR be an overload-derived
                    // accepted port omitted from this overload (e.g. Text(x, y) -> baseline; Image(width) -> height).
                    let is_overload_derived =
                        seen_compiler_ports.contains(aname) && !ov_params.contains(aname);
                    assert!(
                        seen_compiler_aliases.contains(aname) || is_overload_derived,
                        "components/lang/{prim_name}.dt overload {oi} declares \\alias '{aname}', which is neither in {prim_name}::output_aliases() nor an omitted accepted port"
                    );
                }
            }

            // Compiler -> .dt: every port in `output_aliases()` must be declared as an \alias on every overload.
            for &compiler_alias in compiler_aliases {
                assert!(
                    ov_aliases.contains(compiler_alias),
                    "Compiler primitive '{prim_name}' declares output alias '{compiler_alias}' in output_aliases(), but it is missing from overload {oi} of components/lang/{prim_name}.dt"
                );
            }
        }

        // Compiler -> .dt: every port in the primitive's exact `accepted_ports()` slice must be documented.
        for &compiler_port in compiler_ports {
            assert!(
                documented_params.contains(compiler_port),
                "Compiler primitive '{prim_name}' accepts port '{compiler_port}' in accepted_ports(), but it is missing from components/lang/{prim_name}.dt"
            );
        }

        // Event handler parity: interactive primitives must document all standard UI event handlers;
        // non-interactive primitives must not declare any `on_*` parameters.
        let accepts_events = builtin_primitive_accepts_event_handlers(prim_name)
            .unwrap_or_else(|| panic!("Missing accepts_event_handlers for '{prim_name}'"));
        if accepts_events {
            for &ev in STANDARD_EVENT_PORTS {
                assert!(
                    documented_params.contains(ev),
                    "Interactive primitive '{prim_name}' should document event handler '{ev}'"
                );
            }
        } else {
            let stray_events: Vec<&String> = documented_params
                .iter()
                .filter(|p| p.starts_with("on_"))
                .collect();
            assert!(
                stray_events.is_empty(),
                "Non-interactive primitive '{prim_name}' must not declare event handlers in components/lang/{prim_name}.dt: {stray_events:?}"
            );
        }
    }

    // 5. No orphaned / undocumented built-ins or enums in components/lang/.
    let mut expected_comps: HashSet<&str> = HashSet::new();
    expected_comps.extend(public_builtin_primitive_names());
    expected_comps.extend(ALL_SPECIAL_BUILTINS.iter().map(|s| s.name));
    expected_comps.extend(compiler_registry.keys().map(|s| s.as_str()));
    let actual_comps: HashSet<&str> = lang_comps.keys().map(|s| s.as_str()).collect();
    assert_eq!(
        actual_comps, expected_comps,
        "Set of \\BuiltIn components in components/lang/ drifted from compiler"
    );

    let expected_enums: HashSet<&str> = compiler_enums.keys().map(|s| s.as_str()).collect();
    let actual_enums: HashSet<&str> = lang_enums.keys().map(|s| s.as_str()).collect();
    assert_eq!(
        actual_enums, expected_enums,
        "Set of \\Enum definitions in components/lang/ drifted from compiler"
    );
}

#[test]
fn test_special_builtins_runtime_and_typecheck_strictness() {
    // 1. Timer runtime evaluation matches TIMER_SPEC.output_ports 1-to-1.
    let mut timer_env = HashMap::new();
    timer_env.insert(
        VarId::new(NodeId::WINDOW, "t"),
        Value::Timer {
            time: 500_000,
            duration: 1_000_000,
            dt: 16_666,
            frames: 30,
        },
    );
    let target_t = std::sync::Arc::new(Expr::MemberAccess(MemberAccessExpr {
        target: std::sync::Arc::new(Expr::Ident(Ident::from_node(NodeId::WINDOW, Span::default()))),
        member: Ident::new("t", Span::default()),
        span: Span::default(),
    }));
    for &(prop, expected_ty) in TIMER_SPEC.output_ports {
        let expr = Expr::MemberAccess(MemberAccessExpr {
            target: target_t.clone(),
            member: Ident::new(prop, Span::default()),
            span: Span::default(),
        });
        let val = eval_expr(&expr, &timer_env)
            .unwrap_or_else(|e| panic!("eval_expr failed for Timer.{prop}: {e}"));
        assert_eq!(
            val.type_name(),
            expected_ty,
            "Runtime type mismatch for Timer.{prop}"
        );
    }
    let bad_timer_expr = Expr::MemberAccess(MemberAccessExpr {
        target: target_t,
        member: Ident::new("nonexistent", Span::default()),
        span: Span::default(),
    });
    assert!(eval_expr(&bad_timer_expr, &timer_env).is_err());

    // 2. IndefiniteTimer runtime evaluation matches INDEFINITE_TIMER_SPEC.output_ports 1-to-1.
    let mut indef_env = HashMap::new();
    indef_env.insert(
        VarId::new(NodeId::WINDOW, "it"),
        Value::IndefiniteTimer {
            time: 250_000,
            dt: 16_666,
            frames: 15,
        },
    );
    let target_it = std::sync::Arc::new(Expr::MemberAccess(MemberAccessExpr {
        target: std::sync::Arc::new(Expr::Ident(Ident::from_node(NodeId::WINDOW, Span::default()))),
        member: Ident::new("it", Span::default()),
        span: Span::default(),
    }));
    for &(prop, expected_ty) in INDEFINITE_TIMER_SPEC.output_ports {
        let expr = Expr::MemberAccess(MemberAccessExpr {
            target: target_it.clone(),
            member: Ident::new(prop, Span::default()),
            span: Span::default(),
        });
        let val = eval_expr(&expr, &indef_env)
            .unwrap_or_else(|e| panic!("eval_expr failed for IndefiniteTimer.{prop}: {e}"));
        assert_eq!(
            val.type_name(),
            expected_ty,
            "Runtime type mismatch for IndefiniteTimer.{prop}"
        );
    }
    let bad_indef_expr = Expr::MemberAccess(MemberAccessExpr {
        target: target_it,
        member: Ident::new("duration", Span::default()),
        span: Span::default(),
    });
    assert!(eval_expr(&bad_indef_expr, &indef_env).is_err());

    // 3. Window strictly rejects unrelated built-in ports (no fallback to default_builtin_port_type).
    let bad_window_access = r#"\Rect(x: 0, y: 0, width: window.cap_height, height: 10px, color: #000)"#;
    let doc = parse_document(bad_window_access).expect("Parse ok");
    expand_document(&doc)
        .expect_err("window.cap_height must be rejected now that Window uses WINDOW_SPEC strictly");
}

#[test]
fn test_leaf_primitives_reject_content_slots_and_output_aliases_are_immutable() {
    let leaf_cases = [
        r#"\Line(x1: 0, y1: 0, x2: 10px, y2: 10px) { \Rect(x: 0, y: 0, width: 1px, height: 1px, color: #000) }"#,
        r#"\Font(size: 12px) { \Rect(x: 0, y: 0, width: 1px, height: 1px, color: #000) }"#,
        r#"\ParagraphStyle() { }"#,
        r#"\let b = \Box(x: 0, y: 0, width: 10px, height: 10px); \let c = \Clip(box: b) { }"#,
        r#"\Pagination() { }"#,
        r#"\PageBreak() { }"#,
        r#"\let v = \PageValue() { };"#,
        r#"\let l = \PageList() { };"#,
    ];
    for src in leaf_cases {
        let doc = parse_document(src).expect("Parse ok");
        let err = expand_document(&doc)
            .expect_err(&format!("Leaf primitive must reject content slot: {src}"));
        assert!(
            err.to_string().contains("content slot"),
            "Expected content slot error for '{src}', got: {err}"
        );
    }

    let immutable_alias_cases = [
        (r#"\let f = \Font(size: 12px, cap_height: 8px);"#, "cap_height"),
        (r#"\let b = \Box(x: 0, y: 0, width: 10px, height: 10px); \let c = \Clip(box: b, clip: window.clip);"#, "clip"),
        (r#"\Line(x1: 0, y1: 0, x2: 10px, y2: 10px, length: 14px)"#, "length"),
        (r#"\AnchorScope(name: "s", path: "/x")"#, "path"),
    ];
    for (src, alias_name) in immutable_alias_cases {
        let doc = parse_document(src).expect("Parse ok");
        let err = expand_document(&doc)
            .expect_err(&format!("Output alias '{alias_name}' must be immutable in: {src}"));
        assert!(
            matches!(err, CompileError::ImmutableAliasPort { ref port, .. } if port == alias_name),
            "Expected ImmutableAliasPort('{alias_name}') for '{src}', got: {err:?}"
        );
    }
}

#[test]
fn test_box_anchor_anchor_scope_and_bookmark_reject_disallowed_ports() {
    let cases = [
        (r#"\Box(width: 100px, height: 50px, color: #ff0000)"#, "color"),
        (r#"\Box(width: 100px, height: 50px, bogus: 1px)"#, "bogus"),
        (r#"\Anchor(name: "a", color: #ff0000)"#, "color"),
        (r#"\Anchor(name: "a", thickness: 2px)"#, "thickness"),
        (r#"\AnchorScope(name: "s", color: #ff0000)"#, "color"),
        (r#"\AnchorScope(name: "s", path: "/custom")"#, "path"),
        (r#"\Bookmark(title: "Ch 1", color: #ff0000)"#, "color"),
        (r#"\Bookmark(title: "Ch 1", radius: 4px)"#, "radius"),
    ];

    for (src, bad_port) in cases {
        let doc = parse_document(src).expect("Source should parse");
        let err = expand_document(&doc)
            .expect_err(&format!("Expected port '{bad_port}' to be rejected in: {src}"));
        assert!(
            err.to_string().contains(bad_port),
            "Expected error mentioning '{bad_port}' for '{src}', got: {err}"
        );
    }
}
