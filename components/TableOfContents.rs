// DirectedType Standard Component Library: TableOfContents Companion
// Pre-order Component DOM scanner wrapping headings in deterministic `\Anchor`s and appending `\TocEntry` rows.

use directedtype::prelude::*;
use std::collections::HashSet;

#[derive(Default, Debug, Clone)]
pub struct TableOfContents;

impl TableOfContents {
    pub fn new() -> Self {
        Self
    }
}

#[component]
impl TableOfContents {
    pub fn on_prepare(&mut self, ctx: &mut PrepareContext<'_>) -> Result<(), DispatchError> {
        let toc_handle = ctx.self_handle();
        let max_depth = ctx
            .get_port_literal(toc_handle, "max_depth")?
            .and_then(|v| v.as_i64())
            .unwrap_or(2);

        let headings = ctx.query_by_tag(None, &["Chapter", "Section", "H1", "H2", "H3"])?;
        let mut has_chapters = false;
        for &heading in &headings {
            if !ctx.ancestors(heading)?.contains(&toc_handle)
                && ctx.tag_name(heading)? == "Chapter"
            {
                has_chapters = true;
                break;
            }
        }

        let mut used_slugs = HashSet::new();

        for heading in headings {
            if ctx.ancestors(heading)?.contains(&toc_handle) {
                continue;
            }

            let tag = ctx.tag_name(heading)?;
            let level: i64 = match tag.as_str() {
                "Chapter" => 1,
                "Section" | "H1" => {
                    if has_chapters {
                        2
                    } else {
                        1
                    }
                }
                "H2" => {
                    if has_chapters {
                        3
                    } else {
                        2
                    }
                }
                "H3" => {
                    if has_chapters {
                        4
                    } else {
                        3
                    }
                }
                _ => continue,
            };
            if level > max_depth {
                continue;
            }

            let title = ctx
                .get_port_literal(heading, "title")?
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_else(|| ctx.get_subtree_text(heading).unwrap_or_default());

            let number = ctx
                .get_port_literal(heading, "number")?
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();

            let anchor_handle = ensure_heading_anchor(ctx, heading, &title, level, &mut used_slugs)?;

            let entry = ctx.create_element(
                "TocEntry",
                None,
                &[
                    ("number", GuestValue::String(number)),
                    ("title", GuestValue::String(title)),
                    ("level", GuestValue::Integer(level)),
                ],
            )?;
            ctx.set_port_node_ref(entry, "target", anchor_handle)?;
            ctx.append_child(toc_handle, entry)?;
        }

        Ok(())
    }
}

fn ensure_heading_anchor(
    ctx: &mut PrepareContext<'_>,
    heading: GuestNodeHandle,
    title: &str,
    level: i64,
    used_slugs: &mut HashSet<String>,
) -> Result<GuestNodeHandle, DispatchError> {
    if let Some(parent) = ctx.parent(heading)? {
        if ctx.tag_name(parent)? == "Anchor" {
            if let Some(existing) = ctx
                .get_port_literal(parent, "name")?
                .and_then(|v| v.as_str().map(str::to_string))
            {
                used_slugs.insert(existing);
            }
            return Ok(parent);
        }
    }

    let base = format!("toc-{}-{}", level, slugify(title));
    let mut slug = base.clone();
    let mut suffix = 2usize;
    while !used_slugs.insert(slug.clone()) {
        slug = format!("{}-{}", base, suffix);
        suffix += 1;
    }

    let anchor = ctx.create_element("Anchor", None, &[("name", GuestValue::String(slug))])?;
    ctx.wrap_node(heading, anchor)?;
    Ok(anchor)
}

fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in s.chars().flat_map(|c| c.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "section".to_string()
    } else {
        out
    }
}
