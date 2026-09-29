use crate::compiler::error::CompileError;
use crate::dom::handle::NodeHandle;
use crate::error::ParseError;

/// Errors that can occur during Component DOM manipulation and layout transactions.
#[derive(Debug, thiserror::Error)]
pub enum DomError {
    #[error("Invalid or stale node handle: {0:?}")]
    InvalidHandle(NodeHandle),

    #[error("Hierarchy cycle: node {0:?} cannot become a child of itself or one of its descendants")]
    HierarchyCycle(NodeHandle),

    #[error("Node {child:?} is not a child of parent {parent:?}")]
    NoSuchChild {
        parent: NodeHandle,
        child: NodeHandle,
    },

    #[error("Node {0:?} is not in the list of root nodes")]
    NotARoot(NodeHandle),

    #[error("Reference node {before:?} was not found in parent {parent:?}")]
    BeforeNodeNotFound {
        parent: NodeHandle,
        before: NodeHandle,
    },

    #[error("Reference node {0:?} was not found in root nodes")]
    BeforeRootNotFound(NodeHandle),

    #[error("Parse error while parsing DTML fragment: {0}")]
    Parse(#[from] ParseError),

    #[error("Compilation error during layout evaluation: {0}")]
    Compile(#[from] CompileError),

    #[error("Transaction error: {0}")]
    TransactionError(String),
}
