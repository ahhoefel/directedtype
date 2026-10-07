use std::collections::HashMap;
use crate::compiler::error::CompileError;
use crate::compiler::expanded::NodeId;
use crate::span::Span;

/// A strongly typed identifier for a navigation scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ScopeId(pub usize);

impl ScopeId {
    /// The root document navigation scope.
    pub const ROOT: ScopeId = ScopeId(0);
}

/// An isolated navigation scope representing a document, scroll pane, or section.
#[derive(Debug, Clone, PartialEq)]
pub struct NavigationScope {
    pub id: ScopeId,
    pub name: String,
    pub parent: Option<ScopeId>,
    pub children: HashMap<String, ScopeId>,
    pub anchors: HashMap<String, NodeId>,
    pub canonical_path: String,
    pub node_id: Option<NodeId>,
}

/// The hierarchical tree of all navigation scopes across the document.
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeTree {
    pub scopes: Vec<NavigationScope>,
    pub node_to_scope: HashMap<NodeId, ScopeId>,
}

impl Default for ScopeTree {
    fn default() -> Self {
        Self::new()
    }
}

impl ScopeTree {
    /// Creates a new `ScopeTree` initialized with a root scope (`/`).
    pub fn new() -> Self {
        let root = NavigationScope {
            id: ScopeId::ROOT,
            name: String::new(),
            parent: None,
            children: HashMap::new(),
            anchors: HashMap::new(),
            canonical_path: "/".to_string(),
            node_id: None,
        };
        Self {
            scopes: vec![root],
            node_to_scope: HashMap::new(),
        }
    }

    /// Adds a child scope under `parent_id`.
    ///
    /// Validates that no sibling scope with the same name already exists under `parent_id`.
    pub fn add_scope(
        &mut self,
        name: &str,
        parent_id: ScopeId,
        node_id: Option<NodeId>,
        span: Span,
    ) -> Result<ScopeId, CompileError> {
        let parent = &self.scopes[parent_id.0];
        if parent.children.contains_key(name) {
            return Err(CompileError::Custom {
                message: format!(
                    "Duplicate scope name '{}' under parent scope '{}'",
                    name, parent.canonical_path
                ),
                span,
            });
        }

        let canonical_path = if parent_id == ScopeId::ROOT {
            format!("/{}", name)
        } else {
            format!("{}/{}", parent.canonical_path, name)
        };

        let new_id = ScopeId(self.scopes.len());
        let scope = NavigationScope {
            id: new_id,
            name: name.to_string(),
            parent: Some(parent_id),
            children: HashMap::new(),
            anchors: HashMap::new(),
            canonical_path,
            node_id,
        };

        self.scopes.push(scope);
        self.scopes[parent_id.0].children.insert(name.to_string(), new_id);

        if let Some(nid) = node_id {
            self.node_to_scope.insert(nid, new_id);
        }

        Ok(new_id)
    }

    /// Registers an anchor target in `scope_id`.
    ///
    /// Validates that no duplicate anchor with the same name exists in this scope.
    pub fn register_anchor(
        &mut self,
        scope_id: ScopeId,
        anchor_name: &str,
        node_id: NodeId,
        span: Span,
    ) -> Result<(), CompileError> {
        let scope = &mut self.scopes[scope_id.0];
        if scope.anchors.contains_key(anchor_name) {
            return Err(CompileError::Custom {
                message: format!(
                    "Duplicate anchor '#{}' in scope '{}'",
                    anchor_name,
                    if scope.canonical_path == "/" {
                        "root"
                    } else {
                        &scope.canonical_path
                    }
                ),
                span,
            });
        }

        scope.anchors.insert(anchor_name.to_string(), node_id);
        self.node_to_scope.insert(node_id, scope_id);
        Ok(())
    }

    /// Resolves an anchor path from a given context scope.
    ///
    /// - If `target_path` starts with `'/'` (e.g. `"#/pane-left/intro"` or `"/pane-left/intro"`),
    ///   it is resolved as an **absolute path** starting from `ScopeId::ROOT`.
    /// - If `target_path` does not start with `'/'` (e.g. `"#intro"` or `"intro"`),
    ///   it is resolved as a **relative path** starting from `from_scope`, bubbling up to ancestor scopes.
    pub fn resolve_anchor(
        &self,
        from_scope: ScopeId,
        target_path: &str,
    ) -> Option<(NodeId, ScopeId)> {
        let path = target_path.strip_prefix('#').unwrap_or(target_path);
        if path.is_empty() {
            return None;
        }

        if path.starts_with('/') {
            let trimmed = path.trim_start_matches('/');
            let segments: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
            if segments.is_empty() {
                return None;
            }
            self.resolve_absolute_segments(&segments)
        } else {
            let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
            if segments.is_empty() {
                return None;
            }
            self.resolve_relative_segments(from_scope, &segments)
        }
    }

    fn resolve_absolute_segments(&self, segments: &[&str]) -> Option<(NodeId, ScopeId)> {
        let mut curr_id = ScopeId::ROOT;
        for &seg in &segments[..segments.len() - 1] {
            let curr = &self.scopes[curr_id.0];
            curr_id = *curr.children.get(seg)?;
        }

        let last_seg = segments[segments.len() - 1];
        let curr = &self.scopes[curr_id.0];
        let target_node = *curr.anchors.get(last_seg)?;
        Some((target_node, curr_id))
    }

    fn resolve_relative_segments(
        &self,
        mut curr_scope: ScopeId,
        segments: &[&str],
    ) -> Option<(NodeId, ScopeId)> {
        if segments.len() > 1 {
            // Multi-segment relative path, e.g. "sub/step1"
            loop {
                if let Some(target) = self.resolve_relative_from_scope(curr_scope, segments) {
                    return Some(target);
                }
                match self.scopes[curr_scope.0].parent {
                    Some(parent) => curr_scope = parent,
                    None => break,
                }
            }
            None
        } else {
            // Single segment, e.g. "intro"
            let anchor_name = segments[0];
            loop {
                let scope = &self.scopes[curr_scope.0];
                if let Some(&node_id) = scope.anchors.get(anchor_name) {
                    return Some((node_id, curr_scope));
                }
                match scope.parent {
                    Some(parent) => curr_scope = parent,
                    None => break,
                }
            }
            None
        }
    }

    fn resolve_relative_from_scope(
        &self,
        mut curr_scope: ScopeId,
        segments: &[&str],
    ) -> Option<(NodeId, ScopeId)> {
        for &seg in &segments[..segments.len() - 1] {
            let curr = &self.scopes[curr_scope.0];
            curr_scope = *curr.children.get(seg)?;
        }

        let last_seg = segments[segments.len() - 1];
        let curr = &self.scopes[curr_scope.0];
        let target_node = *curr.anchors.get(last_seg)?;
        Some((target_node, curr_scope))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scope_tree_absolute_and_relative_resolution() {
        let mut tree = ScopeTree::new();

        // Root anchors
        tree.register_anchor(ScopeId::ROOT, "top", NodeId(1), Span::default())
            .expect("register root ok");

        // Two independent panes (identical anchor names "intro")
        let left_id = tree
            .add_scope("pane-left", ScopeId::ROOT, Some(NodeId(10)), Span::default())
            .expect("add left ok");
        let right_id = tree
            .add_scope("pane-right", ScopeId::ROOT, Some(NodeId(20)), Span::default())
            .expect("add right ok");

        tree.register_anchor(left_id, "intro", NodeId(11), Span::default())
            .expect("register left intro ok");
        tree.register_anchor(right_id, "intro", NodeId(21), Span::default())
            .expect("register right intro ok");

        // Nested sub-scope in left pane
        let sub_id = tree
            .add_scope("section-1", left_id, Some(NodeId(12)), Span::default())
            .expect("add sub ok");
        tree.register_anchor(sub_id, "step-1", NodeId(13), Span::default())
            .expect("register step-1 ok");

        // 1. Relative resolution inside left pane
        let res_left = tree.resolve_anchor(left_id, "#intro");
        assert_eq!(res_left, Some((NodeId(11), left_id)));

        // 2. Relative resolution inside right pane
        let res_right = tree.resolve_anchor(right_id, "#intro");
        assert_eq!(res_right, Some((NodeId(21), right_id)));

        // 3. Absolute resolution from anywhere
        let abs_left = tree.resolve_anchor(right_id, "#/pane-left/intro");
        assert_eq!(abs_left, Some((NodeId(11), left_id)));

        let abs_right = tree.resolve_anchor(left_id, "#/pane-right/intro");
        assert_eq!(abs_right, Some((NodeId(21), right_id)));

        // 4. Bubbling upward: sub-scope searches for "#intro" which is in parent left_id
        let bubble_res = tree.resolve_anchor(sub_id, "#intro");
        assert_eq!(bubble_res, Some((NodeId(11), left_id)));

        // 5. Bubbling to root: sub-scope searches for "#top"
        let root_res = tree.resolve_anchor(sub_id, "#top");
        assert_eq!(root_res, Some((NodeId(1), ScopeId::ROOT)));

        // 6. Relative multi-segment from left pane: "#section-1/step-1"
        let rel_multi = tree.resolve_anchor(left_id, "#section-1/step-1");
        assert_eq!(rel_multi, Some((NodeId(13), sub_id)));
    }

    #[test]
    fn test_duplicate_anchor_in_same_scope_fails() {
        let mut tree = ScopeTree::new();
        tree.register_anchor(ScopeId::ROOT, "test", NodeId(1), Span::default())
            .expect("first ok");
        let err = tree
            .register_anchor(ScopeId::ROOT, "test", NodeId(2), Span::default())
            .unwrap_err();
        assert!(err.to_string().contains("Duplicate anchor '#test'"));
    }
}
