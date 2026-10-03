use crate::ast::{ComponentDef, Document, Ident, Item, UseDeclaration};
use crate::compiler::error::CompileError;
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

/// Resolves all `\use` declarations in a document and returns the merged component registry.
pub fn resolve_imports<R: FileResolver>(
    doc: &Document,
    base_dir: &Path,
    resolver: &R,
) -> Result<HashMap<String, ComponentDef>, CompileError> {
    let mut registry = HashMap::new();
    let mut active_stack = HashSet::new();
    let mut cache = HashMap::new();
    let mut module_exports = HashMap::new();

    // 1. Index locally declared components in the root document
    for item in &doc.items {
        if let Item::Component(comp) = item {
            registry.insert(comp.name.as_str().to_string(), comp.clone());
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
                &mut active_stack,
                &mut cache,
                &mut module_exports,
            )?;
        }
    }

    Ok(registry)
}

fn resolve_use_decl<R: FileResolver>(
    u: &UseDeclaration,
    base_dir: &Path,
    resolver: &R,
    registry: &mut HashMap<String, ComponentDef>,
    active_stack: &mut HashSet<PathBuf>,
    cache: &mut HashMap<PathBuf, Document>,
    module_exports: &mut HashMap<PathBuf, HashMap<String, ComponentDef>>,
) -> Result<(), CompileError> {
    let raw_path = Path::new(&u.path);
    let resolved_path = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        base_dir.join(raw_path)
    };

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

    let imported_components = if let Some(exported) = module_exports.get(&canonical_path) {
        exported.clone()
    } else {
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

        // Collect components from the imported file and transitively resolve its \use declarations
        let mut components = HashMap::new();
        for item in &imported_doc.items {
            if let Item::Component(comp) = item {
                components.insert(comp.name.as_str().to_string(), comp.clone());
            }
        }
        for item in &imported_doc.items {
            if let Item::Use(nested_u) = item {
                resolve_use_decl(
                    nested_u,
                    imported_base_dir,
                    resolver,
                    &mut components,
                    active_stack,
                    cache,
                    module_exports,
                )?;
            }
        }

        module_exports.insert(canonical_path.clone(), components.clone());
        components
    };

    // Register into caller's registry
    if let Some(alias) = &u.alias {
        let alias_name = alias.as_str();
        let file_stem = raw_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        let target_comp = if let Some(comp) = imported_components.get(file_stem) {
            comp.clone()
        } else if imported_components.len() == 1 {
            imported_components.values().next().unwrap().clone()
        } else if let Some(comp) = imported_components.get(alias_name) {
            comp.clone()
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

        let mut aliased = target_comp.clone();
        aliased.name = Ident::new(alias_name, alias.span);
        registry.insert(alias_name.to_string(), aliased);

        // Also bring in transitive dependencies needed by target_comp
        for (name, comp) in &imported_components {
            if name != target_comp.name.as_str() {
                registry.entry(name.clone()).or_insert_with(|| comp.clone());
            }
        }
    } else {
        // No alias: import all components defined in the file
        for (name, comp) in imported_components {
            registry.insert(name, comp);
        }
    }

    active_stack.remove(&canonical_path);
    Ok(())
}
