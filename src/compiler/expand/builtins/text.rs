use crate::ast::{
    BinaryExpr, BinaryOp, CallExpr, DistanceUnit, Expr, Ident, Literal, MemberAccessExpr,
};
use crate::compiler::error::CompileError;
use crate::compiler::expand::builtins::{
    apply_inline_origin_defaults, resolve_default_spatial_dimensions, BuiltinPrimitive,
    PrimitiveExpandState,
};
use crate::compiler::expand::context::{EnvEntry, ScopeContext};
use crate::compiler::expand::rewrite::rewrite_expr;
use crate::compiler::expanded::NodeId;

pub struct TextPrimitive;

impl BuiltinPrimitive for TextPrimitive {
    fn name(&self) -> &'static str {
        "Text"
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
            "name".to_string(),
            "path".to_string(),
            "font".to_string(),
            "size".to_string(),
            "weight".to_string(),
            "family".to_string(),
            "line_height".to_string(),
            "baseline".to_string(),
            "start_at".to_string(),
            "end_at".to_string(),
            "text_bounds".to_string(),
            "color".to_string(),
        ]
    }

    fn finalize_ports(&self, state: &mut PrimitiveExpandState<'_, '_>) -> Result<(), CompileError> {
        let elem = state.elem;
        let node_id = state.node_id;
        let ctx = state.ctx;

        for port in &elem.ports {
            if matches!(
                port.name.as_str(),
                "start_at" | "starts_at" | "end_at" | "ends_at"
            ) {
                return Err(CompileError::Custom {
                    message: format!(
                        "Property '{}' is not supported directly on Text; use 'text_bounds: \\TextBounds(start_at: ..., end_at: ...)' or '\\env text_bounds = \\TextBounds(...)' instead",
                        port.name.as_str()
                    ),
                    span: port.name.span,
                });
            }
            if port.name.as_str() == "size" {
                return Err(CompileError::Custom {
                    message: "Property 'size' is not supported directly on Text; use 'font: \\Font(size: ...)' or '\\env font = \\Font(...)' instead".to_string(),
                    span: port.name.span,
                });
            }
        }

        if let Some(Expr::Literal(Literal::String(family, span))) = state.ports.get("family") {
            if !family.is_empty() && !crate::compiler::text::font_family_exists(family) {
                return Err(CompileError::FontFamilyNotFound {
                    family: family.clone(),
                    span: *span,
                });
            }
        }
        if let Some(Expr::Literal(Literal::String(family, span))) = state.ports.get("font") {
            if !family.is_empty() && !crate::compiler::text::font_family_exists(family) {
                return Err(CompileError::FontFamilyNotFound {
                    family: family.clone(),
                    span: *span,
                });
            }
        }

        if !state.ports.contains_key("text_bounds") {
            if let Some(EnvEntry::Bound(env_expr)) = ctx.env_scope.get("text_bounds") {
                let empty_children: [NodeId; 0] = [];
                let enclosing_children_borrow =
                    ctx.enclosing_instantiated_children.map(|c| c.borrow());
                let enclosing_child_ids: &[NodeId] = match &enclosing_children_borrow {
                    Some(b) => b.as_slice(),
                    None => &empty_children,
                };
                let lexical_borrow = ctx.lexical_scope.borrow();
                let scope_ctx = ScopeContext::from_element(
                    node_id,
                    ctx,
                    &state.doc.nodes,
                    &*lexical_borrow,
                    state.node_fonts,
                    enclosing_child_ids,
                );
                if let Ok(rewritten) = rewrite_expr(env_expr, &scope_ctx) {
                    state
                        .ports
                        .insert("text_bounds".to_string(), rewritten.clone());
                    state
                        .authored_ports
                        .insert("text_bounds".to_string(), rewritten);
                }
            }
        }

        if let Some(tb_target) = state.ports.get("text_bounds").cloned() {
            let start_at_from_tb = Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(tb_target.clone()),
                member: Ident::new("start_at", elem.span),
                span: elem.span,
            });
            let end_at_from_tb = Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(tb_target),
                member: Ident::new("end_at", elem.span),
                span: elem.span,
            });
            state
                .ports
                .insert("start_at".to_string(), start_at_from_tb.clone());
            state
                .authored_ports
                .insert("start_at".to_string(), start_at_from_tb);
            state
                .ports
                .insert("end_at".to_string(), end_at_from_tb.clone());
            state
                .authored_ports
                .insert("end_at".to_string(), end_at_from_tb);
        } else {
            let default_start_at = Expr::Literal(Literal::Enum(
                "TextStart".to_string(),
                "Capital".to_string(),
                elem.span,
            ));
            state
                .ports
                .insert("start_at".to_string(), default_start_at.clone());
            state
                .authored_ports
                .insert("start_at".to_string(), default_start_at);
            let default_end_at = Expr::Literal(Literal::Enum(
                "TextEnd".to_string(),
                "Baseline".to_string(),
                elem.span,
            ));
            state
                .ports
                .insert("end_at".to_string(), default_end_at.clone());
            state
                .authored_ports
                .insert("end_at".to_string(), default_end_at);
        }

        if !state.ports.contains_key("size") {
            // Check if this is an inline text span inside a parent Text node
            if let Some(parent) = ctx.parent_id {
                if state
                    .doc
                    .get_node(parent)
                    .is_some_and(|n| n.name == "Text")
                {
                    let parent_ident = Expr::Ident(Ident::new(parent.canonical_name(), elem.span));
                    state.ports.insert(
                        "size".to_string(),
                        Expr::MemberAccess(MemberAccessExpr {
                            target: Box::new(parent_ident),
                            member: Ident::new("size", elem.span),
                            span: elem.span,
                        }),
                    );
                }
            }
        }

        if !state.ports.contains_key("size") {
            return Err(CompileError::MissingPort {
                node: "Text".to_string(),
                port: "font".to_string(),
                span: elem.span,
            });
        }

        if !state.ports.contains_key("line_height") {
            if let Some(&fid) = state.node_fonts.get(&node_id) {
                if state
                    .doc
                    .get_node(fid)
                    .is_some_and(|fn_node| fn_node.authored_ports.contains_key("line_height"))
                {
                    let font_ident = Expr::Ident(Ident::new(fid.canonical_name(), elem.span));
                    state.ports.insert(
                        "line_height".to_string(),
                        Expr::MemberAccess(MemberAccessExpr {
                            target: Box::new(font_ident),
                            member: Ident::new("line_height", elem.span),
                            span: elem.span,
                        }),
                    );
                }
            }
        }

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

        if state.ports.contains_key("baseline")
            && !state.ports.contains_key("y")
            && !state.ports.contains_key("top")
        {
            let y_from_baseline = Expr::Binary(BinaryExpr {
                op: BinaryOp::Sub,
                left: Box::new(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("baseline", elem.span),
                    span: elem.span,
                })),
                right: Box::new(Expr::Call(CallExpr {
                    callee: Ident::new("text_baseline_offset", elem.span),
                    args: vec![
                        size_expr.clone(),
                        weight_expr.clone(),
                        font_expr.clone(),
                        start_at_expr.clone(),
                    ],
                    span: elem.span,
                })),
                span: elem.span,
            });
            state
                .ports
                .insert("y".to_string(), y_from_baseline.clone());
            state
                .authored_ports
                .insert("y".to_string(), y_from_baseline);
        } else {
            let baseline_call = Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("y", elem.span),
                    span: elem.span,
                })),
                right: Box::new(Expr::Call(CallExpr {
                    callee: Ident::new("text_baseline_offset", elem.span),
                    args: vec![
                        size_expr.clone(),
                        weight_expr.clone(),
                        font_expr.clone(),
                        start_at_expr.clone(),
                    ],
                    span: elem.span,
                })),
                span: elem.span,
            });
            state
                .ports
                .entry("baseline".to_string())
                .or_insert(baseline_call);
        }

        apply_inline_origin_defaults(state);

        let mut span_style_args: Vec<Expr> = Vec::new();
        let mut unique_span_nodes: Vec<(NodeId, usize, usize)> = Vec::new();
        let ts_list = state.doc.nodes[node_id.0].text_spans.clone();
        for ts in &ts_list {
            for &nid in ts.node_ids.iter().chain(ts.node_id.iter()) {
                if let Some(entry) = unique_span_nodes.iter_mut().find(|(id, _, _)| *id == nid) {
                    entry.1 = entry.1.min(ts.range.start);
                    entry.2 = entry.2.max(ts.range.end);
                } else {
                    unique_span_nodes.push((nid, ts.range.start, ts.range.end));
                }
            }
            if (!ts.node_ids.is_empty() || ts.node_id.is_some()) && ts.range.start < ts.range.end {
                let ts_start = Expr::Literal(Literal::Integer(ts.range.start as i64, elem.span));
                let ts_end = Expr::Literal(Literal::Integer(ts.range.end as i64, elem.span));
                let fid_opt = ts
                    .node_ids
                    .iter()
                    .copied()
                    .chain(ts.node_id)
                    .find_map(|nid| {
                        state
                            .node_fonts
                            .get(&nid)
                            .copied()
                            .or(state.doc.nodes[nid.0].font)
                    });
                let (ts_size, ts_weight, ts_family, ts_italic) = if let Some(fid) = fid_opt {
                    let f_ident = Expr::Ident(Ident::new(fid.canonical_name(), elem.span));
                    let sz = Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(f_ident.clone()),
                        member: Ident::new("size", elem.span),
                        span: elem.span,
                    });
                    let wt = Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(f_ident.clone()),
                        member: Ident::new("weight", elem.span),
                        span: elem.span,
                    });
                    let fam = Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(f_ident.clone()),
                        member: Ident::new("family", elem.span),
                        span: elem.span,
                    });
                    let it = if state.doc.nodes[fid.0].ports.contains_key("italic") {
                        Expr::MemberAccess(MemberAccessExpr {
                            target: Box::new(f_ident),
                            member: Ident::new("italic", elem.span),
                            span: elem.span,
                        })
                    } else {
                        Expr::Literal(Literal::Bool(false, elem.span))
                    };
                    (sz, wt, fam, it)
                } else {
                    (
                        size_expr.clone(),
                        weight_expr.clone(),
                        font_expr.clone(),
                        Expr::Literal(Literal::Bool(false, elem.span)),
                    )
                };
                let bs_expr = ts
                    .node_ids
                    .iter()
                    .copied()
                    .chain(ts.node_id)
                    .find_map(|nid| {
                        if state.doc.nodes[nid.0].ports.contains_key("baseline_shift") {
                            Some(Expr::MemberAccess(MemberAccessExpr {
                                target: Box::new(Expr::Ident(Ident::new(
                                    nid.canonical_name(),
                                    elem.span,
                                ))),
                                member: Ident::new("baseline_shift", elem.span),
                                span: elem.span,
                            }))
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| {
                        Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span))
                    });

                span_style_args.push(ts_start);
                span_style_args.push(ts_end);
                span_style_args.push(ts_size);
                span_style_args.push(ts_weight);
                span_style_args.push(ts_family);
                span_style_args.push(ts_italic);
                span_style_args.push(bs_expr);
            }
        }

        let ib_list = state.doc.nodes[node_id.0].inline_boxes.clone();
        for ib in &ib_list {
            let ib_ident = Expr::Ident(Ident::new(ib.node_id.canonical_name(), elem.span));
            let ib_first = Expr::Literal(Literal::Integer(-(ib.node_id.0 as i64 + 1), elem.span));
            let ib_index = Expr::Literal(Literal::Integer(ib.index as i64, elem.span));
            let ib_width = Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(ib_ident.clone()),
                member: Ident::new("width", elem.span),
                span: elem.span,
            });
            let ib_height = Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(ib_ident.clone()),
                member: Ident::new("height", elem.span),
                span: elem.span,
            });
            let ib_baseline_offset = if state.doc.nodes[ib.node_id.0]
                .ports
                .contains_key("__baseline_offset")
            {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(ib_ident),
                    member: Ident::new("__baseline_offset", elem.span),
                    span: elem.span,
                })
            } else {
                ib_height.clone()
            };
            let ib_false = Expr::Literal(Literal::Bool(false, elem.span));
            let ib_zero = Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span));

            span_style_args.push(ib_first);
            span_style_args.push(ib_index);
            span_style_args.push(ib_width);
            span_style_args.push(ib_height);
            span_style_args.push(ib_baseline_offset);
            span_style_args.push(ib_false);
            span_style_args.push(ib_zero);
        }

        resolve_default_spatial_dimensions(state, &span_style_args)?;

        let text_content = state
            .doc
            .get_node(node_id)
            .and_then(|n| n.text_content.clone());
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

        if !unique_span_nodes.is_empty() || !state.doc.nodes[node_id.0].inline_boxes.is_empty() {
            let text_x_expr = Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(self_ident.clone()),
                member: Ident::new("x", elem.span),
                span: elem.span,
            });
            let text_y_expr = Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(self_ident.clone()),
                member: Ident::new("y", elem.span),
                span: elem.span,
            });
            let text_width_expr = Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(self_ident.clone()),
                member: Ident::new("width", elem.span),
                span: elem.span,
            });
            let line_height_arg = if state.ports.contains_key("line_height") {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("line_height", elem.span),
                    span: elem.span,
                })
            } else {
                Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span))
            };
            let align_arg = if state.ports.contains_key("align") {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident),
                    member: Ident::new("align", elem.span),
                    span: elem.span,
                })
            } else {
                Expr::Literal(Literal::String(String::new(), elem.span))
            };

            for (span_nid, s_start, s_end) in unique_span_nodes {
                let own_shift = if state.doc.nodes[span_nid.0]
                    .ports
                    .contains_key("baseline_shift")
                {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(
                            span_nid.canonical_name(),
                            elem.span,
                        ))),
                        member: Ident::new("baseline_shift", elem.span),
                        span: elem.span,
                    })
                } else {
                    Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span))
                };
                for metric_name in ["x", "y", "width", "height", "baseline"] {
                    let mut m_args = vec![
                        Expr::Literal(Literal::String(metric_name.to_string(), elem.span)),
                        Expr::Literal(Literal::Integer(s_start as i64, elem.span)),
                        Expr::Literal(Literal::Integer(s_end as i64, elem.span)),
                        own_shift.clone(),
                        text_arg_expr.clone(),
                        text_x_expr.clone(),
                        text_y_expr.clone(),
                        text_width_expr.clone(),
                        size_expr.clone(),
                        weight_expr.clone(),
                        font_expr.clone(),
                        start_at_expr.clone(),
                        line_height_arg.clone(),
                        align_arg.clone(),
                    ];
                    m_args.extend(span_style_args.iter().cloned());
                    let call = Expr::Call(CallExpr {
                        callee: Ident::new("span_metric", elem.span),
                        args: m_args,
                        span: elem.span,
                    });
                    state.doc.nodes[span_nid.0]
                        .ports
                        .insert(metric_name.to_string(), call);
                    state.doc.nodes[span_nid.0]
                        .port_types
                        .insert(metric_name.to_string(), "Distance".to_string());
                }
                let span_self = Expr::Ident(Ident::new(span_nid.canonical_name(), elem.span));
                let span_x = Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(span_self.clone()),
                    member: Ident::new("x", elem.span),
                    span: elem.span,
                });
                let span_y = Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(span_self.clone()),
                    member: Ident::new("y", elem.span),
                    span: elem.span,
                });
                let span_w = Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(span_self.clone()),
                    member: Ident::new("width", elem.span),
                    span: elem.span,
                });
                let span_h = Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(span_self),
                    member: Ident::new("height", elem.span),
                    span: elem.span,
                });
                state.doc.nodes[span_nid.0]
                    .ports
                    .insert("left".to_string(), span_x.clone());
                state.doc.nodes[span_nid.0]
                    .ports
                    .insert("top".to_string(), span_y.clone());
                state.doc.nodes[span_nid.0].ports.insert(
                    "right".to_string(),
                    Expr::Binary(BinaryExpr {
                        op: BinaryOp::Add,
                        left: Box::new(span_x),
                        right: Box::new(span_w),
                        span: elem.span,
                    }),
                );
                state.doc.nodes[span_nid.0].ports.insert(
                    "bottom".to_string(),
                    Expr::Binary(BinaryExpr {
                        op: BinaryOp::Add,
                        left: Box::new(span_y),
                        right: Box::new(span_h),
                        span: elem.span,
                    }),
                );
                for sp in ["left", "top", "right", "bottom"] {
                    state.doc.nodes[span_nid.0]
                        .port_types
                        .insert(sp.to_string(), "Distance".to_string());
                }
            }

            for ib in state.doc.nodes[node_id.0].inline_boxes.clone() {
                let ib_nid = ib.node_id;
                for metric_name in ["x", "y"] {
                    let mut m_args = vec![
                        Expr::Literal(Literal::String(metric_name.to_string(), elem.span)),
                        Expr::Literal(Literal::Integer(-(ib_nid.0 as i64 + 1), elem.span)),
                        Expr::Literal(Literal::Integer(ib.index as i64, elem.span)),
                        Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span)),
                        text_arg_expr.clone(),
                        text_x_expr.clone(),
                        text_y_expr.clone(),
                        text_width_expr.clone(),
                        size_expr.clone(),
                        weight_expr.clone(),
                        font_expr.clone(),
                        start_at_expr.clone(),
                        line_height_arg.clone(),
                        align_arg.clone(),
                    ];
                    m_args.extend(span_style_args.iter().cloned());
                    let call = Expr::Call(CallExpr {
                        callee: Ident::new("span_metric", elem.span),
                        args: m_args,
                        span: elem.span,
                    });
                    state.doc.nodes[ib_nid.0]
                        .ports
                        .insert(metric_name.to_string(), call);
                    state.doc.nodes[ib_nid.0]
                        .port_types
                        .insert(metric_name.to_string(), "Distance".to_string());
                }
                let ib_self = Expr::Ident(Ident::new(ib_nid.canonical_name(), elem.span));
                let ib_x = Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(ib_self.clone()),
                    member: Ident::new("x", elem.span),
                    span: elem.span,
                });
                let ib_y = Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(ib_self.clone()),
                    member: Ident::new("y", elem.span),
                    span: elem.span,
                });
                let ib_w = Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(ib_self.clone()),
                    member: Ident::new("width", elem.span),
                    span: elem.span,
                });
                let ib_h = Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(ib_self),
                    member: Ident::new("height", elem.span),
                    span: elem.span,
                });
                state.doc.nodes[ib_nid.0]
                    .ports
                    .insert("left".to_string(), ib_x.clone());
                state.doc.nodes[ib_nid.0]
                    .ports
                    .insert("top".to_string(), ib_y.clone());
                state.doc.nodes[ib_nid.0].ports.insert(
                    "right".to_string(),
                    Expr::Binary(BinaryExpr {
                        op: BinaryOp::Add,
                        left: Box::new(ib_x),
                        right: Box::new(ib_w),
                        span: elem.span,
                    }),
                );
                state.doc.nodes[ib_nid.0].ports.insert(
                    "bottom".to_string(),
                    Expr::Binary(BinaryExpr {
                        op: BinaryOp::Add,
                        left: Box::new(ib_y),
                        right: Box::new(ib_h),
                        span: elem.span,
                    }),
                );
                for sp in ["left", "top", "right", "bottom"] {
                    state.doc.nodes[ib_nid.0]
                        .port_types
                        .insert(sp.to_string(), "Distance".to_string());
                }
            }
        }

        Ok(())
    }

    fn overload_params(&self, state: &PrimitiveExpandState<'_, '_>) -> Vec<String> {
        if state.is_inline_span {
            Vec::new()
        } else {
            let mut params = vec!["x".to_string()];
            let used_baseline = state.elem.ports.iter().any(|p| p.name.as_str() == "baseline")
                && !state.elem.ports.iter().any(|p| p.name.as_str() == "y");
            params.push(if used_baseline { "baseline" } else { "y" }.to_string());
            params.push("font".to_string());
            params
        }
    }
}
