use crate::compiler::error::CompileError;
use crate::compiler::graph::{VarId, VarIndex, VariableGraph};
use crate::span::Span;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// A deterministic, topologically sorted execution schedule for all variables in the graph.
#[derive(Debug, Clone, PartialEq)]
pub struct TopologicalSchedule {
    pub order: Vec<VarId>,
    pub order_indices: Vec<VarIndex>,
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
    let num_vars = graph.var_nodes.len();
    if num_vars == graph.variables.len() && graph.upstream_idx.len() == num_vars {
        let mut in_degrees: Vec<u32> = graph
            .upstream_idx
            .iter()
            .map(|deps| deps.len() as u32)
            .collect();

        let mut ready: BinaryHeap<Reverse<VarIndex>> = in_degrees
            .iter()
            .enumerate()
            .filter(|(_, &deg)| deg == 0)
            .map(|(idx, _)| Reverse(VarIndex(idx as u32)))
            .collect();

        let mut order_indices = Vec::with_capacity(num_vars);

        while let Some(Reverse(curr_idx)) = ready.pop() {
            for &down_idx in &graph.downstream_idx[curr_idx.0 as usize] {
                let deg = &mut in_degrees[down_idx.0 as usize];
                *deg = deg.saturating_sub(1);
                if *deg == 0 {
                    ready.push(Reverse(down_idx));
                }
            }
            order_indices.push(curr_idx);
        }

        if order_indices.len() == num_vars {
            let order = order_indices
                .iter()
                .map(|&idx| graph.var_nodes[idx.0 as usize].id.clone())
                .collect();
            return Ok(TopologicalSchedule {
                order,
                order_indices,
            });
        }
    }

    let mut in_degrees: HashMap<VarId, usize> = HashMap::new();

    // 1. Calculate in-degrees
    for var_id in graph.variables.keys() {
        in_degrees.insert(var_id.clone(), graph.in_degree(var_id));
    }

    // 2. Queue all variables with in-degree 0 in a min-heap ordered by canonical string
    let mut ready: BinaryHeap<Reverse<(String, VarId)>> = in_degrees
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(v, _)| Reverse((v.to_string(), v.clone())))
        .collect();

    let mut order = Vec::with_capacity(graph.variables.len());

    // 3. Process ready queue
    while let Some(Reverse((_, current))) = ready.pop() {
        if let Some(downstream_list) = graph.downstream.get(&current) {
            for downstream in downstream_list {
                if let Some(deg) = in_degrees.get_mut(downstream) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        ready.push(Reverse((downstream.to_string(), downstream.clone())));
                    }
                }
            }
        }
        order.push(current);
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

        let mut node_info = graph.node_info.clone();
        for var_id in &cycle {
            if !node_info.contains_key(&var_id.node) {
                if let Some(var_node) = graph.get_variable(var_id) {
                    if var_node.span.line > 0 {
                        node_info.insert(
                            var_id.node,
                            crate::compiler::graph::GraphNodeInfo {
                                name: String::new(),
                                span: var_node.span,
                            },
                        );
                    }
                }
            }
        }

        return Err(CompileError::CyclicDependency {
            cycle,
            node_info,
            span,
        });
    }

    let order_indices = order
        .iter()
        .filter_map(|v| graph.lookup_var_index(v.node, &v.port))
        .collect();

    Ok(TopologicalSchedule {
        order,
        order_indices,
    })
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
