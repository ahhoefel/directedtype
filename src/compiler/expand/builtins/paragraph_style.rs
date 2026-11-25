use crate::ast::{
    ComponentDef, DistanceUnit, ElementNode, EnumDef, Expr, Ident, Literal, MemberAccessExpr,
};
use crate::compiler::error::CompileError;
use crate::compiler::expand::builtins::{
    default_builtin_port_type, BuiltinPrimitive, PrimitiveExpandState,
};
use crate::compiler::expand::context::ElementContext;
use crate::compiler::expand::typecheck::StaticType;
use crate::compiler::expanded::{ExpandedDocument, NodeId};
use crate::compiler::scope::ScopeId;
use crate::span::Span;
use std::collections::HashMap;

pub struct ParagraphStylePrimitive;

/// All child ports synthesized and forwarded by `\ParagraphStyle`.
pub const PARAGRAPH_STYLE_PORTS: &[&str] = &[
    "align",
    "indent",
    "line_break",
    "word_spacing",
    "space_stretch",
    "space_shrink",
    "tolerance",
    "hyphen_penalty",
    "line_penalty",
    "flagged_demerits",
    "fitness_demerits",
    "sentence_space",
    "looseness",
];

pub fn default_paragraph_style_port_expr(port_name: &str, span: Span) -> Expr {
    match port_name {
        "align" => Expr::Literal(Literal::Enum(
            "Align".to_string(),
            "Left".to_string(),
            span,
        )),
        "indent" | "word_spacing" => {
            Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span))
        }
        "line_break" => Expr::Literal(Literal::Enum(
            "LineBreak".to_string(),
            "Greedy".to_string(),
            span,
        )),
        "space_stretch" => Expr::Literal(Literal::Float(0.5, span)),
        "space_shrink" => Expr::Literal(Literal::Float(1.0 / 3.0, span)),
        "tolerance" => Expr::Literal(Literal::Float(2.0, span)),
        "hyphen_penalty" => Expr::Literal(Literal::Float(50.0, span)),
        "line_penalty" => Expr::Literal(Literal::Float(10.0, span)),
        "flagged_demerits" => Expr::Literal(Literal::Float(3000.0, span)),
        "fitness_demerits" => Expr::Literal(Literal::Float(3000.0, span)),
        "sentence_space" => Expr::Literal(Literal::Float(0.0, span)),
        "looseness" => Expr::Literal(Literal::Integer(0, span)),
        _ => Expr::Literal(Literal::Float(0.0, span)),
    }
}

impl BuiltinPrimitive for ParagraphStylePrimitive {
    fn name(&self) -> &'static str {
        "ParagraphStyle"
    }

    fn is_non_spatial(&self) -> bool {
        true
    }

    fn register_types(
        &self,
        enums: &mut HashMap<String, EnumDef>,
        _registry: &mut HashMap<String, Vec<ComponentDef>>,
        span: Span,
    ) {
        enums
            .entry("LineBreak".to_string())
            .or_insert_with(|| EnumDef {
                name: Ident::new("LineBreak", span),
                variants: vec![Ident::new("Greedy", span), Ident::new("Optimal", span)],
                span,
            });
    }

    fn accepted_ports(&self) -> &'static [&'static str] {
        &[
            "paragraph_style",
            "align",
            "indent",
            "line_break",
            "word_spacing",
            "space_stretch",
            "space_shrink",
            "tolerance",
            "hyphen_penalty",
            "line_penalty",
            "flagged_demerits",
            "fitness_demerits",
            "sentence_space",
            "looseness",
        ]
    }

    fn port_type(&self, port_name: &str) -> Option<StaticType> {
        match port_name {
            "paragraph_style" => Some(StaticType::Node("ParagraphStyle".to_string())),
            "align" => Some(StaticType::Enum("Align".to_string())),
            "indent" | "word_spacing" => Some(StaticType::Distance),
            "line_break" => Some(StaticType::Enum("LineBreak".to_string())),
            "space_stretch"
            | "space_shrink"
            | "tolerance"
            | "hyphen_penalty"
            | "line_penalty"
            | "flagged_demerits"
            | "fitness_demerits"
            | "sentence_space" => Some(StaticType::Float),
            "looseness" => Some(StaticType::Integer),
            _ => default_builtin_port_type(port_name),
        }
    }

    fn scope_ports(&self) -> Vec<String> {
        PARAGRAPH_STYLE_PORTS.iter().map(|s| s.to_string()).collect()
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
        if let Some(slot) = &elem.content {
            return Err(CompileError::Custom {
                message: "'\\ParagraphStyle' does not accept a { ... } content slot".to_string(),
                span: slot.span,
            });
        }
        Ok(ctx.active_scope_id)
    }

    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        let elem = state.elem;
        let span = elem.span;

        if let Some(base_expr) = state.ports.get("paragraph_style").cloned() {
            if let Expr::Literal(lit) = &base_expr {
                return Err(CompileError::TypeMismatch {
                    expected: "ParagraphStyle".to_string(),
                    actual: lit.type_name().to_string(),
                    span: lit.span(),
                });
            }
            if let Expr::Ident(id) = &base_expr {
                if let Some(nid) = id.node_id() {
                    if let Some(node) = state.doc.get_node(nid) {
                        if node.name != "ParagraphStyle" {
                            return Err(CompileError::TypeMismatch {
                                expected: "ParagraphStyle".to_string(),
                                actual: node.name.clone(),
                                span,
                            });
                        }
                    }
                }
            }

            for &pname in PARAGRAPH_STYLE_PORTS {
                if !state.ports.contains_key(pname) {
                    state.ports.insert(
                        pname.to_string(),
                        Expr::MemberAccess(MemberAccessExpr {
                            target: std::sync::Arc::new(base_expr.clone()),
                            member: Ident::new(pname, span),
                            span,
                        }),
                    );
                }
            }
        } else {
            for &pname in PARAGRAPH_STYLE_PORTS {
                state
                    .ports
                    .entry(pname.to_string())
                    .or_insert_with(|| default_paragraph_style_port_expr(pname, span));
            }
        }

        Ok(())
    }

    fn overload_params(&self, state: &PrimitiveExpandState<'_, '_>) -> Vec<String> {
        if state.ports.contains_key("paragraph_style") {
            vec!["paragraph_style".to_string()]
        } else {
            Vec::new()
        }
    }
}
