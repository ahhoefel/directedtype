use crate::ast::{
    ChildrenDirective, ComponentBodyItem, ComponentDef, DistanceUnit, ElementNode, EnvBinding,
    Expr, Ident, LetBinding, LetValue, Literal, MemberAccessExpr, ParamDef,
};
use crate::compiler::error::CompileError;
use crate::compiler::eval::eval_expr;
use crate::compiler::expand::aggregation::resolve_and_filter_component_aggregations;
use crate::compiler::expand::builtins::clip::default_inherited_clip_expr;
use crate::compiler::expand::builtins::{eval_assert_directive, insert_default_spatial_aliases};
use crate::compiler::expand::children::{
    collect_consumer_children, count_consumer_children, expand_block_consumer_children,
    expand_caller_slot_envs, expand_span_consumer_children,
};
use crate::compiler::expand::context::{
    ConsumerChildEntry, ElementContext, EnvEntry, InstanceContext, LexicalBinding, ScopeContext,
};
use crate::compiler::expand::expand_element;
use crate::compiler::expand::rewrite::{
    default_expr_for_state_with_enums, extract_event_binding, resolve_forward_refs_in_range,
    resolve_forward_scope_refs, rewrite_expr, try_preresolve_expr_binding,
};
use crate::compiler::expand::schema::ComponentSchema;
use crate::compiler::expand::text_flow::{
    component_children_is_single_text, finalize_inline_box_component_ports,
    finalize_span_component_ports,
};
use crate::compiler::expand::typecheck::{
    coerce_typed_binding_expr, resolve_ident_or_member_to_literal,
    validate_and_coerce_component_param_types, validate_node_type_annotation,
};
use crate::compiler::expanded::{ExpandedDocument, NodeId};
use crate::compiler::text::{InlineBoxSpan, TextSpan};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// Stateful orchestrator for expanding a single user-defined component instance.
struct ComponentExpander<'a> {
    ctx: &'a InstanceContext<'a>,
    instance: &'a ElementNode,
    comp_def: &'a ComponentDef,
    registry: &'a HashMap<String, Vec<ComponentDef>>,
    doc: &'a mut ExpandedDocument,
    lexical_scope: &'a RefCell<HashMap<String, LexicalBinding>>,
    caller_env_scope: &'a HashMap<String, EnvEntry>,
    node_fonts: &'a mut HashMap<NodeId, NodeId>,

    schema: ComponentSchema,
    is_single_text_context: bool,
    comp_ports: HashMap<String, Expr>,
    explicit_unlowered_ports: HashMap<String, Expr>,
    already_rewritten_ports: HashSet<String>,
    instantiated_children_ids: &'a RefCell<Vec<NodeId>>,
    num_consumer_children: usize,
    internal_scope_ports: Vec<String>,

    consumer_child_entries: Vec<ConsumerChildEntry<'a>>,
    local_scope: RefCell<HashMap<String, LexicalBinding>>,
    internal_body_env_scope: HashMap<String, EnvEntry>,
    children_env_scope: HashMap<String, EnvEntry>,
    all_children_ids: Vec<NodeId>,
    last_child_id: Option<NodeId>,
    start_node_idx: usize,
    start_local_bindings_idx: usize,
    span_full_text: Option<String>,
    span_text_spans: Vec<TextSpan>,
    span_inline_boxes: Vec<InlineBoxSpan>,
}

impl<'a> ComponentExpander<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        ctx: &'a InstanceContext<'a>,
        instance: &'a ElementNode,
        comp_def: &'a ComponentDef,
        registry: &'a HashMap<String, Vec<ComponentDef>>,
        doc: &'a mut ExpandedDocument,
        lexical_scope: &'a RefCell<HashMap<String, LexicalBinding>>,
        caller_env_scope: &'a HashMap<String, EnvEntry>,
        node_fonts: &'a mut HashMap<NodeId, NodeId>,
        instantiated_children_ids: &'a RefCell<Vec<NodeId>>,
    ) -> Result<Self, CompileError> {
        let schema = ComponentSchema::from_def(comp_def)?;
        schema.validate_caller_ports(comp_def, instance, ctx.ambient_authored_ports)?;
        let is_single_text_context =
            !schema.is_span_def && component_children_is_single_text(comp_def, registry);
        if schema.is_span_def {
            doc.get_node_mut(ctx.comp_node_id).unwrap().is_span = true;
        }
        if schema.is_inline_box_def {
            doc.get_node_mut(ctx.comp_node_id).unwrap().is_inline_box = true;
        }

        let mut comp_ports = HashMap::new();
        for (state_name, s) in &schema.declared_states {
            let default_expr = default_expr_for_state_with_enums(s, ctx.enums);
            comp_ports.insert(state_name.clone(), default_expr);
        }
        for (let_name, l) in &schema.declared_lets {
            if !schema.comp_shadowed_ports.contains(let_name) {
                if let Some(LetValue::Expr(raw_expr)) = &l.value {
                    comp_ports.insert(let_name.clone(), raw_expr.clone());
                }
            }
        }
        for (env_name, e) in &schema.declared_envs {
            if !schema.comp_shadowed_ports.contains(env_name) {
                if let Some(raw_expr) = &e.value {
                    if !matches!(raw_expr, Expr::Node(_)) {
                        comp_ports.insert(env_name.clone(), raw_expr.clone());
                    }
                }
            }
        }

        let num_consumer_children = count_consumer_children(
            instance.content.as_ref(),
            schema.is_span_def || is_single_text_context,
            ctx.consumer_children.map_or(0, |c| c.len()),
            registry,
        );

        Ok(Self {
            ctx,
            instance,
            comp_def,
            registry,
            doc,
            lexical_scope,
            caller_env_scope,
            node_fonts,
            schema,
            is_single_text_context,
            comp_ports,
            explicit_unlowered_ports: HashMap::new(),
            already_rewritten_ports: HashSet::new(),
            instantiated_children_ids,
            num_consumer_children,
            internal_scope_ports: Vec::new(),
            consumer_child_entries: Vec::new(),
            local_scope: RefCell::new(HashMap::new()),
            internal_body_env_scope: HashMap::new(),
            children_env_scope: HashMap::new(),
            all_children_ids: Vec::new(),
            last_child_id: None,
            start_node_idx: 0,
            start_local_bindings_idx: 0,
            span_full_text: None,
            span_text_spans: Vec::new(),
            span_inline_boxes: Vec::new(),
        })
    }

    fn pre_resolve_explicit_font_port(&mut self) {
        let Some(font_port) = self
            .instance
            .ports
            .iter()
            .find(|p| p.name.as_str() == "font")
        else {
            return;
        };
        if matches!(&font_port.expr, Expr::Node(_)) {
            return;
        }
        let empty_children: [NodeId; 0] = [];
        let caller_scope_borrow = self.lexical_scope.borrow();
        let caller_inst_borrow = self
            .ctx
            .enclosing_instantiated_children
            .map(|c| c.borrow());
        let caller_child_ids: &[NodeId] = match &caller_inst_borrow {
            Some(b) => b.as_slice(),
            None => &empty_children,
        };
        let caller_scope_ctx = ScopeContext::from_instance(
            self.ctx,
            &self.doc.nodes,
            &caller_scope_borrow,
            self.caller_env_scope,
            self.node_fonts,
            caller_child_ids,
        );
        if let Ok(Expr::Ident(id)) = rewrite_expr(&font_port.expr, &caller_scope_ctx) {
            if let Some(fid) = NodeId::from_canonical_name(id.as_str()) {
                self.node_fonts.insert(self.ctx.comp_node_id, fid);
                self.doc.get_node_mut(self.ctx.comp_node_id).unwrap().font = Some(fid);
            }
        }
    }

    fn expand_explicit_caller_ports(&mut self) -> Result<HashMap<String, Expr>, CompileError> {
        let ctx = self.ctx;
        let empty_children: [NodeId; 0] = [];
        let mut explicit_ports = HashMap::new();

        for port in &self.instance.ports {
            let name = port.name.as_str().to_string();
            self.explicit_unlowered_ports
                .insert(name.clone(), port.expr.clone());
            let is_handler_param = self.comp_def.params.iter().any(|p| {
                p.name.as_str() == name
                    && p.type_annotation
                        .as_ref()
                        .is_some_and(|t| t.name.as_str() == "Handler")
            });
            let expr = match &port.expr {
                Expr::Node(inline_elem) => {
                    let child_ctx = ElementContext {
                        parent_id: Some(ctx.comp_node_id),
                        parent_name: ctx.parent_name,
                        prev_sibling_id: ctx.prev_sibling_id,
                        has_next_sibling: false,
                        parent_ports: ctx.parent_ports,
                        lexical_scope: self.lexical_scope,
                        env_scope: self.caller_env_scope,
                        enclosing_component_id: ctx.enclosing_component_id,
                        enclosing_num_children: ctx.enclosing_num_children,
                        enclosing_instantiated_children: ctx.enclosing_instantiated_children,
                        shadowed_ports: ctx.shadowed_ports,
                        is_let: false,
                        ambient_authored_ports: None,
                        enums: ctx.enums,
                        active_scope_id: ctx.active_scope_id,
                        consumer_children: None,
                    };
                    let child_id = expand_element(
                        inline_elem,
                        &child_ctx,
                        self.registry,
                        self.doc,
                        self.node_fonts,
                    )?;
                    if name == "font" && inline_elem.name.as_str() == "Font" {
                        self.node_fonts.insert(ctx.comp_node_id, child_id);
                        self.doc.get_node_mut(ctx.comp_node_id).unwrap().font = Some(child_id);
                    }
                    Expr::Ident(Ident::new(child_id.canonical_name(), inline_elem.span))
                }
                other => {
                    let caller_scope_borrow = self.lexical_scope.borrow();
                    let caller_inst_borrow =
                        ctx.enclosing_instantiated_children.map(|c| c.borrow());
                    let caller_child_ids: &[NodeId] = match &caller_inst_borrow {
                        Some(b) => b.as_slice(),
                        None => &empty_children,
                    };
                    let caller_scope_ctx = ScopeContext::from_instance(
                        ctx,
                        &self.doc.nodes,
                        &caller_scope_borrow,
                        self.caller_env_scope,
                        self.node_fonts,
                        caller_child_ids,
                    )
                    .as_event_port(name.starts_with("on_") || is_handler_param);
                    rewrite_expr(other, &caller_scope_ctx)?
                }
            };
            explicit_ports.insert(name, expr);
        }

        Ok(explicit_ports)
    }

    fn resolve_param_default_expr(
        &mut self,
        param_name: &str,
        default_expr: &Expr,
    ) -> Result<Expr, CompileError> {
        let ctx = self.ctx;
        match default_expr {
            Expr::Node(inline_elem) => {
                let child_ctx = ElementContext {
                    parent_id: Some(ctx.comp_node_id),
                    parent_name: ctx.parent_name,
                    prev_sibling_id: ctx.prev_sibling_id,
                    has_next_sibling: false,
                    parent_ports: ctx.parent_ports,
                    lexical_scope: self.lexical_scope,
                    env_scope: self.caller_env_scope,
                    enclosing_component_id: Some(ctx.comp_node_id),
                    enclosing_num_children: self.num_consumer_children,
                    enclosing_instantiated_children: Some(self.instantiated_children_ids),
                    shadowed_ports: Some(&self.schema.comp_shadowed_ports),
                    is_let: false,
                    ambient_authored_ports: None,
                    enums: ctx.enums,
                    active_scope_id: ctx.active_scope_id,
                    consumer_children: None,
                };
                let child_id = expand_element(
                    inline_elem,
                    &child_ctx,
                    self.registry,
                    self.doc,
                    self.node_fonts,
                )?;
                if param_name == "font" && inline_elem.name.as_str() == "Font" {
                    self.node_fonts.insert(ctx.comp_node_id, child_id);
                    self.doc.get_node_mut(ctx.comp_node_id).unwrap().font = Some(child_id);
                }
                Ok(Expr::Ident(Ident::new(
                    child_id.canonical_name(),
                    inline_elem.span,
                )))
            }
            other => Ok(other.clone()),
        }
    }

    fn apply_inline_box_origin_defaults(&mut self, parent_is_text_or_span: bool) {
        let ctx = self.ctx;
        let span = self.instance.span;
        for (port_name, edge_name) in [("x", "left"), ("y", "top")] {
            if !self.comp_ports.contains_key(port_name)
                && !self.schema.declared_aliases.contains_key(port_name)
            {
                let default_expr = if !parent_is_text_or_span {
                    if let Some(pid) = ctx.parent_id {
                        if !pid.is_window()
                            && (ctx.parent_ports.contains(&edge_name.to_string())
                                || ctx.parent_ports.contains(&port_name.to_string()))
                        {
                            let member = if ctx.parent_ports.contains(&edge_name.to_string()) {
                                edge_name
                            } else {
                                port_name
                            };
                            Expr::MemberAccess(MemberAccessExpr {
                                target: Box::new(Expr::Ident(Ident::new(
                                    pid.canonical_name(),
                                    span,
                                ))),
                                member: Ident::new(member, span),
                                span,
                            })
                        } else {
                            Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span))
                        }
                    } else {
                        Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span))
                    }
                } else {
                    Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span))
                };
                self.already_rewritten_ports.insert(port_name.to_string());
                self.comp_ports.insert(port_name.to_string(), default_expr);
            }
        }
    }

    fn record_resolved_component_font(&mut self) {
        let Some(font_expr) = self.comp_ports.get("font").cloned() else {
            return;
        };
        let empty_children: [NodeId; 0] = [];
        let resolved_font_expr = if let Expr::Ident(id) = &font_expr {
            if NodeId::from_canonical_name(id.as_str()).is_some() {
                Some(font_expr)
            } else {
                let caller_scope_borrow = self.lexical_scope.borrow();
                let temp_ctx = ScopeContext::from_instance(
                    self.ctx,
                    &self.doc.nodes,
                    &caller_scope_borrow,
                    self.caller_env_scope,
                    self.node_fonts,
                    &empty_children,
                )
                .with_children(&empty_children, self.num_consumer_children)
                .with_shadowed_ports(Some(&self.schema.comp_shadowed_ports));
                rewrite_expr(&font_expr, &temp_ctx).ok()
            }
        } else {
            let caller_scope_borrow = self.lexical_scope.borrow();
            let temp_ctx = ScopeContext::from_instance(
                self.ctx,
                &self.doc.nodes,
                &caller_scope_borrow,
                self.caller_env_scope,
                self.node_fonts,
                &empty_children,
            )
            .with_children(&empty_children, self.num_consumer_children)
            .with_shadowed_ports(Some(&self.schema.comp_shadowed_ports));
            rewrite_expr(&font_expr, &temp_ctx).ok()
        };
        if let Some(Expr::Ident(id)) = resolved_font_expr {
            if let Some(fid) = NodeId::from_canonical_name(id.as_str()) {
                self.comp_ports.insert("font".to_string(), Expr::Ident(id));
                self.node_fonts.insert(self.ctx.comp_node_id, fid);
                self.doc.get_node_mut(self.ctx.comp_node_id).unwrap().font = Some(fid);
            }
        }
    }

    /// Resolves caller arguments, environment bindings, and component parameter defaults (4-Tier Precedence).
    fn resolve_parameters(&mut self) -> Result<(), CompileError> {
        self.pre_resolve_explicit_font_port();
        let mut explicit_ports = self.expand_explicit_caller_ports()?;
        let comp_def = self.comp_def;

        // Resolve parameters according to 4-Tier Precedence
        for param in &comp_def.params {
            if param.name.as_str() == "parent" {
                return Err(CompileError::ReservedPort {
                    node: comp_def.name.as_str().to_string(),
                    port: "parent".to_string(),
                    span: param.name.span,
                });
            }
            let name = param.name.as_str().to_string();
            if !self.schema.comp_scope_ports.contains(&name) {
                self.schema.comp_scope_ports.push(name.clone());
            }

            // Tier 4: Explicit caller argument
            if let Some(explicit_expr) = explicit_ports.remove(&name) {
                self.already_rewritten_ports.insert(name.clone());
                self.comp_ports.insert(name, explicit_expr);
            }
            // Tier 2: Lexical Environment (caller_env_scope) - only for env parameters
            else if param.is_env {
                if let Some(EnvEntry::Bound(env_expr)) = self.caller_env_scope.get(&name) {
                    self.already_rewritten_ports.insert(name.clone());
                    self.comp_ports.insert(name, env_expr.clone());
                } else if let Some(default_expr) = &param.default_edge {
                    let resolved = self.resolve_param_default_expr(&name, default_expr)?;
                    self.comp_ports.insert(name, resolved);
                } else if name == "text_bounds" {
                    let default_tb = Expr::Node(Box::new(ElementNode {
                        name: Ident::new("TextBounds", param.span),
                        key: None,
                        ports: Vec::new(),
                        content: None,
                        span: param.span,
                        handle: None,
                    }));
                    let resolved = self.resolve_param_default_expr(&name, &default_tb)?;
                    self.comp_ports.insert(name, resolved);
                }
            }
            // Tier 1: Component Signature Default (for non-env parameters)
            else if let Some(default_expr) = &param.default_edge {
                let resolved = self.resolve_param_default_expr(&name, default_expr)?;
                self.comp_ports.insert(name, resolved);
            }
        }

        // Any remaining explicit ports not defined in signature (e.g. ad-hoc ports)
        let parent_is_text_or_span = self
            .ctx
            .parent_id
            .and_then(|pid| self.doc.get_node(pid))
            .is_some_and(|p| p.name == "Text" || p.is_span);
        for (name, expr) in explicit_ports {
            let is_purely_ambient = self
                .ctx
                .ambient_authored_ports
                .is_some_and(|a| a.contains_key(&name));
            let allow_standalone_span_ambient =
                (self.schema.is_span_def || self.schema.is_inline_box_def)
                    && !parent_is_text_or_span
                    && matches!(name.as_str(), "x" | "y" | "width");
            if is_purely_ambient && name != "clip" && !allow_standalone_span_ambient {
                continue;
            }
            if !self.schema.comp_scope_ports.contains(&name) {
                self.schema.comp_scope_ports.push(name.clone());
            }
            self.already_rewritten_ports.insert(name.clone());
            self.comp_ports.insert(name, expr);
        }

        if self.schema.is_inline_box_def {
            self.apply_inline_box_origin_defaults(parent_is_text_or_span);
        }

        let has_port_or_alias = |k: &str| {
            self.comp_ports.contains_key(k) || self.schema.declared_aliases.contains_key(k)
        };
        if has_port_or_alias("x") && !self.schema.comp_scope_ports.contains(&"left".to_string()) {
            self.schema.comp_scope_ports.push("left".to_string());
        }
        if has_port_or_alias("y") && !self.schema.comp_scope_ports.contains(&"top".to_string()) {
            self.schema.comp_scope_ports.push("top".to_string());
        }
        if (has_port_or_alias("x") || has_port_or_alias("left"))
            && has_port_or_alias("width")
            && !self.schema.comp_scope_ports.contains(&"right".to_string())
        {
            self.schema.comp_scope_ports.push("right".to_string());
        }
        if (has_port_or_alias("y") || has_port_or_alias("top"))
            && has_port_or_alias("height")
            && !self.schema.comp_scope_ports.contains(&"bottom".to_string())
        {
            self.schema.comp_scope_ports.push("bottom".to_string());
        }

        self.record_resolved_component_font();

        // Default clip port if not explicitly declared
        if !self.comp_ports.contains_key("clip") {
            let default_clip = default_inherited_clip_expr(
                self.ctx.parent_id,
                self.caller_env_scope,
                self.instance.span,
            );
            self.already_rewritten_ports.insert("clip".to_string());
            self.comp_ports.insert("clip".to_string(), default_clip);
        }

        // Validate that all required parameters (parameters without defaults) have been supplied
        for param in &comp_def.params {
            let name = param.name.as_str();
            if param.default_edge.is_none() && !self.comp_ports.contains_key(name) {
                return Err(CompileError::MissingPort {
                    node: comp_def.name.as_str().to_string(),
                    port: name.to_string(),
                    span: self.instance.span,
                });
            }
        }

        self.internal_scope_ports = self.schema.comp_scope_ports.clone();
        for let_name in self.schema.declared_lets.keys() {
            if !self.internal_scope_ports.contains(let_name) {
                self.internal_scope_ports.push(let_name.clone());
            }
        }

        Ok(())
    }

    /// Collects and normalizes consumer children passed to this component instance.
    fn collect_consumer_children(&mut self) -> Result<(), CompileError> {
        self.consumer_child_entries = collect_consumer_children(
            self.ctx,
            self.instance,
            self.comp_def,
            &self.schema.comp_scope_ports,
            self.schema.is_span_def,
            self.is_single_text_context,
            self.lexical_scope,
            self.caller_env_scope,
            self.instantiated_children_ids,
            self.registry,
        )?;
        debug_assert_eq!(
            self.num_consumer_children,
            self.consumer_child_entries.len()
        );
        Ok(())
    }

    fn init_local_lexical_scope(
        &mut self,
    ) -> Result<HashMap<String, LexicalBinding>, CompileError> {
        let ctx = self.ctx;
        let instance = self.instance;
        let comp_def = self.comp_def;
        let empty_children: [NodeId; 0] = [];

        let mut local_scope: HashMap<String, LexicalBinding> = self.lexical_scope.borrow().clone();
        let mut populated_comp_ports: HashSet<String> = HashSet::new();
        for param in &comp_def.params {
            let name = param.name.as_str().to_string();
            populated_comp_ports.insert(name.clone());
            let is_handler_param = param
                .type_annotation
                .as_ref()
                .is_some_and(|t| t.name.as_str() == "Handler");
            if name.starts_with("on_") || is_handler_param {
                if let Some(expr) = self.comp_ports.get(&name).cloned() {
                    if !self.already_rewritten_ports.contains(&name) {
                        let empty_scope = HashMap::new();
                        let event_default_ctx = ScopeContext::from_instance(
                            ctx,
                            &self.doc.nodes,
                            &empty_scope,
                            self.caller_env_scope,
                            self.node_fonts,
                            &empty_children,
                        )
                        .with_children(&empty_children, self.num_consumer_children)
                        .with_shadowed_ports(Some(&self.schema.comp_shadowed_ports))
                        .with_current_ports(&self.schema.comp_scope_ports)
                        .with_enclosing_component(Some(ctx.comp_node_id))
                        .as_event_port(true);
                        let rewritten = rewrite_expr(&expr, &event_default_ctx)?;
                        self.already_rewritten_ports.insert(name.clone());
                        self.comp_ports.insert(name.clone(), rewritten.clone());
                        local_scope.insert(name, LexicalBinding::Expr(rewritten));
                    } else {
                        local_scope.insert(name, LexicalBinding::Expr(expr));
                    }
                    continue;
                }
            }
            if let Some(Expr::Ident(id)) = self.comp_ports.get(&name) {
                if let Some(node_id) = NodeId::from_canonical_name(id.as_str()) {
                    local_scope.insert(name.clone(), LexicalBinding::Node(node_id));
                    continue;
                }
            }
            local_scope.insert(
                name.clone(),
                LexicalBinding::Expr(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(
                        ctx.comp_node_id.canonical_name(),
                        param.name.span,
                    ))),
                    member: param.name.clone(),
                    span: param.name.span,
                })),
            );
        }
        for (let_name, let_binding) in &self.schema.declared_lets {
            if !self.schema.comp_shadowed_ports.contains(let_name) {
                populated_comp_ports.insert(let_name.clone());
                local_scope.insert(
                    let_name.clone(),
                    LexicalBinding::Expr(Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(
                            ctx.comp_node_id.canonical_name(),
                            let_binding.name.span,
                        ))),
                        member: let_binding.name.clone(),
                        span: let_binding.name.span,
                    })),
                );
            }
        }
        for port_name in &self.schema.comp_scope_ports {
            if !populated_comp_ports.contains(port_name) {
                let member_expr = Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(
                        ctx.comp_node_id.canonical_name(),
                        instance.span,
                    ))),
                    member: Ident::new(port_name.clone(), instance.span),
                    span: instance.span,
                });
                let binding = if self.schema.declared_state_names.contains(port_name) {
                    LexicalBinding::State(member_expr)
                } else {
                    LexicalBinding::Expr(member_expr)
                };
                local_scope.insert(port_name.clone(), binding);
            }
        }
        Ok(local_scope)
    }

    fn resolve_env_param_bound_expr(&self, param: &ParamDef) -> Expr {
        let span = self.instance.span;
        let comp_node_id = self.ctx.comp_node_id;
        if param.name.as_str() == "font" {
            if let Some(&fid) = self.node_fonts.get(&comp_node_id) {
                return Expr::Ident(Ident::new(fid.canonical_name(), span));
            }
        } else if let Some(Expr::Ident(id)) = self.comp_ports.get(param.name.as_str()) {
            if NodeId::from_canonical_name(id.as_str()).is_some() {
                return Expr::Ident(id.clone());
            }
        }
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(Expr::Ident(Ident::new(comp_node_id.canonical_name(), span))),
            member: param.name.clone(),
            span,
        })
    }

    /// Initializes the component's local lexical scope, internal body environment scope,
    /// and children environment scope, then runs fixed-point pre-resolution on `\let` and `\env` expressions.
    fn init_scopes_and_preresolve(&mut self) -> Result<(), CompileError> {
        let ctx = self.ctx;
        let instance = self.instance;
        let comp_def = self.comp_def;
        let empty_children: [NodeId; 0] = [];

        let mut local_scope = self.init_local_lexical_scope()?;

        let self_clip_entry = EnvEntry::Bound(Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(Expr::Ident(Ident::new(
                ctx.comp_node_id.canonical_name(),
                instance.span,
            ))),
            member: Ident::new("clip", instance.span),
            span: instance.span,
        }));

        let mut internal_body_env_scope: HashMap<String, EnvEntry> = HashMap::new();
        internal_body_env_scope.insert("clip".to_string(), self_clip_entry.clone());

        let mut children_env_scope = self.caller_env_scope.clone();
        children_env_scope.insert("clip".to_string(), self_clip_entry);

        for param in &comp_def.params {
            if param.is_env {
                let bound_entry = EnvEntry::Bound(self.resolve_env_param_bound_expr(param));
                internal_body_env_scope
                    .insert(param.name.as_str().to_string(), bound_entry.clone());
                children_env_scope.insert(param.name.as_str().to_string(), bound_entry);
            }
        }
        if self.schema.is_inline_box_def {
            for inherited_env in ["font", "text_bounds"] {
                if !internal_body_env_scope.contains_key(inherited_env) {
                    if let Some(entry) = self.caller_env_scope.get(inherited_env) {
                        internal_body_env_scope.insert(inherited_env.to_string(), entry.clone());
                    }
                }
            }
        }

        // Pre-register uninitialized let and env in component body
        for l in &self.schema.all_lets {
            let is_env_param_shadow = comp_def
                .params
                .iter()
                .any(|p| p.is_env && p.name.as_str() == l.name.as_str());
            if l.value.is_none() || is_env_param_shadow {
                children_env_scope.insert(l.name.as_str().to_string(), EnvEntry::Tombstone);
                internal_body_env_scope.insert(l.name.as_str().to_string(), EnvEntry::Tombstone);
            }
            if l.value.is_none() {
                local_scope.insert(l.name.as_str().to_string(), LexicalBinding::Uninitialized);
            }
        }
        for item in &comp_def.body {
            if let ComponentBodyItem::Env(e) = item {
                if e.value.is_none() {
                    local_scope.insert(e.name.as_str().to_string(), LexicalBinding::Uninitialized);
                    children_env_scope.insert(e.name.as_str().to_string(), EnvEntry::Tombstone);
                    internal_body_env_scope
                        .insert(e.name.as_str().to_string(), EnvEntry::Tombstone);
                }
            }
        }

        // Pre-resolve let and env expressions in component body so their declaration order does not matter
        let mut changed = true;
        let mut passes = 0;
        while changed && passes < 20 {
            changed = false;
            passes += 1;
            for let_binding in &self.schema.all_lets {
                if let_binding.value.is_none() {
                    local_scope.insert(
                        let_binding.name.as_str().to_string(),
                        LexicalBinding::Uninitialized,
                    );
                } else if let Some(LetValue::Expr(raw_expr)) = &let_binding.value {
                    if self
                        .schema
                        .comp_shadowed_ports
                        .contains(let_binding.name.as_str())
                    {
                        let rewritten_res = {
                            let scope_ctx = ScopeContext::from_instance(
                                ctx,
                                &self.doc.nodes,
                                &local_scope,
                                &internal_body_env_scope,
                                self.node_fonts,
                                &empty_children,
                            )
                            .with_children(&empty_children, self.num_consumer_children)
                            .with_shadowed_ports(Some(&self.schema.comp_shadowed_ports))
                            .with_current_ports(&self.schema.comp_scope_ports)
                            .with_enclosing_component(Some(ctx.comp_node_id))
                            .with_comp_state(
                                Some(&self.comp_ports),
                                Some(&self.schema.declared_state_names),
                            );
                            rewrite_expr(raw_expr, &scope_ctx)
                        };
                        if try_preresolve_expr_binding(
                            &mut local_scope,
                            let_binding.name.as_str(),
                            rewritten_res,
                            let_binding.type_annotation.as_ref(),
                        )
                        .is_some()
                        {
                            changed = true;
                        }
                    }
                }
            }
            for item in &comp_def.body {
                if let ComponentBodyItem::Env(env_binding) = item {
                    if env_binding.value.is_none() {
                        local_scope.insert(
                            env_binding.name.as_str().to_string(),
                            LexicalBinding::Uninitialized,
                        );
                    } else if let Some(raw_expr) = &env_binding.value {
                        if !matches!(raw_expr, Expr::Node(_)) {
                            let rewritten_res = {
                                let scope_ctx = ScopeContext::from_instance(
                                    ctx,
                                    &self.doc.nodes,
                                    &local_scope,
                                    &internal_body_env_scope,
                                    self.node_fonts,
                                    &empty_children,
                                )
                                .with_children(&empty_children, self.num_consumer_children)
                                .with_shadowed_ports(Some(&self.schema.comp_shadowed_ports))
                                .with_current_ports(&self.schema.comp_scope_ports)
                                .with_enclosing_component(Some(ctx.comp_node_id));
                                rewrite_expr(raw_expr, &scope_ctx)
                            };
                            if try_preresolve_expr_binding(
                                &mut local_scope,
                                env_binding.name.as_str(),
                                rewritten_res,
                                env_binding.type_annotation.as_ref(),
                            )
                            .is_some()
                            {
                                changed = true;
                            }
                        }
                    }
                }
            }
        }

        self.local_scope = RefCell::new(local_scope);
        self.internal_body_env_scope = internal_body_env_scope;
        self.children_env_scope = children_env_scope;
        Ok(())
    }

    fn sync_consumer_children_env_scope(&mut self) {
        for entry in &mut self.consumer_child_entries {
            for (k, v) in &self.children_env_scope {
                entry.caller_env_scope.insert(k.clone(), v.clone());
            }
        }
    }

    fn expand_body_let(&mut self, let_binding: &LetBinding) -> Result<(), CompileError> {
        let ctx = self.ctx;
        let comp_def = self.comp_def;
        let is_env_param_shadow = comp_def
            .params
            .iter()
            .any(|p| p.is_env && p.name.as_str() == let_binding.name.as_str());

        if let_binding.value.is_none() || is_env_param_shadow {
            // Shield/Swallower pattern or Firewall Tombstone
            self.children_env_scope
                .insert(let_binding.name.as_str().to_string(), EnvEntry::Tombstone);
            self.internal_body_env_scope
                .insert(let_binding.name.as_str().to_string(), EnvEntry::Tombstone);
        }

        if let Some(LetValue::Node(elem)) = &let_binding.value {
            validate_node_type_annotation(
                elem.name.as_str(),
                elem.span,
                let_binding.type_annotation.as_ref(),
                ctx.enums,
                self.registry,
            )?;
            let node_id = NodeId(self.doc.nodes.len());
            self.local_scope.borrow_mut().insert(
                let_binding.name.as_str().to_string(),
                LexicalBinding::Node(node_id),
            );
            self.sync_consumer_children_env_scope();
            let elem_ctx = ElementContext {
                parent_id: Some(ctx.comp_node_id),
                parent_name: Some(comp_def.name.as_str()),
                prev_sibling_id: None,
                has_next_sibling: false,
                parent_ports: &self.internal_scope_ports,
                lexical_scope: &self.local_scope,
                env_scope: &self.internal_body_env_scope,
                enclosing_component_id: Some(ctx.comp_node_id),
                enclosing_num_children: self.num_consumer_children,
                enclosing_instantiated_children: Some(self.instantiated_children_ids),
                shadowed_ports: Some(&self.schema.comp_shadowed_ports),
                is_let: true,
                ambient_authored_ports: None,
                enums: ctx.enums,
                active_scope_id: ctx.active_scope_id,
                consumer_children: Some(&self.consumer_child_entries),
            };
            let expanded_node_id = expand_element(
                elem,
                &elem_ctx,
                self.registry,
                self.doc,
                self.node_fonts,
            )?;
            debug_assert_eq!(node_id, expanded_node_id);
            self.doc.get_node_mut(node_id).unwrap().var_name =
                Some(let_binding.name.as_str().to_string());
            self.all_children_ids.push(node_id);
            if elem.name.as_str() == "Font" {
                self.node_fonts.insert(node_id, node_id);
            }
        } else if let Some(LetValue::Expr(raw_expr)) = &let_binding.value {
            let is_param_shadow = self
                .schema
                .comp_shadowed_ports
                .contains(let_binding.name.as_str());
            if is_param_shadow {
                let rewritten = {
                    let scope_borrow = self.local_scope.borrow();
                    let inst_borrow = self.instantiated_children_ids.borrow();
                    let scope_ctx = ScopeContext::from_instance(
                        ctx,
                        &self.doc.nodes,
                        &scope_borrow,
                        &self.internal_body_env_scope,
                        self.node_fonts,
                        &inst_borrow,
                    )
                    .with_children(&inst_borrow, self.num_consumer_children)
                    .with_shadowed_ports(Some(&self.schema.comp_shadowed_ports))
                    .with_current_ports(&self.schema.comp_scope_ports)
                    .with_enclosing_component(Some(ctx.comp_node_id))
                    .with_comp_state(
                        Some(&self.comp_ports),
                        Some(&self.schema.declared_state_names),
                    );
                    rewrite_expr(raw_expr, &scope_ctx)?
                };
                let rewritten =
                    coerce_typed_binding_expr(rewritten, let_binding.type_annotation.as_ref());
                self.doc.local_bindings.push((
                    rewritten.clone(),
                    let_binding
                        .type_annotation
                        .as_ref()
                        .map(|t| t.name.as_str().to_string()),
                    raw_expr.span(),
                ));
                self.local_scope.borrow_mut().insert(
                    let_binding.name.as_str().to_string(),
                    LexicalBinding::Expr(rewritten),
                );
            } else if !is_env_param_shadow {
                let member_expr = Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(
                        ctx.comp_node_id.canonical_name(),
                        let_binding.name.span,
                    ))),
                    member: let_binding.name.clone(),
                    span: let_binding.name.span,
                });
                self.children_env_scope.insert(
                    let_binding.name.as_str().to_string(),
                    EnvEntry::Bound(member_expr.clone()),
                );
                self.internal_body_env_scope.insert(
                    let_binding.name.as_str().to_string(),
                    EnvEntry::Bound(member_expr),
                );
            }
        }
        Ok(())
    }

    fn expand_body_env(&mut self, env_binding: &EnvBinding) -> Result<(), CompileError> {
        let ctx = self.ctx;
        let comp_def = self.comp_def;
        if let Some(raw_expr) = &env_binding.value {
            match raw_expr {
                Expr::Node(elem) => {
                    validate_node_type_annotation(
                        elem.name.as_str(),
                        elem.span,
                        env_binding.type_annotation.as_ref(),
                        ctx.enums,
                        self.registry,
                    )?;
                    self.sync_consumer_children_env_scope();
                    let elem_ctx = ElementContext {
                        parent_id: Some(ctx.comp_node_id),
                        parent_name: Some(comp_def.name.as_str()),
                        prev_sibling_id: None,
                        has_next_sibling: false,
                        parent_ports: &self.internal_scope_ports,
                        lexical_scope: &self.local_scope,
                        env_scope: &self.internal_body_env_scope,
                        enclosing_component_id: Some(ctx.comp_node_id),
                        enclosing_num_children: self.num_consumer_children,
                        enclosing_instantiated_children: Some(self.instantiated_children_ids),
                        shadowed_ports: Some(&self.schema.comp_shadowed_ports),
                        is_let: false,
                        ambient_authored_ports: None,
                        enums: ctx.enums,
                        active_scope_id: ctx.active_scope_id,
                        consumer_children: Some(&self.consumer_child_entries),
                    };
                    let node_id = expand_element(
                        elem,
                        &elem_ctx,
                        self.registry,
                        self.doc,
                        self.node_fonts,
                    )?;
                    self.doc.get_node_mut(node_id).unwrap().var_name =
                        Some(env_binding.name.as_str().to_string());
                    self.all_children_ids.push(node_id);
                    self.local_scope.borrow_mut().insert(
                        env_binding.name.as_str().to_string(),
                        LexicalBinding::Node(node_id),
                    );
                    let bound_expr = Expr::Ident(Ident::new(node_id.canonical_name(), elem.span));
                    self.children_env_scope.insert(
                        env_binding.name.as_str().to_string(),
                        EnvEntry::Bound(bound_expr.clone()),
                    );
                    self.internal_body_env_scope.insert(
                        env_binding.name.as_str().to_string(),
                        EnvEntry::Bound(bound_expr.clone()),
                    );
                    if !self
                        .schema
                        .comp_shadowed_ports
                        .contains(env_binding.name.as_str())
                    {
                        self.comp_ports
                            .insert(env_binding.name.as_str().to_string(), bound_expr);
                        self.already_rewritten_ports
                            .insert(env_binding.name.as_str().to_string());
                    }
                    if elem.name.as_str() == "Font" {
                        self.node_fonts.insert(node_id, node_id);
                        if env_binding.name.as_str() == "font"
                            && !self.schema.comp_shadowed_ports.contains("font")
                        {
                            self.node_fonts.insert(ctx.comp_node_id, node_id);
                            self.doc.get_node_mut(ctx.comp_node_id).unwrap().font = Some(node_id);
                        }
                    }
                }
                _ => {
                    let rewritten = {
                        let scope_borrow = self.local_scope.borrow();
                        let inst_borrow = self.instantiated_children_ids.borrow();
                        let scope_ctx = ScopeContext::from_instance(
                            ctx,
                            &self.doc.nodes,
                            &scope_borrow,
                            &self.internal_body_env_scope,
                            self.node_fonts,
                            &inst_borrow,
                        )
                        .with_children(&inst_borrow, self.num_consumer_children)
                        .with_shadowed_ports(Some(&self.schema.comp_shadowed_ports))
                        .with_current_ports(&self.schema.comp_scope_ports)
                        .with_enclosing_component(Some(ctx.comp_node_id));
                        rewrite_expr(raw_expr, &scope_ctx)?
                    };
                    let rewritten =
                        coerce_typed_binding_expr(rewritten, env_binding.type_annotation.as_ref());
                    if self
                        .schema
                        .comp_shadowed_ports
                        .contains(env_binding.name.as_str())
                    {
                        self.doc.local_bindings.push((
                            rewritten.clone(),
                            env_binding
                                .type_annotation
                                .as_ref()
                                .map(|t| t.name.as_str().to_string()),
                            raw_expr.span(),
                        ));
                    }
                    self.local_scope.borrow_mut().insert(
                        env_binding.name.as_str().to_string(),
                        LexicalBinding::Expr(rewritten.clone()),
                    );
                    self.children_env_scope.insert(
                        env_binding.name.as_str().to_string(),
                        EnvEntry::Bound(rewritten.clone()),
                    );
                    self.internal_body_env_scope.insert(
                        env_binding.name.as_str().to_string(),
                        EnvEntry::Bound(rewritten),
                    );
                }
            }
        } else {
            // Environmental Hole Tombstone
            self.local_scope.borrow_mut().insert(
                env_binding.name.as_str().to_string(),
                LexicalBinding::Uninitialized,
            );
            self.children_env_scope
                .insert(env_binding.name.as_str().to_string(), EnvEntry::Tombstone);
            self.internal_body_env_scope
                .insert(env_binding.name.as_str().to_string(), EnvEntry::Tombstone);
        }
        Ok(())
    }

    fn expand_body_node(&mut self, body_node: &ElementNode) -> Result<(), CompileError> {
        self.sync_consumer_children_env_scope();
        let elem_ctx = ElementContext {
            parent_id: Some(self.ctx.comp_node_id),
            parent_name: Some(self.comp_def.name.as_str()),
            prev_sibling_id: self.last_child_id,
            has_next_sibling: false,
            parent_ports: &self.internal_scope_ports,
            lexical_scope: &self.local_scope,
            env_scope: &self.internal_body_env_scope,
            enclosing_component_id: Some(self.ctx.comp_node_id),
            enclosing_num_children: self.num_consumer_children,
            enclosing_instantiated_children: Some(self.instantiated_children_ids),
            shadowed_ports: Some(&self.schema.comp_shadowed_ports),
            is_let: false,
            ambient_authored_ports: None,
            enums: self.ctx.enums,
            active_scope_id: self.ctx.active_scope_id,
            consumer_children: Some(&self.consumer_child_entries),
        };
        let body_id = expand_element(
            body_node,
            &elem_ctx,
            self.registry,
            self.doc,
            self.node_fonts,
        )?;
        self.all_children_ids.push(body_id);
        self.last_child_id = Some(body_id);
        Ok(())
    }

    fn expand_body_children(&mut self, dir: &ChildrenDirective) -> Result<(), CompileError> {
        for port in &dir.ports {
            if port.name.as_str() == "parent" {
                return Err(CompileError::ReservedPort {
                    node: "Children".to_string(),
                    port: "parent".to_string(),
                    span: port.name.span,
                });
            }
        }
        expand_caller_slot_envs(
            self.ctx,
            self.instance,
            self.comp_def,
            &self.schema.comp_scope_ports,
            self.lexical_scope,
            &mut self.children_env_scope,
            self.registry,
            self.doc,
            self.node_fonts,
            &mut self.all_children_ids,
        )?;
        if !self.schema.is_span_def {
            expand_block_consumer_children(
                self.ctx,
                self.comp_def,
                dir,
                &self.schema.comp_scope_ports,
                &self.internal_scope_ports,
                &self.schema.comp_shadowed_ports,
                &self.comp_ports,
                &self.schema.declared_state_names,
                self.num_consumer_children,
                self.instantiated_children_ids,
                &self.local_scope,
                &self.children_env_scope,
                &self.consumer_child_entries,
                self.registry,
                self.doc,
                self.node_fonts,
                &mut self.all_children_ids,
            )?;
        }
        Ok(())
    }

    /// Expands all items in the component body in declaration order (Painter's Algorithm),
    /// expands `\Span` consumer children when applicable, and resolves forward lexical references.
    fn expand_body(&mut self) -> Result<(), CompileError> {
        self.start_node_idx = self.doc.nodes.len();
        self.start_local_bindings_idx = self.doc.local_bindings.len();

        for item in &self.comp_def.body {
            match item {
                ComponentBodyItem::Let(let_binding) => self.expand_body_let(let_binding)?,
                ComponentBodyItem::Env(env_binding) => self.expand_body_env(env_binding)?,
                ComponentBodyItem::Node(body_node) => self.expand_body_node(body_node)?,
                ComponentBodyItem::Children(dir) => self.expand_body_children(dir)?,
                ComponentBodyItem::Alias(_)
                | ComponentBodyItem::State(_)
                | ComponentBodyItem::Text(_)
                | ComponentBodyItem::ParagraphBreak(_)
                | ComponentBodyItem::Assert(_) => {}
            }
        }

        if self.schema.is_span_def {
            let (full_text, spans, ibs) = expand_span_consumer_children(
                self.ctx,
                self.comp_def,
                &self.schema.comp_scope_ports,
                &self.consumer_child_entries,
                &self.children_env_scope,
                self.registry,
                self.doc,
                self.node_fonts,
                &mut self.all_children_ids,
            )?;
            self.span_full_text = Some(full_text);
            self.span_text_spans = spans;
            self.span_inline_boxes = ibs;
        }

        {
            let mut local_scope_mut = self.local_scope.borrow_mut();
            let inst_ids = self.instantiated_children_ids.borrow();
            for (idx, &child_id) in inst_ids.iter().enumerate() {
                local_scope_mut.insert(
                    format!(
                        "__child_of_{}_{}",
                        self.ctx.comp_node_id.canonical_name(),
                        idx
                    ),
                    LexicalBinding::Node(child_id),
                );
            }
        }

        let local_scope_borrow = self.local_scope.borrow();
        resolve_forward_refs_in_range(
            self.doc,
            self.start_node_idx,
            self.start_local_bindings_idx,
            &local_scope_borrow,
        );

        Ok(())
    }

    /// Rewrites component ports and explicitly declared aliases in the finalized component scope,
    /// returning `(rewritten_ports, comp_authored_ports)`.
    #[allow(clippy::type_complexity)]
    fn rewrite_and_alias_ports(
        &self,
    ) -> Result<(HashMap<String, Expr>, HashMap<String, Expr>), CompileError> {
        let ctx = self.ctx;
        let instance = self.instance;
        let comp_def = self.comp_def;
        let final_local_scope = self.local_scope.borrow();
        let inst_ids = self.instantiated_children_ids.borrow();

        let scope_ctx = ScopeContext::from_instance(
            ctx,
            &self.doc.nodes,
            &final_local_scope,
            &self.internal_body_env_scope,
            self.node_fonts,
            &inst_ids,
        )
        .with_children(&inst_ids, self.num_consumer_children)
        .with_shadowed_ports(Some(&self.schema.comp_shadowed_ports))
        .with_current_ports(&self.schema.comp_scope_ports)
        .with_enclosing_component(Some(ctx.comp_node_id))
        .with_comp_state(
            Some(&self.comp_ports),
            Some(&self.schema.declared_state_names),
        );

        let mut rewritten_ports = HashMap::new();
        for (port_name, port_expr) in &self.comp_ports {
            if self.already_rewritten_ports.contains(port_name) {
                rewritten_ports.insert(port_name.clone(), port_expr.clone());
            } else {
                let is_handler_param = comp_def.params.iter().any(|p| {
                    p.name.as_str() == port_name
                        && p.type_annotation
                            .as_ref()
                            .is_some_and(|t| t.name.as_str() == "Handler")
                });
                let mut port_scope_ctx = scope_ctx.clone();
                port_scope_ctx.is_event_port = port_name.starts_with("on_") || is_handler_param;
                let mut rw = rewrite_expr(port_expr, &port_scope_ctx)?;
                rw = resolve_forward_scope_refs(&rw, &final_local_scope);
                if let Some(l) = self.schema.declared_lets.get(port_name) {
                    rw = coerce_typed_binding_expr(rw, l.type_annotation.as_ref());
                } else if let Some(e) = self.schema.declared_envs.get(port_name) {
                    rw = coerce_typed_binding_expr(rw, e.type_annotation.as_ref());
                } else if let Some(s) = self.schema.declared_states.get(port_name) {
                    rw = coerce_typed_binding_expr(rw, s.type_annotation.as_ref());
                }
                rewritten_ports.insert(port_name.clone(), rw);
            }
        }

        let mut comp_authored_ports = HashMap::new();
        if let Some(ambient_map) = ctx.ambient_authored_ports {
            for (k, v) in ambient_map {
                comp_authored_ports.insert(k.clone(), v.clone());
            }
        }
        for param in &comp_def.params {
            let name = param.name.as_str().to_string();
            if let Some(default_expr) = &param.default_edge {
                comp_authored_ports
                    .entry(name)
                    .or_insert_with(|| default_expr.clone());
            }
        }
        for (state_name, s) in &self.schema.declared_states {
            let default_expr = default_expr_for_state_with_enums(s, ctx.enums);
            comp_authored_ports.insert(state_name.clone(), default_expr);
        }
        for (let_name, l) in &self.schema.declared_lets {
            if !self.schema.comp_shadowed_ports.contains(let_name) {
                if let Some(LetValue::Expr(raw_expr)) = &l.value {
                    comp_authored_ports.insert(let_name.clone(), raw_expr.clone());
                }
            }
        }
        for (env_name, e) in &self.schema.declared_envs {
            if let Some(raw_expr) = &e.value {
                comp_authored_ports.insert(env_name.clone(), raw_expr.clone());
            }
        }
        for port in &instance.ports {
            if let Some(ambient_expr) = ctx
                .ambient_authored_ports
                .and_then(|m| m.get(port.name.as_str()))
            {
                comp_authored_ports.insert(port.name.as_str().to_string(), ambient_expr.clone());
            } else {
                comp_authored_ports.insert(port.name.as_str().to_string(), port.expr.clone());
            }
        }

        for (alias_name, alias_binding) in &self.schema.declared_aliases {
            let mut rewritten_alias = rewrite_expr(&alias_binding.value, &scope_ctx)?;
            rewritten_alias = resolve_forward_scope_refs(&rewritten_alias, &final_local_scope);
            rewritten_alias =
                coerce_typed_binding_expr(rewritten_alias, alias_binding.type_annotation.as_ref());
            rewritten_ports.insert(alias_name.clone(), rewritten_alias);
            comp_authored_ports.insert(alias_name.clone(), alias_binding.value.clone());
        }

        Ok((rewritten_ports, comp_authored_ports))
    }

    /// Applies `\Span` and `\InlineBox` post-processing and synthesizes default `left`, `top`, `right`, `bottom` aliases.
    fn apply_post_processing_and_spatial_aliases(
        &mut self,
        rewritten_ports: &mut HashMap<String, Expr>,
        comp_authored_ports: &mut HashMap<String, Expr>,
    ) {
        if self.schema.is_span_def {
            let local_scope_borrow = self.local_scope.borrow();
            finalize_span_component_ports(
                self.ctx.comp_node_id,
                self.instance.span,
                self.doc,
                &local_scope_borrow,
                self.node_fonts,
                rewritten_ports,
                &self.span_full_text,
                &mut self.span_text_spans,
            );
        }

        insert_default_spatial_aliases(
            self.ctx.comp_node_id,
            self.instance.span,
            rewritten_ports,
            comp_authored_ports,
        );

        if self.schema.is_inline_box_def {
            finalize_inline_box_component_ports(
                self.ctx.comp_node_id,
                self.start_node_idx,
                self.instance.span,
                &self.doc.nodes,
                &self.schema.declared_state_names,
                rewritten_ports,
            );
        }
    }

    /// Resolves `children.*` aggregations, validates and coerces parameter types, and evaluates `\assert(...)` directives.
    fn resolve_aggregations_and_assertions(
        &mut self,
        rewritten_ports: &mut HashMap<String, Expr>,
    ) -> Result<(), CompileError> {
        let ctx = self.ctx;
        let comp_def = self.comp_def;
        let inst_ids = self.instantiated_children_ids.borrow().clone();

        resolve_and_filter_component_aggregations(
            ctx.comp_node_id,
            comp_def.name.as_str(),
            &inst_ids,
            &self.schema.declared_state_names,
            rewritten_ports,
            self.doc,
            self.start_node_idx,
            self.start_local_bindings_idx,
        )?;

        {
            let mut local_scope_mut = self.local_scope.borrow_mut();
            for (idx, &child_id) in inst_ids.iter().enumerate() {
                local_scope_mut.insert(
                    format!("__agg_child_of_{}_{}", ctx.comp_node_id.canonical_name(), idx),
                    LexicalBinding::Node(child_id),
                );
            }
        }

        let final_local_scope = self.local_scope.borrow();
        let scope_ctx = ScopeContext::from_instance(
            ctx,
            &self.doc.nodes,
            &final_local_scope,
            &self.internal_body_env_scope,
            self.node_fonts,
            &inst_ids,
        )
        .with_children(&inst_ids, self.num_consumer_children)
        .with_shadowed_ports(Some(&self.schema.comp_shadowed_ports))
        .with_current_ports(&self.schema.comp_scope_ports)
        .with_enclosing_component(Some(ctx.comp_node_id))
        .with_comp_state(
            Some(&self.comp_ports),
            Some(&self.schema.declared_state_names),
        );

        validate_and_coerce_component_param_types(
            comp_def,
            &self.explicit_unlowered_ports,
            &scope_ctx,
            rewritten_ports,
        )?;

        // Validate assertions in component body
        let mut const_env = HashMap::new();
        const_env.insert(
            crate::compiler::graph::VarId::new(NodeId::WINDOW, "dpi"),
            self.doc
                .window_ports
                .get("dpi")
                .and_then(|e| eval_expr(e, &HashMap::new()).ok())
                .unwrap_or(crate::compiler::value::Value::Float(96.0)),
        );
        for n in &self.doc.nodes {
            if n.name == "Font" {
                for fp in ["size", "weight", "family", "italic"] {
                    if let Some(fp_expr) = n.ports.get(fp) {
                        if let Ok(val) = eval_expr(fp_expr, &const_env) {
                            const_env.insert(crate::compiler::graph::VarId::new(n.id, fp), val);
                        }
                    }
                }
            }
        }
        for (port_name, port_expr) in rewritten_ports.iter() {
            if self.schema.declared_state_names.contains(port_name) {
                continue;
            }
            if let Some(lit) = resolve_ident_or_member_to_literal(port_expr, &scope_ctx) {
                let val = match lit {
                    Literal::Integer(i, _) => crate::compiler::value::Value::Integer(i),
                    Literal::Float(n, _) => crate::compiler::value::Value::Float(n),
                    Literal::Distance(d, _, _) => crate::compiler::value::Value::Distance(d),
                    Literal::Area(a, _, _) => crate::compiler::value::Value::Area(a),
                    Literal::String(s, _) => crate::compiler::value::Value::String(s),
                    Literal::Bool(b, _) => crate::compiler::value::Value::Bool(b),
                    Literal::Color(c, _) => crate::compiler::value::Value::Color(c),
                    Literal::Enum(e, v, _) => crate::compiler::value::Value::Enum {
                        enum_name: e,
                        variant: v,
                    },
                };
                const_env.insert(
                    crate::compiler::graph::VarId::new(ctx.comp_node_id, port_name),
                    val.clone(),
                );
                if let Some(parent) = ctx.parent_id {
                    const_env.insert(crate::compiler::graph::VarId::new(parent, port_name), val);
                }
            } else if let Ok(val) = eval_expr(port_expr, &const_env) {
                const_env.insert(
                    crate::compiler::graph::VarId::new(ctx.comp_node_id, port_name),
                    val.clone(),
                );
                if let Some(parent) = ctx.parent_id {
                    const_env.insert(crate::compiler::graph::VarId::new(parent, port_name), val);
                }
            }
        }

        for item in &comp_def.body {
            if let ComponentBodyItem::Assert(assert_dir) = item {
                eval_assert_directive(
                    assert_dir,
                    &scope_ctx,
                    &const_env,
                    Some(&final_local_scope),
                )?;
            }
        }

        Ok(())
    }

    /// Populates the final `ExpandedNode` for this component instance with its ports,
    /// port type metadata, children, state/env bindings, event handlers, and text spans.
    fn finish(
        self,
        rewritten_ports: HashMap<String, Expr>,
        comp_authored_ports: HashMap<String, Expr>,
    ) {
        let comp_def = self.comp_def;
        let mut node_port_types = HashMap::new();
        node_port_types.insert("clip".to_string(), "Clip".to_string());
        if self.schema.is_span_def || self.schema.is_inline_box_def {
            for sp in [
                "x",
                "y",
                "width",
                "height",
                "baseline",
                "__baseline_offset",
                "left",
                "top",
                "right",
                "bottom",
                "size",
            ] {
                if rewritten_ports.contains_key(sp) {
                    node_port_types.insert(sp.to_string(), "Distance".to_string());
                }
            }
            if rewritten_ports.contains_key("weight") {
                node_port_types.insert("weight".to_string(), "Integer".to_string());
            }
            if rewritten_ports.contains_key("family") {
                node_port_types.insert("family".to_string(), "String".to_string());
            }
        }
        for sp in ["left", "top", "right", "bottom"] {
            if rewritten_ports.contains_key(sp) {
                node_port_types.insert(sp.to_string(), "Distance".to_string());
            }
        }
        for param in &comp_def.params {
            if let Some(ty) = &param.type_annotation {
                node_port_types.insert(
                    param.name.as_str().to_string(),
                    ty.name.as_str().to_string(),
                );
            }
        }
        for (state_name, s) in &self.schema.declared_states {
            if let Some(ty) = &s.type_annotation {
                node_port_types.insert(state_name.clone(), ty.name.as_str().to_string());
            }
        }
        for (let_name, l) in &self.schema.declared_lets {
            if !self.schema.comp_shadowed_ports.contains(let_name) {
                if let Some(ty) = &l.type_annotation {
                    node_port_types.insert(let_name.clone(), ty.name.as_str().to_string());
                }
            }
        }
        for (env_name, e) in &self.schema.declared_envs {
            if !self.schema.comp_shadowed_ports.contains(env_name) {
                if let Some(ty) = &e.type_annotation {
                    node_port_types.insert(env_name.clone(), ty.name.as_str().to_string());
                }
            }
        }
        for (alias_name, a) in &self.schema.declared_aliases {
            if let Some(ty) = &a.type_annotation {
                node_port_types.insert(alias_name.clone(), ty.name.as_str().to_string());
            }
        }

        let node = self.doc.get_node_mut(self.ctx.comp_node_id).unwrap();
        node.is_component = true;
        node.component_path = comp_def.span.file_path();
        node.overload_params = comp_def
            .params
            .iter()
            .filter(|p| p.default_edge.is_none())
            .map(|p| p.name.as_str().to_string())
            .collect();
        node.children = self.all_children_ids;
        node.authored_ports = comp_authored_ports;
        node.private_ports = self
            .schema
            .declared_lets
            .keys()
            .filter(|k| !comp_def.params.iter().any(|p| p.name.as_str() == k.as_str()))
            .cloned()
            .collect();
        for (state_name, s) in &self.schema.declared_states {
            let type_name = s
                .type_annotation
                .as_ref()
                .map(|t| t.name.as_str().to_string());
            node.state_vars.insert(state_name.clone(), type_name);
        }
        for (env_name, e) in &self.schema.declared_envs {
            let type_name = e
                .type_annotation
                .as_ref()
                .map(|t| t.name.as_str().to_string());
            node.env_vars.insert(env_name.clone(), type_name);
        }
        for (port_name, expr) in &rewritten_ports {
            if port_name.starts_with("on_")
                || node_port_types.get(port_name).is_some_and(|t| t == "Handler")
            {
                if let Some(binding) = extract_event_binding(port_name, expr) {
                    node.event_handlers.insert(port_name.clone(), binding);
                }
            }
        }
        if let Some(t) = self.span_full_text {
            node.text_content = Some(t);
            node.text_spans = self.span_text_spans;
            node.inline_boxes = self.span_inline_boxes;
        }
        node.port_types = node_port_types;
        node.ports = rewritten_ports;
    }
}

/// Expands a component instance, wiring parameters, component body, and consumer children.
#[allow(clippy::too_many_arguments)]
pub(super) fn expand_component_instance(
    ctx: &InstanceContext<'_>,
    instance: &ElementNode,
    comp_def: &ComponentDef,
    registry: &HashMap<String, Vec<ComponentDef>>,
    doc: &mut ExpandedDocument,
    lexical_scope: &RefCell<HashMap<String, LexicalBinding>>,
    caller_env_scope: &HashMap<String, EnvEntry>,
    node_fonts: &mut HashMap<NodeId, NodeId>,
) -> Result<(), CompileError> {
    let instantiated_children_ids = RefCell::new(Vec::new());
    let mut expander = ComponentExpander::new(
        ctx,
        instance,
        comp_def,
        registry,
        doc,
        lexical_scope,
        caller_env_scope,
        node_fonts,
        &instantiated_children_ids,
    )?;

    expander.resolve_parameters()?;
    expander.collect_consumer_children()?;
    expander.init_scopes_and_preresolve()?;
    expander.expand_body()?;
    let (mut rewritten_ports, mut comp_authored_ports) = expander.rewrite_and_alias_ports()?;
    expander.apply_post_processing_and_spatial_aliases(
        &mut rewritten_ports,
        &mut comp_authored_ports,
    );
    expander.resolve_aggregations_and_assertions(&mut rewritten_ports)?;
    expander.finish(rewritten_ports, comp_authored_ports);

    Ok(())
}
