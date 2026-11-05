use crate::ast::{ComponentKey, Document};
use crate::compiler::error::CompileError;
use crate::compiler::eval::{evaluate_graph_with_state, invalidate_and_reevaluate};
use crate::compiler::expand::expand_document_with_resolver;
use crate::compiler::expanded::{ExpandedDocument, NodeId};
use crate::compiler::graph::{build_variable_graph_with_window, VarId, VariableGraph};
use crate::compiler::layout::{
    resolve_layout_with_graph, update_resolved_layout, ResolvedLayout, ResolvedNode,
};
use crate::compiler::module::{FileResolver, FsResolver};
use crate::compiler::scope::ScopeId;
use crate::compiler::topo::{sort_graph, TopologicalSchedule};
use crate::compiler::value::Value;
use crate::component::{Context, DispatchError};
use crate::interaction::Event;
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
    pub graph: std::sync::Arc<VariableGraph>,
    pub schedule: TopologicalSchedule,
    pub layout: ResolvedLayout,
    pub state_overrides: HashMap<VarId, Value>,
    pub active_timers: HashSet<VarId>,
    pub instances: crate::component::InstanceManager,
    pub focused_node: Option<NodeId>,
    pub actions: Vec<crate::component::ContextAction>,
}

impl CompiledDocument {
    pub fn new(
        expanded: ExpandedDocument,
        graph: VariableGraph,
        schedule: TopologicalSchedule,
        layout: ResolvedLayout,
        state_overrides: HashMap<VarId, Value>,
    ) -> Self {
        let active_timers = Self::collect_active_timers(&layout.values);
        Self {
            expanded,
            graph: std::sync::Arc::new(graph),
            schedule,
            layout,
            state_overrides,
            active_timers,
            instances: crate::component::InstanceManager::new(),
            focused_node: None,
            actions: Vec::new(),
        }
    }

    fn collect_active_timers(values: &HashMap<VarId, Value>) -> HashSet<VarId> {
        values
            .iter()
            .filter_map(|(var_id, val)| match val {
                Value::Timer { time, duration, .. } if *duration > 0 && *time < *duration => {
                    Some(var_id.clone())
                }
                Value::IndefiniteTimer { .. } => Some(var_id.clone()),
                _ => None,
            })
            .collect()
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
        let graph = std::sync::Arc::new(build_variable_graph_with_window(
            &expanded,
            window_width,
            window_height,
        )?);
        let schedule = sort_graph(&graph)?;
        let state_overrides = HashMap::new();
        let values = evaluate_graph_with_state(&graph, &schedule, &state_overrides)?;
        let layout =
            resolve_layout_with_graph(&expanded, values, Some(std::sync::Arc::clone(&graph)));
        let active_timers = Self::collect_active_timers(&layout.values);

        Ok(Self {
            expanded,
            graph,
            schedule,
            layout,
            state_overrides,
            active_timers,
            instances: crate::component::InstanceManager::new(),
            focused_node: None,
            actions: Vec::new(),
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
            if let Some(mut instance) =
                registry.create_instance_for_node(&node.name, node.component_path.as_deref())
            {
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

                let actions = ctx.take_actions();
                self.actions.extend(actions);

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

        let var_id = VarId::new(node_id, state_name);

        // Coerce numeric/distance values to match the state variable's declared or inferred type
        let mut value = value;
        let target_is_distance = expected_type_opt.as_deref() == Some("Distance")
            || (expected_type_opt.is_none()
                && matches!(self.layout.values.get(&var_id), Some(Value::Distance(_))));
        let target_is_duration = expected_type_opt.as_deref() == Some("Duration")
            || (expected_type_opt.is_none()
                && matches!(self.layout.values.get(&var_id), Some(Value::Duration(_))));
        let target_is_float = expected_type_opt.as_deref() == Some("Float")
            || (expected_type_opt.is_none()
                && matches!(self.layout.values.get(&var_id), Some(Value::Float(_))));

        if target_is_distance {
            match value {
                Value::Float(d) => value = Value::Distance(d),
                Value::Integer(0) => value = Value::Distance(0.0),
                _ => {}
            }
        } else if target_is_duration {
            match value {
                Value::Float(d) => value = Value::Duration((d * 1_000_000.0).round() as i64),
                Value::Integer(0) => value = Value::Duration(0),
                _ => {}
            }
        } else if target_is_float {
            if let Value::Integer(i) = value {
                value = Value::Float(i as f64);
            }
        }

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
        match &value {
            Value::Timer { time, duration, .. } if *duration > 0 && *time < *duration => {
                self.active_timers.insert(var_id.clone());
            }
            Value::IndefiniteTimer { .. } => {
                self.active_timers.insert(var_id.clone());
            }
            _ => {
                self.active_timers.remove(&var_id);
            }
        }
        self.state_overrides.insert(var_id.clone(), value);

        // 4. Incrementally re-evaluate downstream variables in the DAG
        let changed = invalidate_and_reevaluate(
            &self.graph,
            &self.schedule,
            &mut self.layout.values,
            &[var_id],
            &self.state_overrides,
        )?;

        // Sync any downstream timer bindings affected by this state update
        for changed_id in &changed {
            match self.layout.values.get(changed_id) {
                Some(Value::Timer { time, duration, .. }) if *duration > 0 && *time < *duration => {
                    self.active_timers.insert(changed_id.clone());
                }
                Some(Value::IndefiniteTimer { .. }) => {
                    self.active_timers.insert(changed_id.clone());
                }
                _ => {
                    self.active_timers.remove(changed_id);
                }
            }
        }

        // 5. Update layout nodes and structured keys
        update_resolved_layout(&mut self.layout, &self.expanded, &changed);

        Ok(changed)
    }

    /// Returns `true` if there is at least one active fixed-length or indefinite timer in the document.
    pub fn has_active_timers(&self) -> bool {
        !self.active_timers.is_empty()
    }

    /// Advances all active timers in the document by `dt_micros` microseconds (`1s = 1_000_000`, `1ms = 1_000`).
    ///
    /// Fixed-length timers whose elapsed `time` reaches or exceeds `duration` automatically transition
    /// to `Value::Unset` and are removed from `active_timers`.
    /// All downstream DAG variables and layout nodes are incrementally re-evaluated.
    pub fn advance_timers(&mut self, dt_micros: i64) -> Result<HashSet<VarId>, CompileError> {
        let dt_us = dt_micros.max(0);
        if self.active_timers.is_empty() || dt_us == 0 {
            return Ok(HashSet::new());
        }

        let timer_vars: Vec<VarId> = self.active_timers.iter().cloned().collect();
        let mut mutated_vars = Vec::with_capacity(timer_vars.len());

        for var_id in timer_vars {
            let current = self
                .state_overrides
                .get(&var_id)
                .or_else(|| self.layout.values.get(&var_id))
                .cloned();
            match current {
                Some(Value::Timer {
                    time,
                    duration,
                    frames,
                    ..
                }) => {
                    let next_time = time.saturating_add(dt_us);
                    if next_time >= duration {
                        self.active_timers.remove(&var_id);
                        self.state_overrides.insert(var_id.clone(), Value::Unset);
                    } else {
                        self.state_overrides.insert(
                            var_id.clone(),
                            Value::Timer {
                                time: next_time,
                                duration,
                                dt: dt_us,
                                frames: frames.saturating_add(1),
                            },
                        );
                    }
                    mutated_vars.push(var_id);
                }
                Some(Value::IndefiniteTimer { time, frames, .. }) => {
                    let next_time = time.saturating_add(dt_us);
                    self.state_overrides.insert(
                        var_id.clone(),
                        Value::IndefiniteTimer {
                            time: next_time,
                            dt: dt_us,
                            frames: frames.saturating_add(1),
                        },
                    );
                    mutated_vars.push(var_id);
                }
                _ => {
                    self.active_timers.remove(&var_id);
                }
            }
        }

        if mutated_vars.is_empty() {
            return Ok(HashSet::new());
        }

        let changed = invalidate_and_reevaluate(
            &self.graph,
            &self.schedule,
            &mut self.layout.values,
            &mutated_vars,
            &self.state_overrides,
        )?;

        update_resolved_layout(&mut self.layout, &self.expanded, &changed);
        Ok(changed)
    }

    /// Convenience wrapper around `advance_timers` accepting a `std::time::Duration`.
    pub fn advance_timers_duration(
        &mut self,
        dt: std::time::Duration,
    ) -> Result<HashSet<VarId>, CompileError> {
        self.advance_timers(dt.as_micros() as i64)
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

    /// Updates the ambient window dimensions (`window.width`, `window.height`, `window.right`, `window.bottom`)
    /// and incrementally re-evaluates all downstream layout variables.
    pub fn resize_window(
        &mut self,
        window_width: f64,
        window_height: f64,
    ) -> Result<HashSet<VarId>, CompileError> {
        let window_span = Span::default();
        let width_id = VarId::new(NodeId::WINDOW, "width");
        let height_id = VarId::new(NodeId::WINDOW, "height");
        let right_id = VarId::new(NodeId::WINDOW, "right");
        let bottom_id = VarId::new(NodeId::WINDOW, "bottom");
        let px = crate::ast::DistanceUnit::Px;

        let width_expr =
            crate::ast::Expr::Literal(crate::ast::Literal::Distance(window_width, px, window_span));
        let height_expr = crate::ast::Expr::Literal(crate::ast::Literal::Distance(
            window_height,
            px,
            window_span,
        ));
        self.layout.graph = None;
        let graph_mut = std::sync::Arc::make_mut(&mut self.graph);
        graph_mut.update_variable_equation(&width_id, width_expr.clone());
        graph_mut.update_variable_equation(&height_id, height_expr.clone());
        graph_mut.update_variable_equation(&right_id, width_expr);
        graph_mut.update_variable_equation(&bottom_id, height_expr);
        self.layout.graph = Some(std::sync::Arc::clone(&self.graph));

        let dirty_roots = [width_id, height_id, right_id, bottom_id];
        let changed = invalidate_and_reevaluate(
            &self.graph,
            &self.schedule,
            &mut self.layout.values,
            &dirty_roots,
            &self.state_overrides,
        )?;

        update_resolved_layout(&mut self.layout, &self.expanded, &changed);

        Ok(changed)
    }

    /// Returns the current resolved layout.
    pub fn layout(&self) -> &ResolvedLayout {
        &self.layout
    }

    /// Finds a resolved node by its structured key.
    pub fn find_by_key(&self, parent: Option<NodeId>, key: &ComponentKey) -> Option<&ResolvedNode> {
        self.layout.find_by_key(parent, key)
    }

    /// Resolves an anchor path from a given source node.
    pub fn resolve_anchor(
        &self,
        from_node: NodeId,
        target_path: &str,
    ) -> Option<(NodeId, ScopeId)> {
        self.layout.resolve_anchor(from_node, target_path)
    }

    /// Returns the currently focused node ID, if any.
    pub fn focused_node(&self) -> Option<NodeId> {
        self.focused_node
    }

    /// Sets the currently focused node, dispatching `Blur` to the previous node and `Focus` to the new node.
    pub fn set_focused_node(
        &mut self,
        new_focus: Option<NodeId>,
    ) -> Result<HashSet<VarId>, crate::component::DispatchError> {
        if self.focused_node == new_focus {
            return Ok(HashSet::new());
        }

        let mut changed_vars = HashSet::new();
        let old_focus = self.focused_node;
        self.focused_node = new_focus;

        if let Some(old_id) = old_focus {
            let mut blur_event = crate::interaction::Event::new(
                crate::interaction::EventKind::Blur,
                crate::interaction::Point::default(),
                crate::interaction::Point::default(),
                crate::interaction::Modifiers::default(),
                old_id,
            );
            blur_event.bubble_path = self.layout.bubble_path_for_node(old_id);
            let vars = self.dispatch_event(&mut blur_event)?;
            changed_vars.extend(vars);
        }

        if let Some(new_id) = new_focus {
            let mut focus_event = crate::interaction::Event::new(
                crate::interaction::EventKind::Focus,
                crate::interaction::Point::default(),
                crate::interaction::Point::default(),
                crate::interaction::Modifiers::default(),
                new_id,
            );
            focus_event.bubble_path = self.layout.bubble_path_for_node(new_id);
            let vars = self.dispatch_event(&mut focus_event)?;
            changed_vars.extend(vars);
        }

        Ok(changed_vars)
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

        // 1. Populate bubble path if empty via spatial hit-testing or direct target node lookup
        if event.bubble_path.is_empty() {
            if event.kind == EventKind::Focus || event.kind == EventKind::Blur {
                if self.layout.get_node(event.target).is_some() {
                    event.bubble_path = self.layout.bubble_path_for_node(event.target);
                } else {
                    return Ok(HashSet::new());
                }
            } else if let Some(hit) = self.layout.hit_test(event.global_point) {
                event.target = hit.target;
                event.local_point = hit.local_point;
                event.bubble_path = hit.bubble_path;
            } else if self.layout.get_node(event.target).is_some() {
                event.bubble_path = self.layout.bubble_path_for_node(event.target);
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
            EventKind::Focus => "on_focus",
            EventKind::Blur => "on_blur",
            EventKind::Callback => "on_callback",
        };

        let mut all_changed_vars = HashSet::new();

        // 2. Bubble up the ancestor chain
        let bubble_nodes = event.bubble_path.clone();
        for node_id in bubble_nodes {
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

                            let mut ctx = Context::with_target(
                                target_node_id,
                                Some(event.target),
                                target_node.parent,
                                target_node.key.as_ref(),
                                &target_node.properties,
                                &self.layout,
                            );

                            event.propagation_continued = false;

                            component.dispatch(&handler.method, event, &mut ctx)?;

                            let should_call_default = ctx.take_call_default();
                            let mutations = ctx.take_mutations();
                            let actions = ctx.take_actions();
                            drop(ctx);

                            if should_call_default {
                                let default_handler_opt = self
                                    .layout
                                    .get_node(node_id)
                                    .and_then(|n| {
                                        n.default_event_handlers.get(event_handler_name).cloned()
                                    });
                                if let Some(def_handler) = default_handler_opt {
                                    if let EventHandlerTarget::Component(def_target_id) =
                                        def_handler.target
                                    {
                                        if (def_target_id, def_handler.method.as_str())
                                            != (target_node_id, handler.method.as_str())
                                        {
                                            if let Some(def_comp) =
                                                self.instances.get_mut(def_target_id)
                                            {
                                                if let Some(def_node) =
                                                    self.layout.get_node(def_target_id)
                                                {
                                                    let mut def_ctx = Context::with_target(
                                                        def_target_id,
                                                        Some(event.target),
                                                        def_node.parent,
                                                        def_node.key.as_ref(),
                                                        &def_node.properties,
                                                        &self.layout,
                                                    );
                                                    def_comp.dispatch(
                                                        &def_handler.method,
                                                        event,
                                                        &mut def_ctx,
                                                    )?;
                                                    let def_mutations = def_ctx.take_mutations();
                                                    let def_actions = def_ctx.take_actions();
                                                    drop(def_ctx);

                                                    for (state_name, val) in def_mutations {
                                                        if let Ok(changed) = self.set_state(
                                                            def_target_id,
                                                            &state_name,
                                                            val,
                                                        ) {
                                                            all_changed_vars.extend(changed);
                                                        }
                                                    }
                                                    for action in def_actions {
                                                        match action {
                                                            crate::component::ContextAction::InvokeCallback {
                                                                target: cb_target,
                                                                method: cb_method,
                                                                payload: cb_payload,
                                                            } => {
                                                                if let Ok(cb_changed) =
                                                                    self.dispatch_callback(
                                                                        cb_target,
                                                                        &cb_method,
                                                                        cb_payload,
                                                                    )
                                                                {
                                                                    all_changed_vars
                                                                        .extend(cb_changed);
                                                                }
                                                            }
                                                            other => {
                                                                self.actions.push(other);
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            for (state_name, val) in mutations {
                                if let Ok(changed) = self.set_state(target_node_id, &state_name, val) {
                                    all_changed_vars.extend(changed);
                                }
                            }

                            for action in actions {
                                match action {
                                    crate::component::ContextAction::InvokeCallback {
                                        target: cb_target,
                                        method: cb_method,
                                        payload: cb_payload,
                                    } => {
                                        if let Ok(cb_changed) =
                                            self.dispatch_callback(cb_target, &cb_method, cb_payload)
                                        {
                                            all_changed_vars.extend(cb_changed);
                                        }
                                    }
                                    other => {
                                        self.actions.push(other);
                                    }
                                }
                            }

                            // By default, event propagation stops on the first node with a matching handler,
                            // unless event.continue_propagation() was called.
                            if !event.propagation_continued {
                                break;
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

    /// Drains and returns all queued context actions emitted during event dispatch or lifecycle hooks.
    pub fn take_actions(&mut self) -> Vec<crate::component::ContextAction> {
        std::mem::take(&mut self.actions)
    }

    /// Scrolls a container (such as a ScrollView) to bring a descendant target node into view.
    pub fn scroll_container_to_node(
        &mut self,
        container: NodeId,
        target: NodeId,
    ) -> Result<HashSet<VarId>, CompileError> {
        let container_node = match self.layout.get_node(container) {
            Some(n) => n.clone(),
            None => return Ok(HashSet::new()),
        };
        let target_node = match self.layout.get_node(target) {
            Some(n) => n.clone(),
            None => return Ok(HashSet::new()),
        };

        let target_top = if !target_node.fragments.is_empty() {
            target_node.fragments[0].y
        } else {
            target_node.rect.y
        };

        let unscrolled_target_y = target_top;
        let relative_y = (unscrolled_target_y - container_node.rect.y).max(0.0);

        let clip_node = self.layout.nodes.iter().find(|n| n.parent == Some(container) && n.name == "Clip");
        let max_scroll_y = if let Some(clip) = clip_node {
            let mut max_bottom = container_node.rect.y;
            for n in &self.layout.nodes {
                if n.clip == Some(clip.id) {
                    max_bottom = max_bottom.max(n.rect.y + n.rect.height);
                }
            }
            let padding = container_node.properties.get("padding").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let content_h = (max_bottom + padding - container_node.rect.y).max(0.0);
            (content_h - container_node.rect.height).max(0.0)
        } else {
            f64::INFINITY
        };

        let target_scroll_y = relative_y.min(max_scroll_y);
        self.set_state(container, "scroll_y", Value::Distance(target_scroll_y))
    }

    /// Executes a context action on the compiled document (e.g. updating focus on scroll_to actions).
    pub fn execute_action(
        &mut self,
        action: crate::component::ContextAction,
    ) -> Result<HashSet<VarId>, crate::component::DispatchError> {
        match action {
            crate::component::ContextAction::ScrollToNode { target, container } => {
                let mut changed = self.set_focused_node(Some(target))?;
                if let Some(pane_id) = container {
                    if let Ok(pane_changed) = self.scroll_container_to_node(pane_id, target) {
                        changed.extend(pane_changed);
                    }
                }
                Ok(changed)
            }
            crate::component::ContextAction::SetFocus { target } => {
                self.set_focused_node(target)
            }
            crate::component::ContextAction::OpenUrl { .. } => {
                Ok(HashSet::new())
            }
            crate::component::ContextAction::CapturePointer(..)
            | crate::component::ContextAction::ReleasePointer => {
                // Pointer capture is handled by the viewer/interaction layer
                Ok(HashSet::new())
            }
            crate::component::ContextAction::InvokeCallback {
                target,
                method,
                payload,
            } => self.dispatch_callback(target, &method, payload),
        }
    }

    /// Dispatches a callback invocation directly to a target component instance with a typed payload.
    ///
    /// Executes the callback method within the One-Way Pipeline, applies any returned state mutations,
    /// and recursively evaluates any chained callback actions emitted by the target.
    pub fn dispatch_callback(
        &mut self,
        target: NodeId,
        method: &str,
        payload: Value,
    ) -> Result<HashSet<VarId>, DispatchError> {
        let mut all_changed_vars = HashSet::new();
        let pending_actions;

        if let Some(component) = self.instances.get_mut(target) {
            let target_node = self
                .layout
                .get_node(target)
                .ok_or(DispatchError::InstanceNotFound(target))?;

            let mut ctx = Context::new(
                target,
                target_node.parent,
                target_node.key.as_ref(),
                &target_node.properties,
                &self.layout,
            );

            let mut event = Event::callback(target, payload);
            component.dispatch(method, &mut event, &mut ctx)?;

            let mutations = ctx.take_mutations();
            pending_actions = ctx.take_actions();
            drop(ctx);

            for (state_name, val) in mutations {
                if let Ok(changed) = self.set_state(target, &state_name, val) {
                    all_changed_vars.extend(changed);
                }
            }
        } else {
            return Err(DispatchError::InstanceNotFound(target));
        }

        for action in pending_actions {
            match action {
                crate::component::ContextAction::InvokeCallback {
                    target: cb_target,
                    method: cb_method,
                    payload: cb_payload,
                } => {
                    let cascaded = self.dispatch_callback(cb_target, &cb_method, cb_payload)?;
                    all_changed_vars.extend(cascaded);
                }
                other => {
                    self.actions.push(other);
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
