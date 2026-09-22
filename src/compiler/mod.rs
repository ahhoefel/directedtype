pub mod error;
pub mod eval;
pub mod expand;
pub mod expanded;
pub mod graph;
pub mod layout;
pub mod topo;
pub mod value;

pub use error::CompileError;
pub use eval::evaluate_graph;
pub use expand::expand_document;
pub use expanded::{ExpandedDocument, ExpandedNode, NodeId};
pub use graph::{build_variable_graph, VarId, VariableGraph, VariableNode};
pub use layout::{resolve_layout, Rect, ResolvedNode, ResolvedLayout};
pub use topo::{sort_graph, TopologicalSchedule};
pub use value::Value;

use crate::ast::Document;

/// High-level compiler helper: expands an AST document and builds the flat variable dependency graph.
pub fn compile_to_graph(doc: &Document) -> Result<(ExpandedDocument, VariableGraph), CompileError> {
    let expanded = expand_document(doc)?;
    let graph = build_variable_graph(&expanded)?;
    Ok((expanded, graph))
}

/// High-level compiler helper: parses, expands, builds the graph, and computes the topological schedule.
pub fn compile_and_sort(
    doc: &Document,
) -> Result<(ExpandedDocument, VariableGraph, TopologicalSchedule), CompileError> {
    let (expanded, graph) = compile_to_graph(doc)?;
    let schedule = sort_graph(&graph)?;
    Ok((expanded, graph, schedule))
}

/// The complete end-to-end Phase 2 layout pipeline:
/// expands AST -> builds variable graph -> checks cycles / sorts topologically -> evaluates layout math -> produces ResolvedLayout.
pub fn evaluate_document(doc: &Document) -> Result<ResolvedLayout, CompileError> {
    let (expanded, graph, schedule) = compile_and_sort(doc)?;
    let values = evaluate_graph(&graph, &schedule)?;
    let layout = resolve_layout(&expanded, values);
    Ok(layout)
}
