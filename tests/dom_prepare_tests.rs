use std::path::{Path, PathBuf};

use directedtype::compiler::module::{normalize_path, FileResolver, FsResolver, VirtualResolver};
use directedtype::compiler::paginate::{paginate_compiled_document, PaginatedDocument};
use directedtype::compiler::CompiledDocument;
use directedtype::component::ComponentRegistry;
use directedtype::dom::{Dom, DomError};
use directedtype::{parse, Value};

/// Hybrid resolver that serves in-memory test assets (`refs.bib`, `.wasm`, virtual `.dt`)
/// while forwarding stdlib `components/*.dt` lookups to the workspace repository.
struct TestHybridResolver {
    virtual_fs: VirtualResolver,
}

impl TestHybridResolver {
    fn new(virtual_fs: VirtualResolver) -> Self {
        Self { virtual_fs }
    }
}

impl FileResolver for TestHybridResolver {
    fn read(&self, path: &Path) -> Result<String, String> {
        if let Ok(s) = self.virtual_fs.read(path) {
            return Ok(s);
        }
        let norm = normalize_path(path);
        if norm.starts_with("components") {
            return FsResolver.read(path);
        }
        Err(format!("Virtual file not found: {}", path.display()))
    }

    fn read_bytes(&self, path: &Path) -> Result<Vec<u8>, String> {
        if let Ok(b) = self.virtual_fs.read_bytes(path) {
            return Ok(b);
        }
        let norm = normalize_path(path);
        if norm.starts_with("components") {
            return FsResolver.read_bytes(path);
        }
        Err(format!("Virtual binary not found: {}", path.display()))
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, String> {
        Ok(normalize_path(path))
    }
}

fn prepare_compile_and_paginate_with<R: FileResolver>(
    src: &str,
    base_dir: &Path,
    resolver: &R,
    page_w: f64,
    page_h: f64,
) -> (PaginatedDocument, Dom) {
    let parsed = parse(src).expect("source should parse");
    let mut dom = Dom::from_document_with_window(&parsed, page_w, page_h).expect("Dom init");
    let mut registry = ComponentRegistry::standard();
    registry.auto_discover_wasm_for_document(&parsed, base_dir, resolver);
    dom.prepare_with_resolver(base_dir, resolver, Some(&registry))
        .expect("Dom::prepare_with_resolver should succeed");
    let prepared_doc = dom.to_document().expect("dom.to_document");
    let compiled = CompiledDocument::compile_with_registry_and_dpi(
        &prepared_doc,
        page_w,
        page_h,
        72.0,
        base_dir,
        resolver,
        &registry,
    )
    .expect("compiled prepared document");
    let paginated =
        paginate_compiled_document(&compiled, page_w, page_h, 72.0).expect("paginated");
    (paginated, dom)
}

#[test]
fn test_lossless_mixed_content_inline_replace_and_directives() {
    let src = r#"
    \use "components/Bold.dt"
    \use "components/VStack.dt"

    \env font = \Font(family: "Arial", size: 12pt, line_height: 16pt);

    \Span InlineCite(key: String, text: String = "[" + key + "]", env font: Font) {
        \alias current_color = #1d4ed8;
    }

    \VStack(width: 400pt) {
        \let prose_color = #0f172a;
        \Text(id: "para", font: \Font(color: prose_color), width: 400pt) {
            See \InlineCite(key: "knuth84") and \Bold { TeX } for details.
        }
    }
    "#;

    let parsed = parse(src).expect("parse mixed content");
    let mut dom = Dom::from_document_with_window(&parsed, 500.0, 300.0).expect("dom init");
    let cites = dom
        .query_by_tag(None, &["InlineCite"])
        .expect("query InlineCite");
    assert_eq!(cites.len(), 1);
    let cite_h = cites[0];
    let parent_h = dom.parent(cite_h).expect("parent").expect("attached");

    let replacement = dom.create_element("Bold", vec![]);
    dom.set_text(replacement, "[1]").expect("set_text");
    dom.replace_child(parent_h, cite_h, replacement)
        .expect("replace_child");

    let roundtrip_doc = dom.to_document().expect("to_document");
    let compiled = CompiledDocument::compile_with_registry_and_dpi(
        &roundtrip_doc,
        500.0,
        300.0,
        72.0,
        Path::new("."),
        &FsResolver,
        &ComponentRegistry::standard(),
    )
    .expect("compile after inline replace");

    let para_node = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.properties.get("id").and_then(Value::as_str) == Some("para"))
        .expect("para node exists");
    let text = para_node.text_content.as_deref().unwrap_or("");
    assert!(
        text.contains("See") && text.contains("[1]") && text.contains("and") && text.contains("TeX") && text.contains("for details."),
        "Mixed content order should be preserved after inline replace_child, got {:?}",
        text
    );
    let pos_see = text.find("See").unwrap();
    let pos_num = text.find("[1]").unwrap();
    let pos_tex = text.find("TeX").unwrap();
    let pos_det = text.find("for details.").unwrap();
    assert!(
        pos_see < pos_num && pos_num < pos_tex && pos_tex < pos_det,
        "Expected inline order preserved in {:?}",
        text
    );
}

#[test]
fn test_book_toc_autonumbering_multipage_convergence_and_pdf_links() {
    // 10 chapters * (Chapter + H1 + H2) => >24 TOC entries; on a compact 5.0in page the TOC itself
    // spans 2+ pages, pushing Preface and Chapter 1 forward and exercising .page convergence.
    let mut chapters_dt = String::new();
    for i in 1..=8 {
        chapters_dt.push_str(&format!(
            r#"
        \Chapter(title: "Topic {i}") {{
            \H1(title: "Overview {i}") {{
                \H2(title: "Details {i}") {{
                    \Text(width: parent.width) {{
                        Body prose for topic {i}.\Footnote {{ Note for chapter {i}. }}\Sidenote {{ Margin {i}. }}
                    }}
                    \Figure(caption: "Diagram {i}") {{
                        \Rect(width: 120pt, height: 24pt, color: #e2e8f0)
                    }}
                }}
            }}
        }}
        "#
        ));
    }

    let src = format!(
        r#"
    \use "components/Book.dt"
    \use "components/TableOfContents.dt"

    \Book(
        title: "Automated Book Typography",
        two_sided: true,
        reset_footnotes_per_chapter: true,
        reset_figures_per_chapter: true,
        page_width: 8.5in,
        page_height: 5.0in,
        margin_inner: 0.75in,
        margin_outer: 1.85in,
        margin_top: 0.75in,
        margin_bottom: 0.75in,
        sidenote_width: 1.2in,
        sidenote_gutter: 0.2in,
        show_total_pages: true,
        font: \Font(family: "Arial", size: 10.5pt, line_height: 14pt)
    ) {{
        \TableOfContents(title: "Contents", max_depth: 3)

        \Chapter(title: "Preface", unnumbered: true) {{
            \Text(width: parent.width) {{
                Unnumbered preface introducing the manual.
            }}
        }}

        {chapters_dt}

        \Chapter(number: "A", title: "Colophon") {{
            \Text(width: parent.width) {{ Explicit appendix label A preserved. }}
        }}
    }}
    "#
    );

    let (paginated, prepared_dom) =
        prepare_compile_and_paginate_with(&src, Path::new("."), &FsResolver, 612.0, 360.0);

    let get_port_str = |h, port| -> Option<String> {
        match prepared_dom.declared_ports(h).unwrap().get(port) {
            Some(directedtype::ast::Expr::Literal(directedtype::ast::Literal::String(s, _))) => {
                Some(s.clone())
            }
            _ => None,
        }
    };

    // 1. Verify hierarchical numbering on the prepared DOM
    let ch_handles = prepared_dom
        .query_by_tag(None, &["Chapter"])
        .expect("query Chapter");
    assert_eq!(ch_handles.len(), 10);
    // Preface was unnumbered: true -> no number added
    assert!(
        get_port_str(ch_handles[0], "number").is_none(),
        "Preface with unnumbered: true must not get an auto number"
    );
    assert_eq!(get_port_str(ch_handles[1], "number").as_deref(), Some("1"));
    assert_eq!(get_port_str(ch_handles[9], "number").as_deref(), Some("A"));

    // Inspect H1 / H2 / Figure / Footnote numbering
    let h1s = prepared_dom.query_by_tag(None, &["H1"]).unwrap();
    let h2s = prepared_dom.query_by_tag(None, &["H2"]).unwrap();
    let figs = prepared_dom.query_by_tag(None, &["Figure"]).unwrap();
    let fns = prepared_dom.query_by_tag(None, &["Footnote"]).unwrap();
    assert_eq!(get_port_str(h1s[0], "number").as_deref(), Some("1.1"));
    assert_eq!(get_port_str(h2s[0], "number").as_deref(), Some("1.1.1"));
    assert_eq!(get_port_str(figs[0], "number").as_deref(), Some("Figure 1.1"));
    // Footnote resets per chapter (`reset_footnotes_per_chapter: true`) -> chapter 2 footnote is "1" again!
    assert_eq!(get_port_str(fns[1], "marker").as_deref(), Some("1"));

    // 2. Verify TOC itself spans >1 frontmatter page and every TocEntry's displayed page matches actual Chapter page!
    let mut toc_pages_seen = std::collections::BTreeSet::new();
    let mut toc_entries_verified = 0usize;
    for scene in &paginated.pages {
        let Some(inc) = &scene.included_nodes else {
            continue;
        };
        for node in &scene.layout.nodes {
            if !inc.contains(&node.id) || node.name != "TocEntry" {
                continue;
            }
            toc_pages_seen.insert(scene.page_number);
            let title = node
                .properties
                .get("title")
                .and_then(Value::as_str)
                .expect("TocEntry title")
                .to_string();
            let target_id = node
                .properties
                .get("target")
                .and_then(Value::as_node)
                .expect("TocEntry target node");
            let actual_target_page = scene
                .layout
                .nodes
                .get(target_id.0)
                .and_then(|t| t.properties.get("page"))
                .and_then(Value::as_i64)
                .expect("target anchor has settled page");

            // Find rendered page_label link inside this TocEntry (second Text child holding numeric text)
            let mut label_pages = Vec::new();
            for child_id in &node.children {
                if let Some(ch) = scene.layout.get_node(*child_id) {
                    if ch.name == "Text" {
                        if let Some(rendered) = ch.properties.get("text").and_then(Value::as_str) {
                            if let Ok(p) = rendered.trim().parse::<i64>() {
                                label_pages.push(p);
                            }
                        }
                    }
                }
            }
            assert_eq!(
                label_pages,
                vec![actual_target_page],
                "TocEntry '{}' displayed page {:?} must equal actual target anchor page {}",
                title,
                label_pages,
                actual_target_page
            );
            toc_entries_verified += 1;
        }
    }
    assert!(
        toc_pages_seen.len() >= 2,
        "TableOfContents must span at least 2 frontmatter pages to test shift convergence, landed on {:?}",
        toc_pages_seen
    );
    // Preface + 8*(Chapter+H1+H2) + Appendix = 1 + 24 + 1 = 26 entries
    assert_eq!(toc_entries_verified, 26);

    #[cfg(feature = "pdf")]
    {
        use directedtype::pdf::{compile_document_to_pdf, PdfOptions};
        let parsed = parse(&src).unwrap();
        let mut reg = ComponentRegistry::standard();
        reg.auto_discover_wasm_for_document(&parsed, Path::new("."), &FsResolver);
        let opts = PdfOptions {
            compressed: false,
            strict_links: true,
            ..Default::default()
        };
        let pdf_bytes = compile_document_to_pdf(&parsed, Path::new("."), &FsResolver, &reg, &opts)
            .expect("Book + TOC PDF export with strict_links must succeed");
        assert!(pdf_bytes.starts_with(b"%PDF-"));
        let pdf_text = String::from_utf8_lossy(&pdf_bytes);
        assert!(
            (pdf_text.contains("/Subtype/Link") || pdf_text.contains("/Subtype /Link"))
                && pdf_text.contains("/XYZ "),
            "Expected clickable internal /Link annotations with /XYZ destinations in PDF"
        );
    }
}

#[test]
fn test_bibliography_and_cite_citation_and_alpha_sort_virtual_resolver() {
    let bib_content = r#"
@article{knuth84,
  author = {Donald E. Knuth},
  title = {Literate Programming},
  journal = {The Computer Journal},
  year = {1984}
}

@article{turing50,
  author = {Alan M. Turing},
  title = {Computing Machinery and Intelligence},
  journal = {Mind},
  year = {1950}
}
"#;

    let mut vfs = VirtualResolver::new();
    vfs.insert("refs.bib", bib_content);
    let resolver = TestHybridResolver::new(vfs);

    // 1. Citation order (`sort: "citation"`): knuth84 cited first -> [1], turing50 cited second -> [2]
    let src_cite_order = r#"
    \use "components/Article.dt"
    \use "components/Bibliography.dt"
    \use "components/Link.dt"

    \Article(
        title: "Citation Order Paper",
        page_width: 8.5in,
        page_height: 11in,
        font: \Font(family: "Arial", size: 11pt, line_height: 15pt)
    ) {
        \Text(id: "body_para", width: parent.width) {
            Literate systems \Cite(key: "knuth84") pair with machine intelligence \Cite(key: "turing50") and again \Cite(key: "knuth84").
        }
        \Bibliography(title: "References", sort: "citation", src: "refs.bib")
    }
    "#;

    let (pag_cite, _) = prepare_compile_and_paginate_with(
        src_cite_order,
        Path::new("."),
        &resolver,
        612.0,
        792.0,
    );
    let body_node = pag_cite
        .layout
        .nodes
        .iter()
        .find(|n| n.properties.get("id").and_then(Value::as_str) == Some("body_para"))
        .expect("body_para exists");
    let rendered = body_node.text_content.as_deref().unwrap_or("");
    let i1 = rendered.find("[1]").expect("[1]");
    let i2 = rendered.find("[2]").expect("[2]");
    let i3 = rendered.rfind("[1]").expect("second [1]");
    assert!(
        i1 < i2 && i2 < i3,
        "Expected '[1] ... [2] ... [1]' in citation order, got {:?}",
        rendered
    );

    // 2. Alphabetical order (`sort: "alpha"`): Alan M. Turing sorts before Donald E. Knuth -> [2] then [1]
    let src_alpha = src_cite_order.replace("sort: \"citation\"", "sort: \"alpha\"");
    let (pag_alpha, _) =
        prepare_compile_and_paginate_with(&src_alpha, Path::new("."), &resolver, 612.0, 792.0);
    let body_alpha = pag_alpha
        .layout
        .nodes
        .iter()
        .find(|n| n.properties.get("id").and_then(Value::as_str) == Some("body_para"))
        .expect("body_para alpha exists");
    let rendered_alpha = body_alpha.text_content.as_deref().unwrap_or("");
    let a2 = rendered_alpha.find("[2]").expect("[2] for Knuth");
    let a1 = rendered_alpha.find("[1]").expect("[1] for Turing");
    let a2_last = rendered_alpha.rfind("[2]").expect("second [2] for Knuth");
    assert!(
        a2 < a1 && a1 < a2_last,
        "Expected '[2] ... [1] ... [2]' in alpha order, got {:?}",
        rendered_alpha
    );

    let bib_entries: Vec<&str> = pag_alpha
        .layout
        .nodes
        .iter()
        .filter(|n| n.name == "BibEntry")
        .filter_map(|n| n.properties.get("author").and_then(Value::as_str))
        .collect();
    assert_eq!(bib_entries, vec!["Alan M. Turing", "Donald E. Knuth"]);

    #[cfg(feature = "pdf")]
    {
        use directedtype::pdf::{compile_document_to_pdf, PdfOptions};
        let parsed = parse(&src_alpha).unwrap();
        let mut dom = Dom::from_document_with_window(&parsed, 612.0, 792.0).unwrap();
        dom.prepare_with_resolver(Path::new("."), &resolver, Some(&ComponentRegistry::standard()))
            .unwrap();
        let prepared = dom.to_document().unwrap();
        let pdf = compile_document_to_pdf(
            &prepared,
            Path::new("."),
            &resolver,
            &ComponentRegistry::standard(),
            &PdfOptions {
                compressed: false,
                strict_links: true,
                ..Default::default()
            },
        )
        .expect("Bibliography PDF with internal links");
        let text = String::from_utf8_lossy(&pdf);
        assert!(
            (text.contains("/Subtype/Link") || text.contains("/Subtype /Link"))
                && text.contains("/XYZ ")
        );
    }
}

#[test]
fn test_wasm_wat_dt_dom_transform_and_sandbox_rollbacks() {
    // 1. Successful dt_dom mutation via `\use "transform.wasm"` in VirtualResolver
    let wat_ok = r#"
        (module
            (import "dt_dom" "__dt_dom_set_text" (func $set_text (param i64 i32 i32) (result i64)))
            (import "dt_dom" "__dt_dom_create_text" (func $create_text (param i32 i32) (result i64)))
            (memory (export "memory") 1)
            (global $bump (mut i32) (i32.const 1024))
            (data (i32.const 0) "hello from wat")
            (func (export "__dt_alloc") (param $sz i32) (result i32)
                (local $p i32)
                (local.set $p (global.get $bump))
                (global.set $bump (i32.add (local.get $p) (local.get $sz)))
                (local.get $p)
            )
            (func (export "__dt_has_prepare") (result i32)
                (i32.const 1)
            )
            (func (export "__dt_prepare") (param $self i64) (result i64)
                (drop (call $set_text (local.get $self) (i32.const 0) (i32.const 14)))
                (i64.const 0)
            )
        )
    "#;
    let mut vfs_ok = VirtualResolver::new();
    vfs_ok.insert_bytes("transform.wasm", wat_ok.as_bytes());
    let src_ok = r#"
        \use "transform.wasm"
        \Text(id: "target") { original }
    "#;
    let parsed_ok = parse(src_ok).unwrap();
    let mut dom_ok = Dom::from_document(&parsed_ok).unwrap();
    dom_ok
        .prepare_with_resolver(Path::new("."), &vfs_ok, None)
        .expect("WAT transform succeeds");
    let root_h = dom_ok.roots()[0];
    assert_eq!(dom_ok.text_content(root_h).unwrap(), Some("hello from wat"));

    // 2. Fuel exhaustion (`10_000_000`) -> PrepareTrap + rollback of earlier mutation
    let wat_fuel = r#"
        (module
            (import "dt_dom" "__dt_dom_set_text" (func $set_text (param i64 i32 i32) (result i64)))
            (memory (export "memory") 1)
            (global $bump (mut i32) (i32.const 1024))
            (data (i32.const 0) "mutated_before_trap")
            (func (export "__dt_alloc") (param $sz i32) (result i32)
                (local $p i32)
                (local.set $p (global.get $bump))
                (global.set $bump (i32.add (local.get $p) (local.get $sz)))
                (local.get $p)
            )
            (func (export "__dt_prepare") (param $self i64) (result i64)
                (drop (call $set_text (local.get $self) (i32.const 0) (i32.const 19)))
                (loop $inf
                    (br $inf)
                )
                (i64.const 0)
            )
        )
    "#;
    let mut vfs_fuel = VirtualResolver::new();
    vfs_fuel.insert_bytes("transform.wasm", wat_fuel.as_bytes());
    let mut dom_fuel = Dom::from_document(&parsed_ok).unwrap();
    let root_fuel = dom_fuel.roots()[0];
    let err_fuel = dom_fuel
        .prepare_with_resolver(Path::new("."), &vfs_fuel, None)
        .expect_err("Infinite loop must exhaust 10M fuel");
    assert!(
        matches!(err_fuel, DomError::PrepareTrap(_)),
        "Expected PrepareTrap, got {:?}",
        err_fuel
    );
    assert_eq!(
        dom_fuel.text_content(root_fuel).unwrap(),
        Some("original"),
        "Trap must roll back prior mutation atomically"
    );

    // 3. Asset escape (`../secret.txt`) -> AssetAccessDenied + rollback
    let wat_escape = r#"
        (module
            (import "dt_dom" "__dt_dom_set_text" (func $set_text (param i64 i32 i32) (result i64)))
            (import "dt_dom" "__dt_host_read_asset" (func $read_asset (param i32 i32) (result i64)))
            (memory (export "memory") 1)
            (global $bump (mut i32) (i32.const 1024))
            (data (i32.const 0) "mutated_before_escape")
            (data (i32.const 64) "../secret.txt")
            (func (export "__dt_alloc") (param $sz i32) (result i32)
                (local $p i32)
                (local.set $p (global.get $bump))
                (global.set $bump (i32.add (local.get $p) (local.get $sz)))
                (local.get $p)
            )
            (func (export "__dt_prepare") (param $self i64) (result i64)
                (drop (call $set_text (local.get $self) (i32.const 0) (i32.const 21)))
                (drop (call $read_asset (i32.const 64) (i32.const 13)))
                (i64.const 0)
            )
        )
    "#;
    let mut vfs_escape = VirtualResolver::new();
    vfs_escape.insert_bytes("transform.wasm", wat_escape.as_bytes());
    vfs_escape.insert("../secret.txt", "TOP SECRET");
    let mut dom_escape = Dom::from_document(&parsed_ok).unwrap();
    let root_escape = dom_escape.roots()[0];
    let err_escape = dom_escape
        .prepare_with_resolver(Path::new("."), &vfs_escape, None)
        .expect_err("Path traversal outside base_dir must be denied");
    assert!(
        matches!(err_escape, DomError::AssetAccessDenied(_)),
        "Expected AssetAccessDenied, got {:?}",
        err_escape
    );
    assert_eq!(
        dom_escape.text_content(root_escape).unwrap(),
        Some("original"),
        "AssetAccessDenied must roll back prior mutation atomically"
    );

    // 4. Self-spawning cycle exceeding MAX_PREPARE_PASSES = 8 -> PrepareCycle + rollback
    let wat_cycle = r#"
        (module
            (import "dt_dom" "__dt_dom_create_element" (func $create (param i32 i32) (result i64)))
            (import "dt_dom" "__dt_dom_append_child" (func $append (param i64 i64) (result i64)))
            (memory (export "memory") 1)
            (global $bump (mut i32) (i32.const 1024))
            ;; JSON CreateElementWire for {"tag":"LoopNode","key":null,"ports":[]} (40 bytes)
            (data (i32.const 0) "{\"tag\":\"LoopNode\",\"key\":null,\"ports\":[]}")
            (func (export "__dt_alloc") (param $sz i32) (result i32)
                (local $p i32)
                (local.set $p (global.get $bump))
                (global.set $bump (i32.add (local.get $p) (local.get $sz)))
                (local.get $p)
            )
            (func $parse_ok_u64 (param $packed i64) (result i64)
                (local $ptr i32)
                (local $end i32)
                (local $ch i32)
                (local $acc i64)
                (local $in_digits i32)
                (local.set $ptr (i32.wrap_i64 (i64.shr_u (local.get $packed) (i64.const 32))))
                (local.set $end (i32.add (local.get $ptr) (i32.wrap_i64 (local.get $packed))))
                (local.set $acc (i64.const 0))
                (local.set $in_digits (i32.const 0))
                (block $done
                    (loop $scan
                        (br_if $done (i32.ge_u (local.get $ptr) (local.get $end)))
                        (local.set $ch (i32.load8_u (local.get $ptr)))
                        (if (i32.and (i32.ge_u (local.get $ch) (i32.const 48))
                                     (i32.le_u (local.get $ch) (i32.const 57)))
                            (then
                                (local.set $in_digits (i32.const 1))
                                (local.set $acc
                                    (i64.add
                                        (i64.mul (local.get $acc) (i64.const 10))
                                        (i64.extend_i32_u (i32.sub (local.get $ch) (i32.const 48)))))
                            )
                            (else
                                (br_if $done (local.get $in_digits))
                            )
                        )
                        (local.set $ptr (i32.add (local.get $ptr) (i32.const 1)))
                        (br $scan)
                    )
                )
                (local.get $acc)
            )
            (func (export "__dt_prepare") (param $self i64) (result i64)
                (local $res i64)
                (local $child i64)
                (local.set $res (call $create (i32.const 0) (i32.const 40)))
                (local.set $child (call $parse_ok_u64 (local.get $res)))
                (drop (call $append (local.get $self) (local.get $child)))
                (i64.const 0)
            )
        )
    "#;
    let mut vfs_cycle = VirtualResolver::new();
    vfs_cycle.insert_bytes("LoopNode.wasm", wat_cycle.as_bytes());
    let src_cycle = r#"
        \Component LoopNode() {
            \Children
        }
        \LoopNode()
    "#;
    let parsed_cycle = parse(src_cycle).unwrap();
    let mut dom_cycle = Dom::from_document(&parsed_cycle).unwrap();
    let err_cycle = dom_cycle
        .prepare_with_resolver(Path::new("."), &vfs_cycle, None)
        .expect_err("Self-spawning LoopNode must hit MAX_PREPARE_PASSES = 8");
    assert!(
        matches!(err_cycle, DomError::PrepareCycle { pass_limit: 8, .. }),
        "Expected PrepareCycle {{ pass_limit: 8 }}, got {:?}",
        err_cycle
    );
    assert_eq!(
        dom_cycle.query_by_tag(None, &["LoopNode"]).unwrap().len(),
        1,
        "PrepareCycle must roll back all spawned LoopNodes"
    );
}
