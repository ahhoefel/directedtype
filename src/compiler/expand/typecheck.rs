use crate::ast::*;
use crate::compiler::error::CompileError;
use crate::compiler::expand::context::{LexicalBinding, ScopeContext};
use crate::compiler::expanded::{ExpandedDocument, NodeId};
use crate::span::Span;
use std::collections::{HashMap, HashSet};

/// Static compile-time type of a property, port, or expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaticType {
    Integer,
    ZeroInteger,
    Float,
    Distance,
    Area,
    Duration,
    String,
    Boolean,
    Color,
    Handler,
    Timer,
    IndefiniteTimer,
    Unset,
    ViewPort,
    Enum(String),
    Node(String),
}

impl StaticType {
    pub fn canonical_name(&self) -> String {
        match self {
            StaticType::Integer | StaticType::ZeroInteger => "Integer".to_string(),
            StaticType::Float => "Float".to_string(),
            StaticType::Distance => "Distance".to_string(),
            StaticType::Area => "Area".to_string(),
            StaticType::Duration => "Duration".to_string(),
            StaticType::String => "String".to_string(),
            StaticType::Boolean => "Boolean".to_string(),
            StaticType::Color => "Color".to_string(),
            StaticType::Handler | StaticType::Unset => "Handler".to_string(),
            StaticType::Timer => "Timer".to_string(),
            StaticType::IndefiniteTimer => "IndefiniteTimer".to_string(),
            StaticType::ViewPort => "ViewPort".to_string(),
            StaticType::Enum(name) => name.clone(),
            StaticType::Node(name) => name.clone(),
        }
    }

    pub fn type_name(&self) -> String {
        self.canonical_name()
    }

    pub fn decay(&self) -> StaticType {
        self.clone().into_property_type()
    }

    pub fn into_property_type(self) -> StaticType {
        match self {
            StaticType::ZeroInteger => StaticType::Integer,
            StaticType::Unset => StaticType::Handler,
            other => other,
        }
    }

    pub fn from_type_name(
        name: &str,
        span: Span,
        enums: &HashMap<String, EnumDef>,
        registry: &HashMap<String, Vec<ComponentDef>>,
    ) -> Result<Self, CompileError> {
        match name {
            "Integer" => Ok(StaticType::Integer),
            "Float" => Ok(StaticType::Float),
            "Distance" => Ok(StaticType::Distance),
            "Area" => Ok(StaticType::Area),
            "Duration" => Ok(StaticType::Duration),
            "String" => Ok(StaticType::String),
            "Boolean" => Ok(StaticType::Boolean),
            "Color" => Ok(StaticType::Color),
            "Handler" => Ok(StaticType::Handler),
            "Timer" => Ok(StaticType::Timer),
            "IndefiniteTimer" => Ok(StaticType::IndefiniteTimer),
            "ViewPort" => Ok(StaticType::ViewPort),
            "Node" => Ok(StaticType::Node("Node".to_string())),
            "Window" | "Rect" | "Line" | "Text" | "Font" | "Clip" | "Box" | "Anchor" | "AnchorScope" => {
                Ok(StaticType::Node(name.to_string()))
            }
            "Number" => Err(CompileError::Custom {
                message: "Type 'Number' has been removed; use 'Float', 'Integer', or 'Distance' instead"
                    .to_string(),
                span,
            }),
            _ => {
                if enums.contains_key(name) {
                    Ok(StaticType::Enum(name.to_string()))
                } else if registry.contains_key(name)
                    || registry
                        .keys()
                        .any(|k| crate::compiler::graph::unmangle_component_name(k) == name)
                {
                    Ok(StaticType::Node(name.to_string()))
                } else {
                    Err(CompileError::Custom {
                        message: format!("Unknown type '{}'", name),
                        span,
                    })
                }
            }
        }
    }

    pub fn matches(&self, expected: &StaticType) -> bool {
        if self == expected {
            return true;
        }
        match (self, expected) {
            (
                StaticType::ZeroInteger,
                StaticType::Integer
                | StaticType::Float
                | StaticType::Distance
                | StaticType::Area
                | StaticType::Duration,
            ) => true,
            (StaticType::Integer, StaticType::ZeroInteger) => true,
            (StaticType::Integer, StaticType::Float) => true,
            (
                StaticType::Unset,
                StaticType::Handler | StaticType::Timer | StaticType::IndefiniteTimer,
            ) => true,
            (
                StaticType::Handler | StaticType::Timer | StaticType::IndefiniteTimer,
                StaticType::Unset,
            ) => true,
            (StaticType::Node(n), StaticType::ViewPort)
                if n == "Window" || n == "ScrollView" || n == "Node" =>
            {
                true
            }
            (StaticType::Node(n), StaticType::Node(exp))
                if exp == "Clip" && (n == "Window" || n == "Node") =>
            {
                true
            }
            (StaticType::Node(_), StaticType::Node(exp)) if exp == "Node" => true,
            (StaticType::ViewPort, StaticType::Node(exp))
                if exp == "Node" || exp == "Window" || exp == "ScrollView" =>
            {
                true
            }
            (StaticType::Node(n), StaticType::Node(_)) if n == "Node" => true,
            _ => false,
        }
    }
}

pub(super) fn validate_content_items_type_annotations(
    items: &[ContentItem],
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> Result<(), CompileError> {
    for item in items {
        match item {
            ContentItem::Let(l) => {
                if let Some(ty) = &l.type_annotation {
                    StaticType::from_type_name(ty.name.as_str(), ty.span, enums, registry)?;
                }
                if let Some(LetValue::Node(n)) = &l.value {
                    if let Some(slot) = &n.content {
                        validate_content_items_type_annotations(&slot.items, enums, registry)?;
                    }
                }
            }
            ContentItem::Alias(a) => {
                if let Some(ty) = &a.type_annotation {
                    StaticType::from_type_name(ty.name.as_str(), ty.span, enums, registry)?;
                }
            }
            ContentItem::State(s) => {
                if let Some(ty) = &s.type_annotation {
                    StaticType::from_type_name(ty.name.as_str(), ty.span, enums, registry)?;
                }
            }
            ContentItem::Env(e) => {
                if let Some(ty) = &e.type_annotation {
                    StaticType::from_type_name(ty.name.as_str(), ty.span, enums, registry)?;
                }
            }
            ContentItem::Node(n) => {
                if let Some(slot) = &n.content {
                    validate_content_items_type_annotations(&slot.items, enums, registry)?;
                }
            }
            ContentItem::Children(_)
            | ContentItem::Text(_)
            | ContentItem::ParagraphBreak(_)
            | ContentItem::Assert(_) => {}
        }
    }
    Ok(())
}

pub(super) fn coerce_typed_binding_expr(mut expr: Expr, type_annotation: Option<&TypeRef>) -> Expr {
    if let Some(ty) = type_annotation {
        match ty.name.as_str() {
            "Distance" => {
                if let Expr::Literal(Literal::Integer(0, span)) = expr {
                    expr = Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span));
                }
            }
            "Area" => {
                if let Expr::Literal(Literal::Integer(0, span)) = expr {
                    expr = Expr::Literal(Literal::Area(0.0, DistanceUnit::Px, span));
                }
            }
            "Duration" => {
                if let Expr::Literal(Literal::Integer(0, span)) = expr {
                    expr = Expr::Literal(Literal::Duration(0, DurationUnit::S, span));
                }
            }
            "Float" => {
                if let Expr::Literal(Literal::Integer(i, span)) = expr {
                    expr = Expr::Literal(Literal::Float(i as f64, span));
                }
            }
            "Integer" => {
                if let Expr::Literal(Literal::Integer(0, span)) = expr {
                    expr = Expr::Call(CallExpr {
                        callee: Ident::new("to_integer", span),
                        args: vec![Expr::Literal(Literal::Integer(0, span))],
                        span,
                    });
                }
            }
            _ => {}
        }
    } else if let Expr::Literal(Literal::Integer(0, span)) = expr {
        expr = Expr::Call(CallExpr {
            callee: Ident::new("to_integer", span),
            args: vec![Expr::Literal(Literal::Integer(0, span))],
            span,
        });
    }
    expr
}

pub(super) fn validate_node_type_annotation(
    elem_name: &str,
    elem_span: Span,
    type_annotation: Option<&TypeRef>,
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> Result<(), CompileError> {
    if let Some(ty_ref) = type_annotation {
        let expected =
            StaticType::from_type_name(ty_ref.name.as_str(), ty_ref.span, enums, registry)?;
        let actual_name = crate::compiler::graph::unmangle_component_name(elem_name);
        let actual = StaticType::Node(actual_name.to_string());
        if !actual.matches(&expected) {
            return Err(CompileError::TypeMismatch {
                expected: expected.canonical_name(),
                actual: actual.canonical_name(),
                span: elem_span,
            });
        }
    }
    Ok(())
}

/// Validates and coerces a literal or constant expression on a built-in primitive port.
pub(super) fn validate_and_coerce_primitive_port_literal(
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
            | "width"
            | "height"
            | "size"
            | "line_height"
            | "baseline"
            | "radius"
            | "radius_left"
            | "radius_right"
            | "radius_top"
            | "radius_bottom"
            | "radius_top_left"
            | "radius_top_right"
            | "radius_bottom_right"
            | "radius_bottom_left"
            | "border_width"
            | "thickness"
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

/// Validates component parameter types and coerces numeric literals (`0 -> 0px`, `Integer -> Float`).
pub(super) fn validate_and_coerce_component_param_types(
    comp_def: &ComponentDef,
    explicit_unlowered_ports: &HashMap<String, Expr>,
    scope_ctx: &ScopeContext<'_>,
    rewritten_ports: &mut HashMap<String, Expr>,
) -> Result<(), CompileError> {
    for param in &comp_def.params {
        if let Some(type_ref) = &param.type_annotation {
            let expected_type = type_ref.name.as_str();
            let name = param.name.as_str();
            let unlowered_lit = explicit_unlowered_ports
                .get(name)
                .or(param.default_edge.as_ref())
                .and_then(|e| match e {
                    Expr::Literal(lit) => Some(lit.clone()),
                    _ => None,
                });
            if let Some(expr) = rewritten_ports.get_mut(name) {
                if let Some(lit) =
                    unlowered_lit.or_else(|| infer_expr_literal_for_typecheck(expr, scope_ctx))
                {
                    if !lit.matches_type_name(expected_type) {
                        return Err(CompileError::TypeMismatch {
                            expected: expected_type.to_string(),
                            actual: lit.type_name().to_string(),
                            span: expr.span(),
                        });
                    }
                }
                if expected_type == "Distance" {
                    if let Expr::Literal(Literal::Integer(0, span)) = expr {
                        *expr = Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, *span));
                    }
                } else if expected_type == "Area" {
                    if let Expr::Literal(Literal::Integer(0, span)) = expr {
                        *expr = Expr::Literal(Literal::Area(0.0, DistanceUnit::Px, *span));
                    }
                } else if expected_type == "Duration" {
                    if let Expr::Literal(Literal::Integer(0, span)) = expr {
                        *expr = Expr::Literal(Literal::Duration(0, DurationUnit::S, *span));
                    }
                } else if expected_type == "Float" {
                    if let Expr::Literal(Literal::Integer(i, span)) = expr {
                        *expr = Expr::Literal(Literal::Float(*i as f64, *span));
                    }
                } else if expected_type == "Integer" {
                    if let Expr::Literal(Literal::Integer(0, span)) = expr {
                        *expr = Expr::Call(CallExpr {
                            callee: Ident::new("to_integer", *span),
                            args: vec![Expr::Literal(Literal::Integer(0, *span))],
                            span: *span,
                        });
                    }
                }
            }
        }
    }
    Ok(())
}


pub(super) fn infer_expr_literal_for_typecheck<'a>(
    expr: &'a Expr,
    ctx: &ScopeContext<'a>,
) -> Option<Literal> {
    if let Some(lit) = resolve_ident_or_member_to_literal(expr, ctx) {
        return Some(lit);
    }
    match expr {
        Expr::Paren(inner, _) => infer_expr_literal_for_typecheck(inner, ctx),
        Expr::Unary(u) if u.op == UnaryOp::Neg => {
            match infer_expr_literal_for_typecheck(&u.operand, ctx)? {
                Literal::Integer(i, _) => Some(Literal::Integer(-i, u.span)),
                Literal::Float(f, _) => Some(Literal::Float(-f, u.span)),
                Literal::Distance(d, unit, _) => Some(Literal::Distance(-d, unit, u.span)),
                Literal::Area(a, unit, _) => Some(Literal::Area(-a, unit, u.span)),
                Literal::Duration(d, unit, _) => Some(Literal::Duration(-d, unit, u.span)),
                _ => None,
            }
        }
        Expr::MemberAccess(m) => {
            let member = m.member.as_str();
            if matches!(
                member,
                "size"
                    | "width"
                    | "height"
                    | "x"
                    | "y"
                    | "x1"
                    | "y1"
                    | "x2"
                    | "y2"
                    | "thickness"
                    | "head_length"
                    | "head_width"
                    | "length"
                    | "left"
                    | "top"
                    | "right"
                    | "bottom"
                    | "baseline"
                    | "cap_height"
                    | "x_height"
                    | "ascent"
                    | "descent"
                    | "em_top"
                    | "line_gap"
                    | "gap_bottom"
                    | "line_height"
                    | "radius"
                    | "radius_left"
                    | "radius_right"
                    | "radius_top"
                    | "radius_bottom"
                    | "radius_top_left"
                    | "radius_top_right"
                    | "radius_bottom_right"
                    | "radius_bottom_left"
                    | "border_width"
                    | "padding"
                    | "padding_x"
                    | "padding_y"
            ) {
                return Some(Literal::Distance(0.0, DistanceUnit::Px, m.span));
            }
            if member == "dpi" || member == "elapsed" {
                return Some(Literal::Float(96.0, m.span));
            }
            if matches!(member, "time" | "duration" | "dt") {
                return Some(Literal::Duration(0, DurationUnit::S, m.span));
            }
            if member == "weight" || member == "z" || member == "len" || member == "frames" {
                return Some(Literal::Integer(1, m.span));
            }
            None
        }
        Expr::Call(call) => {
            if call.callee.as_str() == "sqrt" {
                return match call
                    .args
                    .first()
                    .and_then(|a| infer_expr_literal_for_typecheck(a, ctx))?
                {
                    Literal::Area(_, _, _) => {
                        Some(Literal::Distance(0.0, DistanceUnit::Px, call.span))
                    }
                    _ => Some(Literal::Float(0.0, call.span)),
                };
            }
            if matches!(
                call.callee.as_str(),
                "distance_to_float"
                    | "to_px"
                    | "to_pt"
                    | "to_float"
                    | "to_number"
                    | "sin"
                    | "cos"
                    | "tan"
                    | "atan2"
            ) {
                return Some(Literal::Float(0.0, call.span));
            }
            if call.callee.as_str() == "to_integer" {
                return Some(Literal::Integer(1, call.span));
            }
            if call.callee.as_str() == "hypot" {
                if let Some(first) = call
                    .args
                    .first()
                    .and_then(|a| infer_expr_literal_for_typecheck(a, ctx))
                {
                    if matches!(first, Literal::Distance(_, _, _)) {
                        return Some(Literal::Distance(0.0, DistanceUnit::Px, call.span));
                    }
                }
                return Some(Literal::Float(0.0, call.span));
            }
            if matches!(
                call.callee.as_str(),
                "text_width"
                    | "text_height"
                    | "text_baseline_offset"
                    | "span_metric"
                    | "font_cap_height"
                    | "font_x_height"
                    | "font_ascent"
                    | "font_descent"
                    | "font_line_gap"
                    | "font_line_height"
            ) {
                return Some(Literal::Distance(0.0, DistanceUnit::Px, call.span));
            }
            if matches!(
                call.callee.as_str(),
                "only" | "max" | "min" | "sum" | "clamp" | "abs" | "round" | "floor" | "ceil"
            ) {
                return call
                    .args
                    .first()
                    .and_then(|a| infer_expr_literal_for_typecheck(a, ctx));
            }
            None
        }
        Expr::MethodCall(mc) => {
            if matches!(
                mc.method.as_str(),
                "to_px" | "to_pt" | "to_seconds" | "to_ms" | "to_float" | "to_number"
            ) {
                return Some(Literal::Float(0.0, mc.span));
            }
            if mc.method.as_str() == "to_integer" {
                return Some(Literal::Integer(1, mc.span));
            }
            None
        }
        Expr::Binary(bin) => {
            let left = infer_expr_literal_for_typecheck(&bin.left, ctx)?;
            let right = infer_expr_literal_for_typecheck(&bin.right, ctx)?;
            match bin.op {
                BinaryOp::Add | BinaryOp::Sub => match (&left, &right) {
                    (Literal::Distance(_, _, _), Literal::Distance(_, _, _))
                    | (Literal::Distance(_, _, _), Literal::Integer(0, _))
                    | (Literal::Integer(0, _), Literal::Distance(_, _, _)) => {
                        Some(Literal::Distance(0.0, DistanceUnit::Px, bin.span))
                    }
                    (Literal::Area(_, _, _), Literal::Area(_, _, _))
                    | (Literal::Area(_, _, _), Literal::Integer(0, _))
                    | (Literal::Integer(0, _), Literal::Area(_, _, _)) => {
                        Some(Literal::Area(0.0, DistanceUnit::Px, bin.span))
                    }
                    (Literal::Duration(_, _, _), Literal::Duration(_, _, _))
                    | (Literal::Duration(_, _, _), Literal::Integer(0, _))
                    | (Literal::Integer(0, _), Literal::Duration(_, _, _)) => {
                        Some(Literal::Duration(0, DurationUnit::S, bin.span))
                    }
                    (Literal::Integer(_, _), Literal::Integer(_, _)) => {
                        Some(Literal::Integer(1, bin.span))
                    }
                    (Literal::Float(_, _), Literal::Float(_, _))
                    | (Literal::Float(_, _), Literal::Integer(_, _))
                    | (Literal::Integer(_, _), Literal::Float(_, _)) => {
                        Some(Literal::Float(0.0, bin.span))
                    }
                    _ => None,
                },
                BinaryOp::Mul => match (&left, &right) {
                    (Literal::Distance(_, _, _), Literal::Distance(_, _, _)) => {
                        Some(Literal::Area(0.0, DistanceUnit::Px, bin.span))
                    }
                    (Literal::Distance(_, _, _), Literal::Integer(_, _) | Literal::Float(_, _))
                    | (Literal::Integer(_, _) | Literal::Float(_, _), Literal::Distance(_, _, _)) => {
                        Some(Literal::Distance(0.0, DistanceUnit::Px, bin.span))
                    }
                    (Literal::Area(_, _, _), Literal::Integer(_, _) | Literal::Float(_, _))
                    | (Literal::Integer(_, _) | Literal::Float(_, _), Literal::Area(_, _, _)) => {
                        Some(Literal::Area(0.0, DistanceUnit::Px, bin.span))
                    }
                    (Literal::Duration(_, _, _), Literal::Integer(_, _) | Literal::Float(_, _))
                    | (Literal::Integer(_, _) | Literal::Float(_, _), Literal::Duration(_, _, _)) => {
                        Some(Literal::Duration(0, DurationUnit::S, bin.span))
                    }
                    (Literal::Integer(_, _), Literal::Integer(_, _)) => {
                        Some(Literal::Integer(1, bin.span))
                    }
                    (Literal::Float(_, _), Literal::Float(_, _))
                    | (Literal::Float(_, _), Literal::Integer(_, _))
                    | (Literal::Integer(_, _), Literal::Float(_, _)) => {
                        Some(Literal::Float(0.0, bin.span))
                    }
                    _ => None,
                },
                BinaryOp::Div => match (&left, &right) {
                    (Literal::Distance(_, _, _), Literal::Distance(_, _, _))
                    | (Literal::Area(_, _, _), Literal::Area(_, _, _))
                    | (Literal::Duration(_, _, _), Literal::Duration(_, _, _)) => {
                        Some(Literal::Float(0.0, bin.span))
                    }
                    (Literal::Area(_, _, _), Literal::Distance(_, _, _))
                    | (Literal::Distance(_, _, _), Literal::Integer(_, _) | Literal::Float(_, _)) => {
                        Some(Literal::Distance(0.0, DistanceUnit::Px, bin.span))
                    }
                    (Literal::Area(_, _, _), Literal::Integer(_, _) | Literal::Float(_, _)) => {
                        Some(Literal::Area(0.0, DistanceUnit::Px, bin.span))
                    }
                    (Literal::Duration(_, _, _), Literal::Integer(_, _) | Literal::Float(_, _)) => {
                        Some(Literal::Duration(0, DurationUnit::S, bin.span))
                    }
                    (
                        Literal::Integer(_, _) | Literal::Float(_, _),
                        Literal::Integer(_, _) | Literal::Float(_, _),
                    ) => Some(Literal::Float(0.0, bin.span)),
                    _ => None,
                },
                _ => None,
            }
        }
        Expr::Ternary(tern) => infer_expr_literal_for_typecheck(&tern.then_expr, ctx)
            .or_else(|| infer_expr_literal_for_typecheck(&tern.else_expr, ctx)),
        _ => None,
    }
}

fn is_handler_expr_unset(expr: &Expr, ctx: &ScopeContext<'_>) -> Option<bool> {
    match expr {
        Expr::Paren(inner, _) => is_handler_expr_unset(inner, ctx),
        Expr::Ident(id) => {
            if id.as_str() == "unset" || id.as_str() == "None" {
                return Some(true);
            }
            if ctx
                .declared_state_names
                .is_some_and(|s| s.contains(id.as_str()))
            {
                return None;
            }
            if let Some(comp_ports) = ctx.comp_ports {
                if let Some(port_expr) = comp_ports.get(id.as_str()) {
                    return match port_expr {
                        Expr::Ident(pid) if pid.as_str() == "unset" || pid.as_str() == "None" => {
                            Some(true)
                        }
                        Expr::MemberAccess(m) => {
                            if let Expr::Ident(target_id) = m.target.as_ref() {
                                if ctx.enums.contains_key(target_id.as_str()) {
                                    return None;
                                }
                                let is_self = target_id.name == Symbol::SELF
                                    || target_id.node_id() == Some(ctx.current_node);
                                Some(is_self)
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                }
            }
            match ctx.lexical_scope.get(&id.name) {
                Some(LexicalBinding::Expr(e)) => return is_handler_expr_unset(e, ctx),
                Some(LexicalBinding::Member {
                    node_id,
                    member,
                    is_state,
                }) => {
                    if *is_state {
                        return None;
                    }
                    let is_target =
                        Some(*node_id) == ctx.parent_node || *node_id == ctx.current_node;
                    if is_target {
                        if let Some(comp_ports) = ctx.comp_ports {
                            if let Some(port_expr) = comp_ports.get(member.as_str()) {
                                return match port_expr {
                                    Expr::Ident(pid)
                                        if pid.name == Symbol::UNSET || pid.name == Symbol::NONE =>
                                    {
                                        Some(true)
                                    }
                                    Expr::MemberAccess(sub_m) => {
                                        if let Expr::Ident(sub_target_id) = sub_m.target.as_ref() {
                                            if ctx.enums.contains_key(sub_target_id.as_str()) {
                                                return None;
                                            }
                                            let is_self = sub_target_id.name == Symbol::SELF
                                                || sub_target_id.node_id()
                                                    == Some(ctx.current_node);
                                            Some(is_self)
                                        } else {
                                            None
                                        }
                                    }
                                    _ => None,
                                };
                            }
                        }
                    }
                }
                _ => {}
            }
            None
        }
        Expr::MemberAccess(m) => {
            if let Expr::Ident(target_id) = m.target.as_ref() {
                if ctx.enums.contains_key(target_id.as_str()) {
                    return None;
                }
                let is_target = target_id.name == Symbol::PARENT
                    || target_id.name == Symbol::SELF
                    || (ctx.parent_node.is_some() && target_id.node_id() == ctx.parent_node)
                    || target_id.node_id() == Some(ctx.current_node);
                if is_target {
                    if ctx
                        .declared_state_names
                        .is_some_and(|s| s.contains(m.member.as_str()))
                    {
                        return None;
                    }
                    if let Some(comp_ports) = ctx.comp_ports {
                        if let Some(port_expr) = comp_ports.get(m.member.as_str()) {
                            return match port_expr {
                                Expr::Ident(pid)
                                    if pid.name == Symbol::UNSET || pid.name == Symbol::NONE =>
                                {
                                    Some(true)
                                }
                                Expr::MemberAccess(sub_m) => {
                                    if let Expr::Ident(sub_target_id) = sub_m.target.as_ref() {
                                        if ctx.enums.contains_key(sub_target_id.as_str()) {
                                            return None;
                                        }
                                        let is_self = sub_target_id.name == Symbol::SELF
                                            || sub_target_id.node_id() == Some(ctx.current_node);
                                        Some(is_self)
                                    } else {
                                        None
                                    }
                                }
                                _ => None,
                            };
                        }
                    }
                }
            }
            None
        }
        _ => None,
    }
}

pub(super) fn resolve_ident_or_member_to_literal<'a>(
    expr: &'a Expr,
    ctx: &ScopeContext<'a>,
) -> Option<Literal> {
    match expr {
        Expr::Literal(lit) => Some(lit.clone()),
        Expr::Call(call) if call.callee.as_str() == "to_integer" && call.args.len() == 1 => {
            if let Expr::Literal(Literal::Integer(i, span)) = &call.args[0] {
                Some(Literal::Integer(*i, *span))
            } else {
                None
            }
        }
        Expr::Paren(inner, _) => resolve_ident_or_member_to_literal(inner, ctx),
        Expr::Ident(id) => {
            if ctx
                .declared_state_names
                .is_some_and(|s| s.contains(id.as_str()))
            {
                return None;
            }
            if let Some(comp_ports) = ctx.comp_ports {
                if let Some(port_expr) = comp_ports.get(id.as_str()) {
                    match port_expr {
                        Expr::Literal(lit) => return Some(lit.clone()),
                        Expr::Call(call) if call.callee.as_str() == "to_integer" && call.args.len() == 1 => {
                            if let Expr::Literal(Literal::Integer(i, span)) = &call.args[0] {
                                return Some(Literal::Integer(*i, *span));
                            }
                        }
                        Expr::MemberAccess(m) => {
                            if let Expr::Ident(target_id) = m.target.as_ref() {
                                if let Some(enum_def) = ctx.enums.get(target_id.as_str()) {
                                    if enum_def.has_variant(m.member.as_str()) {
                                        return Some(Literal::Enum(
                                            target_id.as_str().to_string(),
                                            m.member.as_str().to_string(),
                                            m.span,
                                        ));
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            match ctx.lexical_scope.get(&id.name) {
                Some(LexicalBinding::Expr(e)) => return resolve_ident_or_member_to_literal(e, ctx),
                Some(LexicalBinding::Member {
                    node_id,
                    member,
                    is_state,
                }) => {
                    if *is_state {
                        return None;
                    }
                    let is_target =
                        Some(*node_id) == ctx.parent_node || *node_id == ctx.current_node;
                    if is_target {
                        if let Some(comp_ports) = ctx.comp_ports {
                            if let Some(port_expr) = comp_ports.get(member.as_str()) {
                                match port_expr {
                                    Expr::Literal(lit) => return Some(lit.clone()),
                                    Expr::Call(call)
                                        if call.callee.as_str() == "to_integer"
                                            && call.args.len() == 1 =>
                                    {
                                        if let Expr::Literal(Literal::Integer(i, span)) =
                                            &call.args[0]
                                        {
                                            return Some(Literal::Integer(*i, *span));
                                        }
                                    }
                                    Expr::MemberAccess(sub_m) => {
                                        if let Expr::Ident(sub_target_id) = sub_m.target.as_ref() {
                                            if let Some(enum_def) =
                                                ctx.enums.get(sub_target_id.as_str())
                                            {
                                                if enum_def.has_variant(sub_m.member.as_str()) {
                                                    return Some(Literal::Enum(
                                                        sub_target_id.as_str().to_string(),
                                                        sub_m.member.as_str().to_string(),
                                                        sub_m.span,
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            None
        }
        Expr::MemberAccess(m) => {
            if let Expr::Ident(target_id) = m.target.as_ref() {
                if target_id.as_str() == "children" && m.member.as_str() == "len" {
                    return Some(Literal::Integer(ctx.num_children as i64, m.span));
                }
                if let Some(enum_def) = ctx.enums.get(target_id.as_str()) {
                    if enum_def.has_variant(m.member.as_str()) {
                        return Some(Literal::Enum(
                            target_id.as_str().to_string(),
                            m.member.as_str().to_string(),
                            m.span,
                        ));
                    }
                }
                let is_target = target_id.name == Symbol::PARENT
                    || target_id.name == Symbol::SELF
                    || (ctx.parent_node.is_some() && target_id.node_id() == ctx.parent_node)
                    || target_id.node_id() == Some(ctx.current_node);
                if is_target {
                    if ctx
                        .declared_state_names
                        .is_some_and(|s| s.contains(m.member.as_str()))
                    {
                        return None;
                    }
                    if let Some(comp_ports) = ctx.comp_ports {
                        if let Some(port_expr) = comp_ports.get(m.member.as_str()) {
                            match port_expr {
                                Expr::Literal(lit) => return Some(lit.clone()),
                                Expr::Call(call) if call.callee.as_str() == "to_integer" && call.args.len() == 1 => {
                                    if let Expr::Literal(Literal::Integer(i, span)) = &call.args[0] {
                                        return Some(Literal::Integer(*i, *span));
                                    }
                                }
                                Expr::MemberAccess(sub_m) => {
                                    if let Expr::Ident(sub_target_id) = sub_m.target.as_ref() {
                                        if let Some(enum_def) = ctx.enums.get(sub_target_id.as_str()) {
                                            if enum_def.has_variant(sub_m.member.as_str()) {
                                                return Some(Literal::Enum(
                                                    sub_target_id.as_str().to_string(),
                                                    sub_m.member.as_str().to_string(),
                                                    sub_m.span,
                                                ));
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            None
        }
        Expr::Binary(bin) => {
            if matches!(bin.op, BinaryOp::Eq | BinaryOp::Ne) {
                if let (Some(l_unset), Some(r_unset)) = (
                    is_handler_expr_unset(&bin.left, ctx),
                    is_handler_expr_unset(&bin.right, ctx),
                ) {
                    let res = if bin.op == BinaryOp::Eq {
                        l_unset == r_unset
                    } else {
                        l_unset != r_unset
                    };
                    return Some(Literal::Bool(res, bin.span));
                }
            }
            let left_lit = resolve_ident_or_member_to_literal(&bin.left, ctx)?;
            let right_lit = resolve_ident_or_member_to_literal(&bin.right, ctx)?;
            match bin.op {
                BinaryOp::Eq => {
                    let eq = match (&left_lit, &right_lit) {
                        (Literal::String(s1, _), Literal::String(s2, _)) => s1 == s2,
                        (Literal::Integer(i1, _), Literal::Integer(i2, _)) => i1 == i2,
                        (Literal::Float(n1, _), Literal::Float(n2, _)) => n1 == n2,
                        (Literal::Integer(i1, _), Literal::Float(n2, _)) => (*i1 as f64) == *n2,
                        (Literal::Float(n1, _), Literal::Integer(i2, _)) => *n1 == (*i2 as f64),
                        (Literal::Distance(d1, u1, _), Literal::Distance(d2, u2, _))
                        | (Literal::Area(d1, u1, _), Literal::Area(d2, u2, _))
                            if u1 == u2 =>
                        {
                            d1 == d2
                        }
                        (Literal::Duration(d1, _, _), Literal::Duration(d2, _, _)) => d1 == d2,
                        (
                            Literal::Distance(d1, _, _) | Literal::Area(d1, _, _),
                            Literal::Integer(0, _),
                        )
                        | (
                            Literal::Integer(0, _),
                            Literal::Distance(d1, _, _) | Literal::Area(d1, _, _),
                        ) => *d1 == 0.0,
                        (Literal::Duration(d1, _, _), Literal::Integer(0, _))
                        | (Literal::Integer(0, _), Literal::Duration(d1, _, _)) => *d1 == 0,
                        (Literal::Bool(b1, _), Literal::Bool(b2, _)) => b1 == b2,
                        (Literal::Color(c1, _), Literal::Color(c2, _)) => c1 == c2,
                        (Literal::Enum(e1, v1, _), Literal::Enum(e2, v2, _)) => e1 == e2 && v1 == v2,
                        _ => false,
                    };
                    Some(Literal::Bool(eq, bin.span))
                }
                BinaryOp::Ne => {
                    let ne = match (&left_lit, &right_lit) {
                        (Literal::String(s1, _), Literal::String(s2, _)) => s1 != s2,
                        (Literal::Integer(i1, _), Literal::Integer(i2, _)) => i1 != i2,
                        (Literal::Float(n1, _), Literal::Float(n2, _)) => n1 != n2,
                        (Literal::Integer(i1, _), Literal::Float(n2, _)) => (*i1 as f64) != *n2,
                        (Literal::Float(n1, _), Literal::Integer(i2, _)) => *n1 != (*i2 as f64),
                        (Literal::Distance(d1, u1, _), Literal::Distance(d2, u2, _))
                        | (Literal::Area(d1, u1, _), Literal::Area(d2, u2, _))
                            if u1 == u2 =>
                        {
                            d1 != d2
                        }
                        (Literal::Duration(d1, _, _), Literal::Duration(d2, _, _)) => d1 != d2,
                        (
                            Literal::Distance(d1, _, _) | Literal::Area(d1, _, _),
                            Literal::Integer(0, _),
                        )
                        | (
                            Literal::Integer(0, _),
                            Literal::Distance(d1, _, _) | Literal::Area(d1, _, _),
                        ) => *d1 != 0.0,
                        (Literal::Duration(d1, _, _), Literal::Integer(0, _))
                        | (Literal::Integer(0, _), Literal::Duration(d1, _, _)) => *d1 != 0,
                        (Literal::Bool(b1, _), Literal::Bool(b2, _)) => b1 != b2,
                        (Literal::Color(c1, _), Literal::Color(c2, _)) => c1 != c2,
                        (Literal::Enum(e1, v1, _), Literal::Enum(e2, v2, _)) => e1 != e2 || v1 != v2,
                        _ => true,
                    };
                    Some(Literal::Bool(ne, bin.span))
                }
                BinaryOp::And => {
                    if let (Literal::Bool(b1, _), Literal::Bool(b2, _)) = (&left_lit, &right_lit) {
                        Some(Literal::Bool(*b1 && *b2, bin.span))
                    } else {
                        None
                    }
                }
                BinaryOp::Or => {
                    if let (Literal::Bool(b1, _), Literal::Bool(b2, _)) = (&left_lit, &right_lit) {
                        Some(Literal::Bool(*b1 || *b2, bin.span))
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }
        Expr::Unary(u) => match u.op {
            UnaryOp::Not => {
                if let Some(Literal::Bool(b, _)) = resolve_ident_or_member_to_literal(&u.operand, ctx) {
                    Some(Literal::Bool(!b, u.span))
                } else {
                    None
                }
            }
            UnaryOp::Neg => match resolve_ident_or_member_to_literal(&u.operand, ctx)? {
                Literal::Integer(i, _) => Some(Literal::Integer(-i, u.span)),
                Literal::Float(f, _) => Some(Literal::Float(-f, u.span)),
                Literal::Distance(d, unit, _) => Some(Literal::Distance(-d, unit, u.span)),
                Literal::Area(a, unit, _) => Some(Literal::Area(-a, unit, u.span)),
                Literal::Duration(d, unit, _) => Some(Literal::Duration(-d, unit, u.span)),
                _ => None,
            },
        },
        _ => None,
    }
}

pub(super) fn known_builtin_port_type(node_name: &str, port_name: &str) -> Option<StaticType> {
    if node_name == "Window" {
        return super::builtins::window::window_port_type(port_name);
    }
    if let Some(prim) = super::builtins::lookup_builtin(node_name, true) {
        return prim.port_type(port_name);
    }
    super::builtins::default_builtin_port_type(port_name)
}

struct TypecheckCache<'a> {
    node_caches: Vec<HashMap<Symbol, StaticType>>,
    window_cache: HashMap<Symbol, StaticType>,
    visiting: HashSet<(NodeId, Symbol)>,
    template_types: Option<&'a HashMap<Symbol, StaticType>>,
}

impl<'a> TypecheckCache<'a> {
    fn new(num_nodes: usize) -> Self {
        Self {
            node_caches: vec![HashMap::new(); num_nodes],
            window_cache: HashMap::new(),
            visiting: HashSet::new(),
            template_types: None,
        }
    }

    fn with_template_types(template_types: &'a HashMap<Symbol, StaticType>) -> Self {
        Self {
            node_caches: Vec::new(),
            window_cache: HashMap::new(),
            visiting: HashSet::new(),
            template_types: Some(template_types),
        }
    }

    #[inline]
    fn get(&self, node_id: NodeId, port_sym: Symbol) -> Option<&StaticType> {
        if node_id.is_window() {
            self.window_cache.get(&port_sym)
        } else {
            self.node_caches.get(node_id.0)?.get(&port_sym)
        }
    }

    #[inline]
    fn insert(&mut self, node_id: NodeId, port_sym: Symbol, ty: StaticType) {
        if node_id.is_window() {
            self.window_cache.insert(port_sym, ty);
        } else if let Some(map) = self.node_caches.get_mut(node_id.0) {
            map.insert(port_sym, ty);
        }
    }
}

pub(super) fn infer_template_expr_type(
    expr: &Expr,
    template_types: &HashMap<Symbol, StaticType>,
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> Result<StaticType, CompileError> {
    let empty_doc = ExpandedDocument::new();
    let mut cache = TypecheckCache::with_template_types(template_types);
    infer_expr_type(expr, &empty_doc, enums, registry, &mut cache)
}

/// Strictly type-checks all expanded node ports, window ports, and local bindings at compile time.
pub fn typecheck_expanded_document(
    doc: &mut ExpandedDocument,
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> Result<(), CompileError> {
    let mut cache = TypecheckCache::new(doc.nodes.len());

    // 1. Validate and infer all window ports
    let window_port_names: Vec<String> = doc.window_ports.keys().cloned().collect();
    for port_name in window_port_names {
        let port_sym = Symbol::new(&port_name);
        let inferred = infer_port_type(
            NodeId::WINDOW,
            port_sym,
            &port_name,
            doc,
            enums,
            registry,
            &mut cache,
        )?;
        doc.window_port_types
            .entry(port_name)
            .or_insert_with(|| inferred.type_name());
    }

    // 2. Validate and infer all node ports
    let mut non_preverified_ports: Vec<(String, Symbol)> = Vec::with_capacity(16);
    for idx in 0..doc.nodes.len() {
        let node_id = NodeId(idx);
        non_preverified_ports.clear();
        {
            let node = &doc.nodes[idx];
            for port_name in node.ports.keys() {
                if !node.is_port_preverified(port_name) {
                    non_preverified_ports.push((port_name.clone(), Symbol::new(port_name)));
                }
            }
        }
        for &(ref port_name, port_sym) in &non_preverified_ports {
            let inferred = infer_port_type(
                node_id,
                port_sym,
                port_name,
                doc,
                enums,
                registry,
                &mut cache,
            )?;
            if inferred == StaticType::Handler && !doc.nodes[idx].port_types.contains_key(port_name)
            {
                std::sync::Arc::make_mut(&mut doc.nodes[idx].port_types)
                    .insert(port_name.clone(), "Handler".to_string());
            }
        }
    }

    // 3. Validate and infer all local bindings (\let, \env, \alias)
    for idx in 0..doc.local_bindings.len() {
        let actual = infer_expr_type(
            &doc.local_bindings[idx].0,
            doc,
            enums,
            registry,
            &mut cache,
        )?;
        if let Some(ref expected_name) = doc.local_bindings[idx].1 {
            let span = doc.local_bindings[idx].2;
            let expected = StaticType::from_type_name(expected_name, span, enums, registry)?;
            if !actual.matches(&expected) {
                return Err(CompileError::TypeMismatch {
                    expected: expected.type_name(),
                    actual: actual.type_name(),
                    span: doc.local_bindings[idx].0.span(),
                });
            }
        }
    }

    Ok(())
}

fn infer_port_type(
    node_id: NodeId,
    port_sym: Symbol,
    port_name: &str,
    doc: &ExpandedDocument,
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
    cache: &mut TypecheckCache<'_>,
) -> Result<StaticType, CompileError> {
    if !node_id.is_window() {
        if let Some(node) = doc.get_node(node_id) {
            if let Some(ty) = node.get_preverified_port_type(port_sym, port_name) {
                return Ok(ty.clone());
            }
        }
    }
    if let Some(cached) = cache.get(node_id, port_sym) {
        return Ok(cached.clone());
    }

    let explicit_ty = if node_id.is_window() {
        if let Some(name) = doc.window_port_types.get(port_name) {
            Some(StaticType::from_type_name(
                name,
                Span::default(),
                enums,
                registry,
            )?)
        } else {
            None
        }
    } else if let Some(node) = doc.get_node(node_id) {
        if let Some(name) = node.port_types.get(port_name) {
            if let Some(ty) = node
                .preverified_port_types
                .as_ref()
                .and_then(|m| m.get(&port_sym))
            {
                Some(ty.clone())
            } else {
                Some(StaticType::from_type_name(
                    name,
                    Span::default(),
                    enums,
                    registry,
                )?)
            }
        } else {
            None
        }
    } else {
        None
    };

    let key = (node_id, port_sym);
    if !cache.visiting.insert(key) {
        // Cycle detected: if there is an explicit or builtin type, return it; otherwise let graph cycle checker report it
        if let Some(ty) = explicit_ty {
            return Ok(ty);
        }
        let node_name = if node_id.is_window() {
            "Window"
        } else {
            doc.get_node(node_id).map(|n| n.name.as_str()).unwrap_or("")
        };
        if let Some(builtin) = known_builtin_port_type(node_name, port_name) {
            return Ok(builtin);
        }
        return Ok(StaticType::Float);
    }

    let expr_opt = if node_id.is_window() {
        doc.window_ports.get(port_name)
    } else {
        doc.get_node(node_id).and_then(|n| n.ports.get(port_name))
    };

    let result = if let Some(expr) = expr_opt {
        let actual = infer_expr_type(expr, doc, enums, registry, cache)?;
        if let Some(expected) = explicit_ty {
            if !actual.matches(&expected) {
                cache.visiting.remove(&key);
                return Err(CompileError::TypeMismatch {
                    expected: expected.type_name(),
                    actual: actual.type_name(),
                    span: expr.span(),
                });
            }
            Ok(expected)
        } else {
            let node_name = if node_id.is_window() {
                "Window"
            } else {
                doc.get_node(node_id).map(|n| n.name.as_str()).unwrap_or("")
            };
            if let Some(builtin) = known_builtin_port_type(node_name, port_name) {
                if !node_id.is_window() && doc.get_node(node_id).is_some_and(|n| !n.is_component) {
                    if !actual.matches(&builtin) {
                        cache.visiting.remove(&key);
                        return Err(CompileError::TypeMismatch {
                            expected: builtin.type_name(),
                            actual: actual.type_name(),
                            span: expr.span(),
                        });
                    } else {
                        Ok(builtin)
                    }
                } else {
                    Ok(actual.decay())
                }
            } else {
                Ok(actual.decay())
            }
        }
    } else if let Some(expected) = explicit_ty {
        Ok(expected)
    } else {
        let node_name = if node_id.is_window() {
            "Window"
        } else {
            doc.get_node(node_id).map(|n| n.name.as_str()).unwrap_or("")
        };
        if let Some(builtin) = known_builtin_port_type(node_name, port_name) {
            Ok(builtin)
        } else {
            cache.visiting.remove(&key);
            return Err(CompileError::Custom {
                message: format!(
                    "Node '{}' has no known type for property '{}'",
                    node_name, port_name
                ),
                span: Span::default(),
            });
        }
    };

    cache.visiting.remove(&key);
    if let Ok(ref ty) = result {
        cache.insert(key.0, key.1, ty.clone());
    }
    result
}

fn infer_expr_type(
    expr: &Expr,
    doc: &ExpandedDocument,
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
    cache: &mut TypecheckCache<'_>,
) -> Result<StaticType, CompileError> {
    match expr {
        Expr::Literal(lit) => Ok(match lit {
            Literal::Integer(0, _) => StaticType::ZeroInteger,
            Literal::Integer(_, _) => StaticType::Integer,
            Literal::Float(_, _) => StaticType::Float,
            Literal::Distance(_, _, _) => StaticType::Distance,
            Literal::Area(_, _, _) => StaticType::Area,
            Literal::Duration(_, _, _) => StaticType::Duration,
            Literal::String(_, _) => StaticType::String,
            Literal::Bool(_, _) => StaticType::Boolean,
            Literal::Color(_, _) => StaticType::Color,
            Literal::Enum(enum_name, _, _) => StaticType::Enum(enum_name.clone()),
        }),
        Expr::Paren(inner, _) => infer_expr_type(inner, doc, enums, registry, cache),
        Expr::Ident(id) => {
            if id.name == Symbol::UNSET || id.name == Symbol::NONE {
                return Ok(StaticType::Unset);
            }
            if id.name == Symbol::WINDOW {
                return Ok(StaticType::Node("Window".to_string()));
            }
            if let Some(node_id) = id.node_id() {
                if node_id.is_window() {
                    return Ok(StaticType::Node("Window".to_string()));
                }
                if let Some(node) = doc.get_node(node_id) {
                    return Ok(StaticType::Node(node.name.clone()));
                }
            }
            Err(CompileError::Custom {
                message: format!("Cannot determine static type for identifier '{}'", id.as_str()),
                span: id.span,
            })
        }
        Expr::MemberAccess(m) => {
            if let Expr::Ident(target_id) = m.target.as_ref() {
                if let Some(enum_def) = enums.get(target_id.as_str()) {
                    if enum_def.has_variant(m.member.as_str()) {
                        return Ok(StaticType::Enum(target_id.as_str().to_string()));
                    }
                }
                if let Some(node_id) = target_id.node_id() {
                    if node_id.is_template_self() {
                        if let Some(template_types) = cache.template_types {
                            if let Some(ty) = template_types.get(&m.member.name) {
                                return Ok(ty.clone());
                            }
                        }
                        return Err(CompileError::Custom {
                            message: format!(
                                "Template port '{}' has no known static type",
                                m.member.as_str()
                            ),
                            span: m.span,
                        });
                    }
                    let has_port = if node_id.is_window() {
                        doc.window_ports.contains_key(m.member.as_str())
                            || known_builtin_port_type("Window", m.member.as_str()).is_some()
                    } else if let Some(node) = doc.get_node(node_id) {
                        node.ports.contains_key(m.member.as_str())
                            || node.port_types.contains_key(m.member.as_str())
                            || (!node.is_component
                                && known_builtin_port_type(&node.name, m.member.as_str()).is_some())
                    } else {
                        false
                    };
                    if has_port {
                        return infer_port_type(
                            node_id,
                            m.member.name,
                            m.member.as_str(),
                            doc,
                            enums,
                            registry,
                            cache,
                        );
                    }
                    // Check if it's an event handler method on a component node
                    if let Some(node) = doc.get_node(node_id) {
                        if node.is_component {
                            return Ok(StaticType::Handler);
                        }
                    }
                }
            }
            let target_ty = infer_expr_type(&m.target, doc, enums, registry, cache)?;
            match target_ty {
                StaticType::Node(ref node_name) => {
                    if let Some(builtin) = known_builtin_port_type(node_name, m.member.as_str()) {
                        return Ok(builtin);
                    }
                    if let Some(overloads) = registry.get(node_name).or_else(|| {
                        registry.iter().find_map(|(k, v)| {
                            (crate::compiler::graph::unmangle_component_name(k) == node_name)
                                .then_some(v)
                        })
                    }) {
                        for comp in overloads {
                            if let Some(param) =
                                comp.params.iter().find(|p| p.name.as_str() == m.member.as_str())
                            {
                                if let Some(ref t) = param.type_annotation {
                                    if let Ok(st) = StaticType::from_type_name(
                                        t.name.as_str(),
                                        t.span,
                                        enums,
                                        registry,
                                    ) {
                                        return Ok(st);
                                    }
                                }
                            }
                        }
                        return Ok(StaticType::Handler);
                    }
                    Err(CompileError::Custom {
                        message: format!(
                            "Node '{}' has no known property or method '{}'",
                            node_name, m.member
                        ),
                        span: m.span,
                    })
                }
                StaticType::ViewPort => match m.member.as_str() {
                    "x" | "y" | "width" | "height" | "left" | "top" | "right" | "bottom" => {
                        Ok(StaticType::Distance)
                    }
                    _ => Err(CompileError::Custom {
                        message: format!("ViewPort has no property '{}'", m.member),
                        span: m.span,
                    }),
                },
                StaticType::Timer => match m.member.as_str() {
                    "time" | "duration" | "dt" => Ok(StaticType::Duration),
                    "elapsed" => Ok(StaticType::Float),
                    "frames" => Ok(StaticType::Integer),
                    _ => Err(CompileError::Custom {
                        message: format!("Node 'Timer' has no public property '{}'", m.member),
                        span: m.span,
                    }),
                },
                StaticType::IndefiniteTimer => match m.member.as_str() {
                    "time" | "dt" => Ok(StaticType::Duration),
                    "frames" => Ok(StaticType::Integer),
                    _ => Err(CompileError::Custom {
                        message: format!(
                            "Node 'IndefiniteTimer' has no public property '{}'",
                            m.member
                        ),
                        span: m.span,
                    }),
                },
                other => Err(CompileError::Custom {
                    message: format!(
                        "Cannot access member '{}' on value of type '{}'",
                        m.member,
                        other.type_name()
                    ),
                    span: m.span,
                }),
            }
        }
        Expr::Unary(u) => {
            let op_ty = infer_expr_type(&u.operand, doc, enums, registry, cache)?;
            match u.op {
                UnaryOp::Neg => match op_ty.decay() {
                    StaticType::Integer => Ok(StaticType::Integer),
                    StaticType::Float => Ok(StaticType::Float),
                    StaticType::Distance => Ok(StaticType::Distance),
                    StaticType::Area => Ok(StaticType::Area),
                    StaticType::Duration => Ok(StaticType::Duration),
                    other => Err(CompileError::TypeMismatch {
                        expected: "Float, Integer, Distance, Area, or Duration".to_string(),
                        actual: other.type_name(),
                        span: u.span,
                    }),
                },
                UnaryOp::Not => match op_ty.decay() {
                    StaticType::Boolean
                    | StaticType::Timer
                    | StaticType::IndefiniteTimer
                    | StaticType::Handler => Ok(StaticType::Boolean),
                    other => Err(CompileError::TypeMismatch {
                        expected: "Boolean or Timer".to_string(),
                        actual: other.type_name(),
                        span: u.span,
                    }),
                },
            }
        }
        Expr::Binary(bin) => {
            let left_ty = infer_expr_type(&bin.left, doc, enums, registry, cache)?;
            let right_ty = infer_expr_type(&bin.right, doc, enums, registry, cache)?;
            match bin.op {
                BinaryOp::Add => match (&left_ty, &right_ty) {
                    (StaticType::String, _) | (_, StaticType::String) => Ok(StaticType::String),
                    (StaticType::Distance, StaticType::Distance)
                    | (StaticType::Distance, StaticType::ZeroInteger)
                    | (StaticType::ZeroInteger, StaticType::Distance) => Ok(StaticType::Distance),
                    (StaticType::Area, StaticType::Area)
                    | (StaticType::Area, StaticType::ZeroInteger)
                    | (StaticType::ZeroInteger, StaticType::Area) => Ok(StaticType::Area),
                    (StaticType::Duration, StaticType::Duration)
                    | (StaticType::Duration, StaticType::ZeroInteger)
                    | (StaticType::ZeroInteger, StaticType::Duration) => Ok(StaticType::Duration),
                    (StaticType::Integer | StaticType::ZeroInteger, StaticType::Integer | StaticType::ZeroInteger) => {
                        Ok(StaticType::Integer)
                    }
                    (
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                    ) => Ok(StaticType::Float),
                    (StaticType::Distance, other) => Err(CompileError::TypeMismatch {
                        expected: "Distance".to_string(),
                        actual: other.type_name(),
                        span: bin.right.span(),
                    }),
                    (StaticType::Area, other) => Err(CompileError::TypeMismatch {
                        expected: "Area".to_string(),
                        actual: other.type_name(),
                        span: bin.right.span(),
                    }),
                    (StaticType::Duration, other) => Err(CompileError::TypeMismatch {
                        expected: "Duration".to_string(),
                        actual: other.type_name(),
                        span: bin.right.span(),
                    }),
                    (other, StaticType::Distance) => Err(CompileError::TypeMismatch {
                        expected: "Distance".to_string(),
                        actual: other.type_name(),
                        span: bin.left.span(),
                    }),
                    (other, StaticType::Area) => Err(CompileError::TypeMismatch {
                        expected: "Area".to_string(),
                        actual: other.type_name(),
                        span: bin.left.span(),
                    }),
                    (other, StaticType::Duration) => Err(CompileError::TypeMismatch {
                        expected: "Duration".to_string(),
                        actual: other.type_name(),
                        span: bin.left.span(),
                    }),
                    _ => Err(CompileError::TypeMismatch {
                        expected: left_ty.type_name(),
                        actual: right_ty.type_name(),
                        span: bin.span,
                    }),
                },
                BinaryOp::Sub => match (&left_ty, &right_ty) {
                    (StaticType::Distance, StaticType::Distance)
                    | (StaticType::Distance, StaticType::ZeroInteger)
                    | (StaticType::ZeroInteger, StaticType::Distance) => Ok(StaticType::Distance),
                    (StaticType::Area, StaticType::Area)
                    | (StaticType::Area, StaticType::ZeroInteger)
                    | (StaticType::ZeroInteger, StaticType::Area) => Ok(StaticType::Area),
                    (StaticType::Duration, StaticType::Duration)
                    | (StaticType::Duration, StaticType::ZeroInteger)
                    | (StaticType::ZeroInteger, StaticType::Duration) => Ok(StaticType::Duration),
                    (StaticType::Integer | StaticType::ZeroInteger, StaticType::Integer | StaticType::ZeroInteger) => {
                        Ok(StaticType::Integer)
                    }
                    (
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                    ) => Ok(StaticType::Float),
                    (StaticType::Distance, other) => Err(CompileError::TypeMismatch {
                        expected: "Distance".to_string(),
                        actual: other.type_name(),
                        span: bin.right.span(),
                    }),
                    (StaticType::Area, other) => Err(CompileError::TypeMismatch {
                        expected: "Area".to_string(),
                        actual: other.type_name(),
                        span: bin.right.span(),
                    }),
                    (StaticType::Duration, other) => Err(CompileError::TypeMismatch {
                        expected: "Duration".to_string(),
                        actual: other.type_name(),
                        span: bin.right.span(),
                    }),
                    (other, StaticType::Distance) => Err(CompileError::TypeMismatch {
                        expected: "Distance".to_string(),
                        actual: other.type_name(),
                        span: bin.left.span(),
                    }),
                    (other, StaticType::Area) => Err(CompileError::TypeMismatch {
                        expected: "Area".to_string(),
                        actual: other.type_name(),
                        span: bin.left.span(),
                    }),
                    (other, StaticType::Duration) => Err(CompileError::TypeMismatch {
                        expected: "Duration".to_string(),
                        actual: other.type_name(),
                        span: bin.left.span(),
                    }),
                    _ => Err(CompileError::TypeMismatch {
                        expected: left_ty.type_name(),
                        actual: right_ty.type_name(),
                        span: bin.span,
                    }),
                },
                BinaryOp::Mul => match (&left_ty, &right_ty) {
                    (StaticType::Distance, StaticType::Distance) => Ok(StaticType::Area),
                    (
                        StaticType::Distance,
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                    )
                    | (
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                        StaticType::Distance,
                    ) => Ok(StaticType::Distance),
                    (
                        StaticType::Area,
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                    )
                    | (
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                        StaticType::Area,
                    ) => Ok(StaticType::Area),
                    (
                        StaticType::Duration,
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                    )
                    | (
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                        StaticType::Duration,
                    ) => Ok(StaticType::Duration),
                    (StaticType::Integer | StaticType::ZeroInteger, StaticType::Integer | StaticType::ZeroInteger) => {
                        Ok(StaticType::Integer)
                    }
                    (
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                    ) => Ok(StaticType::Float),
                    _ => Err(CompileError::TypeMismatch {
                        expected: "Float, Integer, Distance, or Duration".to_string(),
                        actual: format!("{} * {}", left_ty.type_name(), right_ty.type_name()),
                        span: bin.span,
                    }),
                },
                BinaryOp::Div => match (&left_ty, &right_ty) {
                    (StaticType::Distance, StaticType::Distance)
                    | (StaticType::Area, StaticType::Area)
                    | (StaticType::Duration, StaticType::Duration) => Ok(StaticType::Float),
                    (StaticType::Area, StaticType::Distance) => Ok(StaticType::Distance),
                    (
                        StaticType::Distance,
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                    ) => Ok(StaticType::Distance),
                    (
                        StaticType::Area,
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                    ) => Ok(StaticType::Area),
                    (
                        StaticType::Duration,
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                    ) => Ok(StaticType::Duration),
                    (
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                    ) => Ok(StaticType::Float),
                    _ => Err(CompileError::TypeMismatch {
                        expected: "Float, Integer, Distance, Area, or Duration".to_string(),
                        actual: format!("{} / {}", left_ty.type_name(), right_ty.type_name()),
                        span: bin.span,
                    }),
                },
                BinaryOp::Rem => match (&left_ty, &right_ty) {
                    (StaticType::Distance, StaticType::Distance) => Ok(StaticType::Distance),
                    (StaticType::Area, StaticType::Area) => Ok(StaticType::Area),
                    (StaticType::Duration, StaticType::Duration) => Ok(StaticType::Duration),
                    (
                        StaticType::Integer | StaticType::ZeroInteger,
                        StaticType::Integer | StaticType::ZeroInteger,
                    ) => Ok(StaticType::Integer),
                    (
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                    ) => Ok(StaticType::Float),
                    _ => Err(CompileError::TypeMismatch {
                        expected: "Float or Integer".to_string(),
                        actual: format!("{} % {}", left_ty.type_name(), right_ty.type_name()),
                        span: bin.span,
                    }),
                },
                BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                    match (&left_ty, &right_ty) {
                        (StaticType::Distance, StaticType::Distance)
                        | (StaticType::Distance, StaticType::ZeroInteger)
                        | (StaticType::ZeroInteger, StaticType::Distance)
                        | (StaticType::Area, StaticType::Area)
                        | (StaticType::Area, StaticType::ZeroInteger)
                        | (StaticType::ZeroInteger, StaticType::Area)
                        | (StaticType::Duration, StaticType::Duration)
                        | (StaticType::Duration, StaticType::ZeroInteger)
                        | (StaticType::ZeroInteger, StaticType::Duration)
                        | (
                            StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                            StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                        ) => Ok(StaticType::Boolean),
                        (StaticType::Distance, other) => Err(CompileError::TypeMismatch {
                            expected: "Distance".to_string(),
                            actual: other.type_name(),
                            span: bin.right.span(),
                        }),
                        (StaticType::Area, other) => Err(CompileError::TypeMismatch {
                            expected: "Area".to_string(),
                            actual: other.type_name(),
                            span: bin.right.span(),
                        }),
                        (StaticType::Duration, other) => Err(CompileError::TypeMismatch {
                            expected: "Duration".to_string(),
                            actual: other.type_name(),
                            span: bin.right.span(),
                        }),
                        (other, StaticType::Distance) => Err(CompileError::TypeMismatch {
                            expected: "Distance".to_string(),
                            actual: other.type_name(),
                            span: bin.left.span(),
                        }),
                        (other, StaticType::Area) => Err(CompileError::TypeMismatch {
                            expected: "Area".to_string(),
                            actual: other.type_name(),
                            span: bin.left.span(),
                        }),
                        (other, StaticType::Duration) => Err(CompileError::TypeMismatch {
                            expected: "Duration".to_string(),
                            actual: other.type_name(),
                            span: bin.left.span(),
                        }),
                        _ => Err(CompileError::TypeMismatch {
                            expected: left_ty.type_name(),
                            actual: right_ty.type_name(),
                            span: bin.span,
                        }),
                    }
                }
                BinaryOp::Eq | BinaryOp::Ne => {
                    if left_ty.matches(&right_ty) || right_ty.matches(&left_ty) {
                        Ok(StaticType::Boolean)
                    } else if matches!(
                        (&left_ty, &right_ty),
                        (StaticType::Color, StaticType::String)
                            | (StaticType::String, StaticType::Color)
                    ) {
                        Ok(StaticType::Boolean)
                    } else {
                        Err(CompileError::TypeMismatch {
                            expected: left_ty.type_name(),
                            actual: right_ty.type_name(),
                            span: bin.span,
                        })
                    }
                }
                BinaryOp::And | BinaryOp::Or => {
                    if left_ty != StaticType::Boolean {
                        return Err(CompileError::TypeMismatch {
                            expected: "Boolean".to_string(),
                            actual: left_ty.type_name(),
                            span: bin.left.span(),
                        });
                    }
                    if right_ty != StaticType::Boolean {
                        return Err(CompileError::TypeMismatch {
                            expected: "Boolean".to_string(),
                            actual: right_ty.type_name(),
                            span: bin.right.span(),
                        });
                    }
                    Ok(StaticType::Boolean)
                }
            }
        }
        Expr::Ternary(tern) => {
            let cond_ty = infer_expr_type(&tern.condition, doc, enums, registry, cache)?;
            if !matches!(
                cond_ty,
                StaticType::Boolean
                    | StaticType::Timer
                    | StaticType::IndefiniteTimer
                    | StaticType::Handler
                    | StaticType::Unset
            ) {
                return Err(CompileError::TypeMismatch {
                    expected: "Boolean".to_string(),
                    actual: cond_ty.type_name(),
                    span: tern.condition.span(),
                });
            }
            let then_ty = infer_expr_type(&tern.then_expr, doc, enums, registry, cache)?;
            let else_ty = infer_expr_type(&tern.else_expr, doc, enums, registry, cache)?;
            if then_ty == else_ty {
                Ok(then_ty.decay())
            } else if then_ty.matches(&else_ty) {
                Ok(else_ty.decay())
            } else if else_ty.matches(&then_ty) {
                Ok(then_ty.decay())
            } else if matches!(
                (&then_ty, &else_ty),
                (StaticType::Node(a), StaticType::Node(b))
                    if matches!(a.as_str(), "Window" | "ScrollView")
                        && matches!(b.as_str(), "Window" | "ScrollView")
            ) {
                Ok(StaticType::ViewPort)
            } else {
                Err(CompileError::TypeMismatch {
                    expected: then_ty.type_name(),
                    actual: else_ty.type_name(),
                    span: tern.else_expr.span(),
                })
            }
        }
        Expr::Call(call) => {
            match call.callee.as_str() {
                "distance_to_float" | "to_px" | "to_pt" | "to_float" | "to_number" => {
                    for arg in &call.args {
                        let _ = infer_expr_type(arg, doc, enums, registry, cache)?;
                    }
                    Ok(StaticType::Float)
                }
                "to_integer" => {
                    for arg in &call.args {
                        let _ = infer_expr_type(arg, doc, enums, registry, cache)?;
                    }
                    Ok(StaticType::Integer)
                }
                "sqrt" => {
                    let mut first_ty = None;
                    for (i, arg) in call.args.iter().enumerate() {
                        let t = infer_expr_type(arg, doc, enums, registry, cache)?;
                        if i == 0 {
                            first_ty = Some(t);
                        }
                    }
                    if matches!(first_ty, Some(StaticType::Area)) {
                        Ok(StaticType::Distance)
                    } else {
                        Ok(StaticType::Float)
                    }
                }
                "round" | "floor" | "ceil" => {
                    let mut first_ty = None;
                    for (i, arg) in call.args.iter().enumerate() {
                        let t = infer_expr_type(arg, doc, enums, registry, cache)?;
                        if i == 0 {
                            first_ty = Some(t);
                        }
                    }
                    match first_ty {
                        Some(StaticType::Distance) => Ok(StaticType::Distance),
                        Some(StaticType::Area) => Ok(StaticType::Area),
                        Some(StaticType::Duration) => Ok(StaticType::Duration),
                        _ => Ok(StaticType::Integer),
                    }
                }
                "to_fixed" | "to_precision" => {
                    for arg in &call.args {
                        let _ = infer_expr_type(arg, doc, enums, registry, cache)?;
                    }
                    Ok(StaticType::String)
                }
                "hsl" | "hsla" | "rgb" | "rgba" => {
                    for arg in &call.args {
                        let _ = infer_expr_type(arg, doc, enums, registry, cache)?;
                    }
                    Ok(StaticType::Color)
                }
                "text_width"
                | "text_height"
                | "text_baseline_offset"
                | "span_metric"
                | "font_cap_height"
                | "font_x_height"
                | "font_ascent"
                | "font_descent"
                | "font_line_gap"
                | "font_line_height" => {
                    for arg in &call.args {
                        let _ = infer_expr_type(arg, doc, enums, registry, cache)?;
                    }
                    Ok(StaticType::Distance)
                }
                "max" | "min" | "sum" | "abs" | "clamp" | "only" | "hypot" => {
                    if call.args.is_empty() {
                        return Ok(StaticType::Distance);
                    }
                    let mut acc_ty =
                        infer_expr_type(&call.args[0], doc, enums, registry, cache)?;
                    for arg in &call.args[1..] {
                        let arg_ty = infer_expr_type(arg, doc, enums, registry, cache)?;
                        if arg_ty.matches(&acc_ty) {
                            // keep acc_ty
                        } else if acc_ty.matches(&arg_ty) {
                            acc_ty = arg_ty;
                        } else {
                            return Err(CompileError::TypeMismatch {
                                expected: acc_ty.type_name(),
                                actual: arg_ty.type_name(),
                                span: arg.span(),
                            });
                        }
                    }
                    if call.callee.as_str() == "hypot" && acc_ty.decay() == StaticType::Integer {
                        Ok(StaticType::Float)
                    } else {
                        Ok(acc_ty.decay())
                    }
                }
                _ => {
                    for arg in &call.args {
                        let _ = infer_expr_type(arg, doc, enums, registry, cache)?;
                    }
                    Ok(StaticType::Float)
                }
            }
        }
        Expr::MethodCall(mc) => {
            let target_ty = infer_expr_type(&mc.target, doc, enums, registry, cache)?;
            for arg in &mc.args {
                let _ = infer_expr_type(arg, doc, enums, registry, cache)?;
            }
            match mc.method.as_str() {
                "append" | "to_fixed" | "to_precision" => Ok(StaticType::String),
                "to_px" | "to_pt" | "to_float" | "to_number" => Ok(StaticType::Float),
                "to_seconds" | "to_ms" => {
                    if !target_ty.matches(&StaticType::Duration) {
                        return Err(CompileError::TypeMismatch {
                            expected: "Duration".to_string(),
                            actual: target_ty.type_name(),
                            span: mc.target.span(),
                        });
                    }
                    Ok(StaticType::Float)
                }
                "to_integer" => Ok(StaticType::Integer),
                _ => Ok(StaticType::Float),
            }
        }
        Expr::Node(elem) => match elem.name.as_str() {
            "Timer" => {
                if elem.content.is_some() {
                    return Err(CompileError::Custom {
                        message: "Node 'Timer' does not accept a content block".to_string(),
                        span: elem.span,
                    });
                }
                if elem.key.is_some() {
                    return Err(CompileError::Custom {
                        message: "Node 'Timer' does not accept a key".to_string(),
                        span: elem.span,
                    });
                }
                if elem.ports.is_empty() {
                    return Err(CompileError::MissingPort {
                        node: "Timer".to_string(),
                        port: "duration".to_string(),
                        span: elem.span,
                    });
                }
                for port in &elem.ports {
                    if port.name.as_str() != "duration" {
                        return Err(CompileError::Custom {
                            message: format!(
                                "Node 'Timer' does not accept property '{}'",
                                port.name.as_str()
                            ),
                            span: port.name.span,
                        });
                    }
                    let port_ty =
                        infer_expr_type(&port.expr, doc, enums, registry, cache)?;
                    if !port_ty.matches(&StaticType::Duration) {
                        return Err(CompileError::TypeMismatch {
                            expected: "Duration".to_string(),
                            actual: port_ty.type_name(),
                            span: port.expr.span(),
                        });
                    }
                }
                Ok(StaticType::Timer)
            }
            "IndefiniteTimer" => {
                if elem.content.is_some() {
                    return Err(CompileError::Custom {
                        message: "Node 'IndefiniteTimer' does not accept a content block"
                            .to_string(),
                        span: elem.span,
                    });
                }
                if elem.key.is_some() {
                    return Err(CompileError::Custom {
                        message: "Node 'IndefiniteTimer' does not accept a key".to_string(),
                        span: elem.span,
                    });
                }
                if let Some(port) = elem.ports.first() {
                    return Err(CompileError::Custom {
                        message: format!(
                            "Node 'IndefiniteTimer' does not accept property '{}'",
                            port.name.as_str()
                        ),
                        span: port.name.span,
                    });
                }
                Ok(StaticType::IndefiniteTimer)
            }
            _ => Ok(StaticType::Node(elem.name.as_str().to_string())),
        },
    }
}

