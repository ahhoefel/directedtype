use crate::compiler::graph::VarId;
use crate::span::Span;
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

    #[error("Cyclic dependency detected in layout graph: {}", format_cycle_path(.cycle))]
    CyclicDependency {
        cycle: Vec<VarId>,
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

    #[error("Type mismatch for state variable: expected '{expected}', found '{actual}'")]
    TypeMismatch {
        expected: String,
        actual: String,
        span: Span,
    },

    #[error("{message}")]
    Custom {
        message: String,
        span: Span,
    },
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
            | CompileError::CyclicDependency { span, .. }
            | CompileError::UninitializedVariableUse { span, .. }
            | CompileError::UndefinedEnvVariable { span, .. }
            | CompileError::BlockedEnvVariable { span, .. }
            | CompileError::BareEnvUse { span, .. }
            | CompileError::Custom { span, .. } => *span,
        }
    }
}

fn format_cycle_path(cycle: &[VarId]) -> String {
    cycle
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(" -> ")
}
