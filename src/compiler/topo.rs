use crate::compiler::error::CompileError;
use crate::compiler::graph::{VarId, VariableGraph};
use crate::span::Span;
use std::collections::{HashMap, HashSet};

/// A deterministic, topologically sorted execution schedule for all variables in the graph.
#[derive(Debug, Clone, PartialEq)]
pub struct TopologicalSchedule {
    pub order: Vec<VarId>,
}

impl TopologicalSchedule {
    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, VarId> {
        self.order.iter()
    }
}

/// Performs Kahn's algorithm on the variable graph to produce a topological evaluation schedule
/// or return a detailed cycle diagnostic error.
pub fn sort_graph(graph: &VariableGraph) -> Result<TopologicalSchedule, CompileError> {
    let mut in_degrees: HashMap<VarId, usize> = HashMap::new();

    // 1. Calculate in-degrees
    for var_id in graph.variables.keys() {
        in_degrees.insert(var_id.clone(), graph.in_degree(var_id));
    }

    // 2. Queue all variables with in-degree 0
    let mut ready: Vec<VarId> = in_degrees
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(v, _)| v.clone())
        .collect();

    // Sort ready queue in reverse canonical order so `pop()` yields ascending canonical order
    ready.sort_by_key(|v| v.to_string());
    ready.reverse();

    let mut order = Vec::with_capacity(graph.variables.len());

    // 3. Process ready queue
    while let Some(current) = ready.pop() {
        order.push(current.clone());

        if let Some(downstream_list) = graph.downstream.get(&current) {
            let mut newly_ready = Vec::new();

            for downstream in downstream_list {
                if let Some(deg) = in_degrees.get_mut(downstream) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        newly_ready.push(downstream.clone());
                    }
                }
            }

            // Insert newly ready variables maintaining deterministic order
            if !newly_ready.is_empty() {
                ready.extend(newly_ready);
                ready.sort_by_key(|v| v.to_string());
                ready.reverse();
            }
        }
    }

    // 4. Check for cycles
    if order.len() < graph.variables.len() {
        // Collect all nodes that were involved in cycles
        let remaining_nodes: HashSet<VarId> = in_degrees
            .iter()
            .filter(|(_, &deg)| deg > 0)
            .map(|(v, _)| v.clone())
            .collect();

        let cycle = find_cycle(graph, &remaining_nodes);
        let span = cycle
            .first()
            .and_then(|v| graph.get_variable(v))
            .map_or(Span::default(), |node| node.span);

        return Err(CompileError::CyclicDependency { cycle, span });
    }

    Ok(TopologicalSchedule { order })
}

/// Finds a directed cycle among the set of remaining nodes using DFS.
fn find_cycle(graph: &VariableGraph, remaining_nodes: &HashSet<VarId>) -> Vec<VarId> {
    let mut sorted_candidates: Vec<VarId> = remaining_nodes.iter().cloned().collect();
    sorted_candidates.sort_by_key(|v| v.to_string());

    let mut visited: HashSet<VarId> = HashSet::new();
    let mut on_path: HashSet<VarId> = HashSet::new();
    let mut path: Vec<VarId> = Vec::new();

    for start in sorted_candidates {
        if !visited.contains(&start) {
            if let Some(cycle) = dfs_cycle(
                &start,
                graph,
                remaining_nodes,
                &mut visited,
                &mut on_path,
                &mut path,
            ) {
                return cycle;
            }
        }
    }

    // Fallback if no specific cycle could be isolated
    remaining_nodes.iter().cloned().collect()
}

fn dfs_cycle(
    current: &VarId,
    graph: &VariableGraph,
    remaining_nodes: &HashSet<VarId>,
    visited: &mut HashSet<VarId>,
    on_path: &mut HashSet<VarId>,
    path: &mut Vec<VarId>,
) -> Option<Vec<VarId>> {
    visited.insert(current.clone());
    on_path.insert(current.clone());
    path.push(current.clone());

    if let Some(downstream_list) = graph.downstream.get(current) {
        let mut sorted_downstream: Vec<VarId> = downstream_list
            .iter()
            .filter(|v| remaining_nodes.contains(v))
            .cloned()
            .collect();
        sorted_downstream.sort_by_key(|v| v.to_string());

        for neighbor in sorted_downstream {
            if on_path.contains(&neighbor) {
                // Found back edge, extract cycle
                let cycle_start = path.iter().position(|x| x == &neighbor).unwrap_or(0);
                let mut cycle = path[cycle_start..].to_vec();
                cycle.push(neighbor);
                return Some(cycle);
            }

            if !visited.contains(&neighbor) {
                if let Some(cycle) =
                    dfs_cycle(&neighbor, graph, remaining_nodes, visited, on_path, path)
                {
                    return Some(cycle);
                }
            }
        }
    }

    path.pop();
    on_path.remove(current);
    None
}
