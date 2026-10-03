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

/// A fully compiled and topologically scheduled DirectedType document with a reactive state store
/// and active typed companion component instances.
///
/// Enables microsecond state mutations, event dispatching, and incremental DAG invalidations.
#[derive(Debug)]
pub struct CompiledDocument {
    pub expanded: ExpandedDocument,
    pub graph: VariableGraph,
    pub schedule: TopologicalSchedule,
    pub layout: ResolvedLayout,
    pub state_overrides: HashMap<VarId, Value>,
    pub instances: crate::component::InstanceManager,
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
            instances: crate::component::InstanceManager::new(),
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
            instances: crate::component::InstanceManager::new(),
        })
    }

    /// Compiles an AST document with a `ComponentRegistry`, instantiating companion components
    /// and executing their `on_mount` lifecycle hooks.
    pub fn compile_with_registry<R: FileResolver>(
        doc: &Document,
        window_width: f64,
        window_height: f64,
        base_dir: &Path,
        resolver: &R,
        registry: &crate::component::ComponentRegistry,
    ) -> Result<Self, CompileError> {
        let mut compiled = Self::compile_with_resolver(doc, window_width, window_height, base_dir, resolver)?;
        compiled.attach_registry(registry)?;
        Ok(compiled)
    }

    /// Attaches a `ComponentRegistry` to this compiled document, instantiating matching components
    /// and invoking their `on_mount` lifecycle hooks.
    pub fn attach_registry(
        &mut self,
        registry: &crate::component::ComponentRegistry,
    ) -> Result<(), CompileError> {
        use crate::component::Context;

        let mut initial_mutations = Vec::new();

        for node in &self.expanded.nodes {
            if let Some(mut instance) = registry.create_instance(&node.name) {
                let resolved = match self.layout.get_node(node.id) {
                    Some(r) => r,
                    None => continue,
                };
                let mut ctx = Context::new(
                    node.id,
                    node.parent,
                    node.key.as_ref(),
                    &resolved.properties,
                    &self.layout,
                );

                instance.on_mount(&mut ctx);
                let mutations = ctx.take_mutations();
                for (state_name, val) in mutations {
                    initial_mutations.push((node.id, state_name, val));
                }

                self.instances.insert(node.id, instance);
            }
        }

        // Apply any initial state mutations produced during on_mount
        for (node_id, state_name, val) in initial_mutations {
            let _ = self.set_state(node_id, &state_name, val);
        }

        Ok(())
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

    /// Dispatches an interaction event through the component hierarchy.
    ///
    /// The event bubbles up from the hit target along `bubble_path`. At each node,
    /// matching event handlers (e.g. `on_click: self.increment`) are invoked on the
    /// target component instance. Any state mutations emitted by component methods
    /// are batch-applied, triggering an incremental layout DAG update.
    pub fn dispatch_event(
        &mut self,
        event: &mut crate::interaction::Event,
    ) -> Result<HashSet<VarId>, crate::component::DispatchError> {
        use crate::component::{Context, DispatchError, EventHandlerTarget};
        use crate::interaction::EventKind;

        // 1. Populate bubble path if empty via spatial hit-testing
        if event.bubble_path.is_empty() {
            if let Some(hit) = self.layout.hit_test(event.global_point) {
                event.target = hit.target;
                event.local_point = hit.local_point;
                event.bubble_path = hit.bubble_path;
            } else {
                return Ok(HashSet::new());
            }
        }

        let event_handler_name = match &event.kind {
            EventKind::Click { .. } => "on_click",
            EventKind::PointerDown { .. } => "on_pointer_down",
            EventKind::PointerUp { .. } => "on_pointer_up",
            EventKind::PointerMove => "on_pointer_move",
            EventKind::PointerEnter => "on_pointer_enter",
            EventKind::PointerLeave => "on_pointer_leave",
            EventKind::Scroll { .. } => "on_scroll",
        };

        let mut all_changed_vars = HashSet::new();

        // 2. Bubble up the ancestor chain
        let bubble_nodes = event.bubble_path.clone();
        for node_id in bubble_nodes {
            if event.propagation_stopped {
                break;
            }

            let handler_opt = self
                .layout
                .get_node(node_id)
                .and_then(|n| n.event_handlers.get(event_handler_name).cloned());

            if let Some(handler) = handler_opt {
                match &handler.target {
                    EventHandlerTarget::Component(target_id) => {
                        let target_node_id = *target_id;
                        if let Some(component) = self.instances.get_mut(target_node_id) {
                            event.current_target = node_id;

                            let target_node = match self.layout.get_node(target_node_id) {
                                Some(n) => n,
                                None => continue,
                            };

                            let mut ctx = Context::new(
                                target_node_id,
                                target_node.parent,
                                target_node.key.as_ref(),
                                &target_node.properties,
                                &self.layout,
                            );

                            component.dispatch(&handler.method, event, &mut ctx)?;

                            let mutations = ctx.take_mutations();
                            for (state_name, val) in mutations {
                                if let Ok(changed) = self.set_state(target_node_id, &state_name, val) {
                                    all_changed_vars.extend(changed);
                                }
                            }
                        } else {
                            return Err(DispatchError::InstanceNotFound(target_node_id));
                        }
                    }
                    EventHandlerTarget::Env(service) => {
                        return Err(DispatchError::UnsupportedTarget(format!("env.{}", service)));
                    }
                }
            }
        }

        Ok(all_changed_vars)
    }

    /// Returns a reference to the active `InstanceManager`.
    pub fn instances(&self) -> &crate::component::InstanceManager {
        &self.instances
    }

    /// Returns a mutable reference to the active `InstanceManager`.
    pub fn instances_mut(&mut self) -> &mut crate::component::InstanceManager {
        &mut self.instances
    }
}
