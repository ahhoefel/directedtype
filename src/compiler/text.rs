use parley::style::{FontFamily, FontWeight, StyleProperty};
use parley::{FontContext, LayoutContext};
use std::cell::RefCell;

thread_local! {
    static FONT_CONTEXT: RefCell<FontContext> = RefCell::new(FontContext::new());
    static LAYOUT_CONTEXT: RefCell<LayoutContext<()>> = RefCell::new(LayoutContext::new());
}

/// Measures the rendered height of the given text when wrapped to `max_width`.
pub fn measure_text_height(
    text: &str,
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
    max_width: f64,
) -> f64 {
    let (_, height) = measure_text_bounds(text, font_size, font_weight, font_family, Some(max_width));
    height
}

/// Measures the natural unconstrained single-line width of the given text.
pub fn measure_text_width(
    text: &str,
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
) -> f64 {
    let (width, _) = measure_text_bounds(text, font_size, font_weight, font_family, None);
    width
}

/// Measures the layout boundaries `(width, height)` of the given text using Parley.
pub fn measure_text_bounds(
    text: &str,
    font_size: f64,
    font_weight: f64,
    font_family: Option<&str>,
    max_width: Option<f64>,
) -> (f64, f64) {
    if text.is_empty() {
        return (0.0, 0.0);
    }

    let size = if font_size > 0.0 { font_size as f32 } else { 16.0 };
    let weight = if font_weight > 0.0 { font_weight as f32 } else { 400.0 };

    FONT_CONTEXT.with(|font_cx_cell| {
        LAYOUT_CONTEXT.with(|layout_cx_cell| {
            let mut font_cx = font_cx_cell.borrow_mut();
            let mut layout_cx = layout_cx_cell.borrow_mut();

            let mut builder = layout_cx.ranged_builder(&mut font_cx, text, 1.0, true);
            builder.push_default(StyleProperty::FontSize(size));
            if (weight - 400.0).abs() > 1.0 {
                builder.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
            }
            if let Some(family) = font_family {
                if !family.is_empty() {
                    builder.push_default(StyleProperty::FontFamily(FontFamily::named(family)));
                }
            }

            let mut layout = builder.build(text);
            layout.break_all_lines(max_width.and_then(|w| if w > 0.0 { Some(w as f32) } else { None }));

            let width = if max_width.is_some() {
                layout.width() as f64
            } else {
                layout.full_width() as f64
            };

            (width, layout.height() as f64)
        })
    })
}
