use std::path::PathBuf;

use directedtype::ast::ComponentKind;
use directedtype::compdoc::{
    extract_catalog, format_overload_signature, generate_docs_pdf, render_catalog_to_dt,
    BindingKind, ChildrenCardinality, DocGenOptions,
};

#[test]
fn test_extract_all_components_catalog_and_scoped_xrefs() {
    let opts = DocGenOptions::default();
    let catalog = extract_catalog(&[PathBuf::from("components")], &opts)
        .expect("Failed to extract catalog for components/");

    // Should discover root `components` and all organized subpackages
    let pkg_names: Vec<&str> = catalog.packages.iter().map(|p| p.name.as_str()).collect();
    for expected_pkg in [
        "components",
        "components/document",
        "components/graphics",
        "components/layout",
        "components/material",
        "components/text",
    ] {
        assert!(
            pkg_names.contains(&expected_pkg),
            "Missing '{expected_pkg}' package in {pkg_names:?}"
        );
    }

    assert!(catalog.module_count() >= 45, "Expected >=45 modules, got {}", catalog.module_count());
    assert!(
        catalog.component_count() >= 85,
        "Expected >=85 components, got {}",
        catalog.component_count()
    );
    assert!(
        catalog.overload_count() >= 165,
        "Expected >=165 overloads, got {}",
        catalog.overload_count()
    );
    assert!(catalog.enum_count() >= 22, "Expected >=22 enums, got {}", catalog.enum_count());
    assert!(
        catalog.binding_count() >= 30,
        "Expected >=30 top-level bindings, got {}",
        catalog.binding_count()
    );

    // Helper to find a module by relative path
    let find_mod = |rel: &str| {
        catalog
            .packages
            .iter()
            .flat_map(|p| &p.modules)
            .find(|m| m.rel_path == rel)
            .unwrap_or_else(|| panic!("Module '{rel}' not found"))
    };

    // 1. TextBounds.dt: colocated TextStart & TextEnd enums linked from TextBounds params
    let tb_mod = find_mod("components/text/TextBounds.dt");
    let ts_enum = tb_mod.enums.iter().find(|e| e.name == "TextStart").unwrap();
    let te_enum = tb_mod.enums.iter().find(|e| e.name == "TextEnd").unwrap();
    let tb_comp = tb_mod.components.iter().find(|c| c.name == "TextBounds").unwrap();
    let start_param = tb_comp.overloads[0].params.iter().find(|p| p.name == "start_at").unwrap();
    let end_param = tb_comp.overloads[0].params.iter().find(|p| p.name == "end_at").unwrap();
    assert_eq!(start_param.type_anchor.as_deref(), Some(ts_enum.anchor.as_str()));
    assert_eq!(end_param.type_anchor.as_deref(), Some(te_enum.anchor.as_str()));

    // 2. Link.dt: colocated LinkStyle + \Span Link with 3 overloads, env params, and inferred alias types
    let link_mod = find_mod("components/text/Link.dt");
    let link_style_comp = link_mod.components.iter().find(|c| c.name == "LinkStyle").unwrap();
    let link_comp = link_mod.components.iter().find(|c| c.name == "Link").unwrap();
    assert_eq!(link_comp.kind, ComponentKind::Span);
    assert_eq!(link_comp.overloads.len(), 3);
    assert_eq!(
        format_overload_signature("Link", &link_comp.overloads[0]),
        "\\Span Link(url, ...) { ... }"
    );
    assert_eq!(
        format_overload_signature("Link", &link_comp.overloads[1]),
        "\\Span Link(to, ...) { ... }"
    );
    assert_eq!(
        format_overload_signature("Link", &link_comp.overloads[2]),
        "\\Span Link(to, text, ...)"
    );
    let env_ls = link_comp.overloads[0]
        .params
        .iter()
        .find(|p| p.name == "link_style")
        .unwrap();
    assert!(env_ls.is_env);
    assert_eq!(env_ls.type_anchor.as_deref(), Some(link_style_comp.anchor.as_str()));
    assert_eq!(
        link_comp.overloads[0]
            .aliases
            .iter()
            .find(|a| a.name == "current_color")
            .and_then(|a| a.type_name.as_deref()),
        Some("Color")
    );
    assert_eq!(
        link_comp.overloads[0]
            .aliases
            .iter()
            .find(|a| a.name == "underline")
            .and_then(|a| a.type_name.as_deref()),
        Some("Boolean")
    );

    // 3. InlineImage.dt: \InlineBox
    let inline_img_mod = find_mod("components/text/InlineImage.dt");
    assert_eq!(inline_img_mod.components[0].kind, ComponentKind::InlineBox);
    assert_eq!(inline_img_mod.components[0].overloads.len(), 3);

    // 4. Bold.dt: provided \env font inferred as Font
    let bold_mod = find_mod("components/text/Bold.dt");
    let bold_env_font = bold_mod.components[0].overloads[0]
        .provided_envs
        .iter()
        .find(|e| e.name == "font")
        .unwrap();
    assert_eq!(bold_env_font.type_name.as_deref(), Some("Font"));

    // 5. Center.dt overload with zero parameters and single child (`{ . }`), plus material Switch (`<= 1`) and VDivider (`== 0`)
    let center_mod = find_mod("components/layout/Center.dt");
    assert_eq!(
        format_overload_signature("Center", center_mod.components[0].overloads.last().unwrap()),
        "\\Component Center() { . }"
    );
    let mat_div_mod = find_mod("components/material/Divider.dt");
    let mat_hdiv = mat_div_mod.components.iter().find(|c| c.name == "HDivider").unwrap();
    let mat_vdiv = mat_div_mod.components.iter().find(|c| c.name == "VDivider").unwrap();
    assert_eq!(mat_hdiv.overloads[0].children_cardinality, ChildrenCardinality::Single);
    assert_eq!(mat_vdiv.overloads[0].children_cardinality, ChildrenCardinality::None);

    // 6. Verify ZERO language-generated spatial aliases (`left`, `top`, `right`, `bottom`) remain,
    // and ZERO aliases / provided envs / bindings across ALL 60 files lack a concrete inferred type!
    let mut missing_types = Vec::new();
    for pkg in &catalog.packages {
        for m in &pkg.modules {
            for c in &m.components {
                for (oi, ov) in c.overloads.iter().enumerate() {
                    for a in &ov.aliases {
                        assert!(
                            !matches!(a.name.as_str(), "left" | "top" | "right" | "bottom"),
                            "Spatial alias '{}' in {}::{} overload {} should be excluded",
                            a.name,
                            m.rel_path,
                            c.name,
                            oi
                        );
                        if a.type_name.is_none() {
                            missing_types.push(format!(
                                "{}::{}[{}].alias {}",
                                m.rel_path, c.name, oi, a.name
                            ));
                        }
                    }
                    for e in &ov.provided_envs {
                        if e.type_name.is_none() {
                            missing_types.push(format!(
                                "{}::{}[{}].env {}",
                                m.rel_path, c.name, oi, e.name
                            ));
                        }
                    }
                    for cp in &ov.children_pushed_ports {
                        if cp.type_name.is_none() {
                            missing_types.push(format!(
                                "{}::{}[{}].children {}",
                                m.rel_path, c.name, oi, cp.name
                            ));
                        }
                    }
                }
            }
            for b in &m.bindings {
                if b.type_name.is_none() {
                    missing_types.push(format!("{}::binding {}", m.rel_path, b.name));
                }
            }
        }
    }
    assert!(
        missing_types.is_empty(),
        "Expected all aliases, envs, and bindings to have inferred types, missing: {missing_types:#?}"
    );

    // 5. material/Palette.dt: Enum + Component + top-level \let bindings with linked inferred types
    let palette_mod = find_mod("components/material/Palette.dt");
    let mat_color = &palette_mod.enums[0];
    assert_eq!(mat_color.name, "MaterialColor");
    assert_eq!(mat_color.variants.len(), 7);
    let color_channel = &palette_mod.components[0];
    assert_eq!(color_channel.name, "ColorChannel");
    let mat_primary = palette_mod
        .bindings
        .iter()
        .find(|b| b.name == "material_primary")
        .unwrap();
    assert_eq!(mat_primary.kind, BindingKind::Let);
    assert_eq!(mat_primary.type_name.as_deref(), Some("ColorChannel"));
    assert_eq!(
        mat_primary.type_anchor.as_deref(),
        Some(color_channel.anchor.as_str())
    );

    // 6. Scoped disambiguation: example kit ButtonVariant vs material ButtonVariant, plus Card/CardFit/Badge
    let combined_catalog = extract_catalog(
        &[
            PathBuf::from("components"),
            PathBuf::from("examples/components"),
        ],
        &opts,
    )
    .expect("Failed to extract combined catalog");
    let find_combined = |rel: &str| {
        combined_catalog
            .packages
            .iter()
            .flat_map(|p| &p.modules)
            .find(|m| m.rel_path == rel)
            .unwrap_or_else(|| panic!("Module '{rel}' not found"))
    };

    let card_mod = find_combined("examples/components/Card.dt");
    let card_style = card_mod.components.iter().find(|c| c.name == "CardStyle").unwrap();
    let card = card_mod.components.iter().find(|c| c.name == "Card").unwrap();
    assert_eq!(card.overloads.len(), 4);
    assert_eq!(
        card.overloads[0].params.iter().find(|p| p.name == "style").unwrap().type_anchor.as_deref(),
        Some(card_style.anchor.as_str())
    );
    let card_fit = card_mod.components.iter().find(|c| c.name == "CardFit").unwrap();
    assert_eq!(card_fit.overloads[0].asserts.len(), 1);
    assert_eq!(card_fit.overloads[0].children_cardinality, ChildrenCardinality::Single);
    assert_eq!(
        format_overload_signature("CardFit", &card_fit.overloads[0]),
        "\\Component CardFit(style, x, y, gap) { . }"
    );

    let badge_mod = find_combined("examples/components/Badge.dt");
    assert_eq!(badge_mod.components[0].kind, ComponentKind::InlineBox);
    assert_eq!(
        format_overload_signature("Badge", &badge_mod.components[0].overloads[0]),
        "\\InlineBox Badge(...) { ... }"
    );

    let ex_btn_mod = find_combined("examples/components/Button.dt");
    let mat_btn_mod = find_combined("components/material/Button.dt");

    let ex_bv_enum = ex_btn_mod
        .enums
        .iter()
        .find(|e| e.name == "ButtonVariant")
        .unwrap();
    let ex_btn_comp = ex_btn_mod.components.iter().find(|c| c.name == "Button").unwrap();
    let ex_btn_variant_param = ex_btn_comp
        .overloads
        .iter()
        .flat_map(|o| &o.params)
        .find(|p| p.type_name.as_deref() == Some("ButtonVariant"))
        .expect("example Button should have a ButtonVariant parameter");
    assert_eq!(
        ex_btn_variant_param.type_anchor.as_deref(),
        Some(ex_bv_enum.anchor.as_str()),
        "examples/components/Button.dt must link ButtonVariant to its own local ButtonVariant enum"
    );

    let mat_bv_enum = mat_btn_mod
        .enums
        .iter()
        .find(|e| e.name == "ButtonVariant")
        .unwrap();
    let mat_btn_variant_param = mat_btn_mod.components[0]
        .overloads
        .iter()
        .flat_map(|o| &o.params)
        .find(|p| p.type_name.as_deref() == Some("ButtonVariant"))
        .expect("material Button should have a ButtonVariant parameter");
    assert_eq!(
        mat_btn_variant_param.type_anchor.as_deref(),
        Some(mat_bv_enum.anchor.as_str()),
        "components/material/Button.dt must link ButtonVariant to its own local ButtonVariant enum"
    );
    assert_ne!(ex_bv_enum.anchor, mat_bv_enum.anchor);

    // 7. Generated .dt parses cleanly back through DirectedType's parser
    let dt_source = render_catalog_to_dt(&catalog);
    directedtype::parse(&dt_source).expect("Generated full component reference .dt must parse cleanly");
}

#[cfg(feature = "pdf")]
#[test]
fn test_generate_internally_linked_pdf_with_toc_and_outlines() {
    use directedtype::pdf::PdfOptions;

    let inputs = vec![
        PathBuf::from("components/Align.dt"),
        PathBuf::from("components/text/Bold.dt"),
        PathBuf::from("components/text/TextBounds.dt"),
        PathBuf::from("components/layout/VStack.dt"),
        PathBuf::from("examples/components/Badge.dt"),
        PathBuf::from("examples/components/Button.dt"),
        PathBuf::from("examples/components/Card.dt"),
        PathBuf::from("components/material/Palette.dt"),
        PathBuf::from("components/material/Button.dt"),
    ];
    let doc_opts = DocGenOptions::default();
    let pdf_opts = PdfOptions {
        compressed: false,
        deterministic: true,
        strict_links: true,
        ..Default::default()
    };

    let (catalog, _dt_source, pdf_bytes) =
        generate_docs_pdf(&inputs, &doc_opts, &pdf_opts).expect("Failed to compile docs PDF");

    assert_eq!(catalog.packages.len(), 5);
    assert!(pdf_bytes.starts_with(b"%PDF-"), "Output must be a valid PDF");

    let text = String::from_utf8_lossy(&pdf_bytes);
    assert!(
        text.contains("/Outlines"),
        "Generated PDF must contain document /Outlines"
    );
    assert!(
        text.contains("/Subtype/Link") || text.contains("/Subtype /Link"),
        "Generated PDF must contain clickable /Link annotations"
    );
    assert!(
        text.contains("/XYZ "),
        "Generated PDF must contain internal /XYZ anchor destinations"
    );
}
