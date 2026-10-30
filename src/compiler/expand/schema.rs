use super::context::{EnvEntry, LexicalBinding};
use super::typecheck::{validate_content_items_type_annotations, StaticType};
use crate::ast::*;
use crate::compiler::error::{CompileError, NoMatchingOverloadDetails};
use crate::span::Span;
use std::collections::{HashMap, HashSet};

/// Pre-validated schema of a `ComponentDef`'s ports, aliases, state, let bindings, and env bindings.
#[derive(Debug, Clone)]
pub struct ComponentSchema {
    pub is_span_def: bool,
    pub is_inline_box_def: bool,
    pub comp_scope_ports: Vec<String>,
    pub declared_aliases: HashMap<String, AliasBinding>,
    pub declared_states: HashMap<String, StateBinding>,
    pub declared_state_names: HashSet<String>,
    pub all_lets: Vec<LetBinding>,
    pub declared_lets: HashMap<String, LetBinding>,
    pub all_let_names: HashSet<String>,
    pub declared_envs: HashMap<String, EnvBinding>,
    pub comp_shadowed_ports: HashSet<String>,
}

impl ComponentSchema {
    /// Validates and builds the port/binding schema for `comp_def`.
    pub fn from_def(comp_def: &ComponentDef) -> Result<Self, CompileError> {
        let is_span_def = comp_def.kind == ComponentKind::Span;
        let is_inline_box_def = comp_def.kind == ComponentKind::InlineBox;

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
            if comp_def.params.iter().any(|p| p.name.as_str() == alias_name) {
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
                if comp_def.params.iter().any(|p| p.name.as_str() == state_name) {
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

        let mut declared_envs: HashMap<String, EnvBinding> = HashMap::new();
        for item in &comp_def.body {
            if let ComponentBodyItem::Env(e) = item {
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
                declared_envs.insert(env_name, e.clone());
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

        Ok(Self {
            is_span_def,
            is_inline_box_def,
            comp_scope_ports,
            declared_aliases,
            declared_states,
            declared_state_names,
            all_lets,
            declared_lets,
            all_let_names,
            declared_envs,
            comp_shadowed_ports,
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
            if self.declared_envs.contains_key(name)
                && !comp_def.params.iter().any(|p| p.name.as_str() == name)
            {
                return Err(CompileError::Custom {
                    message: format!(
                        "Node '{}' environmental property '{}' cannot be set as a parameter",
                        comp_def.name.as_str(),
                        name
                    ),
                    span: port.name.span,
                });
            }
            if self.all_let_names.contains(name)
                && !comp_def.params.iter().any(|p| p.name.as_str() == name)
            {
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
            let all = o.param_names();
            // 1. All provided ports (caller-authored or container-injected via \Children)
            // must be accepted by this overload.
            for port in &elem.ports {
                let name = port.name.as_str();
                if name != "clip" && !all.contains(name) && !name.starts_with("on_") {
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
                        || (param.is_env && matches!(env_scope.get(name), Some(EnvEntry::Bound(_))))
                        || (param.is_env && name == "text_bounds");

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
    lexical_scope: &HashMap<String, LexicalBinding>,
) -> Result<(), CompileError> {
    for part in &key.parts {
        validate_key_expr(node_name, part, lexical_scope)?;
    }
    Ok(())
}

fn validate_key_expr(
    node_name: &str,
    expr: &Expr,
    lexical_scope: &HashMap<String, LexicalBinding>,
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
                lexical_scope.get(name),
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
                lexical_scope.get(member_name),
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
