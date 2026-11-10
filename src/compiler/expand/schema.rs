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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortTable {
    pub slot_names: Vec<String>,
    pub name_to_slot: HashMap<String, PortSlot>,
    pub sym_to_slot: HashMap<Symbol, PortSlot>,
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
        let sym = Symbol::new(name);
        self.slot_names.push(owned.clone());
        self.name_to_slot.insert(owned, slot);
        self.sym_to_slot.insert(sym, slot);
        slot
    }

    #[inline]
    pub fn slot_of(&self, name: &str) -> Option<PortSlot> {
        self.name_to_slot.get(name).copied()
    }

    #[inline]
    pub fn slot_of_sym(&self, sym: Symbol) -> Option<PortSlot> {
        self.sym_to_slot.get(&sym).copied()
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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortMask {
    bits_lo: u64,
    bits_hi: u64,
    overflow: Vec<String>,
}

impl PortMask {
    #[inline]
    pub fn insert_slot(&mut self, slot: PortSlot) {
        let idx = slot.0 as u32;
        if idx < 64 {
            self.bits_lo |= 1u64 << idx;
        } else if idx < 128 {
            self.bits_hi |= 1u64 << (idx - 64);
        }
    }

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
    pub fn remove(&mut self, table: &PortTable, name: &str) {
        if let Some(slot) = table.slot_of(name) {
            let idx = slot.0 as u32;
            if idx < 64 {
                self.bits_lo &= !(1u64 << idx);
                return;
            } else if idx < 128 {
                self.bits_hi &= !(1u64 << (idx - 64));
                return;
            }
        }
        self.overflow.retain(|s| s != name);
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

    #[inline]
    pub fn contains_sym(&self, table: &PortTable, sym: Symbol, name: &str) -> bool {
        if let Some(slot) = table.slot_of_sym(sym) {
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
        Expr::ExprDef(def) => {
            def.params.iter().any(|p| {
                p.default_edge
                    .as_ref()
                    .is_some_and(|e| expr_references_any_name(e, names))
            }) || expr_references_any_name(&def.body, names)
        }
        Expr::Match(m) => {
            m.scrutinee
                .as_ref()
                .is_some_and(|s| expr_references_any_name(s, names))
                || m.arms.iter().any(|arm| {
                    (match &arm.pattern {
                        MatchPattern::Condition(c) => expr_references_any_name(c, names),
                        MatchPattern::Variants(_) | MatchPattern::Wildcard(_) => false,
                    }) || expr_references_any_name(&arm.body, names)
                })
        }
        Expr::Literal(_) => false,
    }
}

/// A pre-compiled relocatable expression template for a component port (`\state`, `\let`, `\env`, `\alias`, or param default).
#[derive(Debug, Clone)]
pub struct CompiledPortTemplate {
    pub name: String,
    pub sym: Symbol,
    pub slot: Option<PortSlot>,
    pub expr: Expr,
    pub is_const_no_self: bool,
    pub const_val: Option<crate::compiler::value::Value>,
    pub has_ternary: bool,
}

/// A pre-compiled component parameter default value.
#[derive(Debug, Clone)]
pub enum CompiledParamDefault {
    Constant {
        expr: Expr,
        const_val: Option<crate::compiler::value::Value>,
    },
    Template(CompiledPortTemplate),
}

/// Pre-compiled relocatable templates and pre-verified static port types for a `ComponentDef`.
#[derive(Debug, Clone)]
pub struct CompiledComponentTemplates {
    pub param_defaults: HashMap<Symbol, CompiledParamDefault>,
    pub state_templates: Vec<CompiledPortTemplate>,
    pub let_templates: Vec<CompiledPortTemplate>,
    pub env_templates: Vec<CompiledPortTemplate>,
    pub alias_templates: Vec<CompiledPortTemplate>,
    pub preverified_port_types: std::sync::Arc<HashMap<Symbol, StaticType>>,
    pub preverified_template_mask: PortMask,
}

/// Pre-validated schema of a `ComponentDef`'s ports, aliases, state, let bindings, and env bindings.
#[derive(Debug, Clone)]
pub struct ComponentSchema {
    pub is_span_def: bool,
    pub is_inline_box_def: bool,
    pub port_table: std::sync::Arc<PortTable>,
    pub param_names: HashSet<String>,
    pub param_syms: HashSet<Symbol>,
    pub template_self_ports: HashSet<Symbol>,
    pub handler_param_names: HashSet<String>,
    pub expr_param_names: HashSet<String>,
    pub env_param_names: HashSet<String>,
    pub overload_params: std::sync::Arc<Vec<String>>,
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
    pub private_ports: std::sync::Arc<HashSet<String>>,
    pub static_port_types: std::sync::Arc<HashMap<String, String>>,
    pub state_var_types: std::sync::Arc<HashMap<String, Option<String>>>,
    pub env_var_types: std::sync::Arc<HashMap<String, Option<String>>>,
    pub base_comp_ports: HashMap<String, Expr>,
    pub base_already_rewritten_ports: PortMask,
    pub base_authored_ports: std::sync::Arc<HashMap<String, Expr>>,
    pub has_interdependent_lets_or_envs: bool,
    pub has_assertions: bool,
    is_single_text_context: OnceLock<bool>,
    compiled_templates: OnceLock<CompiledComponentTemplates>,
}

impl ComponentSchema {
    /// Returns the cached `ComponentSchema` for `comp_def`, validating and building it on first access.
    pub fn for_def(comp_def: &ComponentDef) -> Result<&Self, CompileError> {
        comp_def.schema_cache.get_or_try_init(comp_def)
    }

    /// Returns the pre-compiled relocatable templates and pre-verified port types for `comp_def`,
    /// building and caching them on first access.
    pub fn ensure_templates(
        &self,
        comp_def: &ComponentDef,
        enums: &HashMap<String, EnumDef>,
        registry: &HashMap<String, Vec<ComponentDef>>,
    ) -> &CompiledComponentTemplates {
        self.compiled_templates
            .get_or_init(|| CompiledComponentTemplates::build(self, comp_def, enums, registry))
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
        let mut param_syms = HashSet::with_capacity(comp_def.params.len());
        let mut handler_param_names = HashSet::new();
        let mut expr_param_names = HashSet::new();
        let mut env_param_names = HashSet::new();
        let mut overload_params = Vec::new();
        for param in &comp_def.params {
            let name = param.name.as_str().to_string();
            param_names.insert(name.clone());
            param_syms.insert(param.name.name);
            if param
                .type_annotation
                .as_ref()
                .is_some_and(|t| t.name.as_str() == "Handler")
            {
                handler_param_names.insert(name.clone());
            }
            if param
                .type_annotation
                .as_ref()
                .is_some_and(|t| matches!(t.name.as_str(), "Expr" | "Expression"))
                || matches!(&param.default_edge, Some(Expr::ExprDef(_)))
            {
                expr_param_names.insert(name.clone());
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
                let is_expr_def = matches!(&l.value, Some(LetValue::Expr(Expr::ExprDef(_))))
                    || l.type_annotation
                        .as_ref()
                        .is_some_and(|t| matches!(t.name.as_str(), "Expr" | "Expression"));
                matches!(l.value, Some(LetValue::Expr(_)))
                    && (comp_shadowed_ports.contains(l.name.as_str()) || is_expr_def)
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
        if is_span_def || is_inline_box_def {
            for p in [
                "x", "y", "width", "height", "left", "top", "right", "bottom", "baseline",
            ] {
                static_port_types.insert(p.to_string(), "Distance".to_string());
            }
        }
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
        let static_left = has_static_port("x") && !comp_scope_ports.iter().any(|p| p == "left");
        if static_left {
            comp_scope_ports.push("left".to_string());
            static_port_types
                .entry("left".to_string())
                .or_insert_with(|| "Distance".to_string());
        }
        let static_top = has_static_port("y") && !comp_scope_ports.iter().any(|p| p == "top");
        if static_top {
            comp_scope_ports.push("top".to_string());
            static_port_types
                .entry("top".to_string())
                .or_insert_with(|| "Distance".to_string());
        }
        let static_right = (has_static_port("x") || has_static_port("left"))
            && has_static_port("width")
            && !comp_scope_ports.iter().any(|p| p == "right");
        if static_right {
            comp_scope_ports.push("right".to_string());
            static_port_types
                .entry("right".to_string())
                .or_insert_with(|| "Distance".to_string());
        }
        let static_bottom = (has_static_port("y") || has_static_port("top"))
            && has_static_port("height")
            && !comp_scope_ports.iter().any(|p| p == "bottom");
        if static_bottom {
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

        let mut template_self_ports: HashSet<Symbol> = HashSet::with_capacity(internal_scope_ports.len());
        if !comp_shadowed_ports.contains("clip") && !declared_envs.contains_key("clip") {
            template_self_ports.insert(Symbol::CLIP);
        }
        for sp in [
            "x", "y", "width", "height", "left", "top", "right", "bottom", "baseline",
        ] {
            if comp_scope_ports.iter().any(|p| p == sp)
                && !comp_shadowed_ports.contains(sp)
                && !declared_envs.contains_key(sp)
                && !param_names.contains(sp)
            {
                template_self_ports.insert(Symbol::new(sp));
            }
        }
        for param in &comp_def.params {
            let name = param.name.as_str();
            let is_non_node_typed = param.type_annotation.as_ref().is_some_and(|t| {
                !matches!(
                    t.name.as_str(),
                    "Font"
                        | "Node"
                        | "ViewPort"
                        | "Rect"
                        | "Text"
                        | "Line"
                        | "Clip"
                        | "Box"
                        | "Anchor"
                        | "AnchorScope"
                        | "Handler"
                        | "Expr"
                        | "Expression"
                )
            }) && !matches!(&param.default_edge, Some(Expr::Node(_) | Expr::ExprDef(_)));
            if is_non_node_typed
                && !comp_shadowed_ports.contains(name)
                && !declared_envs.contains_key(name)
                && !handler_param_names.contains(name)
                && !expr_param_names.contains(name)
                && !name.starts_with("on_")
            {
                template_self_ports.insert(param.name.name);
            }
        }
        for (state_name, s) in &declared_states {
            let is_handler = state_name.starts_with("on_")
                || handler_param_names.contains(state_name)
                || s.type_annotation
                    .as_ref()
                    .is_some_and(|t| t.name.as_str() == "Handler");
            if !is_handler && !comp_shadowed_ports.contains(state_name) {
                template_self_ports.insert(s.name.name);
            }
        }
        for (let_name, l) in &declared_lets {
            let is_handler_or_expr = let_name.starts_with("on_")
                || handler_param_names.contains(let_name)
                || expr_param_names.contains(let_name)
                || l.type_annotation
                    .as_ref()
                    .is_some_and(|t| matches!(t.name.as_str(), "Handler" | "Expr" | "Expression"))
                || matches!(&l.value, Some(LetValue::Expr(Expr::ExprDef(_))));
            if !is_handler_or_expr
                && !comp_shadowed_ports.contains(let_name)
                && !env_param_names.contains(let_name)
                && matches!(l.value, Some(LetValue::Expr(_)))
            {
                template_self_ports.insert(l.name.name);
            }
        }
        for (alias_name, a) in &declared_aliases {
            let is_handler_or_expr = alias_name.starts_with("on_")
                || handler_param_names.contains(alias_name)
                || expr_param_names.contains(alias_name)
                || a.type_annotation
                    .as_ref()
                    .is_some_and(|t| matches!(t.name.as_str(), "Handler" | "Expr" | "Expression"))
                || matches!(&a.value, Expr::ExprDef(_));
            if !is_handler_or_expr && !comp_shadowed_ports.contains(alias_name) {
                template_self_ports.insert(a.name.name);
            }
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
        let def_span = comp_def.span;
        if static_left {
            base_authored_ports
                .entry("left".to_string())
                .or_insert_with(|| Expr::Ident(Ident::from_symbol(Symbol::X, def_span)));
        }
        if static_top {
            base_authored_ports
                .entry("top".to_string())
                .or_insert_with(|| Expr::Ident(Ident::from_symbol(Symbol::Y, def_span)));
        }
        if static_right {
            let left_member = if has_static_port("x") {
                Symbol::X
            } else {
                Symbol::LEFT
            };
            base_authored_ports
                .entry("right".to_string())
                .or_insert_with(|| {
                    Expr::Binary(BinaryExpr {
                        op: BinaryOp::Add,
                        left: Box::new(Expr::Ident(Ident::from_symbol(left_member, def_span))),
                        right: Box::new(Expr::Ident(Ident::from_symbol(Symbol::WIDTH, def_span))),
                        span: def_span,
                    })
                });
        }
        if static_bottom {
            let top_member = if has_static_port("y") {
                Symbol::Y
            } else {
                Symbol::TOP
            };
            base_authored_ports
                .entry("bottom".to_string())
                .or_insert_with(|| {
                    Expr::Binary(BinaryExpr {
                        op: BinaryOp::Add,
                        left: Box::new(Expr::Ident(Ident::from_symbol(top_member, def_span))),
                        right: Box::new(Expr::Ident(Ident::from_symbol(Symbol::HEIGHT, def_span))),
                        span: def_span,
                    })
                });
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
            port_table: std::sync::Arc::new(port_table),
            param_names,
            param_syms,
            template_self_ports,
            handler_param_names,
            expr_param_names,
            env_param_names,
            overload_params: std::sync::Arc::new(overload_params),
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
            private_ports: std::sync::Arc::new(private_ports),
            static_port_types: std::sync::Arc::new(static_port_types),
            state_var_types: std::sync::Arc::new(state_var_types),
            env_var_types: std::sync::Arc::new(env_var_types),
            base_comp_ports,
            base_already_rewritten_ports,
            base_authored_ports: std::sync::Arc::new(base_authored_ports),
            has_interdependent_lets_or_envs,
            has_assertions,
            is_single_text_context: OnceLock::new(),
            compiled_templates: OnceLock::new(),
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
        if let Some(default_expr) = &param.default_edge {
            super::typecheck::validate_expr_type_annotations(default_expr, known_enums, registry)?;
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
        Expr::ExprDef(def) => {
            for p in &def.params {
                if let Some(d) = &p.default_edge {
                    collect_expr_lets(d, out);
                }
            }
            collect_expr_lets(&def.body, out);
        }
        Expr::Match(m) => {
            if let Some(s) = &m.scrutinee {
                collect_expr_lets(s, out);
            }
            for arm in &m.arms {
                if let MatchPattern::Condition(c) = &arm.pattern {
                    collect_expr_lets(c, out);
                }
                collect_expr_lets(&arm.body, out);
            }
        }
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
                Some(
                    LexicalBinding::Expr(_)
                        | LexicalBinding::Member {
                            is_state: false,
                            ..
                        }
                        | LexicalBinding::Node(_)
                        | LexicalBinding::Uninitialized
                )
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
                Some(
                    LexicalBinding::Expr(_)
                        | LexicalBinding::Member {
                            is_state: false,
                            ..
                        }
                        | LexicalBinding::Node(_)
                        | LexicalBinding::Uninitialized
                )
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
        Expr::ExprDef(def) => Err(CompileError::InvalidComponentKeyDependency {
            node: node_name.to_string(),
            name: "Expr".to_string(),
            span: def.span,
        }),
        Expr::Match(m) => {
            if let Some(s) = &m.scrutinee {
                validate_key_expr(node_name, s, lexical_scope)?;
            }
            for arm in &m.arms {
                if let MatchPattern::Condition(c) = &arm.pattern {
                    validate_key_expr(node_name, c, lexical_scope)?;
                }
                validate_key_expr(node_name, &arm.body, lexical_scope)?;
            }
            Ok(())
        }
    }
}

pub(super) fn expr_to_const_value(expr: &Expr) -> Option<crate::compiler::value::Value> {
    match expr {
        Expr::Literal(Literal::Integer(i, _)) => Some(crate::compiler::value::Value::Integer(*i)),
        Expr::Literal(Literal::Float(n, _)) => Some(crate::compiler::value::Value::Float(*n)),
        Expr::Literal(Literal::Distance(d, DistanceUnit::Px, _)) => {
            Some(crate::compiler::value::Value::Distance(*d))
        }
        Expr::Literal(Literal::Area(a, DistanceUnit::Px, _)) => {
            Some(crate::compiler::value::Value::Area(*a))
        }
        Expr::Literal(Literal::Duration(d, _, _)) => {
            Some(crate::compiler::value::Value::Duration(*d))
        }
        Expr::Literal(Literal::String(s, _)) => {
            Some(crate::compiler::value::Value::String(s.clone()))
        }
        Expr::Literal(Literal::Bool(b, _)) => Some(crate::compiler::value::Value::Bool(*b)),
        Expr::Literal(Literal::Color(c, _)) => {
            Some(crate::compiler::value::Value::Color(c.clone()))
        }
        Expr::Literal(Literal::Enum(e, v, _)) => Some(crate::compiler::value::Value::Enum {
            enum_name: e.clone(),
            variant: v.clone(),
        }),
        Expr::Call(call) if call.callee.as_str() == "to_integer" && call.args.len() == 1 => {
            if let Expr::Literal(Literal::Integer(i, _)) = &call.args[0] {
                Some(crate::compiler::value::Value::Integer(*i))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn expr_has_template_self_or_ternary(expr: &Expr) -> (bool, bool) {
    match expr {
        Expr::Literal(_) => (false, false),
        Expr::Ident(id) => (
            id.node_id()
                .is_some_and(|n| n.is_template_self()),
            false,
        ),
        Expr::MemberAccess(m) => expr_has_template_self_or_ternary(&m.target),
        Expr::Binary(b) => {
            let (s1, t1) = expr_has_template_self_or_ternary(&b.left);
            let (s2, t2) = expr_has_template_self_or_ternary(&b.right);
            (s1 || s2, t1 || t2)
        }
        Expr::Unary(u) => expr_has_template_self_or_ternary(&u.operand),
        Expr::Paren(inner, _) => expr_has_template_self_or_ternary(inner),
        Expr::Ternary(t) => {
            let (s1, _) = expr_has_template_self_or_ternary(&t.condition);
            let (s2, _) = expr_has_template_self_or_ternary(&t.then_expr);
            let (s3, _) = expr_has_template_self_or_ternary(&t.else_expr);
            (s1 || s2 || s3, true)
        }
        Expr::Call(c) => {
            let mut has_s = false;
            let mut has_t = false;
            for arg in &c.args {
                let (s, t) = expr_has_template_self_or_ternary(arg);
                has_s |= s;
                has_t |= t;
            }
            (has_s, has_t)
        }
        Expr::MethodCall(mc) => {
            let (mut has_s, mut has_t) = expr_has_template_self_or_ternary(&mc.target);
            for arg in &mc.args {
                let (s, t) = expr_has_template_self_or_ternary(arg);
                has_s |= s;
                has_t |= t;
            }
            (has_s, has_t)
        }
        Expr::Node(_) | Expr::ExprDef(_) => (false, false),
        Expr::Match(m) => {
            let mut has_s = false;
            if let Some(s) = &m.scrutinee {
                let (s1, _) = expr_has_template_self_or_ternary(s);
                has_s |= s1;
            }
            for arm in &m.arms {
                if let MatchPattern::Condition(c) = &arm.pattern {
                    let (s1, _) = expr_has_template_self_or_ternary(c);
                    has_s |= s1;
                }
                let (s2, _) = expr_has_template_self_or_ternary(&arm.body);
                has_s |= s2;
            }
            (has_s, true)
        }
    }
}

fn collect_template_self_deps(expr: &Expr, deps: &mut Vec<Symbol>) {
    match expr {
        Expr::MemberAccess(m) => {
            if let Expr::Ident(id) = m.target.as_ref() {
                if id.node_id().is_some_and(|n| n.is_template_self()) {
                    if !deps.contains(&m.member.name) {
                        deps.push(m.member.name);
                    }
                    return;
                }
            }
            collect_template_self_deps(&m.target, deps);
        }
        Expr::Binary(b) => {
            collect_template_self_deps(&b.left, deps);
            collect_template_self_deps(&b.right, deps);
        }
        Expr::Unary(u) => collect_template_self_deps(&u.operand, deps),
        Expr::Paren(inner, _) => collect_template_self_deps(inner, deps),
        Expr::Ternary(t) => {
            collect_template_self_deps(&t.condition, deps);
            collect_template_self_deps(&t.then_expr, deps);
            collect_template_self_deps(&t.else_expr, deps);
        }
        Expr::Call(c) => {
            for arg in &c.args {
                collect_template_self_deps(arg, deps);
            }
        }
        Expr::MethodCall(mc) => {
            collect_template_self_deps(&mc.target, deps);
            for arg in &mc.args {
                collect_template_self_deps(arg, deps);
            }
        }
        Expr::Match(m) => {
            if let Some(s) = &m.scrutinee {
                collect_template_self_deps(s, deps);
            }
            for arm in &m.arms {
                if let MatchPattern::Condition(c) = &arm.pattern {
                    collect_template_self_deps(c, deps);
                }
                collect_template_self_deps(&arm.body, deps);
            }
        }
        Expr::Literal(_) | Expr::Ident(_) | Expr::Node(_) | Expr::ExprDef(_) => {}
    }
}

fn infer_template_branch_zero_coercion(
    other: &Expr,
    zero_span: Span,
    schema: &ComponentSchema,
) -> Option<Option<Expr>> {
    match other {
        Expr::Literal(Literal::Distance(_, _, _)) => Some(Some(Expr::Literal(Literal::Distance(
            0.0,
            DistanceUnit::Px,
            zero_span,
        )))),
        Expr::Literal(Literal::Area(_, _, _)) => Some(Some(Expr::Literal(Literal::Area(
            0.0,
            DistanceUnit::Px,
            zero_span,
        )))),
        Expr::Literal(Literal::Duration(_, _, _)) => Some(Some(Expr::Literal(Literal::Duration(
            0,
            DurationUnit::S,
            zero_span,
        )))),
        Expr::Literal(_) => Some(None),
        Expr::MemberAccess(m) => {
            if let Expr::Ident(id) = m.target.as_ref() {
                if id.node_id().is_some_and(|n| n.is_template_self()) {
                    if let Some(ty) = schema.static_port_types.get(m.member.as_str()) {
                        return match ty.as_str() {
                            "Distance" => Some(Some(Expr::Literal(Literal::Distance(
                                0.0,
                                DistanceUnit::Px,
                                zero_span,
                            )))),
                            "Area" => Some(Some(Expr::Literal(Literal::Area(
                                0.0,
                                DistanceUnit::Px,
                                zero_span,
                            )))),
                            "Duration" => Some(Some(Expr::Literal(Literal::Duration(
                                0,
                                DurationUnit::S,
                                zero_span,
                            )))),
                            _ => Some(None),
                        };
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn try_compile_self_template(
    expr: &Expr,
    schema: &ComponentSchema,
    enums: &HashMap<String, EnumDef>,
) -> Option<Expr> {
    match expr {
        Expr::Literal(Literal::Distance(val, unit, span)) => match unit {
            DistanceUnit::Px => Some(expr.clone()),
            DistanceUnit::Pt => Some(Expr::Binary(BinaryExpr {
                op: BinaryOp::Mul,
                left: Box::new(Expr::Literal(Literal::Distance(
                    *val,
                    DistanceUnit::Px,
                    *span,
                ))),
                right: Box::new(Expr::Binary(BinaryExpr {
                    op: BinaryOp::Div,
                    left: Box::new(Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::from_node(
                            crate::compiler::expanded::NodeId::WINDOW,
                            *span,
                        ))),
                        member: Ident::from_symbol(Symbol::DPI, *span),
                        span: *span,
                    })),
                    right: Box::new(Expr::Literal(Literal::Float(72.0, *span))),
                    span: *span,
                })),
                span: *span,
            })),
            DistanceUnit::Em => None,
        },
        Expr::Literal(Literal::Area(_, DistanceUnit::Pt | DistanceUnit::Em, _)) => None,
        Expr::Literal(_) => Some(expr.clone()),
        Expr::Ident(id) => {
            if id.name == Symbol::TRUE {
                return Some(Expr::Literal(Literal::Bool(true, id.span)));
            }
            if id.name == Symbol::FALSE {
                return Some(Expr::Literal(Literal::Bool(false, id.span)));
            }
            if id.name == Symbol::SELF
                || id.name == Symbol::PARENT
                || id.name == Symbol::WINDOW
                || id.name == Symbol::PREV
                || id.name == Symbol::NEXT
                || id.name == Symbol::CHILD
                || id.name == Symbol::CHILDREN
                || id.name == Symbol::ENV
                || id.name == Symbol::UNSET
                || id.name == Symbol::NONE
            {
                return None;
            }
            if schema.template_self_ports.contains(&id.name) {
                return Some(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::from_node(
                        crate::compiler::expanded::NodeId::TEMPLATE_SELF,
                        id.span,
                    ))),
                    member: *id,
                    span: id.span,
                }));
            }
            None
        }
        Expr::MemberAccess(m) => {
            if let Expr::Ident(target_id) = m.target.as_ref() {
                if !target_id.is_canonical_node_ref() {
                    if let Some(enum_def) = enums.get(target_id.as_str()) {
                        let variant_name = m.member.as_str();
                        if enum_def.has_variant(variant_name) {
                            return Some(Expr::Literal(Literal::Enum(
                                target_id.as_str().to_string(),
                                variant_name.to_string(),
                                m.span,
                            )));
                        }
                        return None;
                    }
                }
                if target_id.name == Symbol::SELF && schema.template_self_ports.contains(&m.member.name) {
                    return Some(Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::from_node(
                            crate::compiler::expanded::NodeId::TEMPLATE_SELF,
                            target_id.span,
                        ))),
                        member: m.member,
                        span: m.span,
                    }));
                }
            }
            None
        }
        Expr::Binary(b) => {
            let left = try_compile_self_template(&b.left, schema, enums)?;
            let right = try_compile_self_template(&b.right, schema, enums)?;
            Some(Expr::Binary(BinaryExpr {
                op: b.op,
                left: Box::new(left),
                right: Box::new(right),
                span: b.span,
            }))
        }
        Expr::Unary(u) => {
            let operand = try_compile_self_template(&u.operand, schema, enums)?;
            Some(Expr::Unary(UnaryExpr {
                op: u.op,
                operand: Box::new(operand),
                span: u.span,
            }))
        }
        Expr::Paren(inner, span) => {
            let compiled = try_compile_self_template(inner, schema, enums)?;
            Some(Expr::Paren(Box::new(compiled), *span))
        }
        Expr::Ternary(t) => {
            let condition = try_compile_self_template(&t.condition, schema, enums)?;
            let mut then_expr = try_compile_self_template(&t.then_expr, schema, enums)?;
            let mut else_expr = try_compile_self_template(&t.else_expr, schema, enums)?;
            if let Expr::Literal(Literal::Integer(0, z_span)) = then_expr {
                let maybe_coerced =
                    infer_template_branch_zero_coercion(&else_expr, z_span, schema)?;
                if let Some(coerced) = maybe_coerced {
                    then_expr = coerced;
                }
            }
            if let Expr::Literal(Literal::Integer(0, z_span)) = else_expr {
                let maybe_coerced =
                    infer_template_branch_zero_coercion(&then_expr, z_span, schema)?;
                if let Some(coerced) = maybe_coerced {
                    else_expr = coerced;
                }
            }
            Some(Expr::Ternary(TernaryExpr {
                condition: Box::new(condition),
                then_expr: Box::new(then_expr),
                else_expr: Box::new(else_expr),
                span: t.span,
            }))
        }
        Expr::Call(c) => {
            if !matches!(
                c.callee.as_str(),
                "min" | "max" | "abs" | "floor" | "ceil" | "round" | "clamp" | "to_integer"
            ) {
                return None;
            }
            let mut args = Vec::with_capacity(c.args.len());
            for arg in &c.args {
                args.push(try_compile_self_template(arg, schema, enums)?);
            }
            Some(Expr::Call(CallExpr {
                callee: c.callee,
                args,
                span: c.span,
            }))
        }
        Expr::MethodCall(_) | Expr::Node(_) | Expr::ExprDef(_) | Expr::Match(_) => None,
    }
}

fn topo_sort_templates(templates: Vec<CompiledPortTemplate>) -> Vec<CompiledPortTemplate> {
    if templates.len() <= 1 {
        return templates;
    }
    let sym_to_idx: HashMap<Symbol, usize> = templates
        .iter()
        .enumerate()
        .map(|(i, t)| (t.sym, i))
        .collect();
    let mut visited = vec![0u8; templates.len()];
    let mut order = Vec::with_capacity(templates.len());
    let mut slots: Vec<Option<CompiledPortTemplate>> = templates.into_iter().map(Some).collect();

    fn dfs(
        idx: usize,
        slots: &[Option<CompiledPortTemplate>],
        sym_to_idx: &HashMap<Symbol, usize>,
        visited: &mut [u8],
        order: &mut Vec<usize>,
    ) {
        visited[idx] = 1;
        if let Some(ref tmpl) = slots[idx] {
            let mut deps = Vec::new();
            collect_template_self_deps(&tmpl.expr, &mut deps);
            for dep in deps {
                if let Some(&dep_idx) = sym_to_idx.get(&dep) {
                    if visited[dep_idx] == 0 {
                        dfs(dep_idx, slots, sym_to_idx, visited, order);
                    }
                }
            }
        }
        visited[idx] = 2;
        order.push(idx);
    }

    for i in 0..slots.len() {
        if visited[i] == 0 {
            dfs(i, &slots, &sym_to_idx, &mut visited, &mut order);
        }
    }

    order
        .into_iter()
        .filter_map(|i| slots[i].take())
        .collect()
}

impl CompiledComponentTemplates {
    pub fn build(
        schema: &ComponentSchema,
        comp_def: &ComponentDef,
        enums: &HashMap<String, EnumDef>,
        registry: &HashMap<String, Vec<ComponentDef>>,
    ) -> Self {
        let mut known_types: HashMap<Symbol, StaticType> =
            HashMap::with_capacity(schema.static_port_types.len() + schema.declared_lets.len());
        for (port_name, ty_name) in schema.static_port_types.iter() {
            if let Ok(st) =
                StaticType::from_type_name(ty_name, Span::default(), enums, registry)
            {
                known_types.insert(Symbol::new(port_name), st);
            }
        }

        let mut preverified_template_mask = PortMask::default();
        if !schema.comp_shadowed_ports.contains("clip")
            && !schema.param_names.contains("clip")
            && !schema.declared_envs.contains_key("clip")
        {
            preverified_template_mask.insert(&schema.port_table, "clip");
        }

        let make_port_template = |name: String, expr: Expr, is_state: bool| -> CompiledPortTemplate {
            let sym = Symbol::new(&name);
            let slot = schema.port_table.slot_of_sym(sym);
            let (has_self, has_ternary) = expr_has_template_self_or_ternary(&expr);
            let is_const_no_self = !has_self && !has_ternary;
            let const_val = if is_state || has_self {
                None
            } else {
                expr_to_const_value(&expr)
            };
            CompiledPortTemplate {
                name,
                sym,
                slot,
                expr,
                is_const_no_self,
                const_val,
                has_ternary,
            }
        };

        let mut param_defaults = HashMap::with_capacity(comp_def.params.len());
        for param in &comp_def.params {
            let Some(default_expr) = &param.default_edge else {
                continue;
            };
            let name = param.name.as_str();
            let is_handler_or_expr = name.starts_with("on_")
                || schema.handler_param_names.contains(name)
                || schema.expr_param_names.contains(name);
            if is_handler_or_expr || matches!(default_expr, Expr::Node(_) | Expr::ExprDef(_)) {
                continue;
            }
            if let Some(compiled) = try_compile_self_template(default_expr, schema, enums) {
                let coerced = super::typecheck::coerce_typed_binding_expr(
                    compiled,
                    param.type_annotation.as_ref(),
                );
                let (has_self, has_ternary) = expr_has_template_self_or_ternary(&coerced);
                if !has_self && !has_ternary && matches!(coerced, Expr::Literal(_) | Expr::Call(_)) {
                    let const_val = expr_to_const_value(&coerced);
                    param_defaults.insert(
                        param.name.name,
                        CompiledParamDefault::Constant {
                            expr: coerced,
                            const_val,
                        },
                    );
                } else {
                    let tmpl = make_port_template(name.to_string(), coerced, false);
                    param_defaults.insert(param.name.name, CompiledParamDefault::Template(tmpl));
                }
            }
        }

        let mut state_templates = Vec::with_capacity(schema.declared_states.len());
        for (state_name, s) in &schema.declared_states {
            let is_handler = state_name.starts_with("on_")
                || schema.handler_param_names.contains(state_name)
                || s.type_annotation
                    .as_ref()
                    .is_some_and(|t| t.name.as_str() == "Handler");
            if is_handler {
                continue;
            }
            let raw_default = super::rewrite::default_expr_for_state_with_enums(s, enums);
            if let Some(compiled) = try_compile_self_template(&raw_default, schema, enums) {
                let coerced =
                    super::typecheck::coerce_typed_binding_expr(compiled, s.type_annotation.as_ref());
                state_templates.push(make_port_template(state_name.clone(), coerced, true));
            }
        }
        state_templates = topo_sort_templates(state_templates);

        let mut let_templates = Vec::with_capacity(schema.declared_lets.len());
        for l in &schema.all_lets {
            let let_name = l.name.as_str();
            if schema.comp_shadowed_ports.contains(let_name) {
                continue;
            }
            let Some(LetValue::Expr(raw_expr)) = &l.value else {
                continue;
            };
            let is_handler_or_expr = let_name.starts_with("on_")
                || schema.handler_param_names.contains(let_name)
                || schema.expr_param_names.contains(let_name)
                || l.type_annotation
                    .as_ref()
                    .is_some_and(|t| matches!(t.name.as_str(), "Handler" | "Expr" | "Expression"))
                || matches!(raw_expr, Expr::ExprDef(_));
            if is_handler_or_expr {
                continue;
            }
            if let Some(compiled) = try_compile_self_template(raw_expr, schema, enums) {
                let coerced =
                    super::typecheck::coerce_typed_binding_expr(compiled, l.type_annotation.as_ref());
                let_templates.push(make_port_template(let_name.to_string(), coerced, false));
            }
        }
        let_templates = topo_sort_templates(let_templates);

        let mut env_templates = Vec::with_capacity(schema.declared_envs.len());
        for e in &schema.all_envs {
            let env_name = e.name.as_str();
            if schema.comp_shadowed_ports.contains(env_name) {
                continue;
            }
            let Some(raw_expr) = &e.value else {
                continue;
            };
            if matches!(
                raw_expr,
                Expr::Node(elem) if !super::is_value_node_name(elem.name.as_str())
            ) {
                continue;
            }
            let is_handler = env_name.starts_with("on_")
                || schema.handler_param_names.contains(env_name)
                || e.type_annotation
                    .as_ref()
                    .is_some_and(|t| t.name.as_str() == "Handler");
            if is_handler {
                continue;
            }
            if let Some(compiled) = try_compile_self_template(raw_expr, schema, enums) {
                let coerced =
                    super::typecheck::coerce_typed_binding_expr(compiled, e.type_annotation.as_ref());
                env_templates.push(make_port_template(env_name.to_string(), coerced, false));
            }
        }
        env_templates = topo_sort_templates(env_templates);

        let mut alias_templates = Vec::with_capacity(schema.declared_aliases.len() + 4);
        for (alias_name, a) in &schema.declared_aliases {
            let is_handler_or_expr = alias_name.starts_with("on_")
                || schema.handler_param_names.contains(alias_name)
                || schema.expr_param_names.contains(alias_name)
                || a.type_annotation
                    .as_ref()
                    .is_some_and(|t| matches!(t.name.as_str(), "Handler" | "Expr" | "Expression"))
                || matches!(&a.value, Expr::ExprDef(_));
            if is_handler_or_expr {
                continue;
            }
            if let Some(compiled) = try_compile_self_template(&a.value, schema, enums) {
                let coerced =
                    super::typecheck::coerce_typed_binding_expr(compiled, a.type_annotation.as_ref());
                alias_templates.push(make_port_template(alias_name.clone(), coerced, false));
            }
        }
        for sp in ["left", "top", "right", "bottom"] {
            if !schema.declared_aliases.contains_key(sp)
                && !schema.param_names.contains(sp)
                && !schema.declared_states.contains_key(sp)
                && !schema.declared_lets.contains_key(sp)
                && !schema.declared_envs.contains_key(sp)
            {
                if let Some(authored_sp) = schema.base_authored_ports.get(sp) {
                    if let Some(compiled) = try_compile_self_template(authored_sp, schema, enums) {
                        alias_templates.push(make_port_template(sp.to_string(), compiled, false));
                    }
                }
            }
        }
        alias_templates = topo_sort_templates(alias_templates);

        // Pre-verify static types of all compiled templates whose dependencies have known static types.
        for param in &comp_def.params {
            let Some(def) = param_defaults.get(&param.name.name) else {
                continue;
            };
            let expr = match def {
                CompiledParamDefault::Constant { expr, .. } => expr,
                CompiledParamDefault::Template(t) => &t.expr,
            };
            if let Ok(actual) =
                super::typecheck::infer_template_expr_type(expr, &known_types, enums, registry)
            {
                if let Some(expected) = known_types.get(&param.name.name) {
                    if actual.matches(expected) {
                        preverified_template_mask.insert(&schema.port_table, param.name.as_str());
                    }
                }
            }
        }

        for tmpl in state_templates
            .iter()
            .chain(let_templates.iter())
            .chain(env_templates.iter())
            .chain(alias_templates.iter())
        {
            if let Ok(actual) = super::typecheck::infer_template_expr_type(
                &tmpl.expr,
                &known_types,
                enums,
                registry,
            ) {
                if let Some(expected) = known_types.get(&tmpl.sym) {
                    if actual.matches(expected) {
                        preverified_template_mask.insert(&schema.port_table, &tmpl.name);
                    }
                } else {
                    let decayed = actual.decay();
                    known_types.insert(tmpl.sym, decayed);
                    preverified_template_mask.insert(&schema.port_table, &tmpl.name);
                }
            }
        }

        Self {
            param_defaults,
            state_templates,
            let_templates,
            env_templates,
            alias_templates,
            preverified_port_types: std::sync::Arc::new(known_types),
            preverified_template_mask,
        }
    }
}
