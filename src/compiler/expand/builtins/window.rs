use crate::ast::{ElementNode, EnumDef, Expr, Ident, Item, MemberAccessExpr};
use crate::compiler::error::CompileError;
use crate::compiler::expand::context::{EnvEntry, LexicalBinding};
use crate::compiler::expand::rewrite::default_expr_for_state_with_enums;
use crate::compiler::expand::schema::collect_all_document_lets;
use crate::compiler::expanded::{ExpandedDocument, NodeId};
use std::collections::{HashMap, HashSet};

pub const ALLOWED_WINDOW_PORTS: &[&str] = &["title", "vscroll", "hscroll", "dpi"];

/// Initializes default window port types, scope ports, and `env clip` binding, then processes
/// any top-level `\Window(...)` declaration and top-level `\state` / `\let` / `\env` pre-registrations.
pub(in crate::compiler::expand) fn init_window_and_top_level_declarations(
    doc: &crate::ast::Document,
    enums: &HashMap<String, EnumDef>,
    expanded_doc: &mut ExpandedDocument,
    global_scope: &mut HashMap<String, LexicalBinding>,
    global_env_scope: &mut HashMap<String, EnvEntry>,
) -> Result<Vec<String>, CompileError> {
    // Default universal clip: window.clip
    global_env_scope.insert(
        "clip".to_string(),
        EnvEntry::Bound(Expr::MemberAccess(MemberAccessExpr {
            target: Box::new(Expr::Ident(Ident::new(
                NodeId::WINDOW.canonical_name(),
                doc.span,
            ))),
            member: Ident::new("clip", doc.span),
            span: doc.span,
        })),
    );

    for p in ["x", "y", "width", "height", "left", "top", "right", "bottom"] {
        expanded_doc
            .window_port_types
            .insert(p.to_string(), "Distance".to_string());
    }
    expanded_doc
        .window_port_types
        .insert("z".to_string(), "Integer".to_string());
    expanded_doc
        .window_port_types
        .insert("clip".to_string(), "Clip".to_string());
    expanded_doc
        .window_port_types
        .insert("dpi".to_string(), "Float".to_string());
    expanded_doc
        .window_port_types
        .insert("title".to_string(), "String".to_string());
    expanded_doc
        .window_port_types
        .insert("vscroll".to_string(), "Boolean".to_string());
    expanded_doc
        .window_port_types
        .insert("hscroll".to_string(), "Boolean".to_string());

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
            if declared_top_states.contains_key(&name)
                || expanded_doc.window_ports.contains_key(&name)
            {
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
            let default_expr = default_expr_for_state_with_enums(s, enums);
            let window_ref = Expr::MemberAccess(MemberAccessExpr {
                target: Box::new(Expr::Ident(Ident::new(
                    NodeId::WINDOW.canonical_name(),
                    s.span,
                ))),
                member: s.name.clone(),
                span: s.span,
            });
            global_scope.insert(name.clone(), LexicalBinding::State(window_ref));
            let type_name = s
                .type_annotation
                .as_ref()
                .map(|t| t.name.as_str().to_string());
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

    Ok(window_scope_ports)
}

