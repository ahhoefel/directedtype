use crate::ast::{
    BinaryExpr, BinaryOp, CallExpr, ChildrenDirective, ComponentBodyItem, ComponentDef,
    ContentItem, DistanceUnit, ElementNode, EnvBinding, Expr, Ident, LetBinding, LetValue, Literal,
    MemberAccessExpr, PortBinding,
};
use crate::compiler::error::CompileError;
use crate::compiler::eval::eval_expr;
use crate::compiler::expand::aggregation::resolve_and_filter_component_aggregations;
use crate::compiler::expand::children::{
    apply_forwarded_directives, collect_consumer_children, count_consumer_children,
    expand_span_consumer_children,
};
use crate::compiler::expand::context::{
    ConsumerChildEntry, ElementContext, EnvEntry, InstanceContext, LexicalBinding, ScopeContext,
};
use crate::compiler::expand::expand_element;
use crate::compiler::expand::rewrite::{
    default_expr_for_state_with_enums, extract_event_binding, resolve_forward_scope_refs,
    rewrite_expr,
};
use crate::compiler::expand::schema::ComponentSchema;
use crate::compiler::expand::text_flow::{
    component_children_is_single_text, finalize_inline_box_component_ports,
    finalize_span_component_ports,
};
use crate::compiler::expand::typecheck::{
    coerce_typed_binding_expr, infer_expr_literal_for_typecheck, resolve_ident_or_member_to_literal,
    validate_node_type_annotation,
};
use crate::compiler::expanded::{ExpandedDocument, NodeId};
use crate::compiler::text::TextSpan;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// Stateful orchestrator for expanding a single user-defined component instance.
struct ComponentExpander<'a, 'ctx> {
    ctx: &'a InstanceContext<'ctx>,
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
}

impl<'a, 'ctx> ComponentExpander<'a, 'ctx> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        ctx: &'a InstanceContext<'ctx>,
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
        })
    }

    /// Resolves caller arguments, environment bindings, and component parameter defaults (4-Tier Precedence).
    fn resolve_parameters(&mut self) -> Result<(), CompileError> {
        let ctx = self.ctx;
        let instance = self.instance;
        let comp_def = self.comp_def;
        let empty_children: [NodeId; 0] = [];

        // Pre-resolve explicit "font" port if passed on this component instance so sibling ports (e.g. `1em`) see it
        if let Some(font_port) = instance.ports.iter().find(|p| p.name.as_str() == "font") {
            if !matches!(&font_port.expr, Expr::Node(_)) {
                let caller_scope_borrow = self.lexical_scope.borrow();
                let caller_inst_borrow = ctx.enclosing_instantiated_children.map(|c| c.borrow());
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
                );
                if let Ok(Expr::Ident(id)) = rewrite_expr(&font_port.expr, &caller_scope_ctx) {
                    if let Some(fid) = NodeId::from_canonical_name(id.as_str()) {
                        self.node_fonts.insert(ctx.comp_node_id, fid);
                        self.doc.get_node_mut(ctx.comp_node_id).unwrap().font = Some(fid);
                    }
                }
            }
        }

        let mut explicit_ports = HashMap::new();
        for port in &instance.ports {
            let name = port.name.as_str().to_string();
            self.explicit_unlowered_ports
                .insert(name.clone(), port.expr.clone());
            let is_handler_param = comp_def.params.iter().any(|p| {
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

            let resolve_default_expr = |default_expr: &Expr,
                                        doc: &mut ExpandedDocument,
                                        node_fonts: &mut HashMap<NodeId, NodeId>|
             -> Result<Expr, CompileError> {
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
                            doc,
                            node_fonts,
                        )?;
                        if name == "font" && inline_elem.name.as_str() == "Font" {
                            node_fonts.insert(ctx.comp_node_id, child_id);
                            doc.get_node_mut(ctx.comp_node_id).unwrap().font = Some(child_id);
                        }
                        Ok(Expr::Ident(Ident::new(
                            child_id.canonical_name(),
                            inline_elem.span,
                        )))
                    }
                    other => Ok(other.clone()),
                }
            };

            // Tier 4: Explicit caller argument
            if let Some(explicit_expr) = explicit_ports.remove(&name) {
                self.already_rewritten_ports.insert(name.clone());
                self.comp_ports.insert(name, explicit_expr);
            }
            // Tier 2: Lexical Environment (caller_env_scope) - only for env parameters
            else if param.is_env {
                if let Some(env_entry) = self.caller_env_scope.get(&name) {
                    match env_entry {
                        EnvEntry::Bound(env_expr) => {
                            self.already_rewritten_ports.insert(name.clone());
                            self.comp_ports.insert(name, env_expr.clone());
                        }
                        EnvEntry::Tombstone => {
                            // Tombstone halts lookup! Fall back to Tier 1 default
                            if let Some(default_expr) = &param.default_edge {
                                let resolved = resolve_default_expr(
                                    default_expr,
                                    self.doc,
                                    self.node_fonts,
                                )?;
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
                                let resolved = resolve_default_expr(
                                    &default_tb,
                                    self.doc,
                                    self.node_fonts,
                                )?;
                                self.comp_ports.insert(name, resolved);
                            }
                        }
                    }
                } else if let Some(default_expr) = &param.default_edge {
                    let resolved =
                        resolve_default_expr(default_expr, self.doc, self.node_fonts)?;
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
                    let resolved = resolve_default_expr(&default_tb, self.doc, self.node_fonts)?;
                    self.comp_ports.insert(name, resolved);
                }
            }
            // Tier 1: Component Signature Default (for non-env parameters)
            else if let Some(default_expr) = &param.default_edge {
                let resolved = resolve_default_expr(default_expr, self.doc, self.node_fonts)?;
                self.comp_ports.insert(name, resolved);
            }
        }

        // Any remaining explicit ports not defined in signature (e.g. ad-hoc ports)
        let parent_is_text_or_span = ctx
            .parent_id
            .and_then(|pid| self.doc.get_node(pid))
            .is_some_and(|p| p.name == "Text" || p.is_span);
        for (name, expr) in explicit_ports {
            let is_purely_ambient = ctx
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
            if !self.comp_ports.contains_key("x") && !self.schema.declared_aliases.contains_key("x")
            {
                let default_x = if !parent_is_text_or_span {
                    if let Some(pid) = ctx.parent_id {
                        if !pid.is_window()
                            && (ctx.parent_ports.contains(&"left".to_string())
                                || ctx.parent_ports.contains(&"x".to_string()))
                        {
                            let member = if ctx.parent_ports.contains(&"left".to_string()) {
                                "left"
                            } else {
                                "x"
                            };
                            Expr::MemberAccess(MemberAccessExpr {
                                target: Box::new(Expr::Ident(Ident::new(
                                    pid.canonical_name(),
                                    instance.span,
                                ))),
                                member: Ident::new(member, instance.span),
                                span: instance.span,
                            })
                        } else {
                            Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, instance.span))
                        }
                    } else {
                        Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, instance.span))
                    }
                } else {
                    Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, instance.span))
                };
                self.already_rewritten_ports.insert("x".to_string());
                self.comp_ports.insert("x".to_string(), default_x);
            }
            if !self.comp_ports.contains_key("y") && !self.schema.declared_aliases.contains_key("y")
            {
                let default_y = if !parent_is_text_or_span {
                    if let Some(pid) = ctx.parent_id {
                        if !pid.is_window()
                            && (ctx.parent_ports.contains(&"top".to_string())
                                || ctx.parent_ports.contains(&"y".to_string()))
                        {
                            let member = if ctx.parent_ports.contains(&"top".to_string()) {
                                "top"
                            } else {
                                "y"
                            };
                            Expr::MemberAccess(MemberAccessExpr {
                                target: Box::new(Expr::Ident(Ident::new(
                                    pid.canonical_name(),
                                    instance.span,
                                ))),
                                member: Ident::new(member, instance.span),
                                span: instance.span,
                            })
                        } else {
                            Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, instance.span))
                        }
                    } else {
                        Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, instance.span))
                    }
                } else {
                    Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, instance.span))
                };
                self.already_rewritten_ports.insert("y".to_string());
                self.comp_ports.insert("y".to_string(), default_y);
            }
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

        // Record component font if resolved (including Tier 1 / Tier 2 defaults)
        if let Some(font_expr) = self.comp_ports.get("font").cloned() {
            let resolved_font_expr = if let Expr::Ident(id) = &font_expr {
                if NodeId::from_canonical_name(id.as_str()).is_some() {
                    Some(font_expr)
                } else {
                    let caller_scope_borrow = self.lexical_scope.borrow();
                    let temp_ctx = ScopeContext::from_instance(
                        ctx,
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
                    ctx,
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
                    self.node_fonts.insert(ctx.comp_node_id, fid);
                    self.doc.get_node_mut(ctx.comp_node_id).unwrap().font = Some(fid);
                }
            }
        }

        // Default clip port if not explicitly declared
        if !self.comp_ports.contains_key("clip") {
            if let Some(EnvEntry::Bound(clip_expr)) = self.caller_env_scope.get("clip") {
                self.already_rewritten_ports.insert("clip".to_string());
                self.comp_ports.insert("clip".to_string(), clip_expr.clone());
            } else {
                let default_clip_expr = if let Some(parent) = ctx.parent_id {
                    if parent.is_window() {
                        Expr::MemberAccess(MemberAccessExpr {
                            target: Box::new(Expr::Ident(Ident::new(
                                NodeId::WINDOW.canonical_name(),
                                instance.span,
                            ))),
                            member: Ident::new("clip", instance.span),
                            span: instance.span,
                        })
                    } else {
                        Expr::MemberAccess(MemberAccessExpr {
                            target: Box::new(Expr::Ident(Ident::new(
                                parent.canonical_name(),
                                instance.span,
                            ))),
                            member: Ident::new("clip", instance.span),
                            span: instance.span,
                        })
                    }
                } else {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(
                            NodeId::WINDOW.canonical_name(),
                            instance.span,
                        ))),
                        member: Ident::new("clip", instance.span),
                        span: instance.span,
                    })
                };
                self.already_rewritten_ports.insert("clip".to_string());
                self.comp_ports.insert("clip".to_string(), default_clip_expr);
            }
        }

        // Validate that all required parameters (parameters without defaults) have been supplied
        for param in &comp_def.params {
            let name = param.name.as_str();
            if param.default_edge.is_none() && !self.comp_ports.contains_key(name) {
                return Err(CompileError::MissingPort {
                    node: comp_def.name.as_str().to_string(),
                    port: name.to_string(),
                    span: instance.span,
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

    /// Initializes the component's local lexical scope, internal body environment scope,
    /// and children environment scope, then runs fixed-point pre-resolution on `\let` and `\env` expressions.
    #[allow(clippy::type_complexity)]
    fn init_scopes_and_preresolve(
        &mut self,
    ) -> Result<
        (
            HashMap<String, LexicalBinding>,
            HashMap<String, EnvEntry>,
            HashMap<String, EnvEntry>,
        ),
        CompileError,
    > {
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
            if let Some(expr) = self.comp_ports.get(&name) {
                if let Expr::Ident(id) = expr {
                    if let Some(node_id) = NodeId::from_canonical_name(id.as_str()) {
                        local_scope.insert(name.clone(), LexicalBinding::Node(node_id));
                        continue;
                    }
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

        // Setup internal_body_env_scope (sealed black box for internal elements)
        let mut internal_body_env_scope: HashMap<String, EnvEntry> = HashMap::new();
        internal_body_env_scope.insert(
            "clip".to_string(),
            EnvEntry::Bound(Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(Expr::Ident(Ident::new(
                    ctx.comp_node_id.canonical_name(),
                    instance.span,
                ))),
                member: Ident::new("clip", instance.span),
                span: instance.span,
            })),
        );
        for param in &comp_def.params {
            if param.is_env {
                let env_bound_expr = if param.name.as_str() == "font" {
                    if let Some(&fid) = self.node_fonts.get(&ctx.comp_node_id) {
                        Expr::Ident(Ident::new(fid.canonical_name(), instance.span))
                    } else {
                        Expr::MemberAccess(MemberAccessExpr {
                            target: Box::new(Expr::Ident(Ident::new(
                                ctx.comp_node_id.canonical_name(),
                                instance.span,
                            ))),
                            member: param.name.clone(),
                            span: instance.span,
                        })
                    }
                } else if let Some(Expr::Ident(id)) = self.comp_ports.get(param.name.as_str()) {
                    if NodeId::from_canonical_name(id.as_str()).is_some() {
                        Expr::Ident(id.clone())
                    } else {
                        Expr::MemberAccess(MemberAccessExpr {
                            target: Box::new(Expr::Ident(Ident::new(
                                ctx.comp_node_id.canonical_name(),
                                instance.span,
                            ))),
                            member: param.name.clone(),
                            span: instance.span,
                        })
                    }
                } else {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(
                            ctx.comp_node_id.canonical_name(),
                            instance.span,
                        ))),
                        member: param.name.clone(),
                        span: instance.span,
                    })
                };
                internal_body_env_scope.insert(
                    param.name.as_str().to_string(),
                    EnvEntry::Bound(env_bound_expr),
                );
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

        // Setup children_env_scope (for \Children)
        let mut children_env_scope = self.caller_env_scope.clone();
        children_env_scope.insert(
            "clip".to_string(),
            EnvEntry::Bound(Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(Expr::Ident(Ident::new(
                    ctx.comp_node_id.canonical_name(),
                    instance.span,
                ))),
                member: Ident::new("clip", instance.span),
                span: instance.span,
            })),
        );
        for param in &comp_def.params {
            if param.is_env {
                let env_bound_expr = if param.name.as_str() == "font" {
                    if let Some(&fid) = self.node_fonts.get(&ctx.comp_node_id) {
                        Expr::Ident(Ident::new(fid.canonical_name(), instance.span))
                    } else {
                        Expr::MemberAccess(MemberAccessExpr {
                            target: Box::new(Expr::Ident(Ident::new(
                                ctx.comp_node_id.canonical_name(),
                                instance.span,
                            ))),
                            member: param.name.clone(),
                            span: instance.span,
                        })
                    }
                } else if let Some(Expr::Ident(id)) = self.comp_ports.get(param.name.as_str()) {
                    if NodeId::from_canonical_name(id.as_str()).is_some() {
                        Expr::Ident(id.clone())
                    } else {
                        Expr::MemberAccess(MemberAccessExpr {
                            target: Box::new(Expr::Ident(Ident::new(
                                ctx.comp_node_id.canonical_name(),
                                instance.span,
                            ))),
                            member: param.name.clone(),
                            span: instance.span,
                        })
                    }
                } else {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(
                            ctx.comp_node_id.canonical_name(),
                            instance.span,
                        ))),
                        member: param.name.clone(),
                        span: instance.span,
                    })
                };
                children_env_scope.insert(
                    param.name.as_str().to_string(),
                    EnvEntry::Bound(env_bound_expr),
                );
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
                    let is_param_shadow = self
                        .schema
                        .comp_shadowed_ports
                        .contains(let_binding.name.as_str());
                    if is_param_shadow {
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
                        if let Ok(rewritten) = rewrite_expr(raw_expr, &scope_ctx) {
                            let rewritten = coerce_typed_binding_expr(
                                rewritten,
                                let_binding.type_annotation.as_ref(),
                            );
                            if let Some(LexicalBinding::Expr(existing)) =
                                local_scope.get(let_binding.name.as_str())
                            {
                                if existing != &rewritten {
                                    local_scope.insert(
                                        let_binding.name.as_str().to_string(),
                                        LexicalBinding::Expr(rewritten),
                                    );
                                    changed = true;
                                }
                            } else {
                                local_scope.insert(
                                    let_binding.name.as_str().to_string(),
                                    LexicalBinding::Expr(rewritten),
                                );
                                changed = true;
                            }
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
                            if let Ok(rewritten) = rewrite_expr(raw_expr, &scope_ctx) {
                                let rewritten = coerce_typed_binding_expr(
                                    rewritten,
                                    env_binding.type_annotation.as_ref(),
                                );
                                if let Some(LexicalBinding::Expr(existing)) =
                                    local_scope.get(env_binding.name.as_str())
                                {
                                    if existing != &rewritten {
                                        local_scope.insert(
                                            env_binding.name.as_str().to_string(),
                                            LexicalBinding::Expr(rewritten),
                                        );
                                        changed = true;
                                    }
                                } else {
                                    local_scope.insert(
                                        env_binding.name.as_str().to_string(),
                                        LexicalBinding::Expr(rewritten),
                                    );
                                    changed = true;
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok((local_scope, internal_body_env_scope, children_env_scope))
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_body_let(
        &mut self,
        let_binding: &LetBinding,
        local_scope: &RefCell<HashMap<String, LexicalBinding>>,
        internal_body_env_scope: &mut HashMap<String, EnvEntry>,
        children_env_scope: &mut HashMap<String, EnvEntry>,
        consumer_child_entries: &mut [ConsumerChildEntry<'ctx>],
        all_children_ids: &mut Vec<NodeId>,
    ) -> Result<(), CompileError> {
        let ctx = self.ctx;
        let comp_def = self.comp_def;
        let is_env_param_shadow = comp_def
            .params
            .iter()
            .any(|p| p.is_env && p.name.as_str() == let_binding.name.as_str());

        if let_binding.value.is_none() || is_env_param_shadow {
            // Shield/Swallower pattern or Firewall Tombstone
            children_env_scope.insert(let_binding.name.as_str().to_string(), EnvEntry::Tombstone);
            internal_body_env_scope
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
            local_scope.borrow_mut().insert(
                let_binding.name.as_str().to_string(),
                LexicalBinding::Node(node_id),
            );
            for entry in consumer_child_entries.iter_mut() {
                for (k, v) in children_env_scope.iter() {
                    entry.caller_env_scope.insert(k.clone(), v.clone());
                }
            }
            let elem_ctx = ElementContext {
                parent_id: Some(ctx.comp_node_id),
                parent_name: Some(comp_def.name.as_str()),
                prev_sibling_id: None,
                has_next_sibling: false,
                parent_ports: &self.internal_scope_ports,
                lexical_scope: local_scope,
                env_scope: internal_body_env_scope,
                enclosing_component_id: Some(ctx.comp_node_id),
                enclosing_num_children: self.num_consumer_children,
                enclosing_instantiated_children: Some(self.instantiated_children_ids),
                shadowed_ports: Some(&self.schema.comp_shadowed_ports),
                is_let: true,
                ambient_authored_ports: None,
                enums: ctx.enums,
                active_scope_id: ctx.active_scope_id,
                consumer_children: Some(consumer_child_entries),
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
            all_children_ids.push(node_id);
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
                    let scope_borrow = local_scope.borrow();
                    let inst_borrow = self.instantiated_children_ids.borrow();
                    let scope_ctx = ScopeContext::from_instance(
                        ctx,
                        &self.doc.nodes,
                        &scope_borrow,
                        internal_body_env_scope,
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
                local_scope.borrow_mut().insert(
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
                children_env_scope.insert(
                    let_binding.name.as_str().to_string(),
                    EnvEntry::Bound(member_expr.clone()),
                );
                internal_body_env_scope.insert(
                    let_binding.name.as_str().to_string(),
                    EnvEntry::Bound(member_expr),
                );
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_body_env(
        &mut self,
        env_binding: &EnvBinding,
        local_scope: &RefCell<HashMap<String, LexicalBinding>>,
        internal_body_env_scope: &mut HashMap<String, EnvEntry>,
        children_env_scope: &mut HashMap<String, EnvEntry>,
        consumer_child_entries: &mut [ConsumerChildEntry<'ctx>],
        all_children_ids: &mut Vec<NodeId>,
    ) -> Result<(), CompileError> {
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
                    for entry in consumer_child_entries.iter_mut() {
                        for (k, v) in children_env_scope.iter() {
                            entry.caller_env_scope.insert(k.clone(), v.clone());
                        }
                    }
                    let elem_ctx = ElementContext {
                        parent_id: Some(ctx.comp_node_id),
                        parent_name: Some(comp_def.name.as_str()),
                        prev_sibling_id: None,
                        has_next_sibling: false,
                        parent_ports: &self.internal_scope_ports,
                        lexical_scope: local_scope,
                        env_scope: internal_body_env_scope,
                        enclosing_component_id: Some(ctx.comp_node_id),
                        enclosing_num_children: self.num_consumer_children,
                        enclosing_instantiated_children: Some(self.instantiated_children_ids),
                        shadowed_ports: Some(&self.schema.comp_shadowed_ports),
                        is_let: false,
                        ambient_authored_ports: None,
                        enums: ctx.enums,
                        active_scope_id: ctx.active_scope_id,
                        consumer_children: Some(consumer_child_entries),
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
                    all_children_ids.push(node_id);
                    local_scope.borrow_mut().insert(
                        env_binding.name.as_str().to_string(),
                        LexicalBinding::Node(node_id),
                    );
                    let bound_expr = Expr::Ident(Ident::new(node_id.canonical_name(), elem.span));
                    children_env_scope.insert(
                        env_binding.name.as_str().to_string(),
                        EnvEntry::Bound(bound_expr.clone()),
                    );
                    internal_body_env_scope.insert(
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
                        let scope_borrow = local_scope.borrow();
                        let inst_borrow = self.instantiated_children_ids.borrow();
                        let scope_ctx = ScopeContext::from_instance(
                            ctx,
                            &self.doc.nodes,
                            &scope_borrow,
                            internal_body_env_scope,
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
                    local_scope.borrow_mut().insert(
                        env_binding.name.as_str().to_string(),
                        LexicalBinding::Expr(rewritten.clone()),
                    );
                    children_env_scope.insert(
                        env_binding.name.as_str().to_string(),
                        EnvEntry::Bound(rewritten.clone()),
                    );
                    internal_body_env_scope.insert(
                        env_binding.name.as_str().to_string(),
                        EnvEntry::Bound(rewritten),
                    );
                }
            }
        } else {
            // Environmental Hole Tombstone
            local_scope.borrow_mut().insert(
                env_binding.name.as_str().to_string(),
                LexicalBinding::Uninitialized,
            );
            children_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Tombstone);
            internal_body_env_scope
                .insert(env_binding.name.as_str().to_string(), EnvEntry::Tombstone);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_body_children(
        &mut self,
        dir: &ChildrenDirective,
        local_scope: &RefCell<HashMap<String, LexicalBinding>>,
        children_env_scope: &mut HashMap<String, EnvEntry>,
        consumer_child_entries: &[ConsumerChildEntry<'ctx>],
        all_children_ids: &mut Vec<NodeId>,
    ) -> Result<(), CompileError> {
        let ctx = self.ctx;
        let instance = self.instance;
        let comp_def = self.comp_def;

        for port in &dir.ports {
            if port.name.as_str() == "parent" {
                return Err(CompileError::ReservedPort {
                    node: "Children".to_string(),
                    port: "parent".to_string(),
                    span: port.name.span,
                });
            }
        }
        if let Some(slot) = &instance.content {
            for item in &slot.items {
                if let ContentItem::Env(env_binding) = item {
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
                                let elem_ctx = ElementContext {
                                    parent_id: Some(ctx.comp_node_id),
                                    parent_name: Some(comp_def.name.as_str()),
                                    prev_sibling_id: None,
                                    has_next_sibling: false,
                                    parent_ports: &self.schema.comp_scope_ports,
                                    lexical_scope: self.lexical_scope,
                                    env_scope: children_env_scope,
                                    enclosing_component_id: ctx.enclosing_component_id,
                                    enclosing_num_children: ctx.enclosing_num_children,
                                    enclosing_instantiated_children: ctx
                                        .enclosing_instantiated_children,
                                    shadowed_ports: ctx.shadowed_ports,
                                    is_let: false,
                                    ambient_authored_ports: None,
                                    enums: ctx.enums,
                                    active_scope_id: ctx.active_scope_id,
                                    consumer_children: None,
                                };
                                let env_node_id = expand_element(
                                    elem,
                                    &elem_ctx,
                                    self.registry,
                                    self.doc,
                                    self.node_fonts,
                                )?;
                                self.doc.get_node_mut(env_node_id).unwrap().var_name =
                                    Some(env_binding.name.as_str().to_string());
                                all_children_ids.push(env_node_id);
                                self.lexical_scope.borrow_mut().insert(
                                    env_binding.name.as_str().to_string(),
                                    LexicalBinding::Node(env_node_id),
                                );
                                let bound_expr =
                                    Expr::Ident(Ident::new(env_node_id.canonical_name(), elem.span));
                                children_env_scope.insert(
                                    env_binding.name.as_str().to_string(),
                                    EnvEntry::Bound(bound_expr),
                                );
                                if elem.name.as_str() == "Font" {
                                    self.node_fonts.insert(env_node_id, env_node_id);
                                }
                            }
                            _ => {
                                let rewritten = {
                                    let scope_borrow = self.lexical_scope.borrow();
                                    let empty_children: [NodeId; 0] = [];
                                    let caller_inst_borrow =
                                        ctx.enclosing_instantiated_children.map(|c| c.borrow());
                                    let caller_child_ids: &[NodeId] = match &caller_inst_borrow {
                                        Some(b) => b.as_slice(),
                                        None => &empty_children,
                                    };
                                    let scope_ctx = ScopeContext::from_instance(
                                        ctx,
                                        &self.doc.nodes,
                                        &scope_borrow,
                                        children_env_scope,
                                        self.node_fonts,
                                        caller_child_ids,
                                    );
                                    rewrite_expr(raw_expr, &scope_ctx)?
                                };
                                let rewritten = coerce_typed_binding_expr(
                                    rewritten,
                                    env_binding.type_annotation.as_ref(),
                                );
                                self.doc.local_bindings.push((
                                    rewritten.clone(),
                                    env_binding
                                        .type_annotation
                                        .as_ref()
                                        .map(|t| t.name.as_str().to_string()),
                                    raw_expr.span(),
                                ));
                                self.lexical_scope.borrow_mut().insert(
                                    env_binding.name.as_str().to_string(),
                                    LexicalBinding::Expr(rewritten.clone()),
                                );
                                children_env_scope.insert(
                                    env_binding.name.as_str().to_string(),
                                    EnvEntry::Bound(rewritten),
                                );
                            }
                        }
                    } else {
                        self.lexical_scope.borrow_mut().insert(
                            env_binding.name.as_str().to_string(),
                            LexicalBinding::Uninitialized,
                        );
                        children_env_scope
                            .insert(env_binding.name.as_str().to_string(), EnvEntry::Tombstone);
                    }
                }
            }
        }
        if !self.schema.is_span_def {
            let mut last_consumer_child_id = None;
            let mut dir_children_ids = Vec::with_capacity(self.num_consumer_children);
            for (child_idx, entry) in consumer_child_entries.iter().enumerate() {
                let has_next_consumer_child = child_idx + 1 < self.num_consumer_children;
                let child_elem = &entry.elem;
                let var_name = &entry.var_name;
                let mut merged_ports = HashMap::new();
                let mut authored_ambient_ports = HashMap::new();

                let mut entry_env_scope = entry.caller_env_scope.clone();
                for (k, v) in children_env_scope.iter() {
                    entry_env_scope.insert(k.clone(), v.clone());
                }

                // Ambient rules from \Children are authored inside the component (Tier 3)
                let child_node_id = NodeId(self.doc.nodes.len());
                {
                    let local_borrow = local_scope.borrow();
                    let inst_borrow = self.instantiated_children_ids.borrow();
                    let mut ambient_scope_ctx = ScopeContext::from_instance(
                        ctx,
                        &self.doc.nodes,
                        &local_borrow,
                        &entry_env_scope,
                        self.node_fonts,
                        &inst_borrow,
                    )
                    .with_parent(
                        Some(ctx.comp_node_id),
                        Some(comp_def.name.as_str()),
                        &self.internal_scope_ports,
                    )
                    .with_siblings(last_consumer_child_id, has_next_consumer_child)
                    .with_children(&inst_borrow, self.num_consumer_children)
                    .with_child_node(Some(child_node_id))
                    .with_shadowed_ports(Some(&self.schema.comp_shadowed_ports))
                    .with_enclosing_component(Some(ctx.comp_node_id))
                    .with_comp_state(
                        Some(&self.comp_ports),
                        Some(&self.schema.declared_state_names),
                    );
                    ambient_scope_ctx.current_node = child_node_id;

                    for ambient in &dir.ports {
                        ambient_scope_ctx.is_event_port = ambient.name.as_str().starts_with("on_");
                        let rewritten = rewrite_expr(&ambient.expr, &ambient_scope_ctx)?;
                        merged_ports.insert(ambient.name.as_str().to_string(), rewritten);
                        authored_ambient_ports
                            .insert(ambient.name.as_str().to_string(), ambient.expr.clone());
                    }
                }

                // Forwarded \Children(...) directives from outer component wrappers
                apply_forwarded_directives(
                    &entry.forwarded_directives,
                    child_node_id,
                    last_consumer_child_id,
                    has_next_consumer_child,
                    &self.doc.nodes,
                    self.node_fonts,
                    ctx.enums,
                    &mut merged_ports,
                    &mut authored_ambient_ports,
                )?;

                // Explicit child ports override ambient rules (Tier 4 > Tier 3)
                for explicit in &child_elem.ports {
                    merged_ports
                        .insert(explicit.name.as_str().to_string(), explicit.expr.clone());
                    authored_ambient_ports.remove(explicit.name.as_str());
                }

                let mut wired_elem = child_elem.clone();
                wired_elem.ports = merged_ports
                    .into_iter()
                    .map(|(name, expr)| PortBinding {
                        name: Ident::new(name, child_elem.span),
                        expr,
                        span: child_elem.span,
                    })
                    .collect();

                let elem_ctx = ElementContext {
                    parent_id: Some(ctx.comp_node_id),
                    parent_name: Some(comp_def.name.as_str()),
                    prev_sibling_id: last_consumer_child_id,
                    has_next_sibling: has_next_consumer_child,
                    parent_ports: &self.schema.comp_scope_ports,
                    lexical_scope: entry.caller_lexical_scope,
                    env_scope: &entry_env_scope,
                    enclosing_component_id: entry.caller_enclosing_component_id,
                    enclosing_num_children: entry.caller_enclosing_num_children,
                    enclosing_instantiated_children: entry.caller_enclosing_instantiated_children,
                    shadowed_ports: entry.caller_shadowed_ports,
                    is_let: var_name.is_some(),
                    ambient_authored_ports: Some(&authored_ambient_ports),
                    enums: ctx.enums,
                    active_scope_id: ctx.active_scope_id,
                    consumer_children: None,
                };
                let child_id = expand_element(
                    &wired_elem,
                    &elem_ctx,
                    self.registry,
                    self.doc,
                    self.node_fonts,
                )?;

                if let Some(v_name) = var_name {
                    self.doc.get_node_mut(child_id).unwrap().var_name = Some(v_name.clone());
                    entry
                        .caller_lexical_scope
                        .borrow_mut()
                        .insert(v_name.clone(), LexicalBinding::Node(child_id));
                }

                for inst_list in &entry.instantiated_ids {
                    inst_list.borrow_mut().push(child_id);
                }
                all_children_ids.push(child_id);
                dir_children_ids.push(child_id);
                last_consumer_child_id = Some(child_id);
            }
            for pair in dir_children_ids.windows(2) {
                local_scope.borrow_mut().insert(
                    format!("__next_of_{}", pair[0].canonical_name()),
                    LexicalBinding::Node(pair[1]),
                );
            }
        }
        Ok(())
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

    // 1. Resolve parameters (4-tier precedence)
    expander.resolve_parameters()?;

    // 2. Collect consumer children passed to this component instance
    let mut consumer_child_entries = collect_consumer_children(
        ctx,
        instance,
        comp_def,
        &expander.schema.comp_scope_ports,
        expander.schema.is_span_def,
        expander.is_single_text_context,
        lexical_scope,
        caller_env_scope,
        &instantiated_children_ids,
        registry,
    )?;
    debug_assert_eq!(expander.num_consumer_children, consumer_child_entries.len());

    // 3. Initialize scopes and pre-resolve component body let/env expressions
    let (local_scope, mut internal_body_env_scope, mut children_env_scope) =
        expander.init_scopes_and_preresolve()?;

    // 4. Process body items in declaration order (Painter's Algorithm)
    let start_node_idx = expander.doc.nodes.len();
    let start_local_bindings_idx = expander.doc.local_bindings.len();
    let local_scope = RefCell::new(local_scope);
    let mut all_children_ids = Vec::new();
    let mut last_child_id: Option<NodeId> = None;

    for item in &comp_def.body {
        match item {
            ComponentBodyItem::Let(let_binding) => {
                expander.expand_body_let(
                    let_binding,
                    &local_scope,
                    &mut internal_body_env_scope,
                    &mut children_env_scope,
                    &mut consumer_child_entries,
                    &mut all_children_ids,
                )?;
            }
            ComponentBodyItem::Env(env_binding) => {
                expander.expand_body_env(
                    env_binding,
                    &local_scope,
                    &mut internal_body_env_scope,
                    &mut children_env_scope,
                    &mut consumer_child_entries,
                    &mut all_children_ids,
                )?;
            }
            ComponentBodyItem::Node(body_node) => {
                for entry in &mut consumer_child_entries {
                    for (k, v) in &children_env_scope {
                        entry.caller_env_scope.insert(k.clone(), v.clone());
                    }
                }
                let elem_ctx = ElementContext {
                    parent_id: Some(ctx.comp_node_id),
                    parent_name: Some(comp_def.name.as_str()),
                    prev_sibling_id: last_child_id,
                    has_next_sibling: false,
                    parent_ports: &expander.internal_scope_ports,
                    lexical_scope: &local_scope,
                    env_scope: &internal_body_env_scope,
                    enclosing_component_id: Some(ctx.comp_node_id),
                    enclosing_num_children: expander.num_consumer_children,
                    enclosing_instantiated_children: Some(&instantiated_children_ids),
                    shadowed_ports: Some(&expander.schema.comp_shadowed_ports),
                    is_let: false,
                    ambient_authored_ports: None,
                    enums: ctx.enums,
                    active_scope_id: ctx.active_scope_id,
                    consumer_children: Some(&consumer_child_entries),
                };
                let body_id = expand_element(
                    body_node,
                    &elem_ctx,
                    expander.registry,
                    expander.doc,
                    expander.node_fonts,
                )?;
                all_children_ids.push(body_id);
                last_child_id = Some(body_id);
            }
            ComponentBodyItem::Children(dir) => {
                expander.expand_body_children(
                    dir,
                    &local_scope,
                    &mut children_env_scope,
                    &consumer_child_entries,
                    &mut all_children_ids,
                )?;
            }
            ComponentBodyItem::Alias(_)
            | ComponentBodyItem::State(_)
            | ComponentBodyItem::Text(_)
            | ComponentBodyItem::ParagraphBreak(_)
            | ComponentBodyItem::Assert(_) => {}
        }
    }

    let mut span_full_text: Option<String> = None;
    let mut span_text_spans: Vec<TextSpan> = Vec::new();
    let mut span_inline_boxes: Vec<crate::compiler::text::InlineBoxSpan> = Vec::new();
    if expander.schema.is_span_def {
        let (full_text, spans, ibs) = expand_span_consumer_children(
            ctx,
            comp_def,
            &expander.schema.comp_scope_ports,
            &consumer_child_entries,
            &children_env_scope,
            expander.registry,
            expander.doc,
            expander.node_fonts,
            &mut all_children_ids,
        )?;
        span_full_text = Some(full_text);
        span_text_spans = spans;
        span_inline_boxes = ibs;
    }

    let mut final_local_scope = local_scope.into_inner();
    let instantiated_children_ids = std::mem::take(&mut *instantiated_children_ids.borrow_mut());
    for (idx, &child_id) in instantiated_children_ids.iter().enumerate() {
        final_local_scope.insert(
            format!("__child_of_{}_{}", ctx.comp_node_id.canonical_name(), idx),
            LexicalBinding::Node(child_id),
        );
    }

    // Post-expansion forward reference resolution for all nodes created within this component
    for node_idx in start_node_idx..expander.doc.nodes.len() {
        let node = &mut expander.doc.nodes[node_idx];
        for expr in node.ports.values_mut() {
            *expr = resolve_forward_scope_refs(expr, &final_local_scope);
        }
    }
    for binding_idx in start_local_bindings_idx..expander.doc.local_bindings.len() {
        let (expr, _, _) = &mut expander.doc.local_bindings[binding_idx];
        *expr = resolve_forward_scope_refs(expr, &final_local_scope);
    }

    // 5. Rewrite component ports and aliases in scope
    let scope_ctx = ScopeContext::from_instance(
        ctx,
        &expander.doc.nodes,
        &final_local_scope,
        &internal_body_env_scope,
        expander.node_fonts,
        &instantiated_children_ids,
    )
    .with_children(&instantiated_children_ids, expander.num_consumer_children)
    .with_shadowed_ports(Some(&expander.schema.comp_shadowed_ports))
    .with_current_ports(&expander.schema.comp_scope_ports)
    .with_enclosing_component(Some(ctx.comp_node_id))
    .with_comp_state(
        Some(&expander.comp_ports),
        Some(&expander.schema.declared_state_names),
    );

    let mut rewritten_ports = HashMap::new();
    for (port_name, port_expr) in &expander.comp_ports {
        if expander.already_rewritten_ports.contains(port_name) {
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
            if let Some(l) = expander.schema.declared_lets.get(port_name) {
                rw = coerce_typed_binding_expr(rw, l.type_annotation.as_ref());
            } else if let Some(e) = expander.schema.declared_envs.get(port_name) {
                rw = coerce_typed_binding_expr(rw, e.type_annotation.as_ref());
            } else if let Some(s) = expander.schema.declared_states.get(port_name) {
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
    for (state_name, s) in &expander.schema.declared_states {
        let default_expr = default_expr_for_state_with_enums(s, ctx.enums);
        comp_authored_ports.insert(state_name.clone(), default_expr);
    }
    for (let_name, l) in &expander.schema.declared_lets {
        if !expander.schema.comp_shadowed_ports.contains(let_name) {
            if let Some(LetValue::Expr(raw_expr)) = &l.value {
                comp_authored_ports.insert(let_name.clone(), raw_expr.clone());
            }
        }
    }
    for (env_name, e) in &expander.schema.declared_envs {
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

    // 6. Process explicitly declared component aliases
    for (alias_name, alias_binding) in &expander.schema.declared_aliases {
        let mut rewritten_alias = rewrite_expr(&alias_binding.value, &scope_ctx)?;
        rewritten_alias = resolve_forward_scope_refs(&rewritten_alias, &final_local_scope);
        rewritten_alias =
            coerce_typed_binding_expr(rewritten_alias, alias_binding.type_annotation.as_ref());
        rewritten_ports.insert(alias_name.clone(), rewritten_alias);
        comp_authored_ports.insert(alias_name.clone(), alias_binding.value.clone());
    }

    if expander.schema.is_span_def {
        finalize_span_component_ports(
            ctx.comp_node_id,
            instance.span,
            expander.doc,
            &final_local_scope,
            expander.node_fonts,
            &mut rewritten_ports,
            &span_full_text,
            &mut span_text_spans,
        );
    }

    // 7. Default spatial aliases if their inputs are present and no alias was explicitly declared
    let self_ident = Expr::Ident(Ident::new(ctx.comp_node_id.canonical_name(), instance.span));
    let self_x = Expr::MemberAccess(MemberAccessExpr {
        target: Box::new(self_ident.clone()),
        member: Ident::new("x", instance.span),
        span: instance.span,
    });
    let self_y = Expr::MemberAccess(MemberAccessExpr {
        target: Box::new(self_ident.clone()),
        member: Ident::new("y", instance.span),
        span: instance.span,
    });
    let self_width = Expr::MemberAccess(MemberAccessExpr {
        target: Box::new(self_ident.clone()),
        member: Ident::new("width", instance.span),
        span: instance.span,
    });
    let self_height = Expr::MemberAccess(MemberAccessExpr {
        target: Box::new(self_ident.clone()),
        member: Ident::new("height", instance.span),
        span: instance.span,
    });

    if !rewritten_ports.contains_key("left") && rewritten_ports.contains_key("x") {
        rewritten_ports.insert("left".to_string(), self_x.clone());
        comp_authored_ports.insert(
            "left".to_string(),
            Expr::Ident(Ident::new("x", instance.span)),
        );
    }
    if !rewritten_ports.contains_key("top") && rewritten_ports.contains_key("y") {
        rewritten_ports.insert("top".to_string(), self_y.clone());
        comp_authored_ports.insert(
            "top".to_string(),
            Expr::Ident(Ident::new("y", instance.span)),
        );
    }
    if !rewritten_ports.contains_key("right")
        && (rewritten_ports.contains_key("x") || rewritten_ports.contains_key("left"))
        && rewritten_ports.contains_key("width")
    {
        let left_base = if rewritten_ports.contains_key("x") {
            self_x
        } else {
            Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(self_ident.clone()),
                member: Ident::new("left", instance.span),
                span: instance.span,
            })
        };
        let left_authored = if rewritten_ports.contains_key("x") {
            "x"
        } else {
            "left"
        };
        let right_expr = Expr::Binary(BinaryExpr {
            op: BinaryOp::Add,
            left: Box::new(left_base),
            right: Box::new(self_width),
            span: instance.span,
        });
        rewritten_ports.insert("right".to_string(), right_expr);
        comp_authored_ports.insert(
            "right".to_string(),
            Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(Expr::Ident(Ident::new(left_authored, instance.span))),
                right: Box::new(Expr::Ident(Ident::new("width", instance.span))),
                span: instance.span,
            }),
        );
    }
    if !rewritten_ports.contains_key("bottom")
        && (rewritten_ports.contains_key("y") || rewritten_ports.contains_key("top"))
        && rewritten_ports.contains_key("height")
    {
        let top_base = if rewritten_ports.contains_key("y") {
            self_y
        } else {
            Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(self_ident),
                member: Ident::new("top", instance.span),
                span: instance.span,
            })
        };
        let top_authored = if rewritten_ports.contains_key("y") {
            "y"
        } else {
            "top"
        };
        let bottom_expr = Expr::Binary(BinaryExpr {
            op: BinaryOp::Add,
            left: Box::new(top_base),
            right: Box::new(self_height),
            span: instance.span,
        });
        rewritten_ports.insert("bottom".to_string(), bottom_expr);
        comp_authored_ports.insert(
            "bottom".to_string(),
            Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(Expr::Ident(Ident::new(top_authored, instance.span))),
                right: Box::new(Expr::Ident(Ident::new("height", instance.span))),
                span: instance.span,
            }),
        );
    }

    if expander.schema.is_inline_box_def {
        finalize_inline_box_component_ports(
            ctx.comp_node_id,
            start_node_idx,
            instance.span,
            &expander.doc.nodes,
            &expander.schema.declared_state_names,
            &mut rewritten_ports,
        );
    }

    // 8. Resolve aggregations, coerce parameter types, and evaluate assertions
    resolve_and_filter_component_aggregations(
        ctx.comp_node_id,
        comp_def.name.as_str(),
        &instantiated_children_ids,
        &expander.schema.declared_state_names,
        &mut rewritten_ports,
        expander.doc,
        start_node_idx,
        start_local_bindings_idx,
    )?;

    for (idx, &child_id) in instantiated_children_ids.iter().enumerate() {
        final_local_scope.insert(
            format!("__agg_child_of_{}_{}", ctx.comp_node_id.canonical_name(), idx),
            LexicalBinding::Node(child_id),
        );
    }

    let scope_ctx = ScopeContext::from_instance(
        ctx,
        &expander.doc.nodes,
        &final_local_scope,
        &internal_body_env_scope,
        expander.node_fonts,
        &instantiated_children_ids,
    )
    .with_children(&instantiated_children_ids, expander.num_consumer_children)
    .with_shadowed_ports(Some(&expander.schema.comp_shadowed_ports))
    .with_current_ports(&expander.schema.comp_scope_ports)
    .with_enclosing_component(Some(ctx.comp_node_id))
    .with_comp_state(
        Some(&expander.comp_ports),
        Some(&expander.schema.declared_state_names),
    );

    // Validate parameter types and coerce 0 -> 0px (for Distance) or Integer -> Float (for Float)
    for param in &comp_def.params {
        if let Some(type_ref) = &param.type_annotation {
            let expected_type = type_ref.name.as_str();
            let name = param.name.as_str();
            let unlowered_lit = expander
                .explicit_unlowered_ports
                .get(name)
                .or(param.default_edge.as_ref())
                .and_then(|e| match e {
                    Expr::Literal(lit) => Some(lit.clone()),
                    _ => None,
                });
            if let Some(expr) = rewritten_ports.get_mut(name) {
                if let Some(lit) =
                    unlowered_lit.or_else(|| infer_expr_literal_for_typecheck(expr, &scope_ctx))
                {
                    if !lit.matches_type_name(expected_type) {
                        return Err(CompileError::TypeMismatch {
                            expected: expected_type.to_string(),
                            actual: lit.type_name().to_string(),
                            span: expr.span(),
                        });
                    }
                }
                if expected_type == "Distance" {
                    if let Expr::Literal(Literal::Integer(0, span)) = expr {
                        *expr = Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, *span));
                    }
                } else if expected_type == "Area" {
                    if let Expr::Literal(Literal::Integer(0, span)) = expr {
                        *expr = Expr::Literal(Literal::Area(0.0, DistanceUnit::Px, *span));
                    }
                } else if expected_type == "Float" {
                    if let Expr::Literal(Literal::Integer(i, span)) = expr {
                        *expr = Expr::Literal(Literal::Float(*i as f64, *span));
                    }
                } else if expected_type == "Integer" {
                    if let Expr::Literal(Literal::Integer(0, span)) = expr {
                        *expr = Expr::Call(CallExpr {
                            callee: Ident::new("to_integer", *span),
                            args: vec![Expr::Literal(Literal::Integer(0, *span))],
                            span: *span,
                        });
                    }
                }
            }
        }
    }

    // Validate assertions in component body
    let mut const_env = HashMap::new();
    const_env.insert(
        crate::compiler::graph::VarId::new(NodeId::WINDOW, "dpi"),
        expander
            .doc
            .window_ports
            .get("dpi")
            .and_then(|e| eval_expr(e, &HashMap::new()).ok())
            .unwrap_or(crate::compiler::value::Value::Float(96.0)),
    );
    for n in &expander.doc.nodes {
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
        if let Some(states) = scope_ctx.declared_state_names {
            if states.contains(port_name) {
                continue;
            }
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
            let mut rewritten = rewrite_expr(&assert_dir.condition, &scope_ctx)?;
            rewritten = resolve_forward_scope_refs(&rewritten, &final_local_scope);
            match eval_expr(&rewritten, &const_env) {
                Ok(val) => {
                    if !val.is_truthy() {
                        let msg = assert_dir.message.clone().unwrap_or_else(|| {
                            format!("Assertion failed: {}", assert_dir.condition)
                        });
                        return Err(CompileError::AssertionFailed {
                            message: msg,
                            span: assert_dir.span,
                        });
                    }
                }
                Err(e) => {
                    return Err(CompileError::Custom {
                        message: format!("Could not evaluate assertion condition: {}", e),
                        span: assert_dir.span,
                    });
                }
            }
        }
    }

    // 9. Finalize component node ports, port_types, and metadata
    let mut node_port_types = HashMap::new();
    node_port_types.insert("clip".to_string(), "Clip".to_string());
    if expander.schema.is_span_def || expander.schema.is_inline_box_def {
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
    for (state_name, s) in &expander.schema.declared_states {
        if let Some(ty) = &s.type_annotation {
            node_port_types.insert(state_name.clone(), ty.name.as_str().to_string());
        }
    }
    for (let_name, l) in &expander.schema.declared_lets {
        if !expander.schema.comp_shadowed_ports.contains(let_name) {
            if let Some(ty) = &l.type_annotation {
                node_port_types.insert(let_name.clone(), ty.name.as_str().to_string());
            }
        }
    }
    for (env_name, e) in &expander.schema.declared_envs {
        if !expander.schema.comp_shadowed_ports.contains(env_name) {
            if let Some(ty) = &e.type_annotation {
                node_port_types.insert(env_name.clone(), ty.name.as_str().to_string());
            }
        }
    }
    for (alias_name, a) in &expander.schema.declared_aliases {
        if let Some(ty) = &a.type_annotation {
            node_port_types.insert(alias_name.clone(), ty.name.as_str().to_string());
        }
    }

    let node = expander.doc.get_node_mut(ctx.comp_node_id).unwrap();
    node.is_component = true;
    node.component_path = comp_def.span.file_path();
    node.overload_params = comp_def
        .params
        .iter()
        .filter(|p| p.default_edge.is_none())
        .map(|p| p.name.as_str().to_string())
        .collect();
    node.children = all_children_ids;
    node.authored_ports = comp_authored_ports;
    node.private_ports = expander
        .schema
        .declared_lets
        .keys()
        .filter(|k| !comp_def.params.iter().any(|p| p.name.as_str() == k.as_str()))
        .cloned()
        .collect();
    for (state_name, s) in &expander.schema.declared_states {
        let type_name = s
            .type_annotation
            .as_ref()
            .map(|t| t.name.as_str().to_string());
        node.state_vars.insert(state_name.clone(), type_name);
    }
    for (env_name, e) in &expander.schema.declared_envs {
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
    if let Some(t) = span_full_text {
        node.text_content = Some(t);
        node.text_spans = span_text_spans;
        node.inline_boxes = span_inline_boxes;
    }
    node.port_types = node_port_types;
    node.ports = rewritten_ports;

    Ok(())
}
