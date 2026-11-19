use krilla::color::rgb;
use krilla::geom::{PathBuilder, Rect, Size, Transform};
use krilla::image::{BitsPerComponent, CustomImage, Image, ImageColorspace};
use krilla::mask::{Mask, MaskType};
use krilla::paint::{Fill, FillRule};
use krilla::surface::Surface;

use crate::compiler::value::Value;
use crate::render::color::parse_color_rgba8;

use super::lower::build_rect_path;
use super::rgba_to_krilla;

#[derive(Clone, Hash)]
struct GrayMaskImage {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

impl CustomImage for GrayMaskImage {
    fn color_channel(&self) -> &[u8] {
        &self.pixels
    }

    fn alpha_channel(&self) -> Option<&[u8]> {
        None
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
        ImageColorspace::Luma
    }
}

/// High-precision approximation of `erf(x)` (Abramowitz & Stegun 7.1.26, max error < 1.5e-7).
#[inline]
fn erf_approx(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let ax = x.abs();
    let t = 1.0 / (1.0 + 0.3275911 * ax);
    let y = 1.0
        - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t
            + 0.254829592)
            * t
            * (-ax * ax).exp();
    sign * y
}

/// Clamps per-corner radii identically to `kurbo::RoundedRect`.
#[inline]
pub(super) fn clamp_radii(w: f64, h: f64, radii: (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
    if w <= 0.0 || h <= 0.0 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let (mut tl, mut tr, mut br, mut bl) = (
        radii.0.max(0.0),
        radii.1.max(0.0),
        radii.2.max(0.0),
        radii.3.max(0.0),
    );
    let mut scale = 1.0f64;
    if tl + tr > w && tl + tr > 0.0 {
        scale = scale.min(w / (tl + tr));
    }
    if br + bl > w && br + bl > 0.0 {
        scale = scale.min(w / (br + bl));
    }
    if tl + bl > h && tl + bl > 0.0 {
        scale = scale.min(h / (tl + bl));
    }
    if tr + br > h && tr + br > 0.0 {
        scale = scale.min(h / (tr + br));
    }
    tl *= scale;
    tr *= scale;
    br *= scale;
    bl *= scale;
    (tl, tr, br, bl)
}

/// Returns the horizontal chord `[left_x, right_x]` of a rounded rectangle at vertical coordinate `y`,
/// or `None` if `y` lies outside `[y0, y1]`.
#[inline]
fn rounded_rect_scanline_span(
    y: f64,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    radii: (f64, f64, f64, f64),
) -> Option<(f64, f64)> {
    if y < y0 || y > y1 {
        return None;
    }
    let (tl, tr, br, bl) = radii;

    let left_inset = if y < y0 + tl && tl > 0.0 {
        let dy = (y0 + tl) - y;
        tl - (tl * tl - dy * dy).max(0.0).sqrt()
    } else if y > y1 - bl && bl > 0.0 {
        let dy = y - (y1 - bl);
        bl - (bl * bl - dy * dy).max(0.0).sqrt()
    } else {
        0.0
    };

    let right_inset = if y < y0 + tr && tr > 0.0 {
        let dy = (y0 + tr) - y;
        tr - (tr * tr - dy * dy).max(0.0).sqrt()
    } else if y > y1 - br && br > 0.0 {
        let dy = y - (y1 - br);
        br - (br * br - dy * dy).max(0.0).sqrt()
    } else {
        0.0
    };

    let lx = x0 + left_inset;
    let rx = x1 - right_inset;
    if rx > lx {
        Some((lx, rx))
    } else {
        None
    }
}

/// Draws a `\Shadow` synthesized behind a `\Rect` at `(x0, y0, x1, y1)`.
///
/// Zero-blur shadows emit pure vector paths; blurred shadows emit a compact
/// analytical 1D Gaussian `erf` `/SMask` luminosity image paired with a vector fill.
pub(super) fn draw_rect_shadow(
    surface: &mut Surface<'_>,
    properties: &std::collections::HashMap<String, Value>,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    rect_radii: (f64, f64, f64, f64),
) {
    let Some(Value::Color(sc) | Value::String(sc)) = properties.get("shadow_color") else {
        return;
    };
    let shadow_rgba = parse_color_rgba8(sc);
    if shadow_rgba.a == 0 {
        return;
    }

    let dx = properties
        .get("shadow_offset_x")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let dy = properties
        .get("shadow_offset_y")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let blur = properties
        .get("shadow_blur")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let spread = properties
        .get("shadow_spread")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    if blur <= 0.0 && spread == 0.0 && dx == 0.0 && dy == 0.0 {
        return;
    }

    let sx0 = x0 + dx - spread;
    let sy0 = y0 + dy - spread;
    let sx1 = x1 + dx + spread;
    let sy1 = y1 + dy + spread;
    let sw = sx1 - sx0;
    let sh = sy1 - sy0;
    if sw <= 0.0 || sh <= 0.0 {
        return;
    }

    let shadow_radii = clamp_radii(
        sw,
        sh,
        (
            (rect_radii.0 + spread).max(0.0),
            (rect_radii.1 + spread).max(0.0),
            (rect_radii.2 + spread).max(0.0),
            (rect_radii.3 + spread).max(0.0),
        ),
    );
    let std_dev = (blur * 0.5).max(0.0);
    let (shadow_rgb, shadow_alpha) = rgba_to_krilla(shadow_rgba);

    // Zero-blur drop shadow -> pure vector geometry
    if std_dev <= 0.0 {
        if let Some(path) = build_rect_path(sx0, sy0, sw, sh, shadow_radii) {
            surface.set_fill(Some(Fill {
                paint: shadow_rgb.into(),
                opacity: shadow_alpha,
                rule: FillRule::NonZero,
            }));
            surface.set_stroke(None);
            surface.draw_path(&path);
        }
        return;
    }

    // Blurred drop shadow -> analytical Gaussian erf /SMask + vector solid fill
    let pad = 3.0 * std_dev;
    let ox = sx0 - pad;
    let oy = sy0 - pad;
    let ow = sw + 2.0 * pad;
    let oh = sh + 2.0 * pad;
    if ow <= 0.0 || oh <= 0.0 {
        return;
    }

    let step = (std_dev / 3.0).clamp(0.5, 2.0);
    let gw = ((ow / step).ceil() as usize).clamp(4, 256);
    let gh = ((oh / step).ceil() as usize).clamp(4, 256);

    let inv_s2 = 1.0 / (std::f64::consts::SQRT_2 * std_dev);
    let has_radii = shadow_radii.0 > 0.0
        || shadow_radii.1 > 0.0
        || shadow_radii.2 > 0.0
        || shadow_radii.3 > 0.0;

    let mut pixels = vec![0u8; gw * gh];

    if !has_radii {
        // Exact closed-form separable 1D Gaussian convolution of an axis-aligned box.
        let mut wx = vec![0.0f64; gw];
        for (px, slot) in wx.iter_mut().enumerate() {
            let x = ox + ((px as f64 + 0.5) / gw as f64) * ow;
            *slot = 0.5 * (erf_approx((x - sx0) * inv_s2) - erf_approx((x - sx1) * inv_s2));
        }
        for py in 0..gh {
            let y = oy + ((py as f64 + 0.5) / gh as f64) * oh;
            let wy = 0.5 * (erf_approx((y - sy0) * inv_s2) - erf_approx((y - sy1) * inv_s2));
            let row_off = py * gw;
            for px in 0..gw {
                let cov = (wx[px] * wy).clamp(0.0, 1.0);
                pixels[row_off + px] = (cov * 255.0).round() as u8;
            }
        }
    } else {
        // Exact horizontal erf scanline integral + symmetric 1D Gaussian vertical quadrature.
        const QUAD_STEPS: usize = 25;
        let inv_2sig2 = 1.0 / (2.0 * std_dev * std_dev);
        let mut row_acc = vec![0.0f64; gw];
        for py in 0..gh {
            let y = oy + ((py as f64 + 0.5) / gh as f64) * oh;
            let y_min = (y - pad).max(sy0);
            let y_max = (y + pad).min(sy1);
            if y_max <= y_min {
                continue;
            }
            let dy_step = (y_max - y_min) / QUAD_STEPS as f64;
            let row_off = py * gw;
            row_acc.fill(0.0);

            let norm_factor = (dy_step / (std::f64::consts::TAU.sqrt() * std_dev)) * 0.5;
            for q in 0..QUAD_STEPS {
                let ys = y_min + (q as f64 + 0.5) * dy_step;
                let Some((lx, rx)) =
                    rounded_rect_scanline_span(ys, sx0, sy0, sx1, sy1, shadow_radii)
                else {
                    continue;
                };
                let diff = ys - y;
                let w_y = (-diff * diff * inv_2sig2).exp() * norm_factor;
                if w_y <= 1e-6 {
                    continue;
                }
                for px in 0..gw {
                    let x = ox + ((px as f64 + 0.5) / gw as f64) * ow;
                    let hx = erf_approx((x - lx) * inv_s2) - erf_approx((x - rx) * inv_s2);
                    row_acc[px] += hx * w_y;
                }
            }
            for px in 0..gw {
                pixels[row_off + px] = (row_acc[px].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
    }

    let mask_image = match Image::from_custom(
        GrayMaskImage {
            pixels,
            width: gw as u32,
            height: gh as u32,
        },
        true,
    ) {
        Ok(img) => img,
        Err(_) => return,
    };

    let Some(mask_size) = Size::from_wh(ow as f32, oh as f32) else {
        return;
    };
    let Some(mask_rect) = Rect::from_xywh(ox as f32, oy as f32, ow as f32, oh as f32) else {
        return;
    };
    let mut mask_pb = PathBuilder::new();
    mask_pb.push_rect(mask_rect);
    let Some(mask_path) = mask_pb.finish() else {
        return;
    };

    let mut sb = surface.stream_builder();
    {
        let mut mask_surface = sb.surface();
        mask_surface.push_transform(&Transform::from_translate(ox as f32, oy as f32));
        mask_surface.draw_image(mask_image, mask_size);
        mask_surface.pop();
        mask_surface.finish();
    }
    let mask = Mask::new(sb.finish(), MaskType::Luminosity);

    surface.push_mask(mask);
    surface.set_fill(Some(Fill {
        paint: rgb::Color::new(shadow_rgb.red(), shadow_rgb.green(), shadow_rgb.blue()).into(),
        opacity: shadow_alpha,
        rule: FillRule::NonZero,
    }));
    surface.set_stroke(None);
    surface.draw_path(&mask_path);
    surface.pop();
}
