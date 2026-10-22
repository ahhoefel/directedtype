use crate::ast::{
    ComponentDef, ContentItem, Document, ElementNode, Expr, Ident, Item, LetValue,
    UseDeclaration,
};
use crate::compiler::error::{AmbiguousOverloadDetails, CompileError};
use crate::parser::parse_document_with_file;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Trait for resolving and loading source files (supports disk and in-memory virtual filesystems).
pub trait FileResolver {
    fn read(&self, path: &Path) -> Result<String, String>;
    fn canonicalize(&self, path: &Path) -> Result<PathBuf, String> {
        Ok(normalize_path(path))
    }
    fn read_bytes(&self, path: &Path) -> Result<Vec<u8>, String> {
        std::fs::read(path).map_err(|e| e.to_string())
    }
    /// Checks if a companion Rust file exists for the given DirectedType file.
    fn find_companion_rs(&self, dt_path: &Path) -> Option<PathBuf> {
        if dt_path.extension().and_then(|e| e.to_str()) == Some("dt") {
            let rs_path = dt_path.with_extension("rs");
            if self.read(&rs_path).is_ok() {
                return Some(rs_path);
            }
        }
        None
    }
    /// Checks if a companion WebAssembly file exists for the given DirectedType file.
    fn find_companion_wasm(&self, dt_path: &Path) -> Option<PathBuf> {
        if dt_path.extension().and_then(|e| e.to_str()) == Some("dt") {
            let wasm_path = dt_path.with_extension("wasm");
            if self.read_bytes(&wasm_path).is_ok() {
                return Some(wasm_path);
            }
        }
        None
    }
}

/// Default filesystem resolver using `std::fs`.
#[derive(Debug, Default, Clone, Copy)]
pub struct FsResolver;

impl FileResolver for FsResolver {
    fn read(&self, path: &Path) -> Result<String, String> {
        std::fs::read_to_string(path).map_err(|e| e.to_string())
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, String> {
        std::fs::canonicalize(path).map_err(|e| e.to_string())
    }
}

/// In-memory virtual resolver useful for testing multi-file imports without touching the filesystem.
#[derive(Debug, Default, Clone)]
pub struct VirtualResolver {
    files: HashMap<PathBuf, String>,
    binary_files: HashMap<PathBuf, Vec<u8>>,
}

impl VirtualResolver {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
            binary_files: HashMap::new(),
        }
    }

    pub fn insert(&mut self, path: impl Into<PathBuf>, content: impl Into<String>) {
        let p = path.into();
        let norm = normalize_path(&p);
        let s = content.into();
        self.files.insert(norm, s.clone());
        self.files.insert(p, s);
    }

    pub fn insert_bytes(&mut self, path: impl Into<PathBuf>, bytes: impl Into<Vec<u8>>) {
        let p = path.into();
        let norm = normalize_path(&p);
        let b = bytes.into();
        self.binary_files.insert(norm, b.clone());
        self.binary_files.insert(p, b);
    }
}

impl FileResolver for VirtualResolver {
    fn read(&self, path: &Path) -> Result<String, String> {
        let norm = normalize_path(path);
        if let Some(content) = self.files.get(&norm) {
            return Ok(content.clone());
        }
        if let Some(content) = self.files.get(path) {
            return Ok(content.clone());
        }
        Err(format!("File not found in virtual filesystem: {}", path.display()))
    }

    fn read_bytes(&self, path: &Path) -> Result<Vec<u8>, String> {
        let norm = normalize_path(path);
        if let Some(bytes) = self.binary_files.get(&norm) {
            return Ok(bytes.clone());
        }
        if let Some(bytes) = self.binary_files.get(path) {
            return Ok(bytes.clone());
        }
        Err(format!("Binary file not found in virtual filesystem: {}", path.display()))
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, String> {
        Ok(normalize_path(path))
    }
}

/// Pure path normalization without requiring filesystem existence (resolves `.` and `..`).
pub fn normalize_path(path: &Path) -> PathBuf {
    let mut prefix = None;
    let mut is_absolute = false;
    let mut components = Vec::new();

    for comp in path.components() {
        match comp {
            std::path::Component::Prefix(p) => {
                prefix = Some(p.as_os_str());
            }
            std::path::Component::RootDir => {
                is_absolute = true;
                components.clear();
            }
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if let Some(last) = components.last() {
                    if last != &std::ffi::OsStr::new("..") {
                        components.pop();
                        continue;
                    }
                }
                if !is_absolute {
                    components.push(std::ffi::OsStr::new(".."));
                }
            }
            std::path::Component::Normal(c) => {
                components.push(c);
            }
        }
    }

    let mut res = PathBuf::new();
    if let Some(p) = prefix {
        res.push(p);
    }
    if is_absolute {
        res.push(std::path::MAIN_SEPARATOR.to_string());
    }
    for c in components {
        res.push(c);
    }

    if res.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        res
    }
}

/// Registers a component overload into the registry, performing strict definition-time ambiguity
/// and duplicate checks.
///
/// If any two overloads could possibly accept the same set of ports for any valid caller invocation,
/// this returns `CompileError::PotentiallyAmbiguousOverloads`.
///
/// An exact duplicate signature returns `CompileError::DuplicateOverloadSignature`.
///
/// If an identical `ComponentDef` is already registered (e.g. via diamond dependency import),
/// this operation is an idempotent no-op and returns `Ok(())`.
pub fn register_component_overload(
    registry: &mut HashMap<String, Vec<ComponentDef>>,
    comp: ComponentDef,
) -> Result<(), CompileError> {
    if comp.name.as_str() == "Window" {
        return Err(CompileError::Custom {
            message: "Cannot define component named 'Window'; 'Window' is reserved for ambient window configuration".to_string(),
            span: comp.name.span,
        });
    }

    let name = comp.name.as_str().to_string();
    let overloads = registry.entry(name.clone()).or_default();

    for existing in overloads.iter() {
        // Idempotent re-import / diamond import: exactly identical component definition
        if existing == &comp
            || (existing.name.as_str() == comp.name.as_str() && existing.span == comp.span)
        {
            return Ok(());
        }

        let req_existing = existing.required_param_names();
        let all_existing = existing.param_names();
        let req_new = comp.required_param_names();
        let all_new = comp.param_names();

        // 1. Exact duplicate signature
        if req_existing == req_new && all_existing == all_new {
            let signature: Vec<String> = comp
                .params
                .iter()
                .map(|p| p.name.as_str().to_string())
                .collect();
            return Err(CompileError::DuplicateOverloadSignature {
                name,
                signature,
                span: existing.span,
                second_span: comp.span,
            });
        }

        // 2. Strict Definition-Time Ambiguity Check:
        // A potential ambiguity exists iff Req(O1) ⊆ All(O2) AND Req(O2) ⊆ All(O1).
        if req_existing.is_subset(&all_new) && req_new.is_subset(&all_existing) {
            let mut witness: Vec<String> = req_existing.union(&req_new).cloned().collect();
            witness.sort();
            let sig_a: Vec<String> = existing
                .params
                .iter()
                .map(|p| p.name.as_str().to_string())
                .collect();
            let sig_b: Vec<String> = comp
                .params
                .iter()
                .map(|p| p.name.as_str().to_string())
                .collect();
            return Err(CompileError::PotentiallyAmbiguousOverloads(Box::new(
                AmbiguousOverloadDetails {
                    name,
                    signature_a: sig_a,
                    signature_b: sig_b,
                    witness_overlap: witness,
                    span: existing.span,
                    second_span: comp.span,
                },
            )));
        }
    }

    overloads.push(comp);
    Ok(())
}

/// Statically verifies that a slice of component overloads sharing the same name are mutually disjoint.
pub fn verify_overload_set(name: &str, overloads: &[ComponentDef]) -> Result<(), CompileError> {
    for i in 0..overloads.len() {
        for j in (i + 1)..overloads.len() {
            let o1 = &overloads[i];
            let o2 = &overloads[j];

            if o1 == o2 || (o1.name.as_str() == o2.name.as_str() && o1.span == o2.span) {
                continue;
            }

            let req1 = o1.required_param_names();
            let all1 = o1.param_names();
            let req2 = o2.required_param_names();
            let all2 = o2.param_names();

            if req1 == req2 && all1 == all2 {
                let signature: Vec<String> = o2
                    .params
                    .iter()
                    .map(|p| p.name.as_str().to_string())
                    .collect();
                return Err(CompileError::DuplicateOverloadSignature {
                    name: name.to_string(),
                    signature,
                    span: o1.span,
                    second_span: o2.span,
                });
            }

            if req1.is_subset(&all2) && req2.is_subset(&all1) {
                let mut witness: Vec<String> = req1.union(&req2).cloned().collect();
                witness.sort();
                let sig_a: Vec<String> = o1
                    .params
                    .iter()
                    .map(|p| p.name.as_str().to_string())
                    .collect();
                let sig_b: Vec<String> = o2
                    .params
                    .iter()
                    .map(|p| p.name.as_str().to_string())
                    .collect();
                return Err(CompileError::PotentiallyAmbiguousOverloads(Box::new(
                    AmbiguousOverloadDetails {
                        name: name.to_string(),
                        signature_a: sig_a,
                        signature_b: sig_b,
                        witness_overlap: witness,
                        span: o1.span,
                        second_span: o2.span,
                    },
                )));
            }
        }
    }
    Ok(())
}

/// Resolves all `\use` declarations in a document and returns the merged component registry and imported items.
pub fn resolve_imports<R: FileResolver>(
    doc: &Document,
    base_dir: &Path,
    resolver: &R,
) -> Result<(HashMap<String, Vec<ComponentDef>>, Vec<crate::ast::Item>), CompileError> {
    let (registry, imported_items, _) = resolve_imports_with_enums(doc, base_dir, resolver)?;
    Ok((registry, imported_items))
}

/// Resolves all `\use` declarations in a document and returns the merged component registry, imported items,
/// and all enums defined across imported modules (for type validation inside imported components).
pub fn resolve_imports_with_enums<R: FileResolver>(
    doc: &Document,
    base_dir: &Path,
    resolver: &R,
) -> Result<
    (
        HashMap<String, Vec<ComponentDef>>,
        Vec<crate::ast::Item>,
        HashMap<String, crate::ast::EnumDef>,
    ),
    CompileError,
> {
    let mut registry: HashMap<String, Vec<ComponentDef>> = HashMap::new();
    let mut imported_items = Vec::new();
    let mut active_stack = HashSet::new();
    let mut cache = HashMap::new();
    let mut module_exports = HashMap::new();
    let mut visited_files = HashSet::new();
    let mut all_module_enums = HashMap::new();

    // 1. Index locally declared components in the root document
    for item in &doc.items {
        if let Item::Component(comp) = item {
            register_component_overload(&mut registry, comp.clone())?;
        }
    }

    // 2. Resolve \use declarations
    for item in &doc.items {
        if let Item::Use(u) = item {
            resolve_use_decl(
                u,
                base_dir,
                resolver,
                &mut registry,
                &mut imported_items,
                &mut active_stack,
                &mut cache,
                &mut module_exports,
                &mut visited_files,
                &mut all_module_enums,
            )?;
        }
    }

    Ok((registry, imported_items, all_module_enums))
}

#[allow(clippy::too_many_arguments)]
fn resolve_use_decl<R: FileResolver>(
    u: &UseDeclaration,
    base_dir: &Path,
    resolver: &R,
    registry: &mut HashMap<String, Vec<ComponentDef>>,
    imported_items: &mut Vec<crate::ast::Item>,
    active_stack: &mut HashSet<PathBuf>,
    cache: &mut HashMap<PathBuf, Document>,
    module_exports: &mut HashMap<PathBuf, HashMap<String, Vec<ComponentDef>>>,
    visited_files: &mut HashSet<PathBuf>,
    all_module_enums: &mut HashMap<String, crate::ast::EnumDef>,
) -> Result<(), CompileError> {
    let raw_path = Path::new(&u.path);
    let mut resolved_path = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        base_dir.join(raw_path)
    };

    if !raw_path.is_absolute() && resolver.read(&resolved_path).is_err() {
        if resolver.read(raw_path).is_ok() {
            resolved_path = raw_path.to_path_buf();
        } else if let Ok(stripped) = raw_path.strip_prefix("..") {
            if resolver.read(stripped).is_ok() {
                resolved_path = stripped.to_path_buf();
            } else if resolver.read(&base_dir.join(stripped)).is_ok() {
                resolved_path = base_dir.join(stripped);
            }
        }
    }

    let canonical_path = resolver
        .canonicalize(&resolved_path)
        .unwrap_or_else(|_| normalize_path(&resolved_path));

    // Cycle detection on the active resolution stack
    if !active_stack.insert(canonical_path.clone()) {
        return Err(CompileError::CyclicImport {
            path: u.path.clone(),
            span: u.span,
        });
    }

    let is_first_visit = visited_files.insert(canonical_path.clone());

    // Fetch parsed document from cache or load and parse it
    let imported_doc = if let Some(cached) = cache.get(&canonical_path) {
        cached.clone()
    } else {
        let source = resolver.read(&resolved_path).map_err(|e| CompileError::ImportError {
            path: u.path.clone(),
            message: e,
            span: u.span,
        })?;

        let parsed = parse_document_with_file(&source, &resolved_path).map_err(|e| {
            CompileError::ImportError {
                path: u.path.clone(),
                message: format!("Parse error in '{}': {}", u.path, e),
                span: u.span,
            }
        })?;

        cache.insert(canonical_path.clone(), parsed.clone());
        parsed
    };

    let imported_base_dir = resolved_path.parent().unwrap_or(Path::new("."));

    // Transitive imports first
    let mut nested_imported_items = Vec::new();
    for item in &imported_doc.items {
        if let Item::Use(nested_u) = item {
            resolve_use_decl(
                nested_u,
                imported_base_dir,
                resolver,
                registry,
                if u.is_named() {
                    &mut nested_imported_items
                } else {
                    imported_items
                },
                active_stack,
                cache,
                module_exports,
                visited_files,
                all_module_enums,
            )?;
        }
    }

    // Collect all enums defined directly in this imported document
    let mut available_enums: HashMap<String, crate::ast::EnumDef> = HashMap::new();
    for item in &imported_doc.items {
        if let Item::Enum(e) = item {
            available_enums.insert(e.name.as_str().to_string(), e.clone());
            all_module_enums.insert(e.name.as_str().to_string(), e.clone());
        }
    }

    // Collect all enums in scope inside this module (including transitive imports)
    let mut module_scope_enums = available_enums.clone();
    for item in imported_items.iter().chain(nested_imported_items.iter()) {
        if let Item::Enum(e) = item {
            module_scope_enums.insert(e.name.as_str().to_string(), e.clone());
        }
    }

    // Collect components from the imported file
    let imported_components = if let Some(exported) = module_exports.get(&canonical_path) {
        exported.clone()
    } else {
        let mut components: HashMap<String, Vec<ComponentDef>> = HashMap::new();
        for item in &imported_doc.items {
            if let Item::Component(comp) = item {
                let mut resolved_comp = comp.clone();
                resolve_component_enum_literals(&mut resolved_comp, &module_scope_enums);
                register_component_overload(&mut components, resolved_comp)?;
            }
        }
        module_exports.insert(canonical_path.clone(), components.clone());
        components
    };

    if !u.items.is_empty() {
        // Named symbol imports: \use { a, b as c } from "..."
        for use_item in &u.items {
            let symbol_name = use_item.name.as_str();
            let target_name = use_item.bound_name();
            let mut found = false;

            // 1. Check if it's a component
            if imported_components.contains_key(symbol_name) {
                found = true;
            }

            // 2. Check if it's an available enum
            if let Some(e) = available_enums.get(symbol_name) {
                found = true;
                let mut target_enum = e.clone();
                if let Some(alias) = &use_item.alias {
                    target_enum.name = Ident::new(alias.as_str(), alias.span);
                    all_module_enums.insert(alias.as_str().to_string(), target_enum.clone());
                }
                if !imported_items.iter().any(|existing| match existing {
                    Item::Enum(ex_e) => ex_e.name.as_str() == target_name,
                    _ => false,
                }) {
                    imported_items.push(Item::Enum(target_enum));
                }
            }

            // 3. Check top-level items (Let, Env, State)
            for item in &imported_doc.items {
                match item {
                    Item::Let(l) if l.name.as_str() == symbol_name => {
                        found = true;
                        let mut target_let = l.clone();
                        if let Some(val) = &mut target_let.value {
                            match val {
                                crate::ast::LetValue::Node(n) => resolve_element_node_enum_literals(n, &module_scope_enums),
                                crate::ast::LetValue::Expr(e) => resolve_expr_enum_literals(e, &module_scope_enums),
                            }
                        }
                        if let Some(alias) = &use_item.alias {
                            target_let.name = Ident::new(alias.as_str(), alias.span);
                        }
                        if !imported_items.iter().any(|existing| match existing {
                            Item::Let(ex_l) => ex_l.name.as_str() == target_name && ex_l.span == target_let.span,
                            _ => false,
                        }) {
                            imported_items.push(Item::Let(target_let));
                        }
                    }
                    Item::Env(env) if env.name.as_str() == symbol_name => {
                        found = true;
                        let mut target_env = env.clone();
                        if let Some(e) = &mut target_env.value {
                            resolve_expr_enum_literals(e, &module_scope_enums);
                        }
                        if let Some(alias) = &use_item.alias {
                            target_env.name = Ident::new(alias.as_str(), alias.span);
                        }
                        if !imported_items.iter().any(|existing| match existing {
                            Item::Env(ex_env) => ex_env.name.as_str() == target_name && ex_env.span == target_env.span,
                            _ => false,
                        }) {
                            imported_items.push(Item::Env(target_env));
                        }
                    }
                    Item::State(s) if s.name.as_str() == symbol_name => {
                        found = true;
                        let mut target_state = s.clone();
                        if let Some(e) = &mut target_state.default {
                            resolve_expr_enum_literals(e, &module_scope_enums);
                        }
                        if let Some(alias) = &use_item.alias {
                            target_state.name = Ident::new(alias.as_str(), alias.span);
                        }
                        if !imported_items.iter().any(|existing| match existing {
                            Item::State(ex_s) => ex_s.name.as_str() == target_name && ex_s.span == target_state.span,
                            _ => false,
                        }) {
                            imported_items.push(Item::State(target_state));
                        }
                    }
                    _ => {}
                }
            }

            if !found {
                return Err(CompileError::ImportError {
                    path: u.path.clone(),
                    message: format!(
                        "Cannot import '{}': symbol not found in '{}'",
                        symbol_name, u.path
                    ),
                    span: use_item.name.span,
                });
            }
        }

        // Compute reachable components from the directly imported component symbols
        let mut reachable: HashSet<String> = HashSet::new();
        let mut worklist: Vec<String> = u
            .items
            .iter()
            .map(|it| it.name.as_str().to_string())
            .filter(|name| imported_components.contains_key(name))
            .collect();

        while let Some(comp_name) = worklist.pop() {
            if reachable.insert(comp_name.clone()) {
                if let Some(overloads) = imported_components.get(&comp_name) {
                    for o in overloads {
                        let mut refs = HashSet::new();
                        collect_referenced_component_names(o, &mut refs);
                        for r in refs {
                            if imported_components.contains_key(&r) && !reachable.contains(&r) {
                                worklist.push(r);
                            }
                        }
                    }
                }
            }
        }

        // Generate module unique hash
        let mut hasher = DefaultHasher::new();
        canonical_path.hash(&mut hasher);
        let path_hash = hasher.finish();

        // Mangle unexported internal helper components and track alias renames
        let mut renames: HashMap<String, String> = HashMap::new();
        for comp_name in &reachable {
            let is_exported = u.items.iter().any(|it| it.name.as_str() == comp_name);
            if !is_exported {
                let mangled = format!("__dt_{:x}_{}", path_hash, comp_name);
                renames.insert(comp_name.clone(), mangled);
            }
        }

        // Register unexported helper components under mangled names
        for (original_name, mangled_name) in &renames {
            if let Some(overloads) = imported_components.get(original_name) {
                for o in overloads {
                    let mut comp = o.clone();
                    rewrite_component_node_names(&mut comp, &renames);
                    comp.name = Ident::new(mangled_name.clone(), comp.name.span);
                    register_component_overload(registry, comp)?;
                }
            }
        }

        // Register exported component symbols under their imported/aliased names
        for use_item in &u.items {
            let symbol_name = use_item.name.as_str();
            let target_name = use_item.bound_name();
            if let Some(overloads) = imported_components.get(symbol_name) {
                for o in overloads {
                    let mut comp = o.clone();
                    let mut local_renames = renames.clone();
                    if symbol_name != target_name {
                        local_renames.insert(symbol_name.to_string(), target_name.to_string());
                    }
                    rewrite_component_node_names(&mut comp, &local_renames);
                    comp.name = Ident::new(
                        target_name,
                        use_item.alias.as_ref().map(|a| a.span).unwrap_or(use_item.name.span),
                    );
                    register_component_overload(registry, comp)?;
                }
            }
        }
    } else if let Some(alias) = &u.alias {
        for item in &imported_doc.items {
            if let Item::Enum(e) = item {
                let mut aliased_enum = e.clone();
                aliased_enum.name = alias.clone();
                imported_items.push(Item::Enum(aliased_enum));
            }
        }

        let alias_name = alias.as_str();
        let file_stem = raw_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        if !imported_components.is_empty() {
            let (original_name, target_comps) = if let Some(comps) = imported_components.get(file_stem) {
                (file_stem.to_string(), comps.clone())
            } else if imported_components.len() == 1 {
                let (name, comps) = imported_components.iter().next().unwrap();
                (name.clone(), comps.clone())
            } else if let Some(comps) = imported_components.get(alias_name) {
                (alias_name.to_string(), comps.clone())
            } else {
                return Err(CompileError::ImportError {
                    path: u.path.clone(),
                    message: format!(
                        "Cannot alias import '{}' as '{}': file defines {} components ({:?}) and none matches file stem '{}'",
                        u.path,
                        alias_name,
                        imported_components.len(),
                        imported_components.keys().collect::<Vec<_>>(),
                        file_stem
                    ),
                    span: alias.span,
                });
            };

            for target_comp in target_comps {
                let mut aliased = target_comp.clone();
                aliased.name = Ident::new(alias_name, alias.span);
                register_component_overload(registry, aliased)?;
            }

            // Also bring in transitive dependencies needed by target_comp
            for (name, comps) in &imported_components {
                if name != &original_name {
                    for comp in comps {
                        register_component_overload(registry, comp.clone())?;
                    }
                }
            }
        }
    } else {
        if is_first_visit {
            for item in &imported_doc.items {
                match item {
                    Item::Let(l) => {
                        imported_items.push(Item::Let(l.clone()));
                    }
                    Item::Env(e) => {
                        imported_items.push(Item::Env(e.clone()));
                    }
                    Item::Enum(e) => {
                        imported_items.push(Item::Enum(e.clone()));
                    }
                    _ => {}
                }
            }
        }

        // No alias: import all components defined in the file
        for (_name, comps) in imported_components {
            for comp in comps {
                register_component_overload(registry, comp)?;
            }
        }
    }

    active_stack.remove(&canonical_path);
    Ok(())
}

fn collect_referenced_component_names(comp: &ComponentDef, out: &mut HashSet<String>) {
    for param in &comp.params {
        if let Some(ty) = &param.type_annotation {
            out.insert(ty.name.as_str().to_string());
        }
        if let Some(default_edge) = &param.default_edge {
            collect_from_expr(default_edge, out);
        }
    }
    for item in &comp.body {
        collect_from_content_item(item, out);
    }
}

fn collect_from_content_item(item: &ContentItem, out: &mut HashSet<String>) {
    match item {
        ContentItem::Node(node) => collect_from_element_node(node, out),
        ContentItem::Let(l) => {
            if let Some(ty) = &l.type_annotation {
                out.insert(ty.name.as_str().to_string());
            }
            if let Some(val) = &l.value {
                match val {
                    LetValue::Node(n) => collect_from_element_node(n, out),
                    LetValue::Expr(e) => collect_from_expr(e, out),
                }
            }
        }
        ContentItem::Children(c) => {
            for port in &c.ports {
                collect_from_expr(&port.expr, out);
            }
        }
        ContentItem::Alias(a) => {
            if let Some(ty) = &a.type_annotation {
                out.insert(ty.name.as_str().to_string());
            }
            collect_from_expr(&a.value, out);
        }
        ContentItem::Assert(a) => collect_from_expr(&a.condition, out),
        ContentItem::State(s) => {
            if let Some(ty) = &s.type_annotation {
                out.insert(ty.name.as_str().to_string());
            }
            if let Some(e) = &s.default {
                collect_from_expr(e, out);
            }
        }
        ContentItem::Env(env) => {
            if let Some(ty) = &env.type_annotation {
                out.insert(ty.name.as_str().to_string());
            }
            if let Some(e) = &env.value {
                collect_from_expr(e, out);
            }
        }
        ContentItem::Text(_) => {}
    }
}

fn collect_from_element_node(node: &ElementNode, out: &mut HashSet<String>) {
    out.insert(node.name.as_str().to_string());
    if let Some(key) = &node.key {
        for part in &key.parts {
            collect_from_expr(part, out);
        }
    }
    for port in &node.ports {
        collect_from_expr(&port.expr, out);
    }
    if let Some(slot) = &node.content {
        for item in &slot.items {
            collect_from_content_item(item, out);
        }
    }
}

fn collect_from_expr(expr: &Expr, out: &mut HashSet<String>) {
    match expr {
        Expr::Node(node) => collect_from_element_node(node, out),
        Expr::MemberAccess(m) => collect_from_expr(&m.target, out),
        Expr::Ternary(t) => {
            collect_from_expr(&t.condition, out);
            collect_from_expr(&t.then_expr, out);
            collect_from_expr(&t.else_expr, out);
        }
        Expr::Call(c) => {
            for arg in &c.args {
                collect_from_expr(arg, out);
            }
        }
        Expr::MethodCall(m) => {
            collect_from_expr(&m.target, out);
            for arg in &m.args {
                collect_from_expr(arg, out);
            }
        }
        Expr::Binary(b) => {
            collect_from_expr(&b.left, out);
            collect_from_expr(&b.right, out);
        }
        Expr::Unary(u) => collect_from_expr(&u.operand, out),
        Expr::Paren(p, _) => collect_from_expr(p, out),
        Expr::Literal(_) | Expr::Ident(_) => {}
    }
}

fn rewrite_component_node_names(comp: &mut ComponentDef, renames: &HashMap<String, String>) {
    for param in &mut comp.params {
        if let Some(ty) = &mut param.type_annotation {
            if let Some(new_name) = renames.get(ty.name.as_str()) {
                ty.name = Ident::new(new_name, ty.name.span);
            }
        }
        if let Some(default_edge) = &mut param.default_edge {
            rewrite_expr(default_edge, renames);
        }
    }
    for item in &mut comp.body {
        rewrite_content_item(item, renames);
    }
}

fn rewrite_content_item(item: &mut ContentItem, renames: &HashMap<String, String>) {
    match item {
        ContentItem::Node(node) => rewrite_element_node(node, renames),
        ContentItem::Let(l) => {
            if let Some(ty) = &mut l.type_annotation {
                if let Some(new_name) = renames.get(ty.name.as_str()) {
                    ty.name = Ident::new(new_name, ty.name.span);
                }
            }
            if let Some(val) = &mut l.value {
                match val {
                    LetValue::Node(n) => rewrite_element_node(n, renames),
                    LetValue::Expr(e) => rewrite_expr(e, renames),
                }
            }
        }
        ContentItem::Children(c) => {
            for port in &mut c.ports {
                rewrite_expr(&mut port.expr, renames);
            }
        }
        ContentItem::Alias(a) => {
            if let Some(ty) = &mut a.type_annotation {
                if let Some(new_name) = renames.get(ty.name.as_str()) {
                    ty.name = Ident::new(new_name, ty.name.span);
                }
            }
            rewrite_expr(&mut a.value, renames);
        }
        ContentItem::Assert(a) => rewrite_expr(&mut a.condition, renames),
        ContentItem::State(s) => {
            if let Some(ty) = &mut s.type_annotation {
                if let Some(new_name) = renames.get(ty.name.as_str()) {
                    ty.name = Ident::new(new_name, ty.name.span);
                }
            }
            if let Some(e) = &mut s.default {
                rewrite_expr(e, renames);
            }
        }
        ContentItem::Env(env) => {
            if let Some(ty) = &mut env.type_annotation {
                if let Some(new_name) = renames.get(ty.name.as_str()) {
                    ty.name = Ident::new(new_name, ty.name.span);
                }
            }
            if let Some(e) = &mut env.value {
                rewrite_expr(e, renames);
            }
        }
        ContentItem::Text(_) => {}
    }
}

fn rewrite_element_node(node: &mut ElementNode, renames: &HashMap<String, String>) {
    if let Some(new_name) = renames.get(node.name.as_str()) {
        node.name = Ident::new(new_name, node.name.span);
    }
    if let Some(key) = &mut node.key {
        for part in &mut key.parts {
            rewrite_expr(part, renames);
        }
    }
    for port in &mut node.ports {
        rewrite_expr(&mut port.expr, renames);
    }
    if let Some(slot) = &mut node.content {
        for item in &mut slot.items {
            rewrite_content_item(item, renames);
        }
    }
}

fn rewrite_expr(expr: &mut Expr, renames: &HashMap<String, String>) {
    match expr {
        Expr::Node(node) => rewrite_element_node(node, renames),
        Expr::MemberAccess(m) => rewrite_expr(&mut m.target, renames),
        Expr::Ternary(t) => {
            rewrite_expr(&mut t.condition, renames);
            rewrite_expr(&mut t.then_expr, renames);
            rewrite_expr(&mut t.else_expr, renames);
        }
        Expr::Call(c) => {
            for arg in &mut c.args {
                rewrite_expr(arg, renames);
            }
        }
        Expr::MethodCall(m) => {
            rewrite_expr(&mut m.target, renames);
            for arg in &mut m.args {
                rewrite_expr(arg, renames);
            }
        }
        Expr::Binary(b) => {
            rewrite_expr(&mut b.left, renames);
            rewrite_expr(&mut b.right, renames);
        }
        Expr::Unary(u) => rewrite_expr(&mut u.operand, renames),
        Expr::Paren(p, _) => rewrite_expr(p, renames),
        Expr::Literal(_) | Expr::Ident(_) => {}
    }
}

fn resolve_component_enum_literals(
    comp: &mut ComponentDef,
    enums: &HashMap<String, crate::ast::EnumDef>,
) {
    for param in &mut comp.params {
        if let Some(default_edge) = &mut param.default_edge {
            resolve_expr_enum_literals(default_edge, enums);
        }
    }
    for item in &mut comp.body {
        resolve_content_item_enum_literals(item, enums);
    }
}

fn resolve_content_item_enum_literals(
    item: &mut ContentItem,
    enums: &HashMap<String, crate::ast::EnumDef>,
) {
    match item {
        ContentItem::Node(node) => resolve_element_node_enum_literals(node, enums),
        ContentItem::Let(l) => {
            if let Some(val) = &mut l.value {
                match val {
                    LetValue::Node(n) => resolve_element_node_enum_literals(n, enums),
                    LetValue::Expr(e) => resolve_expr_enum_literals(e, enums),
                }
            }
        }
        ContentItem::Children(c) => {
            for port in &mut c.ports {
                resolve_expr_enum_literals(&mut port.expr, enums);
            }
        }
        ContentItem::Alias(a) => resolve_expr_enum_literals(&mut a.value, enums),
        ContentItem::Assert(a) => resolve_expr_enum_literals(&mut a.condition, enums),
        ContentItem::State(s) => {
            if let Some(e) = &mut s.default {
                resolve_expr_enum_literals(e, enums);
            }
        }
        ContentItem::Env(env) => {
            if let Some(e) = &mut env.value {
                resolve_expr_enum_literals(e, enums);
            }
        }
        ContentItem::Text(_) => {}
    }
}

fn resolve_element_node_enum_literals(
    node: &mut ElementNode,
    enums: &HashMap<String, crate::ast::EnumDef>,
) {
    if let Some(key) = &mut node.key {
        for part in &mut key.parts {
            resolve_expr_enum_literals(part, enums);
        }
    }
    for port in &mut node.ports {
        resolve_expr_enum_literals(&mut port.expr, enums);
    }
    if let Some(slot) = &mut node.content {
        for item in &mut slot.items {
            resolve_content_item_enum_literals(item, enums);
        }
    }
}

fn resolve_expr_enum_literals(
    expr: &mut Expr,
    enums: &HashMap<String, crate::ast::EnumDef>,
) {
    match expr {
        Expr::MemberAccess(m) => {
            if let Expr::Ident(target_id) = m.target.as_ref() {
                if let Some(enum_def) = enums.get(target_id.as_str()) {
                    if enum_def.has_variant(m.member.as_str()) {
                        *expr = Expr::Literal(crate::ast::Literal::Enum(
                            target_id.as_str().to_string(),
                            m.member.as_str().to_string(),
                            m.span,
                        ));
                        return;
                    }
                }
            }
            resolve_expr_enum_literals(&mut m.target, enums);
        }
        Expr::Node(node) => resolve_element_node_enum_literals(node, enums),
        Expr::Ternary(t) => {
            resolve_expr_enum_literals(&mut t.condition, enums);
            resolve_expr_enum_literals(&mut t.then_expr, enums);
            resolve_expr_enum_literals(&mut t.else_expr, enums);
        }
        Expr::Call(c) => {
            for arg in &mut c.args {
                resolve_expr_enum_literals(arg, enums);
            }
        }
        Expr::MethodCall(m) => {
            resolve_expr_enum_literals(&mut m.target, enums);
            for arg in &mut m.args {
                resolve_expr_enum_literals(arg, enums);
            }
        }
        Expr::Binary(b) => {
            resolve_expr_enum_literals(&mut b.left, enums);
            resolve_expr_enum_literals(&mut b.right, enums);
        }
        Expr::Unary(u) => resolve_expr_enum_literals(&mut u.operand, enums),
        Expr::Paren(p, _) => resolve_expr_enum_literals(p, enums),
        Expr::Literal(_) | Expr::Ident(_) => {}
    }
}

