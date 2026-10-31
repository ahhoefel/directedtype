use super::context::LexicalBinding;
use crate::ast::*;
use crate::compiler::expanded::{ExpandedDocument, ExpandedNode, NodeId};
use crate::compiler::text::{CursorKind, InlineBoxSpan, SpanStyle, TextSpan};
use crate::span::Span;
use std::collections::{HashMap, HashSet};

pub(super) fn is_span_element(
    elem: &ElementNode,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> bool {
    if let Some(overloads) = registry.get(elem.name.as_str()) {
        return overloads
            .iter()
            .any(|def| def.kind == ComponentKind::Span);
    }
    elem.name.as_str() == "Link"
}

pub(super) fn is_inline_box_element(
    elem: &ElementNode,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> bool {
    if let Some(overloads) = registry.get(elem.name.as_str()) {
        return overloads
            .iter()
            .any(|def| def.kind == ComponentKind::InlineBox);
    }
    false
}

pub(super) fn is_inline_element(
    elem: &ElementNode,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> bool {
    is_span_element(elem, registry) || is_inline_box_element(elem, registry)
}

pub(super) fn is_allowed_in_single_text(
    elem: &ElementNode,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> bool {
    if is_inline_element(elem, registry) {
        return true;
    }
    if registry.contains_key(elem.name.as_str()) {
        return false;
    }
    matches!(elem.name.as_str(), "Font" | "Anchor" | "Text" | "Link")
}

pub(super) fn element_places_children_in_single_text(
    elem: &ElementNode,
    registry: &HashMap<String, Vec<ComponentDef>>,
    visited: &mut HashSet<String>,
) -> Option<bool> {
    let slot = elem.content.as_ref()?;
    let has_direct_children_directive = slot
        .items
        .iter()
        .any(|it| matches!(it, ContentItem::Children(_)));
    if has_direct_children_directive {
        if elem.name.as_str() == "Text" {
            return Some(true);
        }
        if let Some(overloads) = registry.get(elem.name.as_str()) {
            if visited.insert(elem.name.as_str().to_string()) {
                for def in overloads {
                    if component_children_is_single_text_with_visited(def, registry, visited) {
                        visited.remove(elem.name.as_str());
                        return Some(true);
                    }
                }
                visited.remove(elem.name.as_str());
            }
            return Some(false);
        }
        return Some(false);
    }
    for item in &slot.items {
        match item {
            ContentItem::Node(child) => {
                if let Some(res) =
                    element_places_children_in_single_text(child, registry, visited)
                {
                    return Some(res);
                }
            }
            ContentItem::Let(l) => {
                if let Some(LetValue::Node(child)) = &l.value {
                    if let Some(res) =
                        element_places_children_in_single_text(child, registry, visited)
                    {
                        return Some(res);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

pub(super) fn component_children_is_single_text_with_visited(
    comp_def: &ComponentDef,
    registry: &HashMap<String, Vec<ComponentDef>>,
    visited: &mut HashSet<String>,
) -> bool {
    if comp_def.kind == ComponentKind::Span {
        return true;
    }
    for item in &comp_def.body {
        match item {
            ComponentBodyItem::Children(_) => return false,
            ComponentBodyItem::Node(elem) => {
                if let Some(res) =
                    element_places_children_in_single_text(elem, registry, visited)
                {
                    return res;
                }
            }
            ComponentBodyItem::Let(l) => {
                if let Some(LetValue::Node(elem)) = &l.value {
                    if let Some(res) =
                        element_places_children_in_single_text(elem, registry, visited)
                    {
                        return res;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

pub(super) fn component_children_is_single_text(
    comp_def: &ComponentDef,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> bool {
    let mut visited = HashSet::new();
    visited.insert(comp_def.name.as_str().to_string());
    component_children_is_single_text_with_visited(comp_def, registry, &mut visited)
}

/// Accumulates an expanded child node's text, spans, and inline boxes into an enclosing `\Text` primitive.
pub(super) fn accumulate_text_child(
    doc: &ExpandedDocument,
    child_id: NodeId,
    child_elem: &ElementNode,
    full_text: &mut String,
    text_spans: &mut Vec<TextSpan>,
    inline_boxes: &mut Vec<InlineBoxSpan>,
    fallback_slot_text: bool,
) {
    if doc.nodes[child_id.0].is_inline_box {
        inline_boxes.push(InlineBoxSpan {
            node_id: child_id,
            index: full_text.len(),
            width: 0.0,
            height: 0.0,
            baseline_offset: 0.0,
        });
        return;
    }

    let child_text = doc.nodes[child_id.0]
        .text_content
        .clone()
        .or_else(|| {
            if fallback_slot_text {
                if let Some(c_slot) = &child_elem.content {
                    let mut parts = Vec::new();
                    for it in &c_slot.items {
                        if let ContentItem::Text(t) = it {
                            parts.push(t.text.as_str());
                        }
                    }
                    if !parts.is_empty() {
                        return Some(parts.join(" "));
                    }
                }
            }
            None
        })
        .unwrap_or_default();

    let start = full_text.len();
    full_text.push_str(&child_text);
    let end = full_text.len();

    for mut sub_ib in doc.nodes[child_id.0].inline_boxes.clone() {
        sub_ib.index += start;
        inline_boxes.push(sub_ib);
    }

    if doc.nodes[child_id.0].is_span && !doc.nodes[child_id.0].text_spans.is_empty() {
        for mut sub_span in doc.nodes[child_id.0].text_spans.clone() {
            sub_span.range = (start + sub_span.range.start)..(start + sub_span.range.end);
            sub_span.push_parent_node(child_id);
            text_spans.push(sub_span);
        }
    } else {
        let is_link = child_elem.name.as_str() == "Link"
            || child_elem.ports.iter().any(|p| p.name.as_str() == "url");

        let mut style = SpanStyle::default();
        if is_link {
            let mut url_val = None;
            let mut color_val = Some("#1a73e8".to_string());
            let mut underline_val = true;

            for p in &child_elem.ports {
                match p.name.as_str() {
                    "url" => {
                        if let Expr::Literal(Literal::String(s, _)) = &p.expr {
                            url_val = Some(s.clone());
                        }
                    }
                    "color" => {
                        if let Expr::Literal(Literal::Color(c, _))
                        | Expr::Literal(Literal::String(c, _)) = &p.expr
                        {
                            color_val = Some(c.clone());
                        }
                    }
                    "underline" => {
                        if let Expr::Literal(Literal::Bool(b, _)) = &p.expr {
                            underline_val = *b;
                        }
                    }
                    _ => {}
                }
            }

            style.url = url_val;
            style.color = color_val;
            style.underline = underline_val;
            style.cursor = Some(CursorKind::Pointer);
        }

        text_spans.push(
            TextSpan::new(start..end)
                .with_node(child_id)
                .with_style(style),
        );
    }
}

/// Finalizes style inheritance and synthetic metric ports (`size`, `weight`, `family`, `x`, `y`, `width`, `height`, `baseline`) for a `\Span` component instance.
#[allow(clippy::too_many_arguments)]
pub(super) fn finalize_span_component_ports(
    comp_node_id: NodeId,
    span: Span,
    doc: &mut ExpandedDocument,
    final_local_scope: &HashMap<String, LexicalBinding>,
    node_fonts: &mut HashMap<NodeId, NodeId>,
    rewritten_ports: &mut HashMap<String, Expr>,
    span_full_text: &Option<String>,
    span_text_spans: &mut [TextSpan],
) {
    if let Some(&LexicalBinding::Node(inner_font_id)) = final_local_scope.get("font") {
        if doc.get_node(inner_font_id).is_some_and(|n| n.name == "Font") {
            node_fonts.insert(comp_node_id, inner_font_id);
            doc.get_node_mut(comp_node_id).unwrap().font = Some(inner_font_id);
            rewritten_ports.insert(
                "font".to_string(),
                Expr::Ident(Ident::new(inner_font_id.canonical_name(), span)),
            );
        }
    }
    let mut own_style = SpanStyle::default();
    if let Some(url_expr) = rewritten_ports.get("url") {
        if let Expr::Literal(Literal::String(s, _)) = url_expr {
            own_style.url = Some(s.clone());
            own_style.cursor = Some(CursorKind::Pointer);
        }
    }
    if let Some(color_expr) = rewritten_ports
        .get("color")
        .or_else(|| rewritten_ports.get("current_color"))
    {
        if let Expr::Literal(Literal::Color(c, _)) | Expr::Literal(Literal::String(c, _)) =
            color_expr
        {
            own_style.color = Some(c.clone());
        }
    }
    if let Some(bg_expr) = rewritten_ports
        .get("bg")
        .or_else(|| rewritten_ports.get("current_bg"))
    {
        if let Expr::Literal(Literal::Color(c, _)) | Expr::Literal(Literal::String(c, _)) = bg_expr
        {
            own_style.bg_color = Some(c.clone());
        }
    }
    if let Some(ul_expr) = rewritten_ports
        .get("underline")
        .or_else(|| rewritten_ports.get("current_underline"))
    {
        if let Expr::Literal(Literal::Bool(b, _)) = ul_expr {
            own_style.underline = *b;
        }
    }
    if let Some(bs_expr) = rewritten_ports.get("baseline_shift") {
        match bs_expr {
            Expr::Literal(Literal::Distance(d, _, _)) | Expr::Literal(Literal::Float(d, _)) => {
                own_style.baseline_shift = *d;
            }
            Expr::Literal(Literal::Integer(i, _)) => {
                own_style.baseline_shift = *i as f64;
            }
            _ => {}
        }
    }
    if let Some(&font_id) = node_fonts.get(&comp_node_id) {
        if let Some(fnode) = doc.get_node(font_id) {
            if let Some(Expr::Literal(Literal::Distance(d, _, _)))
            | Some(Expr::Literal(Literal::Float(d, _))) = fnode.ports.get("size")
            {
                own_style.font_size = Some(*d);
            }
            if let Some(Expr::Literal(Literal::Integer(i, _))) = fnode.ports.get("weight") {
                own_style.font_weight = Some(*i as f64);
            } else if let Some(Expr::Literal(Literal::Float(f, _))) = fnode.ports.get("weight") {
                own_style.font_weight = Some(*f);
            }
            if let Some(Expr::Literal(Literal::String(s, _))) = fnode.ports.get("family") {
                own_style.font_family = Some(s.clone());
            }
            if let Some(Expr::Literal(Literal::Bool(b, _))) = fnode.ports.get("italic") {
                own_style.italic = *b;
            }
        }
    }

    for sub_span in span_text_spans {
        if sub_span.style.url.is_none() {
            sub_span.style.url = own_style.url.clone();
        }
        if sub_span.style.color.is_none() {
            sub_span.style.color = own_style.color.clone();
        }
        if sub_span.style.bg_color.is_none() {
            sub_span.style.bg_color = own_style.bg_color.clone();
        }
        if !sub_span.style.underline && own_style.underline {
            sub_span.style.underline = true;
        }
        if sub_span.style.cursor.is_none() {
            sub_span.style.cursor = own_style.cursor;
        }
        if sub_span.style.font_size.is_none() {
            sub_span.style.font_size = own_style.font_size;
        }
        if sub_span.style.font_weight.is_none() {
            sub_span.style.font_weight = own_style.font_weight;
        }
        if sub_span.style.font_family.is_none() {
            sub_span.style.font_family = own_style.font_family.clone();
        }
        if !sub_span.style.italic && own_style.italic {
            sub_span.style.italic = true;
        }
        if sub_span.style.baseline_shift == 0.0 && own_style.baseline_shift != 0.0 {
            sub_span.style.baseline_shift = own_style.baseline_shift;
        }
    }

    let self_ident_span = Expr::Ident(Ident::new(comp_node_id.canonical_name(), span));
    let (size_expr, weight_expr, family_expr) = if let Some(&fid) = node_fonts.get(&comp_node_id) {
        let f_ident = Expr::Ident(Ident::new(fid.canonical_name(), span));
        (
            Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(f_ident.clone()),
                member: Ident::new("size", span),
                span,
            }),
            Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(f_ident.clone()),
                member: Ident::new("weight", span),
                span,
            }),
            Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(f_ident),
                member: Ident::new("family", span),
                span,
            }),
        )
    } else {
        (
            Expr::Literal(Literal::Distance(16.0, DistanceUnit::Px, span)),
            Expr::Literal(Literal::Integer(400, span)),
            Expr::Literal(Literal::String("sans-serif".to_string(), span)),
        )
    };
    rewritten_ports
        .entry("x".to_string())
        .or_insert_with(|| Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span)));
    rewritten_ports
        .entry("y".to_string())
        .or_insert_with(|| Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span)));
    let text_str = span_full_text.clone().unwrap_or_default();
    let text_lit = Expr::Literal(Literal::String(text_str, span));
    rewritten_ports
        .entry("width".to_string())
        .or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("text_width", span),
                args: vec![
                    text_lit.clone(),
                    size_expr.clone(),
                    weight_expr.clone(),
                    family_expr.clone(),
                ],
                span,
            })
        });
    let width_member = Expr::MemberAccess(MemberAccessExpr {
        target: Box::new(self_ident_span.clone()),
        member: Ident::new("width", span),
        span,
    });
    rewritten_ports
        .entry("height".to_string())
        .or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("text_height", span),
                args: vec![
                    text_lit.clone(),
                    size_expr.clone(),
                    weight_expr.clone(),
                    family_expr.clone(),
                    width_member.clone(),
                    Expr::Literal(Literal::Enum(
                        "TextEnd".to_string(),
                        "Baseline".to_string(),
                        span,
                    )),
                    Expr::Literal(Literal::Enum(
                        "TextStart".to_string(),
                        "Capital".to_string(),
                        span,
                    )),
                    Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span)),
                ],
                span,
            })
        });
    rewritten_ports
        .entry("baseline".to_string())
        .or_insert_with(|| {
            Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident_span.clone()),
                    member: Ident::new("y", span),
                    span,
                })),
                right: Box::new(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident_span),
                    member: Ident::new("height", span),
                    span,
                })),
                span,
            })
        });
}

/// Finalizes `baseline` and `__baseline_offset` ports for an `\InlineBox` component instance.
pub(super) fn finalize_inline_box_component_ports(
    comp_node_id: NodeId,
    start_node_idx: usize,
    span: Span,
    nodes: &[ExpandedNode],
    declared_state_names: &HashSet<String>,
    rewritten_ports: &mut HashMap<String, Expr>,
) {
    if let Some(baseline_expr) = rewritten_ports.get("baseline").cloned() {
        let offset_expr = extract_inline_box_baseline_offset(
            &baseline_expr,
            comp_node_id,
            start_node_idx,
            nodes,
            rewritten_ports,
            declared_state_names,
            0,
        );
        rewritten_ports.insert("__baseline_offset".to_string(), offset_expr);
    } else {
        let self_ident = Expr::Ident(Ident::new(comp_node_id.canonical_name(), span));
        let self_y = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("y", span),
            span,
        });
        let self_height = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident),
            member: Ident::new("height", span),
            span,
        });
        let default_baseline = Expr::Binary(BinaryExpr {
            op: BinaryOp::Add,
            left: Box::new(self_y),
            right: Box::new(self_height.clone()),
            span,
        });
        rewritten_ports.insert("baseline".to_string(), default_baseline);
        rewritten_ports.insert("__baseline_offset".to_string(), self_height);
    }
}

/// Computes the vertical offset `baseline - y` for an `\InlineBox` component by evaluating its
/// `baseline` port equation with `self.y = 0px` and recursively inlining internal children's
/// vertical port equations (`y`, `top`, `bottom`, `baseline`).
pub(super) fn extract_inline_box_baseline_offset(
    expr: &Expr,
    comp_node_id: NodeId,
    start_node_idx: usize,
    nodes: &[ExpandedNode],
    rewritten_ports: &HashMap<String, Expr>,
    declared_state_names: &HashSet<String>,
    depth: usize,
) -> Expr {
    if depth > 16 {
        return expr.clone();
    }
    match expr {
        Expr::MemberAccess(m) => {
            if let Expr::Ident(id) = m.target.as_ref() {
                if let Some(target_nid) = NodeId::from_canonical_name(id.as_str()) {
                    let member = m.member.as_str();
                    if target_nid == comp_node_id {
                        if member == "y" || member == "top" {
                            return Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, m.span));
                        }
                        if member == "bottom" {
                            return Expr::MemberAccess(MemberAccessExpr {
                                target: m.target.clone(),
                                member: Ident::new("height", m.member.span),
                                span: m.span,
                            });
                        }
                        if member != "width"
                            && member != "height"
                            && !declared_state_names.contains(member)
                        {
                            if let Some(port_expr) = rewritten_ports.get(member) {
                                return extract_inline_box_baseline_offset(
                                    port_expr,
                                    comp_node_id,
                                    start_node_idx,
                                    nodes,
                                    rewritten_ports,
                                    declared_state_names,
                                    depth + 1,
                                );
                            }
                        }
                    } else if target_nid.0 >= start_node_idx && target_nid.0 < nodes.len() {
                        if matches!(member, "y" | "top" | "bottom" | "baseline") {
                            if let Some(port_expr) = nodes[target_nid.0].ports.get(member) {
                                return extract_inline_box_baseline_offset(
                                    port_expr,
                                    comp_node_id,
                                    start_node_idx,
                                    nodes,
                                    rewritten_ports,
                                    declared_state_names,
                                    depth + 1,
                                );
                            }
                        }
                    }
                }
            }
            Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(extract_inline_box_baseline_offset(
                    &m.target,
                    comp_node_id,
                    start_node_idx,
                    nodes,
                    rewritten_ports,
                    declared_state_names,
                    depth + 1,
                )),
                member: m.member.clone(),
                span: m.span,
            })
        }
        Expr::Binary(b) => Expr::Binary(BinaryExpr {
            op: b.op,
            left: Box::new(extract_inline_box_baseline_offset(
                &b.left,
                comp_node_id,
                start_node_idx,
                nodes,
                rewritten_ports,
                declared_state_names,
                depth + 1,
            )),
            right: Box::new(extract_inline_box_baseline_offset(
                &b.right,
                comp_node_id,
                start_node_idx,
                nodes,
                rewritten_ports,
                declared_state_names,
                depth + 1,
            )),
            span: b.span,
        }),
        Expr::Unary(u) => Expr::Unary(UnaryExpr {
            op: u.op,
            operand: Box::new(extract_inline_box_baseline_offset(
                &u.operand,
                comp_node_id,
                start_node_idx,
                nodes,
                rewritten_ports,
                declared_state_names,
                depth + 1,
            )),
            span: u.span,
        }),
        Expr::Paren(inner, span) => Expr::Paren(
            Box::new(extract_inline_box_baseline_offset(
                inner,
                comp_node_id,
                start_node_idx,
                nodes,
                rewritten_ports,
                declared_state_names,
                depth + 1,
            )),
            *span,
        ),
        Expr::Call(c) => Expr::Call(CallExpr {
            callee: c.callee.clone(),
            args: c
                .args
                .iter()
                .map(|a| {
                    extract_inline_box_baseline_offset(
                        a,
                        comp_node_id,
                        start_node_idx,
                        nodes,
                        rewritten_ports,
                        declared_state_names,
                        depth + 1,
                    )
                })
                .collect(),
            span: c.span,
        }),
        Expr::Ternary(cond) => Expr::Ternary(TernaryExpr {
            condition: Box::new(extract_inline_box_baseline_offset(
                &cond.condition,
                comp_node_id,
                start_node_idx,
                nodes,
                rewritten_ports,
                declared_state_names,
                depth + 1,
            )),
            then_expr: Box::new(extract_inline_box_baseline_offset(
                &cond.then_expr,
                comp_node_id,
                start_node_idx,
                nodes,
                rewritten_ports,
                declared_state_names,
                depth + 1,
            )),
            else_expr: Box::new(extract_inline_box_baseline_offset(
                &cond.else_expr,
                comp_node_id,
                start_node_idx,
                nodes,
                rewritten_ports,
                declared_state_names,
                depth + 1,
            )),
            span: cond.span,
        }),
        other => other.clone(),
    }
}

