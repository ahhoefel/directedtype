use crate::ast::{ComponentDef, Document, Ident, Item, UseDeclaration};
use crate::compiler::error::{AmbiguousOverloadDetails, CompileError};
use crate::parser::parse_document;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Trait for resolving and loading source files (supports disk and in-memory virtual filesystems).
pub trait FileResolver {
    fn read(&self, path: &Path) -> Result<String, String>;
    fn canonicalize(&self, path: &Path) -> Result<PathBuf, String> {
        Ok(normalize_path(path))
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
}

impl VirtualResolver {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
        }
    }

    pub fn insert(&mut self, path: impl Into<PathBuf>, content: impl Into<String>) {
        let p = path.into();
        let norm = normalize_path(&p);
        let s = content.into();
        self.files.insert(norm, s.clone());
        self.files.insert(p, s);
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
    let mut registry: HashMap<String, Vec<ComponentDef>> = HashMap::new();
    let mut imported_items = Vec::new();
    let mut active_stack = HashSet::new();
    let mut cache = HashMap::new();
    let mut module_exports = HashMap::new();
    let mut visited_files = HashSet::new();

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
            )?;
        }
    }

    Ok((registry, imported_items))
}

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

        let parsed = parse_document(&source).map_err(|e| CompileError::ImportError {
            path: u.path.clone(),
            message: format!("Parse error in '{}': {}", u.path, e),
            span: u.span,
        })?;

        cache.insert(canonical_path.clone(), parsed.clone());
        parsed
    };

    let imported_base_dir = resolved_path.parent().unwrap_or(Path::new("."));

    // Transitive imports first
    for item in &imported_doc.items {
        if let Item::Use(nested_u) = item {
            resolve_use_decl(
                nested_u,
                imported_base_dir,
                resolver,
                registry,
                imported_items,
                active_stack,
                cache,
                module_exports,
                visited_files,
            )?;
        }
    }

    // Collect components from the imported file
    let imported_components = if let Some(exported) = module_exports.get(&canonical_path) {
        exported.clone()
    } else {
        let mut components: HashMap<String, Vec<ComponentDef>> = HashMap::new();
        for item in &imported_doc.items {
            if let Item::Component(comp) = item {
                register_component_overload(&mut components, comp.clone())?;
            }
        }
        module_exports.insert(canonical_path.clone(), components.clone());
        components
    };

    // Collect top-level let, env, and enum bindings
    if let Some(alias) = &u.alias {
        for item in &imported_doc.items {
            if let Item::Enum(e) = item {
                let mut aliased_enum = e.clone();
                aliased_enum.name = alias.clone();
                imported_items.push(Item::Enum(aliased_enum));
            }
        }
    } else if is_first_visit {
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

    // Register into caller's registry
    if let Some(alias) = &u.alias {
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
