use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use vello::kurbo::{
    Affine, BezPath, Cap, Dashes, Join, Point, Rect, RoundedRect, RoundedRectRadii, Stroke,
};
use vello::peniko::{
    BlendMode, Blob, Brush, Color, ColorStop, Extend, Fill, Gradient, ImageAlphaType, ImageBrush,
    ImageData, ImageFormat as PenikoImageFormat, Mix,
};
use vello::Scene;

use crate::compiler::image::{lookup_loaded_image, DecodedImageData};
use crate::compiler::layout::{extract_corner_radii, ResolvedLayout, ResolvedNode};
use crate::compiler::value::Value;
use crate::render::color::parse_color;

static VELLO_SVG_CACHE: OnceLock<RwLock<HashMap<(String, u64), Arc<Scene>>>> = OnceLock::new();

/// Clears any cached `vello::Scene` for a modified SVG path on hot-reload.
pub fn invalidate_vello_svg_cache(path: &std::path::Path) {
    let norm = crate::compiler::module::normalize_path(path)
        .to_string_lossy()
        .to_string();
    let canon = std::fs::canonicalize(path)
        .ok()
        .map(|p| p.to_string_lossy().to_string());
    if let Some(lock) = VELLO_SVG_CACHE.get() {
        if let Ok(mut map) = lock.write() {
            map.retain(|(k, _), _| k != &norm && canon.as_deref() != Some(k.as_str()));
        }
    }
}

/// Renders an `\Image` paint primitive into `active_scene`.
pub fn draw_image_node(
    active_scene: &mut Scene,
    node: &ResolvedNode,
    layout: &ResolvedLayout,
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

    let x0 = node.rect.x;
    let y0 = node.rect.y;
    let x1 = x0 + node.rect.width;
    let y1 = y0 + node.rect.height;

    let (tl, tr, br, bl) = extract_corner_radii(&node.properties);
    let has_radius = tl > 0.0 || tr > 0.0 || br > 0.0 || bl > 0.0;
    let radii = RoundedRectRadii::new(tl, tr, br, bl);

    // 1. Drop shadow (matching \Rect)
    if let Some(Value::Color(sc) | Value::String(sc)) = node.properties.get("shadow_color") {
        let shadow_color = parse_color(sc);
        if shadow_color.components[3] > 0.0 {
            let dx = node
                .properties
                .get("shadow_offset_x")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let dy = node
                .properties
                .get("shadow_offset_y")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let blur = node
                .properties
                .get("shadow_blur")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let spread = node
                .properties
                .get("shadow_spread")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);

            if blur > 0.0 || spread != 0.0 || dx != 0.0 || dy != 0.0 {
                let sx0 = x0 + dx - spread;
                let sy0 = y0 + dy - spread;
                let sx1 = x1 + dx + spread;
                let sy1 = y1 + dy + spread;
                if sx1 > sx0 && sy1 > sy0 {
                    let base_radius = tl.max(tr).max(br).max(bl);
                    let shadow_radius = (base_radius + spread).max(0.0);
                    let std_dev = (blur * 0.5).max(0.0);
                    let shadow_rect = Rect::new(sx0, sy0, sx1, sy1);
                    if std_dev > 0.0 {
                        active_scene.draw_blurred_rounded_rect(
                            Affine::IDENTITY,
                            shadow_rect,
                            shadow_color,
                            shadow_radius,
                            std_dev,
                        );
                    } else if shadow_radius > 0.0 {
                        let rrect = RoundedRect::new(sx0, sy0, sx1, sy1, shadow_radius);
                        active_scene.fill(
                            Fill::NonZero,
                            Affine::IDENTITY,
                            Brush::Solid(shadow_color),
                            None,
                            &rrect,
                        );
                    } else {
                        active_scene.fill(
                            Fill::NonZero,
                            Affine::IDENTITY,
                            Brush::Solid(shadow_color),
                            None,
                            &shadow_rect,
                        );
                    }
                }
            }
        }
    }

    let opacity = node
        .properties
        .get("opacity")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0)
        .clamp(0.0, 1.0) as f32;

    if opacity > 0.0 {
        let is_svg = matches!(asset.decoded, DecodedImageData::Svg { .. });
        let needs_layer = has_radius || is_svg || opacity < 1.0 - 1e-6;

        if needs_layer {
            if opacity < 1.0 - 1e-6 {
                if has_radius {
                    let rrect = RoundedRect::new(x0, y0, x1, y1, radii);
                    active_scene.push_layer(
                        Fill::NonZero,
                        BlendMode::default(),
                        opacity,
                        Affine::IDENTITY,
                        &rrect,
                    );
                } else {
                    let rect = Rect::new(x0, y0, x1, y1);
                    active_scene.push_layer(
                        Fill::NonZero,
                        BlendMode::default(),
                        opacity,
                        Affine::IDENTITY,
                        &rect,
                    );
                }
            } else if has_radius {
                let rrect = RoundedRect::new(x0, y0, x1, y1, radii);
                active_scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rrect);
            } else {
                let rect = Rect::new(x0, y0, x1, y1);
                active_scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rect);
            }
        }

        match &asset.decoded {
            DecodedImageData::Raster {
                width,
                height,
                rgba8,
                ..
            } => {
                if *width > 0 && *height > 0 {
                    let raw_dyn: Arc<dyn AsRef<[u8]> + Send + Sync> = rgba8.clone();
                    let image_data = ImageData {
                        data: Blob::new(raw_dyn),
                        format: PenikoImageFormat::Rgba8,
                        alpha_type: ImageAlphaType::Alpha,
                        width: *width,
                        height: *height,
                    };
                    let sx = node.rect.width / (*width as f64);
                    let sy = node.rect.height / (*height as f64);
                    let transform =
                        Affine::translate((x0, y0)) * Affine::scale_non_uniform(sx, sy);
                    active_scene.draw_image(&ImageBrush::new(image_data), transform);
                }
            }
            DecodedImageData::Svg { tree, .. } => {
                let tree_size = tree.size();
                let tw = tree_size.width() as f64;
                let th = tree_size.height() as f64;
                if tw > 0.0 && th > 0.0 {
                    let svg_scene = get_or_build_svg_scene(&asset.path, asset.content_hash, tree);
                    let sx = node.rect.width / tw;
                    let sy = node.rect.height / th;
                    let transform =
                        Affine::translate((x0, y0)) * Affine::scale_non_uniform(sx, sy);
                    active_scene.append(&svg_scene, Some(transform));
                }
            }
        }

        if needs_layer {
            active_scene.pop_layer();
        }
    }

    // 3. Optional border stroke on top (matching \Rect)
    let stroke_width = node
        .properties
        .get("border_width")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    if stroke_width > 0.0 && opacity > 0.0 {
        let mut stroke_color = node
            .properties
            .get("border_color")
            .map(|v| match v {
                Value::Color(c) | Value::String(c) => parse_color(c),
                _ => Color::BLACK,
            })
            .unwrap_or(Color::BLACK);
        if opacity < 1.0 - 1e-6 {
            stroke_color = stroke_color.multiply_alpha(opacity);
        }

        let stroke = Stroke::new(stroke_width);
        if has_radius {
            let rrect = RoundedRect::new(x0, y0, x1, y1, radii);
            active_scene.stroke(
                &stroke,
                Affine::IDENTITY,
                Brush::Solid(stroke_color),
                None,
                &rrect,
            );
        } else {
            let rect = Rect::new(x0, y0, x1, y1);
            active_scene.stroke(
                &stroke,
                Affine::IDENTITY,
                Brush::Solid(stroke_color),
                None,
                &rect,
            );
        }
    }
}

fn get_or_build_svg_scene(path: &str, content_hash: u64, tree: &usvg::Tree) -> Arc<Scene> {
    let lock = VELLO_SVG_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    let key = (path.to_string(), content_hash);
    if let Ok(map) = lock.read() {
        if let Some(cached) = map.get(&key) {
            return Arc::clone(cached);
        }
    }

    let mut scene = Scene::new();
    render_usvg_group(&mut scene, tree.root(), Affine::IDENTITY);
    let arc = Arc::new(scene);

    if let Ok(mut map) = lock.write() {
        map.insert(key, Arc::clone(&arc));
    }
    arc
}

fn usvg_transform_to_affine(t: usvg::Transform) -> Affine {
    Affine::new([
        t.sx as f64,
        t.ky as f64,
        t.kx as f64,
        t.sy as f64,
        t.tx as f64,
        t.ty as f64,
    ])
}

fn usvg_path_to_bezpath(path: &usvg::tiny_skia_path::Path) -> BezPath {
    let mut bez = BezPath::new();
    for seg in path.segments() {
        match seg {
            usvg::tiny_skia_path::PathSegment::MoveTo(p) => {
                bez.move_to((p.x as f64, p.y as f64));
            }
            usvg::tiny_skia_path::PathSegment::LineTo(p) => {
                bez.line_to((p.x as f64, p.y as f64));
            }
            usvg::tiny_skia_path::PathSegment::QuadTo(p1, p2) => {
                bez.quad_to((p1.x as f64, p1.y as f64), (p2.x as f64, p2.y as f64));
            }
            usvg::tiny_skia_path::PathSegment::CubicTo(p1, p2, p3) => {
                bez.curve_to(
                    (p1.x as f64, p1.y as f64),
                    (p2.x as f64, p2.y as f64),
                    (p3.x as f64, p3.y as f64),
                );
            }
            usvg::tiny_skia_path::PathSegment::Close => {
                bez.close_path();
            }
        }
    }
    bez
}

fn usvg_blend_to_peniko(bm: usvg::BlendMode) -> BlendMode {
    let mix = match bm {
        usvg::BlendMode::Normal => Mix::Normal,
        usvg::BlendMode::Multiply => Mix::Multiply,
        usvg::BlendMode::Screen => Mix::Screen,
        usvg::BlendMode::Overlay => Mix::Overlay,
        usvg::BlendMode::Darken => Mix::Darken,
        usvg::BlendMode::Lighten => Mix::Lighten,
        usvg::BlendMode::ColorDodge => Mix::ColorDodge,
        usvg::BlendMode::ColorBurn => Mix::ColorBurn,
        usvg::BlendMode::HardLight => Mix::HardLight,
        usvg::BlendMode::SoftLight => Mix::SoftLight,
        usvg::BlendMode::Difference => Mix::Difference,
        usvg::BlendMode::Exclusion => Mix::Exclusion,
        usvg::BlendMode::Hue => Mix::Hue,
        usvg::BlendMode::Saturation => Mix::Saturation,
        usvg::BlendMode::Color => Mix::Color,
        usvg::BlendMode::Luminosity => Mix::Luminosity,
    };
    BlendMode::from(mix)
}

fn usvg_spread_to_extend(sm: usvg::SpreadMethod) -> Extend {
    match sm {
        usvg::SpreadMethod::Pad => Extend::Pad,
        usvg::SpreadMethod::Reflect => Extend::Reflect,
        usvg::SpreadMethod::Repeat => Extend::Repeat,
    }
}

fn usvg_stops_to_peniko(stops: &[usvg::Stop], opacity_mul: f32) -> Vec<ColorStop> {
    stops
        .iter()
        .map(|s| {
            let c = s.color();
            let a = (s.opacity().get() * opacity_mul).clamp(0.0, 1.0);
            let alpha_u8 = (a * 255.0).round() as u8;
            ColorStop::from((
                s.offset().get(),
                Color::from_rgba8(c.red, c.green, c.blue, alpha_u8),
            ))
        })
        .collect()
}

fn usvg_paint_to_brush(paint: &usvg::Paint, opacity: f32) -> Option<(Brush, Option<Affine>)> {
    match paint {
        usvg::Paint::Color(c) => {
            let alpha_u8 = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
            Some((
                Brush::Solid(Color::from_rgba8(c.red, c.green, c.blue, alpha_u8)),
                None,
            ))
        }
        usvg::Paint::LinearGradient(lg) => {
            let stops = usvg_stops_to_peniko(lg.stops(), opacity);
            let grad = Gradient::new_linear(
                Point::new(lg.x1() as f64, lg.y1() as f64),
                Point::new(lg.x2() as f64, lg.y2() as f64),
            )
            .with_extend(usvg_spread_to_extend(lg.spread_method()))
            .with_stops(stops.as_slice());
            Some((
                Brush::Gradient(grad),
                Some(usvg_transform_to_affine(lg.transform())),
            ))
        }
        usvg::Paint::RadialGradient(rg) => {
            let stops = usvg_stops_to_peniko(rg.stops(), opacity);
            let grad = Gradient::new_two_point_radial(
                Point::new(rg.fx() as f64, rg.fy() as f64),
                0.0,
                Point::new(rg.cx() as f64, rg.cy() as f64),
                rg.r().get(),
            )
            .with_extend(usvg_spread_to_extend(rg.spread_method()))
            .with_stops(stops.as_slice());
            Some((
                Brush::Gradient(grad),
                Some(usvg_transform_to_affine(rg.transform())),
            ))
        }
        usvg::Paint::Pattern(_) => None,
    }
}

fn collect_clip_bezpath(clip: &usvg::ClipPath, base_transform: Affine, out: &mut BezPath) {
    let t = base_transform * usvg_transform_to_affine(clip.transform());
    collect_group_paths_into_bezpath(clip.root(), t, out);
}

fn collect_group_paths_into_bezpath(group: &usvg::Group, transform: Affine, out: &mut BezPath) {
    for child in group.children() {
        match child {
            usvg::Node::Path(p) => {
                let mut bp = usvg_path_to_bezpath(p.data());
                bp.apply_affine(transform);
                out.extend(bp);
            }
            usvg::Node::Group(g) => {
                collect_group_paths_into_bezpath(
                    g,
                    transform * usvg_transform_to_affine(g.transform()),
                    out,
                );
            }
            usvg::Node::Text(txt) => {
                collect_group_paths_into_bezpath(txt.flattened(), transform, out);
            }
            usvg::Node::Image(_) => {}
        }
    }
}

fn render_usvg_group(scene: &mut Scene, group: &usvg::Group, parent_transform: Affine) {
    let transform = parent_transform * usvg_transform_to_affine(group.transform());
    let opacity = group.opacity().get();
    let blend = group.blend_mode();
    let has_clip = group.clip_path().is_some();
    let needs_group_layer = has_clip || opacity < 1.0 - 1e-6 || blend != usvg::BlendMode::Normal;

    let mut pushed_clip = false;
    if needs_group_layer {
        if let Some(clip) = group.clip_path() {
            let mut clip_bez = BezPath::new();
            collect_clip_bezpath(clip, transform, &mut clip_bez);
            scene.push_layer(
                Fill::NonZero,
                usvg_blend_to_peniko(blend),
                opacity,
                Affine::IDENTITY,
                &clip_bez,
            );
            pushed_clip = true;
        } else {
            let lb = group.layer_bounding_box();
            let rect = Rect::new(
                lb.left() as f64,
                lb.top() as f64,
                lb.right() as f64,
                lb.bottom() as f64,
            );
            scene.push_layer(
                Fill::NonZero,
                usvg_blend_to_peniko(blend),
                opacity,
                transform,
                &rect,
            );
            pushed_clip = true;
        }
    }

    for child in group.children() {
        match child {
            usvg::Node::Group(sub_group) => {
                render_usvg_group(scene, sub_group, transform);
            }
            usvg::Node::Path(path) => {
                render_usvg_path(scene, path, transform);
            }
            usvg::Node::Text(text) => {
                render_usvg_group(scene, text.flattened(), transform);
            }
            usvg::Node::Image(img) => {
                render_usvg_embedded_image(scene, img, transform);
            }
        }
    }

    if pushed_clip {
        scene.pop_layer();
    }
}

fn render_usvg_path(scene: &mut Scene, path: &usvg::Path, transform: Affine) {
    if !path.is_visible() {
        return;
    }
    let bez = usvg_path_to_bezpath(path.data());

    let do_fill = |scene: &mut Scene| {
        if let Some(fill) = path.fill() {
            let fill_rule = match fill.rule() {
                usvg::FillRule::NonZero => Fill::NonZero,
                usvg::FillRule::EvenOdd => Fill::EvenOdd,
            };
            if let Some((brush, brush_xform)) = usvg_paint_to_brush(fill.paint(), fill.opacity().get()) {
                scene.fill(fill_rule, transform, &brush, brush_xform, &bez);
            }
        }
    };

    let do_stroke = |scene: &mut Scene| {
        if let Some(stroke) = path.stroke() {
            let cap = match stroke.linecap() {
                usvg::LineCap::Butt => Cap::Butt,
                usvg::LineCap::Round => Cap::Round,
                usvg::LineCap::Square => Cap::Square,
            };
            let join = match stroke.linejoin() {
                usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => Join::Miter,
                usvg::LineJoin::Round => Join::Round,
                usvg::LineJoin::Bevel => Join::Bevel,
            };
            let mut kstroke = Stroke::new(stroke.width().get() as f64)
                .with_caps(cap)
                .with_join(join)
                .with_miter_limit(stroke.miterlimit().get() as f64);
            if let Some(dashes) = stroke.dasharray() {
                let d: Dashes = dashes.iter().map(|&v| v as f64).collect();
                kstroke = kstroke.with_dashes(stroke.dashoffset() as f64, d);
            }
            if let Some((brush, brush_xform)) =
                usvg_paint_to_brush(stroke.paint(), stroke.opacity().get())
            {
                scene.stroke(&kstroke, transform, &brush, brush_xform, &bez);
            }
        }
    };

    match path.paint_order() {
        usvg::PaintOrder::FillAndStroke => {
            do_fill(scene);
            do_stroke(scene);
        }
        usvg::PaintOrder::StrokeAndFill => {
            do_stroke(scene);
            do_fill(scene);
        }
    }
}

fn render_usvg_embedded_image(scene: &mut Scene, img: &usvg::Image, transform: Affine) {
    if !img.is_visible() {
        return;
    }
    let size = img.size();
    let w = size.width() as f64;
    let h = size.height() as f64;
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    match img.kind() {
        usvg::ImageKind::SVG(sub_tree) => {
            let tw = sub_tree.size().width() as f64;
            let th = sub_tree.size().height() as f64;
            if tw > 0.0 && th > 0.0 {
                let sub_xform = transform * Affine::scale_non_uniform(w / tw, h / th);
                render_usvg_group(scene, sub_tree.root(), sub_xform);
            }
        }
        usvg::ImageKind::JPEG(bytes) | usvg::ImageKind::PNG(bytes) | usvg::ImageKind::GIF(bytes) | usvg::ImageKind::WEBP(bytes) => {
            if let Ok(decoded) = image::load_from_memory(bytes) {
                let rgba = decoded.to_rgba8();
                let (iw, ih) = rgba.dimensions();
                if iw > 0 && ih > 0 {
                    let raw: Arc<Vec<u8>> = Arc::new(rgba.into_raw());
                    let img_data = ImageData {
                        data: Blob::new(raw),
                        format: PenikoImageFormat::Rgba8,
                        alpha_type: ImageAlphaType::Alpha,
                        width: iw,
                        height: ih,
                    };
                    let sub_xform =
                        transform * Affine::scale_non_uniform(w / (iw as f64), h / (ih as f64));
                    scene.draw_image(&ImageBrush::new(img_data), sub_xform);
                }
            }
        }
    }
}
