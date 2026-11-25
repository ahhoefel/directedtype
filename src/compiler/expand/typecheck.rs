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
    Path,
    Boolean,
    Color,
    Handler,
    Timer,
    IndefiniteTimer,
    Unset,
    ViewPort,
    Expr,
    Enum(String),
    Node(String),
    PageValueSlice(Option<NodeId>),
    PageListSlice(Option<NodeId>),
    BucketPayload(Option<NodeId>),
    PagePrevRecursive,
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
            StaticType::Path => "Path".to_string(),
            StaticType::Boolean => "Boolean".to_string(),
            StaticType::Color => "Color".to_string(),
            StaticType::Handler | StaticType::Unset => "Handler".to_string(),
            StaticType::Timer => "Timer".to_string(),
            StaticType::IndefiniteTimer => "IndefiniteTimer".to_string(),
            StaticType::ViewPort => "ViewPort".to_string(),
            StaticType::Expr => "Expr".to_string(),
            StaticType::Enum(name) => name.clone(),
            StaticType::Node(name) => name.clone(),
            StaticType::PageValueSlice(_) => "PageValueSlice".to_string(),
            StaticType::PageListSlice(_) => "PageListSlice".to_string(),
            StaticType::BucketPayload(_) => "BucketPayload".to_string(),
            StaticType::PagePrevRecursive => "PagePrevRecursive".to_string(),
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
            "Path" => Ok(StaticType::Path),
            "Boolean" => Ok(StaticType::Boolean),
            "Color" => Ok(StaticType::Color),
            "Handler" => Ok(StaticType::Handler),
            "Timer" => Ok(StaticType::Timer),
            "IndefiniteTimer" => Ok(StaticType::IndefiniteTimer),
            "ViewPort" => Ok(StaticType::ViewPort),
            "Expr" | "Expression" => Ok(StaticType::Expr),
            "Node" => Ok(StaticType::Node("Node".to_string())),
            "Window"
            | "Rect"
            | "Line"
            | "Text"
            | "Image"
            | "Font"
            | "Clip"
            | "Box"
            | "Anchor"
            | "AnchorScope"
            | "Bookmark"
            | "Page"
            | "Pagination"
            | "ParagraphStyle"
            | "PageBreak"
            | "PageValue"
            | "PageList"
            | "PagedFlow"
            | "FlowBody" => {
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
            (
                StaticType::PageListSlice(_) | StaticType::PageValueSlice(_),
                StaticType::PageListSlice(None) | StaticType::PageValueSlice(None),
            ) => true,
            (StaticType::PagePrevRecursive, _) | (_, StaticType::PagePrevRecursive) => true,
            _ => false,
        }
    }
}

pub(super) fn validate_expr_type_annotations(
    expr: &Expr,
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> Result<(), CompileError> {
    match expr {
        Expr::ExprDef(def) => {
            for param in &def.params {
                if let Some(ty) = &param.type_annotation {
                    StaticType::from_type_name(ty.name.as_str(), ty.span, enums, registry)?;
                }
                if let Some(default_expr) = &param.default_edge {
                    validate_expr_type_annotations(default_expr, enums, registry)?;
                }
            }
            validate_expr_type_annotations(&def.body, enums, registry)?;
        }
        Expr::Binary(b) => {
            validate_expr_type_annotations(&b.left, enums, registry)?;
            validate_expr_type_annotations(&b.right, enums, registry)?;
        }
        Expr::Unary(u) => validate_expr_type_annotations(&u.operand, enums, registry)?,
        Expr::Ternary(t) => {
            validate_expr_type_annotations(&t.condition, enums, registry)?;
            validate_expr_type_annotations(&t.then_expr, enums, registry)?;
            validate_expr_type_annotations(&t.else_expr, enums, registry)?;
        }
        Expr::MemberAccess(m) => validate_expr_type_annotations(&m.target, enums, registry)?,
        Expr::Index(idx) => {
            validate_expr_type_annotations(&idx.target, enums, registry)?;
            validate_expr_type_annotations(&idx.index, enums, registry)?;
        }
        Expr::Call(c) => {
            for arg in &c.args {
                validate_expr_type_annotations(arg, enums, registry)?;
            }
        }
        Expr::MethodCall(mc) => {
            validate_expr_type_annotations(&mc.target, enums, registry)?;
            for arg in &mc.args {
                validate_expr_type_annotations(arg, enums, registry)?;
            }
        }
        Expr::Paren(inner, _) => validate_expr_type_annotations(inner, enums, registry)?,
        Expr::Node(elem) => {
            for port in &elem.ports {
                validate_expr_type_annotations(&port.expr, enums, registry)?;
            }
            if let Some(slot) = &elem.content {
                validate_content_items_type_annotations(&slot.items, enums, registry)?;
            }
        }
        Expr::Match(m) => {
            if let Some(s) = &m.scrutinee {
                validate_expr_type_annotations(s, enums, registry)?;
            }
            for arm in &m.arms {
                if let MatchPattern::Condition(c) = &arm.pattern {
                    validate_expr_type_annotations(c, enums, registry)?;
                }
                validate_expr_type_annotations(&arm.body, enums, registry)?;
            }
        }
        Expr::Literal(_) | Expr::Ident(_) | Expr::DomNodeRef(_, _) => {}
    }
    Ok(())
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
                match &l.value {
                    Some(LetValue::Node(n)) => {
                        for port in &n.ports {
                            validate_expr_type_annotations(&port.expr, enums, registry)?;
                        }
                        if let Some(slot) = &n.content {
                            validate_content_items_type_annotations(&slot.items, enums, registry)?;
                        }
                    }
                    Some(LetValue::Expr(e)) => {
                        validate_expr_type_annotations(e, enums, registry)?;
                    }
                    None => {}
                }
            }
            ContentItem::Alias(a) => {
                if let Some(ty) = &a.type_annotation {
                    StaticType::from_type_name(ty.name.as_str(), ty.span, enums, registry)?;
                }
                if let Some(val) = &a.value {
                    validate_expr_type_annotations(val, enums, registry)?;
                }
            }
            ContentItem::State(s) => {
                if let Some(ty) = &s.type_annotation {
                    StaticType::from_type_name(ty.name.as_str(), ty.span, enums, registry)?;
                }
                if let Some(d) = &s.default {
                    validate_expr_type_annotations(d, enums, registry)?;
                }
            }
            ContentItem::Env(e) => {
                if let Some(ty) = &e.type_annotation {
                    StaticType::from_type_name(ty.name.as_str(), ty.span, enums, registry)?;
                }
                if let Some(v) = &e.value {
                    validate_expr_type_annotations(v, enums, registry)?;
                }
            }
            ContentItem::Node(n) => {
                for port in &n.ports {
                    validate_expr_type_annotations(&port.expr, enums, registry)?;
                }
                if let Some(slot) = &n.content {
                    validate_content_items_type_annotations(&slot.items, enums, registry)?;
                }
            }
            ContentItem::Children(dir) => {
                for port in &dir.ports {
                    validate_expr_type_annotations(&port.expr, enums, registry)?;
                    validate_children_sibling_guards(&port.expr, false, false)?;
                }
            }
            ContentItem::Text(_)
            | ContentItem::ParagraphBreak(_)
            | ContentItem::Assert(_) => {}
        }
    }
    Ok(())
}

fn extract_sibling_guards(
    cond: &Expr,
    prev_guarded: bool,
    next_guarded: bool,
) -> Result<(bool, bool, bool, bool), CompileError> {
    match cond {
        Expr::Paren(inner, _) => extract_sibling_guards(inner, prev_guarded, next_guarded),
        Expr::Ident(id) => {
            if id.name == Symbol::PREV {
                Ok((true, false, false, false))
            } else if id.name == Symbol::NEXT {
                Ok((false, false, true, false))
            } else {
                Ok((false, false, false, false))
            }
        }
        Expr::Unary(u) if u.op == UnaryOp::Not => {
            let (pp, np, pn, nn) = extract_sibling_guards(&u.operand, prev_guarded, next_guarded)?;
            Ok((np, pp, nn, pn))
        }
        Expr::Binary(bin) => match bin.op {
            BinaryOp::Eq | BinaryOp::Ne => {
                fn unwrap_ident(e: &Expr) -> Option<Symbol> {
                    match e {
                        Expr::Paren(inner, _) => unwrap_ident(inner),
                        Expr::Ident(id) => Some(id.name),
                        _ => None,
                    }
                }
                let l_sym = unwrap_ident(&bin.left);
                let r_sym = unwrap_ident(&bin.right);
                let is_unset_sym =
                    |s: Option<Symbol>| matches!(s, Some(Symbol::UNSET | Symbol::NONE));
                if (l_sym == Some(Symbol::PREV) && is_unset_sym(r_sym))
                    || (r_sym == Some(Symbol::PREV) && is_unset_sym(l_sym))
                {
                    if bin.op == BinaryOp::Ne {
                        return Ok((true, false, false, false));
                    } else {
                        return Ok((false, true, false, false));
                    }
                }
                if (l_sym == Some(Symbol::NEXT) && is_unset_sym(r_sym))
                    || (r_sym == Some(Symbol::NEXT) && is_unset_sym(l_sym))
                {
                    if bin.op == BinaryOp::Ne {
                        return Ok((false, false, true, false));
                    } else {
                        return Ok((false, false, false, true));
                    }
                }
                validate_children_sibling_guards(&bin.left, prev_guarded, next_guarded)?;
                validate_children_sibling_guards(&bin.right, prev_guarded, next_guarded)?;
                Ok((false, false, false, false))
            }
            BinaryOp::And => {
                let (l_pp, l_np, l_pn, l_nn) =
                    extract_sibling_guards(&bin.left, prev_guarded, next_guarded)?;
                let (r_pp, r_np, r_pn, r_nn) = extract_sibling_guards(
                    &bin.right,
                    prev_guarded || l_pp,
                    next_guarded || l_pn,
                )?;
                Ok((l_pp || r_pp, l_np && r_np, l_pn || r_pn, l_nn && r_nn))
            }
            BinaryOp::Or => {
                let (l_pp, l_np, l_pn, l_nn) =
                    extract_sibling_guards(&bin.left, prev_guarded, next_guarded)?;
                let (r_pp, r_np, r_pn, r_nn) = extract_sibling_guards(
                    &bin.right,
                    prev_guarded || l_np,
                    next_guarded || l_nn,
                )?;
                Ok((l_pp && r_pp, l_np || r_np, l_pn && r_pn, l_nn || r_nn))
            }
            _ => {
                validate_children_sibling_guards(&bin.left, prev_guarded, next_guarded)?;
                validate_children_sibling_guards(&bin.right, prev_guarded, next_guarded)?;
                Ok((false, false, false, false))
            }
        },
        _ => {
            validate_children_sibling_guards(cond, prev_guarded, next_guarded)?;
            Ok((false, false, false, false))
        }
    }
}

pub(super) fn validate_children_sibling_guards(
    expr: &Expr,
    prev_guarded: bool,
    next_guarded: bool,
) -> Result<(), CompileError> {
    match expr {
        Expr::Literal(_) => Ok(()),
        Expr::Paren(inner, _) => validate_children_sibling_guards(inner, prev_guarded, next_guarded),
        Expr::Ident(id) => {
            if id.name == Symbol::PREV && !prev_guarded {
                return Err(CompileError::Custom {
                    message:
                        "Cannot access 'prev' without a guard in '\\Children'; guard with 'prev ? ... : ...'"
                            .to_string(),
                    span: id.span,
                });
            }
            if id.name == Symbol::NEXT && !next_guarded {
                return Err(CompileError::Custom {
                    message:
                        "Cannot access 'next' without a guard in '\\Children'; guard with 'next ? ... : ...'"
                            .to_string(),
                    span: id.span,
                });
            }
            Ok(())
        }
        Expr::MemberAccess(m) => {
            validate_children_sibling_guards(&m.target, prev_guarded, next_guarded)
        }
        Expr::Unary(u) => {
            if u.op == UnaryOp::Not {
                let _ = extract_sibling_guards(expr, prev_guarded, next_guarded)?;
                Ok(())
            } else {
                validate_children_sibling_guards(&u.operand, prev_guarded, next_guarded)
            }
        }
        Expr::Binary(b) => {
            if matches!(
                b.op,
                BinaryOp::And | BinaryOp::Or | BinaryOp::Eq | BinaryOp::Ne
            ) {
                let _ = extract_sibling_guards(expr, prev_guarded, next_guarded)?;
                Ok(())
            } else {
                validate_children_sibling_guards(&b.left, prev_guarded, next_guarded)?;
                validate_children_sibling_guards(&b.right, prev_guarded, next_guarded)
            }
        }
        Expr::Ternary(t) => {
            let (pp, np, pn, nn) =
                extract_sibling_guards(&t.condition, prev_guarded, next_guarded)?;
            validate_children_sibling_guards(&t.then_expr, prev_guarded || pp, next_guarded || pn)?;
            validate_children_sibling_guards(&t.else_expr, prev_guarded || np, next_guarded || nn)
        }
        Expr::Call(c) => {
            for arg in &c.args {
                validate_children_sibling_guards(arg, prev_guarded, next_guarded)?;
            }
            Ok(())
        }
        Expr::MethodCall(mc) => {
            validate_children_sibling_guards(&mc.target, prev_guarded, next_guarded)?;
            for arg in &mc.args {
                validate_children_sibling_guards(arg, prev_guarded, next_guarded)?;
            }
            Ok(())
        }
        Expr::Node(elem) => {
            for port in &elem.ports {
                validate_children_sibling_guards(&port.expr, prev_guarded, next_guarded)?;
            }
            Ok(())
        }
        Expr::ExprDef(def) => {
            for p in &def.params {
                if let Some(d) = &p.default_edge {
                    validate_children_sibling_guards(d, prev_guarded, next_guarded)?;
                }
            }
            validate_children_sibling_guards(&def.body, prev_guarded, next_guarded)
        }
        Expr::Match(m) => {
            if let Some(s) = &m.scrutinee {
                validate_children_sibling_guards(s, prev_guarded, next_guarded)?;
            }
            let mut acc_neg_prev = false;
            let mut acc_neg_next = false;
            for arm in &m.arms {
                if let MatchPattern::Condition(c) = &arm.pattern {
                    let (pp, np, pn, nn) = extract_sibling_guards(
                        c,
                        prev_guarded || acc_neg_prev,
                        next_guarded || acc_neg_next,
                    )?;
                    validate_children_sibling_guards(
                        &arm.body,
                        prev_guarded || acc_neg_prev || pp,
                        next_guarded || acc_neg_next || pn,
                    )?;
                    acc_neg_prev |= np;
                    acc_neg_next |= nn;
                } else {
                    validate_children_sibling_guards(
                        &arm.body,
                        prev_guarded || acc_neg_prev,
                        next_guarded || acc_neg_next,
                    )?;
                }
            }
            Ok(())
        }
        Expr::Index(idx) => {
            validate_children_sibling_guards(&idx.target, prev_guarded, next_guarded)?;
            validate_children_sibling_guards(&idx.index, prev_guarded, next_guarded)
        }
        Expr::DomNodeRef(_, _) => Ok(()),
    }
}

/// Coerces syntactic string literals (`"..."`) into `Literal::Path` (recursing into parentheses,
/// ternaries, and `match` arms) while leaving identifier/variable references untouched.
pub(crate) fn coerce_syntactic_path_expr(expr: &Expr) -> Expr {
    match expr {
        Expr::Literal(Literal::String(s, span)) => Expr::Literal(Literal::Path(s.clone(), *span)),
        Expr::Paren(inner, span) => {
            Expr::Paren(std::sync::Arc::new(coerce_syntactic_path_expr(inner)), *span)
        }
        Expr::Ternary(t) => {
            let mut next = t.clone();
            next.then_expr = std::sync::Arc::new(coerce_syntactic_path_expr(&t.then_expr));
            next.else_expr = std::sync::Arc::new(coerce_syntactic_path_expr(&t.else_expr));
            Expr::Ternary(next)
        }
        Expr::Match(m) => {
            let mut next = (**m).clone();
            for arm in &mut next.arms {
                arm.body = coerce_syntactic_path_expr(&arm.body);
            }
            Expr::Match(std::sync::Arc::new(next))
        }
        other => other.clone(),
    }
}

/// Pre-coerces syntactic AST literals before `rewrite_expr` when the target type annotation is `Path`.
pub(crate) fn pre_coerce_ast_for_type<'a>(
    raw_expr: &'a Expr,
    type_annotation: Option<&TypeRef>,
) -> std::borrow::Cow<'a, Expr> {
    if type_annotation.is_some_and(|t| t.name.as_str() == "Path") {
        std::borrow::Cow::Owned(coerce_syntactic_path_expr(raw_expr))
    } else {
        std::borrow::Cow::Borrowed(raw_expr)
    }
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
            | "letter_spacing"
            | "baseline"
            | "indent"
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
    } else if matches!(port_name, "italic" | "small_caps" | "underline") {
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
    } else if port_name == "src" {
        let unlowered_lit = match orig_expr {
            Expr::Literal(Literal::String(s, sp)) => Some(Literal::Path(s.clone(), *sp)),
            Expr::Literal(lit) => Some(lit.clone()),
            _ => None,
        };
        if let Some(lit) =
            unlowered_lit.or_else(|| infer_expr_literal_for_typecheck(rewritten, scope_ctx))
        {
            if !lit.matches_type_name("Path") {
                return Err(CompileError::TypeMismatch {
                    expected: "Path".to_string(),
                    actual: lit.type_name().to_string(),
                    span: orig_expr.span(),
                });
            }
        }
    } else if port_name == "dpi" || port_name == "opacity" {
        let unlowered_lit = match orig_expr {
            Expr::Literal(lit) => Some(lit.clone()),
            _ => None,
        };
        if let Some(lit) =
            unlowered_lit.or_else(|| infer_expr_literal_for_typecheck(rewritten, scope_ctx))
        {
            if !lit.matches_type_name("Float") {
                return Err(CompileError::TypeMismatch {
                    expected: "Float".to_string(),
                    actual: lit.type_name().to_string(),
                    span: orig_expr.span(),
                });
            }
        }
        if let Expr::Literal(Literal::Integer(i, span)) = *rewritten {
            *rewritten = Expr::Literal(Literal::Float(i as f64, span));
        }
    }
    Ok(())
}

/// Validates component parameter types and coerces numeric literals (`0 -> 0px`, `Integer -> Float`).
pub(super) fn validate_and_coerce_component_param_types(
    comp_def: &ComponentDef,
    instance_ports: &[PortBinding],
    preverified_ports: &super::schema::PortMask,
    port_table: &super::schema::PortTable,
    scope_ctx: &ScopeContext<'_>,
    rewritten_ports: &mut HashMap<String, Expr>,
) -> Result<(), CompileError> {
    for param in &comp_def.params {
        if let Some(type_ref) = &param.type_annotation {
            let name = param.name.as_str();
            if preverified_ports.contains_sym(port_table, param.name.name, name) {
                continue;
            }
            let expected_type = type_ref.name.as_str();
            let unlowered_lit = instance_ports
                .iter()
                .find(|p| p.name.name == param.name.name)
                .map(|p| &p.expr)
                .or(param.default_edge.as_ref())
                .and_then(|e| match e {
                    Expr::Literal(Literal::String(s, sp)) if expected_type == "Path" => {
                        Some(Literal::Path(s.clone(), *sp))
                    }
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
            if let Expr::Ident(target_id) = m.target.as_ref() {
                if let Some(nid) = target_id.node_id() {
                    if member.starts_with("__expr_") {
                        if let Some(mat_expr) =
                            super::rewrite::lookup_materialized_expr_port(nid, member, ctx.nodes)
                        {
                            return infer_expr_literal_for_typecheck(&mat_expr, ctx);
                        }
                    }
                }
            }
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
                    | "letter_spacing"
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
            if DistanceUnit::from_conversion_fn(call.callee.as_str()).is_some()
                || matches!(
                    call.callee.as_str(),
                    "distance_to_float"
                        | "to_float"
                        | "to_number"
                        | "sin"
                        | "cos"
                        | "tan"
                        | "atan2"
                )
            {
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
                    | "__image_intrinsic_width"
                    | "__image_intrinsic_height"
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
                "__image_aspect_ratio" | "__image_file_dpi"
            ) {
                return Some(Literal::Float(1.0, call.span));
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
            if DistanceUnit::from_conversion_fn(mc.method.as_str()).is_some()
                || matches!(
                    mc.method.as_str(),
                    "to_seconds" | "to_ms" | "to_float" | "to_number"
                )
            {
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

fn is_component_handler_port(port_name: &str, ctx: &ScopeContext<'_>) -> bool {
    if port_name.starts_with("on_") {
        return true;
    }
    for nid in [
        Some(ctx.current_node),
        ctx.parent_node,
        ctx.enclosing_component,
    ]
    .into_iter()
    .flatten()
    {
        if let Some(node) = ctx.nodes.get(nid.0) {
            if let Some(ty) = node.port_types.get(port_name) {
                return ty == "Handler";
            }
        }
    }
    false
}

pub(super) fn is_handler_expr_unset(expr: &Expr, ctx: &ScopeContext<'_>) -> Option<bool> {
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
            if is_component_handler_port(id.as_str(), ctx) {
                if let Some(comp_ports) = ctx.comp_ports {
                    if let Some(port_expr) = comp_ports.get(id.as_str()) {
                        return match port_expr {
                            Expr::Ident(pid)
                                if pid.as_str() == "unset" || pid.as_str() == "None" =>
                            {
                                Some(true)
                            }
                            Expr::MemberAccess(m) => {
                                if let Expr::Ident(target_id) = m.target.as_ref() {
                                    if ctx.enums.contains_key(target_id.as_str()) {
                                        return None;
                                    }
                                    let is_self = target_id.name == Symbol::SELF
                                        || target_id
                                            .node_id()
                                            .is_some_and(|n| n == ctx.current_node || n.is_template_self());
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
            match ctx.lexical_scope.get(&id.name) {
                Some(LexicalBinding::Expr(e)) if is_component_handler_port(id.as_str(), ctx) => {
                    return is_handler_expr_unset(e, ctx);
                }
                Some(LexicalBinding::Member {
                    node_id,
                    member,
                    is_state,
                }) => {
                    if *is_state || !is_component_handler_port(member.as_str(), ctx) {
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
                                                || sub_target_id
                                                    .node_id()
                                                    .is_some_and(|n| n == ctx.current_node || n.is_template_self());
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
                    || target_id
                        .node_id()
                        .is_some_and(|n| n == ctx.current_node || n.is_template_self());
                if is_target {
                    if ctx
                        .declared_state_names
                        .is_some_and(|s| s.contains(m.member.as_str()))
                        || !is_component_handler_port(m.member.as_str(), ctx)
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
                                            || sub_target_id
                                                .node_id()
                                                .is_some_and(|n| n == ctx.current_node || n.is_template_self());
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

pub(super) fn resolve_const_bool_or_handler<'a>(
    expr: &'a Expr,
    ctx: &ScopeContext<'a>,
) -> Option<bool> {
    if let Some(Literal::Bool(b, _)) = resolve_ident_or_member_to_literal(expr, ctx) {
        Some(b)
    } else if let Some(is_unset) = is_handler_expr_unset(expr, ctx) {
        Some(!is_unset)
    } else {
        None
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
                if let Some(nid) = target_id.node_id() {
                    if m.member.as_str().starts_with("__expr_") {
                        if let Some(mat_expr) = super::rewrite::lookup_materialized_expr_port(
                            nid,
                            m.member.as_str(),
                            ctx.nodes,
                        ) {
                            return resolve_ident_or_member_to_literal(&mat_expr, ctx);
                        }
                    }
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
                    || target_id
                        .node_id()
                        .is_some_and(|n| n == ctx.current_node || n.is_template_self());
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
            if bin.op == BinaryOp::And {
                let b1 = resolve_const_bool_or_handler(&bin.left, ctx)?;
                let b2 = resolve_const_bool_or_handler(&bin.right, ctx)?;
                return Some(Literal::Bool(b1 && b2, bin.span));
            }
            if bin.op == BinaryOp::Or {
                let b1 = resolve_const_bool_or_handler(&bin.left, ctx)?;
                let b2 = resolve_const_bool_or_handler(&bin.right, ctx)?;
                return Some(Literal::Bool(b1 || b2, bin.span));
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
                        (Literal::Path(p1, _), Literal::Path(p2, _)) => p1 == p2,
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
                        (Literal::Path(p1, _), Literal::Path(p2, _)) => p1 != p2,
                        (Literal::Color(c1, _), Literal::Color(c2, _)) => c1 != c2,
                        (Literal::Enum(e1, v1, _), Literal::Enum(e2, v2, _)) => e1 != e2 || v1 != v2,
                        _ => true,
                    };
                    Some(Literal::Bool(ne, bin.span))
                }
                _ => None,
            }
        }
        Expr::Unary(u) => match u.op {
            UnaryOp::Not => {
                let b = resolve_const_bool_or_handler(&u.operand, ctx)?;
                Some(Literal::Bool(!b, u.span))
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
    if node_name == "Node" {
        return match port_name {
            "page" | "end_page" => Some(StaticType::Integer),
            "x" | "y" | "width" | "height" | "left" | "top" | "right" | "bottom" => {
                Some(StaticType::Distance)
            }
            _ => None,
        };
    }
    if node_name == "Window" {
        return super::builtins::window::window_port_type(port_name);
    }
    if node_name == "Page" {
        return match port_name {
            "number" | "count" => Some(StaticType::Integer),
            "is_first" | "is_last" | "is_odd" | "is_recto" | "is_even" | "is_verso" => {
                Some(StaticType::Boolean)
            }
            "width" | "height" | "left" | "top" | "right" | "bottom" => Some(StaticType::Distance),
            "dpi" => Some(StaticType::Float),
            "prev" => Some(StaticType::Node("PageTemplate".to_string())),
            _ => None,
        };
    }
    if let Some(prim) = super::builtins::lookup_builtin(node_name, true) {
        return prim.port_type(port_name);
    }
    super::builtins::default_builtin_port_type(port_name)
}

struct TypecheckCache<'a> {
    node_caches: Vec<HashMap<Symbol, StaticType>>,
    window_cache: HashMap<Symbol, StaticType>,
    page_cache: HashMap<Symbol, StaticType>,
    visiting: HashSet<(NodeId, Symbol)>,
    guarded_timers: HashSet<(NodeId, Symbol)>,
    template_types: Option<&'a HashMap<Symbol, StaticType>>,
    current_self_node: Option<NodeId>,
    bucket_schemas: HashMap<NodeId, HashMap<String, StaticType>>,
}

impl<'a> TypecheckCache<'a> {
    fn new(
        num_nodes: usize,
        bucket_schemas: HashMap<NodeId, HashMap<String, StaticType>>,
    ) -> Self {
        Self {
            node_caches: vec![HashMap::new(); num_nodes],
            window_cache: HashMap::new(),
            page_cache: HashMap::new(),
            visiting: HashSet::new(),
            guarded_timers: HashSet::new(),
            template_types: None,
            current_self_node: None,
            bucket_schemas,
        }
    }

    fn with_template_types(template_types: &'a HashMap<Symbol, StaticType>) -> Self {
        Self {
            node_caches: Vec::new(),
            window_cache: HashMap::new(),
            page_cache: HashMap::new(),
            visiting: HashSet::new(),
            guarded_timers: HashSet::new(),
            template_types: Some(template_types),
            current_self_node: None,
            bucket_schemas: HashMap::new(),
        }
    }

    #[inline]
    fn get(&self, node_id: NodeId, port_sym: Symbol) -> Option<&StaticType> {
        if node_id.is_window() {
            self.window_cache.get(&port_sym)
        } else if node_id.is_page() {
            self.page_cache.get(&port_sym)
        } else {
            self.node_caches.get(node_id.0)?.get(&port_sym)
        }
    }

    #[inline]
    fn insert(&mut self, node_id: NodeId, port_sym: Symbol, ty: StaticType) {
        if node_id.is_window() {
            self.window_cache.insert(port_sym, ty);
        } else if node_id.is_page() {
            self.page_cache.insert(port_sym, ty);
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
    typecheck_expanded_document_inner(doc, enums, registry)?;
    for flow in &doc.paged_flows {
        let mut probe_doc = flow.probe_doc.clone();
        probe_doc.page_template_private_ports = doc.page_template_private_ports.clone();
        probe_doc.deposits = doc.deposits.clone();
        typecheck_expanded_document_inner(&mut probe_doc, enums, registry)?;
    }
    Ok(())
}

fn typecheck_expanded_document_inner(
    doc: &mut ExpandedDocument,
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> Result<(), CompileError> {
    let bucket_schemas = infer_bucket_schemas(doc, enums, registry)?;
    let mut cache = TypecheckCache::new(doc.nodes.len(), bucket_schemas);

    // 0. Validate page.prev & page bucket guard discipline and privacy rules before expression inference
    let empty_guards: HashSet<(NodeId, BucketIndexKey, bool)> = HashSet::new();
    for expr in doc.window_ports.values() {
        validate_page_prev_guards(expr, false)?;
        validate_bucket_guards(expr, doc, &empty_guards)?;
    }
    for expr in doc.page_ports.values() {
        validate_page_prev_guards(expr, false)?;
        validate_bucket_guards(expr, doc, &empty_guards)?;
    }
    for node in &doc.nodes {
        for expr in node.ports.values() {
            validate_page_prev_guards(expr, false)?;
            validate_bucket_guards(expr, doc, &empty_guards)?;
        }
    }
    for (expr, _, _) in &doc.local_bindings {
        validate_page_prev_guards(expr, false)?;
        validate_bucket_guards(expr, doc, &empty_guards)?;
    }

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
            if (inferred == StaticType::Handler || inferred == StaticType::Expr)
                && !doc.nodes[idx].port_types.contains_key(port_name)
            {
                std::sync::Arc::make_mut(&mut doc.nodes[idx].port_types)
                    .insert(port_name.clone(), inferred.type_name());
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
    if !node_id.is_window() && !node_id.is_page() {
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
    } else if node_id.is_page() {
        known_builtin_port_type("Page", port_name).map(|st| match (&st, &doc.page_template_component) {
            (StaticType::Node(n), Some(tmpl_name)) if n == "PageTemplate" && port_name == "prev" => {
                StaticType::Node(tmpl_name.clone())
            }
            _ => st,
        })
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

    if !cache.visiting.is_empty() && !node_id.is_window() && !node_id.is_page() {
        if let Some(node) = doc.get_node(node_id) {
            if node.ports.contains_key(port_name)
                && doc
                    .page_template_component
                    .as_ref()
                    .is_none_or(|tmpl| &node.name != tmpl)
            {
                if let Some(expected) = explicit_ty {
                    return Ok(expected);
                }
                if !node.is_component {
                    if let Some(builtin) = known_builtin_port_type(&node.name, port_name) {
                        return Ok(builtin);
                    }
                }
            }
        }
    }

    let key = (node_id, port_sym);
    if !cache.visiting.insert(key) {
        // Cycle detected: if there is an explicit or builtin type, return it; otherwise let graph cycle checker report it
        if let Some(ty) = explicit_ty {
            return Ok(ty);
        }
        let node_name = if node_id.is_window() {
            "Window"
        } else if node_id.is_page() {
            "Page"
        } else {
            doc.get_node(node_id).map(|n| n.name.as_str()).unwrap_or("")
        };
        if let Some(builtin) = known_builtin_port_type(node_name, port_name) {
            return Ok(builtin);
        }
        return Ok(StaticType::Float);
    }

    let materialized_fallback = if node_id.is_window() {
        doc.window_ports
            .get(port_name)
            .is_none()
            .then(|| super::rewrite::get_materialized_expr_port(node_id, port_name))
            .flatten()
    } else if node_id.is_page() {
        doc.page_ports
            .get(port_name)
            .is_none()
            .then(|| super::rewrite::get_materialized_expr_port(node_id, port_name))
            .flatten()
    } else {
        doc.get_node(node_id)
            .and_then(|n| n.ports.get(port_name))
            .is_none()
            .then(|| super::rewrite::get_materialized_expr_port(node_id, port_name))
            .flatten()
    };
    let expr_opt = if node_id.is_window() {
        doc.window_ports.get(port_name)
    } else if node_id.is_page() {
        doc.page_ports.get(port_name)
    } else {
        doc.get_node(node_id).and_then(|n| n.ports.get(port_name))
    }
    .or(materialized_fallback.as_ref());

    let result = if let Some(expr) = expr_opt {
        let saved_guards = std::mem::take(&mut cache.guarded_timers);
        let saved_self_node = cache.current_self_node;
        cache.current_self_node = Some(node_id);
        let actual_res = infer_expr_type(expr, doc, enums, registry, cache);
        cache.current_self_node = saved_self_node;
        cache.guarded_timers = saved_guards;
        let actual = match actual_res {
            Ok(ty) => ty,
            Err(e) => {
                cache.visiting.remove(&key);
                return Err(e);
            }
        };
        if let Some(expected) = explicit_ty {
            let is_page_prev_unset = node_id.is_page()
                && port_name == "prev"
                && matches!(actual, StaticType::Unset);
            if !actual.matches(&expected) && !is_page_prev_unset {
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
            } else if node_id.is_page() {
                "Page"
            } else {
                doc.get_node(node_id).map(|n| n.name.as_str()).unwrap_or("")
            };
            if let Some(builtin) = known_builtin_port_type(node_name, port_name) {
                if !node_id.is_window()
                    && !node_id.is_page()
                    && doc.get_node(node_id).is_some_and(|n| !n.is_component)
                {
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
        } else if node_id.is_page() {
            "Page"
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

fn canonical_timer_key(expr: &Expr, current_self_node: Option<NodeId>) -> Option<(NodeId, Symbol)> {
    match expr {
        Expr::Paren(inner, _) => canonical_timer_key(inner, current_self_node),
        Expr::MemberAccess(m) => {
            if let Expr::Ident(id) = m.target.as_ref() {
                if let Some(raw_node_id) = id.node_id() {
                    let node_id = if raw_node_id.is_template_self() {
                        current_self_node.unwrap_or(raw_node_id)
                    } else {
                        raw_node_id
                    };
                    return Some((node_id, m.member.name));
                }
            }
            None
        }
        _ => None,
    }
}

fn unwrap_parens(mut expr: &Expr) -> &Expr {
    while let Expr::Paren(inner, _) = expr {
        expr = inner;
    }
    expr
}

fn is_direct_indefinite_timer_node(expr: &Expr) -> bool {
    matches!(
        unwrap_parens(expr),
        Expr::Node(elem) if elem.name.as_str() == "IndefiniteTimer"
    )
}

fn is_non_nullable_indefinite_timer_expr(
    expr: &Expr,
    doc: &ExpandedDocument,
    current_self_node: Option<NodeId>,
    depth: usize,
) -> bool {
    if depth > 4 {
        return false;
    }
    if is_direct_indefinite_timer_node(expr) {
        return true;
    }
    if let Some((node_id, port_sym)) = canonical_timer_key(expr, current_self_node) {
        let port_name = port_sym.as_str();
        if node_id.is_window() {
            if !doc.window_state_vars.contains_key(port_name) {
                if let Some(port_expr) = doc.window_ports.get(port_name) {
                    return is_non_nullable_indefinite_timer_expr(
                        port_expr,
                        doc,
                        Some(NodeId::WINDOW),
                        depth + 1,
                    );
                }
            }
        } else if !node_id.is_template_self() && !node_id.is_page() {
            if let Some(node) = doc.get_node(node_id) {
                if !node.state_vars.contains_key(port_name) {
                    if let Some(port_expr) = node.ports.get(port_name) {
                        return is_non_nullable_indefinite_timer_expr(
                            port_expr,
                            doc,
                            Some(node_id),
                            depth + 1,
                        );
                    }
                }
            }
        }
    }
    false
}

fn is_nullable_timer_target(
    target: &Expr,
    target_ty: &StaticType,
    doc: &ExpandedDocument,
    current_self_node: Option<NodeId>,
) -> bool {
    match target_ty {
        StaticType::Timer => true,
        StaticType::IndefiniteTimer => {
            !is_non_nullable_indefinite_timer_expr(target, doc, current_self_node, 0)
        }
        _ => false,
    }
}

fn extract_timer_guards(
    cond: &Expr,
    doc: &ExpandedDocument,
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
    cache: &mut TypecheckCache<'_>,
    depth: usize,
) -> (HashSet<(NodeId, Symbol)>, HashSet<(NodeId, Symbol)>) {
    if depth > 4 {
        return (HashSet::new(), HashSet::new());
    }
    match cond {
        Expr::Paren(inner, _) => extract_timer_guards(inner, doc, enums, registry, cache, depth),
        Expr::Unary(u) if u.op == UnaryOp::Not => {
            let (pos, neg) =
                extract_timer_guards(&u.operand, doc, enums, registry, cache, depth);
            (neg, pos)
        }
        Expr::Binary(bin) => match bin.op {
            BinaryOp::Ne | BinaryOp::Eq => {
                let is_unset_expr = |e: &Expr| {
                    matches!(
                        unwrap_parens(e),
                        Expr::Ident(id) if id.name == Symbol::UNSET || id.name == Symbol::NONE
                    )
                };
                let timer_key = if is_unset_expr(&bin.right) {
                    canonical_timer_key(&bin.left, cache.current_self_node)
                } else if is_unset_expr(&bin.left) {
                    canonical_timer_key(&bin.right, cache.current_self_node)
                } else {
                    None
                };
                let mut pos = HashSet::new();
                let mut neg = HashSet::new();
                if let Some(k) = timer_key {
                    if bin.op == BinaryOp::Ne {
                        pos.insert(k);
                    } else {
                        neg.insert(k);
                    }
                }
                (pos, neg)
            }
            BinaryOp::And => {
                let (l_pos, l_neg) =
                    extract_timer_guards(&bin.left, doc, enums, registry, cache, depth);
                let (r_pos, r_neg) =
                    extract_timer_guards(&bin.right, doc, enums, registry, cache, depth);
                let pos = l_pos.union(&r_pos).copied().collect();
                let neg = l_neg.intersection(&r_neg).copied().collect();
                (pos, neg)
            }
            BinaryOp::Or => {
                let (l_pos, l_neg) =
                    extract_timer_guards(&bin.left, doc, enums, registry, cache, depth);
                let (r_pos, r_neg) =
                    extract_timer_guards(&bin.right, doc, enums, registry, cache, depth);
                let pos = l_pos.intersection(&r_pos).copied().collect();
                let neg = l_neg.union(&r_neg).copied().collect();
                (pos, neg)
            }
            _ => (HashSet::new(), HashSet::new()),
        },
        Expr::MemberAccess(_) => {
            let mut pos = HashSet::new();
            if let Some(k) = canonical_timer_key(cond, cache.current_self_node) {
                if matches!(
                    infer_expr_type(cond, doc, enums, registry, cache),
                    Ok(StaticType::Timer | StaticType::IndefiniteTimer)
                ) {
                    pos.insert(k);
                    return (pos, HashSet::new());
                }
                let (node_id, port_sym) = k;
                let port_name = port_sym.as_str();
                if node_id.is_window() {
                    if !doc.window_state_vars.contains_key(port_name) {
                        if let Some(port_expr) = doc.window_ports.get(port_name) {
                            let saved_self = cache.current_self_node;
                            cache.current_self_node = Some(NodeId::WINDOW);
                            let res = extract_timer_guards(
                                port_expr,
                                doc,
                                enums,
                                registry,
                                cache,
                                depth + 1,
                            );
                            cache.current_self_node = saved_self;
                            return res;
                        }
                    }
                } else if !node_id.is_template_self() && !node_id.is_page() {
                    if let Some(node) = doc.get_node(node_id) {
                        if !node.state_vars.contains_key(port_name) {
                            if let Some(port_expr) = node.ports.get(port_name) {
                                let saved_self = cache.current_self_node;
                                cache.current_self_node = Some(node_id);
                                let res = extract_timer_guards(
                                    port_expr,
                                    doc,
                                    enums,
                                    registry,
                                    cache,
                                    depth + 1,
                                );
                                cache.current_self_node = saved_self;
                                return res;
                            }
                        }
                    }
                }
            }
            (pos, HashSet::new())
        }
        _ => (HashSet::new(), HashSet::new()),
    }
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
            Literal::Path(_, _) => StaticType::Path,
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
            if id.name == Symbol::PAGE {
                return Err(CompileError::PageOutsidePagedFlow { span: id.span });
            }
            if id.name == Symbol::PREV {
                return Err(CompileError::Custom {
                    message: "Cannot access 'prev' without a guard; no previous sibling exists (guard with 'prev ? ... : ...')".to_string(),
                    span: id.span,
                });
            }
            if id.name == Symbol::NEXT {
                return Err(CompileError::Custom {
                    message: "Cannot access 'next' without a guard; no next sibling exists (guard with 'next ? ... : ...')".to_string(),
                    span: id.span,
                });
            }
            if let Some(node_id) = id.node_id() {
                if node_id.is_window() {
                    return Ok(StaticType::Node("Window".to_string()));
                }
                if node_id.is_page() {
                    return Ok(StaticType::Node("Page".to_string()));
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
        Expr::DomNodeRef(_, _) => Ok(StaticType::Node("Node".to_string())),
        Expr::MemberAccess(m) => infer_member_access_type(m, doc, enums, registry, cache),
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
                UnaryOp::Not => {
                    if is_page_prev_expr(&u.operand) {
                        return Ok(StaticType::Boolean);
                    }
                    match op_ty.decay() {
                        StaticType::Boolean
                        | StaticType::Timer
                        | StaticType::IndefiniteTimer
                        | StaticType::Handler => Ok(StaticType::Boolean),
                        other => Err(CompileError::TypeMismatch {
                            expected: "Boolean or Timer".to_string(),
                            actual: other.type_name(),
                            span: u.span,
                        }),
                    }
                }
            }
        }
        Expr::Binary(bin) => infer_binary_type(bin, doc, enums, registry, cache),
        Expr::Ternary(tern) => infer_ternary_type(tern, doc, enums, registry, cache),
        Expr::Call(call) => infer_call_type(call, doc, enums, registry, cache),
        Expr::MethodCall(mc) => {
            let target_ty = infer_expr_type(&mc.target, doc, enums, registry, cache)?;
            for arg in &mc.args {
                let _ = infer_expr_type(arg, doc, enums, registry, cache)?;
            }
            match mc.method.as_str() {
                "append" | "to_fixed" | "to_precision" => Ok(StaticType::String),
                name if DistanceUnit::from_conversion_fn(name).is_some()
                    || matches!(name, "to_float" | "to_number") =>
                {
                    Ok(StaticType::Float)
                }
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
        Expr::Node(elem) => infer_node_expr_type(elem, doc, enums, registry, cache),
        Expr::ExprDef(_) => Ok(StaticType::Expr),
        Expr::Match(m) => infer_match_type(m, doc, enums, registry, cache),
        Expr::Index(idx) => {
            let target_ty = infer_expr_type(&idx.target, doc, enums, registry, cache)?;
            let index_ty = infer_expr_type(&idx.index, doc, enums, registry, cache)?;
            let valid_index = matches!(
                index_ty,
                StaticType::Integer | StaticType::ZeroInteger
            ) || matches!(&index_ty, StaticType::Node(n) if n == "Page")
                || doc
                    .page_template_component
                    .as_ref()
                    .is_some_and(|tmpl| matches!(&index_ty, StaticType::Node(n) if n == tmpl));
            if !valid_index {
                return Err(CompileError::TypeMismatch {
                    expected: "Integer or Page".to_string(),
                    actual: index_ty.type_name(),
                    span: idx.index.span(),
                });
            }
            let bucket_nid = crate::compiler::expand::builtins::pagination::resolve_bucket_node_id(
                &idx.target,
                doc,
            );
            match &target_ty {
                StaticType::Node(n) if n == "PageValue" => {
                    Ok(StaticType::PageValueSlice(bucket_nid))
                }
                StaticType::Node(n) if n == "PageList" => Ok(StaticType::PageListSlice(bucket_nid)),
                other => Err(CompileError::TypeMismatch {
                    expected: "PageValue or PageList".to_string(),
                    actual: other.type_name(),
                    span: idx.target.span(),
                }),
            }
        }
    }
}

#[inline(never)]
fn infer_member_access_type(
    m: &MemberAccessExpr,
    doc: &ExpandedDocument,
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
    cache: &mut TypecheckCache<'_>,
) -> Result<StaticType, CompileError> {
    if let Expr::Ident(target_id) = m.target.as_ref() {
        if let Some(enum_def) = enums.get(target_id.as_str()) {
            if enum_def.has_variant(m.member.as_str()) {
                return Ok(StaticType::Enum(target_id.as_str().to_string()));
            }
        }
        if let Some(raw_node_id) = target_id.node_id() {
            let node_id = if raw_node_id.is_template_self() {
                if let Some(template_types) = cache.template_types {
                    if let Some(ty) = template_types.get(&m.member.name) {
                        return Ok(ty.clone());
                    }
                    return Err(CompileError::Custom {
                        message: format!(
                            "Template port '{}' has no known static type",
                            m.member.as_str()
                        ),
                        span: m.span,
                    });
                }
                if let Some(self_node) = cache.current_self_node {
                    self_node
                } else {
                    return Err(CompileError::Custom {
                        message: format!(
                            "Template port '{}' has no known static type",
                            m.member.as_str()
                        ),
                        span: m.span,
                    });
                }
            } else {
                raw_node_id
            };
            let has_port = if node_id.is_window() {
                doc.window_ports.contains_key(m.member.as_str())
                    || super::rewrite::has_materialized_expr_port(
                        node_id,
                        m.member.as_str(),
                    )
                    || known_builtin_port_type("Window", m.member.as_str()).is_some()
            } else if node_id.is_page() {
                doc.page_ports.contains_key(m.member.as_str())
                    || super::rewrite::has_materialized_expr_port(
                        node_id,
                        m.member.as_str(),
                    )
                    || known_builtin_port_type("Page", m.member.as_str()).is_some()
            } else if let Some(node) = doc.get_node(node_id) {
                node.ports.contains_key(m.member.as_str())
                    || node.port_types.contains_key(m.member.as_str())
                    || super::rewrite::has_materialized_expr_port(
                        node_id,
                        m.member.as_str(),
                    )
                    || (!node.is_component
                        && known_builtin_port_type(&node.name, m.member.as_str()).is_some())
            } else {
                super::rewrite::has_materialized_expr_port(node_id, m.member.as_str())
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
    if is_page_prev_expr(&m.target) {
        let member_name = m.member.as_str();
        if doc.page_template_private_ports.contains(member_name) {
            return Err(CompileError::Custom {
                message: format!(
                    "Cannot access private property '{}' on 'page.prev'; expose it via \\alias",
                    member_name
                ),
                span: m.span,
            });
        }
        if let Some(tmpl_node) = doc.page_template_component.as_ref().and_then(|tname| {
            doc.nodes
                .iter()
                .find(|n| n.is_component && &n.name == tname)
        }) {
            let sym = m.member.name;
            if cache.visiting.contains(&(tmpl_node.id, sym)) {
                if let Some(ty_name) = tmpl_node.port_types.get(member_name) {
                    return StaticType::from_type_name(
                        ty_name,
                        m.span,
                        enums,
                        registry,
                    );
                }
                return Ok(StaticType::PagePrevRecursive);
            }
            if tmpl_node.ports.contains_key(member_name)
                || tmpl_node.port_types.contains_key(member_name)
            {
                return infer_port_type(
                    tmpl_node.id,
                    sym,
                    member_name,
                    doc,
                    enums,
                    registry,
                    cache,
                );
            }
            return Err(CompileError::Custom {
                message: format!(
                    "Page template '{}' has no public property '{}'",
                    tmpl_node.name, member_name
                ),
                span: m.span,
            });
        }
    }
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
        StaticType::Timer => {
            let prop_ty = match m.member.as_str() {
                "time" | "duration" | "dt" => StaticType::Duration,
                "elapsed" => StaticType::Float,
                "frames" => StaticType::Integer,
                _ => {
                    return Err(CompileError::Custom {
                        message: format!(
                            "Node 'Timer' has no public property '{}'",
                            m.member
                        ),
                        span: m.span,
                    });
                }
            };
            if is_nullable_timer_target(
                &m.target,
                &target_ty,
                doc,
                cache.current_self_node,
            ) {
                let is_guarded =
                    canonical_timer_key(&m.target, cache.current_self_node)
                        .is_some_and(|k| cache.guarded_timers.contains(&k));
                if !is_guarded {
                    return Err(CompileError::Custom {
                        message: format!(
                            "Cannot access property '{}' on unguarded 'Timer'; guard with 'timer ? ... : ...'",
                            m.member
                        ),
                        span: m.span,
                    });
                }
            }
            Ok(prop_ty)
        }
        StaticType::IndefiniteTimer => {
            let prop_ty = match m.member.as_str() {
                "time" | "dt" => StaticType::Duration,
                "frames" => StaticType::Integer,
                _ => {
                    return Err(CompileError::Custom {
                        message: format!(
                            "Node 'IndefiniteTimer' has no public property '{}'",
                            m.member
                        ),
                        span: m.span,
                    });
                }
            };
            if is_nullable_timer_target(
                &m.target,
                &target_ty,
                doc,
                cache.current_self_node,
            ) {
                let is_guarded =
                    canonical_timer_key(&m.target, cache.current_self_node)
                        .is_some_and(|k| cache.guarded_timers.contains(&k));
                if !is_guarded {
                    return Err(CompileError::Custom {
                        message: format!(
                            "Cannot access property '{}' on unguarded 'IndefiniteTimer'; guard with 'timer ? ... : ...'",
                            m.member
                        ),
                        span: m.span,
                    });
                }
            }
            Ok(prop_ty)
        }
        StaticType::PageValueSlice(bucket_nid) => match m.member.as_str() {
            "has_value" => Ok(StaticType::Boolean),
            "value" | "first" => Ok(StaticType::BucketPayload(bucket_nid)),
            other => Err(CompileError::Custom {
                message: format!("PageValue slice has no property '{other}'"),
                span: m.span,
            }),
        },
        StaticType::PageListSlice(bucket_nid) => match m.member.as_str() {
            "len" => Ok(StaticType::Integer),
            "first" | "last" => Ok(StaticType::BucketPayload(bucket_nid)),
            other => Err(CompileError::Custom {
                message: format!("PageList slice has no property '{other}'"),
                span: m.span,
            }),
        },
        StaticType::BucketPayload(bucket_nid) => {
            let member_name = m.member.as_str();
            if let Some(bid) = bucket_nid {
                if let Some(schema_map) = cache.bucket_schemas.get(&bid) {
                    if let Some(prop_ty) = schema_map.get(member_name) {
                        return Ok(prop_ty.clone());
                    }
                    return Err(CompileError::Custom {
                        message: format!(
                            "Bucket payload has no public property '{member_name}'"
                        ),
                        span: m.span,
                    });
                }
            }
            if cache.template_types.is_some() {
                return Err(CompileError::Custom {
                    message: "Deferred bucket payload typecheck".to_string(),
                    span: m.span,
                });
            }
            // Empty bucket (0 deposits in document): short-circuited under guard
            Ok(StaticType::PagePrevRecursive)
        }
        StaticType::PagePrevRecursive => Ok(StaticType::PagePrevRecursive),
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

#[inline(never)]
fn infer_binary_type(
    bin: &BinaryExpr,
    doc: &ExpandedDocument,
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
    cache: &mut TypecheckCache<'_>,
) -> Result<StaticType, CompileError> {
    let left_ty = infer_expr_type(&bin.left, doc, enums, registry, cache)?;
    let right_ty = if matches!(bin.op, BinaryOp::And | BinaryOp::Or) {
        let (l_pos, l_neg) =
            extract_timer_guards(&bin.left, doc, enums, registry, cache, 0);
        let saved = cache.guarded_timers.clone();
        if bin.op == BinaryOp::And {
            cache.guarded_timers.extend(l_pos);
        } else {
            cache.guarded_timers.extend(l_neg);
        }
        let res = infer_expr_type(&bin.right, doc, enums, registry, cache);
        cache.guarded_timers = saved;
        res?
    } else {
        infer_expr_type(&bin.right, doc, enums, registry, cache)?
    };
    match bin.op {
        BinaryOp::Add => match (&left_ty, &right_ty) {
            (StaticType::PagePrevRecursive, other)
            | (other, StaticType::PagePrevRecursive) => Ok(other.clone()),
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
            (StaticType::PagePrevRecursive, other)
            | (other, StaticType::PagePrevRecursive) => Ok(other.clone()),
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
            (StaticType::PagePrevRecursive, _) | (_, StaticType::PagePrevRecursive) => {
                Ok(StaticType::PagePrevRecursive)
            }
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
            (StaticType::PagePrevRecursive, _) | (_, StaticType::PagePrevRecursive) => {
                Ok(StaticType::PagePrevRecursive)
            }
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
            (StaticType::PagePrevRecursive, other)
            | (other, StaticType::PagePrevRecursive) => Ok(other.clone()),
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
                (StaticType::PagePrevRecursive, _) | (_, StaticType::PagePrevRecursive) => {
                    Ok(StaticType::Boolean)
                }
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
            if (is_page_prev_expr(&bin.left) && matches!(right_ty, StaticType::Unset))
                || (is_page_prev_expr(&bin.right) && matches!(left_ty, StaticType::Unset))
            {
                Ok(StaticType::Boolean)
            } else if left_ty.matches(&right_ty) || right_ty.matches(&left_ty) {
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
            if !is_page_prev_expr(&bin.left)
                && !matches!(
                    left_ty,
                    StaticType::Boolean
                        | StaticType::Timer
                        | StaticType::IndefiniteTimer
                        | StaticType::Handler
                        | StaticType::Unset
                )
            {
                return Err(CompileError::TypeMismatch {
                    expected: "Boolean".to_string(),
                    actual: left_ty.type_name(),
                    span: bin.left.span(),
                });
            }
            if !is_page_prev_expr(&bin.right)
                && !matches!(
                    right_ty,
                    StaticType::Boolean
                        | StaticType::Timer
                        | StaticType::IndefiniteTimer
                        | StaticType::Handler
                        | StaticType::Unset
                )
            {
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

#[inline(never)]
fn infer_ternary_type(
    tern: &TernaryExpr,
    doc: &ExpandedDocument,
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
    cache: &mut TypecheckCache<'_>,
) -> Result<StaticType, CompileError> {
    let cond_ty = infer_expr_type(&tern.condition, doc, enums, registry, cache)?;
    if !is_page_prev_expr(&tern.condition)
        && !matches!(
            cond_ty,
            StaticType::Boolean
                | StaticType::Timer
                | StaticType::IndefiniteTimer
                | StaticType::Handler
                | StaticType::Unset
        )
    {
        return Err(CompileError::TypeMismatch {
            expected: "Boolean".to_string(),
            actual: cond_ty.type_name(),
            span: tern.condition.span(),
        });
    }
    let (pos_guards, neg_guards) =
        extract_timer_guards(&tern.condition, doc, enums, registry, cache, 0);
    let saved_guards = cache.guarded_timers.clone();
    cache.guarded_timers.extend(pos_guards);
    let then_res = infer_expr_type(&tern.then_expr, doc, enums, registry, cache);
    cache.guarded_timers.clone_from(&saved_guards);
    let then_ty = then_res?;

    cache.guarded_timers.extend(neg_guards);
    let else_res = infer_expr_type(&tern.else_expr, doc, enums, registry, cache);
    cache.guarded_timers = saved_guards;
    let else_ty = else_res?;
    if matches!(then_ty, StaticType::PagePrevRecursive) {
        Ok(else_ty.decay())
    } else if matches!(else_ty, StaticType::PagePrevRecursive) {
        Ok(then_ty.decay())
    } else if then_ty == else_ty {
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

#[inline(never)]
fn infer_call_type(
    call: &CallExpr,
    doc: &ExpandedDocument,
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
    cache: &mut TypecheckCache<'_>,
) -> Result<StaticType, CompileError> {
    match call.callee.as_str() {
        name if DistanceUnit::from_conversion_fn(name).is_some()
            || matches!(name, "distance_to_float" | "to_float" | "to_number") =>
        {
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
        | "__image_intrinsic_width"
        | "__image_intrinsic_height"
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
        "__image_aspect_ratio" | "__image_file_dpi" => {
            for arg in &call.args {
                let _ = infer_expr_type(arg, doc, enums, registry, cache)?;
            }
            Ok(StaticType::Float)
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

#[inline(never)]
fn infer_node_expr_type(
    elem: &ElementNode,
    doc: &ExpandedDocument,
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
    cache: &mut TypecheckCache<'_>,
) -> Result<StaticType, CompileError> {
    match elem.name.as_str() {
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
    }
}

#[inline(never)]
fn infer_match_type(
    m: &MatchExpr,
    doc: &ExpandedDocument,
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
    cache: &mut TypecheckCache<'_>,
) -> Result<StaticType, CompileError> {
    let mut acc_ty: Option<StaticType> = None;
    let saved_guards = cache.guarded_timers.clone();
    let mut acc_neg_guards: HashSet<(NodeId, Symbol)> = HashSet::new();
    for arm in &m.arms {
        let mut arm_pos_guards = HashSet::new();
        let mut arm_neg_guards = HashSet::new();
        if let MatchPattern::Condition(c) = &arm.pattern {
            cache.guarded_timers.clone_from(&saved_guards);
            cache.guarded_timers.extend(acc_neg_guards.iter().copied());
            let c_res = infer_expr_type(c, doc, enums, registry, cache);
            cache.guarded_timers.clone_from(&saved_guards);
            let c_ty = c_res?;
            if !matches!(
                c_ty,
                StaticType::Boolean
                    | StaticType::Timer
                    | StaticType::IndefiniteTimer
                    | StaticType::Handler
                    | StaticType::Unset
            ) {
                return Err(CompileError::TypeMismatch {
                    expected: "Boolean".to_string(),
                    actual: c_ty.type_name(),
                    span: c.span(),
                });
            }
            let (pos, neg) = extract_timer_guards(c, doc, enums, registry, cache, 0);
            arm_pos_guards = pos;
            arm_neg_guards = neg;
        }
        cache.guarded_timers.clone_from(&saved_guards);
        cache.guarded_timers.extend(acc_neg_guards.iter().copied());
        cache.guarded_timers.extend(arm_pos_guards);
        let arm_res = infer_expr_type(&arm.body, doc, enums, registry, cache);
        cache.guarded_timers.clone_from(&saved_guards);
        let arm_ty = arm_res?;
        acc_neg_guards.extend(arm_neg_guards);
        if let Some(prev_ty) = &acc_ty {
            if arm_ty == *prev_ty || arm_ty.matches(prev_ty) {
                // keep prev_ty
            } else if prev_ty.matches(&arm_ty) {
                acc_ty = Some(arm_ty.decay());
            } else {
                return Err(CompileError::TypeMismatch {
                    expected: prev_ty.type_name(),
                    actual: arm_ty.type_name(),
                    span: arm.body.span(),
                });
            }
        } else {
            acc_ty = Some(arm_ty);
        }
    }
    Ok(acc_ty.unwrap_or(StaticType::Integer).decay())
}

pub(super) fn is_page_prev_expr(expr: &Expr) -> bool {
    let mut cur = expr;
    while let Expr::Paren(inner, _) = cur {
        cur = inner;
    }
    let Expr::MemberAccess(m) = cur else {
        return false;
    };
    if m.member.as_str() != "prev" {
        return false;
    }
    let mut tgt = m.target.as_ref();
    while let Expr::Paren(inner, _) = tgt {
        tgt = inner;
    }
    matches!(tgt, Expr::Ident(id) if id.node_id().is_some_and(|nid| nid.is_page()) || id.as_str() == "page")
}

fn is_page_port(expr: &Expr, port: &str) -> bool {
    let mut cur = expr;
    while let Expr::Paren(inner, _) = cur {
        cur = inner;
    }
    let Expr::MemberAccess(m) = cur else {
        return false;
    };
    if m.member.as_str() != port {
        return false;
    }
    let mut tgt = m.target.as_ref();
    while let Expr::Paren(inner, _) = tgt {
        tgt = inner;
    }
    matches!(tgt, Expr::Ident(id) if id.node_id().is_some_and(|nid| nid.is_page()) || id.as_str() == "page")
}

fn is_unset_expr(expr: &Expr) -> bool {
    let mut cur = expr;
    while let Expr::Paren(inner, _) = cur {
        cur = inner;
    }
    matches!(cur, Expr::Ident(id) if id.name == Symbol::UNSET)
}

fn literal_int_val(expr: &Expr) -> Option<i64> {
    let mut cur = expr;
    while let Expr::Paren(inner, _) = cur {
        cur = inner;
    }
    match cur {
        Expr::Literal(Literal::Integer(n, _)) => Some(*n),
        _ => None,
    }
}

fn literal_bool_val(expr: &Expr) -> Option<bool> {
    let mut cur = expr;
    while let Expr::Paren(inner, _) = cur {
        cur = inner;
    }
    match cur {
        Expr::Literal(Literal::Bool(b, _)) => Some(*b),
        _ => None,
    }
}

/// Returns `(then_guarded, else_guarded)` for `page.prev` access safety within a boolean condition.
fn extract_page_prev_guards(cond: &Expr, prev_guarded: bool) -> Result<(bool, bool), CompileError> {
    let cond = unwrap_parens(cond);
    if is_page_prev_expr(cond) {
        return Ok((true, false));
    }
    if is_page_port(cond, "is_first") {
        return Ok((false, true));
    }
    match cond {
        Expr::Unary(u) if u.op == UnaryOp::Not => {
            let (t, e) = extract_page_prev_guards(&u.operand, prev_guarded)?;
            Ok((e, t))
        }
        Expr::Binary(bin) => match bin.op {
            BinaryOp::Eq => {
                if (is_page_prev_expr(&bin.left) && is_unset_expr(&bin.right))
                    || (is_page_prev_expr(&bin.right) && is_unset_expr(&bin.left))
                {
                    return Ok((false, true));
                }
                if (is_page_port(&bin.left, "is_first") && literal_bool_val(&bin.right) == Some(false))
                    || (is_page_port(&bin.right, "is_first") && literal_bool_val(&bin.left) == Some(false))
                {
                    return Ok((true, false));
                }
                if (is_page_port(&bin.left, "is_first") && literal_bool_val(&bin.right) == Some(true))
                    || (is_page_port(&bin.right, "is_first") && literal_bool_val(&bin.left) == Some(true))
                {
                    return Ok((false, true));
                }
                if (is_page_port(&bin.left, "number") && literal_int_val(&bin.right) == Some(1))
                    || (is_page_port(&bin.right, "number") && literal_int_val(&bin.left) == Some(1))
                {
                    return Ok((false, true));
                }
                validate_page_prev_guards(&bin.left, prev_guarded)?;
                validate_page_prev_guards(&bin.right, prev_guarded)?;
                Ok((false, false))
            }
            BinaryOp::Ne => {
                if (is_page_prev_expr(&bin.left) && is_unset_expr(&bin.right))
                    || (is_page_prev_expr(&bin.right) && is_unset_expr(&bin.left))
                {
                    return Ok((true, false));
                }
                if (is_page_port(&bin.left, "is_first") && literal_bool_val(&bin.right) == Some(true))
                    || (is_page_port(&bin.right, "is_first") && literal_bool_val(&bin.left) == Some(true))
                {
                    return Ok((true, false));
                }
                if (is_page_port(&bin.left, "is_first") && literal_bool_val(&bin.right) == Some(false))
                    || (is_page_port(&bin.right, "is_first") && literal_bool_val(&bin.left) == Some(false))
                {
                    return Ok((false, true));
                }
                if (is_page_port(&bin.left, "number") && literal_int_val(&bin.right) == Some(1))
                    || (is_page_port(&bin.right, "number") && literal_int_val(&bin.left) == Some(1))
                {
                    return Ok((true, false));
                }
                validate_page_prev_guards(&bin.left, prev_guarded)?;
                validate_page_prev_guards(&bin.right, prev_guarded)?;
                Ok((false, false))
            }
            BinaryOp::Gt => {
                if (is_page_port(&bin.left, "number") && literal_int_val(&bin.right).is_some_and(|n| n >= 1))
                    || (is_page_port(&bin.right, "number") && literal_int_val(&bin.left).is_some_and(|n| n <= 1))
                {
                    return Ok((true, false));
                }
                validate_page_prev_guards(&bin.left, prev_guarded)?;
                validate_page_prev_guards(&bin.right, prev_guarded)?;
                Ok((false, false))
            }
            BinaryOp::Ge => {
                if (is_page_port(&bin.left, "number") && literal_int_val(&bin.right).is_some_and(|n| n >= 2))
                    || (is_page_port(&bin.right, "number") && literal_int_val(&bin.left).is_some_and(|n| n <= 1))
                {
                    return Ok((true, false));
                }
                validate_page_prev_guards(&bin.left, prev_guarded)?;
                validate_page_prev_guards(&bin.right, prev_guarded)?;
                Ok((false, false))
            }
            BinaryOp::Lt => {
                if (is_page_port(&bin.right, "number") && literal_int_val(&bin.left).is_some_and(|n| n >= 1))
                    || (is_page_port(&bin.left, "number") && literal_int_val(&bin.right).is_some_and(|n| n <= 2))
                {
                    let then_g = is_page_port(&bin.right, "number");
                    return Ok((then_g, !then_g));
                }
                validate_page_prev_guards(&bin.left, prev_guarded)?;
                validate_page_prev_guards(&bin.right, prev_guarded)?;
                Ok((false, false))
            }
            BinaryOp::Le => {
                if (is_page_port(&bin.right, "number") && literal_int_val(&bin.left).is_some_and(|n| n >= 2))
                    || (is_page_port(&bin.left, "number") && literal_int_val(&bin.right).is_some_and(|n| n <= 1))
                {
                    let then_g = is_page_port(&bin.right, "number");
                    return Ok((then_g, !then_g));
                }
                validate_page_prev_guards(&bin.left, prev_guarded)?;
                validate_page_prev_guards(&bin.right, prev_guarded)?;
                Ok((false, false))
            }
            BinaryOp::And => {
                let (l_pos, l_neg) = extract_page_prev_guards(&bin.left, prev_guarded)?;
                let (r_pos, r_neg) =
                    extract_page_prev_guards(&bin.right, prev_guarded || l_pos)?;
                Ok((l_pos || r_pos, l_neg && r_neg))
            }
            BinaryOp::Or => {
                let (l_pos, l_neg) = extract_page_prev_guards(&bin.left, prev_guarded)?;
                let (r_pos, r_neg) =
                    extract_page_prev_guards(&bin.right, prev_guarded || l_neg)?;
                Ok((l_pos && r_pos, l_neg || r_neg))
            }
            _ => {
                validate_page_prev_guards(&bin.left, prev_guarded)?;
                validate_page_prev_guards(&bin.right, prev_guarded)?;
                Ok((false, false))
            }
        },
        _ => {
            validate_page_prev_guards(cond, prev_guarded)?;
            Ok((false, false))
        }
    }
}

pub(super) fn validate_page_prev_guards(
    expr: &Expr,
    prev_guarded: bool,
) -> Result<(), CompileError> {
    match expr {
        Expr::Literal(_) | Expr::Ident(_) | Expr::DomNodeRef(_, _) => Ok(()),
        Expr::Paren(inner, _) => validate_page_prev_guards(inner, prev_guarded),
        Expr::MemberAccess(m) => {
            if is_page_prev_expr(&m.target) && !prev_guarded {
                return Err(CompileError::UnguardedPagePrevAccess {
                    port: m.member.as_str().to_string(),
                    span: m.span,
                });
            }
            if let Expr::Index(idx) = unwrap_parens(&m.target) {
                if is_page_prev_expr(&idx.index)
                    && matches!(m.member.as_str(), "value" | "first" | "last")
                    && !prev_guarded
                {
                    return Err(CompileError::UnguardedPagePrevAccess {
                        port: m.member.as_str().to_string(),
                        span: m.span,
                    });
                }
            }
            validate_page_prev_guards(&m.target, prev_guarded)
        }
        Expr::Index(idx) => {
            validate_page_prev_guards(&idx.target, prev_guarded)?;
            validate_page_prev_guards(&idx.index, prev_guarded)
        }
        Expr::Unary(u) => {
            if u.op == UnaryOp::Not {
                let _ = extract_page_prev_guards(expr, prev_guarded)?;
                Ok(())
            } else {
                validate_page_prev_guards(&u.operand, prev_guarded)
            }
        }
        Expr::Binary(b) => {
            if matches!(
                b.op,
                BinaryOp::And
                    | BinaryOp::Or
                    | BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Gt
                    | BinaryOp::Ge
                    | BinaryOp::Lt
                    | BinaryOp::Le
            ) {
                let _ = extract_page_prev_guards(expr, prev_guarded)?;
                Ok(())
            } else {
                validate_page_prev_guards(&b.left, prev_guarded)?;
                validate_page_prev_guards(&b.right, prev_guarded)
            }
        }
        Expr::Ternary(t) => {
            let (pos_g, neg_g) = extract_page_prev_guards(&t.condition, prev_guarded)?;
            validate_page_prev_guards(&t.then_expr, prev_guarded || pos_g)?;
            validate_page_prev_guards(&t.else_expr, prev_guarded || neg_g)
        }
        Expr::Call(c) => {
            for arg in &c.args {
                validate_page_prev_guards(arg, prev_guarded)?;
            }
            Ok(())
        }
        Expr::MethodCall(mc) => {
            validate_page_prev_guards(&mc.target, prev_guarded)?;
            for arg in &mc.args {
                validate_page_prev_guards(arg, prev_guarded)?;
            }
            Ok(())
        }
        Expr::ExprDef(def) => {
            for p in &def.params {
                if let Some(d) = &p.default_edge {
                    validate_page_prev_guards(d, prev_guarded)?;
                }
            }
            validate_page_prev_guards(&def.body, prev_guarded)
        }
        Expr::Match(m) => {
            if let Some(scrutinee) = &m.scrutinee {
                validate_page_prev_guards(scrutinee, prev_guarded)?;
                let scrutinee_is_page_prev = is_page_prev_expr(scrutinee);
                for arm in &m.arms {
                    let mut arm_guarded = prev_guarded;
                    if let MatchPattern::Condition(cond) = &arm.pattern {
                        validate_page_prev_guards(cond, prev_guarded)?;
                        let (pos_g, _) = extract_page_prev_guards(cond, prev_guarded)?;
                        arm_guarded |= pos_g;
                    } else if scrutinee_is_page_prev
                        && !matches!(arm.pattern, MatchPattern::Wildcard(_))
                    {
                        arm_guarded = true;
                    }
                    validate_page_prev_guards(&arm.body, arm_guarded)?;
                }
            } else {
                for arm in &m.arms {
                    let mut arm_guarded = prev_guarded;
                    if let MatchPattern::Condition(cond) = &arm.pattern {
                        validate_page_prev_guards(cond, prev_guarded)?;
                        let (pos_g, _) = extract_page_prev_guards(cond, prev_guarded)?;
                        arm_guarded |= pos_g;
                    }
                    validate_page_prev_guards(&arm.body, arm_guarded)?;
                }
            }
            Ok(())
        }
        Expr::Node(elem) => {
            for p in &elem.ports {
                validate_page_prev_guards(&p.expr, prev_guarded)?;
            }
            Ok(())
        }
    }
}

fn infer_deposit_elem_public_ports(
    elem: &ElementNode,
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> Result<HashMap<String, StaticType>, CompileError> {
    let mut map: HashMap<String, StaticType> = HashMap::new();
    let tag = elem.name.as_str();
    if let Some(overloads) = registry.get(tag) {
        let mut matching: Vec<&ComponentDef> = overloads
            .iter()
            .filter(|def| {
                let Ok(schema) = super::schema::ComponentSchema::for_def(def) else {
                    return false;
                };
                elem.ports.iter().all(|p| {
                    let pname = p.name.as_str();
                    pname == "clip"
                        || pname == "pagination"
                        || pname.starts_with("on_")
                        || schema.param_names.contains(pname)
                })
            })
            .collect();
        if matching.is_empty() {
            matching.extend(overloads.iter());
        }
        let mut combined: Option<HashMap<String, StaticType>> = None;
        for def in matching {
            let schema = super::schema::ComponentSchema::for_def(def)?;
            let mut overload_map: HashMap<String, StaticType> = HashMap::new();
            for p in &def.params {
                let pname = p.name.as_str();
                if let Some(ty_str) = schema.static_port_types.get(pname) {
                    if let Ok(st) = StaticType::from_type_name(ty_str, p.span, enums, registry) {
                        overload_map.insert(pname.to_string(), st);
                    }
                }
            }
            for (aname, alias_b) in &schema.declared_aliases {
                if schema.private_ports.contains(aname) {
                    continue;
                }
                let ty = if let Some(ty_str) = schema.static_port_types.get(aname) {
                    StaticType::from_type_name(ty_str, alias_b.span, enums, registry)?
                } else if let Some(ann) = &alias_b.type_annotation {
                    StaticType::from_type_name(ann.name.as_str(), ann.span, enums, registry)?
                } else if matches!(
                    aname.as_str(),
                    "x" | "y" | "width" | "height" | "left" | "top" | "right" | "bottom"
                ) {
                    StaticType::Distance
                } else {
                    StaticType::PagePrevRecursive
                };
                overload_map.insert(aname.clone(), ty);
            }
            for (pname, ty_str) in schema.static_port_types.iter() {
                if !schema.private_ports.contains(pname) && !overload_map.contains_key(pname) {
                    if let Ok(st) =
                        StaticType::from_type_name(ty_str, elem.span, enums, registry)
                    {
                        overload_map.insert(pname.clone(), st);
                    }
                }
            }
            for sp in ["left", "top", "right", "bottom"] {
                if !schema.private_ports.contains(sp)
                    && (schema.comp_scope_ports.iter().any(|p| p == sp)
                        || overload_map.contains_key("x")
                        || overload_map.contains_key("width"))
                {
                    overload_map.entry(sp.to_string()).or_insert(StaticType::Distance);
                }
            }
            for pg_port in ["page", "end_page"] {
                if !schema.private_ports.contains(pg_port) {
                    overload_map.insert(pg_port.to_string(), StaticType::Integer);
                }
            }

            combined = Some(match combined {
                None => overload_map,
                Some(mut prev) => {
                    prev.retain(|k, ty| {
                        overload_map.get(k).is_some_and(|other| {
                            ty == other || ty.matches(other) || other.matches(ty)
                        })
                    });
                    prev
                }
            });
        }
        if let Some(m) = combined {
            map = m;
        }
    } else if let Some(prim) = super::builtins::lookup_builtin(tag, true) {
        let candidate_ports = [
            "x", "y", "width", "height", "left", "top", "right", "bottom", "z", "opacity",
            "color", "text", "align", "indent", "paragraph_style", "size", "weight", "family",
            "italic", "small_caps", "baseline", "page", "end_page",
        ];
        for pname in candidate_ports {
            if matches!(pname, "left" | "top" | "right" | "bottom") && !prim.is_non_spatial() {
                map.insert(pname.to_string(), StaticType::Distance);
            } else if matches!(pname, "page" | "end_page") {
                map.insert(pname.to_string(), StaticType::Integer);
            } else if prim.accepts_port(pname) {
                if let Some(ty) = prim.port_type(pname) {
                    map.insert(pname.to_string(), ty);
                }
            }
        }
    }
    Ok(map)
}

fn infer_bucket_schemas(
    doc: &ExpandedDocument,
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> Result<HashMap<NodeId, HashMap<String, StaticType>>, CompileError> {
    let mut schemas: HashMap<NodeId, HashMap<String, StaticType>> = HashMap::new();
    for dep in &doc.deposits {
        let elem_map = infer_deposit_elem_public_ports(&dep.payload.elem, enums, registry)?;
        match schemas.entry(dep.bucket_id) {
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(elem_map);
            }
            std::collections::hash_map::Entry::Occupied(mut o) => {
                let acc = o.get_mut();
                acc.retain(|k, ty| {
                    elem_map.get(k).is_some_and(|other| {
                        let t1 = ty.clone().decay();
                        let t2 = other.clone().decay();
                        t1 == t2 || t1.matches(&t2) || t2.matches(&t1)
                    })
                });
            }
        }
    }
    Ok(schemas)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum BucketIndexKey {
    Page,
    Node(NodeId),
    Int(i64),
    ExprText(String),
}

fn canonical_bucket_index_key(expr: &Expr) -> BucketIndexKey {
    let mut cur = expr;
    while let Expr::Paren(inner, _) = cur {
        cur = inner;
    }
    match cur {
        Expr::Ident(id) => {
            if let Some(nid) = id.node_id() {
                if nid.is_page() {
                    BucketIndexKey::Page
                } else {
                    BucketIndexKey::Node(nid)
                }
            } else {
                BucketIndexKey::ExprText(id.as_str().to_string())
            }
        }
        Expr::Literal(Literal::Integer(i, _)) => BucketIndexKey::Int(*i),
        _ => BucketIndexKey::ExprText(format!("{cur}")),
    }
}

fn resolve_bucket_slice_member<'a>(
    expr: &'a Expr,
    doc: &ExpandedDocument,
) -> Option<(NodeId, BucketIndexKey, bool, &'a str)> {
    let mut cur = expr;
    while let Expr::Paren(inner, _) = cur {
        cur = inner;
    }
    let Expr::MemberAccess(m) = cur else {
        return None;
    };
    let mut inner_tgt = m.target.as_ref();
    while let Expr::Paren(inner, _) = inner_tgt {
        inner_tgt = inner;
    }
    let Expr::Index(idx) = inner_tgt else {
        return None;
    };
    let bucket_id =
        crate::compiler::expand::builtins::pagination::resolve_bucket_node_id(&idx.target, doc)?;
    let is_list = doc.get_node(bucket_id)?.name == "PageList";
    let key = canonical_bucket_index_key(&idx.index);
    Some((bucket_id, key, is_list, m.member.as_str()))
}

fn extract_bucket_guards(
    cond: &Expr,
    doc: &ExpandedDocument,
    guarded: &HashSet<(NodeId, BucketIndexKey, bool)>,
    depth: usize,
) -> Result<(Vec<(NodeId, BucketIndexKey, bool)>, Vec<(NodeId, BucketIndexKey, bool)>), CompileError>
{
    let cond = unwrap_parens(cond);
    if depth < 8 {
        if let Expr::MemberAccess(m) = cond {
            if !matches!(unwrap_parens(&m.target), Expr::Index(_)) {
                if let Expr::Ident(id) = unwrap_parens(&m.target) {
                    if let Some(nid) = id.node_id() {
                        if !nid.is_window() && !nid.is_page() {
                            if let Some(port_expr) =
                                doc.get_node(nid).and_then(|n| n.ports.get(m.member.as_str()))
                            {
                                return extract_bucket_guards(port_expr, doc, guarded, depth + 1);
                            }
                        }
                    }
                }
            }
        }
    }

    if let Some((bid, key, is_list, mem)) = resolve_bucket_slice_member(cond, doc) {
        if !is_list && mem == "has_value" {
            return Ok((vec![(bid, key, false)], Vec::new()));
        }
        validate_bucket_guards(cond, doc, guarded)?;
        return Ok((Vec::new(), Vec::new()));
    }

    match cond {
        Expr::Unary(u) if matches!(u.op, UnaryOp::Not) => {
            let (p, n) = extract_bucket_guards(&u.operand, doc, guarded, depth + 1)?;
            Ok((n, p))
        }
        Expr::Binary(b) => match b.op {
            BinaryOp::And => {
                let (lp, ln) = extract_bucket_guards(&b.left, doc, guarded, depth + 1)?;
                let mut next_guarded = guarded.clone();
                next_guarded.extend(lp.iter().cloned());
                let (rp, rn) = extract_bucket_guards(&b.right, doc, &next_guarded, depth + 1)?;
                let mut pos = lp;
                pos.extend(rp);
                let _ = (ln, rn);
                Ok((pos, Vec::new()))
            }
            BinaryOp::Or => {
                let (_lp, ln) = extract_bucket_guards(&b.left, doc, guarded, depth + 1)?;
                let mut next_neg = guarded.clone();
                next_neg.extend(ln.iter().cloned());
                let (_rp, rn) = extract_bucket_guards(&b.right, doc, &next_neg, depth + 1)?;
                let mut neg = ln;
                neg.extend(rn);
                Ok((Vec::new(), neg))
            }
            BinaryOp::Eq | BinaryOp::Ne => {
                let check_bool_has_value = |lhs: &Expr, rhs: &Expr| {
                    let (bid, key, is_list, mem) = resolve_bucket_slice_member(lhs, doc)?;
                    if !is_list && mem == "has_value" {
                        if let Expr::Literal(Literal::Bool(v, _)) = unwrap_parens(rhs) {
                            return Some((bid, key, *v));
                        }
                    }
                    None
                };
                if let Some((bid, key, expected_bool)) =
                    check_bool_has_value(&b.left, &b.right)
                        .or_else(|| check_bool_has_value(&b.right, &b.left))
                {
                    let is_true_when_eq = expected_bool;
                    let true_in_pos = if b.op == BinaryOp::Eq {
                        is_true_when_eq
                    } else {
                        !is_true_when_eq
                    };
                    return if true_in_pos {
                        Ok((vec![(bid, key, false)], Vec::new()))
                    } else {
                        Ok((Vec::new(), vec![(bid, key, false)]))
                    };
                }
                let check_len_zero = |lhs: &Expr, rhs: &Expr| {
                    let (bid, key, is_list, mem) = resolve_bucket_slice_member(lhs, doc)?;
                    if is_list && mem == "len" && literal_int_val(rhs) == Some(0) {
                        return Some((bid, key));
                    }
                    None
                };
                if let Some((bid, key)) =
                    check_len_zero(&b.left, &b.right).or_else(|| check_len_zero(&b.right, &b.left))
                {
                    return if b.op == BinaryOp::Ne {
                        Ok((vec![(bid, key, true)], Vec::new()))
                    } else {
                        Ok((Vec::new(), vec![(bid, key, true)]))
                    };
                }
                validate_bucket_guards(&b.left, doc, guarded)?;
                validate_bucket_guards(&b.right, doc, guarded)?;
                Ok((Vec::new(), Vec::new()))
            }
            BinaryOp::Gt | BinaryOp::Ge | BinaryOp::Lt | BinaryOp::Le => {
                let lhs_len = resolve_bucket_slice_member(&b.left, doc)
                    .filter(|(_, _, is_list, mem)| *is_list && *mem == "len");
                let rhs_len = resolve_bucket_slice_member(&b.right, doc)
                    .filter(|(_, _, is_list, mem)| *is_list && *mem == "len");
                if let (Some((bid, key, _, _)), Some(v)) = (lhs_len, literal_int_val(&b.right)) {
                    let guarantees_nonempty = match b.op {
                        BinaryOp::Gt => v >= 0,
                        BinaryOp::Ge => v >= 1,
                        _ => false,
                    };
                    let else_guarantees_nonempty = match b.op {
                        BinaryOp::Le => v <= 0,
                        BinaryOp::Lt => v <= 1,
                        _ => false,
                    };
                    if guarantees_nonempty {
                        return Ok((vec![(bid, key, true)], Vec::new()));
                    }
                    if else_guarantees_nonempty {
                        return Ok((Vec::new(), vec![(bid, key, true)]));
                    }
                }
                if let (Some(v), Some((bid, key, _, _))) = (literal_int_val(&b.left), rhs_len) {
                    let guarantees_nonempty = match b.op {
                        BinaryOp::Lt => v >= 0,
                        BinaryOp::Le => v >= 1,
                        _ => false,
                    };
                    let else_guarantees_nonempty = match b.op {
                        BinaryOp::Ge => v <= 0,
                        BinaryOp::Gt => v <= 1,
                        _ => false,
                    };
                    if guarantees_nonempty {
                        return Ok((vec![(bid, key, true)], Vec::new()));
                    }
                    if else_guarantees_nonempty {
                        return Ok((Vec::new(), vec![(bid, key, true)]));
                    }
                }
                validate_bucket_guards(&b.left, doc, guarded)?;
                validate_bucket_guards(&b.right, doc, guarded)?;
                Ok((Vec::new(), Vec::new()))
            }
            _ => {
                validate_bucket_guards(&b.left, doc, guarded)?;
                validate_bucket_guards(&b.right, doc, guarded)?;
                Ok((Vec::new(), Vec::new()))
            }
        },
        _ => {
            validate_bucket_guards(cond, doc, guarded)?;
            Ok((Vec::new(), Vec::new()))
        }
    }
}

fn validate_bucket_guards(
    expr: &Expr,
    doc: &ExpandedDocument,
    guarded: &HashSet<(NodeId, BucketIndexKey, bool)>,
) -> Result<(), CompileError> {
    match expr {
        Expr::Literal(_) | Expr::Ident(_) | Expr::DomNodeRef(_, _) => Ok(()),
        Expr::Paren(inner, _) => validate_bucket_guards(inner, doc, guarded),
        Expr::Unary(u) => validate_bucket_guards(&u.operand, doc, guarded),
        Expr::MemberAccess(m) => {
            if let Some((bid, key, is_list, mem)) = resolve_bucket_slice_member(expr, doc) {
                let needs_guard = if is_list {
                    matches!(mem, "first" | "last")
                } else {
                    matches!(mem, "value" | "first")
                };
                if needs_guard && !guarded.contains(&(bid, key, is_list)) {
                    let expected_guard = if is_list {
                        format!("bucket[...].len > 0 ? ... : ...")
                    } else {
                        format!("bucket[...].has_value ? ... : ...")
                    };
                    return Err(CompileError::UnguardedBucketAccess {
                        access: format!(".{mem}"),
                        expected_guard,
                        span: m.span,
                    });
                }
            }
            validate_bucket_guards(&m.target, doc, guarded)
        }
        Expr::Index(idx) => {
            validate_bucket_guards(&idx.target, doc, guarded)?;
            validate_bucket_guards(&idx.index, doc, guarded)
        }
        Expr::Binary(b) => {
            if matches!(
                b.op,
                BinaryOp::And
                    | BinaryOp::Or
                    | BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Gt
                    | BinaryOp::Ge
                    | BinaryOp::Lt
                    | BinaryOp::Le
            ) {
                let _ = extract_bucket_guards(expr, doc, guarded, 0)?;
                Ok(())
            } else {
                validate_bucket_guards(&b.left, doc, guarded)?;
                validate_bucket_guards(&b.right, doc, guarded)
            }
        }
        Expr::Ternary(t) => {
            let (pos_g, neg_g) = extract_bucket_guards(&t.condition, doc, guarded, 0)?;
            let mut then_guarded = guarded.clone();
            then_guarded.extend(pos_g);
            validate_bucket_guards(&t.then_expr, doc, &then_guarded)?;

            let mut else_guarded = guarded.clone();
            else_guarded.extend(neg_g);
            validate_bucket_guards(&t.else_expr, doc, &else_guarded)
        }
        Expr::Call(c) => {
            for arg in &c.args {
                validate_bucket_guards(arg, doc, guarded)?;
            }
            Ok(())
        }
        Expr::MethodCall(mc) => {
            validate_bucket_guards(&mc.target, doc, guarded)?;
            for arg in &mc.args {
                validate_bucket_guards(arg, doc, guarded)?;
            }
            Ok(())
        }
        Expr::ExprDef(def) => {
            for p in &def.params {
                if let Some(d) = &p.default_edge {
                    validate_bucket_guards(d, doc, guarded)?;
                }
            }
            validate_bucket_guards(&def.body, doc, guarded)
        }
        Expr::Match(m) => {
            if let Some(s) = &m.scrutinee {
                validate_bucket_guards(s, doc, guarded)?;
            }
            for arm in &m.arms {
                let mut arm_guarded = guarded.clone();
                if let MatchPattern::Condition(c) = &arm.pattern {
                    validate_bucket_guards(c, doc, guarded)?;
                    let (pos_g, _) = extract_bucket_guards(c, doc, guarded, 0)?;
                    arm_guarded.extend(pos_g);
                }
                validate_bucket_guards(&arm.body, doc, &arm_guarded)?;
            }
            Ok(())
        }
        Expr::Node(elem) => {
            for p in &elem.ports {
                validate_bucket_guards(&p.expr, doc, guarded)?;
            }
            Ok(())
        }
    }
}

