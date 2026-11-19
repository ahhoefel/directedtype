use std::path::Path;

use directedtype::compiler::{
    compile_document_with_resolver, evaluate_document_with_resolver, CompileError, NodeId, Value,
    VirtualResolver,
};
use directedtype::parser::{parse_document, parse_document_with_file};

/// Builds an uncompressed/valid PNG in memory with optional `pHYs` physical density in DPI.
fn make_png_bytes(width: u32, height: u32, rgba: [u8; 4], dpi: Option<u32>) -> Vec<u8> {
    let mut img = image::RgbaImage::new(width, height);
    for px in img.pixels_mut() {
        *px = image::Rgba(rgba);
    }
    let mut raw_png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut raw_png), image::ImageFormat::Png)
        .expect("encode PNG");

    let Some(target_dpi) = dpi else {
        return raw_png;
    };

    // Insert a PNG `pHYs` chunk immediately after IHDR (byte 33: 8 sig + 25 IHDR)
    let ppm = ((target_dpi as f64) / 0.0254).round() as u32;
    let mut phys_data = [0u8; 9];
    phys_data[0..4].copy_from_slice(&ppm.to_be_bytes());
    phys_data[4..8].copy_from_slice(&ppm.to_be_bytes());
    phys_data[8] = 1; // unit = meter

    // Compute IEEE CRC32 over chunk type + data
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in b"pHYs".iter().chain(phys_data.iter()) {
        crc ^= b as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    crc = !crc;

    let insert_pos = 33usize;
    let mut out = Vec::with_capacity(raw_png.len() + 21);
    out.extend_from_slice(&raw_png[..insert_pos]);
    out.extend_from_slice(&9u32.to_be_bytes());
    out.extend_from_slice(b"pHYs");
    out.extend_from_slice(&phys_data);
    out.extend_from_slice(&crc.to_be_bytes());
    out.extend_from_slice(&raw_png[insert_pos..]);
    out
}

/// Builds a JPEG in memory with optional EXIF Orientation (`1..=8`) and EXIF DPI (`XResolution`).
fn make_jpeg_bytes(
    width: u32,
    height: u32,
    top_rgb: [u8; 3],
    bottom_rgb: [u8; 3],
    exif_orientation: Option<u16>,
    exif_dpi: Option<u32>,
) -> Vec<u8> {
    let mut img = image::RgbImage::new(width, height);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let _ = x;
        *px = if y < height / 2 {
            image::Rgb(top_rgb)
        } else {
            image::Rgb(bottom_rgb)
        };
    }
    let mut jpeg = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 100);
    encoder
        .encode_image(&img)
        .expect("encode JPEG");

    if exif_orientation.is_none() && exif_dpi.is_none() {
        return jpeg;
    }

    // Build minimal little-endian APP1 Exif segment with Orientation (0x0112),
    // XResolution (0x011A), and ResolutionUnit (0x0128 = 2 inches).
    let mut tiff = Vec::new();
    tiff.extend_from_slice(b"II"); // little-endian
    tiff.extend_from_slice(&42u16.to_le_bytes());
    tiff.extend_from_slice(&8u32.to_le_bytes()); // IFD0 offset = 8

    let mut entries: Vec<[u8; 12]> = Vec::new();
    let mut extra_data: Vec<u8> = Vec::new();

    if let Some(orient) = exif_orientation {
        let mut e = [0u8; 12];
        e[0..2].copy_from_slice(&0x0112u16.to_le_bytes());
        e[2..4].copy_from_slice(&3u16.to_le_bytes()); // SHORT
        e[4..8].copy_from_slice(&1u32.to_le_bytes());
        e[8..10].copy_from_slice(&orient.to_le_bytes());
        entries.push(e);
    }

    if let Some(dpi) = exif_dpi {
        // XResolution rational offset comes after: 8 (header) + 2 (count) + (N+1)*12 + 4 (next IFD)
        let total_entries = entries.len() + 2;
        let rational_offset = (8 + 2 + total_entries * 12 + 4) as u32;

        let mut e_xres = [0u8; 12];
        e_xres[0..2].copy_from_slice(&0x011Au16.to_le_bytes());
        e_xres[2..4].copy_from_slice(&5u16.to_le_bytes()); // RATIONAL
        e_xres[4..8].copy_from_slice(&1u32.to_le_bytes());
        e_xres[8..12].copy_from_slice(&rational_offset.to_le_bytes());
        entries.push(e_xres);

        let mut e_unit = [0u8; 12];
        e_unit[0..2].copy_from_slice(&0x0128u16.to_le_bytes());
        e_unit[2..4].copy_from_slice(&3u16.to_le_bytes()); // SHORT
        e_unit[4..8].copy_from_slice(&1u32.to_le_bytes());
        e_unit[8..10].copy_from_slice(&2u16.to_le_bytes()); // 2 = inch
        entries.push(e_unit);

        extra_data.extend_from_slice(&dpi.to_le_bytes());
        extra_data.extend_from_slice(&1u32.to_le_bytes());
    }

    tiff.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for e in &entries {
        tiff.extend_from_slice(e);
    }
    tiff.extend_from_slice(&0u32.to_le_bytes()); // next IFD = 0
    tiff.extend_from_slice(&extra_data);

    let mut app1_payload = Vec::from(&b"Exif\0\0"[..]);
    app1_payload.extend_from_slice(&tiff);
    let seg_len = (app1_payload.len() + 2) as u16;

    let mut out = Vec::with_capacity(jpeg.len() + app1_payload.len() + 4);
    out.extend_from_slice(&jpeg[..2]); // SOI 0xFF 0xD8
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&seg_len.to_be_bytes());
    out.extend_from_slice(&app1_payload);
    out.extend_from_slice(&jpeg[2..]);
    out
}

#[test]
fn test_image_three_disjoint_overloads_and_dpi() {
    let mut resolver = VirtualResolver::new();
    // 120x60 PNG without embedded DPI
    resolver.insert_bytes("/proj/plain.png", make_png_bytes(120, 60, [255, 0, 0, 255], None));
    // 300x150 PNG with 300 DPI pHYs metadata (1in x 0.5in physical)
    resolver.insert_bytes(
        "/proj/scan_300dpi.png",
        make_png_bytes(300, 150, [0, 180, 0, 255], Some(300)),
    );

    let src = r#"
    // 1. Default intrinsic (1px per pixel -> 120px x 60px; missing file_dpi falls back to window.dpi = 96)
    \let a = \Image(src: "plain.png", x: 10px, y: 20px)

    // 2. Intrinsic using file_dpi metadata (300px * 96 / 300 = 96px x 48px)
    \let b = \Image(src: "scan_300dpi.png", x: 10px, y: a.bottom + 10px, dpi: b.file_dpi)

    // 3. Width-constrained (aspect ratio = 2.0 -> height = 100px)
    \let c = \Image(src: "plain.png", x: 10px, y: b.bottom + 10px, width: 200px)

    // 4. Height-constrained (aspect ratio = 2.0 -> width = 80px)
    \let d = \Image(src: "plain.png", x: 10px, y: c.bottom + 10px, height: 40px)
    "#;

    let doc = parse_document_with_file(src, Path::new("/proj/main.dt")).expect("parse");
    let layout = evaluate_document_with_resolver(&doc, 800.0, 600.0, Path::new("/proj"), &resolver)
        .expect("evaluate");

    let find_by_var = |name: &str| {
        layout
            .nodes
            .iter()
            .find(|n| n.var_name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("missing node {}", name))
    };

    let a = find_by_var("a");
    assert!((a.rect.width - 120.0).abs() < 1e-6);
    assert!((a.rect.height - 60.0).abs() < 1e-6);
    assert_eq!(a.properties.get("file_dpi"), Some(&Value::Float(96.0)));
    assert_eq!(a.properties.get("aspect_ratio"), Some(&Value::Float(2.0)));

    let b = find_by_var("b");
    assert_eq!(b.properties.get("file_dpi"), Some(&Value::Float(300.0)));
    assert!((b.rect.width - 96.0).abs() < 1e-6);
    assert!((b.rect.height - 48.0).abs() < 1e-6);

    let c = find_by_var("c");
    assert!((c.rect.width - 200.0).abs() < 1e-6);
    assert!((c.rect.height - 100.0).abs() < 1e-6);

    let d = find_by_var("d");
    assert!((d.rect.width - 80.0).abs() < 1e-6);
    assert!((d.rect.height - 40.0).abs() < 1e-6);
}

#[test]
fn test_image_rejects_both_width_and_height_or_missing_xy() {
    let mut resolver = VirtualResolver::new();
    resolver.insert_bytes("/proj/a.png", make_png_bytes(40, 20, [0, 0, 255, 255], None));

    // Both width and height rejected (prevents aspect ratio distortion)
    let doc_both = parse_document_with_file(
        r#"\Image(src: "a.png", x: 0px, y: 0px, width: 40px, height: 40px)"#,
        Path::new("/proj/main.dt"),
    )
    .unwrap();
    let err_both =
        evaluate_document_with_resolver(&doc_both, 800.0, 600.0, Path::new("/proj"), &resolver)
            .unwrap_err();
    assert!(
        matches!(err_both, CompileError::NoMatchingOverload(_)),
        "expected NoMatchingOverload for (width, height), got: {:?}",
        err_both
    );

    // Both width and dpi rejected
    let doc_w_dpi = parse_document_with_file(
        r#"\Image(src: "a.png", x: 0px, y: 0px, width: 40px, dpi: 144)"#,
        Path::new("/proj/main.dt"),
    )
    .unwrap();
    let err_w_dpi =
        evaluate_document_with_resolver(&doc_w_dpi, 800.0, 600.0, Path::new("/proj"), &resolver)
            .unwrap_err();
    assert!(
        matches!(err_w_dpi, CompileError::NoMatchingOverload(_)),
        "expected NoMatchingOverload for (width, dpi), got: {:?}",
        err_w_dpi
    );

    // Missing x or y rejected
    let doc_no_xy = parse_document_with_file(
        r#"\Image(src: "a.png", width: 40px)"#,
        Path::new("/proj/main.dt"),
    )
    .unwrap();
    let err_no_xy =
        evaluate_document_with_resolver(&doc_no_xy, 800.0, 600.0, Path::new("/proj"), &resolver)
            .unwrap_err();
    assert!(
        matches!(err_no_xy, CompileError::MissingPort { ref port, .. } if port == "x"),
        "expected MissingPort 'x', got: {:?}",
        err_no_xy
    );
}

#[test]
fn test_path_type_source_file_anchoring_and_string_rejection() {
    let mut resolver = VirtualResolver::new();
    // Brand SVG lives next to BrandHeader.dt (40x40 -> aspect 1.0)
    resolver.insert_bytes(
        "/workspace/components/widgets/brand.svg",
        br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><rect width="40" height="40" fill="red"/></svg>"##
            .to_vec(),
    );
    // Caller's avatar PNG lives next to pages/profile.dt (90x30 -> aspect 3.0)
    resolver.insert_bytes(
        "/workspace/pages/alice.png",
        make_png_bytes(90, 30, [20, 120, 220, 255], None),
    );

    resolver.insert(
        "/workspace/components/widgets/BrandHeader.dt",
        r#"
        \Component BrandHeader(avatar: Path, x: Distance, y: Distance) {
            \let logo = \Image(src: "brand.svg", x: x, y: y, height: 20px)
            \let user = \Image(src: avatar, x: logo.right + 8px, y: y, height: 20px)
            \alias width = user.right - x;
            \alias height = 20px;
        }
        "#,
    );

    let app_src = r#"
    \use "../components/widgets/BrandHeader.dt"
    \let hdr = \BrandHeader(avatar: "alice.png", x: 10px, y: 10px)
    "#;
    let doc = parse_document_with_file(app_src, Path::new("/workspace/pages/profile.dt")).unwrap();
    let layout = evaluate_document_with_resolver(
        &doc,
        800.0,
        600.0,
        Path::new("/workspace/pages"),
        &resolver,
    )
    .expect("cross-module source-anchored Path resolution");

    // logo: 20px * 1.0 = 20px; gap: 8px; user: 20px * 3.0 = 60px -> total width = 88px
    let hdr = layout
        .nodes
        .iter()
        .find(|n| n.var_name.as_deref() == Some("hdr"))
        .unwrap();
    assert!((hdr.rect.width - 88.0).abs() < 1e-6);

    // Passing String parameter to Image(src: ...) must be a compile-time TypeMismatch
    let bad_comp_src = r#"
    \Component BadAvatar(avatar: String, x: Distance, y: Distance) {
        \Image(src: avatar, x: x, y: y)
    }
    \BadAvatar(avatar: "alice.png", x: 0px, y: 0px)
    "#;
    let bad_doc =
        parse_document_with_file(bad_comp_src, Path::new("/workspace/pages/profile.dt")).unwrap();
    let bad_err = evaluate_document_with_resolver(
        &bad_doc,
        800.0,
        600.0,
        Path::new("/workspace/pages"),
        &resolver,
    )
    .unwrap_err();
    assert!(
        matches!(
            bad_err,
            CompileError::TypeMismatch {
                ref expected,
                ref actual,
                ..
            } if expected == "Path" && actual == "String"
        ),
        "expected TypeMismatch(Path vs String), got: {:?}",
        bad_err
    );

    // Untyped `\let s = "alice.png"` has type String, so passing it to `src: Path` also fails
    let untyped_let_src = r#"
    \let s = "alice.png";
    \Image(src: s, x: 0px, y: 0px)
    "#;
    let untyped_doc =
        parse_document_with_file(untyped_let_src, Path::new("/workspace/pages/profile.dt"))
            .unwrap();
    let untyped_err = evaluate_document_with_resolver(
        &untyped_doc,
        800.0,
        600.0,
        Path::new("/workspace/pages"),
        &resolver,
    )
    .unwrap_err();
    assert!(matches!(untyped_err, CompileError::TypeMismatch { .. }));
}

#[test]
fn test_upfront_preloading_and_instant_ternary_swap() {
    let mut resolver = VirtualResolver::new();
    resolver.insert_bytes(
        "/theme/light.svg",
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#ffffff"/></svg>"##
            .to_vec(),
    );
    resolver.insert_bytes(
        "/theme/dark.svg",
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="20"><rect width="80" height="20" fill="#000000"/></svg>"##
            .to_vec(),
    );

    let src = r#"
    \state is_dark: Boolean = false;
    \let icon = \Image(
        src: is_dark ? "dark.svg" : "light.svg",
        x: 0px,
        y: 0px
    )
    "#;
    let doc = parse_document_with_file(src, Path::new("/theme/app.dt")).unwrap();
    let mut compiled =
        compile_document_with_resolver(&doc, 800.0, 600.0, Path::new("/theme"), &resolver)
            .expect("compile with ternary paths");

    // Both branches must be preloaded upfront even though `is_dark` starts false!
    assert_eq!(compiled.expanded.image_assets.len(), 2);

    let icon_id = compiled
        .layout
        .nodes
        .iter()
        .find(|n| n.var_name.as_deref() == Some("icon"))
        .unwrap()
        .id;
    assert!((compiled.layout.get_node(icon_id).unwrap().rect.width - 40.0).abs() < 1e-6);

    // Mutate state `is_dark = true` -> swaps instantaneously using preloaded store
    compiled
        .set_state(NodeId::WINDOW, "is_dark", Value::Bool(true))
        .expect("toggle is_dark");
    assert!((compiled.layout.get_node(icon_id).unwrap().rect.width - 80.0).abs() < 1e-6);

    // Missing inactive branch fails immediately at compile time (never deferred to runtime)
    let broken_src = r#"
    \state is_dark: Boolean = false;
    \Image(src: is_dark ? "missing_dark.svg" : "light.svg", x: 0px, y: 0px)
    "#;
    let broken_doc = parse_document_with_file(broken_src, Path::new("/theme/app.dt")).unwrap();
    assert!(
        compile_document_with_resolver(&broken_doc, 800.0, 600.0, Path::new("/theme"), &resolver)
            .is_err()
    );
}

#[test]
fn test_jpeg_exif_orientation_and_dpi_metadata() {
    let mut resolver = VirtualResolver::new();
    // Raw buffer is 80x40, but EXIF Orientation = 6 (90 deg CW) -> upright is 40x80!
    // Also embeds 192 DPI in EXIF.
    let jpeg_bytes = make_jpeg_bytes(80, 40, [255, 0, 0], [0, 0, 255], Some(6), Some(192));
    resolver.insert_bytes("/photos/rotated.jpg", jpeg_bytes);

    let src = r#"
    \let raw = \Image(src: "rotated.jpg", x: 0px, y: 0px)
    \let scaled = \Image(src: "rotated.jpg", x: 100px, y: 0px, dpi: scaled.file_dpi)
    "#;
    let doc = parse_document_with_file(src, Path::new("/photos/view.dt")).unwrap();
    let layout =
        evaluate_document_with_resolver(&doc, 800.0, 600.0, Path::new("/photos"), &resolver)
            .expect("evaluate oriented JPEG");

    let raw = layout
        .nodes
        .iter()
        .find(|n| n.var_name.as_deref() == Some("raw"))
        .unwrap();
    // Swapped to 40x80!
    assert!((raw.rect.width - 40.0).abs() < 1e-6);
    assert!((raw.rect.height - 80.0).abs() < 1e-6);

    let scaled = layout
        .nodes
        .iter()
        .find(|n| n.var_name.as_deref() == Some("scaled"))
        .unwrap();
    assert_eq!(scaled.properties.get("file_dpi"), Some(&Value::Float(192.0)));
    // 96 / 192 = 0.5 -> 20px x 40px
    assert!((scaled.rect.width - 20.0).abs() < 1e-6);
    assert!((scaled.rect.height - 40.0).abs() < 1e-6);
}

#[test]
fn test_svg_physical_units_and_inline_image_component() {
    let inline_dt = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("components/InlineImage.dt"),
    )
    .expect("read InlineImage.dt");

    let mut resolver = VirtualResolver::new();
    resolver.insert("/app/components/InlineImage.dt", inline_dt);
    // Physical 1in x 0.5in SVG
    resolver.insert_bytes(
        "/app/badge.svg",
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1in" height="0.5in" viewBox="0 0 100 50"><rect width="100" height="50" fill="#2563eb"/></svg>"##
            .to_vec(),
    );

    let src = r#"
    \use "components/InlineImage.dt"
    \env font = \Font(family: "Arial", size: 20px);

    \let phys = \Image(src: "badge.svg", x: 0px, y: 0px)
    \let para = \Text(x: 0px, y: 80px, color: #000000) {
        Click \InlineImage(src: "badge.svg", height: 1em, baseline_offset: 0.8em) here
    }
    "#;
    let doc = parse_document_with_file(src, Path::new("/app/index.dt")).unwrap();
    let layout = evaluate_document_with_resolver(&doc, 800.0, 600.0, Path::new("/app"), &resolver)
        .expect("evaluate InlineImage + physical SVG");

    // 1in x 0.5in at 96 DPI = 96px x 48px
    let phys = layout
        .nodes
        .iter()
        .find(|n| n.var_name.as_deref() == Some("phys"))
        .unwrap();
    assert!((phys.rect.width - 96.0).abs() < 1e-6);
    assert!((phys.rect.height - 48.0).abs() < 1e-6);

    let inline_img = layout
        .nodes
        .iter()
        .find(|n| n.name == "InlineImage")
        .expect("InlineImage node");
    // height = 1em = 20px, aspect = 2.0 -> width = 40px
    assert!((inline_img.rect.height - 20.0).abs() < 1e-6);
    assert!((inline_img.rect.width - 40.0).abs() < 1e-6);
    // Parley positioned it after "Click " (x > 0)
    assert!(inline_img.rect.x > 10.0);
    let baseline = inline_img
        .properties
        .get("baseline")
        .and_then(|v| v.as_f64())
        .unwrap();
    assert!((baseline - (inline_img.rect.y + 16.0)).abs() < 1e-4);

    // Standalone \InlineImage outside \Text without x and y must fail to compile!
    let bad_inline = parse_document_with_file(
        r#"
        \use "components/InlineImage.dt"
        \InlineImage(src: "badge.svg", height: 16px)
        "#,
        Path::new("/app/index.dt"),
    )
    .unwrap();
    assert!(
        evaluate_document_with_resolver(&bad_inline, 800.0, 600.0, Path::new("/app"), &resolver)
            .is_err(),
        "Standalone InlineImage without x and y should produce a compile error"
    );
}

#[cfg(feature = "vello-renderer")]
#[test]
fn test_headless_vello_renders_png_jpeg_and_svg_with_radius() {
    use directedtype::render::{HeadlessRenderer, SceneOptions};

    let mut resolver = VirtualResolver::new();
    // Solid red PNG
    resolver.insert_bytes(
        "/render/red.png",
        make_png_bytes(40, 40, [255, 0, 0, 255], None),
    );
    // Solid green SVG circle/rect
    resolver.insert_bytes(
        "/render/green.svg",
        br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><rect x="0" y="0" width="40" height="40" fill="#00ff00"/></svg>"##
            .to_vec(),
    );
    // Solid blue JPEG
    resolver.insert_bytes(
        "/render/blue.jpg",
        make_jpeg_bytes(40, 40, [0, 0, 255], [0, 0, 255], None, None),
    );

    let src = r#"
    \Image(src: "red.png", x: 10px, y: 10px, width: 40px, radius: 20px)
    \Image(src: "green.svg", x: 60px, y: 10px, width: 40px)
    \Image(src: "blue.jpg", x: 110px, y: 10px, width: 40px)
    "#;
    let doc = parse_document_with_file(src, Path::new("/render/scene.dt")).unwrap();
    let layout =
        evaluate_document_with_resolver(&doc, 200.0, 80.0, Path::new("/render"), &resolver)
            .expect("layout");

    let mut renderer = match HeadlessRenderer::new() {
        Ok(r) => r,
        Err(_) => return, // Skip if CI environment has no GPU adapter
    };
    let img = renderer
        .render_layout(&layout, 200, 80, &SceneOptions::default())
        .expect("render Vello image");

    // Center of red rounded PNG (30, 30) is red; clipped corner (11, 11) stays white!
    let p_red_center = img.get_pixel(30, 30).0;
    assert!(p_red_center[0] > 230 && p_red_center[1] < 25 && p_red_center[2] < 25);
    let p_red_corner = img.get_pixel(11, 11).0;
    assert!(
        p_red_corner[0] > 240 && p_red_corner[1] > 240 && p_red_corner[2] > 240,
        "Expected rounded corner (11,11) to be clipped to white background, got {:?}",
        p_red_corner
    );

    // Center of green SVG (80, 30) is green
    let p_green = img.get_pixel(80, 30).0;
    assert!(p_green[1] > 230 && p_green[0] < 25 && p_green[2] < 25);

    // Center of blue JPEG (130, 30) is blue
    let p_blue = img.get_pixel(130, 30).0;
    assert!(p_blue[2] > 220 && p_blue[0] < 30 && p_blue[1] < 30);
}

#[cfg(feature = "pdf")]
#[test]
fn test_pdf_export_embeds_png_raw_jpeg_and_vector_svg() {
    use directedtype::component::ComponentRegistry;
    use directedtype::pdf::{compile_document_to_pdf, PdfOptions};

    let mut resolver = VirtualResolver::new();
    resolver.insert_bytes(
        "/pdf/photo.jpg",
        make_jpeg_bytes(60, 40, [220, 40, 40], [40, 40, 220], None, Some(300)),
    );
    resolver.insert_bytes(
        "/pdf/alpha.png",
        make_png_bytes(50, 50, [20, 180, 120, 180], None),
    );
    resolver.insert_bytes(
        "/pdf/diagram.svg",
        br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 50">
            <defs>
                <linearGradient id="g" x1="0" y1="0" x2="100" y2="0" gradientUnits="userSpaceOnUse">
                    <stop offset="0%" stop-color="#3b82f6"/>
                    <stop offset="100%" stop-color="#10b981"/>
                </linearGradient>
            </defs>
            <rect x="2" y="2" width="96" height="46" rx="8" fill="url(#g)" stroke="#0f172a" stroke-width="2"/>
        </svg>"##
            .to_vec(),
    );

    let src = r#"
    \let j = \Image(src: "photo.jpg", x: 36pt, y: 36pt, dpi: j.file_dpi)
    \let p = \Image(src: "alpha.png", x: 36pt, y: 100pt, width: 72pt, radius: 8pt)
    \let s = \Image(src: "diagram.svg", x: 36pt, y: 200pt, width: 200pt)
    "#;
    let doc = parse_document(src).unwrap();
    let opts = PdfOptions {
        compressed: false,
        deterministic: true,
        fallback_width: Some(612.0),
        fallback_height: Some(792.0),
        ..Default::default()
    };
    let pdf_bytes = compile_document_to_pdf(
        &doc,
        Path::new("/pdf"),
        &resolver,
        &ComponentRegistry::standard(),
        &opts,
    )
    .expect("export PDF with JPG, PNG, and SVG");

    assert!(pdf_bytes.starts_with(b"%PDF-1.7"));
    let pdf_str = String::from_utf8_lossy(&pdf_bytes);
    // Upright JPEG must be embedded natively via /DCTDecode without re-encoding
    assert!(
        pdf_str.contains("/DCTDecode"),
        "Expected native /DCTDecode JPEG stream in PDF output"
    );
    // PNG transparency should emit an /SMask soft mask
    assert!(
        pdf_str.contains("/SMask"),
        "Expected /SMask alpha channel for translucent PNG in PDF output"
    );
}

#[cfg(feature = "pdf")]
#[test]
fn test_paged_flow_splits_images_cleanly_across_pages() {
    use directedtype::component::ComponentRegistry;
    use directedtype::compiler::{paginate_compiled_document, CompiledDocument};
    use directedtype::pdf::{render_pages_to_pdf, PdfOptions};

    let mut resolver = VirtualResolver::new();
    resolver.insert_bytes(
        "/book/fig1.png",
        make_png_bytes(120, 90, [200, 50, 50, 255], None),
    );
    resolver.insert_bytes(
        "/book/fig2.svg",
        br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 90"><rect width="120" height="90" fill="#22c55e"/></svg>"##
            .to_vec(),
    );

    let src = r#"
    \Component SimplePage {
        \alias width = 200pt;
        \alias height = 160pt;
        \FlowBody(x: 20pt, y: 20pt, width: 160pt, height: 120pt)
    }

    \PagedFlow(page: \SimplePage) {
        \Image(src: "fig1.png", width: 120pt)
        \Image(src: "fig2.svg", width: 120pt)
    }
    "#;
    let doc = parse_document_with_file(src, Path::new("/book/doc.dt")).unwrap();
    let compiled = CompiledDocument::compile_with_registry_and_dpi(
        &doc,
        200.0,
        160.0,
        72.0,
        Path::new("/book"),
        &resolver,
        &ComponentRegistry::standard(),
    )
    .expect("compile paged document with images");

    // Each image is 120pt x 90pt inside a 120pt-tall FlowBody -> must paginate across 2 pages!
    let paginated = paginate_compiled_document(&compiled, 200.0, 160.0, 72.0)
        .expect("paginate document with images");
    assert_eq!(paginated.pages.len(), 2, "Expected 2 pages for two 90pt images in 120pt body");

    let pdf_bytes = render_pages_to_pdf(&paginated.pages, Some("Paged Images"), &PdfOptions::default())
        .expect("render 2-page PDF with PNG and SVG");
    assert!(pdf_bytes.starts_with(b"%PDF-1.7"));
}

