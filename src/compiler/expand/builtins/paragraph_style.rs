use crate::ast::{ComponentDef, DistanceUnit, ElementNode, Expr, Ident, Literal, MemberAccessExpr};
use crate::compiler::error::CompileError;
use crate::compiler::expand::builtins::{
    default_builtin_port_type, BuiltinPrimitive, PrimitiveExpandState,
};
use crate::compiler::expand::context::ElementContext;
use crate::compiler::expand::typecheck::StaticType;
use crate::compiler::expanded::{ExpandedDocument, NodeId};
use crate::compiler::scope::ScopeId;
use std::collections::HashMap;

pub struct ParagraphStylePrimitive;

impl BuiltinPrimitive for ParagraphStylePrimitive {
    fn name(&self) -> &'static str {
        "ParagraphStyle"
    }

    fn is_non_spatial(&self) -> bool {
        true
    }

    fn accepts_port(&self, port_name: &str) -> bool {
        matches!(port_name, "paragraph_style" | "align" | "indent")
    }

    fn port_type(&self, port_name: &str) -> Option<StaticType> {
        match port_name {
            "paragraph_style" => Some(StaticType::Node("ParagraphStyle".to_string())),
            "align" => Some(StaticType::Enum("Align".to_string())),
            "indent" => Some(StaticType::Distance),
            _ => default_builtin_port_type(port_name),
        }
    }

    fn scope_ports(&self) -> Vec<String> {
        vec!["align".to_string(), "indent".to_string()]
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

            if !state.ports.contains_key("align") {
                state.ports.insert(
                    "align".to_string(),
                    Expr::MemberAccess(MemberAccessExpr {
                        target: std::sync::Arc::new(base_expr.clone()),
                        member: Ident::new("align", span),
                        span,
                    }),
                );
            }
            if !state.ports.contains_key("indent") {
                state.ports.insert(
                    "indent".to_string(),
                    Expr::MemberAccess(MemberAccessExpr {
                        target: std::sync::Arc::new(base_expr),
                        member: Ident::new("indent", span),
                        span,
                    }),
                );
            }
        } else {
            state.ports.entry("align".to_string()).or_insert_with(|| {
                Expr::Literal(Literal::Enum(
                    "Align".to_string(),
                    "Left".to_string(),
                    span,
                ))
            });
            state.ports.entry("indent".to_string()).or_insert_with(|| {
                Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span))
            });
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
