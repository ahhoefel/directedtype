use crate::ast::{ComponentKey, Document};
use crate::compiler::error::CompileError;
use crate::compiler::eval::{evaluate_graph_with_state, invalidate_and_reevaluate};
use crate::compiler::expand::expand_document_with_resolver;
use crate::compiler::expanded::{ExpandedDocument, NodeId};
use crate::compiler::graph::{build_variable_graph_with_window, VarId, VariableGraph};
use crate::compiler::layout::{resolve_layout, update_resolved_layout, ResolvedLayout, ResolvedNode};
use crate::compiler::module::{FileResolver, FsResolver};
use crate::compiler::topo::{sort_graph, TopologicalSchedule};
use crate::compiler::value::Value;
use crate::span::Span;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// A fully compiled and topologically scheduled DirectedType document with a reactive state store.
///
/// Enables microsecond state mutations and incremental DAG invalidations without re-parsing,
/// re-expanding, or re-sorting the graph.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledDocument {
    pub expanded: ExpandedDocument,
    pub graph: VariableGraph,
    pub schedule: TopologicalSchedule,
    pub layout: ResolvedLayout,
    pub state_overrides: HashMap<VarId, Value>,
}

impl CompiledDocument {
    pub fn new(
        expanded: ExpandedDocument,
        graph: VariableGraph,
        schedule: TopologicalSchedule,
        layout: ResolvedLayout,
        state_overrides: HashMap<VarId, Value>,
    ) -> Self {
        Self {
            expanded,
            graph,
            schedule,
            layout,
            state_overrides,
        }
    }

    /// Compiles an AST document into an execution-ready `CompiledDocument` using default window dimensions (800x600).
    pub fn compile(doc: &Document) -> Result<Self, CompileError> {
        Self::compile_with_window(doc, 800.0, 600.0)
    }

    /// Compiles an AST document into an execution-ready `CompiledDocument` with explicit window dimensions.
    pub fn compile_with_window(
        doc: &Document,
        window_width: f64,
        window_height: f64,
    ) -> Result<Self, CompileError> {
        Self::compile_with_resolver(
            doc,
            window_width,
            window_height,
            Path::new("."),
            &FsResolver,
        )
    }

    /// Compiles an AST document with explicit window dimensions, base directory, and custom FileResolver.
    pub fn compile_with_resolver<R: FileResolver>(
        doc: &Document,
        window_width: f64,
        window_height: f64,
        base_dir: &Path,
        resolver: &R,
    ) -> Result<Self, CompileError> {
        let expanded = expand_document_with_resolver(doc, base_dir, resolver)?;
        let graph = build_variable_graph_with_window(&expanded, window_width, window_height)?;
        let schedule = sort_graph(&graph)?;
        let state_overrides = HashMap::new();
        let values = evaluate_graph_with_state(&graph, &schedule, &state_overrides)?;
        let layout = resolve_layout(&expanded, values);

        Ok(Self {
            expanded,
            graph,
            schedule,
            layout,
            state_overrides,
        })
    }

    /// Mutates a declared reactive state variable on a component instance, triggering an
    /// incremental topological re-evaluation of all downstream dependent variables in microseconds.
    ///
    /// Returns the set of `VarId`s whose computed values were updated.
    pub fn set_state(
        &mut self,
        node_id: NodeId,
        state_name: &str,
        value: Value,
    ) -> Result<HashSet<VarId>, CompileError> {
        let span = if node_id.is_window() {
            Span::default()
        } else {
            self.expanded
                .get_node(node_id)
                .map(|n| n.span)
                .ok_or(CompileError::NodeNotFound {
                    node: node_id,
                    span: Span::default(),
                })?
        };

        // 1. Verify that the variable is actually a declared state variable on the target
        let expected_type_opt: Option<String> = if node_id.is_window() {
            if !self.expanded.window_state_vars.contains_key(state_name) {
                return Err(CompileError::NotAStateVariable {
                    node: "window".to_string(),
                    var: state_name.to_string(),
                    span,
                });
            }
            self.expanded.window_state_vars.get(state_name).and_then(|opt| opt.clone())
        } else {
            let node = self.expanded.get_node(node_id).unwrap();
            if !node.state_vars.contains_key(state_name) {
                return Err(CompileError::NotAStateVariable {
                    node: node.name.clone(),
                    var: state_name.to_string(),
                    span,
                });
            }
            node.state_vars.get(state_name).and_then(|opt| opt.clone())
        };

        // 2. Validate type annotation if one was declared
        if let Some(expected_type) = expected_type_opt {
            if !value.matches_type_name(&expected_type) {
                return Err(CompileError::TypeMismatch {
                    expected: expected_type,
                    actual: value.type_name().to_string(),
                    span,
                });
            }
        }

        // 3. Inject new state value into the cell
        let var_id = VarId::new(node_id, state_name);
        self.state_overrides.insert(var_id.clone(), value);

        // 4. Incrementally re-evaluate downstream variables in the DAG
        let changed = invalidate_and_reevaluate(
            &self.graph,
            &self.schedule,
            &mut self.layout.values,
            &[var_id],
            &self.state_overrides,
        )?;

        // 5. Update layout nodes and structured keys
        update_resolved_layout(&mut self.layout, &self.expanded, &changed);

        Ok(changed)
    }

    /// Mutates a state variable on a child component identified by its structured identity key.
    pub fn set_state_by_key(
        &mut self,
        parent: Option<NodeId>,
        key: &ComponentKey,
        state_name: &str,
        value: Value,
    ) -> Result<HashSet<VarId>, CompileError> {
        let target_node_id = self
            .layout
            .find_by_key(parent, key)
            .map(|n| n.id)
            .ok_or_else(|| CompileError::Custom {
                message: format!("Component with structured key '{}' not found", key),
                span: key.span,
            })?;

        self.set_state(target_node_id, state_name, value)
    }

    /// Returns the active runtime value for a state variable (either overridden or initial).
    pub fn get_state(&self, node_id: NodeId, state_name: &str) -> Option<&Value> {
        self.state_overrides
            .get(&VarId::new(node_id, state_name))
            .or_else(|| self.layout.get_value(node_id, state_name))
    }

    /// Returns the current resolved layout.
    pub fn layout(&self) -> &ResolvedLayout {
        &self.layout
    }

    /// Finds a resolved node by its structured key.
    pub fn find_by_key(&self, parent: Option<NodeId>, key: &ComponentKey) -> Option<&ResolvedNode> {
        self.layout.find_by_key(parent, key)
    }
}
