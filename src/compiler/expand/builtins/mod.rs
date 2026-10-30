pub mod anchor;
pub mod box_prim;
pub mod clip;
pub mod font;
pub mod line;
pub mod rect;
pub mod text;
pub mod window;

use crate::ast::{
    AssertDirective, BinaryExpr, BinaryOp, CallExpr, ChildrenDirective, ComponentDef, ContentItem,
    DistanceUnit, ElementNode, EnumDef, Expr, Ident, LetBinding, LetValue, Literal,
    MemberAccessExpr, ParamDef, PortBinding, TypeRef,
};
use crate::compiler::error::CompileError;
use crate::compiler::eval::eval_expr;
use crate::compiler::expand::children::apply_forwarded_directives;
use crate::compiler::expand::context::{ElementContext, EnvEntry, LexicalBinding, ScopeContext};
use crate::compiler::expand::expand_element;
use crate::compiler::expand::rewrite::{extract_event_binding, rewrite_expr};
use crate::compiler::expand::typecheck::{
    infer_expr_literal_for_typecheck, validate_node_type_annotation, StaticType,
};
use crate::compiler::expanded::{ExpandedDocument, NodeId};
use crate::compiler::scope::ScopeId;
use crate::compiler::text::{InlineBoxSpan, TextSpan};
use crate::component::EventHandlerBinding;
use crate::span::Span;
use std::collections::HashMap;

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
    enums.entry("TextEnd".to_string()).or_insert_with(|| EnumDef {
        name: Ident::new("TextEnd", span),
        variants: vec![
            Ident::new("Baseline", span),
            Ident::new("Descender", span),
            Ident::new("GapBottom", span),
        ],
        span,
    });
    enums
        .entry("TextStart".to_string())
        .or_insert_with(|| EnumDef {
            name: Ident::new("TextStart", span),
            variants: vec![
                Ident::new("Capital", span),
                Ident::new("Ascender", span),
                Ident::new("EmTop", span),
            ],
            span,
        });
    enums.entry("LineCap".to_string()).or_insert_with(|| EnumDef {
        name: Ident::new("LineCap", span),
        variants: vec![
            Ident::new("Butt", span),
            Ident::new("Round", span),
            Ident::new("Square", span),
        ],
        span,
    });
    registry
        .entry("TextBounds".to_string())
        .or_insert_with(|| {
            vec![ComponentDef {
                kind: crate::ast::ComponentKind::Component,
                name: Ident::new("TextBounds", span),
                params: vec![
                    ParamDef {
                        name: Ident::new("start_at", span),
                        type_annotation: Some(TypeRef {
                            name: Ident::new("TextStart", span),
                            span,
                        }),
                        default_edge: Some(Expr::MemberAccess(MemberAccessExpr {
                            target: Box::new(Expr::Ident(Ident::new("TextStart", span))),
                            member: Ident::new("Capital", span),
                            span,
                        })),
                        is_env: false,
                        span,
                    },
                    ParamDef {
                        name: Ident::new("end_at", span),
                        type_annotation: Some(TypeRef {
                            name: Ident::new("TextEnd", span),
                            span,
                        }),
                        default_edge: Some(Expr::MemberAccess(MemberAccessExpr {
                            target: Box::new(Expr::Ident(Ident::new("TextEnd", span))),
                            member: Ident::new("Baseline", span),
                            span,
                        })),
                        is_env: false,
                        span,
                    },
                ],
                body: Vec::new(),
                span,
            }]
        });
    registry.entry("Shadow".to_string()).or_insert_with(|| {
        vec![ComponentDef {
            kind: crate::ast::ComponentKind::Component,
            name: Ident::new("Shadow", span),
            params: vec![
                ParamDef {
                    name: Ident::new("offset_x", span),
                    type_annotation: Some(TypeRef {
                        name: Ident::new("Distance", span),
                        span,
                    }),
                    default_edge: Some(Expr::Literal(Literal::Distance(
                        0.0,
                        DistanceUnit::Px,
                        span,
                    ))),
                    is_env: false,
                    span,
                },
                ParamDef {
                    name: Ident::new("offset_y", span),
                    type_annotation: Some(TypeRef {
                        name: Ident::new("Distance", span),
                        span,
                    }),
                    default_edge: Some(Expr::Literal(Literal::Distance(
                        0.0,
                        DistanceUnit::Px,
                        span,
                    ))),
                    is_env: false,
                    span,
                },
                ParamDef {
                    name: Ident::new("blur", span),
                    type_annotation: Some(TypeRef {
                        name: Ident::new("Distance", span),
                        span,
                    }),
                    default_edge: Some(Expr::Literal(Literal::Distance(
                        0.0,
                        DistanceUnit::Px,
                        span,
                    ))),
                    is_env: false,
                    span,
                },
                ParamDef {
                    name: Ident::new("spread", span),
                    type_annotation: Some(TypeRef {
                        name: Ident::new("Distance", span),
                        span,
                    }),
                    default_edge: Some(Expr::Literal(Literal::Distance(
                        0.0,
                        DistanceUnit::Px,
                        span,
                    ))),
                    is_env: false,
                    span,
                },
                ParamDef {
                    name: Ident::new("color", span),
                    type_annotation: Some(TypeRef {
                        name: Ident::new("Color", span),
                        span,
                    }),
                    default_edge: Some(Expr::Literal(Literal::Color(
                        "#00000000".to_string(),
                        span,
                    ))),
                    is_env: false,
                    span,
                },
            ],
            body: Vec::new(),
            span,
        }]
    });
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

/// Resolves default `height` and `width` ports for spatial primitives (including text measurement calls).
pub(super) fn resolve_default_spatial_dimensions(
    state: &mut PrimitiveExpandState<'_, '_>,
    span_style_args: &[Expr],
) -> Result<(), CompileError> {
    let elem = state.elem;
    let node_id = state.node_id;
    let is_anchor_like = elem.name.as_str() == "Anchor" || elem.name.as_str() == "AnchorScope";
    if is_anchor_like {
        return Ok(());
    }

    let text_content = state
        .doc
        .get_node(node_id)
        .and_then(|n| n.text_content.clone());
    let has_dynamic_text = state.ports.contains_key("text") || state.ports.contains_key("content");
    let has_inline_boxes = state
        .doc
        .get_node(node_id)
        .is_some_and(|n| !n.inline_boxes.is_empty());
    let has_text = text_content.as_ref().is_some_and(|t| !t.is_empty())
        || has_dynamic_text
        || has_inline_boxes;

    let self_ident = Expr::Ident(Ident::new(node_id.canonical_name(), elem.span));
    let size_expr = if state.ports.contains_key("size") {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("size", elem.span),
            span: elem.span,
        })
    } else {
        Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span))
    };

    let weight_expr = if state.ports.contains_key("weight") {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("weight", elem.span),
            span: elem.span,
        })
    } else {
        Expr::Literal(Literal::Integer(400, elem.span))
    };

    let font_expr = if state.ports.contains_key("family") {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("family", elem.span),
            span: elem.span,
        })
    } else if state.ports.contains_key("font") {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("font", elem.span),
            span: elem.span,
        })
    } else {
        Expr::Literal(Literal::String(String::new(), elem.span))
    };

    let start_at_expr = if state.ports.contains_key("start_at") {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("start_at", elem.span),
            span: elem.span,
        })
    } else {
        Expr::Literal(Literal::Enum(
            "TextStart".to_string(),
            "Capital".to_string(),
            elem.span,
        ))
    };

    let text_arg_expr = if state.ports.contains_key("text") {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("text", elem.span),
            span: elem.span,
        })
    } else if state.ports.contains_key("content") {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("content", elem.span),
            span: elem.span,
        })
    } else {
        let text_str = text_content.unwrap_or_default();
        Expr::Literal(Literal::String(text_str, elem.span))
    };

    // Height defaults / resolution
    if !state.ports.contains_key("height") {
        if (elem.name.as_str() == "Text" || has_text || state.ports.contains_key("width"))
            && has_text
        {
            let width_expr = Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(self_ident.clone()),
                member: Ident::new("width", elem.span),
                span: elem.span,
            });
            let end_at_expr = if state.ports.contains_key("end_at") {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("end_at", elem.span),
                    span: elem.span,
                })
            } else {
                Expr::Literal(Literal::Enum(
                    "TextEnd".to_string(),
                    "Baseline".to_string(),
                    elem.span,
                ))
            };
            let line_height_arg = if state.ports.contains_key("line_height") {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("line_height", elem.span),
                    span: elem.span,
                })
            } else {
                Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span))
            };
            let mut height_args = vec![
                text_arg_expr.clone(),
                size_expr.clone(),
                weight_expr.clone(),
                font_expr.clone(),
                width_expr,
                end_at_expr,
                start_at_expr,
                line_height_arg,
            ];
            height_args.extend(span_style_args.iter().cloned());
            let height_call = Expr::Call(CallExpr {
                callee: Ident::new("text_height", elem.span),
                args: height_args,
                span: elem.span,
            });
            state.ports.insert("height".to_string(), height_call);
        } else if state.ports.contains_key("size") {
            state.ports.insert(
                "height".to_string(),
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident),
                    member: Ident::new("size", elem.span),
                    span: elem.span,
                }),
            );
        } else if state.is_inline_span {
            state.ports.insert(
                "height".to_string(),
                Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span)),
            );
        } else if elem.name.as_str() != "Font" && elem.name.as_str() != "Clip" {
            return Err(CompileError::MissingPort {
                node: elem.name.as_str().to_string(),
                port: "height".to_string(),
                span: elem.span,
            });
        }
    }

    // Width defaults / resolution
    if !state.ports.contains_key("width") {
        if (elem.name.as_str() == "Text" || has_text) && has_text {
            let mut width_args = vec![text_arg_expr, size_expr, weight_expr, font_expr];
            width_args.extend(span_style_args.iter().cloned());
            let width_call = Expr::Call(CallExpr {
                callee: Ident::new("text_width", elem.span),
                args: width_args,
                span: elem.span,
            });
            state.ports.insert("width".to_string(), width_call);
        } else if state.is_inline_span {
            state.ports.insert(
                "width".to_string(),
                Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span)),
            );
        } else if elem.name.as_str() != "Font" && elem.name.as_str() != "Clip" {
            return Err(CompileError::MissingPort {
                node: elem.name.as_str().to_string(),
                port: "width".to_string(),
                span: elem.span,
            });
        }
    }

    Ok(())
}

struct PreResolvedFont {
    font_node_id: Option<NodeId>,
    pre_expanded_font_expr: Option<Expr>,
    pre_rewritten_font_expr: Option<Expr>,
    ambient_font_port_expr: Option<Expr>,
}

/// Pre-resolves an explicit `font` port or ambient `env font` before expanding the primitive's content slot.
fn pre_resolve_primitive_font(
    node_id: NodeId,
    elem: &ElementNode,
    ctx: &ElementContext<'_>,
    registry: &HashMap<String, Vec<ComponentDef>>,
    doc: &mut ExpandedDocument,
    node_fonts: &mut HashMap<NodeId, NodeId>,
    child_ids: &mut Vec<NodeId>,
    last_child_id: &mut Option<NodeId>,
) -> Result<PreResolvedFont, CompileError> {
    let mut font_node_id = None;
    let mut pre_expanded_font_expr: Option<Expr> = None;
    let mut pre_rewritten_font_expr: Option<Expr> = None;
    let mut ambient_font_port_expr: Option<Expr> = None;

    if let Some(font_port) = elem.ports.iter().find(|p| p.name.as_str() == "font") {
        match &font_port.expr {
            Expr::Node(inline_elem) => {
                let parent_id = if ctx.is_let {
                    ctx.parent_id
                } else {
                    Some(node_id)
                };
                let parent_name = if ctx.is_let {
                    ctx.parent_name
                } else {
                    Some(elem.name.as_str())
                };
                let child_ctx = ElementContext {
                    parent_id,
                    parent_name,
                    prev_sibling_id: *last_child_id,
                    has_next_sibling: false,
                    parent_ports: ctx.parent_ports,
                    lexical_scope: ctx.lexical_scope,
                    env_scope: ctx.env_scope,
                    enclosing_component_id: ctx.enclosing_component_id,
                    enclosing_num_children: ctx.enclosing_num_children,
                    enclosing_instantiated_children: ctx.enclosing_instantiated_children,
                    shadowed_ports: ctx.shadowed_ports,
                    is_let: ctx.is_let,
                    ambient_authored_ports: None,
                    enums: ctx.enums,
                    active_scope_id: ctx.active_scope_id,
                    consumer_children: None,
                };
                let child_id = expand_element(inline_elem, &child_ctx, registry, doc, node_fonts)?;
                if !matches!(inline_elem.name.as_str(), "Font" | "TextBounds" | "Shadow") {
                    child_ids.push(child_id);
                    *last_child_id = Some(child_id);
                }
                let ident_expr =
                    Expr::Ident(Ident::new(child_id.canonical_name(), inline_elem.span));
                font_node_id = Some(child_id);
                pre_expanded_font_expr = Some(ident_expr.clone());
                pre_rewritten_font_expr = Some(ident_expr);
            }
            other => {
                let empty_enclosing_children: [NodeId; 0] = [];
                let enclosing_children_borrow =
                    ctx.enclosing_instantiated_children.map(|c| c.borrow());
                let enclosing_child_ids: &[NodeId] = match &enclosing_children_borrow {
                    Some(b) => b.as_slice(),
                    None => &empty_enclosing_children,
                };
                let lexical_borrow = ctx.lexical_scope.borrow();
                let temp_scope_ctx = ScopeContext::from_element(
                    node_id,
                    ctx,
                    &doc.nodes,
                    &lexical_borrow,
                    node_fonts,
                    enclosing_child_ids,
                );
                if let Ok(rewritten) = rewrite_expr(other, &temp_scope_ctx) {
                    if let Expr::Ident(id) = &rewritten {
                        if let Some(fid) = NodeId::from_canonical_name(id.as_str()) {
                            font_node_id = Some(fid);
                        }
                    }
                    pre_rewritten_font_expr = Some(rewritten);
                }
            }
        }
    } else if let Some(EnvEntry::Bound(env_expr)) = ctx.env_scope.get("font") {
        let empty_enclosing_children: [NodeId; 0] = [];
        let enclosing_children_borrow = ctx.enclosing_instantiated_children.map(|c| c.borrow());
        let enclosing_child_ids: &[NodeId] = match &enclosing_children_borrow {
            Some(b) => b.as_slice(),
            None => &empty_enclosing_children,
        };
        let lexical_borrow = ctx.lexical_scope.borrow();
        let temp_scope_ctx = ScopeContext::from_element(
            node_id,
            ctx,
            &doc.nodes,
            &lexical_borrow,
            node_fonts,
            enclosing_child_ids,
        );
        if let Ok(rewritten) = rewrite_expr(env_expr, &temp_scope_ctx) {
            if let Expr::Ident(id) = &rewritten {
                if let Some(fid) = NodeId::from_canonical_name(id.as_str()) {
                    if fid != node_id {
                        font_node_id = Some(fid);
                    }
                    if elem.name.as_str() != "Font" {
                        ambient_font_port_expr =
                            Some(Expr::Ident(Ident::new(fid.canonical_name(), elem.span)));
                    }
                }
            } else if elem.name.as_str() != "Font"
                && (matches!(rewritten, Expr::Literal(Literal::String(_, _)))
                    || elem.name.as_str() == "Text")
            {
                ambient_font_port_expr = Some(rewritten.clone());
            }
            pre_rewritten_font_expr = Some(rewritten);
        }
    }

    if let Some(fid) = font_node_id {
        node_fonts.insert(node_id, fid);
        doc.get_node_mut(node_id).unwrap().font = Some(fid);
    } else if elem.name.as_str() == "Font" {
        node_fonts.insert(node_id, node_id);
        doc.get_node_mut(node_id).unwrap().font = Some(node_id);
    }

    Ok(PreResolvedFont {
        font_node_id,
        pre_expanded_font_expr,
        pre_rewritten_font_expr,
        ambient_font_port_expr,
    })
}

/// Wires default `x = parent.left` and `y = parent.top` onto a child element of an `\Anchor` or `\AnchorScope`.
fn wire_wrapper_child_origin(node_id: NodeId, child_elem: &ElementNode) -> ElementNode {
    let mut child = child_elem.clone();
    if !child.ports.iter().any(|p| p.name.as_str() == "x") {
        child.ports.push(PortBinding {
            name: Ident::new("x", child.span),
            expr: Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(Expr::Ident(Ident::new(
                    node_id.canonical_name(),
                    child.span,
                ))),
                member: Ident::new("left", child.span),
                span: child.span,
            }),
            span: child.span,
        });
    }
    if !child.ports.iter().any(|p| p.name.as_str() == "y") {
        child.ports.push(PortBinding {
            name: Ident::new("y", child.span),
            expr: Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(Expr::Ident(Ident::new(
                    node_id.canonical_name(),
                    child.span,
                ))),
                member: Ident::new("top", child.span),
                span: child.span,
            }),
            span: child.span,
        });
    }
    child
}

#[allow(clippy::too_many_arguments)]
fn expand_primitive_content_node(
    primitive: &dyn BuiltinPrimitive,
    node_id: NodeId,
    elem: &ElementNode,
    child_elem: &ElementNode,
    ctx: &ElementContext<'_>,
    content_env_scope: &HashMap<String, EnvEntry>,
    primitive_scope_ports: &[String],
    active_scope_for_children: ScopeId,
    registry: &HashMap<String, Vec<ComponentDef>>,
    doc: &mut ExpandedDocument,
    node_fonts: &mut HashMap<NodeId, NodeId>,
    child_ids: &mut Vec<NodeId>,
    last_child_id: &mut Option<NodeId>,
    full_text: &mut String,
    text_spans: &mut Vec<TextSpan>,
    inline_boxes: &mut Vec<InlineBoxSpan>,
) -> Result<(), CompileError> {
    primitive.validate_content_child(child_elem, registry)?;
    let child_ctx = ElementContext {
        parent_id: Some(node_id),
        parent_name: Some(elem.name.as_str()),
        prev_sibling_id: *last_child_id,
        has_next_sibling: false,
        parent_ports: primitive_scope_ports,
        lexical_scope: ctx.lexical_scope,
        env_scope: content_env_scope,
        enclosing_component_id: ctx.enclosing_component_id,
        enclosing_num_children: ctx.enclosing_num_children,
        enclosing_instantiated_children: ctx.enclosing_instantiated_children,
        shadowed_ports: ctx.shadowed_ports,
        is_let: false,
        ambient_authored_ports: None,
        enums: ctx.enums,
        active_scope_id: active_scope_for_children,
        consumer_children: ctx.consumer_children,
    };

    let wired_child_elem;
    let target_elem = if primitive.wires_child_origin_to_self() {
        wired_child_elem = wire_wrapper_child_origin(node_id, child_elem);
        &wired_child_elem
    } else {
        child_elem
    };

    let child_id = expand_element(target_elem, &child_ctx, registry, doc, node_fonts)?;
    child_ids.push(child_id);
    *last_child_id = Some(child_id);

    primitive.on_content_child(
        doc,
        child_id,
        child_elem,
        full_text,
        text_spans,
        inline_boxes,
        true,
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn expand_primitive_content_let(
    primitive: &dyn BuiltinPrimitive,
    node_id: NodeId,
    elem: &ElementNode,
    let_binding: &LetBinding,
    ctx: &ElementContext<'_>,
    content_env_scope: &HashMap<String, EnvEntry>,
    primitive_scope_ports: &[String],
    active_scope_for_children: ScopeId,
    registry: &HashMap<String, Vec<ComponentDef>>,
    doc: &mut ExpandedDocument,
    node_fonts: &mut HashMap<NodeId, NodeId>,
    child_ids: &mut Vec<NodeId>,
    last_child_id: &mut Option<NodeId>,
    full_text: &mut String,
    text_spans: &mut Vec<TextSpan>,
    inline_boxes: &mut Vec<InlineBoxSpan>,
) -> Result<(), CompileError> {
    if let Some(LetValue::Node(child_elem)) = &let_binding.value {
        validate_node_type_annotation(
            child_elem.name.as_str(),
            child_elem.span,
            let_binding.type_annotation.as_ref(),
            ctx.enums,
            registry,
        )?;
        primitive.validate_content_child(child_elem, registry)?;
        let child_ctx = ElementContext {
            parent_id: Some(node_id),
            parent_name: Some(elem.name.as_str()),
            prev_sibling_id: *last_child_id,
            has_next_sibling: false,
            parent_ports: primitive_scope_ports,
            lexical_scope: ctx.lexical_scope,
            env_scope: content_env_scope,
            enclosing_component_id: ctx.enclosing_component_id,
            enclosing_num_children: ctx.enclosing_num_children,
            enclosing_instantiated_children: ctx.enclosing_instantiated_children,
            shadowed_ports: ctx.shadowed_ports,
            is_let: true,
            ambient_authored_ports: None,
            enums: ctx.enums,
            active_scope_id: active_scope_for_children,
            consumer_children: ctx.consumer_children,
        };

        let wired_child_elem;
        let target_elem = if primitive.wires_child_origin_to_self() {
            wired_child_elem = wire_wrapper_child_origin(node_id, child_elem);
            &wired_child_elem
        } else {
            child_elem
        };

        let child_id = expand_element(target_elem, &child_ctx, registry, doc, node_fonts)?;
        doc.get_node_mut(child_id).unwrap().var_name = Some(let_binding.name.as_str().to_string());
        ctx.lexical_scope.borrow_mut().insert(
            let_binding.name.as_str().to_string(),
            LexicalBinding::Node(child_id),
        );
        child_ids.push(child_id);
        *last_child_id = Some(child_id);

        if child_elem.name.as_str() == "Font" {
            node_fonts.insert(child_id, child_id);
        }

        primitive.on_content_child(
            doc,
            child_id,
            child_elem,
            full_text,
            text_spans,
            inline_boxes,
            true,
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn expand_primitive_content_children(
    primitive: &dyn BuiltinPrimitive,
    node_id: NodeId,
    elem: &ElementNode,
    dir: &ChildrenDirective,
    ctx: &ElementContext<'_>,
    content_env_scope: &HashMap<String, EnvEntry>,
    primitive_scope_ports: &[String],
    active_scope_for_children: ScopeId,
    registry: &HashMap<String, Vec<ComponentDef>>,
    doc: &mut ExpandedDocument,
    node_fonts: &mut HashMap<NodeId, NodeId>,
    child_ids: &mut Vec<NodeId>,
    last_child_id: &mut Option<NodeId>,
    full_text: &mut String,
    text_spans: &mut Vec<TextSpan>,
    inline_boxes: &mut Vec<InlineBoxSpan>,
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
    let Some(consumer_items) = ctx.consumer_children else {
        return Ok(());
    };

    let num_consumer_children = consumer_items.len();
    let empty_children: [NodeId; 0] = [];
    for (child_idx, entry) in consumer_items.iter().enumerate() {
        if let Some(chunk) = &entry.raw_text {
            if primitive.forwards_font_to_content_env()
                && dir.ports.is_empty()
                && entry.forwarded_directives.is_empty()
            {
                primitive.on_content_text(&chunk.text, full_text, text_spans);
                continue;
            }
        }

        let has_next_sibling = child_idx + 1 < num_consumer_children;
        let child_elem = &entry.elem;
        if entry.raw_text.is_none() {
            primitive.validate_content_child(child_elem, registry)?;
        }

        let mut merged_ports = HashMap::new();
        let mut authored_ambient_ports = HashMap::new();
        let child_node_id = NodeId(doc.nodes.len());

        {
            let lexical_borrow = ctx.lexical_scope.borrow();
            let inst_borrow = ctx.enclosing_instantiated_children.map(|c| c.borrow());
            let enc_child_ids: &[NodeId] = match &inst_borrow {
                Some(b) => b.as_slice(),
                None => &empty_children,
            };
            let mut ambient_scope_ctx = ScopeContext::from_element(
                child_node_id,
                ctx,
                &doc.nodes,
                &lexical_borrow,
                node_fonts,
                enc_child_ids,
            )
            .with_parent(
                Some(node_id),
                Some(elem.name.as_str()),
                primitive_scope_ports,
            )
            .with_siblings(*last_child_id, has_next_sibling)
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
            *last_child_id,
            has_next_sibling,
            &doc.nodes,
            node_fonts,
            ctx.enums,
            &mut merged_ports,
            &mut authored_ambient_ports,
        )?;

        for explicit in &child_elem.ports {
            merged_ports.insert(explicit.name.as_str().to_string(), explicit.expr.clone());
            authored_ambient_ports.remove(explicit.name.as_str());
        }

        if primitive.wires_child_origin_to_self() {
            if !merged_ports.contains_key("x") {
                merged_ports.insert(
                    "x".to_string(),
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(
                            node_id.canonical_name(),
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
                            node_id.canonical_name(),
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
        let child_env_scope = if primitive.forwards_font_to_content_env() {
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
            parent_id: Some(node_id),
            parent_name: Some(elem.name.as_str()),
            prev_sibling_id: *last_child_id,
            has_next_sibling,
            parent_ports: primitive_scope_ports,
            lexical_scope: entry.caller_lexical_scope,
            env_scope: child_env_scope,
            enclosing_component_id: entry.caller_enclosing_component_id,
            enclosing_num_children: entry.caller_enclosing_num_children,
            enclosing_instantiated_children: entry.caller_enclosing_instantiated_children,
            shadowed_ports: entry.caller_shadowed_ports,
            is_let: entry.var_name.is_some(),
            ambient_authored_ports: Some(&authored_ambient_ports),
            enums: ctx.enums,
            active_scope_id: active_scope_for_children,
            consumer_children: None,
        };
        let child_id = expand_element(&wired_elem, &child_ctx, registry, doc, node_fonts)?;
        if let Some(prev_id) = *last_child_id {
            ctx.lexical_scope.borrow_mut().insert(
                format!("__next_of_{}", prev_id.canonical_name()),
                LexicalBinding::Node(child_id),
            );
        }
        if let Some(v_name) = &entry.var_name {
            doc.get_node_mut(child_id).unwrap().var_name = Some(v_name.clone());
            entry
                .caller_lexical_scope
                .borrow_mut()
                .insert(v_name.clone(), LexicalBinding::Node(child_id));
        }
        if child_elem.name.as_str() == "Font" {
            node_fonts.insert(child_id, child_id);
        }
        for inst_list in &entry.instantiated_ids {
            inst_list.borrow_mut().push(child_id);
        }
        child_ids.push(child_id);
        *last_child_id = Some(child_id);
        primitive.on_content_child(
            doc,
            child_id,
            child_elem,
            full_text,
            text_spans,
            inline_boxes,
            false,
        );
    }

    Ok(())
}

/// Validates and coerces a literal or constant expression on a built-in primitive port.
fn validate_and_coerce_primitive_port_literal(
    port_name: &str,
    orig_expr: &Expr,
    rewritten: &mut Expr,
    scope_ctx: &ScopeContext<'_>,
) -> Result<(), CompileError> {
    if matches!(
        port_name,
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
            | "size"
            | "line_height"
            | "baseline"
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
            | "shadow_offset_x"
            | "shadow_offset_y"
            | "shadow_blur"
            | "shadow_spread"
            | "scroll_x"
            | "scroll_y"
    ) {
        let unlowered_lit = match orig_expr {
            Expr::Literal(lit) => Some(lit.clone()),
            _ => None,
        };
        if let Some(lit) =
            unlowered_lit.or_else(|| infer_expr_literal_for_typecheck(rewritten, scope_ctx))
        {
            if !lit.matches_type_name("Distance") {
                return Err(CompileError::TypeMismatch {
                    expected: "Distance".to_string(),
                    actual: lit.type_name().to_string(),
                    span: orig_expr.span(),
                });
            }
        }
        if let Expr::Literal(Literal::Integer(0, span)) = *rewritten {
            *rewritten = Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span));
        }
    } else if matches!(port_name, "z" | "weight") {
        let unlowered_lit = match orig_expr {
            Expr::Literal(lit) => Some(lit.clone()),
            _ => None,
        };
        if let Some(lit) =
            unlowered_lit.or_else(|| infer_expr_literal_for_typecheck(rewritten, scope_ctx))
        {
            if !lit.matches_type_name("Integer") {
                return Err(CompileError::TypeMismatch {
                    expected: "Integer".to_string(),
                    actual: lit.type_name().to_string(),
                    span: orig_expr.span(),
                });
            }
        }
    } else if matches!(port_name, "italic" | "underline") {
        let unlowered_lit = match orig_expr {
            Expr::Literal(lit) => Some(lit.clone()),
            _ => None,
        };
        if let Some(lit) =
            unlowered_lit.or_else(|| infer_expr_literal_for_typecheck(rewritten, scope_ctx))
        {
            if !lit.matches_type_name("Boolean") {
                return Err(CompileError::TypeMismatch {
                    expected: "Boolean".to_string(),
                    actual: lit.type_name().to_string(),
                    span: orig_expr.span(),
                });
            }
        }
    }
    Ok(())
}

/// Forwards `size`, `weight`, and `family` ports from a resolved `\Font` node or `font` port onto a spatial primitive.
fn forward_resolved_font_metrics(state: &mut PrimitiveExpandState<'_, '_>) {
    let elem = state.elem;
    if let Some(fid) = state.font_node_id {
        let font_ident = Expr::Ident(Ident::new(fid.canonical_name(), elem.span));
        if !state.ports.contains_key("size") {
            state.ports.insert(
                "size".to_string(),
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(font_ident.clone()),
                    member: Ident::new("size", elem.span),
                    span: elem.span,
                }),
            );
        }
        if !state.ports.contains_key("weight") {
            state.ports.insert(
                "weight".to_string(),
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(font_ident.clone()),
                    member: Ident::new("weight", elem.span),
                    span: elem.span,
                }),
            );
        }
        if !state.ports.contains_key("family") {
            state.ports.insert(
                "family".to_string(),
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(font_ident),
                    member: Ident::new("family", elem.span),
                    span: elem.span,
                }),
            );
        }
    } else if let Some(font_target) = state.ports.get("font").cloned() {
        if matches!(font_target, Expr::Literal(Literal::String(_, _))) {
            if !state.ports.contains_key("family") {
                state.ports.insert("family".to_string(), font_target);
            }
        } else {
            if !state.ports.contains_key("size") {
                state.ports.insert(
                    "size".to_string(),
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(font_target.clone()),
                        member: Ident::new("size", elem.span),
                        span: elem.span,
                    }),
                );
            }
            if !state.ports.contains_key("weight") {
                state.ports.insert(
                    "weight".to_string(),
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(font_target.clone()),
                        member: Ident::new("weight", elem.span),
                        span: elem.span,
                    }),
                );
            }
            if !state.ports.contains_key("family") {
                state.ports.insert(
                    "family".to_string(),
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(font_target),
                        member: Ident::new("family", elem.span),
                        span: elem.span,
                    }),
                );
            }
        }
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

    // Handle Navigation Scopes and Anchors
    let active_scope_for_children = primitive.pre_expand_children(node_id, elem, ctx, doc)?;

    // 1. Expand nested child nodes in content slot
    let mut child_ids = Vec::new();
    let mut last_child_id = None;
    let mut full_text = String::new();
    let mut text_spans = Vec::new();
    let mut inline_boxes: Vec<InlineBoxSpan> = Vec::new();

    let mut primitive_scope_ports = primitive.scope_ports();
    for p in &elem.ports {
        let pname = p.name.as_str().to_string();
        if !primitive_scope_ports.contains(&pname) {
            primitive_scope_ports.push(pname);
        }
    }

    let PreResolvedFont {
        mut font_node_id,
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

    let text_content_env_scope_storage;
    let content_env_scope: &HashMap<String, EnvEntry> = if primitive.forwards_font_to_content_env()
    {
        if let Some(fid) = font_node_id {
            let mut env_clone = ctx.env_scope.clone();
            env_clone.insert(
                "font".to_string(),
                EnvEntry::Bound(Expr::Ident(Ident::new(fid.canonical_name(), elem.span))),
            );
            text_content_env_scope_storage = Some(env_clone);
            text_content_env_scope_storage.as_ref().unwrap()
        } else if let Some(ref font_expr) = pre_rewritten_font_expr {
            let mut env_clone = ctx.env_scope.clone();
            env_clone.insert("font".to_string(), EnvEntry::Bound(font_expr.clone()));
            text_content_env_scope_storage = Some(env_clone);
            text_content_env_scope_storage.as_ref().unwrap()
        } else {
            ctx.env_scope
        }
    } else {
        ctx.env_scope
    };

    if let Some(slot) = &elem.content {
        for item in &slot.items {
            match item {
                ContentItem::Text(chunk) => {
                    primitive.on_content_text(&chunk.text, &mut full_text, &mut text_spans);
                }
                ContentItem::Node(child_elem) => {
                    expand_primitive_content_node(
                        primitive,
                        node_id,
                        elem,
                        child_elem,
                        ctx,
                        content_env_scope,
                        &primitive_scope_ports,
                        active_scope_for_children,
                        registry,
                        doc,
                        node_fonts,
                        &mut child_ids,
                        &mut last_child_id,
                        &mut full_text,
                        &mut text_spans,
                        &mut inline_boxes,
                    )?;
                }
                ContentItem::Let(l) => {
                    expand_primitive_content_let(
                        primitive,
                        node_id,
                        elem,
                        l,
                        ctx,
                        content_env_scope,
                        &primitive_scope_ports,
                        active_scope_for_children,
                        registry,
                        doc,
                        node_fonts,
                        &mut child_ids,
                        &mut last_child_id,
                        &mut full_text,
                        &mut text_spans,
                        &mut inline_boxes,
                    )?;
                }
                ContentItem::ParagraphBreak(span) => {
                    primitive.on_paragraph_break(*span)?;
                }
                ContentItem::Alias(_)
                | ContentItem::State(_)
                | ContentItem::Env(_)
                | ContentItem::Assert(_) => {}
                ContentItem::Children(dir) => {
                    expand_primitive_content_children(
                        primitive,
                        node_id,
                        elem,
                        dir,
                        ctx,
                        content_env_scope,
                        &primitive_scope_ports,
                        active_scope_for_children,
                        registry,
                        doc,
                        node_fonts,
                        &mut child_ids,
                        &mut last_child_id,
                        &mut full_text,
                        &mut text_spans,
                        &mut inline_boxes,
                    )?;
                }
            }
        }
    }

    primitive.finish_content(node_id, doc, full_text, text_spans, inline_boxes);

    // 1b. Expand any inline node element expressions in ports
    let mut resolved_port_exprs = Vec::with_capacity(elem.ports.len() + 1);
    for port in &elem.ports {
        if port.name.as_str() == "font" {
            if let Some(ref pre_expr) = pre_expanded_font_expr {
                resolved_port_exprs.push(("font".to_string(), pre_expr.clone()));
                continue;
            }
        }
        match &port.expr {
            Expr::Node(inline_elem) => {
                let parent_id = if ctx.is_let {
                    ctx.parent_id
                } else {
                    Some(node_id)
                };
                let parent_name = if ctx.is_let {
                    ctx.parent_name
                } else {
                    Some(elem.name.as_str())
                };
                let child_ctx = ElementContext {
                    parent_id,
                    parent_name,
                    prev_sibling_id: last_child_id,
                    has_next_sibling: false,
                    parent_ports: ctx.parent_ports,
                    lexical_scope: ctx.lexical_scope,
                    env_scope: ctx.env_scope,
                    enclosing_component_id: ctx.enclosing_component_id,
                    enclosing_num_children: ctx.enclosing_num_children,
                    enclosing_instantiated_children: ctx.enclosing_instantiated_children,
                    shadowed_ports: ctx.shadowed_ports,
                    is_let: ctx.is_let,
                    ambient_authored_ports: None,
                    enums: ctx.enums,
                    active_scope_id: ctx.active_scope_id,
                    consumer_children: None,
                };
                let child_id = expand_element(inline_elem, &child_ctx, registry, doc, node_fonts)?;
                if !matches!(inline_elem.name.as_str(), "Font" | "TextBounds" | "Shadow") {
                    child_ids.push(child_id);
                    last_child_id = Some(child_id);
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
    if let Some(amb_font) = ambient_font_port_expr {
        resolved_port_exprs.push(("font".to_string(), amb_font));
    }

    // 2. Rewrite element ports
    let empty_enclosing_children: [NodeId; 0] = [];
    let enclosing_children_borrow = ctx.enclosing_instantiated_children.map(|c| c.borrow());
    let enclosing_child_ids: &[NodeId] = match &enclosing_children_borrow {
        Some(b) => b.as_slice(),
        None => &empty_enclosing_children,
    };

    // Fallback check if `font` referenced a `\let` declared inside `elem.content`
    if font_node_id.is_none() {
        if let Some((_, raw_font_expr)) =
            resolved_port_exprs.iter().find(|(name, _)| name == "font")
        {
            let lexical_borrow = ctx.lexical_scope.borrow();
            let temp_scope_ctx = ScopeContext::from_element(
                node_id,
                ctx,
                &doc.nodes,
                &lexical_borrow,
                node_fonts,
                enclosing_child_ids,
            );
            if let Ok(Expr::Ident(id)) = rewrite_expr(raw_font_expr, &temp_scope_ctx) {
                if let Some(fid) = NodeId::from_canonical_name(id.as_str()) {
                    font_node_id = Some(fid);
                }
            }
        }
    }

    if let Some(fid) = font_node_id {
        node_fonts.insert(node_id, fid);
        doc.get_node_mut(node_id).unwrap().font = Some(fid);
    } else if elem.name.as_str() == "Font" {
        node_fonts.insert(node_id, node_id);
        doc.get_node_mut(node_id).unwrap().font = Some(node_id);
    }

    let mut ports = HashMap::new();
    let mut authored_ports = HashMap::new();
    let mut event_handlers = HashMap::new();

    {
        let lexical_borrow = ctx.lexical_scope.borrow();
        let scope_ctx = ScopeContext::from_element(
            node_id,
            ctx,
            &doc.nodes,
            &lexical_borrow,
            node_fonts,
            enclosing_child_ids,
        );

        if let Some(content_slot) = &elem.content {
            for item in &content_slot.items {
                if let ContentItem::Assert(assert_dir) = item {
                    eval_assert_directive(assert_dir, &scope_ctx)?;
                }
            }
        }

        for (port_name, expr) in resolved_port_exprs {
            let is_ambient = ctx
                .ambient_authored_ports
                .is_some_and(|m| m.contains_key(&port_name));
            if primitive.ignores_ambient_box_ports()
                && is_ambient
                && matches!(port_name.as_str(), "x" | "y" | "width" | "height")
            {
                continue;
            }
            if let Some(ambient_expr) = ctx.ambient_authored_ports.and_then(|m| m.get(&port_name)) {
                authored_ports.insert(port_name.clone(), ambient_expr.clone());
            } else if let Some(orig_port) = elem.ports.iter().find(|p| p.name.as_str() == port_name)
            {
                authored_ports.insert(port_name.clone(), orig_port.expr.clone());
            } else if port_name == "font" {
                authored_ports.insert(
                    port_name.clone(),
                    Expr::Ident(Ident::new("font", elem.span)),
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

    let is_inline_span = ctx
        .parent_id
        .is_some_and(|p| doc.get_node(p).is_some_and(|n| n.name == "Text"));

    let mut state = PrimitiveExpandState {
        node_id,
        elem,
        ctx,
        doc,
        node_fonts,
        active_scope_for_children,
        font_node_id,
        is_inline_span,
        child_ids,
        ports,
        authored_ports,
        event_handlers,
    };

    if primitive.is_non_spatial() {
        primitive.finalize_ports(&mut state)?;
        let overload_params = primitive.overload_params(&state);
        let node = state.doc.get_node_mut(node_id).unwrap();
        node.overload_params = overload_params;
        node.children = state.child_ids;
        node.ports = state.ports;
        node.authored_ports = state.authored_ports;
        node.font = Some(node_id);
        state.node_fonts.insert(node_id, node_id);
        return Ok(());
    }

    // Default `clip` port for non-Clip spatial primitives
    if primitive.name() != "Clip" && !state.ports.contains_key("clip") {
        let default_clip = if let Some(EnvEntry::Bound(clip_expr)) = ctx.env_scope.get("clip") {
            clip_expr.clone()
        } else if let Some(parent) = ctx.parent_id {
            let target_name = if parent.is_window() {
                NodeId::WINDOW.canonical_name()
            } else {
                parent.canonical_name()
            };
            Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(Expr::Ident(Ident::new(target_name, elem.span))),
                member: Ident::new("clip", elem.span),
                span: elem.span,
            })
        } else {
            Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(Expr::Ident(Ident::new(
                    NodeId::WINDOW.canonical_name(),
                    elem.span,
                ))),
                member: Ident::new("clip", elem.span),
                span: elem.span,
            })
        };
        state.ports.insert("clip".to_string(), default_clip);
    }

    forward_resolved_font_metrics(&mut state);

    // Primitive-specific port validation & synthesis
    primitive.finalize_ports(&mut state)?;

    // Fallback inline origin & spatial dimension resolution for non-Text primitives
    if primitive.name() != "Text" {
        apply_inline_origin_defaults(&mut state);
        resolve_default_spatial_dimensions(&mut state, &[])?;
    }

    // Default spatial alias ports (`left`, `top`, `right`, `bottom`)
    insert_default_spatial_aliases(
        node_id,
        elem.span,
        &mut state.ports,
        &mut state.authored_ports,
    );

    let overload_params = primitive.overload_params(&state);
    let node = state.doc.get_node_mut(node_id).unwrap();
    node.overload_params = overload_params;
    node.children = state.child_ids;
    node.ports = state.ports;
    node.authored_ports = state.authored_ports;
    node.event_handlers = state.event_handlers;

    Ok(())
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
