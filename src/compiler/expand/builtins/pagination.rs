use crate::ast::{
    BinaryExpr, BinaryOp, ComponentDef, DistanceUnit, ElementNode, EnumDef, Expr, Ident,
    LetValue, Literal, MatchExpr, MatchPattern, MemberAccessExpr, PortBinding, Symbol,
};
use crate::compiler::error::CompileError;
use crate::compiler::eval::eval_expr_for_node;
use crate::compiler::expand::builtins::{
    default_builtin_port_type, lookup_builtin, static_type_from_canonical_name, BuiltinChildren,
    BuiltinPrimitive, PrimitiveExpandState, SpecialBuiltinSpec,
};
use crate::compiler::expand::context::{ElementContext, LexicalBinding, ScopeContext};
use crate::compiler::expand::expand_element;
use crate::compiler::expand::rewrite::{
    flush_all_materialized_expr_ports, resolve_forward_refs_in_range, restore_rewrite_tracking,
    rewrite_expr, save_rewrite_tracking, RewriteTrackingSnapshot,
};
use crate::compiler::expand::text_flow::{coalesce_block_content_items, CoalescedBlockItem};
use crate::compiler::expand::typecheck::StaticType;
use crate::compiler::expanded::{
    ExpandedDocument, NodeId, OwnedConsumerChild, PageBreakTarget, PagedFlowRecord, PaginationSpec,
    DepositRecord,
};
use crate::compiler::graph::{build_variable_graph_with_window_and_dpi, VarId, VariableGraph};
use crate::compiler::scope::ScopeId;
use crate::compiler::topo::{sort_graph, TopologicalSchedule};
use crate::compiler::value::Value;
use crate::span::Span;
use std::collections::{HashMap, VecDeque};

pub struct PagedFlowPrimitive;
pub struct FlowBodyPrimitive;
pub struct PaginationPrimitive;
pub struct PageBreakPrimitive;
pub struct PageValuePrimitive;
pub struct PageListPrimitive;
pub struct DepositPrimitive;
pub struct CollectPrimitive;

/// RAII guard that always restores outer rewrite-tracking thread-locals even on error returns.
struct RewriteTrackingGuard(Option<RewriteTrackingSnapshot>);

impl Drop for RewriteTrackingGuard {
    fn drop(&mut self) {
        if let Some(snap) = self.0.take() {
            restore_rewrite_tracking(snap);
        }
    }
}

/// Resolves simple `Paren` / `MemberAccess(Ident(node_id), port)` chains within `doc`.
pub(crate) fn resolve_constant_expr_in_doc<'a>(
    mut expr: &'a Expr,
    doc: &'a ExpandedDocument,
) -> &'a Expr {
    for _ in 0..16 {
        match expr {
            Expr::Paren(inner, _) => expr = inner,
            Expr::MemberAccess(m) => {
                if let Expr::Ident(id) = m.target.as_ref() {
                    if let Some(nid) = id.node_id() {
                        if let Some(node) = doc.get_node(nid) {
                            if let Some(next) = node.ports.get(m.member.as_str()) {
                                expr = next;
                                continue;
                            }
                        }
                    }
                }
                break;
            }
            _ => break,
        }
    }
    expr
}

pub(crate) fn eval_constant_break_target(expr: &Expr, doc: &ExpandedDocument) -> Option<PageBreakTarget> {
    let resolved = resolve_constant_expr_in_doc(expr, doc);
    match resolved {
        Expr::Literal(Literal::Enum(en, variant, _)) if en == "Break" => match variant.as_str() {
            "Auto" => Some(PageBreakTarget::Auto),
            "Page" => Some(PageBreakTarget::Page),
            "Recto" => Some(PageBreakTarget::Recto),
            "Verso" => Some(PageBreakTarget::Verso),
            _ => None,
        },
        Expr::MemberAccess(m) => {
            if let Expr::Ident(id) = m.target.as_ref() {
                if id.as_str() == "Break" {
                    return match m.member.as_str() {
                        "Auto" => Some(PageBreakTarget::Auto),
                        "Page" => Some(PageBreakTarget::Page),
                        "Recto" => Some(PageBreakTarget::Recto),
                        "Verso" => Some(PageBreakTarget::Verso),
                        _ => None,
                    };
                }
            }
            None
        }
        _ => None,
    }
}

fn eval_constant_bool(expr: &Expr, doc: &ExpandedDocument) -> Option<bool> {
    match resolve_constant_expr_in_doc(expr, doc) {
        Expr::Literal(Literal::Bool(b, _)) => Some(*b),
        _ => None,
    }
}

/// Extracts `PaginationSpec` from an expanded `\Pagination` node in `doc`.
pub(crate) fn resolve_pagination_spec_from_node(
    doc: &ExpandedDocument,
    pag_node_id: NodeId,
    default_breakable: bool,
) -> PaginationSpec {
    let mut spec = PaginationSpec {
        breakable: default_breakable,
        keep_with_next: false,
        break_before: PageBreakTarget::Auto,
    };
    let Some(node) = doc.get_node(pag_node_id) else {
        return spec;
    };
    if let Some(e) = node.ports.get("breakable") {
        if let Some(b) = eval_constant_bool(e, doc) {
            spec.breakable = b;
        }
    }
    if let Some(e) = node.ports.get("keep_with_next") {
        if let Some(b) = eval_constant_bool(e, doc) {
            spec.keep_with_next = b;
        }
    }
    if let Some(e) = node.ports.get("break_before") {
        if let Some(bt) = eval_constant_break_target(e, doc) {
            spec.break_before = bt;
        }
    }
    spec
}

/// Resolves an expression pointing to a `\Pagination` node ID, following simple variable/member chains.
pub(crate) fn resolve_pagination_node_id(expr: &Expr, doc: &ExpandedDocument) -> Option<NodeId> {
    match resolve_constant_expr_in_doc(expr, doc) {
        Expr::Ident(id) => {
            let nid = id.node_id()?;
            if doc
                .get_node(nid)
                .is_some_and(|n| n.name == "Pagination")
            {
                Some(nid)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Merges component body `\let pagination = ...` defaults with explicitly authored fields from a call-site `pagination:` port.
pub(crate) fn merge_pagination_specs(
    body_pag_node_id: Option<NodeId>,
    caller_pag_expr: Option<&Expr>,
    doc: &ExpandedDocument,
    default_breakable: bool,
) -> PaginationSpec {
    let mut spec = if let Some(body_id) = body_pag_node_id {
        resolve_pagination_spec_from_node(doc, body_id, default_breakable)
    } else {
        PaginationSpec {
            breakable: default_breakable,
            keep_with_next: false,
            break_before: PageBreakTarget::Auto,
        }
    };

    if let Some(caller_expr) = caller_pag_expr {
        if let Some(caller_id) = resolve_pagination_node_id(caller_expr, doc) {
            if let Some(caller_node) = doc.get_node(caller_id) {
                if caller_node.authored_ports.contains_key("breakable") {
                    if let Some(e) = caller_node.ports.get("breakable") {
                        if let Some(b) = eval_constant_bool(e, doc) {
                            spec.breakable = b;
                        }
                    }
                }
                if caller_node.authored_ports.contains_key("keep_with_next") {
                    if let Some(e) = caller_node.ports.get("keep_with_next") {
                        if let Some(b) = eval_constant_bool(e, doc) {
                            spec.keep_with_next = b;
                        }
                    }
                }
                if caller_node.authored_ports.contains_key("break_before") {
                    if let Some(e) = caller_node.ports.get("break_before") {
                        if let Some(bt) = eval_constant_break_target(e, doc) {
                            spec.break_before = bt;
                        }
                    }
                }
            }
        }
    }

    spec
}

/// Checks if a primitive or component element accepts `port_name`.
pub(crate) fn element_accepts_port(
    elem: &ElementNode,
    port_name: &str,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> bool {
    if let Some(prim) = lookup_builtin(elem.name.as_str(), false) {
        return prim.accepts_port(port_name);
    }
    if let Some(overloads) = registry.get(elem.name.as_str()) {
        return overloads
            .iter()
            .any(|def| def.params.iter().any(|p| p.name.as_str() == port_name))
            || matches!(port_name, "clip" | "pagination");
    }
    false
}

/// Follows `Ident` / `MemberAccess` / `Paren` chains inside `doc` to locate a `PageValue` or `PageList` bucket node.
pub(crate) fn resolve_bucket_node_id(expr: &Expr, doc: &ExpandedDocument) -> Option<NodeId> {
    let mut cur = expr;
    for _ in 0..32 {
        match cur {
            Expr::Paren(inner, _) => cur = inner,
            Expr::Ident(id) => {
                let nid = id.node_id()?;
                if doc
                    .get_node(nid)
                    .is_some_and(|n| matches!(n.name.as_str(), "PageValue" | "PageList"))
                {
                    return Some(nid);
                }
                return None;
            }
            Expr::MemberAccess(m) => {
                if let Expr::Ident(id) = m.target.as_ref() {
                    if let Some(nid) = id.node_id() {
                        if let Some(node) = doc.get_node(nid) {
                            if let Some(next) = node.ports.get(m.member.as_str()) {
                                cur = next;
                                continue;
                            }
                        }
                    }
                }
                return None;
            }
            _ => return None,
        }
    }
    None
}

/// Evaluates only the transitive upstream subgraph of `target` in `graph`.
pub(crate) fn eval_var_slice(
    graph: &VariableGraph,
    schedule: &TopologicalSchedule,
    target: VarId,
) -> Result<Value, CompileError> {
    let Some(target_idx) = graph.lookup_var_index_sym(target.node, target.port) else {
        return Err(CompileError::Custom {
            message: format!("Missing variable '{}.{}'", target.node.canonical_str(), target.port),
            span: Span::default(),
        });
    };

    let num_vars = graph.var_nodes.len();
    let mut needed = vec![false; num_vars];
    let mut queue = VecDeque::new();
    needed[target_idx.0 as usize] = true;
    queue.push_back(target_idx);
    while let Some(cur) = queue.pop_front() {
        for &up in &graph.upstream_idx[cur.0 as usize] {
            if !needed[up.0 as usize] {
                needed[up.0 as usize] = true;
                queue.push_back(up);
            }
        }
    }

    let mut env: HashMap<VarId, Value> = HashMap::new();
    for &var_idx in &schedule.order_indices {
        let idx = var_idx.0 as usize;
        if !needed[idx] {
            continue;
        }
        let vnode = &graph.var_nodes[idx];
        let val = eval_expr_for_node(&vnode.equation, vnode.id.node, &env)?;
        env.insert(vnode.id, val);
    }

    env.remove(&target).ok_or_else(|| CompileError::Custom {
        message: format!("Failed to evaluate '{}.{}'", target.node.canonical_str(), target.port),
        span: Span::default(),
    })
}

/// Collects all transitive upstream `VarId`s reachable from `target` in `graph`.
fn collect_transitive_upstream_vars(graph: &VariableGraph, target: VarId) -> Vec<VarId> {
    let Some(start) = graph.lookup_var_index_sym(target.node, target.port) else {
        return Vec::new();
    };
    let mut visited = vec![false; graph.var_nodes.len()];
    let mut queue = VecDeque::new();
    visited[start.0 as usize] = true;
    queue.push_back(start);
    let mut out = Vec::new();
    while let Some(cur) = queue.pop_front() {
        out.push(graph.var_nodes[cur.0 as usize].id);
        for &up in &graph.upstream_idx[cur.0 as usize] {
            if !visited[up.0 as usize] {
                visited[up.0 as usize] = true;
                queue.push_back(up);
            }
        }
    }
    out
}

/// Authoritative ambient property schema for `page.*` inside a `\PagedFlow` page template.
pub const PAGE_SPEC: SpecialBuiltinSpec = SpecialBuiltinSpec {
    name: "Page",
    accepted_ports: &[],
    required_ports: &[],
    output_ports: &[
        ("number", "Integer"),
        ("count", "Integer"),
        ("is_first", "Boolean"),
        ("is_last", "Boolean"),
        ("is_odd", "Boolean"),
        ("is_even", "Boolean"),
        ("is_recto", "Boolean"),
        ("is_verso", "Boolean"),
        ("prev", "Node"),
        ("width", "Distance"),
        ("height", "Distance"),
        ("dpi", "Float"),
        ("left", "Distance"),
        ("top", "Distance"),
        ("right", "Distance"),
        ("bottom", "Distance"),
    ],
    children: BuiltinChildren::None,
};

/// Returns the static type of an ambient `page.*` property, backed strictly by `PAGE_SPEC`.
pub(in crate::compiler::expand) fn page_port_type(port_name: &str) -> Option<StaticType> {
    if port_name == "prev" {
        return Some(StaticType::Node("PageTemplate".to_string()));
    }
    PAGE_SPEC
        .output_port_type(port_name)
        .map(static_type_from_canonical_name)
}

/// Runs `populate_page_ports` on a fresh document and returns the exact set of populated `page.*` keys.
pub fn populated_page_port_names() -> Vec<String> {
    let mut doc = ExpandedDocument::new();
    populate_page_ports(&mut doc, 1, 1, true, true, None, None, Span::default());
    let mut names: Vec<String> = doc.page_ports.keys().cloned().collect();
    names.sort();
    names
}

/// Populates standard `page.*` aliases on `doc.page_ports` for single-page probe/page instantiation.
pub(crate) fn populate_page_ports(
    doc: &mut ExpandedDocument,
    page_number: i64,
    page_count: i64,
    is_first: bool,
    is_last: bool,
    prev_page_root: Option<NodeId>,
    template_root_id: Option<NodeId>,
    span: Span,
) {
    let win_ident = Expr::Ident(Ident::from_node(NodeId::WINDOW, span));
    let page_ident = Expr::Ident(Ident::from_node(NodeId::PAGE, span));

    doc.page_ports.insert(
        "number".to_string(),
        Expr::Literal(Literal::Integer(page_number, span)),
    );
    doc.page_ports.insert(
        "count".to_string(),
        Expr::Literal(Literal::Integer(page_count, span)),
    );
    doc.page_ports.insert(
        "is_first".to_string(),
        Expr::Literal(Literal::Bool(is_first, span)),
    );
    doc.page_ports.insert(
        "is_last".to_string(),
        Expr::Literal(Literal::Bool(is_last, span)),
    );
    let is_odd = page_number.rem_euclid(2) == 1;
    doc.page_ports.insert(
        "is_odd".to_string(),
        Expr::Literal(Literal::Bool(is_odd, span)),
    );
    doc.page_ports.insert(
        "is_recto".to_string(),
        Expr::Literal(Literal::Bool(is_odd, span)),
    );
    doc.page_ports.insert(
        "is_even".to_string(),
        Expr::Literal(Literal::Bool(!is_odd, span)),
    );
    doc.page_ports.insert(
        "is_verso".to_string(),
        Expr::Literal(Literal::Bool(!is_odd, span)),
    );
    doc.page_ports.insert(
        "prev".to_string(),
        match prev_page_root {
            Some(prev_id) => Expr::Ident(Ident::from_node(prev_id, span)),
            None => Expr::Ident(Ident::from_symbol(crate::symbol::Symbol::UNSET, span)),
        },
    );
    doc.page_ports.insert(
        "left".to_string(),
        Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span)),
    );
    doc.page_ports.insert(
        "top".to_string(),
        Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span)),
    );
    doc.page_ports.insert(
        "dpi".to_string(),
        Expr::MemberAccess(MemberAccessExpr {
            target: std::sync::Arc::new(win_ident.clone()),
            member: Ident::new("dpi", span),
            span,
        }),
    );

    let has_root_w = template_root_id
        .and_then(|rid| doc.get_node(rid))
        .is_some_and(|n| n.ports.contains_key("width"));
    let has_root_h = template_root_id
        .and_then(|rid| doc.get_node(rid))
        .is_some_and(|n| n.ports.contains_key("height"));

    let width_expr = if has_root_w {
        Expr::MemberAccess(MemberAccessExpr {
            target: std::sync::Arc::new(Expr::Ident(Ident::from_node(
                template_root_id.unwrap(),
                span,
            ))),
            member: Ident::new("width", span),
            span,
        })
    } else {
        Expr::MemberAccess(MemberAccessExpr {
            target: std::sync::Arc::new(win_ident.clone()),
            member: Ident::new("width", span),
            span,
        })
    };
    let height_expr = if has_root_h {
        Expr::MemberAccess(MemberAccessExpr {
            target: std::sync::Arc::new(Expr::Ident(Ident::from_node(
                template_root_id.unwrap(),
                span,
            ))),
            member: Ident::new("height", span),
            span,
        })
    } else {
        Expr::MemberAccess(MemberAccessExpr {
            target: std::sync::Arc::new(win_ident),
            member: Ident::new("height", span),
            span,
        })
    };

    doc.page_ports.insert("width".to_string(), width_expr);
    doc.page_ports.insert("height".to_string(), height_expr);
    doc.page_ports.insert(
        "right".to_string(),
        Expr::Binary(BinaryExpr {
            op: BinaryOp::Add,
            left: std::sync::Arc::new(Expr::MemberAccess(MemberAccessExpr {
                target: std::sync::Arc::new(page_ident.clone()),
                member: Ident::new("left", span),
                span,
            })),
            right: std::sync::Arc::new(Expr::MemberAccess(MemberAccessExpr {
                target: std::sync::Arc::new(page_ident.clone()),
                member: Ident::new("width", span),
                span,
            })),
            span,
        }),
    );
    doc.page_ports.insert(
        "bottom".to_string(),
        Expr::Binary(BinaryExpr {
            op: BinaryOp::Add,
            left: std::sync::Arc::new(Expr::MemberAccess(MemberAccessExpr {
                target: std::sync::Arc::new(page_ident.clone()),
                member: Ident::new("top", span),
                span,
            })),
            right: std::sync::Arc::new(Expr::MemberAccess(MemberAccessExpr {
                target: std::sync::Arc::new(page_ident),
                member: Ident::new("height", span),
                span,
            })),
            span,
        }),
    );
}

impl BuiltinPrimitive for PaginationPrimitive {
    fn name(&self) -> &'static str {
        "Pagination"
    }

    fn is_non_spatial(&self) -> bool {
        true
    }

    fn accepted_ports(&self) -> &'static [&'static str] {
        &["breakable", "keep_with_next", "break_before"]
    }

    fn required_port_sets(&self) -> &'static [&'static [&'static str]] {
        &[&[]]
    }

    fn children_cardinality(&self) -> BuiltinChildren {
        BuiltinChildren::None
    }

    fn register_types(
        &self,
        enums: &mut HashMap<String, EnumDef>,
        _registry: &mut HashMap<String, Vec<ComponentDef>>,
        span: Span,
    ) {
        enums.entry("Break".to_string()).or_insert_with(|| EnumDef {
            name: Ident::new("Break", span),
            variants: vec![
                Ident::new("Auto", span),
                Ident::new("Page", span),
                Ident::new("Recto", span),
                Ident::new("Verso", span),
            ],
            span,
        });
    }

    fn port_type(&self, port_name: &str) -> Option<StaticType> {
        match port_name {
            "breakable" | "keep_with_next" => Some(StaticType::Boolean),
            "break_before" => Some(StaticType::Enum("Break".to_string())),
            _ => default_builtin_port_type(port_name),
        }
    }

    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        let span = state.elem.span;
        state
            .ports
            .entry("breakable".to_string())
            .or_insert_with(|| Expr::Literal(Literal::Bool(false, span)));
        state
            .ports
            .entry("keep_with_next".to_string())
            .or_insert_with(|| Expr::Literal(Literal::Bool(false, span)));
        state.ports.entry("break_before".to_string()).or_insert_with(|| {
            Expr::Literal(Literal::Enum(
                "Break".to_string(),
                "Auto".to_string(),
                span,
            ))
        });
        Ok(())
    }

    fn overload_params(&self, _state: &PrimitiveExpandState<'_, '_>) -> Vec<String> {
        Vec::new()
    }
}

impl BuiltinPrimitive for PageBreakPrimitive {
    fn name(&self) -> &'static str {
        "PageBreak"
    }

    fn is_non_spatial(&self) -> bool {
        true
    }

    fn accepted_ports(&self) -> &'static [&'static str] {
        &["to"]
    }

    fn required_port_sets(&self) -> &'static [&'static [&'static str]] {
        &[&[]]
    }

    fn children_cardinality(&self) -> BuiltinChildren {
        BuiltinChildren::None
    }

    fn port_type(&self, port_name: &str) -> Option<StaticType> {
        match port_name {
            "to" => Some(StaticType::Enum("Break".to_string())),
            _ => default_builtin_port_type(port_name),
        }
    }

    fn pre_expand_children(
        &self,
        _node_id: NodeId,
        elem: &ElementNode,
        ctx: &ElementContext<'_>,
        _doc: &mut ExpandedDocument,
        _registry: &HashMap<String, Vec<ComponentDef>>,
        _node_fonts: &HashMap<NodeId, NodeId>,
    ) -> Result<ScopeId, CompileError> {
        if let Some(comp_name) = ctx.in_unbreakable_flow_container {
            return Err(CompileError::PageBreakInUnbreakableContainer {
                component: comp_name.to_string(),
                span: elem.span,
            });
        }
        Ok(ctx.active_scope_id)
    }

    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        let span = state.elem.span;
        state.ports.entry("to".to_string()).or_insert_with(|| {
            Expr::Literal(Literal::Enum(
                "Break".to_string(),
                "Page".to_string(),
                span,
            ))
        });
        Ok(())
    }

    fn overload_params(&self, _state: &PrimitiveExpandState<'_, '_>) -> Vec<String> {
        vec!["to".to_string()]
    }
}

impl BuiltinPrimitive for FlowBodyPrimitive {
    fn name(&self) -> &'static str {
        "FlowBody"
    }

    fn accepted_ports(&self) -> &'static [&'static str] {
        &["x", "y", "width", "height", "z", "clip", "id"]
    }

    fn required_port_sets(&self) -> &'static [&'static [&'static str]] {
        &[&["width", "height"]]
    }

    fn children_cardinality(&self) -> BuiltinChildren {
        BuiltinChildren::None
    }

    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        let span = state.elem.span;
        if let Some(pid) = state.ctx.parent_id {
            let p_ident = Expr::Ident(Ident::from_node(pid, span));
            state.ports.entry("x".to_string()).or_insert_with(|| {
                Expr::MemberAccess(MemberAccessExpr {
                    target: std::sync::Arc::new(p_ident.clone()),
                    member: Ident::new("left", span),
                    span,
                })
            });
            state.ports.entry("y".to_string()).or_insert_with(|| {
                Expr::MemberAccess(MemberAccessExpr {
                    target: std::sync::Arc::new(p_ident),
                    member: Ident::new("top", span),
                    span,
                })
            });
        }
        Ok(())
    }
}

impl BuiltinPrimitive for PagedFlowPrimitive {
    fn name(&self) -> &'static str {
        "PagedFlow"
    }

    fn output_aliases(&self) -> &'static [&'static str] {
        &["end_page", "page_count"]
    }

    fn required_port_sets(&self) -> &'static [&'static [&'static str]] {
        &[&["page"]]
    }

    fn children_cardinality(&self) -> BuiltinChildren {
        BuiltinChildren::Multiple
    }

    fn accepted_ports(&self) -> &'static [&'static str] {
        &[
            "page",
            "orphans",
            "widows",
            "gap",
            "start_page",
            "x",
            "y",
            "width",
            "height",
            "z",
            "clip",
            "pagination",
            "id",
        ]
    }

    fn accepts_event_handlers(&self) -> bool {
        true
    }

    fn coalesces_block_content(&self) -> bool {
        true
    }

    fn sets_paged_flow(&self) -> bool {
        true
    }

    fn port_type(&self, port_name: &str) -> Option<StaticType> {
        match port_name {
            "page" => Some(StaticType::Node("Node".to_string())),
            "orphans" | "widows" | "start_page" | "end_page" | "page_count" => {
                Some(StaticType::Integer)
            }
            "gap" | "__flow_body_width" => Some(StaticType::Distance),
            "pagination" => Some(StaticType::Node("Pagination".to_string())),
            _ => default_builtin_port_type(port_name),
        }
    }

    fn scope_ports(&self) -> Vec<String> {
        vec![
            "x".to_string(),
            "y".to_string(),
            "width".to_string(),
            "height".to_string(),
            "z".to_string(),
            "left".to_string(),
            "top".to_string(),
            "right".to_string(),
            "bottom".to_string(),
            "clip".to_string(),
            "__flow_body_width".to_string(),
            "end_page".to_string(),
            "page_count".to_string(),
        ]
    }

    fn pre_expand_children(
        &self,
        node_id: NodeId,
        elem: &ElementNode,
        ctx: &ElementContext<'_>,
        doc: &mut ExpandedDocument,
        registry: &HashMap<String, Vec<ComponentDef>>,
        node_fonts: &HashMap<NodeId, NodeId>,
    ) -> Result<ScopeId, CompileError> {
        if ctx.in_paged_flow || ctx.in_page_template {
            return Err(CompileError::NestedPagedFlow { span: elem.span });
        }

        let page_port = elem
            .ports
            .iter()
            .find(|p| p.name.as_str() == "page")
            .ok_or_else(|| CompileError::MissingPort {
                node: "PagedFlow".to_string(),
                port: "page".to_string(),
                span: elem.span,
            })?;

        let mut raw_page_expr = &page_port.expr;
        while let Expr::Paren(inner, _) = raw_page_expr {
            raw_page_expr = inner;
        }
        let template_elem = match raw_page_expr {
            Expr::Node(e) => e.as_ref().clone(),
            other => {
                return Err(CompileError::TypeMismatch {
                    expected: "Page template component (e.g. \\BookPage())".to_string(),
                    actual: format!("{other}"),
                    span: page_port.expr.span(),
                });
            }
        };

        let page_template_elem = OwnedConsumerChild {
            elem: template_elem.clone(),
            var_name: None,
            raw_text: None,
            caller_lexical_scope: ctx.lexical_scope.borrow().clone(),
            caller_env_scope: ctx.env_scope.clone(),
            caller_enclosing_component_id: ctx.enclosing_component_id,
            caller_enclosing_num_children: ctx.enclosing_num_children,
            caller_shadowed_ports: ctx.shadowed_ports.cloned().unwrap_or_default(),
            outer_consumer_children: Vec::new(),
        };

        // Expand page template into an isolated scratch ExpandedDocument with guarded tracking.
        let (
            probe_doc,
            probe_root_id,
            flow_body_id,
            flow_body_width,
            probe_page_width,
            probe_page_height,
            width_depends_on_window,
            private_ports,
        ) = {
            let _guard = RewriteTrackingGuard(Some(save_rewrite_tracking()));
            let base_node_len = doc.nodes.len();
            let mut probe_doc = ExpandedDocument::new();
            probe_doc.nodes = doc.nodes.clone();
            probe_doc.window_ports = doc.window_ports.clone();
            probe_doc.window_port_types = doc.window_port_types.clone();
            probe_doc.window_state_vars = doc.window_state_vars.clone();
            probe_doc.local_bindings = doc.local_bindings.clone();
            probe_doc.scope_tree = doc.scope_tree.clone();
            probe_doc.default_dpi = doc.default_dpi;
            probe_doc.page_template_component = Some(template_elem.name.as_str().to_string());

            if let (Some(enc_id), Some(enc_ports)) =
                (ctx.enclosing_component_id, ctx.enclosing_comp_ports)
            {
                if let Some(enc_node) = probe_doc.get_node_mut(enc_id) {
                    let enc_parent = enc_node.parent.unwrap_or(NodeId::WINDOW);
                    for (k, v) in enc_ports {
                        let mut resolved = v.clone();
                        resolve_probe_enclosing_port_refs(
                            &mut resolved,
                            enc_id,
                            enc_parent,
                            enc_ports,
                        );
                        enc_node.ports.entry(k.clone()).or_insert(resolved);
                    }
                }
            }

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
            populate_page_ports(
                &mut probe_doc,
                1,
                1,
                true,
                true,
                None,
                None,
                template_elem.span,
            );

            let mut probe_node_fonts = node_fonts.clone();
            let probe_ctx = ElementContext {
                parent_id: Some(NodeId::WINDOW),
                parent_name: Some("window"),
                prev_sibling_id: None,
                has_next_sibling: false,
                parent_ports: &window_scope_ports,
                lexical_scope: ctx.lexical_scope,
                env_scope: ctx.env_scope,
                enclosing_component_id: ctx.enclosing_component_id,
                enclosing_num_children: ctx.enclosing_num_children,
                enclosing_instantiated_children: None,
                shadowed_ports: ctx.shadowed_ports,
                is_let: false,
                ambient_authored_ports: None,
                enums: ctx.enums,
                active_scope_id: ScopeId::ROOT,
                consumer_children: None,
                in_paged_flow: false,
                in_page_template: true,
                in_unbreakable_flow_container: None,
                enclosing_comp_ports: ctx.enclosing_comp_ports,
            };

            let probe_root_id = expand_element(
                &template_elem,
                &probe_ctx,
                registry,
                &mut probe_doc,
                &mut probe_node_fonts,
            )?;

            let start_lb = doc.local_bindings.len();
            {
                let lex_borrow = ctx.lexical_scope.borrow();
                resolve_forward_refs_in_range(&mut probe_doc, base_node_len, start_lb, &lex_borrow);
            }
            flush_all_materialized_expr_ports(&mut probe_doc);

            populate_page_ports(
                &mut probe_doc,
                1,
                1,
                true,
                true,
                None,
                Some(probe_root_id),
                template_elem.span,
            );

            let flow_bodies: Vec<NodeId> = probe_doc
                .nodes
                .iter()
                .skip(base_node_len)
                .filter(|n| n.name == "FlowBody")
                .map(|n| n.id)
                .collect();
            if flow_bodies.len() != 1 {
                return Err(CompileError::InvalidFlowBodyCount {
                    found: flow_bodies.len(),
                    span: elem.span,
                });
            }
            let flow_body_id = flow_bodies[0];
            let flow_body_span = probe_doc.nodes[flow_body_id.0].span;

            let probe_graph = build_variable_graph_with_window_and_dpi(
                &probe_doc,
                800.0,
                600.0,
                probe_doc.default_dpi,
            )?;
            let probe_schedule = sort_graph(&probe_graph)?;

            let width_deps =
                collect_transitive_upstream_vars(&probe_graph, VarId::new(flow_body_id, "width"));
            let mut width_depends_on_window = false;
            for dep in &width_deps {
                let is_illegal_page_port = dep.node.is_page()
                    && !matches!(
                        dep.port.as_str(),
                        "dpi" | "width" | "height" | "left" | "top" | "right" | "bottom"
                    );
                let is_bucket = probe_doc
                    .get_node(dep.node)
                    .is_some_and(|n| matches!(n.name.as_str(), "PageValue" | "PageList"));
                if is_illegal_page_port || is_bucket {
                    return Err(CompileError::FlowBodyWidthDependsOnPage {
                        span: flow_body_span,
                    });
                }
                if dep.node.is_window() && dep.port.as_str() != "dpi" {
                    width_depends_on_window = true;
                }
            }

            let height_deps =
                collect_transitive_upstream_vars(&probe_graph, VarId::new(flow_body_id, "height"));
            for dep in &height_deps {
                let is_total_page_dep = (dep.node.is_page()
                    && matches!(dep.port.as_str(), "count" | "is_last"))
                    || (dep.node == node_id
                        && matches!(dep.port.as_str(), "page_count" | "end_page"));
                if is_total_page_dep {
                    return Err(CompileError::FlowBodyHeightDependsOnTotalPageCount {
                        span: flow_body_span,
                    });
                }
            }

            let flow_body_width =
                match eval_var_slice(&probe_graph, &probe_schedule, VarId::new(flow_body_id, "width"))? {
                    Value::Distance(d) => d,
                    other => {
                        return Err(CompileError::TypeMismatch {
                            expected: "Distance".to_string(),
                            actual: other.type_name().to_string(),
                            span: flow_body_span,
                        });
                    }
                };

            let probe_page_width =
                match eval_var_slice(&probe_graph, &probe_schedule, VarId::new(NodeId::PAGE, "width")) {
                    Ok(Value::Distance(d)) => d,
                    _ => flow_body_width,
                };
            let probe_page_height =
                match eval_var_slice(&probe_graph, &probe_schedule, VarId::new(NodeId::PAGE, "height")) {
                    Ok(Value::Distance(d)) => d,
                    _ => 792.0,
                };

            let mut private_ports = (*probe_doc.nodes[probe_root_id.0].private_ports).clone();
            for state_k in probe_doc.nodes[probe_root_id.0].state_vars.keys() {
                private_ports.insert(state_k.clone());
            }
            for &cid in &probe_doc.nodes[probe_root_id.0].children {
                if let Some(child) = probe_doc.get_node(cid) {
                    if let Some(ref vn) = child.var_name {
                        if !probe_doc.nodes[probe_root_id.0].ports.contains_key(vn) {
                            private_ports.insert(vn.clone());
                        }
                    }
                }
            }
            probe_doc.page_template_private_ports = private_ports.clone();
            (
                probe_doc,
                probe_root_id,
                flow_body_id,
                flow_body_width,
                probe_page_width,
                probe_page_height,
                width_depends_on_window,
                private_ports,
            )
        };

        let _ = probe_root_id;
        doc.page_template_component = Some(template_elem.name.as_str().to_string());
        doc.page_template_private_ports = private_ports;
        populate_page_ports(doc, 1, 1, true, true, None, None, template_elem.span);
        doc.page_ports.insert(
            "width".to_string(),
            Expr::Literal(Literal::Distance(
                probe_page_width,
                DistanceUnit::Px,
                template_elem.span,
            )),
        );
        doc.page_ports.insert(
            "height".to_string(),
            Expr::Literal(Literal::Distance(
                probe_page_height,
                DistanceUnit::Px,
                template_elem.span,
            )),
        );

        // Seed `__flow_body_width`, default `width`, and default `height` onto the PagedFlow node
        // so children expanding in `expand_content_slot` and Phase 1 shaping have valid initial geometry.
        if let Some(flow_node) = doc.get_node_mut(node_id) {
            flow_node.ports.insert(
                "__flow_body_width".to_string(),
                Expr::Literal(Literal::Distance(
                    flow_body_width,
                    DistanceUnit::Px,
                    elem.span,
                )),
            );
            flow_node.ports.insert(
                "width".to_string(),
                Expr::Literal(Literal::Distance(
                    probe_page_width,
                    DistanceUnit::Px,
                    elem.span,
                )),
            );
            flow_node.ports.insert(
                "height".to_string(),
                Expr::Literal(Literal::Distance(
                    probe_page_height,
                    DistanceUnit::Px,
                    elem.span,
                )),
            );
        }

        // Also propagate probe_base_node_len cleanly
        let probe_base_node_len = probe_doc.nodes.len().min(doc.nodes.len());
        doc.paged_flows.push(PagedFlowRecord {
            node_id,
            page_template_elem,
            probe_doc,
            probe_base_node_len,
            flow_body_node_id: flow_body_id,
            width_depends_on_window,
            span: elem.span,
        });

        Ok(ctx.active_scope_id)
    }

    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        let span = state.elem.span;
        // Drop raw AST `Expr::Node` from `page` port so graph/typecheck never scan unrewritten template ports;
        // the captured AST template lives in `PagedFlowRecord.page_template_elem`.
        state.ports.remove("page");

        state
            .ports
            .entry("orphans".to_string())
            .or_insert_with(|| Expr::Literal(Literal::Integer(2, span)));
        state
            .ports
            .entry("widows".to_string())
            .or_insert_with(|| Expr::Literal(Literal::Integer(2, span)));
        state
            .ports
            .entry("gap".to_string())
            .or_insert_with(|| Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span)));
        state
            .ports
            .entry("start_page".to_string())
            .or_insert_with(|| Expr::Literal(Literal::Integer(1, span)));
        state
            .ports
            .entry("end_page".to_string())
            .or_insert_with(|| Expr::Literal(Literal::Integer(1, span)));
        state
            .ports
            .entry("page_count".to_string())
            .or_insert_with(|| Expr::Literal(Literal::Integer(1, span)));

        if let Some(existing_node) = state.doc.get_node(state.node_id) {
            for seeded in ["__flow_body_width", "width", "height"] {
                if !state.ports.contains_key(seeded) {
                    if let Some(val) = existing_node.ports.get(seeded) {
                        state.ports.insert(seeded.to_string(), val.clone());
                    }
                }
            }
        }
        state.ports.entry("width".to_string()).or_insert_with(|| {
            Expr::Literal(Literal::Distance(612.0, DistanceUnit::Px, span))
        });
        state.ports.entry("height".to_string()).or_insert_with(|| {
            Expr::Literal(Literal::Distance(792.0, DistanceUnit::Px, span))
        });

        if let Some(pid) = state.ctx.parent_id {
            let p_ident = Expr::Ident(Ident::from_node(pid, span));
            state.ports.entry("x".to_string()).or_insert_with(|| {
                Expr::MemberAccess(MemberAccessExpr {
                    target: std::sync::Arc::new(p_ident.clone()),
                    member: Ident::new("left", span),
                    span,
                })
            });
            state.ports.entry("y".to_string()).or_insert_with(|| {
                Expr::MemberAccess(MemberAccessExpr {
                    target: std::sync::Arc::new(p_ident),
                    member: Ident::new("top", span),
                    span,
                })
            });
        } else {
            state
                .ports
                .entry("x".to_string())
                .or_insert_with(|| Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span)));
            state
                .ports
                .entry("y".to_string())
                .or_insert_with(|| Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span)));
        }

        Ok(())
    }
}

fn resolve_probe_enclosing_port_refs(
    expr: &mut Expr,
    enc_id: NodeId,
    enc_parent: NodeId,
    enc_ports: &HashMap<String, Expr>,
) {
    use std::sync::Arc;
    match expr {
        Expr::Ident(id) => {
            if id.name == Symbol::PARENT {
                *id = Ident::from_node(enc_parent, id.span);
            } else if id.name == Symbol::WINDOW {
                *id = Ident::from_node(NodeId::WINDOW, id.span);
            } else if id.name == Symbol::SELF
                || id.node_id().is_some_and(|nid| nid.is_template_self())
            {
                *id = Ident::from_node(enc_id, id.span);
            } else if id.node_id().is_none() && enc_ports.contains_key(id.as_str()) {
                let sp = id.span;
                let mem = *id;
                *expr = Expr::MemberAccess(MemberAccessExpr {
                    target: Arc::new(Expr::Ident(Ident::from_node(enc_id, sp))),
                    member: mem,
                    span: sp,
                });
            }
        }
        Expr::MemberAccess(m) => {
            resolve_probe_enclosing_port_refs(
                Arc::make_mut(&mut m.target),
                enc_id,
                enc_parent,
                enc_ports,
            );
        }
        Expr::Index(idx) => {
            resolve_probe_enclosing_port_refs(
                Arc::make_mut(&mut idx.target),
                enc_id,
                enc_parent,
                enc_ports,
            );
            resolve_probe_enclosing_port_refs(
                Arc::make_mut(&mut idx.index),
                enc_id,
                enc_parent,
                enc_ports,
            );
        }
        Expr::Binary(b) => {
            resolve_probe_enclosing_port_refs(
                Arc::make_mut(&mut b.left),
                enc_id,
                enc_parent,
                enc_ports,
            );
            resolve_probe_enclosing_port_refs(
                Arc::make_mut(&mut b.right),
                enc_id,
                enc_parent,
                enc_ports,
            );
        }
        Expr::Unary(u) => {
            resolve_probe_enclosing_port_refs(
                Arc::make_mut(&mut u.operand),
                enc_id,
                enc_parent,
                enc_ports,
            );
        }
        Expr::Ternary(t) => {
            resolve_probe_enclosing_port_refs(
                Arc::make_mut(&mut t.condition),
                enc_id,
                enc_parent,
                enc_ports,
            );
            resolve_probe_enclosing_port_refs(
                Arc::make_mut(&mut t.then_expr),
                enc_id,
                enc_parent,
                enc_ports,
            );
            resolve_probe_enclosing_port_refs(
                Arc::make_mut(&mut t.else_expr),
                enc_id,
                enc_parent,
                enc_ports,
            );
        }
        Expr::Paren(p, _) => {
            resolve_probe_enclosing_port_refs(Arc::make_mut(p), enc_id, enc_parent, enc_ports);
        }
        Expr::Call(c) => {
            for arg in &mut c.args {
                resolve_probe_enclosing_port_refs(arg, enc_id, enc_parent, enc_ports);
            }
        }
        Expr::MethodCall(mc) => {
            resolve_probe_enclosing_port_refs(
                Arc::make_mut(&mut mc.target),
                enc_id,
                enc_parent,
                enc_ports,
            );
            for arg in &mut mc.args {
                resolve_probe_enclosing_port_refs(arg, enc_id, enc_parent, enc_ports);
            }
        }
        Expr::ExprDef(def) => {
            let mut body = (*def.body).clone();
            resolve_probe_enclosing_port_refs(&mut body, enc_id, enc_parent, enc_ports);
            Arc::make_mut(def).body = Arc::new(body);
        }
        Expr::Match(m) => {
            let mut next_scrutinee = m.scrutinee.as_deref().cloned();
            if let Some(ref mut s) = next_scrutinee {
                resolve_probe_enclosing_port_refs(s, enc_id, enc_parent, enc_ports);
            }
            let mut next_arms = Vec::with_capacity(m.arms.len());
            for arm in &m.arms {
                let mut cloned = arm.clone();
                if let MatchPattern::Condition(ref mut cond) = cloned.pattern {
                    resolve_probe_enclosing_port_refs(cond, enc_id, enc_parent, enc_ports);
                }
                resolve_probe_enclosing_port_refs(&mut cloned.body, enc_id, enc_parent, enc_ports);
                next_arms.push(cloned);
            }
            *Arc::make_mut(m) = MatchExpr {
                scrutinee: next_scrutinee.map(Arc::new),
                arms: next_arms,
                span: m.span,
            };
        }
        Expr::Node(elem) => {
            for p in &mut elem.ports {
                resolve_probe_enclosing_port_refs(&mut p.expr, enc_id, enc_parent, enc_ports);
            }
        }
        Expr::Literal(_) | Expr::DomNodeRef(_, _) => {}
    }
}

impl BuiltinPrimitive for PageValuePrimitive {
    fn name(&self) -> &'static str {
        "PageValue"
    }

    fn is_non_spatial(&self) -> bool {
        true
    }

    fn accepted_ports(&self) -> &'static [&'static str] {
        &[]
    }

    fn required_port_sets(&self) -> &'static [&'static [&'static str]] {
        &[&[]]
    }

    fn children_cardinality(&self) -> BuiltinChildren {
        BuiltinChildren::None
    }

    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        state.ports.insert(
            "__bucket_count".to_string(),
            Expr::Literal(Literal::Integer(0, state.elem.span)),
        );
        Ok(())
    }

    fn overload_params(&self, _state: &PrimitiveExpandState<'_, '_>) -> Vec<String> {
        Vec::new()
    }
}

impl BuiltinPrimitive for PageListPrimitive {
    fn name(&self) -> &'static str {
        "PageList"
    }

    fn is_non_spatial(&self) -> bool {
        true
    }

    fn accepted_ports(&self) -> &'static [&'static str] {
        &[]
    }

    fn required_port_sets(&self) -> &'static [&'static [&'static str]] {
        &[&[]]
    }

    fn children_cardinality(&self) -> BuiltinChildren {
        BuiltinChildren::None
    }

    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        state.ports.insert(
            "__bucket_count".to_string(),
            Expr::Literal(Literal::Integer(0, state.elem.span)),
        );
        Ok(())
    }

    fn overload_params(&self, _state: &PrimitiveExpandState<'_, '_>) -> Vec<String> {
        Vec::new()
    }
}

impl BuiltinPrimitive for DepositPrimitive {
    fn name(&self) -> &'static str {
        "Deposit"
    }

    fn is_non_spatial(&self) -> bool {
        true
    }

    fn accepted_ports(&self) -> &'static [&'static str] {
        &["into"]
    }

    fn required_port_sets(&self) -> &'static [&'static [&'static str]] {
        &[&["into"]]
    }

    fn children_cardinality(&self) -> BuiltinChildren {
        BuiltinChildren::Single
    }

    fn defers_content_slot(&self) -> bool {
        true
    }

    fn port_type(&self, port_name: &str) -> Option<StaticType> {
        match port_name {
            "into" => Some(StaticType::Node("Node".to_string())),
            _ => default_builtin_port_type(port_name),
        }
    }

    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        let elem = state.elem;
        let into_expr = state
            .ports
            .get("into")
            .ok_or_else(|| CompileError::MissingPort {
                node: "Deposit".to_string(),
                port: "into".to_string(),
                span: elem.span,
            })?;

        let Some(bucket_id) = resolve_bucket_node_id(into_expr, state.doc) else {
            return Err(CompileError::InvalidDepositTarget {
                found: format!("{into_expr}"),
                span: into_expr.span(),
            });
        };

        let items = elem
            .content
            .as_ref()
            .map(|s| s.items.as_slice())
            .unwrap_or(&[]);
        let registry = state.registry;
        let coalesced = coalesce_block_content_items(items, registry);

        let mut captured_lexical_scope = state.ctx.lexical_scope.borrow().clone();
        let mut elements: Vec<(ElementNode, Option<crate::ast::TextChunk>)> = Vec::new();

        for item in coalesced {
            match item {
                CoalescedBlockItem::Element {
                    elem: child_elem,
                    raw_text,
                    ..
                } => {
                    elements.push((child_elem, raw_text));
                }
                CoalescedBlockItem::NonFlowLet(l) => {
                    if let Some(LetValue::Expr(raw_expr)) = &l.value {
                        let empty_children: [NodeId; 0] = [];
                        let scope_ctx = ScopeContext::from_element(
                            state.node_id,
                            state.ctx,
                            &state.doc.nodes,
                            &captured_lexical_scope,
                            state.node_fonts,
                            &empty_children,
                        );
                        let rewritten = rewrite_expr(raw_expr, &scope_ctx)?;
                        let rewritten = crate::compiler::expand::typecheck::coerce_typed_binding_expr(
                            rewritten,
                            l.type_annotation.as_ref(),
                        );
                        captured_lexical_scope
                            .insert(l.name.name, LexicalBinding::Expr(rewritten));
                    }
                }
                CoalescedBlockItem::Children(_) => {}
            }
        }

        if elements.len() != 1 {
            return Err(CompileError::InvalidDepositChildCount {
                found: elements.len(),
                span: elem.span,
            });
        }

        let outer_consumer_children = own_consumer_children_slice(state.ctx.consumer_children);
        let (root_elem, raw_text) = elements.into_iter().next().unwrap();
        let payload = OwnedConsumerChild {
            elem: root_elem,
            var_name: None,
            raw_text,
            caller_lexical_scope: captured_lexical_scope,
            caller_env_scope: state.ctx.env_scope.clone(),
            caller_enclosing_component_id: state.ctx.enclosing_component_id,
            caller_enclosing_num_children: state.ctx.enclosing_num_children,
            caller_shadowed_ports: state.ctx.shadowed_ports.cloned().unwrap_or_default(),
            outer_consumer_children,
        };

        state.doc.deposits.push(DepositRecord {
            deposit_id: state.node_id,
            bucket_id,
            host_node_id: None,
            inline_byte_index: None,
            payload,
            span: elem.span,
        });

        Ok(())
    }

    fn overload_params(&self, _state: &PrimitiveExpandState<'_, '_>) -> Vec<String> {
        vec!["into".to_string()]
    }
}

fn own_consumer_children_slice(
    entries: Option<&[crate::compiler::expand::context::ConsumerChildEntry<'_>]>,
) -> Vec<OwnedConsumerChild> {
    let Some(list) = entries else {
        return Vec::new();
    };
    list.iter()
        .map(|e| OwnedConsumerChild {
            elem: e.elem.clone(),
            var_name: e.var_name.clone(),
            raw_text: e.raw_text.clone(),
            caller_lexical_scope: e.effective_lexical_scope().borrow().clone(),
            caller_env_scope: e.caller_env_scope.clone(),
            caller_enclosing_component_id: e.caller_enclosing_component_id,
            caller_enclosing_num_children: e.caller_enclosing_num_children,
            caller_shadowed_ports: e
                .effective_shadowed_ports()
                .cloned()
                .unwrap_or_default(),
            outer_consumer_children: e.owned_children.clone(),
        })
        .collect()
}

/// Validates and extracts `(bucket_id, is_list)` from a `\Collect(from: bucket[page])` expression.
pub(crate) fn extract_collect_bucket_target(
    from_expr: &Expr,
    doc: &ExpandedDocument,
) -> Option<(NodeId, bool)> {
    let mut cur = from_expr;
    while let Expr::Paren(inner, _) = cur {
        cur = inner;
    }
    let Expr::Index(idx) = cur else {
        return None;
    };
    let mut idx_inner = idx.index.as_ref();
    while let Expr::Paren(inner, _) = idx_inner {
        idx_inner = inner;
    }
    let Expr::Ident(page_ident) = idx_inner else {
        return None;
    };
    if !page_ident.node_id().is_some_and(|nid| nid.is_page()) {
        return None;
    }
    let bucket_id = resolve_bucket_node_id(&idx.target, doc)?;
    let is_list = doc
        .get_node(bucket_id)
        .is_some_and(|n| n.name == "PageList");
    if !is_list {
        return None;
    }
    Some((bucket_id, true))
}

impl BuiltinPrimitive for CollectPrimitive {
    fn name(&self) -> &'static str {
        "Collect"
    }

    fn is_non_spatial(&self) -> bool {
        true
    }

    fn accepted_ports(&self) -> &'static [&'static str] {
        &["from"]
    }

    fn required_port_sets(&self) -> &'static [&'static [&'static str]] {
        &[&["from"]]
    }

    fn children_cardinality(&self) -> BuiltinChildren {
        BuiltinChildren::None
    }

    fn port_type(&self, port_name: &str) -> Option<StaticType> {
        match port_name {
            "from" => Some(StaticType::PageListSlice(None)),
            _ => default_builtin_port_type(port_name),
        }
    }

    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        let elem = state.elem;
        let from_expr = state
            .ports
            .get("from")
            .ok_or_else(|| CompileError::MissingPort {
                node: "Collect".to_string(),
                port: "from".to_string(),
                span: elem.span,
            })?;

        if extract_collect_bucket_target(from_expr, state.doc).is_none() {
            return Err(CompileError::InvalidCollectTarget {
                found: format!("{from_expr}"),
                span: from_expr.span(),
            });
        }
        Ok(())
    }

    fn overload_params(&self, _state: &PrimitiveExpandState<'_, '_>) -> Vec<String> {
        vec!["from".to_string()]
    }
}

/// Wires default `x`, `y` (`prev ? prev.bottom : self.top`), and `width` (`self.__flow_body_width`)
/// onto direct spatial children of `\PagedFlow` that accept them and omit explicit values.
pub(super) fn wire_paged_flow_child_elem(
    flow_node_id: NodeId,
    child_elem: &ElementNode,
    prev_sibling_id: Option<NodeId>,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> ElementNode {
    let mut wired = child_elem.clone();
    let span = child_elem.span;
    let flow_ident = Expr::Ident(Ident::from_node(flow_node_id, span));

    let has_explicit = |name: &str| child_elem.ports.iter().any(|p| p.name.as_str() == name);

    if !has_explicit("x") && element_accepts_port(child_elem, "x", registry) {
        wired.ports.push(PortBinding {
            name: Ident::new("x", span),
            expr: Expr::MemberAccess(MemberAccessExpr {
                target: std::sync::Arc::new(flow_ident.clone()),
                member: Ident::from_symbol(crate::symbol::Symbol::LEFT, span),
                span,
            }),
            span,
        });
    }
    if !has_explicit("y") && element_accepts_port(child_elem, "y", registry) {
        let y_expr = if let Some(prev_id) = prev_sibling_id {
            Expr::MemberAccess(MemberAccessExpr {
                target: std::sync::Arc::new(Expr::Ident(Ident::from_node(prev_id, span))),
                member: Ident::from_symbol(crate::symbol::Symbol::BOTTOM, span),
                span,
            })
        } else {
            Expr::MemberAccess(MemberAccessExpr {
                target: std::sync::Arc::new(flow_ident.clone()),
                member: Ident::from_symbol(crate::symbol::Symbol::TOP, span),
                span,
            })
        };
        wired.ports.push(PortBinding {
            name: Ident::new("y", span),
            expr: y_expr,
            span,
        });
    }
    if !has_explicit("width") && element_accepts_port(child_elem, "width", registry) {
        wired.ports.push(PortBinding {
            name: Ident::new("width", span),
            expr: Expr::MemberAccess(MemberAccessExpr {
                target: std::sync::Arc::new(flow_ident),
                member: Ident::new("__flow_body_width", span),
                span,
            }),
            span,
        });
    }
    wired
}
