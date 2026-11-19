use std::path::Path;

use directedtype::compiler::error::CompileError;
use directedtype::compiler::module::FsResolver;
use directedtype::compiler::paginate::{paginate_compiled_document, PageScene, PaginatedDocument};
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

fn page_texts_with_id<'a>(
    scene: &'a PageScene,
    id: &str,
) -> Vec<(String, f64, f64, f64, f64)> {
    let mut out = Vec::new();
    let Some(included) = &scene.included_nodes else {
        return out;
    };
    for node in &scene.layout.nodes {
        if !included.contains(&node.id) {
            continue;
        }
        if node.properties.get("id").and_then(|v| v.as_str()) == Some(id) {
            let text = node
                .text_content
                .clone()
                .or_else(|| {
                    node.properties
                        .get("text")
                        .and_then(|v| v.as_str().map(|s| s.to_string()))
                })
                .unwrap_or_default();
            let off = scene.node_offsets.get(&node.id).copied().unwrap_or((0.0, 0.0));
            out.push((
                text,
                node.rect.x + off.0,
                node.rect.y + off.1,
                node.rect.width,
                node.rect.height,
            ));
        }
    }
    out
}

fn page_nodes_by_name<'a>(
    scene: &'a PageScene,
    name: &str,
) -> Vec<(&'a directedtype::compiler::layout::ResolvedNode, (f64, f64))> {
    let Some(included) = &scene.included_nodes else {
        return Vec::new();
    };
    scene
        .layout
        .nodes
        .iter()
        .filter(|n| included.contains(&n.id) && n.name == name)
        .map(|n| (n, scene.node_offsets.get(&n.id).copied().unwrap_or((0.0, 0.0))))
        .collect()
}

#[test]
fn test_page_value_overwrite_and_prev_rollforward_and_recto_blank() {
    let src = r#"
    \env font = \Font(family: "Arial", size: 12pt, line_height: 16pt);
    \let meta = \PageValue();

    \Component Meta(title: String) {
        \let private_note = "secret";
    }

    \Component PVPage(width: Distance = 400pt, height: Distance = 200pt, env font: Font) {
        \let has_m = meta[page].has_value;
        \alias cur_title: String = has_m
            ? meta[page].value.title
            : (page.prev ? page.prev.cur_title : "NONE");
        \let first_title = has_m ? meta[page].first.title : "NONE";

        \Text(id: "cur_lbl", x: 10pt, y: 10pt, font: font, text: cur_title)
        \Text(id: "first_lbl", x: 10pt, y: 28pt, font: font, text: first_title)
        \FlowBody(x: 20pt, y: 60pt, width: 360pt, height: 120pt)
    }

    \PagedFlow(page: \PVPage()) {
        \Deposit(into: meta) { \Meta(title: "FirstOnP1") }
        \Deposit(into: meta) { \Meta(title: "OverwriteP1") }
        \Text(text: "Body on page 1")

        \PageBreak(to: Break.Page)
        \Text(text: "Continuation on page 2 with no deposit")

        \PageBreak(to: Break.Recto)
        \Deposit(into: meta) { \Meta(title: "ChapterTwoOnP3") }
        \Text(text: "Starts on recto page 3")
    }
    "#;

    let paginated = compile_and_paginate(src).expect("PageValue test should compile & paginate");
    assert_eq!(paginated.page_count, 3, "P1, P2(verso), P3(recto)");

    // Page 1: overwrite semantics -> value = OverwriteP1, first = FirstOnP1
    let p1_cur = page_texts_with_id(&paginated.pages[0], "cur_lbl");
    let p1_first = page_texts_with_id(&paginated.pages[0], "first_lbl");
    assert_eq!(p1_cur[0].0, "OverwriteP1");
    assert_eq!(p1_first[0].0, "FirstOnP1");

    // Page 2 (verso, no deposit): rolls forward OverwriteP1, first is NONE
    let p2_cur = page_texts_with_id(&paginated.pages[1], "cur_lbl");
    let p2_first = page_texts_with_id(&paginated.pages[1], "first_lbl");
    assert_eq!(p2_cur[0].0, "OverwriteP1");
    assert_eq!(p2_first[0].0, "NONE");

    // Page 3 (recto): ChapterTwoOnP3
    let p3_cur = page_texts_with_id(&paginated.pages[2], "cur_lbl");
    assert_eq!(p3_cur[0].0, "ChapterTwoOnP3");

    // Verify Break.Recto from Page 1 (recto -> blank Page 2 verso -> Chapter 2 Page 3 recto)
    // leaves Page 2 completely free of Page 3's deposit!
    let src_recto_skip = r#"
    \env font = \Font(family: "Arial", size: 12pt, line_height: 16pt);
    \let meta = \PageValue();

    \Component Meta(title: String) {}

    \Component PVPage2(width: Distance = 400pt, height: Distance = 200pt, env font: Font) {
        \let has_m = meta[page].has_value;
        \alias running: String = has_m
            ? meta[page].value.title
            : (page.prev ? page.prev.running : "NONE");
        \Text(id: "hdr", x: 10pt, y: 10pt, font: font, text: has_m ? ("OPEN:" + running) : ("CONT:" + running))
        \FlowBody(x: 20pt, y: 50pt, width: 360pt, height: 130pt)
    }

    \PagedFlow(page: \PVPage2()) {
        \Deposit(into: meta) { \Meta(title: "Ch1") }
        \Text(text: "Ch1 only on page 1")

        \Text(
            pagination: \Pagination(break_before: Break.Recto),
            text: "Wait - direct deposit before break vs component"
        )
    }
    "#;
    let p_skip_base = compile_and_paginate(src_recto_skip).expect("base recto skip");
    assert_eq!(p_skip_base.page_count, 3);
    assert_eq!(page_texts_with_id(&p_skip_base.pages[1], "hdr")[0].0, "CONT:Ch1");
}

#[test]
fn test_page_list_collect_footnotes_shrinks_flow_body() {
    let src = r#"
    \use "components/VStackStretch.dt"
    \env font = \Font(family: "Arial", size: 11pt, line_height: 15pt);
    \let footnotes = \PageList();

    \Component FnEntry(
        marker: String,
        env font: Font,
        x: Distance = parent.left,
        y: Distance = parent.top,
        width: Distance = parent.width
    ) {
        \Text(x: x, y: y, width: width, font: font, text: marker + " Footnote line")
        \alias right = x + width;
        \alias height = 24pt;
        \alias bottom = y + height;
    }

    \InlineBox FnCall(
        marker: String,
        env font: Font,
        x: Distance = parent.left,
        y: Distance = parent.top
    ) {
        \let m = \Text(x: x, y: y, font: font, text: marker);
        \alias width = m.width;
        \alias height = m.height;
        \Deposit(into: footnotes) {
            \FnEntry(marker: marker)
        }
    }

    \Component FnPage(width: Distance = 400pt, height: Distance = 300pt, env font: Font) {
        \let has_fn = footnotes[page].len > 0;
        \let fn_stack = \VStackStretch(
            x: 20pt,
            y: 280pt - fn_stack.height,
            width: 360pt,
            gap: 4pt
        ) {
            \Collect(from: footnotes[page])
        };
        \let reserved = has_fn ? (fn_stack.height + 10pt) : 0pt;
        \FlowBody(x: 20pt, y: 20pt, width: 360pt, height: 260pt - reserved)
    }

    \PagedFlow(page: \FnPage()) {
        \Text(text: "Page 1 without any footnotes")
        \PageBreak(to: Break.Page)
        \Text {
            First callout\FnCall(marker: "1") and second callout\FnCall(marker: "2") on page 2.
        }
        \PageBreak(to: Break.Page)
        \Text(text: "Page 3 clean again")
    }
    "#;

    let paginated = compile_and_paginate(src).expect("PageList footnote collection should succeed");
    assert_eq!(paginated.page_count, 3);

    let fb_h = |page_idx: usize| -> f64 {
        let fb = page_nodes_by_name(&paginated.pages[page_idx], "FlowBody");
        assert_eq!(fb.len(), 1);
        fb[0].0.rect.height
    };
    assert!((fb_h(0) - 260.0).abs() < 1e-4, "P1 FlowBody.height = 260");
    // Page 2 has 2 FnEntries of 24pt each + 4pt gap + 10pt reserve = 62pt
    assert!(
        (fb_h(1) - (260.0 - 62.0)).abs() < 1e-4,
        "P2 FlowBody.height = 198, got {}",
        fb_h(1)
    );
    assert!((fb_h(2) - 260.0).abs() < 1e-4, "P3 FlowBody.height = 260");

    assert_eq!(page_nodes_by_name(&paginated.pages[0], "FnEntry").len(), 0);
    assert_eq!(page_nodes_by_name(&paginated.pages[1], "FnEntry").len(), 2);
    assert_eq!(page_nodes_by_name(&paginated.pages[2], "FnEntry").len(), 0);
}

#[test]
fn test_footnote_rollback_on_overflow_and_widows_pullback() {
    // Case 1: Overflow rollback — P1 has 44pt FlowBody height and 16pt first block leaves 28pt.
    // A footnote on the next paragraph needs 50pt footnote reserve -> FlowBody contracts to -6pt,
    // so both the paragraph AND the footnote must roll back off Page 1 and land on Page 2!
    let src_overflow = r#"
    \use "components/VStackStretch.dt"
    \env font = \Font(family: "Arial", size: 11pt, line_height: 15pt);
    \let footnotes = \PageList();

    \Component BigFn(
        msg: String,
        env font: Font,
        x: Distance = parent.left,
        y: Distance = parent.top,
        width: Distance = parent.width
    ) {
        \Text(x: x, y: y, width: width, font: font, text: msg)
        \alias right = x + width;
        \alias height = 40pt;
        \alias bottom = y + height;
    }

    \InlineBox CallFn(
        msg: String,
        env font: Font,
        x: Distance = parent.left,
        y: Distance = parent.top
    ) {
        \let m = \Text(x: x, y: y, font: font, text: "*");
        \alias width = m.width;
        \alias height = m.height;
        \Deposit(into: footnotes) { \BigFn(msg: msg) }
    }

    \Component RollPage(width: Distance = 400pt, height: Distance = 200pt, env font: Font) {
        \let has_fn = footnotes[page].len > 0;
        \let fn_stack = \VStackStretch(
            x: 20pt,
            y: 120pt - fn_stack.height,
            width: 360pt
        ) {
            \Collect(from: footnotes[page])
        };
        \let reserved = has_fn ? (fn_stack.height + 10pt) : 0pt;
        \FlowBody(x: 20pt, y: 20pt, width: 360pt, height: 100pt - reserved)
    }

    \PagedFlow(page: \RollPage()) {
        \Rect(id: "top_box", height: 70pt, color: #cbd5e1)
        \Text(id: "fn_para") { Hello\CallFn(msg: "Should move to page 2") world }
    }
    "#;

    let paginated = compile_and_paginate(src_overflow).expect("overflow rollback succeeds across 2 pages");
    assert_eq!(paginated.page_count, 2);
    assert_eq!(
        page_nodes_by_name(&paginated.pages[0], "BigFn").len(),
        0,
        "Footnote must roll back off Page 1 when it forces overflow"
    );
    assert_eq!(
        page_nodes_by_name(&paginated.pages[1], "BigFn").len(),
        1,
        "Footnote must commit onto Page 2 alongside its host line"
    );

    // Case 2: Widows pullback rollback — 5 lines total, each ~16pt. FlowBody = 70pt (fits 4 lines).
    // With widows = 2, line 3 (0-indexed: 0,1,2,3 initially fit -> remaining 1 < 2 -> pulls back to 3 lines)
    // MUST pull line 3 and its footnote off Page 1 onto Page 2!
    let src_widow = r#"
    \use "components/VStackStretch.dt"
    \env font = \Font(family: "Arial", size: 12pt, line_height: 16pt);
    \let footnotes = \PageList();

    \Component WFn(
        env font: Font,
        x: Distance = parent.left,
        y: Distance = parent.top,
        width: Distance = parent.width
    ) {
        \Text(x: x, y: y, width: width, font: font, text: "Widow fn")
        \alias right = x + width;
        \alias height = 10pt;
        \alias bottom = y + height;
    }

    \InlineBox CallWFn(env font: Font, x: Distance = parent.left, y: Distance = parent.top) {
        \let m = \Text(x: x, y: y, font: font, text: "*");
        \alias width = m.width;
        \alias height = m.height;
        \Deposit(into: footnotes) { \WFn() }
    }

    \Component WidowFnPage(width: Distance = 300pt, height: Distance = 200pt, env font: Font) {
        \let has_fn = footnotes[page].len > 0;
        \let fn_stack = \VStackStretch(
            x: 20pt,
            y: 96pt - fn_stack.height,
            width: 260pt
        ) {
            \Collect(from: footnotes[page])
        };
        \let reserved = has_fn ? (fn_stack.height + 6pt) : 0pt;
        // 5 lines * 16pt = 80pt. Height 76pt fits 4 lines without footnote; even with 16pt fn, 60pt fits 3 lines!
        // Specifically put the callout right at the pulled-back 4th line.
        \FlowBody(x: 20pt, y: 20pt, width: 260pt, height: 84pt - reserved)
    }

    \PagedFlow(page: \WidowFnPage(), orphans: 2, widows: 2) {
        \Text(id: "wpara") {
            wordone wordone wordone wordone wordone wordone wordone wordone wordone
            wordtwo wordtwo wordtwo wordtwo wordtwo wordtwo wordtwo
            wordthree wordthree wordthree\CallWFn()
            wordfour wordfour wordfour wordfour wordfour
        }
    }
    "#;
    let pw = compile_and_paginate(src_widow).expect("widow pullback footnote should paginate");
    assert_eq!(pw.page_count, 2, "paragraph splits cleanly into 2 pages");
    let p1_slice = pw.pages[0]
        .text_line_slices
        .values()
        .next()
        .expect("page 1 slice");
    let p2_slice = pw.pages[1]
        .text_line_slices
        .values()
        .next()
        .expect("page 2 slice");
    assert!(
        p2_slice.line_range.len() >= 2,
        "widows = 2 preserved on page 2: {:?}",
        p2_slice.line_range
    );
    // Find which line holds the InlineBox (footnote)
    let para_atom_fn_page = if page_nodes_by_name(&pw.pages[0], "WFn").is_empty() {
        1
    } else {
        0
    };
    let box_page = if page_nodes_by_name(&pw.pages[0], "CallWFn").is_empty() {
        1
    } else {
        0
    };
    assert_eq!(
        para_atom_fn_page, box_page,
        "Footnote WFn must appear on the EXACT same page as its inline CallWFn callout (P1 lines {:?}, P2 lines {:?})",
        p1_slice.line_range, p2_slice.line_range
    );
}

#[test]
fn test_oversized_footnote_overflow_error() {
    let src = r#"
    \use "components/VStackStretch.dt"
    \env font = \Font(family: "Arial", size: 12pt, line_height: 16pt);
    \let footnotes = \PageList();

    \Component HugeFn(
        env font: Font,
        x: Distance = parent.left,
        y: Distance = parent.top,
        width: Distance = parent.width
    ) {
        \Text(x: x, y: y, width: width, font: font, text: "Huge")
        \alias right = x + width;
        \alias height = 250pt;
        \alias bottom = y + height;
    }

    \InlineBox CallHuge(env font: Font, x: Distance = parent.left, y: Distance = parent.top) {
        \let m = \Text(x: x, y: y, font: font, text: "*");
        \alias width = m.width;
        \alias height = m.height;
        \Deposit(into: footnotes) { \HugeFn() }
    }

    \Component TinyPage(width: Distance = 300pt, height: Distance = 200pt, env font: Font) {
        \let has_fn = footnotes[page].len > 0;
        \let fn_stack = \VStackStretch(
            x: 20pt,
            y: 180pt - fn_stack.height,
            width: 260pt
        ) {
            \Collect(from: footnotes[page])
        };
        \let reserved = has_fn ? fn_stack.height : 0pt;
        \FlowBody(x: 20pt, y: 20pt, width: 260pt, height: 160pt - reserved)
    }

    \PagedFlow(page: \TinyPage()) {
        \Text { Text with\CallHuge() huge footnote }
    }
    "#;

    let err = compile_expect_err(src);
    assert!(
        matches!(err, CompileError::PaginationOverflowError { .. }),
        "Expected PaginationOverflowError, got {:?}",
        err
    );
}

#[test]
fn test_sidenote_column_baseline_and_collision_avoidance() {
    let src = r#"
    \use "components/Book.dt"

    \Book(
        title: "Sidenote Test",
        two_sided: false,
        page_width: 8.5in,
        page_height: 11in,
        margin_inner: 0.75in,
        margin_outer: 2.5in,
        sidenote_width: 1.75in,
        sidenote_gutter: 0.25in,
        sidenote_gap: 12pt
    ) {
        \Text(width: parent.width) {
            First sentence callout.\Sidenote(marker: "a") { First sidenote commentary here. }
            Second callout right next to it.\Sidenote(marker: "b") { Second sidenote pushed below the first. }
        }
        \Rect(height: 180pt, color: #f8fafc)
        \Text(width: parent.width) {
            Third callout much further down the page.\Sidenote(marker: "c") { Far lower sidenote aligns to its own line. }
        }
    }
    "#;

    let paginated = compile_and_paginate(src).expect("Sidenotes should paginate cleanly");
    assert_eq!(paginated.page_count, 1);

    let mut notes = page_nodes_by_name(&paginated.pages[0], "SidenoteEntry");
    notes.sort_by_key(|(n, _)| n.id.0);
    assert_eq!(notes.len(), 3, "Three SidenoteEntries collected on page 1");

    let n1_y = notes[0].0.rect.y + notes[0].1 .1;
    let n1_h = notes[0].0.rect.height;
    let n2_y = notes[1].0.rect.y + notes[1].1 .1;
    let n2_h = notes[1].0.rect.height;
    let n3_y = notes[2].0.rect.y + notes[2].1 .1;

    assert!(
        n2_y >= n1_y + n1_h + 12.0 - 1e-3,
        "Second sidenote ({:.2}) must not collide with first ({:.2} + {:.2} + 12.0 = {:.2})",
        n2_y,
        n1_y,
        n1_h,
        n1_y + n1_h + 12.0
    );
    assert!(
        n3_y > n2_y + n2_h + 100.0,
        "Third sidenote ({:.2}) should align far below note 2 ({:.2} + {:.2}) near its prose baseline",
        n3_y,
        n2_y,
        n2_h
    );
}

#[test]
fn test_bucket_and_collect_static_safety_diagnostics() {
    // 1. Unguarded bucket `.value` read
    let unguarded = r#"
    \env font = \Font(family: "Arial", size: 12pt, line_height: 16pt);
    \let b = \PageValue();
    \Component Item(title: String) {}
    \Component BadPage(width: Distance = 300pt, height: Distance = 200pt, env font: Font) {
        \Text(x: 10pt, y: 10pt, font: font, text: b[page].value.title)
        \FlowBody(x: 10pt, y: 40pt, width: 280pt, height: 150pt)
    }
    \PagedFlow(page: \BadPage()) {
        \Deposit(into: b) { \Item(title: "x") }
        \Text(text: "hi")
    }
    "#;
    let err_unguarded = compile_expect_err(unguarded);
    assert!(
        matches!(err_unguarded, CompileError::UnguardedBucketAccess { .. }),
        "Expected UnguardedBucketAccess, got {:?}",
        err_unguarded
    );

    // 2. Deposit with 0 children or 2 children
    let dep_zero = r#"
    \env font = \Font(family: "Arial", size: 12pt, line_height: 16pt);
    \let b = \PageList();
    \Component P(width: Distance = 300pt, height: Distance = 200pt) {
        \FlowBody(x: 10pt, y: 10pt, width: 280pt, height: 180pt)
    }
    \PagedFlow(page: \P()) {
        \Deposit(into: b) {}
        \Text(text: "hi")
    }
    "#;
    assert!(matches!(
        compile_expect_err(dep_zero),
        CompileError::InvalidDepositChildCount { found: 0, .. }
    ));

    let dep_two = r#"
    \env font = \Font(family: "Arial", size: 12pt, line_height: 16pt);
    \let b = \PageList();
    \Component P(width: Distance = 300pt, height: Distance = 200pt) {
        \FlowBody(x: 10pt, y: 10pt, width: 280pt, height: 180pt)
    }
    \PagedFlow(page: \P()) {
        \Deposit(into: b) {
            \Rect(height: 10pt, color: #000000)
            \Rect(height: 10pt, color: #000000)
        }
        \Text(text: "hi")
    }
    "#;
    assert!(matches!(
        compile_expect_err(dep_two),
        CompileError::InvalidDepositChildCount { found: 2, .. }
    ));

    // 3. InvalidCollectTarget: PageValue bucket in \Collect
    let collect_pv = r#"
    \use "components/VStackStretch.dt"
    \env font = \Font(family: "Arial", size: 12pt, line_height: 16pt);
    \let b = \PageValue();
    \Component P(width: Distance = 300pt, height: Distance = 200pt) {
        \VStackStretch(x: 10pt, y: 10pt, width: 280pt) { \Collect(from: b[page]) }
        \FlowBody(x: 10pt, y: 50pt, width: 280pt, height: 140pt)
    }
    \PagedFlow(page: \P()) { \Text(text: "hi") }
    "#;
    assert!(matches!(
        compile_expect_err(collect_pv),
        CompileError::InvalidCollectTarget { .. }
    ));

    // 4. InvalidCollectTarget: page.prev in \Collect
    let collect_prev = r#"
    \use "components/VStackStretch.dt"
    \env font = \Font(family: "Arial", size: 12pt, line_height: 16pt);
    \let b = \PageList();
    \Component P(width: Distance = 300pt, height: Distance = 200pt) {
        \VStackStretch(x: 10pt, y: 10pt, width: 280pt) { \Collect(from: b[page.prev]) }
        \FlowBody(x: 10pt, y: 50pt, width: 280pt, height: 140pt)
    }
    \PagedFlow(page: \P()) { \Text(text: "hi") }
    "#;
    assert!(matches!(
        compile_expect_err(collect_prev),
        CompileError::InvalidCollectTarget { .. }
    ));

    // 5. Private \let access on bucket[page].value.<let_var>
    let private_let = r#"
    \env font = \Font(family: "Arial", size: 12pt, line_height: 16pt);
    \let b = \PageValue();
    \Component Item(title: String) {
        \let secret_val = "hidden";
    }
    \Component BadPage(width: Distance = 300pt, height: Distance = 200pt, env font: Font) {
        \let s = b[page].has_value ? b[page].value.secret_val : "";
        \Text(x: 10pt, y: 10pt, font: font, text: s)
        \FlowBody(x: 10pt, y: 40pt, width: 280pt, height: 150pt)
    }
    \PagedFlow(page: \BadPage()) {
        \Deposit(into: b) { \Item(title: "x") }
        \Text(text: "hi")
    }
    "#;
    assert!(compile_and_paginate(private_let).is_err());
}

#[test]
fn test_book_and_article_stdlib_end_to_end() {
    let book_src = std::fs::read_to_string("examples/book_demo.dt").expect("read examples/book_demo.dt");
    let book_paginated = compile_and_paginate(&book_src).expect("examples/book_demo.dt compiles cleanly");
    assert_eq!(
        book_paginated.page_count, 3,
        "book_demo should have 3 pages with zero leading blank pages"
    );

    // Chapter 1 and its leading \Bookmark must both land on Page 1 (no blank leading pages)
    let p1 = &book_paginated.pages[0];
    assert!(
        page_nodes_by_name(p1, "Text").iter().any(|(n, _)| {
            n.text_content.as_deref() == Some("Single-Pass Pagination")
                || n.properties.get("text").and_then(Value::as_str)
                    == Some("Single-Pass Pagination")
        }),
        "Chapter 1 heading must start on Page 1"
    );
    assert!(
        page_nodes_by_name(p1, "Bookmark").iter().any(|(n, _)| {
            n.properties.get("title").and_then(Value::as_str)
                == Some("Chapter 1: Single-Pass Pagination")
        }),
        "Chapter 1 Bookmark must be placed on Page 1"
    );

    // Verify Chapter 2 and its leading \Bookmark open together on Page 3 (recto) and suppress header
    let ch2_pages: Vec<i64> = book_paginated
        .pages
        .iter()
        .filter(|s| {
            page_nodes_by_name(s, "Text").iter().any(|(n, _)| {
                n.text_content.as_deref() == Some("Composable Page Buckets")
                    || n.properties.get("text").and_then(Value::as_str)
                        == Some("Composable Page Buckets")
            })
        })
        .map(|s| s.page_number)
        .collect();
    assert_eq!(ch2_pages, vec![3], "Chapter 2 must open on Page 3 (recto)");
    let p3 = &book_paginated.pages[2];
    assert!(
        page_nodes_by_name(p3, "Bookmark").iter().any(|(n, _)| {
            n.properties.get("title").and_then(Value::as_str)
                == Some("Chapter 2: Composable Page Buckets")
        }),
        "Chapter 2 Bookmark must travel with Break.Recto onto Page 3"
    );

    // On every page, the full-page `page_bg` \Rect must precede all placed \FlowBody \Text nodes in Painter's Order
    for scene in &book_paginated.pages {
        let order = scene.render_order();
        let bg_pos = order
            .iter()
            .position(|n| {
                n.name == "Rect"
                    && (n.rect.width - 612.0).abs() < 1e-4
                    && (n.rect.height - 792.0).abs() < 1e-4
            })
            .unwrap_or_else(|| panic!("Missing full-page page_bg Rect on page {}", scene.page_number));
        let range = scene
            .page_node_range
            .as_ref()
            .expect("paginated scene must record page_node_range");
        let body_text_positions: Vec<usize> = order
            .iter()
            .enumerate()
            .filter(|(_, n)| {
                n.name == "Text"
                    && !range.contains(&n.id.0)
                    && n.rect.width > 0.0
                    && n.rect.height > 0.0
            })
            .map(|(idx, _)| idx)
            .collect();
        assert!(
            !body_text_positions.is_empty(),
            "Page {} must contain at least one placed flow-body Text node",
            scene.page_number
        );
        for body_pos in body_text_positions {
            assert!(
                bg_pos < body_pos,
                "Page {}: page_bg Rect (order {}) must paint before flow-body Text (order {})",
                scene.page_number,
                bg_pos,
                body_pos
            );
        }
    }

    // Chapter 2 opener page must suppress running header (`text: ""`) in top header zone (< 60pt)
    let ch2_scene = &book_paginated.pages[(ch2_pages[0] - 1) as usize];
    let has_running_hdr_on_opener = page_nodes_by_name(ch2_scene, "Text")
        .iter()
        .any(|(n, off)| {
            let y = n.rect.y + off.1;
            let txt = n
                .text_content
                .as_deref()
                .or_else(|| n.properties.get("text").and_then(Value::as_str))
                .unwrap_or("");
            y < 60.0 && !txt.is_empty()
        });
    assert!(
        !has_running_hdr_on_opener,
        "Chapter 2 opener page must suppress running header"
    );

    // Verify Article.dt compiles & paginates with suppressed header on P1 and "Page X of Y" footer
    let para = vec!["typography"; 140].join(" ");
    let article_src = format!(
        r#"
        \use "components/Article.dt"
        \use {{ Footnote, Sidenote }} from "components/Book.dt";

        \Article(
            title: "Reactive Multi-Page Typesetting",
            author: "DirectedType Team",
            date: "2026",
            page_width: 8.5in,
            page_height: 4.5in,
            margin_top: 0.7in,
            margin_bottom: 0.7in
        ) {{
            \Section(number: "1", title: "Introduction") {{
                \Text(width: parent.width) {{
                    Opening prose\Footnote(marker: "1") {{ First note. }} with {para}.
                }}
            }}
        }}
        "#
    );
    let art_paginated = compile_and_paginate(&article_src).expect("Article.dt should compile & paginate");
    assert!(art_paginated.page_count >= 2);
    let total = art_paginated.page_count;
    // Last footer text on each page should read "Page k of total"
    for (idx, scene) in art_paginated.pages.iter().enumerate() {
        let mut saw_footer = false;
        let expected = format!("Page {} of {}", idx + 1, total);
        if let Some(inc) = &scene.included_nodes {
            for n in &scene.layout.nodes {
                if inc.contains(&n.id) && n.name == "Text" {
                    let t = n
                        .text_content
                        .as_deref()
                        .or_else(|| n.properties.get("text").and_then(Value::as_str));
                    if t == Some(expected.as_str()) {
                        saw_footer = true;
                    }
                }
            }
        }
        assert!(saw_footer, "Missing footer '{}' on article page {}", expected, idx + 1);
    }
}

#[cfg(feature = "pdf")]
#[test]
fn test_book_demo_and_article_pdf_export() {
    use directedtype::pdf::{compile_document_to_pdf, PdfOptions};

    let book_src = std::fs::read_to_string("examples/book_demo.dt").expect("read examples/book_demo.dt");
    let doc = parse(&book_src).expect("parse book_demo");
    let registry = ComponentRegistry::standard();
    let pdf_bytes = compile_document_to_pdf(
        &doc,
        Path::new("."),
        &FsResolver,
        &registry,
        &PdfOptions::default(),
    )
    .expect("export book_demo.dt to PDF");
    assert!(pdf_bytes.starts_with(b"%PDF-"));
    assert!(pdf_bytes.len() > 10_000);
}
