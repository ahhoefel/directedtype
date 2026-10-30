pub mod anchor;
pub mod box_prim;
pub mod clip;
pub mod font;
pub mod line;
pub mod rect;
pub mod text;
pub mod window;

use crate::ast::{
    AssertDirective, BinaryExpr, BinaryOp, ChildrenDirective, ComponentDef, ContentItem,
    DistanceUnit, ElementNode, EnumDef, Expr, Ident, LetBinding, LetValue, Literal,
    MemberAccessExpr, PortBinding,
};
use crate::compiler::error::CompileError;
use crate::compiler::eval::eval_expr;
use crate::compiler::expand::children::apply_forwarded_directives;
use crate::compiler::expand::context::{ElementContext, EnvEntry, LexicalBinding, ScopeContext};
use crate::compiler::expand::expand_element;
use crate::compiler::expand::rewrite::{extract_event_binding, rewrite_expr};
use crate::compiler::expand::typecheck::{
    validate_and_coerce_primitive_port_literal, validate_node_type_annotation, StaticType,
};
use crate::compiler::expanded::{ExpandedDocument, NodeId};
use crate::compiler::scope::ScopeId;
use crate::compiler::text::{InlineBoxSpan, TextSpan};
use crate::component::EventHandlerBinding;
use crate::span::Span;
use std::collections::HashMap;

use self::anchor::wire_wrapper_child_origin;
use self::clip::default_inherited_clip_expr;
use self::font::{forward_resolved_font_metrics, pre_resolve_primitive_font, PreResolvedFont};
use self::text::resolve_default_spatial_dimensions;

/// Shared mutable state passed to `BuiltinPrimitive::finalize_ports`.
pub(in crate::compiler::expand) struct PrimitiveExpandState<'a, 'ctx> {
    pub node_id: NodeId,
    pub elem: &'a ElementNode,
    pub ctx: &'a ElementContext<'ctx>,
    pub doc: &'a mut ExpandedDocument,
    pub node_fonts: &'a mut HashMap<NodeId, NodeId>,
    pub active_scope_for_children: ScopeId,
    pub font_node_id: Option<NodeId>,
    pub is_inline_span: bool,
    pub child_ids: Vec<NodeId>,
    pub ports: HashMap<String, Expr>,
    pub authored_ports: HashMap<String, Expr>,
    pub event_handlers: HashMap<String, EventHandlerBinding>,
}

/// Encapsulates the schema, scope behavior, content-slot hooks, and port finalization of a built-in primitive element.
pub(in crate::compiler::expand) trait BuiltinPrimitive: Sync {
    /// Canonical primitive name (e.g. `"Rect"`, `"Line"`, `"Text"`).
    fn name(&self) -> &'static str;

    /// Returns true if this primitive has no spatial bounding box (`\Font`).
    fn is_non_spatial(&self) -> bool {
        false
    }

    /// Returns true if this primitive ignores ambient `x`, `y`, `width`, `height` ports (`\Line`).
    fn ignores_ambient_box_ports(&self) -> bool {
        false
    }

    /// Returns true if child elements inside this primitive's content slot default `x`/`y` to `self.left`/`self.top` (`\Anchor`, `\AnchorScope`).
    fn wires_child_origin_to_self(&self) -> bool {
        false
    }

    /// Returns true if an explicit or ambient `font` on this primitive is forwarded into `env font` for its content slot (`\Text`).
    fn forwards_font_to_content_env(&self) -> bool {
        false
    }

    /// Registers any built-in enums or auxiliary struct-like components owned by this primitive.
    fn register_types(
        &self,
        _enums: &mut HashMap<String, EnumDef>,
        _registry: &mut HashMap<String, Vec<ComponentDef>>,
        _span: Span,
    ) {
    }

    /// Static type of a built-in port on this primitive.
    fn port_type(&self, port_name: &str) -> Option<StaticType> {
        default_builtin_port_type(port_name)
    }

    /// Default ports exposed in scope to children inside this primitive's content slot.
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
            "name".to_string(),
            "path".to_string(),
        ]
    }

    /// Hook invoked before expanding children in the primitive's content slot.
    /// Returns the active `ScopeId` to pass down to children.
    fn pre_expand_children(
        &self,
        _node_id: NodeId,
        _elem: &ElementNode,
        ctx: &ElementContext<'_>,
        _doc: &mut ExpandedDocument,
    ) -> Result<ScopeId, CompileError> {
        Ok(ctx.active_scope_id)
    }

    /// Validates whether `child_elem` is permitted inside this primitive's content slot.
    fn validate_content_child(
        &self,
        _child_elem: &ElementNode,
        _registry: &HashMap<String, Vec<ComponentDef>>,
    ) -> Result<(), CompileError> {
        Ok(())
    }

    /// Hook invoked when a blank-line paragraph break appears in this primitive's content slot.
    fn on_paragraph_break(&self, _span: Span) -> Result<(), CompileError> {
        Ok(())
    }

    /// Hook invoked when a plain text chunk appears in this primitive's content slot.
    fn on_content_text(&self, _text: &str, _full_text: &mut String, _text_spans: &mut Vec<TextSpan>) {}

    /// Hook invoked after a child element in this primitive's content slot has been expanded.
    #[allow(clippy::too_many_arguments)]
    fn on_content_child(
        &self,
        _doc: &mut ExpandedDocument,
        _child_id: NodeId,
        _child_elem: &ElementNode,
        _full_text: &mut String,
        _text_spans: &mut Vec<TextSpan>,
        _inline_boxes: &mut Vec<InlineBoxSpan>,
        _apply_own_style: bool,
    ) {
    }

    /// Hook invoked after all items in this primitive's content slot have been expanded.
    fn finish_content(
        &self,
        node_id: NodeId,
        doc: &mut ExpandedDocument,
        _full_text: String,
        _text_spans: Vec<TextSpan>,
        _inline_boxes: Vec<InlineBoxSpan>,
    ) {
        if let Some(tc) = &doc.nodes[node_id.0].text_content {
            doc.nodes[node_id.0].text_spans = vec![TextSpan::new(0..tc.len())];
        }
    }

    /// Validates required ports and synthesizes primitive-specific default and derived ports.
    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError>;

    /// Returns the canonical overload parameter names for this primitive node.
    fn overload_params(&self, state: &PrimitiveExpandState<'_, '_>) -> Vec<String> {
        if state.ports.contains_key("size")
            && !state.elem.ports.iter().any(|p| p.name.as_str() == "height")
        {
            vec![
                "x".to_string(),
                "y".to_string(),
                "width".to_string(),
                "size".to_string(),
            ]
        } else {
            vec![
                "x".to_string(),
                "y".to_string(),
                "width".to_string(),
                "height".to_string(),
            ]
        }
    }
}

/// Returns the static type of standard built-in spatial, typographic, visual, and handler ports.
pub(in crate::compiler::expand) fn default_builtin_port_type(port_name: &str) -> Option<StaticType> {
    match port_name {
        "x"
        | "y"
        | "x1"
        | "y1"
        | "x2"
        | "y2"
        | "start_x"
        | "start_y"
        | "end_x"
        | "end_y"
        | "width"
        | "height"
        | "left"
        | "top"
        | "right"
        | "bottom"
        | "size"
        | "line_height"
        | "baseline"
        | "cap_height"
        | "x_height"
        | "ascent"
        | "descent"
        | "em_top"
        | "line_gap"
        | "gap_bottom"
        | "radius"
        | "corner_radius"
        | "radius_left"
        | "radius_right"
        | "radius_top"
        | "radius_bottom"
        | "left_radius"
        | "right_radius"
        | "top_radius"
        | "bottom_radius"
        | "radius_top_left"
        | "radius_top_right"
        | "radius_bottom_right"
        | "radius_bottom_left"
        | "top_left_radius"
        | "top_right_radius"
        | "bottom_right_radius"
        | "bottom_left_radius"
        | "border_width"
        | "thickness"
        | "stroke_width"
        | "head_length"
        | "head_width"
        | "length"
        | "shadow_offset_x"
        | "shadow_offset_y"
        | "shadow_blur"
        | "shadow_spread"
        | "scroll_x"
        | "scroll_y" => Some(StaticType::Distance),
        "z" | "weight" => Some(StaticType::Integer),
        "opacity" => Some(StaticType::Float),
        "color" | "bg_color" | "border_color" | "stroke_color" | "shadow_color" => {
            Some(StaticType::Color)
        }
        "family" | "text" | "content" | "cursor" | "url" | "name" | "path" | "id" => {
            Some(StaticType::String)
        }
        "underline" | "italic" => Some(StaticType::Boolean),
        "clip" | "up" => Some(StaticType::Node("Clip".to_string())),
        "font" => Some(StaticType::Node("Font".to_string())),
        "text_bounds" => Some(StaticType::Node("TextBounds".to_string())),
        "shadow" => Some(StaticType::Node("Shadow".to_string())),
        "start_at" => Some(StaticType::Enum("TextStart".to_string())),
        "end_at" => Some(StaticType::Enum("TextEnd".to_string())),
        "cap" | "line_cap" => Some(StaticType::Enum("LineCap".to_string())),
        p if p.starts_with("on_") => Some(StaticType::Handler),
        _ => None,
    }
}

/// Evaluates an `\assert(...)` directive in the given `scope_ctx`.
pub(in crate::compiler::expand) fn eval_assert_directive(
    assert_dir: &AssertDirective,
    scope_ctx: &ScopeContext<'_>,
) -> Result<(), CompileError> {
    let rewritten = rewrite_expr(&assert_dir.condition, scope_ctx)?;
    match eval_expr(&rewritten, &HashMap::new()) {
        Ok(val) => {
            if !val.is_truthy() {
                let msg = assert_dir
                    .message
                    .clone()
                    .unwrap_or_else(|| format!("Assertion failed: {}", assert_dir.condition));
                return Err(CompileError::AssertionFailed {
                    message: msg,
                    span: assert_dir.span,
                });
            }
            Ok(())
        }
        Err(e) => Err(CompileError::Custom {
            message: format!("Could not evaluate assertion condition: {}", e),
            span: assert_dir.span,
        }),
    }
}

const ALL_BUILTINS: &[&dyn BuiltinPrimitive] = &[
    &rect::RectPrimitive,
    &line::LinePrimitive,
    &text::TextPrimitive,
    &font::FontPrimitive,
    &clip::ClipPrimitive,
    &box_prim::BoxPrimitive,
    &anchor::AnchorPrimitive,
    &anchor::AnchorScopePrimitive,
    &box_prim::LinkPrimitive,
];

/// Looks up a built-in primitive handler by element name.
pub(in crate::compiler::expand) fn lookup_builtin(
    name: &str,
    is_inline_span: bool,
) -> Option<&'static dyn BuiltinPrimitive> {
    match name {
        "Rect" => Some(&rect::RectPrimitive),
        "Line" => Some(&line::LinePrimitive),
        "Text" => Some(&text::TextPrimitive),
        "Font" => Some(&font::FontPrimitive),
        "Clip" => Some(&clip::ClipPrimitive),
        "Box" => Some(&box_prim::BoxPrimitive),
        "Anchor" => Some(&anchor::AnchorPrimitive),
        "AnchorScope" => Some(&anchor::AnchorScopePrimitive),
        "Link" if is_inline_span => Some(&box_prim::LinkPrimitive),
        _ => None,
    }
}

/// Registers built-in standard enums (`TextEnd`, `TextStart`, `LineCap`) and built-in struct-like
/// components (`TextBounds`, `Shadow`) into `enums` and `registry`.
pub fn register_builtin_enums_and_components(
    enums: &mut HashMap<String, EnumDef>,
    registry: &mut HashMap<String, Vec<ComponentDef>>,
    span: Span,
) {
    for prim in ALL_BUILTINS {
        prim.register_types(enums, registry, span);
    }
}

/// Inserts default spatial alias ports (`left`, `top`, `right`, `bottom`) onto `ports` and `authored_ports`.
/// Shared by both primitive elements and user-defined components.
pub fn insert_default_spatial_aliases(
    node_id: NodeId,
    span: Span,
    ports: &mut HashMap<String, Expr>,
    authored_ports: &mut HashMap<String, Expr>,
) {
    let self_ident = Expr::Ident(Ident::new(node_id.canonical_name(), span));
    if !ports.contains_key("left") {
        ports.insert(
            "left".to_string(),
            Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(self_ident.clone()),
                member: Ident::new("x", span),
                span,
            }),
        );
        authored_ports.insert("left".to_string(), Expr::Ident(Ident::new("x", span)));
    }
    if !ports.contains_key("top") {
        ports.insert(
            "top".to_string(),
            Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(self_ident.clone()),
                member: Ident::new("y", span),
                span,
            }),
        );
        authored_ports.insert("top".to_string(), Expr::Ident(Ident::new("y", span)));
    }
    if !ports.contains_key("right") {
        ports.insert(
            "right".to_string(),
            Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("x", span),
                    span,
                })),
                right: Box::new(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("width", span),
                    span,
                })),
                span,
            }),
        );
        authored_ports.insert(
            "right".to_string(),
            Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(Expr::Ident(Ident::new("x", span))),
                right: Box::new(Expr::Ident(Ident::new("width", span))),
                span,
            }),
        );
    }
    if !ports.contains_key("bottom") {
        ports.insert(
            "bottom".to_string(),
            Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("y", span),
                    span,
                })),
                right: Box::new(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident),
                    member: Ident::new("height", span),
                    span,
                })),
                span,
            }),
        );
        authored_ports.insert(
            "bottom".to_string(),
            Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(Expr::Ident(Ident::new("y", span))),
                right: Box::new(Expr::Ident(Ident::new("height", span))),
                span,
            }),
        );
    }
}

/// Applies default `x` and `y` origin ports when an element is an inline span or direct child of an `\InlineBox`.
pub(super) fn apply_inline_origin_defaults(state: &mut PrimitiveExpandState<'_, '_>) {
    let elem = state.elem;
    let ctx = state.ctx;
    let parent_is_inline_box = ctx
        .parent_id
        .is_some_and(|p| state.doc.get_node(p).is_some_and(|n| n.is_inline_box));

    if state.is_inline_span {
        state
            .ports
            .entry("x".to_string())
            .or_insert_with(|| Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span)));
        state
            .ports
            .entry("y".to_string())
            .or_insert_with(|| Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span)));
    } else if parent_is_inline_box {
        if let Some(pid) = ctx.parent_id {
            let p_ident = Expr::Ident(Ident::new(pid.canonical_name(), elem.span));
            state.ports.entry("x".to_string()).or_insert_with(|| {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(p_ident.clone()),
                    member: Ident::new("x", elem.span),
                    span: elem.span,
                })
            });
            state.ports.entry("y".to_string()).or_insert_with(|| {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(p_ident),
                    member: Ident::new("y", elem.span),
                    span: elem.span,
                })
            });
        }
    }
}

/// Stateful template pipeline for expanding a single built-in primitive element.
struct PrimitiveExpander<'a, 'ctx> {
    primitive: &'static dyn BuiltinPrimitive,
    node_id: NodeId,
    elem: &'a ElementNode,
    ctx: &'a ElementContext<'ctx>,
    registry: &'a HashMap<String, Vec<ComponentDef>>,
    doc: &'a mut ExpandedDocument,
    node_fonts: &'a mut HashMap<NodeId, NodeId>,

    active_scope_for_children: ScopeId,
    primitive_scope_ports: Vec<String>,
    font_node_id: Option<NodeId>,
    pre_expanded_font_expr: Option<Expr>,
    pre_rewritten_font_expr: Option<Expr>,
    ambient_font_port_expr: Option<Expr>,

    child_ids: Vec<NodeId>,
    last_child_id: Option<NodeId>,
    full_text: String,
    text_spans: Vec<TextSpan>,
    inline_boxes: Vec<InlineBoxSpan>,
}

impl<'a, 'ctx> PrimitiveExpander<'a, 'ctx> {
    fn new(
        primitive: &'static dyn BuiltinPrimitive,
        node_id: NodeId,
        elem: &'a ElementNode,
        ctx: &'a ElementContext<'ctx>,
        registry: &'a HashMap<String, Vec<ComponentDef>>,
        doc: &'a mut ExpandedDocument,
        node_fonts: &'a mut HashMap<NodeId, NodeId>,
    ) -> Result<Self, CompileError> {
        // Validate that caller does not attempt to override immutable spatial alias ports
        for port in &elem.ports {
            let is_purely_ambient = ctx
                .ambient_authored_ports
                .is_some_and(|a| a.contains_key(port.name.as_str()));
            if is_purely_ambient {
                continue;
            }
            if matches!(port.name.as_str(), "left" | "top" | "right" | "bottom") {
                return Err(CompileError::ImmutableAliasPort {
                    node: elem.name.as_str().to_string(),
                    port: port.name.as_str().to_string(),
                    span: port.name.span,
                });
            }
        }

        let active_scope_for_children = primitive.pre_expand_children(node_id, elem, ctx, doc)?;

        let mut primitive_scope_ports = primitive.scope_ports();
        for p in &elem.ports {
            let pname = p.name.as_str().to_string();
            if !primitive_scope_ports.contains(&pname) {
                primitive_scope_ports.push(pname);
            }
        }

        let mut child_ids = Vec::new();
        let mut last_child_id = None;
        let PreResolvedFont {
            font_node_id,
            pre_expanded_font_expr,
            pre_rewritten_font_expr,
            ambient_font_port_expr,
        } = pre_resolve_primitive_font(
            node_id,
            elem,
            ctx,
            registry,
            doc,
            node_fonts,
            &mut child_ids,
            &mut last_child_id,
        )?;

        Ok(Self {
            primitive,
            node_id,
            elem,
            ctx,
            registry,
            doc,
            node_fonts,
            active_scope_for_children,
            primitive_scope_ports,
            font_node_id,
            pre_expanded_font_expr,
            pre_rewritten_font_expr,
            ambient_font_port_expr,
            child_ids,
            last_child_id,
            full_text: String::new(),
            text_spans: Vec::new(),
            inline_boxes: Vec::new(),
        })
    }

    fn expand_content_slot(&mut self) -> Result<(), CompileError> {
        let text_content_env_scope_storage;
        let content_env_scope: &HashMap<String, EnvEntry> =
            if self.primitive.forwards_font_to_content_env() {
                if let Some(fid) = self.font_node_id {
                    let mut env_clone = self.ctx.env_scope.clone();
                    env_clone.insert(
                        "font".to_string(),
                        EnvEntry::Bound(Expr::Ident(Ident::new(
                            fid.canonical_name(),
                            self.elem.span,
                        ))),
                    );
                    text_content_env_scope_storage = Some(env_clone);
                    text_content_env_scope_storage.as_ref().unwrap()
                } else if let Some(ref font_expr) = self.pre_rewritten_font_expr {
                    let mut env_clone = self.ctx.env_scope.clone();
                    env_clone.insert("font".to_string(), EnvEntry::Bound(font_expr.clone()));
                    text_content_env_scope_storage = Some(env_clone);
                    text_content_env_scope_storage.as_ref().unwrap()
                } else {
                    self.ctx.env_scope
                }
            } else {
                self.ctx.env_scope
            };

        if let Some(slot) = &self.elem.content {
            for item in &slot.items {
                match item {
                    ContentItem::Text(chunk) => {
                        self.primitive.on_content_text(
                            &chunk.text,
                            &mut self.full_text,
                            &mut self.text_spans,
                        );
                    }
                    ContentItem::Node(child_elem) => {
                        self.expand_content_node(child_elem, content_env_scope)?;
                    }
                    ContentItem::Let(l) => {
                        self.expand_content_let(l, content_env_scope)?;
                    }
                    ContentItem::ParagraphBreak(span) => {
                        self.primitive.on_paragraph_break(*span)?;
                    }
                    ContentItem::Alias(_)
                    | ContentItem::State(_)
                    | ContentItem::Env(_)
                    | ContentItem::Assert(_) => {}
                    ContentItem::Children(dir) => {
                        self.expand_content_children(dir, content_env_scope)?;
                    }
                }
            }
        }

        let full_text = std::mem::take(&mut self.full_text);
        let text_spans = std::mem::take(&mut self.text_spans);
        let inline_boxes = std::mem::take(&mut self.inline_boxes);
        self.primitive
            .finish_content(self.node_id, self.doc, full_text, text_spans, inline_boxes);
        Ok(())
    }

    fn expand_content_node(
        &mut self,
        child_elem: &ElementNode,
        content_env_scope: &HashMap<String, EnvEntry>,
    ) -> Result<(), CompileError> {
        self.primitive
            .validate_content_child(child_elem, self.registry)?;
        let child_ctx = ElementContext {
            parent_id: Some(self.node_id),
            parent_name: Some(self.elem.name.as_str()),
            prev_sibling_id: self.last_child_id,
            has_next_sibling: false,
            parent_ports: &self.primitive_scope_ports,
            lexical_scope: self.ctx.lexical_scope,
            env_scope: content_env_scope,
            enclosing_component_id: self.ctx.enclosing_component_id,
            enclosing_num_children: self.ctx.enclosing_num_children,
            enclosing_instantiated_children: self.ctx.enclosing_instantiated_children,
            shadowed_ports: self.ctx.shadowed_ports,
            is_let: false,
            ambient_authored_ports: None,
            enums: self.ctx.enums,
            active_scope_id: self.active_scope_for_children,
            consumer_children: self.ctx.consumer_children,
        };

        let wired_child_elem;
        let target_elem = if self.primitive.wires_child_origin_to_self() {
            wired_child_elem = wire_wrapper_child_origin(self.node_id, child_elem);
            &wired_child_elem
        } else {
            child_elem
        };

        let child_id = expand_element(
            target_elem,
            &child_ctx,
            self.registry,
            self.doc,
            self.node_fonts,
        )?;
        self.child_ids.push(child_id);
        self.last_child_id = Some(child_id);

        self.primitive.on_content_child(
            self.doc,
            child_id,
            child_elem,
            &mut self.full_text,
            &mut self.text_spans,
            &mut self.inline_boxes,
            true,
        );
        Ok(())
    }

    fn expand_content_let(
        &mut self,
        let_binding: &LetBinding,
        content_env_scope: &HashMap<String, EnvEntry>,
    ) -> Result<(), CompileError> {
        if let Some(LetValue::Node(child_elem)) = &let_binding.value {
            validate_node_type_annotation(
                child_elem.name.as_str(),
                child_elem.span,
                let_binding.type_annotation.as_ref(),
                self.ctx.enums,
                self.registry,
            )?;
            self.primitive
                .validate_content_child(child_elem, self.registry)?;
            let child_ctx = ElementContext {
                parent_id: Some(self.node_id),
                parent_name: Some(self.elem.name.as_str()),
                prev_sibling_id: self.last_child_id,
                has_next_sibling: false,
                parent_ports: &self.primitive_scope_ports,
                lexical_scope: self.ctx.lexical_scope,
                env_scope: content_env_scope,
                enclosing_component_id: self.ctx.enclosing_component_id,
                enclosing_num_children: self.ctx.enclosing_num_children,
                enclosing_instantiated_children: self.ctx.enclosing_instantiated_children,
                shadowed_ports: self.ctx.shadowed_ports,
                is_let: true,
                ambient_authored_ports: None,
                enums: self.ctx.enums,
                active_scope_id: self.active_scope_for_children,
                consumer_children: self.ctx.consumer_children,
            };

            let wired_child_elem;
            let target_elem = if self.primitive.wires_child_origin_to_self() {
                wired_child_elem = wire_wrapper_child_origin(self.node_id, child_elem);
                &wired_child_elem
            } else {
                child_elem
            };

            let child_id = expand_element(
                target_elem,
                &child_ctx,
                self.registry,
                self.doc,
                self.node_fonts,
            )?;
            self.doc.get_node_mut(child_id).unwrap().var_name =
                Some(let_binding.name.as_str().to_string());
            self.ctx.lexical_scope.borrow_mut().insert(
                let_binding.name.as_str().to_string(),
                LexicalBinding::Node(child_id),
            );
            self.child_ids.push(child_id);
            self.last_child_id = Some(child_id);

            if child_elem.name.as_str() == "Font" {
                self.node_fonts.insert(child_id, child_id);
            }

            self.primitive.on_content_child(
                self.doc,
                child_id,
                child_elem,
                &mut self.full_text,
                &mut self.text_spans,
                &mut self.inline_boxes,
                true,
            );
        }
        Ok(())
    }

    fn expand_content_children(
        &mut self,
        dir: &ChildrenDirective,
        content_env_scope: &HashMap<String, EnvEntry>,
    ) -> Result<(), CompileError> {
        for port in &dir.ports {
            if port.name.as_str() == "parent" {
                return Err(CompileError::ReservedPort {
                    node: "Children".to_string(),
                    port: "parent".to_string(),
                    span: port.name.span,
                });
            }
        }
        let Some(consumer_items) = self.ctx.consumer_children else {
            return Ok(());
        };

        let num_consumer_children = consumer_items.len();
        let empty_children: [NodeId; 0] = [];
        for (child_idx, entry) in consumer_items.iter().enumerate() {
            if let Some(chunk) = &entry.raw_text {
                if self.primitive.forwards_font_to_content_env()
                    && dir.ports.is_empty()
                    && entry.forwarded_directives.is_empty()
                {
                    self.primitive.on_content_text(
                        &chunk.text,
                        &mut self.full_text,
                        &mut self.text_spans,
                    );
                    continue;
                }
            }

            let has_next_sibling = child_idx + 1 < num_consumer_children;
            let child_elem = &entry.elem;
            if entry.raw_text.is_none() {
                self.primitive
                    .validate_content_child(child_elem, self.registry)?;
            }

            let mut merged_ports = HashMap::new();
            let mut authored_ambient_ports = HashMap::new();
            let child_node_id = NodeId(self.doc.nodes.len());

            {
                let lexical_borrow = self.ctx.lexical_scope.borrow();
                let inst_borrow = self
                    .ctx
                    .enclosing_instantiated_children
                    .map(|c| c.borrow());
                let enc_child_ids: &[NodeId] = match &inst_borrow {
                    Some(b) => b.as_slice(),
                    None => &empty_children,
                };
                let mut ambient_scope_ctx = ScopeContext::from_element(
                    child_node_id,
                    self.ctx,
                    &self.doc.nodes,
                    &lexical_borrow,
                    self.node_fonts,
                    enc_child_ids,
                )
                .with_parent(
                    Some(self.node_id),
                    Some(self.elem.name.as_str()),
                    &self.primitive_scope_ports,
                )
                .with_siblings(self.last_child_id, has_next_sibling)
                .with_child_node(Some(child_node_id))
                .with_env_scope(content_env_scope);
                for ambient in &dir.ports {
                    ambient_scope_ctx.is_event_port = ambient.name.as_str().starts_with("on_");
                    let rewritten = rewrite_expr(&ambient.expr, &ambient_scope_ctx)?;
                    merged_ports.insert(ambient.name.as_str().to_string(), rewritten);
                    authored_ambient_ports
                        .insert(ambient.name.as_str().to_string(), ambient.expr.clone());
                }
            }

            apply_forwarded_directives(
                &entry.forwarded_directives,
                child_node_id,
                self.last_child_id,
                has_next_sibling,
                &self.doc.nodes,
                self.node_fonts,
                self.ctx.enums,
                &mut merged_ports,
                &mut authored_ambient_ports,
            )?;

            for explicit in &child_elem.ports {
                merged_ports.insert(explicit.name.as_str().to_string(), explicit.expr.clone());
                authored_ambient_ports.remove(explicit.name.as_str());
            }

            if self.primitive.wires_child_origin_to_self() {
                if !merged_ports.contains_key("x") {
                    merged_ports.insert(
                        "x".to_string(),
                        Expr::MemberAccess(MemberAccessExpr {
                            target: Box::new(Expr::Ident(Ident::new(
                                self.node_id.canonical_name(),
                                child_elem.span,
                            ))),
                            member: Ident::new("left", child_elem.span),
                            span: child_elem.span,
                        }),
                    );
                }
                if !merged_ports.contains_key("y") {
                    merged_ports.insert(
                        "y".to_string(),
                        Expr::MemberAccess(MemberAccessExpr {
                            target: Box::new(Expr::Ident(Ident::new(
                                self.node_id.canonical_name(),
                                child_elem.span,
                            ))),
                            member: Ident::new("top", child_elem.span),
                            span: child_elem.span,
                        }),
                    );
                }
            }

            let mut wired_elem = child_elem.clone();
            wired_elem.ports = merged_ports
                .into_iter()
                .map(|(name, expr)| PortBinding {
                    name: Ident::new(name, child_elem.span),
                    expr,
                    span: child_elem.span,
                })
                .collect();

            let text_child_env_storage;
            let child_env_scope = if self.primitive.forwards_font_to_content_env() {
                if let Some(font_entry) = content_env_scope.get("font") {
                    let mut env_clone = entry.caller_env_scope.clone();
                    env_clone.insert("font".to_string(), font_entry.clone());
                    text_child_env_storage = Some(env_clone);
                    text_child_env_storage.as_ref().unwrap()
                } else {
                    &entry.caller_env_scope
                }
            } else {
                &entry.caller_env_scope
            };

            let child_ctx = ElementContext {
                parent_id: Some(self.node_id),
                parent_name: Some(self.elem.name.as_str()),
                prev_sibling_id: self.last_child_id,
                has_next_sibling,
                parent_ports: &self.primitive_scope_ports,
                lexical_scope: entry.caller_lexical_scope,
                env_scope: child_env_scope,
                enclosing_component_id: entry.caller_enclosing_component_id,
                enclosing_num_children: entry.caller_enclosing_num_children,
                enclosing_instantiated_children: entry.caller_enclosing_instantiated_children,
                shadowed_ports: entry.caller_shadowed_ports,
                is_let: entry.var_name.is_some(),
                ambient_authored_ports: Some(&authored_ambient_ports),
                enums: self.ctx.enums,
                active_scope_id: self.active_scope_for_children,
                consumer_children: None,
            };
            let child_id = expand_element(
                &wired_elem,
                &child_ctx,
                self.registry,
                self.doc,
                self.node_fonts,
            )?;
            if let Some(prev_id) = self.last_child_id {
                self.ctx.lexical_scope.borrow_mut().insert(
                    format!("__next_of_{}", prev_id.canonical_name()),
                    LexicalBinding::Node(child_id),
                );
            }
            if let Some(v_name) = &entry.var_name {
                self.doc.get_node_mut(child_id).unwrap().var_name = Some(v_name.clone());
                entry
                    .caller_lexical_scope
                    .borrow_mut()
                    .insert(v_name.clone(), LexicalBinding::Node(child_id));
            }
            if child_elem.name.as_str() == "Font" {
                self.node_fonts.insert(child_id, child_id);
            }
            for inst_list in &entry.instantiated_ids {
                inst_list.borrow_mut().push(child_id);
            }
            self.child_ids.push(child_id);
            self.last_child_id = Some(child_id);
            self.primitive.on_content_child(
                self.doc,
                child_id,
                child_elem,
                &mut self.full_text,
                &mut self.text_spans,
                &mut self.inline_boxes,
                false,
            );
        }

        Ok(())
    }

    fn expand_inline_port_nodes(&mut self) -> Result<Vec<(String, Expr)>, CompileError> {
        let mut resolved_port_exprs = Vec::with_capacity(self.elem.ports.len() + 1);
        for port in &self.elem.ports {
            if port.name.as_str() == "font" {
                if let Some(ref pre_expr) = self.pre_expanded_font_expr {
                    resolved_port_exprs.push(("font".to_string(), pre_expr.clone()));
                    continue;
                }
            }
            match &port.expr {
                Expr::Node(inline_elem) => {
                    let parent_id = if self.ctx.is_let {
                        self.ctx.parent_id
                    } else {
                        Some(self.node_id)
                    };
                    let parent_name = if self.ctx.is_let {
                        self.ctx.parent_name
                    } else {
                        Some(self.elem.name.as_str())
                    };
                    let child_ctx = ElementContext {
                        parent_id,
                        parent_name,
                        prev_sibling_id: self.last_child_id,
                        has_next_sibling: false,
                        parent_ports: self.ctx.parent_ports,
                        lexical_scope: self.ctx.lexical_scope,
                        env_scope: self.ctx.env_scope,
                        enclosing_component_id: self.ctx.enclosing_component_id,
                        enclosing_num_children: self.ctx.enclosing_num_children,
                        enclosing_instantiated_children: self.ctx.enclosing_instantiated_children,
                        shadowed_ports: self.ctx.shadowed_ports,
                        is_let: self.ctx.is_let,
                        ambient_authored_ports: None,
                        enums: self.ctx.enums,
                        active_scope_id: self.ctx.active_scope_id,
                        consumer_children: None,
                    };
                    let child_id = expand_element(
                        inline_elem,
                        &child_ctx,
                        self.registry,
                        self.doc,
                        self.node_fonts,
                    )?;
                    if !matches!(inline_elem.name.as_str(), "Font" | "TextBounds" | "Shadow") {
                        self.child_ids.push(child_id);
                        self.last_child_id = Some(child_id);
                    }
                    resolved_port_exprs.push((
                        port.name.as_str().to_string(),
                        Expr::Ident(Ident::new(child_id.canonical_name(), inline_elem.span)),
                    ));
                }
                other => {
                    resolved_port_exprs.push((port.name.as_str().to_string(), other.clone()));
                }
            }
        }
        if let Some(amb_font) = self.ambient_font_port_expr.take() {
            resolved_port_exprs.push(("font".to_string(), amb_font));
        }
        Ok(resolved_port_exprs)
    }

    #[allow(clippy::type_complexity)]
    fn rewrite_ports_and_eval_asserts(
        &mut self,
        resolved_port_exprs: Vec<(String, Expr)>,
    ) -> Result<
        (
            HashMap<String, Expr>,
            HashMap<String, Expr>,
            HashMap<String, EventHandlerBinding>,
        ),
        CompileError,
    > {
        let empty_enclosing_children: [NodeId; 0] = [];
        let enclosing_children_borrow = self
            .ctx
            .enclosing_instantiated_children
            .map(|c| c.borrow());
        let enclosing_child_ids: &[NodeId] = match &enclosing_children_borrow {
            Some(b) => b.as_slice(),
            None => &empty_enclosing_children,
        };

        // Fallback check if `font` referenced a `\let` declared inside `elem.content`
        if self.font_node_id.is_none() {
            if let Some((_, raw_font_expr)) =
                resolved_port_exprs.iter().find(|(name, _)| name == "font")
            {
                let lexical_borrow = self.ctx.lexical_scope.borrow();
                let temp_scope_ctx = ScopeContext::from_element(
                    self.node_id,
                    self.ctx,
                    &self.doc.nodes,
                    &lexical_borrow,
                    self.node_fonts,
                    enclosing_child_ids,
                );
                if let Ok(Expr::Ident(id)) = rewrite_expr(raw_font_expr, &temp_scope_ctx) {
                    if let Some(fid) = NodeId::from_canonical_name(id.as_str()) {
                        self.font_node_id = Some(fid);
                    }
                }
            }
        }

        if let Some(fid) = self.font_node_id {
            self.node_fonts.insert(self.node_id, fid);
            self.doc.get_node_mut(self.node_id).unwrap().font = Some(fid);
        } else if self.primitive.is_non_spatial() {
            self.node_fonts.insert(self.node_id, self.node_id);
            self.doc.get_node_mut(self.node_id).unwrap().font = Some(self.node_id);
        }

        let mut ports = HashMap::new();
        let mut authored_ports = HashMap::new();
        let mut event_handlers = HashMap::new();

        {
            let lexical_borrow = self.ctx.lexical_scope.borrow();
            let scope_ctx = ScopeContext::from_element(
                self.node_id,
                self.ctx,
                &self.doc.nodes,
                &lexical_borrow,
                self.node_fonts,
                enclosing_child_ids,
            );

            if let Some(content_slot) = &self.elem.content {
                for item in &content_slot.items {
                    if let ContentItem::Assert(assert_dir) = item {
                        eval_assert_directive(assert_dir, &scope_ctx)?;
                    }
                }
            }

            for (port_name, expr) in resolved_port_exprs {
                let is_ambient = self
                    .ctx
                    .ambient_authored_ports
                    .is_some_and(|m| m.contains_key(&port_name));
                if self.primitive.ignores_ambient_box_ports()
                    && is_ambient
                    && matches!(port_name.as_str(), "x" | "y" | "width" | "height")
                {
                    continue;
                }
                if let Some(ambient_expr) = self
                    .ctx
                    .ambient_authored_ports
                    .and_then(|m| m.get(&port_name))
                {
                    authored_ports.insert(port_name.clone(), ambient_expr.clone());
                } else if let Some(orig_port) = self
                    .elem
                    .ports
                    .iter()
                    .find(|p| p.name.as_str() == port_name)
                {
                    authored_ports.insert(port_name.clone(), orig_port.expr.clone());
                } else if port_name == "font" {
                    authored_ports.insert(
                        port_name.clone(),
                        Expr::Ident(Ident::new("font", self.elem.span)),
                    );
                } else {
                    authored_ports.insert(port_name.clone(), expr.clone());
                }

                if port_name.starts_with("on_") {
                    let mut event_scope_ctx = scope_ctx.clone();
                    event_scope_ctx.is_event_port = true;
                    let rewritten = rewrite_expr(&expr, &event_scope_ctx)?;
                    if let Some(binding) = extract_event_binding(&port_name, &rewritten) {
                        event_handlers.insert(port_name, binding);
                        continue;
                    }
                }

                let mut rewritten = rewrite_expr(&expr, &scope_ctx)?;
                validate_and_coerce_primitive_port_literal(
                    &port_name,
                    &expr,
                    &mut rewritten,
                    &scope_ctx,
                )?;
                ports.insert(port_name, rewritten);
            }
        }

        Ok((ports, authored_ports, event_handlers))
    }

    fn finish(
        self,
        ports: HashMap<String, Expr>,
        authored_ports: HashMap<String, Expr>,
        event_handlers: HashMap<String, EventHandlerBinding>,
    ) -> Result<(), CompileError> {
        let is_inline_span = self
            .ctx
            .parent_id
            .is_some_and(|p| self.doc.get_node(p).is_some_and(|n| n.name == "Text"));

        let mut state = PrimitiveExpandState {
            node_id: self.node_id,
            elem: self.elem,
            ctx: self.ctx,
            doc: self.doc,
            node_fonts: self.node_fonts,
            active_scope_for_children: self.active_scope_for_children,
            font_node_id: self.font_node_id,
            is_inline_span,
            child_ids: self.child_ids,
            ports,
            authored_ports,
            event_handlers,
        };

        if self.primitive.is_non_spatial() {
            self.primitive.finalize_ports(&mut state)?;
            let overload_params = self.primitive.overload_params(&state);
            let node = state.doc.get_node_mut(self.node_id).unwrap();
            node.overload_params = overload_params;
            node.children = state.child_ids;
            node.ports = state.ports;
            node.authored_ports = state.authored_ports;
            node.font = Some(self.node_id);
            state.node_fonts.insert(self.node_id, self.node_id);
            return Ok(());
        }

        if self.primitive.name() != "Clip" && !state.ports.contains_key("clip") {
            let default_clip = default_inherited_clip_expr(
                self.ctx.parent_id,
                self.ctx.env_scope,
                self.elem.span,
            );
            state.ports.insert("clip".to_string(), default_clip);
        }

        forward_resolved_font_metrics(&mut state);

        self.primitive.finalize_ports(&mut state)?;

        if self.primitive.name() != "Text" {
            apply_inline_origin_defaults(&mut state);
            resolve_default_spatial_dimensions(&mut state, &[])?;
        }

        insert_default_spatial_aliases(
            self.node_id,
            self.elem.span,
            &mut state.ports,
            &mut state.authored_ports,
        );

        let overload_params = self.primitive.overload_params(&state);
        let node = state.doc.get_node_mut(self.node_id).unwrap();
        node.overload_params = overload_params;
        node.children = state.child_ids;
        node.ports = state.ports;
        node.authored_ports = state.authored_ports;
        node.event_handlers = state.event_handlers;

        Ok(())
    }
}

/// Expands a primitive element (e.g. `\Rect`, `\Text`, `\Font`, `\Line`, `\Clip`, `\Box`, `\Anchor`, `\AnchorScope`).
pub(in crate::compiler::expand) fn expand_primitive_element(
    node_id: NodeId,
    elem: &ElementNode,
    ctx: &ElementContext<'_>,
    registry: &HashMap<String, Vec<ComponentDef>>,
    doc: &mut ExpandedDocument,
    node_fonts: &mut HashMap<NodeId, NodeId>,
) -> Result<(), CompileError> {
    let is_inline_span = ctx
        .parent_id
        .is_some_and(|p| doc.get_node(p).is_some_and(|n| n.name == "Text"));

    let Some(primitive) = lookup_builtin(elem.name.as_str(), is_inline_span) else {
        return Err(CompileError::UndefinedComponent {
            name: elem.name.as_str().to_string(),
            span: elem.name.span,
        });
    };

    let mut expander =
        PrimitiveExpander::new(primitive, node_id, elem, ctx, registry, doc, node_fonts)?;
    expander.expand_content_slot()?;
    let resolved_port_exprs = expander.expand_inline_port_nodes()?;
    let (ports, authored_ports, event_handlers) =
        expander.rewrite_ports_and_eval_asserts(resolved_port_exprs)?;
    expander.finish(ports, authored_ports, event_handlers)
}
