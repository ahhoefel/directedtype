use crate::ast::{
    ComponentDef, EnumDef, Expr, Ident, Literal, MemberAccessExpr,
};
use crate::compiler::error::CompileError;
use crate::compiler::expand::builtins::{
    default_builtin_port_type, BuiltinChildren, BuiltinDefault, BuiltinPrimitive,
    PrimitiveExpandState,
};
use crate::compiler::expand::typecheck::StaticType;
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

const PARAGRAPH_STYLE_BASE_DEFAULTS: &[(&str, BuiltinDefault)] = &[
    ("align", BuiltinDefault::Enum("Align", "Left")),
    ("indent", BuiltinDefault::Px(0.0)),
    ("line_break", BuiltinDefault::Enum("LineBreak", "Greedy")),
    ("word_spacing", BuiltinDefault::Px(0.0)),
    ("space_stretch", BuiltinDefault::Float(0.5)),
    ("space_shrink", BuiltinDefault::Float(1.0 / 3.0)),
    ("tolerance", BuiltinDefault::Float(2.0)),
    ("hyphen_penalty", BuiltinDefault::Float(50.0)),
    ("line_penalty", BuiltinDefault::Float(10.0)),
    ("flagged_demerits", BuiltinDefault::Float(3000.0)),
    ("fitness_demerits", BuiltinDefault::Float(3000.0)),
    ("sentence_space", BuiltinDefault::Float(0.0)),
    ("looseness", BuiltinDefault::Integer(0)),
];

const PARAGRAPH_STYLE_DERIVED_DEFAULTS: &[(&str, BuiltinDefault)] = &[
    ("align", BuiltinDefault::Expr("paragraph_style.align")),
    ("indent", BuiltinDefault::Expr("paragraph_style.indent")),
    ("line_break", BuiltinDefault::Expr("paragraph_style.line_break")),
    ("word_spacing", BuiltinDefault::Expr("paragraph_style.word_spacing")),
    ("space_stretch", BuiltinDefault::Expr("paragraph_style.space_stretch")),
    ("space_shrink", BuiltinDefault::Expr("paragraph_style.space_shrink")),
    ("tolerance", BuiltinDefault::Expr("paragraph_style.tolerance")),
    ("hyphen_penalty", BuiltinDefault::Expr("paragraph_style.hyphen_penalty")),
    ("line_penalty", BuiltinDefault::Expr("paragraph_style.line_penalty")),
    ("flagged_demerits", BuiltinDefault::Expr("paragraph_style.flagged_demerits")),
    ("fitness_demerits", BuiltinDefault::Expr("paragraph_style.fitness_demerits")),
    ("sentence_space", BuiltinDefault::Expr("paragraph_style.sentence_space")),
    ("looseness", BuiltinDefault::Expr("paragraph_style.looseness")),
];

pub fn default_paragraph_style_port_expr(port_name: &str, span: Span) -> Expr {
    PARAGRAPH_STYLE_BASE_DEFAULTS
        .iter()
        .find(|(name, _)| *name == port_name)
        .and_then(|(_, def)| def.to_literal_expr(span))
        .unwrap_or_else(|| Expr::Literal(Literal::Float(0.0, span)))
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

    fn required_port_sets(&self) -> &'static [&'static [&'static str]] {
        &[&[], &["paragraph_style"]]
    }

    fn default_port_sets(&self) -> &'static [&'static [(&'static str, BuiltinDefault)]] {
        &[PARAGRAPH_STYLE_BASE_DEFAULTS, PARAGRAPH_STYLE_DERIVED_DEFAULTS]
    }

    fn children_cardinality(&self) -> BuiltinChildren {
        BuiltinChildren::None
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
