// DirectedType Standard Component Library: Book Companion
// Native pre-layout numbering pass for \Book hierarchies (\Chapter, \H1, \H2, \H3, \Figure, \Footnote, \Sidenote).

use directedtype::prelude::*;

#[derive(Default, Debug, Clone)]
pub struct Book;

impl Book {
    pub fn new() -> Self {
        Self
    }
}

#[component]
impl Book {
    pub fn on_prepare(&mut self, ctx: &mut PrepareContext<'_>) -> Result<(), DispatchError> {
        let book_handle = ctx.self_handle();
        let reset_fn_per_ch = ctx
            .get_port_literal(book_handle, "reset_footnotes_per_chapter")?
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let reset_fig_per_ch = ctx
            .get_port_literal(book_handle, "reset_figures_per_chapter")?
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let nodes = ctx.query_by_tag(
            Some(book_handle),
            &["Chapter", "H1", "H2", "H3", "Figure", "Footnote", "Sidenote"],
        )?;

        let mut chapter: i64 = 0;
        let mut h1: i64 = 0;
        let mut h2: i64 = 0;
        let mut h3: i64 = 0;
        let mut figure: i64 = 0;
        let mut footnote: i64 = 0;
        let mut sidenote: i64 = 0;

        for node in nodes {
            let unnumbered = ctx
                .get_port_literal(node, "unnumbered")?
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if unnumbered {
                continue;
            }

            let tag = ctx.tag_name(node)?;
            match tag.as_str() {
                "Chapter" => {
                    chapter += 1;
                    h1 = 0;
                    h2 = 0;
                    h3 = 0;
                    if reset_fig_per_ch {
                        figure = 0;
                    }
                    if reset_fn_per_ch {
                        footnote = 0;
                    }
                    set_if_absent(ctx, node, "number", GuestValue::String(format!("{}", chapter)))?;
                }
                "H1" => {
                    h1 += 1;
                    h2 = 0;
                    h3 = 0;
                    let label = if chapter > 0 {
                        format!("{}.{}", chapter, h1)
                    } else {
                        format!("{}", h1)
                    };
                    set_if_absent(ctx, node, "number", GuestValue::String(label))?;
                }
                "H2" => {
                    h2 += 1;
                    h3 = 0;
                    let label = if chapter > 0 {
                        format!("{}.{}.{}", chapter, h1.max(1), h2)
                    } else {
                        format!("{}.{}", h1.max(1), h2)
                    };
                    set_if_absent(ctx, node, "number", GuestValue::String(label))?;
                }
                "H3" => {
                    h3 += 1;
                    let label = if chapter > 0 {
                        format!("{}.{}.{}.{}", chapter, h1.max(1), h2.max(1), h3)
                    } else {
                        format!("{}.{}.{}", h1.max(1), h2.max(1), h3)
                    };
                    set_if_absent(ctx, node, "number", GuestValue::String(label))?;
                }
                "Figure" => {
                    figure += 1;
                    let label = if chapter > 0 {
                        format!("Figure {}.{}", chapter, figure)
                    } else {
                        format!("Figure {}", figure)
                    };
                    set_if_absent(ctx, node, "number", GuestValue::String(label))?;
                }
                "Footnote" => {
                    footnote += 1;
                    set_if_absent(
                        ctx,
                        node,
                        "marker",
                        GuestValue::String(format!("{}", footnote)),
                    )?;
                }
                "Sidenote" => {
                    sidenote += 1;
                    set_if_absent(
                        ctx,
                        node,
                        "marker",
                        GuestValue::String(format!("{}", sidenote)),
                    )?;
                }
                _ => {}
            }
        }

        Ok(())
    }
}

fn set_if_absent(
    ctx: &mut PrepareContext<'_>,
    node: GuestNodeHandle,
    port: &str,
    val: GuestValue,
) -> Result<(), DispatchError> {
    if ctx.get_port_literal(node, port)?.is_none() {
        ctx.set_port_literal(node, port, val)?;
    }
    Ok(())
}
