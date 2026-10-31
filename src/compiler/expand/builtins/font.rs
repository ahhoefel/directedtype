use crate::ast::{
    BinaryExpr, BinaryOp, CallExpr, ComponentDef, ElementNode, Expr, Ident, Literal,
    MemberAccessExpr,
};
use crate::compiler::error::{CompileError, NoMatchingOverloadDetails};
use crate::compiler::expand::builtins::{
    default_builtin_port_type, BuiltinPrimitive, PrimitiveExpandState,
};
use crate::compiler::expand::context::{ElementContext, EnvEntry, ScopeContext};
use crate::compiler::expand::expand_element;
use crate::compiler::expand::rewrite::rewrite_expr;
use crate::compiler::expand::typecheck::StaticType;
use crate::compiler::expanded::{ExpandedDocument, NodeId};
use std::collections::HashMap;

pub struct FontPrimitive;

const FONT_PUBLIC_PORTS: &[&str] = &["font", "family", "size", "weight", "line_height", "italic"];

pub(super) struct PreResolvedFont {
    pub font_node_id: Option<NodeId>,
    pub pre_expanded_font_expr: Option<Expr>,
    pub pre_rewritten_font_expr: Option<Expr>,
    pub ambient_font_port_expr: Option<Expr>,
}

/// Pre-resolves an explicit `font` port or ambient `env font` before expanding the primitive's content slot.
#[allow(clippy::too_many_arguments)]
pub(super) fn pre_resolve_primitive_font(
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
            } else if elem.name.as_str() == "Text" {
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

impl BuiltinPrimitive for FontPrimitive {
    fn name(&self) -> &'static str {
        "Font"
    }

    fn is_non_spatial(&self) -> bool {
        true
    }

    fn accepts_port(&self, _port_name: &str) -> bool {
        true
    }

    fn port_type(&self, port_name: &str) -> Option<StaticType> {
        match port_name {
            "size" | "line_height" | "cap_height" | "x_height" | "ascent" | "descent"
            | "em_top" | "line_gap" | "gap_bottom" => Some(StaticType::Distance),
            "weight" => Some(StaticType::Integer),
            "family" => Some(StaticType::String),
            "italic" => Some(StaticType::Boolean),
            "font" => Some(StaticType::Node("Font".to_string())),
            _ => default_builtin_port_type(port_name),
        }
    }

    fn scope_ports(&self) -> Vec<String> {
        vec![
            "family".to_string(),
            "size".to_string(),
            "weight".to_string(),
            "line_height".to_string(),
            "cap_height".to_string(),
            "x_height".to_string(),
            "descent".to_string(),
            "ascent".to_string(),
            "em_top".to_string(),
            "line_gap".to_string(),
            "gap_bottom".to_string(),
        ]
    }

    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        let elem = state.elem;
        let node_id = state.node_id;

        for port in &elem.ports {
            let name = port.name.as_str();
            if !FONT_PUBLIC_PORTS.contains(&name) {
                let mut provided: Vec<String> =
                    elem.ports.iter().map(|p| p.name.as_str().to_string()).collect();
                provided.sort();
                let available_signatures = vec![
                    vec![
                        "size".to_string(),
                        "weight".to_string(),
                        "family".to_string(),
                        "line_height".to_string(),
                        "italic".to_string(),
                    ],
                    vec![
                        "font".to_string(),
                        "size".to_string(),
                        "weight".to_string(),
                        "family".to_string(),
                        "line_height".to_string(),
                        "italic".to_string(),
                    ],
                ];
                return Err(CompileError::NoMatchingOverload(Box::new(
                    NoMatchingOverloadDetails {
                        name: "Font".to_string(),
                        provided_ports: provided,
                        available_signatures,
                        span: port.name.span,
                    },
                )));
            }
        }

        if let Some(Expr::Literal(lit)) = state.ports.get("font") {
            return Err(CompileError::TypeMismatch {
                expected: "Font".to_string(),
                actual: lit.type_name().to_string(),
                span: lit.span(),
            });
        }

        let base_font_id = state.font_node_id.filter(|&fid| fid != node_id);
        if let Some(fid) = base_font_id {
            if let Some(fnode) = state.doc.get_node(fid) {
                if fnode.name != "Font" {
                    return Err(CompileError::TypeMismatch {
                        expected: "Font".to_string(),
                        actual: fnode.name.clone(),
                        span: elem.span,
                    });
                }
            }
        }

        let base_font_target = base_font_id
            .map(|fid| Expr::Ident(Ident::new(fid.canonical_name(), elem.span)))
            .or_else(|| state.ports.get("font").cloned());

        if let Some(Expr::Literal(Literal::String(ref family, span))) =
            state.ports.get("family").cloned()
        {
            if !family.is_empty() && !crate::compiler::text::font_family_exists(family) {
                return Err(CompileError::FontFamilyNotFound {
                    family: family.clone(),
                    span,
                });
            }
        }

        if let Some(base_target) = base_font_target {
            if !state.ports.contains_key("size") {
                state.ports.insert(
                    "size".to_string(),
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(base_target.clone()),
                        member: Ident::new("size", elem.span),
                        span: elem.span,
                    }),
                );
            }
            if !state.ports.contains_key("weight") {
                state.ports.insert(
                    "weight".to_string(),
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(base_target.clone()),
                        member: Ident::new("weight", elem.span),
                        span: elem.span,
                    }),
                );
            }
            if !state.ports.contains_key("family") {
                state.ports.insert(
                    "family".to_string(),
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(base_target.clone()),
                        member: Ident::new("family", elem.span),
                        span: elem.span,
                    }),
                );
            }
            if !state.ports.contains_key("italic") {
                state.ports.insert(
                    "italic".to_string(),
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(base_target.clone()),
                        member: Ident::new("italic", elem.span),
                        span: elem.span,
                    }),
                );
            }
            let size_overridden = elem.ports.iter().any(|p| p.name.as_str() == "size");
            if !state.ports.contains_key("line_height") && !size_overridden {
                let base_has_authored_lh = base_font_id
                    .and_then(|fid| state.doc.get_node(fid))
                    .is_some_and(|fn_node| fn_node.authored_ports.contains_key("line_height"));
                if base_has_authored_lh {
                    let lh_expr = Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(base_target),
                        member: Ident::new("line_height", elem.span),
                        span: elem.span,
                    });
                    state.ports.insert("line_height".to_string(), lh_expr.clone());
                    state.authored_ports.insert("line_height".to_string(), lh_expr);
                }
            }
        } else {
            // Base constructor: size is required when no base font is in scope or passed explicitly
            if !state.ports.contains_key("size") {
                return Err(CompileError::MissingPort {
                    node: "Font".to_string(),
                    port: "size".to_string(),
                    span: elem.span,
                });
            }
            // Default weight to 400 if not specified
            if !state.ports.contains_key("weight") {
                state.ports.insert(
                    "weight".to_string(),
                    Expr::Literal(Literal::Integer(400, elem.span)),
                );
            }
            // Default family to empty string if not specified
            if !state.ports.contains_key("family") {
                state.ports.insert(
                    "family".to_string(),
                    Expr::Literal(Literal::String(String::new(), elem.span)),
                );
            }
            // Default italic to false if not specified
            if !state.ports.contains_key("italic") {
                state.ports.insert(
                    "italic".to_string(),
                    Expr::Literal(Literal::Bool(false, elem.span)),
                );
            }
        }

        let self_ident = Expr::Ident(Ident::new(node_id.canonical_name(), elem.span));
        let size_ref = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("size", elem.span),
            span: elem.span,
        });
        let weight_ref = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("weight", elem.span),
            span: elem.span,
        });
        let family_ref = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("family", elem.span),
            span: elem.span,
        });

        // Add typographic metric ports: cap_height, x_height, descent, ascent, em_top, line_gap, gap_bottom, line_height
        state.ports.entry("cap_height".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("font_cap_height", elem.span),
                args: vec![size_ref.clone(), weight_ref.clone(), family_ref.clone()],
                span: elem.span,
            })
        });
        state.ports.entry("x_height".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("font_x_height", elem.span),
                args: vec![size_ref.clone(), weight_ref.clone(), family_ref.clone()],
                span: elem.span,
            })
        });
        state.ports.entry("descent".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("font_descent", elem.span),
                args: vec![size_ref.clone(), weight_ref.clone(), family_ref.clone()],
                span: elem.span,
            })
        });
        state.ports.entry("ascent".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("font_ascent", elem.span),
                args: vec![size_ref.clone(), weight_ref.clone(), family_ref.clone()],
                span: elem.span,
            })
        });
        let descent_ref = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("descent", elem.span),
            span: elem.span,
        });
        state.ports.entry("em_top".to_string()).or_insert_with(|| {
            Expr::Binary(BinaryExpr {
                op: BinaryOp::Sub,
                left: Box::new(size_ref.clone()),
                right: Box::new(descent_ref.clone()),
                span: elem.span,
            })
        });
        state.ports.entry("line_gap".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("font_line_gap", elem.span),
                args: vec![size_ref.clone(), weight_ref.clone(), family_ref.clone()],
                span: elem.span,
            })
        });
        let line_gap_ref = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident),
            member: Ident::new("line_gap", elem.span),
            span: elem.span,
        });
        state.ports.entry("gap_bottom".to_string()).or_insert_with(|| {
            Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(descent_ref),
                right: Box::new(line_gap_ref),
                span: elem.span,
            })
        });
        state.ports.entry("line_height".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("font_line_height", elem.span),
                args: vec![size_ref, weight_ref, family_ref],
                span: elem.span,
            })
        });

        Ok(())
    }

    fn overload_params(&self, state: &PrimitiveExpandState<'_, '_>) -> Vec<String> {
        let has_explicit_font = state.elem.ports.iter().any(|p| p.name.as_str() == "font");
        let has_explicit_size = state.elem.ports.iter().any(|p| p.name.as_str() == "size");
        if has_explicit_font || !has_explicit_size {
            vec!["font".to_string()]
        } else {
            vec!["size".to_string()]
        }
    }
}

