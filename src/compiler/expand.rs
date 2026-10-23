use crate::ast::*;
use crate::compiler::error::{CompileError, NoMatchingOverloadDetails};
use crate::compiler::eval::eval_expr;
use crate::compiler::expanded::{ExpandedDocument, ExpandedNode, NodeId};
use crate::compiler::scope::ScopeId;
use crate::compiler::text::{CursorKind, SpanStyle, TextSpan};
use crate::span::Span;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// Static compile-time type of a property, port, or expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaticType {
    Integer,
    ZeroInteger,
    Float,
    Distance,
    Area,
    String,
    Boolean,
    Color,
    Handler,
    Unset,
    ViewPort,
    Enum(String),
    Node(String),
}

impl StaticType {
    pub fn canonical_name(&self) -> String {
        match self {
            StaticType::Integer | StaticType::ZeroInteger => "Integer".to_string(),
            StaticType::Float => "Float".to_string(),
            StaticType::Distance => "Distance".to_string(),
            StaticType::Area => "Area".to_string(),
            StaticType::String => "String".to_string(),
            StaticType::Boolean => "Boolean".to_string(),
            StaticType::Color => "Color".to_string(),
            StaticType::Handler | StaticType::Unset => "Handler".to_string(),
            StaticType::ViewPort => "ViewPort".to_string(),
            StaticType::Enum(name) => name.clone(),
            StaticType::Node(name) => name.clone(),
        }
    }

    pub fn type_name(&self) -> String {
        self.canonical_name()
    }

    pub fn decay(&self) -> StaticType {
        self.clone().into_property_type()
    }

    pub fn into_property_type(self) -> StaticType {
        match self {
            StaticType::ZeroInteger => StaticType::Integer,
            StaticType::Unset => StaticType::Handler,
            other => other,
        }
    }

    pub fn from_type_name(
        name: &str,
        span: Span,
        enums: &HashMap<String, EnumDef>,
        registry: &HashMap<String, Vec<ComponentDef>>,
    ) -> Result<Self, CompileError> {
        match name {
            "Integer" => Ok(StaticType::Integer),
            "Float" => Ok(StaticType::Float),
            "Distance" => Ok(StaticType::Distance),
            "Area" => Ok(StaticType::Area),
            "String" => Ok(StaticType::String),
            "Boolean" => Ok(StaticType::Boolean),
            "Color" => Ok(StaticType::Color),
            "Handler" => Ok(StaticType::Handler),
            "ViewPort" => Ok(StaticType::ViewPort),
            "Node" => Ok(StaticType::Node("Node".to_string())),
            "Window" | "Rect" | "Line" | "Text" | "Font" | "Clip" | "Box" | "Anchor" | "AnchorScope" => {
                Ok(StaticType::Node(name.to_string()))
            }
            "Number" => Err(CompileError::Custom {
                message: "Type 'Number' has been removed; use 'Float', 'Integer', or 'Distance' instead"
                    .to_string(),
                span,
            }),
            _ => {
                if enums.contains_key(name) {
                    Ok(StaticType::Enum(name.to_string()))
                } else if registry.contains_key(name) {
                    Ok(StaticType::Node(name.to_string()))
                } else {
                    Err(CompileError::Custom {
                        message: format!("Unknown type '{}'", name),
                        span,
                    })
                }
            }
        }
    }

    pub fn matches(&self, expected: &StaticType) -> bool {
        if self == expected {
            return true;
        }
        match (self, expected) {
            (
                StaticType::ZeroInteger,
                StaticType::Integer | StaticType::Float | StaticType::Distance | StaticType::Area,
            ) => true,
            (StaticType::Integer, StaticType::ZeroInteger) => true,
            (StaticType::Integer, StaticType::Float) => true,
            (StaticType::Unset, StaticType::Handler) => true,
            (StaticType::Handler, StaticType::Unset) => true,
            (StaticType::Node(n), StaticType::ViewPort)
                if n == "Window" || n == "ScrollView" || n == "Node" =>
            {
                true
            }
            (StaticType::Node(n), StaticType::Node(exp))
                if exp == "Clip" && (n == "Window" || n == "Node") =>
            {
                true
            }
            (StaticType::Node(_), StaticType::Node(exp)) if exp == "Node" => true,
            (StaticType::ViewPort, StaticType::Node(exp))
                if exp == "Node" || exp == "Window" || exp == "ScrollView" =>
            {
                true
            }
            (StaticType::Node(n), StaticType::Node(_)) if n == "Node" => true,
            _ => false,
        }
    }
}

fn validate_content_items_type_annotations(
    items: &[ContentItem],
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> Result<(), CompileError> {
    for item in items {
        match item {
            ContentItem::Let(l) => {
                if let Some(ty) = &l.type_annotation {
                    StaticType::from_type_name(ty.name.as_str(), ty.span, enums, registry)?;
                }
                if let Some(LetValue::Node(n)) = &l.value {
                    if let Some(slot) = &n.content {
                        validate_content_items_type_annotations(&slot.items, enums, registry)?;
                    }
                }
            }
            ContentItem::Alias(a) => {
                if let Some(ty) = &a.type_annotation {
                    StaticType::from_type_name(ty.name.as_str(), ty.span, enums, registry)?;
                }
            }
            ContentItem::State(s) => {
                if let Some(ty) = &s.type_annotation {
                    StaticType::from_type_name(ty.name.as_str(), ty.span, enums, registry)?;
                }
            }
            ContentItem::Env(e) => {
                if let Some(ty) = &e.type_annotation {
                    StaticType::from_type_name(ty.name.as_str(), ty.span, enums, registry)?;
                }
            }
            ContentItem::Node(n) => {
                if let Some(slot) = &n.content {
                    validate_content_items_type_annotations(&slot.items, enums, registry)?;
                }
            }
            ContentItem::Children(_) | ContentItem::Text(_) | ContentItem::Assert(_) => {}
        }
    }
    Ok(())
}

fn coerce_typed_binding_expr(mut expr: Expr, type_annotation: Option<&TypeRef>) -> Expr {
    if let Some(ty) = type_annotation {
        match ty.name.as_str() {
            "Distance" => {
                if let Expr::Literal(Literal::Integer(0, span)) = expr {
                    expr = Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span));
                }
            }
            "Area" => {
                if let Expr::Literal(Literal::Integer(0, span)) = expr {
                    expr = Expr::Literal(Literal::Area(0.0, DistanceUnit::Px, span));
                }
            }
            "Float" => {
                if let Expr::Literal(Literal::Integer(i, span)) = expr {
                    expr = Expr::Literal(Literal::Float(i as f64, span));
                }
            }
            "Integer" => {
                if let Expr::Literal(Literal::Integer(0, span)) = expr {
                    expr = Expr::Call(CallExpr {
                        callee: Ident::new("to_integer", span),
                        args: vec![Expr::Literal(Literal::Integer(0, span))],
                        span,
                    });
                }
            }
            _ => {}
        }
    } else if let Expr::Literal(Literal::Integer(0, span)) = expr {
        expr = Expr::Call(CallExpr {
            callee: Ident::new("to_integer", span),
            args: vec![Expr::Literal(Literal::Integer(0, span))],
            span,
        });
    }
    expr
}

fn validate_node_type_annotation(
    elem_name: &str,
    elem_span: Span,
    type_annotation: Option<&TypeRef>,
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> Result<(), CompileError> {
    if let Some(ty_ref) = type_annotation {
        let expected =
            StaticType::from_type_name(ty_ref.name.as_str(), ty_ref.span, enums, registry)?;
        let actual = StaticType::Node(elem_name.to_string());
        if !actual.matches(&expected) {
            return Err(CompileError::TypeMismatch {
                expected: expected.canonical_name(),
                actual: actual.canonical_name(),
                span: elem_span,
            });
        }
    }
    Ok(())
}

/// A lexical binding in the component or document scope.
#[derive(Debug, Clone)]
pub enum LexicalBinding {
    Expr(Expr),
    State(Expr),
    Node(NodeId),
    Uninitialized,
}

/// An environmental entry in the active Lexical Scope Stack.
#[derive(Debug, Clone, PartialEq)]
pub enum EnvEntry {
    Bound(Expr),
    Tombstone,
}

/// Context used when rewriting expressions to bind variables to concrete node IDs.
#[derive(Debug, Clone)]
pub struct ScopeContext<'a> {
    pub current_node: NodeId,
    pub parent_node: Option<NodeId>,
    pub parent_name: Option<&'a str>,
    pub prev_sibling: Option<NodeId>,
    pub has_next_sibling: bool,
    pub child_ids: &'a [NodeId],
    pub num_children: usize,
    pub child_node: Option<NodeId>,
    pub nodes: &'a [ExpandedNode],
    pub shadowed_ports: Option<&'a HashSet<String>>,
    pub is_event_port: bool,
    pub parent_ports: &'a [String],
    pub current_ports: &'a [String],
    pub lexical_scope: &'a HashMap<String, LexicalBinding>,
    pub env_scope: &'a HashMap<String, EnvEntry>,
    pub node_fonts: &'a HashMap<NodeId, NodeId>,
    pub enclosing_component: Option<NodeId>,
    pub comp_ports: Option<&'a HashMap<String, Expr>>,
    pub declared_state_names: Option<&'a HashSet<String>>,
    pub enums: &'a HashMap<String, EnumDef>,
}

/// Expands a parsed AST `Document` into an `ExpandedDocument` using default current directory.
pub fn expand_document(doc: &Document) -> Result<ExpandedDocument, CompileError> {
    expand_document_with_resolver(doc, std::path::Path::new("."), &crate::compiler::module::FsResolver)
}

/// Expands a parsed AST `Document` into an `ExpandedDocument` with a specific base directory.
pub fn expand_document_with_base_dir(
    doc: &Document,
    base_dir: &std::path::Path,
) -> Result<ExpandedDocument, CompileError> {
    expand_document_with_resolver(doc, base_dir, &crate::compiler::module::FsResolver)
}

/// Expands a parsed AST `Document` into an `ExpandedDocument` with a custom file resolver.
pub fn expand_document_with_resolver<R: crate::compiler::module::FileResolver>(
    doc: &Document,
    base_dir: &std::path::Path,
    resolver: &R,
) -> Result<ExpandedDocument, CompileError> {
    let (mut registry, imported_items, all_module_enums) =
        crate::compiler::module::resolve_imports_with_enums(doc, base_dir, resolver)?;
    let mut merged_items = imported_items;
    merged_items.extend(doc.items.clone());
    let merged_doc = Document {
        items: merged_items,
        span: doc.span,
    };
    let doc = &merged_doc;

    let mut enums: HashMap<String, EnumDef> = HashMap::new();
    for item in &doc.items {
        if let Item::Enum(e) = item {
            let name = e.name.as_str().to_string();
            if enums.contains_key(&name) {
                return Err(CompileError::DuplicateEnum {
                    name,
                    span: e.name.span,
                });
            }
            enums.insert(name, e.clone());
        }
    }

    // Built-in standard enums
    enums.entry("TextEnd".to_string()).or_insert_with(|| EnumDef {
        name: Ident::new("TextEnd", doc.span),
        variants: vec![
            Ident::new("Baseline", doc.span),
            Ident::new("Descender", doc.span),
            Ident::new("GapBottom", doc.span),
        ],
        span: doc.span,
    });
    enums.entry("TextStart".to_string()).or_insert_with(|| EnumDef {
        name: Ident::new("TextStart", doc.span),
        variants: vec![
            Ident::new("Capital", doc.span),
            Ident::new("Ascender", doc.span),
            Ident::new("EmTop", doc.span),
        ],
        span: doc.span,
    });
    enums.entry("LineCap".to_string()).or_insert_with(|| EnumDef {
        name: Ident::new("LineCap", doc.span),
        variants: vec![
            Ident::new("Butt", doc.span),
            Ident::new("Round", doc.span),
            Ident::new("Square", doc.span),
        ],
        span: doc.span,
    });
    registry.entry("TextBounds".to_string()).or_insert_with(|| {
        vec![ComponentDef {
            name: Ident::new("TextBounds", doc.span),
            params: vec![
                ParamDef {
                    name: Ident::new("start_at", doc.span),
                    type_annotation: Some(TypeRef {
                        name: Ident::new("TextStart", doc.span),
                        span: doc.span,
                    }),
                    default_edge: Some(Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new("TextStart", doc.span))),
                        member: Ident::new("Capital", doc.span),
                        span: doc.span,
                    })),
                    is_env: false,
                    span: doc.span,
                },
                ParamDef {
                    name: Ident::new("end_at", doc.span),
                    type_annotation: Some(TypeRef {
                        name: Ident::new("TextEnd", doc.span),
                        span: doc.span,
                    }),
                    default_edge: Some(Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new("TextEnd", doc.span))),
                        member: Ident::new("Baseline", doc.span),
                        span: doc.span,
                    })),
                    is_env: false,
                    span: doc.span,
                },
            ],
            body: Vec::new(),
            span: doc.span,
        }]
    });

    let mut known_enums = all_module_enums;
    for (k, v) in &enums {
        known_enums.insert(k.clone(), v.clone());
    }

    // Validate all explicit type annotations across all registered components and top-level items
    for overloads in registry.values() {
        for comp_def in overloads {
            for param in &comp_def.params {
                if let Some(ty) = &param.type_annotation {
                    StaticType::from_type_name(ty.name.as_str(), ty.span, &known_enums, &registry)?;
                }
            }
            validate_content_items_type_annotations(&comp_def.body, &known_enums, &registry)?;
        }
    }
    for item in &doc.items {
        match item {
            Item::Let(l) => {
                if let Some(ty) = &l.type_annotation {
                    StaticType::from_type_name(ty.name.as_str(), ty.span, &enums, &registry)?;
                }
                if let Some(LetValue::Node(n)) = &l.value {
                    if let Some(slot) = &n.content {
                        validate_content_items_type_annotations(&slot.items, &enums, &registry)?;
                    }
                }
            }
            Item::State(s) => {
                if let Some(ty) = &s.type_annotation {
                    StaticType::from_type_name(ty.name.as_str(), ty.span, &enums, &registry)?;
                }
            }
            Item::Env(e) => {
                if let Some(ty) = &e.type_annotation {
                    StaticType::from_type_name(ty.name.as_str(), ty.span, &enums, &registry)?;
                }
            }
            Item::Node(n) => {
                if let Some(slot) = &n.content {
                    validate_content_items_type_annotations(&slot.items, &enums, &registry)?;
                }
            }
            _ => {}
        }
    }

    let mut global_scope = HashMap::new();
    let mut global_env_scope: HashMap<String, EnvEntry> = HashMap::new();
    let mut node_fonts: HashMap<NodeId, NodeId> = HashMap::new();

    // Default universal clip: window.clip
    global_env_scope.insert(
        "clip".to_string(),
        EnvEntry::Bound(Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(Expr::Ident(Ident::new(NodeId::WINDOW.canonical_name(), doc.span))),
            member: Ident::new("clip", doc.span),
            span: doc.span,
        })),
    );

    let mut expanded_doc = ExpandedDocument::new();
    for p in ["x", "y", "width", "height", "left", "top", "right", "bottom"] {
        expanded_doc.window_port_types.insert(p.to_string(), "Distance".to_string());
    }
    expanded_doc.window_port_types.insert("z".to_string(), "Integer".to_string());
    expanded_doc.window_port_types.insert("clip".to_string(), "Clip".to_string());
    expanded_doc.window_port_types.insert("dpi".to_string(), "Float".to_string());
    expanded_doc.window_port_types.insert("title".to_string(), "String".to_string());
    expanded_doc.window_port_types.insert("vscroll".to_string(), "Boolean".to_string());
    expanded_doc.window_port_types.insert("hscroll".to_string(), "Boolean".to_string());

    let mut window_scope_ports = vec![
        "x".to_string(),
        "y".to_string(),
        "width".to_string(),
        "height".to_string(),
        "z".to_string(),
        "clip".to_string(),
        "left".to_string(),
        "top".to_string(),
        "right".to_string(),
        "bottom".to_string(),
        "dpi".to_string(),
    ];

    // Pre-register top-level Window declaration
    let mut window_decl: Option<&ElementNode> = None;
    for item in &doc.items {
        if let Item::Node(elem) = item {
            if elem.name.as_str() == "Window" {
                if let Some(first) = window_decl {
                    return Err(CompileError::DuplicateWindowDeclaration {
                        span: first.span,
                        second_span: elem.span,
                    });
                }
                if let Some(content) = &elem.content {
                    return Err(CompileError::WindowContentNotAllowed {
                        span: content.span,
                    });
                }
                if let Some(key) = &elem.key {
                    return Err(CompileError::Custom {
                        message: "\\Window declaration does not accept a key".to_string(),
                        span: key.span,
                    });
                }
                window_decl = Some(elem);
            }
        }
    }

    const ALLOWED_WINDOW_PORTS: &[&str] = &["title", "vscroll", "hscroll", "dpi"];

    if let Some(window_elem) = window_decl {
        let mut seen_window_ports = HashSet::new();
        for port in &window_elem.ports {
            let name = port.name.as_str().to_string();
            if !ALLOWED_WINDOW_PORTS.contains(&name.as_str()) {
                return Err(CompileError::DisallowedWindowPort {
                    port: name,
                    span: port.name.span,
                });
            }
            if !seen_window_ports.insert(name.clone()) {
                return Err(CompileError::DuplicatePort {
                    node: "Window".to_string(),
                    port: name,
                    span: port.name.span,
                });
            }
            if !window_scope_ports.contains(&name) {
                window_scope_ports.push(name.clone());
            }
            expanded_doc.window_ports.insert(name, port.expr.clone());
        }
    }

    // Pre-register top-level state items
    let mut declared_top_states = HashMap::new();
    for item in &doc.items {
        if let Item::State(s) = item {
            let name = s.name.as_str().to_string();
            if name == "parent" || name == "window" || name == "self" {
                return Err(CompileError::ReservedPort {
                    node: "Document".to_string(),
                    port: name,
                    span: s.name.span,
                });
            }
            if declared_top_states.contains_key(&name) || expanded_doc.window_ports.contains_key(&name) {
                return Err(CompileError::DuplicatePort {
                    node: "Document".to_string(),
                    port: name,
                    span: s.name.span,
                });
            }
            declared_top_states.insert(name.clone(), s.clone());
            if !window_scope_ports.contains(&name) {
                window_scope_ports.push(name.clone());
            }
            let default_expr = default_expr_for_state_with_enums(s, &enums);
            let window_ref = Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(Expr::Ident(Ident::new(NodeId::WINDOW.canonical_name(), s.span))),
                member: s.name.clone(),
                span: s.span,
            });
            global_scope.insert(name.clone(), LexicalBinding::State(window_ref));
            let type_name = s.type_annotation.as_ref().map(|t| t.name.as_str().to_string());
            if let Some(ref tn) = type_name {
                expanded_doc.window_port_types.insert(name.clone(), tn.clone());
            }
            expanded_doc.window_state_vars.insert(name.clone(), type_name);
            expanded_doc.window_ports.insert(name, default_expr);
        }
    }

    let all_doc_lets = collect_all_document_lets(doc);
    let mut all_doc_let_names: HashSet<String> = HashSet::new();
    for l in &all_doc_lets {
        let let_name = l.name.as_str().to_string();
        if let_name == "parent" || let_name == "window" || let_name == "self" {
            return Err(CompileError::ReservedPort {
                node: "Document".to_string(),
                port: let_name,
                span: l.name.span,
            });
        }
        if declared_top_states.contains_key(&let_name)
            || expanded_doc.window_ports.contains_key(&let_name)
            || all_doc_let_names.contains(&let_name)
        {
            return Err(CompileError::DuplicatePort {
                node: "Document".to_string(),
                port: let_name,
                span: l.name.span,
            });
        }
        all_doc_let_names.insert(let_name);
    }

    // Validate component declarations (aliases, states, lets, envs) across all registered components
    for overloads in registry.values() {
        for comp_def in overloads {
            let mut comp_aliases = HashSet::new();
            for a in collect_all_component_aliases(comp_def) {
                let alias_name = a.name.as_str().to_string();
                if alias_name == "parent" {
                    return Err(CompileError::ReservedPort {
                        node: comp_def.name.as_str().to_string(),
                        port: "parent".to_string(),
                        span: a.name.span,
                    });
                }
                if comp_def.params.iter().any(|p| p.name.as_str() == alias_name)
                    || !comp_aliases.insert(alias_name.clone())
                {
                    return Err(CompileError::DuplicatePort {
                        node: comp_def.name.as_str().to_string(),
                        port: alias_name,
                        span: a.name.span,
                    });
                }
            }
            let mut comp_states = HashSet::new();
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
                    if comp_def.params.iter().any(|p| p.name.as_str() == state_name)
                        || comp_aliases.contains(&state_name)
                        || !comp_states.insert(state_name.clone())
                    {
                        return Err(CompileError::DuplicatePort {
                            node: comp_def.name.as_str().to_string(),
                            port: state_name,
                            span: s.name.span,
                        });
                    }
                }
            }
            let mut comp_lets = HashSet::new();
            for l in collect_all_component_lets(comp_def) {
                let let_name = l.name.as_str().to_string();
                if let_name == "parent" {
                    return Err(CompileError::ReservedPort {
                        node: comp_def.name.as_str().to_string(),
                        port: "parent".to_string(),
                        span: l.name.span,
                    });
                }
                if comp_aliases.contains(&let_name)
                    || comp_states.contains(&let_name)
                    || !comp_lets.insert(let_name.clone())
                {
                    return Err(CompileError::DuplicatePort {
                        node: comp_def.name.as_str().to_string(),
                        port: let_name,
                        span: l.name.span,
                    });
                }
            }
            let mut comp_envs = HashSet::new();
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
                    if comp_aliases.contains(&env_name)
                        || comp_states.contains(&env_name)
                        || comp_lets.contains(&env_name)
                        || !comp_envs.insert(env_name.clone())
                    {
                        return Err(CompileError::DuplicatePort {
                            node: comp_def.name.as_str().to_string(),
                            port: env_name,
                            span: e.name.span,
                        });
                    }
                }
            }
        }
    }

    // Pre-register uninitialized let and env items
    for l in &all_doc_lets {
        if l.value.is_none() {
            global_scope.insert(l.name.as_str().to_string(), LexicalBinding::Uninitialized);
            global_env_scope.insert(l.name.as_str().to_string(), EnvEntry::Tombstone);
        }
    }
    for item in &doc.items {
        if let Item::Env(e) = item {
            if e.value.is_none() {
                global_scope.insert(e.name.as_str().to_string(), LexicalBinding::Uninitialized);
                global_env_scope.insert(e.name.as_str().to_string(), EnvEntry::Tombstone);
            }
        }
    }

    // 2. Pre-resolve top-level let and env expressions
    let empty_ports: [String; 0] = [];
    let empty_children: [NodeId; 0] = [];
    let mut changed = true;
    let mut passes = 0;
    while changed && passes < 20 {
        changed = false;
        passes += 1;
        for item in &doc.items {
            match item {
                Item::Let(let_binding) => {
                    if let_binding.value.is_none() {
                        global_scope.insert(let_binding.name.as_str().to_string(), LexicalBinding::Uninitialized);
                    } else if let Some(LetValue::Expr(raw_expr)) = &let_binding.value {
                        let scope_ctx = ScopeContext {
                            current_node: NodeId::WINDOW,
                            parent_node: None,
                            parent_name: None,
                            prev_sibling: None,
                            has_next_sibling: false,
                            child_ids: &empty_children,
                            num_children: 0,
                            child_node: None,
                            nodes: &expanded_doc.nodes,
                            shadowed_ports: None,
                            is_event_port: false,
                            parent_ports: &empty_ports,
                            current_ports: &empty_ports,
                            lexical_scope: &global_scope,
                            env_scope: &global_env_scope,
                            node_fonts: &node_fonts,
                            enclosing_component: None,
                            comp_ports: None,
                            declared_state_names: None,
                            enums: &enums,
                        };
                        if let Ok(rewritten) = rewrite_expr(raw_expr, &scope_ctx) {
                            let rewritten = coerce_typed_binding_expr(
                                rewritten,
                                let_binding.type_annotation.as_ref(),
                            );
                            if let Some(LexicalBinding::Expr(existing)) = global_scope.get(let_binding.name.as_str()) {
                                if existing != &rewritten {
                                    global_scope.insert(let_binding.name.as_str().to_string(), LexicalBinding::Expr(rewritten));
                                    changed = true;
                                }
                            } else {
                                global_scope.insert(let_binding.name.as_str().to_string(), LexicalBinding::Expr(rewritten));
                                changed = true;
                            }
                        }
                    }
                }
                Item::Env(env_binding) => {
                    if env_binding.value.is_none() {
                        global_scope.insert(env_binding.name.as_str().to_string(), LexicalBinding::Uninitialized);
                        global_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Tombstone);
                    } else if let Some(raw_expr) = &env_binding.value {
                        if !matches!(raw_expr, Expr::Node(_)) {
                            let scope_ctx = ScopeContext {
                                current_node: NodeId::WINDOW,
                                parent_node: None,
                                parent_name: None,
                                prev_sibling: None,
                                has_next_sibling: false,
                                child_ids: &empty_children,
                                num_children: 0,
                                child_node: None,
                                nodes: &expanded_doc.nodes,
                                shadowed_ports: None,
                                is_event_port: false,
                                parent_ports: &empty_ports,
                                current_ports: &empty_ports,
                                lexical_scope: &global_scope,
                                env_scope: &global_env_scope,
                                node_fonts: &node_fonts,
                                enclosing_component: None,
                                comp_ports: None,
                                declared_state_names: None,
                                enums: &enums,
                            };
                            if let Ok(rewritten) = rewrite_expr(raw_expr, &scope_ctx) {
                                let rewritten = coerce_typed_binding_expr(
                                    rewritten,
                                    env_binding.type_annotation.as_ref(),
                                );
                                if let Some(LexicalBinding::Expr(existing)) = global_scope.get(env_binding.name.as_str()) {
                                    if existing != &rewritten {
                                        global_scope.insert(env_binding.name.as_str().to_string(), LexicalBinding::Expr(rewritten.clone()));
                                        global_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Bound(rewritten));
                                        changed = true;
                                    }
                                } else {
                                    global_scope.insert(env_binding.name.as_str().to_string(), LexicalBinding::Expr(rewritten.clone()));
                                    global_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Bound(rewritten));
                                    changed = true;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let window_scope_ctx = ScopeContext {
        current_node: NodeId::WINDOW,
        parent_node: None,
        parent_name: None,
        prev_sibling: None,
        has_next_sibling: false,
        child_ids: &empty_children,
        num_children: 0,
        child_node: None,
        nodes: &expanded_doc.nodes,
        shadowed_ports: None,
        is_event_port: false,
        parent_ports: &empty_ports,
        current_ports: &empty_ports,
        lexical_scope: &global_scope,
        env_scope: &global_env_scope,
        node_fonts: &node_fonts,
        enclosing_component: None,
        comp_ports: None,
        declared_state_names: None,
        enums: &enums,
    };
    for (name, expr) in expanded_doc.window_ports.clone() {
        if let Ok(rewritten) = rewrite_expr(&expr, &window_scope_ctx) {
            expanded_doc.window_ports.insert(name, rewritten);
        }
    }

    // 3. Expand root elements and top-level node bindings in document order
    let mut last_root_id = None;
    let global_scope = RefCell::new(global_scope);

    for item in &doc.items {
        match item {
            Item::Component(_) => {}
            Item::Env(env_binding) => {
                if let Some(Expr::Node(elem)) = &env_binding.value {
                    validate_node_type_annotation(
                        elem.name.as_str(),
                        elem.span,
                        env_binding.type_annotation.as_ref(),
                        &enums,
                        &registry,
                    )?;
                    let elem_ctx = ElementContext {
                        parent_id: Some(NodeId::WINDOW),
                        parent_name: Some("window"),
                        prev_sibling_id: last_root_id,
                        has_next_sibling: false,
                        parent_ports: &window_scope_ports,
                        lexical_scope: &global_scope,
                        env_scope: &global_env_scope,
                        enclosing_component_id: None,
                        enclosing_num_children: 0,
                        enclosing_instantiated_children: None,
                        shadowed_ports: None,
                        is_let: true,
                        ambient_authored_ports: None,
                        enums: &enums,
                        active_scope_id: ScopeId::ROOT,
                        consumer_children: None,
                    };
                    let root_id = expand_element(elem, &elem_ctx, &registry, &mut expanded_doc, &mut node_fonts)?;
                    expanded_doc.get_node_mut(root_id).unwrap().var_name = Some(env_binding.name.as_str().to_string());
                    expanded_doc.roots.push(root_id);
                    last_root_id = Some(root_id);
                    global_scope.borrow_mut().insert(
                        env_binding.name.as_str().to_string(),
                        LexicalBinding::Node(root_id),
                    );
                    let bound_expr = Expr::Ident(Ident::new(root_id.canonical_name(), elem.span));
                    global_env_scope.insert(
                        env_binding.name.as_str().to_string(),
                        EnvEntry::Bound(bound_expr),
                    );
                    if elem.name.as_str() == "Font" {
                        node_fonts.insert(root_id, root_id);
                    }
                } else if let Some(raw_expr) = &env_binding.value {
                    let rewritten = {
                        let scope_borrow = global_scope.borrow();
                        let scope_ctx = ScopeContext {
                            current_node: NodeId::WINDOW,
                            parent_node: None,
                            parent_name: None,
                            prev_sibling: None,
                            has_next_sibling: false,
                            child_ids: &empty_children,
                            num_children: 0,
                            child_node: None,
                            nodes: &expanded_doc.nodes,
                            shadowed_ports: None,
                            is_event_port: false,
                            parent_ports: &empty_ports,
                            current_ports: &empty_ports,
                            lexical_scope: &*scope_borrow,
                            env_scope: &global_env_scope,
                            node_fonts: &node_fonts,
                            enclosing_component: None,
                            comp_ports: None,
                            declared_state_names: None,
                            enums: &enums,
                        };
                        rewrite_expr(raw_expr, &scope_ctx)?
                    };
                    let rewritten = coerce_typed_binding_expr(
                        rewritten,
                        env_binding.type_annotation.as_ref(),
                    );
                    expanded_doc.local_bindings.push((
                        rewritten.clone(),
                        env_binding.type_annotation.as_ref().map(|t| t.name.as_str().to_string()),
                        raw_expr.span(),
                    ));
                    global_scope.borrow_mut().insert(
                        env_binding.name.as_str().to_string(),
                        LexicalBinding::Expr(rewritten.clone()),
                    );
                    global_env_scope.insert(
                        env_binding.name.as_str().to_string(),
                        EnvEntry::Bound(rewritten),
                    );
                }
            }
            Item::Node(node) => {
                if node.name.as_str() == "Window" {
                    continue;
                }
                let elem_ctx = ElementContext {
                    parent_id: Some(NodeId::WINDOW),
                    parent_name: Some("window"),
                    prev_sibling_id: last_root_id,
                    has_next_sibling: false,
                    parent_ports: &window_scope_ports,
                    lexical_scope: &global_scope,
                    env_scope: &global_env_scope,
                    enclosing_component_id: None,
                    enclosing_num_children: 0,
                    enclosing_instantiated_children: None,
                    shadowed_ports: None,
                    is_let: false,
                    ambient_authored_ports: None,
                    enums: &enums,
                    active_scope_id: ScopeId::ROOT,
                    consumer_children: None,
                };
                let root_id = expand_element(node, &elem_ctx, &registry, &mut expanded_doc, &mut node_fonts)?;
                expanded_doc.roots.push(root_id);
                last_root_id = Some(root_id);
            }
            Item::Let(let_binding) => match &let_binding.value {
                None => {}
                Some(LetValue::Expr(raw_expr)) => {
                    let rewritten = {
                        let scope_borrow = global_scope.borrow();
                        let scope_ctx = ScopeContext {
                            current_node: NodeId::WINDOW,
                            parent_node: None,
                            parent_name: None,
                            prev_sibling: None,
                            has_next_sibling: false,
                            child_ids: &empty_children,
                            num_children: 0,
                            child_node: None,
                            nodes: &expanded_doc.nodes,
                            shadowed_ports: None,
                            is_event_port: false,
                            parent_ports: &empty_ports,
                            current_ports: &empty_ports,
                            lexical_scope: &*scope_borrow,
                            env_scope: &global_env_scope,
                            node_fonts: &node_fonts,
                            enclosing_component: None,
                            comp_ports: None,
                            declared_state_names: None,
                            enums: &enums,
                        };
                        rewrite_expr(raw_expr, &scope_ctx)?
                    };
                    let rewritten = coerce_typed_binding_expr(
                        rewritten,
                        let_binding.type_annotation.as_ref(),
                    );
                    expanded_doc.local_bindings.push((
                        rewritten.clone(),
                        let_binding.type_annotation.as_ref().map(|t| t.name.as_str().to_string()),
                        raw_expr.span(),
                    ));
                    global_scope.borrow_mut().insert(
                        let_binding.name.as_str().to_string(),
                        LexicalBinding::Expr(rewritten),
                    );
                }
                Some(LetValue::Node(elem)) => {
                    validate_node_type_annotation(
                        elem.name.as_str(),
                        elem.span,
                        let_binding.type_annotation.as_ref(),
                        &enums,
                        &registry,
                    )?;
                    let root_id = NodeId(expanded_doc.nodes.len());
                    global_scope.borrow_mut().insert(
                        let_binding.name.as_str().to_string(),
                        LexicalBinding::Node(root_id),
                    );
                    let elem_ctx = ElementContext {
                        parent_id: Some(NodeId::WINDOW),
                        parent_name: Some("window"),
                        prev_sibling_id: last_root_id,
                        has_next_sibling: false,
                        parent_ports: &window_scope_ports,
                        lexical_scope: &global_scope,
                        env_scope: &global_env_scope,
                        enclosing_component_id: None,
                        enclosing_num_children: 0,
                        enclosing_instantiated_children: None,
                        shadowed_ports: None,
                        is_let: true,
                        ambient_authored_ports: None,
                        enums: &enums,
                        active_scope_id: ScopeId::ROOT,
                        consumer_children: None,
                    };
                    let expanded_root_id = expand_element(elem, &elem_ctx, &registry, &mut expanded_doc, &mut node_fonts)?;
                    debug_assert_eq!(root_id, expanded_root_id);
                    expanded_doc.get_node_mut(root_id).unwrap().var_name = Some(let_binding.name.as_str().to_string());
                    expanded_doc.roots.push(root_id);
                    last_root_id = Some(root_id);
                    if elem.name.as_str() == "Font" {
                        node_fonts.insert(root_id, root_id);
                    }
                }
            }
            Item::State(_) => {}
            Item::Use(_) => {}
            Item::Enum(_) => {}
        }
    }

    let final_global_scope = global_scope.borrow();
    let final_window_scope_ctx = ScopeContext {
        current_node: NodeId::WINDOW,
        parent_node: None,
        parent_name: None,
        prev_sibling: None,
        has_next_sibling: false,
        child_ids: &empty_children,
        num_children: 0,
        child_node: None,
        nodes: &expanded_doc.nodes,
        shadowed_ports: None,
        is_event_port: false,
        parent_ports: &empty_ports,
        current_ports: &empty_ports,
        lexical_scope: &*final_global_scope,
        env_scope: &global_env_scope,
        node_fonts: &node_fonts,
        enclosing_component: None,
        comp_ports: None,
        declared_state_names: None,
        enums: &enums,
    };
    for (name, expr) in expanded_doc.window_ports.clone() {
        let rewritten = rewrite_expr(&expr, &final_window_scope_ctx)?;
        expanded_doc.window_ports.insert(name, rewritten);
    }
    for expr in expanded_doc.window_ports.values_mut() {
        *expr = resolve_forward_scope_refs(expr, &final_global_scope);
    }
    for node in &mut expanded_doc.nodes {
        for expr in node.ports.values_mut() {
            *expr = resolve_forward_scope_refs(expr, &final_global_scope);
        }
    }
    for (expr, _, _) in &mut expanded_doc.local_bindings {
        *expr = resolve_forward_scope_refs(expr, &final_global_scope);
    }

    typecheck_expanded_document(&mut expanded_doc, &known_enums, &registry)?;

    Ok(expanded_doc)
}

#[derive(Clone)]
struct ForwardedChildrenDirective<'a> {
    pub ports: &'a [PortBinding],
    pub parent_id: NodeId,
    pub parent_name: &'a str,
    pub parent_ports: Vec<String>,
    pub lexical_scope: &'a RefCell<HashMap<String, LexicalBinding>>,
    pub env_scope: HashMap<String, EnvEntry>,
    pub enclosing_component_id: Option<NodeId>,
    pub enclosing_num_children: usize,
    pub enclosing_instantiated_children: Option<&'a RefCell<Vec<NodeId>>>,
    pub shadowed_ports: Option<&'a HashSet<String>>,
    pub comp_ports: Option<&'a HashMap<String, Expr>>,
    pub declared_state_names: Option<&'a HashSet<String>>,
}

#[derive(Clone)]
struct ConsumerChildEntry<'a> {
    pub elem: ElementNode,
    pub var_name: Option<String>,
    pub raw_text: Option<TextChunk>,
    pub caller_lexical_scope: &'a RefCell<HashMap<String, LexicalBinding>>,
    pub caller_env_scope: HashMap<String, EnvEntry>,
    pub caller_enclosing_component_id: Option<NodeId>,
    pub caller_enclosing_num_children: usize,
    pub caller_enclosing_instantiated_children: Option<&'a RefCell<Vec<NodeId>>>,
    pub caller_shadowed_ports: Option<&'a HashSet<String>>,
    pub instantiated_ids: Vec<&'a RefCell<Vec<NodeId>>>,
    pub forwarded_directives: Vec<ForwardedChildrenDirective<'a>>,
}

struct InstanceContext<'a> {
    pub comp_node_id: NodeId,
    pub parent_id: Option<NodeId>,
    pub parent_name: Option<&'a str>,
    pub prev_sibling_id: Option<NodeId>,
    pub has_next_sibling: bool,
    pub parent_ports: &'a [String],
    pub ambient_authored_ports: Option<&'a HashMap<String, Expr>>,
    pub enclosing_component_id: Option<NodeId>,
    pub enclosing_num_children: usize,
    pub enclosing_instantiated_children: Option<&'a RefCell<Vec<NodeId>>>,
    pub shadowed_ports: Option<&'a HashSet<String>>,
    pub enums: &'a HashMap<String, EnumDef>,
    pub active_scope_id: ScopeId,
    pub consumer_children: Option<&'a [ConsumerChildEntry<'a>]>,
}

fn collect_nested_aliases<'a>(items: &'a [ContentItem], out: &mut Vec<&'a AliasBinding>) {
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

pub fn collect_all_component_aliases<'a>(comp_def: &'a ComponentDef) -> Vec<&'a AliasBinding> {
    let mut aliases = Vec::new();
    collect_nested_aliases(&comp_def.body, &mut aliases);
    aliases
}

fn collect_expr_lets<'a>(expr: &'a Expr, out: &mut Vec<&'a LetBinding>) {
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

fn collect_element_lets<'a>(elem: &'a ElementNode, out: &mut Vec<&'a LetBinding>) {
    for port in &elem.ports {
        collect_expr_lets(&port.expr, out);
    }
    if let Some(content) = &elem.content {
        collect_nested_lets(&content.items, out);
    }
}

fn collect_nested_lets<'a>(items: &'a [ContentItem], out: &mut Vec<&'a LetBinding>) {
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
            ContentItem::Text(_) => {}
        }
    }
}

pub fn collect_all_component_lets<'a>(comp_def: &'a ComponentDef) -> Vec<&'a LetBinding> {
    let mut lets = Vec::new();
    collect_nested_lets(&comp_def.body, &mut lets);
    lets
}

pub fn collect_all_document_lets<'a>(doc: &'a Document) -> Vec<&'a LetBinding> {
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

fn resolve_forward_scope_refs(expr: &Expr, scope: &HashMap<String, LexicalBinding>) -> Expr {
    match expr {
        Expr::Ident(id) => {
            if let Some(binding) = scope.get(id.as_str()) {
                match binding {
                    LexicalBinding::Node(node_id) => {
                        Expr::Ident(Ident::new(node_id.canonical_name(), id.span))
                    }
                    LexicalBinding::Expr(e) | LexicalBinding::State(e) => e.clone(),
                    LexicalBinding::Uninitialized => expr.clone(),
                }
            } else {
                expr.clone()
            }
        }
        Expr::MemberAccess(m) => {
            let target_ident_name = match m.target.as_ref() {
                Expr::Ident(id) => Some(id.as_str()),
                _ => None,
            };
            if let Some(target_name) = target_ident_name {
                if let Some(LexicalBinding::Node(node_id)) = scope.get(target_name) {
                    return Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(node_id.canonical_name(), m.target.span()))),
                        member: m.member.clone(),
                        span: m.span,
                    });
                }
            }
            Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(resolve_forward_scope_refs(&m.target, scope)),
                member: m.member.clone(),
                span: m.span,
            })
        }
        Expr::Binary(b) => Expr::Binary(BinaryExpr {
            op: b.op,
            left: Box::new(resolve_forward_scope_refs(&b.left, scope)),
            right: Box::new(resolve_forward_scope_refs(&b.right, scope)),
            span: b.span,
        }),
        Expr::Unary(u) => Expr::Unary(UnaryExpr {
            op: u.op,
            operand: Box::new(resolve_forward_scope_refs(&u.operand, scope)),
            span: u.span,
        }),
        Expr::Ternary(t) => Expr::Ternary(TernaryExpr {
            condition: Box::new(resolve_forward_scope_refs(&t.condition, scope)),
            then_expr: Box::new(resolve_forward_scope_refs(&t.then_expr, scope)),
            else_expr: Box::new(resolve_forward_scope_refs(&t.else_expr, scope)),
            span: t.span,
        }),
        Expr::Paren(inner, span) => Expr::Paren(
            Box::new(resolve_forward_scope_refs(inner, scope)),
            *span,
        ),
        Expr::Call(c) => Expr::Call(CallExpr {
            callee: c.callee.clone(),
            args: c.args.iter().map(|a| resolve_forward_scope_refs(a, scope)).collect(),
            span: c.span,
        }),
        Expr::MethodCall(m) => Expr::MethodCall(MethodCallExpr {
            target: Box::new(resolve_forward_scope_refs(&m.target, scope)),
            method: m.method.clone(),
            args: m.args.iter().map(|a| resolve_forward_scope_refs(a, scope)).collect(),
            span: m.span,
        }),
        _ => expr.clone(),
    }
}

struct ElementContext<'a> {
    pub parent_id: Option<NodeId>,
    pub parent_name: Option<&'a str>,
    pub prev_sibling_id: Option<NodeId>,
    pub has_next_sibling: bool,
    pub parent_ports: &'a [String],
    pub lexical_scope: &'a RefCell<HashMap<String, LexicalBinding>>,
    pub env_scope: &'a HashMap<String, EnvEntry>,
    pub enclosing_component_id: Option<NodeId>,
    pub enclosing_num_children: usize,
    pub enclosing_instantiated_children: Option<&'a RefCell<Vec<NodeId>>>,
    pub shadowed_ports: Option<&'a HashSet<String>>,
    pub is_let: bool,
    pub ambient_authored_ports: Option<&'a HashMap<String, Expr>>,
    pub enums: &'a HashMap<String, EnumDef>,
    pub active_scope_id: ScopeId,
    pub consumer_children: Option<&'a [ConsumerChildEntry<'a>]>,
}

/// Statically selects the unique matching component overload for an invocation.
fn select_component_overload<'a>(
    overloads: &'a [ComponentDef],
    comp_name: &str,
    elem: &ElementNode,
    ambient_authored_ports: Option<&HashMap<String, Expr>>,
    env_scope: &HashMap<String, EnvEntry>,
    span: Span,
) -> Result<&'a ComponentDef, CompileError> {
    let explicit_ports: HashSet<String> =
        elem.ports.iter().map(|p| p.name.as_str().to_string()).collect();

    let any_overload_accepts = |port_name: &str| -> bool {
        overloads.iter().any(|o| o.param_names().contains(port_name))
    };

    let matching: Vec<&ComponentDef> = overloads
        .iter()
        .filter(|o| {
            let all = o.param_names();
            // 1. All caller-authored ports must be accepted by this overload.
            // Purely ambient ports injected by a container or standard spatial properties
            // (x, y, z, clip) only disqualify an overload if at least one overload of this
            // component actually accepts the port (e.g. ambient width injected by stretch).
            for port in &elem.ports {
                let name = port.name.as_str();
                let is_purely_ambient = ambient_authored_ports.is_some_and(|a| a.contains_key(name));
                let is_unrelated_ambient = is_purely_ambient && !any_overload_accepts(name);
                let is_standard_spatial = matches!(name, "x" | "y" | "z" | "clip");
                let is_unrelated_spatial = is_standard_spatial && !any_overload_accepts(name);
                if !is_unrelated_ambient
                    && !is_unrelated_spatial
                    && !all.contains(name)
                    && !name.starts_with("on_")
                {
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
                .filter(|p| {
                    let is_ambient = ambient_authored_ports.is_some_and(|a| a.contains_key(p));
                    (!is_ambient || any_overload_accepts(p)) && !p.starts_with("on_")
                })
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

/// Expands a single element node (either a component invocation or a primitive).
fn expand_element(
    elem: &ElementNode,
    ctx: &ElementContext<'_>,
    registry: &HashMap<String, Vec<ComponentDef>>,
    doc: &mut ExpandedDocument,
    node_fonts: &mut HashMap<NodeId, NodeId>,
) -> Result<NodeId, CompileError> {
    if elem.name.as_str() == "Window" {
        return Err(CompileError::NestedWindowDeclaration { span: elem.span });
    }

    for port in &elem.ports {
        if port.name.as_str() == "parent" {
            return Err(CompileError::ReservedPort {
                node: elem.name.as_str().to_string(),
                port: "parent".to_string(),
                span: port.name.span,
            });
        }
    }

    let node_id = NodeId(doc.nodes.len());
    let mut expanded = ExpandedNode::new(node_id, elem.name.as_str(), elem.span);
    expanded.handle = elem.handle;
    expanded.parent = ctx.parent_id;
    expanded.prev_sibling = ctx.prev_sibling_id;
    expanded.scope_id = Some(ctx.active_scope_id);
    expanded.enclosing_component = ctx.enclosing_component_id;
    if let Some(ambients) = ctx.ambient_authored_ports {
        expanded.ambient_ports = ambients.keys().cloned().collect();
    }
    doc.scope_tree.node_to_scope.insert(node_id, ctx.active_scope_id);

    if let Some(key) = &elem.key {
        validate_component_key(elem.name.as_str(), key, &ctx.lexical_scope.borrow())?;
        let empty_ports: [String; 0] = [];
        let empty_children: [NodeId; 0] = [];
        let inst_borrow = ctx.enclosing_instantiated_children.map(|c| c.borrow());
        let child_ids_slice: &[NodeId] = match &inst_borrow {
            Some(b) => b.as_slice(),
            None => &empty_children,
        };
        let key_scope_ctx = ScopeContext {
            current_node: node_id,
            parent_node: ctx.parent_id,
            parent_name: ctx.parent_name,
            prev_sibling: ctx.prev_sibling_id,
            has_next_sibling: ctx.has_next_sibling,
            child_ids: child_ids_slice,
            num_children: ctx.enclosing_num_children,
            child_node: None,
            nodes: &doc.nodes,
            shadowed_ports: ctx.shadowed_ports,
            is_event_port: false,
            parent_ports: ctx.parent_ports,
            current_ports: &empty_ports,
            lexical_scope: &*ctx.lexical_scope.borrow(),
            env_scope: ctx.env_scope,
            node_fonts,
            enclosing_component: ctx.enclosing_component_id,
            comp_ports: None,
            declared_state_names: None,
            enums: ctx.enums,
        };
        let mut rewritten_parts = Vec::new();
        for part in &key.parts {
            rewritten_parts.push(rewrite_expr(part, &key_scope_ctx)?);
        }
        expanded.key = Some(ComponentKey::new(rewritten_parts, key.span));
    }

    // Check if element has text content directly in content slot
    if let Some(content_slot) = &elem.content {
        let mut text_parts = Vec::new();
        for item in &content_slot.items {
            if let ContentItem::Text(chunk) = item {
                text_parts.push(chunk.text.as_str());
            }
        }
        if !text_parts.is_empty() {
            expanded.text_content = Some(text_parts.join(" "));
        }
    }

    // Allocate placeholder in doc.nodes to reserve index
    doc.nodes.push(expanded);

    // If it's an invocation of a user-defined component:
    if let Some(overloads) = registry.get(elem.name.as_str()) {
        // Validate that caller does not attempt to override immutable spatial alias ports or declared aliases
        for port in &elem.ports {
            let name = port.name.as_str();
            let is_purely_ambient = ctx.ambient_authored_ports.is_some_and(|a| a.contains_key(name));
            if is_purely_ambient {
                continue;
            }
            let is_accepted_param = overloads.iter().any(|o| o.param_names().contains(name));
            if !is_accepted_param
                && (matches!(name, "left" | "top" | "right" | "bottom")
                    || overloads.iter().any(|o| {
                        collect_all_component_aliases(o).iter().any(|a| a.name.as_str() == name)
                    }))
            {
                return Err(CompileError::ImmutableAliasPort {
                    node: elem.name.as_str().to_string(),
                    port: name.to_string(),
                    span: port.name.span,
                });
            }
        }

        let comp_def = if overloads.len() == 1 {
            &overloads[0]
        } else {
            select_component_overload(
                overloads,
                elem.name.as_str(),
                elem,
                ctx.ambient_authored_ports,
                ctx.env_scope,
                elem.span,
            )?
        };
        let inst_ctx = InstanceContext {
            comp_node_id: node_id,
            parent_id: ctx.parent_id,
            parent_name: ctx.parent_name,
            prev_sibling_id: ctx.prev_sibling_id,
            has_next_sibling: ctx.has_next_sibling,
            parent_ports: ctx.parent_ports,
            ambient_authored_ports: ctx.ambient_authored_ports,
            enclosing_component_id: ctx.enclosing_component_id,
            enclosing_num_children: ctx.enclosing_num_children,
            enclosing_instantiated_children: ctx.enclosing_instantiated_children,
            shadowed_ports: ctx.shadowed_ports,
            enums: ctx.enums,
            active_scope_id: ctx.active_scope_id,
            consumer_children: ctx.consumer_children,
        };
        expand_component_instance(
            &inst_ctx,
            elem,
            comp_def,
            registry,
            doc,
            ctx.lexical_scope,
            ctx.env_scope,
            node_fonts,
        )?;
    } else {
        // It's a primitive element (e.g. \Rect, \Text, \Header, etc.)
        expand_primitive_element(
            node_id,
            elem,
            ctx,
            registry,
            doc,
            node_fonts,
        )?;
    }

    Ok(node_id)
}

/// Expands a component instance, wiring parameters, component body, and consumer children.
#[allow(clippy::too_many_arguments)]
fn expand_component_instance(
    ctx: &InstanceContext<'_>,
    instance: &ElementNode,
    comp_def: &ComponentDef,
    registry: &HashMap<String, Vec<ComponentDef>>,
    doc: &mut ExpandedDocument,
    lexical_scope: &RefCell<HashMap<String, LexicalBinding>>,
    caller_env_scope: &HashMap<String, EnvEntry>,
    node_fonts: &mut HashMap<NodeId, NodeId>,
) -> Result<(), CompileError> {
    // 1. Gather parameter definitions and map consumer arguments
    let mut comp_scope_ports = vec![
        "clip".to_string(),
    ];
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

    // Validate that caller does not attempt to override immutable alias ports or set private state / let / env
    for port in &instance.ports {
        let name = port.name.as_str();
        let is_purely_ambient = ctx.ambient_authored_ports.is_some_and(|a| a.contains_key(name));
        if is_purely_ambient {
            continue;
        }
        if declared_aliases.contains_key(name) || matches!(name, "left" | "top" | "right" | "bottom") {
            return Err(CompileError::ImmutableAliasPort {
                node: comp_def.name.as_str().to_string(),
                port: name.to_string(),
                span: port.name.span,
            });
        }
        if declared_states.contains_key(name) {
            return Err(CompileError::PrivateStatePort {
                node: comp_def.name.as_str().to_string(),
                port: name.to_string(),
                span: port.name.span,
            });
        }
        if declared_envs.contains_key(name) && !comp_def.params.iter().any(|p| p.name.as_str() == name) {
            return Err(CompileError::Custom {
                message: format!(
                    "Node '{}' environmental property '{}' cannot be set as a parameter",
                    comp_def.name.as_str(),
                    name
                ),
                span: port.name.span,
            });
        }
        if all_let_names.contains(name) && !comp_def.params.iter().any(|p| p.name.as_str() == name) {
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

    let mut comp_ports = HashMap::new();
    for (state_name, s) in &declared_states {
        let default_expr = default_expr_for_state_with_enums(s, ctx.enums);
        comp_ports.insert(state_name.clone(), default_expr);
    }
    for (let_name, l) in &declared_lets {
        if !comp_shadowed_ports.contains(let_name) {
            if let Some(LetValue::Expr(raw_expr)) = &l.value {
                comp_ports.insert(let_name.clone(), raw_expr.clone());
            }
        }
    }
    for (env_name, e) in &declared_envs {
        if !comp_shadowed_ports.contains(env_name) {
            if let Some(raw_expr) = &e.value {
                if !matches!(raw_expr, Expr::Node(_)) {
                    comp_ports.insert(env_name.clone(), raw_expr.clone());
                }
            }
        }
    }

    // Map explicit arguments passed to the component (Tier 4)
    let mut explicit_ports = HashMap::new();
    let mut already_rewritten_ports: HashSet<String> = HashSet::new();
    let empty_ports: [String; 0] = [];
    let empty_children: [NodeId; 0] = [];
    let instantiated_children_ids = RefCell::new(Vec::new());
    let num_consumer_children = instance.content.as_ref().map_or(0, |slot| {
        slot.items
            .iter()
            .map(|item| match item {
                ContentItem::Node(_) => 1,
                ContentItem::Text(chunk) if !chunk.text.trim().is_empty() => 1,
                ContentItem::Children(_) => ctx.consumer_children.map_or(0, |c| c.len()),
                ContentItem::Let(l) if matches!(l.value, Some(LetValue::Node(_))) => 1,
                _ => 0,
            })
            .sum()
    });

    // Pre-resolve explicit "font" port if passed on this component instance so sibling ports (e.g. `1em`) see it
    if let Some(font_port) = instance.ports.iter().find(|p| p.name.as_str() == "font") {
        if !matches!(&font_port.expr, Expr::Node(_)) {
            let caller_scope_borrow = lexical_scope.borrow();
            let caller_inst_borrow = ctx.enclosing_instantiated_children.map(|c| c.borrow());
            let caller_child_ids: &[NodeId] = match &caller_inst_borrow {
                Some(b) => b.as_slice(),
                None => &empty_children,
            };
            let caller_scope_ctx = ScopeContext {
                current_node: ctx.comp_node_id,
                parent_node: ctx.parent_id,
                parent_name: ctx.parent_name,
                prev_sibling: ctx.prev_sibling_id,
                has_next_sibling: ctx.has_next_sibling,
                child_ids: caller_child_ids,
                num_children: ctx.enclosing_num_children,
                child_node: None,
                nodes: &doc.nodes,
                shadowed_ports: ctx.shadowed_ports,
                is_event_port: false,
                parent_ports: ctx.parent_ports,
                current_ports: &empty_ports,
                lexical_scope: &*caller_scope_borrow,
                env_scope: caller_env_scope,
                node_fonts,
                enclosing_component: ctx.enclosing_component_id,
                comp_ports: None,
                declared_state_names: None,
                enums: ctx.enums,
            };
            if let Ok(Expr::Ident(id)) = rewrite_expr(&font_port.expr, &caller_scope_ctx) {
                if let Some(fid) = NodeId::from_canonical_name(id.as_str()) {
                    node_fonts.insert(ctx.comp_node_id, fid);
                    doc.get_node_mut(ctx.comp_node_id).unwrap().font = Some(fid);
                }
            }
        }
    }

    let mut explicit_unlowered_ports = HashMap::new();
    for port in &instance.ports {
        let name = port.name.as_str().to_string();
        explicit_unlowered_ports.insert(name.clone(), port.expr.clone());
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
                    lexical_scope,
                    env_scope: caller_env_scope,
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
                let child_id = expand_element(inline_elem, &child_ctx, registry, doc, node_fonts)?;
                if name == "font" && inline_elem.name.as_str() == "Font" {
                    node_fonts.insert(ctx.comp_node_id, child_id);
                    doc.get_node_mut(ctx.comp_node_id).unwrap().font = Some(child_id);
                }
                Expr::Ident(Ident::new(child_id.canonical_name(), inline_elem.span))
            }
            other => {
                let caller_scope_borrow = lexical_scope.borrow();
                let caller_inst_borrow = ctx.enclosing_instantiated_children.map(|c| c.borrow());
                let caller_child_ids: &[NodeId] = match &caller_inst_borrow {
                    Some(b) => b.as_slice(),
                    None => &empty_children,
                };
                let caller_scope_ctx = ScopeContext {
                    current_node: ctx.comp_node_id,
                    parent_node: ctx.parent_id,
                    parent_name: ctx.parent_name,
                    prev_sibling: ctx.prev_sibling_id,
                    has_next_sibling: ctx.has_next_sibling,
                    child_ids: caller_child_ids,
                    num_children: ctx.enclosing_num_children,
                    child_node: None,
                    nodes: &doc.nodes,
                    shadowed_ports: ctx.shadowed_ports,
                    is_event_port: name.starts_with("on_") || is_handler_param,
                    parent_ports: ctx.parent_ports,
                    current_ports: &empty_ports,
                    lexical_scope: &*caller_scope_borrow,
                    env_scope: caller_env_scope,
                    node_fonts,
                    enclosing_component: ctx.enclosing_component_id,
                    comp_ports: None,
                    declared_state_names: None,
                    enums: ctx.enums,
                };
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
        if !comp_scope_ports.contains(&name) {
            comp_scope_ports.push(name.clone());
        }

        let mut resolve_default_expr = |default_expr: &Expr| -> Result<Expr, CompileError> {
            match default_expr {
                Expr::Node(inline_elem) => {
                    let child_ctx = ElementContext {
                        parent_id: Some(ctx.comp_node_id),
                        parent_name: ctx.parent_name,
                        prev_sibling_id: ctx.prev_sibling_id,
                        has_next_sibling: false,
                        parent_ports: ctx.parent_ports,
                        lexical_scope,
                        env_scope: caller_env_scope,
                        enclosing_component_id: Some(ctx.comp_node_id),
                        enclosing_num_children: num_consumer_children,
                        enclosing_instantiated_children: Some(&instantiated_children_ids),
                        shadowed_ports: Some(&comp_shadowed_ports),
                        is_let: false,
                        ambient_authored_ports: None,
                        enums: ctx.enums,
                        active_scope_id: ctx.active_scope_id,
                        consumer_children: None,
                    };
                    let child_id = expand_element(inline_elem, &child_ctx, registry, doc, node_fonts)?;
                    if name == "font" && inline_elem.name.as_str() == "Font" {
                        node_fonts.insert(ctx.comp_node_id, child_id);
                        doc.get_node_mut(ctx.comp_node_id).unwrap().font = Some(child_id);
                    }
                    Ok(Expr::Ident(Ident::new(child_id.canonical_name(), inline_elem.span)))
                }
                other => Ok(other.clone()),
            }
        };

        // Tier 4: Explicit caller argument
        if let Some(explicit_expr) = explicit_ports.remove(&name) {
            already_rewritten_ports.insert(name.clone());
            comp_ports.insert(name, explicit_expr);
        }
        // Tier 2: Lexical Environment (caller_env_scope) - only for env parameters
        else if param.is_env {
            if let Some(env_entry) = caller_env_scope.get(&name) {
                match env_entry {
                    EnvEntry::Bound(env_expr) => {
                        already_rewritten_ports.insert(name.clone());
                        comp_ports.insert(name, env_expr.clone());
                    }
                    EnvEntry::Tombstone => {
                        // Tombstone halts lookup! Fall back to Tier 1 default
                        if let Some(default_expr) = &param.default_edge {
                            let resolved = resolve_default_expr(default_expr)?;
                            comp_ports.insert(name, resolved);
                        } else if name == "text_bounds" {
                            let default_tb = Expr::Node(Box::new(ElementNode {
                                name: Ident::new("TextBounds", param.span),
                                key: None,
                                ports: Vec::new(),
                                content: None,
                                span: param.span,
                                handle: None,
                            }));
                            let resolved = resolve_default_expr(&default_tb)?;
                            comp_ports.insert(name, resolved);
                        }
                    }
                }
            } else if let Some(default_expr) = &param.default_edge {
                let resolved = resolve_default_expr(default_expr)?;
                comp_ports.insert(name, resolved);
            } else if name == "text_bounds" {
                let default_tb = Expr::Node(Box::new(ElementNode {
                    name: Ident::new("TextBounds", param.span),
                    key: None,
                    ports: Vec::new(),
                    content: None,
                    span: param.span,
                    handle: None,
                }));
                let resolved = resolve_default_expr(&default_tb)?;
                comp_ports.insert(name, resolved);
            }
        }
        // Tier 1: Component Signature Default (for non-env parameters)
        else if let Some(default_expr) = &param.default_edge {
            let resolved = resolve_default_expr(default_expr)?;
            comp_ports.insert(name, resolved);
        }
    }

    // Any remaining explicit ports not defined in signature (e.g. ad-hoc ports)
    for (name, expr) in explicit_ports {
        let is_purely_ambient = ctx.ambient_authored_ports.is_some_and(|a| a.contains_key(&name));
        if is_purely_ambient && name != "clip" {
            continue;
        }
        if !comp_scope_ports.contains(&name) {
            comp_scope_ports.push(name.clone());
        }
        already_rewritten_ports.insert(name.clone());
        comp_ports.insert(name, expr);
    }

    let has_port_or_alias = |k: &str| comp_ports.contains_key(k) || declared_aliases.contains_key(k);
    if has_port_or_alias("x") && !comp_scope_ports.contains(&"left".to_string()) {
        comp_scope_ports.push("left".to_string());
    }
    if has_port_or_alias("y") && !comp_scope_ports.contains(&"top".to_string()) {
        comp_scope_ports.push("top".to_string());
    }
    if (has_port_or_alias("x") || has_port_or_alias("left"))
        && has_port_or_alias("width")
        && !comp_scope_ports.contains(&"right".to_string())
    {
        comp_scope_ports.push("right".to_string());
    }
    if (has_port_or_alias("y") || has_port_or_alias("top"))
        && has_port_or_alias("height")
        && !comp_scope_ports.contains(&"bottom".to_string())
    {
        comp_scope_ports.push("bottom".to_string());
    }

    // Record component font if resolved (including Tier 1 / Tier 2 defaults)
    if let Some(font_expr) = comp_ports.get("font").cloned() {
        let resolved_font_expr = if let Expr::Ident(id) = &font_expr {
            if NodeId::from_canonical_name(id.as_str()).is_some() {
                Some(font_expr)
            } else {
                let caller_scope_borrow = lexical_scope.borrow();
                let temp_ctx = ScopeContext {
                    current_node: ctx.comp_node_id,
                    parent_node: ctx.parent_id,
                    parent_name: ctx.parent_name,
                    prev_sibling: ctx.prev_sibling_id,
                    has_next_sibling: ctx.has_next_sibling,
                    child_ids: &empty_children,
                    num_children: num_consumer_children,
                    child_node: None,
                    nodes: &doc.nodes,
                    shadowed_ports: Some(&comp_shadowed_ports),
                    is_event_port: false,
                    parent_ports: ctx.parent_ports,
                    current_ports: &empty_ports,
                    lexical_scope: &*caller_scope_borrow,
                    env_scope: caller_env_scope,
                    node_fonts,
                    enclosing_component: ctx.enclosing_component_id,
                    comp_ports: None,
                    declared_state_names: None,
                    enums: ctx.enums,
                };
                rewrite_expr(&font_expr, &temp_ctx).ok()
            }
        } else {
            let caller_scope_borrow = lexical_scope.borrow();
            let temp_ctx = ScopeContext {
                current_node: ctx.comp_node_id,
                parent_node: ctx.parent_id,
                parent_name: ctx.parent_name,
                prev_sibling: ctx.prev_sibling_id,
                has_next_sibling: ctx.has_next_sibling,
                child_ids: &empty_children,
                num_children: num_consumer_children,
                child_node: None,
                nodes: &doc.nodes,
                shadowed_ports: Some(&comp_shadowed_ports),
                is_event_port: false,
                parent_ports: ctx.parent_ports,
                current_ports: &empty_ports,
                lexical_scope: &*caller_scope_borrow,
                env_scope: caller_env_scope,
                node_fonts,
                enclosing_component: ctx.enclosing_component_id,
                comp_ports: None,
                declared_state_names: None,
                enums: ctx.enums,
            };
            rewrite_expr(&font_expr, &temp_ctx).ok()
        };
        if let Some(Expr::Ident(id)) = resolved_font_expr {
            if let Some(fid) = NodeId::from_canonical_name(id.as_str()) {
                comp_ports.insert("font".to_string(), Expr::Ident(id));
                node_fonts.insert(ctx.comp_node_id, fid);
                doc.get_node_mut(ctx.comp_node_id).unwrap().font = Some(fid);
            }
        }
    }

    // Default clip port if not explicitly declared
    if !comp_ports.contains_key("clip") {
        if let Some(EnvEntry::Bound(clip_expr)) = caller_env_scope.get("clip") {
            already_rewritten_ports.insert("clip".to_string());
            comp_ports.insert("clip".to_string(), clip_expr.clone());
        } else {
            let default_clip_expr = if let Some(parent) = ctx.parent_id {
                if parent.is_window() {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(NodeId::WINDOW.canonical_name(), instance.span))),
                        member: Ident::new("clip", instance.span),
                        span: instance.span,
                    })
                } else {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(parent.canonical_name(), instance.span))),
                        member: Ident::new("clip", instance.span),
                        span: instance.span,
                    })
                }
            } else {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(NodeId::WINDOW.canonical_name(), instance.span))),
                    member: Ident::new("clip", instance.span),
                    span: instance.span,
                })
            };
            already_rewritten_ports.insert("clip".to_string());
            comp_ports.insert("clip".to_string(), default_clip_expr);
        }
    }

    // Validate that all required parameters (parameters without defaults) have been supplied
    for param in &comp_def.params {
        let name = param.name.as_str();
        if param.default_edge.is_none() && !comp_ports.contains_key(name) {
            return Err(CompileError::MissingPort {
                node: comp_def.name.as_str().to_string(),
                port: name.to_string(),
                span: instance.span,
            });
        }
    }

    let mut internal_scope_ports = comp_scope_ports.clone();
    for let_name in declared_lets.keys() {
        if !internal_scope_ports.contains(let_name) {
            internal_scope_ports.push(let_name.clone());
        }
    }

    // 2. Collect consumer children passed to this component instance
    let mut consumer_child_entries: Vec<ConsumerChildEntry<'_>> = Vec::new();
    if let Some(slot) = &instance.content {
        for item in &slot.items {
            match item {
                ContentItem::Node(child_elem) => {
                    consumer_child_entries.push(ConsumerChildEntry {
                        elem: child_elem.clone(),
                        var_name: None,
                        raw_text: None,
                        caller_lexical_scope: lexical_scope,
                        caller_env_scope: caller_env_scope.clone(),
                        caller_enclosing_component_id: ctx.enclosing_component_id,
                        caller_enclosing_num_children: ctx.enclosing_num_children,
                        caller_enclosing_instantiated_children: ctx.enclosing_instantiated_children,
                        caller_shadowed_ports: ctx.shadowed_ports,
                        instantiated_ids: vec![&instantiated_children_ids],
                        forwarded_directives: Vec::new(),
                    });
                }
                ContentItem::Text(chunk) => {
                    let trimmed = chunk.text.trim();
                    if !trimmed.is_empty() {
                        let synthetic_text = ElementNode {
                            name: Ident::new("Text", chunk.span),
                            key: None,
                            ports: Vec::new(),
                            content: Some(ContentSlot {
                                items: vec![ContentItem::Text(chunk.clone())],
                                span: chunk.span,
                            }),
                            span: chunk.span,
                            handle: None,
                        };
                        consumer_child_entries.push(ConsumerChildEntry {
                            elem: synthetic_text,
                            var_name: None,
                            raw_text: Some(chunk.clone()),
                            caller_lexical_scope: lexical_scope,
                            caller_env_scope: caller_env_scope.clone(),
                            caller_enclosing_component_id: ctx.enclosing_component_id,
                            caller_enclosing_num_children: ctx.enclosing_num_children,
                            caller_enclosing_instantiated_children: ctx.enclosing_instantiated_children,
                            caller_shadowed_ports: ctx.shadowed_ports,
                            instantiated_ids: vec![&instantiated_children_ids],
                            forwarded_directives: Vec::new(),
                        });
                    }
                }
                ContentItem::Children(forward_dir) => {
                    for port in &forward_dir.ports {
                        if port.name.as_str() == "parent" {
                            return Err(CompileError::ReservedPort {
                                node: "Children".to_string(),
                                port: "parent".to_string(),
                                span: port.name.span,
                            });
                        }
                    }
                    if let Some(outer_entries) = ctx.consumer_children {
                        for outer_entry in outer_entries {
                            let mut entry = outer_entry.clone();
                            entry.instantiated_ids.push(&instantiated_children_ids);
                            if !forward_dir.ports.is_empty() {
                                entry.forwarded_directives.push(ForwardedChildrenDirective {
                                    ports: &forward_dir.ports,
                                    parent_id: ctx.comp_node_id,
                                    parent_name: comp_def.name.as_str(),
                                    parent_ports: comp_scope_ports.clone(),
                                    lexical_scope,
                                    env_scope: caller_env_scope.clone(),
                                    enclosing_component_id: ctx.enclosing_component_id,
                                    enclosing_num_children: ctx.enclosing_num_children,
                                    enclosing_instantiated_children: ctx.enclosing_instantiated_children,
                                    shadowed_ports: ctx.shadowed_ports,
                                    comp_ports: None,
                                    declared_state_names: None,
                                });
                            }
                            consumer_child_entries.push(entry);
                        }
                    }
                }
                ContentItem::Let(l) => {
                    if let Some(LetValue::Node(child_elem)) = &l.value {
                        validate_node_type_annotation(
                            child_elem.name.as_str(),
                            child_elem.span,
                            l.type_annotation.as_ref(),
                            ctx.enums,
                            registry,
                        )?;
                        consumer_child_entries.push(ConsumerChildEntry {
                            elem: child_elem.clone(),
                            var_name: Some(l.name.as_str().to_string()),
                            raw_text: None,
                            caller_lexical_scope: lexical_scope,
                            caller_env_scope: caller_env_scope.clone(),
                            caller_enclosing_component_id: ctx.enclosing_component_id,
                            caller_enclosing_num_children: ctx.enclosing_num_children,
                            caller_enclosing_instantiated_children: ctx.enclosing_instantiated_children,
                            caller_shadowed_ports: ctx.shadowed_ports,
                            instantiated_ids: vec![&instantiated_children_ids],
                            forwarded_directives: Vec::new(),
                        });
                    }
                }
                ContentItem::Alias(_) | ContentItem::State(_) | ContentItem::Env(_) | ContentItem::Assert(_) => {}
            }
        }
    }
    debug_assert_eq!(num_consumer_children, consumer_child_entries.len());

    // 3. Expand component body items in declaration order (Painter's Algorithm)
    let mut all_children_ids = Vec::new();
    let mut last_child_id: Option<NodeId> = None;
    let mut local_scope: HashMap<String, LexicalBinding> = lexical_scope.borrow().clone();
    let mut populated_comp_ports: HashSet<String> = HashSet::new();
    for param in &comp_def.params {
        let name = param.name.as_str().to_string();
        populated_comp_ports.insert(name.clone());
        let is_handler_param = param
            .type_annotation
            .as_ref()
            .is_some_and(|t| t.name.as_str() == "Handler");
        if name.starts_with("on_") || is_handler_param {
            if let Some(expr) = comp_ports.get(&name).cloned() {
                if !already_rewritten_ports.contains(&name) {
                    let empty_scope = HashMap::new();
                    let event_default_ctx = ScopeContext {
                        current_node: ctx.comp_node_id,
                        parent_node: ctx.parent_id,
                        parent_name: ctx.parent_name,
                        prev_sibling: ctx.prev_sibling_id,
                        has_next_sibling: ctx.has_next_sibling,
                        child_ids: &empty_children,
                        num_children: num_consumer_children,
                        child_node: None,
                        nodes: &doc.nodes,
                        shadowed_ports: Some(&comp_shadowed_ports),
                        is_event_port: true,
                        parent_ports: ctx.parent_ports,
                        current_ports: &comp_scope_ports,
                        lexical_scope: &empty_scope,
                        env_scope: caller_env_scope,
                        node_fonts: &*node_fonts,
                        enclosing_component: Some(ctx.comp_node_id),
                        comp_ports: None,
                        declared_state_names: None,
                        enums: ctx.enums,
                    };
                    let rewritten = rewrite_expr(&expr, &event_default_ctx)?;
                    already_rewritten_ports.insert(name.clone());
                    comp_ports.insert(name.clone(), rewritten.clone());
                    local_scope.insert(name, LexicalBinding::Expr(rewritten));
                } else {
                    local_scope.insert(name, LexicalBinding::Expr(expr));
                }
                continue;
            }
        }
        if let Some(expr) = comp_ports.get(&name) {
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
                target: Box::new(Expr::Ident(Ident::new(ctx.comp_node_id.canonical_name(), param.name.span))),
                member: param.name.clone(),
                span: param.name.span,
            })),
        );
    }
    for (let_name, let_binding) in &declared_lets {
        if !comp_shadowed_ports.contains(let_name) {
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
    for port_name in &comp_scope_ports {
        if !populated_comp_ports.contains(port_name) {
            let member_expr = Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(Expr::Ident(Ident::new(
                    ctx.comp_node_id.canonical_name(),
                    instance.span,
                ))),
                member: Ident::new(port_name.clone(), instance.span),
                span: instance.span,
            });
            let binding = if declared_state_names.contains(port_name) {
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
            target: Box::new(Expr::Ident(Ident::new(ctx.comp_node_id.canonical_name(), instance.span))),
            member: Ident::new("clip", instance.span),
            span: instance.span,
        })),
    );
    for param in &comp_def.params {
        if param.is_env {
            let env_bound_expr = if param.name.as_str() == "font" {
                if let Some(&fid) = node_fonts.get(&ctx.comp_node_id) {
                    Expr::Ident(Ident::new(fid.canonical_name(), instance.span))
                } else {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(ctx.comp_node_id.canonical_name(), instance.span))),
                        member: param.name.clone(),
                        span: instance.span,
                    })
                }
            } else if let Some(Expr::Ident(id)) = comp_ports.get(param.name.as_str()) {
                if NodeId::from_canonical_name(id.as_str()).is_some() {
                    Expr::Ident(id.clone())
                } else {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(ctx.comp_node_id.canonical_name(), instance.span))),
                        member: param.name.clone(),
                        span: instance.span,
                    })
                }
            } else {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(ctx.comp_node_id.canonical_name(), instance.span))),
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

    // Setup children_env_scope (for \Children)
    let mut children_env_scope = caller_env_scope.clone();
    children_env_scope.insert(
        "clip".to_string(),
        EnvEntry::Bound(Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(Expr::Ident(Ident::new(ctx.comp_node_id.canonical_name(), instance.span))),
            member: Ident::new("clip", instance.span),
            span: instance.span,
        })),
    );
    for param in &comp_def.params {
        if param.is_env {
            let env_bound_expr = if param.name.as_str() == "font" {
                if let Some(&fid) = node_fonts.get(&ctx.comp_node_id) {
                    Expr::Ident(Ident::new(fid.canonical_name(), instance.span))
                } else {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(ctx.comp_node_id.canonical_name(), instance.span))),
                        member: param.name.clone(),
                        span: instance.span,
                    })
                }
            } else if let Some(Expr::Ident(id)) = comp_ports.get(param.name.as_str()) {
                if NodeId::from_canonical_name(id.as_str()).is_some() {
                    Expr::Ident(id.clone())
                } else {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(ctx.comp_node_id.canonical_name(), instance.span))),
                        member: param.name.clone(),
                        span: instance.span,
                    })
                }
            } else {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(ctx.comp_node_id.canonical_name(), instance.span))),
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
    for l in &all_lets {
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
                internal_body_env_scope.insert(e.name.as_str().to_string(), EnvEntry::Tombstone);
            }
        }
    }

    // Pre-resolve let and env expressions in component body so their declaration order does not matter
    let empty_children: [NodeId; 0] = [];
    let mut changed = true;
    let mut passes = 0;
    while changed && passes < 20 {
        changed = false;
        passes += 1;
        for let_binding in &all_lets {
            if let_binding.value.is_none() {
                local_scope.insert(let_binding.name.as_str().to_string(), LexicalBinding::Uninitialized);
            } else if let Some(LetValue::Expr(raw_expr)) = &let_binding.value {
                let is_param_shadow = comp_shadowed_ports.contains(let_binding.name.as_str());
                if is_param_shadow {
                    let scope_ctx = ScopeContext {
                        current_node: ctx.comp_node_id,
                        parent_node: ctx.parent_id,
                        parent_name: ctx.parent_name,
                        prev_sibling: ctx.prev_sibling_id,
                        has_next_sibling: ctx.has_next_sibling,
                        child_ids: &empty_children,
                        num_children: num_consumer_children,
                        child_node: None,
                        nodes: &doc.nodes,
                        shadowed_ports: Some(&comp_shadowed_ports),
                        is_event_port: false,
                        parent_ports: ctx.parent_ports,
                        current_ports: &comp_scope_ports,
                        lexical_scope: &local_scope,
                        env_scope: &internal_body_env_scope,
                        node_fonts: &*node_fonts,
                        enclosing_component: Some(ctx.comp_node_id),
                        comp_ports: Some(&comp_ports),
                        declared_state_names: Some(&declared_state_names),
                        enums: ctx.enums,
                    };
                    if let Ok(rewritten) = rewrite_expr(raw_expr, &scope_ctx) {
                        let rewritten = coerce_typed_binding_expr(
                            rewritten,
                            let_binding.type_annotation.as_ref(),
                        );
                        if let Some(LexicalBinding::Expr(existing)) = local_scope.get(let_binding.name.as_str()) {
                            if existing != &rewritten {
                                local_scope.insert(let_binding.name.as_str().to_string(), LexicalBinding::Expr(rewritten));
                                changed = true;
                            }
                        } else {
                            local_scope.insert(let_binding.name.as_str().to_string(), LexicalBinding::Expr(rewritten));
                            changed = true;
                        }
                    }
                }
            }
        }
        for item in &comp_def.body {
            if let ComponentBodyItem::Env(env_binding) = item {
                if env_binding.value.is_none() {
                    local_scope.insert(env_binding.name.as_str().to_string(), LexicalBinding::Uninitialized);
                } else if let Some(raw_expr) = &env_binding.value {
                    if !matches!(raw_expr, Expr::Node(_)) {
                        let scope_ctx = ScopeContext {
                            current_node: ctx.comp_node_id,
                            parent_node: ctx.parent_id,
                            parent_name: ctx.parent_name,
                            prev_sibling: ctx.prev_sibling_id,
                            has_next_sibling: ctx.has_next_sibling,
                            child_ids: &empty_children,
                            num_children: num_consumer_children,
                            child_node: None,
                            nodes: &doc.nodes,
                            shadowed_ports: Some(&comp_shadowed_ports),
                            is_event_port: false,
                            parent_ports: ctx.parent_ports,
                            current_ports: &comp_scope_ports,
                            lexical_scope: &local_scope,
                            env_scope: &internal_body_env_scope,
                            node_fonts: &*node_fonts,
                            enclosing_component: Some(ctx.comp_node_id),
                            comp_ports: None,
                            declared_state_names: None,
                            enums: ctx.enums,
                        };
                        if let Ok(rewritten) = rewrite_expr(raw_expr, &scope_ctx) {
                            let rewritten = coerce_typed_binding_expr(
                                rewritten,
                                env_binding.type_annotation.as_ref(),
                            );
                            if let Some(LexicalBinding::Expr(existing)) = local_scope.get(env_binding.name.as_str()) {
                                if existing != &rewritten {
                                    local_scope.insert(env_binding.name.as_str().to_string(), LexicalBinding::Expr(rewritten));
                                    changed = true;
                                }
                            } else {
                                local_scope.insert(env_binding.name.as_str().to_string(), LexicalBinding::Expr(rewritten));
                                changed = true;
                            }
                        }
                    }
                }
            }
        }
    }

    // Process body items in declaration order
    let start_node_idx = doc.nodes.len();
    let start_local_bindings_idx = doc.local_bindings.len();
    let local_scope = RefCell::new(local_scope);
    for item in &comp_def.body {
        match item {
            ComponentBodyItem::Let(let_binding) => {
                let is_env_param_shadow = comp_def
                    .params
                    .iter()
                    .any(|p| p.is_env && p.name.as_str() == let_binding.name.as_str());

                if let_binding.value.is_none() || is_env_param_shadow {
                    // Shield/Swallower pattern or Firewall Tombstone
                    children_env_scope.insert(let_binding.name.as_str().to_string(), EnvEntry::Tombstone);
                    internal_body_env_scope.insert(let_binding.name.as_str().to_string(), EnvEntry::Tombstone);
                }

                if let Some(LetValue::Node(elem)) = &let_binding.value {
                    validate_node_type_annotation(
                        elem.name.as_str(),
                        elem.span,
                        let_binding.type_annotation.as_ref(),
                        ctx.enums,
                        registry,
                    )?;
                    let node_id = NodeId(doc.nodes.len());
                    local_scope.borrow_mut().insert(
                        let_binding.name.as_str().to_string(),
                        LexicalBinding::Node(node_id),
                    );
                    for entry in &mut consumer_child_entries {
                        for (k, v) in &children_env_scope {
                            entry.caller_env_scope.insert(k.clone(), v.clone());
                        }
                    }
                    let elem_ctx = ElementContext {
                        parent_id: Some(ctx.comp_node_id),
                        parent_name: Some(comp_def.name.as_str()),
                        prev_sibling_id: None,
                        has_next_sibling: false,
                        parent_ports: &internal_scope_ports,
                        lexical_scope: &local_scope,
                        env_scope: &internal_body_env_scope,
                        enclosing_component_id: Some(ctx.comp_node_id),
                        enclosing_num_children: num_consumer_children,
                        enclosing_instantiated_children: Some(&instantiated_children_ids),
                        shadowed_ports: Some(&comp_shadowed_ports),
                        is_let: true,
                        ambient_authored_ports: None,
                        enums: ctx.enums,
                        active_scope_id: ctx.active_scope_id,
                        consumer_children: Some(&consumer_child_entries),
                    };
                    let expanded_node_id = expand_element(
                        elem,
                        &elem_ctx,
                        registry,
                        doc,
                        node_fonts,
                    )?;
                    debug_assert_eq!(node_id, expanded_node_id);
                    doc.get_node_mut(node_id).unwrap().var_name = Some(let_binding.name.as_str().to_string());
                    all_children_ids.push(node_id);
                    if elem.name.as_str() == "Font" {
                        node_fonts.insert(node_id, node_id);
                    }
                } else if let Some(LetValue::Expr(raw_expr)) = &let_binding.value {
                    let is_param_shadow = comp_shadowed_ports.contains(let_binding.name.as_str());
                    if is_param_shadow {
                        let rewritten = {
                            let scope_borrow = local_scope.borrow();
                            let inst_borrow = instantiated_children_ids.borrow();
                            let scope_ctx = ScopeContext {
                                current_node: ctx.comp_node_id,
                                parent_node: ctx.parent_id,
                                parent_name: ctx.parent_name,
                                prev_sibling: ctx.prev_sibling_id,
                                has_next_sibling: ctx.has_next_sibling,
                                child_ids: &inst_borrow,
                                num_children: num_consumer_children,
                                child_node: None,
                                nodes: &doc.nodes,
                                shadowed_ports: Some(&comp_shadowed_ports),
                                is_event_port: false,
                                parent_ports: ctx.parent_ports,
                                current_ports: &comp_scope_ports,
                                lexical_scope: &*scope_borrow,
                                env_scope: &internal_body_env_scope,
                                node_fonts: &*node_fonts,
                                enclosing_component: Some(ctx.comp_node_id),
                                comp_ports: Some(&comp_ports),
                                declared_state_names: Some(&declared_state_names),
                                enums: ctx.enums,
                            };
                            rewrite_expr(raw_expr, &scope_ctx)?
                        };
                        let rewritten = coerce_typed_binding_expr(
                            rewritten,
                            let_binding.type_annotation.as_ref(),
                        );
                        doc.local_bindings.push((
                            rewritten.clone(),
                            let_binding.type_annotation.as_ref().map(|t| t.name.as_str().to_string()),
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
            }
            ComponentBodyItem::Env(env_binding) => {
                if let Some(raw_expr) = &env_binding.value {
                    match raw_expr {
                        Expr::Node(elem) => {
                            validate_node_type_annotation(
                                elem.name.as_str(),
                                elem.span,
                                env_binding.type_annotation.as_ref(),
                                ctx.enums,
                                registry,
                            )?;
                            for entry in &mut consumer_child_entries {
                                for (k, v) in &children_env_scope {
                                    entry.caller_env_scope.insert(k.clone(), v.clone());
                                }
                            }
                            let elem_ctx = ElementContext {
                                parent_id: Some(ctx.comp_node_id),
                                parent_name: Some(comp_def.name.as_str()),
                                prev_sibling_id: None,
                                has_next_sibling: false,
                                parent_ports: &internal_scope_ports,
                                lexical_scope: &local_scope,
                                env_scope: &internal_body_env_scope,
                                enclosing_component_id: Some(ctx.comp_node_id),
                                enclosing_num_children: num_consumer_children,
                                enclosing_instantiated_children: Some(&instantiated_children_ids),
                                shadowed_ports: Some(&comp_shadowed_ports),
                                is_let: false,
                                ambient_authored_ports: None,
                                enums: ctx.enums,
                                active_scope_id: ctx.active_scope_id,
                                consumer_children: Some(&consumer_child_entries),
                            };
                            let node_id = expand_element(
                                elem,
                                &elem_ctx,
                                registry,
                                doc,
                                node_fonts,
                            )?;
                            doc.get_node_mut(node_id).unwrap().var_name = Some(env_binding.name.as_str().to_string());
                            all_children_ids.push(node_id);
                            local_scope.borrow_mut().insert(
                                env_binding.name.as_str().to_string(),
                                LexicalBinding::Node(node_id),
                            );
                            let bound_expr = Expr::Ident(Ident::new(node_id.canonical_name(), elem.span));
                            children_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Bound(bound_expr.clone()));
                            internal_body_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Bound(bound_expr.clone()));
                            if !comp_shadowed_ports.contains(env_binding.name.as_str()) {
                                comp_ports.insert(env_binding.name.as_str().to_string(), bound_expr);
                            }
                            if elem.name.as_str() == "Font" {
                                node_fonts.insert(node_id, node_id);
                                if env_binding.name.as_str() == "font" {
                                    node_fonts.insert(ctx.comp_node_id, node_id);
                                    doc.get_node_mut(ctx.comp_node_id).unwrap().font = Some(node_id);
                                }
                            }
                        }
                        _ => {
                            let rewritten = {
                                let scope_borrow = local_scope.borrow();
                                let inst_borrow = instantiated_children_ids.borrow();
                                let scope_ctx = ScopeContext {
                                    current_node: ctx.comp_node_id,
                                    parent_node: ctx.parent_id,
                                    parent_name: ctx.parent_name,
                                    prev_sibling: ctx.prev_sibling_id,
                                    has_next_sibling: ctx.has_next_sibling,
                                    child_ids: &inst_borrow,
                                    num_children: num_consumer_children,
                                    child_node: None,
                                    nodes: &doc.nodes,
                                    shadowed_ports: Some(&comp_shadowed_ports),
                                    is_event_port: false,
                                    parent_ports: ctx.parent_ports,
                                    current_ports: &comp_scope_ports,
                                    lexical_scope: &*scope_borrow,
                                    env_scope: &internal_body_env_scope,
                                    node_fonts: &*node_fonts,
                                    enclosing_component: Some(ctx.comp_node_id),
                                    comp_ports: None,
                                    declared_state_names: None,
                                    enums: ctx.enums,
                                };
                                rewrite_expr(raw_expr, &scope_ctx)?
                            };
                            let rewritten = coerce_typed_binding_expr(
                                rewritten,
                                env_binding.type_annotation.as_ref(),
                            );
                            if comp_shadowed_ports.contains(env_binding.name.as_str()) {
                                doc.local_bindings.push((
                                    rewritten.clone(),
                                    env_binding.type_annotation.as_ref().map(|t| t.name.as_str().to_string()),
                                    raw_expr.span(),
                                ));
                            }
                            local_scope.borrow_mut().insert(
                                env_binding.name.as_str().to_string(),
                                LexicalBinding::Expr(rewritten.clone()),
                            );
                            children_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Bound(rewritten.clone()));
                            internal_body_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Bound(rewritten));
                        }
                    }
                } else {
                    // Environmental Hole Tombstone
                    local_scope.borrow_mut().insert(env_binding.name.as_str().to_string(), LexicalBinding::Uninitialized);
                    children_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Tombstone);
                    internal_body_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Tombstone);
                }
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
                    parent_ports: &internal_scope_ports,
                    lexical_scope: &local_scope,
                    env_scope: &internal_body_env_scope,
                    enclosing_component_id: Some(ctx.comp_node_id),
                    enclosing_num_children: num_consumer_children,
                    enclosing_instantiated_children: Some(&instantiated_children_ids),
                    shadowed_ports: Some(&comp_shadowed_ports),
                    is_let: false,
                    ambient_authored_ports: None,
                    enums: ctx.enums,
                    active_scope_id: ctx.active_scope_id,
                    consumer_children: Some(&consumer_child_entries),
                };
                let body_id = expand_element(
                    body_node,
                    &elem_ctx,
                    registry,
                    doc,
                    node_fonts,
                )?;
                all_children_ids.push(body_id);
                last_child_id = Some(body_id);
            }
            ComponentBodyItem::Children(dir) => {
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
                                            registry,
                                        )?;
                                        let elem_ctx = ElementContext {
                                            parent_id: Some(ctx.comp_node_id),
                                            parent_name: Some(comp_def.name.as_str()),
                                            prev_sibling_id: None,
                                            has_next_sibling: false,
                                            parent_ports: &comp_scope_ports,
                                            lexical_scope,
                                            env_scope: &children_env_scope,
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
                                        let env_node_id = expand_element(
                                            elem,
                                            &elem_ctx,
                                            registry,
                                            doc,
                                            node_fonts,
                                        )?;
                                        doc.get_node_mut(env_node_id).unwrap().var_name = Some(env_binding.name.as_str().to_string());
                                        all_children_ids.push(env_node_id);
                                        lexical_scope.borrow_mut().insert(
                                            env_binding.name.as_str().to_string(),
                                            LexicalBinding::Node(env_node_id),
                                        );
                                        let bound_expr = Expr::Ident(Ident::new(env_node_id.canonical_name(), elem.span));
                                        children_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Bound(bound_expr));
                                        if elem.name.as_str() == "Font" {
                                            node_fonts.insert(env_node_id, env_node_id);
                                        }
                                    }
                                    _ => {
                                        let rewritten = {
                                            let scope_borrow = lexical_scope.borrow();
                                            let empty_children: [NodeId; 0] = [];
                                            let empty_ports: [String; 0] = [];
                                            let caller_inst_borrow = ctx.enclosing_instantiated_children.map(|c| c.borrow());
                                            let caller_child_ids: &[NodeId] = match &caller_inst_borrow {
                                                Some(b) => b.as_slice(),
                                                None => &empty_children,
                                            };
                                            let scope_ctx = ScopeContext {
                                                current_node: ctx.comp_node_id,
                                                parent_node: ctx.parent_id,
                                                parent_name: ctx.parent_name,
                                                prev_sibling: ctx.prev_sibling_id,
                                                has_next_sibling: ctx.has_next_sibling,
                                                child_ids: caller_child_ids,
                                                num_children: ctx.enclosing_num_children,
                                                child_node: None,
                                                nodes: &doc.nodes,
                                                shadowed_ports: ctx.shadowed_ports,
                                                is_event_port: false,
                                                parent_ports: ctx.parent_ports,
                                                current_ports: &empty_ports,
                                                lexical_scope: &*scope_borrow,
                                                env_scope: &children_env_scope,
                                                node_fonts: &*node_fonts,
                                                enclosing_component: ctx.enclosing_component_id,
                                                comp_ports: None,
                                                declared_state_names: None,
                                                enums: ctx.enums,
                                            };
                                            rewrite_expr(raw_expr, &scope_ctx)?
                                        };
                                        let rewritten = coerce_typed_binding_expr(
                                            rewritten,
                                            env_binding.type_annotation.as_ref(),
                                        );
                                        doc.local_bindings.push((
                                            rewritten.clone(),
                                            env_binding.type_annotation.as_ref().map(|t| t.name.as_str().to_string()),
                                            raw_expr.span(),
                                        ));
                                        lexical_scope.borrow_mut().insert(
                                            env_binding.name.as_str().to_string(),
                                            LexicalBinding::Expr(rewritten.clone()),
                                        );
                                        children_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Bound(rewritten));
                                    }
                                }
                            } else {
                                lexical_scope.borrow_mut().insert(env_binding.name.as_str().to_string(), LexicalBinding::Uninitialized);
                                children_env_scope.insert(env_binding.name.as_str().to_string(), EnvEntry::Tombstone);
                            }
                        }
                    }
                }
                let mut last_consumer_child_id = None;
                let mut dir_children_ids = Vec::with_capacity(num_consumer_children);
                for (child_idx, entry) in consumer_child_entries.iter().enumerate() {
                    let has_next_consumer_child = child_idx + 1 < num_consumer_children;
                    let child_elem = &entry.elem;
                    let var_name = &entry.var_name;
                    let mut merged_ports = HashMap::new();
                    let mut authored_ambient_ports = HashMap::new();

                    let mut entry_env_scope = entry.caller_env_scope.clone();
                    for (k, v) in &children_env_scope {
                        entry_env_scope.insert(k.clone(), v.clone());
                    }

                    // Ambient rules from \Children are authored inside the component (Tier 3)
                    let child_node_id = NodeId(doc.nodes.len());
                    let empty_children: [NodeId; 0] = [];
                    let empty_ports: [String; 0] = [];
                    {
                        let local_borrow = local_scope.borrow();
                        let inst_borrow = instantiated_children_ids.borrow();
                        let mut ambient_scope_ctx = ScopeContext {
                            current_node: child_node_id,
                            parent_node: Some(ctx.comp_node_id),
                            parent_name: Some(comp_def.name.as_str()),
                            prev_sibling: last_consumer_child_id,
                            has_next_sibling: has_next_consumer_child,
                            child_ids: &inst_borrow,
                            num_children: num_consumer_children,
                            child_node: Some(child_node_id),
                            nodes: &doc.nodes,
                            shadowed_ports: Some(&comp_shadowed_ports),
                            is_event_port: false,
                            parent_ports: &internal_scope_ports,
                            current_ports: &empty_ports,
                            lexical_scope: &*local_borrow,
                            env_scope: &entry_env_scope,
                            node_fonts: &*node_fonts,
                            enclosing_component: Some(ctx.comp_node_id),
                            comp_ports: Some(&comp_ports),
                            declared_state_names: Some(&declared_state_names),
                            enums: ctx.enums,
                        };

                        for ambient in &dir.ports {
                            ambient_scope_ctx.is_event_port = ambient.name.as_str().starts_with("on_");
                            let rewritten = rewrite_expr(&ambient.expr, &ambient_scope_ctx)?;
                            merged_ports.insert(ambient.name.as_str().to_string(), rewritten);
                            authored_ambient_ports.insert(ambient.name.as_str().to_string(), ambient.expr.clone());
                        }
                    }

                    // Forwarded \Children(...) directives from outer component wrappers
                    for fwd in &entry.forwarded_directives {
                        let fwd_lexical_borrow = fwd.lexical_scope.borrow();
                        let fwd_inst_borrow = fwd.enclosing_instantiated_children.map(|c| c.borrow());
                        let fwd_child_ids: &[NodeId] = match &fwd_inst_borrow {
                            Some(b) => b.as_slice(),
                            None => &empty_children,
                        };
                        let mut fwd_scope_ctx = ScopeContext {
                            current_node: child_node_id,
                            parent_node: Some(fwd.parent_id),
                            parent_name: Some(fwd.parent_name),
                            prev_sibling: last_consumer_child_id,
                            has_next_sibling: has_next_consumer_child,
                            child_ids: fwd_child_ids,
                            num_children: fwd.enclosing_num_children,
                            child_node: Some(child_node_id),
                            nodes: &doc.nodes,
                            shadowed_ports: fwd.shadowed_ports,
                            is_event_port: false,
                            parent_ports: &fwd.parent_ports,
                            current_ports: &empty_ports,
                            lexical_scope: &*fwd_lexical_borrow,
                            env_scope: &fwd.env_scope,
                            node_fonts: &*node_fonts,
                            enclosing_component: fwd.enclosing_component_id,
                            comp_ports: fwd.comp_ports,
                            declared_state_names: fwd.declared_state_names,
                            enums: ctx.enums,
                        };
                        for ambient in fwd.ports {
                            fwd_scope_ctx.is_event_port = ambient.name.as_str().starts_with("on_");
                            let rewritten = rewrite_expr(&ambient.expr, &fwd_scope_ctx)?;
                            merged_ports.insert(ambient.name.as_str().to_string(), rewritten);
                            authored_ambient_ports.insert(ambient.name.as_str().to_string(), ambient.expr.clone());
                        }
                    }

                    // Explicit child ports override ambient rules (Tier 4 > Tier 3)
                    for explicit in &child_elem.ports {
                        merged_ports.insert(explicit.name.as_str().to_string(), explicit.expr.clone());
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
                        parent_ports: &comp_scope_ports,
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
                        registry,
                        doc,
                        node_fonts,
                    )?;

                    if let Some(v_name) = var_name {
                        doc.get_node_mut(child_id).unwrap().var_name = Some(v_name.clone());
                        entry.caller_lexical_scope.borrow_mut().insert(v_name.clone(), LexicalBinding::Node(child_id));
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
            ComponentBodyItem::Alias(_) => {}
            ComponentBodyItem::State(_) => {}
            ComponentBodyItem::Text(_) => {}
            ComponentBodyItem::Assert(_) => {}
        }
    }

    let mut final_local_scope = local_scope.into_inner();
    let instantiated_children_ids = instantiated_children_ids.into_inner();
    for (idx, &child_id) in instantiated_children_ids.iter().enumerate() {
        final_local_scope.insert(
            format!("__child_of_{}_{}", ctx.comp_node_id.canonical_name(), idx),
            LexicalBinding::Node(child_id),
        );
    }

    // Post-expansion forward reference resolution for all nodes created within this component
    for node_idx in start_node_idx..doc.nodes.len() {
        let node = &mut doc.nodes[node_idx];
        for expr in node.ports.values_mut() {
            *expr = resolve_forward_scope_refs(expr, &final_local_scope);
        }
    }
    for binding_idx in start_local_bindings_idx..doc.local_bindings.len() {
        let (expr, _, _) = &mut doc.local_bindings[binding_idx];
        *expr = resolve_forward_scope_refs(expr, &final_local_scope);
    }

    // 6. Rewrite component ports in scope
    let scope_ctx = ScopeContext {
        current_node: ctx.comp_node_id,
        parent_node: ctx.parent_id,
        parent_name: ctx.parent_name,
        prev_sibling: ctx.prev_sibling_id,
        has_next_sibling: ctx.has_next_sibling,
        child_ids: &instantiated_children_ids,
        num_children: num_consumer_children,
        child_node: None,
        nodes: &doc.nodes,
        shadowed_ports: Some(&comp_shadowed_ports),
        is_event_port: false,
        parent_ports: ctx.parent_ports,
        current_ports: &comp_scope_ports,
        lexical_scope: &final_local_scope,
        env_scope: &internal_body_env_scope,
        node_fonts: &*node_fonts,
        enclosing_component: Some(ctx.comp_node_id),
        comp_ports: Some(&comp_ports),
        declared_state_names: Some(&declared_state_names),
        enums: ctx.enums,
    };

    let mut rewritten_ports = HashMap::new();
    for (port_name, port_expr) in &comp_ports {
        if already_rewritten_ports.contains(port_name) {
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
            if let Some(l) = declared_lets.get(port_name) {
                rw = coerce_typed_binding_expr(rw, l.type_annotation.as_ref());
            } else if let Some(e) = declared_envs.get(port_name) {
                rw = coerce_typed_binding_expr(rw, e.type_annotation.as_ref());
            } else if let Some(s) = declared_states.get(port_name) {
                rw = coerce_typed_binding_expr(rw, s.type_annotation.as_ref());
            }
            rewritten_ports.insert(port_name.clone(), rw);
        }
    }

    // Validate parameter types and coerce 0 -> 0px (for Distance) or Integer -> Float (for Float)
    for param in &comp_def.params {
        if let Some(type_ref) = &param.type_annotation {
            let expected_type = type_ref.name.as_str();
            let name = param.name.as_str();
            let unlowered_lit = explicit_unlowered_ports
                .get(name)
                .or(param.default_edge.as_ref())
                .and_then(|e| match e {
                    Expr::Literal(lit) => Some(lit.clone()),
                    _ => None,
                });
            if let Some(expr) = rewritten_ports.get_mut(name) {
                if let Some(lit) = unlowered_lit.or_else(|| infer_expr_literal_for_typecheck(expr, &scope_ctx)) {
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
        doc.window_ports
            .get("dpi")
            .and_then(|e| eval_expr(e, &HashMap::new()).ok())
            .unwrap_or(crate::compiler::value::Value::Float(96.0)),
    );
    for n in &doc.nodes {
        if n.name == "Font" {
            if let Some(size_expr) = n.ports.get("size") {
                if let Ok(val) = eval_expr(size_expr, &const_env) {
                    const_env.insert(crate::compiler::graph::VarId::new(n.id, "size"), val);
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
                Literal::Enum(e, v, _) => crate::compiler::value::Value::Enum { enum_name: e, variant: v },
            };
            const_env.insert(crate::compiler::graph::VarId::new(ctx.comp_node_id, port_name), val.clone());
            if let Some(parent) = ctx.parent_id {
                const_env.insert(crate::compiler::graph::VarId::new(parent, port_name), val);
            }
        } else if let Ok(val) = eval_expr(port_expr, &const_env) {
            const_env.insert(crate::compiler::graph::VarId::new(ctx.comp_node_id, port_name), val.clone());
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
                        let msg = assert_dir
                            .message
                            .clone()
                            .unwrap_or_else(|| format!("Assertion failed: {}", assert_dir.condition));
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

    let mut comp_authored_ports = HashMap::new();
    if let Some(ambient_map) = ctx.ambient_authored_ports {
        for (k, v) in ambient_map {
            comp_authored_ports.insert(k.clone(), v.clone());
        }
    }
    for param in &comp_def.params {
        let name = param.name.as_str().to_string();
        if let Some(default_expr) = &param.default_edge {
            comp_authored_ports.entry(name).or_insert_with(|| default_expr.clone());
        }
    }
    for (state_name, s) in &declared_states {
        let default_expr = default_expr_for_state_with_enums(s, ctx.enums);
        comp_authored_ports.insert(state_name.clone(), default_expr);
    }
    for (let_name, l) in &declared_lets {
        if !comp_shadowed_ports.contains(let_name) {
            if let Some(LetValue::Expr(raw_expr)) = &l.value {
                comp_authored_ports.insert(let_name.clone(), raw_expr.clone());
            }
        }
    }
    for (env_name, e) in &declared_envs {
        if let Some(raw_expr) = &e.value {
            comp_authored_ports.insert(env_name.clone(), raw_expr.clone());
        }
    }
    for port in &instance.ports {
        if let Some(ambient_expr) = ctx.ambient_authored_ports.and_then(|m| m.get(port.name.as_str())) {
            comp_authored_ports.insert(port.name.as_str().to_string(), ambient_expr.clone());
        } else {
            comp_authored_ports.insert(port.name.as_str().to_string(), port.expr.clone());
        }
    }

    // 7. Process explicitly declared component aliases
    let alias_scope_ctx = ScopeContext {
        current_node: ctx.comp_node_id,
        parent_node: ctx.parent_id,
        parent_name: ctx.parent_name,
        prev_sibling: ctx.prev_sibling_id,
        has_next_sibling: ctx.has_next_sibling,
        child_ids: &instantiated_children_ids,
        num_children: num_consumer_children,
        child_node: None,
        nodes: &doc.nodes,
        shadowed_ports: Some(&comp_shadowed_ports),
        is_event_port: false,
        parent_ports: ctx.parent_ports,
        current_ports: &comp_scope_ports,
        lexical_scope: &final_local_scope,
        env_scope: &internal_body_env_scope,
        node_fonts: &*node_fonts,
        enclosing_component: Some(ctx.comp_node_id),
        comp_ports: Some(&comp_ports),
        declared_state_names: Some(&declared_state_names),
        enums: ctx.enums,
    };
    for (alias_name, alias_binding) in &declared_aliases {
        let mut rewritten_alias = rewrite_expr(&alias_binding.value, &alias_scope_ctx)?;
        rewritten_alias = resolve_forward_scope_refs(&rewritten_alias, &final_local_scope);
        rewritten_alias = coerce_typed_binding_expr(rewritten_alias, alias_binding.type_annotation.as_ref());
        rewritten_ports.insert(alias_name.clone(), rewritten_alias);
        comp_authored_ports.insert(alias_name.clone(), alias_binding.value.clone());
    }

    // 8. Default spatial aliases if their inputs are present and no alias was explicitly declared
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
        comp_authored_ports.insert("left".to_string(), Expr::Ident(Ident::new("x", instance.span)));
    }
    if !rewritten_ports.contains_key("top") && rewritten_ports.contains_key("y") {
        rewritten_ports.insert("top".to_string(), self_y.clone());
        comp_authored_ports.insert("top".to_string(), Expr::Ident(Ident::new("y", instance.span)));
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

    let mut node_port_types = HashMap::new();
    node_port_types.insert("clip".to_string(), "Clip".to_string());
    for sp in ["left", "top", "right", "bottom"] {
        if rewritten_ports.contains_key(sp) {
            node_port_types.insert(sp.to_string(), "Distance".to_string());
        }
    }
    for param in &comp_def.params {
        if let Some(ty) = &param.type_annotation {
            node_port_types.insert(param.name.as_str().to_string(), ty.name.as_str().to_string());
        }
    }
    for (state_name, s) in &declared_states {
        if let Some(ty) = &s.type_annotation {
            node_port_types.insert(state_name.clone(), ty.name.as_str().to_string());
        }
    }
    for (let_name, l) in &declared_lets {
        if !comp_shadowed_ports.contains(let_name) {
            if let Some(ty) = &l.type_annotation {
                node_port_types.insert(let_name.clone(), ty.name.as_str().to_string());
            }
        }
    }
    for (env_name, e) in &declared_envs {
        if !comp_shadowed_ports.contains(env_name) {
            if let Some(ty) = &e.type_annotation {
                node_port_types.insert(env_name.clone(), ty.name.as_str().to_string());
            }
        }
    }
    for (alias_name, a) in &declared_aliases {
        if let Some(ty) = &a.type_annotation {
            node_port_types.insert(alias_name.clone(), ty.name.as_str().to_string());
        }
    }

    let node = doc.get_node_mut(ctx.comp_node_id).unwrap();
    node.is_component = true;
    node.overload_params = comp_def
        .params
        .iter()
        .filter(|p| p.default_edge.is_none())
        .map(|p| p.name.as_str().to_string())
        .collect();
    node.children = all_children_ids;
    node.authored_ports = comp_authored_ports;
    node.private_ports = declared_lets
        .keys()
        .filter(|k| !comp_def.params.iter().any(|p| p.name.as_str() == k.as_str()))
        .cloned()
        .collect();
    for (state_name, s) in &declared_states {
        let type_name = s.type_annotation.as_ref().map(|t| t.name.as_str().to_string());
        node.state_vars.insert(state_name.clone(), type_name);
    }
    for (env_name, e) in &declared_envs {
        let type_name = e.type_annotation.as_ref().map(|t| t.name.as_str().to_string());
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
    node.port_types = node_port_types;
    node.ports = rewritten_ports;

    Ok(())
}

/// Expands a primitive element (e.g. \Rect, \Text, \Font, \Header, etc.)
fn expand_primitive_element(
    node_id: NodeId,
    elem: &ElementNode,
    ctx: &ElementContext<'_>,
    registry: &HashMap<String, Vec<ComponentDef>>,
    doc: &mut ExpandedDocument,
    node_fonts: &mut HashMap<NodeId, NodeId>,
) -> Result<(), CompileError> {
    let is_inline_span = ctx
        .parent_id
        .is_some_and(|p| doc.get_node(p).is_some_and(|n| n.name == "Text"));

    if !matches!(
        elem.name.as_str(),
        "Rect" | "Line" | "Text" | "Font" | "Clip" | "Box" | "Anchor" | "AnchorScope"
    ) && !(is_inline_span && elem.name.as_str() == "Link")
    {
        return Err(CompileError::UndefinedComponent {
            name: elem.name.as_str().to_string(),
            span: elem.name.span,
        });
    }

    // Validate that caller does not attempt to override immutable spatial alias ports
    for port in &elem.ports {
        let is_purely_ambient = ctx.ambient_authored_ports.is_some_and(|a| a.contains_key(port.name.as_str()));
        if is_purely_ambient {
            continue;
        }
        if matches!(port.name.as_str(), "left" | "top" | "right" | "bottom") {
            return Err(CompileError::ImmutableAliasPort {
                node: elem.name.as_str().to_string(),
                port: port.name.as_str().to_string(),
                span: port.name.span,
            });
        }
    }

    // Handle Navigation Scopes and Anchors
    let mut active_scope_for_children = ctx.active_scope_id;

    if elem.name.as_str() == "AnchorScope" {
        let scope_name = elem.ports.iter().find(|p| p.name.as_str() == "name").and_then(|p| match &p.expr {
            Expr::Literal(Literal::String(s, _)) => Some(s.clone()),
            Expr::Ident(id) => Some(id.as_str().to_string()),
            _ => None,
        });

        let name_str = match scope_name {
            Some(n) => n,
            None => {
                return Err(CompileError::MissingPort {
                    node: "AnchorScope".to_string(),
                    port: "name".to_string(),
                    span: elem.span,
                });
            }
        };

        let new_scope_id = doc.scope_tree.add_scope(
            &name_str,
            ctx.active_scope_id,
            Some(node_id),
            elem.span,
        )?;
        doc.nodes[node_id.0].scope_id = Some(new_scope_id);
        doc.scope_tree.node_to_scope.insert(node_id, new_scope_id);
        active_scope_for_children = new_scope_id;
    } else if elem.name.as_str() == "Anchor" {
        let anchor_name = elem.ports.iter().find(|p| p.name.as_str() == "name").and_then(|p| match &p.expr {
            Expr::Literal(Literal::String(s, _)) => Some(s.clone()),
            Expr::Ident(id) => Some(id.as_str().to_string()),
            _ => None,
        });

        let name_str = match anchor_name {
            Some(n) => n,
            None => {
                return Err(CompileError::MissingPort {
                    node: "Anchor".to_string(),
                    port: "name".to_string(),
                    span: elem.span,
                });
            }
        };

        doc.scope_tree.register_anchor(
            ctx.active_scope_id,
            &name_str,
            node_id,
            elem.span,
        )?;
        doc.nodes[node_id.0].anchor_name = Some(name_str);
    }

    // 1. Expand nested child nodes in content slot
    let mut child_ids = Vec::new();
    let mut last_child_id = None;
    let mut full_text = String::new();
    let mut text_spans = Vec::new();

    let mut primitive_scope_ports = if elem.name.as_str() == "Font" {
        vec![
            "family".to_string(),
            "size".to_string(),
            "weight".to_string(),
            "line_height".to_string(),
            "cap_height".to_string(),
            "x_height".to_string(),
            "descent".to_string(),
            "ascent".to_string(),
            "em_top".to_string(),
            "line_gap".to_string(),
            "gap_bottom".to_string(),
        ]
    } else {
        vec![
            "x".to_string(),
            "y".to_string(),
            "width".to_string(),
            "height".to_string(),
            "z".to_string(),
            "left".to_string(),
            "top".to_string(),
            "right".to_string(),
            "bottom".to_string(),
            "clip".to_string(),
            "name".to_string(),
            "path".to_string(),
        ]
    };
    for p in &elem.ports {
        let pname = p.name.as_str().to_string();
        if !primitive_scope_ports.contains(&pname) {
            primitive_scope_ports.push(pname);
        }
    }

    if let Some(slot) = &elem.content {
        for item in &slot.items {
            match item {
                ContentItem::Text(chunk) => {
                    if elem.name.as_str() == "Text" {
                        let start = full_text.len();
                        full_text.push_str(&chunk.text);
                        let end = full_text.len();
                        text_spans.push(TextSpan::new(start..end));
                    }
                }
                ContentItem::Node(child_elem) => {
                    let child_ctx = ElementContext {
                        parent_id: Some(node_id),
                        parent_name: Some(elem.name.as_str()),
                        prev_sibling_id: last_child_id,
                        has_next_sibling: false,
                        parent_ports: &primitive_scope_ports,
                        lexical_scope: ctx.lexical_scope,
                        env_scope: ctx.env_scope,
                        enclosing_component_id: ctx.enclosing_component_id,
                        enclosing_num_children: ctx.enclosing_num_children,
                        enclosing_instantiated_children: ctx.enclosing_instantiated_children,
                        shadowed_ports: ctx.shadowed_ports,
                        is_let: false,
                        ambient_authored_ports: None,
                        enums: ctx.enums,
                        active_scope_id: active_scope_for_children,
                        consumer_children: ctx.consumer_children,
                    };

                    let is_wrapper = elem.name.as_str() == "Anchor" || elem.name.as_str() == "AnchorScope";
                    let wired_child_elem;
                    let target_elem = if is_wrapper {
                        let mut child = child_elem.clone();
                        let has_x = child.ports.iter().any(|p| p.name.as_str() == "x");
                        let has_y = child.ports.iter().any(|p| p.name.as_str() == "y");

                        if !has_x {
                            child.ports.push(PortBinding {
                                name: Ident::new("x", child.span),
                                expr: Expr::MemberAccess(MemberAccessExpr {
                                    target: Box::new(Expr::Ident(Ident::new(node_id.canonical_name(), child.span))),
                                    member: Ident::new("left", child.span),
                                    span: child.span,
                                }),
                                span: child.span,
                            });
                        }
                        if !has_y {
                            child.ports.push(PortBinding {
                                name: Ident::new("y", child.span),
                                expr: Expr::MemberAccess(MemberAccessExpr {
                                    target: Box::new(Expr::Ident(Ident::new(node_id.canonical_name(), child.span))),
                                    member: Ident::new("top", child.span),
                                    span: child.span,
                                }),
                                span: child.span,
                            });
                        }
                        wired_child_elem = child;
                        &wired_child_elem
                    } else {
                        child_elem
                    };

                    let child_id = expand_element(
                        target_elem,
                        &child_ctx,
                        registry,
                        doc,
                        node_fonts,
                    )?;
                    child_ids.push(child_id);
                    last_child_id = Some(child_id);

                    if elem.name.as_str() == "Text" {
                        // Extract text from the expanded child node
                        let child_text = doc.nodes[child_id.0]
                            .text_content
                            .clone()
                            .or_else(|| {
                                if let Some(c_slot) = &child_elem.content {
                                    let mut parts = Vec::new();
                                    for it in &c_slot.items {
                                        if let ContentItem::Text(t) = it {
                                            parts.push(t.text.as_str());
                                        }
                                    }
                                    if !parts.is_empty() {
                                        return Some(parts.join(" "));
                                    }
                                }
                                None
                            })
                            .unwrap_or_default();

                        let start = full_text.len();
                        full_text.push_str(&child_text);
                        let end = full_text.len();

                        let is_link = child_elem.name.as_str() == "Link"
                            || child_elem.ports.iter().any(|p| p.name.as_str() == "url");

                        let mut style = SpanStyle::default();
                        if is_link {
                            let mut url_val = None;
                            let mut color_val = Some("#1a73e8".to_string());
                            let mut underline_val = true;

                            for p in &child_elem.ports {
                                match p.name.as_str() {
                                    "url" => {
                                        if let Expr::Literal(Literal::String(s, _)) = &p.expr {
                                            url_val = Some(s.clone());
                                        }
                                    }
                                    "color" => {
                                        if let Expr::Literal(Literal::Color(c, _))
                                        | Expr::Literal(Literal::String(c, _)) = &p.expr
                                        {
                                            color_val = Some(c.clone());
                                        }
                                    }
                                    "underline" => {
                                        if let Expr::Literal(Literal::Bool(b, _)) = &p.expr {
                                            underline_val = *b;
                                        }
                                    }
                                    _ => {}
                                }
                            }

                            style.url = url_val;
                            style.color = color_val;
                            style.underline = underline_val;
                            style.cursor = Some(CursorKind::Pointer);
                        }

                        text_spans.push(TextSpan {
                            range: start..end,
                            node_id: Some(child_id),
                            style,
                        });
                    }
                }
                ContentItem::Let(l) => {
                    if let Some(LetValue::Node(child_elem)) = &l.value {
                        validate_node_type_annotation(
                            child_elem.name.as_str(),
                            child_elem.span,
                            l.type_annotation.as_ref(),
                            ctx.enums,
                            registry,
                        )?;
                        let child_ctx = ElementContext {
                            parent_id: Some(node_id),
                            parent_name: Some(elem.name.as_str()),
                            prev_sibling_id: last_child_id,
                            has_next_sibling: false,
                            parent_ports: &primitive_scope_ports,
                            lexical_scope: ctx.lexical_scope,
                            env_scope: ctx.env_scope,
                            enclosing_component_id: ctx.enclosing_component_id,
                            enclosing_num_children: ctx.enclosing_num_children,
                            enclosing_instantiated_children: ctx.enclosing_instantiated_children,
                            shadowed_ports: ctx.shadowed_ports,
                            is_let: true,
                            ambient_authored_ports: None,
                            enums: ctx.enums,
                            active_scope_id: active_scope_for_children,
                            consumer_children: ctx.consumer_children,
                        };

                        let is_wrapper = elem.name.as_str() == "Anchor" || elem.name.as_str() == "AnchorScope";
                        let wired_child_elem;
                        let target_elem = if is_wrapper {
                            let mut child = child_elem.clone();
                            let has_x = child.ports.iter().any(|p| p.name.as_str() == "x");
                            let has_y = child.ports.iter().any(|p| p.name.as_str() == "y");

                            if !has_x {
                                child.ports.push(PortBinding {
                                    name: Ident::new("x", child.span),
                                    expr: Expr::MemberAccess(MemberAccessExpr {
                                        target: Box::new(Expr::Ident(Ident::new(node_id.canonical_name(), child.span))),
                                        member: Ident::new("left", child.span),
                                        span: child.span,
                                    }),
                                    span: child.span,
                                });
                            }
                            if !has_y {
                                child.ports.push(PortBinding {
                                    name: Ident::new("y", child.span),
                                    expr: Expr::MemberAccess(MemberAccessExpr {
                                        target: Box::new(Expr::Ident(Ident::new(node_id.canonical_name(), child.span))),
                                        member: Ident::new("top", child.span),
                                        span: child.span,
                                    }),
                                    span: child.span,
                                });
                            }
                            wired_child_elem = child;
                            &wired_child_elem
                        } else {
                            child_elem
                        };

                        let child_id = expand_element(
                            target_elem,
                            &child_ctx,
                            registry,
                            doc,
                            node_fonts,
                        )?;
                        doc.get_node_mut(child_id).unwrap().var_name = Some(l.name.as_str().to_string());
                        ctx.lexical_scope.borrow_mut().insert(l.name.as_str().to_string(), LexicalBinding::Node(child_id));
                        child_ids.push(child_id);
                        last_child_id = Some(child_id);

                        if child_elem.name.as_str() == "Font" {
                            node_fonts.insert(child_id, child_id);
                        }

                        if elem.name.as_str() == "Text" {
                            let child_text = doc.nodes[child_id.0]
                                .text_content
                                .clone()
                                .or_else(|| {
                                    if let Some(c_slot) = &child_elem.content {
                                        let mut parts = Vec::new();
                                        for it in &c_slot.items {
                                            if let ContentItem::Text(t) = it {
                                                parts.push(t.text.as_str());
                                            }
                                        }
                                        if !parts.is_empty() {
                                            return Some(parts.join(" "));
                                        }
                                    }
                                    None
                                })
                                .unwrap_or_default();

                            let start = full_text.len();
                            full_text.push_str(&child_text);
                            let end = full_text.len();

                            let is_link = child_elem.name.as_str() == "Link"
                                || child_elem.ports.iter().any(|p| p.name.as_str() == "url");

                            let mut style = SpanStyle::default();
                            if is_link {
                                let mut url_val = None;
                                let mut color_val = Some("#1a73e8".to_string());
                                let mut underline_val = true;

                                for p in &child_elem.ports {
                                    match p.name.as_str() {
                                        "url" => {
                                            if let Expr::Literal(Literal::String(s, _)) = &p.expr {
                                                url_val = Some(s.clone());
                                            }
                                        }
                                        "color" => {
                                            if let Expr::Literal(Literal::Color(c, _))
                                            | Expr::Literal(Literal::String(c, _)) = &p.expr
                                            {
                                                color_val = Some(c.clone());
                                            }
                                        }
                                        "underline" => {
                                            if let Expr::Literal(Literal::Bool(b, _)) = &p.expr {
                                                underline_val = *b;
                                            }
                                        }
                                        _ => {}
                                    }
                                }

                                style.url = url_val;
                                style.color = color_val;
                                style.underline = underline_val;
                                style.cursor = Some(CursorKind::Pointer);
                            }

                            text_spans.push(TextSpan {
                                range: start..end,
                                node_id: Some(child_id),
                                style,
                            });
                        }
                    }
                }
                ContentItem::Alias(_) | ContentItem::State(_) | ContentItem::Env(_) | ContentItem::Assert(_) => {}
                ContentItem::Children(dir) => {
                    for port in &dir.ports {
                        if port.name.as_str() == "parent" {
                            return Err(CompileError::ReservedPort {
                                node: "Children".to_string(),
                                port: "parent".to_string(),
                                span: port.name.span,
                            });
                        }
                    }
                    if let Some(consumer_items) = ctx.consumer_children {
                        let num_consumer_children = consumer_items.len();
                        let empty_children: [NodeId; 0] = [];
                        let empty_ports: [String; 0] = [];
                        for (child_idx, entry) in consumer_items.iter().enumerate() {
                            if let Some(chunk) = &entry.raw_text {
                                if elem.name.as_str() == "Text"
                                    && dir.ports.is_empty()
                                    && entry.forwarded_directives.is_empty()
                                {
                                    let start = full_text.len();
                                    full_text.push_str(&chunk.text);
                                    let end = full_text.len();
                                    text_spans.push(TextSpan::new(start..end));
                                    continue;
                                }
                            }

                            let has_next_sibling = child_idx + 1 < num_consumer_children;
                            let child_elem = &entry.elem;
                            let mut merged_ports = HashMap::new();
                            let mut authored_ambient_ports = HashMap::new();
                            let child_node_id = NodeId(doc.nodes.len());

                            {
                                let lexical_borrow = ctx.lexical_scope.borrow();
                                let inst_borrow = ctx.enclosing_instantiated_children.map(|c| c.borrow());
                                let enc_child_ids: &[NodeId] = match &inst_borrow {
                                    Some(b) => b.as_slice(),
                                    None => &empty_children,
                                };
                                let mut ambient_scope_ctx = ScopeContext {
                                    current_node: child_node_id,
                                    parent_node: Some(node_id),
                                    parent_name: Some(elem.name.as_str()),
                                    prev_sibling: last_child_id,
                                    has_next_sibling,
                                    child_ids: enc_child_ids,
                                    num_children: ctx.enclosing_num_children,
                                    child_node: Some(child_node_id),
                                    nodes: &doc.nodes,
                                    shadowed_ports: ctx.shadowed_ports,
                                    is_event_port: false,
                                    parent_ports: &primitive_scope_ports,
                                    current_ports: &empty_ports,
                                    lexical_scope: &*lexical_borrow,
                                    env_scope: ctx.env_scope,
                                    node_fonts: &*node_fonts,
                                    enclosing_component: ctx.enclosing_component_id,
                                    comp_ports: None,
                                    declared_state_names: None,
                                    enums: ctx.enums,
                                };
                                for ambient in &dir.ports {
                                    ambient_scope_ctx.is_event_port = ambient.name.as_str().starts_with("on_");
                                    let rewritten = rewrite_expr(&ambient.expr, &ambient_scope_ctx)?;
                                    merged_ports.insert(ambient.name.as_str().to_string(), rewritten);
                                    authored_ambient_ports.insert(ambient.name.as_str().to_string(), ambient.expr.clone());
                                }
                            }

                            for fwd in &entry.forwarded_directives {
                                let fwd_lexical_borrow = fwd.lexical_scope.borrow();
                                let fwd_inst_borrow = fwd.enclosing_instantiated_children.map(|c| c.borrow());
                                let fwd_child_ids: &[NodeId] = match &fwd_inst_borrow {
                                    Some(b) => b.as_slice(),
                                    None => &empty_children,
                                };
                                let mut fwd_scope_ctx = ScopeContext {
                                    current_node: child_node_id,
                                    parent_node: Some(fwd.parent_id),
                                    parent_name: Some(fwd.parent_name),
                                    prev_sibling: last_child_id,
                                    has_next_sibling,
                                    child_ids: fwd_child_ids,
                                    num_children: fwd.enclosing_num_children,
                                    child_node: Some(child_node_id),
                                    nodes: &doc.nodes,
                                    shadowed_ports: fwd.shadowed_ports,
                                    is_event_port: false,
                                    parent_ports: &fwd.parent_ports,
                                    current_ports: &empty_ports,
                                    lexical_scope: &*fwd_lexical_borrow,
                                    env_scope: &fwd.env_scope,
                                    node_fonts: &*node_fonts,
                                    enclosing_component: fwd.enclosing_component_id,
                                    comp_ports: fwd.comp_ports,
                                    declared_state_names: fwd.declared_state_names,
                                    enums: ctx.enums,
                                };
                                for ambient in fwd.ports {
                                    fwd_scope_ctx.is_event_port = ambient.name.as_str().starts_with("on_");
                                    let rewritten = rewrite_expr(&ambient.expr, &fwd_scope_ctx)?;
                                    merged_ports.insert(ambient.name.as_str().to_string(), rewritten);
                                    authored_ambient_ports.insert(ambient.name.as_str().to_string(), ambient.expr.clone());
                                }
                            }

                            for explicit in &child_elem.ports {
                                merged_ports.insert(explicit.name.as_str().to_string(), explicit.expr.clone());
                                authored_ambient_ports.remove(explicit.name.as_str());
                            }

                            let is_wrapper = elem.name.as_str() == "Anchor" || elem.name.as_str() == "AnchorScope";
                            if is_wrapper {
                                if !merged_ports.contains_key("x") {
                                    merged_ports.insert(
                                        "x".to_string(),
                                        Expr::MemberAccess(MemberAccessExpr {
                                            target: Box::new(Expr::Ident(Ident::new(node_id.canonical_name(), child_elem.span))),
                                            member: Ident::new("left", child_elem.span),
                                            span: child_elem.span,
                                        }),
                                    );
                                }
                                if !merged_ports.contains_key("y") {
                                    merged_ports.insert(
                                        "y".to_string(),
                                        Expr::MemberAccess(MemberAccessExpr {
                                            target: Box::new(Expr::Ident(Ident::new(node_id.canonical_name(), child_elem.span))),
                                            member: Ident::new("top", child_elem.span),
                                            span: child_elem.span,
                                        }),
                                    );
                                }
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

                            let child_ctx = ElementContext {
                                parent_id: Some(node_id),
                                parent_name: Some(elem.name.as_str()),
                                prev_sibling_id: last_child_id,
                                has_next_sibling,
                                parent_ports: &primitive_scope_ports,
                                lexical_scope: entry.caller_lexical_scope,
                                env_scope: &entry.caller_env_scope,
                                enclosing_component_id: entry.caller_enclosing_component_id,
                                enclosing_num_children: entry.caller_enclosing_num_children,
                                enclosing_instantiated_children: entry.caller_enclosing_instantiated_children,
                                shadowed_ports: entry.caller_shadowed_ports,
                                is_let: entry.var_name.is_some(),
                                ambient_authored_ports: Some(&authored_ambient_ports),
                                enums: ctx.enums,
                                active_scope_id: active_scope_for_children,
                                consumer_children: None,
                            };
                            let child_id = expand_element(
                                &wired_elem,
                                &child_ctx,
                                registry,
                                doc,
                                node_fonts,
                            )?;
                            if let Some(prev_id) = last_child_id {
                                ctx.lexical_scope.borrow_mut().insert(
                                    format!("__next_of_{}", prev_id.canonical_name()),
                                    LexicalBinding::Node(child_id),
                                );
                            }
                            if let Some(v_name) = &entry.var_name {
                                doc.get_node_mut(child_id).unwrap().var_name = Some(v_name.clone());
                                entry.caller_lexical_scope.borrow_mut().insert(v_name.clone(), LexicalBinding::Node(child_id));
                            }
                            if child_elem.name.as_str() == "Font" {
                                node_fonts.insert(child_id, child_id);
                            }
                            for inst_list in &entry.instantiated_ids {
                                inst_list.borrow_mut().push(child_id);
                            }
                            child_ids.push(child_id);
                            last_child_id = Some(child_id);
                            if elem.name.as_str() == "Text" {
                                let child_text = doc.nodes[child_id.0]
                                    .text_content
                                    .clone()
                                    .unwrap_or_default();
                                let start = full_text.len();
                                full_text.push_str(&child_text);
                                let end = full_text.len();
                                let is_link = child_elem.name.as_str() == "Link"
                                    || child_elem.ports.iter().any(|p| p.name.as_str() == "url");
                                let mut style = SpanStyle::default();
                                if is_link {
                                    style.url = child_elem.ports.iter().find(|p| p.name.as_str() == "url").and_then(|p| match &p.expr {
                                        Expr::Literal(Literal::String(s, _)) => Some(s.clone()),
                                        _ => None,
                                    });
                                    style.color = Some("#1a73e8".to_string());
                                    style.underline = true;
                                    style.cursor = Some(CursorKind::Pointer);
                                }
                                text_spans.push(TextSpan {
                                    range: start..end,
                                    node_id: Some(child_id),
                                    style,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    if elem.name.as_str() == "Text" && !full_text.is_empty() {
        doc.nodes[node_id.0].text_content = Some(full_text);
        doc.nodes[node_id.0].text_spans = text_spans;
    } else if let Some(tc) = &doc.nodes[node_id.0].text_content {
        doc.nodes[node_id.0].text_spans = vec![TextSpan::new(0..tc.len())];
    }

    // 1b. Expand any inline node element expressions in ports
    let mut resolved_port_exprs = Vec::with_capacity(elem.ports.len());
    for port in &elem.ports {
        match &port.expr {
            Expr::Node(inline_elem) => {
                let parent_id = if ctx.is_let {
                    ctx.parent_id
                } else {
                    Some(node_id)
                };
                let parent_name = if ctx.is_let {
                    ctx.parent_name
                } else {
                    Some(elem.name.as_str())
                };
                let child_ctx = ElementContext {
                    parent_id,
                    parent_name,
                    prev_sibling_id: last_child_id,
                    has_next_sibling: false,
                    parent_ports: ctx.parent_ports,
                    lexical_scope: ctx.lexical_scope,
                    env_scope: ctx.env_scope,
                    enclosing_component_id: ctx.enclosing_component_id,
                    enclosing_num_children: ctx.enclosing_num_children,
                    enclosing_instantiated_children: ctx.enclosing_instantiated_children,
                    shadowed_ports: ctx.shadowed_ports,
                    is_let: ctx.is_let,
                    ambient_authored_ports: None,
                    enums: ctx.enums,
                    active_scope_id: ctx.active_scope_id,
                    consumer_children: None,
                };
                let child_id = expand_element(inline_elem, &child_ctx, registry, doc, node_fonts)?;
                if inline_elem.name.as_str() != "Font" && inline_elem.name.as_str() != "TextBounds" {
                    child_ids.push(child_id);
                    last_child_id = Some(child_id);
                }
                resolved_port_exprs.push((
                    port.name.as_str().to_string(),
                    Expr::Ident(Ident::new(child_id.canonical_name(), inline_elem.span)),
                ));
            }
            other => {
                resolved_port_exprs.push((port.name.as_str().to_string(), other.clone()));
            }
        }
    }

    // 2. Rewrite element ports
    let empty_ports: [String; 0] = [];
    let empty_enclosing_children: [NodeId; 0] = [];
    let enclosing_children_borrow = ctx.enclosing_instantiated_children.map(|c| c.borrow());
    let enclosing_child_ids: &[NodeId] = match &enclosing_children_borrow {
        Some(b) => b.as_slice(),
        None => &empty_enclosing_children,
    };
    let current_node = node_id;
    let parent_node = ctx.parent_id;
    let current_ports = &empty_ports;
    let num_children = ctx.enclosing_num_children;

    // First, determine if this element has a font node (explicitly or ambiently)
    let mut font_node_id = None;
    let lexical_borrow = ctx.lexical_scope.borrow();
    if let Some((_, raw_font_expr)) = resolved_port_exprs.iter().find(|(name, _)| name == "font") {
        let temp_scope_ctx = ScopeContext {
            current_node,
            parent_node,
            parent_name: ctx.parent_name,
            prev_sibling: ctx.prev_sibling_id,
            has_next_sibling: ctx.has_next_sibling,
            child_ids: enclosing_child_ids,
            num_children,
            child_node: None,
            nodes: &doc.nodes,
            shadowed_ports: ctx.shadowed_ports,
            is_event_port: false,
            parent_ports: ctx.parent_ports,
            current_ports,
            lexical_scope: &*lexical_borrow,
            env_scope: ctx.env_scope,
            node_fonts: &*node_fonts,
            enclosing_component: ctx.enclosing_component_id,
            comp_ports: None,
            declared_state_names: None,
            enums: ctx.enums,
        };
        if let Ok(Expr::Ident(id)) = rewrite_expr(raw_font_expr, &temp_scope_ctx) {
            if let Some(fid) = NodeId::from_canonical_name(id.as_str()) {
                font_node_id = Some(fid);
            }
        }
    } else if elem.name.as_str() != "Font" {
        if let Some(EnvEntry::Bound(env_expr)) = ctx.env_scope.get("font") {
            let temp_scope_ctx = ScopeContext {
                current_node,
                parent_node,
                parent_name: ctx.parent_name,
                prev_sibling: ctx.prev_sibling_id,
                has_next_sibling: ctx.has_next_sibling,
                child_ids: enclosing_child_ids,
                num_children,
                child_node: None,
                nodes: &doc.nodes,
                shadowed_ports: ctx.shadowed_ports,
                is_event_port: false,
                parent_ports: ctx.parent_ports,
                current_ports,
                lexical_scope: &*lexical_borrow,
                env_scope: ctx.env_scope,
                node_fonts: &*node_fonts,
                enclosing_component: ctx.enclosing_component_id,
                comp_ports: None,
                declared_state_names: None,
                enums: ctx.enums,
            };
            if let Ok(rewritten) = rewrite_expr(env_expr, &temp_scope_ctx) {
                if let Expr::Ident(id) = &rewritten {
                    if let Some(fid) = NodeId::from_canonical_name(id.as_str()) {
                        font_node_id = Some(fid);
                        resolved_port_exprs.push(("font".to_string(), Expr::Ident(Ident::new(fid.canonical_name(), elem.span))));
                    }
                } else if matches!(rewritten, Expr::Literal(Literal::String(_, _))) {
                    resolved_port_exprs.push(("font".to_string(), rewritten));
                } else if elem.name.as_str() == "Text" {
                    resolved_port_exprs.push(("font".to_string(), rewritten));
                }
            }
        }
    }

    if let Some(fid) = font_node_id {
        node_fonts.insert(node_id, fid);
        doc.get_node_mut(node_id).unwrap().font = Some(fid);
    } else if elem.name.as_str() == "Font" {
        node_fonts.insert(node_id, node_id);
        doc.get_node_mut(node_id).unwrap().font = Some(node_id);
    }

    let scope_ctx = ScopeContext {
        current_node,
        parent_node,
        parent_name: ctx.parent_name,
        prev_sibling: ctx.prev_sibling_id,
        has_next_sibling: ctx.has_next_sibling,
        child_ids: enclosing_child_ids,
        num_children,
        child_node: None,
        nodes: &doc.nodes,
        shadowed_ports: ctx.shadowed_ports,
        is_event_port: false,
        parent_ports: ctx.parent_ports,
        current_ports,
        lexical_scope: &*lexical_borrow,
        env_scope: ctx.env_scope,
        node_fonts: &*node_fonts,
        enclosing_component: ctx.enclosing_component_id,
        comp_ports: None,
        declared_state_names: None,
        enums: ctx.enums,
    };

    let mut ports = HashMap::new();
    let mut authored_ports = HashMap::new();
    let mut event_handlers = HashMap::new();

    if let Some(content_slot) = &elem.content {
        for item in &content_slot.items {
            if let ContentItem::Assert(assert_dir) = item {
                let rewritten = rewrite_expr(&assert_dir.condition, &scope_ctx)?;
                match eval_expr(&rewritten, &HashMap::new()) {
                    Ok(val) => {
                        if !val.is_truthy() {
                            let msg = assert_dir
                                .message
                                .clone()
                                .unwrap_or_else(|| format!("Assertion failed: {}", assert_dir.condition));
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
    }
    for (port_name, expr) in resolved_port_exprs {
        let is_ambient = ctx
            .ambient_authored_ports
            .is_some_and(|m| m.contains_key(&port_name));
        if elem.name.as_str() == "Line"
            && is_ambient
            && matches!(port_name.as_str(), "x" | "y" | "width" | "height")
        {
            continue;
        }
        if let Some(ambient_expr) = ctx.ambient_authored_ports.and_then(|m| m.get(&port_name)) {
            authored_ports.insert(port_name.clone(), ambient_expr.clone());
        } else if let Some(orig_port) = elem.ports.iter().find(|p| p.name.as_str() == port_name) {
            authored_ports.insert(port_name.clone(), orig_port.expr.clone());
        } else if port_name == "font" {
            authored_ports.insert(
                port_name.clone(),
                Expr::Ident(Ident::new("font", elem.span)),
            );
        } else {
            authored_ports.insert(port_name.clone(), expr.clone());
        }

        if port_name.starts_with("on_") {
            let mut event_scope_ctx = scope_ctx.clone();
            event_scope_ctx.is_event_port = true;
            let rewritten = rewrite_expr(&expr, &event_scope_ctx)?;
            if let Some(binding) = extract_event_binding(&port_name, &rewritten) {
                event_handlers.insert(port_name, binding);
                continue;
            }
        }

        let mut rewritten = rewrite_expr(&expr, &scope_ctx)?;
        if matches!(
            port_name.as_str(),
            "x"
                | "y"
                | "x1"
                | "y1"
                | "x2"
                | "y2"
                | "start_x"
                | "start_y"
                | "end_x"
                | "end_y"
                | "width"
                | "height"
                | "size"
                | "line_height"
                | "baseline"
                | "radius"
                | "corner_radius"
                | "radius_left"
                | "radius_right"
                | "radius_top"
                | "radius_bottom"
                | "left_radius"
                | "right_radius"
                | "top_radius"
                | "bottom_radius"
                | "radius_top_left"
                | "radius_top_right"
                | "radius_bottom_right"
                | "radius_bottom_left"
                | "top_left_radius"
                | "top_right_radius"
                | "bottom_right_radius"
                | "bottom_left_radius"
                | "border_width"
                | "thickness"
                | "stroke_width"
                | "scroll_x"
                | "scroll_y"
        ) {
            let unlowered_lit = match &expr {
                Expr::Literal(lit) => Some(lit.clone()),
                _ => None,
            };
            if let Some(lit) = unlowered_lit.or_else(|| infer_expr_literal_for_typecheck(&rewritten, &scope_ctx)) {
                if !lit.matches_type_name("Distance") {
                    return Err(CompileError::TypeMismatch {
                        expected: "Distance".to_string(),
                        actual: lit.type_name().to_string(),
                        span: expr.span(),
                    });
                }
            }
            if let Expr::Literal(Literal::Integer(0, span)) = rewritten {
                rewritten = Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span));
            }
        } else if matches!(port_name.as_str(), "z" | "weight") {
            let unlowered_lit = match &expr {
                Expr::Literal(lit) => Some(lit.clone()),
                _ => None,
            };
            if let Some(lit) = unlowered_lit.or_else(|| infer_expr_literal_for_typecheck(&rewritten, &scope_ctx)) {
                if !lit.matches_type_name("Integer") {
                    return Err(CompileError::TypeMismatch {
                        expected: "Integer".to_string(),
                        actual: lit.type_name().to_string(),
                        span: expr.span(),
                    });
                }
            }
        }
        ports.insert(port_name, rewritten);
    }

    if elem.name.as_str() == "AnchorScope" {
        let canonical_path = doc.scope_tree.scopes[active_scope_for_children.0].canonical_path.clone();
        let name_str = doc.scope_tree.scopes[active_scope_for_children.0].name.clone();
        ports.insert("name".to_string(), Expr::Literal(Literal::String(name_str, elem.span)));
        ports.insert("path".to_string(), Expr::Literal(Literal::String(canonical_path, elem.span)));
    } else if elem.name.as_str() == "Anchor" {
        if let Some(ref a_name) = doc.nodes[node_id.0].anchor_name {
            ports.insert("name".to_string(), Expr::Literal(Literal::String(a_name.clone(), elem.span)));
        }
    }

    if elem.name.as_str() == "Font" {
        const FONT_PUBLIC_PORTS: &[&str] = &["family", "size", "weight", "line_height"];
        for port in &elem.ports {
            let name = port.name.as_str();
            if !FONT_PUBLIC_PORTS.contains(&name) {
                let mut provided: Vec<String> = elem.ports.iter().map(|p| p.name.as_str().to_string()).collect();
                provided.sort();
                let available_signatures = vec![vec![
                    "size".to_string(),
                    "weight".to_string(),
                    "family".to_string(),
                    "line_height".to_string(),
                ]];
                return Err(CompileError::NoMatchingOverload(Box::new(
                    NoMatchingOverloadDetails {
                        name: "Font".to_string(),
                        provided_ports: provided,
                        available_signatures,
                        span: port.name.span,
                    },
                )));
            }
        }

        // Size is a required field on Font
        if !ports.contains_key("size") {
            return Err(CompileError::MissingPort {
                node: "Font".to_string(),
                port: "size".to_string(),
                span: elem.span,
            });
        }
        // Default weight to 400 if not specified
        if !ports.contains_key("weight") {
            ports.insert(
                "weight".to_string(),
                Expr::Literal(Literal::Integer(400, elem.span)),
            );
        }
        // Default family to empty string if not specified
        if !ports.contains_key("family") {
            ports.insert(
                "family".to_string(),
                Expr::Literal(Literal::String(String::new(), elem.span)),
            );
        } else if let Some(Expr::Literal(Literal::String(family, span))) = ports.get("family") {
            if !family.is_empty() && !crate::compiler::text::font_family_exists(family) {
                return Err(CompileError::FontFamilyNotFound {
                    family: family.clone(),
                    span: *span,
                });
            }
        }

        let self_ident = Expr::Ident(Ident::new(node_id.canonical_name(), elem.span));
        let size_ref = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("size", elem.span),
            span: elem.span,
        });
        let weight_ref = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("weight", elem.span),
            span: elem.span,
        });
        let family_ref = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("family", elem.span),
            span: elem.span,
        });

        // Add typographic metric ports: cap_height, x_height, descent, ascent, em_top, line_gap, gap_bottom, line_height
        ports.entry("cap_height".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("font_cap_height", elem.span),
                args: vec![size_ref.clone(), weight_ref.clone(), family_ref.clone()],
                span: elem.span,
            })
        });
        ports.entry("x_height".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("font_x_height", elem.span),
                args: vec![size_ref.clone(), weight_ref.clone(), family_ref.clone()],
                span: elem.span,
            })
        });
        ports.entry("descent".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("font_descent", elem.span),
                args: vec![size_ref.clone(), weight_ref.clone(), family_ref.clone()],
                span: elem.span,
            })
        });
        ports.entry("ascent".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("font_ascent", elem.span),
                args: vec![size_ref.clone(), weight_ref.clone(), family_ref.clone()],
                span: elem.span,
            })
        });
        let descent_ref = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("descent", elem.span),
            span: elem.span,
        });
        ports.entry("em_top".to_string()).or_insert_with(|| {
            Expr::Binary(BinaryExpr {
                op: BinaryOp::Sub,
                left: Box::new(size_ref.clone()),
                right: Box::new(descent_ref.clone()),
                span: elem.span,
            })
        });
        ports.entry("line_gap".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("font_line_gap", elem.span),
                args: vec![size_ref.clone(), weight_ref.clone(), family_ref.clone()],
                span: elem.span,
            })
        });
        let line_gap_ref = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("line_gap", elem.span),
            span: elem.span,
        });
        ports.entry("gap_bottom".to_string()).or_insert_with(|| {
            Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(descent_ref),
                right: Box::new(line_gap_ref),
                span: elem.span,
            })
        });
        ports.entry("line_height".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("font_line_height", elem.span),
                args: vec![size_ref, weight_ref, family_ref],
                span: elem.span,
            })
        });

        let node = doc.get_node_mut(node_id).unwrap();
        node.overload_params = vec!["size".to_string()];
        node.children = child_ids;
        node.ports = ports;
        node.authored_ports = authored_ports;
        node.font = Some(node_id);
        node_fonts.insert(node_id, node_id);
        return Ok(());
    }

    if elem.name.as_str() == "Clip" {
        if !ports.contains_key("box") {
            return Err(CompileError::MissingPort {
                node: "Clip".to_string(),
                port: "box".to_string(),
                span: elem.span,
            });
        }
        if !ports.contains_key("up") {
            let default_up = if let Some(EnvEntry::Bound(clip_expr)) = ctx.env_scope.get("clip") {
                clip_expr.clone()
            } else if let Some(parent) = ctx.parent_id {
                if parent.is_window() {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(NodeId::WINDOW.canonical_name(), elem.span))),
                        member: Ident::new("clip", elem.span),
                        span: elem.span,
                    })
                } else {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(parent.canonical_name(), elem.span))),
                        member: Ident::new("clip", elem.span),
                        span: elem.span,
                    })
                }
            } else {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(NodeId::WINDOW.canonical_name(), elem.span))),
                    member: Ident::new("clip", elem.span),
                    span: elem.span,
                })
            };
            ports.insert("up".to_string(), default_up);
        }
        ports.insert(
            "clip".to_string(),
            Expr::Ident(Ident::new(node_id.canonical_name(), elem.span)),
        );
        if let Some(box_expr) = ports.get("box").cloned() {
            ports.entry("x".to_string()).or_insert_with(|| {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(box_expr.clone()),
                    member: Ident::new("x", elem.span),
                    span: elem.span,
                })
            });
            ports.entry("y".to_string()).or_insert_with(|| {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(box_expr.clone()),
                    member: Ident::new("y", elem.span),
                    span: elem.span,
                })
            });
            ports.entry("width".to_string()).or_insert_with(|| {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(box_expr.clone()),
                    member: Ident::new("width", elem.span),
                    span: elem.span,
                })
            });
            ports.entry("height".to_string()).or_insert_with(|| {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(box_expr.clone()),
                    member: Ident::new("height", elem.span),
                    span: elem.span,
                })
            });
        }
    } else if !ports.contains_key("clip") {
        let default_clip = if let Some(EnvEntry::Bound(clip_expr)) = ctx.env_scope.get("clip") {
            clip_expr.clone()
        } else if let Some(parent) = ctx.parent_id {
            if parent.is_window() {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(NodeId::WINDOW.canonical_name(), elem.span))),
                    member: Ident::new("clip", elem.span),
                    span: elem.span,
                })
            } else {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(parent.canonical_name(), elem.span))),
                    member: Ident::new("clip", elem.span),
                    span: elem.span,
                })
            }
        } else {
            Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(Expr::Ident(Ident::new(NodeId::WINDOW.canonical_name(), elem.span))),
                member: Ident::new("clip", elem.span),
                span: elem.span,
            })
        };
        ports.insert("clip".to_string(), default_clip);
    }

    if elem.name.as_str() == "Rect" {
        for port in ["x", "y", "width", "height"] {
            if !ports.contains_key(port) {
                return Err(CompileError::MissingPort {
                    node: "Rect".to_string(),
                    port: port.to_string(),
                    span: elem.span,
                });
            }
        }
        if !ports.contains_key("color") && !ports.contains_key("bg_color") {
            return Err(CompileError::MissingPort {
                node: "Rect".to_string(),
                port: "color".to_string(),
                span: elem.span,
            });
        }
    }

    if elem.name.as_str() == "Line" {
        for (alias, canonical) in [
            ("start_x", "x1"),
            ("start_y", "y1"),
            ("end_x", "x2"),
            ("end_y", "y2"),
        ] {
            if !ports.contains_key(canonical) {
                if let Some(val) = ports.get(alias).cloned() {
                    ports.insert(canonical.to_string(), val);
                }
                if let Some(authored) = authored_ports.get(alias).cloned() {
                    authored_ports.insert(canonical.to_string(), authored);
                }
            }
        }

        for port in ["x1", "y1", "x2", "y2"] {
            if !ports.contains_key(port) {
                return Err(CompileError::MissingPort {
                    node: "Line".to_string(),
                    port: port.to_string(),
                    span: elem.span,
                });
            }
        }

        if !ports.contains_key("thickness") {
            if let Some(sw) = ports
                .get("stroke_width")
                .or_else(|| ports.get("border_width"))
                .cloned()
            {
                ports.insert("thickness".to_string(), sw);
            } else {
                ports.insert(
                    "thickness".to_string(),
                    Expr::Literal(Literal::Distance(1.0, DistanceUnit::Px, elem.span)),
                );
            }
        }

        if !ports.contains_key("color") {
            if let Some(sc) = ports
                .get("stroke_color")
                .or_else(|| ports.get("border_color"))
                .cloned()
            {
                ports.insert("color".to_string(), sc);
            } else {
                ports.insert(
                    "color".to_string(),
                    Expr::Literal(Literal::Color("#000000".to_string(), elem.span)),
                );
            }
        }

        if !ports.contains_key("cap") {
            if let Some(lc) = ports.get("line_cap").cloned() {
                ports.insert("cap".to_string(), lc);
            } else {
                ports.insert(
                    "cap".to_string(),
                    Expr::Literal(Literal::Enum(
                        "LineCap".to_string(),
                        "Butt".to_string(),
                        elem.span,
                    )),
                );
            }
        }

        let self_ident = Expr::Ident(Ident::new(node_id.canonical_name(), elem.span));
        let x1_ref = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("x1", elem.span),
            span: elem.span,
        });
        let y1_ref = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("y1", elem.span),
            span: elem.span,
        });
        let x2_ref = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("x2", elem.span),
            span: elem.span,
        });
        let y2_ref = Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident),
            member: Ident::new("y2", elem.span),
            span: elem.span,
        });

        ports.entry("x".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("min", elem.span),
                args: vec![x1_ref.clone(), x2_ref.clone()],
                span: elem.span,
            })
        });
        ports.entry("y".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("min", elem.span),
                args: vec![y1_ref.clone(), y2_ref.clone()],
                span: elem.span,
            })
        });
        ports.entry("width".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("abs", elem.span),
                args: vec![Expr::Binary(BinaryExpr {
                    op: BinaryOp::Sub,
                    left: Box::new(x2_ref),
                    right: Box::new(x1_ref),
                    span: elem.span,
                })],
                span: elem.span,
            })
        });
        ports.entry("height".to_string()).or_insert_with(|| {
            Expr::Call(CallExpr {
                callee: Ident::new("abs", elem.span),
                args: vec![Expr::Binary(BinaryExpr {
                    op: BinaryOp::Sub,
                    left: Box::new(y2_ref),
                    right: Box::new(y1_ref),
                    span: elem.span,
                })],
                span: elem.span,
            })
        });
    }

    // Base Spatial Trait defaults for height and width
    let text_content = doc
        .get_node(node_id)
        .and_then(|n| n.text_content.clone());
    let has_dynamic_text = ports.contains_key("text") || ports.contains_key("content");
    let has_text = text_content.as_ref().is_some_and(|t| !t.is_empty()) || has_dynamic_text;

    if let Some(fid) = font_node_id {
        let font_ident = Expr::Ident(Ident::new(fid.canonical_name(), elem.span));
        if !ports.contains_key("size") {
            ports.insert(
                "size".to_string(),
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(font_ident.clone()),
                    member: Ident::new("size", elem.span),
                    span: elem.span,
                }),
            );
        }
        if !ports.contains_key("weight") {
            ports.insert(
                "weight".to_string(),
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(font_ident.clone()),
                    member: Ident::new("weight", elem.span),
                    span: elem.span,
                }),
            );
        }
        if !ports.contains_key("family") {
            ports.insert(
                "family".to_string(),
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(font_ident.clone()),
                    member: Ident::new("family", elem.span),
                    span: elem.span,
                }),
            );
        }
    } else if let Some(font_target) = ports.get("font").cloned() {
        if matches!(font_target, Expr::Literal(Literal::String(_, _))) {
            if !ports.contains_key("family") {
                ports.insert("family".to_string(), font_target);
            }
        } else {
            if !ports.contains_key("size") {
                ports.insert(
                    "size".to_string(),
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(font_target.clone()),
                        member: Ident::new("size", elem.span),
                        span: elem.span,
                    }),
                );
            }
            if !ports.contains_key("weight") {
                ports.insert(
                    "weight".to_string(),
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(font_target.clone()),
                        member: Ident::new("weight", elem.span),
                        span: elem.span,
                    }),
                );
            }
            if !ports.contains_key("family") {
                ports.insert(
                    "family".to_string(),
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(font_target.clone()),
                        member: Ident::new("family", elem.span),
                        span: elem.span,
                    }),
                );
            }
        }
    }

    if elem.name.as_str() == "Text" {
        for port in &elem.ports {
            if matches!(
                port.name.as_str(),
                "start_at" | "starts_at" | "end_at" | "ends_at"
            ) {
                return Err(CompileError::Custom {
                    message: format!(
                        "Property '{}' is not supported directly on Text; use 'text_bounds: \\TextBounds(start_at: ..., end_at: ...)' or '\\env text_bounds = \\TextBounds(...)' instead",
                        port.name.as_str()
                    ),
                    span: port.name.span,
                });
            }
            if port.name.as_str() == "size" {
                return Err(CompileError::Custom {
                    message: "Property 'size' is not supported directly on Text; use 'font: \\Font(size: ...)' or '\\env font = \\Font(...)' instead".to_string(),
                    span: port.name.span,
                });
            }
        }

        if let Some(Expr::Literal(Literal::String(family, span))) = ports.get("family") {
            if !family.is_empty() && !crate::compiler::text::font_family_exists(family) {
                return Err(CompileError::FontFamilyNotFound {
                    family: family.clone(),
                    span: *span,
                });
            }
        }
        if let Some(Expr::Literal(Literal::String(family, span))) = ports.get("font") {
            if !family.is_empty() && !crate::compiler::text::font_family_exists(family) {
                return Err(CompileError::FontFamilyNotFound {
                    family: family.clone(),
                    span: *span,
                });
            }
        }

        if !ports.contains_key("text_bounds") {
            if let Some(EnvEntry::Bound(env_expr)) = ctx.env_scope.get("text_bounds") {
                if let Ok(rewritten) = rewrite_expr(env_expr, &scope_ctx) {
                    ports.insert("text_bounds".to_string(), rewritten.clone());
                    authored_ports.insert("text_bounds".to_string(), rewritten);
                }
            }
        }

        if let Some(tb_target) = ports.get("text_bounds").cloned() {
            let start_at_from_tb = Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(tb_target.clone()),
                member: Ident::new("start_at", elem.span),
                span: elem.span,
            });
            let end_at_from_tb = Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(tb_target),
                member: Ident::new("end_at", elem.span),
                span: elem.span,
            });
            ports.insert("start_at".to_string(), start_at_from_tb.clone());
            authored_ports.insert("start_at".to_string(), start_at_from_tb);
            ports.insert("end_at".to_string(), end_at_from_tb.clone());
            authored_ports.insert("end_at".to_string(), end_at_from_tb);
        } else {
            let default_start_at = Expr::Literal(Literal::Enum("TextStart".to_string(), "Capital".to_string(), elem.span));
            ports.insert("start_at".to_string(), default_start_at.clone());
            authored_ports.insert("start_at".to_string(), default_start_at);
            let default_end_at = Expr::Literal(Literal::Enum("TextEnd".to_string(), "Baseline".to_string(), elem.span));
            ports.insert("end_at".to_string(), default_end_at.clone());
            authored_ports.insert("end_at".to_string(), default_end_at);
        }

        if !ports.contains_key("size") {
            // Check if this is an inline text span inside a parent Text node
            if let Some(parent) = ctx.parent_id {
                if doc.get_node(parent).is_some_and(|n| n.name == "Text") {
                    let parent_ident = Expr::Ident(Ident::new(parent.canonical_name(), elem.span));
                    ports.insert(
                        "size".to_string(),
                        Expr::MemberAccess(MemberAccessExpr {
                            target: Box::new(parent_ident),
                            member: Ident::new("size", elem.span),
                            span: elem.span,
                        }),
                    );
                }
            }
        }

        if !ports.contains_key("size") {
            return Err(CompileError::MissingPort {
                node: "Text".to_string(),
                port: "font".to_string(),
                span: elem.span,
            });
        }

        if !ports.contains_key("line_height") {
            if let Some(&fid) = node_fonts.get(&node_id) {
                if doc.get_node(fid).is_some_and(|fn_node| fn_node.authored_ports.contains_key("line_height")) {
                    let font_ident = Expr::Ident(Ident::new(fid.canonical_name(), elem.span));
                    ports.insert(
                        "line_height".to_string(),
                        Expr::MemberAccess(MemberAccessExpr {
                            target: Box::new(font_ident),
                            member: Ident::new("line_height", elem.span),
                            span: elem.span,
                        }),
                    );
                }
            }
        }
    }

    let self_ident = Expr::Ident(Ident::new(node_id.canonical_name(), elem.span));
    let size_expr = if ports.contains_key("size") {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("size", elem.span),
            span: elem.span,
        })
    } else {
        Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span))
    };

    let weight_expr = if ports.contains_key("weight") {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("weight", elem.span),
            span: elem.span,
        })
    } else {
        Expr::Literal(Literal::Integer(400, elem.span))
    };

    let font_expr = if ports.contains_key("family") {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("family", elem.span),
            span: elem.span,
        })
    } else if ports.contains_key("font") {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("font", elem.span),
            span: elem.span,
        })
    } else {
        Expr::Literal(Literal::String(String::new(), elem.span))
    };

    let start_at_expr = if ports.contains_key("start_at") {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("start_at", elem.span),
            span: elem.span,
        })
    } else {
        Expr::Literal(Literal::Enum("TextStart".to_string(), "Capital".to_string(), elem.span))
    };

    if elem.name.as_str() == "Text" {
        if ports.contains_key("baseline") && !ports.contains_key("y") && !ports.contains_key("top") {
            let y_from_baseline = Expr::Binary(BinaryExpr {
                op: BinaryOp::Sub,
                left: Box::new(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("baseline", elem.span),
                    span: elem.span,
                })),
                right: Box::new(Expr::Call(CallExpr {
                    callee: Ident::new("text_baseline_offset", elem.span),
                    args: vec![
                        size_expr.clone(),
                        weight_expr.clone(),
                        font_expr.clone(),
                        start_at_expr.clone(),
                    ],
                    span: elem.span,
                })),
                span: elem.span,
            });
            ports.insert("y".to_string(), y_from_baseline.clone());
            authored_ports.insert("y".to_string(), y_from_baseline);
        } else {
            let baseline_call = Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("y", elem.span),
                    span: elem.span,
                })),
                right: Box::new(Expr::Call(CallExpr {
                    callee: Ident::new("text_baseline_offset", elem.span),
                    args: vec![
                        size_expr.clone(),
                        weight_expr.clone(),
                        font_expr.clone(),
                        start_at_expr.clone(),
                    ],
                    span: elem.span,
                })),
                span: elem.span,
            });
            ports.entry("baseline".to_string()).or_insert(baseline_call);
        }
    }

    let text_arg_expr = if ports.contains_key("text") {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("text", elem.span),
            span: elem.span,
        })
    } else if ports.contains_key("content") {
        Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(self_ident.clone()),
            member: Ident::new("content", elem.span),
            span: elem.span,
        })
    } else {
        let text_str = text_content.clone().unwrap_or_default();
        Expr::Literal(Literal::String(text_str, elem.span))
    };

    let is_anchor_like = elem.name.as_str() == "Anchor" || elem.name.as_str() == "AnchorScope";

    if is_anchor_like {
        if child_ids.len() == 1 {
            let cid = child_ids[0];
            let cid_ident = Expr::Ident(Ident::new(cid.canonical_name(), elem.span));

            // Child origin defaults to Anchor origin
            doc.nodes[cid.0].ports.entry("x".to_string()).or_insert_with(|| {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("x", elem.span),
                    span: elem.span,
                })
            });
            doc.nodes[cid.0].ports.entry("y".to_string()).or_insert_with(|| {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("y", elem.span),
                    span: elem.span,
                })
            });

            // Anchor dimensions inherit from Child dimensions if not explicitly set
            if !ports.contains_key("width") {
                ports.insert(
                    "width".to_string(),
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(cid_ident.clone()),
                        member: Ident::new("width", elem.span),
                        span: elem.span,
                    }),
                );
            } else {
                doc.nodes[cid.0].ports.entry("width".to_string()).or_insert_with(|| {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(self_ident.clone()),
                        member: Ident::new("width", elem.span),
                        span: elem.span,
                    })
                });
            }

            if !ports.contains_key("height") {
                ports.insert(
                    "height".to_string(),
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(cid_ident.clone()),
                        member: Ident::new("height", elem.span),
                        span: elem.span,
                    }),
                );
            } else {
                doc.nodes[cid.0].ports.entry("height".to_string()).or_insert_with(|| {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(self_ident.clone()),
                        member: Ident::new("height", elem.span),
                        span: elem.span,
                    })
                });
            }
        } else if child_ids.is_empty() {
            ports.entry("x".to_string()).or_insert_with(|| Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span)));
            ports.entry("y".to_string()).or_insert_with(|| Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span)));
            ports.entry("width".to_string()).or_insert_with(|| Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span)));
            ports.entry("height".to_string()).or_insert_with(|| Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span)));
        } else {
            for cid in &child_ids {
                doc.nodes[cid.0].ports.entry("x".to_string()).or_insert_with(|| {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(self_ident.clone()),
                        member: Ident::new("x", elem.span),
                        span: elem.span,
                    })
                });
                doc.nodes[cid.0].ports.entry("y".to_string()).or_insert_with(|| {
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(self_ident.clone()),
                        member: Ident::new("y", elem.span),
                        span: elem.span,
                    })
                });
            }
            if !ports.contains_key("width") {
                let first_cid = child_ids[0];
                ports.insert(
                    "width".to_string(),
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(first_cid.canonical_name(), elem.span))),
                        member: Ident::new("width", elem.span),
                        span: elem.span,
                    }),
                );
            }
            if !ports.contains_key("height") {
                let first_cid = child_ids[0];
                ports.insert(
                    "height".to_string(),
                    Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(first_cid.canonical_name(), elem.span))),
                        member: Ident::new("height", elem.span),
                        span: elem.span,
                    }),
                );
            }
        }

        ports.entry("z".to_string()).or_insert_with(|| Expr::Literal(Literal::Integer(0, elem.span)));
    }

    let is_inline_span = ctx.parent_id.is_some_and(|p| doc.get_node(p).is_some_and(|n| n.name == "Text"));

    if is_inline_span {
        ports.entry("x".to_string()).or_insert_with(|| Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span)));
        ports.entry("y".to_string()).or_insert_with(|| Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span)));
    }

    // Height defaults / resolution
    if !is_anchor_like && !ports.contains_key("height") {
        if (elem.name.as_str() == "Text" || has_text || ports.contains_key("width")) && has_text {
            // Text wrapping with Parley: height depends on width, size, weight, font
            let width_expr = Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(self_ident.clone()),
                member: Ident::new("width", elem.span),
                span: elem.span,
            });
            let end_at_expr = if ports.contains_key("end_at") {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("end_at", elem.span),
                    span: elem.span,
                })
            } else {
                Expr::Literal(Literal::Enum("TextEnd".to_string(), "Baseline".to_string(), elem.span))
            };
            let line_height_arg = if ports.contains_key("line_height") {
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("line_height", elem.span),
                    span: elem.span,
                })
            } else {
                Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span))
            };
            let height_call = Expr::Call(CallExpr {
                callee: Ident::new("text_height", elem.span),
                args: vec![
                    text_arg_expr.clone(),
                    size_expr.clone(),
                    weight_expr.clone(),
                    font_expr.clone(),
                    width_expr,
                    end_at_expr,
                    start_at_expr,
                    line_height_arg,
                ],
                span: elem.span,
            });
            ports.insert("height".to_string(), height_call);
        } else if ports.contains_key("size") {
            // E.g. \Header(size: 32px) -> height: self.size
            ports.insert(
                "height".to_string(),
                Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("size", elem.span),
                    span: elem.span,
                }),
            );
        } else if is_inline_span {
            ports.insert(
                "height".to_string(),
                Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span)),
            );
        } else if elem.name.as_str() != "Font" && elem.name.as_str() != "Clip" {
            return Err(CompileError::MissingPort {
                node: elem.name.as_str().to_string(),
                port: "height".to_string(),
                span: elem.span,
            });
        }
    }

    // Width defaults / resolution
    if !is_anchor_like && !ports.contains_key("width") {
        if (elem.name.as_str() == "Text" || has_text) && has_text {
            let width_call = Expr::Call(CallExpr {
                callee: Ident::new("text_width", elem.span),
                args: vec![
                    text_arg_expr,
                    size_expr,
                    weight_expr,
                    font_expr,
                ],
                span: elem.span,
            });
            ports.insert("width".to_string(), width_call);
        } else if is_inline_span {
            ports.insert(
                "width".to_string(),
                Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, elem.span)),
            );
        } else if elem.name.as_str() != "Font" && elem.name.as_str() != "Clip" {
            return Err(CompileError::MissingPort {
                node: elem.name.as_str().to_string(),
                port: "width".to_string(),
                span: elem.span,
            });
        }
    }

    // Default spatial alias ports for spatial primitives (Font returned early above)
    let self_ident = Expr::Ident(Ident::new(node_id.canonical_name(), elem.span));
    if !ports.contains_key("left") {
        ports.insert(
            "left".to_string(),
            Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(self_ident.clone()),
                member: Ident::new("x", elem.span),
                span: elem.span,
            }),
        );
        authored_ports.insert("left".to_string(), Expr::Ident(Ident::new("x", elem.span)));
    }
    if !ports.contains_key("top") {
        ports.insert(
            "top".to_string(),
            Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(self_ident.clone()),
                member: Ident::new("y", elem.span),
                span: elem.span,
            }),
        );
        authored_ports.insert("top".to_string(), Expr::Ident(Ident::new("y", elem.span)));
    }
    if !ports.contains_key("right") {
        ports.insert(
            "right".to_string(),
            Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("x", elem.span),
                    span: elem.span,
                })),
                right: Box::new(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("width", elem.span),
                    span: elem.span,
                })),
                span: elem.span,
            }),
        );
        authored_ports.insert(
            "right".to_string(),
            Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(Expr::Ident(Ident::new("x", elem.span))),
                right: Box::new(Expr::Ident(Ident::new("width", elem.span))),
                span: elem.span,
            }),
        );
    }
    if !ports.contains_key("bottom") {
        ports.insert(
            "bottom".to_string(),
            Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident.clone()),
                    member: Ident::new("y", elem.span),
                    span: elem.span,
                })),
                right: Box::new(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(self_ident),
                    member: Ident::new("height", elem.span),
                    span: elem.span,
                })),
                span: elem.span,
            }),
        );
        authored_ports.insert(
            "bottom".to_string(),
            Expr::Binary(BinaryExpr {
                op: BinaryOp::Add,
                left: Box::new(Expr::Ident(Ident::new("y", elem.span))),
                right: Box::new(Expr::Ident(Ident::new("height", elem.span))),
                span: elem.span,
            }),
        );
    }

    let overload_params = match elem.name.as_str() {
        "Clip" => vec!["box".to_string()],
        "Anchor" | "AnchorScope" => vec!["name".to_string()],
        "Line" => vec![
            "x1".to_string(),
            "y1".to_string(),
            "x2".to_string(),
            "y2".to_string(),
        ],
        "Rect" => {
            let color_port = if ports.contains_key("bg_color") && !ports.contains_key("color") {
                "bg_color"
            } else {
                "color"
            };
            vec![
                "x".to_string(),
                "y".to_string(),
                "width".to_string(),
                "height".to_string(),
                color_port.to_string(),
            ]
        }
        "Text" => {
            if is_inline_span {
                Vec::new()
            } else {
                let mut params = vec!["x".to_string()];
                let used_baseline = elem.ports.iter().any(|p| p.name.as_str() == "baseline")
                    && !elem.ports.iter().any(|p| p.name.as_str() == "y");
                params.push(if used_baseline { "baseline" } else { "y" }.to_string());
                params.push("font".to_string());
                params
            }
        }
        _ => {
            if ports.contains_key("size") && !elem.ports.iter().any(|p| p.name.as_str() == "height") {
                vec!["x".to_string(), "y".to_string(), "width".to_string(), "size".to_string()]
            } else {
                vec!["x".to_string(), "y".to_string(), "width".to_string(), "height".to_string()]
            }
        }
    };

    let node = doc.get_node_mut(node_id).unwrap();
    node.overload_params = overload_params;
    node.children = child_ids;
    node.ports = ports;
    node.authored_ports = authored_ports;
    node.event_handlers = event_handlers;

    Ok(())
}

fn infer_expr_literal_for_typecheck<'a>(
    expr: &'a Expr,
    ctx: &ScopeContext<'a>,
) -> Option<Literal> {
    if let Some(lit) = resolve_ident_or_member_to_literal(expr, ctx) {
        return Some(lit);
    }
    match expr {
        Expr::Paren(inner, _) => infer_expr_literal_for_typecheck(inner, ctx),
        Expr::Unary(u) if u.op == UnaryOp::Neg => {
            match infer_expr_literal_for_typecheck(&u.operand, ctx)? {
                Literal::Integer(i, _) => Some(Literal::Integer(-i, u.span)),
                Literal::Float(f, _) => Some(Literal::Float(-f, u.span)),
                Literal::Distance(d, unit, _) => Some(Literal::Distance(-d, unit, u.span)),
                Literal::Area(a, unit, _) => Some(Literal::Area(-a, unit, u.span)),
                _ => None,
            }
        }
        Expr::MemberAccess(m) => {
            let member = m.member.as_str();
            if matches!(
                member,
                "size"
                    | "width"
                    | "height"
                    | "x"
                    | "y"
                    | "x1"
                    | "y1"
                    | "x2"
                    | "y2"
                    | "start_x"
                    | "start_y"
                    | "end_x"
                    | "end_y"
                    | "thickness"
                    | "head_length"
                    | "head_width"
                    | "length"
                    | "left"
                    | "top"
                    | "right"
                    | "bottom"
                    | "baseline"
                    | "cap_height"
                    | "x_height"
                    | "ascent"
                    | "descent"
                    | "em_top"
                    | "line_gap"
                    | "gap_bottom"
                    | "line_height"
                    | "radius"
                    | "corner_radius"
                    | "radius_left"
                    | "radius_right"
                    | "radius_top"
                    | "radius_bottom"
                    | "left_radius"
                    | "right_radius"
                    | "top_radius"
                    | "bottom_radius"
                    | "radius_top_left"
                    | "radius_top_right"
                    | "radius_bottom_right"
                    | "radius_bottom_left"
                    | "top_left_radius"
                    | "top_right_radius"
                    | "bottom_right_radius"
                    | "bottom_left_radius"
                    | "border_width"
                    | "stroke_width"
                    | "padding"
                    | "padding_x"
                    | "padding_y"
            ) {
                return Some(Literal::Distance(0.0, DistanceUnit::Px, m.span));
            }
            if member == "dpi" {
                return Some(Literal::Float(96.0, m.span));
            }
            if member == "weight" || member == "z" || member == "len" {
                return Some(Literal::Integer(1, m.span));
            }
            None
        }
        Expr::Call(call) => {
            if call.callee.as_str() == "sqrt" {
                return match call
                    .args
                    .first()
                    .and_then(|a| infer_expr_literal_for_typecheck(a, ctx))?
                {
                    Literal::Area(_, _, _) => {
                        Some(Literal::Distance(0.0, DistanceUnit::Px, call.span))
                    }
                    _ => Some(Literal::Float(0.0, call.span)),
                };
            }
            if matches!(
                call.callee.as_str(),
                "distance_to_float"
                    | "to_px"
                    | "to_pt"
                    | "to_float"
                    | "to_number"
                    | "sin"
                    | "cos"
                    | "tan"
                    | "atan2"
            ) {
                return Some(Literal::Float(0.0, call.span));
            }
            if call.callee.as_str() == "to_integer" {
                return Some(Literal::Integer(1, call.span));
            }
            if call.callee.as_str() == "hypot" {
                if let Some(first) = call
                    .args
                    .first()
                    .and_then(|a| infer_expr_literal_for_typecheck(a, ctx))
                {
                    if matches!(first, Literal::Distance(_, _, _)) {
                        return Some(Literal::Distance(0.0, DistanceUnit::Px, call.span));
                    }
                }
                return Some(Literal::Float(0.0, call.span));
            }
            if matches!(
                call.callee.as_str(),
                "text_width"
                    | "text_height"
                    | "text_baseline_offset"
                    | "font_cap_height"
                    | "font_x_height"
                    | "font_ascent"
                    | "font_descent"
                    | "font_line_gap"
                    | "font_line_height"
            ) {
                return Some(Literal::Distance(0.0, DistanceUnit::Px, call.span));
            }
            if matches!(
                call.callee.as_str(),
                "only" | "max" | "min" | "sum" | "clamp" | "abs" | "round" | "floor" | "ceil"
            ) {
                return call
                    .args
                    .first()
                    .and_then(|a| infer_expr_literal_for_typecheck(a, ctx));
            }
            None
        }
        Expr::MethodCall(mc) => {
            if matches!(mc.method.as_str(), "to_px" | "to_pt" | "to_float" | "to_number") {
                return Some(Literal::Float(0.0, mc.span));
            }
            if mc.method.as_str() == "to_integer" {
                return Some(Literal::Integer(1, mc.span));
            }
            None
        }
        Expr::Binary(bin) => {
            let left = infer_expr_literal_for_typecheck(&bin.left, ctx)?;
            let right = infer_expr_literal_for_typecheck(&bin.right, ctx)?;
            match bin.op {
                BinaryOp::Add | BinaryOp::Sub => match (&left, &right) {
                    (Literal::Distance(_, _, _), Literal::Distance(_, _, _))
                    | (Literal::Distance(_, _, _), Literal::Integer(0, _))
                    | (Literal::Integer(0, _), Literal::Distance(_, _, _)) => {
                        Some(Literal::Distance(0.0, DistanceUnit::Px, bin.span))
                    }
                    (Literal::Area(_, _, _), Literal::Area(_, _, _))
                    | (Literal::Area(_, _, _), Literal::Integer(0, _))
                    | (Literal::Integer(0, _), Literal::Area(_, _, _)) => {
                        Some(Literal::Area(0.0, DistanceUnit::Px, bin.span))
                    }
                    (Literal::Integer(_, _), Literal::Integer(_, _)) => {
                        Some(Literal::Integer(1, bin.span))
                    }
                    (Literal::Float(_, _), Literal::Float(_, _))
                    | (Literal::Float(_, _), Literal::Integer(_, _))
                    | (Literal::Integer(_, _), Literal::Float(_, _)) => {
                        Some(Literal::Float(0.0, bin.span))
                    }
                    _ => None,
                },
                BinaryOp::Mul => match (&left, &right) {
                    (Literal::Distance(_, _, _), Literal::Distance(_, _, _)) => {
                        Some(Literal::Area(0.0, DistanceUnit::Px, bin.span))
                    }
                    (Literal::Distance(_, _, _), Literal::Integer(_, _) | Literal::Float(_, _))
                    | (Literal::Integer(_, _) | Literal::Float(_, _), Literal::Distance(_, _, _)) => {
                        Some(Literal::Distance(0.0, DistanceUnit::Px, bin.span))
                    }
                    (Literal::Area(_, _, _), Literal::Integer(_, _) | Literal::Float(_, _))
                    | (Literal::Integer(_, _) | Literal::Float(_, _), Literal::Area(_, _, _)) => {
                        Some(Literal::Area(0.0, DistanceUnit::Px, bin.span))
                    }
                    (Literal::Integer(_, _), Literal::Integer(_, _)) => {
                        Some(Literal::Integer(1, bin.span))
                    }
                    (Literal::Float(_, _), Literal::Float(_, _))
                    | (Literal::Float(_, _), Literal::Integer(_, _))
                    | (Literal::Integer(_, _), Literal::Float(_, _)) => {
                        Some(Literal::Float(0.0, bin.span))
                    }
                    _ => None,
                },
                BinaryOp::Div => match (&left, &right) {
                    (Literal::Distance(_, _, _), Literal::Distance(_, _, _))
                    | (Literal::Area(_, _, _), Literal::Area(_, _, _)) => {
                        Some(Literal::Float(0.0, bin.span))
                    }
                    (Literal::Area(_, _, _), Literal::Distance(_, _, _))
                    | (Literal::Distance(_, _, _), Literal::Integer(_, _) | Literal::Float(_, _)) => {
                        Some(Literal::Distance(0.0, DistanceUnit::Px, bin.span))
                    }
                    (Literal::Area(_, _, _), Literal::Integer(_, _) | Literal::Float(_, _)) => {
                        Some(Literal::Area(0.0, DistanceUnit::Px, bin.span))
                    }
                    (
                        Literal::Integer(_, _) | Literal::Float(_, _),
                        Literal::Integer(_, _) | Literal::Float(_, _),
                    ) => Some(Literal::Float(0.0, bin.span)),
                    _ => None,
                },
                _ => None,
            }
        }
        _ => None,
    }
}

fn resolve_ident_or_member_to_literal<'a>(
    expr: &'a Expr,
    ctx: &ScopeContext<'a>,
) -> Option<Literal> {
    match expr {
        Expr::Literal(lit) => Some(lit.clone()),
        Expr::Call(call) if call.callee.as_str() == "to_integer" && call.args.len() == 1 => {
            if let Expr::Literal(Literal::Integer(i, span)) = &call.args[0] {
                Some(Literal::Integer(*i, *span))
            } else {
                None
            }
        }
        Expr::Paren(inner, _) => resolve_ident_or_member_to_literal(inner, ctx),
        Expr::Ident(id) => {
            if let Some(comp_ports) = ctx.comp_ports {
                if let Some(port_expr) = comp_ports.get(id.as_str()) {
                    match port_expr {
                        Expr::Literal(lit) => return Some(lit.clone()),
                        Expr::Call(call) if call.callee.as_str() == "to_integer" && call.args.len() == 1 => {
                            if let Expr::Literal(Literal::Integer(i, span)) = &call.args[0] {
                                return Some(Literal::Integer(*i, *span));
                            }
                        }
                        Expr::MemberAccess(m) => {
                            if let Expr::Ident(target_id) = m.target.as_ref() {
                                if let Some(enum_def) = ctx.enums.get(target_id.as_str()) {
                                    if enum_def.has_variant(m.member.as_str()) {
                                        return Some(Literal::Enum(
                                            target_id.as_str().to_string(),
                                            m.member.as_str().to_string(),
                                            m.span,
                                        ));
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            if let Some(LexicalBinding::Expr(e) | LexicalBinding::State(e)) = ctx.lexical_scope.get(id.as_str()) {
                return resolve_ident_or_member_to_literal(e, ctx);
            }
            None
        }
        Expr::MemberAccess(m) => {
            if let Expr::Ident(target_id) = m.target.as_ref() {
                if target_id.as_str() == "children" && m.member.as_str() == "len" {
                    return Some(Literal::Integer(ctx.num_children as i64, m.span));
                }
                if let Some(enum_def) = ctx.enums.get(target_id.as_str()) {
                    if enum_def.has_variant(m.member.as_str()) {
                        return Some(Literal::Enum(
                            target_id.as_str().to_string(),
                            m.member.as_str().to_string(),
                            m.span,
                        ));
                    }
                }
                let is_target = target_id.as_str() == "parent"
                    || target_id.as_str() == "self"
                    || ctx.parent_node.is_some_and(|p| target_id.as_str() == p.canonical_name())
                    || target_id.as_str() == ctx.current_node.canonical_name();
                if is_target {
                    if let Some(comp_ports) = ctx.comp_ports {
                        if let Some(port_expr) = comp_ports.get(m.member.as_str()) {
                            match port_expr {
                                Expr::Literal(lit) => return Some(lit.clone()),
                                Expr::Call(call) if call.callee.as_str() == "to_integer" && call.args.len() == 1 => {
                                    if let Expr::Literal(Literal::Integer(i, span)) = &call.args[0] {
                                        return Some(Literal::Integer(*i, *span));
                                    }
                                }
                                Expr::MemberAccess(sub_m) => {
                                    if let Expr::Ident(sub_target_id) = sub_m.target.as_ref() {
                                        if let Some(enum_def) = ctx.enums.get(sub_target_id.as_str()) {
                                            if enum_def.has_variant(sub_m.member.as_str()) {
                                                return Some(Literal::Enum(
                                                    sub_target_id.as_str().to_string(),
                                                    sub_m.member.as_str().to_string(),
                                                    sub_m.span,
                                                ));
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            None
        }
        Expr::Binary(bin) => {
            let left_lit = resolve_ident_or_member_to_literal(&bin.left, ctx)?;
            let right_lit = resolve_ident_or_member_to_literal(&bin.right, ctx)?;
            match bin.op {
                BinaryOp::Eq => {
                    let eq = match (&left_lit, &right_lit) {
                        (Literal::String(s1, _), Literal::String(s2, _)) => s1 == s2,
                        (Literal::Integer(i1, _), Literal::Integer(i2, _)) => i1 == i2,
                        (Literal::Float(n1, _), Literal::Float(n2, _)) => n1 == n2,
                        (Literal::Integer(i1, _), Literal::Float(n2, _)) => (*i1 as f64) == *n2,
                        (Literal::Float(n1, _), Literal::Integer(i2, _)) => *n1 == (*i2 as f64),
                        (Literal::Distance(d1, u1, _), Literal::Distance(d2, u2, _))
                        | (Literal::Area(d1, u1, _), Literal::Area(d2, u2, _))
                            if u1 == u2 =>
                        {
                            d1 == d2
                        }
                        (Literal::Distance(d1, _, _) | Literal::Area(d1, _, _), Literal::Integer(0, _))
                        | (Literal::Integer(0, _), Literal::Distance(d1, _, _) | Literal::Area(d1, _, _)) => {
                            *d1 == 0.0
                        }
                        (Literal::Bool(b1, _), Literal::Bool(b2, _)) => b1 == b2,
                        (Literal::Enum(e1, v1, _), Literal::Enum(e2, v2, _)) => e1 == e2 && v1 == v2,
                        _ => false,
                    };
                    Some(Literal::Bool(eq, bin.span))
                }
                BinaryOp::Ne => {
                    let ne = match (&left_lit, &right_lit) {
                        (Literal::String(s1, _), Literal::String(s2, _)) => s1 != s2,
                        (Literal::Integer(i1, _), Literal::Integer(i2, _)) => i1 != i2,
                        (Literal::Float(n1, _), Literal::Float(n2, _)) => n1 != n2,
                        (Literal::Integer(i1, _), Literal::Float(n2, _)) => (*i1 as f64) != *n2,
                        (Literal::Float(n1, _), Literal::Integer(i2, _)) => *n1 != (*i2 as f64),
                        (Literal::Distance(d1, u1, _), Literal::Distance(d2, u2, _))
                        | (Literal::Area(d1, u1, _), Literal::Area(d2, u2, _))
                            if u1 == u2 =>
                        {
                            d1 != d2
                        }
                        (Literal::Distance(d1, _, _) | Literal::Area(d1, _, _), Literal::Integer(0, _))
                        | (Literal::Integer(0, _), Literal::Distance(d1, _, _) | Literal::Area(d1, _, _)) => {
                            *d1 != 0.0
                        }
                        (Literal::Bool(b1, _), Literal::Bool(b2, _)) => b1 != b2,
                        (Literal::Enum(e1, v1, _), Literal::Enum(e2, v2, _)) => e1 != e2 || v1 != v2,
                        _ => true,
                    };
                    Some(Literal::Bool(ne, bin.span))
                }
                BinaryOp::And => {
                    if let (Literal::Bool(b1, _), Literal::Bool(b2, _)) = (&left_lit, &right_lit) {
                        Some(Literal::Bool(*b1 && *b2, bin.span))
                    } else {
                        None
                    }
                }
                BinaryOp::Or => {
                    if let (Literal::Bool(b1, _), Literal::Bool(b2, _)) = (&left_lit, &right_lit) {
                        Some(Literal::Bool(*b1 || *b2, bin.span))
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }
        Expr::Unary(u) => match u.op {
            UnaryOp::Not => {
                if let Some(Literal::Bool(b, _)) = resolve_ident_or_member_to_literal(&u.operand, ctx) {
                    Some(Literal::Bool(!b, u.span))
                } else {
                    None
                }
            }
            UnaryOp::Neg => match resolve_ident_or_member_to_literal(&u.operand, ctx)? {
                Literal::Integer(i, _) => Some(Literal::Integer(-i, u.span)),
                Literal::Float(f, _) => Some(Literal::Float(-f, u.span)),
                Literal::Distance(d, unit, _) => Some(Literal::Distance(-d, unit, u.span)),
                Literal::Area(a, unit, _) => Some(Literal::Area(-a, unit, u.span)),
                _ => None,
            },
        },
        _ => None,
    }
}

/// Evaluates sibling existence conditions (`prev`, `next`, `!prev`, `!next`, `&&`, `||`) at expansion time.
fn eval_sibling_condition(expr: &Expr, ctx: &ScopeContext<'_>) -> Option<bool> {
    match expr {
        Expr::Paren(inner, _) => eval_sibling_condition(inner, ctx),
        Expr::Ident(id) => match id.as_str() {
            "prev" => Some(ctx.prev_sibling.is_some()),
            "next" => Some(ctx.has_next_sibling),
            _ => None,
        },
        Expr::Unary(u) if u.op == UnaryOp::Not => {
            eval_sibling_condition(&u.operand, ctx).map(|b| !b)
        }
        Expr::Binary(bin) => match bin.op {
            BinaryOp::And => {
                let l = eval_sibling_condition(&bin.left, ctx)?;
                let r = eval_sibling_condition(&bin.right, ctx)?;
                Some(l && r)
            }
            BinaryOp::Or => {
                let l = eval_sibling_condition(&bin.left, ctx)?;
                let r = eval_sibling_condition(&bin.right, ctx)?;
                Some(l || r)
            }
            _ => None,
        },
        _ => None,
    }
}

/// Resolves the active font's `.size` expression in the current scope for `em` units and `distance_to_float(d, em)`.
fn resolve_active_font_size_expr(ctx: &ScopeContext<'_>, span: Span) -> Result<Expr, CompileError> {
    if let Some(&fid) = ctx.node_fonts.get(&ctx.current_node) {
        if fid != ctx.current_node {
            return Ok(Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(Expr::Ident(Ident::new(fid.canonical_name(), span))),
                member: Ident::new("size", span),
                span,
            }));
        }
    }
    match ctx.env_scope.get("font") {
        Some(EnvEntry::Bound(font_expr)) => {
            let rewritten_font = rewrite_expr(font_expr, ctx)?;
            Ok(Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(rewritten_font),
                member: Ident::new("size", span),
                span,
            }))
        }
        Some(EnvEntry::Tombstone) => Err(CompileError::BlockedEnvVariable {
            name: "font".to_string(),
            span,
        }),
        None => Err(CompileError::UndefinedEnvVariable {
            name: "font".to_string(),
            span,
        }),
    }
}

/// Recursively rewrites an expression into canonical node variable references and resolves derived aliases.
pub fn rewrite_expr(expr: &Expr, ctx: &ScopeContext<'_>) -> Result<Expr, CompileError> {
    match expr {
        Expr::Ident(id) => {
            if id.as_str() == "env" {
                return Err(CompileError::BareEnvUse { span: id.span });
            }
            if ctx.enums.contains_key(id.as_str()) {
                return Err(CompileError::BareEnumUse {
                    name: id.as_str().to_string(),
                    span: id.span,
                });
            }

            // Check lexical scope first (let bindings, component ports, named nodes)
            if id.as_str() != "self"
                && id.as_str() != "parent"
                && id.as_str() != "window"
                && id.as_str() != "prev"
                && id.as_str() != "next"
                && id.as_str() != "child"
            {
                if let Some(binding) = ctx.lexical_scope.get(id.as_str()) {
                    match binding {
                        LexicalBinding::Expr(e) | LexicalBinding::State(e) => return Ok(e.clone()),
                        LexicalBinding::Node(node_id) => {
                            return Ok(Expr::Ident(Ident::new(node_id.canonical_name(), id.span)));
                        }
                        LexicalBinding::Uninitialized => {
                            return Err(CompileError::UninitializedVariableUse {
                                name: id.as_str().to_string(),
                                span: id.span,
                            });
                        }
                    }
                }
            }

            // In an event port (on_*), a bare method name that wasn't in lexical_scope
            // resolves to the enclosing component's method (or current_node if in signature default).
            if ctx.is_event_port
                && id.as_str() != "unset"
                && id.as_str() != "None"
                && id.as_str() != "self"
                && id.as_str() != "parent"
                && id.as_str() != "window"
                && id.as_str() != "prev"
                && id.as_str() != "next"
                && id.as_str() != "child"
                && NodeId::from_canonical_name(id.as_str()).is_none()
            {
                let target_id = ctx.enclosing_component.unwrap_or(ctx.current_node);
                return Ok(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(target_id.canonical_name(), id.span))),
                    member: id.clone(),
                    span: id.span,
                }));
            }

            // Check if it's a port on the current node (e.g. component parameter in default expression)
            if ctx.current_ports.iter().any(|p| p == id.as_str()) {
                return Ok(Expr::MemberAccess(MemberAccessExpr {
                    target: Box::new(Expr::Ident(Ident::new(ctx.current_node.canonical_name(), id.span))),
                    member: id.clone(),
                    span: id.span,
                }));
            }

            if id.as_str() == "window" {
                return Ok(Expr::Ident(Ident::new(NodeId::WINDOW.canonical_name(), id.span)));
            }
            if id.as_str() == "self" {
                return Err(CompileError::Custom {
                    message: "Bare 'self' is not allowed; use 'self.<method>' in an event handler or 'self.<property>' to access a shadowed component property".to_string(),
                    span: id.span,
                });
            }
            if id.as_str() == "child" {
                if let Some(cid) = ctx.child_node {
                    return Ok(Expr::Ident(Ident::new(cid.canonical_name(), id.span)));
                } else {
                    return Err(CompileError::Custom {
                        message: "'child' can only be used inside a '\\Children(...)' block".to_string(),
                        span: id.span,
                    });
                }
            }
            if id.as_str() == "parent" {
                if let Some(parent) = ctx.parent_node {
                    return Ok(Expr::Ident(Ident::new(parent.canonical_name(), id.span)));
                }
            }
            if id.as_str() == "prev" {
                if let Some(prev) = ctx.prev_sibling {
                    return Ok(Expr::Ident(Ident::new(prev.canonical_name(), id.span)));
                }
            }
            if id.as_str() == "next" {
                if ctx.has_next_sibling {
                    if let Some(cid) = ctx.child_node {
                        return Ok(Expr::Ident(Ident::new(
                            format!("__next_of_{}", cid.canonical_name()),
                            id.span,
                        )));
                    }
                }
            }
            if id.as_str() == "num_children" {
                return Err(CompileError::Custom {
                    message: "'num_children' is no longer supported; use 'children.len' instead".to_string(),
                    span: id.span,
                });
            }
            Ok(expr.clone())
        }

        Expr::Ternary(tern) => {
            // Recurrence resolution for `prev` / `next` conditions
            if let Some(cond_val) = eval_sibling_condition(&tern.condition, ctx) {
                let (chosen_raw, other_raw) = if cond_val {
                    (&tern.then_expr, &tern.else_expr)
                } else {
                    (&tern.else_expr, &tern.then_expr)
                };
                let mut chosen = rewrite_expr(chosen_raw, ctx)?;
                if let Expr::Literal(Literal::Integer(0, span)) = chosen {
                    if let Ok(other_rewritten) = rewrite_expr(other_raw, ctx) {
                        match infer_expr_literal_for_typecheck(&other_rewritten, ctx) {
                            Some(Literal::Distance(_, _, _)) => {
                                chosen = Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span));
                            }
                            Some(Literal::Area(_, _, _)) => {
                                chosen = Expr::Literal(Literal::Area(0.0, DistanceUnit::Px, span));
                            }
                            _ => {}
                        }
                    }
                }
                return Ok(chosen);
            }
            let rewritten_cond = rewrite_expr(&tern.condition, ctx)?;
            let mut const_env = HashMap::new();
            if let Some(comp_ports) = ctx.comp_ports {
                for (port_name, port_expr) in comp_ports.iter() {
                    if let Some(states) = ctx.declared_state_names {
                        if states.contains(port_name) {
                            continue;
                        }
                    }
                    if let Some(lit) = resolve_ident_or_member_to_literal(port_expr, ctx) {
                        let val = match lit {
                            Literal::Integer(i, _) => crate::compiler::value::Value::Integer(i),
                            Literal::Float(n, _) => crate::compiler::value::Value::Float(n),
                            Literal::Distance(d, _, _) => crate::compiler::value::Value::Distance(d),
                            Literal::Area(a, _, _) => crate::compiler::value::Value::Area(a),
                            Literal::String(s, _) => crate::compiler::value::Value::String(s),
                            Literal::Bool(b, _) => crate::compiler::value::Value::Bool(b),
                            Literal::Color(c, _) => crate::compiler::value::Value::Color(c),
                            Literal::Enum(e, v, _) => crate::compiler::value::Value::Enum { enum_name: e, variant: v },
                        };
                        const_env.insert(crate::compiler::graph::VarId::new(ctx.current_node, port_name), val.clone());
                        if let Some(parent) = ctx.parent_node {
                            const_env.insert(crate::compiler::graph::VarId::new(parent, port_name), val);
                        }
                    } else if let Ok(val) = crate::compiler::eval::eval_expr(port_expr, &HashMap::new()) {
                        const_env.insert(crate::compiler::graph::VarId::new(ctx.current_node, port_name), val.clone());
                        if let Some(parent) = ctx.parent_node {
                            const_env.insert(crate::compiler::graph::VarId::new(parent, port_name), val);
                        }
                    }
                }
            }
            if let Ok(crate::compiler::value::Value::Bool(b)) =
                crate::compiler::eval::eval_expr(&rewritten_cond, &const_env)
            {
                let (chosen_raw, other_raw) = if b {
                    (&tern.then_expr, &tern.else_expr)
                } else {
                    (&tern.else_expr, &tern.then_expr)
                };
                let mut chosen = rewrite_expr(chosen_raw, ctx)?;
                if let Expr::Literal(Literal::Integer(0, span)) = chosen {
                    if let Ok(other_rewritten) = rewrite_expr(other_raw, ctx) {
                        match infer_expr_literal_for_typecheck(&other_rewritten, ctx) {
                            Some(Literal::Distance(_, _, _)) => {
                                chosen = Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, span));
                            }
                            Some(Literal::Area(_, _, _)) => {
                                chosen = Expr::Literal(Literal::Area(0.0, DistanceUnit::Px, span));
                            }
                            _ => {}
                        }
                    }
                }
                return Ok(chosen);
            }
            Ok(Expr::Ternary(TernaryExpr {
                condition: Box::new(rewritten_cond),
                then_expr: Box::new(rewrite_expr(&tern.then_expr, ctx)?),
                else_expr: Box::new(rewrite_expr(&tern.else_expr, ctx)?),
                span: tern.span,
            }))
        }

        Expr::MemberAccess(m) => {
            if m.member.as_str() == "baseline" {
                let mut inner_target = m.target.as_ref();
                while let Expr::Paren(p, _) = inner_target {
                    inner_target = p.as_ref();
                }
                if let Expr::MemberAccess(inner) = inner_target {
                    if inner.member.as_str() == "font" {
                        let text_target = rewrite_expr(&inner.target, ctx)?;
                        return Ok(Expr::MemberAccess(MemberAccessExpr {
                            target: Box::new(text_target),
                            member: Ident::new("baseline", m.span),
                            span: m.span,
                        }));
                    }
                }
            }

            let target_ident_name = match m.target.as_ref() {
                Expr::Ident(id) => Some(id.as_str().to_string()),
                _ => None,
            };

            // Enum variant access and children.len
            if let Some(target_name) = &target_ident_name {
                if target_name == "children" {
                    if m.member.as_str() == "len" {
                        return Ok(Expr::Literal(Literal::Integer(ctx.num_children as i64, m.span)));
                    } else {
                        return Err(CompileError::Custom {
                            message: format!(
                                "Cannot access 'children.{}' outside of an aggregation function; use 'children.len' for the child count",
                                m.member
                            ),
                            span: m.span,
                        });
                    }
                }
                if let Some(enum_def) = ctx.enums.get(target_name) {
                    let variant_name = m.member.as_str();
                    if enum_def.has_variant(variant_name) {
                        return Ok(Expr::Literal(Literal::Enum(
                            target_name.clone(),
                            variant_name.to_string(),
                            m.span,
                        )));
                    } else {
                        return Err(CompileError::UnknownEnumVariant {
                            enum_name: target_name.clone(),
                            variant: variant_name.to_string(),
                            span: m.member.span,
                        });
                    }
                }
            }

            if target_ident_name.as_deref() == Some("env") {
                let member_name = m.member.as_str();
                return match ctx.env_scope.get(member_name) {
                    Some(EnvEntry::Bound(expr)) => Ok(expr.clone()),
                    Some(EnvEntry::Tombstone) => Err(CompileError::BlockedEnvVariable {
                        name: member_name.to_string(),
                        span: m.span,
                    }),
                    None => Err(CompileError::UndefinedEnvVariable {
                        name: member_name.to_string(),
                        span: m.span,
                    }),
                };
            }

            // Resolve target (parent, window, prev, next, self, child, or named node in lexical scope)
            let resolved_target = if let Some(target_name) = &target_ident_name {
                if target_name == "self" {
                    if ctx.is_event_port {
                        Expr::Ident(Ident::new(ctx.current_node.canonical_name(), m.target.span()))
                    } else if let (Some(comp_id), Some(shadowed)) = (ctx.enclosing_component, ctx.shadowed_ports) {
                        if shadowed.contains(m.member.as_str()) {
                            Expr::Ident(Ident::new(comp_id.canonical_name(), m.target.span()))
                        } else {
                            return Err(CompileError::Custom {
                                message: format!(
                                    "Cannot use 'self.{}' because property '{}' is not shadowed by a local '\\let' or '\\env' binding; use '{}' directly",
                                    m.member, m.member, m.member
                                ),
                                span: m.span,
                            });
                        }
                    } else {
                        return Err(CompileError::Custom {
                            message: format!(
                                "Cannot use 'self.{}' outside of a component; 'self' is only allowed in event handlers or to access a shadowed component property",
                                m.member
                            ),
                            span: m.span,
                        });
                    }
                } else if target_name == "child" {
                    if let Some(cid) = ctx.child_node {
                        Expr::Ident(Ident::new(cid.canonical_name(), m.target.span()))
                    } else {
                        return Err(CompileError::Custom {
                            message: "'child' can only be used inside a '\\Children(...)' block".to_string(),
                            span: m.target.span(),
                        });
                    }
                } else if let Some(LexicalBinding::Node(node_id)) = ctx.lexical_scope.get(target_name) {
                    Expr::Ident(Ident::new(node_id.canonical_name(), m.target.span()))
                } else if target_name == "parent" {
                    if let Some(parent) = ctx.parent_node {
                        if m.member.as_str() != "parent"
                            && !ctx.parent_ports.is_empty()
                            && !ctx.parent_ports.contains(&m.member.as_str().to_string())
                        {
                            let parent_name = ctx.parent_name.unwrap_or("unknown");
                            return Err(CompileError::Custom {
                                message: format!("Node '{}' has no public property '{}'", parent_name, m.member),
                                span: m.span,
                            });
                        }
                        Expr::Ident(Ident::new(parent.canonical_name(), m.target.span()))
                    } else {
                        rewrite_expr(&m.target, ctx)?
                    }
                } else if target_name == "window" {
                    Expr::Ident(Ident::new(NodeId::WINDOW.canonical_name(), m.target.span()))
                } else if target_name == "prev" {
                    if let Some(prev) = ctx.prev_sibling {
                        Expr::Ident(Ident::new(prev.canonical_name(), m.target.span()))
                    } else {
                        rewrite_expr(&m.target, ctx)?
                    }
                } else if target_name == "next" {
                    if ctx.has_next_sibling {
                        if let Some(cid) = ctx.child_node {
                            Expr::Ident(Ident::new(
                                format!("__next_of_{}", cid.canonical_name()),
                                m.target.span(),
                            ))
                        } else {
                            rewrite_expr(&m.target, ctx)?
                        }
                    } else {
                        rewrite_expr(&m.target, ctx)?
                    }
                } else {
                    rewrite_expr(&m.target, ctx)?
                }
            } else {
                rewrite_expr(&m.target, ctx)?
            };

            let target_node_id = match &resolved_target {
                Expr::Ident(id) => NodeId::from_canonical_name(id.as_str()),
                _ => None,
            };

            // Chained parent access: e.g. parent.parent -> parent_id of target_node_id
            if m.member.as_str() == "parent" {
                if let Some(tid) = target_node_id {
                    let parent_id = if Some(tid) == ctx.child_node {
                        ctx.parent_node
                    } else {
                        ctx.nodes.get(tid.0).and_then(|n| n.parent)
                    };
                    if let Some(pid) = parent_id {
                        return Ok(Expr::Ident(Ident::new(pid.canonical_name(), m.span)));
                    } else {
                        return Err(CompileError::Custom {
                            message: "Node has no parent".to_string(),
                            span: m.span,
                        });
                    }
                }
            }

            // Font node reference access: e.g. label.font -> font_node_id
            if m.member.as_str() == "font" {
                if let Some(tid) = target_node_id {
                    if let Some(&fid) = ctx.node_fonts.get(&tid) {
                        return Ok(Expr::Ident(Ident::new(fid.canonical_name(), m.span)));
                    }
                }
            }

            // Typographic metric access: e.g. label.cap_height -> fid.cap_height
            if [
                "cap_height",
                "x_height",
                "descent",
                "ascent",
                "em_top",
                "line_gap",
                "gap_bottom",
                "line_height",
            ]
            .contains(&m.member.as_str())
            {
                if let Some(tid) = target_node_id {
                    if let Some(&fid) = ctx.node_fonts.get(&tid) {
                        return Ok(Expr::MemberAccess(MemberAccessExpr {
                            target: Box::new(Expr::Ident(Ident::new(fid.canonical_name(), m.span))),
                            member: m.member.clone(),
                            span: m.span,
                        }));
                    }
                }
            }

            if let Some(tid) = target_node_id {
                if let Some(target_node) = ctx.nodes.get(tid.0) {
                    if target_node.private_ports.contains(m.member.as_str())
                        && ctx.current_node != tid
                        && ctx.enclosing_component != Some(tid)
                        && target_ident_name.as_deref() != Some("parent")
                    {
                        return Err(CompileError::Custom {
                            message: format!(
                                "Node '{}' has no public property '{}'",
                                target_node.name, m.member
                            ),
                            span: m.span,
                        });
                    }
                }
            }

            Ok(Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(resolved_target),
                member: m.member.clone(),
                span: m.span,
            }))
        }

        Expr::Call(call) => {
            if call.callee.as_str() == "distance_to_float" {
                if call.args.len() != 2 {
                    return Err(CompileError::Custom {
                        message: format!(
                            "distance_to_float() expects 2 arguments (distance, unit), got {}",
                            call.args.len()
                        ),
                        span: call.span,
                    });
                }
                let dist_expr = rewrite_expr(&call.args[0], ctx)?;
                let unit_expr = match &call.args[1] {
                    Expr::Ident(unit_id) => {
                        let Some(unit) = DistanceUnit::from_str(unit_id.as_str()) else {
                            return Err(CompileError::Custom {
                                message: format!(
                                    "Second argument to distance_to_float() must be a unit ('px', 'pt', or 'em'), got '{}'",
                                    unit_id.as_str()
                                ),
                                span: unit_id.span,
                            });
                        };
                        rewrite_expr(&Expr::Literal(Literal::Distance(1.0, unit, unit_id.span)), ctx)?
                    }
                    already_lowered @ (Expr::Literal(Literal::Distance(_, DistanceUnit::Px, _))
                    | Expr::Binary(_)) => rewrite_expr(already_lowered, ctx)?,
                    other => {
                        return Err(CompileError::Custom {
                            message: "Second argument to distance_to_float() must be a unit ('px', 'pt', or 'em')".to_string(),
                            span: other.span(),
                        });
                    }
                };
                return Ok(Expr::Call(CallExpr {
                    callee: call.callee.clone(),
                    args: vec![dist_expr, unit_expr],
                    span: call.span,
                }));
            }

            let mut rewritten_args = Vec::new();
            let mut had_children = false;
            let mut children_member = None;
            for arg in &call.args {
                if let Expr::MemberAccess(m) = arg {
                    if let Expr::Ident(target_id) = m.target.as_ref() {
                        if target_id.as_str() == "children" && m.member.as_str() != "len" {
                            had_children = true;
                            children_member = Some(m.member.as_str().to_string());
                            if ctx.child_ids.len() == ctx.num_children {
                                for &child_id in ctx.child_ids {
                                    let child_access = Expr::MemberAccess(MemberAccessExpr {
                                        target: Box::new(Expr::Ident(Ident::new(
                                            child_id.canonical_name(),
                                            target_id.span,
                                        ))),
                                        member: m.member.clone(),
                                        span: m.span,
                                    });
                                    rewritten_args.push(child_access);
                                }
                            } else if let Some(comp_id) = ctx.enclosing_component {
                                for idx in 0..ctx.num_children {
                                    let target_name = ctx
                                        .child_ids
                                        .get(idx)
                                        .map(|id| id.canonical_name())
                                        .unwrap_or_else(|| {
                                            format!(
                                                "__child_of_{}_{}",
                                                comp_id.canonical_name(),
                                                idx
                                            )
                                        });
                                    let child_access = Expr::MemberAccess(MemberAccessExpr {
                                        target: Box::new(Expr::Ident(Ident::new(
                                            target_name,
                                            target_id.span,
                                        ))),
                                        member: m.member.clone(),
                                        span: m.span,
                                    });
                                    rewritten_args.push(child_access);
                                }
                            }
                            continue;
                        }
                    }
                }
                rewritten_args.push(rewrite_expr(arg, ctx)?);
            }

            if had_children && rewritten_args.is_empty() && call.callee.as_str() != "only" {
                let is_int_member = matches!(
                    children_member.as_deref(),
                    Some("z" | "weight" | "index" | "len")
                );
                if is_int_member {
                    return Ok(Expr::Literal(Literal::Integer(0, call.span)));
                } else {
                    return Ok(Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, call.span)));
                }
            }

            Ok(Expr::Call(CallExpr {
                callee: call.callee.clone(),
                args: rewritten_args,
                span: call.span,
            }))
        }

        Expr::MethodCall(mc) => {
            let mut rewritten_args = Vec::new();
            for arg in &mc.args {
                rewritten_args.push(rewrite_expr(arg, ctx)?);
            }
            Ok(Expr::MethodCall(MethodCallExpr {
                target: Box::new(rewrite_expr(&mc.target, ctx)?),
                method: mc.method.clone(),
                args: rewritten_args,
                span: mc.span,
            }))
        }

        Expr::Binary(bin) => Ok(Expr::Binary(BinaryExpr {
            op: bin.op,
            left: Box::new(rewrite_expr(&bin.left, ctx)?),
            right: Box::new(rewrite_expr(&bin.right, ctx)?),
            span: bin.span,
        })),

        Expr::Unary(u) => Ok(Expr::Unary(UnaryExpr {
            op: u.op,
            operand: Box::new(rewrite_expr(&u.operand, ctx)?),
            span: u.span,
        })),

        Expr::Paren(inner, span) => {
            Ok(Expr::Paren(Box::new(rewrite_expr(inner, ctx)?), *span))
        }

        Expr::Literal(Literal::Distance(val, unit, span)) => match unit {
            DistanceUnit::Px => Ok(expr.clone()),
            DistanceUnit::Pt => Ok(Expr::Binary(BinaryExpr {
                op: BinaryOp::Mul,
                left: Box::new(Expr::Literal(Literal::Distance(
                    *val,
                    DistanceUnit::Px,
                    *span,
                ))),
                right: Box::new(Expr::Binary(BinaryExpr {
                    op: BinaryOp::Div,
                    left: Box::new(Expr::MemberAccess(MemberAccessExpr {
                        target: Box::new(Expr::Ident(Ident::new(
                            NodeId::WINDOW.canonical_name(),
                            *span,
                        ))),
                        member: Ident::new("dpi", *span),
                        span: *span,
                    })),
                    right: Box::new(Expr::Literal(Literal::Float(72.0, *span))),
                    span: *span,
                })),
                span: *span,
            })),
            DistanceUnit::Em => {
                let font_size_expr = resolve_active_font_size_expr(ctx, *span)?;
                Ok(Expr::Binary(BinaryExpr {
                    op: BinaryOp::Mul,
                    left: Box::new(Expr::Literal(Literal::Float(*val, *span))),
                    right: Box::new(font_size_expr),
                    span: *span,
                }))
            }
        },
        Expr::Literal(_) => Ok(expr.clone()),
        Expr::Node(n) => Ok(Expr::Node(n.clone())),
    }
}

/// Returns the initial default expression for a state binding.
pub fn default_expr_for_state(state: &StateBinding) -> Expr {
    default_expr_for_state_with_enums(state, &HashMap::new())
}

/// Returns the initial default expression for a state binding, resolving enum defaults if needed.
pub fn default_expr_for_state_with_enums(
    state: &StateBinding,
    enums: &HashMap<String, EnumDef>,
) -> Expr {
    let ty_name = state.type_annotation.as_ref().map(|t| t.name.as_str());
    if let Some(def) = &state.default {
        if let Expr::Literal(Literal::Integer(0, span)) = def {
            if ty_name == Some("Distance") {
                return Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, *span));
            } else if ty_name == Some("Area") {
                return Expr::Literal(Literal::Area(0.0, DistanceUnit::Px, *span));
            } else if ty_name == Some("Float") {
                return Expr::Literal(Literal::Float(0.0, *span));
            }
        } else if let Expr::Literal(Literal::Integer(i, span)) = def {
            if ty_name == Some("Float") {
                return Expr::Literal(Literal::Float(*i as f64, *span));
            }
        }
        def.clone()
    } else if let Some(ty) = ty_name {
        match ty {
            "Boolean" => Expr::Literal(Literal::Bool(false, state.span)),
            "String" => Expr::Literal(Literal::String(String::new(), state.span)),
            "Color" => Expr::Literal(Literal::Color("#000000".to_string(), state.span)),
            "Float" => Expr::Literal(Literal::Float(0.0, state.span)),
            "Distance" => Expr::Literal(Literal::Distance(0.0, DistanceUnit::Px, state.span)),
            "Area" => Expr::Literal(Literal::Area(0.0, DistanceUnit::Px, state.span)),
            "Handler" => Expr::Ident(Ident::new("unset", state.span)),
            other => {
                if let Some(enum_def) = enums.get(other) {
                    if let Some(first_var) = enum_def.variants.first() {
                        return Expr::Literal(Literal::Enum(
                            other.to_string(),
                            first_var.as_str().to_string(),
                            state.span,
                        ));
                    }
                }
                Expr::Literal(Literal::Integer(0, state.span))
            }
        }
    } else {
        Expr::Literal(Literal::Integer(0, state.span))
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

/// Helper to parse an event handler binding expression: `target.method` or `method`.
fn extract_event_binding(
    port_name: &str,
    expr: &Expr,
) -> Option<crate::component::EventHandlerBinding> {
    use crate::component::{EventHandlerBinding, EventHandlerTarget};
    match expr {
        Expr::MemberAccess(m) => {
            let target = match m.target.as_ref() {
                Expr::Ident(id) => {
                    if let Some(node_id) = NodeId::from_canonical_name(id.as_str()) {
                        EventHandlerTarget::Component(node_id)
                    } else if id.as_str().starts_with("env.") || id.as_str() == "env" {
                        EventHandlerTarget::Env(m.member.as_str().to_string())
                    } else {
                        return None;
                    }
                }
                Expr::MemberAccess(inner_m) => {
                    if let Expr::Ident(root_id) = inner_m.target.as_ref() {
                        if root_id.as_str() == "env" {
                            EventHandlerTarget::Env(inner_m.member.as_str().to_string())
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                _ => return None,
            };
            Some(EventHandlerBinding {
                name: port_name.to_string(),
                target,
                method: m.member.as_str().to_string(),
            })
        }
        _ => None,
    }
}

fn known_builtin_port_type(node_name: &str, port_name: &str) -> Option<StaticType> {
    if node_name == "Window" {
        match port_name {
            "x" | "y" | "width" | "height" | "left" | "top" | "right" | "bottom"
            | "scroll_x" | "scroll_y" => return Some(StaticType::Distance),
            "dpi" => return Some(StaticType::Float),
            "clip" => return Some(StaticType::Node("Clip".to_string())),
            _ => {}
        }
    }
    match port_name {
        "x"
        | "y"
        | "x1"
        | "y1"
        | "x2"
        | "y2"
        | "start_x"
        | "start_y"
        | "end_x"
        | "end_y"
        | "width"
        | "height"
        | "left"
        | "top"
        | "right"
        | "bottom"
        | "size"
        | "line_height"
        | "baseline"
        | "cap_height"
        | "x_height"
        | "ascent"
        | "descent"
        | "em_top"
        | "line_gap"
        | "gap_bottom"
        | "radius"
        | "corner_radius"
        | "radius_left"
        | "radius_right"
        | "radius_top"
        | "radius_bottom"
        | "left_radius"
        | "right_radius"
        | "top_radius"
        | "bottom_radius"
        | "radius_top_left"
        | "radius_top_right"
        | "radius_bottom_right"
        | "radius_bottom_left"
        | "top_left_radius"
        | "top_right_radius"
        | "bottom_right_radius"
        | "bottom_left_radius"
        | "border_width"
        | "thickness"
        | "stroke_width"
        | "head_length"
        | "head_width"
        | "length"
        | "scroll_x"
        | "scroll_y" => Some(StaticType::Distance),
        "z" | "weight" => Some(StaticType::Integer),
        "opacity" => Some(StaticType::Float),
        "color" | "bg_color" | "border_color" | "stroke_color" => Some(StaticType::Color),
        "family" | "text" | "content" | "cursor" | "url" | "name" | "path" | "id" => {
            Some(StaticType::String)
        }
        "underline" => Some(StaticType::Boolean),
        "clip" | "up" => Some(StaticType::Node("Clip".to_string())),
        "font" => Some(StaticType::Node("Font".to_string())),
        "text_bounds" => Some(StaticType::Node("TextBounds".to_string())),
        "start_at" => Some(StaticType::Enum("TextStart".to_string())),
        "end_at" => Some(StaticType::Enum("TextEnd".to_string())),
        "cap" | "line_cap" => Some(StaticType::Enum("LineCap".to_string())),
        p if p.starts_with("on_") => Some(StaticType::Handler),
        _ => None,
    }
}

/// Strictly type-checks all expanded node ports, window ports, and local bindings at compile time.
pub fn typecheck_expanded_document(
    doc: &mut ExpandedDocument,
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
) -> Result<(), CompileError> {
    let mut cache: HashMap<(NodeId, String), StaticType> = HashMap::new();
    let mut visiting: HashSet<(NodeId, String)> = HashSet::new();

    // 1. Validate and infer all window ports
    let window_port_names: Vec<String> = doc.window_ports.keys().cloned().collect();
    for port_name in window_port_names {
        let inferred = infer_port_type(
            NodeId::WINDOW,
            &port_name,
            doc,
            enums,
            registry,
            &mut cache,
            &mut visiting,
        )?;
        doc.window_port_types
            .entry(port_name)
            .or_insert_with(|| inferred.type_name());
    }

    // 2. Validate and infer all node ports
    for idx in 0..doc.nodes.len() {
        let node_id = NodeId(idx);
        let port_names: Vec<String> = doc.nodes[idx].ports.keys().cloned().collect();
        for port_name in port_names {
            let inferred = infer_port_type(
                node_id,
                &port_name,
                doc,
                enums,
                registry,
                &mut cache,
                &mut visiting,
            )?;
            doc.nodes[idx]
                .port_types
                .entry(port_name)
                .or_insert_with(|| inferred.type_name());
        }
    }

    // 3. Validate and infer all local bindings (\let, \env, \alias)
    let local_bindings = doc.local_bindings.clone();
    for (expr, explicit_type, span) in local_bindings {
        let actual = infer_expr_type(&expr, doc, enums, registry, &mut cache, &mut visiting)?;
        if let Some(expected_name) = explicit_type {
            let expected = StaticType::from_type_name(&expected_name, span, enums, registry)?;
            if !actual.matches(&expected) {
                return Err(CompileError::TypeMismatch {
                    expected: expected.type_name(),
                    actual: actual.type_name(),
                    span: expr.span(),
                });
            }
        }
    }

    Ok(())
}

fn infer_port_type(
    node_id: NodeId,
    port_name: &str,
    doc: &ExpandedDocument,
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
    cache: &mut HashMap<(NodeId, String), StaticType>,
    visiting: &mut HashSet<(NodeId, String)>,
) -> Result<StaticType, CompileError> {
    let key = (node_id, port_name.to_string());
    if let Some(cached) = cache.get(&key) {
        return Ok(cached.clone());
    }

    let explicit_type_name = if node_id.is_window() {
        doc.window_port_types.get(port_name).cloned()
    } else {
        doc.get_node(node_id)
            .and_then(|n| n.port_types.get(port_name).cloned())
    };

    let explicit_ty = if let Some(ref name) = explicit_type_name {
        Some(StaticType::from_type_name(
            name,
            Span::default(),
            enums,
            registry,
        )?)
    } else {
        None
    };

    if !visiting.insert(key.clone()) {
        // Cycle detected: if there is an explicit or builtin type, return it; otherwise let graph cycle checker report it
        if let Some(ty) = explicit_ty {
            return Ok(ty);
        }
        let node_name = if node_id.is_window() {
            "Window"
        } else {
            doc.get_node(node_id).map(|n| n.name.as_str()).unwrap_or("")
        };
        if let Some(builtin) = known_builtin_port_type(node_name, port_name) {
            return Ok(builtin);
        }
        return Ok(StaticType::Float);
    }

    let expr_opt = if node_id.is_window() {
        doc.window_ports.get(port_name)
    } else {
        doc.get_node(node_id).and_then(|n| n.ports.get(port_name))
    };

    let result = if let Some(expr) = expr_opt {
        let actual = infer_expr_type(expr, doc, enums, registry, cache, visiting)?;
        if let Some(expected) = explicit_ty {
            if !actual.matches(&expected) {
                visiting.remove(&key);
                return Err(CompileError::TypeMismatch {
                    expected: expected.type_name(),
                    actual: actual.type_name(),
                    span: expr.span(),
                });
            }
            Ok(expected)
        } else {
            let node_name = if node_id.is_window() {
                "Window"
            } else {
                doc.get_node(node_id).map(|n| n.name.as_str()).unwrap_or("")
            };
            if let Some(builtin) = known_builtin_port_type(node_name, port_name) {
                if !node_id.is_window() && doc.get_node(node_id).is_some_and(|n| !n.is_component) {
                    if node_name == "Text" && port_name == "font" && actual == StaticType::String {
                        Ok(StaticType::String)
                    } else if !actual.matches(&builtin) {
                        visiting.remove(&key);
                        return Err(CompileError::TypeMismatch {
                            expected: builtin.type_name(),
                            actual: actual.type_name(),
                            span: expr.span(),
                        });
                    } else {
                        Ok(builtin)
                    }
                } else {
                    Ok(actual.decay())
                }
            } else {
                Ok(actual.decay())
            }
        }
    } else if let Some(expected) = explicit_ty {
        Ok(expected)
    } else {
        let node_name = if node_id.is_window() {
            "Window"
        } else {
            doc.get_node(node_id).map(|n| n.name.as_str()).unwrap_or("")
        };
        if let Some(builtin) = known_builtin_port_type(node_name, port_name) {
            Ok(builtin)
        } else {
            visiting.remove(&key);
            return Err(CompileError::Custom {
                message: format!(
                    "Node '{}' has no known type for property '{}'",
                    node_name, port_name
                ),
                span: Span::default(),
            });
        }
    };

    visiting.remove(&key);
    if let Ok(ref ty) = result {
        cache.insert(key, ty.clone());
    }
    result
}

fn infer_expr_type(
    expr: &Expr,
    doc: &ExpandedDocument,
    enums: &HashMap<String, EnumDef>,
    registry: &HashMap<String, Vec<ComponentDef>>,
    cache: &mut HashMap<(NodeId, String), StaticType>,
    visiting: &mut HashSet<(NodeId, String)>,
) -> Result<StaticType, CompileError> {
    match expr {
        Expr::Literal(lit) => Ok(match lit {
            Literal::Integer(0, _) => StaticType::ZeroInteger,
            Literal::Integer(_, _) => StaticType::Integer,
            Literal::Float(_, _) => StaticType::Float,
            Literal::Distance(_, _, _) => StaticType::Distance,
            Literal::Area(_, _, _) => StaticType::Area,
            Literal::String(_, _) => StaticType::String,
            Literal::Bool(_, _) => StaticType::Boolean,
            Literal::Color(_, _) => StaticType::Color,
            Literal::Enum(enum_name, _, _) => StaticType::Enum(enum_name.clone()),
        }),
        Expr::Paren(inner, _) => infer_expr_type(inner, doc, enums, registry, cache, visiting),
        Expr::Ident(id) => {
            if id.as_str() == "unset" || id.as_str() == "None" {
                return Ok(StaticType::Handler);
            }
            if id.as_str() == "window" {
                return Ok(StaticType::Node("Window".to_string()));
            }
            if let Some(node_id) = NodeId::from_canonical_name(id.as_str()) {
                if node_id.is_window() {
                    return Ok(StaticType::Node("Window".to_string()));
                }
                if let Some(node) = doc.get_node(node_id) {
                    return Ok(StaticType::Node(node.name.clone()));
                }
            }
            Err(CompileError::Custom {
                message: format!("Cannot determine static type for identifier '{}'", id.as_str()),
                span: id.span,
            })
        }
        Expr::MemberAccess(m) => {
            if let Expr::Ident(target_id) = m.target.as_ref() {
                if let Some(enum_def) = enums.get(target_id.as_str()) {
                    if enum_def.has_variant(m.member.as_str()) {
                        return Ok(StaticType::Enum(target_id.as_str().to_string()));
                    }
                }
                if let Some(node_id) = NodeId::from_canonical_name(target_id.as_str()) {
                    let has_port = if node_id.is_window() {
                        doc.window_ports.contains_key(m.member.as_str())
                            || known_builtin_port_type("Window", m.member.as_str()).is_some()
                    } else if let Some(node) = doc.get_node(node_id) {
                        node.ports.contains_key(m.member.as_str())
                            || node.port_types.contains_key(m.member.as_str())
                            || (!node.is_component
                                && known_builtin_port_type(&node.name, m.member.as_str()).is_some())
                    } else {
                        false
                    };
                    if has_port {
                        return infer_port_type(
                            node_id,
                            m.member.as_str(),
                            doc,
                            enums,
                            registry,
                            cache,
                            visiting,
                        );
                    }
                    // Check if it's an event handler method on a component node
                    if let Some(node) = doc.get_node(node_id) {
                        if node.is_component {
                            return Ok(StaticType::Handler);
                        }
                    }
                }
            }
            let target_ty = infer_expr_type(&m.target, doc, enums, registry, cache, visiting)?;
            match target_ty {
                StaticType::Node(ref node_name) => {
                    if let Some(builtin) = known_builtin_port_type(node_name, m.member.as_str()) {
                        return Ok(builtin);
                    }
                    if let Some(overloads) = registry.get(node_name) {
                        for comp in overloads {
                            if let Some(param) =
                                comp.params.iter().find(|p| p.name.as_str() == m.member.as_str())
                            {
                                if let Some(ref t) = param.type_annotation {
                                    if let Ok(st) = StaticType::from_type_name(
                                        t.name.as_str(),
                                        t.span,
                                        enums,
                                        registry,
                                    ) {
                                        return Ok(st);
                                    }
                                }
                            }
                        }
                        return Ok(StaticType::Handler);
                    }
                    Err(CompileError::Custom {
                        message: format!(
                            "Node '{}' has no known property or method '{}'",
                            node_name, m.member
                        ),
                        span: m.span,
                    })
                }
                StaticType::ViewPort => match m.member.as_str() {
                    "x" | "y" | "width" | "height" | "left" | "top" | "right" | "bottom" => {
                        Ok(StaticType::Distance)
                    }
                    _ => Err(CompileError::Custom {
                        message: format!("ViewPort has no property '{}'", m.member),
                        span: m.span,
                    }),
                },
                other => Err(CompileError::Custom {
                    message: format!(
                        "Cannot access member '{}' on value of type '{}'",
                        m.member,
                        other.type_name()
                    ),
                    span: m.span,
                }),
            }
        }
        Expr::Unary(u) => {
            let op_ty = infer_expr_type(&u.operand, doc, enums, registry, cache, visiting)?;
            match u.op {
                UnaryOp::Neg => match op_ty.decay() {
                    StaticType::Integer => Ok(StaticType::Integer),
                    StaticType::Float => Ok(StaticType::Float),
                    StaticType::Distance => Ok(StaticType::Distance),
                    StaticType::Area => Ok(StaticType::Area),
                    other => Err(CompileError::TypeMismatch {
                        expected: "Float, Integer, Distance, or Area".to_string(),
                        actual: other.type_name(),
                        span: u.span,
                    }),
                },
                UnaryOp::Not => match op_ty.decay() {
                    StaticType::Boolean => Ok(StaticType::Boolean),
                    other => Err(CompileError::TypeMismatch {
                        expected: "Boolean".to_string(),
                        actual: other.type_name(),
                        span: u.span,
                    }),
                },
            }
        }
        Expr::Binary(bin) => {
            let left_ty = infer_expr_type(&bin.left, doc, enums, registry, cache, visiting)?;
            let right_ty = infer_expr_type(&bin.right, doc, enums, registry, cache, visiting)?;
            match bin.op {
                BinaryOp::Add => match (&left_ty, &right_ty) {
                    (StaticType::String, _) | (_, StaticType::String) => Ok(StaticType::String),
                    (StaticType::Distance, StaticType::Distance)
                    | (StaticType::Distance, StaticType::ZeroInteger)
                    | (StaticType::ZeroInteger, StaticType::Distance) => Ok(StaticType::Distance),
                    (StaticType::Area, StaticType::Area)
                    | (StaticType::Area, StaticType::ZeroInteger)
                    | (StaticType::ZeroInteger, StaticType::Area) => Ok(StaticType::Area),
                    (StaticType::Integer | StaticType::ZeroInteger, StaticType::Integer | StaticType::ZeroInteger) => {
                        Ok(StaticType::Integer)
                    }
                    (
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                    ) => Ok(StaticType::Float),
                    (StaticType::Distance, other) => Err(CompileError::TypeMismatch {
                        expected: "Distance".to_string(),
                        actual: other.type_name(),
                        span: bin.right.span(),
                    }),
                    (other, StaticType::Distance) => Err(CompileError::TypeMismatch {
                        expected: "Distance".to_string(),
                        actual: other.type_name(),
                        span: bin.left.span(),
                    }),
                    (StaticType::Area, other) => Err(CompileError::TypeMismatch {
                        expected: "Area".to_string(),
                        actual: other.type_name(),
                        span: bin.right.span(),
                    }),
                    (other, StaticType::Area) => Err(CompileError::TypeMismatch {
                        expected: "Area".to_string(),
                        actual: other.type_name(),
                        span: bin.left.span(),
                    }),
                    _ => Err(CompileError::TypeMismatch {
                        expected: left_ty.type_name(),
                        actual: right_ty.type_name(),
                        span: bin.span,
                    }),
                },
                BinaryOp::Sub => match (&left_ty, &right_ty) {
                    (StaticType::Distance, StaticType::Distance)
                    | (StaticType::Distance, StaticType::ZeroInteger)
                    | (StaticType::ZeroInteger, StaticType::Distance) => Ok(StaticType::Distance),
                    (StaticType::Area, StaticType::Area)
                    | (StaticType::Area, StaticType::ZeroInteger)
                    | (StaticType::ZeroInteger, StaticType::Area) => Ok(StaticType::Area),
                    (StaticType::Integer | StaticType::ZeroInteger, StaticType::Integer | StaticType::ZeroInteger) => {
                        Ok(StaticType::Integer)
                    }
                    (
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                    ) => Ok(StaticType::Float),
                    (StaticType::Distance, other) => Err(CompileError::TypeMismatch {
                        expected: "Distance".to_string(),
                        actual: other.type_name(),
                        span: bin.right.span(),
                    }),
                    (other, StaticType::Distance) => Err(CompileError::TypeMismatch {
                        expected: "Distance".to_string(),
                        actual: other.type_name(),
                        span: bin.left.span(),
                    }),
                    (StaticType::Area, other) => Err(CompileError::TypeMismatch {
                        expected: "Area".to_string(),
                        actual: other.type_name(),
                        span: bin.right.span(),
                    }),
                    (other, StaticType::Area) => Err(CompileError::TypeMismatch {
                        expected: "Area".to_string(),
                        actual: other.type_name(),
                        span: bin.left.span(),
                    }),
                    _ => Err(CompileError::TypeMismatch {
                        expected: left_ty.type_name(),
                        actual: right_ty.type_name(),
                        span: bin.span,
                    }),
                },
                BinaryOp::Mul => match (&left_ty, &right_ty) {
                    (StaticType::Distance, StaticType::Distance) => Ok(StaticType::Area),
                    (
                        StaticType::Distance,
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                    )
                    | (
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                        StaticType::Distance,
                    ) => Ok(StaticType::Distance),
                    (
                        StaticType::Area,
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                    )
                    | (
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                        StaticType::Area,
                    ) => Ok(StaticType::Area),
                    (StaticType::Integer | StaticType::ZeroInteger, StaticType::Integer | StaticType::ZeroInteger) => {
                        Ok(StaticType::Integer)
                    }
                    (
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                    ) => Ok(StaticType::Float),
                    _ => Err(CompileError::TypeMismatch {
                        expected: "Float, Integer, or Distance".to_string(),
                        actual: format!("{} * {}", left_ty.type_name(), right_ty.type_name()),
                        span: bin.span,
                    }),
                },
                BinaryOp::Div => match (&left_ty, &right_ty) {
                    (StaticType::Distance, StaticType::Distance)
                    | (StaticType::Area, StaticType::Area) => Ok(StaticType::Float),
                    (StaticType::Area, StaticType::Distance) => Ok(StaticType::Distance),
                    (
                        StaticType::Distance,
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                    ) => Ok(StaticType::Distance),
                    (
                        StaticType::Area,
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                    ) => Ok(StaticType::Area),
                    (
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                    ) => Ok(StaticType::Float),
                    _ => Err(CompileError::TypeMismatch {
                        expected: "Float, Integer, Distance, or Area".to_string(),
                        actual: format!("{} / {}", left_ty.type_name(), right_ty.type_name()),
                        span: bin.span,
                    }),
                },
                BinaryOp::Rem => match (&left_ty, &right_ty) {
                    (StaticType::Distance, StaticType::Distance) => Ok(StaticType::Distance),
                    (StaticType::Area, StaticType::Area) => Ok(StaticType::Area),
                    (
                        StaticType::Integer | StaticType::ZeroInteger,
                        StaticType::Integer | StaticType::ZeroInteger,
                    ) => Ok(StaticType::Integer),
                    (
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                        StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                    ) => Ok(StaticType::Float),
                    _ => Err(CompileError::TypeMismatch {
                        expected: "Float or Integer".to_string(),
                        actual: format!("{} % {}", left_ty.type_name(), right_ty.type_name()),
                        span: bin.span,
                    }),
                },
                BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                    match (&left_ty, &right_ty) {
                        (StaticType::Distance, StaticType::Distance)
                        | (StaticType::Distance, StaticType::ZeroInteger)
                        | (StaticType::ZeroInteger, StaticType::Distance)
                        | (StaticType::Area, StaticType::Area)
                        | (StaticType::Area, StaticType::ZeroInteger)
                        | (StaticType::ZeroInteger, StaticType::Area)
                        | (
                            StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                            StaticType::Float | StaticType::Integer | StaticType::ZeroInteger,
                        ) => Ok(StaticType::Boolean),
                        (StaticType::Distance, other) => Err(CompileError::TypeMismatch {
                            expected: "Distance".to_string(),
                            actual: other.type_name(),
                            span: bin.right.span(),
                        }),
                        (other, StaticType::Distance) => Err(CompileError::TypeMismatch {
                            expected: "Distance".to_string(),
                            actual: other.type_name(),
                            span: bin.left.span(),
                        }),
                        (StaticType::Area, other) => Err(CompileError::TypeMismatch {
                            expected: "Area".to_string(),
                            actual: other.type_name(),
                            span: bin.right.span(),
                        }),
                        (other, StaticType::Area) => Err(CompileError::TypeMismatch {
                            expected: "Area".to_string(),
                            actual: other.type_name(),
                            span: bin.left.span(),
                        }),
                        _ => Err(CompileError::TypeMismatch {
                            expected: left_ty.type_name(),
                            actual: right_ty.type_name(),
                            span: bin.span,
                        }),
                    }
                }
                BinaryOp::Eq | BinaryOp::Ne => {
                    if left_ty.matches(&right_ty) || right_ty.matches(&left_ty) {
                        Ok(StaticType::Boolean)
                    } else if matches!(
                        (&left_ty, &right_ty),
                        (StaticType::Color, StaticType::String)
                            | (StaticType::String, StaticType::Color)
                    ) {
                        Ok(StaticType::Boolean)
                    } else {
                        Err(CompileError::TypeMismatch {
                            expected: left_ty.type_name(),
                            actual: right_ty.type_name(),
                            span: bin.span,
                        })
                    }
                }
                BinaryOp::And | BinaryOp::Or => {
                    if left_ty != StaticType::Boolean {
                        return Err(CompileError::TypeMismatch {
                            expected: "Boolean".to_string(),
                            actual: left_ty.type_name(),
                            span: bin.left.span(),
                        });
                    }
                    if right_ty != StaticType::Boolean {
                        return Err(CompileError::TypeMismatch {
                            expected: "Boolean".to_string(),
                            actual: right_ty.type_name(),
                            span: bin.right.span(),
                        });
                    }
                    Ok(StaticType::Boolean)
                }
            }
        }
        Expr::Ternary(tern) => {
            let cond_ty = infer_expr_type(&tern.condition, doc, enums, registry, cache, visiting)?;
            if cond_ty != StaticType::Boolean {
                return Err(CompileError::TypeMismatch {
                    expected: "Boolean".to_string(),
                    actual: cond_ty.type_name(),
                    span: tern.condition.span(),
                });
            }
            let then_ty = infer_expr_type(&tern.then_expr, doc, enums, registry, cache, visiting)?;
            let else_ty = infer_expr_type(&tern.else_expr, doc, enums, registry, cache, visiting)?;
            if then_ty == else_ty {
                Ok(then_ty.decay())
            } else if then_ty.matches(&else_ty) {
                Ok(else_ty.decay())
            } else if else_ty.matches(&then_ty) {
                Ok(then_ty.decay())
            } else if matches!(
                (&then_ty, &else_ty),
                (StaticType::Node(a), StaticType::Node(b))
                    if matches!(a.as_str(), "Window" | "ScrollView")
                        && matches!(b.as_str(), "Window" | "ScrollView")
            ) {
                Ok(StaticType::ViewPort)
            } else {
                Err(CompileError::TypeMismatch {
                    expected: then_ty.type_name(),
                    actual: else_ty.type_name(),
                    span: tern.else_expr.span(),
                })
            }
        }
        Expr::Call(call) => {
            match call.callee.as_str() {
                "distance_to_float" | "to_px" | "to_pt" | "to_float" | "to_number" => {
                    for arg in &call.args {
                        let _ = infer_expr_type(arg, doc, enums, registry, cache, visiting)?;
                    }
                    Ok(StaticType::Float)
                }
                "to_integer" => {
                    for arg in &call.args {
                        let _ = infer_expr_type(arg, doc, enums, registry, cache, visiting)?;
                    }
                    Ok(StaticType::Integer)
                }
                "sqrt" => {
                    let mut first_ty = None;
                    for (i, arg) in call.args.iter().enumerate() {
                        let t = infer_expr_type(arg, doc, enums, registry, cache, visiting)?;
                        if i == 0 {
                            first_ty = Some(t);
                        }
                    }
                    if matches!(first_ty, Some(StaticType::Area)) {
                        Ok(StaticType::Distance)
                    } else {
                        Ok(StaticType::Float)
                    }
                }
                "round" | "floor" | "ceil" => {
                    let mut first_ty = None;
                    for (i, arg) in call.args.iter().enumerate() {
                        let t = infer_expr_type(arg, doc, enums, registry, cache, visiting)?;
                        if i == 0 {
                            first_ty = Some(t);
                        }
                    }
                    match first_ty {
                        Some(StaticType::Distance) => Ok(StaticType::Distance),
                        Some(StaticType::Area) => Ok(StaticType::Area),
                        _ => Ok(StaticType::Integer),
                    }
                }
                "to_fixed" | "to_precision" => {
                    for arg in &call.args {
                        let _ = infer_expr_type(arg, doc, enums, registry, cache, visiting)?;
                    }
                    Ok(StaticType::String)
                }
                "hsl" | "hsla" | "rgb" | "rgba" => {
                    for arg in &call.args {
                        let _ = infer_expr_type(arg, doc, enums, registry, cache, visiting)?;
                    }
                    Ok(StaticType::Color)
                }
                "text_width"
                | "text_height"
                | "text_baseline_offset"
                | "font_cap_height"
                | "font_x_height"
                | "font_ascent"
                | "font_descent"
                | "font_line_gap"
                | "font_line_height" => {
                    for arg in &call.args {
                        let _ = infer_expr_type(arg, doc, enums, registry, cache, visiting)?;
                    }
                    Ok(StaticType::Distance)
                }
                "max" | "min" | "sum" | "abs" | "clamp" | "only" | "hypot" => {
                    if call.args.is_empty() {
                        return Ok(StaticType::Distance);
                    }
                    let mut acc_ty =
                        infer_expr_type(&call.args[0], doc, enums, registry, cache, visiting)?;
                    for arg in &call.args[1..] {
                        let arg_ty = infer_expr_type(arg, doc, enums, registry, cache, visiting)?;
                        if arg_ty.matches(&acc_ty) {
                            // keep acc_ty
                        } else if acc_ty.matches(&arg_ty) {
                            acc_ty = arg_ty;
                        } else {
                            return Err(CompileError::TypeMismatch {
                                expected: acc_ty.type_name(),
                                actual: arg_ty.type_name(),
                                span: arg.span(),
                            });
                        }
                    }
                    if call.callee.as_str() == "hypot" && acc_ty.decay() == StaticType::Integer {
                        Ok(StaticType::Float)
                    } else {
                        Ok(acc_ty.decay())
                    }
                }
                _ => {
                    for arg in &call.args {
                        let _ = infer_expr_type(arg, doc, enums, registry, cache, visiting)?;
                    }
                    Ok(StaticType::Float)
                }
            }
        }
        Expr::MethodCall(mc) => {
            let _ = infer_expr_type(&mc.target, doc, enums, registry, cache, visiting)?;
            for arg in &mc.args {
                let _ = infer_expr_type(arg, doc, enums, registry, cache, visiting)?;
            }
            match mc.method.as_str() {
                "append" | "to_fixed" | "to_precision" => Ok(StaticType::String),
                "to_px" | "to_pt" | "to_float" | "to_number" => Ok(StaticType::Float),
                "to_integer" => Ok(StaticType::Integer),
                _ => Ok(StaticType::Float),
            }
        }
        Expr::Node(elem) => Ok(StaticType::Node(elem.name.as_str().to_string())),
    }
}

