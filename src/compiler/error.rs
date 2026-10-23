use crate::compiler::expanded::NodeId;
use crate::compiler::graph::{GraphNodeInfo, VarId};
use crate::span::Span;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq)]
pub enum CompileError {
    #[error("Undefined component '{name}'{}", .span.format_location_suffix())]
    UndefinedComponent {
        name: String,
        span: Span,
    },

    #[error("Property '{port}' is reserved on node '{node}'{}", .span.format_location_suffix())]
    ReservedPort {
        node: String,
        port: String,
        span: Span,
    },

    #[error("Node '{node}' is missing required property '{port}'{}", .span.format_location_suffix())]
    MissingPort {
        node: String,
        port: String,
        span: Span,
    },

    #[error("Property '{port}' defined multiple times on node '{node}'{}", .span.format_location_suffix())]
    DuplicatePort {
        node: String,
        port: String,
        span: Span,
    },

    #[error("Cyclic dependency detected in layout graph:\n{}", format_cycle_path(.cycle, .node_info))]
    CyclicDependency {
        cycle: Vec<VarId>,
        node_info: HashMap<NodeId, GraphNodeInfo>,
        span: Span,
    },

    #[error("Attempted to use explicitly uninitialized property '{name}'{}", .span.format_location_suffix())]
    UninitializedVariableUse {
        name: String,
        span: Span,
    },

    #[error("Undefined environmental property '{name}'{}", .span.format_location_suffix())]
    UndefinedEnvVariable {
        name: String,
        span: Span,
    },

    #[error("Environmental property '{name}' is firewalled/swallowed and cannot be accessed{}", .span.format_location_suffix())]
    BlockedEnvVariable {
        name: String,
        span: Span,
    },

    #[error("Cannot use bare 'env' as an expression; access an environmental property via 'env.<name>'{}", .span.format_location_suffix())]
    BareEnvUse {
        span: Span,
    },

    #[error("Cannot override immutable alias property '{port}' on node '{node}'{}", .span.format_location_suffix())]
    ImmutableAliasPort {
        node: String,
        port: String,
        span: Span,
    },

    #[error("Component key on '{node}' cannot depend on layout property or formula '{name}'{}", .span.format_location_suffix())]
    InvalidComponentKeyDependency {
        node: String,
        name: String,
        span: Span,
    },

    #[error("Property '{port}' on node '{node}' is a state property and cannot be set by caller{}", .span.format_location_suffix())]
    PrivateStatePort {
        node: String,
        port: String,
        span: Span,
    },

    #[error("Import failed for '{path}'{}: {message}", .span.format_location_suffix())]
    ImportError {
        path: String,
        message: String,
        span: Span,
    },

    #[error("Cyclic import detected for '{path}'{}", .span.format_location_suffix())]
    CyclicImport {
        path: String,
        span: Span,
    },

    #[error("Assertion failed: {message}{}", .span.format_location_suffix())]
    AssertionFailed {
        message: String,
        span: Span,
    },

    #[error("Unresolved identifier '{name}'. Did you forget to import it?{}", .span.format_location_suffix())]
    UnresolvedIdentifier {
        name: String,
        span: Span,
    },

    #[error("Node '{node:?}' not found in compiled document{}", .span.format_location_suffix())]
    NodeNotFound {
        node: crate::compiler::expanded::NodeId,
        span: Span,
    },

    #[error("Property '{var}' on node '{node}' is not a declared state property{}", .span.format_location_suffix())]
    NotAStateVariable {
        node: String,
        var: String,
        span: Span,
    },

    #[error("Type mismatch: expected '{expected}', found '{actual}'{}", .span.format_location_suffix())]
    TypeMismatch {
        expected: String,
        actual: String,
        span: Span,
    },

    #[error("Unknown variant '{variant}' on enum '{enum_name}'{}", .span.format_location_suffix())]
    UnknownEnumVariant {
        enum_name: String,
        variant: String,
        span: Span,
    },

    #[error("Cannot use bare enum '{name}' as an expression; use '{name}.<Variant>'{}", .span.format_location_suffix())]
    BareEnumUse {
        name: String,
        span: Span,
    },

    #[error("Duplicate enum declaration '{name}'{}", .span.format_location_suffix())]
    DuplicateEnum {
        name: String,
        span: Span,
    },

    #[error("Potentially ambiguous overloads for component '{}': signatures {:?} and {:?} both accept properties {:?}{}", .0.name, .0.signature_a, .0.signature_b, .0.witness_overlap, .0.span.format_location_suffix())]
    PotentiallyAmbiguousOverloads(Box<AmbiguousOverloadDetails>),

    #[error("No matching overload for component '{}' with provided properties {:?}{}", .0.name, .0.provided_ports, .0.span.format_location_suffix())]
    NoMatchingOverload(Box<NoMatchingOverloadDetails>),

    #[error("Duplicate overload signature for component '{name}' with properties {signature:?}{}", .span.format_location_suffix())]
    DuplicateOverloadSignature {
        name: String,
        signature: Vec<String>,
        span: Span,
        second_span: Span,
    },

    #[error("Font family '{family}' not found{}", .span.format_location_suffix())]
    FontFamilyNotFound {
        family: String,
        span: Span,
    },

    #[error("Duplicate \\Window declaration{}", .span.format_location_suffix())]
    DuplicateWindowDeclaration {
        span: Span,
        second_span: Span,
    },

    #[error("\\Window declaration cannot be nested inside another element or component{}", .span.format_location_suffix())]
    NestedWindowDeclaration {
        span: Span,
    },

    #[error("\\Window declaration does not accept children{}", .span.format_location_suffix())]
    WindowContentNotAllowed {
        span: Span,
    },

    #[error("Disallowed property '{port}' on \\Window. Ambient spatial geometry and arbitrary custom properties cannot be set on \\Window; only viewer runtime control properties ('title', 'dpi', 'vscroll', 'hscroll') are supported{}", .span.format_location_suffix())]
    DisallowedWindowPort {
        port: String,
        span: Span,
    },

    #[error("{message}{}", .span.format_location_suffix())]
    Custom {
        message: String,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct AmbiguousOverloadDetails {
    pub name: String,
    pub signature_a: Vec<String>,
    pub signature_b: Vec<String>,
    pub witness_overlap: Vec<String>,
    pub span: Span,
    pub second_span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NoMatchingOverloadDetails {
    pub name: String,
    pub provided_ports: Vec<String>,
    pub available_signatures: Vec<Vec<String>>,
    pub span: Span,
}

impl CompileError {
    pub fn span(&self) -> Span {
        match self {
            CompileError::UndefinedComponent { span, .. }
            | CompileError::ReservedPort { span, .. }
            | CompileError::MissingPort { span, .. }
            | CompileError::DuplicatePort { span, .. }
            | CompileError::ImmutableAliasPort { span, .. }
            | CompileError::InvalidComponentKeyDependency { span, .. }
            | CompileError::PrivateStatePort { span, .. }
            | CompileError::ImportError { span, .. }
            | CompileError::CyclicImport { span, .. }
            | CompileError::NodeNotFound { span, .. }
            | CompileError::NotAStateVariable { span, .. }
            | CompileError::TypeMismatch { span, .. }
            | CompileError::UnknownEnumVariant { span, .. }
            | CompileError::BareEnumUse { span, .. }
            | CompileError::DuplicateEnum { span, .. }
            | CompileError::CyclicDependency { span, .. }
            | CompileError::UninitializedVariableUse { span, .. }
            | CompileError::UndefinedEnvVariable { span, .. }
            | CompileError::BlockedEnvVariable { span, .. }
            | CompileError::BareEnvUse { span, .. }
            | CompileError::DuplicateOverloadSignature { span, .. }
            | CompileError::FontFamilyNotFound { span, .. }
            | CompileError::DuplicateWindowDeclaration { span, .. }
            | CompileError::NestedWindowDeclaration { span, .. }
            | CompileError::WindowContentNotAllowed { span, .. }
            | CompileError::DisallowedWindowPort { span, .. }
            | CompileError::AssertionFailed { span, .. }
            | CompileError::UnresolvedIdentifier { span, .. }
            | CompileError::Custom { span, .. } => *span,
            CompileError::PotentiallyAmbiguousOverloads(details) => details.span,
            CompileError::NoMatchingOverload(details) => details.span,
        }
    }
}

fn format_cycle_path(cycle: &[VarId], node_info: &HashMap<NodeId, GraphNodeInfo>) -> String {
    cycle
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let prefix = if i == 0 { "     " } else { "  -> " };
            match node_info.get(&v.node) {
                Some(info) => {
                    let var_label = if !info.name.is_empty() {
                        if v.node.is_window() {
                            format!("{}.{}", info.name, v.port)
                        } else {
                            format!("{}#{}.{}", info.name, v.node.0, v.port)
                        }
                    } else {
                        v.to_string()
                    };
                    let loc_suffix = info.span.format_location_suffix();
                    format!("{prefix}{var_label}{loc_suffix}")
                }
                None => format!("{prefix}{v}"),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
