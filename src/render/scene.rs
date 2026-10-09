use std::collections::{HashMap, HashSet};
use std::ops::Range;

use parley::layout::{AlignmentOptions, PositionedLayoutItem};
use parley::style::{FontFamily, FontWeight, StyleProperty};
use parley::{Alignment, FontContext, LayoutContext};
use vello::kurbo::{Affine, Rect, RoundedRect, Stroke};
use vello::peniko::{Brush, Color, Fill};
use vello::Scene;

use crate::compiler::expanded::NodeId;
use crate::compiler::layout::ResolvedLayout;
use crate::compiler::value::Value;
use crate::render::color::{color_to_rgba8, parse_color};

/// Captures styling parameters for an inline rich text span to detect visual changes.
#[derive(Clone, Debug, PartialEq)]
pub struct SpanRenderKey {
    pub range: Range<usize>,
    pub color: Option<String>,
    pub underline: bool,
    pub bg_color: Option<String>,
}

/// The set of input properties that determine a text node's local glyph layout and visual presentation.
/// Crucially excludes `node.rect.x` and `node.rect.y`, allowing 100% cache hits during scrolling and translation.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRenderKey {
    pub text: String,
    pub width: f64,
    pub font_size: f32,
    pub font_weight: f32,
    pub font_family: Option<String>,
    pub color: [u8; 4],
    pub align: Option<String>,
    pub spans: Vec<SpanRenderKey>,
    pub start_at: Option<String>,
}

/// A cached local `vello::Scene` rendered at origin `(0.0, 0.0)` for a single text node.
pub struct CachedTextScene {
    pub key: TextRenderKey,
    pub scene: Scene,
    pub y_offset: f64,
}

/// 1-element cache per text node (`NodeId` -> previous call inputs and pre-recorded local `vello::Scene`).
///
/// When only `y` changes during scrolling, the cache hits, completely bypassing
/// Parley text shaping, OpenType font layout, bidi analysis, and line-breaking passes.
#[derive(Default)]
pub struct TextSceneCache {
    pub entries: HashMap<NodeId, CachedTextScene>,
}

impl TextSceneCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn get(&self, id: &NodeId) -> Option<&CachedTextScene> {
        self.entries.get(id)
    }

    pub fn insert(&mut self, id: NodeId, entry: CachedTextScene) {
        self.entries.insert(id, entry);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Prunes entries for nodes that no longer exist in the active layout.
    pub fn prune(&mut self, active_nodes: &HashSet<NodeId>) {
        self.entries.retain(|id, _| active_nodes.contains(id));
    }
}

/// A cached local `vello::Scene` for a clip layer recorded at static layout coordinates.
pub struct CachedClipScene {
    pub content_hash: u64,
    pub scene: Scene,
}

/// Cache for clip sub-scenes (`NodeId` -> `CachedClipScene`).
///
/// When scrolling occurs, only `scroll_x` and `scroll_y` change. The children's static layout
/// coordinates and styles remain 100% identical. The pre-recorded clip scene is reused directly,
/// and transformed on the GPU via `Affine::translate((-scroll_x, -scroll_y))`.
#[derive(Default)]
pub struct ClipSceneCache {
    pub entries: HashMap<NodeId, CachedClipScene>,
}

impl ClipSceneCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn get(&self, id: &NodeId) -> Option<&CachedClipScene> {
        self.entries.get(id)
    }

    pub fn insert(&mut self, id: NodeId, entry: CachedClipScene) {
        self.entries.insert(id, entry);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Prunes entries for clip nodes that no longer exist in the active layout.
    pub fn prune(&mut self, active_nodes: &HashSet<NodeId>) {
        self.entries.retain(|id, _| active_nodes.contains(id));
    }
}

/// Computes a fast hash of all content and styling parameters for primitives within a clip container.
///
/// Crucially excludes the clip's own `scroll_x` and `scroll_y`, allowing 100% cache hits
/// during hardware layer translation.
pub fn compute_clip_content_hash(clip_id: NodeId, layout: &ResolvedLayout) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    // Include the clip's box bounds
    if let Some(box_id) = layout.get_value(clip_id, "box").and_then(|v| v.as_node()) {
        if let Some(box_node) = layout.get_node(box_id) {
            box_node.rect.x.to_bits().hash(&mut hasher);
            box_node.rect.y.to_bits().hash(&mut hasher);
            box_node.rect.width.to_bits().hash(&mut hasher);
            box_node.rect.height.to_bits().hash(&mut hasher);
        }
    }

    // Hash all primitives belonging to this clip
    for node in &layout.nodes {
        if node.clip == Some(clip_id) {
            node.id.hash(&mut hasher);
            node.rect.x.to_bits().hash(&mut hasher);
            node.rect.y.to_bits().hash(&mut hasher);
            node.rect.width.to_bits().hash(&mut hasher);
            node.rect.height.to_bits().hash(&mut hasher);
            node.z.to_bits().hash(&mut hasher);
            node.text_content.hash(&mut hasher);
            for (k, v) in &node.properties {
                k.hash(&mut hasher);
                match v {
                    Value::Number(n) => n.to_bits().hash(&mut hasher),
                    Value::String(s) => s.hash(&mut hasher),
                    Value::Bool(b) => b.hash(&mut hasher),
                    Value::Color(c) => c.hash(&mut hasher),
                    _ => {}
                }
            }
        } else if node.name == "Clip" && layout.get_value(node.id, "up").and_then(|v| v.as_node()) == Some(clip_id) {
            // Nested clip: hash its ID and its scroll state
            node.id.hash(&mut hasher);
            if let Some(sx) = layout.get_value(node.id, "scroll_x").and_then(|v| v.as_f64()) {
                sx.to_bits().hash(&mut hasher);
            }
            if let Some(sy) = layout.get_value(node.id, "scroll_y").and_then(|v| v.as_f64()) {
                sy.to_bits().hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

struct ActiveClipEntry {
    clip_id: NodeId,
    scene: Scene,
    scroll_x: f64,
    scroll_y: f64,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    radius: f64,
    is_skipped: bool,
    expected_nodes: usize,
    rendered_nodes: usize,
}

fn pop_clip_entry(
    clip_entry: ActiveClipEntry,
    parent_scene: &mut Scene,
    clip_cache: &mut ClipSceneCache,
    layout: &ResolvedLayout,
) {
    if clip_entry.is_skipped {
        return;
    }

    if clip_entry.rendered_nodes == clip_entry.expected_nodes {
        let content_hash = compute_clip_content_hash(clip_entry.clip_id, layout);
        clip_cache.insert(
            clip_entry.clip_id,
            CachedClipScene {
                content_hash,
                scene: clip_entry.scene.clone(),
            },
        );
    }

    if clip_entry.radius > 0.0 {
        let rrect = RoundedRect::new(
            clip_entry.x,
            clip_entry.y,
            clip_entry.x + clip_entry.width,
            clip_entry.y + clip_entry.height,
            clip_entry.radius,
        );
        parent_scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rrect);
    } else {
        let rect = Rect::new(
            clip_entry.x,
            clip_entry.y,
            clip_entry.x + clip_entry.width,
            clip_entry.y + clip_entry.height,
        );
        parent_scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rect);
    }

    parent_scene.append(
        &clip_entry.scene,
        Some(Affine::translate((-clip_entry.scroll_x, -clip_entry.scroll_y))),
    );
    parent_scene.pop_layer();
}

/// Options for building a Vello scene from a ResolvedLayout.
#[derive(Debug, Clone)]
pub struct SceneOptions {
    /// Optional background color for the viewport / canvas.
    pub background: Option<Color>,
    /// Display / DPI scale factor (e.g. 2.0 on macOS Retina displays).
    pub scale_factor: f64,
    /// Scroll offset (scroll_x, scroll_y) in logical document coordinates.
    pub scroll_offset: (f64, f64),
}

impl Default for SceneOptions {
    fn default() -> Self {
        Self {
            background: Some(Color::WHITE),
            scale_factor: 1.0,
            scroll_offset: (0.0, 0.0),
        }
    }
}

fn get_clip_chain(
    clip_id: Option<NodeId>,
    layout: &ResolvedLayout,
) -> Vec<NodeId> {
    let mut chain = Vec::new();
    let mut curr = clip_id;
    let mut visited = std::collections::HashSet::new();

    while let Some(id) = curr {
        if id.is_window() || !visited.insert(id) {
            break;
        }
        chain.push(id);
        curr = layout
            .get_value(id, "up")
            .and_then(|v| v.as_node())
            .filter(|up_id| !up_id.is_window());
    }

    chain.reverse(); // root-to-leaf order
    chain
}

/// Builds a `vello::Scene` from a `ResolvedLayout` using active `TextSceneCache` and `ClipSceneCache`.
pub fn build_scene(
    layout: &ResolvedLayout,
    font_cx: &mut FontContext,
    _layout_cx: &mut LayoutContext<()>,
    text_cache: &mut TextSceneCache,
    clip_cache: &mut ClipSceneCache,
    options: &SceneOptions,
) -> Scene {
    let mut root_scene = Scene::new();
    let mut text_layout_cx = LayoutContext::<[u8; 4]>::new();

    // 1. Draw canvas background if requested
    if let Some(bg) = options.background {
        if bg.components[3] > 0.0 {
            // Find bounding box or maximum extents
            let mut max_w: f64 = 0.0;
            let mut max_h: f64 = 0.0;
            for node in &layout.nodes {
                max_w = max_w.max(node.rect.x + node.rect.width);
                max_h = max_h.max(node.rect.y + node.rect.height);
            }
            if max_w > 0.0 && max_h > 0.0 {
                let rect = Rect::new(0.0, 0.0, max_w, max_h);
                root_scene.fill(Fill::NonZero, Affine::IDENTITY, Brush::Solid(bg), None, &rect);
            }
        }
    }

    let mut clip_stack: Vec<ActiveClipEntry> = Vec::new();

    // 2. Iterate in topological painter's order
    for node in layout.render_order() {
        if !node.is_paint_primitive() {
            continue;
        }

        // Inline descendants of a Text node are rendered as styled spans by the parent Text node
        let mut is_inline_descendant = false;
        let mut curr_parent = node.parent;
        while let Some(pid) = curr_parent {
            if let Some(pnode) = layout.get_node(pid) {
                if pnode.name == "Text" {
                    is_inline_descendant = true;
                    break;
                }
                curr_parent = pnode.parent;
            } else {
                break;
            }
        }
        if is_inline_descendant {
            continue;
        }

        let target_chain = get_clip_chain(node.clip, layout);
        let common_len = clip_stack
            .iter()
            .map(|e| e.clip_id)
            .zip(&target_chain)
            .take_while(|(a, b)| a == *b)
            .count();

        while clip_stack.len() > common_len {
            let clip_entry = clip_stack.pop().unwrap();
            let parent_scene = if let Some(parent) = clip_stack.last_mut() {
                &mut parent.scene
            } else {
                &mut root_scene
            };
            pop_clip_entry(clip_entry, parent_scene, clip_cache, layout);
        }

        for &clip_node_id in &target_chain[common_len..] {
            let (x, y, width, height, radius) = if let Some(box_node_id) = layout.get_value(clip_node_id, "box").and_then(|v| v.as_node()) {
                let get_box_val = |port: &str| -> f64 {
                    layout
                        .get_value(box_node_id, port)
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0)
                };
                let x = get_box_val("x");
                let y = get_box_val("y");
                let w = get_box_val("width");
                let h = get_box_val("height");
                let radius = layout
                    .get_value(box_node_id, "radius")
                    .or_else(|| layout.get_value(box_node_id, "corner_radius"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                (x, y, w, h, radius)
            } else {
                (0.0, 0.0, 0.0, 0.0, 0.0)
            };

            let scroll_x = layout
                .get_value(clip_node_id, "scroll_x")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let scroll_y = layout
                .get_value(clip_node_id, "scroll_y")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);

            let expected_nodes = layout.nodes.iter().filter(|n| n.clip == Some(clip_node_id)).count();

            // If the parent clip is already skipped, this child clip is also skipped
            let parent_skipped = clip_stack.last().map_or(false, |e| e.is_skipped);
            if parent_skipped {
                clip_stack.push(ActiveClipEntry {
                    clip_id: clip_node_id,
                    scene: Scene::new(),
                    scroll_x,
                    scroll_y,
                    x,
                    y,
                    width,
                    height,
                    radius,
                    is_skipped: true,
                    expected_nodes,
                    rendered_nodes: 0,
                });
                continue;
            }

            let content_hash = compute_clip_content_hash(clip_node_id, layout);
            if let Some(cached) = clip_cache.get(&clip_node_id) {
                if cached.content_hash == content_hash {
                    // Cache Hit: Immediately append the cached clip scene transformed by GPU translate!
                    let parent_scene = if let Some(parent) = clip_stack.last_mut() {
                        &mut parent.scene
                    } else {
                        &mut root_scene
                    };

                    if radius > 0.0 {
                        let rrect = RoundedRect::new(x, y, x + width, y + height, radius);
                        parent_scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rrect);
                    } else {
                        let rect = Rect::new(x, y, x + width, y + height);
                        parent_scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rect);
                    }
                    parent_scene.append(&cached.scene, Some(Affine::translate((-scroll_x, -scroll_y))));
                    parent_scene.pop_layer();

                    clip_stack.push(ActiveClipEntry {
                        clip_id: clip_node_id,
                        scene: Scene::new(),
                        scroll_x,
                        scroll_y,
                        x,
                        y,
                        width,
                        height,
                        radius,
                        is_skipped: true,
                        expected_nodes,
                        rendered_nodes: 0,
                    });
                    continue;
                }
            }

            // Cache Miss:
            clip_stack.push(ActiveClipEntry {
                clip_id: clip_node_id,
                scene: Scene::new(),
                scroll_x,
                scroll_y,
                x,
                y,
                width,
                height,
                radius,
                is_skipped: false,
                expected_nodes,
                rendered_nodes: 0,
            });
        }

        // If any clip in the active stack is skipped, skip rendering this node
        if clip_stack.iter().any(|e| e.is_skipped) {
            continue;
        }

        // Count this node towards the active clip
        if let Some(entry) = clip_stack.last_mut() {
            if node.clip == Some(entry.clip_id) {
                entry.rendered_nodes += 1;
            }
        }

        let active_scene = if let Some(current) = clip_stack.last_mut() {
            &mut current.scene
        } else {
            &mut root_scene
        };

        let is_rect = node.name == "Rect";

        // Render filled rectangle (only for \Rect paint primitives)
        if is_rect && node.rect.width > 0.0 && node.rect.height > 0.0 {
            let color_val = node
                .properties
                .get("color")
                .or_else(|| node.properties.get("bg_color"));

            let color = match color_val {
                Some(Value::Color(c)) | Some(Value::String(c)) => parse_color(c),
                _ => Color::TRANSPARENT,
            };

            let radius = node
                .properties
                .get("radius")
                .or_else(|| node.properties.get("corner_radius"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);

            let x0 = node.rect.x;
            let y0 = node.rect.y;
            let x1 = x0 + node.rect.width;
            let y1 = y0 + node.rect.height;

            if color.components[3] > 0.0 {
                if radius > 0.0 {
                    let rrect = RoundedRect::new(x0, y0, x1, y1, radius);
                    active_scene.fill(Fill::NonZero, Affine::IDENTITY, Brush::Solid(color), None, &rrect);
                } else {
                    let rect = Rect::new(x0, y0, x1, y1);
                    active_scene.fill(Fill::NonZero, Affine::IDENTITY, Brush::Solid(color), None, &rect);
                }
            }

            // Stroke / border
            let stroke_width = node
                .properties
                .get("border_width")
                .or_else(|| node.properties.get("stroke_width"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);

            if stroke_width > 0.0 {
                let stroke_color = node
                    .properties
                    .get("border_color")
                    .or_else(|| node.properties.get("stroke_color"))
                    .map(|v| match v {
                        Value::Color(c) | Value::String(c) => parse_color(c),
                        _ => Color::BLACK,
                    })
                    .unwrap_or(Color::BLACK);

                let stroke = Stroke::new(stroke_width);
                if radius > 0.0 {
                    let rrect = RoundedRect::new(x0, y0, x1, y1, radius);
                    active_scene.stroke(&stroke, Affine::IDENTITY, Brush::Solid(stroke_color), None, &rrect);
                } else {
                    let rect = Rect::new(x0, y0, x1, y1);
                    active_scene.stroke(&stroke, Affine::IDENTITY, Brush::Solid(stroke_color), None, &rect);
                }
            }
        }

        // Render text
        let is_text_primitive = node.name == "Text"
            || (node.children.is_empty()
                && (node.properties.contains_key("text") || node.properties.contains_key("content")));
        if !is_text_primitive {
            continue;
        }

        let dynamic_text = node
            .properties
            .get("text")
            .or_else(|| node.properties.get("content"))
            .and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(format!("{n}")),
                Value::Bool(b) => Some(format!("{b}")),
                _ => None,
            });
        let active_text = dynamic_text.as_ref().or(node.text_content.as_ref());
        if let Some(text) = active_text {
            if !text.is_empty() {
                let font_size = node
                    .properties
                    .get("font_size")
                    .or_else(|| node.properties.get("size"))
                    .and_then(|v| v.as_f64())
                    .or_else(|| {
                        node.font.and_then(|fid| {
                            layout.get_node(fid).and_then(|fn_node| {
                                fn_node
                                    .properties
                                    .get("size")
                                    .or_else(|| fn_node.properties.get("font_size"))
                                    .and_then(|v| v.as_f64())
                            })
                        })
                    })
                    .unwrap_or(16.0) as f32;

                let font_weight = node
                    .properties
                    .get("font_weight")
                    .or_else(|| node.properties.get("weight"))
                    .and_then(|v| v.as_f64())
                    .or_else(|| {
                        node.font.and_then(|fid| {
                            layout.get_node(fid).and_then(|fn_node| {
                                fn_node
                                    .properties
                                    .get("weight")
                                    .or_else(|| fn_node.properties.get("font_weight"))
                                    .and_then(|v| v.as_f64())
                            })
                        })
                    })
                    .unwrap_or(400.0) as f32;

                let font_family = node
                    .properties
                    .get("font_family")
                    .or_else(|| node.properties.get("family"))
                    .and_then(|v| v.as_str())
                    .or_else(|| {
                        node.properties.get("font").and_then(|v| match v {
                            Value::String(s) => Some(s.as_str()),
                            _ => None,
                        })
                    })
                    .or_else(|| {
                        node.font.and_then(|fid| {
                            layout.get_node(fid).and_then(|fn_node| {
                                fn_node
                                    .properties
                                    .get("family")
                                    .or_else(|| fn_node.properties.get("font"))
                                    .and_then(|v| v.as_str())
                            })
                        })
                    });

                let text_color = node
                    .properties
                    .get("color")
                    .or_else(|| node.properties.get("text_color"))
                    .map(|v| match v {
                        Value::Color(c) | Value::String(c) => parse_color(c),
                        _ => Color::BLACK,
                    })
                    .unwrap_or(Color::BLACK);

                let align_str = node
                    .properties
                    .get("align")
                    .or_else(|| node.properties.get("text_align"))
                    .and_then(|v| v.as_str());

                // Build span render keys to detect hover/style changes
                let mut span_keys = Vec::with_capacity(node.text_spans.len());
                for span in &node.text_spans {
                    let mut color_str = span.style.color.clone();
                    let mut underline = span.style.underline;
                    let mut bg_str = None;

                    if let Some(child_id) = span.node_id {
                        if let Some(child_node) = layout.get_node(child_id) {
                            if let Some(bg) = child_node.properties.get("current_bg").and_then(|v| v.as_str()) {
                                bg_str = Some(bg.to_string());
                            }
                            if let Some(c) = child_node.properties.get("current_color").and_then(|v| v.as_str()) {
                                color_str = Some(c.to_string());
                            }
                            if let Some(u) = child_node.properties.get("current_underline").and_then(|v| v.as_bool()) {
                                underline = u;
                            }
                        }
                    }

                    span_keys.push(SpanRenderKey {
                        range: span.range.clone(),
                        color: color_str,
                        underline,
                        bg_color: bg_str,
                    });
                }

                let start_at_prop = node
                    .properties
                    .get("start_at")
                    .or_else(|| node.properties.get("starts_at"))
                    .and_then(|v| v.as_str());

                let current_key = TextRenderKey {
                    text: text.to_string(),
                    width: node.rect.width,
                    font_size,
                    font_weight,
                    font_family: font_family.map(|s| s.to_string()),
                    color: color_to_rgba8(&text_color),
                    align: align_str.map(|s| s.to_string()),
                    spans: span_keys,
                    start_at: start_at_prop.map(|s| s.to_string()),
                };

                // Check 1-element cache:
                if let Some(cached) = text_cache.get(&node.id) {
                    if cached.key == current_key {
                        // CACHE HIT: 100% of the time during scrolling!
                        active_scene.append(&cached.scene, Some(Affine::translate((node.rect.x, node.rect.y + cached.y_offset))));
                        continue;
                    }
                }

                // CACHE MISS: Build local scene at origin (0.0, 0.0)
                let mut local_scene = Scene::new();

                let mut builder = text_layout_cx.ranged_builder(font_cx, text, 1.0, true);
                builder.push_default(StyleProperty::FontSize(font_size));
                builder.push_default(StyleProperty::Brush(color_to_rgba8(&text_color)));
                if (font_weight - 400.0).abs() > 1.0 {
                    builder.push_default(StyleProperty::FontWeight(FontWeight::new(font_weight)));
                }
                if let Some(family) = font_family {
                    if !family.is_empty() {
                        builder.push_default(StyleProperty::FontFamily(FontFamily::named(family)));
                    }
                }

                // Draw any span background highlights (e.g. focused_bg on links) in local coordinates
                for span in &node.text_spans {
                    if let Some(child_id) = span.node_id {
                        if let Some(child_node) = layout.get_node(child_id) {
                            if let Some(bg_str) = child_node.properties.get("current_bg").and_then(|v| v.as_str()) {
                                let bg_color = parse_color(bg_str);
                                if bg_color.components[3] > 0.0 {
                                    for frag in &child_node.fragments {
                                        let local_x = frag.x - node.rect.x;
                                        let local_y = frag.y - node.rect.y;
                                        let pad_rect = Rect::new(
                                            local_x - 2.0,
                                            local_y - 1.0,
                                            local_x + frag.width + 2.0,
                                            local_y + frag.height + 1.0,
                                        );
                                        let rounded = RoundedRect::from_rect(pad_rect, 4.0);
                                        local_scene.fill(
                                            Fill::NonZero,
                                            Affine::IDENTITY,
                                            Brush::Solid(bg_color),
                                            None,
                                            &rounded,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }

                // Push span styles (colors & underlines) to range builder
                for span in &node.text_spans {
                    let mut color_str = span.style.color.as_deref();
                    let mut underline = span.style.underline;

                    if let Some(child_id) = span.node_id {
                        if let Some(child_node) = layout.get_node(child_id) {
                            if let Some(c) = child_node.properties.get("current_color").and_then(|v| v.as_str()) {
                                color_str = Some(c);
                            }
                            if let Some(u) = child_node.properties.get("current_underline").and_then(|v| v.as_bool()) {
                                underline = u;
                            }
                        }
                    }

                    if let Some(c_str) = color_str {
                        let c = parse_color(c_str);
                        builder.push(StyleProperty::Brush(color_to_rgba8(&c)), span.range.clone());
                    }
                    if underline {
                        builder.push(StyleProperty::Underline(true), span.range.clone());
                    }
                }

                let mut layout_text = builder.build(text);
                if node.rect.width > 0.0 {
                    layout_text.break_all_lines(Some(node.rect.width as f32));
                }

                let alignment = match align_str {
                    Some("center") | Some("Center") => Alignment::Center,
                    Some("right") | Some("Right") | Some("end") | Some("End") => Alignment::End,
                    Some("justify") | Some("Justify") => Alignment::Justify,
                    _ => Alignment::Start,
                };
                layout_text.align(alignment, AlignmentOptions::default());

                // Render lines and glyphs into local_scene
                for line in layout_text.lines() {
                    for item in line.items() {
                        if let PositionedLayoutItem::GlyphRun(glyph_run) = item {
                            let run = glyph_run.run();
                            let font = run.font();
                            let brush_u8 = glyph_run.style().brush;
                            let run_color = Color::from_rgba8(brush_u8[0], brush_u8[1], brush_u8[2], brush_u8[3]);

                            let glyphs = glyph_run.positioned_glyphs().map(|g| vello::Glyph {
                                id: g.id,
                                x: g.x,
                                y: g.y,
                            });
                            local_scene
                                .draw_glyphs(font)
                                .font_size(run.font_size())
                                .transform(Affine::IDENTITY)
                                .brush(Brush::Solid(run_color))
                                .draw(Fill::NonZero, glyphs);

                            // Draw underline if requested by span/style
                            if glyph_run.style().underline.is_some() {
                                let mut min_gx = f32::MAX;
                                let mut max_gx = f32::MIN;
                                let mut gy = 0.0;
                                for g in glyph_run.positioned_glyphs() {
                                    if g.x < min_gx {
                                        min_gx = g.x;
                                    }
                                    let end_x = g.x + g.advance;
                                    if end_x > max_gx {
                                        max_gx = end_x;
                                    }
                                    gy = g.y;
                                }

                                if min_gx < max_gx {
                                    let underline_rect = vello::kurbo::Rect::new(
                                        min_gx as f64,
                                        gy as f64 + 2.0,
                                        max_gx as f64,
                                        gy as f64 + 3.2,
                                    );
                                    local_scene.fill(
                                        Fill::NonZero,
                                        Affine::IDENTITY,
                                        Brush::Solid(run_color),
                                        None,
                                        &underline_rect,
                                    );
                                }
                            }
                        }
                    }
                }

                // Append local scene to active scene translated to node's position:
                let y_offset = crate::compiler::text::text_y_offset(font_size as f64, font_weight as f64, font_family, start_at_prop);
                active_scene.append(&local_scene, Some(Affine::translate((node.rect.x, node.rect.y + y_offset))));

                // Save in 1-element cache:
                text_cache.insert(node.id, CachedTextScene {
                    key: current_key,
                    scene: local_scene,
                    y_offset,
                });
            }
        }
    }

    while let Some(clip_entry) = clip_stack.pop() {
        let parent_scene = if let Some(parent) = clip_stack.last_mut() {
            &mut parent.scene
        } else {
            &mut root_scene
        };
        pop_clip_entry(clip_entry, parent_scene, clip_cache, layout);
    }

    // Periodically prune dead nodes from caches if they have grown noticeably larger than active nodes
    if text_cache.len() > layout.nodes.len() + 64 {
        let active_ids: HashSet<NodeId> = layout.nodes.iter().map(|n| n.id).collect();
        text_cache.prune(&active_ids);
    }
    if clip_cache.len() > layout.nodes.len() + 16 {
        let active_ids: HashSet<NodeId> = layout.nodes.iter().map(|n| n.id).collect();
        clip_cache.prune(&active_ids);
    }

    let scale = if options.scale_factor > 0.0 {
        options.scale_factor
    } else {
        1.0
    };
    let has_scale = (scale - 1.0).abs() > 0.001;
    let has_scroll = options.scroll_offset.0.abs() > 0.001 || options.scroll_offset.1.abs() > 0.001;

    if has_scale || has_scroll {
        let transform = Affine::scale(scale) * Affine::translate((-options.scroll_offset.0, -options.scroll_offset.1));
        let mut transformed_scene = Scene::new();
        transformed_scene.append(&root_scene, Some(transform));
        transformed_scene
    } else {
        root_scene
    }
}

/// Convenience helper to build a `vello::Scene` without providing an external cache.
pub fn build_scene_without_cache(
    layout: &ResolvedLayout,
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    options: &SceneOptions,
) -> Scene {
    let mut text_cache = TextSceneCache::new();
    let mut clip_cache = ClipSceneCache::new();
    build_scene(layout, font_cx, layout_cx, &mut text_cache, &mut clip_cache, options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::layout::{Rect, ResolvedNode};
    use crate::span::Span;

    #[test]
    fn test_text_scene_cache_hit_on_scroll() {
        let mut font_cx = FontContext::new();
        let mut layout_cx = LayoutContext::new();
        let mut text_cache = TextSceneCache::new();
        let mut clip_cache = ClipSceneCache::new();
        let options = SceneOptions::default();

        let node = ResolvedNode {
            id: NodeId(1),
            name: "Text".to_string(),
            key: None,
            parent: None,
            children: Vec::new(),
            rect: Rect::new(10.0, 50.0, 200.0, 30.0),
            z: 0.0,
            clip: None,
            text_content: Some("Hello World".to_string()),
            text_spans: Vec::new(),
            fragments: Vec::new(),
            anchor_name: None,
            scope_id: None,
            properties: HashMap::new(),
            state_vars: HashMap::new(),
            event_handlers: HashMap::new(),
            formulas: HashMap::new(),
            span: Span::default(),
            handle: None,
            font: None,
            var_name: None,
        };

        let mut layout = ResolvedLayout {
            roots: vec![NodeId(1)],
            nodes: vec![node.clone()],
            values: HashMap::new(),
            scope_tree: Default::default(),
            graph: None,
        };

        // Frame 1: Initial render (cache miss)
        let _scene1 = build_scene(&layout, &mut font_cx, &mut layout_cx, &mut text_cache, &mut clip_cache, &options);
        assert_eq!(text_cache.len(), 1);
        let key_before = text_cache.get(&NodeId(1)).unwrap().key.clone();

        // Frame 2: Simulating vertical scroll (rect.y changes from 50.0 to 180.0)
        layout.nodes[0].rect.y = 180.0;
        let _scene2 = build_scene(&layout, &mut font_cx, &mut layout_cx, &mut text_cache, &mut clip_cache, &options);
        assert_eq!(text_cache.len(), 1);
        let key_after = text_cache.get(&NodeId(1)).unwrap().key.clone();

        // The key must match identically despite y moving, confirming a 100% cache hit!
        assert_eq!(key_before, key_after);

        // Frame 3: Mutating text content invalidates and updates cache
        layout.nodes[0].text_content = Some("Updated Text".to_string());
        let _scene3 = build_scene(&layout, &mut font_cx, &mut layout_cx, &mut text_cache, &mut clip_cache, &options);
        assert_eq!(text_cache.len(), 1);
        assert_eq!(text_cache.get(&NodeId(1)).unwrap().key.text, "Updated Text");
    }

    #[test]
    fn test_clip_scene_cache_hit_on_scroll() {
        let mut font_cx = FontContext::new();
        let mut layout_cx = LayoutContext::new();
        let mut text_cache = TextSceneCache::new();
        let mut clip_cache = ClipSceneCache::new();
        let options = SceneOptions::default();

        let box_node = ResolvedNode {
            id: NodeId(10),
            name: "Box".to_string(),
            key: None,
            parent: None,
            children: Vec::new(),
            rect: Rect::new(0.0, 0.0, 300.0, 200.0),
            z: 0.0,
            clip: None,
            text_content: None,
            text_spans: Vec::new(),
            fragments: Vec::new(),
            anchor_name: None,
            scope_id: None,
            properties: HashMap::from([
                ("x".to_string(), Value::Number(0.0)),
                ("y".to_string(), Value::Number(0.0)),
                ("width".to_string(), Value::Number(300.0)),
                ("height".to_string(), Value::Number(200.0)),
            ]),
            state_vars: HashMap::new(),
            event_handlers: HashMap::new(),
            formulas: HashMap::new(),
            span: Span::default(),
            handle: None,
            font: None,
            var_name: None,
        };

        let clip_node = ResolvedNode {
            id: NodeId(2),
            name: "Clip".to_string(),
            key: None,
            parent: None,
            children: vec![NodeId(3)],
            rect: Rect::new(0.0, 0.0, 300.0, 200.0),
            z: 0.0,
            clip: None,
            text_content: None,
            text_spans: Vec::new(),
            fragments: Vec::new(),
            anchor_name: None,
            scope_id: None,
            properties: HashMap::from([
                ("box".to_string(), Value::Node(NodeId(10))),
                ("scroll_y".to_string(), Value::Number(0.0)),
            ]),
            state_vars: HashMap::new(),
            event_handlers: HashMap::new(),
            formulas: HashMap::new(),
            span: Span::default(),
            handle: None,
            font: None,
            var_name: None,
        };

        let child_node = ResolvedNode {
            id: NodeId(3),
            name: "Rect".to_string(),
            key: None,
            parent: Some(NodeId(2)),
            children: Vec::new(),
            rect: Rect::new(10.0, 20.0, 100.0, 50.0),
            z: 0.0,
            clip: Some(NodeId(2)),
            text_content: None,
            text_spans: Vec::new(),
            fragments: Vec::new(),
            anchor_name: None,
            scope_id: None,
            properties: HashMap::from([
                ("color".to_string(), Value::Color("#3b82f6".to_string())),
            ]),
            state_vars: HashMap::new(),
            event_handlers: HashMap::new(),
            formulas: HashMap::new(),
            span: Span::default(),
            handle: None,
            font: None,
            var_name: None,
        };

        let mut values = HashMap::new();
        values.insert(crate::compiler::graph::VarId::new(NodeId(2), "box"), Value::Node(NodeId(10)));
        values.insert(crate::compiler::graph::VarId::new(NodeId(2), "scroll_y"), Value::Number(0.0));
        values.insert(crate::compiler::graph::VarId::new(NodeId(10), "x"), Value::Number(0.0));
        values.insert(crate::compiler::graph::VarId::new(NodeId(10), "y"), Value::Number(0.0));
        values.insert(crate::compiler::graph::VarId::new(NodeId(10), "width"), Value::Number(300.0));
        values.insert(crate::compiler::graph::VarId::new(NodeId(10), "height"), Value::Number(200.0));

        let mut layout = ResolvedLayout {
            roots: vec![NodeId(2)],
            nodes: vec![box_node, clip_node, child_node],
            values,
            scope_tree: Default::default(),
            graph: None,
        };

        // Frame 1: Cache Miss
        let _scene1 = build_scene(&layout, &mut font_cx, &mut layout_cx, &mut text_cache, &mut clip_cache, &options);
        assert_eq!(clip_cache.len(), 1);
        let hash_before = clip_cache.get(&NodeId(2)).unwrap().content_hash;

        // Frame 2: Simulating vertical scroll - scroll_y changes on clip node and in values
        layout.values.insert(crate::compiler::graph::VarId::new(NodeId(2), "scroll_y"), Value::Number(50.0));
        layout.nodes[1].properties.insert("scroll_y".to_string(), Value::Number(50.0));

        // Child node static coordinates remain completely unchanged!
        assert_eq!(layout.nodes[2].rect.y, 20.0);

        let _scene2 = build_scene(&layout, &mut font_cx, &mut layout_cx, &mut text_cache, &mut clip_cache, &options);
        assert_eq!(clip_cache.len(), 1);
        let hash_after = clip_cache.get(&NodeId(2)).unwrap().content_hash;

        // Content hash must be 100% identical!
        assert_eq!(hash_before, hash_after, "Clip content hash must be identical when scrolling");
    }
}

