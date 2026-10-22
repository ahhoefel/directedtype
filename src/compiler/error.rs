use crate::compiler::expanded::NodeId;
use crate::compiler::graph::{GraphNodeInfo, VarId};
use crate::span::Span;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq)]
pub enum CompileError {
    #[error("Undefined component '{name}'")]
    UndefinedComponent {
        name: String,
        span: Span,
    },

    #[error("Port '{port}' is reserved on node '{node}'")]
    ReservedPort {
        node: String,
        port: String,
        span: Span,
    },

    #[error("Node '{node}' is missing required port '{port}'")]
    MissingPort {
        node: String,
        port: String,
        span: Span,
    },

    #[error("Port '{port}' defined multiple times on node '{node}'")]
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

    #[error("Attempted to use explicitly uninitialized variable '{name}'")]
    UninitializedVariableUse {
        name: String,
        span: Span,
    },

    #[error("Undefined environmental variable '{name}'")]
    UndefinedEnvVariable {
        name: String,
        span: Span,
    },

    #[error("Environmental variable '{name}' is firewalled/swallowed and cannot be accessed")]
    BlockedEnvVariable {
        name: String,
        span: Span,
    },

    #[error("Cannot use bare 'env' as an expression; access an environmental variable via 'env.<name>'")]
    BareEnvUse {
        span: Span,
    },

    #[error("Cannot override immutable alias port '{port}' on node '{node}'")]
    ImmutableAliasPort {
        node: String,
        port: String,
        span: Span,
    },

    #[error("Component key on '{node}' cannot depend on layout port or formula '{name}'")]
    InvalidComponentKeyDependency {
        node: String,
        name: String,
        span: Span,
    },

    #[error("Port '{port}' on node '{node}' is private component state and cannot be set by caller")]
    PrivateStatePort {
        node: String,
        port: String,
        span: Span,
    },

    #[error("Import failed for '{path}': {message}")]
    ImportError {
        path: String,
        message: String,
        span: Span,
    },

    #[error("Cyclic import detected for '{path}'")]
    CyclicImport {
        path: String,
        span: Span,
    },

    #[error("Assertion failed: {message}")]
    AssertionFailed {
        message: String,
        span: Span,
    },

    #[error("Unresolved identifier '{name}'. Did you forget to import it?")]
    UnresolvedIdentifier {
        name: String,
        span: Span,
    },

    #[error("Node '{node:?}' not found in compiled document")]
    NodeNotFound {
        node: crate::compiler::expanded::NodeId,
        span: Span,
    },

    #[error("Variable '{var}' on node '{node}' is not a declared state variable")]
    NotAStateVariable {
        node: String,
        var: String,
        span: Span,
    },

    #[error("Type mismatch: expected '{expected}', found '{actual}'")]
    TypeMismatch {
        expected: String,
        actual: String,
        span: Span,
    },

    #[error("Unknown variant '{variant}' on enum '{enum_name}'")]
    UnknownEnumVariant {
        enum_name: String,
        variant: String,
        span: Span,
    },

    #[error("Cannot use bare enum '{name}' as an expression; use '{name}.<Variant>'")]
    BareEnumUse {
        name: String,
        span: Span,
    },

    #[error("Duplicate enum declaration '{name}'")]
    DuplicateEnum {
        name: String,
        span: Span,
    },

    #[error("Potentially ambiguous overloads for component '{}': signatures {:?} and {:?} both accept ports {:?}", .0.name, .0.signature_a, .0.signature_b, .0.witness_overlap)]
    PotentiallyAmbiguousOverloads(Box<AmbiguousOverloadDetails>),

    #[error("No matching overload for component '{}' with provided ports {:?}", .0.name, .0.provided_ports)]
    NoMatchingOverload(Box<NoMatchingOverloadDetails>),

    #[error("Duplicate overload signature for component '{name}' with ports {signature:?}")]
    DuplicateOverloadSignature {
        name: String,
        signature: Vec<String>,
        span: Span,
        second_span: Span,
    },

    #[error("Font family '{family}' not found")]
    FontFamilyNotFound {
        family: String,
        span: Span,
    },

    #[error("Duplicate \\Window declaration")]
    DuplicateWindowDeclaration {
        span: Span,
        second_span: Span,
    },

    #[error("\\Window declaration cannot be nested inside another element or component")]
    NestedWindowDeclaration {
        span: Span,
    },

    #[error("\\Window declaration does not accept children")]
    WindowContentNotAllowed {
        span: Span,
    },

    #[error("Disallowed port '{port}' on \\Window. Ambient spatial geometry and arbitrary custom ports cannot be set on \\Window; only viewer runtime control ports ('title', 'dpi', 'vscroll', 'hscroll') are supported")]
    DisallowedWindowPort {
        port: String,
        span: Span,
    },

    #[error("{message}")]
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
                    let loc_suffix = if info.span.line > 0 {
                        match info.span.file_name() {
                            Some(file) => {
                                format!(" ({}:{}:{})", file, info.span.line, info.span.column)
                            }
                            None => format!(" ({}:{})", info.span.line, info.span.column),
                        }
                    } else {
                        String::new()
                    };
                    format!("{prefix}{var_label}{loc_suffix}")
                }
                None => format!("{prefix}{v}"),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
