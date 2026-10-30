use crate::ast::{
    DistanceUnit, ElementNode, Expr, Ident, Literal, MemberAccessExpr, PortBinding,
};
use crate::compiler::error::CompileError;
use crate::compiler::expand::builtins::{BuiltinPrimitive, PrimitiveExpandState};
use crate::compiler::expand::context::ElementContext;
use crate::compiler::expanded::{ExpandedDocument, NodeId};
use crate::compiler::scope::ScopeId;

pub struct AnchorPrimitive;
pub struct AnchorScopePrimitive;

/// Wires default `x = parent.left` and `y = parent.top` onto a child element of an `\Anchor` or `\AnchorScope`.
pub(super) fn wire_wrapper_child_origin(node_id: NodeId, child_elem: &ElementNode) -> ElementNode {
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

fn finalize_anchor_like_geometry(state: &mut PrimitiveExpandState<'_, '_>) {
    let elem = state.elem;
    let node_id = state.node_id;
    let self_ident = Expr::Ident(Ident::new(node_id.canonical_name(), elem.span));

    if state.child_ids.len() == 1 {
        let cid = state.child_ids[0];
        let cid_ident = Expr::Ident(Ident::new(cid.canonical_name(), elem.span));

        // Child origin defaults to Anchor origin
        state.doc.nodes[cid.0]
            .ports
            .entry("x".to_string())
            .or_insert_with(|| {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("x", elem.span),
                    span: elem.span,
                })
            });
        state.doc.nodes[cid.0]
            .ports
            .entry("y".to_string())
            .or_insert_with(|| {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("y", elem.span),
                    span: elem.span,
                })
            });

        // Anchor dimensions inherit from Child dimensions if not explicitly set
        if !state.ports.contains_key("width") {
            state.ports.insert(
                "width".to_string(),
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(cid_ident.clone()),
                    member: Ident::new("width", elem.span),
                    span: elem.span,
                }),
            );
        } else {
            state.doc.nodes[cid.0]
                .ports
                .entry("width".to_string())
                .or_insert_with(|| {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(self_ident.clone()),
                        member: Ident::new("width", elem.span),
                        span: elem.span,
                    })
                });
        }

        if !state.ports.contains_key("height") {
            state.ports.insert(
                "height".to_string(),
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(cid_ident),
                    member: Ident::new("height", elem.span),
                    span: elem.span,
                }),
            );
        } else {
            state.doc.nodes[cid.0]
                .ports
                .entry("height".to_string())
                .or_insert_with(|| {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(self_ident),
                        member: Ident::new("height", elem.span),
                        span: elem.span,
                    })
                });
        }
    } else if state.child_ids.is_empty() {
        state
            .ports
            .entry("x".to_string())
            .or_insert_with(|| Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span)));
        state
            .ports
            .entry("y".to_string())
            .or_insert_with(|| Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span)));
        state
            .ports
            .entry("width".to_string())
            .or_insert_with(|| Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span)));
        state
            .ports
            .entry("height".to_string())
            .or_insert_with(|| Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span)));
    } else {
        for &cid in &state.child_ids {
            state.doc.nodes[cid.0]
                .ports
                .entry("x".to_string())
                .or_insert_with(|| {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(self_ident.clone()),
                        member: Ident::new("x", elem.span),
                        span: elem.span,
                    })
                });
            state.doc.nodes[cid.0]
                .ports
                .entry("y".to_string())
                .or_insert_with(|| {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(self_ident.clone()),
                        member: Ident::new("y", elem.span),
                        span: elem.span,
                    })
                });
        }
        if !state.ports.contains_key("width") {
            let first_cid = state.child_ids[0];
            state.ports.insert(
                "width".to_string(),
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(
                        first_cid.canonical_name(),
                        elem.span,
                    ))),
                    member: Ident::new("width", elem.span),
                    span: elem.span,
                }),
            );
        }
        if !state.ports.contains_key("height") {
            let first_cid = state.child_ids[0];
            state.ports.insert(
                "height".to_string(),
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(
                        first_cid.canonical_name(),
                        elem.span,
                    ))),
                    member: Ident::new("height", elem.span),
                    span: elem.span,
                }),
            );
        }
    }

    state
        .ports
        .entry("z".to_string())
        .or_insert_with(|| Expr::Literal(Literal::Integer(0, elem.span)));
}

impl BuiltinPrimitive for AnchorPrimitive {
    fn name(&self) -> &'static str {
        "Anchor"
    }

    fn wires_child_origin_to_self(&self) -> bool {
        true
    }

    fn pre_expand_children(
        &self,
        node_id: NodeId,
        elem: &ElementNode,
        ctx: &ElementContext<'_>,
        doc: &mut ExpandedDocument,
    ) -> Result<ScopeId, CompileError> {
        let anchor_name = elem
            .ports
            .iter()
            .find(|p| p.name.as_str() == "name")
            .and_then(|p| match &p.expr {
                Expr::Literal(Literal::String(s, _)) => Some(s.clone()),
                Expr::Ident(id) => Some(id.as_str().to_string()),
                _ => None,
            });

        let name_str = match anchor_name {
            Some(n) => n,
            None => {
                return Err(CompileError::MissingPort {
                    node: "Anchor".to_string(),
                    port: "name".to_string(),
                    span: elem.span,
                });
            }
        };

        doc.scope_tree
            .register_anchor(ctx.active_scope_id, &name_str, node_id, elem.span)?;
        doc.nodes[node_id.0].anchor_name = Some(name_str);
        Ok(ctx.active_scope_id)
    }

    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        if let Some(ref a_name) = state.doc.nodes[state.node_id.0].anchor_name {
            state.ports.insert(
                "name".to_string(),
                Expr::Literal(Literal::String(a_name.clone(), state.elem.span)),
            );
        }
        finalize_anchor_like_geometry(state);
        Ok(())
    }

    fn overload_params(&self, _state: &PrimitiveExpandState<'_, '_>) -> Vec<String> {
        vec!["name".to_string()]
    }
}

impl BuiltinPrimitive for AnchorScopePrimitive {
    fn name(&self) -> &'static str {
        "AnchorScope"
    }

    fn wires_child_origin_to_self(&self) -> bool {
        true
    }

    fn pre_expand_children(
        &self,
        node_id: NodeId,
        elem: &ElementNode,
        ctx: &ElementContext<'_>,
        doc: &mut ExpandedDocument,
    ) -> Result<ScopeId, CompileError> {
        let scope_name = elem
            .ports
            .iter()
            .find(|p| p.name.as_str() == "name")
            .and_then(|p| match &p.expr {
                Expr::Literal(Literal::String(s, _)) => Some(s.clone()),
                Expr::Ident(id) => Some(id.as_str().to_string()),
                _ => None,
            });

        let name_str = match scope_name {
            Some(n) => n,
            None => {
                return Err(CompileError::MissingPort {
                    node: "AnchorScope".to_string(),
                    port: "name".to_string(),
                    span: elem.span,
                });
            }
        };

        let new_scope_id = doc.scope_tree.add_scope(
            &name_str,
            ctx.active_scope_id,
            Some(node_id),
            elem.span,
        )?;
        doc.nodes[node_id.0].scope_id = Some(new_scope_id);
        doc.scope_tree.node_to_scope.insert(node_id, new_scope_id);
        Ok(new_scope_id)
    }

    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        let canonical_path =
            state.doc.scope_tree.scopes[state.active_scope_for_children.0]
                .canonical_path
                .clone();
        let name_str = state.doc.scope_tree.scopes[state.active_scope_for_children.0]
            .name
            .clone();
        state.ports.insert(
            "name".to_string(),
            Expr::Literal(Literal::String(name_str, state.elem.span)),
        );
        state.ports.insert(
            "path".to_string(),
            Expr::Literal(Literal::String(canonical_path, state.elem.span)),
        );
        finalize_anchor_like_geometry(state);
        Ok(())
    }

    fn overload_params(&self, _state: &PrimitiveExpandState<'_, '_>) -> Vec<String> {
        vec!["name".to_string()]
    }
}

