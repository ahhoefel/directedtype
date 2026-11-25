use std::path::Path;

use directedtype::compiler::error::CompileError;
use directedtype::compiler::expanded::NodeId;
use directedtype::compiler::module::FsResolver;
use directedtype::compiler::paginate::{paginate_compiled_document, PaginatedDocument};
use directedtype::compiler::CompiledDocument;
use directedtype::component::ComponentRegistry;
use directedtype::parse;
use directedtype::Value;

fn compile_and_paginate(src: &str) -> Result<PaginatedDocument, CompileError> {
    let doc = parse(src).expect("source should parse");
    let registry = ComponentRegistry::standard();
    let compiled = CompiledDocument::compile_with_registry_and_dpi(
        &doc,
        612.0,
        792.0,
        72.0,
        Path::new("."),
        &FsResolver,
        &registry,
    )?;
    paginate_compiled_document(&compiled, 612.0, 792.0, 72.0)
}

fn compile_expect_err(src: &str) -> CompileError {
    match compile_and_paginate(src) {
        Ok(p) => panic!(
            "Expected compile/pagination error, succeeded with {} pages",
            p.page_count
        ),
        Err(e) => e,
    }
}

#[test]
fn test_multipage_paragraph_splitting_slices_and_ranges() {
    let words = vec!["typography"; 120].join(" ");
    let src = format!(
        r#"
        \env font = \Font(family: "Arial", size: 12pt, line_height: 16pt);

        \Component SimplePage(width: Distance = 300pt, height: Distance = 120pt) {{
            \FlowBody(x: 20pt, y: 20pt, width: 260pt, height: 80pt)
        }}

        \PagedFlow(page: \SimplePage(), orphans: 2, widows: 2) {{
            \Text(id: "para1") {{ {words} }}
        }}
        "#
    );

    let paginated = compile_and_paginate(&src).expect("should paginate across 3+ pages");
    assert!(
        paginated.page_count >= 3,
        "Expected >= 3 pages, got {}",
        paginated.page_count
    );

    let para_node = paginated
        .layout
        .nodes
        .iter()
        .find(|n| n.properties.get("id").and_then(|v| v.as_str()) == Some("para1"))
        .expect("para1 present in layout");

    let mut covered_lines = Vec::new();
    for scene in &paginated.pages {
        if let Some(slice) = scene.text_line_slices.get(&para_node.id) {
            assert!(
                !slice.line_range.is_empty(),
                "Page {} slice must not be empty",
                scene.page_number
            );
            covered_lines.push(slice.line_range.clone());
        }
    }
    assert_eq!(covered_lines.len(), paginated.page_count);
    assert_eq!(covered_lines[0].start, 0);
    for w in covered_lines.windows(2) {
        assert_eq!(
            w[0].end, w[1].start,
            "Slices must partition lines strictly without gaps or overlaps: {:?}",
            covered_lines
        );
    }
}

#[test]
fn test_orphans_and_widows_enforcement() {
    // 10 lines * 14pt, 5 lines: with body height ~62pt (4 lines of ~14pt each)
    // First test orphans: fill page 1 except 1 line space -> paragraph with 4 lines must move whole to page 2 when orphans=2.
    let src_orphans = r#"
    \env font = \Font(family: "Arial", size: 12pt, line_height: 16pt);

    \Component SmallPage(width: Distance = 240pt, height: Distance = 100pt) {
        \FlowBody(x: 20pt, y: 10pt, width: 200pt, height: 80pt)
    }

    \PagedFlow(page: \SmallPage(), orphans: 2, widows: 2) {
        \Rect(width: 200pt, height: 55pt, color: #e2e8f0)
        \Text(id: "para") { line one alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi }
    }
    "#;
    let p_orphans = compile_and_paginate(src_orphans).expect("orphans should paginate");
    assert_eq!(p_orphans.page_count, 2);
    let para_id = p_orphans
        .layout
        .nodes
        .iter()
        .find(|n| n.properties.get("id").and_then(|v| v.as_str()) == Some("para"))
        .unwrap()
        .id;
    assert!(
        !p_orphans.pages[0].text_line_slices.contains_key(&para_id),
        "Orphan-violating 1-line start must push entire paragraph to page 2"
    );
    assert!(p_orphans.pages[1].text_line_slices.contains_key(&para_id));

    // Widows test: with height: 50pt at ~16pt line_height, ~3 lines fit per page -> exercises widow pullback so final slice keeps >= 2 lines.
    let src_widows = r#"
    \env font = \Font(family: "Arial", size: 12pt, line_height: 16pt);

    \Component WidowPage(width: Distance = 180pt, height: Distance = 80pt) {
        \FlowBody(x: 10pt, y: 10pt, width: 140pt, height: 50pt)
    }

    \PagedFlow(page: \WidowPage(), orphans: 2, widows: 2) {
        \Text(id: "wpara") { alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau upsilon phi chi psi omega }
    }
    "#;
    let p_widows = compile_and_paginate(src_widows).expect("widows should paginate");
    assert!(p_widows.page_count >= 2);
    let wpara_id = p_widows
        .layout
        .nodes
        .iter()
        .find(|n| n.properties.get("id").and_then(|v| v.as_str()) == Some("wpara"))
        .unwrap()
        .id;
    let total_slices: Vec<_> = p_widows
        .pages
        .iter()
        .filter_map(|pg| pg.text_line_slices.get(&wpara_id))
        .collect();
    let last_slice = total_slices.last().unwrap();
    if total_slices.len() == 2 && (last_slice.line_range.end >= 4) {
        assert!(
            last_slice.line_range.len() >= 2,
            "Widow pullback must leave >= 2 lines on final page, got {:?}",
            last_slice.line_range
        );
    }
}

#[test]
fn test_keep_with_next_chains_move_together() {
    let src = r#"
    \env font = \Font(family: "Arial", size: 12pt, line_height: 16pt);

    \Component ShortPage(width: Distance = 240pt, height: Distance = 120pt) {
        \FlowBody(x: 20pt, y: 10pt, width: 200pt, height: 100pt)
    }

    \PagedFlow(page: \ShortPage(), orphans: 2, widows: 2) {
        \Rect(id: "filler", width: 200pt, height: 70pt, color: #e2e8f0)
        \Rect(id: "h2", width: 200pt, height: 15pt, color: #1e293b, pagination: \Pagination(keep_with_next: true))
        \Rect(id: "h3", width: 200pt, height: 15pt, color: #334155, pagination: \Pagination(keep_with_next: true))
        \Text(id: "body") { This opening paragraph needs at least two orphan lines after both chained headings. }
    }
    "#;

    let paginated = compile_and_paginate(src).expect("chained keep_with_next paginates");
    assert_eq!(paginated.page_count, 2);

    let find_id = |id_val: &str| {
        paginated
            .layout
            .nodes
            .iter()
            .find(|n| n.properties.get("id").and_then(|v| v.as_str()) == Some(id_val))
            .unwrap()
            .id
    };
    let h2_id = find_id("h2");
    let h3_id = find_id("h3");
    let body_id = find_id("body");

    assert_eq!(
        paginated.layout.get_value(h2_id, "page"),
        Some(&Value::Integer(2))
    );
    assert_eq!(
        paginated.layout.get_value(h3_id, "page"),
        Some(&Value::Integer(2))
    );
    assert_eq!(
        paginated.layout.get_value(body_id, "page"),
        Some(&Value::Integer(2))
    );
}

#[test]
fn test_break_targets_recto_verso_and_page_break() {
    let src = r#"
    \Component ParityPage(width: Distance = 300pt, height: Distance = 200pt) {
        \FlowBody(x: 20pt, y: 20pt, width: 260pt, height: 160pt)
    }

    \PagedFlow(page: \ParityPage(), start_page: 1) {
        \Rect(id: "b1", width: 100pt, height: 20pt, color: #000000)
        \PageBreak(to: Break.Recto)
        \Rect(id: "b2_recto", width: 100pt, height: 20pt, color: #111111)
        \Rect(id: "b3_verso", width: 100pt, height: 20pt, color: #222222, pagination: \Pagination(break_before: Break.Verso))
        \PageBreak()
        \Rect(id: "b4_page", width: 100pt, height: 20pt, color: #333333)
    }
    "#;

    let paginated = compile_and_paginate(src).expect("parity breaks paginate");
    // b1 -> page 1 (odd).
    // PageBreak(Recto) from page 1 with content: skips page 2 (blank verso), lands on page 3!
    // b2_recto -> page 3.
    // b3_verso has break_before: Break.Verso -> advances to page 4 (even).
    // PageBreak() from page 4 with content -> advances to page 5.
    assert_eq!(paginated.page_count, 5);

    let find_page = |id_val: &str| {
        let nid = paginated
            .layout
            .nodes
            .iter()
            .find(|n| n.properties.get("id").and_then(|v| v.as_str()) == Some(id_val))
            .unwrap()
            .id;
        paginated
            .layout
            .get_value(nid, "page")
            .and_then(|v| v.as_i64())
            .unwrap()
    };

    assert_eq!(find_page("b1"), 1);
    assert_eq!(find_page("b2_recto"), 3);
    assert_eq!(find_page("b3_verso"), 4);
    assert_eq!(find_page("b4_page"), 5);
}

#[test]
fn test_page_prev_alias_rollforward_and_privacy_and_guard_errors() {
    let valid_src = r#"
    \env font = \Font(family: "Arial", size: 12pt, line_height: 16pt);

    \Component RunningPage(width: Distance = 300pt, height: Distance = 200pt, env font: Font) {
        \let is_opener = page.is_first;
        \alias trail: Integer = is_opener ? 10 : (page.prev ? page.prev.trail + 1 : 0);
        \Text(x: 20pt, y: 10pt, text: "T=" + trail + " total=" + page.count)
        \FlowBody(x: 20pt, y: 40pt, width: 260pt, height: 140pt)
    }

    \PagedFlow(page: \RunningPage()) {
        \Rect(width: 100pt, height: 20pt, color: #000000)
        \PageBreak(to: Break.Recto)
        \Rect(width: 100pt, height: 20pt, color: #111111)
    }
    "#;
    let paginated = compile_and_paginate(valid_src).expect("guarded page.prev alias should pass");
    assert_eq!(paginated.page_count, 3);
    // Page 1 trail=10, blank verso Page 2 trail=11, Page 3 trail=12
    let running_texts: Vec<String> = paginated
        .pages
        .iter()
        .map(|pg| {
            let mut found = String::new();
            if let Some(inc) = &pg.included_nodes {
                for &nid in inc {
                    if let Some(n) = pg.layout.get_node(nid) {
                        if let Some(Value::String(s)) = n.properties.get("text") {
                            if s.starts_with("T=") {
                                found = s.clone();
                            }
                        }
                    }
                }
            }
            found
        })
        .collect();
    assert_eq!(
        running_texts,
        vec![
            "T=10 total=3".to_string(),
            "T=11 total=3".to_string(),
            "T=12 total=3".to_string()
        ]
    );

    // Unguarded page.prev must error even on a 1-page document
    let unguarded_src = r#"
    \env font = \Font(size: 12pt);
    \Component BadGuardPage(width: Distance = 300pt, height: Distance = 200pt) {
        \alias trail: Integer = page.prev.trail + 1;
        \FlowBody(x: 20pt, y: 20pt, width: 260pt, height: 160pt)
    }
    \PagedFlow(page: \BadGuardPage()) {
        \Rect(width: 100pt, height: 20pt, color: #000000)
    }
    "#;
    let err = compile_expect_err(unguarded_src);
    assert!(
        matches!(err, CompileError::UnguardedPagePrevAccess { .. }),
        "Expected UnguardedPagePrevAccess, got {err:?}"
    );

    // Private \let cannot be read across page.prev
    let private_let_src = r#"
    \env font = \Font(size: 12pt);
    \Component PrivLetPage(width: Distance = 300pt, height: Distance = 200pt) {
        \let secret_val = page.number * 10;
        \let prev_secret = page.prev ? page.prev.secret_val : 0;
        \FlowBody(x: 20pt, y: 20pt, width: 260pt, height: 160pt)
    }
    \PagedFlow(page: \PrivLetPage()) {
        \Rect(width: 100pt, height: 20pt, color: #000000)
    }
    "#;
    let err_priv = compile_expect_err(private_let_src);
    assert!(
        format!("{err_priv}").contains("secret_val") || format!("{err_priv}").contains("private"),
        "Expected private port rejection on page.prev.secret_val, got {err_priv}"
    );
}

#[test]
fn test_shallow_pagination_isolation_and_breakable_chrome_guard() {
    // Container with break_before: Recto + breakable: true only breaks the container once, not every child!
    let shallow_src = r#"
    \Component BreakSection(
        x: Distance = parent.left,
        y: Distance = parent.top,
        width: Distance = parent.width,
        pagination: Pagination = \Pagination(breakable: true, break_before: Break.Recto)
    ) {
        \alias height = 40pt;
        \alias right = x + width;
        \alias bottom = y + height;
        \Children(x: x, y: prev ? prev.bottom : y)
    }

    \Component SP(width: Distance = 300pt, height: Distance = 200pt) {
        \FlowBody(x: 20pt, y: 20pt, width: 260pt, height: 160pt)
    }

    \PagedFlow(page: \SP()) {
        \Rect(id: "intro", width: 100pt, height: 20pt, color: #000000)
        \BreakSection() {
            \Rect(id: "c1", width: 100pt, height: 20pt, color: #111111)
            \Rect(id: "c2", width: 100pt, height: 20pt, color: #222222)
        }
    }
    "#;
    let p = compile_and_paginate(shallow_src).expect("shallow pagination should succeed");
    let find_p = |id_val: &str| {
        let nid = p
            .layout
            .nodes
            .iter()
            .find(|n| n.properties.get("id").and_then(|v| v.as_str()) == Some(id_val))
            .unwrap()
            .id;
        p.layout.get_value(nid, "page").and_then(|v| v.as_i64()).unwrap()
    };
    assert_eq!(find_p("intro"), 1);
    assert_eq!(find_p("c1"), 3);
    assert_eq!(
        find_p("c2"),
        3,
        "Second child inside breakable Break.Recto container must NOT inherit break_before: Recto"
    );

    // UnsupportedBreakableChrome when breakable container draws full-height background Rect
    let chrome_src = r#"
    \Component BadChromeCard(
        x: Distance = parent.left,
        y: Distance = parent.top,
        width: Distance = parent.width,
        height: Distance = 100pt
    ) {
        \let pagination = \Pagination(breakable: true);
        \Rect(x: x, y: y, width: width, height: height, color: #f1f5f9)
        \Children
    }
    \BadChromeCard(width: 200pt) {
        \Rect(x: 0pt, y: 0pt, width: 50pt, height: 20pt, color: #000000)
    }
    "#;
    let err = compile_expect_err(chrome_src);
    assert!(
        matches!(err, CompileError::UnsupportedBreakableChrome { .. }),
        "Expected UnsupportedBreakableChrome, got {err:?}"
    );
}

#[test]
fn test_static_flow_body_invariants_and_page_break_guard() {
    // 1. FlowBody.width depends on page.is_recto
    let bad_w = r#"
    \Component BadW(width: Distance = 300pt, height: Distance = 200pt) {
        \FlowBody(x: 20pt, y: 20pt, width: page.is_recto ? 200pt : 220pt, height: 150pt)
    }
    \PagedFlow(page: \BadW()) {
        \Rect(width: 100pt, height: 20pt, color: #000000)
    }
    "#;
    assert!(matches!(
        compile_expect_err(bad_w),
        CompileError::FlowBodyWidthDependsOnPage { .. }
    ));

    // 2. FlowBody.height depends on page.count
    let bad_h = r#"
    \Component BadH(width: Distance = 300pt, height: Distance = 200pt) {
        \FlowBody(x: 20pt, y: 20pt, width: 200pt, height: page.count > 1 ? 140pt : 160pt)
    }
    \PagedFlow(page: \BadH()) {
        \Rect(width: 100pt, height: 20pt, color: #000000)
    }
    "#;
    assert!(matches!(
        compile_expect_err(bad_h),
        CompileError::FlowBodyHeightDependsOnTotalPageCount { .. }
    ));

    // 3. InvalidFlowBodyCount: 0 and 2 FlowBody nodes
    let zero_fb = r#"
    \Component ZeroFB(width: Distance = 300pt, height: Distance = 200pt) {
        \Rect(x: 0pt, y: 0pt, width: width, height: height, color: #ffffff)
    }
    \PagedFlow(page: \ZeroFB()) {
        \Rect(width: 100pt, height: 20pt, color: #000000)
    }
    "#;
    assert!(matches!(
        compile_expect_err(zero_fb),
        CompileError::InvalidFlowBodyCount { found: 0, .. }
    ));

    let two_fb = r#"
    \Component TwoFB(width: Distance = 300pt, height: Distance = 200pt) {
        \FlowBody(x: 10pt, y: 10pt, width: 100pt, height: 100pt)
        \FlowBody(x: 120pt, y: 10pt, width: 100pt, height: 100pt)
    }
    \PagedFlow(page: \TwoFB()) {
        \Rect(width: 50pt, height: 20pt, color: #000000)
    }
    "#;
    assert!(matches!(
        compile_expect_err(two_fb),
        CompileError::InvalidFlowBodyCount { found: 2, .. }
    ));

    // 4. PageBreak inside unbreakable component
    let pb_in_unbreakable = r#"
    \Component SolidBox(x: Distance = parent.left, y: Distance = parent.top, width: Distance = parent.width) {
        \alias height = 40pt;
        \Children
    }
    \Component SimpleP(width: Distance = 300pt, height: Distance = 200pt) {
        \FlowBody(x: 20pt, y: 20pt, width: 260pt, height: 160pt)
    }
    \PagedFlow(page: \SimpleP()) {
        \SolidBox() {
            \PageBreak()
        }
    }
    "#;
    assert!(matches!(
        compile_expect_err(pb_in_unbreakable),
        CompileError::PageBreakInUnbreakableContainer { .. }
    ));
}

#[test]
fn test_forward_page_convergence_and_overflow_error() {
    // Forward .page reference in Pass 1 defaults to 1, target lands on Page 3 after PageBreak(Recto), settles in Pass 2!
    let conv_src = r#"
    \env font = \Font(family: "Arial", size: 12pt, line_height: 16pt);

    \Component ConvPage(width: Distance = 300pt, height: Distance = 200pt) {
        \FlowBody(x: 20pt, y: 20pt, width: 260pt, height: 160pt)
    }

    \PagedFlow(page: \ConvPage()) {
        \Text(id: "xref") { See target on page \Text(text: "" + target_box.page). }
        \PageBreak(to: Break.Recto)
        \let target_box = \Rect(width: 100pt, height: 30pt, color: #0f172a);
    }
    "#;
    let paginated = compile_and_paginate(conv_src).expect("forward .page converges");
    assert_eq!(paginated.page_count, 3);
    let xref_node = paginated
        .layout
        .nodes
        .iter()
        .find(|n| n.properties.get("id").and_then(|v| v.as_str()) == Some("xref"))
        .unwrap();
    let xref_text = xref_node
        .properties
        .get("text")
        .and_then(|v| v.as_str())
        .or(xref_node.text_content.as_deref())
        .unwrap();
    assert_eq!(xref_text, "See target on page 3.");

    // Oversized atomic block exceeding empty FlowBody.height must error with PaginationOverflowError
    let overflow_src = r#"
    \Component TinyPage(width: Distance = 300pt, height: Distance = 100pt) {
        \FlowBody(x: 20pt, y: 20pt, width: 260pt, height: 50pt)
    }
    \PagedFlow(page: \TinyPage()) {
        \Rect(width: 100pt, height: 80pt, color: #ef4444)
    }
    "#;
    assert!(matches!(
        compile_expect_err(overflow_src),
        CompileError::PaginationOverflowError { .. }
    ));

    // Confirm window layout value resolves cleanly
    assert!(paginated.layout.get_value(NodeId::WINDOW, "width").is_some());
}

#[cfg(feature = "pdf")]
#[test]
fn test_multipage_pdf_export_and_sliced_links() {
    use directedtype::pdf::{compile_source_to_pdf, PdfOptions};

    let words = vec!["typographic"; 85].join(" ");
    let src = format!(
        r##"
        \use "components/text/Link.dt"
        \use "theme/default.dt"

        \env font = \Font(family: "Arial", size: 12pt, line_height: 16pt);
        \env link_style = link_default;

        \Component PdfPage(width: Distance = 360pt, height: Distance = 160pt, env font: Font) {{
            \Text(x: 20pt, y: 8pt, text: "Page " + page.number + " / " + page.count)
            \FlowBody(x: 20pt, y: 30pt, width: 320pt, height: 100pt)
        }}

        \PagedFlow(page: \PdfPage(), orphans: 1, widows: 1) {{
            \Text {{ Visit \Link(url: "https://directedtype.dev"){{DirectedType Site}} and read: {words} \Link(url: "#end"){{Jump End}} }}
            \PageBreak()
            \Anchor(name: "end") {{
                \Text {{ End Section }}
            }}
        }}
        "##
    );

    let opts = PdfOptions {
        compressed: false,
        deterministic: true,
        fallback_width: Some(360.0),
        fallback_height: Some(160.0),
        ..Default::default()
    };
    let bytes = compile_source_to_pdf(&src, Path::new("."), &opts).expect("multi-page PDF export");
    let pdf_str = String::from_utf8_lossy(&bytes);
    assert!(pdf_str.starts_with("%PDF-"));
    let page_count = pdf_str
        .match_indices("/Type/Page")
        .chain(pdf_str.match_indices("/Type /Page"))
        .filter(|&(idx, pat)| {
            let after = idx + pat.len();
            !pdf_str.as_bytes().get(after).is_some_and(|&b| b == b's')
        })
        .count();
    assert!(
        page_count >= 3,
        "Expected >= 3 /Type /Page objects in PDF, found {page_count}"
    );
    assert!(
        pdf_str.contains("https://directedtype.dev"),
        "Expected external URI link annotation in multi-page PDF"
    );
    assert!(
        pdf_str.contains("/Subtype/Link") || pdf_str.contains("/Subtype /Link"),
        "Expected PDF /Link annotations across split paragraphs"
    );
}

#[test]
fn test_page_break_inside_vstack_and_forwarded_wrapper() {
    let src = r#"
    \use "components/layout/VStackStretch.dt"

    \Component Wrapper(
        x: Distance = parent.left,
        y: Distance = parent.top,
        width: Distance = parent.width,
        pagination: Pagination = \Pagination(breakable: true)
    ) {
        \let body = \VStackStretch(x: x, y: y, width: width, gap: 4pt, pagination: \Pagination(breakable: true)) {
            \Children
        };
        \alias height: Distance = body.height;
    }

    \Component SimplePage(width: Distance = 300pt, height: Distance = 200pt) {
        \FlowBody(x: 20pt, y: 20pt, width: 260pt, height: 160pt)
    }

    \PagedFlow(page: \SimplePage(), start_page: 1) {
        \VStackStretch(width: 260pt, gap: 4pt) {
            \Rect(id: "direct_a", width: 100pt, height: 20pt, color: #000000)
            \PageBreak()
            \Rect(id: "direct_b", width: 100pt, height: 20pt, color: #111111)
        }
        \Wrapper(width: 260pt) {
            \Rect(id: "wrapped_a", width: 100pt, height: 20pt, color: #222222)
            \PageBreak()
            \Rect(id: "wrapped_b", width: 100pt, height: 20pt, color: #333333)
        }
    }
    "#;

    let paginated = compile_and_paginate(src).expect("PageBreak inside VStackStretch should compile and paginate");
    let find_page = |id_val: &str| {
        let nid = paginated
            .layout
            .nodes
            .iter()
            .find(|n| n.properties.get("id").and_then(|v| v.as_str()) == Some(id_val))
            .unwrap()
            .id;
        paginated
            .layout
            .get_value(nid, "page")
            .and_then(|v| v.as_i64())
            .unwrap()
    };

    assert_eq!(find_page("direct_a"), 1);
    assert_eq!(find_page("direct_b"), 2);
    assert_eq!(find_page("wrapped_a"), 2);
    assert_eq!(find_page("wrapped_b"), 3);
    assert_eq!(paginated.page_count, 3);
}
