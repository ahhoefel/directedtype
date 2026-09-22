use crate::ast::*;
use crate::compiler::error::CompileError;
use crate::compiler::expanded::{ExpandedDocument, ExpandedNode, NodeId};
use std::collections::HashMap;

/// Context used when rewriting expressions to bind variables to concrete node IDs.
#[derive(Debug, Clone)]
pub struct ScopeContext<'a> {
    pub current_node: NodeId,
    pub parent_node: Option<NodeId>,
    pub prev_sibling: Option<NodeId>,
    pub child_ids: &'a [NodeId],
    pub parent_ports: &'a [String],
}

/// Expands a parsed AST `Document` into an `ExpandedDocument`.
pub fn expand_document(doc: &Document) -> Result<ExpandedDocument, CompileError> {
    let mut registry = HashMap::new();
    let mut root_elements = Vec::new();

    // 1. Index all component definitions
    for item in &doc.items {
        match item {
            Item::Component(comp) => {
                registry.insert(comp.name.as_str().to_string(), comp.clone());
            }
            Item::Node(node) => {
                root_elements.push(node.clone());
            }
        }
    }

    let mut expanded_doc = ExpandedDocument::new();

    // 2. Expand root elements
    for root_elem in &root_elements {
        let root_id = expand_element(
            root_elem,
            None,
            None,
            &[],
            &registry,
            &mut expanded_doc,
        )?;
        expanded_doc.roots.push(root_id);
    }

    Ok(expanded_doc)
}

struct InstanceContext<'a> {
    pub comp_node_id: NodeId,
    pub parent_id: Option<NodeId>,
    pub prev_sibling_id: Option<NodeId>,
    pub parent_ports: &'a [String],
}

/// Expands a single element node (either a component invocation or a primitive).
fn expand_element(
    elem: &ElementNode,
    parent_id: Option<NodeId>,
    prev_sibling_id: Option<NodeId>,
    parent_ports: &[String],
    registry: &HashMap<String, ComponentDef>,
    doc: &mut ExpandedDocument,
) -> Result<NodeId, CompileError> {
    let node_id = NodeId(doc.nodes.len());
    let mut expanded = ExpandedNode::new(node_id, elem.name.as_str(), elem.span);
    expanded.parent = parent_id;
    expanded.prev_sibling = prev_sibling_id;

    // Check if element has text content directly in content slot
    if let Some(content_slot) = &elem.content {
        let mut text_parts = Vec::new();
        for item in &content_slot.items {
            if let ContentItem::Text(chunk) = item {
                text_parts.push(chunk.text.as_str());
            }
        }
        if !text_parts.is_empty() {
            expanded.text_content = Some(text_parts.join(" "));
        }
    }

    // Allocate placeholder in doc.nodes to reserve index
    doc.nodes.push(expanded);

    // If it's an invocation of a user-defined component:
    if let Some(comp_def) = registry.get(elem.name.as_str()).cloned() {
        let inst_ctx = InstanceContext {
            comp_node_id: node_id,
            parent_id,
            prev_sibling_id,
            parent_ports,
        };
        expand_component_instance(
            &inst_ctx,
            elem,
            &comp_def,
            registry,
            doc,
        )?;
    } else {
        // It's a primitive element (e.g. \Rect, \Text, \Header, etc.)
        expand_primitive_element(
            node_id,
            elem,
            parent_id,
            prev_sibling_id,
            parent_ports,
            registry,
            doc,
        )?;
    }

    Ok(node_id)
}

/// Expands a component instance, wiring parameters, component body, and consumer children.
fn expand_component_instance(
    ctx: &InstanceContext<'_>,
    instance: &ElementNode,
    comp_def: &ComponentDef,
    registry: &HashMap<String, ComponentDef>,
    doc: &mut ExpandedDocument,
) -> Result<(), CompileError> {
    // 1. Gather parameter definitions and map consumer arguments
    let mut comp_scope_ports = vec![
        "x".to_string(),
        "y".to_string(),
        "width".to_string(),
        "height".to_string(),
        "z".to_string(),
    ];
    let mut comp_ports = HashMap::new();

    // Register parameters with default values
    for param in &comp_def.params {
        let name = param.name.as_str().to_string();
        if !comp_scope_ports.contains(&name) {
            comp_scope_ports.push(name.clone());
        }
        if let Some(default_expr) = &param.default_edge {
            comp_ports.insert(name, default_expr.clone());
        }
    }

    // Override with explicit arguments passed to the component
    for port in &instance.ports {
        let name = port.name.as_str().to_string();
        if !comp_scope_ports.contains(&name) {
            comp_scope_ports.push(name.clone());
        }
        comp_ports.insert(name, port.expr.clone());
    }

    // 2. Expand consumer children passed to this component instance
    let mut consumer_child_nodes = Vec::new();
    if let Some(slot) = &instance.content {
        for item in &slot.items {
            match item {
                ContentItem::Node(child_elem) => {
                    consumer_child_nodes.push(child_elem.clone());
                }
                ContentItem::Text(_) | ContentItem::Children(_) => {}
            }
        }
    }

    // 3. Expand component body items in declaration order (Painter's Algorithm)
    let mut all_children_ids = Vec::new();
    let mut instantiated_children_ids = Vec::new();
    let mut last_child_id: Option<NodeId> = None;

    for item in &comp_def.body {
        match item {
            ComponentBodyItem::Node(body_node) => {
                let body_id = expand_element(
                    body_node,
                    Some(ctx.comp_node_id),
                    last_child_id,
                    &comp_scope_ports,
                    registry,
                    doc,
                )?;
                all_children_ids.push(body_id);
                last_child_id = Some(body_id);
            }
            ComponentBodyItem::Children(dir) => {
                for child_elem in &consumer_child_nodes {
                    let mut merged_ports = HashMap::new();

                    // Ambient rules from \Children
                    for ambient in &dir.ports {
                        merged_ports.insert(ambient.name.as_str().to_string(), ambient.expr.clone());
                    }

                    // Explicit child ports override ambient rules (Section 6 Precedence)
                    for explicit in &child_elem.ports {
                        merged_ports.insert(explicit.name.as_str().to_string(), explicit.expr.clone());
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

                    let child_id = expand_element(
                        &wired_elem,
                        Some(ctx.comp_node_id),
                        last_child_id,
                        &comp_scope_ports,
                        registry,
                        doc,
                    )?;

                    instantiated_children_ids.push(child_id);
                    all_children_ids.push(child_id);
                    last_child_id = Some(child_id);
                }
            }
        }
    }

    // 6. Rewrite component ports in scope
    let scope_ctx = ScopeContext {
        current_node: ctx.comp_node_id,
        parent_node: ctx.parent_id,
        prev_sibling: ctx.prev_sibling_id,
        child_ids: &instantiated_children_ids,
        parent_ports: ctx.parent_ports,
    };

    let mut rewritten_ports = HashMap::new();
    for (port_name, port_expr) in comp_ports {
        rewritten_ports.insert(port_name, rewrite_expr(&port_expr, &scope_ctx));
    }

    let node = doc.get_node_mut(ctx.comp_node_id).unwrap();
    node.children = all_children_ids;
    node.ports = rewritten_ports;

    Ok(())
}

/// Expands a primitive element (e.g. \Rect, \Text, \Header, etc.)
fn expand_primitive_element(
    node_id: NodeId,
    elem: &ElementNode,
    parent_id: Option<NodeId>,
    prev_sibling_id: Option<NodeId>,
    parent_ports: &[String],
    registry: &HashMap<String, ComponentDef>,
    doc: &mut ExpandedDocument,
) -> Result<(), CompileError> {
    // 1. Expand nested child nodes in content slot
    let mut child_ids = Vec::new();
    let mut last_child_id = None;

    if let Some(slot) = &elem.content {
        for item in &slot.items {
            if let ContentItem::Node(child_elem) = item {
                let child_id = expand_element(
                    child_elem,
                    Some(node_id),
                    last_child_id,
                    parent_ports,
                    registry,
                    doc,
                )?;
                child_ids.push(child_id);
                last_child_id = Some(child_id);
            }
        }
    }

    // 2. Rewrite element ports
    let ctx = ScopeContext {
        current_node: node_id,
        parent_node: parent_id,
        prev_sibling: prev_sibling_id,
        child_ids: &child_ids,
        parent_ports,
    };

    let mut ports = HashMap::new();
    for port in &elem.ports {
        let rewritten = rewrite_expr(&port.expr, &ctx);
        ports.insert(port.name.as_str().to_string(), rewritten);
    }

    // Base Spatial Trait defaults for height and width
    let text_len = doc
        .get_node(node_id)
        .and_then(|n| n.text_content.as_ref())
        .map_or(0.0, |t| t.len() as f64);
    let has_text = text_len > 0.0;

    // Height defaults
    if !ports.contains_key("height") {
        if elem.name.as_str() == "Text" || (has_text && ports.contains_key("width")) {
            // Text wrapping: height depends on width and text length
            let width_expr = Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(Expr::Ident(Ident::new(node_id.canonical_name(), elem.span))),
                member: Ident::new("width", elem.span),
                span: elem.span,
            });
            let text_len_expr =
                Expr::Literal(Literal::Number((text_len * 8.5).max(24.0), elem.span));
            let div_expr = Expr::Binary(BinaryExpr {
                op: BinaryOp::Div,
                left: Box::new(text_len_expr),
                right: Box::new(width_expr),
                span: elem.span,
            });
            let base_height = if ports.contains_key("size") {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(node_id.canonical_name(), elem.span))),
                    member: Ident::new("size", elem.span),
                    span: elem.span,
                })
            } else {
                Expr::Literal(Literal::Number(20.0, elem.span))
            };
            let mul_expr = Expr::Binary(BinaryExpr {
                op: BinaryOp::Mul,
                left: Box::new(div_expr),
                right: Box::new(base_height.clone()),
                span: elem.span,
            });
            let final_expr = Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(mul_expr),
                right: Box::new(base_height),
                span: elem.span,
            });
            ports.insert("height".to_string(), final_expr);
        } else if ports.contains_key("size") {
            // E.g. \Header(size: 32) -> height: self.size
            ports.insert(
                "height".to_string(),
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(node_id.canonical_name(), elem.span))),
                    member: Ident::new("size", elem.span),
                    span: elem.span,
                }),
            );
        } else {
            ports.insert(
                "height".to_string(),
                Expr::Literal(Literal::Number(24.0, elem.span)),
            );
        }
    }

    // Width defaults
    if !ports.contains_key("width") {
        let default_w = if has_text {
            (text_len * 8.0).max(100.0)
        } else {
            100.0
        };
        ports.insert(
            "width".to_string(),
            Expr::Literal(Literal::Number(default_w, elem.span)),
        );
    }

    let node = doc.get_node_mut(node_id).unwrap();
    node.children = child_ids;
    node.ports = ports;

    Ok(())
}

/// Recursively rewrites an expression into canonical node variable references and resolves derived aliases.
pub fn rewrite_expr(expr: &Expr, ctx: &ScopeContext<'_>) -> Expr {
    match expr {
        Expr::Ident(id) => {
            // Check if it's a port on the parent container in scope
            if ctx.parent_ports.iter().any(|p| p == id.as_str()) {
                if let Some(parent) = ctx.parent_node {
                    return Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(parent.canonical_name(), id.span))),
                        member: id.clone(),
                        span: id.span,
                    });
                }
            }
            if id.as_str() == "self" {
                return Expr::Ident(Ident::new(ctx.current_node.canonical_name(), id.span));
            }
            if id.as_str() == "parent" {
                if let Some(parent) = ctx.parent_node {
                    return Expr::Ident(Ident::new(parent.canonical_name(), id.span));
                }
            }
            if id.as_str() == "prev" {
                if let Some(prev) = ctx.prev_sibling {
                    return Expr::Ident(Ident::new(prev.canonical_name(), id.span));
                }
            }
            expr.clone()
        }

        Expr::Ternary(tern) => {
            // Recurrence resolution for `prev ? then_expr : else_expr`
            if let Expr::Ident(id) = tern.condition.as_ref() {
                if id.as_str() == "prev" {
                    if ctx.prev_sibling.is_none() {
                        // Base case: prev does not exist -> else_expr
                        return rewrite_expr(&tern.else_expr, ctx);
                    } else {
                        // Recurrence step: prev exists -> then_expr
                        return rewrite_expr(&tern.then_expr, ctx);
                    }
                }
            }
            Expr::Ternary(TernaryExpr {
                condition: Box::new(rewrite_expr(&tern.condition, ctx)),
                then_expr: Box::new(rewrite_expr(&tern.then_expr, ctx)),
                else_expr: Box::new(rewrite_expr(&tern.else_expr, ctx)),
                span: tern.span,
            })
        }

        Expr::MemberAccess(m) => {
            let target_ident_name = match m.target.as_ref() {
                Expr::Ident(id) => Some(id.as_str().to_string()),
                _ => None,
            };

            // Resolve target (parent, prev, self)
            let resolved_target = if let Some(target_name) = &target_ident_name {
                if target_name == "parent" {
                    if let Some(parent) = ctx.parent_node {
                        Expr::Ident(Ident::new(parent.canonical_name(), m.target.span()))
                    } else {
                        rewrite_expr(&m.target, ctx)
                    }
                } else if target_name == "prev" {
                    if let Some(prev) = ctx.prev_sibling {
                        Expr::Ident(Ident::new(prev.canonical_name(), m.target.span()))
                    } else {
                        rewrite_expr(&m.target, ctx)
                    }
                } else if target_name == "self" {
                    Expr::Ident(Ident::new(ctx.current_node.canonical_name(), m.target.span()))
                } else {
                    rewrite_expr(&m.target, ctx)
                }
            } else {
                rewrite_expr(&m.target, ctx)
            };

            // Resolve derived spatial aliases:
            // left -> x, top -> y, right -> x + width, bottom -> y + height
            match m.member.as_str() {
                "left" => Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(resolved_target),
                    member: Ident::new("x", m.member.span),
                    span: m.span,
                }),
                "top" => Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(resolved_target),
                    member: Ident::new("y", m.member.span),
                    span: m.span,
                }),
                "right" => {
                    let x = Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(resolved_target.clone()),
                        member: Ident::new("x", m.member.span),
                        span: m.span,
                    });
                    let width = Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(resolved_target),
                        member: Ident::new("width", m.member.span),
                        span: m.span,
                    });
                    Expr::Binary(BinaryExpr {
                        op: BinaryOp::Add,
                        left: Box::new(x),
                        right: Box::new(width),
                        span: m.span,
                    })
                }
                "bottom" => {
                    let y = Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(resolved_target.clone()),
                        member: Ident::new("y", m.member.span),
                        span: m.span,
                    });
                    let height = Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(resolved_target),
                        member: Ident::new("height", m.member.span),
                        span: m.span,
                    });
                    Expr::Binary(BinaryExpr {
                        op: BinaryOp::Add,
                        left: Box::new(y),
                        right: Box::new(height),
                        span: m.span,
                    })
                }
                _ => Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(resolved_target),
                    member: m.member.clone(),
                    span: m.span,
                }),
            }
        }

        Expr::Call(call) => {
            let mut rewritten_args = Vec::new();
            for arg in &call.args {
                if let Expr::MemberAccess(m) = arg {
                    if let Expr::Ident(target_id) = m.target.as_ref() {
                        if target_id.as_str() == "children" {
                            for &child_id in ctx.child_ids {
                                let child_access = Expr::MemberAccess(MemberAccessExpr {
                                    target: Box::new(Expr::Ident(Ident::new(
                                        child_id.canonical_name(),
                                        target_id.span,
                                    ))),
                                    member: m.member.clone(),
                                    span: m.span,
                                });
                                rewritten_args.push(child_access);
                            }
                            continue;
                        }
                    }
                }
                rewritten_args.push(rewrite_expr(arg, ctx));
            }

            if rewritten_args.is_empty() && !ctx.child_ids.is_empty() {
                return Expr::Literal(Literal::Number(0.0, call.span));
            }

            Expr::Call(CallExpr {
                callee: call.callee.clone(),
                args: rewritten_args,
                span: call.span,
            })
        }

        Expr::Binary(bin) => Expr::Binary(BinaryExpr {
            op: bin.op,
            left: Box::new(rewrite_expr(&bin.left, ctx)),
            right: Box::new(rewrite_expr(&bin.right, ctx)),
            span: bin.span,
        }),

        Expr::Unary(u) => Expr::Unary(UnaryExpr {
            op: u.op,
            operand: Box::new(rewrite_expr(&u.operand, ctx)),
            span: u.span,
        }),

        Expr::Paren(inner, span) => {
            Expr::Paren(Box::new(rewrite_expr(inner, ctx)), *span)
        }

        Expr::Literal(_) => expr.clone(),
    }
}
