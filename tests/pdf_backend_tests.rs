#![cfg(feature = "pdf")]

use std::path::Path;

use directedtype::pdf::{
    compile_file_to_pdf, compile_source_to_pdf, PdfOptions, PdfVersionOpt,
};

#[test]
fn test_pdf_header_trailer_and_versions() {
    let src = r#"
    \Rect(x: 0pt, y: 0pt, width: 8.5in, height: 11in, color: #f8fafc)
    "#;

    let opts_17 = PdfOptions {
        pdf_version: PdfVersionOpt::Pdf17,
        compressed: false,
        deterministic: true,
        fallback_width: Some(612.0),
        fallback_height: Some(792.0),
        ..Default::default()
    };
    let bytes_17 = compile_source_to_pdf(src, Path::new("."), &opts_17).expect("PDF 1.7 export");
    let text_17 = String::from_utf8_lossy(&bytes_17);
    assert!(text_17.starts_with("%PDF-1.7"), "Expected %PDF-1.7 header");
    assert!(text_17.trim_end().ends_with("%%EOF"), "Expected %%EOF trailer");
    assert!(
        text_17.contains("/MediaBox[0 0 612 792]")
            || text_17.contains("/MediaBox [0 0 612 792]"),
        "Expected US Letter MediaBox [0 0 612 792] in PDF output:\n{}",
        &text_17[..text_17.len().min(2000)]
    );

    let opts_20 = PdfOptions {
        pdf_version: PdfVersionOpt::Pdf20,
        ..opts_17
    };
    let bytes_20 = compile_source_to_pdf(src, Path::new("."), &opts_20).expect("PDF 2.0 export");
    assert!(bytes_20.starts_with(b"%PDF-2.0"), "Expected %PDF-2.0 header");
    assert!(String::from_utf8_lossy(&bytes_20)
        .trim_end()
        .ends_with("%%EOF"));
}

#[test]
fn test_pdf_deterministic_output() {
    let src = r#"
    \use "components/VStackStretch.dt"
    \use "components/Link.dt"
    \use "theme/default.dt"

    \env font = \Font(size: 12pt, line_height: 16pt);
    \env link_style = link_default;

    \VStackStretch(x: 40pt, y: 40pt, width: 300pt, gap: 10pt) {
        \Text(color: #0f172a) { Deterministic Typography fi fl }
        \Text(color: #1e293b) { Visit \Link(url: "https://example.com"){Example} }
    }
    "#;
    let opts = PdfOptions {
        compressed: true,
        deterministic: true,
        fallback_width: Some(400.0),
        fallback_height: Some(300.0),
        ..Default::default()
    };

    let b1 = compile_source_to_pdf(src, Path::new("."), &opts).expect("First deterministic build");
    let b2 = compile_source_to_pdf(src, Path::new("."), &opts).expect("Second deterministic build");
    assert_eq!(b1, b2, "Deterministic PDF bytes must match identically across runs");
}

#[test]
fn test_pdf_explicit_window_dpi_override_scales_correctly() {
    let src = r#"
    \Window(dpi: 96.0)
    \Rect(x: 0pt, y: 0pt, width: 100pt, height: 100pt, color: #2563eb)
    "#;
    let opts = PdfOptions {
        compressed: false,
        deterministic: true,
        fallback_width: None,
        fallback_height: None,
        ..Default::default()
    };
    let bytes = compile_source_to_pdf(src, Path::new("."), &opts).expect("DPI override export");
    let text = String::from_utf8_lossy(&bytes);
    // 800 * (72 / 96) = 600, 600 * (72 / 96) = 450
    assert!(
        text.contains("/MediaBox[0 0 600 450]") || text.contains("/MediaBox [0 0 600 450]"),
        "Expected 72/96 scaled MediaBox [0 0 600 450], got:\n{}",
        &text[..text.len().min(2000)]
    );
    // krilla merges the root page Y-flip with dpi_scale: 0.75 0 0 -0.75 0 450 cm (or 0.75 0 0 0.75)
    assert!(
        text.contains("0.75 0 0 -0.75") || text.contains("0.75 0 0 0.75"),
        "Expected 0.75 scale CTM in content stream when \\Window(dpi: 96.0):\n{}",
        &text[..text.len().min(2000)]
    );
}

#[test]
fn test_pdf_tounicode_glyphs_opacity_and_transparent_rect() {
    let src = r#"
    \env font = \Font(size: 14pt, line_height: 18pt);

    // Transparent fill rect should NOT emit a fill polygon, while keeping its border
    \Rect(x: 10pt, y: 10pt, width: 100pt, height: 40pt, color: #00000000, border_width: 1pt, border_color: #112233)

    // Visible text + transparent #00000000 text + opacity 0.0 text must ALL emit BT...ET text operators
    \Text(x: 20pt, y: 70pt, width: 200pt, color: #000000) { Searchable visible line }
    \Text(x: 20pt, y: 100pt, width: 200pt, color: #00000000) { Hidden alpha zero text }
    \Text(x: 20pt, y: 130pt, width: 200pt, color: #000000, opacity: 0.0) { Opacity zero text }
    "#;
    let opts = PdfOptions {
        compressed: false,
        deterministic: true,
        fallback_width: Some(300.0),
        fallback_height: Some(200.0),
        ..Default::default()
    };
    let bytes = compile_source_to_pdf(src, Path::new("."), &opts).expect("Text & opacity PDF export");
    let text = String::from_utf8_lossy(&bytes);

    assert!(text.contains("/ToUnicode"), "Expected /ToUnicode CMap for searchable text");
    assert!(text.contains("BT"), "Expected BT text operator");
    assert!(text.contains("ET"), "Expected ET text operator");
    // Transparent text and opacity 0.0 text must BOTH emit ExtGState with ca 0
    assert!(
        text.contains("/ca 0"),
        "Expected /ca 0 graphics state for transparent / opacity:0 text:\n{}",
        &text[..text.len().min(4000)]
    );
    // Check that all 3 text blocks emitted `Tj` or `TJ` operators inside `BT` blocks
    let bt_count = text.matches(" BT").count() + text.matches("\nBT").count();
    assert!(
        bt_count >= 3,
        "Expected >= 3 BT text blocks (visible, #00000000, opacity:0.0), got {} in:\n{}",
        bt_count,
        &text[..text.len().min(4000)]
    );
}

#[test]
fn test_pdf_shadow_smask_and_hard_shadow() {
    let src_blurred = r#"
    \Rect(
        x: 40pt,
        y: 40pt,
        width: 120pt,
        height: 60pt,
        radius: 8pt,
        color: #ffffff,
        shadow: \Shadow(offset_x: 0pt, offset_y: 4pt, blur: 10pt, spread: 0pt, color: #00000040)
    )
    "#;
    let opts = PdfOptions {
        compressed: false,
        deterministic: true,
        fallback_width: Some(240.0),
        fallback_height: Some(180.0),
        ..Default::default()
    };
    let blurred_bytes = compile_source_to_pdf(src_blurred, Path::new("."), &opts).expect("Blurred shadow PDF");
    let blurred_text = String::from_utf8_lossy(&blurred_bytes);
    assert!(blurred_text.contains("/SMask"), "Expected /SMask soft mask for blurred \\Shadow");
    assert!(blurred_text.contains("/DeviceGray"), "Expected /DeviceGray luminance image for \\Shadow");

    let src_hard = r#"
    \Rect(
        x: 40pt,
        y: 40pt,
        width: 120pt,
        height: 60pt,
        color: #ffffff,
        shadow: \Shadow(offset_x: 4pt, offset_y: 4pt, blur: 0pt, spread: 0pt, color: #00000080)
    )
    "#;
    let hard_bytes = compile_source_to_pdf(src_hard, Path::new("."), &opts).expect("Hard shadow PDF");
    let hard_text = String::from_utf8_lossy(&hard_bytes);
    assert!(!hard_text.contains("/SMask"), "Hard shadow (blur: 0) should stay pure vector without /SMask");
}

#[test]
fn test_pdf_links_internal_anchors_and_outlines() {
    let src = r##"
    \use "components/VStackStretch.dt"
    \use "components/Link.dt"
    \use "theme/default.dt"

    \env font = \Font(size: 12pt, line_height: 16pt);
    \env link_style = link_default;

    \VStackStretch(x: 40pt, y: 40pt, width: 320pt, gap: 16pt) {
        \Text { Go to \Link(url: "#methods"){Experimental Methods} }
        \Text { See \Link(url: "https://example.com/docs"){Documentation} }
        \Bookmark(title: "1. Methods", level: 1) {
            \Anchor("methods") {
                \Text {
                    1. Experimental Methods
                }
            }
        }
    }
    "##;
    let opts = PdfOptions {
        compressed: false,
        deterministic: true,
        fallback_width: Some(450.0),
        fallback_height: Some(400.0),
        strict_links: true,
        ..Default::default()
    };
    let bytes = compile_source_to_pdf(src, Path::new("."), &opts).expect("Links & outline PDF");
    let text = String::from_utf8_lossy(&bytes);

    assert!(
        text.contains("/Subtype/Link") || text.contains("/Subtype /Link"),
        "Expected /Link annotations in:\n{}",
        &text[..text.len().min(4000)]
    );
    assert!(
        text.contains("/URI(https://example.com/docs)")
            || text.contains("/URI (https://example.com/docs)"),
        "Expected external URI action in:\n{}",
        &text[..text.len().min(4000)]
    );
    assert!(
        text.contains("/XYZ "),
        "Expected internal /XYZ destination in:\n{}",
        &text[..text.len().min(4000)]
    );
    assert!(
        text.contains("/Outlines"),
        "Expected PDF document /Outlines in:\n{}",
        &text[..text.len().min(4000)]
    );
}

#[test]
fn test_pdf_existing_examples_export() {
    let opts = PdfOptions {
        compressed: true,
        deterministic: true,
        ..Default::default()
    };

    for example in ["examples/text_flow_demo.dt", "examples/link_demo.dt"] {
        let bytes = compile_file_to_pdf(Path::new(example), &opts)
            .unwrap_or_else(|e| panic!("Expected '{}' to export cleanly to PDF: {}", example, e));
        assert!(bytes.starts_with(b"%PDF-1.7"));
        assert!(bytes.len() > 500, "Expected non-trivial PDF output for {}", example);
    }
}

#[test]
fn test_pdf_book_demo_body_text_painted_over_page_background() {
    let opts = PdfOptions {
        compressed: false,
        deterministic: true,
        ..Default::default()
    };
    let bytes = compile_file_to_pdf(Path::new("examples/book_demo.dt"), &opts)
        .expect("book_demo.dt should export uncompressed cleanly");
    let text = String::from_utf8_lossy(&bytes);

    assert!(
        text.contains("/Type/Pages/Count 3/Kids[") || text.contains("/Type /Pages /Count 3"),
        "Expected 3 pages with zero leading blank pages in book_demo PDF:\n{}",
        &text[..text.len().min(1500)]
    );

    let bg_rect_op = "0 0 m 612 0 l 612 792 l 0 792 l";
    let mut page_streams_with_bg = 0usize;
    let mut rest = text.as_ref();
    while let Some(start) = rest.find("stream\n") {
        let after_start = &rest[start + "stream\n".len()..];
        let Some(end) = after_start.find("\nendstream") else {
            break;
        };
        let stream_body = &after_start[..end];
        if let Some(bg_pos) = stream_body.find(bg_rect_op) {
            page_streams_with_bg += 1;
            let first_bt = stream_body
                .find("\nBT ")
                .or_else(|| stream_body.find(" BT "))
                .expect("Each book_demo page stream must emit text operators");
            assert!(
                bg_pos < first_bt,
                "Full-page background fill ({}) must occur BEFORE first BT text operator ({}) in page stream {}",
                bg_pos,
                first_bt,
                page_streams_with_bg
            );
        }
        rest = &after_start[end + "\nendstream".len()..];
    }
    assert_eq!(
        page_streams_with_bg, 3,
        "Expected all 3 page content streams to paint full-page background before body text"
    );
}
