use krilla::destination::XyzDestination;
use krilla::geom::Point;
use krilla::outline::{Outline, OutlineNode};

use super::links::{node_page_position, AnchorIndex};
use super::PageScene;

struct OutlineEntry {
    level: u32,
    title: String,
    page_index: usize,
    x: f32,
    y: f32,
}

/// Builds a hierarchical PDF bookmark outline from resolved nodes carrying
/// `outline_title` / `outline_level` metadata or `\Bookmark` primitives.
pub fn build_document_outline(pages: &[PageScene], anchors: &AnchorIndex) -> Option<Outline> {
    let mut entries: Vec<OutlineEntry> = Vec::new();

    for page_scene in pages {
        let layout = &page_scene.layout;
        for node in &layout.nodes {
            if let Some(included) = &page_scene.included_nodes {
                if !included.contains(&node.id) {
                    continue;
                }
            }

            let title_opt = node
                .properties
                .get("outline_title")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    if node.name == "Bookmark" {
                        node.properties.get("title").and_then(|v| v.as_str())
                    } else {
                        None
                    }
                });

            let Some(title) = title_opt else {
                continue;
            };
            if title.is_empty() {
                continue;
            }

            let level = node
                .properties
                .get("outline_level")
                .or_else(|| node.properties.get("level"))
                .and_then(|v| v.as_i64())
                .unwrap_or(crate::compiler::expand::builtins::DEFAULT_OUTLINE_LEVEL)
                .clamp(1, 16) as u32;

            let (target_page, x, y) = if let Some(anchor_url) = node
                .properties
                .get("url")
                .or_else(|| node.properties.get("target"))
                .and_then(|v| v.as_str())
            {
                if let Some(dest) = anchors.resolve(layout, node.id, anchor_url) {
                    (dest.page_index, dest.x, dest.y)
                } else {
                    let (px, py) = node_page_position(page_scene, node);
                    (page_scene.page_index, px, py)
                }
            } else {
                let (px, py) = node_page_position(page_scene, node);
                (page_scene.page_index, px, py)
            };

            entries.push(OutlineEntry {
                level,
                title: title.to_string(),
                page_index: target_page,
                x,
                y,
            });
        }
    }

    if entries.is_empty() {
        return None;
    }

    let mut roots: Vec<OutlineNode> = Vec::new();
    let mut stack: Vec<(u32, OutlineNode)> = Vec::new();

    for entry in entries {
        while stack.last().is_some_and(|(top_lvl, _)| *top_lvl >= entry.level) {
            let (_, finished) = stack.pop().unwrap();
            if let Some((_, parent)) = stack.last_mut() {
                parent.push_child(finished);
            } else {
                roots.push(finished);
            }
        }
        let dest = XyzDestination::new(entry.page_index, Point::from_xy(entry.x, entry.y));
        stack.push((entry.level, OutlineNode::new(entry.title, dest)));
    }

    while let Some((_, finished)) = stack.pop() {
        if let Some((_, parent)) = stack.last_mut() {
            parent.push_child(finished);
        } else {
            roots.push(finished);
        }
    }

    let mut outline = Outline::new();
    for root in roots {
        outline.push_child(root);
    }
    Some(outline)
}
