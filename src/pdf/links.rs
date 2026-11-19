use std::collections::HashMap;
use std::sync::Arc;

use krilla::action::{Action, LinkAction};
use krilla::annotation::{Annotation, LinkAnnotation, Target};
use krilla::destination::{Destination, XyzDestination};
use krilla::geom::{Point, Quadrilateral, Rect as KrillaRect};
use krilla::page::Page;

use crate::compiler::expanded::NodeId;
use crate::compiler::layout::{Rect, ResolvedLayout, ResolvedNode};

use super::lower::get_clip_chain;
use super::{PageScene, TextSliceSpec};

/// Resolved physical destination inside the emitted PDF.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnchorTarget {
    pub page_index: usize,
    pub x: f32,
    pub y: f32,
}

/// Pre-indexed map of all anchor targets across all pages in the document.
#[derive(Debug, Default)]
pub struct AnchorIndex {
    pub by_layout_node: HashMap<(usize, NodeId), AnchorTarget>,
    pub by_node_id: HashMap<NodeId, AnchorTarget>,
    pub by_any_node: HashMap<NodeId, AnchorTarget>,
    pub by_name: HashMap<String, AnchorTarget>,
}

impl AnchorIndex {
    pub fn resolve(
        &self,
        layout: &Arc<ResolvedLayout>,
        from_node: NodeId,
        url: &str,
    ) -> Option<AnchorTarget> {
        let layout_key = Arc::as_ptr(layout) as usize;
        if let Some((target_id, _scope_id)) = layout.resolve_anchor(from_node, url) {
            if let Some(hit) = self.by_layout_node.get(&(layout_key, target_id)) {
                return Some(*hit);
            }
            if let Some(hit) = self.by_node_id.get(&target_id) {
                return Some(*hit);
            }
        }
        let bare = url.strip_prefix('#').unwrap_or(url).trim_start_matches('/');
        self.by_name.get(bare).copied()
    }

    pub fn resolve_node(&self, layout: &Arc<ResolvedLayout>, target_id: NodeId) -> Option<AnchorTarget> {
        let layout_key = Arc::as_ptr(layout) as usize;
        if let Some(hit) = self.by_layout_node.get(&(layout_key, target_id)) {
            return Some(*hit);
        }
        if let Some(hit) = self.by_node_id.get(&target_id) {
            return Some(*hit);
        }
        self.by_any_node.get(&target_id).copied()
    }
}

/// Computes the visual `(x, y)` destination of an anchor or bookmark node on a page.
pub(super) fn node_page_position(page_scene: &PageScene, node: &ResolvedNode) -> (f32, f32) {
    let layout = &page_scene.layout;
    let node_off = page_scene.node_offsets.get(&node.id).copied().unwrap_or((0.0, 0.0));
    let (sx, sy) = layout.clip_scroll_offset(node.clip);
    let (raw_x, raw_y) = if !node.fragments.is_empty() {
        (node.fragments[0].x, node.fragments[0].y)
    } else {
        (node.rect.x, node.rect.y)
    };
    let page_x = ((raw_x - sx + node_off.0) * page_scene.dpi_scale as f64)
        .clamp(0.0, page_scene.width) as f32;
    let page_y = ((raw_y - sy + node_off.1) * page_scene.dpi_scale as f64)
        .clamp(0.0, page_scene.height) as f32;
    (page_x, page_y)
}

/// Pre-pass across all pages indexing every `\Anchor` node so forward and cross-page links resolve cleanly.
pub fn index_anchors(pages: &[PageScene]) -> AnchorIndex {
    let mut index = AnchorIndex::default();
    for page_scene in pages {
        let layout = &page_scene.layout;
        let layout_key = Arc::as_ptr(layout) as usize;
        for node in &layout.nodes {
            if let Some(included) = &page_scene.included_nodes {
                if !included.contains(&node.id) {
                    continue;
                }
            }
            let (x, y) = node_page_position(page_scene, node);
            let target = AnchorTarget {
                page_index: page_scene.page_index,
                x,
                y,
            };
            index.by_any_node.entry(node.id).or_insert(target);
            if node.anchor_name.is_none() && node.name != "Anchor" {
                continue;
            }
            index.by_layout_node.entry((layout_key, node.id)).or_insert(target);
            index.by_node_id.entry(node.id).or_insert(target);
            if let Some(ref aname) = node.anchor_name {
                index.by_name.entry(aname.clone()).or_insert(target);
            }
        }
    }
    index
}

fn enclosing_sliced_text<'a>(
    layout: &'a ResolvedLayout,
    page_scene: &'a PageScene,
    node: &'a ResolvedNode,
) -> Option<(&'a ResolvedNode, &'a TextSliceSpec)> {
    if node.fragments.is_empty() || page_scene.text_line_slices.is_empty() {
        return None;
    }
    let mut cur_id = node.id;
    let mut visited = 0usize;
    while let Some(cur_node) = layout.get_node(cur_id) {
        if cur_node.is_inline_box {
            return None;
        }
        let Some(parent_id) = cur_node.parent else {
            break;
        };
        if let Some(spec) = page_scene.text_line_slices.get(&parent_id) {
            let host = layout.get_node(parent_id)?;
            return Some((host, spec));
        }
        cur_id = parent_id;
        visited += 1;
        if visited > 32 {
            break;
        }
    }
    None
}

fn clip_rect_for_node(
    page_scene: &PageScene,
    clip_id: Option<NodeId>,
) -> Option<Rect> {
    let layout = &page_scene.layout;
    let chain = get_clip_chain(clip_id, layout);
    if chain.is_empty() {
        return None;
    }

    let mut cur: Option<Rect> = None;
    for cid in chain {
        let Some(box_id) = layout.get_value(cid, "box").and_then(|v| v.as_node()) else {
            continue;
        };
        if box_id.is_window() {
            continue;
        }
        let get_box_val = |port: &str| -> f64 {
            layout
                .get_value(box_id, port)
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
        };
        let off = page_scene.node_offsets.get(&cid).copied().unwrap_or((0.0, 0.0));
        let bx = Rect::new(
            get_box_val("x") + off.0,
            get_box_val("y") + off.1,
            get_box_val("width"),
            get_box_val("height"),
        );
        cur = Some(match cur {
            None => bx,
            Some(prev) => intersect_rects(prev, bx)?,
        });
    }
    cur
}

fn intersect_rects(a: Rect, b: Rect) -> Option<Rect> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.width).min(b.x + b.width);
    let y1 = (a.y + a.height).min(b.y + b.height);
    if x1 > x0 && y1 > y0 {
        Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
    } else {
        None
    }
}

/// Emits PDF `/Link` annotations (external URIs and internal `#anchor` `/XYZ` destinations)
/// for all link nodes present on `page_scene`.
pub fn emit_page_links(
    page_scene: &PageScene,
    page: &mut Page<'_>,
    anchors: &AnchorIndex,
    strict_links: bool,
) -> Result<(), String> {
    let layout = &page_scene.layout;
    let scale = page_scene.dpi_scale as f64;

    for node in &layout.nodes {
        if let Some(included) = &page_scene.included_nodes {
            if !included.contains(&node.id) {
                continue;
            }
        }
        let target = if let Some(target_nid) = node.properties.get("to").and_then(|v| v.as_node()) {
            if let Some(dest) = anchors.resolve_node(layout, target_nid) {
                Target::Destination(Destination::Xyz(XyzDestination::new(
                    dest.page_index,
                    Point::from_xy(dest.x, dest.y),
                )))
            } else {
                let msg = format!("Unresolved internal PDF link target '{}'", target_nid.0);
                if strict_links {
                    return Err(msg);
                }
                eprintln!("Warning: {}", msg);
                continue;
            }
        } else {
            let Some(url) = node.properties.get("url").and_then(|v| v.as_str()) else {
                continue;
            };
            if url.is_empty() {
                continue;
            }

            if url.starts_with('#') {
                if let Some(dest) = anchors.resolve(layout, node.id, url) {
                    Target::Destination(Destination::Xyz(XyzDestination::new(
                        dest.page_index,
                        Point::from_xy(dest.x, dest.y),
                    )))
                } else {
                    let msg = format!("Unresolved internal PDF link anchor '{}'", url);
                    if strict_links {
                        return Err(msg);
                    }
                    eprintln!("Warning: {}", msg);
                    continue;
                }
            } else {
                Target::Action(Action::Link(LinkAction::new(url.to_string())))
            }
        };

        let raw_hitboxes: &[Rect] = if !node.fragments.is_empty() {
            &node.fragments
        } else {
            std::slice::from_ref(&node.rect)
        };

        let (sx, sy) = layout.clip_scroll_offset(node.clip);
        let clip_bounds = clip_rect_for_node(page_scene, node.clip);
        let sliced_host = enclosing_sliced_text(layout, page_scene, node);

        let (eff_off, y_adjust, active_y_band) = if let Some((host, spec)) = sliced_host {
            let host_off = page_scene
                .node_offsets
                .get(&host.id)
                .copied()
                .unwrap_or((0.0, 0.0));
            let (y_adj, origin_y) = if spec.line_range.start > 0 {
                (
                    -spec.full_y_offset - spec.slice_top_offset,
                    host.rect.y + host_off.1 - spec.slice_top_offset,
                )
            } else {
                (0.0, host.rect.y + host_off.1 + spec.full_y_offset)
            };
            let band = if spec.active_parley_y_max > spec.active_parley_y_min {
                Some((
                    origin_y - sy + spec.active_parley_y_min - 1.0,
                    origin_y - sy + spec.active_parley_y_max + 1.0,
                ))
            } else {
                Some((1.0, -1.0))
            };
            (host_off, y_adj, band)
        } else {
            let node_off = page_scene
                .node_offsets
                .get(&node.id)
                .copied()
                .unwrap_or((0.0, 0.0));
            (node_off, 0.0, None)
        };

        let mut pdf_rects: Vec<KrillaRect> = Vec::with_capacity(raw_hitboxes.len());
        for hb in raw_hitboxes {
            if hb.width <= 0.0 || hb.height <= 0.0 {
                continue;
            }
            let shifted = Rect::new(
                hb.x - sx + eff_off.0,
                hb.y - sy + eff_off.1 + y_adjust,
                hb.width,
                hb.height,
            );
            if let Some((ymin, ymax)) = active_y_band {
                let mid_y = shifted.y + shifted.height * 0.5;
                if mid_y < ymin || mid_y > ymax {
                    continue;
                }
            }
            let clipped = if let Some(cb) = clip_bounds {
                match intersect_rects(shifted, cb) {
                    Some(c) => c,
                    None => continue,
                }
            } else {
                shifted
            };

            let Some(kr) = KrillaRect::from_xywh(
                (clipped.x * scale) as f32,
                (clipped.y * scale) as f32,
                (clipped.width * scale) as f32,
                (clipped.height * scale) as f32,
            ) else {
                continue;
            };
            pdf_rects.push(kr);
        }

        if pdf_rects.is_empty() {
            continue;
        }

        let link_annot = if pdf_rects.len() == 1 {
            LinkAnnotation::new(pdf_rects[0], target)
        } else {
            let quads: Vec<Quadrilateral> = pdf_rects.into_iter().map(Quadrilateral::from).collect();
            LinkAnnotation::new_with_quad_points(quads, target)
        };

        page.add_annotation(Annotation::new_link(link_annot, None));
    }

    Ok(())
}
