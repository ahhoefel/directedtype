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
    \use "components/LineBreak.dt"

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
        \use "components/LineBreak.dt"
        \use "components/TextBounds.dt"
        \use "components/TextStart.dt"
        \use "components/TextEnd.dt"

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
    \use "components/LineBreak.dt"

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
        \use "components/LineBreak.dt"
        \use "components/TextBounds.dt"
        \use "components/TextStart.dt"
        \use "components/TextEnd.dt"

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
    \use "components/LineBreak.dt"

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

        // Compute ground-truth lines by shaping `active_text` with existing `\|` breaks
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
            &ParagraphLayoutOptions::default(),
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
            &ParagraphLayoutOptions::default(),
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
