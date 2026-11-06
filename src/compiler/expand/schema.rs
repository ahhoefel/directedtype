use super::context::{EnvEntry, LexicalBinding};
use super::typecheck::{validate_content_items_type_annotations, StaticType};
use crate::ast::*;
use crate::compiler::error::{CompileError, NoMatchingOverloadDetails};
use crate::span::Span;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

/// Lazy per-definition cache for `ComponentSchema`.
///
/// Cloning creates an empty cache so that if a `ComponentDef` is cloned and mutated during
/// module import resolution, it never retains a stale schema. `PartialEq` always returns
/// `true` so structural equality of `ComponentDef` ignores the cache.
#[derive(Debug, Default)]
pub struct CachedComponentSchema(OnceLock<ComponentSchema>);

impl Clone for CachedComponentSchema {
    fn clone(&self) -> Self {
        Self(OnceLock::new())
    }
}

impl PartialEq for CachedComponentSchema {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl CachedComponentSchema {
    pub fn get_or_try_init(
        &self,
        comp_def: &ComponentDef,
    ) -> Result<&ComponentSchema, CompileError> {
        if let Some(schema) = self.0.get() {
            return Ok(schema);
        }
        let schema = ComponentSchema::from_def(comp_def)?;
        let _ = self.0.set(schema);
        Ok(self.0.get().expect("OnceLock just initialized"))
    }

    pub fn invalidate(&mut self) {
        self.0 = OnceLock::new();
    }
}

/// Zero-based slot offset of a port within a component's `PortTable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PortSlot(pub u16);

/// Fixed-layout table mapping port names of a component definition to dense `PortSlot` offsets.
#[derive(Debug, Clone, Default)]
pub struct PortTable {
    pub slot_names: Vec<String>,
    pub name_to_slot: HashMap<String, PortSlot>,
}

impl PortTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_or_insert(&mut self, name: &str) -> PortSlot {
        if let Some(&slot) = self.name_to_slot.get(name) {
            return slot;
        }
        let slot = PortSlot(self.slot_names.len() as u16);
        let owned = name.to_string();
        self.slot_names.push(owned.clone());
        self.name_to_slot.insert(owned, slot);
        slot
    }

    #[inline]
    pub fn slot_of(&self, name: &str) -> Option<PortSlot> {
        self.name_to_slot.get(name).copied()
    }

    #[inline]
    pub fn name_of(&self, slot: PortSlot) -> Option<&str> {
        self.slot_names.get(slot.0 as usize).map(|s| s.as_str())
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.slot_names.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.slot_names.is_empty()
    }
}

/// Bitset tracking port presence over a `PortTable`, with a small overflow list for dynamic ports (`on_*`).
#[derive(Debug, Clone, Default)]
pub struct PortMask {
    bits_lo: u64,
    bits_hi: u64,
    overflow: Vec<String>,
}

impl PortMask {
    #[inline]
    pub fn insert(&mut self, table: &PortTable, name: &str) {
        if let Some(slot) = table.slot_of(name) {
            let idx = slot.0 as u32;
            if idx < 64 {
                self.bits_lo |= 1u64 << idx;
                return;
            } else if idx < 128 {
                self.bits_hi |= 1u64 << (idx - 64);
                return;
            }
        }
        if !self.overflow.iter().any(|s| s == name) {
            self.overflow.push(name.to_string());
        }
    }

    #[inline]
    pub fn contains(&self, table: &PortTable, name: &str) -> bool {
        if let Some(slot) = table.slot_of(name) {
            let idx = slot.0 as u32;
            if idx < 64 {
                return (self.bits_lo & (1u64 << idx)) != 0;
            } else if idx < 128 {
                return (self.bits_hi & (1u64 << (idx - 64))) != 0;
            }
        }
        self.overflow.iter().any(|s| s == name)
    }
}

fn expr_references_any_name(expr: &Expr, names: &HashSet<String>) -> bool {
    match expr {
        Expr::Ident(id) => names.contains(id.as_str()),
        Expr::MemberAccess(m) => expr_references_any_name(&m.target, names),
        Expr::Binary(b) => {
            expr_references_any_name(&b.left, names) || expr_references_any_name(&b.right, names)
        }
        Expr::Unary(u) => expr_references_any_name(&u.operand, names),
        Expr::Ternary(t) => {
            expr_references_any_name(&t.condition, names)
                || expr_references_any_name(&t.then_expr, names)
                || expr_references_any_name(&t.else_expr, names)
        }
        Expr::Paren(inner, _) => expr_references_any_name(inner, names),
        Expr::Call(c) => c.args.iter().any(|a| expr_references_any_name(a, names)),
        Expr::MethodCall(mc) => {
            expr_references_any_name(&mc.target, names)
                || mc.args.iter().any(|a| expr_references_any_name(a, names))
        }
        Expr::Node(n) => n.ports.iter().any(|p| expr_references_any_name(&p.expr, names)),
        Expr::Literal(_) => false,
    }
}

/// Pre-validated schema of a `ComponentDef`'s ports, aliases, state, let bindings, and env bindings.
#[derive(Debug, Clone)]
pub struct ComponentSchema {
    pub is_span_def: bool,
    pub is_inline_box_def: bool,
    pub port_table: PortTable,
    pub param_names: HashSet<String>,
    pub handler_param_names: HashSet<String>,
    pub env_param_names: HashSet<String>,
    pub overload_params: Vec<String>,
    pub comp_scope_ports: Vec<String>,
    pub internal_scope_ports: Vec<String>,
    pub declared_aliases: HashMap<String, AliasBinding>,
    pub declared_states: HashMap<String, StateBinding>,
    pub declared_state_names: HashSet<String>,
    pub enum_state_bindings: Vec<(String, StateBinding)>,
    pub all_lets: Vec<LetBinding>,
    pub declared_lets: HashMap<String, LetBinding>,
    pub all_let_names: HashSet<String>,
    pub shadowed_expr_lets: Vec<LetBinding>,
    pub all_envs: Vec<EnvBinding>,
    pub declared_envs: HashMap<String, EnvBinding>,
    pub comp_shadowed_ports: HashSet<String>,
    pub private_ports: HashSet<String>,
    pub static_port_types: HashMap<String, String>,
    pub state_var_types: HashMap<String, Option<String>>,
    pub env_var_types: HashMap<String, Option<String>>,
    pub base_comp_ports: HashMap<String, Expr>,
    pub base_already_rewritten_ports: PortMask,
    pub base_authored_ports: HashMap<String, Expr>,
    pub has_interdependent_lets_or_envs: bool,
    pub has_assertions: bool,
    is_single_text_context: OnceLock<bool>,
}

impl ComponentSchema {
    /// Returns the cached `ComponentSchema` for `comp_def`, validating and building it on first access.
    pub fn for_def(comp_def: &ComponentDef) -> Result<&Self, CompileError> {
        comp_def.schema_cache.get_or_try_init(comp_def)
    }

    /// Returns whether this component places its `\Children()` inside a single `\Text` context,
    /// computing and caching the result once per `ComponentDef`.
    pub fn is_single_text_context(
        &self,
        comp_def: &ComponentDef,
        registry: &HashMap<String, Vec<ComponentDef>>,
    ) -> bool {
        *self.is_single_text_context.get_or_init(|| {
            !self.is_span_def
                && super::text_flow::component_children_is_single_text(comp_def, registry)
        })
    }

    /// Validates and builds the port/binding schema for `comp_def`.
    pub fn from_def(comp_def: &ComponentDef) -> Result<Self, CompileError> {
        let is_span_def = comp_def.kind == ComponentKind::Span;
        let is_inline_box_def = comp_def.kind == ComponentKind::InlineBox;

        let mut param_names = HashSet::with_capacity(comp_def.params.len());
        let mut handler_param_names = HashSet::new();
        let mut env_param_names = HashSet::new();
        let mut overload_params = Vec::new();
        for param in &comp_def.params {
            let name = param.name.as_str().to_string();
            param_names.insert(name.clone());
            if param
                .type_annotation
                .as_ref()
                .is_some_and(|t| t.name.as_str() == "Handler")
            {
                handler_param_names.insert(name.clone());
            }
            if param.is_env {
                env_param_names.insert(name.clone());
            }
            if param.default_edge.is_none() {
                overload_params.push(name);
            }
        }

        let mut comp_scope_ports = vec!["clip".to_string()];
        if is_span_def || is_inline_box_def {
            for p in [
                "x", "y", "width", "height", "left", "top", "right", "bottom", "baseline",
            ] {
                comp_scope_ports.push(p.to_string());
            }
        }

        let mut declared_aliases: HashMap<String, AliasBinding> = HashMap::new();
        let all_aliases = collect_all_component_aliases(comp_def);
        for a in all_aliases {
            let alias_name = a.name.as_str().to_string();
            if alias_name == "parent" {
                return Err(CompileError::ReservedPort {
                    node: comp_def.name.as_str().to_string(),
                    port: "parent".to_string(),
                    span: a.name.span,
                });
            }
            if param_names.contains(&alias_name) {
                return Err(CompileError::DuplicatePort {
                    node: comp_def.name.as_str().to_string(),
                    port: alias_name,
                    span: a.name.span,
                });
            }
            if declared_aliases.contains_key(&alias_name) {
                return Err(CompileError::DuplicatePort {
                    node: comp_def.name.as_str().to_string(),
                    port: alias_name,
                    span: a.name.span,
                });
            }
            if !comp_scope_ports.contains(&alias_name) {
                comp_scope_ports.push(alias_name.clone());
            }
            declared_aliases.insert(alias_name, a.clone());
        }

        let mut declared_states: HashMap<String, StateBinding> = HashMap::new();
        for item in &comp_def.body {
            if let ComponentBodyItem::State(s) = item {
                let state_name = s.name.as_str().to_string();
                if state_name == "parent" {
                    return Err(CompileError::ReservedPort {
                        node: comp_def.name.as_str().to_string(),
                        port: "parent".to_string(),
                        span: s.name.span,
                    });
                }
                if param_names.contains(&state_name) {
                    return Err(CompileError::DuplicatePort {
                        node: comp_def.name.as_str().to_string(),
                        port: state_name,
                        span: s.name.span,
                    });
                }
                if declared_aliases.contains_key(&state_name) {
                    return Err(CompileError::DuplicatePort {
                        node: comp_def.name.as_str().to_string(),
                        port: state_name,
                        span: s.name.span,
                    });
                }
                if declared_states.contains_key(&state_name) {
                    return Err(CompileError::DuplicatePort {
                        node: comp_def.name.as_str().to_string(),
                        port: state_name,
                        span: s.name.span,
                    });
                }
                if !comp_scope_ports.contains(&state_name) {
                    comp_scope_ports.push(state_name.clone());
                }
                declared_states.insert(state_name, s.clone());
            }
        }

        let declared_state_names: HashSet<String> = declared_states.keys().cloned().collect();

        let all_lets = collect_all_component_lets(comp_def);
        let mut declared_lets: HashMap<String, LetBinding> = HashMap::new();
        let mut all_let_names: HashSet<String> = HashSet::new();
        for l in &all_lets {
            let let_name = l.name.as_str().to_string();
            if let_name == "parent" {
                return Err(CompileError::ReservedPort {
                    node: comp_def.name.as_str().to_string(),
                    port: "parent".to_string(),
                    span: l.name.span,
                });
            }
            if declared_aliases.contains_key(&let_name) {
                return Err(CompileError::DuplicatePort {
                    node: comp_def.name.as_str().to_string(),
                    port: let_name,
                    span: l.name.span,
                });
            }
            if declared_states.contains_key(&let_name) {
                return Err(CompileError::DuplicatePort {
                    node: comp_def.name.as_str().to_string(),
                    port: let_name,
                    span: l.name.span,
                });
            }
            if all_let_names.contains(&let_name) {
                return Err(CompileError::DuplicatePort {
                    node: comp_def.name.as_str().to_string(),
                    port: let_name,
                    span: l.name.span,
                });
            }
            all_let_names.insert(let_name.clone());
            if let Some(LetValue::Expr(_)) = &l.value {
                declared_lets.insert(let_name, (*l).clone());
            }
        }

        let mut all_envs: Vec<EnvBinding> = Vec::new();
        let mut declared_envs: HashMap<String, EnvBinding> = HashMap::new();
        let mut has_assertions = false;
        for item in &comp_def.body {
            if let ComponentBodyItem::Assert(_) = item {
                has_assertions = true;
            } else if let ComponentBodyItem::Env(e) = item {
                let env_name = e.name.as_str().to_string();
                if env_name == "parent" {
                    return Err(CompileError::ReservedPort {
                        node: comp_def.name.as_str().to_string(),
                        port: "parent".to_string(),
                        span: e.name.span,
                    });
                }
                if declared_aliases.contains_key(&env_name) {
                    return Err(CompileError::DuplicatePort {
                        node: comp_def.name.as_str().to_string(),
                        port: env_name,
                        span: e.name.span,
                    });
                }
                if declared_states.contains_key(&env_name) {
                    return Err(CompileError::DuplicatePort {
                        node: comp_def.name.as_str().to_string(),
                        port: env_name,
                        span: e.name.span,
                    });
                }
                if all_let_names.contains(&env_name) {
                    return Err(CompileError::DuplicatePort {
                        node: comp_def.name.as_str().to_string(),
                        port: env_name,
                        span: e.name.span,
                    });
                }
                if declared_envs.contains_key(&env_name) {
                    return Err(CompileError::DuplicatePort {
                        node: comp_def.name.as_str().to_string(),
                        port: env_name,
                        span: e.name.span,
                    });
                }
                if !comp_scope_ports.contains(&env_name) {
                    comp_scope_ports.push(env_name.clone());
                }
                all_envs.push(e.clone());
                declared_envs.insert(env_name, e.clone());
            }
        }

        for param in &comp_def.params {
            let name = param.name.as_str();
            if name != "parent" && !comp_scope_ports.iter().any(|p| p == name) {
                comp_scope_ports.push(name.to_string());
            }
        }

        let mut comp_shadowed_ports: HashSet<String> = HashSet::new();
        for p in comp_def
            .params
            .iter()
            .map(|p| p.name.as_str())
            .chain(["clip", "left", "top", "right", "bottom"])
        {
            if all_let_names.contains(p) || declared_envs.contains_key(p) {
                comp_shadowed_ports.insert(p.to_string());
            }
        }

        let all_lets: Vec<LetBinding> = all_lets.into_iter().cloned().collect();
        let shadowed_expr_lets: Vec<LetBinding> = all_lets
            .iter()
            .filter(|l| {
                matches!(l.value, Some(LetValue::Expr(_)))
                    && comp_shadowed_ports.contains(l.name.as_str())
            })
            .cloned()
            .collect();

        let private_ports: HashSet<String> = declared_lets
            .keys()
            .filter(|k| !param_names.contains(*k))
            .cloned()
            .collect();

        let mut static_port_types: HashMap<String, String> = HashMap::new();
        static_port_types.insert("clip".to_string(), "Clip".to_string());
        for param in &comp_def.params {
            if let Some(ty) = &param.type_annotation {
                static_port_types.insert(
                    param.name.as_str().to_string(),
                    ty.name.as_str().to_string(),
                );
            }
        }
        for (state_name, s) in &declared_states {
            if let Some(ty) = &s.type_annotation {
                static_port_types.insert(state_name.clone(), ty.name.as_str().to_string());
            }
        }
        for (let_name, l) in &declared_lets {
            if !comp_shadowed_ports.contains(let_name) {
                if let Some(ty) = &l.type_annotation {
                    static_port_types.insert(let_name.clone(), ty.name.as_str().to_string());
                }
            }
        }
        for (env_name, e) in &declared_envs {
            if !comp_shadowed_ports.contains(env_name) {
                if let Some(ty) = &e.type_annotation {
                    static_port_types.insert(env_name.clone(), ty.name.as_str().to_string());
                }
            }
        }
        for (alias_name, a) in &declared_aliases {
            if let Some(ty) = &a.type_annotation {
                static_port_types.insert(alias_name.clone(), ty.name.as_str().to_string());
            }
        }

        let state_var_types: HashMap<String, Option<String>> = declared_states
            .iter()
            .map(|(k, s)| {
                (
                    k.clone(),
                    s.type_annotation
                        .as_ref()
                        .map(|t| t.name.as_str().to_string()),
                )
            })
            .collect();

        let env_var_types: HashMap<String, Option<String>> = declared_envs
            .iter()
            .map(|(k, e)| {
                (
                    k.clone(),
                    e.type_annotation
                        .as_ref()
                        .map(|t| t.name.as_str().to_string()),
                )
            })
            .collect();

        let has_static_port = |k: &str| {
            param_names.contains(k)
                || declared_states.contains_key(k)
                || declared_lets.contains_key(k)
                || declared_envs.contains_key(k)
                || declared_aliases.contains_key(k)
        };
        if has_static_port("x") && !comp_scope_ports.iter().any(|p| p == "left") {
            comp_scope_ports.push("left".to_string());
            static_port_types
                .entry("left".to_string())
                .or_insert_with(|| "Distance".to_string());
        }
        if has_static_port("y") && !comp_scope_ports.iter().any(|p| p == "top") {
            comp_scope_ports.push("top".to_string());
            static_port_types
                .entry("top".to_string())
                .or_insert_with(|| "Distance".to_string());
        }
        if (has_static_port("x") || has_static_port("left"))
            && has_static_port("width")
            && !comp_scope_ports.iter().any(|p| p == "right")
        {
            comp_scope_ports.push("right".to_string());
            static_port_types
                .entry("right".to_string())
                .or_insert_with(|| "Distance".to_string());
        }
        if (has_static_port("y") || has_static_port("top"))
            && has_static_port("height")
            && !comp_scope_ports.iter().any(|p| p == "bottom")
        {
            comp_scope_ports.push("bottom".to_string());
            static_port_types
                .entry("bottom".to_string())
                .or_insert_with(|| "Distance".to_string());
        }

        let mut internal_scope_ports = comp_scope_ports.clone();
        for let_name in declared_lets.keys() {
            if !internal_scope_ports.contains(let_name) {
                internal_scope_ports.push(let_name.clone());
            }
        }

        let mut port_table = PortTable::new();
        for p in &internal_scope_ports {
            port_table.get_or_insert(p);
        }
        for sp in ["left", "top", "right", "bottom"] {
            port_table.get_or_insert(sp);
        }

        let is_non_unit_literal = |expr: &Expr| -> bool {
            matches!(
                expr,
                Expr::Literal(lit)
                    if !matches!(
                        lit,
                        Literal::Distance(_, DistanceUnit::Pt | DistanceUnit::Em, _)
                    )
            )
        };

        let empty_enums = HashMap::new();
        let mut base_comp_ports = HashMap::with_capacity(
            declared_states.len() + declared_lets.len() + declared_envs.len() + comp_def.params.len() + 4,
        );
        let mut base_already_rewritten_ports = PortMask::default();
        let mut enum_state_bindings = Vec::new();
        for (state_name, s) in &declared_states {
            let needs_enum_lookup = s.default.is_none()
                && s.type_annotation.as_ref().is_some_and(|t| {
                    !matches!(
                        t.name.as_str(),
                        "Boolean"
                            | "String"
                            | "Color"
                            | "Float"
                            | "Integer"
                            | "Distance"
                            | "Area"
                            | "Duration"
                            | "Handler"
                            | "Timer"
                            | "IndefiniteTimer"
                    )
                });
            if needs_enum_lookup {
                enum_state_bindings.push((state_name.clone(), s.clone()));
            } else {
                let mut default_expr =
                    super::rewrite::default_expr_for_state_with_enums(s, &empty_enums);
                let is_handler = state_name.starts_with("on_")
                    || handler_param_names.contains(state_name)
                    || s.type_annotation
                        .as_ref()
                        .is_some_and(|t| t.name.as_str() == "Handler");
                if !is_handler && is_non_unit_literal(&default_expr) {
                    default_expr = super::typecheck::coerce_typed_binding_expr(
                        default_expr,
                        s.type_annotation.as_ref(),
                    );
                    base_already_rewritten_ports.insert(&port_table, state_name);
                }
                base_comp_ports.insert(state_name.clone(), default_expr);
            }
        }
        for (let_name, l) in &declared_lets {
            if !comp_shadowed_ports.contains(let_name) {
                if let Some(LetValue::Expr(raw_expr)) = &l.value {
                    let mut expr = raw_expr.clone();
                    let is_handler = let_name.starts_with("on_")
                        || handler_param_names.contains(let_name)
                        || l.type_annotation
                            .as_ref()
                            .is_some_and(|t| t.name.as_str() == "Handler");
                    if !is_handler && is_non_unit_literal(&expr) {
                        expr = super::typecheck::coerce_typed_binding_expr(
                            expr,
                            l.type_annotation.as_ref(),
                        );
                        base_already_rewritten_ports.insert(&port_table, let_name);
                    }
                    base_comp_ports.insert(let_name.clone(), expr);
                }
            }
        }
        for (env_name, e) in &declared_envs {
            if !comp_shadowed_ports.contains(env_name) {
                if let Some(raw_expr) = &e.value {
                    if !matches!(
                        raw_expr,
                        Expr::Node(elem) if !super::is_value_node_name(elem.name.as_str())
                    ) {
                        let mut expr = raw_expr.clone();
                        let is_handler = env_name.starts_with("on_")
                            || handler_param_names.contains(env_name)
                            || e.type_annotation
                                .as_ref()
                                .is_some_and(|t| t.name.as_str() == "Handler");
                        if !is_handler && is_non_unit_literal(&expr) {
                            expr = super::typecheck::coerce_typed_binding_expr(
                                expr,
                                e.type_annotation.as_ref(),
                            );
                            base_already_rewritten_ports.insert(&port_table, env_name);
                        }
                        base_comp_ports.insert(env_name.clone(), expr);
                    }
                }
            }
        }

        let mut base_authored_ports = HashMap::with_capacity(
            comp_def.params.len()
                + declared_states.len()
                + declared_lets.len()
                + declared_envs.len()
                + declared_aliases.len()
                + 4,
        );
        for param in &comp_def.params {
            if let Some(default_expr) = &param.default_edge {
                base_authored_ports
                    .entry(param.name.as_str().to_string())
                    .or_insert_with(|| default_expr.clone());
            }
        }
        for (state_name, s) in &declared_states {
            let default_expr = super::rewrite::default_expr_for_state_with_enums(s, &empty_enums);
            base_authored_ports.insert(state_name.clone(), default_expr);
        }
        for (let_name, l) in &declared_lets {
            if !comp_shadowed_ports.contains(let_name) {
                if let Some(LetValue::Expr(raw_expr)) = &l.value {
                    base_authored_ports.insert(let_name.clone(), raw_expr.clone());
                }
            }
        }
        for (env_name, e) in &declared_envs {
            if let Some(raw_expr) = &e.value {
                base_authored_ports.insert(env_name.clone(), raw_expr.clone());
            }
        }
        for (alias_name, alias_binding) in &declared_aliases {
            base_authored_ports.insert(alias_name.clone(), alias_binding.value.clone());
        }

        let mut preresolve_names: HashSet<String> = HashSet::new();
        for l in &shadowed_expr_lets {
            preresolve_names.insert(l.name.as_str().to_string());
        }
        for e in &all_envs {
            preresolve_names.insert(e.name.as_str().to_string());
        }
        let has_interdependent_lets_or_envs = if preresolve_names.len() <= 1 {
            false
        } else {
            shadowed_expr_lets.iter().any(|l| {
                matches!(&l.value, Some(LetValue::Expr(expr)) if expr_references_any_name(expr, &preresolve_names))
            }) || all_envs.iter().any(|e| {
                matches!(&e.value, Some(expr) if expr_references_any_name(expr, &preresolve_names))
            })
        };

        Ok(Self {
            is_span_def,
            is_inline_box_def,
            port_table,
            param_names,
            handler_param_names,
            env_param_names,
            overload_params,
            comp_scope_ports,
            internal_scope_ports,
            declared_aliases,
            declared_states,
            declared_state_names,
            enum_state_bindings,
            all_lets,
            declared_lets,
            all_let_names,
            shadowed_expr_lets,
            all_envs,
            declared_envs,
            comp_shadowed_ports,
            private_ports,
            static_port_types,
            state_var_types,
            env_var_types,
            base_comp_ports,
            base_already_rewritten_ports,
            base_authored_ports,
            has_interdependent_lets_or_envs,
            has_assertions,
            is_single_text_context: OnceLock::new(),
        })
    }

    /// Validates that caller-authored or container-injected ports on `instance` do not attempt to
    /// override immutable alias ports or set private state / let / env bindings.
    pub fn validate_caller_ports(
        &self,
        comp_def: &ComponentDef,
        instance: &ElementNode,
    ) -> Result<(), CompileError> {
        for port in &instance.ports {
            let name = port.name.as_str();
            if self.declared_aliases.contains_key(name)
                || matches!(name, "left" | "top" | "right" | "bottom")
            {
                return Err(CompileError::ImmutableAliasPort {
                    node: comp_def.name.as_str().to_string(),
                    port: name.to_string(),
                    span: port.name.span,
                });
            }
            if self.declared_states.contains_key(name) {
                return Err(CompileError::PrivateStatePort {
                    node: comp_def.name.as_str().to_string(),
                    port: name.to_string(),
                    span: port.name.span,
                });
            }
            if self.declared_envs.contains_key(name) && !self.param_names.contains(name) {
                return Err(CompileError::Custom {
                    message: format!(
                        "Node '{}' environmental property '{}' cannot be set as a parameter",
                        comp_def.name.as_str(),
                        name
                    ),
                    span: port.name.span,
                });
            }
            if self.all_let_names.contains(name) && !self.param_names.contains(name) {
                return Err(CompileError::Custom {
                    message: format!(
                        "Node '{}' has no public property '{}'",
                        comp_def.name.as_str(),
                        name
                    ),
                    span: port.name.span,
                });
            }
        }
        Ok(())
    }
}

/// Validates a registered `ComponentDef`'s parameter/body type annotations and kind-specific rules (`\Span`, `\InlineBox`).
pub fn validate_registered_component_def(
    comp_def: &ComponentDef,
    known_enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> Result<(), CompileError> {
    for param in &comp_def.params {
        if let Some(ty) = &param.type_annotation {
            StaticType::from_type_name(ty.name.as_str(), ty.span, known_enums, registry)?;
        }
    }
    validate_content_items_type_annotations(&comp_def.body, known_enums, registry)?;
    if comp_def.kind == ComponentKind::Span {
        for item in &comp_def.body {
            match item {
                ContentItem::Node(elem) if elem.name.as_str() != "Children" => {
                    return Err(CompileError::Custom {
                        message: format!(
                            "Visual component or primitive '\\{}' is not allowed inside a \\Span definition ('{}'); \\Span bodies may only contain port/environment directives and \\Children",
                            elem.name.as_str(),
                            comp_def.name.as_str()
                        ),
                        span: elem.span,
                    });
                }
                ContentItem::Let(l) => {
                    if let Some(LetValue::Node(elem)) = &l.value {
                        if !matches!(elem.name.as_str(), "Font" | "TextBounds" | "Shadow")
                            && !elem.name.as_str().ends_with("Style")
                        {
                            return Err(CompileError::Custom {
                                message: format!(
                                    "Visual component or primitive '\\{}' is not allowed inside a \\Span definition ('{}'); \\Span bodies may only contain port/environment directives and \\Children",
                                    elem.name.as_str(),
                                    comp_def.name.as_str()
                                ),
                                span: elem.span,
                            });
                        }
                    }
                }
                ContentItem::Text(chunk) if !chunk.text.trim().is_empty() => {
                    return Err(CompileError::Custom {
                        message: format!(
                            "Literal text is not allowed directly inside a \\Span definition ('{}'); \\Span bodies may only contain port/environment directives and \\Children",
                            comp_def.name.as_str()
                        ),
                        span: chunk.span,
                    });
                }
                _ => {}
            }
        }
    }
    if comp_def.kind == ComponentKind::InlineBox {
        let all_aliases = collect_all_component_aliases(comp_def);
        for req_port in ["width", "height"] {
            let has_param = comp_def.params.iter().any(|p| p.name.as_str() == req_port);
            let has_alias = all_aliases.iter().any(|a| a.name.as_str() == req_port);
            if !has_param && !has_alias {
                return Err(CompileError::Custom {
                    message: format!(
                        "\\InlineBox '{}' must define '{}' (as a parameter or \\alias)",
                        comp_def.name.as_str(),
                        req_port
                    ),
                    span: comp_def.span,
                });
            }
        }
    }
    Ok(())
}

pub(super) fn collect_nested_aliases<'a>(items: &'a [ContentItem], out: &mut Vec<&'a AliasBinding>) {
    for item in items {
        match item {
            ContentItem::Alias(a) => out.push(a),
            ContentItem::Node(n) => {
                if let Some(content) = &n.content {
                    collect_nested_aliases(&content.items, out);
                }
                for port in &n.ports {
                    if let Expr::Node(elem) = &port.expr {
                        if let Some(content) = &elem.content {
                            collect_nested_aliases(&content.items, out);
                        }
                    }
                }
            }
            ContentItem::Let(l) => {
                if let Some(LetValue::Node(n)) = &l.value {
                    if let Some(content) = &n.content {
                        collect_nested_aliases(&content.items, out);
                    }
                    for port in &n.ports {
                        if let Expr::Node(elem) = &port.expr {
                            if let Some(content) = &elem.content {
                                collect_nested_aliases(&content.items, out);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

pub fn collect_all_component_aliases(comp_def: &ComponentDef) -> Vec<&AliasBinding> {
    let mut aliases = Vec::new();
    collect_nested_aliases(&comp_def.body, &mut aliases);
    aliases
}

pub(super) fn collect_expr_lets<'a>(expr: &'a Expr, out: &mut Vec<&'a LetBinding>) {
    match expr {
        Expr::Node(elem) => collect_element_lets(elem, out),
        Expr::Binary(b) => {
            collect_expr_lets(&b.left, out);
            collect_expr_lets(&b.right, out);
        }
        Expr::Unary(u) => collect_expr_lets(&u.operand, out),
        Expr::Paren(p, _) => collect_expr_lets(p, out),
        Expr::Ternary(t) => {
            collect_expr_lets(&t.condition, out);
            collect_expr_lets(&t.then_expr, out);
            collect_expr_lets(&t.else_expr, out);
        }
        Expr::Call(c) => {
            for a in &c.args {
                collect_expr_lets(a, out);
            }
        }
        Expr::MethodCall(m) => {
            collect_expr_lets(&m.target, out);
            for a in &m.args {
                collect_expr_lets(a, out);
            }
        }
        Expr::MemberAccess(m) => collect_expr_lets(&m.target, out),
        Expr::Literal(_) | Expr::Ident(_) => {}
    }
}

pub(super) fn collect_element_lets<'a>(elem: &'a ElementNode, out: &mut Vec<&'a LetBinding>) {
    for port in &elem.ports {
        collect_expr_lets(&port.expr, out);
    }
    if let Some(content) = &elem.content {
        collect_nested_lets(&content.items, out);
    }
}

pub(super) fn collect_nested_lets<'a>(items: &'a [ContentItem], out: &mut Vec<&'a LetBinding>) {
    for item in items {
        match item {
            ContentItem::Let(l) => {
                out.push(l);
                match &l.value {
                    Some(LetValue::Node(n)) => collect_element_lets(n, out),
                    Some(LetValue::Expr(e)) => collect_expr_lets(e, out),
                    None => {}
                }
            }
            ContentItem::Node(n) => collect_element_lets(n, out),
            ContentItem::Env(e) => {
                if let Some(expr) = &e.value {
                    collect_expr_lets(expr, out);
                }
            }
            ContentItem::Alias(a) => collect_expr_lets(&a.value, out),
            ContentItem::State(s) => {
                if let Some(expr) = &s.default {
                    collect_expr_lets(expr, out);
                }
            }
            ContentItem::Children(c) => {
                for port in &c.ports {
                    collect_expr_lets(&port.expr, out);
                }
            }
            ContentItem::Assert(a) => collect_expr_lets(&a.condition, out),
            ContentItem::Text(_) | ContentItem::ParagraphBreak(_) => {}
        }
    }
}

pub fn collect_all_component_lets(comp_def: &ComponentDef) -> Vec<&LetBinding> {
    let mut lets = Vec::new();
    collect_nested_lets(&comp_def.body, &mut lets);
    lets
}

pub fn collect_all_document_lets(doc: &Document) -> Vec<&LetBinding> {
    let mut lets = Vec::new();
    for item in &doc.items {
        match item {
            Item::Let(l) => {
                lets.push(l);
                match &l.value {
                    Some(LetValue::Node(n)) => collect_element_lets(n, &mut lets),
                    Some(LetValue::Expr(e)) => collect_expr_lets(e, &mut lets),
                    None => {}
                }
            }
            Item::Node(n) => collect_element_lets(n, &mut lets),
            Item::Env(e) => {
                if let Some(expr) = &e.value {
                    collect_expr_lets(expr, &mut lets);
                }
            }
            Item::State(s) => {
                if let Some(expr) = &s.default {
                    collect_expr_lets(expr, &mut lets);
                }
            }
            Item::Component(_) | Item::Use(_) | Item::Enum(_) => {}
        }
    }
    lets
}

/// Statically selects the unique matching component overload for an invocation.
pub(super) fn select_component_overload<'a>(
    overloads: &'a [ComponentDef],
    comp_name: &str,
    elem: &ElementNode,
    env_scope: &HashMap<String, EnvEntry>,
    span: Span,
) -> Result<&'a ComponentDef, CompileError> {
    let explicit_ports: HashSet<String> =
        elem.ports.iter().map(|p| p.name.as_str().to_string()).collect();

    let matching: Vec<&ComponentDef> = overloads
        .iter()
        .filter(|o| {
            let has_param = |name: &str| {
                ComponentSchema::for_def(o)
                    .map(|s| s.param_names.contains(name))
                    .unwrap_or_else(|_| o.params.iter().any(|p| p.name.as_str() == name))
            };
            // 1. All provided ports (caller-authored or container-injected via \Children)
            // must be accepted by this overload.
            for port in &elem.ports {
                let name = port.name.as_str();
                if name != "clip" && !has_param(name) && !name.starts_with("on_") {
                    return false;
                }
            }

            // 2. All required parameters (those without defaults) must be satisfied
            // by caller explicit arguments, container-injected ambient arguments,
            // or by an ambient bound env variable.
            for param in &o.params {
                if param.default_edge.is_none() {
                    let name = param.name.as_str();
                    let satisfied = explicit_ports.contains(name)
                        || (param.is_env && matches!(env_scope.get(name), Some(EnvEntry::Bound(_))));

                    if !satisfied {
                        return false;
                    }
                }
            }

            true
        })
        .collect();

    match matching.len() {
        1 => Ok(matching[0]),
        _ => {
            let mut provided: Vec<String> = explicit_ports
                .into_iter()
                .filter(|p| p != "clip" && !p.starts_with("on_"))
                .collect();
            provided.sort();
            let available_signatures: Vec<Vec<String>> = overloads
                .iter()
                .map(|o| o.params.iter().map(|p| p.name.as_str().to_string()).collect())
                .collect();
            Err(CompileError::NoMatchingOverload(Box::new(
                NoMatchingOverloadDetails {
                    name: comp_name.to_string(),
                    provided_ports: provided,
                    available_signatures,
                    span,
                },
            )))
        }
    }
}

/// Validates that component identity expressions (before `;`) do not depend on layout geometry or layout let formulas.
pub fn validate_component_key(
    node_name: &str,
    key: &ComponentKey,
    lexical_scope: &HashMap<Symbol, LexicalBinding>,
) -> Result<(), CompileError> {
    for part in &key.parts {
        validate_key_expr(node_name, part, lexical_scope)?;
    }
    Ok(())
}

fn validate_key_expr(
    node_name: &str,
    expr: &Expr,
    lexical_scope: &HashMap<Symbol, LexicalBinding>,
) -> Result<(), CompileError> {
    const SPATIAL_PORTS: &[&str] = &[
        "x", "y", "z", "width", "height", "clip", "left", "top", "right", "bottom",
    ];

    match expr {
        Expr::Literal(_) => Ok(()),
        Expr::Ident(id) => {
            let name = id.as_str();
            if name == "true" || name == "false" {
                return Ok(());
            }
            if SPATIAL_PORTS.contains(&name) {
                return Err(CompileError::InvalidComponentKeyDependency {
                    node: node_name.to_string(),
                    name: name.to_string(),
                    span: id.span,
                });
            }
            if name == "self" || name == "parent" || name == "prev" || name == "next" || name == "window" {
                return Err(CompileError::InvalidComponentKeyDependency {
                    node: node_name.to_string(),
                    name: name.to_string(),
                    span: id.span,
                });
            }
            if matches!(
                lexical_scope.get(&id.name),
                Some(LexicalBinding::Expr(_) | LexicalBinding::Node(_) | LexicalBinding::Uninitialized)
            ) {
                return Err(CompileError::InvalidComponentKeyDependency {
                    node: node_name.to_string(),
                    name: name.to_string(),
                    span: id.span,
                });
            }
            Ok(())
        }
        Expr::MemberAccess(m) => {
            let member_name = m.member.as_str();
            if SPATIAL_PORTS.contains(&member_name) {
                return Err(CompileError::InvalidComponentKeyDependency {
                    node: node_name.to_string(),
                    name: member_name.to_string(),
                    span: m.span,
                });
            }
            if matches!(
                lexical_scope.get(&m.member.name),
                Some(LexicalBinding::Expr(_) | LexicalBinding::Node(_) | LexicalBinding::Uninitialized)
            ) {
                return Err(CompileError::InvalidComponentKeyDependency {
                    node: node_name.to_string(),
                    name: member_name.to_string(),
                    span: m.span,
                });
            }
            match m.target.as_ref() {
                Expr::Ident(id) if matches!(id.as_str(), "self" | "parent" | "window") => Ok(()),
                other => validate_key_expr(node_name, other, lexical_scope),
            }
        }
        Expr::Binary(b) => {
            validate_key_expr(node_name, &b.left, lexical_scope)?;
            validate_key_expr(node_name, &b.right, lexical_scope)
        }
        Expr::Unary(u) => validate_key_expr(node_name, &u.operand, lexical_scope),
        Expr::Paren(inner, _) => validate_key_expr(node_name, inner, lexical_scope),
        Expr::Ternary(t) => {
            validate_key_expr(node_name, &t.condition, lexical_scope)?;
            validate_key_expr(node_name, &t.then_expr, lexical_scope)?;
            validate_key_expr(node_name, &t.else_expr, lexical_scope)
        }
        Expr::Call(c) => {
            for arg in &c.args {
                validate_key_expr(node_name, arg, lexical_scope)?;
            }
            Ok(())
        }
        Expr::MethodCall(mc) => {
            validate_key_expr(node_name, &mc.target, lexical_scope)?;
            for arg in &mc.args {
                validate_key_expr(node_name, arg, lexical_scope)?;
            }
            Ok(())
        }
        Expr::Node(n) => Err(CompileError::InvalidComponentKeyDependency {
            node: node_name.to_string(),
            name: n.name.as_str().to_string(),
            span: n.span,
        }),
    }
}
