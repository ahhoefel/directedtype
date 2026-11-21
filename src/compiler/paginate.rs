use crate::ast::{ComponentDef, DistanceUnit, EnumDef, Expr, Ident, Literal, MatchPattern, Symbol};
use crate::compiler::compiled::CompiledDocument;
use crate::compiler::error::CompileError;
use crate::compiler::eval::{eval_expr_for_node, evaluate_graph};
use crate::compiler::expand::builtins::pagination::{
    eval_constant_break_target, eval_var_slice, populate_page_ports,
};
use crate::compiler::expand::context::{ConsumerChildEntry, ElementContext, LexicalBinding};
use crate::compiler::expand::expand_element;
use crate::compiler::expand::rewrite::{
    flush_all_materialized_expr_ports, resolve_forward_refs_in_range, restore_rewrite_tracking,
    save_rewrite_tracking, set_window_port_types, RewriteTrackingSnapshot,
};
use crate::compiler::expanded::{
    ExpandedDocument, ExpandedNode, NodeId, PageBreakTarget, PagedFlowRecord,
};
use crate::compiler::graph::{build_variable_graph_with_window_and_dpi, VarId};
use crate::compiler::layout::{resolve_layout_with_graph, Rect, ResolvedLayout, ResolvedNode};
use crate::compiler::scope::ScopeId;
use crate::compiler::text::{resolve_active_rich_text, InlineBoxSpan, ShapedParagraph};
use crate::compiler::topo::sort_graph;
use crate::compiler::value::Value;
use crate::span::Span;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::sync::Arc;

/// Line-slice specification for a `\Text` node split across a page boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct TextSliceSpec {
    pub line_range: Range<usize>,
    pub slice_top_offset: f64,
    pub slice_height: f64,
    pub full_y_offset: f64,
    pub active_parley_y_min: f64,
    pub active_parley_y_max: f64,
}

/// Self-contained scene descriptor for a single physical PDF/paginated page.
#[derive(Debug, Clone)]
pub struct PageScene {
    pub page_index: usize,
    pub page_number: i64,
    pub width: f64,
    pub height: f64,
    pub dpi_scale: f32,
    pub layout: Arc<ResolvedLayout>,
    pub included_nodes: Option<HashSet<NodeId>>,
    pub node_offsets: HashMap<NodeId, (f64, f64)>,
    pub text_line_slices: HashMap<NodeId, TextSliceSpec>,
    pub page_node_range: Option<Range<usize>>,
    pub flow_body_node_id: Option<NodeId>,
}

impl PageScene {
    /// Returns the nodes active on this page ordered for rendering (Painter's Algorithm).
    ///
    /// Placed flow-body items (`!page_node_range.contains(&node.id.0)`) sort at the
    /// declaration slot of `\FlowBody` within the page template so page backgrounds
    /// declared before `\FlowBody` paint underneath flow content while overlays
    /// declared after `\FlowBody` paint above flow content.
    pub fn render_order(&self) -> Vec<&ResolvedNode> {
        let mut nodes: Vec<&ResolvedNode> = match &self.included_nodes {
            Some(included) => self
                .layout
                .nodes
                .iter()
                .filter(|n| included.contains(&n.id))
                .collect(),
            None => self.layout.nodes.iter().collect(),
        };

        let slot_key = |node: &ResolvedNode| -> (usize, usize) {
            if let (Some(range), Some(fb_id)) = (&self.page_node_range, self.flow_body_node_id) {
                if range.contains(&node.id.0) {
                    (node.id.0, 0)
                } else {
                    (fb_id.0, node.id.0.saturating_add(1))
                }
            } else {
                (node.id.0, 0)
            }
        };

        nodes.sort_by(|a, b| {
            a.z.partial_cmp(&b.z)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| slot_key(a).cmp(&slot_key(b)))
        });
        nodes
    }
}

/// Complete paginated output across all pages produced by a document's `\PagedFlow`.
#[derive(Debug, Clone)]
pub struct PaginatedDocument {
    pub pages: Vec<PageScene>,
    pub page_count: usize,
    pub end_page: i64,
    pub layout: Arc<ResolvedLayout>,
}

/// RAII guard restoring outer rewrite-tracking thread-locals after isolated page template expansion.
struct RewriteTrackingGuard(Option<RewriteTrackingSnapshot>);

impl Drop for RewriteTrackingGuard {
    fn drop(&mut self) {
        if let Some(snap) = self.0.take() {
            restore_rewrite_tracking(snap);
        }
    }
}

#[derive(Debug, Clone)]
enum FlowAtomKind {
    Paragraph {
        node_id: NodeId,
        shaped: Arc<ShapedParagraph>,
        inline_boxes: Vec<InlineBoxSpan>,
        /// `(deposit_idx, line_index)` pairs recorded in source order.
        inline_deposits: Vec<(usize, usize)>,
        /// Subtree node IDs attached per inline box index (`inline_boxes[idx].node_id`).
        inline_box_subtrees: Vec<Vec<NodeId>>,
        /// Non-box subtree nodes (spans, links, anchors) for this paragraph.
        span_node_ids: Vec<NodeId>,
    },
    AtomicBlock {
        node_id: NodeId,
        rect: Rect,
        subtree_node_ids: Vec<NodeId>,
        deposits: Vec<usize>,
        is_zero_marker: bool,
    },
    PageBreakDirective {
        node_id: NodeId,
        to: PageBreakTarget,
        deposits: Vec<usize>,
    },
}

#[derive(Debug, Clone)]
struct FlowAtom {
    kind: FlowAtomKind,
    rel_x: f64,
    gap_before: f64,
    keep_with_next: bool,
    break_before: PageBreakTarget,
    first_page: i64,
    last_page: i64,
    span: Span,
}

#[derive(Debug, Clone)]
struct ContainerSpan {
    node_id: NodeId,
    first_atom: usize,
    last_atom_exclusive: usize,
}

#[derive(Debug, Clone)]
enum PlacedFlowItem {
    Atomic {
        atom_idx: usize,
        target_x: f64,
        target_y: f64,
    },
    TextSlice {
        atom_idx: usize,
        line_range: Range<usize>,
        target_x: f64,
        target_y: f64,
    },
}

struct WorkingPage {
    page_index: usize,
    page_number: i64,
    page_root_id: NodeId,
    page_ctx_id: NodeId,
    flow_body_id: NodeId,
    node_start: usize,
    node_end: usize,
    lb_start: usize,
    flow_body_rect: Rect,
    page_width: f64,
    page_height: f64,
    cursor_y: f64,
    has_placed_items: bool,
    placed_items: Vec<PlacedFlowItem>,
    bucket_page_commits: Vec<usize>,
    uncollected_deposit_subtrees: HashSet<NodeId>,
}

fn collect_subtree_node_ids(root_id: NodeId, doc: &ExpandedDocument) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![root_id];
    let mut visited = HashSet::new();
    while let Some(nid) = stack.pop() {
        if !visited.insert(nid) {
            continue;
        }
        out.push(nid);
        if let Some(node) = doc.get_node(nid) {
            for &cid in node.children.iter().rev() {
                stack.push(cid);
            }
        }
    }
    out
}

fn extract_node_active_text(node: &ResolvedNode, layout: &ResolvedLayout) -> Option<String> {
    resolve_active_rich_text(node, layout).0
}

fn shape_resolved_text_node(node: &ResolvedNode, layout: &ResolvedLayout) -> ShapedParagraph {
    let (Some(active_text), resolved_spans, resolved_boxes, _) =
        resolve_active_rich_text(node, layout)
    else {
        return ShapedParagraph {
            lines: Vec::new(),
            total_width: 0.0,
            total_height: 0.0,
            first_top_coord: 0.0,
            last_bottom_coord: 0.0,
            full_y_offset: 0.0,
            byte_deltas: Vec::new(),
            sc_entries: Vec::new(),
        };
    };
    let fnode = node
        .font
        .or_else(|| node.properties.get("font").and_then(|v| v.as_node()))
        .and_then(|fid| layout.get_node(fid));

    let font_size = fnode
        .and_then(|f| f.properties.get("size"))
        .and_then(|v| v.as_f64())
        .unwrap_or(16.0);
    let font_weight = fnode
        .and_then(|f| f.properties.get("weight"))
        .and_then(|v| v.as_f64())
        .unwrap_or(400.0);
    let font_family = fnode
        .and_then(|f| f.properties.get("family"))
        .and_then(|v| v.as_str());
    let font_italic = fnode
        .and_then(|f| f.properties.get("italic"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let font_small_caps = fnode
        .and_then(|f| f.properties.get("small_caps"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let letter_spacing = fnode
        .and_then(|f| f.properties.get("letter_spacing"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let line_height = fnode
        .filter(|f| f.formulas.has_custom_line_height())
        .and_then(|f| f.properties.get("line_height"))
        .and_then(|v| v.as_f64())
        .filter(|&h| h > 0.0);

    let end_at = node.properties.get("end_at").and_then(|v| v.as_str());
    let start_at = node.properties.get("start_at").and_then(|v| v.as_str());
    let align = node.properties.get("align").and_then(|v| v.as_str());
    let indent = node
        .properties
        .get("indent")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let para_opts =
        crate::compiler::text::ParagraphLayoutOptions::from_node_properties(&node.properties);
    let max_width = (node.rect.width > 0.0).then_some(node.rect.width);

    crate::compiler::text::shape_paragraph_lines_with_options(
        &active_text,
        font_size,
        font_weight,
        font_family,
        letter_spacing,
        max_width,
        end_at,
        start_at,
        line_height,
        align,
        indent,
        font_italic,
        font_small_caps,
        &resolved_spans,
        &resolved_boxes,
        &para_opts,
    )
}

#[allow(clippy::too_many_arguments)]
fn flatten_flow_children(
    child_ids: &[NodeId],
    doc: &ExpandedDocument,
    layout: &ResolvedLayout,
    flow_origin_x: f64,
    prev_visual_bottom: &mut f64,
    pending_deposits: &mut Vec<usize>,
    pending_break_before: &mut PageBreakTarget,
    deposit_by_id: &HashMap<NodeId, usize>,
    atoms: &mut Vec<FlowAtom>,
    containers: &mut Vec<ContainerSpan>,
) {
    for &cid in child_ids {
        let Some(exp_node) = doc.get_node(cid) else {
            continue;
        };
        if matches!(
            exp_node.name.as_str(),
            "Font" | "Pagination" | "ParagraphStyle" | "PageValue" | "PageList" | "Collect"
        ) {
            continue;
        }
        if exp_node.name == "Deposit" {
            if let Some(&dep_idx) = deposit_by_id.get(&cid) {
                pending_deposits.push(dep_idx);
            }
            continue;
        }
        if exp_node.name == "PageBreak" {
            let to = exp_node
                .ports
                .get("to")
                .and_then(|e| eval_constant_break_target(e, doc))
                .unwrap_or(PageBreakTarget::Page);
            let combined_to = if to == PageBreakTarget::Auto {
                *pending_break_before
            } else {
                to
            };
            *pending_break_before = PageBreakTarget::Auto;
            let attached = std::mem::take(pending_deposits);
            atoms.push(FlowAtom {
                kind: FlowAtomKind::PageBreakDirective {
                    node_id: cid,
                    to: combined_to,
                    deposits: attached,
                },
                rel_x: 0.0,
                gap_before: 0.0,
                keep_with_next: false,
                break_before: PageBreakTarget::Auto,
                first_page: 1,
                last_page: 1,
                span: exp_node.span,
            });
            continue;
        }

        // Transparent breakable container (`\Section`, `\Chapter`, `\VStackStretch`, etc.)
        // or `\Anchor` / `\AnchorScope` / `\Bookmark` wrapper around a breakable subtree.
        let is_transparent_wrapper = matches!(
            exp_node.name.as_str(),
            "Anchor" | "AnchorScope" | "Bookmark"
        ) && !exp_node.children.is_empty();
        if (exp_node.is_component && exp_node.pagination.breakable) || is_transparent_wrapper {
            let first_atom_idx = atoms.len();
            if exp_node.pagination.break_before != PageBreakTarget::Auto
                && *pending_break_before == PageBreakTarget::Auto
            {
                *pending_break_before = exp_node.pagination.break_before;
            }
            flatten_flow_children(
                &exp_node.children,
                doc,
                layout,
                flow_origin_x,
                prev_visual_bottom,
                pending_deposits,
                pending_break_before,
                deposit_by_id,
                atoms,
                containers,
            );
            let last_atom_idx = atoms.len();
            if last_atom_idx > first_atom_idx {
                if exp_node.pagination.keep_with_next {
                    atoms[last_atom_idx - 1].keep_with_next = true;
                }
                containers.push(ContainerSpan {
                    node_id: cid,
                    first_atom: first_atom_idx,
                    last_atom_exclusive: last_atom_idx,
                });
            }
            continue;
        }

        let Some(res_node) = layout.get_node(cid) else {
            continue;
        };

        // Breakable `\Text` paragraph
        if exp_node.name == "Text" && exp_node.pagination.breakable {
            let shaped = Arc::new(shape_resolved_text_node(res_node, layout));
            let active_text = extract_node_active_text(res_node, layout).unwrap_or_default();
            let gap_before = (res_node.rect.y - *prev_visual_bottom).max(0.0);
            *prev_visual_bottom = (*prev_visual_bottom).max(res_node.rect.y + res_node.rect.height);

            // Compute splice shifts from the un-spliced ExpandedNode placeholders so
            // `exp_node.inline_deposits` byte offsets align cleanly with `active_text`.
            let deposit_shifts: Vec<(usize, usize)> = exp_node
                .text_spans
                .iter()
                .filter_map(|ts| {
                    if ts.range.is_empty() {
                        let cid2 = ts.node_id?;
                        let exp_child = doc.get_node(cid2)?;
                        let res_child = layout.get_node(cid2)?;
                        if exp_child.text_content.is_none() {
                            let s = res_child
                                .properties
                                .get("text")
                                .and_then(crate::compiler::text::format_text_port_value)?;
                            return Some((ts.range.start, s.len()));
                        }
                    }
                    None
                })
                .collect();

            let box_subtrees: Vec<Vec<NodeId>> = res_node
                .inline_boxes
                .iter()
                .map(|ib| collect_subtree_node_ids(ib.node_id, doc))
                .collect();
            let box_subtree_set: HashSet<NodeId> =
                box_subtrees.iter().flatten().copied().collect();
            let span_node_ids: Vec<NodeId> = collect_subtree_node_ids(cid, doc)
                .into_iter()
                .filter(|nid| !box_subtree_set.contains(nid))
                .collect();

            let mut inline_deps: Vec<(usize, usize)> = std::mem::take(pending_deposits)
                .into_iter()
                .map(|dep_idx| (dep_idx, 0))
                .collect();
            for &(dep_id, orig_byte_idx) in &exp_node.inline_deposits {
                if let Some(&dep_idx) = deposit_by_id.get(&dep_id) {
                    let ib_line_idx = box_subtrees
                        .iter()
                        .enumerate()
                        .find(|(_, sub)| sub.contains(&dep_id))
                        .and_then(|(ib_idx, _)| {
                            shaped
                                .lines
                                .iter()
                                .position(|l| l.inline_box_indices.contains(&ib_idx))
                        });
                    let line_idx = ib_line_idx.unwrap_or_else(|| {
                        let shift: usize = deposit_shifts
                            .iter()
                            .filter(|&&(p_pos, _)| p_pos <= orig_byte_idx)
                            .map(|&(_, len)| len)
                            .sum();
                        shaped.line_for_deposit_byte(orig_byte_idx + shift, &active_text)
                    });
                    inline_deps.push((dep_idx, line_idx));
                }
            }

            let atom_break = if exp_node.pagination.break_before != PageBreakTarget::Auto {
                let b = exp_node.pagination.break_before;
                *pending_break_before = PageBreakTarget::Auto;
                b
            } else {
                let b = *pending_break_before;
                *pending_break_before = PageBreakTarget::Auto;
                b
            };

            atoms.push(FlowAtom {
                kind: FlowAtomKind::Paragraph {
                    node_id: cid,
                    shaped,
                    inline_boxes: res_node.inline_boxes.clone(),
                    inline_deposits: inline_deps,
                    inline_box_subtrees: box_subtrees,
                    span_node_ids,
                },
                rel_x: res_node.rect.x - flow_origin_x,
                gap_before,
                keep_with_next: exp_node.pagination.keep_with_next,
                break_before: atom_break,
                first_page: 1,
                last_page: 1,
                span: exp_node.span,
            });
            continue;
        }

        // Atomic Block (`Component` with `breakable: false`, `Rect`, `Line`, standalone `InlineBox`,
        // unbreakable `Text`, or `Bookmark` / `Anchor`).
        if !exp_node.is_component
            && !exp_node.is_paint_primitive()
            && !exp_node.is_inline_box
            && !matches!(exp_node.name.as_str(), "Anchor" | "AnchorScope" | "Bookmark")
        {
            continue;
        }

        let subtree = collect_subtree_node_ids(cid, doc);
        let mut attached = std::mem::take(pending_deposits);
        for &sub_id in &subtree {
            if let Some(&dep_idx) = deposit_by_id.get(&sub_id) {
                attached.push(dep_idx);
            }
        }
        // Standalone zero-size Anchor/Bookmark sticks cleanly with the following heading/block.
        let is_zero_marker = matches!(exp_node.name.as_str(), "Bookmark" | "Anchor")
            && exp_node.children.is_empty();
        let kwn = exp_node.pagination.keep_with_next || is_zero_marker;

        let atom_break = if exp_node.pagination.break_before != PageBreakTarget::Auto {
            let b = exp_node.pagination.break_before;
            *pending_break_before = PageBreakTarget::Auto;
            b
        } else {
            let b = *pending_break_before;
            *pending_break_before = PageBreakTarget::Auto;
            b
        };

        let gap_before = if is_zero_marker {
            0.0
        } else {
            (res_node.rect.y - *prev_visual_bottom).max(0.0)
        };
        *prev_visual_bottom = (*prev_visual_bottom).max(res_node.rect.y + res_node.rect.height);

        atoms.push(FlowAtom {
            kind: FlowAtomKind::AtomicBlock {
                node_id: cid,
                rect: res_node.rect,
                subtree_node_ids: subtree,
                deposits: attached,
                is_zero_marker,
            },
            rel_x: res_node.rect.x - flow_origin_x,
            gap_before,
            keep_with_next: kwn,
            break_before: atom_break,
            first_page: 1,
            last_page: 1,
            span: exp_node.span,
        });
    }
}

/// Moves any explicit `break_before` backward across preceding standalone zero-height marker
/// chains (`\Bookmark` / empty `\Anchor` with `keep_with_next: true`) so markers travel onto the
/// new page alongside the block they annotate instead of staying on the prior page.
fn propagate_break_before_across_zero_markers(atoms: &mut [FlowAtom]) {
    if atoms.len() < 2 {
        return;
    }
    for i in (0..atoms.len() - 1).rev() {
        let is_zero_kwn = matches!(
            atoms[i].kind,
            FlowAtomKind::AtomicBlock {
                is_zero_marker: true,
                ..
            }
        ) && atoms[i].keep_with_next;
        if is_zero_kwn
            && atoms[i].break_before == PageBreakTarget::Auto
            && atoms[i + 1].break_before != PageBreakTarget::Auto
        {
            atoms[i].break_before = atoms[i + 1].break_before;
            atoms[i + 1].break_before = PageBreakTarget::Auto;
        }
    }
}

fn min_chain_entry_height(atoms: &[FlowAtom], idx: usize, orphans: usize, is_root_in_page: bool) -> f64 {
    if idx >= atoms.len() {
        return 0.0;
    }
    let atom = &atoms[idx];
    let gap = if is_root_in_page { 0.0 } else { atom.gap_before };
    match &atom.kind {
        FlowAtomKind::PageBreakDirective { .. } => 0.0,
        FlowAtomKind::AtomicBlock { rect, .. } => {
            let mut h = gap + rect.height;
            if atom.keep_with_next && idx + 1 < atoms.len() {
                h += min_chain_entry_height(atoms, idx + 1, orphans, false);
            }
            h
        }
        FlowAtomKind::Paragraph { shaped, .. } => {
            if atom.keep_with_next && idx + 1 < atoms.len() {
                let next_h = min_chain_entry_height(atoms, idx + 1, orphans, false);
                gap + shaped.total_height + next_h
            } else {
                let min_lines = orphans.min(shaped.lines.len()).max(1);
                gap + shaped.slice_height(0, min_lines)
            }
        }
    }
}

fn single_atom_min_height(atom: &FlowAtom, orphans: usize) -> f64 {
    match &atom.kind {
        FlowAtomKind::PageBreakDirective { .. } => 0.0,
        FlowAtomKind::AtomicBlock { rect, .. } => rect.height,
        FlowAtomKind::Paragraph { shaped, .. } => {
            if shaped.lines.is_empty() {
                0.0
            } else {
                let lines_needed = if atom.keep_with_next {
                    shaped.lines.len()
                } else {
                    orphans.min(shaped.lines.len()).max(1)
                };
                shaped.slice_height(0, lines_needed)
            }
        }
    }
}

/// Replaces `NodeId::PAGE` identifiers inside `[node_start..node_end]` with concrete per-page `page_ctx_id`.
fn remap_page_id_in_expr(expr: &mut Expr, page_ctx_id: NodeId) {
    match expr {
        Expr::Ident(id) => {
            if id.node_id().is_some_and(|nid| nid.is_page()) {
                *id = Ident::from_node(page_ctx_id, id.span);
            }
        }
        Expr::MemberAccess(m) => {
            remap_page_id_in_expr(Arc::make_mut(&mut m.target), page_ctx_id);
        }
        Expr::Index(idx) => {
            remap_page_id_in_expr(Arc::make_mut(&mut idx.target), page_ctx_id);
            remap_page_id_in_expr(Arc::make_mut(&mut idx.index), page_ctx_id);
        }
        Expr::Binary(b) => {
            remap_page_id_in_expr(Arc::make_mut(&mut b.left), page_ctx_id);
            remap_page_id_in_expr(Arc::make_mut(&mut b.right), page_ctx_id);
        }
        Expr::Unary(u) => {
            remap_page_id_in_expr(Arc::make_mut(&mut u.operand), page_ctx_id);
        }
        Expr::Ternary(t) => {
            remap_page_id_in_expr(Arc::make_mut(&mut t.condition), page_ctx_id);
            remap_page_id_in_expr(Arc::make_mut(&mut t.then_expr), page_ctx_id);
            remap_page_id_in_expr(Arc::make_mut(&mut t.else_expr), page_ctx_id);
        }
        Expr::Paren(inner, _) => {
            remap_page_id_in_expr(Arc::make_mut(inner), page_ctx_id);
        }
        Expr::Call(c) => {
            for arg in &mut c.args {
                remap_page_id_in_expr(arg, page_ctx_id);
            }
        }
        Expr::MethodCall(mc) => {
            remap_page_id_in_expr(Arc::make_mut(&mut mc.target), page_ctx_id);
            for arg in &mut mc.args {
                remap_page_id_in_expr(arg, page_ctx_id);
            }
        }
        Expr::ExprDef(def) => {
            remap_page_id_in_expr(Arc::make_mut(def).body.make_mut_if_needed(), page_ctx_id);
        }
        Expr::Match(m) => {
            if let Some(ref s) = m.scrutinee {
                let mut cloned_s = (**s).clone();
                remap_page_id_in_expr(&mut cloned_s, page_ctx_id);
                *Arc::make_mut(m) = crate::ast::MatchExpr {
                    scrutinee: Some(Arc::new(cloned_s)),
                    arms: m
                        .arms
                        .iter()
                        .map(|arm| {
                            let mut cloned_arm = arm.clone();
                            if let MatchPattern::Condition(ref mut cond) = cloned_arm.pattern {
                                remap_page_id_in_expr(cond, page_ctx_id);
                            }
                            remap_page_id_in_expr(&mut cloned_arm.body, page_ctx_id);
                            cloned_arm
                        })
                        .collect(),
                    span: m.span,
                };
            } else {
                let mut new_arms = Vec::with_capacity(m.arms.len());
                for arm in &m.arms {
                    let mut cloned_arm = arm.clone();
                    if let MatchPattern::Condition(ref mut cond) = cloned_arm.pattern {
                        remap_page_id_in_expr(cond, page_ctx_id);
                    }
                    remap_page_id_in_expr(&mut cloned_arm.body, page_ctx_id);
                    new_arms.push(cloned_arm);
                }
                Arc::make_mut(m).arms = new_arms;
            }
        }
        Expr::Node(elem) => {
            for p in &mut elem.ports {
                remap_page_id_in_expr(&mut p.expr, page_ctx_id);
            }
        }
        Expr::Literal(_) | Expr::DomNodeRef(_, _) => {}
    }
}

trait ArcMakeMutExt {
    fn make_mut_if_needed(&mut self) -> &mut Expr;
}

impl ArcMakeMutExt for Arc<Expr> {
    #[inline]
    fn make_mut_if_needed(&mut self) -> &mut Expr {
        Arc::make_mut(self)
    }
}

/// Evaluates only the transitive upstream subgraphs of `targets` in `graph`.
fn eval_vars_slice(
    graph: &crate::compiler::graph::VariableGraph,
    schedule: &crate::compiler::topo::TopologicalSchedule,
    targets: &[VarId],
) -> Result<HashMap<VarId, Value>, CompileError> {
    let num_vars = graph.var_nodes.len();
    let mut needed = vec![false; num_vars];
    let mut queue = VecDeque::new();
    for &t in targets {
        if let Some(tidx) = graph.lookup_var_index_sym(t.node, t.port) {
            if !needed[tidx.0 as usize] {
                needed[tidx.0 as usize] = true;
                queue.push_back(tidx);
            }
        }
    }
    while let Some(cur) = queue.pop_front() {
        for &up in &graph.upstream_idx[cur.0 as usize] {
            if !needed[up.0 as usize] {
                needed[up.0 as usize] = true;
                queue.push_back(up);
            }
        }
    }
    let mut env = HashMap::new();
    for &var_idx in &schedule.order_indices {
        let idx = var_idx.0 as usize;
        if !needed[idx] {
            continue;
        }
        let vnode = &graph.var_nodes[idx];
        let val = eval_expr_for_node(&vnode.equation, vnode.id.node, &env)?;
        env.insert(vnode.id, val);
    }
    Ok(env)
}

#[allow(clippy::too_many_arguments)]
fn expand_standalone_deposit_payload(
    pages_doc: &mut ExpandedDocument,
    dep_idx: usize,
    window_scope_ports: &[String],
    registry: &HashMap<String, Vec<ComponentDef>>,
    enums: &HashMap<String, EnumDef>,
    page_fonts: &mut HashMap<NodeId, NodeId>,
) -> Result<(NodeId, Range<usize>), CompileError> {
    let payload = pages_doc.deposits[dep_idx].payload.clone();
    let dep_start = pages_doc.nodes.len();
    let dep_lb = pages_doc.local_bindings.len();
    let dep_lex_cell = RefCell::new(payload.caller_lexical_scope);
    let sub_lex_cells: Vec<std::rc::Rc<RefCell<HashMap<Symbol, LexicalBinding>>>> = payload
        .outer_consumer_children
        .iter()
        .map(|oc| std::rc::Rc::new(RefCell::new(oc.caller_lexical_scope.clone())))
        .collect();
    let sub_entries: Vec<ConsumerChildEntry<'_>> = payload
        .outer_consumer_children
        .iter()
        .zip(sub_lex_cells.iter())
        .map(|(oc, lex_rc)| ConsumerChildEntry {
            elem: oc.elem.clone(),
            var_name: oc.var_name.clone(),
            raw_text: oc.raw_text.clone(),
            caller_lexical_scope: &dep_lex_cell,
            owned_caller_lexical_scope: Some(std::rc::Rc::clone(lex_rc)),
            caller_env_scope: oc.caller_env_scope.clone(),
            caller_enclosing_component_id: oc.caller_enclosing_component_id,
            caller_enclosing_num_children: oc.caller_enclosing_num_children,
            caller_enclosing_instantiated_children: None,
            caller_shadowed_ports: None,
            owned_shadowed_ports: (!oc.caller_shadowed_ports.is_empty())
                .then(|| oc.caller_shadowed_ports.clone()),
            instantiated_ids: Vec::new(),
            forwarded_directives: Vec::new(),
            deposit_payload_idx: None,
            owned_children: oc.outer_consumer_children.clone(),
        })
        .collect();

    let dep_ctx = ElementContext {
        parent_id: Some(NodeId::WINDOW),
        parent_name: Some("window"),
        prev_sibling_id: None,
        has_next_sibling: false,
        parent_ports: window_scope_ports,
        lexical_scope: &dep_lex_cell,
        env_scope: &payload.caller_env_scope,
        enclosing_component_id: payload.caller_enclosing_component_id,
        enclosing_num_children: payload.caller_enclosing_num_children,
        enclosing_instantiated_children: None,
        shadowed_ports: (!payload.caller_shadowed_ports.is_empty())
            .then_some(&payload.caller_shadowed_ports),
        is_let: false,
        ambient_authored_ports: None,
        enums,
        active_scope_id: ScopeId::ROOT,
        consumer_children: (!sub_entries.is_empty()).then_some(sub_entries.as_slice()),
        in_paged_flow: false,
        in_page_template: true,
        in_unbreakable_flow_container: None,
        enclosing_comp_ports: None,
    };

    let dep_root_id = expand_element(&payload.elem, &dep_ctx, registry, pages_doc, page_fonts)?;
    {
        let lex_borrow = dep_lex_cell.borrow();
        resolve_forward_refs_in_range(pages_doc, dep_start, dep_lb, &lex_borrow);
    }
    flush_all_materialized_expr_ports(pages_doc);
    let dep_end = pages_doc.nodes.len();
    Ok((dep_root_id, dep_start..dep_end))
}

#[allow(clippy::too_many_arguments)]
fn instantiate_page_in_doc(
    pages_doc: &mut ExpandedDocument,
    flow_rec: &PagedFlowRecord,
    page_index: usize,
    page_number: i64,
    prev_page_root: Option<NodeId>,
    active_deposits: &[usize],
    flow_base_len: usize,
    window_width: f64,
    window_height: f64,
    default_dpi: f64,
) -> Result<WorkingPage, CompileError> {
    let empty_reg = HashMap::new();
    let empty_enums = HashMap::new();
    let registry = pages_doc
        .paged_registry
        .clone()
        .unwrap_or_else(|| Arc::new(empty_reg));
    let enums = pages_doc
        .paged_enums
        .clone()
        .unwrap_or_else(|| Arc::new(empty_enums));

    let tmpl = &flow_rec.page_template_elem;
    let span = tmpl.elem.span;
    let node_start = pages_doc.nodes.len();
    let lb_start = pages_doc.local_bindings.len();
    let page_ctx_id = NodeId(node_start);
    let mut page_ctx_node = ExpandedNode::new(page_ctx_id, "PageContext", span);
    page_ctx_node.ports.insert(
        "__page_number".to_string(),
        Expr::Literal(Literal::Integer(page_number, span)),
    );
    pages_doc.nodes.push(page_ctx_node);

    let mut uncollected_deposit_subtrees = HashSet::new();

    let page_root_id = {
        let _guard = RewriteTrackingGuard(Some(save_rewrite_tracking()));
        set_window_port_types(&pages_doc.window_port_types);

        pages_doc.active_page_deposits.clear();
        for &dep_idx in active_deposits {
            if let Some(dep) = pages_doc.deposits.get(dep_idx) {
                pages_doc
                    .active_page_deposits
                    .entry(dep.bucket_id)
                    .or_default()
                    .push(dep_idx);
            }
        }
        pages_doc.active_page_deposit_nodes.clear();

        populate_page_ports(
            pages_doc,
            page_number,
            1,
            page_index == 0,
            false,
            prev_page_root,
            None,
            span,
        );

        let lex_cell = RefCell::new(tmpl.caller_lexical_scope.clone());
        let window_scope_ports = vec![
            "x".to_string(),
            "y".to_string(),
            "width".to_string(),
            "height".to_string(),
            "left".to_string(),
            "top".to_string(),
            "right".to_string(),
            "bottom".to_string(),
            "clip".to_string(),
            "dpi".to_string(),
        ];
        let mut page_fonts = HashMap::new();
        for n in &pages_doc.nodes {
            if let Some(fid) = n.font {
                page_fonts.insert(n.id, fid);
            } else if n.name == "Font" {
                page_fonts.insert(n.id, n.id);
            }
        }
        let elem_ctx = ElementContext {
            parent_id: Some(NodeId::WINDOW),
            parent_name: Some("window"),
            prev_sibling_id: None,
            has_next_sibling: false,
            parent_ports: &window_scope_ports,
            lexical_scope: &lex_cell,
            env_scope: &tmpl.caller_env_scope,
            enclosing_component_id: tmpl.caller_enclosing_component_id,
            enclosing_num_children: tmpl.caller_enclosing_num_children,
            enclosing_instantiated_children: None,
            shadowed_ports: (!tmpl.caller_shadowed_ports.is_empty())
                .then_some(&tmpl.caller_shadowed_ports),
            is_let: false,
            ambient_authored_ports: None,
            enums: enums.as_ref(),
            active_scope_id: ScopeId::ROOT,
            consumer_children: None,
            in_paged_flow: false,
            in_page_template: true,
            in_unbreakable_flow_container: None,
            enclosing_comp_ports: None,
        };

        let rid = expand_element(
            &tmpl.elem,
            &elem_ctx,
            registry.as_ref(),
            pages_doc,
            &mut page_fonts,
        )?;
        {
            let lex_borrow = lex_cell.borrow();
            resolve_forward_refs_in_range(
                pages_doc,
                page_root_start_idx(node_start),
                lb_start,
                &lex_borrow,
            );
        }
        flush_all_materialized_expr_ports(pages_doc);

        let mut deposit_node_by_idx: HashMap<usize, NodeId> = HashMap::new();
        for &(dep_idx, child_id) in &pages_doc.active_page_deposit_nodes {
            deposit_node_by_idx.entry(dep_idx).or_insert(child_id);
        }
        for &dep_idx in active_deposits {
            if !deposit_node_by_idx.contains_key(&dep_idx) {
                let (dep_root_id, dep_range) = expand_standalone_deposit_payload(
                    pages_doc,
                    dep_idx,
                    &window_scope_ports,
                    registry.as_ref(),
                    enums.as_ref(),
                    &mut page_fonts,
                )?;
                deposit_node_by_idx.insert(dep_idx, dep_root_id);
                for nid_idx in dep_range {
                    uncollected_deposit_subtrees.insert(NodeId(nid_idx));
                }
            }
        }
        pages_doc.active_page_deposits.clear();
        pages_doc.active_page_deposit_nodes.clear();

        let suffix = format!("_p_{page_number}");
        let safe_base = flow_base_len.min(pages_doc.nodes.len());
        for bnode in &mut pages_doc.nodes[..safe_base] {
            if bnode.name == "PageValue" || bnode.name == "PageList" {
                bnode.ports.retain(|k, _| !k.ends_with(&suffix));
            }
        }
        let mut roots_by_bucket: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for &dep_idx in active_deposits {
            if let (Some(dep), Some(&root_nid)) =
                (pages_doc.deposits.get(dep_idx), deposit_node_by_idx.get(&dep_idx))
            {
                roots_by_bucket.entry(dep.bucket_id).or_default().push(root_nid);
            }
        }
        for (bucket_id, roots) in roots_by_bucket {
            if roots.is_empty() {
                continue;
            }
            if let Some(bnode) = pages_doc.get_node_mut(bucket_id) {
                let bspan = bnode.span;
                let first_id = *roots.first().unwrap();
                let last_id = *roots.last().unwrap();
                if bnode.name == "PageValue" {
                    bnode.ports.insert(
                        format!("has_value_p_{page_number}"),
                        Expr::Literal(Literal::Bool(true, bspan)),
                    );
                    bnode.ports.insert(
                        format!("first_p_{page_number}"),
                        Expr::Ident(Ident::from_node(first_id, bspan)),
                    );
                    bnode.ports.insert(
                        format!("value_p_{page_number}"),
                        Expr::Ident(Ident::from_node(last_id, bspan)),
                    );
                } else if bnode.name == "PageList" {
                    bnode.ports.insert(
                        format!("len_p_{page_number}"),
                        Expr::Literal(Literal::Integer(roots.len() as i64, bspan)),
                    );
                    bnode.ports.insert(
                        format!("first_p_{page_number}"),
                        Expr::Ident(Ident::from_node(first_id, bspan)),
                    );
                    bnode.ports.insert(
                        format!("last_p_{page_number}"),
                        Expr::Ident(Ident::from_node(last_id, bspan)),
                    );
                }
            }
        }

        populate_page_ports(
            pages_doc,
            page_number,
            1,
            page_index == 0,
            false,
            prev_page_root,
            Some(rid),
            span,
        );
        for (lb_expr, _, _) in &mut pages_doc.local_bindings[lb_start..] {
            remap_page_id_in_expr(lb_expr, page_ctx_id);
        }
        rid
    };

    let page_ports_snapshot = pages_doc.page_ports.clone();
    if let Some(ctx_node) = pages_doc.get_node_mut(page_ctx_id) {
        for (k, mut v) in page_ports_snapshot {
            remap_page_id_in_expr(&mut v, page_ctx_id);
            ctx_node.ports.insert(k, v);
        }
        ctx_node.ports.insert(
            "__page_number".to_string(),
            Expr::Literal(Literal::Integer(page_number, span)),
        );
    }
    if let Some(root_node) = pages_doc.get_node_mut(page_root_id) {
        root_node.ports.insert(
            "__page_number".to_string(),
            Expr::Literal(Literal::Integer(page_number, span)),
        );
    }

    let node_end = pages_doc.nodes.len();
    for node in &mut pages_doc.nodes[node_start + 1..node_end] {
        for expr in node.ports.values_mut() {
            remap_page_id_in_expr(expr, page_ctx_id);
        }
    }

    let flow_body_id = pages_doc.nodes[node_start..node_end]
        .iter()
        .find(|n| n.name == "FlowBody")
        .map(|n| n.id)
        .ok_or(CompileError::InvalidFlowBodyCount { found: 0, span })?;

    let graph = build_variable_graph_with_window_and_dpi(
        pages_doc,
        window_width,
        window_height,
        default_dpi,
    )?;
    let schedule = sort_graph(&graph)?;
    let vals = eval_vars_slice(
        &graph,
        &schedule,
        &[
            VarId::new(flow_body_id, "x"),
            VarId::new(flow_body_id, "y"),
            VarId::new(flow_body_id, "width"),
            VarId::new(flow_body_id, "height"),
            VarId::new(page_ctx_id, "width"),
            VarId::new(page_ctx_id, "height"),
        ],
    )?;

    let get_f64 = |nid: NodeId, port: &str, default: f64| -> f64 {
        vals.get(&VarId::new(nid, port))
            .and_then(|v| v.as_f64())
            .unwrap_or(default)
    };
    let fb_x = get_f64(flow_body_id, "x", 0.0);
    let fb_y = get_f64(flow_body_id, "y", 0.0);
    let fb_w = get_f64(flow_body_id, "width", 468.0);
    let fb_h = get_f64(flow_body_id, "height", 648.0);
    let page_width = get_f64(page_ctx_id, "width", window_width).max(1.0);
    let page_height = get_f64(page_ctx_id, "height", window_height).max(1.0);

    Ok(WorkingPage {
        page_index,
        page_number,
        page_root_id,
        page_ctx_id,
        flow_body_id,
        node_start,
        node_end,
        lb_start,
        flow_body_rect: Rect::new(fb_x, fb_y, fb_w, fb_h),
        page_width,
        page_height,
        cursor_y: fb_y,
        has_placed_items: false,
        placed_items: Vec::new(),
        bucket_page_commits: active_deposits.to_vec(),
        uncollected_deposit_subtrees,
    })
}

#[allow(clippy::too_many_arguments)]
fn rebuild_open_page_for_deposits(
    pages_doc: &mut ExpandedDocument,
    working_pages: &mut Vec<WorkingPage>,
    flow_rec: &PagedFlowRecord,
    flow_base_len: usize,
    window_width: f64,
    window_height: f64,
    default_dpi: f64,
) -> Result<(), CompileError> {
    let prev_root = (working_pages.len() >= 2)
        .then(|| working_pages[working_pages.len() - 2].page_root_id);
    let old = working_pages.pop().expect("open tail page");
    let used_y = old.cursor_y - old.flow_body_rect.y;
    let old_fb_y = old.flow_body_rect.y;
    let mut placed_items = old.placed_items;
    let has_placed_items = old.has_placed_items;
    let commits = old.bucket_page_commits;

    pages_doc.nodes.truncate(old.node_start);
    pages_doc.local_bindings.truncate(old.lb_start);

    let mut rebuilt = instantiate_page_in_doc(
        pages_doc,
        flow_rec,
        old.page_index,
        old.page_number,
        prev_root,
        &commits,
        flow_base_len,
        window_width,
        window_height,
        default_dpi,
    )?;

    let delta_fb_y = rebuilt.flow_body_rect.y - old_fb_y;
    if delta_fb_y.abs() > 1e-6 {
        for item in &mut placed_items {
            match item {
                PlacedFlowItem::Atomic { target_y, .. }
                | PlacedFlowItem::TextSlice { target_y, .. } => {
                    *target_y += delta_fb_y;
                }
            }
        }
    }
    rebuilt.placed_items = placed_items;
    rebuilt.has_placed_items = has_placed_items;
    rebuilt.bucket_page_commits = commits;
    rebuilt.cursor_y = if has_placed_items {
        rebuilt.flow_body_rect.y + used_y
    } else {
        rebuilt.flow_body_rect.y
    };
    working_pages.push(rebuilt);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn commit_non_empty_deposits_with_advance(
    pages_doc: &mut ExpandedDocument,
    working_pages: &mut Vec<WorkingPage>,
    flow_rec: &PagedFlowRecord,
    deps: &[usize],
    flow_base_len: usize,
    window_width: f64,
    window_height: f64,
    default_dpi: f64,
    span: Span,
) -> Result<(), CompileError> {
    if deps.is_empty() {
        return Ok(());
    }
    let prev_len = working_pages.last().unwrap().bucket_page_commits.len();
    let had_items = working_pages.last().unwrap().has_placed_items;
    working_pages
        .last_mut()
        .unwrap()
        .bucket_page_commits
        .extend(deps.iter().copied());
    rebuild_open_page_for_deposits(
        pages_doc,
        working_pages,
        flow_rec,
        flow_base_len,
        window_width,
        window_height,
        default_dpi,
    )?;

    let cur = working_pages.last().unwrap();
    let bottom_limit = cur.flow_body_rect.y + cur.flow_body_rect.height;
    let fits = cur.flow_body_rect.height >= -1e-6 && cur.cursor_y <= bottom_limit + 1e-6;
    if fits {
        return Ok(());
    }
    let rem_h = cur.flow_body_rect.height;

    working_pages
        .last_mut()
        .unwrap()
        .bucket_page_commits
        .truncate(prev_len);
    rebuild_open_page_for_deposits(
        pages_doc,
        working_pages,
        flow_rec,
        flow_base_len,
        window_width,
        window_height,
        default_dpi,
    )?;

    if !had_items {
        return Err(CompileError::PaginationOverflowError {
            message: format!(
                "Page bucket deposits overflow empty \\FlowBody (remaining height {:.2}pt)",
                rem_h
            ),
            span,
        });
    }

    advance_for_break_target(
        pages_doc,
        working_pages,
        flow_rec,
        PageBreakTarget::Page,
        false,
        flow_base_len,
        window_width,
        window_height,
        default_dpi,
    )?;
    working_pages
        .last_mut()
        .unwrap()
        .bucket_page_commits
        .extend(deps.iter().copied());
    rebuild_open_page_for_deposits(
        pages_doc,
        working_pages,
        flow_rec,
        flow_base_len,
        window_width,
        window_height,
        default_dpi,
    )?;
    let new_cur = working_pages.last().unwrap();
    if new_cur.flow_body_rect.height < -1e-6 {
        return Err(CompileError::PaginationOverflowError {
            message: format!(
                "Page bucket deposits overflow empty \\FlowBody (remaining height {:.2}pt)",
                new_cur.flow_body_rect.height
            ),
            span,
        });
    }
    Ok(())
}

#[inline]
fn page_root_start_idx(node_start: usize) -> usize {
    node_start + 1
}

#[allow(clippy::too_many_arguments)]
fn advance_for_break_target(
    pages_doc: &mut ExpandedDocument,
    working_pages: &mut Vec<WorkingPage>,
    flow_rec: &PagedFlowRecord,
    to: PageBreakTarget,
    explicit_directive: bool,
    flow_base_len: usize,
    window_width: f64,
    window_height: f64,
    default_dpi: f64,
) -> Result<(), CompileError> {
    if to == PageBreakTarget::Auto {
        return Ok(());
    }
    let need_initial_break = {
        let cur = working_pages.last().unwrap();
        let parity_mismatch = match to {
            PageBreakTarget::Recto => cur.page_number.rem_euclid(2) != 1,
            PageBreakTarget::Verso => cur.page_number.rem_euclid(2) != 0,
            PageBreakTarget::Page | PageBreakTarget::Auto => false,
        };
        cur.has_placed_items || parity_mismatch || (explicit_directive && cur.has_placed_items)
    };
    if !need_initial_break {
        return Ok(());
    }
    loop {
        let next_idx = working_pages.len();
        let last = working_pages.last().unwrap();
        let next_num = last.page_number + 1;
        let prev_root = Some(last.page_root_id);
        let new_page = instantiate_page_in_doc(
            pages_doc,
            flow_rec,
            next_idx,
            next_num,
            prev_root,
            &[],
            flow_base_len,
            window_width,
            window_height,
            default_dpi,
        )?;
        let parity_ok = match to {
            PageBreakTarget::Page | PageBreakTarget::Auto => true,
            PageBreakTarget::Recto => new_page.page_number.rem_euclid(2) == 1,
            PageBreakTarget::Verso => new_page.page_number.rem_euclid(2) == 0,
        };
        working_pages.push(new_page);
        if parity_ok {
            break;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn pack_atoms_onto_pages(
    flow_doc: &ExpandedDocument,
    flow_rec: &PagedFlowRecord,
    atoms: &mut [FlowAtom],
    trailing_deposits: &[usize],
    flow_base_len: usize,
    start_page: i64,
    orphans: usize,
    widows: usize,
    window_width: f64,
    window_height: f64,
    default_dpi: f64,
) -> Result<(ExpandedDocument, Vec<WorkingPage>), CompileError> {
    let mut pages_doc = flow_doc.clone();
    pages_doc.paged_flows.clear();

    let mut working_pages = vec![instantiate_page_in_doc(
        &mut pages_doc,
        flow_rec,
        0,
        start_page,
        None,
        &[],
        flow_base_len,
        window_width,
        window_height,
        default_dpi,
    )?];

    for atom_idx in 0..atoms.len() {
        let break_before = atoms[atom_idx].break_before;
        if break_before != PageBreakTarget::Auto {
            advance_for_break_target(
                &mut pages_doc,
                &mut working_pages,
                flow_rec,
                break_before,
                false,
                flow_base_len,
                window_width,
                window_height,
                default_dpi,
            )?;
        }

        if let FlowAtomKind::PageBreakDirective {
            to,
            ref deposits,
            ..
        } = atoms[atom_idx].kind
        {
            if !deposits.is_empty() {
                let deps = deposits.clone();
                let sp = atoms[atom_idx].span;
                commit_non_empty_deposits_with_advance(
                    &mut pages_doc,
                    &mut working_pages,
                    flow_rec,
                    &deps,
                    flow_base_len,
                    window_width,
                    window_height,
                    default_dpi,
                    sp,
                )?;
            }
            let cur_num = working_pages.last().unwrap().page_number;
            atoms[atom_idx].first_page = cur_num;
            atoms[atom_idx].last_page = cur_num;
            advance_for_break_target(
                &mut pages_doc,
                &mut working_pages,
                flow_rec,
                to,
                true,
                flow_base_len,
                window_width,
                window_height,
                default_dpi,
            )?;
            continue;
        }

        // Check `keep_with_next` chain minimum height against current page if non-empty.
        if atoms[atom_idx].keep_with_next {
            let chain_min = min_chain_entry_height(
                atoms,
                atom_idx,
                orphans,
                !working_pages.last().unwrap().has_placed_items,
            );
            let cur = working_pages.last().unwrap();
            let avail = cur.flow_body_rect.y + cur.flow_body_rect.height - cur.cursor_y;
            if cur.has_placed_items && chain_min > avail + 1e-6 {
                advance_for_break_target(
                    &mut pages_doc,
                    &mut working_pages,
                    flow_rec,
                    PageBreakTarget::Page,
                    false,
                    flow_base_len,
                    window_width,
                    window_height,
                    default_dpi,
                )?;
            }
        }

        enum AtomStep {
            Atomic {
                rect: Rect,
                rel_x: f64,
                gap_before: f64,
                deposits: Vec<usize>,
                is_zero_marker: bool,
                span: Span,
            },
            Para {
                shaped: Arc<ShapedParagraph>,
                rel_x: f64,
                gap_before: f64,
                inline_deposits: Vec<(usize, usize)>,
                span: Span,
            },
        }
        let step = match &atoms[atom_idx].kind {
            FlowAtomKind::AtomicBlock {
                rect,
                deposits,
                is_zero_marker,
                ..
            } => AtomStep::Atomic {
                rect: *rect,
                rel_x: atoms[atom_idx].rel_x,
                gap_before: atoms[atom_idx].gap_before,
                deposits: deposits.clone(),
                is_zero_marker: *is_zero_marker,
                span: atoms[atom_idx].span,
            },
            FlowAtomKind::Paragraph {
                shaped,
                inline_deposits,
                ..
            } => AtomStep::Para {
                shaped: Arc::clone(shaped),
                rel_x: atoms[atom_idx].rel_x,
                gap_before: atoms[atom_idx].gap_before,
                inline_deposits: inline_deposits.clone(),
                span: atoms[atom_idx].span,
            },
            FlowAtomKind::PageBreakDirective { .. } => unreachable!(),
        };

        match step {
            AtomStep::Atomic {
                rect,
                rel_x,
                gap_before,
                deposits,
                is_zero_marker,
                span,
            } => {
                loop {
                    let cur_has_items = working_pages.last().unwrap().has_placed_items;
                    let gap = if cur_has_items { gap_before } else { 0.0 };
                    let prev_commits_len = working_pages.last().unwrap().bucket_page_commits.len();
                    if !deposits.is_empty() {
                        working_pages
                            .last_mut()
                            .unwrap()
                            .bucket_page_commits
                            .extend(deposits.iter().copied());
                        rebuild_open_page_for_deposits(
                            &mut pages_doc,
                            &mut working_pages,
                            flow_rec,
                            flow_base_len,
                            window_width,
                            window_height,
                            default_dpi,
                        )?;
                    }
                    let cur = working_pages.last().unwrap();
                    let bottom_limit = cur.flow_body_rect.y + cur.flow_body_rect.height;
                    let fits = cur.flow_body_rect.height >= -1e-6
                        && cur.cursor_y <= bottom_limit + 1e-6
                        && cur.cursor_y + gap + rect.height <= bottom_limit + 1e-6;
                    if fits {
                        let cur = working_pages.last_mut().unwrap();
                        let ty = cur.cursor_y + gap;
                        let tx = cur.flow_body_rect.x + rel_x;
                        cur.placed_items.push(PlacedFlowItem::Atomic {
                            atom_idx,
                            target_x: tx,
                            target_y: ty,
                        });
                        cur.cursor_y = ty + rect.height;
                        if !is_zero_marker {
                            cur.has_placed_items = true;
                        }
                        let pnum = cur.page_number;
                        atoms[atom_idx].first_page = pnum;
                        atoms[atom_idx].last_page = pnum;
                        break;
                    }
                    let overflow_msg = if cur.flow_body_rect.height < 0.0 {
                        format!(
                            "Page bucket deposits overflow empty \\FlowBody (remaining height {:.2}pt)",
                            cur.flow_body_rect.height
                        )
                    } else {
                        format!(
                            "Atomic block height ({:.2}pt) exceeds \\FlowBody height ({:.2}pt)",
                            rect.height, cur.flow_body_rect.height
                        )
                    };
                    if !deposits.is_empty() {
                        working_pages
                            .last_mut()
                            .unwrap()
                            .bucket_page_commits
                            .truncate(prev_commits_len);
                        rebuild_open_page_for_deposits(
                            &mut pages_doc,
                            &mut working_pages,
                            flow_rec,
                            flow_base_len,
                            window_width,
                            window_height,
                            default_dpi,
                        )?;
                    }
                    if !cur_has_items {
                        return Err(CompileError::PaginationOverflowError {
                            message: overflow_msg,
                            span,
                        });
                    }
                    advance_for_break_target(
                        &mut pages_doc,
                        &mut working_pages,
                        flow_rec,
                        PageBreakTarget::Page,
                        false,
                        flow_base_len,
                        window_width,
                        window_height,
                        default_dpi,
                    )?;
                }
            }
            AtomStep::Para {
                shaped,
                rel_x,
                gap_before,
                inline_deposits,
                span,
            } => {
                let total_lines = shaped.lines.len();
                if total_lines == 0 {
                    if !inline_deposits.is_empty() {
                        let deps: Vec<usize> = inline_deposits.iter().map(|&(d, _)| d).collect();
                        commit_non_empty_deposits_with_advance(
                            &mut pages_doc,
                            &mut working_pages,
                            flow_rec,
                            &deps,
                            flow_base_len,
                            window_width,
                            window_height,
                            default_dpi,
                            span,
                        )?;
                    }
                    let pnum = working_pages.last().unwrap().page_number;
                    atoms[atom_idx].first_page = pnum;
                    atoms[atom_idx].last_page = pnum;
                    continue;
                }
                let mut line_start = 0usize;
                let mut first_p = i64::MAX;
                let mut last_p = i64::MIN;

                while line_start < total_lines {
                    let (page_has_items, fb_h, base_commits_len) = {
                        let cur = working_pages.last().unwrap();
                        (
                            cur.has_placed_items,
                            cur.flow_body_rect.height,
                            cur.bucket_page_commits.len(),
                        )
                    };
                    let gap = if page_has_items && line_start == 0 {
                        gap_before
                    } else {
                        0.0
                    };

                    let mut fit_count = 0usize;
                    let mut commit_len_at_count: Vec<usize> = vec![base_commits_len];
                    let mut last_failed_due_to_deposit = false;

                    while line_start + fit_count < total_lines {
                        let line_idx = line_start + fit_count;
                        let line_deps: Vec<usize> = inline_deposits
                            .iter()
                            .filter(|&&(_, dl)| {
                                if line_idx + 1 == total_lines {
                                    dl >= line_idx
                                } else {
                                    dl == line_idx
                                }
                            })
                            .map(|&(d, _)| d)
                            .collect();

                        let prev_len = working_pages.last().unwrap().bucket_page_commits.len();
                        if !line_deps.is_empty() {
                            working_pages
                                .last_mut()
                                .unwrap()
                                .bucket_page_commits
                                .extend(line_deps.iter().copied());
                            rebuild_open_page_for_deposits(
                                &mut pages_doc,
                                &mut working_pages,
                                flow_rec,
                                flow_base_len,
                                window_width,
                                window_height,
                                default_dpi,
                            )?;
                        }

                        let cur = working_pages.last().unwrap();
                        let bottom_limit = cur.flow_body_rect.y + cur.flow_body_rect.height;
                        let cur_cursor_start = cur.cursor_y + gap;
                        let slice_h = shaped.slice_height(line_start, line_idx + 1);
                        let fits = cur.flow_body_rect.height >= -1e-6
                            && cur.cursor_y <= bottom_limit + 1e-6
                            && cur_cursor_start + slice_h <= bottom_limit + 1e-6;

                        if fits {
                            fit_count += 1;
                            commit_len_at_count.push(cur.bucket_page_commits.len());
                        } else {
                            last_failed_due_to_deposit =
                                cur.flow_body_rect.height < 0.0 || !line_deps.is_empty();
                            if !line_deps.is_empty() {
                                working_pages
                                    .last_mut()
                                    .unwrap()
                                    .bucket_page_commits
                                    .truncate(prev_len);
                                rebuild_open_page_for_deposits(
                                    &mut pages_doc,
                                    &mut working_pages,
                                    flow_rec,
                                    flow_base_len,
                                    window_width,
                                    window_height,
                                    default_dpi,
                                )?;
                            }
                            break;
                        }
                    }

                    let min_req = single_atom_min_height(&atoms[atom_idx], orphans);
                    let _ = min_req;

                    if fit_count == 0 {
                        if !page_has_items {
                            let one_h = shaped.slice_height(line_start, line_start + 1);
                            let msg = if last_failed_due_to_deposit {
                                format!(
                                    "Paragraph line ({:.2}pt) and its page bucket deposit/footnote exceed empty \\FlowBody height ({:.2}pt)",
                                    one_h, fb_h
                                )
                            } else {
                                format!(
                                    "Single paragraph line height ({:.2}pt) exceeds empty \\FlowBody height ({:.2}pt)",
                                    one_h, fb_h
                                )
                            };
                            return Err(CompileError::PaginationOverflowError {
                                message: msg,
                                span,
                            });
                        }
                        advance_for_break_target(
                            &mut pages_doc,
                            &mut working_pages,
                            flow_rec,
                            PageBreakTarget::Page,
                            false,
                            flow_base_len,
                            window_width,
                            window_height,
                            default_dpi,
                        )?;
                        continue;
                    }

                    // Enforce `orphans` on first slice of paragraph when page already has items.
                    if line_start == 0
                        && line_start + fit_count < total_lines
                        && fit_count < orphans
                        && page_has_items
                    {
                        if working_pages.last().unwrap().bucket_page_commits.len() > base_commits_len
                        {
                            working_pages
                                .last_mut()
                                .unwrap()
                                .bucket_page_commits
                                .truncate(base_commits_len);
                            rebuild_open_page_for_deposits(
                                &mut pages_doc,
                                &mut working_pages,
                                flow_rec,
                                flow_base_len,
                                window_width,
                                window_height,
                                default_dpi,
                            )?;
                        }
                        advance_for_break_target(
                            &mut pages_doc,
                            &mut working_pages,
                            flow_rec,
                            PageBreakTarget::Page,
                            false,
                            flow_base_len,
                            window_width,
                            window_height,
                            default_dpi,
                        )?;
                        continue;
                    }

                    // Enforce `widows` pullback if remainder would have fewer than `widows` lines.
                    let remaining_after = total_lines - (line_start + fit_count);
                    if remaining_after > 0 && remaining_after < widows {
                        let pullback = widows - remaining_after;
                        let adjusted = fit_count.saturating_sub(pullback);
                        if ((line_start == 0 && adjusted < orphans) || adjusted == 0)
                            && page_has_items
                        {
                            if working_pages.last().unwrap().bucket_page_commits.len()
                                > base_commits_len
                            {
                                working_pages
                                    .last_mut()
                                    .unwrap()
                                    .bucket_page_commits
                                    .truncate(base_commits_len);
                                rebuild_open_page_for_deposits(
                                    &mut pages_doc,
                                    &mut working_pages,
                                    flow_rec,
                                    flow_base_len,
                                    window_width,
                                    window_height,
                                    default_dpi,
                                )?;
                            }
                            advance_for_break_target(
                                &mut pages_doc,
                                &mut working_pages,
                                flow_rec,
                                PageBreakTarget::Page,
                                false,
                                flow_base_len,
                                window_width,
                                window_height,
                                default_dpi,
                            )?;
                            continue;
                        }
                        if adjusted > 0 && (line_start > 0 || adjusted >= orphans || !page_has_items)
                        {
                            fit_count = adjusted;
                            let target_commits_len = commit_len_at_count[fit_count];
                            if working_pages.last().unwrap().bucket_page_commits.len()
                                > target_commits_len
                            {
                                working_pages
                                    .last_mut()
                                    .unwrap()
                                    .bucket_page_commits
                                    .truncate(target_commits_len);
                                rebuild_open_page_for_deposits(
                                    &mut pages_doc,
                                    &mut working_pages,
                                    flow_rec,
                                    flow_base_len,
                                    window_width,
                                    window_height,
                                    default_dpi,
                                )?;
                            }
                        } else if !page_has_items {
                            fit_count = fit_count.max(1);
                        }
                    }

                    let line_end = line_start + fit_count;
                    let slice_h = shaped.slice_height(line_start, line_end);

                    let cur = working_pages.last_mut().unwrap();
                    let target_y = cur.cursor_y + gap;
                    let target_x = cur.flow_body_rect.x + rel_x;
                    let pnum = cur.page_number;
                    cur.placed_items.push(PlacedFlowItem::TextSlice {
                        atom_idx,
                        line_range: line_start..line_end,
                        target_x,
                        target_y,
                    });
                    cur.cursor_y = target_y + slice_h;
                    cur.has_placed_items = true;

                    first_p = first_p.min(pnum);
                    last_p = last_p.max(pnum);
                    line_start = line_end;

                    if line_start < total_lines {
                        advance_for_break_target(
                            &mut pages_doc,
                            &mut working_pages,
                            flow_rec,
                            PageBreakTarget::Page,
                            false,
                            flow_base_len,
                            window_width,
                            window_height,
                            default_dpi,
                        )?;
                    }
                }
                atoms[atom_idx].first_page = first_p;
                atoms[atom_idx].last_page = last_p;
            }
        }
    }

    if !trailing_deposits.is_empty() {
        let fallback_span = flow_rec.span;
        commit_non_empty_deposits_with_advance(
            &mut pages_doc,
            &mut working_pages,
            flow_rec,
            trailing_deposits,
            flow_base_len,
            window_width,
            window_height,
            default_dpi,
            fallback_span,
        )?;
    }

    Ok((pages_doc, working_pages))
}

fn collect_all_slice_parley_bounds(shaped: &ShapedParagraph, range: &Range<usize>) -> (f64, f64) {
    let mut y_min = f64::MAX;
    let mut y_max = f64::MIN;
    for idx in range.clone() {
        if let Some(line) = shaped.lines.get(idx) {
            y_min = y_min.min(line.parley_min_y);
            y_max = y_max.max(line.parley_max_y);
        }
    }
    if y_min > y_max {
        (0.0, 0.0)
    } else {
        (y_min, y_max)
    }
}

fn flow_nodes_differ(a: &ResolvedLayout, b: &ResolvedLayout, len: usize) -> bool {
    let n = len.min(a.nodes.len()).min(b.nodes.len());
    for i in 0..n {
        let na = &a.nodes[i];
        let nb = &b.nodes[i];
        if (na.rect.x - nb.rect.x).abs() > 1e-6
            || (na.rect.y - nb.rect.y).abs() > 1e-6
            || (na.rect.width - nb.rect.width).abs() > 1e-6
            || (na.rect.height - nb.rect.height).abs() > 1e-6
        {
            return true;
        }
        if extract_node_active_text(na, a) != extract_node_active_text(nb, b) {
            return true;
        }
    }
    false
}

/// Executes Phase 1–3 pagination across up to 3 cross-reference passes for a document containing `\PagedFlow`.
pub fn paginate_compiled_document(
    compiled: &CompiledDocument,
    window_width: f64,
    window_height: f64,
    default_dpi: f64,
) -> Result<PaginatedDocument, CompileError> {
    let mut flow_doc = compiled.expanded.clone();
    if flow_doc.paged_flows.is_empty() {
        let layout = Arc::new(compiled.layout.clone());
        let page_w = layout
            .get_value(NodeId::WINDOW, "width")
            .and_then(|v| v.as_f64())
            .unwrap_or(window_width)
            .max(1.0);
        let page_h = layout
            .get_value(NodeId::WINDOW, "height")
            .and_then(|v| v.as_f64())
            .unwrap_or(window_height)
            .max(1.0);
        return Ok(PaginatedDocument {
            pages: vec![PageScene {
                page_index: 0,
                page_number: 1,
                width: page_w,
                height: page_h,
                dpi_scale: 1.0,
                layout: Arc::clone(&layout),
                included_nodes: None,
                node_offsets: HashMap::new(),
                text_line_slices: HashMap::new(),
                page_node_range: None,
                flow_body_node_id: None,
            }],
            page_count: 1,
            end_page: 1,
            layout,
        });
    }

    let flow_rec_initial = flow_doc.paged_flows[0].clone();
    let flow_id = flow_rec_initial.node_id;

    // Re-evaluate window-dependent `FlowBody.width` if needed at exact active dimensions.
    if flow_rec_initial.width_depends_on_window {
        let pgraph = build_variable_graph_with_window_and_dpi(
            &flow_rec_initial.probe_doc,
            window_width,
            window_height,
            default_dpi,
        )?;
        let psched = sort_graph(&pgraph)?;
        if let Ok(Value::Distance(new_w)) = eval_var_slice(
            &pgraph,
            &psched,
            VarId::new(flow_rec_initial.flow_body_node_id, "width"),
        ) {
            if let Some(fnode) = flow_doc.get_node_mut(flow_id) {
                fnode.ports.insert(
                    "__flow_body_width".to_string(),
                    Expr::Literal(Literal::Distance(
                        new_w,
                        crate::ast::DistanceUnit::Px,
                        flow_rec_initial.span,
                    )),
                );
            }
        }
    }

    let mut flow_layout = if flow_rec_initial.width_depends_on_window {
        let g = Arc::new(build_variable_graph_with_window_and_dpi(
            &flow_doc,
            window_width,
            window_height,
            default_dpi,
        )?);
        let sched = sort_graph(&g)?;
        let vals = evaluate_graph(&g, &sched)?;
        resolve_layout_with_graph(&flow_doc, vals, Some(g))
    } else {
        compiled.layout.clone()
    };

    let base_flow_len = flow_doc.nodes.len();
    let deposit_by_id: HashMap<NodeId, usize> = flow_doc
        .deposits
        .iter()
        .enumerate()
        .map(|(i, d)| (d.deposit_id, i))
        .collect();

    const MAX_PASSES: usize = 3;
    for pass in 0..MAX_PASSES {
        let orphans = flow_layout
            .get_value(flow_id, "orphans")
            .and_then(|v| v.as_i64())
            .unwrap_or(2)
            .max(1) as usize;
        let widows = flow_layout
            .get_value(flow_id, "widows")
            .and_then(|v| v.as_i64())
            .unwrap_or(2)
            .max(1) as usize;
        let start_page = flow_layout
            .get_value(flow_id, "start_page")
            .and_then(|v| v.as_i64())
            .unwrap_or(1);
        let flow_gap = flow_layout
            .get_value(flow_id, "gap")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let flow_rect = flow_layout
            .get_node(flow_id)
            .map(|n| n.rect)
            .unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0));
        let flow_children = flow_doc.nodes[flow_id.0].children.clone();

        let mut prev_visual_bottom = flow_rect.y;
        let mut pending_deposits = Vec::new();
        let mut pending_break = PageBreakTarget::Auto;
        let mut atoms = Vec::new();
        let mut containers = Vec::new();

        flatten_flow_children(
            &flow_children,
            &flow_doc,
            &flow_layout,
            flow_rect.x,
            &mut prev_visual_bottom,
            &mut pending_deposits,
            &mut pending_break,
            &deposit_by_id,
            &mut atoms,
            &mut containers,
        );
        propagate_break_before_across_zero_markers(&mut atoms);

        let (mut pages_doc, working_pages) = pack_atoms_onto_pages(
            &flow_doc,
            &flow_rec_initial,
            &mut atoms,
            &pending_deposits,
            base_flow_len,
            start_page,
            orphans,
            widows,
            window_width,
            window_height,
            default_dpi,
        )?;

        let total_pages = working_pages.len();
        let end_page = start_page + total_pages as i64 - 1;
        let total_width = working_pages
            .iter()
            .map(|p| p.page_width)
            .fold(0.0_f64, f64::max);
        let total_height = working_pages.iter().map(|p| p.page_height).sum::<f64>()
            + (total_pages.saturating_sub(1) as f64) * flow_gap;

        // Assign updated `.page`, `.end_page`, `page_count`, `width`, `height` on `flow_doc` AND `pages_doc`.
        let set_port_int = |doc: &mut ExpandedDocument, nid: NodeId, port: &str, v: i64| {
            if let Some(node) = doc.get_node_mut(nid) {
                let sp = node.span;
                node.ports
                    .insert(port.to_string(), Expr::Literal(Literal::Integer(v, sp)));
            }
        };
        let set_port_dist = |doc: &mut ExpandedDocument, nid: NodeId, port: &str, v: f64| {
            if let Some(node) = doc.get_node_mut(nid) {
                let sp = node.span;
                node.ports.insert(
                    port.to_string(),
                    Expr::Literal(Literal::Distance(v, crate::ast::DistanceUnit::Px, sp)),
                );
            }
        };

        for doc_target in [&mut flow_doc, &mut pages_doc] {
            set_port_int(doc_target, flow_id, "page_count", total_pages as i64);
            set_port_int(doc_target, flow_id, "end_page", end_page);
            set_port_dist(doc_target, flow_id, "width", total_width);
            set_port_dist(doc_target, flow_id, "height", total_height);

            for atom in &atoms {
                let root_nid = match &atom.kind {
                    FlowAtomKind::Paragraph { node_id, .. }
                    | FlowAtomKind::AtomicBlock { node_id, .. }
                    | FlowAtomKind::PageBreakDirective { node_id, .. } => *node_id,
                };
                set_port_int(doc_target, root_nid, "page", atom.first_page);
                set_port_int(doc_target, root_nid, "end_page", atom.last_page);
                if let FlowAtomKind::AtomicBlock {
                    subtree_node_ids, ..
                } = &atom.kind
                {
                    for &sid in subtree_node_ids {
                        set_port_int(doc_target, sid, "page", atom.first_page);
                        set_port_int(doc_target, sid, "end_page", atom.last_page);
                    }
                }
            }
            for cont in &containers {
                let first_p = atoms[cont.first_atom].first_page;
                let last_p = atoms[cont.last_atom_exclusive - 1].last_page;
                set_port_int(doc_target, cont.node_id, "page", first_p);
                set_port_int(doc_target, cont.node_id, "end_page", last_p);
            }
        }

        // Re-evaluate flow graph once with settled `.page` / `.end_page` / `page_count`
        let next_flow_graph = Arc::new(build_variable_graph_with_window_and_dpi(
            &flow_doc,
            window_width,
            window_height,
            default_dpi,
        )?);
        let next_flow_schedule = sort_graph(&next_flow_graph)?;
        let next_flow_vals = evaluate_graph(&next_flow_graph, &next_flow_schedule)?;
        let next_flow_layout =
            resolve_layout_with_graph(&flow_doc, next_flow_vals, Some(next_flow_graph));

        let changed = flow_nodes_differ(&flow_layout, &next_flow_layout, base_flow_len);
        if changed {
            if pass + 1 >= MAX_PASSES {
                return Err(CompileError::PaginationConvergenceError {
                    message: "Forward .page / page.count references altered flow layout across 3 passes"
                        .to_string(),
                    span: flow_rec_initial.span,
                });
            }
            flow_layout = next_flow_layout;
            continue;
        }

        // Phase 3: Finalize `page.count` & `page.is_last` across all pages and evaluate chrome once.
        for (k, wp) in working_pages.iter().enumerate() {
            let sp = pages_doc.nodes[wp.page_ctx_id.0].span;
            if let Some(ctx_node) = pages_doc.get_node_mut(wp.page_ctx_id) {
                ctx_node.ports.insert(
                    "count".to_string(),
                    Expr::Literal(Literal::Integer(total_pages as i64, sp)),
                );
                ctx_node.ports.insert(
                    "is_last".to_string(),
                    Expr::Literal(Literal::Bool(k + 1 == total_pages, sp)),
                );
            }
            for placed in &wp.placed_items {
                if let PlacedFlowItem::TextSlice {
                    atom_idx,
                    line_range,
                    target_y,
                    ..
                } = placed
                {
                    if let FlowAtomKind::Paragraph {
                        node_id,
                        shaped,
                        inline_boxes,
                        ..
                    } = &atoms[*atom_idx].kind
                    {
                        if inline_boxes.is_empty() {
                            continue;
                        }
                        if let Some(text_rnode) = next_flow_layout.get_node(*node_id) {
                            let slice_top = shaped.slice_top(line_range.start);
                            let box_dy_adjust = if line_range.start > 0 {
                                *target_y - slice_top - shaped.full_y_offset - text_rnode.rect.y
                            } else {
                                *target_y - text_rnode.rect.y
                            };
                            let mut active_box_indices = HashSet::new();
                            for lidx in line_range.clone() {
                                if let Some(line) = shaped.lines.get(lidx) {
                                    for &ib_idx in &line.inline_box_indices {
                                        active_box_indices.insert(ib_idx);
                                    }
                                }
                            }
                            for (ib_idx, ib) in inline_boxes.iter().enumerate() {
                                if active_box_indices.contains(&ib_idx) {
                                    if let Some(flow_baseline) = next_flow_layout
                                        .get_value(ib.node_id, "baseline")
                                        .and_then(|v| v.as_f64())
                                    {
                                        if let Some(bnode) = pages_doc.get_node_mut(ib.node_id) {
                                            let bsp = bnode.span;
                                            bnode.ports.insert(
                                                "baseline".to_string(),
                                                Expr::Literal(Literal::Distance(
                                                    flow_baseline + box_dy_adjust,
                                                    DistanceUnit::Px,
                                                    bsp,
                                                )),
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let pages_graph = Arc::new(build_variable_graph_with_window_and_dpi(
            &pages_doc,
            window_width,
            window_height,
            default_dpi,
        )?);
        let pages_schedule = sort_graph(&pages_graph)?;
        let pages_vals = evaluate_graph(&pages_graph, &pages_schedule)?;
        let mut pages_layout =
            resolve_layout_with_graph(&pages_doc, pages_vals, Some(pages_graph));

        // Preserve flow layout's Phase 1 nodes + updated `.page`/`.end_page` cleanly at indices 0..base_flow_len.
        for i in 0..base_flow_len.min(next_flow_layout.nodes.len()) {
            pages_layout.nodes[i] = next_flow_layout.nodes[i].clone();
        }
        let shared_layout = Arc::new(pages_layout);

        let resolved_dpi = shared_layout
            .get_value(NodeId::WINDOW, "dpi")
            .and_then(|v| v.as_f64())
            .unwrap_or(default_dpi);
        let explicit_dpi_override = flow_doc.window_ports.contains_key("dpi")
            && resolved_dpi > 0.0
            && (resolved_dpi - 72.0).abs() > 1e-6;
        let dpi_scale = if explicit_dpi_override {
            (72.0 / resolved_dpi) as f32
        } else {
            1.0
        };

        let mut scenes = Vec::with_capacity(total_pages);
        for wp in &working_pages {
            let mut included = HashSet::new();
            let mut offsets = HashMap::new();
            let mut slices = HashMap::new();

            for nid_idx in wp.node_start..wp.node_end {
                let nid = NodeId(nid_idx);
                if !wp.uncollected_deposit_subtrees.contains(&nid) {
                    included.insert(nid);
                }
            }

            for placed in &wp.placed_items {
                match placed {
                    PlacedFlowItem::Atomic {
                        atom_idx,
                        target_x,
                        target_y,
                    } => {
                        if let FlowAtomKind::AtomicBlock {
                            rect,
                            subtree_node_ids,
                            ..
                        } = &atoms[*atom_idx].kind
                        {
                            let dx = *target_x - rect.x;
                            let dy = *target_y - rect.y;
                            for &sid in subtree_node_ids {
                                included.insert(sid);
                                offsets.insert(sid, (dx, dy));
                            }
                        }
                    }
                    PlacedFlowItem::TextSlice {
                        atom_idx,
                        line_range,
                        target_x,
                        target_y,
                    } => {
                        if let FlowAtomKind::Paragraph {
                            node_id,
                            shaped,
                            inline_boxes,
                            inline_box_subtrees,
                            span_node_ids,
                            ..
                        } = &atoms[*atom_idx].kind
                        {
                            let Some(text_rnode) = shared_layout.get_node(*node_id) else {
                                continue;
                            };
                            let dx = *target_x - text_rnode.rect.x;
                            let dy = *target_y - text_rnode.rect.y;
                            let slice_top = shaped.slice_top(line_range.start);
                            let slice_h = shaped.slice_height(line_range.start, line_range.end);
                            let (pmin, pmax) = collect_all_slice_parley_bounds(shaped, line_range);

                            for &sid in span_node_ids {
                                included.insert(sid);
                                offsets.insert(sid, (dx, dy));
                            }
                            slices.insert(
                                *node_id,
                                TextSliceSpec {
                                    line_range: line_range.clone(),
                                    slice_top_offset: slice_top,
                                    slice_height: slice_h,
                                    full_y_offset: shaped.full_y_offset,
                                    active_parley_y_min: pmin,
                                    active_parley_y_max: pmax,
                                },
                            );

                            let box_dy_adjust = if line_range.start > 0 {
                                *target_y - slice_top - shaped.full_y_offset - text_rnode.rect.y
                            } else {
                                dy
                            };
                            let mut active_box_indices = HashSet::new();
                            for lidx in line_range.clone() {
                                if let Some(line) = shaped.lines.get(lidx) {
                                    for &ib_idx in &line.inline_box_indices {
                                        active_box_indices.insert(ib_idx);
                                    }
                                }
                            }
                            for (ib_idx, _ib) in inline_boxes.iter().enumerate() {
                                if active_box_indices.contains(&ib_idx) {
                                    for &bnid in &inline_box_subtrees[ib_idx] {
                                        included.insert(bnid);
                                        offsets.insert(bnid, (dx, box_dy_adjust));
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Include transparent containers (including `\Anchor` / `\AnchorScope` wrappers)
            // on the page where their first flow atom starts so PDF link indexing sees their screen position.
            for cont in &containers {
                let Some(cnode) = shared_layout.get_node(cont.node_id) else {
                    continue;
                };
                let first_placement = wp.placed_items.iter().find_map(|placed| match placed {
                    PlacedFlowItem::Atomic {
                        atom_idx,
                        target_x,
                        target_y,
                    } if *atom_idx == cont.first_atom => Some((*target_x, *target_y)),
                    PlacedFlowItem::TextSlice {
                        atom_idx,
                        line_range,
                        target_x,
                        target_y,
                    } if *atom_idx == cont.first_atom && line_range.start == 0 => {
                        Some((*target_x, *target_y))
                    }
                    _ => None,
                });
                if let Some((tx, ty)) = first_placement {
                    included.insert(cont.node_id);
                    offsets.insert(cont.node_id, (tx - cnode.rect.x, ty - cnode.rect.y));
                }
            }

            scenes.push(PageScene {
                page_index: wp.page_index,
                page_number: wp.page_number,
                width: wp.page_width * dpi_scale as f64,
                height: wp.page_height * dpi_scale as f64,
                dpi_scale,
                layout: Arc::clone(&shared_layout),
                included_nodes: Some(included),
                node_offsets: offsets,
                text_line_slices: slices,
                page_node_range: Some(wp.node_start..wp.node_end),
                flow_body_node_id: Some(wp.flow_body_id),
            });
        }

        return Ok(PaginatedDocument {
            pages: scenes,
            page_count: total_pages,
            end_page,
            layout: shared_layout,
        });
    }

    Err(CompileError::PaginationConvergenceError {
        message: "Failed to converge within 3 pagination passes".to_string(),
        span: flow_rec_initial.span,
    })
}

