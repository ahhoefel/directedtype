use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use directedtype::ast::{ComponentDef, ComponentKind, ContentItem, EnumDef, Item};
use directedtype::compiler::error::CompileError;
use directedtype::compiler::expand::builtins::{
    builtin_primitive_accepts_port, builtin_primitive_is_immutable_alias,
    builtin_primitive_port_type_name, public_builtin_primitive_names,
    register_builtin_enums_and_components,
};
use directedtype::compiler::expand::expand_document;
use directedtype::compiler::module::{resolve_imports, FsResolver, VirtualResolver};
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

/// Every known non-internal port across all built-in primitives, used to detect when a compiler
/// primitive accepts a new port that has not yet been documented in `components/lang/*.dt`.
const ALL_KNOWN_BUILTIN_PORTS: &[&str] = &[
    "x",
    "y",
    "x1",
    "y1",
    "x2",
    "y2",
    "width",
    "height",
    "z",
    "color",
    "border_color",
    "border_width",
    "radius",
    "radius_left",
    "radius_right",
    "radius_top",
    "radius_bottom",
    "radius_top_left",
    "radius_top_right",
    "radius_bottom_right",
    "radius_bottom_left",
    "shadow",
    "scroll_x",
    "scroll_y",
    "clip",
    "up",
    "box",
    "pagination",
    "cursor",
    "opacity",
    "name",
    "path",
    "id",
    "thickness",
    "cap",
    "baseline",
    "text",
    "font",
    "paragraph_style",
    "underline",
    "text_bounds",
    "src",
    "dpi",
    "alt",
    "family",
    "size",
    "weight",
    "line_height",
    "italic",
    "small_caps",
    "letter_spacing",
    "align",
    "indent",
    "line_break",
    "word_spacing",
    "space_stretch",
    "space_shrink",
    "tolerance",
    "hyphen_penalty",
    "line_penalty",
    "flagged_demerits",
    "fitness_demerits",
    "sentence_space",
    "looseness",
    "outline_title",
    "outline_level",
    "title",
    "target",
    "level",
    "page",
    "orphans",
    "widows",
    "gap",
    "start_page",
    "breakable",
    "keep_with_next",
    "break_before",
    "to",
    "into",
    "from",
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

    // 2. Special non-primitive built-ins (Window, Page, Timer, IndefiniteTimer) must be documented.
    for special in ["Window", "Page", "Timer", "IndefiniteTimer"] {
        assert!(
            lang_comps.contains_key(special),
            "Special built-in '{special}' is missing from components/lang/"
        );
    }

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
    //    - Every accepted port in ALL_KNOWN_BUILTIN_PORTS must be documented in at least one
    //      overload in components/lang/.
    //    - Every declared alias on a primitive must match its compiler port_type (when known).
    for prim_name in public_builtin_primitive_names() {
        assert_eq!(
            builtin_primitive_accepts_port(prim_name, "__arbitrary_unknown_port__"),
            Some(false),
            "Compiler primitive '{prim_name}' must not accept arbitrary unknown ports"
        );

        let overloads = &lang_comps[prim_name];
        let mut documented_params: HashSet<String> = HashSet::new();

        for ov in overloads {
            for param in &ov.params {
                let pname = param.name.as_str();
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
                }
            }
        }

        for &known_port in ALL_KNOWN_BUILTIN_PORTS {
            if builtin_primitive_accepts_port(prim_name, known_port) == Some(true) {
                // `Clip.clip` is a read-only self-reference alias (`\alias clip: Clip;`).
                if prim_name == "Clip" && known_port == "clip" {
                    continue;
                }
                assert!(
                    documented_params.contains(known_port),
                    "Compiler primitive '{prim_name}' accepts port '{known_port}', but it is missing from components/lang/{prim_name}.dt"
                );
            }
        }

        // Interactive primitives that accept `on_click` must document all standard UI event handlers.
        if builtin_primitive_accepts_port(prim_name, "on_click") == Some(true) {
            for &ev in STANDARD_EVENT_PORTS {
                assert!(
                    documented_params.contains(ev),
                    "Interactive primitive '{prim_name}' should document event handler '{ev}'"
                );
            }
        }
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
