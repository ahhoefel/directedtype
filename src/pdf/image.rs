use std::sync::Arc;

use krilla::color::rgb;
use krilla::geom::{Path, PathBuilder, Size, Transform};
use krilla::image::{BitsPerComponent, CustomImage, Image, ImageColorspace};
use krilla::paint::{
    Fill, FillRule, LineCap, LineJoin, LinearGradient, Paint, RadialGradient, SpreadMethod, Stop,
    Stroke, StrokeDash,
};
use krilla::surface::Surface;
use krilla::Data;

use crate::compiler::image::{lookup_loaded_image, DecodedImageData};
use crate::compiler::layout::{extract_corner_radii, ResolvedLayout, ResolvedNode};
use crate::compiler::value::Value;
use crate::render::color::{parse_color_rgba8, Rgba8};

use super::lower::build_rect_path;
use super::shadow::draw_rect_shadow;
use super::{norm_f32, rgba_to_krilla};

#[derive(Clone, Hash)]
struct UprightRgbaPdfImage {
    rgb_pixels: Vec<u8>,
    alpha_pixels: Option<Vec<u8>>,
    width: u32,
    height: u32,
}

impl UprightRgbaPdfImage {
    fn from_rgba8(width: u32, height: u32, rgba: &[u8]) -> Self {
        let count = (width as usize).saturating_mul(height as usize);
        let mut rgb_pixels = Vec::with_capacity(count * 3);
        let mut has_non_opaque = false;
        for px in rgba.chunks_exact(4) {
            rgb_pixels.push(px[0]);
            rgb_pixels.push(px[1]);
            rgb_pixels.push(px[2]);
            if px[3] < 255 {
                has_non_opaque = true;
            }
        }
        let alpha_pixels = if has_non_opaque {
            let mut a = Vec::with_capacity(count);
            for px in rgba.chunks_exact(4) {
                a.push(px[3]);
            }
            Some(a)
        } else {
            None
        };
        Self {
            rgb_pixels,
            alpha_pixels,
            width,
            height,
        }
    }
}

impl CustomImage for UprightRgbaPdfImage {
    fn color_channel(&self) -> &[u8] {
        &self.rgb_pixels
    }

    fn alpha_channel(&self) -> Option<&[u8]> {
        self.alpha_pixels.as_deref()
    }

    fn bits_per_component(&self) -> BitsPerComponent {
        BitsPerComponent::Eight
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn icc_profile(&self) -> Option<&[u8]> {
        None
    }

    fn color_space(&self) -> ImageColorspace {
        ImageColorspace::Rgb
    }
}

/// Renders an `\Image` primitive onto a `krilla::surface::Surface` in top-down PDF coordinates.
pub(super) fn draw_image_node_pdf(
    surface: &mut Surface<'_>,
    node: &ResolvedNode,
    layout: &ResolvedLayout,
    node_offset: (f64, f64),
    pushed_opacity: bool,
) {
    if node.rect.width <= 0.0 || node.rect.height <= 0.0 {
        return;
    }

    let Some(src_path) = node.properties.get("src").and_then(|v| v.as_str()) else {
        return;
    };
    if src_path.is_empty() {
        return;
    }

    let Ok(asset) = lookup_loaded_image(src_path, Some(&layout.image_assets)) else {
        return;
    };

    let x0 = node.rect.x + node_offset.0;
    let y0 = node.rect.y + node_offset.1;
    let w = node.rect.width;
    let h = node.rect.height;
    let x1 = x0 + w;
    let y1 = y0 + h;
    let radii = extract_corner_radii(&node.properties);
    let has_radius = radii.0 > 1e-6 || radii.1 > 1e-6 || radii.2 > 1e-6 || radii.3 > 1e-6;

    // 1. Drop shadow behind image box
    draw_rect_shadow(surface, &node.properties, x0, y0, x1, y1, radii);

    let rect_path_opt = build_rect_path(x0, y0, w, h, radii);
    let is_svg = matches!(asset.decoded, DecodedImageData::Svg { .. });
    let pushed_clip = if has_radius || is_svg {
        if let Some(ref clip_path) = rect_path_opt {
            surface.push_clip_path(clip_path, &FillRule::NonZero);
            true
        } else {
            false
        }
    } else {
        false
    };

    match &asset.decoded {
        DecodedImageData::Raster {
            width,
            height,
            rgba8,
            raw_bytes,
            is_raw_upright_jpeg,
            is_raw_upright_png,
        } => {
            if let Some(pdf_size) = Size::from_wh(w as f32, h as f32) {
                let krilla_img_opt = if *is_raw_upright_jpeg {
                    Image::from_jpeg(Data::from(Arc::clone(raw_bytes)), true).ok()
                } else if *is_raw_upright_png {
                    Image::from_png(Data::from(Arc::clone(raw_bytes)), true).ok()
                } else {
                    None
                }
                .or_else(|| {
                    if *width > 0 && *height > 0 {
                        Image::from_custom(
                            UprightRgbaPdfImage::from_rgba8(*width, *height, rgba8),
                            true,
                        )
                        .ok()
                    } else {
                        None
                    }
                });

                if let Some(krilla_img) = krilla_img_opt {
                    surface.push_transform(&Transform::from_translate(x0 as f32, y0 as f32));
                    surface.draw_image(krilla_img, pdf_size);
                    surface.pop();
                }
            }
        }
        DecodedImageData::Svg { tree, .. } => {
            let tw = tree.size().width() as f64;
            let th = tree.size().height() as f64;
            if tw > 0.0 && th > 0.0 {
                let sx = (w / tw) as f32;
                let sy = (h / th) as f32;
                surface.push_transform(&Transform::from_translate(x0 as f32, y0 as f32));
                surface.push_transform(&Transform::from_scale(sx, sy));
                render_usvg_group_pdf(surface, tree.root());
                surface.pop();
                surface.pop();
            }
        }
    }

    if pushed_clip {
        surface.pop();
    }

    // 3. Optional border stroke on top (matching \Rect)
    if let Some(rect_path) = rect_path_opt {
        let border_width = node
            .properties
            .get("border_width")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        if border_width > 0.0 {
            let border_rgba = node
                .properties
                .get("border_color")
                .map(|v| match v {
                    Value::Color(c) | Value::String(c) => parse_color_rgba8(c),
                    _ => Rgba8::from_rgba8(0, 0, 0, 255),
                })
                .unwrap_or_else(|| Rgba8::from_rgba8(0, 0, 0, 255));
            if border_rgba.a > 0 || pushed_opacity {
                let (border_rgb, border_alpha) = rgba_to_krilla(border_rgba);
                surface.set_fill(None);
                surface.set_stroke(Some(Stroke {
                    paint: border_rgb.into(),
                    width: border_width as f32,
                    opacity: border_alpha,
                    ..Default::default()
                }));
                surface.draw_path(&rect_path);
            }
        }
    }
}

fn usvg_transform_to_krilla(t: usvg::Transform) -> Transform {
    Transform::from_row(t.sx, t.ky, t.kx, t.sy, t.tx, t.ty)
}

fn usvg_path_to_krilla(path: &usvg::tiny_skia_path::Path) -> Option<Path> {
    let mut pb = PathBuilder::new();
    for seg in path.segments() {
        match seg {
            usvg::tiny_skia_path::PathSegment::MoveTo(p) => {
                pb.move_to(p.x, p.y);
            }
            usvg::tiny_skia_path::PathSegment::LineTo(p) => {
                pb.line_to(p.x, p.y);
            }
            usvg::tiny_skia_path::PathSegment::QuadTo(p1, p2) => {
                pb.quad_to(p1.x, p1.y, p2.x, p2.y);
            }
            usvg::tiny_skia_path::PathSegment::CubicTo(p1, p2, p3) => {
                pb.cubic_to(p1.x, p1.y, p2.x, p2.y, p3.x, p3.y);
            }
            usvg::tiny_skia_path::PathSegment::Close => {
                pb.close();
            }
        }
    }
    pb.finish()
}

fn push_group_paths_transformed(
    group: &usvg::Group,
    transform: usvg::Transform,
    pb: &mut PathBuilder,
) {
    for child in group.children() {
        match child {
            usvg::Node::Path(p) => {
                if let Some(transformed) = p.data().clone().transform(transform) {
                    for seg in transformed.segments() {
                        match seg {
                            usvg::tiny_skia_path::PathSegment::MoveTo(pt) => pb.move_to(pt.x, pt.y),
                            usvg::tiny_skia_path::PathSegment::LineTo(pt) => pb.line_to(pt.x, pt.y),
                            usvg::tiny_skia_path::PathSegment::QuadTo(p1, p2) => {
                                pb.quad_to(p1.x, p1.y, p2.x, p2.y)
                            }
                            usvg::tiny_skia_path::PathSegment::CubicTo(p1, p2, p3) => {
                                pb.cubic_to(p1.x, p1.y, p2.x, p2.y, p3.x, p3.y)
                            }
                            usvg::tiny_skia_path::PathSegment::Close => pb.close(),
                        }
                    }
                }
            }
            usvg::Node::Group(g) => {
                push_group_paths_transformed(g, transform.pre_concat(g.transform()), pb);
            }
            usvg::Node::Text(txt) => {
                push_group_paths_transformed(txt.flattened(), transform, pb);
            }
            usvg::Node::Image(_) => {}
        }
    }
}

fn usvg_spread_to_krilla(sm: usvg::SpreadMethod) -> SpreadMethod {
    match sm {
        usvg::SpreadMethod::Pad => SpreadMethod::Pad,
        usvg::SpreadMethod::Reflect => SpreadMethod::Reflect,
        usvg::SpreadMethod::Repeat => SpreadMethod::Repeat,
    }
}

fn usvg_stops_to_krilla(stops: &[usvg::Stop]) -> Vec<Stop> {
    stops
        .iter()
        .map(|s| {
            let c = s.color();
            Stop {
                offset: norm_f32(s.offset().get() as f64),
                color: rgb::Color::new(c.red, c.green, c.blue).into(),
                opacity: norm_f32(s.opacity().get() as f64),
            }
        })
        .collect()
}

fn usvg_paint_to_krilla(paint: &usvg::Paint) -> Option<Paint> {
    match paint {
        usvg::Paint::Color(c) => Some(rgb::Color::new(c.red, c.green, c.blue).into()),
        usvg::Paint::LinearGradient(lg) => Some(
            LinearGradient {
                x1: lg.x1(),
                y1: lg.y1(),
                x2: lg.x2(),
                y2: lg.y2(),
                transform: usvg_transform_to_krilla(lg.transform()),
                spread_method: usvg_spread_to_krilla(lg.spread_method()),
                stops: usvg_stops_to_krilla(lg.stops()),
                anti_alias: true,
            }
            .into(),
        ),
        usvg::Paint::RadialGradient(rg) => Some(
            RadialGradient {
                fx: rg.fx(),
                fy: rg.fy(),
                fr: 0.0,
                cx: rg.cx(),
                cy: rg.cy(),
                cr: rg.r().get(),
                transform: usvg_transform_to_krilla(rg.transform()),
                spread_method: usvg_spread_to_krilla(rg.spread_method()),
                stops: usvg_stops_to_krilla(rg.stops()),
                anti_alias: true,
            }
            .into(),
        ),
        usvg::Paint::Pattern(_) => None,
    }
}

fn render_usvg_group_pdf(surface: &mut Surface<'_>, group: &usvg::Group) {
    let t = group.transform();
    let has_transform = !t.is_identity();
    if has_transform {
        surface.push_transform(&usvg_transform_to_krilla(t));
    }

    let mut pushed_clip = false;
    if let Some(clip) = group.clip_path() {
        let mut pb = PathBuilder::new();
        push_group_paths_transformed(clip.root(), clip.transform(), &mut pb);
        if let Some(clip_path) = pb.finish() {
            surface.push_clip_path(&clip_path, &FillRule::NonZero);
            pushed_clip = true;
        }
    }

    let opacity = group.opacity().get();
    let pushed_opacity = if opacity < 1.0 - 1e-6 {
        surface.push_opacity(norm_f32(opacity as f64));
        true
    } else {
        false
    };

    for child in group.children() {
        match child {
            usvg::Node::Group(sub_group) => render_usvg_group_pdf(surface, sub_group),
            usvg::Node::Path(path) => render_usvg_path_pdf(surface, path),
            usvg::Node::Text(text) => render_usvg_group_pdf(surface, text.flattened()),
            usvg::Node::Image(img) => render_usvg_embedded_image_pdf(surface, img),
        }
    }

    if pushed_opacity {
        surface.pop();
    }
    if pushed_clip {
        surface.pop();
    }
    if has_transform {
        surface.pop();
    }
}

fn render_usvg_path_pdf(surface: &mut Surface<'_>, path: &usvg::Path) {
    if !path.is_visible() {
        return;
    }
    let Some(kpath) = usvg_path_to_krilla(path.data()) else {
        return;
    };

    let build_fill = || -> Option<Fill> {
        let fill = path.fill()?;
        let paint = usvg_paint_to_krilla(fill.paint())?;
        let rule = match fill.rule() {
            usvg::FillRule::NonZero => FillRule::NonZero,
            usvg::FillRule::EvenOdd => FillRule::EvenOdd,
        };
        Some(Fill {
            paint,
            opacity: norm_f32(fill.opacity().get() as f64),
            rule,
        })
    };

    let build_stroke = || -> Option<Stroke> {
        let stroke = path.stroke()?;
        let paint = usvg_paint_to_krilla(stroke.paint())?;
        let line_cap = match stroke.linecap() {
            usvg::LineCap::Butt => LineCap::Butt,
            usvg::LineCap::Round => LineCap::Round,
            usvg::LineCap::Square => LineCap::Square,
        };
        let line_join = match stroke.linejoin() {
            usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => LineJoin::Miter,
            usvg::LineJoin::Round => LineJoin::Round,
            usvg::LineJoin::Bevel => LineJoin::Bevel,
        };
        let dash = stroke.dasharray().map(|arr| StrokeDash {
            array: arr.to_vec(),
            offset: stroke.dashoffset(),
        });
        Some(Stroke {
            paint,
            width: stroke.width().get(),
            miter_limit: stroke.miterlimit().get(),
            line_cap,
            line_join,
            opacity: norm_f32(stroke.opacity().get() as f64),
            dash,
        })
    };

    match path.paint_order() {
        usvg::PaintOrder::FillAndStroke => {
            if let Some(fill) = build_fill() {
                surface.set_fill(Some(fill));
                surface.set_stroke(None);
                surface.draw_path(&kpath);
            }
            if let Some(stroke) = build_stroke() {
                surface.set_fill(None);
                surface.set_stroke(Some(stroke));
                surface.draw_path(&kpath);
            }
        }
        usvg::PaintOrder::StrokeAndFill => {
            if let Some(stroke) = build_stroke() {
                surface.set_fill(None);
                surface.set_stroke(Some(stroke));
                surface.draw_path(&kpath);
            }
            if let Some(fill) = build_fill() {
                surface.set_fill(Some(fill));
                surface.set_stroke(None);
                surface.draw_path(&kpath);
            }
        }
    }
}

fn render_usvg_embedded_image_pdf(surface: &mut Surface<'_>, img: &usvg::Image) {
    if !img.is_visible() {
        return;
    }
    let sz = img.size();
    let Some(pdf_size) = Size::from_wh(sz.width(), sz.height()) else {
        return;
    };
    match img.kind() {
        usvg::ImageKind::SVG(sub_tree) => {
            let tw = sub_tree.size().width();
            let th = sub_tree.size().height();
            if tw > 0.0 && th > 0.0 {
                surface.push_transform(&Transform::from_scale(sz.width() / tw, sz.height() / th));
                render_usvg_group_pdf(surface, sub_tree.root());
                surface.pop();
            }
        }
        usvg::ImageKind::JPEG(bytes) => {
            if let Ok(kimg) = Image::from_jpeg(Data::from(Arc::clone(bytes)), true) {
                surface.draw_image(kimg, pdf_size);
            }
        }
        usvg::ImageKind::PNG(bytes) => {
            if let Ok(kimg) = Image::from_png(Data::from(Arc::clone(bytes)), true) {
                surface.draw_image(kimg, pdf_size);
            }
        }
        usvg::ImageKind::GIF(bytes) | usvg::ImageKind::WEBP(bytes) => {
            if let Ok(decoded) = image::load_from_memory(bytes) {
                let rgba = decoded.to_rgba8();
                let (iw, ih) = rgba.dimensions();
                if iw > 0 && ih > 0 {
                    let custom = UprightRgbaPdfImage::from_rgba8(iw, ih, rgba.as_raw());
                    if let Ok(kimg) = Image::from_custom(custom, true) {
                        surface.draw_image(kimg, pdf_size);
                    }
                }
            }
        }
    }
}
