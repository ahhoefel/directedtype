//! Automatic component documentation generator (`compdoc` / `dtdoc`).
//!
//! Parses DirectedType `.dt` files directly into their AST, extracts all public declarations
//! (`\Component`, `\Span`, `\InlineBox`, overloads, parameters, `env` parameters, defaults,
//! `\alias` ports, provided `\env` bindings, `\Children` contracts, `\assert` invariants,
//! `\Enum` definitions, top-level `\let`/`\env`/`\state` bindings, and `\use` imports),
//! resolves cross-module internal links, emits a styled DirectedType reference document with
//! an automatic `\TableOfContents` and PDF outlines, and compiles it to an internally linked
//! vector PDF.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::{Path, PathBuf};

use crate::ast::{
    AliasBinding, AssertDirective, BinaryOp, ChildrenDirective, ComponentBodyItem, ComponentDef,
    ComponentKind, ContentItem, Document, ElementNode, EnumDef, EnvBinding, Expr, Item,
    LetBinding, LetValue, Literal, MatchPattern, ParamDef, StateBinding, UnaryOp, UseDeclaration,
};
use crate::error::ParseError;
use crate::parser::parse_document_with_file;
#[cfg(feature = "pdf")]
use crate::pdf::{compile_source_to_pdf, PdfExportError, PdfOptions};

/// Errors produced while scanning, parsing, or compiling component documentation.
#[derive(Debug, thiserror::Error)]
pub enum CompDocError {
    #[error("I/O error for '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("No .dt files found in the specified input path(s)")]
    NoInputFiles,
    #[error("Parse error in '{path}': {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: ParseError,
    },
    #[cfg(feature = "pdf")]
    #[error("PDF export error: {0}")]
    Pdf(#[from] PdfExportError),
}

/// Configuration for automatic component documentation generation.
#[derive(Debug, Clone)]
pub struct DocGenOptions {
    /// Document title shown on the cover header, running headers, and PDF metadata.
    pub title: String,
    /// Optional subtitle shown beneath the document title.
    pub subtitle: Option<String>,
    /// Base directory for computing relative package/module paths and resolving `\use` imports.
    pub root_dir: PathBuf,
}

impl Default for DocGenOptions {
    fn default() -> Self {
        Self {
            title: "DirectedType Component Reference".to_string(),
            subtitle: Some(
                "Automatically generated API reference from component definitions".to_string(),
            ),
            root_dir: PathBuf::from("."),
        }
    }
}

/// Complete extracted documentation catalog across one or more packages.
#[derive(Debug, Clone, PartialEq)]
pub struct DocCatalog {
    pub title: String,
    pub subtitle: Option<String>,
    pub packages: Vec<PackageDoc>,
}

impl DocCatalog {
    /// Total number of documented `.dt` module files.
    pub fn module_count(&self) -> usize {
        self.packages.iter().map(|p| p.modules.len()).sum()
    }

    /// Total number of unique named components (`\Component`, `\Span`, `\InlineBox`).
    pub fn component_count(&self) -> usize {
        self.packages
            .iter()
            .flat_map(|p| &p.modules)
            .map(|m| m.components.len())
            .sum()
    }

    /// Total number of component overload definitions.
    pub fn overload_count(&self) -> usize {
        self.packages
            .iter()
            .flat_map(|p| &p.modules)
            .flat_map(|m| &m.components)
            .map(|c| c.overloads.len())
            .sum()
    }

    /// Total number of documented `\Enum` definitions.
    pub fn enum_count(&self) -> usize {
        self.packages
            .iter()
            .flat_map(|p| &p.modules)
            .map(|m| m.enums.len())
            .sum()
    }

    /// Total number of documented top-level public bindings (`\let`, `\env`, `\state`).
    pub fn binding_count(&self) -> usize {
        self.packages
            .iter()
            .flat_map(|p| &p.modules)
            .map(|m| m.bindings.len())
            .sum()
    }
}

/// A directory-grouped package of `.dt` modules (e.g., `components`, `components/material`).
#[derive(Debug, Clone, PartialEq)]
pub struct PackageDoc {
    pub name: String,
    pub anchor: String,
    pub modules: Vec<ModuleDoc>,
}

/// Extracted public API of a single `.dt` source file.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleDoc {
    pub rel_path: String,
    pub stem: String,
    pub package: String,
    pub anchor: String,
    pub imports: Vec<ImportDoc>,
    pub enums: Vec<EnumDoc>,
    pub components: Vec<ComponentDoc>,
    pub bindings: Vec<BindingDoc>,
}

impl ModuleDoc {
    /// Returns true if this module exports a single component or enum whose name matches the file
    /// stem and has no other components or enums (allowing a compact single-level heading).
    pub fn is_single_matching_symbol(&self) -> bool {
        if !self.bindings.is_empty() {
            return false;
        }
        match (self.components.len(), self.enums.len()) {
            (1, 0) => self.components[0].name == self.stem,
            (0, 1) => self.enums[0].name == self.stem,
            _ => false,
        }
    }
}

/// A `\use` import declaration in a module.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportDoc {
    pub path: String,
    pub alias: Option<String>,
    pub items: Vec<ImportItemDoc>,
    pub target_anchor: Option<String>,
}

/// A named symbol imported in `\use { a, b as c } from "..."`.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportItemDoc {
    pub name: String,
    pub alias: Option<String>,
}

/// A public `\Enum` declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumDoc {
    pub name: String,
    pub anchor: String,
    pub variants: Vec<String>,
}

/// A public component (`\Component`, `\Span`, or `\InlineBox`) with one or more overloads.
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentDoc {
    pub name: String,
    pub anchor: String,
    pub kind: ComponentKind,
    pub overloads: Vec<OverloadDoc>,
}

/// How many children a component overload accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildrenCardinality {
    /// Does not accept children (leaf component).
    None,
    /// Accepts at most one child (e.g. `\assert children.len == 1`, `<= 1`, or `only(children.*)`).
    Single,
    /// Accepts zero or more children (`\Children`).
    Multiple,
}

/// A single overload signature and public interface of a component.
#[derive(Debug, Clone, PartialEq)]
pub struct OverloadDoc {
    pub anchor: String,
    pub kind: ComponentKind,
    pub params: Vec<ParamDoc>,
    pub aliases: Vec<AliasDoc>,
    pub provided_envs: Vec<EnvDoc>,
    pub accepts_children: bool,
    pub children_cardinality: ChildrenCardinality,
    pub children_pushed_ports: Vec<ChildPortDoc>,
    pub asserts: Vec<AssertDoc>,
}

/// A port explicitly pushed onto children via `\Children(port: expr, ...)`.
#[derive(Debug, Clone, PartialEq)]
pub struct ChildPortDoc {
    pub name: String,
    pub type_name: Option<String>,
    pub type_anchor: Option<String>,
    pub expr: String,
}

/// A parameter in a component signature.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamDoc {
    pub is_env: bool,
    pub name: String,
    pub type_name: Option<String>,
    pub type_anchor: Option<String>,
    pub default_value: Option<String>,
}

/// A public read-only `\alias` port exported by a component.
#[derive(Debug, Clone, PartialEq)]
pub struct AliasDoc {
    pub name: String,
    pub type_name: Option<String>,
    pub type_anchor: Option<String>,
    pub expr: String,
}

/// An `\env` binding provided by a component to its descendant subtree.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvDoc {
    pub name: String,
    pub type_name: Option<String>,
    pub type_anchor: Option<String>,
    pub expr: Option<String>,
}

/// A contract assertion (`\assert condition [as "message"];`) inside a component.
#[derive(Debug, Clone, PartialEq)]
pub struct AssertDoc {
    pub condition: String,
    pub message: Option<String>,
}

/// Kind of a top-level exported value binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingKind {
    Let,
    Env,
    State,
}

impl BindingKind {
    pub fn keyword(self) -> &'static str {
        match self {
            BindingKind::Let => "\\let",
            BindingKind::Env => "\\env",
            BindingKind::State => "\\state",
        }
    }
}

/// A top-level exported `\let`, `\env`, or `\state` binding in a module.
#[derive(Debug, Clone, PartialEq)]
pub struct BindingDoc {
    pub kind: BindingKind,
    pub name: String,
    pub anchor: String,
    pub type_name: Option<String>,
    pub type_anchor: Option<String>,
    pub value: Option<String>,
}

/// Discovers all `.dt` files under `inputs` (files or directories) recursively in deterministic order.
pub fn discover_dt_files(inputs: &[PathBuf]) -> Result<Vec<PathBuf>, CompDocError> {
    let mut out = Vec::new();
    for input in inputs {
        collect_dt_files(input, &mut out)?;
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn collect_dt_files(path: &Path, out: &mut Vec<PathBuf>) -> Result<(), CompDocError> {
    if path.is_file() {
        if path.extension().and_then(|s| s.to_str()) == Some("dt") {
            out.push(path.to_path_buf());
        }
        return Ok(());
    }
    let entries = fs::read_dir(path).map_err(|source| CompDocError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut subpaths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    subpaths.sort();

    for p in subpaths {
        if p.is_dir() {
            collect_dt_files(&p, out)?;
        } else if p.extension().and_then(|s| s.to_str()) == Some("dt") {
            out.push(p);
        }
    }
    Ok(())
}

/// Extracts a complete, cross-referenced `DocCatalog` from the given `.dt` files or directories.
pub fn extract_catalog(
    inputs: &[PathBuf],
    options: &DocGenOptions,
) -> Result<DocCatalog, CompDocError> {
    let files = discover_dt_files(inputs)?;
    if files.is_empty() {
        return Err(CompDocError::NoInputFiles);
    }

    let mut packages_map: BTreeMap<String, Vec<ModuleDoc>> = BTreeMap::new();
    let mut parsed_docs: HashMap<String, Document> = HashMap::new();
    let mut used_anchors = HashSet::new();

    for file_path in &files {
        let source = fs::read_to_string(file_path).map_err(|source| CompDocError::Io {
            path: file_path.clone(),
            source,
        })?;
        let doc = parse_document_with_file(&source, file_path).map_err(|source| {
            CompDocError::Parse {
                path: file_path.clone(),
                source,
            }
        })?;

        let rel_path = compute_rel_path(file_path, &options.root_dir);
        let norm_rel = normalize_rel_str(&rel_path);
        let module = extract_module(&doc, &rel_path, &mut used_anchors);
        parsed_docs.insert(norm_rel, doc);
        packages_map
            .entry(module.package.clone())
            .or_default()
            .push(module);
    }

    let mut package_entries: Vec<(String, Vec<ModuleDoc>)> = packages_map.into_iter().collect();
    // Sort shallower packages first (e.g. `components` before `components/material`), then alphabetically.
    package_entries.sort_by(|(a, _), (b, _)| {
        let depth_a = a.split('/').count();
        let depth_b = b.split('/').count();
        depth_a.cmp(&depth_b).then_with(|| a.cmp(b))
    });

    let mut packages = Vec::with_capacity(package_entries.len());
    for (pkg_name, mut modules) in package_entries {
        modules.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        let anchor = unique_anchor(
            &format!("pkg-{}", slugify(&pkg_name)),
            &mut used_anchors,
        );
        packages.push(PackageDoc {
            name: pkg_name,
            anchor,
            modules,
        });
    }

    let mut catalog = DocCatalog {
        title: options.title.clone(),
        subtitle: options.subtitle.clone(),
        packages,
    };

    infer_catalog_types(&mut catalog, &parsed_docs);
    strip_language_generated_aliases(&mut catalog);
    resolve_cross_references(&mut catalog);
    Ok(catalog)
}

fn compute_rel_path(path: &Path, root_dir: &Path) -> String {
    let rel = path.strip_prefix(root_dir).unwrap_or(path);
    let cleaned = rel.to_string_lossy().replace('\\', "/");
    cleaned.trim_start_matches("./").to_string()
}

fn extract_module(
    doc: &Document,
    rel_path: &str,
    used_anchors: &mut HashSet<String>,
) -> ModuleDoc {
    let p = Path::new(rel_path);
    let stem = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(rel_path)
        .to_string();
    let package = p
        .parent()
        .map(|parent| parent.to_string_lossy().replace('\\', "/"))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| ".".to_string());

    let mod_slug = slugify(rel_path.trim_end_matches(".dt"));
    let mod_anchor = unique_anchor(&format!("mod-{mod_slug}"), used_anchors);

    let mut imports = Vec::new();
    let mut enums = Vec::new();
    let mut components: Vec<ComponentDoc> = Vec::new();
    let mut comp_indices: HashMap<String, usize> = HashMap::new();
    let mut bindings = Vec::new();

    for item in &doc.items {
        match item {
            Item::Use(u) => imports.push(extract_import(u)),
            Item::Enum(e) => enums.push(extract_enum(e, &mod_slug, used_anchors)),
            Item::Component(c) => {
                let name = c.name.as_str().to_string();
                if let Some(&idx) = comp_indices.get(&name) {
                    let ov_num = components[idx].overloads.len() + 1;
                    let ov_anchor = unique_anchor(
                        &format!("{}-ov-{ov_num}", components[idx].anchor),
                        used_anchors,
                    );
                    let overload = extract_overload(c, ov_anchor);
                    components[idx].overloads.push(overload);
                } else {
                    let sym_anchor = unique_anchor(
                        &format!("sym-{mod_slug}-{}", slugify(&name)),
                        used_anchors,
                    );
                    let ov_anchor = unique_anchor(&format!("{sym_anchor}-ov-1"), used_anchors);
                    let overload = extract_overload(c, ov_anchor);
                    comp_indices.insert(name.clone(), components.len());
                    components.push(ComponentDoc {
                        name,
                        anchor: sym_anchor,
                        kind: c.kind,
                        overloads: vec![overload],
                    });
                }
            }
            Item::Let(l) => bindings.push(extract_let_binding(l, &mod_slug, used_anchors)),
            Item::Env(e) => bindings.push(extract_env_top_binding(e, &mod_slug, used_anchors)),
            Item::State(s) => bindings.push(extract_state_top_binding(s, &mod_slug, used_anchors)),
            Item::Node(_) => {}
        }
    }

    let mut module = ModuleDoc {
        rel_path: rel_path.to_string(),
        stem,
        package,
        anchor: mod_anchor,
        imports,
        enums,
        components,
        bindings,
    };

    // When a module exports a single symbol matching its file stem, share one anchor
    // between the module and that symbol so we never need nested \Anchor wrappers.
    if module.is_single_matching_symbol() {
        if let Some(comp) = module.components.first() {
            module.anchor = comp.anchor.clone();
        } else if let Some(en) = module.enums.first() {
            module.anchor = en.anchor.clone();
        }
    }

    module
}

fn extract_import(u: &UseDeclaration) -> ImportDoc {
    ImportDoc {
        path: u.path.clone(),
        alias: u.alias.as_ref().map(|a| a.as_str().to_string()),
        items: u
            .items
            .iter()
            .map(|it| ImportItemDoc {
                name: it.name.as_str().to_string(),
                alias: it.alias.as_ref().map(|a| a.as_str().to_string()),
            })
            .collect(),
        target_anchor: None,
    }
}

fn extract_enum(
    e: &EnumDef,
    mod_slug: &str,
    used_anchors: &mut HashSet<String>,
) -> EnumDoc {
    let name = e.name.as_str().to_string();
    let anchor = unique_anchor(
        &format!("sym-{mod_slug}-{}", slugify(&name)),
        used_anchors,
    );
    EnumDoc {
        name,
        anchor,
        variants: e.variants.iter().map(|v| v.as_str().to_string()).collect(),
    }
}

fn extract_overload(comp: &ComponentDef, anchor: String) -> OverloadDoc {
    let params = comp.params.iter().map(extract_param).collect();
    let mut aliases = Vec::new();
    let mut provided_envs = Vec::new();
    let mut asserts = Vec::new();
    let mut accepts_children = false;
    let mut children_pushed_ports: Vec<ChildPortDoc> = Vec::new();
    let mut seen_child_ports: HashSet<String> = HashSet::new();

    for item in &comp.body {
        match item {
            ComponentBodyItem::Alias(a) => aliases.push(extract_alias(a)),
            ComponentBodyItem::Env(e) => provided_envs.push(extract_env_doc(e)),
            ComponentBodyItem::Assert(a) => asserts.push(extract_assert(a)),
            _ => {}
        }
    }

    scan_children_usage(
        &comp.body,
        &mut accepts_children,
        &mut children_pushed_ports,
        &mut seen_child_ports,
    );

    let children_cardinality = detect_children_cardinality(comp, accepts_children);

    OverloadDoc {
        anchor,
        kind: comp.kind,
        params,
        aliases,
        provided_envs,
        accepts_children: children_cardinality != ChildrenCardinality::None,
        children_cardinality,
        children_pushed_ports,
        asserts,
    }
}

fn detect_children_cardinality(
    comp: &ComponentDef,
    has_children_directive: bool,
) -> ChildrenCardinality {
    if !has_children_directive {
        return ChildrenCardinality::None;
    }

    // Check explicit `\assert` constraints on `children.len`
    for item in &comp.body {
        if let ComponentBodyItem::Assert(a) = item {
            if is_children_len_zero(&a.condition) {
                return ChildrenCardinality::None;
            }
            if is_children_len_single(&a.condition) {
                return ChildrenCardinality::Single;
            }
        }
    }

    // Check for `only(children.*)` usage anywhere in parameter defaults or body
    for p in &comp.params {
        if let Some(def) = &p.default_edge {
            if expr_uses_only_children(def) {
                return ChildrenCardinality::Single;
            }
        }
    }
    if content_uses_only_children(&comp.body) {
        return ChildrenCardinality::Single;
    }

    ChildrenCardinality::Multiple
}

fn is_children_len_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Paren(inner, _) => is_children_len_expr(inner),
        Expr::MemberAccess(m) => {
            m.member.as_str() == "len"
                && matches!(m.target.as_ref(), Expr::Ident(id) if id.as_str() == "children")
        }
        _ => false,
    }
}

fn int_literal_value(expr: &Expr) -> Option<i64> {
    match expr {
        Expr::Paren(inner, _) => int_literal_value(inner),
        Expr::Literal(Literal::Integer(n, _)) => Some(*n),
        _ => None,
    }
}

fn is_children_len_zero(expr: &Expr) -> bool {
    match expr {
        Expr::Paren(inner, _) => is_children_len_zero(inner),
        Expr::Binary(b) => match b.op {
            BinaryOp::Eq => {
                (is_children_len_expr(&b.left) && int_literal_value(&b.right) == Some(0))
                    || (is_children_len_expr(&b.right) && int_literal_value(&b.left) == Some(0))
            }
            BinaryOp::And => is_children_len_zero(&b.left) || is_children_len_zero(&b.right),
            _ => false,
        },
        _ => false,
    }
}

fn is_children_len_single(expr: &Expr) -> bool {
    match expr {
        Expr::Paren(inner, _) => is_children_len_single(inner),
        Expr::Binary(b) => match b.op {
            BinaryOp::Eq => {
                (is_children_len_expr(&b.left) && int_literal_value(&b.right) == Some(1))
                    || (is_children_len_expr(&b.right) && int_literal_value(&b.left) == Some(1))
            }
            BinaryOp::Le => is_children_len_expr(&b.left) && int_literal_value(&b.right) == Some(1),
            BinaryOp::Ge => int_literal_value(&b.left) == Some(1) && is_children_len_expr(&b.right),
            BinaryOp::And => is_children_len_single(&b.left) || is_children_len_single(&b.right),
            _ => false,
        },
        _ => false,
    }
}

fn expr_refs_children(expr: &Expr) -> bool {
    match expr {
        Expr::Ident(id) => id.as_str() == "children",
        Expr::MemberAccess(m) => expr_refs_children(&m.target),
        Expr::Paren(inner, _) => expr_refs_children(inner),
        _ => false,
    }
}

fn expr_uses_only_children(expr: &Expr) -> bool {
    match expr {
        Expr::Call(c) => {
            (c.callee.as_str() == "only" && c.args.iter().any(expr_refs_children))
                || c.args.iter().any(expr_uses_only_children)
        }
        Expr::MethodCall(mc) => {
            expr_uses_only_children(&mc.target) || mc.args.iter().any(expr_uses_only_children)
        }
        Expr::Binary(b) => expr_uses_only_children(&b.left) || expr_uses_only_children(&b.right),
        Expr::Unary(u) => expr_uses_only_children(&u.operand),
        Expr::Ternary(t) => {
            expr_uses_only_children(&t.condition)
                || expr_uses_only_children(&t.then_expr)
                || expr_uses_only_children(&t.else_expr)
        }
        Expr::MemberAccess(m) => expr_uses_only_children(&m.target),
        Expr::Index(ix) => {
            expr_uses_only_children(&ix.target) || expr_uses_only_children(&ix.index)
        }
        Expr::Paren(inner, _) => expr_uses_only_children(inner),
        Expr::Node(n) => {
            n.ports.iter().any(|p| expr_uses_only_children(&p.expr))
                || n.content
                    .as_ref()
                    .is_some_and(|s| content_uses_only_children(&s.items))
        }
        Expr::ExprDef(ed) => expr_uses_only_children(&ed.body),
        Expr::Match(m) => {
            m.scrutinee.as_deref().is_some_and(expr_uses_only_children)
                || m.arms.iter().any(|a| expr_uses_only_children(&a.body))
        }
        Expr::Literal(_) | Expr::Ident(_) | Expr::DomNodeRef(_, _) => false,
    }
}

fn content_uses_only_children(items: &[ContentItem]) -> bool {
    for item in items {
        let hit = match item {
            ContentItem::Alias(a) => expr_uses_only_children(&a.value),
            ContentItem::Let(l) => match &l.value {
                Some(LetValue::Expr(e)) => expr_uses_only_children(e),
                Some(LetValue::Node(n)) => {
                    n.ports.iter().any(|p| expr_uses_only_children(&p.expr))
                        || n.content
                            .as_ref()
                            .is_some_and(|s| content_uses_only_children(&s.items))
                }
                None => false,
            },
            ContentItem::Env(e) => e.value.as_ref().is_some_and(expr_uses_only_children),
            ContentItem::State(s) => s.default.as_ref().is_some_and(expr_uses_only_children),
            ContentItem::Assert(a) => expr_uses_only_children(&a.condition),
            ContentItem::Children(c) => c.ports.iter().any(|p| expr_uses_only_children(&p.expr)),
            ContentItem::Node(n) => {
                n.ports.iter().any(|p| expr_uses_only_children(&p.expr))
                    || n.content
                        .as_ref()
                        .is_some_and(|s| content_uses_only_children(&s.items))
            }
            ContentItem::Text(_) | ContentItem::ParagraphBreak(_) => false,
        };
        if hit {
            return true;
        }
    }
    false
}

fn scan_children_usage(
    items: &[ContentItem],
    accepts_children: &mut bool,
    pushed_ports: &mut Vec<ChildPortDoc>,
    seen_ports: &mut HashSet<String>,
) {
    for item in items {
        match item {
            ContentItem::Children(ChildrenDirective { ports, .. }) => {
                *accepts_children = true;
                for pb in ports {
                    let pname = pb.name.as_str().to_string();
                    if seen_ports.insert(pname.clone()) {
                        pushed_ports.push(ChildPortDoc {
                            name: pname,
                            type_name: None,
                            type_anchor: None,
                            expr: format_expr(&pb.expr),
                        });
                    }
                }
            }
            ContentItem::Node(node) => {
                if let Some(slot) = &node.content {
                    scan_children_usage(&slot.items, accepts_children, pushed_ports, seen_ports);
                }
            }
            ContentItem::Let(LetBinding {
                value: Some(LetValue::Node(node)),
                ..
            }) => {
                if let Some(slot) = &node.content {
                    scan_children_usage(&slot.items, accepts_children, pushed_ports, seen_ports);
                }
            }
            _ => {}
        }
    }
}

fn collect_children_port_exprs<'a>(
    items: &'a [ContentItem],
    out: &mut HashMap<&'a str, &'a Expr>,
) {
    for item in items {
        match item {
            ContentItem::Children(ChildrenDirective { ports, .. }) => {
                for pb in ports {
                    out.entry(pb.name.as_str()).or_insert(&pb.expr);
                }
            }
            ContentItem::Node(node) => {
                if let Some(slot) = &node.content {
                    collect_children_port_exprs(&slot.items, out);
                }
            }
            ContentItem::Let(LetBinding {
                value: Some(LetValue::Node(node)),
                ..
            }) => {
                if let Some(slot) = &node.content {
                    collect_children_port_exprs(&slot.items, out);
                }
            }
            _ => {}
        }
    }
}

fn extract_param(p: &ParamDef) -> ParamDoc {
    ParamDoc {
        is_env: p.is_env,
        name: p.name.as_str().to_string(),
        type_name: p.type_annotation.as_ref().map(|t| t.name.as_str().to_string()),
        type_anchor: None,
        default_value: p.default_edge.as_ref().map(format_expr),
    }
}

fn extract_alias(a: &AliasBinding) -> AliasDoc {
    AliasDoc {
        name: a.name.as_str().to_string(),
        type_name: a.type_annotation.as_ref().map(|t| t.name.as_str().to_string()),
        type_anchor: None,
        expr: format_expr(&a.value),
    }
}

fn extract_env_doc(e: &EnvBinding) -> EnvDoc {
    EnvDoc {
        name: e.name.as_str().to_string(),
        type_name: e.type_annotation.as_ref().map(|t| t.name.as_str().to_string()),
        type_anchor: None,
        expr: e.value.as_ref().map(format_expr),
    }
}

fn extract_assert(a: &AssertDirective) -> AssertDoc {
    AssertDoc {
        condition: format_expr(&a.condition),
        message: a.message.clone(),
    }
}

fn extract_let_binding(
    l: &LetBinding,
    mod_slug: &str,
    used_anchors: &mut HashSet<String>,
) -> BindingDoc {
    let name = l.name.as_str().to_string();
    let anchor = unique_anchor(
        &format!("sym-{mod_slug}-{}", slugify(&name)),
        used_anchors,
    );
    let inferred_type = l
        .type_annotation
        .as_ref()
        .map(|t| t.name.as_str().to_string())
        .or_else(|| l.value.as_ref().and_then(infer_let_value_type));
    BindingDoc {
        kind: BindingKind::Let,
        name,
        anchor,
        type_name: inferred_type,
        type_anchor: None,
        value: l.value.as_ref().map(format_let_value),
    }
}

fn extract_env_top_binding(
    e: &EnvBinding,
    mod_slug: &str,
    used_anchors: &mut HashSet<String>,
) -> BindingDoc {
    let name = e.name.as_str().to_string();
    let anchor = unique_anchor(
        &format!("sym-{mod_slug}-{}", slugify(&name)),
        used_anchors,
    );
    let inferred_type = e
        .type_annotation
        .as_ref()
        .map(|t| t.name.as_str().to_string())
        .or_else(|| e.value.as_ref().and_then(infer_expr_type));
    BindingDoc {
        kind: BindingKind::Env,
        name,
        anchor,
        type_name: inferred_type,
        type_anchor: None,
        value: e.value.as_ref().map(format_expr),
    }
}

fn extract_state_top_binding(
    s: &StateBinding,
    mod_slug: &str,
    used_anchors: &mut HashSet<String>,
) -> BindingDoc {
    let name = s.name.as_str().to_string();
    let anchor = unique_anchor(
        &format!("sym-{mod_slug}-{}", slugify(&name)),
        used_anchors,
    );
    let inferred_type = s
        .type_annotation
        .as_ref()
        .map(|t| t.name.as_str().to_string())
        .or_else(|| s.default.as_ref().and_then(infer_expr_type));
    BindingDoc {
        kind: BindingKind::State,
        name,
        anchor,
        type_name: inferred_type,
        type_anchor: None,
        value: s.default.as_ref().map(format_expr),
    }
}

fn infer_let_value_type(val: &LetValue) -> Option<String> {
    match val {
        LetValue::Node(n) => Some(n.name.as_str().to_string()),
        LetValue::Expr(e) => infer_expr_type(e),
    }
}

fn infer_expr_type(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Literal(lit) => Some(lit.type_name().to_string()),
        Expr::Node(n) => Some(n.name.as_str().to_string()),
        Expr::ExprDef(_) => Some("Expr".to_string()),
        _ => None,
    }
}

/// Returns true for spatial bounding-box aliases automatically generated by the language.
fn is_language_generated_alias(name: &str) -> bool {
    matches!(name, "left" | "top" | "right" | "bottom")
}

/// Removes language-generated spatial aliases (`left`, `top`, `right`, `bottom`) from documented
/// component overloads after type inference has resolved all dependent aliases.
fn strip_language_generated_aliases(catalog: &mut DocCatalog) {
    for pkg in &mut catalog.packages {
        for m in &mut pkg.modules {
            for c in &mut m.components {
                for ov in &mut c.overloads {
                    ov.aliases.retain(|a| !is_language_generated_alias(&a.name));
                }
            }
        }
    }
}

/// Per-component member type table (component name -> member name -> type name).
type ComponentMemberTypes = HashMap<String, HashMap<String, String>>;

/// Infers static types for all unannotated `\alias` ports, provided `\env` bindings, and top-level
/// `\let`/`\env`/`\state` bindings across the catalog using fixed-point expression inference.
fn infer_catalog_types(catalog: &mut DocCatalog, parsed_docs: &HashMap<String, Document>) {
    // Run 2 catalog-wide passes so cross-component alias accesses (e.g. `style.padding_x`)
    // pick up alias types inferred on pass 1.
    for _pass in 0..2 {
        let mut comp_members: ComponentMemberTypes = HashMap::new();
        let mut enum_names: HashSet<String> = HashSet::new();
        let mut mod_binding_types: HashMap<String, HashMap<String, String>> = HashMap::new();

        for pkg in &catalog.packages {
            for m in &pkg.modules {
                let norm = normalize_rel_str(&m.rel_path);
                for e in &m.enums {
                    enum_names.insert(e.name.clone());
                }
                let bmap = mod_binding_types.entry(norm).or_default();
                for b in &m.bindings {
                    if let Some(ty) = &b.type_name {
                        bmap.insert(b.name.clone(), ty.clone());
                    }
                }
                for c in &m.components {
                    let members = comp_members.entry(c.name.clone()).or_default();
                    for ov in &c.overloads {
                        for p in &ov.params {
                            if let Some(ty) = &p.type_name {
                                members.entry(p.name.clone()).or_insert_with(|| ty.clone());
                            }
                        }
                        for a in &ov.aliases {
                            if let Some(ty) = &a.type_name {
                                members.entry(a.name.clone()).or_insert_with(|| ty.clone());
                            }
                        }
                    }
                }
            }
        }

        let known_mod_paths: HashMap<String, String> = catalog
            .packages
            .iter()
            .flat_map(|p| &p.modules)
            .map(|m| {
                let n = normalize_rel_str(&m.rel_path);
                (n.clone(), n)
            })
            .collect();

        for pkg in &mut catalog.packages {
            for m in &mut pkg.modules {
                let norm = normalize_rel_str(&m.rel_path);
                let Some(doc) = parsed_docs.get(&norm) else {
                    continue;
                };
                let mod_dir = Path::new(&norm).parent().unwrap_or(Path::new(""));

                // Gather visible top-level binding types (imported + local)
                let mut module_scope: HashMap<String, String> = HashMap::new();
                for imp in &m.imports {
                    if let Some(resolved) =
                        resolve_import_target_path(&imp.path, mod_dir, &known_mod_paths)
                    {
                        if let Some(imported_bindings) = mod_binding_types.get(&resolved) {
                            if imp.items.is_empty() {
                                for (k, v) in imported_bindings {
                                    module_scope.insert(k.clone(), v.clone());
                                }
                            } else {
                                for item in &imp.items {
                                    if let Some(v) = imported_bindings.get(&item.name) {
                                        let bound = item
                                            .alias
                                            .as_deref()
                                            .unwrap_or(item.name.as_str())
                                            .to_string();
                                        module_scope.insert(bound, v.clone());
                                    }
                                }
                            }
                        }
                    }
                }
                for b in &m.bindings {
                    if let Some(ty) = &b.type_name {
                        module_scope.insert(b.name.clone(), ty.clone());
                    }
                }

                // Infer unannotated top-level bindings (fixed-point)
                let mut top_exprs: HashMap<String, &Expr> = HashMap::new();
                for item in &doc.items {
                    match item {
                        Item::Let(l) => {
                            if let Some(LetValue::Expr(e)) = &l.value {
                                top_exprs.insert(l.name.as_str().to_string(), e);
                            }
                        }
                        Item::Env(e) => {
                            if let Some(val) = &e.value {
                                top_exprs.insert(e.name.as_str().to_string(), val);
                            }
                        }
                        Item::State(s) => {
                            if let Some(def) = &s.default {
                                top_exprs.insert(s.name.as_str().to_string(), def);
                            }
                        }
                        _ => {}
                    }
                }
                for _ in 0..4 {
                    let mut changed = false;
                    for b in &mut m.bindings {
                        if b.type_name.is_none() {
                            if let Some(expr) = top_exprs.get(&b.name) {
                                if let Some(ty) = infer_expr_in_scope(
                                    expr,
                                    &module_scope,
                                    &HashMap::new(),
                                    &comp_members,
                                    &enum_names,
                                    None,
                                ) {
                                    module_scope.insert(b.name.clone(), ty.clone());
                                    b.type_name = Some(ty);
                                    changed = true;
                                }
                            }
                        }
                    }
                    if !changed {
                        break;
                    }
                }

                // Group AST ComponentDefs by name in source order to match ComponentDoc::overloads
                let mut ast_comps: HashMap<&str, Vec<&ComponentDef>> = HashMap::new();
                for item in &doc.items {
                    if let Item::Component(c) = item {
                        ast_comps.entry(c.name.as_str()).or_default().push(c);
                    }
                }

                // Infer types for every overload's aliases and provided_envs
                for comp_doc in &mut m.components {
                    let Some(defs) = ast_comps.get(comp_doc.name.as_str()) else {
                        continue;
                    };
                    for (ov_doc, comp_def) in comp_doc.overloads.iter_mut().zip(defs.iter()) {
                        infer_overload_types(
                            &comp_doc.name,
                            ov_doc,
                            comp_def,
                            &module_scope,
                            &comp_members,
                            &enum_names,
                        );
                    }
                }
            }
        }
    }
}

fn infer_overload_types(
    comp_name: &str,
    ov_doc: &mut OverloadDoc,
    comp_def: &ComponentDef,
    module_scope: &HashMap<String, String>,
    comp_members: &ComponentMemberTypes,
    enum_names: &HashSet<String>,
) {
    let mut local_scope = module_scope.clone();
    let mut expr_return_types: HashMap<String, String> = HashMap::new();

    // Seed with signature parameters
    for p in &ov_doc.params {
        if let Some(ty) = &p.type_name {
            local_scope.insert(p.name.clone(), ty.clone());
        }
    }

    // Seed with explicit annotations on body items and element bindings
    for item in &comp_def.body {
        match item {
            ComponentBodyItem::State(s) => {
                if let Some(ty) = &s.type_annotation {
                    local_scope.insert(s.name.as_str().to_string(), ty.name.as_str().to_string());
                } else if let Some(def) = &s.default {
                    if let Some(ty) = infer_expr_in_scope(
                        def,
                        &local_scope,
                        &expr_return_types,
                        comp_members,
                        enum_names,
                        Some(comp_name),
                    ) {
                        local_scope.insert(s.name.as_str().to_string(), ty);
                    }
                }
            }
            ComponentBodyItem::Let(l) => {
                if let Some(ty) = &l.type_annotation {
                    local_scope.insert(l.name.as_str().to_string(), ty.name.as_str().to_string());
                } else if let Some(LetValue::Node(n)) = &l.value {
                    local_scope.insert(l.name.as_str().to_string(), n.name.as_str().to_string());
                }
            }
            ComponentBodyItem::Alias(a) => {
                if let Some(ty) = &a.type_annotation {
                    local_scope.insert(a.name.as_str().to_string(), ty.name.as_str().to_string());
                }
            }
            ComponentBodyItem::Env(e) => {
                if let Some(ty) = &e.type_annotation {
                    local_scope.insert(e.name.as_str().to_string(), ty.name.as_str().to_string());
                }
            }
            _ => {}
        }
    }

    // Fixed-point pass over all body `\let`, `\state`, `\alias`, and `\env` expressions
    for _ in 0..8 {
        let mut changed = false;
        for item in &comp_def.body {
            match item {
                ComponentBodyItem::Let(l) => {
                    let name = l.name.as_str();
                    if let Some(LetValue::Expr(e)) = &l.value {
                        if let Expr::ExprDef(edef) = e {
                            let mut sub_scope = local_scope.clone();
                            for p in &edef.params {
                                if let Some(ty) = &p.type_annotation {
                                    sub_scope.insert(
                                        p.name.as_str().to_string(),
                                        ty.name.as_str().to_string(),
                                    );
                                }
                            }
                            if let Some(ret) = infer_expr_in_scope(
                                &edef.body,
                                &sub_scope,
                                &expr_return_types,
                                comp_members,
                                enum_names,
                                Some(comp_name),
                            ) {
                                expr_return_types.insert(name.to_string(), ret);
                            }
                        }
                        if !local_scope.contains_key(name) {
                            if let Some(ty) = infer_expr_in_scope(
                                e,
                                &local_scope,
                                &expr_return_types,
                                comp_members,
                                enum_names,
                                Some(comp_name),
                            ) {
                                local_scope.insert(name.to_string(), ty);
                                changed = true;
                            }
                        }
                    }
                }
                ComponentBodyItem::Alias(a) => {
                    let name = a.name.as_str();
                    if !local_scope.contains_key(name) {
                        if let Some(ty) = infer_expr_in_scope(
                            &a.value,
                            &local_scope,
                            &expr_return_types,
                            comp_members,
                            enum_names,
                            Some(comp_name),
                        ) {
                            local_scope.insert(name.to_string(), ty);
                            changed = true;
                        }
                    }
                }
                ComponentBodyItem::Env(e) => {
                    let name = e.name.as_str();
                    if let Some(val) = &e.value {
                        if let Some(ty) = infer_expr_in_scope(
                            val,
                            &local_scope,
                            &expr_return_types,
                            comp_members,
                            enum_names,
                            Some(comp_name),
                        ) {
                            if local_scope.get(name).map(String::as_str) != Some(ty.as_str()) {
                                local_scope.insert(name.to_string(), ty);
                                changed = true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if !changed {
            break;
        }
    }

    // Write back inferred types onto OverloadDoc's aliases and provided_envs
    let alias_exprs: HashMap<&str, &Expr> = comp_def
        .body
        .iter()
        .filter_map(|it| match it {
            ComponentBodyItem::Alias(a) => Some((a.name.as_str(), &a.value)),
            _ => None,
        })
        .collect();
    for a_doc in &mut ov_doc.aliases {
        if a_doc.type_name.is_none() {
            a_doc.type_name = local_scope.get(&a_doc.name).cloned().or_else(|| {
                alias_exprs.get(a_doc.name.as_str()).and_then(|e| {
                    infer_expr_in_scope(
                        e,
                        &local_scope,
                        &expr_return_types,
                        comp_members,
                        enum_names,
                        Some(comp_name),
                    )
                })
            });
        }
    }

    let env_exprs: HashMap<&str, &Expr> = comp_def
        .body
        .iter()
        .filter_map(|it| match it {
            ComponentBodyItem::Env(e) => e.value.as_ref().map(|v| (e.name.as_str(), v)),
            _ => None,
        })
        .collect();
    for e_doc in &mut ov_doc.provided_envs {
        if e_doc.type_name.is_none() {
            e_doc.type_name = env_exprs
                .get(e_doc.name.as_str())
                .and_then(|e| {
                    infer_expr_in_scope(
                        e,
                        &local_scope,
                        &expr_return_types,
                        comp_members,
                        enum_names,
                        Some(comp_name),
                    )
                })
                .or_else(|| local_scope.get(&e_doc.name).cloned());
        }
    }

    let mut child_port_exprs: HashMap<&str, &Expr> = HashMap::new();
    collect_children_port_exprs(&comp_def.body, &mut child_port_exprs);
    for cp in &mut ov_doc.children_pushed_ports {
        if cp.type_name.is_none() {
            cp.type_name = child_port_exprs
                .get(cp.name.as_str())
                .and_then(|e| {
                    infer_expr_in_scope(
                        e,
                        &local_scope,
                        &expr_return_types,
                        comp_members,
                        enum_names,
                        Some(comp_name),
                    )
                })
                .or_else(|| builtin_member_type(&cp.name).map(str::to_string));
        }
    }
}

fn infer_expr_in_scope(
    expr: &Expr,
    scope: &HashMap<String, String>,
    expr_returns: &HashMap<String, String>,
    comp_members: &ComponentMemberTypes,
    enum_names: &HashSet<String>,
    self_comp: Option<&str>,
) -> Option<String> {
    match expr {
        Expr::Literal(lit) => Some(lit.type_name().to_string()),
        Expr::Node(n) => Some(n.name.as_str().to_string()),
        Expr::ExprDef(_) => Some("Expr".to_string()),
        Expr::Paren(inner, _) => {
            infer_expr_in_scope(inner, scope, expr_returns, comp_members, enum_names, self_comp)
        }
        Expr::Ident(id) => {
            let s = id.as_str();
            if let Some(ty) = scope.get(s) {
                return Some(ty.clone());
            }
            if enum_names.contains(s) {
                return Some(s.to_string());
            }
            match s {
                "self" => self_comp.map(str::to_string),
                "window" => Some("ViewPort".to_string()),
                "page" => Some("Page".to_string()),
                "parent" | "prev" | "next" | "child" | "children" => Some("Node".to_string()),
                "left" | "top" | "right" | "bottom" | "x" | "y" | "width" | "height"
                | "baseline" => Some("Distance".to_string()),
                _ => None,
            }
        }
        Expr::Unary(u) => match u.op {
            UnaryOp::Not => Some("Boolean".to_string()),
            UnaryOp::Neg => infer_expr_in_scope(
                &u.operand,
                scope,
                expr_returns,
                comp_members,
                enum_names,
                self_comp,
            ),
        },
        Expr::Binary(b) => {
            let lt = infer_expr_in_scope(
                &b.left,
                scope,
                expr_returns,
                comp_members,
                enum_names,
                self_comp,
            );
            let rt = infer_expr_in_scope(
                &b.right,
                scope,
                expr_returns,
                comp_members,
                enum_names,
                self_comp,
            );
            match b.op {
                BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge
                | BinaryOp::And
                | BinaryOp::Or => Some("Boolean".to_string()),
                BinaryOp::Add => match (lt.as_deref(), rt.as_deref()) {
                    (Some("String"), _) | (_, Some("String")) => Some("String".to_string()),
                    (Some("Distance"), _) | (_, Some("Distance")) => Some("Distance".to_string()),
                    (Some("Duration"), _) | (_, Some("Duration")) => Some("Duration".to_string()),
                    (Some("Area"), _) | (_, Some("Area")) => Some("Area".to_string()),
                    (Some("Float"), _) | (_, Some("Float")) => Some("Float".to_string()),
                    _ => lt.or(rt),
                },
                BinaryOp::Sub => match (lt.as_deref(), rt.as_deref()) {
                    (Some("Distance"), _) | (_, Some("Distance")) => Some("Distance".to_string()),
                    (Some("Duration"), _) | (_, Some("Duration")) => Some("Duration".to_string()),
                    (Some("Area"), _) | (_, Some("Area")) => Some("Area".to_string()),
                    (Some("Float"), _) | (_, Some("Float")) => Some("Float".to_string()),
                    _ => lt.or(rt),
                },
                BinaryOp::Mul => match (lt.as_deref(), rt.as_deref()) {
                    (Some("Distance"), Some("Distance")) => Some("Area".to_string()),
                    (Some("Distance"), _) | (_, Some("Distance")) => Some("Distance".to_string()),
                    (Some("Duration"), _) | (_, Some("Duration")) => Some("Duration".to_string()),
                    (Some("Area"), _) | (_, Some("Area")) => Some("Area".to_string()),
                    (Some("Float"), _) | (_, Some("Float")) => Some("Float".to_string()),
                    _ => lt.or(rt),
                },
                BinaryOp::Div => match (lt.as_deref(), rt.as_deref()) {
                    (Some("Area"), Some("Distance")) => Some("Distance".to_string()),
                    (Some("Distance"), Some("Distance")) => Some("Float".to_string()),
                    (Some("Duration"), Some("Duration")) => Some("Float".to_string()),
                    (Some("Distance"), _) => Some("Distance".to_string()),
                    (Some("Duration"), _) => Some("Duration".to_string()),
                    _ => Some("Float".to_string()),
                },
                BinaryOp::Rem => lt.or(rt).or_else(|| Some("Integer".to_string())),
            }
        }
        Expr::Ternary(t) => {
            let tt = infer_expr_in_scope(
                &t.then_expr,
                scope,
                expr_returns,
                comp_members,
                enum_names,
                self_comp,
            );
            let et = infer_expr_in_scope(
                &t.else_expr,
                scope,
                expr_returns,
                comp_members,
                enum_names,
                self_comp,
            );
            merge_branch_types(tt, et)
        }
        Expr::Match(m) => {
            let mut acc: Option<String> = None;
            for arm in &m.arms {
                let arm_ty = infer_expr_in_scope(
                    &arm.body,
                    scope,
                    expr_returns,
                    comp_members,
                    enum_names,
                    self_comp,
                );
                acc = merge_branch_types(acc, arm_ty);
            }
            acc
        }
        Expr::Call(c) => {
            let callee = c.callee.as_str();
            if let Some(ret) = expr_returns.get(callee) {
                return Some(ret.clone());
            }
            match callee {
                "min" | "max" | "clamp" | "abs" | "sum" | "only" | "first" | "last" | "avg"
                | "hypot" => {
                    let mut acc: Option<String> = None;
                    for arg in &c.args {
                        let at = infer_expr_in_scope(
                            arg,
                            scope,
                            expr_returns,
                            comp_members,
                            enum_names,
                            self_comp,
                        );
                        acc = merge_branch_types(acc, at);
                    }
                    acc
                }
                "sqrt" => {
                    let arg_ty = c.args.first().and_then(|a| {
                        infer_expr_in_scope(
                            a,
                            scope,
                            expr_returns,
                            comp_members,
                            enum_names,
                            self_comp,
                        )
                    });
                    if arg_ty.as_deref() == Some("Area") {
                        Some("Distance".to_string())
                    } else {
                        Some("Float".to_string())
                    }
                }
                "floor" | "ceil" | "round" => {
                    let arg_ty = c.args.first().and_then(|a| {
                        infer_expr_in_scope(
                            a,
                            scope,
                            expr_returns,
                            comp_members,
                            enum_names,
                            self_comp,
                        )
                    });
                    if arg_ty.as_deref() == Some("Distance") {
                        Some("Distance".to_string())
                    } else {
                        Some("Integer".to_string())
                    }
                }
                "to_px" | "to_pt" | "to_in" | "to_pc" | "to_mm" | "to_cm" => {
                    Some("Distance".to_string())
                }
                "rgb" | "rgba" | "hsl" | "mix_color" | "with_alpha" => Some("Color".to_string()),
                "str" | "format" => Some("String".to_string()),
                "lerp" => {
                    let a = c.args.first().and_then(|x| {
                        infer_expr_in_scope(
                            x,
                            scope,
                            expr_returns,
                            comp_members,
                            enum_names,
                            self_comp,
                        )
                    });
                    a.or_else(|| Some("Float".to_string()))
                }
                "sin" | "cos" | "tan" | "atan2" | "pow" | "log" | "exp" => {
                    Some("Float".to_string())
                }
                _ => None,
            }
        }
        Expr::MethodCall(mc) => {
            let method = mc.method.as_str();
            match method {
                "to_px" | "to_pt" | "to_in" | "to_pc" | "to_mm" | "to_cm" => {
                    Some("Distance".to_string())
                }
                "to_string" => Some("String".to_string()),
                "with_alpha" | "lighten" | "darken" => Some("Color".to_string()),
                "eval" => {
                    if let Expr::Ident(id) = mc.target.as_ref() {
                        expr_returns.get(id.as_str()).cloned()
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }
        Expr::MemberAccess(m) => {
            let member = m.member.as_str();

            // Enum variant access (e.g. `Align.Center`)
            if let Expr::Ident(id) = m.target.as_ref() {
                let target_name = id.as_str();
                if enum_names.contains(target_name) {
                    return Some(target_name.to_string());
                }
                if target_name == "self" {
                    if let Some(ty) = scope.get(member) {
                        return Some(ty.clone());
                    }
                }
            }

            let target_ty = infer_expr_in_scope(
                &m.target,
                scope,
                expr_returns,
                comp_members,
                enum_names,
                self_comp,
            );

            // Check catalog component member types first (e.g. `style.padding_x`, `link_style.bg`)
            if let Some(ref tname) = target_ty {
                if let Some(members) = comp_members.get(tname) {
                    if let Some(mty) = members.get(member) {
                        return Some(mty.clone());
                    }
                }
                if tname == "Page" && member == "prev" {
                    return Some("Page".to_string());
                }
            }

            builtin_member_type(member).map(str::to_string)
        }
        Expr::Index(ix) => {
            let target_ty = infer_expr_in_scope(
                &ix.target,
                scope,
                expr_returns,
                comp_members,
                enum_names,
                self_comp,
            );
            match target_ty.as_deref() {
                Some("PageValue") => Some("PageValueSlice".to_string()),
                Some("PageList") => Some("PageListSlice".to_string()),
                _ => Some("Node".to_string()),
            }
        }
        Expr::DomNodeRef(_, _) => Some("Node".to_string()),
    }
}

fn merge_branch_types(a: Option<String>, b: Option<String>) -> Option<String> {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some(x), Some(y)) if x == y => Some(x),
        // Prefer more specific dimensional/floating types over bare `0` (`Integer`)
        (Some(x), Some(y)) if x == "Integer" && matches!(y.as_str(), "Distance" | "Float" | "Duration" | "Area") => {
            Some(y)
        }
        (Some(x), Some(y)) if y == "Integer" && matches!(x.as_str(), "Distance" | "Float" | "Duration" | "Area") => {
            Some(x)
        }
        (Some(x), _) => Some(x),
    }
}

fn builtin_member_type(member: &str) -> Option<&'static str> {
    match member {
        "x" | "y" | "left" | "top" | "right" | "bottom" | "width" | "height" | "baseline"
        | "baseline_offset" | "cap_height" | "ascent" | "descent" | "size" | "line_height"
        | "padding" | "padding_x" | "padding_y" | "radius" | "border_width" | "thickness"
        | "gap" | "offset_x" | "offset_y" | "blur" | "spread" | "scroll_x" | "scroll_y"
        | "content_width" | "content_height" | "viewport_width" | "viewport_height"
        | "max_scroll_x" | "max_scroll_y" | "dpi" => Some("Distance"),
        "len" | "count" | "number" | "page" | "page_count" | "end_page" | "weight"
        | "tabindex" | "index" | "z" | "orphans" | "widows" => Some("Integer"),
        "color" | "bg" | "border_color" | "hover_color" | "focused_color" | "hover_bg"
        | "focused_bg" | "text_color" | "main" | "light" | "dark" | "contrast_text" => {
            Some("Color")
        }
        "underline" | "hover_underline" | "focused_underline" | "italic" | "bold"
        | "small_caps" | "breakable" | "is_first" | "is_last" | "is_odd" | "is_even"
        | "checked" | "disabled" | "selected" | "active" | "focused" | "hovered"
        | "pressed" | "dragging" => Some("Boolean"),
        "text" | "title" | "label" | "family" | "url" | "name" | "running_chapter" => {
            Some("String")
        }
        "opacity" | "progress" | "ratio" | "scale" | "elapsed_secs" => Some("Float"),
        "font" => Some("Font"),
        "value" => Some("Float"),
        _ => None,
    }
}

/// Resolves internal anchor cross-references for module imports and type annotations.
fn resolve_cross_references(catalog: &mut DocCatalog) {
    // 1. Index every module path -> (pkg_idx, mod_idx), module anchor, and exported type symbols.
    let mut mod_anchor_by_path: HashMap<String, String> = HashMap::new();
    let mut mod_symbols_by_path: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut mod_stem_by_path: HashMap<String, String> = HashMap::new();
    let mut global_type_anchors: HashMap<String, Vec<String>> = HashMap::new();

    for pkg in &catalog.packages {
        for m in &pkg.modules {
            let norm = normalize_rel_str(&m.rel_path);
            mod_anchor_by_path.insert(norm.clone(), m.anchor.clone());
            mod_stem_by_path.insert(norm.clone(), m.stem.clone());

            let mut syms = HashMap::new();
            for e in &m.enums {
                syms.insert(e.name.clone(), e.anchor.clone());
                global_type_anchors
                    .entry(e.name.clone())
                    .or_default()
                    .push(e.anchor.clone());
            }
            for c in &m.components {
                syms.insert(c.name.clone(), c.anchor.clone());
                global_type_anchors
                    .entry(c.name.clone())
                    .or_default()
                    .push(c.anchor.clone());
            }
            mod_symbols_by_path.insert(norm, syms);
        }
    }

    // 2. For each module, resolve its imports and its visible type-name -> anchor scope.
    for pkg in &mut catalog.packages {
        for m in &mut pkg.modules {
            let mod_norm = normalize_rel_str(&m.rel_path);
            let mod_dir = Path::new(&mod_norm).parent().unwrap_or(Path::new(""));

            let mut visible_types: HashMap<String, String> = HashMap::new();

            // Imported symbols first (so local definitions override them).
            for imp in &mut m.imports {
                let target_path = resolve_import_target_path(&imp.path, mod_dir, &mod_anchor_by_path);
                if let Some(ref resolved) = target_path {
                    imp.target_anchor = mod_anchor_by_path.get(resolved).cloned();
                    if let Some(target_syms) = mod_symbols_by_path.get(resolved) {
                        if !imp.items.is_empty() {
                            for item in &imp.items {
                                if let Some(anchor) = target_syms.get(&item.name) {
                                    let bound = item
                                        .alias
                                        .as_deref()
                                        .unwrap_or(item.name.as_str())
                                        .to_string();
                                    visible_types.insert(bound, anchor.clone());
                                }
                            }
                        } else if let Some(alias) = &imp.alias {
                            let stem = mod_stem_by_path
                                .get(resolved)
                                .map(String::as_str)
                                .unwrap_or("");
                            if let Some(anchor) = target_syms
                                .get(stem)
                                .or_else(|| target_syms.get(alias))
                                .or_else(|| {
                                    if target_syms.len() == 1 {
                                        target_syms.values().next()
                                    } else {
                                        None
                                    }
                                })
                            {
                                visible_types.insert(alias.clone(), anchor.clone());
                            }
                        } else {
                            for (sym_name, anchor) in target_syms {
                                visible_types.insert(sym_name.clone(), anchor.clone());
                            }
                        }
                    }
                }
            }

            // Local symbols defined in this module take highest precedence.
            for e in &m.enums {
                visible_types.insert(e.name.clone(), e.anchor.clone());
            }
            for c in &m.components {
                visible_types.insert(c.name.clone(), c.anchor.clone());
            }

            let lookup_type = |type_name: &str| -> Option<String> {
                if let Some(a) = visible_types.get(type_name) {
                    return Some(a.clone());
                }
                if let Some(list) = global_type_anchors.get(type_name) {
                    if list.len() == 1 {
                        return Some(list[0].clone());
                    }
                }
                None
            };

            for c in &mut m.components {
                for ov in &mut c.overloads {
                    for p in &mut ov.params {
                        if let Some(ref tn) = p.type_name {
                            p.type_anchor = lookup_type(tn);
                        }
                    }
                    for a in &mut ov.aliases {
                        if let Some(ref tn) = a.type_name {
                            a.type_anchor = lookup_type(tn);
                        }
                    }
                    for env in &mut ov.provided_envs {
                        if let Some(ref tn) = env.type_name {
                            env.type_anchor = lookup_type(tn);
                        }
                    }
                    for cp in &mut ov.children_pushed_ports {
                        if let Some(ref tn) = cp.type_name {
                            cp.type_anchor = lookup_type(tn);
                        }
                    }
                }
            }

            for b in &mut m.bindings {
                if let Some(ref tn) = b.type_name {
                    b.type_anchor = lookup_type(tn);
                }
            }
        }
    }
}

fn resolve_import_target_path(
    import_path: &str,
    mod_dir: &Path,
    known_modules: &HashMap<String, String>,
) -> Option<String> {
    let direct = normalize_rel_str(import_path);
    if known_modules.contains_key(&direct) {
        return Some(direct);
    }
    let joined = normalize_rel_str(&mod_dir.join(import_path).to_string_lossy());
    if known_modules.contains_key(&joined) {
        return Some(joined);
    }
    None
}

fn normalize_rel_str(raw: &str) -> String {
    let normalized = raw.replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    for seg in normalized.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn unique_anchor(base: &str, used: &mut HashSet<String>) -> String {
    if used.insert(base.to_string()) {
        return base.to_string();
    }
    let mut n = 2usize;
    loop {
        let candidate = format!("{base}-{n}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in s.chars().flat_map(|c| c.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "item".to_string()
    } else {
        out
    }
}

// ============================================================================
// AST Pretty-Printer
// ============================================================================

fn format_let_value(val: &LetValue) -> String {
    match val {
        LetValue::Expr(e) => format_expr(e),
        LetValue::Node(n) => format_element_node(n),
    }
}

fn format_element_node(node: &ElementNode) -> String {
    let mut out = format!("\\{}", node.name.as_str());
    if !node.ports.is_empty() || node.key.is_some() {
        out.push('(');
        let mut first = true;
        if let Some(key) = &node.key {
            out.push_str(&key.format_key());
            out.push(';');
            first = false;
        }
        for pb in &node.ports {
            if !first {
                out.push_str(", ");
            }
            first = false;
            out.push_str(pb.name.as_str());
            out.push_str(": ");
            out.push_str(&format_expr(&pb.expr));
        }
        out.push(')');
    }
    if node.content.is_some() {
        out.push_str(" { ... }");
    }
    out
}

/// Formats an AST expression into readable DirectedType source text.
pub fn format_expr(expr: &Expr) -> String {
    match expr {
        Expr::Literal(lit) => match lit {
            Literal::String(s, _) | Literal::Path(s, _) => {
                format!("\"{}\"", escape_display_string(s))
            }
            other => format!("{other}"),
        },
        Expr::Ident(id) => id.as_str().to_string(),
        Expr::MemberAccess(m) => format!("{}.{}", format_expr(&m.target), m.member.as_str()),
        Expr::Index(ix) => format!("{}[{}]", format_expr(&ix.target), format_expr(&ix.index)),
        Expr::Ternary(t) => format!(
            "{} ? {} : {}",
            format_expr(&t.condition),
            format_expr(&t.then_expr),
            format_expr(&t.else_expr)
        ),
        Expr::Call(c) => {
            let args: Vec<String> = c.args.iter().map(format_expr).collect();
            format!("{}({})", c.callee.as_str(), args.join(", "))
        }
        Expr::MethodCall(m) => {
            let args: Vec<String> = m.args.iter().map(format_expr).collect();
            format!(
                "{}.{}({})",
                format_expr(&m.target),
                m.method.as_str(),
                args.join(", ")
            )
        }
        Expr::Binary(b) => format!(
            "{} {} {}",
            format_expr(&b.left),
            b.op,
            format_expr(&b.right)
        ),
        Expr::Unary(u) => format!("{}{}", u.op, format_expr(&u.operand)),
        Expr::Paren(inner, _) => format!("({})", format_expr(inner)),
        Expr::Node(node) => format_element_node(node),
        Expr::ExprDef(def) => {
            if def.params.is_empty() {
                format!("\\Expr {{ {} }}", format_expr(&def.body))
            } else {
                let params: Vec<String> = def
                    .params
                    .iter()
                    .map(|p| {
                        let mut s = p.name.as_str().to_string();
                        if let Some(ty) = &p.type_annotation {
                            s.push_str(": ");
                            s.push_str(ty.name.as_str());
                        }
                        if let Some(d) = &p.default_edge {
                            s.push_str(" = ");
                            s.push_str(&format_expr(d));
                        }
                        s
                    })
                    .collect();
                format!(
                    "\\Expr({}) {{ {} }}",
                    params.join(", "),
                    format_expr(&def.body)
                )
            }
        }
        Expr::Match(m) => {
            let mut out = if let Some(scrut) = &m.scrutinee {
                format!("match {} {{ ", format_expr(scrut))
            } else {
                "match { ".to_string()
            };
            for (i, arm) in m.arms.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                match &arm.pattern {
                    MatchPattern::Wildcard(_) => {
                        let _ = write!(out, "_ => {}", format_expr(&arm.body));
                    }
                    MatchPattern::Condition(cond) => {
                        let _ = write!(
                            out,
                            "{} => {}",
                            format_expr(cond),
                            format_expr(&arm.body)
                        );
                    }
                    MatchPattern::Variants(vars) => {
                        let pats: Vec<String> = vars
                            .iter()
                            .map(|v| {
                                if let Some(en) = &v.enum_name {
                                    format!("{}.{}", en.as_str(), v.variant.as_str())
                                } else {
                                    v.variant.as_str().to_string()
                                }
                            })
                            .collect();
                        let _ = write!(
                            out,
                            "{} => {}",
                            pats.join(" | "),
                            format_expr(&arm.body)
                        );
                    }
                }
            }
            out.push_str(" }");
            out
        }
        Expr::DomNodeRef(h, _) => format!("__dom_node({}:{})", h.index(), h.generation()),
    }
}

fn escape_display_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

fn kind_keyword(kind: ComponentKind) -> &'static str {
    match kind {
        ComponentKind::Component => "\\Component",
        ComponentKind::Span => "\\Span",
        ComponentKind::InlineBox => "\\InlineBox",
    }
}

fn kind_label(kind: ComponentKind) -> &'static str {
    match kind {
        ComponentKind::Component => "Component",
        ComponentKind::Span => "Span",
        ComponentKind::InlineBox => "InlineBox",
    }
}

fn kind_colors(kind: ComponentKind) -> (&'static str, &'static str) {
    match kind {
        ComponentKind::Component => ("#dbeafe", "#1e40af"),
        ComponentKind::Span => ("#f3e8ff", "#6b21a8"),
        ComponentKind::InlineBox => ("#ccfbf1", "#115e59"),
    }
}

/// Formats an overload signature compactly as `\Kind Name(req1, req2, ...) { ... }`,
/// listing only required non-env parameters, appending `...` when optional or
/// environmental parameters exist, and appending `{ . }` (single child) or `{ ... }`
/// (multiple children) when the overload accepts children.
pub fn format_overload_signature(name: &str, ov: &OverloadDoc) -> String {
    let mut args: Vec<&str> = ov
        .params
        .iter()
        .filter(|p| !p.is_env && p.default_value.is_none())
        .map(|p| p.name.as_str())
        .collect();
    let has_optional = ov
        .params
        .iter()
        .any(|p| p.is_env || p.default_value.is_some());
    if has_optional {
        args.push("...");
    }
    let children_suffix = match ov.children_cardinality {
        ChildrenCardinality::None => "",
        ChildrenCardinality::Single => " { . }",
        ChildrenCardinality::Multiple => " { ... }",
    };
    if args.is_empty() {
        format!("{} {}(){}", kind_keyword(ov.kind), name, children_suffix)
    } else {
        format!(
            "{} {}({}){}",
            kind_keyword(ov.kind),
            name,
            args.join(", "),
            children_suffix
        )
    }
}

// ============================================================================
// DirectedType `.dt` Emitter
// ============================================================================

fn escape_dt_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

fn escape_dt_content(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '~' => out.push_str("\\~"),
            '/' => out.push_str("\\/"),
            '*' => out.push_str("\\*"),
            '\n' => out.push_str(" "),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// Renders an extracted `DocCatalog` into a complete DirectedType `.dt` source document
/// with an automatic clickable `\TableOfContents`, PDF outlines, package quick indexes,
/// and cross-referenced internal `#anchor` links.
pub fn render_catalog_to_dt(catalog: &DocCatalog) -> String {
    let mut dt = String::with_capacity(128 * 1024);

    // 1. Imports & Document Stylesheet Components
    dt.push_str("// Auto-generated DirectedType Component Reference\n");
    dt.push_str("\\use \"components/Align.dt\"\n");
    dt.push_str("\\use \"components/TextBounds.dt\"\n");
    dt.push_str("\\use \"components/VStackStretch.dt\"\n");
    dt.push_str("\\use \"components/HDivider.dt\"\n");
    dt.push_str("\\use \"components/LinkStyle.dt\"\n");
    dt.push_str("\\use \"components/Link.dt\"\n");
    dt.push_str("\\use \"components/TableOfContents.dt\"\n\n");

    dt.push_str(DOC_STYLESHEET_DT);
    dt.push('\n');

    // 2. Document Environment & PagedFlow Root
    let escaped_title = escape_dt_string(&catalog.title);
    let _ = writeln!(
        dt,
        "\\env font = \\Font(family: \"Arial\", size: 9.5pt, line_height: 13.5pt, color: #0f172a);"
    );
    let _ = writeln!(
        dt,
        "\\env link_style = \\LinkStyle(color: #1d4ed8, underline: false, bg: #00000000);"
    );
    let _ = writeln!(dt);
    let _ = writeln!(dt, "\\Window(title: \"{escaped_title}\")");
    let _ = writeln!(
        dt,
        "\\PagedFlow(\n    gap: 0pt,\n    orphans: 2,\n    widows: 2,\n    page: \\DocPage(doc_title: \"{escaped_title}\")\n) {{"
    );
    let _ = writeln!(
        dt,
        "    \\VStackStretch(width: 7.0in, gap: 6pt, pagination: \\Pagination(breakable: true)) {{"
    );

    // 3. Cover / Title Header Banner
    let summary = format!(
        "{} {}  ·  {} {}  ·  {} {} ({} {})  ·  {} {}  ·  {} public {}",
        catalog.packages.len(),
        plural(catalog.packages.len(), "package", "packages"),
        catalog.module_count(),
        plural(catalog.module_count(), "module", "modules"),
        catalog.component_count(),
        plural(catalog.component_count(), "component", "components"),
        catalog.overload_count(),
        plural(catalog.overload_count(), "overload", "overloads"),
        catalog.enum_count(),
        plural(catalog.enum_count(), "enum", "enums"),
        catalog.binding_count(),
        plural(catalog.binding_count(), "binding", "bindings"),
    );
    let subtitle_str = catalog.subtitle.as_deref().unwrap_or("");
    let _ = writeln!(
        dt,
        "            \\DocCover(title: \"{}\", subtitle: \"{}\", summary: \"{}\")",
        escaped_title,
        escape_dt_string(subtitle_str),
        escape_dt_string(&summary)
    );

    // 4. Table of Contents (auto-populated with clickable links & converging page numbers)
    let _ = writeln!(
        dt,
        "            \\Anchor(name: \"toc-top\", outline_title: \"Table of Contents\", outline_level: 1) {{"
    );
    let _ = writeln!(
        dt,
        "                \\TableOfContents(title: \"Table of Contents\", max_depth: 3)"
    );
    let _ = writeln!(dt, "            }}");

    // 5. Packages, Quick Indexes, Modules, Components, Enums, and Public Bindings
    for pkg in &catalog.packages {
        emit_package(&mut dt, pkg);
    }

    dt.push_str("    }\n");
    dt.push_str("}\n");

    dt
}

fn plural<'a>(n: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if n == 1 {
        singular
    } else {
        plural
    }
}

fn emit_package(dt: &mut String, pkg: &PackageDoc) {
    let pkg_title = format!("Package: {}", pkg.name);
    let comp_total: usize = pkg.modules.iter().map(|m| m.components.len()).sum();
    let enum_total: usize = pkg.modules.iter().map(|m| m.enums.len()).sum();
    let bind_total: usize = pkg.modules.iter().map(|m| m.bindings.len()).sum();
    let pkg_meta = format!(
        "{} {}  ·  {} {}  ·  {} {}  ·  {} public {}",
        pkg.modules.len(),
        plural(pkg.modules.len(), "module", "modules"),
        comp_total,
        plural(comp_total, "component", "components"),
        enum_total,
        plural(enum_total, "enum", "enums"),
        bind_total,
        plural(bind_total, "binding", "bindings"),
    );

    let _ = writeln!(
        dt,
        "            \\Anchor(name: \"{}\", outline_title: \"{}\", outline_level: 1) {{",
        escape_dt_string(&pkg.anchor),
        escape_dt_string(&pkg_title)
    );
    let _ = writeln!(
        dt,
        "                \\H1(title: \"{}\", meta: \"{}\")",
        escape_dt_string(&pkg_title),
        escape_dt_string(&pkg_meta)
    );
    let _ = writeln!(dt, "            }}");

    // Package Quick Index (pkgsite-style clickable overview of all symbols in this package)
    emit_package_index(dt, pkg);

    for module in &pkg.modules {
        emit_module(dt, module);
    }
}

fn emit_package_index(dt: &mut String, pkg: &PackageDoc) {
    let mut comp_links: Vec<(&str, &str)> = Vec::new();
    let mut enum_links: Vec<(&str, &str)> = Vec::new();
    let mut bind_links: Vec<(&str, &str)> = Vec::new();

    for m in &pkg.modules {
        for c in &m.components {
            comp_links.push((&c.name, &c.anchor));
        }
        for e in &m.enums {
            enum_links.push((&e.name, &e.anchor));
        }
        for b in &m.bindings {
            bind_links.push((&b.name, &b.anchor));
        }
    }
    comp_links.sort_by(|a, b| a.0.cmp(b.0).then_with(|| a.1.cmp(b.1)));
    enum_links.sort_by(|a, b| a.0.cmp(b.0).then_with(|| a.1.cmp(b.1)));
    bind_links.sort_by(|a, b| a.0.cmp(b.0).then_with(|| a.1.cmp(b.1)));

    if !comp_links.is_empty() {
        emit_index_group(dt, "Components Index", &comp_links);
    }
    if !enum_links.is_empty() {
        emit_index_group(dt, "Enums Index", &enum_links);
    }
    if !bind_links.is_empty() {
        emit_index_group(dt, "Public Bindings Index", &bind_links);
    }
}

fn emit_index_group(dt: &mut String, heading: &str, items: &[(&str, &str)]) {
    let _ = writeln!(
        dt,
        "            \\DocSubLabel(text: \"{}\")",
        escape_dt_string(heading)
    );
    let _ = writeln!(
        dt,
        "            \\Text(font: \\Font(font: font, family: \"Courier New\", size: 8.2pt, line_height: 13pt, color: #334155)) {{"
    );
    dt.push_str("                ");
    for (i, (name, anchor)) in items.iter().enumerate() {
        if i > 0 {
            dt.push_str("  ·  ");
        }
        let _ = write!(
            dt,
            "\\Link(url: \"#{}\"){{{}}}",
            escape_dt_string(anchor),
            escape_dt_content(name)
        );
    }
    dt.push('\n');
    dt.push_str("            }\n");
}

fn emit_module(dt: &mut String, module: &ModuleDoc) {
    let single_match = module.is_single_matching_symbol();

    if single_match {
        // Single-symbol module matching filename stem: unify Module + Symbol at Level 2.
        if let Some(comp) = module.components.first() {
            let (badge_bg, badge_fg) = kind_colors(comp.kind);
            let _ = writeln!(
                dt,
                "            \\Anchor(name: \"{}\", outline_title: \"{}\", outline_level: 2) {{",
                escape_dt_string(&comp.anchor),
                escape_dt_string(&comp.name)
            );
            let _ = writeln!(
                dt,
                "                \\H2(title: \"{}\", badge: \"{}\", badge_bg: {}, badge_fg: {})",
                escape_dt_string(&comp.name),
                kind_label(comp.kind),
                badge_bg,
                badge_fg
            );
            let _ = writeln!(dt, "            }}");

            emit_module_imports(dt, &module.imports);
            emit_component_body(dt, comp);
            return;
        }
        if let Some(en) = module.enums.first() {
            let _ = writeln!(
                dt,
                "            \\Anchor(name: \"{}\", outline_title: \"{}\", outline_level: 2) {{",
                escape_dt_string(&en.anchor),
                escape_dt_string(&en.name)
            );
            let _ = writeln!(
                dt,
                "                \\H2(title: \"{}\", badge: \"Enum\", badge_bg: #fef3c7, badge_fg: #92400e)",
                escape_dt_string(&en.name)
            );
            let _ = writeln!(dt, "            }}");

            emit_module_imports(dt, &module.imports);
            emit_enum_body(dt, en);
            return;
        }
    }

    // Multi-symbol or module-level file: Level 2 for Module, Level 3 for each Enum and Component.
    let mod_heading = format!("{}.dt", module.stem);
    let _ = writeln!(
        dt,
        "            \\Anchor(name: \"{}\", outline_title: \"{}\", outline_level: 2) {{",
        escape_dt_string(&module.anchor),
        escape_dt_string(&mod_heading)
    );
    let _ = writeln!(
        dt,
        "                \\H2(title: \"{}\", badge: \"Module\", badge_bg: #e2e8f0, badge_fg: #334155)",
        escape_dt_string(&mod_heading)
    );
    let _ = writeln!(dt, "            }}");

    emit_module_imports(dt, &module.imports);

    for en in &module.enums {
        let _ = writeln!(
            dt,
            "            \\Anchor(name: \"{}\", outline_title: \"{}\", outline_level: 3) {{",
            escape_dt_string(&en.anchor),
            escape_dt_string(&en.name)
        );
        let _ = writeln!(
            dt,
            "                \\H3(title: \"{}\", badge: \"Enum\", badge_bg: #fef3c7, badge_fg: #92400e)",
            escape_dt_string(&en.name)
        );
        let _ = writeln!(dt, "            }}");
        emit_enum_body(dt, en);
    }

    for comp in &module.components {
        let (badge_bg, badge_fg) = kind_colors(comp.kind);
        let _ = writeln!(
            dt,
            "            \\Anchor(name: \"{}\", outline_title: \"{}\", outline_level: 3) {{",
            escape_dt_string(&comp.anchor),
            escape_dt_string(&comp.name)
        );
        let _ = writeln!(
            dt,
            "                \\H3(title: \"{}\", badge: \"{}\", badge_bg: {}, badge_fg: {})",
            escape_dt_string(&comp.name),
            kind_label(comp.kind),
            badge_bg,
            badge_fg
        );
        let _ = writeln!(dt, "            }}");
        emit_component_body(dt, comp);
    }

    if !module.bindings.is_empty() {
        emit_bindings_section(dt, &module.bindings);
    }
}

fn emit_module_imports(dt: &mut String, imports: &[ImportDoc]) {
    if imports.is_empty() {
        return;
    }
    let _ = writeln!(
        dt,
        "            \\Text(font: \\Font(font: font, size: 8.3pt, line_height: 12.5pt, color: #475569)) {{"
    );
    dt.push_str("                Imports: ");
    for (i, imp) in imports.iter().enumerate() {
        if i > 0 {
            dt.push_str(", ");
        }
        if let Some(anchor) = &imp.target_anchor {
            let _ = write!(
                dt,
                "\\Link(url: \"#{}\"){{{}}}",
                escape_dt_string(anchor),
                escape_dt_content(&imp.path)
            );
        } else {
            dt.push_str(&escape_dt_content(&imp.path));
        }
        if let Some(alias) = &imp.alias {
            let _ = write!(dt, " as {}", escape_dt_content(alias));
        } else if !imp.items.is_empty() {
            let names: Vec<String> = imp
                .items
                .iter()
                .map(|it| match &it.alias {
                    Some(al) => format!("{} as {}", it.name, al),
                    None => it.name.clone(),
                })
                .collect();
            let _ = write!(dt, " ({})", escape_dt_content(&names.join(", ")));
        }
    }
    dt.push('\n');
    dt.push_str("            }\n");
}

fn emit_enum_body(dt: &mut String, en: &EnumDoc) {
    let variants_sig = format!("\\Enum {} {{ {} }}", en.name, en.variants.join(", "));
    let _ = writeln!(
        dt,
        "            \\DocSigBox(code: \"{}\")",
        escape_dt_string(&variants_sig)
    );
    let _ = writeln!(dt, "            \\Table3() {{");
    let _ = writeln!(
        dt,
        "                \\Table3Header(h1: \"Variant\", h2: \"Qualified Value\", h3: \"Index\")"
    );
    let last_idx = en.variants.len().saturating_sub(1);
    for (idx, variant) in en.variants.iter().enumerate() {
        let bg = if idx % 2 == 1 { "#f8fafc" } else { "#ffffff" };
        let is_last = idx == last_idx;
        let mb = if is_last { "10pt" } else { "0pt" };
        let qualified = format!("{}.{}", en.name, variant);
        let _ = writeln!(
            dt,
            "                \\Table3Row(col1: \"{}\", col2: \"{}\", col3: \"{}\", bg: {}, is_last: {}, margin_bottom: {})",
            escape_dt_string(variant),
            escape_dt_string(&qualified),
            idx,
            bg,
            is_last,
            mb
        );
    }
    let _ = writeln!(dt, "            }}");
}

struct UnifiedPropRow<'a> {
    section_label: &'a str,
    section_color: &'a str,
    section_break: bool,
    name: String,
    name_color: &'a str,
    type_display: String,
    type_anchor: Option<&'a str>,
    value_display: String,
    value_color: &'a str,
}

fn emit_component_body(dt: &mut String, comp: &ComponentDoc) {
    let multi = comp.overloads.len() > 1;

    // Stacked clickable overload synopsis box: all overload signatures one after another,
    // each linked directly to its specific overload's property table.
    if multi {
        let _ = writeln!(dt, "            \\DocOverloadSynopsis() {{");
        for ov in &comp.overloads {
            let sig = format_overload_signature(&comp.name, ov);
            let _ = writeln!(
                dt,
                "                \\Text(z: 1, font: \\Font(font: font, family: \"Courier New\", size: 8pt, weight: 600, line_height: 11.5pt, color: #1d4ed8)) {{ \\Link(url: \"#{}\"){{{}}} }}",
                escape_dt_string(&ov.anchor),
                escape_dt_content(&sig)
            );
        }
        let _ = writeln!(dt, "            }}");
    }

    for (idx, ov) in comp.overloads.iter().enumerate() {
        // Compact overload signature box: `\Span Link(url, ...) { ... }`
        let sig = format_overload_signature(&comp.name, ov);
        if multi {
            let tag = format!("{} / {}", idx + 1, comp.overloads.len());
            let _ = writeln!(
                dt,
                "            \\Anchor(name: \"{}\") {{",
                escape_dt_string(&ov.anchor)
            );
            let _ = writeln!(
                dt,
                "                \\DocSigBox(code: \"{}\", tag: \"{}\")",
                escape_dt_string(&sig),
                escape_dt_string(&tag)
            );
            let _ = writeln!(dt, "            }}");
        } else {
            let _ = writeln!(
                dt,
                "            \\DocSigBox(code: \"{}\")",
                escape_dt_string(&sig)
            );
        }

        // Build unified property table rows in canonical section order:
        // 1. Required Parameters
        // 2. Optional Parameters (including env parameters)
        // 3. Aliases (non-spatial)
        // 4. Env for Children
        let mut rows: Vec<UnifiedPropRow<'_>> = Vec::new();
        let mut has_prev_section = false;

        let req_params: Vec<&ParamDoc> = ov
            .params
            .iter()
            .filter(|p| !p.is_env && p.default_value.is_none())
            .collect();
        if !req_params.is_empty() {
            for (i, p) in req_params.iter().enumerate() {
                rows.push(UnifiedPropRow {
                    section_label: if i == 0 { "Required Parameters" } else { "" },
                    section_color: "#b91c1c",
                    section_break: i == 0 && has_prev_section,
                    name: p.name.clone(),
                    name_color: "#0f172a",
                    type_display: p.type_name.clone().unwrap_or_else(|| "Any".to_string()),
                    type_anchor: p.type_anchor.as_deref(),
                    value_display: String::new(),
                    value_color: "#64748b",
                });
            }
            has_prev_section = true;
        }

        let opt_params: Vec<&ParamDoc> = ov
            .params
            .iter()
            .filter(|p| p.is_env || p.default_value.is_some())
            .collect();
        if !opt_params.is_empty() {
            for (i, p) in opt_params.iter().enumerate() {
                let name = if p.is_env {
                    format!("env {}", p.name)
                } else {
                    p.name.clone()
                };
                let name_color = if p.is_env { "#7c3aed" } else { "#0f172a" };
                let (val_str, val_color) = match &p.default_value {
                    Some(def) => (def.clone(), "#334155"),
                    None => ("from env".to_string(), "#7c3aed"),
                };
                rows.push(UnifiedPropRow {
                    section_label: if i == 0 { "Optional Parameters" } else { "" },
                    section_color: "#475569",
                    section_break: i == 0 && has_prev_section,
                    name,
                    name_color,
                    type_display: p.type_name.clone().unwrap_or_else(|| "Any".to_string()),
                    type_anchor: p.type_anchor.as_deref(),
                    value_display: val_str,
                    value_color: val_color,
                });
            }
            has_prev_section = true;
        }

        if !ov.aliases.is_empty() {
            for (i, a) in ov.aliases.iter().enumerate() {
                rows.push(UnifiedPropRow {
                    section_label: if i == 0 { "Aliases" } else { "" },
                    section_color: "#0f766e",
                    section_break: i == 0 && has_prev_section,
                    name: a.name.clone(),
                    name_color: "#0f766e",
                    type_display: a.type_name.clone().unwrap_or_else(|| "Any".to_string()),
                    type_anchor: a.type_anchor.as_deref(),
                    value_display: a.expr.clone(),
                    value_color: "#334155",
                });
            }
            has_prev_section = true;
        }

        if !ov.provided_envs.is_empty() {
            for (i, env) in ov.provided_envs.iter().enumerate() {
                rows.push(UnifiedPropRow {
                    section_label: if i == 0 { "Env for Children" } else { "" },
                    section_color: "#6d28d9",
                    section_break: i == 0 && has_prev_section,
                    name: env.name.clone(),
                    name_color: "#6d28d9",
                    type_display: env.type_name.clone().unwrap_or_else(|| "Any".to_string()),
                    type_anchor: env.type_anchor.as_deref(),
                    value_display: env.expr.clone().unwrap_or_else(|| "—".to_string()),
                    value_color: "#334155",
                });
            }
            has_prev_section = true;
        }

        if !ov.children_pushed_ports.is_empty() {
            for (i, cp) in ov.children_pushed_ports.iter().enumerate() {
                rows.push(UnifiedPropRow {
                    section_label: if i == 0 { "Passed to Children" } else { "" },
                    section_color: "#0369a1",
                    section_break: i == 0 && has_prev_section,
                    name: cp.name.clone(),
                    name_color: "#0369a1",
                    type_display: cp.type_name.clone().unwrap_or_else(|| "Any".to_string()),
                    type_anchor: cp.type_anchor.as_deref(),
                    value_display: cp.expr.clone(),
                    value_color: "#334155",
                });
            }
        }

        if !rows.is_empty() {
            let has_col4 = rows.iter().any(|r| !r.value_display.is_empty());
            let max_col4_chars = rows
                .iter()
                .map(|r| r.value_display.chars().count())
                .max()
                .unwrap_or(0);
            // Monospace Courier New at 7.4pt has ~4.45pt glyph advance; add 16pt horizontal margin.
            let col4_hint_pt = if has_col4 {
                (max_col4_chars as f64 * 4.55 + 16.0).ceil() as u32
            } else {
                0
            };

            let _ = writeln!(
                dt,
                "            \\PropTable(has_col4: {}, col4_hint: {}pt) {{",
                has_col4, col4_hint_pt
            );
            let _ = writeln!(
                dt,
                "                \\PropTableHeader(h1: \"Property\", h2: \"Name\", h3: \"Type\", h4: \"Default / Value\")"
            );
            let last_idx = rows.len().saturating_sub(1);
            for (r_idx, row) in rows.iter().enumerate() {
                let bg = if r_idx % 2 == 1 { "#f8fafc" } else { "#ffffff" };
                let is_last = r_idx == last_idx;
                let next_is_section = rows.get(r_idx + 1).is_some_and(|n| n.section_break);
                let mb = if is_last { "10pt" } else { "0pt" };
                if let Some(anchor) = row.type_anchor {
                    let _ = writeln!(
                        dt,
                        "                \\PropRowLinked(col1: \"{}\", col2: \"{}\", col4: \"{}\", bg: {}, col1_color: {}, col2_color: {}, col4_color: {}, section_break: {}, hide_bottom_rule: {}, is_last: {}, margin_bottom: {}) {{ \\Link(url: \"#{}\"){{{}}} }}",
                        escape_dt_string(row.section_label),
                        escape_dt_string(&row.name),
                        escape_dt_string(&row.value_display),
                        bg,
                        row.section_color,
                        row.name_color,
                        row.value_color,
                        row.section_break,
                        next_is_section,
                        is_last,
                        mb,
                        escape_dt_string(anchor),
                        escape_dt_content(&row.type_display)
                    );
                } else {
                    let _ = writeln!(
                        dt,
                        "                \\PropRow(col1: \"{}\", col2: \"{}\", col3: \"{}\", col4: \"{}\", bg: {}, col1_color: {}, col2_color: {}, col4_color: {}, section_break: {}, hide_bottom_rule: {}, is_last: {}, margin_bottom: {})",
                        escape_dt_string(row.section_label),
                        escape_dt_string(&row.name),
                        escape_dt_string(&row.type_display),
                        escape_dt_string(&row.value_display),
                        bg,
                        row.section_color,
                        row.name_color,
                        row.value_color,
                        row.section_break,
                        next_is_section,
                        is_last,
                        mb
                    );
                }
            }
            let _ = writeln!(dt, "            }}");
        }

        // Assertions / Contracts
        if !ov.asserts.is_empty() {
            for assert in &ov.asserts {
                let line = match &assert.message {
                    Some(msg) => format!("\\assert {} — \"{}\"", assert.condition, msg),
                    None => format!("\\assert {}", assert.condition),
                };
                let _ = writeln!(
                    dt,
                    "            \\DocAssertBox(text: \"{}\")",
                    escape_dt_string(&line)
                );
            }
        }
    }
}

fn emit_bindings_section(dt: &mut String, bindings: &[BindingDoc]) {
    let _ = writeln!(
        dt,
        "            \\DocSubLabel(text: \"Public Bindings & Constants\")"
    );

    // Compute tight monospace widths (+ gentle 16pt horizontal margin) capped to 7.0in (504pt).
    let max_c1_chars = bindings
        .iter()
        .map(|b| b.kind.keyword().len() + 1 + b.name.chars().count())
        .max()
        .unwrap_or(7)
        .max("Binding".len());
    let max_c2_chars = bindings
        .iter()
        .map(|b| b.type_name.as_deref().unwrap_or("Any").chars().count())
        .max()
        .unwrap_or(4)
        .max("Type".len());
    let max_c3_chars = bindings
        .iter()
        .map(|b| b.value.as_deref().unwrap_or("—").chars().count())
        .max()
        .unwrap_or(5)
        .max("Value".len());

    let col1_w = ((max_c1_chars as f64 * 4.75 + 16.0).ceil() as u32).min(210);
    let col2_w = ((max_c2_chars as f64 * 4.75 + 16.0).ceil() as u32).min(140);
    let max_total: u32 = 504;
    let avail_c3 = max_total.saturating_sub(col1_w + col2_w).max(90);
    let col3_w = ((max_c3_chars as f64 * 4.55 + 16.0).ceil() as u32).min(avail_c3);
    let table_w = (col1_w + col2_w + col3_w).min(max_total);

    let _ = writeln!(
        dt,
        "            \\VStackStretch(width: {}pt, gap: 0pt, pagination: \\Pagination(breakable: true)) {{",
        table_w
    );
    let _ = writeln!(
        dt,
        "                \\BindingTableHeader(h1: \"Binding\", h2: \"Type\", h3: \"Value\", col1_w: {}pt, col2_w: {}pt, col3_w: {}pt, table_w: {}pt)",
        col1_w, col2_w, col3_w, table_w
    );
    let last_idx = bindings.len().saturating_sub(1);
    for (idx, b) in bindings.iter().enumerate() {
        let bg = if idx % 2 == 1 { "#f8fafc" } else { "#ffffff" };
        let is_last = idx == last_idx;
        let mb = if is_last { "10pt" } else { "0pt" };
        let col1 = format!("{} {}", b.kind.keyword(), b.name);
        let type_display = b.type_name.as_deref().unwrap_or("Any");
        let val_display = b.value.as_deref().unwrap_or("—");

        let _ = writeln!(
            dt,
            "                \\Anchor(name: \"{}\") {{",
            escape_dt_string(&b.anchor)
        );
        if let Some(anchor) = &b.type_anchor {
            let _ = writeln!(
                dt,
                "                    \\BindingRowLinked(col1: \"{}\", col3: \"{}\", bg: {}, col1_w: {}pt, col2_w: {}pt, col3_w: {}pt, table_w: {}pt, is_last: {}, margin_bottom: {}) {{ \\Link(url: \"#{}\"){{{}}} }}",
                escape_dt_string(&col1),
                escape_dt_string(val_display),
                bg,
                col1_w,
                col2_w,
                col3_w,
                table_w,
                is_last,
                mb,
                escape_dt_string(anchor),
                escape_dt_content(type_display)
            );
        } else {
            let _ = writeln!(
                dt,
                "                    \\BindingRow(col1: \"{}\", col2: \"{}\", col3: \"{}\", bg: {}, col1_w: {}pt, col2_w: {}pt, col3_w: {}pt, table_w: {}pt, is_last: {}, margin_bottom: {})",
                escape_dt_string(&col1),
                escape_dt_string(type_display),
                escape_dt_string(val_display),
                bg,
                col1_w,
                col2_w,
                col3_w,
                table_w,
                is_last,
                mb
            );
        }
        let _ = writeln!(dt, "                }}");
    }
    let _ = writeln!(dt, "            }}");
}

/// Compiles a `DocCatalog` into vector PDF bytes using DirectedType's native PDF backend.
#[cfg(feature = "pdf")]
pub fn compile_catalog_to_pdf(
    catalog: &DocCatalog,
    base_dir: &Path,
    pdf_options: &PdfOptions,
) -> Result<Vec<u8>, CompDocError> {
    let dt_source = render_catalog_to_dt(catalog);
    Ok(compile_source_to_pdf(&dt_source, base_dir, pdf_options)?)
}

/// End-to-end helper: scans `inputs`, extracts public component documentation, renders `.dt`,
/// and compiles directly to vector PDF bytes.
#[cfg(feature = "pdf")]
pub fn generate_docs_pdf(
    inputs: &[PathBuf],
    doc_options: &DocGenOptions,
    pdf_options: &PdfOptions,
) -> Result<(DocCatalog, String, Vec<u8>), CompDocError> {
    let catalog = extract_catalog(inputs, doc_options)?;
    let dt_source = render_catalog_to_dt(&catalog);
    let pdf_bytes = compile_source_to_pdf(&dt_source, &doc_options.root_dir, pdf_options)?;
    Ok((catalog, dt_source, pdf_bytes))
}

// ============================================================================
// Embedded DirectedType Reference Stylesheet
// ============================================================================

const DOC_STYLESHEET_DT: &str = r##"
// Page template with running title, clickable "Contents" link, and "Page X of Y" footer.
\Component DocPage(
    doc_title: String,
    width: Distance = 8.5in,
    height: Distance = 11in,
    x: Distance = page.left,
    y: Distance = page.top,
    margin_x: Distance = 0.75in,
    margin_top: Distance = 0.80in,
    margin_bottom: Distance = 0.78in,
    env font: Font,
    env link_style: LinkStyle
) {
    \let body_x = x + margin_x;
    \let body_w = width - 2 * margin_x;
    \let body_top = y + margin_top;
    \let body_h = height - margin_top - margin_bottom;
    \let show_header = !page.is_first;
    \let chrome_font = \Font(font: font, size: 8pt, weight: 400, line_height: 11pt, color: #64748b);

    \alias right: Distance = x + width;
    \alias bottom: Distance = y + height;

    \Rect(x: x, y: y, width: width, height: height, color: #ffffff)

    // Running header (pages 2+)
    \Text(
        x: body_x,
        y: y + 0.38in,
        width: body_w - 80pt,
        font: chrome_font,
        text: show_header ? doc_title : ""
    )
    \let toc_nav = \Text(
        x: body_x + body_w - toc_nav.width,
        y: y + 0.38in,
        font: \Font(font: chrome_font, weight: 600, color: #1d4ed8)
    ) {
        \Link(url: show_header ? "#toc-top" : "") {
            \Text(text: show_header ? "Contents" : "")
        }
    };
    \Rect(
        x: body_x,
        y: y + 0.57in,
        width: show_header ? body_w : 0pt,
        height: show_header ? 0.5pt : 0pt,
        color: #e2e8f0,
        opacity: show_header ? 1.0 : 0.0
    )

    \FlowBody(
        x: body_x,
        y: body_top,
        width: body_w,
        height: body_h
    )

    // Footer divider & page counter
    \Rect(
        x: body_x,
        y: y + height - 0.56in,
        width: body_w,
        height: 0.5pt,
        color: #e2e8f0
    )
    \Text(
        x: body_x,
        y: y + height - 0.48in,
        width: body_w,
        font: chrome_font,
        paragraph_style: \ParagraphStyle(align: Align.Center),
        text: "Page " + page.number + " of " + page.count
    )
}

// Cover banner at the top of page 1
\Component DocCover(
    title: String,
    subtitle: String = "",
    summary: String = "",
    x: Distance = parent.left,
    y: Distance = parent.top,
    width: Distance = parent.width,
    pagination: Pagination = \Pagination(breakable: false, keep_with_next: true),
    env font: Font
) {
    \Rect(x: x, y: y, width: width, height: 3pt, color: #2563eb, radius: 1.5pt)

    \let t = \Text(
        x: x,
        y: y + 12pt,
        width: width,
        font: \Font(font: font, size: 22pt, weight: 700, line_height: 27pt, color: #0f172a),
        text: title
    );
    \let sub = \Text(
        x: x,
        y: t.bottom + 7pt,
        width: width,
        font: \Font(font: font, size: 10pt, weight: 400, line_height: 14pt, color: #475569),
        text: subtitle
    );
    \let stat_bg_y = sub.bottom + 12pt;
    \let stat = \Text(
        x: x + 10pt,
        y: stat_bg_y + 6pt,
        width: width - 20pt,
        z: 1,
        font: \Font(font: font, size: 8.5pt, weight: 600, line_height: 12pt, color: #1e40af),
        text: summary
    );
    \let stat_h = stat.height + 12pt;
    \Rect(
        x: x,
        y: stat_bg_y,
        width: width,
        height: stat_h,
        z: 0,
        color: #eff6ff,
        border_color: #bfdbfe,
        border_width: 0.75pt,
        radius: 4pt
    )

    \alias right: Distance = x + width;
    \alias height: Distance = (stat_bg_y + stat_h + 10pt) - y;
    \alias bottom: Distance = y + height;
}

// Level-1 Package Heading (discovered by TableOfContents.rs as H1)
\Component H1(
    title: String,
    number: String = "",
    meta: String = "",
    x: Distance = parent.left,
    y: Distance = parent.top,
    width: Distance = parent.width,
    pagination: Pagination = \Pagination(breakable: false, keep_with_next: true, break_before: Break.Page),
    env font: Font
) {
    \let heading = \Text(
        x: x + 12pt,
        y: y + 1pt,
        width: width - 12pt,
        font: \Font(font: font, size: 16pt, weight: 700, line_height: 20pt, color: #0f172a),
        text: title
    );
    \let sub = \Text(
        x: x + 12pt,
        y: heading.bottom + 6pt,
        width: width - 12pt,
        font: \Font(font: font, size: 8.5pt, weight: 400, line_height: 12pt, color: #64748b),
        text: meta
    );
    \let bar_h = max(sub.bottom, y + 34pt) - y;
    \Rect(x: x, y: y, width: 4pt, height: bar_h, color: #2563eb, radius: 2pt)
    \let rule_y = y + bar_h + 8pt;
    \Rect(x: x, y: rule_y, width: width, height: 1pt, color: #cbd5e1)

    \alias right: Distance = x + width;
    \alias height: Distance = (rule_y + 5pt) - y;
    \alias bottom: Distance = y + height;
}

// Level-2 Module / Single-Symbol Heading (discovered by TableOfContents.rs as H2)
\Component H2(
    title: String,
    number: String = "",
    badge: String = "Component",
    badge_bg: Color = #dbeafe,
    badge_fg: Color = #1e40af,
    x: Distance = parent.left,
    y: Distance = parent.top,
    width: Distance = parent.width,
    pagination: Pagination = \Pagination(breakable: false, keep_with_next: true),
    env font: Font
) {
    \let top_pad = 8pt;
    \Rect(x: x, y: y + 1pt, width: width, height: 0.75pt, color: #cbd5e1)

    \let badge_y = y + top_pad + 1pt;
    \let badge_text = \Text(
        x: x + width - badge_text.width - 6pt,
        y: badge_y + 2pt,
        z: 1,
        font: \Font(font: font, size: 7.5pt, weight: 700, line_height: 10pt, color: badge_fg),
        text: badge
    );
    \let badge_w = badge_text.width + 12pt;
    \let badge_h = badge_text.height + 4pt;
    \Rect(
        x: x + width - badge_w,
        y: badge_y,
        width: badge_w,
        height: badge_h,
        z: 0,
        color: badge_bg,
        radius: 3pt
    )

    \let heading = \Text(
        x: x,
        y: y + top_pad,
        width: width - badge_w - 10pt,
        font: \Font(font: font, size: 12.2pt, weight: 700, line_height: 15.5pt, color: #0f172a),
        text: title
    );

    \alias right: Distance = x + width;
    \alias height: Distance = (max(heading.bottom, badge_y + badge_h) + 1pt) - y;
    \alias bottom: Distance = y + height;
}

// Level-3 Component / Enum Heading inside a multi-symbol module (discovered by TableOfContents.rs as H3)
\Component H3(
    title: String,
    number: String = "",
    badge: String = "Component",
    badge_bg: Color = #dbeafe,
    badge_fg: Color = #1e40af,
    x: Distance = parent.left,
    y: Distance = parent.top,
    width: Distance = parent.width,
    pagination: Pagination = \Pagination(breakable: false, keep_with_next: true),
    env font: Font
) {
    \let top_pad = 5pt;
    \let badge_y = y + top_pad + 1pt;
    \let badge_text = \Text(
        x: x + width - badge_text.width - 5pt,
        y: badge_y + 2pt,
        z: 1,
        font: \Font(font: font, size: 7.2pt, weight: 700, line_height: 9.5pt, color: badge_fg),
        text: badge
    );
    \let badge_w = badge_text.width + 10pt;
    \let badge_h = badge_text.height + 4pt;
    \Rect(
        x: x + width - badge_w,
        y: badge_y,
        width: badge_w,
        height: badge_h,
        z: 0,
        color: badge_bg,
        radius: 3pt
    )

    \let heading = \Text(
        x: x,
        y: y + top_pad,
        width: width - badge_w - 8pt,
        font: \Font(font: font, size: 10.8pt, weight: 700, line_height: 14pt, color: #1e293b),
        text: title
    );

    \alias right: Distance = x + width;
    \alias height: Distance = (max(heading.bottom, badge_y + badge_h) + 1pt) - y;
    \alias bottom: Distance = y + height;
}

// Stacked clickable overload signature synopsis box.
\Component DocOverloadSynopsis(
    x: Distance = parent.left,
    y: Distance = parent.top,
    width: Distance = parent.width,
    pagination: Pagination = \Pagination(breakable: false, keep_with_next: true)
) {
    \let pad_x = 8pt;
    \let pad_y = 5pt;
    \let stack = \VStackStretch(
        x: x + pad_x,
        y: y + pad_y,
        width: width - 2 * pad_x,
        gap: 4.5pt
    ) {
        \Children
    };
    \let box_h = stack.height + 2 * pad_y;
    \Rect(
        x: x,
        y: y,
        width: width,
        height: box_h,
        z: 0,
        color: #eff6ff,
        border_color: #bfdbfe,
        border_width: 0.75pt,
        radius: 3.5pt
    )

    \alias right: Distance = x + width;
    \alias height: Distance = box_h;
    \alias bottom: Distance = y + height;
}

\Component DocSubLabel(
    text: String,
    x: Distance = parent.left,
    y: Distance = parent.top,
    width: Distance = parent.width,
    pagination: Pagination = \Pagination(breakable: false, keep_with_next: true),
    env font: Font
) {
    \let t = \Text(
        x: x,
        y: y + 2pt,
        width: width,
        font: \Font(font: font, size: 8pt, weight: 700, line_height: 11pt, color: #475569),
        text: text
    );
    \alias right: Distance = x + width;
    \alias height: Distance = t.bottom - y;
    \alias bottom: Distance = y + height;
}

\let doc_line_bounds = \TextBounds(start_at: TextStart.LineHeight, end_at: TextEnd.LineHeight);
\let doc_cap_bounds = \TextBounds(start_at: TextStart.Capital, end_at: TextEnd.Baseline);

\Component DocSigBox(
    code: String,
    tag: String = "",
    x: Distance = parent.left,
    y: Distance = parent.top,
    width: Distance = parent.width,
    pagination: Pagination = \Pagination(breakable: false, keep_with_next: true),
    env font: Font
) {
    \let pad_x = 8pt;
    \let pad_y = 4pt;
    \let has_tag = tag != "";
    \let tag_text = \Text(
        x: x + width - pad_x - tag_text.width,
        y: y + pad_y + (m.baseline_offset - tag_text.baseline_offset),
        z: 1,
        font: \Font(font: font, size: 7.2pt, weight: 600, line_height: 11pt, color: #64748b),
        text_bounds: doc_line_bounds,
        text: tag
    );
    \let tag_reserve = has_tag ? (tag_text.width + 12pt) : 0pt;
    \let m = \Text(
        x: x + pad_x,
        y: y + pad_y,
        width: width - 2 * pad_x - tag_reserve,
        z: 1,
        font: \Font(font: font, family: "Courier New", size: 8pt, weight: 600, line_height: 11pt, color: #0f172a),
        text_bounds: doc_line_bounds,
        text: code
    );
    \let box_h = max(m.height, has_tag ? tag_text.height : 0pt) + 2 * pad_y;
    \Rect(
        x: x,
        y: y,
        width: width,
        height: box_h,
        z: 0,
        color: #f8fafc,
        border_color: #e2e8f0,
        border_width: 0.75pt,
        radius: 3.5pt
    )

    \alias right: Distance = x + width;
    \alias height: Distance = box_h;
    \alias bottom: Distance = y + height;
}

// Auto-sizing 4-column unified property table (Property | Name | Type | Default / Value).
\Component PropTable(
    has_col4: Boolean = true,
    col4_hint: Distance = 0pt,
    x: Distance = parent.left,
    y: Distance = parent.top,
    width: Distance = parent.width,
    pagination: Pagination = \Pagination(breakable: true)
) {
    \let pad_col = 16pt;
    \let col1_w = max(children.nat_w1) + pad_col;
    \let col2_w = max(children.nat_w2) + pad_col;
    \let col3_w = max(children.nat_w3) + pad_col;
    \let col4_avail = max(80pt, width - col1_w - col2_w - col3_w);
    \let col4_w = has_col4 ? min(max(col4_hint, max(children.nat_w4) + pad_col), col4_avail) : 0pt;
    \let table_w = min(width, col1_w + col2_w + col3_w + col4_w);

    \alias right: Distance = x + table_w;
    \alias height: Distance = sum(children.height);
    \alias bottom: Distance = y + height;

    \Children(
        x: x,
        y: prev ? prev.bottom : y,
        col1_w: col1_w,
        col2_w: col2_w,
        col3_w: col3_w,
        col4_w: col4_w,
        table_w: table_w,
        has_col4: has_col4
    )
}

\Component PropTableHeader(
    h1: String,
    h2: String,
    h3: String,
    h4: String,
    col1_w: Distance = 0pt,
    col2_w: Distance = 0pt,
    col3_w: Distance = 0pt,
    col4_w: Distance = 0pt,
    table_w: Distance = parent.width,
    has_col4: Boolean = true,
    x: Distance = parent.left,
    y: Distance = parent.top,
    pagination: Pagination = \Pagination(breakable: false, keep_with_next: true),
    env font: Font
) {
    \let row_h = 16.5pt;
    \let hdr_font = \Font(font: font, size: 7.4pt, weight: 700, line_height: 11pt, color: #475569);
    \let t1 = \Text(x: x + 6pt, y: y + (row_h - t1.height) / 2, z: 1, font: hdr_font, text_bounds: doc_cap_bounds, text: h1);
    \let t2 = \Text(x: x + col1_w + 6pt, y: y + (row_h - t1.height) / 2, z: 1, font: hdr_font, text_bounds: doc_cap_bounds, text: h2);
    \let t3 = \Text(x: x + col1_w + col2_w + 6pt, y: y + (row_h - t1.height) / 2, z: 1, font: hdr_font, text_bounds: doc_cap_bounds, text: h3);
    \let t4 = \Text(x: x + col1_w + col2_w + col3_w + 6pt, y: y + (row_h - t1.height) / 2, z: 1, font: hdr_font, text_bounds: doc_cap_bounds, text: has_col4 ? h4 : "");

    \Rect(x: x, y: y, width: table_w, height: row_h, z: 0, color: #f1f5f9)
    \Rect(x: x, y: y, width: table_w, height: 0.75pt, z: 2, color: #334155)
    \Rect(x: x, y: y, width: 0.75pt, height: row_h, z: 2, color: #334155)
    \Rect(x: x + table_w - 0.75pt, y: y, width: 0.75pt, height: row_h, z: 2, color: #334155)
    \Rect(x: x, y: y + row_h - 0.75pt, width: table_w, height: 0.75pt, z: 1, color: #64748b)

    \alias nat_w1: Distance = t1.width;
    \alias nat_w2: Distance = t2.width;
    \alias nat_w3: Distance = t3.width;
    \alias nat_w4: Distance = has_col4 ? t4.width : 0pt;
    \alias width: Distance = table_w;
    \alias right: Distance = x + table_w;
    \alias height: Distance = row_h;
    \alias bottom: Distance = y + height;
}

\Component PropRow(
    col1: String,
    col2: String,
    col3: String,
    col4: String,
    bg: Color = #ffffff,
    col1_color: Color = #475569,
    col2_color: Color = #0f172a,
    col3_color: Color = #475569,
    col4_color: Color = #334155,
    section_break: Boolean = false,
    hide_bottom_rule: Boolean = false,
    is_last: Boolean = false,
    margin_bottom: Distance = 0pt,
    col1_w: Distance = 0pt,
    col2_w: Distance = 0pt,
    col3_w: Distance = 0pt,
    col4_w: Distance = 0pt,
    table_w: Distance = parent.width,
    has_col4: Boolean = true,
    x: Distance = parent.left,
    y: Distance = parent.top,
    pagination: Pagination = \Pagination(breakable: false),
    env font: Font
) {
    \let pad_y = 2.5pt;
    \let top_pad = section_break ? (pad_y + 0.75pt) : pad_y;
    \let f1 = \Font(font: font, size: 7.2pt, weight: 700, line_height: 11pt, color: col1_color);
    \let f2 = \Font(font: font, family: "Courier New", size: 7.5pt, weight: 600, line_height: 11pt, color: col2_color);
    \let f3 = \Font(font: font, family: "Courier New", size: 7.5pt, weight: 400, line_height: 11pt, color: col3_color);
    \let f4 = \Font(font: font, family: "Courier New", size: 7.4pt, weight: 400, line_height: 11pt, color: col4_color);

    \let c2 = \Text(x: x + col1_w + 6pt, y: y + top_pad, z: 1, font: f2, text_bounds: doc_line_bounds, text: col2);
    \let c1 = \Text(x: x + 6pt, y: y + top_pad + (c2.baseline_offset - c1.baseline_offset), z: 1, font: f1, text_bounds: doc_line_bounds, text: col1);
    \let c3 = \Text(x: x + col1_w + col2_w + 6pt, y: y + top_pad + (c2.baseline_offset - c3.baseline_offset), z: 1, font: f3, text_bounds: doc_line_bounds, text: col3);
    \let c4 = \Text(
        x: x + col1_w + col2_w + col3_w + 6pt,
        y: y + top_pad + (c2.baseline_offset - c4.baseline_offset),
        width: max(1pt, col4_w - 12pt),
        z: 1,
        font: f4,
        text_bounds: doc_line_bounds,
        text: has_col4 ? col4 : ""
    );
    \let c4_h = has_col4 ? c4.height : 0pt;
    \let content_h = max(max(c2.height, c3.height), c4_h);
    \let row_h = top_pad + content_h + pad_y;

    \Rect(x: x, y: y, width: table_w, height: row_h, z: 0, color: bg)
    \Rect(
        x: x + 0.75pt,
        y: y,
        width: section_break ? max(0pt, table_w - 1.5pt) : 0pt,
        height: section_break ? 0.75pt : 0pt,
        z: 1,
        color: #94a3b8,
        opacity: section_break ? 1.0 : 0.0
    )
    \Rect(x: x, y: y, width: 0.75pt, height: row_h, z: 2, color: #334155)
    \Rect(x: x + table_w - 0.75pt, y: y, width: 0.75pt, height: row_h, z: 2, color: #334155)
    \Rect(
        x: is_last ? x : (x + 0.75pt),
        y: y + row_h - (is_last ? 0.75pt : 0.5pt),
        width: is_last ? table_w : (hide_bottom_rule ? 0pt : max(0pt, table_w - 1.5pt)),
        height: is_last ? 0.75pt : (hide_bottom_rule ? 0pt : 0.5pt),
        z: 2,
        color: is_last ? #334155 : #e2e8f0
    )

    \alias nat_w1: Distance = c1.width;
    \alias nat_w2: Distance = c2.width;
    \alias nat_w3: Distance = c3.width;
    \alias nat_w4: Distance = 0pt;
    \alias width: Distance = table_w;
    \alias right: Distance = x + table_w;
    \alias height: Distance = row_h + margin_bottom;
    \alias bottom: Distance = y + height;
}

\Component PropRowLinked(
    col1: String,
    col2: String,
    col4: String,
    bg: Color = #ffffff,
    col1_color: Color = #475569,
    col2_color: Color = #0f172a,
    col4_color: Color = #334155,
    section_break: Boolean = false,
    hide_bottom_rule: Boolean = false,
    is_last: Boolean = false,
    margin_bottom: Distance = 0pt,
    col1_w: Distance = 0pt,
    col2_w: Distance = 0pt,
    col3_w: Distance = 0pt,
    col4_w: Distance = 0pt,
    table_w: Distance = parent.width,
    has_col4: Boolean = true,
    x: Distance = parent.left,
    y: Distance = parent.top,
    pagination: Pagination = \Pagination(breakable: false),
    env font: Font
) {
    \let pad_y = 2.5pt;
    \let top_pad = section_break ? (pad_y + 0.75pt) : pad_y;
    \let f1 = \Font(font: font, size: 7.2pt, weight: 700, line_height: 11pt, color: col1_color);
    \let f2 = \Font(font: font, family: "Courier New", size: 7.5pt, weight: 600, line_height: 11pt, color: col2_color);
    \let f3 = \Font(font: font, family: "Courier New", size: 7.5pt, weight: 600, line_height: 11pt, color: #1d4ed8);
    \let f4 = \Font(font: font, family: "Courier New", size: 7.4pt, weight: 400, line_height: 11pt, color: col4_color);

    \let c2 = \Text(x: x + col1_w + 6pt, y: y + top_pad, z: 1, font: f2, text_bounds: doc_line_bounds, text: col2);
    \let c1 = \Text(x: x + 6pt, y: y + top_pad + (c2.baseline_offset - c1.baseline_offset), z: 1, font: f1, text_bounds: doc_line_bounds, text: col1);
    \let c3 = \Text(x: x + col1_w + col2_w + 6pt, y: y + top_pad + (c2.baseline_offset - c3.baseline_offset), z: 1, font: f3, text_bounds: doc_line_bounds) {
        \Children
    };
    \let c4 = \Text(
        x: x + col1_w + col2_w + col3_w + 6pt,
        y: y + top_pad + (c2.baseline_offset - c4.baseline_offset),
        width: max(1pt, col4_w - 12pt),
        z: 1,
        font: f4,
        text_bounds: doc_line_bounds,
        text: has_col4 ? col4 : ""
    );
    \let c4_h = has_col4 ? c4.height : 0pt;
    \let content_h = max(max(c2.height, c3.height), c4_h);
    \let row_h = top_pad + content_h + pad_y;

    \Rect(x: x, y: y, width: table_w, height: row_h, z: 0, color: bg)
    \Rect(
        x: x + 0.75pt,
        y: y,
        width: section_break ? max(0pt, table_w - 1.5pt) : 0pt,
        height: section_break ? 0.75pt : 0pt,
        z: 1,
        color: #94a3b8,
        opacity: section_break ? 1.0 : 0.0
    )
    \Rect(x: x, y: y, width: 0.75pt, height: row_h, z: 2, color: #334155)
    \Rect(x: x + table_w - 0.75pt, y: y, width: 0.75pt, height: row_h, z: 2, color: #334155)
    \Rect(
        x: is_last ? x : (x + 0.75pt),
        y: y + row_h - (is_last ? 0.75pt : 0.5pt),
        width: is_last ? table_w : (hide_bottom_rule ? 0pt : max(0pt, table_w - 1.5pt)),
        height: is_last ? 0.75pt : (hide_bottom_rule ? 0pt : 0.5pt),
        z: 2,
        color: is_last ? #334155 : #e2e8f0
    )

    \alias nat_w1: Distance = c1.width;
    \alias nat_w2: Distance = c2.width;
    \alias nat_w3: Distance = c3.width;
    \alias nat_w4: Distance = 0pt;
    \alias width: Distance = table_w;
    \alias right: Distance = x + table_w;
    \alias height: Distance = row_h + margin_bottom;
    \alias bottom: Distance = y + height;
}

// Auto-sizing 3-column table for Enums (Variant | Qualified Value | Index).
\Component Table3(
    x: Distance = parent.left,
    y: Distance = parent.top,
    width: Distance = parent.width,
    pagination: Pagination = \Pagination(breakable: true)
) {
    \let pad_col = 16pt;
    \let col1_w = max(children.nat_w1) + pad_col;
    \let col2_w = max(children.nat_w2) + pad_col;
    \let col3_w = max(children.nat_w3) + pad_col;
    \let table_w = min(width, col1_w + col2_w + col3_w);

    \alias right: Distance = x + table_w;
    \alias height: Distance = sum(children.height);
    \alias bottom: Distance = y + height;

    \Children(
        x: x,
        y: prev ? prev.bottom : y,
        col1_w: col1_w,
        col2_w: col2_w,
        col3_w: col3_w,
        table_w: table_w
    )
}

\Component Table3Header(
    h1: String,
    h2: String,
    h3: String,
    col1_w: Distance = 0pt,
    col2_w: Distance = 0pt,
    col3_w: Distance = 0pt,
    table_w: Distance = parent.width,
    x: Distance = parent.left,
    y: Distance = parent.top,
    pagination: Pagination = \Pagination(breakable: false, keep_with_next: true),
    env font: Font
) {
    \let row_h = 16.5pt;
    \let hdr_font = \Font(font: font, size: 7.4pt, weight: 700, line_height: 11pt, color: #475569);
    \let t1 = \Text(x: x + 6pt, y: y + (row_h - t1.height) / 2, z: 1, font: hdr_font, text_bounds: doc_cap_bounds, text: h1);
    \let t2 = \Text(x: x + col1_w + 6pt, y: y + (row_h - t1.height) / 2, z: 1, font: hdr_font, text_bounds: doc_cap_bounds, text: h2);
    \let t3 = \Text(x: x + col1_w + col2_w + 6pt, y: y + (row_h - t1.height) / 2, z: 1, font: hdr_font, text_bounds: doc_cap_bounds, text: h3);

    \Rect(x: x, y: y, width: table_w, height: row_h, z: 0, color: #f1f5f9)
    \Rect(x: x, y: y, width: table_w, height: 0.75pt, z: 2, color: #334155)
    \Rect(x: x, y: y, width: 0.75pt, height: row_h, z: 2, color: #334155)
    \Rect(x: x + table_w - 0.75pt, y: y, width: 0.75pt, height: row_h, z: 2, color: #334155)
    \Rect(x: x, y: y + row_h - 0.75pt, width: table_w, height: 0.75pt, z: 1, color: #64748b)

    \alias nat_w1: Distance = t1.width;
    \alias nat_w2: Distance = t2.width;
    \alias nat_w3: Distance = t3.width;
    \alias width: Distance = table_w;
    \alias right: Distance = x + table_w;
    \alias height: Distance = row_h;
    \alias bottom: Distance = y + height;
}

\Component Table3Row(
    col1: String,
    col2: String,
    col3: String,
    bg: Color = #ffffff,
    is_last: Boolean = false,
    margin_bottom: Distance = 0pt,
    col1_w: Distance = 0pt,
    col2_w: Distance = 0pt,
    col3_w: Distance = 0pt,
    table_w: Distance = parent.width,
    x: Distance = parent.left,
    y: Distance = parent.top,
    pagination: Pagination = \Pagination(breakable: false),
    env font: Font
) {
    \let pad_y = 2.5pt;
    \let f1 = \Font(font: font, family: "Courier New", size: 7.5pt, weight: 600, line_height: 11pt, color: #0f172a);
    \let f2 = \Font(font: font, family: "Courier New", size: 7.5pt, weight: 400, line_height: 11pt, color: #475569);
    \let f3 = \Font(font: font, family: "Courier New", size: 7.4pt, weight: 400, line_height: 11pt, color: #64748b);

    \let c1 = \Text(x: x + 6pt, y: y + pad_y, z: 1, font: f1, text_bounds: doc_line_bounds, text: col1);
    \let c2 = \Text(x: x + col1_w + 6pt, y: y + pad_y + (c1.baseline_offset - c2.baseline_offset), z: 1, font: f2, text_bounds: doc_line_bounds, text: col2);
    \let c3 = \Text(x: x + col1_w + col2_w + 6pt, y: y + pad_y + (c1.baseline_offset - c3.baseline_offset), z: 1, font: f3, text_bounds: doc_line_bounds, text: col3);
    \let row_h = max(max(c1.height, c2.height), c3.height) + 2 * pad_y;

    \Rect(x: x, y: y, width: table_w, height: row_h, z: 0, color: bg)
    \Rect(x: x, y: y, width: 0.75pt, height: row_h, z: 2, color: #334155)
    \Rect(x: x + table_w - 0.75pt, y: y, width: 0.75pt, height: row_h, z: 2, color: #334155)
    \Rect(
        x: is_last ? x : (x + 0.75pt),
        y: y + row_h - (is_last ? 0.75pt : 0.5pt),
        width: is_last ? table_w : max(0pt, table_w - 1.5pt),
        height: is_last ? 0.75pt : 0.5pt,
        z: 2,
        color: is_last ? #334155 : #e2e8f0
    )

    \alias nat_w1: Distance = c1.width;
    \alias nat_w2: Distance = c2.width;
    \alias nat_w3: Distance = c3.width;
    \alias width: Distance = table_w;
    \alias right: Distance = x + table_w;
    \alias height: Distance = row_h + margin_bottom;
    \alias bottom: Distance = y + height;
}

// Compact auto-fitted 3-column rows for anchored Public Bindings.
\Component BindingTableHeader(
    h1: String,
    h2: String,
    h3: String,
    col1_w: Distance,
    col2_w: Distance,
    col3_w: Distance,
    table_w: Distance,
    x: Distance = parent.left,
    y: Distance = parent.top,
    width: Distance = parent.width,
    pagination: Pagination = \Pagination(breakable: false, keep_with_next: true),
    env font: Font
) {
    \let row_h = 16.5pt;
    \let hdr_font = \Font(font: font, size: 7.4pt, weight: 700, line_height: 11pt, color: #475569);
    \Rect(x: x, y: y, width: table_w, height: row_h, z: 0, color: #f1f5f9)
    \let t1 = \Text(x: x + 6pt, y: y + (row_h - t1.height) / 2, z: 1, font: hdr_font, text_bounds: doc_cap_bounds, text: h1);
    \Text(x: x + col1_w + 6pt, y: y + (row_h - t1.height) / 2, z: 1, font: hdr_font, text_bounds: doc_cap_bounds, text: h2)
    \Text(x: x + col1_w + col2_w + 6pt, y: y + (row_h - t1.height) / 2, z: 1, font: hdr_font, text_bounds: doc_cap_bounds, text: h3)
    \Rect(x: x, y: y, width: table_w, height: 0.75pt, z: 2, color: #334155)
    \Rect(x: x, y: y, width: 0.75pt, height: row_h, z: 2, color: #334155)
    \Rect(x: x + table_w - 0.75pt, y: y, width: 0.75pt, height: row_h, z: 2, color: #334155)
    \Rect(x: x, y: y + row_h - 0.75pt, width: table_w, height: 0.75pt, z: 1, color: #64748b)

    \alias right: Distance = x + table_w;
    \alias height: Distance = row_h;
    \alias bottom: Distance = y + height;
}

\Component BindingRow(
    col1: String,
    col2: String,
    col3: String,
    bg: Color = #ffffff,
    is_last: Boolean = false,
    margin_bottom: Distance = 0pt,
    col1_w: Distance,
    col2_w: Distance,
    col3_w: Distance,
    table_w: Distance,
    x: Distance = parent.left,
    y: Distance = parent.top,
    width: Distance = parent.width,
    pagination: Pagination = \Pagination(breakable: false),
    env font: Font
) {
    \let pad_y = 2.5pt;
    \let f1 = \Font(font: font, family: "Courier New", size: 7.5pt, weight: 600, line_height: 11pt, color: #0f172a);
    \let f2 = \Font(font: font, family: "Courier New", size: 7.5pt, weight: 400, line_height: 11pt, color: #475569);
    \let f3 = \Font(font: font, family: "Courier New", size: 7.4pt, weight: 400, line_height: 11pt, color: #334155);

    \let c1 = \Text(x: x + 6pt, y: y + pad_y, width: max(1pt, col1_w - 12pt), z: 1, font: f1, text_bounds: doc_line_bounds, text: col1);
    \let c2 = \Text(x: x + col1_w + 6pt, y: y + pad_y + (c1.baseline_offset - c2.baseline_offset), width: max(1pt, col2_w - 12pt), z: 1, font: f2, text_bounds: doc_line_bounds, text: col2);
    \let c3 = \Text(x: x + col1_w + col2_w + 6pt, y: y + pad_y + (c1.baseline_offset - c3.baseline_offset), width: max(1pt, col3_w - 12pt), z: 1, font: f3, text_bounds: doc_line_bounds, text: col3);
    \let row_h = max(max(c1.height, c2.height), c3.height) + 2 * pad_y;

    \Rect(x: x, y: y, width: table_w, height: row_h, z: 0, color: bg)
    \Rect(x: x, y: y, width: 0.75pt, height: row_h, z: 2, color: #334155)
    \Rect(x: x + table_w - 0.75pt, y: y, width: 0.75pt, height: row_h, z: 2, color: #334155)
    \Rect(
        x: is_last ? x : (x + 0.75pt),
        y: y + row_h - (is_last ? 0.75pt : 0.5pt),
        width: is_last ? table_w : max(0pt, table_w - 1.5pt),
        height: is_last ? 0.75pt : 0.5pt,
        z: 2,
        color: is_last ? #334155 : #e2e8f0
    )

    \alias right: Distance = x + table_w;
    \alias height: Distance = row_h + margin_bottom;
    \alias bottom: Distance = y + height;
}

\Component BindingRowLinked(
    col1: String,
    col3: String,
    bg: Color = #ffffff,
    is_last: Boolean = false,
    margin_bottom: Distance = 0pt,
    col1_w: Distance,
    col2_w: Distance,
    col3_w: Distance,
    table_w: Distance,
    x: Distance = parent.left,
    y: Distance = parent.top,
    width: Distance = parent.width,
    pagination: Pagination = \Pagination(breakable: false),
    env font: Font
) {
    \let pad_y = 2.5pt;
    \let f1 = \Font(font: font, family: "Courier New", size: 7.5pt, weight: 600, line_height: 11pt, color: #0f172a);
    \let f2 = \Font(font: font, family: "Courier New", size: 7.5pt, weight: 600, line_height: 11pt, color: #1d4ed8);
    \let f3 = \Font(font: font, family: "Courier New", size: 7.4pt, weight: 400, line_height: 11pt, color: #334155);

    \let c1 = \Text(x: x + 6pt, y: y + pad_y, width: max(1pt, col1_w - 12pt), z: 1, font: f1, text_bounds: doc_line_bounds, text: col1);
    \let c2 = \Text(x: x + col1_w + 6pt, y: y + pad_y + (c1.baseline_offset - c2.baseline_offset), width: max(1pt, col2_w - 12pt), z: 1, font: f2, text_bounds: doc_line_bounds) {
        \Children
    };
    \let c3 = \Text(x: x + col1_w + col2_w + 6pt, y: y + pad_y + (c1.baseline_offset - c3.baseline_offset), width: max(1pt, col3_w - 12pt), z: 1, font: f3, text_bounds: doc_line_bounds, text: col3);
    \let row_h = max(max(c1.height, c2.height), c3.height) + 2 * pad_y;

    \Rect(x: x, y: y, width: table_w, height: row_h, z: 0, color: bg)
    \Rect(x: x, y: y, width: 0.75pt, height: row_h, z: 2, color: #334155)
    \Rect(x: x + table_w - 0.75pt, y: y, width: 0.75pt, height: row_h, z: 2, color: #334155)
    \Rect(
        x: is_last ? x : (x + 0.75pt),
        y: y + row_h - (is_last ? 0.75pt : 0.5pt),
        width: is_last ? table_w : max(0pt, table_w - 1.5pt),
        height: is_last ? 0.75pt : 0.5pt,
        z: 2,
        color: is_last ? #334155 : #e2e8f0
    )

    \alias right: Distance = x + table_w;
    \alias height: Distance = row_h + margin_bottom;
    \alias bottom: Distance = y + height;
}

\Component DocAssertBox(
    text: String,
    x: Distance = parent.left,
    y: Distance = parent.top,
    width: Distance = parent.width,
    pagination: Pagination = \Pagination(breakable: false),
    env font: Font
) {
    \let pad_x = 8pt;
    \let pad_y = 4pt;
    \let f = \Font(font: font, family: "Courier New", size: 7.6pt, weight: 400, line_height: 10.5pt, color: #92400e);
    \let t = \Text(x: x + pad_x, y: y + pad_y, width: width - 2 * pad_x, z: 1, font: f, text: text);
    \let h = t.height + 2 * pad_y;
    \Rect(x: x, y: y, width: width, height: h, z: 0, color: #fffbeb, border_color: #fde68a, border_width: 0.5pt, radius: 3pt)

    \alias right: Distance = x + width;
    \alias height: Distance = h;
    \alias bottom: Distance = y + height;
}
"##;
