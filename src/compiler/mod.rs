pub mod error;
pub mod eval;
pub mod expand;
pub mod expanded;
pub mod graph;
pub mod layout;
pub mod text;
pub mod topo;
pub mod value;

pub use error::CompileError;
pub use eval::evaluate_graph;
pub use expand::expand_document;
pub use expanded::{ExpandedDocument, ExpandedNode, NodeId};
pub use graph::{build_variable_graph, build_variable_graph_with_window, VarId, VariableGraph, VariableNode};
pub use layout::{resolve_layout, Rect, ResolvedNode, ResolvedLayout};
pub use topo::{sort_graph, TopologicalSchedule};
pub use value::Value;

use crate::ast::Document;

/// High-level compiler helper: expands an AST document and builds the flat variable dependency graph
/// using default window dimensions (800x600).
pub fn compile_to_graph(doc: &Document) -> Result<(ExpandedDocument, VariableGraph), CompileError> {
    compile_to_graph_with_window(doc, 800.0, 600.0)
}

/// High-level compiler helper: expands an AST document and builds the flat variable dependency graph
/// with explicit window dimensions.
pub fn compile_to_graph_with_window(
    doc: &Document,
    window_width: f64,
    window_height: f64,
) -> Result<(ExpandedDocument, VariableGraph), CompileError> {
    let expanded = expand_document(doc)?;
    let graph = build_variable_graph_with_window(&expanded, window_width, window_height)?;
    Ok((expanded, graph))
}

/// High-level compiler helper: parses, expands, builds the graph, and computes the topological schedule
/// using default window dimensions (800x600).
pub fn compile_and_sort(
    doc: &Document,
) -> Result<(ExpandedDocument, VariableGraph, TopologicalSchedule), CompileError> {
    compile_and_sort_with_window(doc, 800.0, 600.0)
}

/// High-level compiler helper: parses, expands, builds the graph, and computes the topological schedule
/// with explicit window dimensions.
pub fn compile_and_sort_with_window(
    doc: &Document,
    window_width: f64,
    window_height: f64,
) -> Result<(ExpandedDocument, VariableGraph, TopologicalSchedule), CompileError> {
    let (expanded, graph) = compile_to_graph_with_window(doc, window_width, window_height)?;
    let schedule = sort_graph(&graph)?;
    Ok((expanded, graph, schedule))
}

/// The complete end-to-end Phase 2 layout pipeline:
/// expands AST -> builds variable graph -> checks cycles / sorts topologically -> evaluates layout math -> produces ResolvedLayout
/// using default window dimensions (800x600).
pub fn evaluate_document(doc: &Document) -> Result<ResolvedLayout, CompileError> {
    evaluate_document_with_window(doc, 800.0, 600.0)
}

/// The complete end-to-end layout pipeline with explicit viewport / window dimensions:
/// expands AST -> builds variable graph -> checks cycles / sorts topologically -> evaluates layout math -> produces ResolvedLayout.
pub fn evaluate_document_with_window(
    doc: &Document,
    window_width: f64,
    window_height: f64,
) -> Result<ResolvedLayout, CompileError> {
    let (expanded, graph, schedule) = compile_and_sort_with_window(doc, window_width, window_height)?;
    let values = evaluate_graph(&graph, &schedule)?;
    let layout = resolve_layout(&expanded, values);
    Ok(layout)
}
