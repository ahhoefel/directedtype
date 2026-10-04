pub mod compiled;
pub mod error;
pub mod eval;
pub mod expand;
pub mod expanded;
pub mod graph;
pub mod layout;
pub mod module;
pub mod text;
pub mod topo;
pub mod value;

pub use compiled::CompiledDocument;
pub use error::CompileError;
pub use eval::{
    evaluate_graph, evaluate_graph_with_state, find_downstream_dependents,
    invalidate_and_reevaluate,
};
pub use expand::{expand_document, expand_document_with_base_dir, expand_document_with_resolver};
pub use expanded::{ExpandedDocument, ExpandedNode, NodeId};
pub use graph::{
    build_variable_graph, build_variable_graph_with_window, VarId, VariableGraph, VariableNode,
};
pub use layout::{resolve_layout, update_resolved_layout, Rect, ResolvedLayout, ResolvedNode};
pub use module::{
    normalize_path, register_component_overload, resolve_imports, verify_overload_set,
    FileResolver, FsResolver, VirtualResolver,
};
pub use topo::{sort_graph, TopologicalSchedule};
pub use value::Value;

use crate::ast::Document;
use std::path::Path;

/// Compiles an AST document into an execution-ready `CompiledDocument` using default window dimensions (800x600).
pub fn compile_document(doc: &Document) -> Result<CompiledDocument, CompileError> {
    compile_document_with_window(doc, 800.0, 600.0)
}

/// Compiles an AST document into an execution-ready `CompiledDocument` with explicit window dimensions.
pub fn compile_document_with_window(
    doc: &Document,
    window_width: f64,
    window_height: f64,
) -> Result<CompiledDocument, CompileError> {
    CompiledDocument::compile_with_window(doc, window_width, window_height)
}

/// Compiles an AST document with explicit window dimensions, base directory, and custom FileResolver.
pub fn compile_document_with_resolver<R: FileResolver>(
    doc: &Document,
    window_width: f64,
    window_height: f64,
    base_dir: &Path,
    resolver: &R,
) -> Result<CompiledDocument, CompileError> {
    CompiledDocument::compile_with_resolver(doc, window_width, window_height, base_dir, resolver)
}

/// Compiles an AST document with explicit window dimensions, base directory, FileResolver, and ComponentRegistry.
pub fn compile_document_with_registry<R: FileResolver>(
    doc: &Document,
    window_width: f64,
    window_height: f64,
    base_dir: &Path,
    resolver: &R,
    registry: &crate::component::ComponentRegistry,
) -> Result<CompiledDocument, CompileError> {
    CompiledDocument::compile_with_registry(doc, window_width, window_height, base_dir, resolver, registry)
}

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

/// End-to-end layout pipeline with a custom FileResolver and base directory.
pub fn evaluate_document_with_resolver<R: FileResolver>(
    doc: &Document,
    window_width: f64,
    window_height: f64,
    base_dir: &Path,
    resolver: &R,
) -> Result<ResolvedLayout, CompileError> {
    let expanded = expand_document_with_resolver(doc, base_dir, resolver)?;
    let graph = build_variable_graph_with_window(&expanded, window_width, window_height)?;
    let schedule = sort_graph(&graph)?;
    let values = evaluate_graph(&graph, &schedule)?;
    let layout = resolve_layout(&expanded, values);
    Ok(layout)
}

/// End-to-end layout pipeline with a custom base directory using standard filesystem resolver.
pub fn evaluate_document_with_base_dir(
    doc: &Document,
    base_dir: &Path,
) -> Result<ResolvedLayout, CompileError> {
    evaluate_document_with_resolver(doc, 800.0, 600.0, base_dir, &FsResolver)
}
