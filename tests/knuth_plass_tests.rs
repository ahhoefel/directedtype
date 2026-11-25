use std::path::Path;

use directedtype::compiler::text::{
    measure_rich_text_bounds_full, prepare_rich_text_for_shaping_with_options,
    register_local_fonts_for_base_dir, resolve_active_rich_text, shape_paragraph_lines_with_options,
    InlineBoxSpan, LineBreakMode, ParagraphLayoutOptions, TextSpan, FORCED_JUSTIFY_BREAK,
};
use directedtype::compiler::{compile_document_with_resolver, evaluate_document_with_base_dir, FsResolver};
use directedtype::parser::parse_document;

fn eval_src(src: &str) -> directedtype::compiler::ResolvedLayout {
    let doc = parse_document(src).expect("parse should succeed");
    evaluate_document_with_base_dir(&doc, Path::new(".")).expect("evaluation should succeed")
}

#[test]
fn test_paragraph_style_line_break_optimal_and_inheritance() {
    let src = r#"
    \use "components/Align.dt"

    \let base_ps = \ParagraphStyle(
        align: Align.Justify,
        indent: 16pt,
        line_break: LineBreak.Optimal,
        word_spacing: 0.5pt,
        space_stretch: 0.6,
        space_shrink: 0.35,
        tolerance: 2.5,
        hyphen_penalty: 60.0,
        looseness: 0
    );
    \let child_ps = \ParagraphStyle(paragraph_style: base_ps, indent: 0pt);

    \env font = \Font(family: "Arial", size: 12pt, line_height: 16pt);

    \Text(x: 0pt, y: 0pt, width: 240pt, paragraph_style: base_ps) {
        Alice was beginning to get very tired of sitting by her sister on the bank, and of having nothing to do.
    }
    \Text(x: 0pt, y: 100pt, width: 240pt, paragraph_style: child_ps) {
        Alice was beginning to get very tired of sitting by her sister on the bank, and of having nothing to do.
    }
    "#;

    let layout = eval_src(src);
    let texts: Vec<_> = layout
        .nodes
        .iter()
        .filter(|n| n.name == "Text" && n.text_content.is_some())
        .collect();
    assert_eq!(texts.len(), 2);

    let opts0 = ParagraphLayoutOptions::from_node_properties(&texts[0].properties);
    let opts1 = ParagraphLayoutOptions::from_node_properties(&texts[1].properties);
    assert_eq!(opts0.line_break, LineBreakMode::Optimal);
    assert_eq!(opts1.line_break, LineBreakMode::Optimal);
    assert!((opts0.word_spacing - opts1.word_spacing).abs() < 1e-6);
    assert!((opts0.space_stretch - 0.6).abs() < 1e-6);
    assert!((opts1.space_stretch - 0.6).abs() < 1e-6);
    assert!((opts0.space_shrink - 0.35).abs() < 1e-6);
    assert!((opts1.space_shrink - 0.35).abs() < 1e-6);
    assert!((opts0.tolerance - 2.5).abs() < 1e-6);
    assert!((opts1.hyphen_penalty - 60.0).abs() < 1e-6);
}

#[test]
fn test_knuth_plass_shrinkability_reduces_height_consistently_across_eval_and_pagination() {
    // Measure natural single-line width of a 6-word sentence, then set container width slightly narrower
    // (by ~15% of total space width) so Greedy spills the last word onto line 2 while Optimal shrinks
    // the spaces and fits cleanly on 1 line!
    let sentence = "one two three four five six seven";
    let (nat_w, _) = measure_rich_text_bounds_full(
        sentence,
        12.0,
        400.0,
        Some("Arial"),
        false,
        0.0,
        None,
        Some("LineHeight"),
        Some("LineHeight"),
        Some(18.0),
        Some("Justify"),
        0.0,
        &[],
        &[],
        false,
        &ParagraphLayoutOptions::default(),
    );
    // 6 spaces at 12pt Arial ≈ 20pt total space -> 33% shrink ≈ 6.6pt.
    // Subtract 3.0pt from natural width: greedy MUST wrap to 2 lines, Knuth-Plass fits on 1 line!
    let tight_w = nat_w - 3.0;

    let src = format!(
        r#"
        \use "components/Align.dt"

        \env font = \Font(family: "Arial", size: 12px, line_height: 18px);
        \env text_bounds = \TextBounds(start_at: TextStart.LineHeight, end_at: TextEnd.LineHeight);

        \Text(
            x: 0px,
            y: 0px,
            width: {tight_w:.3}px,
            paragraph_style: \ParagraphStyle(align: Align.Justify, line_break: LineBreak.Greedy)
        ) {{ {sentence} }}

        \Text(
            x: 0px,
            y: 100px,
            width: {tight_w:.3}px,
            paragraph_style: \ParagraphStyle(align: Align.Justify, line_break: LineBreak.Optimal)
        ) {{ {sentence} }}
        "#
    );

    let layout = eval_src(&src);
    let texts: Vec<_> = layout
        .nodes
        .iter()
        .filter(|n| n.name == "Text" && n.text_content.is_some())
        .collect();
    assert_eq!(texts.len(), 2);

    let greedy_h = texts[0].rect.height;
    let optimal_h = texts[1].rect.height;
    assert!(
        greedy_h > 25.0,
        "Greedy should wrap to 2 lines (~36pt), got {greedy_h}"
    );
    assert!(
        optimal_h < 25.0,
        "Knuth-Plass with shrinkable spaces should fit on 1 line (~18pt), got {optimal_h}"
    );

    // Verify shape_paragraph_lines_with_options matches eval height
    let shaped_opt = shape_paragraph_lines_with_options(
        sentence,
        12.0,
        400.0,
        Some("Arial"),
        0.0,
        Some(tight_w),
        Some("LineHeight"),
        Some("LineHeight"),
        Some(18.0),
        Some("Justify"),
        0.0,
        false,
        false,
        &[],
        &[],
        &ParagraphLayoutOptions {
            line_break: LineBreakMode::Optimal,
            ..Default::default()
        },
    );
    assert_eq!(shaped_opt.lines.len(), 1);
    assert!((shaped_opt.total_height - optimal_h).abs() < 0.1);
}

#[test]
fn test_knuth_plass_soft_hyphens_nonbreaking_hyphens_and_forced_breaks() {
    let src = r#"
    \use "components/Align.dt"

    \env font = \Font(family: "Arial", size: 12pt, line_height: 16pt);
    \env paragraph_style = \ParagraphStyle(align: Align.Justify, line_break: LineBreak.Optimal);

    \Text(x: 0pt, y: 0pt, width: 140pt) {
        Alice was beginning to get very tired\|
        of sitting by her sister on the bank, with bread\=and\=butter and cu\-ri\-ouser things happening.
    }
    "#;

    let layout = eval_src(src);
    let node = layout
        .nodes
        .iter()
        .find(|n| n.name == "Text" && n.text_content.is_some())
        .expect("should have Text node");
    let text = node.text_content.as_deref().unwrap();
    let opts = ParagraphLayoutOptions::from_node_properties(&node.properties);
    let mut font_cx = directedtype::compiler::text::create_font_context();
    let prep = prepare_rich_text_for_shaping_with_options(
        &mut font_cx,
        text,
        12.0,
        400.0,
        Some("Arial"),
        false,
        false,
        0.0,
        Some(16.0),
        Some(node.rect.width),
        Some("Justify"),
        0.0,
        &node.text_spans,
        &node.inline_boxes,
        &opts,
    )
    .expect("Optimal mode should return PreparedRichText");

    assert!(prep.has_explicit_breaks);
    // Untaken soft hyphens must all be stripped
    assert!(
        !prep.text.contains('\u{00AD}'),
        "No raw U+00AD soft hyphen should remain: {:?}",
        prep.text
    );
    // Forced break after "tired" must still be present
    assert!(
        prep.text.contains("tired\u{2060}\n"),
        "Mid-paragraph forced break \\| must be preserved: {:?}",
        prep.text
    );
    // Non-breaking hyphen word `bread-and-butter` must remain intact on a single line
    let lines: Vec<&str> = prep.text.split('\n').collect();
    assert!(
        lines.iter().any(|l| l.contains("bread\u{2060}-\u{2060}and\u{2060}-\u{2060}butter")),
        "Non-breaking hyphenated compound must stay intact on one line: {:?}",
        lines
    );
}

#[test]
fn test_knuth_plass_word_spacing_and_looseness_and_ragged() {
    let body = "In olden times when wishing still helped one, there lived a king whose daughters were all beautiful; and the youngest was so beautiful that the sun itself, which has seen so much, was astonished whenever it shone in her face.";

    let src = format!(
        r#"
        \use "components/Align.dt"

        \env font = \Font(family: "Arial", size: 12px, line_height: 16px);
        \env text_bounds = \TextBounds(start_at: TextStart.LineHeight, end_at: TextEnd.LineHeight);

        \Text(x: 0px, y: 0px, width: 220px, paragraph_style: \ParagraphStyle(align: Align.Justify, line_break: LineBreak.Optimal, word_spacing: 0px)) {{ {body} }}
        \Text(x: 0px, y: 200px, width: 220px, paragraph_style: \ParagraphStyle(align: Align.Justify, line_break: LineBreak.Optimal, word_spacing: 4px)) {{ {body} }}
        \Text(x: 0px, y: 400px, width: 220px, paragraph_style: \ParagraphStyle(align: Align.Justify, line_break: LineBreak.Optimal, looseness: 1, tolerance: 10.0)) {{ {body} }}
        \Text(x: 0px, y: 600px, width: 220px, paragraph_style: \ParagraphStyle(align: Align.Left, line_break: LineBreak.Optimal)) {{ {body} }}
        \Text(x: 0px, y: 800px, width: 220px, paragraph_style: \ParagraphStyle(align: Align.Center, line_break: LineBreak.Optimal)) {{ {body} }}
        "#
    );

    let layout = eval_src(&src);
    let texts: Vec<_> = layout
        .nodes
        .iter()
        .filter(|n| n.name == "Text" && n.text_content.is_some())
        .collect();
    assert_eq!(texts.len(), 5);

    let h_base = texts[0].rect.height;
    let h_wide_ws = texts[1].rect.height;
    let h_loose = texts[2].rect.height;
    let h_left = texts[3].rect.height;
    let h_center = texts[4].rect.height;

    assert!(
        h_wide_ws > h_base + 10.0,
        "Large positive word_spacing (4pt) should increase line count: base={h_base}, wide={h_wide_ws}"
    );
    assert!(
        (h_loose - (h_base + 16.0)).abs() < 0.5,
        "looseness: 1 should add 1 line (16pt): base={h_base}, loose={h_loose}"
    );
    assert!(h_left > 40.0 && h_center > 40.0);
}

#[cfg(feature = "pdf")]
#[test]
fn test_knuth_plass_pdf_export_with_shrunk_and_stretched_lines() {
    use directedtype::pdf::{compile_source_to_pdf, PdfOptions};

    let src = r#"
    \use "components/Align.dt"

    \env font = \Font(family: "Arial", size: 12pt, line_height: 16pt);
    \env paragraph_style = \ParagraphStyle(
        align: Align.Justify,
        indent: 16pt,
        line_break: LineBreak.Optimal,
        word_spacing: 0.2pt
    );

    \Text(x: 36pt, y: 36pt, width: 260pt) {
        In olden times when wishing still helped one, there lived a king whose daughters were all beautiful;
        and the youngest was so beautiful that the sun itself, which has seen so much, was astonished whenever it shone in her face.
    }
    "#;

    let opts = PdfOptions {
        compressed: false,
        deterministic: true,
        fallback_width: Some(340.0),
        fallback_height: Some(240.0),
        ..Default::default()
    };
    let bytes = compile_source_to_pdf(src, Path::new("."), &opts).expect("KP PDF export should succeed");
    assert!(bytes.starts_with(b"%PDF-"));
    assert!(bytes.len() > 500);
}

#[test]
fn test_tight_final_line_shrinks_under_justify() {
    use directedtype::compiler::text::{
        break_lines_for_layout, compute_manual_justify_line_spaces, create_font_context,
    };

    let sentence = "one two three four five six seven eight";
    let mut font_cx = create_font_context();
    let mut layout_cx = parley::LayoutContext::<()>::new();

    // Measure unconstrained width at word_spacing = +2.0px
    let mut b = layout_cx.ranged_builder(&mut font_cx, sentence, 1.0, true);
    b.push_default(parley::style::StyleProperty::FontSize(16.0));
    b.push_default(parley::style::StyleProperty::FontFamily(
        parley::style::FontFamily::from("Arial"),
    ));
    b.push_default(parley::style::StyleProperty::WordSpacing(2.0));
    let mut layout = b.build(sentence);
    break_lines_for_layout(&mut layout, None, true, Some("Justify"));
    let full_w = layout.lines().next().unwrap().metrics().advance as f64;

    // Case 1: Short final line (target_width > full_w) -> must stay ragged (0.0, 0)
    let line = layout.lines().next().unwrap();
    let (extra_short, n_short) = compute_manual_justify_line_spaces(
        &line,
        0,
        1,
        sentence,
        Some("Justify"),
        full_w + 20.0,
    );
    assert_eq!(extra_short, 0.0);
    assert_eq!(n_short, 0);

    // Case 2: Tight final line (target_width = full_w - 7.0) -> must shrink evenly to target_width!
    let target_w = full_w - 7.0;
    let (extra_tight, n_tight) = compute_manual_justify_line_spaces(
        &line,
        0,
        1,
        sentence,
        Some("Justify"),
        target_w,
    );
    assert_eq!(n_tight, 7, "should find all 7 spaces");
    assert!(
        (extra_tight - (-1.0)).abs() < 0.01,
        "expected -1.0px per space, got {extra_tight}"
    );
}

// ============================================================================
// Alice's Adventures in Wonderland (1866 Macmillan) Ground-Truth Benchmark
// ============================================================================
// Run with:
//   cargo test --release --test knuth_plass_tests benchmark_alice_knuth_plass_sweep -- --ignored --nocapture

#[derive(Clone)]
struct AliceParagraph {
    text_with_gt: String,
    unforced_text: String,
    unforced_spans: Vec<TextSpan>,
    unforced_boxes: Vec<InlineBoxSpan>,
    gt_lines: Vec<String>,
    /// Byte positions in `unforced_text` corresponding to each ground-truth `\|` break.
    gt_break_positions: Vec<(usize, bool)>, // (byte_pos_in_unforced, is_space_break)
    has_soft_hyphen: bool,
    width: f64,
    indent: f64,
    align: String,
    font_size: f64,
    font_weight: f64,
    font_family: Option<String>,
    font_italic: bool,
    font_small_caps: bool,
    letter_spacing: f64,
    line_height: Option<f64>,
    node_opts: ParagraphLayoutOptions,
}

fn normalize_line_text(raw: &str) -> String {
    raw.replace('\u{2060}', "")
        .replace('\u{00AD}', "")
        .replace('\n', "")
        .trim()
        .to_string()
}

fn adjust_offset(offset: usize, pos: usize, old_len: usize, new_len: usize) -> usize {
    if offset <= pos {
        offset
    } else if offset >= pos + old_len {
        (offset - old_len) + new_len
    } else {
        pos
    }
}


fn extract_alice_paragraphs() -> Vec<AliceParagraph> {
    let alice_dir = Path::new("examples/alice");
    register_local_fonts_for_base_dir(alice_dir);
    let src = std::fs::read_to_string(alice_dir.join("alice.dt")).expect("read alice.dt");
    let doc = parse_document(&src).expect("parse alice.dt");
    let compiled =
        compile_document_with_resolver(&doc, 800.0, 600.0, alice_dir, &FsResolver)
            .expect("compile alice.dt");
    let layout = &compiled.layout;

    // Collect the exact explicit compound-word `-\|` pairs from `ch01.dt`..`ch12.dt` (NOT `\-\|`)
    let mut explicit_hyphen_pairs: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for entry in std::fs::read_dir(alice_dir).expect("read alice dir") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("dt") {
            continue;
        }
        let s = std::fs::read_to_string(&path).unwrap_or_default();
        let mut search_from = 0usize;
        while let Some(rel) = s[search_from..].find("-\\|") {
            let idx = search_from + rel;
            search_from = idx + 3;
            // Exclude `\-\|` (preceded by `\`)
            if idx > 0 && s.as_bytes()[idx - 1] == b'\\' {
                continue;
            }
            let pre_tail: String = s[..idx + 1]
                .chars()
                .rev()
                .take(6)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let after = s[idx + 3..].trim_start();
            let suf_head: String = after.chars().take(6).collect();
            explicit_hyphen_pairs.insert((pre_tail, suf_head));
        }
    }

    let mut paragraphs = Vec::new();
    for node in &layout.nodes {
        if node.name != "Text" || node.rect.width <= 0.0 {
            continue;
        }
        let (Some(active_text), resolved_spans, resolved_boxes, _) =
            resolve_active_rich_text(node, layout)
        else {
            continue;
        };
        if !active_text.contains(FORCED_JUSTIFY_BREAK) {
            continue;
        }

        let fnode = node
            .font
            .or_else(|| node.properties.get("font").and_then(|v| v.as_node()))
            .and_then(|fid| layout.get_node(fid));
        let font_size = fnode
            .and_then(|f| f.properties.get("size"))
            .and_then(|v| v.as_f64())
            .unwrap_or(12.0);
        let font_weight = fnode
            .and_then(|f| f.properties.get("weight"))
            .and_then(|v| v.as_f64())
            .unwrap_or(400.0);
        let font_family = fnode
            .and_then(|f| f.properties.get("family"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let font_italic = fnode
            .and_then(|f| f.properties.get("italic"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let font_small_caps = fnode
            .and_then(|f| f.properties.get("small_caps"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let letter_spacing = fnode
            .and_then(|f| f.properties.get("letter_spacing"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let line_height = fnode
            .filter(|f| f.formulas.has_custom_line_height())
            .and_then(|f| f.properties.get("line_height"))
            .and_then(|v| v.as_f64())
            .filter(|&h| h > 0.0);
        let align = node
            .properties
            .get("align")
            .and_then(|v| v.as_str())
            .unwrap_or("Justify")
            .to_string();
        let indent = node
            .properties
            .get("indent")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let node_opts = ParagraphLayoutOptions::from_node_properties(&node.properties);
        // Compute ground-truth lines by shaping `active_text` with the node's own options (respecting existing `\|` breaks)
        let gt_shaped = shape_paragraph_lines_with_options(
            &active_text,
            font_size,
            font_weight,
            font_family.as_deref(),
            letter_spacing,
            Some(node.rect.width),
            Some("LineHeight"),
            Some("LineHeight"),
            line_height,
            Some(&align),
            indent,
            font_italic,
            font_small_caps,
            &resolved_spans,
            &resolved_boxes,
            &node_opts,
        );
        // Extract exact rendered text of each ground-truth line from `prepare_rich_text_for_shaping_with_options`
        let mut font_cx = directedtype::compiler::text::create_font_context();
        let gt_prep = prepare_rich_text_for_shaping_with_options(
            &mut font_cx,
            &active_text,
            font_size as f32,
            font_weight as f32,
            font_family.as_deref(),
            font_italic,
            font_small_caps,
            letter_spacing,
            line_height,
            Some(node.rect.width),
            Some(&align),
            indent,
            &resolved_spans,
            &resolved_boxes,
            &node_opts,
        );
        let gt_eff_text = gt_prep.as_ref().map(|p| p.text.as_str()).unwrap_or(&active_text);
        let gt_lines: Vec<String> = gt_shaped
            .lines
            .iter()
            .map(|l| normalize_line_text(&gt_eff_text[l.byte_range.clone()]))
            .collect();

        // Collect exact `explicit_hyphen` contexts from ch*.dt (`-\|` NOT preceded by `\`)
        // so `oc\-\|` cleanly restores to `oc\u{00AD}curred` while `half-\|` stays `half-past`!
        let mut unforced_text = active_text.clone();
        let mut unforced_spans = resolved_spans.to_vec();
        let mut unforced_boxes = resolved_boxes.to_vec();
        let mut gt_break_positions: Vec<(usize, bool)> = Vec::new();
        let fb_len = FORCED_JUSTIFY_BREAK.len();

        while let Some(pos) = unforced_text.find(FORCED_JUSTIFY_BREAK) {
            let prev_ch = unforced_text[..pos].chars().next_back();
            let is_em_dash = prev_ch == Some('\u{2014}');
            let is_hyphen = prev_ch == Some('-');
            let is_explicit_compound_hyphen = is_hyphen && {
                // Check if the ~8-char prefix + "-" + ~6-char suffix matches one of the 13 explicit `-\|` in ch*.dt
                let pre = &unforced_text[..pos];
                let suf = &unforced_text[pos + fb_len..];
                let pre_tail: String = pre.chars().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect();
                let suf_head: String = suf.chars().take(6).collect();
                explicit_hyphen_pairs.contains(&(pre_tail, suf_head))
            };

            if is_hyphen && !is_explicit_compound_hyphen {
                // Restore `-\u{2060}\n` (which came from `\-\|`) back to discretionary `U+00AD` (`\-`)!
                let replace_start = pos - 1;
                let old_len = 1 + fb_len;
                let repl = "\u{00AD}";
                let new_len = repl.len();
                unforced_text.replace_range(replace_start..replace_start + old_len, repl);
                for sp in &mut unforced_spans {
                    let s = adjust_offset(sp.range.start, replace_start, old_len, new_len);
                    let e = adjust_offset(sp.range.end, replace_start, old_len, new_len);
                    sp.range = s..e.max(s);
                }
                for ib in &mut unforced_boxes {
                    ib.index = adjust_offset(ib.index, replace_start, old_len, new_len);
                }
                // Ground-truth break position is right after `\u{00AD}` (so re-inserting `\|` turns `\u{00AD}` into `-\u{2060}\n`)
                gt_break_positions.push((replace_start, false));
            } else {
                let join_directly = is_em_dash || is_explicit_compound_hyphen;
                let repl = if join_directly { "" } else { " " };
                let new_len = repl.len();
                unforced_text.replace_range(pos..pos + fb_len, repl);
                for sp in &mut unforced_spans {
                    let s = adjust_offset(sp.range.start, pos, fb_len, new_len);
                    let e = adjust_offset(sp.range.end, pos, fb_len, new_len);
                    sp.range = s..e.max(s);
                }
                for ib in &mut unforced_boxes {
                    ib.index = adjust_offset(ib.index, pos, fb_len, new_len);
                }
                gt_break_positions.push((pos, !join_directly));
            }
        }

        let has_soft_hyphen = unforced_text.contains('\u{00AD}');
        paragraphs.push(AliceParagraph {
            text_with_gt: active_text,
            unforced_text,
            unforced_spans,
            unforced_boxes,
            gt_lines,
            gt_break_positions,
            has_soft_hyphen,
            width: node.rect.width,
            indent,
            align,
            font_size,
            font_weight,
            font_family,
            font_italic,
            font_small_caps,
            letter_spacing,
            line_height,
            node_opts,
        });
    }
    paragraphs
}

fn shape_candidate_lines(
    p: &AliceParagraph,
    text: &str,
    spans: &[TextSpan],
    boxes: &[InlineBoxSpan],
    opts: &ParagraphLayoutOptions,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<()>,
) -> Vec<String> {
    let prep = prepare_rich_text_for_shaping_with_options(
        font_cx,
        text,
        p.font_size as f32,
        p.font_weight as f32,
        p.font_family.as_deref(),
        p.font_italic,
        p.font_small_caps,
        p.letter_spacing,
        p.line_height,
        Some(p.width),
        Some(&p.align),
        p.indent,
        spans,
        boxes,
        opts,
    );
    if let Some(ref pr) = prep {
        if pr.has_explicit_breaks {
            return pr.text.split('\n').map(normalize_line_text).collect();
        }
    }

    let (eff_text, eff_spans, eff_boxes, smcp_ranges, synth_ranges): (
        &str,
        &[TextSpan],
        &[InlineBoxSpan],
        &[(std::ops::Range<usize>, bool)],
        &[(std::ops::Range<usize>, f32)],
    ) = match &prep {
        Some(pr) => (
            pr.text.as_str(),
            pr.spans.as_slice(),
            pr.inline_boxes.as_slice(),
            pr.smcp_ranges.as_slice(),
            pr.synthetic_size_ranges.as_slice(),
        ),
        None => (text, spans, boxes, &[], &[]),
    };

    let mut builder = layout_cx.ranged_builder(font_cx, eff_text, 1.0, true);
    builder.push_default(parley::style::StyleProperty::FontSize(p.font_size as f32));
    if (p.font_weight - 400.0).abs() > 1.0 {
        builder.push_default(parley::style::StyleProperty::FontWeight(
            parley::style::FontWeight::new(p.font_weight as f32),
        ));
    }
    if let Some(ref fam) = p.font_family {
        if !fam.is_empty() {
            builder.push_default(parley::style::StyleProperty::FontFamily(
                parley::style::FontFamily::from(fam.as_str()),
            ));
        }
    }
    if p.font_italic {
        builder.push_default(parley::style::StyleProperty::FontStyle(
            parley::style::FontStyle::Italic,
        ));
    }
    if p.letter_spacing.abs() > 1e-4 {
        builder.push_default(parley::style::StyleProperty::LetterSpacing(
            p.letter_spacing as f32,
        ));
    }
    if opts.word_spacing.abs() > 1e-4 {
        builder.push_default(parley::style::StyleProperty::WordSpacing(
            opts.word_spacing as f32,
        ));
    }
    for sp in eff_spans {
        if !sp.range.is_empty() && sp.range.end <= eff_text.len() {
            if let Some(sz) = sp.style.font_size {
                if sz > 0.0 {
                    builder.push(
                        parley::style::StyleProperty::FontSize(sz as f32),
                        sp.range.clone(),
                    );
                }
            }
            if let Some(wt) = sp.style.font_weight {
                if wt > 0.0 {
                    builder.push(
                        parley::style::StyleProperty::FontWeight(
                            parley::style::FontWeight::new(wt as f32),
                        ),
                        sp.range.clone(),
                    );
                }
            }
            if let Some(ref fam) = sp.style.font_family {
                if !fam.is_empty() {
                    builder.push(
                        parley::style::StyleProperty::FontFamily(
                            parley::style::FontFamily::from(fam.as_str()),
                        ),
                        sp.range.clone(),
                    );
                }
            }
            if sp.style.italic {
                builder.push(
                    parley::style::StyleProperty::FontStyle(parley::style::FontStyle::Italic),
                    sp.range.clone(),
                );
            }
        }
    }
    directedtype::compiler::text::push_small_caps_styles(
        &mut builder,
        smcp_ranges,
        synth_ranges,
        eff_text.len(),
    );
    directedtype::compiler::text::push_first_line_indent_box(
        &mut builder,
        p.indent,
        !eff_text.is_empty() || !eff_boxes.is_empty(),
    );
    for ib in eff_boxes {
        builder.push_inline_box(parley::InlineBox {
            id: ib.node_id.0 as u64,
            kind: parley::InlineBoxKind::InFlow,
            index: ib.index.min(eff_text.len()),
            width: ib.width.max(0.0) as f32,
            height: ib.baseline_offset.max(0.0) as f32,
        });
    }
    let mut layout = builder.build(eff_text);
    layout.break_all_lines(Some(p.width as f32));
    layout
        .lines()
        .map(|l| normalize_line_text(&eff_text[l.text_range()]))
        .collect()
}

/// Computes cumulative non-space character offsets of each line break for prefix-aligned break matching.
fn line_break_cum_chars(lines: &[String]) -> Vec<usize> {
    let mut out = Vec::with_capacity(lines.len().saturating_sub(1));
    let mut acc = 0usize;
    for line in lines.iter().take(lines.len().saturating_sub(1)) {
        // Strip trailing discretionary hyphen from count only? No, keep exact non-space chars
        // except normalize trailing hyphen: compare exact joined text prefix length!
        acc += line.chars().filter(|c| !c.is_whitespace()).count();
        out.push(acc);
    }
    out
}

#[derive(Default)]
struct EvalSummary {
    total_paras: usize,
    exact_paras: usize,
    line_count_match_paras: usize,
    total_gt_breaks: usize,
    matched_breaks: usize,
    no_shy_paras: usize,
    no_shy_exact: usize,
    shy_paras: usize,
    shy_exact: usize,
}

fn evaluate_config(
    paragraphs: &[AliceParagraph],
    opts: &ParagraphLayoutOptions,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<()>,
) -> EvalSummary {
    let mut sum = EvalSummary::default();
    for p in paragraphs {
        sum.total_paras += 1;
        if p.has_soft_hyphen {
            sum.shy_paras += 1;
        } else {
            sum.no_shy_paras += 1;
        }
        let cand_lines = shape_candidate_lines(
            p,
            &p.unforced_text,
            &p.unforced_spans,
            &p.unforced_boxes,
            opts,
            font_cx,
            layout_cx,
        );
        if cand_lines.len() == p.gt_lines.len() {
            sum.line_count_match_paras += 1;
        }
        if cand_lines == p.gt_lines {
            sum.exact_paras += 1;
            if p.has_soft_hyphen {
                sum.shy_exact += 1;
            } else {
                sum.no_shy_exact += 1;
            }
        }
        let gt_cum = line_break_cum_chars(&p.gt_lines);
        let cand_cum = line_break_cum_chars(&cand_lines);
        sum.total_gt_breaks += gt_cum.len();
        for &c in &gt_cum {
            if cand_cum.contains(&c) {
                sum.matched_breaks += 1;
            }
        }
    }
    sum
}

/// Computes exact minimal residual `\|` breaks needed to reproduce 100% of 1866 breaks
/// by iteratively inserting `\|` at the earliest mismatching line break and re-running `opts`.
fn compute_minimal_residual_breaks(
    paragraphs: &[AliceParagraph],
    opts: &ParagraphLayoutOptions,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<()>,
) -> (usize, usize) {
    let mut total_inserted = 0usize;
    let mut paras_needing_any = 0usize;

    for p in paragraphs {
        let mut cur_text = p.unforced_text.clone();
        let mut cur_spans = p.unforced_spans.clone();
        let mut cur_boxes = p.unforced_boxes.clone();
        let mut break_positions = p.gt_break_positions.clone();
        let mut inserted_here = 0usize;

        for _step in 0..=p.gt_break_positions.len() {
            let cand_lines =
                shape_candidate_lines(p, &cur_text, &cur_spans, &cur_boxes, opts, font_cx, layout_cx);
            if cand_lines == p.gt_lines {
                break;
            }
            // Find first line index `k` where `cand_lines[k] != p.gt_lines[k]`
            let mismatch_idx = (0..p.gt_lines.len().saturating_sub(1))
                .find(|&k| cand_lines.get(k) != p.gt_lines.get(k));
            let Some(k) = mismatch_idx else {
                break;
            };
            if k >= break_positions.len() {
                break;
            }
            let (pos, is_space) = break_positions[k];
            let is_shy_break = !is_space && cur_text[pos..].starts_with('\u{00AD}');
            let (old_len, repl_str) = if is_space {
                (1usize, FORCED_JUSTIFY_BREAK)
            } else if is_shy_break {
                ('\u{00AD}'.len_utf8(), "-\u{2060}\n")
            } else {
                (0usize, FORCED_JUSTIFY_BREAK)
            };
            // Avoid double-inserting if already forced
            if cur_text[pos..].starts_with(FORCED_JUSTIFY_BREAK)
                || cur_text[pos..].starts_with("-\u{2060}\n")
            {
                break;
            }
            let new_len = repl_str.len();
            cur_text.replace_range(pos..pos + old_len, repl_str);
            for sp in &mut cur_spans {
                let s = adjust_offset(sp.range.start, pos, old_len, new_len);
                let e = adjust_offset(sp.range.end, pos, old_len, new_len);
                sp.range = s..e.max(s);
            }
            for ib in &mut cur_boxes {
                ib.index = adjust_offset(ib.index, pos, old_len, new_len);
            }
            let delta = new_len as isize - old_len as isize;
            for bp in &mut break_positions[k + 1..] {
                bp.0 = (bp.0 as isize + delta) as usize;
            }
            inserted_here += 1;
        }
        if inserted_here > 0 {
            paras_needing_any += 1;
            total_inserted += inserted_here;
        }
    }
    (total_inserted, paras_needing_any)
}

#[test]
#[ignore]
fn benchmark_alice_knuth_plass_sweep() {
    let paragraphs = extract_alice_paragraphs();
    let mut font_cx = directedtype::compiler::text::create_font_context();
    let mut layout_cx = parley::LayoutContext::<()>::new();
    let total_gt_forced: usize = paragraphs.iter().map(|p| p.gt_break_positions.len()).sum();
    let total_shy_paras = paragraphs.iter().filter(|p| p.has_soft_hyphen).count();
    println!(
        "\n=== Alice 1866 Ground Truth: {} paragraphs ({} with soft hyphens), {} forced \\| breaks ===",
        paragraphs.len(),
        total_shy_paras,
        total_gt_forced
    );

    let print_row = |label: &str, s: &EvalSummary| {
        println!(
            "{:<48} | breaks {:4}/{:4} ({:5.1}%) | exact paras {:3}/{:3} ({:5.1}%) | line-cnt {:3}/{:3} ({:5.1}%) | no-shy {:5.1}% | shy {:5.1}%",
            label,
            s.matched_breaks,
            s.total_gt_breaks,
            100.0 * s.matched_breaks as f64 / s.total_gt_breaks.max(1) as f64,
            s.exact_paras,
            s.total_paras,
            100.0 * s.exact_paras as f64 / s.total_paras.max(1) as f64,
            s.line_count_match_paras,
            s.total_paras,
            100.0 * s.line_count_match_paras as f64 / s.total_paras.max(1) as f64,
            100.0 * s.no_shy_exact as f64 / s.no_shy_paras.max(1) as f64,
            100.0 * s.shy_exact as f64 / s.shy_paras.max(1) as f64,
        );
    };

    // Note: At default 96 DPI, 1pt = 96/72 = 1.33333px (so 12pt font = 16px, 270pt measure = 360px).
    let pt_to_px = 96.0 / 72.0;

    // 0. Measure empirical space widths of the 2,625 GT lines in Old Standard TT at 12pt!
    {
        let mut space_builder = layout_cx.ranged_builder(&mut font_cx, " ", 1.0, true);
        space_builder.push_default(parley::style::StyleProperty::FontSize(12.0 * pt_to_px as f32));
        space_builder.push_default(parley::style::StyleProperty::FontFamily(
            parley::style::FontFamily::from("Old Standard TT"),
        ));
        let mut sp_layout = space_builder.build(" ");
        sp_layout.break_all_lines(None);
        let nat_space_px = sp_layout.full_width() as f64;
        let nat_space_pt = nat_space_px / pt_to_px;

        let mut extra_per_space_pt: Vec<f64> = Vec::new();
        let mut free_per_line_pt: Vec<f64> = Vec::new();
        let mut first_line_extra_pt: Vec<f64> = Vec::new();
        let mut body_line_extra_pt: Vec<f64> = Vec::new();

        for p in &paragraphs {
            let shaped = shape_paragraph_lines_with_options(
                &p.text_with_gt,
                p.font_size,
                p.font_weight,
                p.font_family.as_deref(),
                p.letter_spacing,
                Some(p.width),
                Some("LineHeight"),
                Some("LineHeight"),
                p.line_height,
                Some(&p.align),
                p.indent,
                p.font_italic,
                p.font_small_caps,
                &[],
                &[],
                &ParagraphLayoutOptions::default(),
            );
            let n_lines = shaped.lines.len();
            for (li, gt_line_str) in p.gt_lines.iter().enumerate() {
                if li + 1 >= n_lines {
                    continue;
                }
                let n_sp = gt_line_str.chars().filter(|&c| c == ' ' || c == '\u{00A0}').count();
                if n_sp == 0 {
                    continue;
                }
                // Measure natural width of this GT line
                let mut b = layout_cx.ranged_builder(&mut font_cx, gt_line_str, 1.0, true);
                b.push_default(parley::style::StyleProperty::FontSize(p.font_size as f32));
                if let Some(ref fam) = p.font_family {
                    b.push_default(parley::style::StyleProperty::FontFamily(
                        parley::style::FontFamily::from(fam.as_str()),
                    ));
                }
                let mut l = b.build(gt_line_str);
                l.break_all_lines(None);
                let line_nat_px = l.full_width() as f64 + if li == 0 { p.indent } else { 0.0 };
                let free_px = p.width - line_nat_px;
                let free_pt = free_px / pt_to_px;
                let extra_pt = free_pt / (n_sp as f64);
                free_per_line_pt.push(free_pt);
                extra_per_space_pt.push(extra_pt);
                if li == 0 && p.indent > 0.0 {
                    first_line_extra_pt.push(extra_pt);
                } else {
                    body_line_extra_pt.push(extra_pt);
                }
            }
        }
        extra_per_space_pt.sort_by(|a, b| a.total_cmp(b));
        free_per_line_pt.sort_by(|a, b| a.total_cmp(b));
        let mean_extra = extra_per_space_pt.iter().sum::<f64>() / extra_per_space_pt.len().max(1) as f64;
        let p10 = extra_per_space_pt[extra_per_space_pt.len() / 10];
        let p50 = extra_per_space_pt[extra_per_space_pt.len() / 2];
        let p90 = extra_per_space_pt[extra_per_space_pt.len() * 9 / 10];
        let mean_free = free_per_line_pt.iter().sum::<f64>() / free_per_line_pt.len().max(1) as f64;
        let mean_first = first_line_extra_pt.iter().sum::<f64>() / first_line_extra_pt.len().max(1) as f64;
        let mean_body = body_line_extra_pt.iter().sum::<f64>() / body_line_extra_pt.len().max(1) as f64;
        println!(
            "\n--- Empirical 1866 GT Space Geometry (Old Standard TT 12pt) ---\n\
             Font natural space: {:.2}pt ({:.2}px)\n\
             GT justified extra/space: mean={:+.2}pt, median={:+.2}pt, p10={:+.2}pt, p90={:+.2}pt\n\
             GT total justified space: mean={:.2}pt (vs natural {:.2}pt, ratio={:.2}x)\n\
             GT mean free width/line:  {:+.2}pt | first-line extra/sp={:+.2}pt vs body-line extra/sp={:+.2}pt",
            nat_space_pt,
            nat_space_px,
            mean_extra,
            p50,
            p10,
            p90,
            nat_space_pt + mean_extra,
            nat_space_pt,
            (nat_space_pt + mean_extra) / nat_space_pt,
            mean_free,
            mean_first,
            mean_body,
        );
    }

    // 1. Greedy baseline sweep over word_spacing (0pt to +7pt)
    println!("\n--- Greedy Baseline Sweep (word_spacing in pt) ---");
    let mut best_greedy_ws_pt = 0.0f64;
    let mut best_greedy_breaks = 0usize;
    for &ws_pt in &[0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5, 6.0, 7.0] {
        let opts = ParagraphLayoutOptions {
            line_break: LineBreakMode::Greedy,
            word_spacing: ws_pt * pt_to_px,
            ..Default::default()
        };
        let sum = evaluate_config(&paragraphs, &opts, &mut font_cx, &mut layout_cx);
        print_row(&format!("Greedy ws={:+.2}pt ({:+.2}px)", ws_pt, ws_pt * pt_to_px), &sum);
        if sum.matched_breaks > best_greedy_breaks {
            best_greedy_breaks = sum.matched_breaks;
            best_greedy_ws_pt = ws_pt;
        }
    }

    // 2. Knuth-Plass sweep over word_spacing (0pt to +8pt, classic stretch=0.5, shrink=0.333)
    println!("\n--- Knuth-Plass Sweep (word_spacing in pt, classic stretch=0.50 shrink=0.33) ---");
    let mut best_ws_pt = 0.0f64;
    let mut best_breaks = 0usize;
    for &ws_pt in &[0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5, 6.0, 7.0, 8.0] {
        let opts = ParagraphLayoutOptions {
            line_break: LineBreakMode::Optimal,
            word_spacing: ws_pt * pt_to_px,
            ..Default::default()
        };
        let sum = evaluate_config(&paragraphs, &opts, &mut font_cx, &mut layout_cx);
        print_row(&format!("KP ws={:+.2}pt ({:+.2}px)", ws_pt, ws_pt * pt_to_px), &sum);
        if sum.matched_breaks > best_breaks {
            best_breaks = sum.matched_breaks;
            best_ws_pt = ws_pt;
        }
    }

    // 3. Fine sweep around best_ws_pt + stretch/shrink/sentence_space/hyphen_penalty
    println!("\n--- Knuth-Plass Fine Sweep (around ws={:+.2}pt) ---", best_ws_pt);
    let mut best_opts = ParagraphLayoutOptions {
        line_break: LineBreakMode::Optimal,
        word_spacing: best_ws_pt * pt_to_px,
        ..Default::default()
    };
    let mut best_score = (0usize, 0usize, 0usize); // (matched_breaks, exact_paras, line_count)

    for ws_step in -6..=6 {
        let ws_pt = best_ws_pt + (ws_step as f64) * 0.25;
        for &(stretch, shrink) in &[
            (0.5, 1.0 / 3.0),
            (0.5, 0.25),
            (0.5, 0.40),
            (0.67, 0.333),
            (0.35, 0.20),
            (0.35, 0.15),
        ] {
            for &sent in &[0.0, 0.25, 0.5] {
                for &hp in &[50.0, 20.0, 10.0] {
                    let opts = ParagraphLayoutOptions {
                        line_break: LineBreakMode::Optimal,
                        word_spacing: ws_pt * pt_to_px,
                        space_stretch: stretch,
                        space_shrink: shrink,
                        sentence_space: sent,
                        hyphen_penalty: hp,
                        ..Default::default()
                    };
                    let sum = evaluate_config(&paragraphs, &opts, &mut font_cx, &mut layout_cx);
                    let score = (sum.matched_breaks, sum.exact_paras, sum.line_count_match_paras);
                    if score > best_score {
                        best_score = score;
                        best_opts = opts;
                        print_row(
                            &format!(
                                "BEST ws={:+.2}pt st={:.2} sh={:.2} se={:.2} hp={:.0}",
                                ws_pt, stretch, shrink, sent, hp
                            ),
                            &sum,
                        );
                    }
                }
            }
        }
    }

    // 4. Residual minimal `\|` breaks comparison!
    println!("\n--- Minimal Residual \\| Breaks Needed for 100% 1866 Fidelity ---");
    let configs: Vec<(&str, ParagraphLayoutOptions)> = vec![
        ("Greedy default (ws=0.0pt)", ParagraphLayoutOptions::default()),
        (
            "Greedy tuned best_ws",
            ParagraphLayoutOptions {
                line_break: LineBreakMode::Greedy,
                word_spacing: best_greedy_ws_pt * pt_to_px,
                ..Default::default()
            },
        ),
        (
            "KP default (ws=0.0pt, st=0.5, sh=0.33)",
            ParagraphLayoutOptions {
                line_break: LineBreakMode::Optimal,
                ..Default::default()
            },
        ),
        (
            "KP tuned best_ws only",
            ParagraphLayoutOptions {
                line_break: LineBreakMode::Optimal,
                word_spacing: best_ws_pt * pt_to_px,
                ..Default::default()
            },
        ),
        ("KP tuned full best", best_opts),
    ];

    for (name, opts) in &configs {
        let sum = evaluate_config(&paragraphs, opts, &mut font_cx, &mut layout_cx);
        let (residual, paras_touched) =
            compute_minimal_residual_breaks(&paragraphs, opts, &mut font_cx, &mut layout_cx);
        let pct_removed = 100.0 * (1.0 - (residual as f64 / total_gt_forced.max(1) as f64));
        println!(
            "{:<38} | residual \\|: {:4}/{:4} ({:5.1}% removed) | paras needing \\|: {:3}/{:3} | line-cnt match: {:3}/{:3}",
            name,
            residual,
            total_gt_forced,
            pct_removed,
            paras_touched,
            paragraphs.len(),
            sum.line_count_match_paras,
            paragraphs.len(),
        );
    }

    // 5. Print first 8 mismatching paragraphs under `best_opts` for qualitative inspection
    println!("\n--- Sample Mismatches under KP tuned full best ({:?}) ---", best_opts);
    let mut shown = 0;
    for (idx, p) in paragraphs.iter().enumerate() {
        let cand = shape_candidate_lines(
            p,
            &p.unforced_text,
            &p.unforced_spans,
            &p.unforced_boxes,
            &best_opts,
            &mut font_cx,
            &mut layout_cx,
        );
        if cand != p.gt_lines {
            println!(
                "\n[Para #{idx} w={:.1} indent={:.1} shy={}] GT={} lines vs KP={} lines",
                p.width,
                p.indent,
                p.has_soft_hyphen,
                p.gt_lines.len(),
                cand.len()
            );
            let n = p.gt_lines.len().max(cand.len());
            for k in 0..n {
                let g = p.gt_lines.get(k).map(|s| s.as_str()).unwrap_or("<none>");
                let c = cand.get(k).map(|s| s.as_str()).unwrap_or("<none>");
                let mark = if g == c { "  " } else { "!=" };
                println!("  L{k:02} {mark} GT: {g}");
                if g != c {
                    println!("         KP: {c}");
                }
            }
            shown += 1;
            if shown >= 8 {
                break;
            }
        }
    }
    let _ = paragraphs.first().map(|p| &p.text_with_gt);
}

#[test]
#[ignore]
fn analyze_alice_measure_and_longest_lines() {
    let paragraphs = extract_alice_paragraphs();
    let mut font_cx = directedtype::compiler::text::create_font_context();
    let mut layout_cx = parley::LayoutContext::<()>::new();
    let pt_to_px = 96.0 / 72.0;

    // Measure natural width (at word_spacing = 0) of every non-final body GT line (w == 270pt == 360px)
    // using exact spans, small caps, inline boxes, and indent!
    let mut body_lines: Vec<(f64, f64, usize, String)> = Vec::new(); // (nat_w_pt, min_shrunk_w_pt, n_spaces, text)
    let nat_space_pt = {
        let mut sb = layout_cx.ranged_builder(&mut font_cx, " ", 1.0, true);
        sb.push_default(parley::style::StyleProperty::FontSize(12.0 * pt_to_px as f32));
        sb.push_default(parley::style::StyleProperty::FontFamily(
            parley::style::FontFamily::from("Old Standard TT"),
        ));
        let mut l = sb.build(" ");
        l.break_all_lines(None);
        (l.full_width() as f64) / pt_to_px
    };

    for p in &paragraphs {
        // Focus on full-width body paragraphs (270pt = 360px)
        if (p.width - 360.0).abs() > 1.0 {
            continue;
        }
        // Shape GT text with Parley at break_all_lines(None) so each `\n` line gets exact span/italic/small-caps/inline-box advance!
        let prep = prepare_rich_text_for_shaping_with_options(
            &mut font_cx,
            &p.text_with_gt,
            p.font_size as f32,
            p.font_weight as f32,
            p.font_family.as_deref(),
            p.font_italic,
            p.font_small_caps,
            p.letter_spacing,
            p.line_height,
            Some(p.width),
            Some(&p.align),
            p.indent,
            &p.unforced_spans, // Wait: use GT spans? Re-extract from GT layout via p.gt_lines below
            &[],
            &ParagraphLayoutOptions::default(),
        );
        let _ = prep;
        // Measure each GT line directly (note: slight diff for italic/DropInitial; let's also report exact Parley layout line widths!)
        let n_lines = p.gt_lines.len();
        for (li, line_str) in p.gt_lines.iter().enumerate() {
            if li + 1 >= n_lines {
                continue;
            }
            let mut b = layout_cx.ranged_builder(&mut font_cx, line_str, 1.0, true);
            b.push_default(parley::style::StyleProperty::FontSize(p.font_size as f32));
            if let Some(ref fam) = p.font_family {
                b.push_default(parley::style::StyleProperty::FontFamily(
                    parley::style::FontFamily::from(fam.as_str()),
                ));
            }
            if p.font_italic {
                b.push_default(parley::style::StyleProperty::FontStyle(
                    parley::style::FontStyle::Italic,
                ));
            }
            if li == 0 && p.indent > 0.0 {
                directedtype::compiler::text::push_first_line_indent_box(&mut b, p.indent, true);
            }
            let mut l = b.build(line_str);
            l.break_all_lines(None);
            let w_pt = (l.full_width() as f64) / pt_to_px;
            let n_sp = line_str.chars().filter(|&c| c == ' ' || c == '\u{00A0}').count();
            let shrunk33_pt = w_pt - (n_sp as f64) * nat_space_pt * (1.0 / 3.0);
            body_lines.push((w_pt, shrunk33_pt, n_sp, line_str.clone()));
        }
    }

    body_lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    let n = body_lines.len();
    let mean_w: f64 = body_lines.iter().map(|x| x.0).sum::<f64>() / n.max(1) as f64;
    println!("\n=== Exact Natural Width Distribution of {} Full-Measure 1866 Body Lines ===", n);
    println!(
        "min={:.1}pt | p10={:.1}pt | p25={:.1}pt | p50={:.1}pt | mean={:.1}pt | p75={:.1}pt | p90={:.1}pt | p95={:.1}pt | p99={:.1}pt | max={:.1}pt",
        body_lines[0].0,
        body_lines[n / 10].0,
        body_lines[n / 4].0,
        body_lines[n / 2].0,
        mean_w,
        body_lines[n * 3 / 4].0,
        body_lines[n * 9 / 10].0,
        body_lines[n * 95 / 100].0,
        body_lines[n * 99 / 100].0,
        body_lines[n - 1].0,
    );

    println!("\n--- Top 15 Longest Natural Lines in 1866 Alice (at 12pt Old Standard TT, ws=0pt) ---");
    for (w_pt, shrunk_pt, n_sp, text) in body_lines.iter().rev().take(15) {
        println!(
            "  nat={:5.1}pt | shrunk(33%)={:5.1}pt | spaces={:2} | {}",
            w_pt, shrunk_pt, n_sp, text
        );
    }

    println!("\n--- How Many GT Lines Exceed Measure M (Unshrunk vs 33%-Shrunk)? ---");
    for &m_pt in &[230.0, 235.0, 240.0, 245.0, 250.0, 255.0, 260.0, 265.0, 270.0] {
        let over_nat = body_lines.iter().filter(|x| x.0 > m_pt).count();
        let over_shrunk = body_lines.iter().filter(|x| x.1 > m_pt).count();
        println!(
            "  measure={:5.1}pt | > nat: {:4}/{:4} ({:4.1}%) | > 33%-shrunk: {:4}/{:4} ({:4.1}%)",
            m_pt,
            over_nat,
            n,
            100.0 * over_nat as f64 / n.max(1) as f64,
            over_shrunk,
            n,
            100.0 * over_shrunk as f64 / n.max(1) as f64,
        );
    }

    // Now sweep (body_measure_pt, word_spacing_pt) with Knuth-Plass (scaling WrapLeft/WrapRight proportionally!)
    println!("\n=== Joint Sweep: Body Measure (230pt..270pt) x Word Spacing (0pt..+4.5pt) under Knuth-Plass ===");
    println!("(Scaling narrow wrap column widths by measure/270.0 as well; st=0.50, sh=0.33, se=0.50, hp=20)");
    for &m_pt in &[235.0, 240.0, 245.0, 250.0, 255.0, 260.0, 265.0, 270.0] {
        let scale = m_pt / 270.0;
        let scaled_paras: Vec<AliceParagraph> = paragraphs
            .iter()
            .map(|p| {
                let mut q = p.clone();
                q.width = p.width * scale;
                q
            })
            .collect();

        let mut best_for_m = (0usize, 0usize, 0usize, 0.0f64, 0.0f64);
        for ws_q in -2..=18 {
            let ws_pt = (ws_q as f64) * 0.25;
            let opts = ParagraphLayoutOptions {
                line_break: LineBreakMode::Optimal,
                word_spacing: ws_pt * pt_to_px,
                space_stretch: 0.50,
                space_shrink: 1.0 / 3.0,
                sentence_space: 0.50,
                hyphen_penalty: 20.0,
                ..Default::default()
            };
            let sum = evaluate_config(&scaled_paras, &opts, &mut font_cx, &mut layout_cx);
            if sum.matched_breaks > best_for_m.0 {
                // Also compute average justified space at this measure
                let avg_free_pt = m_pt - mean_w;
                best_for_m = (
                    sum.matched_breaks,
                    sum.exact_paras,
                    sum.line_count_match_paras,
                    ws_pt,
                    avg_free_pt,
                );
            }
        }
        // Also evaluate ws=0.0pt directly at this measure!
        let opts_zero = ParagraphLayoutOptions {
            line_break: LineBreakMode::Optimal,
            word_spacing: 0.0,
            space_stretch: 0.50,
            space_shrink: 1.0 / 3.0,
            sentence_space: 0.50,
            hyphen_penalty: 20.0,
            ..Default::default()
        };
        let sum_zero = evaluate_config(&scaled_paras, &opts_zero, &mut font_cx, &mut layout_cx);
        let opts_best = ParagraphLayoutOptions {
            line_break: LineBreakMode::Optimal,
            word_spacing: best_for_m.3 * pt_to_px,
            space_stretch: 0.50,
            space_shrink: 1.0 / 3.0,
            sentence_space: 0.50,
            hyphen_penalty: 20.0,
            ..Default::default()
        };
        let (res_best, _) = compute_minimal_residual_breaks(
            &scaled_paras,
            &opts_best,
            &mut font_cx,
            &mut layout_cx,
        );
        let (res_zero, _) = compute_minimal_residual_breaks(
            &scaled_paras,
            &opts_zero,
            &mut font_cx,
            &mut layout_cx,
        );
        println!(
            "measure={:5.1}pt | ws=0.0pt: breaks {:5.1}% exact {:5.1}% line-cnt {:5.1}% res\\|={:4} | best ws={:+.2}pt: breaks {:5.1}% exact {:5.1}% line-cnt {:5.1}% res\\|={:4} (avg free={:+.1}pt)",
            m_pt,
            100.0 * sum_zero.matched_breaks as f64 / sum_zero.total_gt_breaks as f64,
            100.0 * sum_zero.exact_paras as f64 / sum_zero.total_paras as f64,
            100.0 * sum_zero.line_count_match_paras as f64 / sum_zero.total_paras as f64,
            res_zero,
            best_for_m.3,
            100.0 * best_for_m.0 as f64 / sum_zero.total_gt_breaks as f64,
            100.0 * best_for_m.1 as f64 / sum_zero.total_paras as f64,
            100.0 * best_for_m.2 as f64 / sum_zero.total_paras as f64,
            res_best,
            best_for_m.4,
        );
    }
}


#[test]
#[ignore]
fn analyze_alice_255pt_shrinkage() {
    let paragraphs = extract_alice_paragraphs();
    let mut font_cx = directedtype::compiler::text::create_font_context();
    let mut layout_cx = parley::LayoutContext::<()>::new();
    let pt_to_px = 96.0 / 72.0;
    let target_m_pt = 255.0;
    let target_m_px = target_m_pt * pt_to_px;
    let nat_space_pt = 3.36;

    // 1. Inspect rendered space distribution for ALL 2,469 full-measure 1866 GT lines at 255pt!
    // Also inspect ALL narrow wrapped-illustration paragraphs (`width < 359px`) when body shrinks 270 -> 255 (-15pt)!
    println!("\n=== Wrapped Illustration Columns at 255pt (-15pt from 270pt) ===");
    for (p_idx, p) in paragraphs.iter().enumerate() {
        if (p.width - 360.0).abs() <= 1.0 {
            continue;
        }
        let old_w_pt = p.width / pt_to_px;
        let new_w_pt = old_w_pt - 15.0;
        let mut max_line_pt = 0.0f64;
        let mut max_line_shrunk_pt = 0.0f64;
        let mut worst_line = String::new();
        for (li, line_str) in p.gt_lines.iter().enumerate() {
            let mut b = layout_cx.ranged_builder(&mut font_cx, line_str, 1.0, true);
            b.push_default(parley::style::StyleProperty::FontSize(p.font_size as f32));
            if let Some(ref fam) = p.font_family {
                b.push_default(parley::style::StyleProperty::FontFamily(
                    parley::style::FontFamily::from(fam.as_str()),
                ));
            }
            if li == 0 && p.indent > 0.0 {
                directedtype::compiler::text::push_first_line_indent_box(&mut b, p.indent, true);
            }
            let mut l = b.build(line_str);
            l.break_all_lines(None);
            let w_pt = (l.full_width() as f64) / pt_to_px;
            let n_sp = line_str.chars().filter(|&c| c == ' ' || c == '\u{00A0}').count();
            let sh_pt = w_pt - (n_sp as f64) * nat_space_pt * (1.0 / 3.0);
            if w_pt > max_line_pt {
                max_line_pt = w_pt;
                max_line_shrunk_pt = sh_pt;
                worst_line = line_str.clone();
            }
        }
        let flag = if max_line_shrunk_pt > new_w_pt { "OVERFLOW!" } else if max_line_pt > new_w_pt { "tight" } else { "ok" };
        println!(
            "  P#{:03} old_w={:5.1}pt -> new_w={:5.1}pt | max_nat={:5.1}pt shrunk33={:5.1}pt [{:<9}] | {}",
            p_idx, old_w_pt, new_w_pt, max_line_pt, max_line_shrunk_pt, flag, worst_line
        );
    }
    let mut tight_lines: Vec<(f64, f64, f64, usize, usize, String)> = Vec::new(); // (rendered_sp_pt, nat_w_pt, pct_of_nat, n_sp, para_idx, text)
    let mut buckets = [0usize; 7]; // <0, 0..1.5, 1.5..2.24, 2.24..3.36, 3.36..5.0, 5.0..7.0, >=7.0

    for (p_idx, p) in paragraphs.iter().enumerate() {
        if (p.width - 360.0).abs() > 1.0 {
            continue;
        }
        let n_lines = p.gt_lines.len();
        for (li, line_str) in p.gt_lines.iter().enumerate() {
            if li + 1 >= n_lines {
                continue;
            }
            let mut b = layout_cx.ranged_builder(&mut font_cx, line_str, 1.0, true);
            b.push_default(parley::style::StyleProperty::FontSize(p.font_size as f32));
            if let Some(ref fam) = p.font_family {
                b.push_default(parley::style::StyleProperty::FontFamily(
                    parley::style::FontFamily::from(fam.as_str()),
                ));
            }
            if p.font_italic {
                b.push_default(parley::style::StyleProperty::FontStyle(
                    parley::style::FontStyle::Italic,
                ));
            }
            if li == 0 && p.indent > 0.0 {
                directedtype::compiler::text::push_first_line_indent_box(&mut b, p.indent, true);
            }
            let mut l = b.build(line_str);
            l.break_all_lines(None);
            let w_pt = (l.full_width() as f64) / pt_to_px;
            let n_sp = line_str.chars().filter(|&c| c == ' ' || c == '\u{00A0}').count();
            if n_sp == 0 {
                continue;
            }
            let free_pt = target_m_pt - w_pt;
            let rendered_sp_pt = nat_space_pt + free_pt / (n_sp as f64);
            let pct_of_nat = 100.0 * rendered_sp_pt / nat_space_pt;

            let bi = if rendered_sp_pt < 0.0 {
                0
            } else if rendered_sp_pt < 1.5 {
                1
            } else if rendered_sp_pt < 2.24 {
                2
            } else if rendered_sp_pt < 3.36 {
                3
            } else if rendered_sp_pt < 5.0 {
                4
            } else if rendered_sp_pt < 7.0 {
                5
            } else {
                6
            };
            buckets[bi] += 1;

            if rendered_sp_pt < 3.36 {
                tight_lines.push((rendered_sp_pt, w_pt, pct_of_nat, n_sp, p_idx, line_str.clone()));
            }
        }
    }

    let total: usize = buckets.iter().sum();
    println!("\n=== Rendered Space Width Distribution at 255pt (1866 GT breaks, nat space = 3.36pt) ===");
    let labels = [
        "< 0.00pt (OVERFLOW)",
        "0.00–1.50pt (>55% shrink)",
        "1.50–2.24pt (33–55% shrink)",
        "2.24–3.36pt (0–33% shrink)",
        "3.36–5.00pt (mild stretch)",
        "5.00–7.00pt (normal book)",
        ">= 7.00pt (loose)",
    ];
    for (lbl, &cnt) in labels.iter().zip(buckets.iter()) {
        println!("  {:<28}: {:4}/{:4} ({:5.2}%)", lbl, cnt, total, 100.0 * cnt as f64 / total as f64);
    }

    tight_lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    println!("\n--- All {} Tight Lines (< 3.36pt natural space) at 255pt (showing tightest 20) ---", tight_lines.len());
    for (sp, nat_w, pct, n_sp, p_idx, txt) in tight_lines.iter().take(20) {
        println!(
            "  sp={:5.2}pt ({:5.1}% of nat) | nat_w={:5.1}pt | n_sp={:2} | P#{:03} | {}",
            sp, pct, nat_w, n_sp, p_idx, txt
        );
    }

    // 2. Sweep (word_spacing, space_stretch, space_shrink) at 255pt (keeping narrow wraps unchanged vs scaled)
    println!("\n=== Knuth-Plass Parameter Sweep at 255pt (body=255pt, wraps scaled 255/270) ===");
    let scale = 255.0 / 270.0;
    let scaled_paras: Vec<AliceParagraph> = paragraphs
        .iter()
        .map(|p| {
            let mut q = p.clone();
            if (p.width - 360.0).abs() <= 1.0 {
                q.width = target_m_px;
            } else {
                q.width = p.width * scale;
            }
            q
        })
        .collect();

    for &ws_pt in &[1.5, 2.0, 2.25, 2.5, 2.75, 3.0] {
        for &(st, sh) in &[
            (0.35, 0.25),
            (0.45, 0.33),
            (0.50, 0.45),
            (0.50, 0.55),
            (0.55, 0.60),
        ] {
            let nom_sp = nat_space_pt + ws_pt;
            let min_sp = nom_sp * (1.0 - sh);
            let max_sp = nom_sp * (1.0 + st);
            let opts = ParagraphLayoutOptions {
                line_break: LineBreakMode::Optimal,
                word_spacing: ws_pt * pt_to_px,
                space_stretch: st,
                space_shrink: sh,
                sentence_space: 0.50,
                hyphen_penalty: 20.0,
                ..Default::default()
            };
            let sum = evaluate_config(&scaled_paras, &opts, &mut font_cx, &mut layout_cx);
            let (res, _) = compute_minimal_residual_breaks(&scaled_paras, &opts, &mut font_cx, &mut layout_cx);
            println!(
                "  ws={:+.2}pt (nom={:.2}pt, min={:.2}pt, max={:.2}pt) st={:.2} sh={:.2} | breaks {:5.1}% exact {:5.1}% line-cnt {:5.1}% | res\\|={:4} ({:4.1}% rem)",
                ws_pt, nom_sp, min_sp, max_sp, st, sh,
                100.0 * sum.matched_breaks as f64 / sum.total_gt_breaks as f64,
                100.0 * sum.exact_paras as f64 / sum.total_paras as f64,
                100.0 * sum.line_count_match_paras as f64 / sum.total_paras as f64,
                res,
                100.0 * (1.0 - res as f64 / sum.total_gt_breaks as f64),
            );
        }
    }
}

#[test]
#[ignore]
fn prune_alice_redundant_breaks_and_verify() {
    use directedtype::compiler::paginate_compiled_document;

    let alice_dir = Path::new("examples/alice");
    register_local_fonts_for_base_dir(alice_dir);
    let src = std::fs::read_to_string(alice_dir.join("alice.dt")).expect("read alice.dt");
    let doc = parse_document(&src).expect("parse alice.dt");
    let compiled =
        compile_document_with_resolver(&doc, 800.0, 600.0, alice_dir, &FsResolver)
            .expect("compile alice.dt");
    let paginated = paginate_compiled_document(&compiled, 800.0, 600.0, 96.0).expect("paginate alice.dt");
    println!(
        "\n=== Pre-Prune Alice 255pt Pagination: {} total pages ===",
        paginated.pages.len()
    );
    let pre_prune_page_count = paginated.pages.len();
    // Print any Text node whose shaped line count differs from its explicit `\|` count + 1 (or any single-line paragraph that wrapped!)
    {
        let mut fc = directedtype::compiler::text::create_font_context();
        let mut diff_count = 0usize;
        for node in &compiled.layout.nodes {
            if node.name != "Text" || node.rect.width <= 0.0 {
                continue;
            }
            let opts = ParagraphLayoutOptions::from_node_properties(&node.properties);
            if !opts.is_optimal() {
                continue;
            }
            let (Some(txt), spans, boxes, _) = resolve_active_rich_text(node, &compiled.layout) else {
                continue;
            };
            let expected_lines = txt.matches(FORCED_JUSTIFY_BREAK).count() + 1;
            let fnode = node
                .font
                .or_else(|| node.properties.get("font").and_then(|v| v.as_node()))
                .and_then(|fid| compiled.layout.get_node(fid));
            let prep = prepare_rich_text_for_shaping_with_options(
                &mut fc,
                &txt,
                fnode.and_then(|f| f.properties.get("size")).and_then(|v| v.as_f64()).unwrap_or(16.0) as f32,
                fnode.and_then(|f| f.properties.get("weight")).and_then(|v| v.as_f64()).unwrap_or(400.0) as f32,
                fnode.and_then(|f| f.properties.get("family")).and_then(|v| v.as_str()),
                fnode.and_then(|f| f.properties.get("italic")).and_then(|v| v.as_bool()).unwrap_or(false),
                fnode.and_then(|f| f.properties.get("small_caps")).and_then(|v| v.as_bool()).unwrap_or(false),
                fnode.and_then(|f| f.properties.get("letter_spacing")).and_then(|v| v.as_f64()).unwrap_or(0.0),
                fnode.and_then(|f| f.properties.get("line_height")).and_then(|v| v.as_f64()).filter(|&h| h > 0.0),
                Some(node.rect.width),
                node.properties.get("align").and_then(|v| v.as_str()),
                node.properties.get("indent").and_then(|v| v.as_f64()).unwrap_or(0.0),
                &spans,
                &boxes,
                &opts,
            );
            let eff = prep.as_ref().map(|p| p.text.as_str()).unwrap_or(&txt);
            let actual_lines = eff.split('\n').count();
            if actual_lines != expected_lines {
                diff_count += 1;
                if diff_count <= 25 {
                    println!(
                        "  LINE-CNT DIFF #{diff_count}: w={:.1}pt exp={} got={} | {:?}",
                        node.rect.width * 72.0 / 96.0,
                        expected_lines,
                        actual_lines,
                        eff.chars().take(110).collect::<String>()
                    );
                }
            }
        }
        println!("Total Text nodes with actual_lines != forced_breaks+1: {diff_count}");
    }

    // Find exact book folio numbers and lines of the 4 errata paragraphs!
    let errata_needles = [
        ("p67 minutes.", "some minutes."),
        ("p87 that's", "not at all a proper way"),
        ("p102 asked.", "manage?” Alice"),
        ("p119 soldiers", "The soldiers were silent"),
        ("p120 Duchess?", "where’s the Duchess?”"),
    ];
    let mut font_cx = directedtype::compiler::text::create_font_context();
    let mut layout_cx = parley::LayoutContext::<()>::new();

    println!("\n=== Exact Folio & Knuth-Plass Lines of the 4 Errata Paragraphs (255pt) ===");
    for (label, needle) in &errata_needles {
        for (page_idx, page) in paginated.pages.iter().enumerate() {
            let folio = (page_idx as i32 + 1) - 10;
            for node in page.render_order() {
                {
                    if let (Some(txt), spans, boxes, _) = resolve_active_rich_text(node, &page.layout) {
                        if txt.contains(needle) {
                            let fnode = node
                                .font
                                .or_else(|| node.properties.get("font").and_then(|v| v.as_node()))
                                .and_then(|fid| page.layout.get_node(fid));
                            let opts = ParagraphLayoutOptions::from_node_properties(&node.properties);
                            let prep = prepare_rich_text_for_shaping_with_options(
                                &mut font_cx,
                                &txt,
                                fnode.and_then(|f| f.properties.get("size")).and_then(|v| v.as_f64()).unwrap_or(16.0) as f32,
                                fnode.and_then(|f| f.properties.get("weight")).and_then(|v| v.as_f64()).unwrap_or(400.0) as f32,
                                fnode.and_then(|f| f.properties.get("family")).and_then(|v| v.as_str()),
                                fnode.and_then(|f| f.properties.get("italic")).and_then(|v| v.as_bool()).unwrap_or(false),
                                fnode.and_then(|f| f.properties.get("small_caps")).and_then(|v| v.as_bool()).unwrap_or(false),
                                fnode.and_then(|f| f.properties.get("letter_spacing")).and_then(|v| v.as_f64()).unwrap_or(0.0),
                                fnode.and_then(|f| f.properties.get("line_height")).and_then(|v| v.as_f64()).filter(|&h| h > 0.0),
                                Some(node.rect.width),
                                node.properties.get("align").and_then(|v| v.as_str()),
                                node.properties.get("indent").and_then(|v| v.as_f64()).unwrap_or(0.0),
                                &spans,
                                &boxes,
                                &opts,
                            );
                            let eff = prep.as_ref().map(|p| p.text.as_str()).unwrap_or(&txt);
                            let body_top_px = 67.4 * 96.0 / 72.0;
                            let off_y = page.node_offsets.get(&node.id).map(|o| o.1).unwrap_or(0.0);
                            let page_y = node.rect.y + off_y;
                            let start_line_on_page = ((page_y - body_top_px) / 24.0).round() as i32 + 1;
                            let slice_range = page.text_line_slices.get(&node.id).map(|s| s.line_range.clone());
                            let slice_start = slice_range.as_ref().map(|r| r.start).unwrap_or(0);
                            let slice_end = slice_range.as_ref().map(|r| r.end).unwrap_or(usize::MAX);
                            println!("\n[{label}] Book Page (folio) = p. {folio} (page_y={page_y:.1}px, page_line_start={start_line_on_page}, slice={slice_start}..{slice_end}):");
                            for (li, line) in eff.split('\n').enumerate() {
                                if li >= slice_start && li < slice_end {
                                    let page_ln = start_line_on_page + (li - slice_start) as i32;
                                    println!("  p.{folio} l.{page_ln:2}: {}", normalize_line_text(line));
                                }
                            }
                            break;
                        }
                    }
                }
            }
        }
    }

    // Build exact per-paragraph target lines & minimal keep mask across all ch01..ch12 `\|` breaks!
    let paragraphs = extract_alice_paragraphs();
    let compiled_total_forced: usize = paragraphs.iter().map(|p| p.gt_break_positions.len()).sum();

    let chapter_files: Vec<std::path::PathBuf> = (1..=12)
        .map(|i| alice_dir.join(format!("ch{:02}.dt", i)))
        .collect();
    let source_total_forced: usize = chapter_files
        .iter()
        .map(|f| std::fs::read_to_string(f).unwrap().matches("\\|").count())
        .sum();
    println!(
        "\n=== Forced-break bijection check: compiled={} vs ch01..ch12 source={} ===",
        compiled_total_forced, source_total_forced
    );
    assert_eq!(
        compiled_total_forced, source_total_forced,
        "1-to-1 bijection between compiled \\| breaks and ch01..ch12 source \\| escapes must hold!"
    );

    // Compute exact global keep mask (one bool per `\|` in document order) using each node's EXACT live options from AliceStyle.dt!
    let mut global_keep: Vec<bool> = Vec::with_capacity(compiled_total_forced);
    let mut baseline_gt_lines_all: Vec<Vec<String>> = Vec::with_capacity(paragraphs.len());

    for p in &paragraphs {
        baseline_gt_lines_all.push(p.gt_lines.clone());
        let n_breaks = p.gt_break_positions.len();
        let mut keep_local = vec![false; n_breaks];

        let mut cur_text = p.unforced_text.clone();
        let mut cur_spans = p.unforced_spans.clone();
        let mut cur_boxes = p.unforced_boxes.clone();
        let mut break_positions = p.gt_break_positions.clone();

        for _step in 0..=n_breaks {
            let cand_lines = shape_candidate_lines(
                p,
                &cur_text,
                &cur_spans,
                &cur_boxes,
                &p.node_opts,
                &mut font_cx,
                &mut layout_cx,
            );
            if cand_lines == p.gt_lines {
                break;
            }
            let mismatch_idx = (0..p.gt_lines.len().saturating_sub(1))
                .find(|&k| cand_lines.get(k) != p.gt_lines.get(k));
            let Some(k) = mismatch_idx else {
                break;
            };
            if k >= break_positions.len() || keep_local[k] {
                // Fallback: force all remaining breaks in this paragraph if stuck
                for j in 0..n_breaks {
                    keep_local[j] = true;
                }
                break;
            }
            keep_local[k] = true;
            let (pos, is_space) = break_positions[k];
            let is_shy_break = !is_space && cur_text[pos..].starts_with('\u{00AD}');
            let (old_len, repl_str) = if is_space {
                (1usize, FORCED_JUSTIFY_BREAK)
            } else if is_shy_break {
                ('\u{00AD}'.len_utf8(), "-\u{2060}\n")
            } else {
                (0usize, FORCED_JUSTIFY_BREAK)
            };
            let new_len = repl_str.len();
            cur_text.replace_range(pos..pos + old_len, repl_str);
            for sp in &mut cur_spans {
                let s = adjust_offset(sp.range.start, pos, old_len, new_len);
                let e = adjust_offset(sp.range.end, pos, old_len, new_len);
                sp.range = s..e.max(s);
            }
            for ib in &mut cur_boxes {
                ib.index = adjust_offset(ib.index, pos, old_len, new_len);
            }
            let delta = new_len as isize - old_len as isize;
            for bp in &mut break_positions[k + 1..] {
                bp.0 = (bp.0 as isize + delta) as usize;
            }
        }
        global_keep.extend(keep_local);
    }

    let kept_count = global_keep.iter().filter(|&&b| b).count();
    let removed_count = global_keep.len() - kept_count;
    println!(
        "\n=== Pruning Plan: keeping {}/{} \\| breaks, removing {} ({:.1}% removed) ===",
        kept_count,
        global_keep.len(),
        removed_count,
        100.0 * removed_count as f64 / global_keep.len().max(1) as f64
    );

    if std::env::var("APPLY_ALICE_PRUNE").ok().as_deref() == Some("1") {
        let mut cursor = 0usize;
        for ch_path in &chapter_files {
            let orig = std::fs::read_to_string(ch_path).unwrap();
            let mut out = String::with_capacity(orig.len());
            let mut i = 0usize;
            let bytes = orig.as_bytes();
            let mut ch_kept = 0usize;
            let mut ch_removed = 0usize;

            while i < bytes.len() {
                if i + 1 < bytes.len() && bytes[i] == b'\\' && bytes[i + 1] == b'|' {
                    let keep = global_keep[cursor];
                    cursor += 1;
                    if keep {
                        out.push_str("\\|");
                        i += 2;
                        ch_kept += 1;
                    } else {
                        ch_removed += 1;
                        // Determine whether `\|` immediately follows `\-`, `-`, or `—` (UTF-8 `\u{2014}`):
                        // if so, join across newline+indentation with zero space!
                        let join_no_space = out.ends_with("\\-")
                            || out.ends_with('-')
                            || out.ends_with('\u{2014}');
                        i += 2; // skip `\|`
                        if join_no_space {
                            // Consume optional spaces/tabs, single `\n`, and leading indentation on next line
                            while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\r') {
                                i += 1;
                            }
                            if i < bytes.len() && bytes[i] == b'\n' {
                                i += 1;
                                while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
                                    i += 1;
                                }
                            }
                        }
                    }
                } else {
                    let ch = orig[i..].chars().next().unwrap();
                    out.push(ch);
                    i += ch.len_utf8();
                }
            }
            std::fs::write(ch_path, &out).unwrap();
            println!(
                "  {}: kept {:3}, removed {:3}",
                ch_path.file_name().unwrap().to_string_lossy(),
                ch_kept,
                ch_removed
            );
        }
        assert_eq!(cursor, global_keep.len());

        // Re-compile & re-paginate Alice and assert 100% identical line breaks & 202 pages!
        let src2 = std::fs::read_to_string(alice_dir.join("alice.dt")).expect("read alice.dt");
        let doc2 = parse_document(&src2).expect("parse pruned alice.dt");
        let compiled2 =
            compile_document_with_resolver(&doc2, 800.0, 600.0, alice_dir, &FsResolver)
                .expect("compile pruned alice.dt");
        let paginated2 = paginate_compiled_document(&compiled2, 800.0, 600.0, 96.0).expect("paginate pruned alice.dt");
        println!(
            "\n=== Post-Prune Verification: {} total pages (pre-prune={}) ===",
            paginated2.pages.len(),
            pre_prune_page_count
        );
        assert_eq!(
            paginated2.pages.len(),
            pre_prune_page_count,
            "Pruned Alice must preserve exact page count!"
        );

        // Verify every single page has the exact same number of items & every text node matches line-by-line!
        let mut total_text_nodes_checked = 0usize;
        let mut mismatched_nodes = 0usize;
        let mut p1_lines_all: Vec<Vec<String>> = Vec::new();
        let mut p2_lines_all: Vec<Vec<String>> = Vec::new();

        for (layout_ref, out_vec) in [(&compiled.layout, &mut p1_lines_all), (&compiled2.layout, &mut p2_lines_all)] {
            for node in &layout_ref.nodes {
                if node.name != "Text" || node.rect.width <= 0.0 {
                    continue;
                }
                let (Some(txt), spans, boxes, _) = resolve_active_rich_text(node, layout_ref) else {
                    continue;
                };
                let opts = ParagraphLayoutOptions::from_node_properties(&node.properties);
                if !opts.is_optimal() {
                    continue;
                }
                let fnode = node
                    .font
                    .or_else(|| node.properties.get("font").and_then(|v| v.as_node()))
                    .and_then(|fid| layout_ref.get_node(fid));
                let prep = prepare_rich_text_for_shaping_with_options(
                    &mut font_cx,
                    &txt,
                    fnode.and_then(|f| f.properties.get("size")).and_then(|v| v.as_f64()).unwrap_or(16.0) as f32,
                    fnode.and_then(|f| f.properties.get("weight")).and_then(|v| v.as_f64()).unwrap_or(400.0) as f32,
                    fnode.and_then(|f| f.properties.get("family")).and_then(|v| v.as_str()),
                    fnode.and_then(|f| f.properties.get("italic")).and_then(|v| v.as_bool()).unwrap_or(false),
                    fnode.and_then(|f| f.properties.get("small_caps")).and_then(|v| v.as_bool()).unwrap_or(false),
                    fnode.and_then(|f| f.properties.get("letter_spacing")).and_then(|v| v.as_f64()).unwrap_or(0.0),
                    fnode.and_then(|f| f.properties.get("line_height")).and_then(|v| v.as_f64()).filter(|&h| h > 0.0),
                    Some(node.rect.width),
                    node.properties.get("align").and_then(|v| v.as_str()),
                    node.properties.get("indent").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    &spans,
                    &boxes,
                    &opts,
                );
                let eff = prep.as_ref().map(|p| p.text.as_str()).unwrap_or(&txt);
                out_vec.push(eff.split('\n').map(normalize_line_text).collect());
            }
        }
        assert_eq!(p1_lines_all.len(), p2_lines_all.len());
        for (idx, (l1, l2)) in p1_lines_all.iter().zip(p2_lines_all.iter()).enumerate() {
            total_text_nodes_checked += 1;
            if l1 != l2 {
                mismatched_nodes += 1;
                if mismatched_nodes <= 10 {
                    println!("MISMATCH on Text #{idx}:\n  BEFORE: {l1:?}\n  AFTER:  {l2:?}");
                }
            }
        }
        println!(
            "=== Verified {}/{} optimal Text nodes: {} mismatches ===",
            total_text_nodes_checked - mismatched_nodes,
            total_text_nodes_checked,
            mismatched_nodes
        );
        assert_eq!(mismatched_nodes, 0, "Every single paragraph must match 100% identically after pruning!");
    }
}

#[test]
#[ignore]
fn analyze_alice_255pt_word_spacing_tradeoff() {
    use std::collections::HashMap;
    use directedtype::compiler::FileResolver;

    struct HeadChaptersResolver {
        overrides: HashMap<String, String>,
    }
    impl FileResolver for HeadChaptersResolver {
        fn read(&self, path: &Path) -> Result<String, String> {
            if let Some(fname) = path.file_name().and_then(|s| s.to_str()) {
                if let Some(content) = self.overrides.get(fname) {
                    return Ok(content.clone());
                }
            }
            FsResolver.read(path)
        }
    }

    let mut overrides = HashMap::new();
    for i in 1..=12 {
        let fname = format!("ch{:02}.dt", i);
        let out = std::process::Command::new("git")
            .args(["show", &format!("HEAD:examples/alice/{fname}")])
            .output()
            .expect("git show HEAD chXX.dt");
        assert!(out.status.success(), "git show failed for {fname}");
        overrides.insert(fname, String::from_utf8(out.stdout).expect("utf8"));
    }

    let alice_dir = Path::new("examples/alice");
    register_local_fonts_for_base_dir(alice_dir);
    let src = std::fs::read_to_string(alice_dir.join("alice.dt")).expect("read alice.dt");
    let doc = parse_document(&src).expect("parse alice.dt");
    let resolver = HeadChaptersResolver { overrides };
    let compiled = compile_document_with_resolver(&doc, 800.0, 600.0, alice_dir, &resolver)
        .expect("compile alice.dt with HEAD chapters");

    let pt_to_px = 96.0 / 72.0;
    let nat_space_pt = 3.36_f64;

    // Collect ALL body optimal Text nodes (including 1-line paragraphs with 0 forced breaks!)
    struct AnyPara {
        text_with_gt: String,
        gt_line_count: usize,
        width: f64,
        indent: f64,
        align: String,
        font_size: f64,
        font_weight: f64,
        font_family: Option<String>,
        font_italic: bool,
        font_small_caps: bool,
        letter_spacing: f64,
        line_height: Option<f64>,
        spans: Vec<TextSpan>,
        boxes: Vec<InlineBoxSpan>,
    }

    let mut all_body_paras: Vec<AnyPara> = Vec::new();
    let mut explicit_hyphen_pairs: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for s in resolver.overrides.values() {
        let mut search_from = 0usize;
        while let Some(rel) = s[search_from..].find("-\\|") {
            let idx = search_from + rel;
            search_from = idx + 3;
            if idx > 0 && s.as_bytes()[idx - 1] == b'\\' {
                continue;
            }
            let pre_tail: String = s[..idx + 1]
                .chars()
                .rev()
                .take(6)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let after = s[idx + 3..].trim_start();
            let suf_head: String = after.chars().take(6).collect();
            explicit_hyphen_pairs.insert((pre_tail, suf_head));
        }
    }

    let mut multi_paras: Vec<AliceParagraph> = Vec::new();
    let mut font_cx = directedtype::compiler::text::create_font_context();
    let mut layout_cx = parley::LayoutContext::<()>::new();

    for node in &compiled.layout.nodes {
        if node.name != "Text" || node.rect.width <= 0.0 {
            continue;
        }
        let opts = ParagraphLayoutOptions::from_node_properties(&node.properties);
        if !opts.is_optimal() {
            continue;
        }
        let (Some(active_text), resolved_spans, resolved_boxes, _) =
            resolve_active_rich_text(node, &compiled.layout)
        else {
            continue;
        };
        let fnode = node
            .font
            .or_else(|| node.properties.get("font").and_then(|v| v.as_node()))
            .and_then(|fid| compiled.layout.get_node(fid));
        let font_size = fnode
            .and_then(|f| f.properties.get("size"))
            .and_then(|v| v.as_f64())
            .unwrap_or(16.0);
        let font_weight = fnode
            .and_then(|f| f.properties.get("weight"))
            .and_then(|v| v.as_f64())
            .unwrap_or(400.0);
        let font_family = fnode
            .and_then(|f| f.properties.get("family"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let font_italic = fnode
            .and_then(|f| f.properties.get("italic"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let font_small_caps = fnode
            .and_then(|f| f.properties.get("small_caps"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let letter_spacing = fnode
            .and_then(|f| f.properties.get("letter_spacing"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let line_height = fnode
            .filter(|f| f.formulas.has_custom_line_height())
            .and_then(|f| f.properties.get("line_height"))
            .and_then(|v| v.as_f64())
            .filter(|&h| h > 0.0);
        let align = node
            .properties
            .get("align")
            .and_then(|v| v.as_str())
            .unwrap_or("Justify")
            .to_string();
        let indent = node
            .properties
            .get("indent")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        // Full-measure 255pt body paragraphs only (340px)
        let is_full_measure = (node.rect.width - 255.0 * pt_to_px).abs() < 1.0;
        let gt_line_count = active_text.matches(FORCED_JUSTIFY_BREAK).count() + 1;

        if is_full_measure {
            all_body_paras.push(AnyPara {
                text_with_gt: active_text.clone(),
                gt_line_count,
                width: node.rect.width,
                indent,
                align: align.clone(),
                font_size,
                font_weight,
                font_family: font_family.clone(),
                font_italic,
                font_small_caps,
                letter_spacing,
                line_height,
                spans: resolved_spans.clone(),
                boxes: resolved_boxes.clone(),
            });
        }

        if !active_text.contains(FORCED_JUSTIFY_BREAK) || !is_full_measure {
            continue;
        }
        // Build ground-truth lines from the explicit `\|` breaks in HEAD!
        let gt_lines: Vec<String> = active_text
            .split('\n')
            .map(normalize_line_text)
            .collect();

        let mut unforced_text = active_text.clone();
        let mut unforced_spans = resolved_spans.clone();
        let mut unforced_boxes = resolved_boxes.clone();
        let mut gt_break_positions: Vec<(usize, bool)> = Vec::new();
        let fb_len = FORCED_JUSTIFY_BREAK.len();
        while let Some(pos) = unforced_text.find(FORCED_JUSTIFY_BREAK) {
            let prev_ch = unforced_text[..pos].chars().next_back();
            let is_em_dash = prev_ch == Some('\u{2014}');
            let is_hyphen = prev_ch == Some('-');
            let is_explicit_compound_hyphen = is_hyphen && {
                let pre = &unforced_text[..pos];
                let suf = &unforced_text[pos + fb_len..];
                let pre_tail: String = pre.chars().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect();
                let suf_head: String = suf.chars().take(6).collect();
                explicit_hyphen_pairs.contains(&(pre_tail, suf_head))
            };
            if is_hyphen && !is_explicit_compound_hyphen {
                let replace_start = pos - 1;
                let old_len = 1 + fb_len;
                let repl = "\u{00AD}";
                let new_len = repl.len();
                unforced_text.replace_range(replace_start..replace_start + old_len, repl);
                for sp in &mut unforced_spans {
                    let s = adjust_offset(sp.range.start, replace_start, old_len, new_len);
                    let e = adjust_offset(sp.range.end, replace_start, old_len, new_len);
                    sp.range = s..e.max(s);
                }
                for ib in &mut unforced_boxes {
                    ib.index = adjust_offset(ib.index, replace_start, old_len, new_len);
                }
                gt_break_positions.push((replace_start, false));
            } else {
                let join_directly = is_em_dash || is_explicit_compound_hyphen;
                let repl = if join_directly { "" } else { " " };
                let new_len = repl.len();
                unforced_text.replace_range(pos..pos + fb_len, repl);
                for sp in &mut unforced_spans {
                    let s = adjust_offset(sp.range.start, pos, fb_len, new_len);
                    let e = adjust_offset(sp.range.end, pos, fb_len, new_len);
                    sp.range = s..e.max(s);
                }
                for ib in &mut unforced_boxes {
                    ib.index = adjust_offset(ib.index, pos, fb_len, new_len);
                }
                gt_break_positions.push((pos, !join_directly));
            }
        }
        multi_paras.push(AliceParagraph {
            text_with_gt: active_text,
            unforced_text,
            unforced_spans,
            unforced_boxes,
            gt_lines,
            gt_break_positions,
            has_soft_hyphen: false,
            width: node.rect.width,
            indent,
            align,
            font_size,
            font_weight,
            font_family,
            font_italic,
            font_small_caps,
            letter_spacing,
            line_height,
            node_opts: ParagraphLayoutOptions::default(),
        });
    }

    // 1. Diagnose the exact `"door."` line AND every final/1-line paragraph > 245pt at ws=0pt!
    println!("\n=== All Final/Single-Line Paragraphs > 245pt at Natural Space (ws=0pt, 3.36pt) ===");
    let mut final_lines_wide: Vec<(f64, f64, usize, bool, String)> = Vec::new();
    for p in &all_body_paras {
        // Skip End Matter paragraphs (they don't use 1866 `\|` ground truth)
        if p.text_with_gt.contains("line-break adjustments were made") || p.text_with_gt.contains("removed stray opening") {
            continue;
        }
        let last_seg = p.text_with_gt.split('\n').last().unwrap_or("");
        let line_str = normalize_line_text(last_seg);
        let is_single_line = p.gt_line_count == 1;
        let mut b = layout_cx.ranged_builder(&mut font_cx, &line_str, 1.0, true);
        b.push_default(parley::style::StyleProperty::FontSize(p.font_size as f32));
        if let Some(ref fam) = p.font_family {
            b.push_default(parley::style::StyleProperty::FontFamily(parley::style::FontFamily::from(fam.as_str())));
        }
        if is_single_line && p.indent > 0.0 {
            directedtype::compiler::text::push_first_line_indent_box(&mut b, p.indent, true);
        }
        let mut l = b.build(&line_str);
        l.break_all_lines(None);
        let w_pt = l.full_width() as f64 / pt_to_px;
        let n_sp = line_str.chars().filter(|&c| c == ' ' || c == '\u{00A0}').count();
        let req_sp_255 = if n_sp > 0 { nat_space_pt + (255.0 - w_pt) / (n_sp as f64) } else { 0.0 };
        if w_pt > 245.0 {
            final_lines_wide.push((w_pt, req_sp_255, n_sp, is_single_line, line_str));
        }
    }
    final_lines_wide.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (w, req_sp, n_sp, is_1l, txt) in &final_lines_wide {
        let pct = 100.0 * req_sp / nat_space_pt;
        let tag = if *is_1l { "1-line (+16pt indent)" } else { "last-line            " };
        println!("  nat_w={:6.2}pt | at 255pt sp={:5.2}pt ({:5.1}% nat) | n_sp={:2} | {} | {}", w, req_sp, pct, n_sp, tag, txt);
    }

    // 2. Sweep word_spacing at 255pt (with min_sp pinned near 2.24pt AND with standard 33% shrink!)
    println!(
        "\n=== 255pt Word-Spacing Trade-off Across {} Full-Measure Body Paragraphs ({} multi-line, {} GT breaks) ===",
        all_body_paras.len(),
        multi_paras.len(),
        multi_paras.iter().map(|p| p.gt_break_positions.len()).sum::<usize>(),
    );
    println!("(A) Shrink floor pinned to ~2.24pt (33% below natural 3.36pt space):");
    for &ws_pt in &[0.0, 0.25, 0.50, 0.75, 1.00, 1.25, 1.50, 1.75, 2.00, 2.25, 2.50, 2.75] {
        let nom_sp = nat_space_pt + ws_pt;
        // Choose space_shrink so nom_sp * (1 - sh) == 2.24pt (capped at >= 0.33)
        let sh = (1.0 - 2.24 / nom_sp).max(0.33);
        let opts = ParagraphLayoutOptions {
            line_break: LineBreakMode::Optimal,
            word_spacing: ws_pt * pt_to_px,
            space_stretch: 0.50,
            space_shrink: sh,
            sentence_space: 0.50,
            hyphen_penalty: 20.0,
            ..Default::default()
        };

        // Check how many paragraphs (keeping ALL 1866 `\|` breaks!) STILL spill an extra line on a final/1-line segment!
        let mut spills: Vec<String> = Vec::new();
        for p in &all_body_paras {
            let prep = prepare_rich_text_for_shaping_with_options(
                &mut font_cx,
                &p.text_with_gt,
                p.font_size as f32,
                p.font_weight as f32,
                p.font_family.as_deref(),
                p.font_italic,
                p.font_small_caps,
                p.letter_spacing,
                p.line_height,
                Some(p.width),
                Some(&p.align),
                p.indent,
                &p.spans,
                &p.boxes,
                &opts,
            );
            let eff = prep.as_ref().map(|pr| pr.text.as_str()).unwrap_or(&p.text_with_gt);
            let actual_lines = eff.split('\n').count();
            if actual_lines != p.gt_line_count {
                let last_two: Vec<&str> = eff.split('\n').rev().take(2).collect();
                spills.push(format!(
                    "{}->{} ({:?} / {:?})",
                    p.gt_line_count,
                    actual_lines,
                    normalize_line_text(last_two.get(1).copied().unwrap_or("")),
                    normalize_line_text(last_two.first().copied().unwrap_or(""))
                ));
            }
        }

        let sum = evaluate_config(&multi_paras, &opts, &mut font_cx, &mut layout_cx);
        let (res, _) = compute_minimal_residual_breaks(&multi_paras, &opts, &mut font_cx, &mut layout_cx);
        println!(
            "  ws={:+.2}pt (nom={:.2}pt, sh={:.2}, min={:.2}pt) | spills={:2} | breaks {:5.1}% exact {:5.1}% | res\\|={:4} ({:4.1}% rem)",
            ws_pt,
            nom_sp,
            sh,
            nom_sp * (1.0 - sh),
            spills.len(),
            100.0 * sum.matched_breaks as f64 / sum.total_gt_breaks as f64,
            100.0 * sum.exact_paras as f64 / sum.total_paras as f64,
            res,
            100.0 * (1.0 - res as f64 / sum.total_gt_breaks as f64),
        );
        if !spills.is_empty() && spills.len() <= 10 && matches!(ws_pt as i32, 0 | 1 | 2) || (ws_pt - 1.25).abs() < 0.01 || (ws_pt - 1.50).abs() < 0.01 || (ws_pt - 0.75).abs() < 0.01 {
            for sp in &spills {
                println!("      SPILL: {sp}");
            }
        }
    }

    println!("\n(B) Standard TeX 33% shrink (space_shrink = 0.33, space_stretch = 0.50):");
    for &ws_pt in &[0.0, 0.25, 0.50, 0.75, 1.00, 1.25, 1.50, 1.75, 2.00] {
        let nom_sp = nat_space_pt + ws_pt;
        let opts = ParagraphLayoutOptions {
            line_break: LineBreakMode::Optimal,
            word_spacing: ws_pt * pt_to_px,
            space_stretch: 0.50,
            space_shrink: 0.33,
            sentence_space: 0.50,
            hyphen_penalty: 20.0,
            ..Default::default()
        };
        let mut spills = 0usize;
        for p in &all_body_paras {
            let prep = prepare_rich_text_for_shaping_with_options(
                &mut font_cx,
                &p.text_with_gt,
                p.font_size as f32,
                p.font_weight as f32,
                p.font_family.as_deref(),
                p.font_italic,
                p.font_small_caps,
                p.letter_spacing,
                p.line_height,
                Some(p.width),
                Some(&p.align),
                p.indent,
                &p.spans,
                &p.boxes,
                &opts,
            );
            let eff = prep.as_ref().map(|pr| pr.text.as_str()).unwrap_or(&p.text_with_gt);
            if eff.split('\n').count() != p.gt_line_count {
                spills += 1;
            }
        }
        let sum = evaluate_config(&multi_paras, &opts, &mut font_cx, &mut layout_cx);
        let (res, _) = compute_minimal_residual_breaks(&multi_paras, &opts, &mut font_cx, &mut layout_cx);
        println!(
            "  ws={:+.2}pt (nom={:.2}pt, sh=0.33, min={:.2}pt) | spills={:2} | breaks {:5.1}% exact {:5.1}% | res\\|={:4} ({:4.1}% rem)",
            ws_pt,
            nom_sp,
            nom_sp * 0.67,
            spills,
            100.0 * sum.matched_breaks as f64 / sum.total_gt_breaks as f64,
            100.0 * sum.exact_paras as f64 / sum.total_paras as f64,
            res,
            100.0 * (1.0 - res as f64 / sum.total_gt_breaks as f64),
        );
    }
}

#[test]
#[ignore]
fn inspect_9_spills_at_255pt_and_test_looseness() {
    use std::collections::HashMap;
    use directedtype::compiler::FileResolver;

    // Compile CURRENT working-tree alice.dt (which has the 4 errata, wrap fixes, and 255pt ws=2.75pt)
    // AND compare line counts against HEAD ch01..ch12!
    struct HeadChaptersResolver {
        overrides: HashMap<String, String>,
    }
    impl FileResolver for HeadChaptersResolver {
        fn read(&self, path: &Path) -> Result<String, String> {
            if let Some(fname) = path.file_name().and_then(|s| s.to_str()) {
                if let Some(content) = self.overrides.get(fname) {
                    return Ok(content.clone());
                }
            }
            FsResolver.read(path)
        }
    }

    let mut overrides = HashMap::new();
    for fname in [
        "alice.dt", "AliceStyle.dt", "front_matter.dt",
        "ch01.dt", "ch02.dt", "ch03.dt", "ch04.dt", "ch05.dt", "ch06.dt",
        "ch07.dt", "ch08.dt", "ch09.dt", "ch10.dt", "ch11.dt", "ch12.dt",
    ] {
        let out = std::process::Command::new("git")
            .args(["show", &format!("4f92eee:examples/alice/{fname}")])
            .output()
            .expect("git show 4f92eee");
        assert!(out.status.success(), "git show 4f92eee failed for {fname}");
        overrides.insert(fname.to_string(), String::from_utf8(out.stdout).expect("utf8"));
    }

    let alice_dir = Path::new("examples/alice");
    register_local_fonts_for_base_dir(alice_dir);
    let src_cur = std::fs::read_to_string(alice_dir.join("alice.dt")).expect("read alice.dt");
    let doc_cur = parse_document(&src_cur).expect("parse alice.dt");
    let src_gt = overrides.get("alice.dt").unwrap().clone();
    let doc_gt = parse_document(&src_gt).expect("parse 4f92eee alice.dt");
    let compiled_cur = compile_document_with_resolver(&doc_cur, 800.0, 600.0, alice_dir, &FsResolver).unwrap();
    let compiled_head = compile_document_with_resolver(&doc_gt, 800.0, 600.0, alice_dir, &HeadChaptersResolver { overrides }).unwrap();

    let pt_to_px = 96.0 / 72.0;
    let mut font_cx = directedtype::compiler::text::create_font_context();
    let mut layout_cx = parley::LayoutContext::<()>::new();

    // Collect prose Text nodes (Justify/JustifyAll) from both, excluding front/end matter
    let collect_texts = |layout: &directedtype::compiler::ResolvedLayout, only_optimal: bool| {
        let mut v = Vec::new();
        for node in &layout.nodes {
            if node.name != "Text" || node.rect.width <= 0.0 {
                continue;
            }
            let opts = ParagraphLayoutOptions::from_node_properties(&node.properties);
            let align = node.properties.get("align").and_then(|v| v.as_str()).unwrap_or("Left").to_string();
            if align != "Justify" && align != "JustifyAll" {
                continue;
            }
            if only_optimal && !opts.is_optimal() {
                continue;
            }
            let (Some(txt), spans, boxes, _) = resolve_active_rich_text(node, layout) else {
                continue;
            };
            if txt.contains("Old Standard TT") || txt.contains("generous word and sentence") || txt.contains("Knuth") || txt.contains("minor typographical") || txt.contains("digitally typeset") {
                continue;
            }
            let fnode = node.font
                .or_else(|| node.properties.get("font").and_then(|v| v.as_node()))
                .and_then(|fid| layout.get_node(fid));
            let font_size = fnode.and_then(|f| f.properties.get("size")).and_then(|v| v.as_f64()).unwrap_or(16.0);
            let font_weight = fnode.and_then(|f| f.properties.get("weight")).and_then(|v| v.as_f64()).unwrap_or(400.0);
            let font_family = fnode.and_then(|f| f.properties.get("family")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let font_italic = fnode.and_then(|f| f.properties.get("italic")).and_then(|v| v.as_bool()).unwrap_or(false);
            let font_small_caps = fnode.and_then(|f| f.properties.get("small_caps")).and_then(|v| v.as_bool()).unwrap_or(false);
            let letter_spacing = fnode.and_then(|f| f.properties.get("letter_spacing")).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let line_height = fnode.filter(|f| f.formulas.has_custom_line_height()).and_then(|f| f.properties.get("line_height")).and_then(|v| v.as_f64()).filter(|&h| h > 0.0);
            let indent = node.properties.get("indent").and_then(|v| v.as_f64()).unwrap_or(0.0);
            v.push((txt, spans, boxes, node.rect.width, indent, align, font_size, font_weight, font_family, font_italic, font_small_caps, letter_spacing, line_height, opts));
        }
        v
    };

    let cur_nodes = collect_texts(&compiled_cur.layout, true);
    let head_nodes = collect_texts(&compiled_head.layout, false);
    assert_eq!(cur_nodes.len(), head_nodes.len(), "prose node count mismatch");

    println!("\n=== Exact Line-by-Line Diff vs 1866 Ground Truth (4f92eee) Across {} Prose Paragraphs ===", cur_nodes.len());
    let mut spill_idx = 0usize;
    for (idx, (cur, head)) in cur_nodes.iter().zip(head_nodes.iter()).enumerate() {
        let head_gt_lines: Vec<String> = head.0.split('\n').map(normalize_line_text).collect();
        let prep = prepare_rich_text_for_shaping_with_options(
            &mut font_cx, &cur.0, cur.6 as f32, cur.7 as f32, cur.8.as_deref(), cur.9, cur.10, cur.11, cur.12,
            Some(cur.3), Some(&cur.5), cur.4, &cur.1, &cur.2, &cur.13,
        );
        let cur_lines: Vec<String> = prep.as_ref().map(|p| p.text.as_str()).unwrap_or(&cur.0).split('\n').map(normalize_line_text).collect();
        if cur_lines != head_gt_lines {
            spill_idx += 1;
            println!(
                "\nSPILL #{spill_idx} (node #{idx}, w={:.1}pt, indent={:.1}pt, align={}, gt_lines={} -> got={}):",
                cur.3 / pt_to_px, cur.4 / pt_to_px, cur.5, head_gt_lines.len(), cur_lines.len()
            );
            println!("  1866 GT lines:");
            for (i, l) in head_gt_lines.iter().enumerate() {
                println!("    GT[{i}]: {l}");
            }
            println!("  Current KP (ws=+2.75pt) lines:");
            for (i, l) in cur_lines.iter().enumerate() {
                println!("    KP[{i}]: {l}");
            }

            // Test 1: looseness = -1 (with existing opts)
            let mut opts_loose = cur.13.clone();
            opts_loose.looseness = -1;
            let p_loose = prepare_rich_text_for_shaping_with_options(
                &mut font_cx, &cur.0, cur.6 as f32, cur.7 as f32, cur.8.as_deref(), cur.9, cur.10, cur.11, cur.12,
                Some(cur.3), Some(&cur.5), cur.4, &cur.1, &cur.2, &opts_loose,
            );
            let l_loose: Vec<String> = p_loose.as_ref().map(|p| p.text.as_str()).unwrap_or(&cur.0).split('\n').map(normalize_line_text).collect();
            println!("  Test A (looseness=-1, same ws=+2.75pt, sh=0.63): {} lines (matches GT? {})", l_loose.len(), l_loose == head_gt_lines);
            if l_loose != head_gt_lines {
                for (i, l) in l_loose.iter().enumerate() {
                    println!("    A[{i}]: {l}");
                }
            }

            // Test 2: word_spacing = 0pt (keeping sh=0.45 so even 1.97pt fits)
            let mut opts_ws0 = cur.13.clone();
            opts_ws0.word_spacing = 0.0;
            opts_ws0.space_shrink = 0.45;
            let p_ws0 = prepare_rich_text_for_shaping_with_options(
                &mut font_cx, &cur.0, cur.6 as f32, cur.7 as f32, cur.8.as_deref(), cur.9, cur.10, cur.11, cur.12,
                Some(cur.3), Some(&cur.5), cur.4, &cur.1, &cur.2, &opts_ws0,
            );
            let l_ws0: Vec<String> = p_ws0.as_ref().map(|p| p.text.as_str()).unwrap_or(&cur.0).split('\n').map(normalize_line_text).collect();
            println!("  Test B (ws=0pt, sh=0.45 on whole para): {} lines (matches GT? {})", l_ws0.len(), l_ws0 == head_gt_lines);
            if l_ws0 != head_gt_lines {
                for (i, l) in l_ws0.iter().enumerate() {
                    println!("    B[{i}]: {l}");
                }
            }

            // Test 3: looseness=-1 AND sh=0.68 (min space 1.95pt)
            let mut opts_l68 = cur.13.clone();
            opts_l68.looseness = -1;
            opts_l68.space_shrink = 0.68;
            let p_l68 = prepare_rich_text_for_shaping_with_options(
                &mut font_cx, &cur.0, cur.6 as f32, cur.7 as f32, cur.8.as_deref(), cur.9, cur.10, cur.11, cur.12,
                Some(cur.3), Some(&cur.5), cur.4, &cur.1, &cur.2, &opts_l68,
            );
            let l_l68: Vec<String> = p_l68.as_ref().map(|p| p.text.as_str()).unwrap_or(&cur.0).split('\n').map(normalize_line_text).collect();
            println!("  Test C (looseness=-1, sh=0.68): {} lines (matches GT? {})", l_l68.len(), l_l68 == head_gt_lines);
            if l_l68 != head_gt_lines {
                for (i, l) in l_l68.iter().enumerate() {
                    println!("    C[{i}]: {l}");
                }
            }

            // Measure natural width & required rendered space of the last GT line
            let last_gt = head_gt_lines.last().unwrap();
            let mut b = layout_cx.ranged_builder(&mut font_cx, last_gt, 1.0, true);
            b.push_default(parley::style::StyleProperty::FontSize(cur.6 as f32));
            if let Some(ref fam) = cur.8 {
                b.push_default(parley::style::StyleProperty::FontFamily(parley::style::FontFamily::from(fam.as_str())));
            }
            if head_gt_lines.len() == 1 && cur.4 > 0.0 {
                directedtype::compiler::text::push_first_line_indent_box(&mut b, cur.4, true);
            }
            let mut l = b.build(last_gt);
            l.break_all_lines(None);
            let w_nat = l.full_width() as f64 / pt_to_px;
            let n_sp = last_gt.chars().filter(|&c| c == ' ' || c == '\u{00A0}').count();
            let target_pt = cur.3 / pt_to_px;
            let req_sp = 3.36 + (target_pt - w_nat) / (n_sp.max(1) as f64);
            println!("  Last GT line metrics: nat_w={w_nat:.2}pt vs target={target_pt:.1}pt | n_sp={n_sp} | rendered_sp_if_1line={req_sp:.2}pt ({:.1}% of 3.36pt)", 100.0 * req_sp / 3.36);
        }
    }
    println!("\nTotal spills found: {spill_idx}");
}

#[test]
#[ignore]
fn compare_word_spacing_percentiles_255_262_270() {
    use std::collections::HashMap;
    use directedtype::compiler::FileResolver;

    struct HeadChaptersResolver {
        overrides: HashMap<String, String>,
    }
    impl FileResolver for HeadChaptersResolver {
        fn read(&self, path: &Path) -> Result<String, String> {
            if let Some(fname) = path.file_name().and_then(|s| s.to_str()) {
                if let Some(content) = self.overrides.get(fname) {
                    return Ok(content.clone());
                }
            }
            FsResolver.read(path)
        }
    }

    let mut overrides = HashMap::new();
    for i in 1..=12 {
        let fname = format!("ch{:02}.dt", i);
        let out = std::process::Command::new("git")
            .args(["show", &format!("HEAD:examples/alice/{fname}")])
            .output()
            .expect("git show HEAD chXX.dt");
        overrides.insert(fname, String::from_utf8(out.stdout).expect("utf8"));
    }

    let alice_dir = Path::new("examples/alice");
    register_local_fonts_for_base_dir(alice_dir);
    let src = std::fs::read_to_string(alice_dir.join("alice.dt")).expect("read alice.dt");
    let doc = parse_document(&src).expect("parse alice.dt");
    let compiled = compile_document_with_resolver(&doc, 800.0, 600.0, alice_dir, &HeadChaptersResolver { overrides }).unwrap();

    let pt_to_px = 96.0 / 72.0;
    let nat_sp_pt = 3.36_f64;
    let mut font_cx = directedtype::compiler::text::create_font_context();
    let mut layout_cx = parley::LayoutContext::<()>::new();

    // Measure every full-measure justified line (non-final lines of multi-line paras, plus JustifyAll final lines)
    // AND also record natural widths of ALL full-width lines (including final/1-line >= 245pt).
    let mut just_lines: Vec<(f64, usize)> = Vec::new(); // (nat_w_pt, n_spaces)
    for node in &compiled.layout.nodes {
        if node.name != "Text" || (node.rect.width - 255.0 * pt_to_px).abs() > 1.0 {
            continue;
        }
        let (Some(txt), _, _, _) = resolve_active_rich_text(node, &compiled.layout) else {
            continue;
        };
        if txt.contains("line-break adjustments") || txt.contains("removed stray opening") {
            continue;
        }
        let fnode = node.font
            .or_else(|| node.properties.get("font").and_then(|v| v.as_node()))
            .and_then(|fid| compiled.layout.get_node(fid));
        let font_size = fnode.and_then(|f| f.properties.get("size")).and_then(|v| v.as_f64()).unwrap_or(16.0);
        let font_family = fnode.and_then(|f| f.properties.get("family")).and_then(|v| v.as_str());
        let font_italic = fnode.and_then(|f| f.properties.get("italic")).and_then(|v| v.as_bool()).unwrap_or(false);
        let align = node.properties.get("align").and_then(|v| v.as_str()).unwrap_or("Justify");
        let indent = node.properties.get("indent").and_then(|v| v.as_f64()).unwrap_or(0.0);

        let lines: Vec<String> = txt.split('\n').map(normalize_line_text).collect();
        let n_lines = lines.len();
        for (li, line_str) in lines.iter().enumerate() {
            let is_justified = (li + 1 < n_lines) || align == "JustifyAll";
            if !is_justified {
                continue;
            }
            let n_sp = line_str.chars().filter(|&c| c == ' ' || c == '\u{00A0}').count();
            if n_sp == 0 {
                continue;
            }
            let mut b = layout_cx.ranged_builder(&mut font_cx, line_str, 1.0, true);
            b.push_default(parley::style::StyleProperty::FontSize(font_size as f32));
            if let Some(fam) = font_family {
                b.push_default(parley::style::StyleProperty::FontFamily(parley::style::FontFamily::from(fam)));
            }
            if font_italic {
                b.push_default(parley::style::StyleProperty::FontStyle(parley::style::FontStyle::Italic));
            }
            if li == 0 && indent > 0.0 {
                directedtype::compiler::text::push_first_line_indent_box(&mut b, indent, true);
            }
            let mut l = b.build(line_str);
            l.break_all_lines(None);
            let w_pt = l.full_width() as f64 / pt_to_px;
            just_lines.push((w_pt, n_sp));
        }
    }

    let pct = |sorted: &[f64], p: f64| -> f64 {
        let idx = ((sorted.len() - 1) as f64 * p / 100.0).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
    };

    let mut widths: Vec<f64> = just_lines.iter().map(|&(w, _)| w).collect();
    widths.sort_by(|a, b| a.total_cmp(b));
    let mean_w = widths.iter().sum::<f64>() / widths.len() as f64;
    println!("\n=== Natural Line Widths in 12pt Old Standard TT ({} full justified lines) ===", widths.len());
    println!(
        "  min={:.2} p01={:.2} p05={:.2} p10={:.2} p50={:.2} mean={:.2} p90={:.2} p95={:.2} p99={:.2} max={:.2}",
        widths[0], pct(&widths, 1.0), pct(&widths, 5.0), pct(&widths, 10.0),
        pct(&widths, 50.0), mean_w, pct(&widths, 90.0), pct(&widths, 95.0), pct(&widths, 99.0), *widths.last().unwrap()
    );

    println!("\n=== Rendered Justified Word-Space Advance (and est. ink-to-ink gap = +0.81pt) ===");
    for &m in &[255.0_f64, 262.0, 266.0, 270.0] {
        let mut sps: Vec<f64> = just_lines
            .iter()
            .map(|&(w, n_sp)| nat_sp_pt + (m - w) / (n_sp as f64))
            .collect();
        sps.sort_by(|a, b| a.total_cmp(b));
        let mean_sp = sps.iter().sum::<f64>() / sps.len() as f64;
        let below_nat = sps.iter().filter(|&&s| s < nat_sp_pt).count();
        let below_224 = sps.iter().filter(|&&s| s < 2.24).count();
        println!(
            "  M={:.0}pt space adv: min={:5.2} p01={:5.2} p05={:5.2} p10={:5.2} p50={:5.2} mean={:5.2} p90={:5.2} p95={:5.2} p99={:5.2} max={:5.2} | <3.36pt: {:3} ({:4.1}%), <2.24pt: {:2}",
            m, sps[0], pct(&sps, 1.0), pct(&sps, 5.0), pct(&sps, 10.0),
            pct(&sps, 50.0), mean_sp, pct(&sps, 90.0), pct(&sps, 95.0), pct(&sps, 99.0), *sps.last().unwrap(),
            below_nat, 100.0 * below_nat as f64 / sps.len() as f64, below_224
        );
        println!(
            "  M={:.0}pt ink gap  : min={:5.2} p01={:5.2} p05={:5.2} p10={:5.2} p50={:5.2} mean={:5.2} p90={:5.2} p95={:5.2} p99={:5.2} max={:5.2}",
            m, sps[0]+0.81, pct(&sps, 1.0)+0.81, pct(&sps, 5.0)+0.81, pct(&sps, 10.0)+0.81,
            pct(&sps, 50.0)+0.81, mean_sp+0.81, pct(&sps, 90.0)+0.81, pct(&sps, 95.0)+0.81, pct(&sps, 99.0)+0.81, *sps.last().unwrap()+0.81
        );
    }
}

#[test]
#[ignore]
fn analyze_alice_265pt_sweep() {
    use std::collections::HashMap;
    use directedtype::compiler::FileResolver;

    struct HeadChaptersResolver {
        overrides: HashMap<String, String>,
    }
    impl FileResolver for HeadChaptersResolver {
        fn read(&self, path: &Path) -> Result<String, String> {
            if let Some(fname) = path.file_name().and_then(|s| s.to_str()) {
                if let Some(content) = self.overrides.get(fname) {
                    return Ok(content.clone());
                }
            }
            FsResolver.read(path)
        }
    }

    let mut overrides = HashMap::new();
    for i in 1..=12 {
        let fname = format!("ch{:02}.dt", i);
        let out = std::process::Command::new("git")
            .args(["show", &format!("HEAD:examples/alice/{fname}")])
            .output()
            .expect("git show HEAD chXX.dt");
        overrides.insert(fname, String::from_utf8(out.stdout).expect("utf8"));
    }
    // Use HEAD AliceStyle.dt too, or current AliceStyle.dt with width replaced to 265pt!
    // Even cleaner: compile with HEAD AliceStyle (270pt), then scale full-measure nodes from 270pt -> 265pt
    // and narrow wrap nodes by -5pt (or check exact wrap widths)!
    let head_style = std::process::Command::new("git")
        .args(["show", "HEAD:examples/alice/AliceStyle.dt"])
        .output()
        .expect("git show HEAD AliceStyle.dt");
    overrides.insert("AliceStyle.dt".to_string(), String::from_utf8(head_style.stdout).expect("utf8"));
    let head_alice = std::process::Command::new("git")
        .args(["show", "HEAD:examples/alice/alice.dt"])
        .output()
        .expect("git show HEAD alice.dt");
    overrides.insert("alice.dt".to_string(), String::from_utf8(head_alice.stdout).expect("utf8"));

    let alice_dir = Path::new("examples/alice");
    register_local_fonts_for_base_dir(alice_dir);
    let src = overrides.get("alice.dt").unwrap().clone();
    let doc = parse_document(&src).expect("parse HEAD alice.dt");
    let resolver = HeadChaptersResolver { overrides };
    let compiled = compile_document_with_resolver(&doc, 800.0, 600.0, alice_dir, &resolver)
        .expect("compile HEAD alice.dt");

    let mut explicit_hyphen_pairs: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for (fname, s) in &resolver.overrides {
        if !fname.starts_with("ch") { continue; }
        let mut search_from = 0usize;
        while let Some(rel) = s[search_from..].find("-\\|") {
            let idx = search_from + rel;
            search_from = idx + 3;
            if idx > 0 && s.as_bytes()[idx - 1] == b'\\' { continue; }
            let pre_tail: String = s[..idx + 1].chars().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect();
            let suf_head: String = s[idx + 3..].trim_start().chars().take(6).collect();
            explicit_hyphen_pairs.insert((pre_tail, suf_head));
        }
    }

    let pt_to_px = 96.0 / 72.0;
    let target_m_pt = 265.0_f64;
    let target_m_px = target_m_pt * pt_to_px;
    let nat_sp_pt = 3.36_f64;

    let mut font_cx = directedtype::compiler::text::create_font_context();
    let mut layout_cx = parley::LayoutContext::<()>::new();

    let mut all_body_nodes: Vec<(String, Vec<TextSpan>, Vec<InlineBoxSpan>, f64, f64, String, f64, f64, Option<String>, bool, bool, f64, Option<f64>)> = Vec::new();
    let mut multi_paras: Vec<AliceParagraph> = Vec::new();

    for node in &compiled.layout.nodes {
        if node.name != "Text" || node.rect.width <= 0.0 { continue; }
        let align = node.properties.get("align").and_then(|v| v.as_str()).unwrap_or("Left").to_string();
        if align != "Justify" && align != "JustifyAll" { continue; }
        let (Some(active_text), spans, boxes, _) = resolve_active_rich_text(node, &compiled.layout) else { continue; };
        let fnode = node.font
            .or_else(|| node.properties.get("font").and_then(|v| v.as_node()))
            .and_then(|fid| compiled.layout.get_node(fid));
        let font_size = fnode.and_then(|f| f.properties.get("size")).and_then(|v| v.as_f64()).unwrap_or(16.0);
        let font_weight = fnode.and_then(|f| f.properties.get("weight")).and_then(|v| v.as_f64()).unwrap_or(400.0);
        let font_family = fnode.and_then(|f| f.properties.get("family")).and_then(|v| v.as_str()).map(|s| s.to_string());
        let font_italic = fnode.and_then(|f| f.properties.get("italic")).and_then(|v| v.as_bool()).unwrap_or(false);
        let font_small_caps = fnode.and_then(|f| f.properties.get("small_caps")).and_then(|v| v.as_bool()).unwrap_or(false);
        let letter_spacing = fnode.and_then(|f| f.properties.get("letter_spacing")).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let line_height = fnode.filter(|f| f.formulas.has_custom_line_height()).and_then(|f| f.properties.get("line_height")).and_then(|v| v.as_f64()).filter(|&h| h > 0.0);
        let indent = node.properties.get("indent").and_then(|v| v.as_f64()).unwrap_or(0.0);

        // Full-measure in HEAD is 270pt (360px) -> map to 265pt; narrow wraps keep -5pt (or same width if shifted)
        let is_full = (node.rect.width - 270.0 * pt_to_px).abs() < 1.0;
        let new_w = if is_full { target_m_px } else { node.rect.width - 5.0 * pt_to_px };

        if is_full {
            all_body_nodes.push((
                active_text.clone(), spans.clone(), boxes.clone(), new_w, indent, align.clone(),
                font_size, font_weight, font_family.clone(), font_italic, font_small_caps, letter_spacing, line_height,
            ));
        }

        if !active_text.contains(FORCED_JUSTIFY_BREAK) { continue; }
        let gt_lines: Vec<String> = active_text.split('\n').map(normalize_line_text).collect();

        let mut unforced_text = active_text.clone();
        let mut unforced_spans = spans.clone();
        let mut unforced_boxes = boxes.clone();
        let mut gt_break_positions: Vec<(usize, bool)> = Vec::new();
        let fb_len = FORCED_JUSTIFY_BREAK.len();
        while let Some(pos) = unforced_text.find(FORCED_JUSTIFY_BREAK) {
            let prev_ch = unforced_text[..pos].chars().next_back();
            let is_em_dash = prev_ch == Some('\u{2014}');
            let is_hyphen = prev_ch == Some('-');
            let is_explicit_compound_hyphen = is_hyphen && {
                let pre_tail: String = unforced_text[..pos].chars().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect();
                let suf_head: String = unforced_text[pos + fb_len..].chars().take(6).collect();
                explicit_hyphen_pairs.contains(&(pre_tail, suf_head))
            };
            if is_hyphen && !is_explicit_compound_hyphen {
                let replace_start = pos - 1;
                let old_len = 1 + fb_len;
                let repl = "\u{00AD}";
                unforced_text.replace_range(replace_start..replace_start + old_len, repl);
                for sp in &mut unforced_spans {
                    let s = adjust_offset(sp.range.start, replace_start, old_len, repl.len());
                    let e = adjust_offset(sp.range.end, replace_start, old_len, repl.len());
                    sp.range = s..e.max(s);
                }
                for ib in &mut unforced_boxes {
                    ib.index = adjust_offset(ib.index, replace_start, old_len, repl.len());
                }
                gt_break_positions.push((replace_start, false));
            } else {
                let join_directly = is_em_dash || is_explicit_compound_hyphen;
                let repl = if join_directly { "" } else { " " };
                unforced_text.replace_range(pos..pos + fb_len, repl);
                for sp in &mut unforced_spans {
                    let s = adjust_offset(sp.range.start, pos, fb_len, repl.len());
                    let e = adjust_offset(sp.range.end, pos, fb_len, repl.len());
                    sp.range = s..e.max(s);
                }
                for ib in &mut unforced_boxes {
                    ib.index = adjust_offset(ib.index, pos, fb_len, repl.len());
                }
                gt_break_positions.push((pos, !join_directly));
            }
        }
        multi_paras.push(AliceParagraph {
            text_with_gt: active_text,
            unforced_text,
            unforced_spans,
            unforced_boxes,
            gt_lines,
            gt_break_positions,
            has_soft_hyphen: false,
            width: new_w,
            indent,
            align,
            font_size,
            font_weight,
            font_family,
            font_italic,
            font_small_caps,
            letter_spacing,
            line_height,
            node_opts: ParagraphLayoutOptions::default(),
        });
    }

    // 1. Check ALL lines (body + final + 1-line!) wider than 265pt at natural ws=0pt
    println!("\n=== All Full-Measure Lines (Body + Final + 1-Line) > 265pt at Natural Space (3.36pt) ===");
    for (txt, _, _, _, indent, _, fs, _, fam, ital, _, _, _) in &all_body_nodes {
        let lines: Vec<String> = txt.split('\n').map(normalize_line_text).collect();
        for (li, lstr) in lines.iter().enumerate() {
            let mut b = layout_cx.ranged_builder(&mut font_cx, lstr, 1.0, true);
            b.push_default(parley::style::StyleProperty::FontSize(*fs as f32));
            if let Some(ref f) = fam {
                b.push_default(parley::style::StyleProperty::FontFamily(parley::style::FontFamily::from(f.as_str())));
            }
            if *ital {
                b.push_default(parley::style::StyleProperty::FontStyle(parley::style::FontStyle::Italic));
            }
            if li == 0 && *indent > 0.0 {
                directedtype::compiler::text::push_first_line_indent_box(&mut b, *indent, true);
            }
            let mut l = b.build(lstr);
            l.break_all_lines(None);
            let w_pt = l.full_width() as f64 / pt_to_px;
            if w_pt > 265.0 {
                let n_sp = lstr.chars().filter(|&c| c == ' ' || c == '\u{00A0}').count();
                let sp = nat_sp_pt + (265.0 - w_pt) / (n_sp.max(1) as f64);
                let kind = if lines.len() == 1 { "1-line" } else if li + 1 == lines.len() { "last  " } else { "body  " };
                println!("  [{kind}] nat_w={w_pt:6.2}pt | at 265pt sp={sp:5.2}pt ({:5.1}% nat) | n_sp={n_sp:2} | {lstr}", 100.0 * sp / nat_sp_pt);
            }
        }
    }

    // 2. Sweep word_spacing at 265pt across ALL 765 multi-line paras (2625 GT breaks) + check final/1-line spills!
    let total_gt: usize = multi_paras.iter().map(|p| p.gt_break_positions.len()).sum();
    println!("\n=== Knuth-Plass Sweep at 265pt ({} multi-line paras, {} GT breaks; {} full-measure paras) ===",
        multi_paras.len(), total_gt, all_body_nodes.len());

    for &ws_pt in &[2.50, 3.00, 3.25, 3.50, 3.75, 4.00, 4.25, 4.50] {
        let nom_sp = nat_sp_pt + ws_pt;
        for &st in &[0.50_f64, 0.70, 0.90] {
            let sh = (1.0 - 3.00 / nom_sp).max(0.33);
            let opts = ParagraphLayoutOptions {
                line_break: LineBreakMode::Optimal,
                word_spacing: ws_pt * pt_to_px,
                space_stretch: st,
                space_shrink: sh,
                sentence_space: 0.50,
                hyphen_penalty: 20.0,
                ..Default::default()
            };
            let mut spills = 0usize;
            for (txt, sp, bx, w, ind, al, fs, fw, fam, ital, sc, ls, lh) in &all_body_nodes {
                let exp = txt.matches(FORCED_JUSTIFY_BREAK).count() + 1;
                let prep = prepare_rich_text_for_shaping_with_options(
                    &mut font_cx, txt, *fs as f32, *fw as f32, fam.as_deref(), *ital, *sc, *ls, *lh,
                    Some(*w), Some(al), *ind, sp, bx, &opts,
                );
                let got = prep.as_ref().map(|p| p.text.as_str()).unwrap_or(txt).split('\n').count();
                if got != exp {
                    spills += 1;
                }
            }
            let sum = evaluate_config(&multi_paras, &opts, &mut font_cx, &mut layout_cx);
            let (res, _) = compute_minimal_residual_breaks(&multi_paras, &opts, &mut font_cx, &mut layout_cx);
            println!(
                "  ws={:+.2}pt (nom={:.2}pt, st={:.2}, sh={:.2}, min={:.2}pt) | spills={:2} | breaks {:5.1}% exact {:5.1}% | res\\|={:4} ({:4.1}% rem)",
                ws_pt, nom_sp, st, sh, nom_sp * (1.0 - sh), spills,
                100.0 * sum.matched_breaks as f64 / sum.total_gt_breaks as f64,
                100.0 * sum.exact_paras as f64 / sum.total_paras as f64,
                res,
                100.0 * (1.0 - res as f64 / sum.total_gt_breaks as f64),
            );
        }
    }
}
