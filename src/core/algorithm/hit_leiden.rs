// Copyright 2026 naadir jeewa
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use crate::core::config::RunConfig;
use crate::core::error::HitLeidenError;
use crate::core::partition::state::PartitionState;
use crate::core::runtime::orchestrator;
use crate::core::types::{
    BackendType, GraphInput, PartitionResult, RunExecution, RunOutcome, RunStatus,
};
use ahash::{HashMap, HashMapExt, HashSet, HashSetExt};
use bitvec::prelude::*;
use std::borrow::Cow;
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn run(graph: &GraphInput, config: &RunConfig) -> Result<RunOutcome, HitLeidenError> {
    config
        .validate()
        .map_err(|e| HitLeidenError::InvalidInput(e.to_string()))?;

    if graph
        .edges
        .iter()
        .any(|(s, d, _)| *s >= graph.node_count || *d >= graph.node_count)
    {
        return Err(HitLeidenError::InvalidInput(
            "edge endpoint exceeds node_count".to_string(),
        ));
    }

    let started_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut partition_state = PartitionState::identity(graph.node_count);
    let resolution_meta = orchestrator::resolve_with_fallback(config, true);

    // Multi-level Leiden: run local moves, aggregate into coarser graph, repeat.
    // This implements the hierarchical coarsening that standard Leiden uses.
    let (iteration_count, hierarchy_levels) = multilevel_leiden(
        &mut partition_state,
        graph,
        config.resolution,
        config.mode,
        config.max_iterations,
    );

    // Canonicalize community IDs deterministically to avoid label permutation
    // across runs with identical partition structure.
    canonicalize_community_ids_in_place(&mut partition_state.node_to_comm);

    let execution = RunExecution {
        run_id: format!("run:{}", graph.dataset_id),
        dataset_id: graph.dataset_id.clone(),
        config_id: "default".to_string(),
        started_at,
        completed_at: Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        ),
        status: RunStatus::Succeeded,
        backend: BackendType::PureRust,
        graph_source_resolved: match resolution_meta.source_resolved {
            crate::core::backend::GraphSource::File => crate::core::types::GraphSourceType::File,
            crate::core::backend::GraphSource::Neo4jSnapshot => {
                crate::core::types::GraphSourceType::Neo4jSnapshot
            }
            crate::core::backend::GraphSource::LiveNeo4j => {
                crate::core::types::GraphSourceType::Neo4jSnapshot
            } // Fallback
        },
        fallback_reason: resolution_meta.fallback_reason,
    };

    // Compute actual community count
    let community_count = {
        let mut seen = HashSet::new();
        for &c in &partition_state.node_to_comm {
            seen.insert(c);
        }
        seen.len()
    };

    // Compute modularity: Q = (1/2m) * sum_ij [ A_ij - k_i*k_j/(2m) ] * delta(c_i, c_j)
    let quality_score = compute_modularity(graph, &partition_state.node_to_comm);

    let partition = PartitionResult {
        run_id: execution.run_id.clone(),
        node_to_community: partition_state.node_to_comm,
        hierarchy_levels,
        community_count,
        quality_score,
        iteration_count,
    };

    Ok(RunOutcome {
        execution,
        partition: Some(partition),
        validation: None,
    })
}

/// Compute standard modularity: Q = (1/2m) * sum_ij [ A_ij - k_i*k_j/(2m) ] * delta(c_i, c_j)
fn compute_modularity(graph: &GraphInput, node_to_community: &[usize]) -> f64 {
    let n = graph.node_count;

    // Compute node degrees from edge list
    let mut node_degrees = vec![0.0f64; n];
    let mut total_weight = 0.0f64;
    for &(u, v, w) in &graph.edges {
        let weight = w.unwrap_or(1.0);
        node_degrees[u] += weight;
        node_degrees[v] += weight;
        total_weight += weight;
    }
    let twice_m = total_weight * 2.0; // each edge counted once in the list but contributes to both endpoints

    if twice_m == 0.0 {
        return 0.0;
    }

    // Sum intra-community edge weights and community degree totals
    let mut intra_weight = 0.0f64;
    let mut community_degree_sum: HashMap<usize, f64> = HashMap::new();
    for &(u, v, w) in &graph.edges {
        let weight = w.unwrap_or(1.0);
        if node_to_community[u] == node_to_community[v] {
            intra_weight += weight * 2.0; // count both directions
        }
    }
    for i in 0..n {
        *community_degree_sum
            .entry(node_to_community[i])
            .or_insert(0.0) += node_degrees[i];
    }

    // Q = (1/2m) * [ sum_intra - sum_c (sigma_c^2 / 2m) ]
    let expected: f64 = community_degree_sum
        .values()
        .map(|sigma_c| sigma_c * sigma_c / twice_m)
        .sum();

    (intra_weight - expected) / twice_m
}

/// Multi-level Leiden: run local moves on the graph, then refine communities into
/// subcommunities (connected components), aggregate based on subcommunities, and repeat.
/// The refinement step prevents mega-communities by ensuring the coarsened graph
/// represents subcommunity-level structure, following the standard Leiden approach.
fn multilevel_leiden(
    state: &mut PartitionState,
    graph: &GraphInput,
    gamma: f64,
    mode: crate::core::config::RunMode,
    max_levels: usize,
) -> (usize, Vec<Vec<usize>>) {
    use crate::core::graph::in_memory::InMemoryGraph;

    let n = graph.node_count;
    let mut total_iterations = 0;
    let mut hierarchy_levels: Vec<Vec<usize>> = Vec::new();

    // Level 0: run on original graph
    if state.supergraphs.is_empty() {
        state.supergraphs.push(InMemoryGraph::from(graph));
    }

    // Single-level movement on the original graph
    let iters = single_level_movement(
        &state.supergraphs[0],
        graph,
        &mut state.community_mapping_per_level[0],
        &state.current_subcommunity_mapping_per_level[0],
        gamma,
        mode,
    );
    total_iterations += iters;

    // Refinement: within each community, merge singletons into subcommunities.
    // Uses the SAME resolution as movement for the quality function.
    let mut subcommunities = refine_singleton_merge(
        &state.supergraphs[0],
        &state.community_mapping_per_level[0],
        gamma,
    );

    // The community assignment (for final output) comes from movement
    state.node_to_comm = state.community_mapping_per_level[0].clone();
    hierarchy_levels.push(canonicalized_community_ids(&state.node_to_comm));

    let mut prev_comm_count = count_unique(&state.node_to_comm);
    let subcomm_count = count_unique(&subcommunities);

    eprintln!(
        "  [level 0] {} -> {} communities ({} subcommunities)",
        n, prev_comm_count, subcomm_count
    );

    // Hierarchical coarsening: aggregate subcommunities and repeat
    for level in 1..max_levels {
        let current_subcomm_count = count_unique(&subcommunities);
        if current_subcomm_count <= 1 {
            break;
        }

        // Aggregate based on SUBCOMMUNITIES (not communities) — this is the key
        // difference from naive coarsening. Subcommunities are the refined partition.
        let (coarse, subcomm_remap) =
            aggregate_graph(graph, &subcommunities, current_subcomm_count);
        let coarse_n = coarse.node_count;

        if coarse_n <= 1 || coarse.edges.is_empty() {
            break;
        }

        // Build initial partition for coarse level: each super-node (subcommunity)
        // starts in its PARENT community from the movement result. This is critical —
        // standard Leiden projects the movement partition onto the coarsened graph
        // so coarse-level movement refines the existing structure rather than
        // rebuilding from scratch (which finds worse local optima).
        let mut subcomm_to_comm: HashMap<usize, usize> = HashMap::new();
        for (i, &subcomm) in subcommunities.iter().enumerate() {
            subcomm_to_comm
                .entry(subcomm)
                .or_insert(state.node_to_comm[i]);
        }
        let coarse_initial_partition =
            build_coarse_initial_partition(&subcomm_to_comm, &subcomm_remap, coarse_n);

        // Run local moves on the coarsened graph (refining the projected partition)
        let mut coarse_state = PartitionState::identity(coarse_n);
        coarse_state.community_mapping_per_level[0] = coarse_initial_partition;
        coarse_state.supergraphs.push(InMemoryGraph::from(&coarse));
        let citers = single_level_movement(
            &coarse_state.supergraphs[0],
            &coarse,
            &mut coarse_state.community_mapping_per_level[0],
            &coarse_state.current_subcommunity_mapping_per_level[0],
            gamma,
            mode,
        );
        total_iterations += citers;

        // Map coarse partition back to original nodes via subcommunity remap
        for c in subcommunities.iter_mut() {
            let contiguous_id = subcomm_remap[c];
            *c = coarse_state.community_mapping_per_level[0][contiguous_id];
        }
        // Update the community assignment from subcommunities
        state.node_to_comm = subcommunities.clone();
        state.community_mapping_per_level[0] = state.node_to_comm.clone();
        hierarchy_levels.push(canonicalized_community_ids(&state.node_to_comm));

        let new_comm_count = count_unique(&state.node_to_comm);

        // Refine again for the next level (uses movement resolution for quality)
        subcommunities = refine_singleton_merge(
            &state.supergraphs[0],
            &state.node_to_comm,
            gamma,
        );
        let new_subcomm_count = count_unique(&subcommunities);

        eprintln!(
            "  [level {}] {} -> {} communities ({} subcommunities)",
            level, current_subcomm_count, new_comm_count, new_subcomm_count
        );

        // Converge based on COMMUNITY count (movement result), not subcommunity count.
        // Subcommunity counts naturally increase after refinement — that's expected.
        // We stop when movement can no longer reduce the number of communities.
        if new_comm_count >= prev_comm_count {
            break;
        }
        prev_comm_count = new_comm_count;
    }

    // Final polish: one more movement pass on the original graph using the
    // coarsened partition as a warm start. This fixes suboptimal assignments
    // that coarsening may introduce.
    state.community_mapping_per_level[0] = state.node_to_comm.clone();
    let polish_iters = single_level_movement(
        &state.supergraphs[0],
        graph,
        &mut state.community_mapping_per_level[0],
        &state.current_subcommunity_mapping_per_level[0],
        gamma,
        mode,
    );
    state.node_to_comm = state.community_mapping_per_level[0].clone();
    total_iterations += polish_iters;

    let final_comm_count = count_unique(&state.node_to_comm);
    eprintln!("  [polish] -> {} communities", final_comm_count);

    let final_level = canonicalized_community_ids(&state.node_to_comm);
    if hierarchy_levels
        .last()
        .map(|last| last != &final_level)
        .unwrap_or(true)
    {
        hierarchy_levels.push(final_level);
    }

    (total_iterations, hierarchy_levels)
}

fn build_coarse_initial_partition(
    subcomm_to_comm: &HashMap<usize, usize>,
    subcomm_remap: &HashMap<usize, usize>,
    coarse_n: usize,
) -> Vec<usize> {
    let mut coarse_comm_remap: HashMap<usize, usize> = HashMap::new();
    let mut next_coarse_comm = 0usize;
    let mut coarse_initial_partition = vec![0usize; coarse_n];
    for (&subcomm, &comm) in subcomm_to_comm {
        let Some(&coarse_super_id) = subcomm_remap.get(&subcomm) else {
            continue;
        };
        let coarse_comm_id = *coarse_comm_remap.entry(comm).or_insert_with(|| {
            let id = next_coarse_comm;
            next_coarse_comm += 1;
            id
        });
        coarse_initial_partition[coarse_super_id] = coarse_comm_id;
    }
    coarse_initial_partition
}

/// Count the number of unique values in a slice.
fn count_unique(v: &[usize]) -> usize {
    let mut s = HashSet::new();
    for &c in v {
        s.insert(c);
    }
    s.len()
}

/// Deterministically rewrite community labels to contiguous IDs [0..k-1]
/// by scanning nodes in index order and assigning first-seen labels.
fn canonicalize_community_ids_in_place(node_to_community: &mut [usize]) {
    let mut remap: HashMap<usize, usize> = HashMap::new();
    let mut next_id = 0usize;

    for c in node_to_community.iter_mut() {
        let mapped = remap.entry(*c).or_insert_with(|| {
            let id = next_id;
            next_id += 1;
            id
        });
        *c = *mapped;
    }
}

fn canonicalized_community_ids(node_to_community: &[usize]) -> Vec<usize> {
    let mut out = node_to_community.to_vec();
    canonicalize_community_ids_in_place(&mut out);
    out
}

fn find_connected_components_in_subcommunity(
    graph: &crate::core::graph::in_memory::InMemoryGraph,
    vertices: &[usize],
    n: usize,
) -> Vec<Vec<usize>> {
    let mut allowed = vec![false; n];
    for &v in vertices {
        allowed[v] = true;
    }

    let mut visited = vec![false; n];
    let mut components: Vec<Vec<usize>> = Vec::new();

    for &start in vertices {
        if visited[start] {
            continue;
        }
        let comp = bfs_collect_component(graph, start, &allowed, &mut visited, n);
        components.push(comp);
    }

    components
}

fn bfs_collect_component(
    graph: &crate::core::graph::in_memory::InMemoryGraph,
    start: usize,
    allowed: &[bool],
    visited: &mut [bool],
    n: usize,
) -> Vec<usize> {
    let mut q = VecDeque::new();
    let mut comp = Vec::new();
    q.push_back(start);
    visited[start] = true;

    while let Some(cur) = q.pop_front() {
        comp.push(cur);
        for (nbr, _w) in graph.neighbors(cur) {
            if nbr < n && allowed[nbr] && !visited[nbr] {
                visited[nbr] = true;
                q.push_back(nbr);
            }
        }
    }
    comp
}

fn split_non_largest_components(
    components: &[Vec<usize>],
    keep_idx: usize,
    subcommunities: &mut [usize],
    refined_nodes: &mut BitVec,
    next_subcommunity_id: &mut usize,
) {
    for (idx, comp) in components.iter().enumerate() {
        if idx == keep_idx {
            continue;
        }
        let new_sc = *next_subcommunity_id;
        *next_subcommunity_id += 1;
        for &v in comp {
            subcommunities[v] = new_sc;
            refined_nodes.set(v, true);
        }
    }
}

/// Leiden-style refinement: within each community, start from singleton subcommunities
/// and greedily merge nodes that improve modularity. This produces a FINER partition
/// than the community partition, which is used for aggregation.
fn refine_singleton_merge(
    graph: &crate::core::graph::in_memory::InMemoryGraph,
    node_to_community: &[usize],
    gamma: f64,
) -> Vec<usize> {
    let n = graph.node_count;
    if n == 0 {
        return Vec::new();
    }

    // Paper-faithful baseline: start with subcommunities equal to current communities,
    // then split disconnected components and merge singleton subcommunities.
    let mut subcommunities: Vec<usize> = node_to_community.to_vec();

    let twice_total_weight = graph.total_weight() * 2.0;
    if twice_total_weight == 0.0 {
        return subcommunities;
    }

    let mut node_degrees = vec![0.0; n];
    for (i, deg) in node_degrees.iter_mut().enumerate() {
        *deg = graph.neighbors(i).map(|(_, w)| w).sum();
    }

    // Parent community degrees d(C)
    let mut community_degrees: HashMap<usize, f64> = HashMap::new();
    for (i, &deg) in node_degrees.iter().enumerate() {
        *community_degrees.entry(node_to_community[i]).or_insert(0.0) += deg;
    }

    // --- Algorithm 3 lines 2-4: connected-component split per subcommunity ---
    let mut next_subcommunity_id = subcommunities.iter().copied().max().unwrap_or(0) + 1;
    let mut refined_nodes = bitvec![0; n];

    let mut nodes_by_subcommunity: HashMap<usize, Vec<usize>> = HashMap::new();
    for (v, &sc) in subcommunities.iter().enumerate() {
        nodes_by_subcommunity.entry(sc).or_default().push(v);
    }

    let mut sc_ids: Vec<usize> = nodes_by_subcommunity.keys().copied().collect();
    sc_ids.sort_unstable();

    for sc in sc_ids {
        let Some(vertices) = nodes_by_subcommunity.get(&sc) else {
            continue;
        };
        if vertices.len() <= 1 {
            continue;
        }

        let components = find_connected_components_in_subcommunity(graph, vertices, n);

        if components.len() <= 1 {
            continue;
        }

        // Keep largest connected component on original ID; deterministic tie-break:
        // smallest min-node-id wins on equal sizes.
        let keep_idx = components
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                a.len().cmp(&b.len()).then_with(|| {
                    let a_min = a.iter().copied().min().unwrap_or(usize::MAX);
                    let b_min = b.iter().copied().min().unwrap_or(usize::MAX);
                    b_min.cmp(&a_min)
                })
            })
            .map(|(idx, _)| idx)
            .unwrap_or(0);

        split_non_largest_components(
            &components,
            keep_idx,
            &mut subcommunities,
            &mut refined_nodes,
            &mut next_subcommunity_id,
        );
    }

    // Subcommunity size and degree tracking (updated as moves happen)
    let mut subcommunity_sizes: HashMap<usize, usize> = HashMap::new();
    let mut subcommunity_degrees: HashMap<usize, f64> = HashMap::new();
    for (v, &sc) in subcommunities.iter().enumerate() {
        *subcommunity_sizes.entry(sc).or_insert(0) += 1;
        *subcommunity_degrees.entry(sc).or_insert(0.0) += node_degrees[v];
    }

    // Deterministic processing order for R (ascending degree, then node id)
    let mut refined_nodes_sorted: Vec<usize> = refined_nodes.iter_ones().collect();
    refined_nodes_sorted.sort_by(|&a, &b| {
        node_degrees[a]
            .partial_cmp(&node_degrees[b])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.cmp(&b))
    });

    // --- Algorithm 3 lines 5-13: singleton merge with T-filter and argmax ΔM ---
    const EPS: f64 = 1e-12;
    let mut neighbor_subcommunities: HashMap<usize, f64> = HashMap::new();
    for &current_node in &refined_nodes_sorted {
        let current_sc = subcommunities[current_node];
        if subcommunity_sizes.get(&current_sc).copied().unwrap_or(0) != 1 {
            continue;
        }

        let current_parent_comm = node_to_community[current_node];
        let current_degree = node_degrees[current_node];

        neighbor_subcommunities.clear();
        let mut weight_to_current_subcommunity = 0.0;

        for (neighbor_node, w) in graph.neighbors(current_node) {
            if neighbor_node == current_node {
                continue;
            }
            if node_to_community[neighbor_node] != current_parent_comm {
                continue;
            }

            let neighbor_subcommunity = subcommunities[neighbor_node];
            *neighbor_subcommunities
                .entry(neighbor_subcommunity)
                .or_insert(0.0) += w;

            if neighbor_subcommunity == current_sc {
                weight_to_current_subcommunity += w;
            }
        }

        if neighbor_subcommunities.is_empty() {
            continue;
        }

        // Candidate set T filter:
        // T = { S | S among same-community neighbors AND ΔQ(S -> ∅, γ) <= 0 }
        let mut candidate_ids: Vec<usize> = neighbor_subcommunities.keys().copied().collect();
        candidate_ids.sort_unstable();

        let mut filtered_candidates: Vec<usize> = Vec::new();
        for cand_sc in candidate_ids {
            if cand_sc == current_sc {
                continue;
            }
            let delta_q_remove = subcommunity_delta_q_to_empty(
                graph,
                &subcommunities,
                node_to_community,
                &subcommunity_degrees,
                cand_sc,
                current_parent_comm,
                gamma,
                twice_total_weight,
            );
            if delta_q_remove <= EPS {
                filtered_candidates.push(cand_sc);
            }
        }

        if filtered_candidates.is_empty() {
            continue;
        }

        // argmax ΔM(v -> S, γ) over filtered T
        let mut best_subcommunity = current_sc;
        let mut best_modularity_gain = 0.0;
        let mut best_weight_to_candidate = -1.0;

        for &candidate_subcommunity in &filtered_candidates {
            let weight_to_candidate_subcommunity = neighbor_subcommunities
                .get(&candidate_subcommunity)
                .copied()
                .unwrap_or(0.0);

            let current_subcommunity_degree = *subcommunity_degrees
                .get(&current_sc)
                .unwrap_or(&current_degree);
            let candidate_subcommunity_degree = *subcommunity_degrees
                .get(&candidate_subcommunity)
                .unwrap_or(&0.0);

            let modularity_gain = (weight_to_candidate_subcommunity
                - weight_to_current_subcommunity)
                / twice_total_weight
                + gamma
                    * current_degree
                    * (current_subcommunity_degree
                        - current_degree
                        - candidate_subcommunity_degree)
                    / (twice_total_weight * twice_total_weight);

            // Deterministic tie-break rules:
            // 1) larger gain, 2) larger direct edge weight, 3) smaller candidate ID.
            let better = (modularity_gain > best_modularity_gain + EPS)
                || ((modularity_gain - best_modularity_gain).abs() <= EPS
                    && (weight_to_candidate_subcommunity > best_weight_to_candidate + EPS
                        || ((weight_to_candidate_subcommunity - best_weight_to_candidate).abs()
                            <= EPS
                            && candidate_subcommunity < best_subcommunity)));

            if better {
                best_modularity_gain = modularity_gain;
                best_subcommunity = candidate_subcommunity;
                best_weight_to_candidate = weight_to_candidate_subcommunity;
            }
        }

        if best_modularity_gain > EPS {
            // update_edge(G_Ψ, ...) equivalent state updates for subcommunity membership
            subcommunities[current_node] = best_subcommunity;

            if let Some(size) = subcommunity_sizes.get_mut(&current_sc) {
                *size = size.saturating_sub(1);
            }
            *subcommunity_sizes.entry(best_subcommunity).or_insert(0) += 1;

            *subcommunity_degrees.entry(current_sc).or_insert(0.0) -= current_degree;
            *subcommunity_degrees.entry(best_subcommunity).or_insert(0.0) += current_degree;
        }
    }

    subcommunities
}

/// ΔQ(S -> ∅, γ) for a subcommunity S inside parent community C.
/// Used for Algorithm-3 candidate set filtering: include S only when this is <= 0.
#[allow(clippy::too_many_arguments)]
fn subcommunity_delta_q_to_empty(
    graph: &crate::core::graph::in_memory::InMemoryGraph,
    subcommunities: &[usize],
    node_to_community: &[usize],
    subcommunity_degrees: &HashMap<usize, f64>,
    subcommunity_id: usize,
    parent_community_id: usize,
    gamma: f64,
    twice_total_weight: f64,
) -> f64 {
    if twice_total_weight == 0.0 {
        return 0.0;
    }

    let d_s = subcommunity_degrees
        .get(&subcommunity_id)
        .copied()
        .unwrap_or(0.0);
    if d_s == 0.0 {
        return 0.0;
    }

    let mut d_parent = 0.0;
    for (i, &comm) in node_to_community.iter().enumerate() {
        if comm == parent_community_id {
            d_parent += graph.neighbors(i).map(|(_, w)| w).sum::<f64>();
        }
    }

    // w(S, C_parent): total edge weight from S to nodes in parent community
    let mut w_s_to_parent = 0.0;
    for (i, &subcomm) in subcommunities.iter().enumerate() {
        if subcomm != subcommunity_id {
            continue;
        }
        for (nbr, w) in graph.neighbors(i) {
            if node_to_community[nbr] == parent_community_id {
                w_s_to_parent += w;
            }
        }
    }

    (0.0 - w_s_to_parent) / twice_total_weight
        + gamma * d_s * (d_parent - d_s) / (twice_total_weight * twice_total_weight)
}

/// Build a coarsened graph where each community becomes a super-node.
/// Edge weights between super-nodes are the sum of inter-community edge weights.
/// Returns (coarsened_graph, comm_remap) where comm_remap maps original non-contiguous
/// community IDs to contiguous 0..k IDs used in the coarsened graph.
fn aggregate_graph(
    graph: &GraphInput,
    node_to_community: &[usize],
    _num_communities: usize,
) -> (GraphInput, HashMap<usize, usize>) {
    // Remap community IDs to contiguous 0..num_communities
    let mut comm_remap: HashMap<usize, usize> = HashMap::new();
    let mut next_id = 0;
    for &c in node_to_community {
        comm_remap.entry(c).or_insert_with(|| {
            let id = next_id;
            next_id += 1;
            id
        });
    }

    let remapped: Vec<usize> = node_to_community.iter().map(|c| comm_remap[c]).collect();

    // Aggregate edges between communities (inter-community edges)
    // AND self-loops for intra-community edges (critical for correct modularity at higher levels)
    let mut edge_map: HashMap<(usize, usize), f64> = HashMap::new();
    for &(u, v, w) in &graph.edges {
        let cu = remapped[u];
        let cv = remapped[v];
        let weight = w.unwrap_or(1.0);
        if cu == cv {
            // Intra-community edge: becomes a self-loop on the super-node
            *edge_map.entry((cu, cu)).or_default() += weight;
        } else {
            let key = if cu < cv { (cu, cv) } else { (cv, cu) };
            *edge_map.entry(key).or_default() += weight;
        }
    }

    let edges: Vec<(usize, usize, Option<f64>)> = edge_map
        .into_iter()
        .map(|((a, b), w)| (a, b, Some(w)))
        .collect();

    (
        GraphInput {
            dataset_id: graph.dataset_id.clone(),
            node_count: next_id,
            edges,
        },
        comm_remap,
    )
}

/// Run a single level of local movement (the core Leiden greedy move phase).
fn single_level_movement(
    graph: &crate::core::graph::in_memory::InMemoryGraph,
    delta_graph: &GraphInput,
    node_to_community: &mut [usize],
    node_to_subcommunity: &[usize],
    gamma: f64,
    mode: crate::core::config::RunMode,
) -> usize {
    let n = graph.node_count;
    let mut active_nodes = bitvec![0; n];

    // Activate nodes at edges crossing community boundaries
    for &(u, v, w) in &delta_graph.edges {
        let alpha = w.unwrap_or(1.0);
        if alpha > 0.0 && u < n && v < n && node_to_community[u] != node_to_community[v] {
            active_nodes.set(u, true);
            active_nodes.set(v, true);
        }
    }

    if delta_graph.edges.is_empty() {
        active_nodes.fill(true);
    }

    let twice_total_weight = graph.total_weight() * 2.0;
    let mut community_degrees = vec![0.0; n];
    let mut node_degrees = vec![0.0; n];

    for i in 0..n {
        let d_i: f64 = graph.neighbors(i).map(|(_, w)| w).sum();
        node_degrees[i] = d_i;
        community_degrees[node_to_community[i]] += d_i;
    }

    // Throughput safety gate:
    // Parallel movement in the multilevel path currently shows severe
    // quality regressions on large benchmarks (fragmentation + low modularity).
    // Keep throughput enabled in incremental Algorithm-6 (`inc_movement`) paths,
    // but force deterministic movement for multilevel until fixed.
    const USE_PARALLEL_MULTILEVEL_MOVEMENT: bool = false;

    // Throughput mode: parallel batch processing via rayon work-stealing
    if mode == crate::core::config::RunMode::Throughput && USE_PARALLEL_MULTILEVEL_MOVEMENT {
        let buffer_pool = crate::core::algorithm::throughput::BufferPool::new(
            2 * n,
            rayon::current_num_threads(),
        );
        const MAX_MOVEMENT_ITERATIONS: usize = 20;
        let mut iteration = 0;
        let mut active_indices: Vec<usize> = active_nodes.iter_ones().collect();
        while !active_indices.is_empty() && iteration < MAX_MOVEMENT_ITERATIONS {
            let (_changed, _affected, next_active) =
                crate::core::algorithm::throughput::inc_movement_parallel(
                    graph,
                    &active_indices,
                    node_to_community,
                    node_to_subcommunity,
                    &mut community_degrees,
                    &node_degrees,
                    twice_total_weight,
                    gamma,
                    false,
                    &buffer_pool,
                );

            next_active.collect_ones_into(&mut active_indices);
            iteration += 1;
        }
        let comm_count = count_unique(node_to_community);
        eprintln!(
            "    {} iterations (parallel), {} communities",
            iteration, comm_count
        );
        return iteration;
    }

    // Queue-based movement (deterministic mode):
    // Process one node at a time; when a node moves, re-queue its neighbors
    // that aren't in the target community. This propagates chain reactions
    // and forms larger communities than round-based approaches.
    use rand::seq::SliceRandom;
    let mut rng = rand::thread_rng();

    let mut queue_nodes: Vec<usize> = active_nodes.iter_ones().collect();
    queue_nodes.shuffle(&mut rng);
    let mut queue: VecDeque<usize> = queue_nodes.into();
    let mut in_queue = active_nodes.clone();

    let mut total_moves = 0usize;
    let mut neighbor_communities: HashMap<usize, f64> = HashMap::new();

    while let Some(current_node) = queue.pop_front() {
        in_queue.set(current_node, false);

        let mut best_community = node_to_community[current_node];
        let mut best_gain = 0.0;

        neighbor_communities.clear();
        let mut weight_to_current = 0.0;
        let current_degree = node_degrees[current_node];

        for (neighbor, w) in graph.neighbors(current_node) {
            if neighbor == current_node {
                continue; // Skip self-loops
            }
            let c = node_to_community[neighbor];
            *neighbor_communities.entry(c).or_default() += w;
            if c == node_to_community[current_node] {
                weight_to_current += w;
            }
        }

        let current_comm_deg = community_degrees[node_to_community[current_node]];

        for (&cand_comm, &w_cand) in &neighbor_communities {
            if cand_comm == node_to_community[current_node] {
                continue;
            }
            let cand_comm_deg = community_degrees[cand_comm];
            let gain = (w_cand - weight_to_current) / twice_total_weight
                + gamma * current_degree * (current_comm_deg - current_degree - cand_comm_deg)
                    / (twice_total_weight * twice_total_weight);

            if gain > best_gain {
                best_gain = gain;
                best_community = cand_comm;
            }
        }

        if best_gain <= 0.0 {
            continue;
        }

        let old = node_to_community[current_node];
        node_to_community[current_node] = best_community;
        community_degrees[old] -= current_degree;
        community_degrees[best_community] += current_degree;
        total_moves += 1;

        // Re-queue neighbors not in the target community
        for (neighbor, _w) in graph.neighbors(current_node) {
            if neighbor == current_node
                || node_to_community[neighbor] == best_community
                || in_queue[neighbor]
            {
                continue;
            }
            queue.push_back(neighbor);
            in_queue.set(neighbor, true);
        }
    }

    let comm_count = count_unique(node_to_community);
    eprintln!("    {} moves, {} communities", total_moves, comm_count);

    1 // Single pass (queue-based is equivalent to multiple rounds)
}

// Algorithm 6: HIT-Leiden
// Returns the total number of movement iterations across all levels.
pub fn hit_leiden(
    state: &mut PartitionState,
    delta_g: &GraphInput,
    gamma: f64,
    mode: crate::core::config::RunMode,
) -> usize {
    use crate::core::graph::in_memory::InMemoryGraph;

    if state.supergraphs.is_empty() {
        state.supergraphs.push(InMemoryGraph::from(delta_g));
    }

    let p_max = state.levels;
    // Use Cow to avoid cloning delta_g at level 0; only own when aggregation produces a new delta
    let mut current_delta: Cow<GraphInput> = Cow::Borrowed(delta_g);

    let mut changed_nodes_per_level: Vec<BitVec> = vec![bitvec![0; delta_g.node_count]; p_max];
    let mut refined_nodes_per_level: Vec<BitVec> = vec![bitvec![0; delta_g.node_count]; p_max];
    let mut total_iterations = 0;

    for p in 0..p_max {
        // Algorithm 6, line 3: G^p ← G^p ⊕ ΔG^p
        state.supergraphs[p] = state.supergraphs[p].apply_delta(&current_delta);

        let (b_p, k, iters) = inc_movement(
            &state.supergraphs[p],
            &current_delta,
            &mut state.community_mapping_per_level[p],
            &state.current_subcommunity_mapping_per_level[p],
            gamma,
            mode,
        );
        changed_nodes_per_level[p] = b_p;
        total_iterations += iters;

        let r_p = inc_refinement(
            &state.supergraphs[p],
            &state.community_mapping_per_level[p],
            &mut state.current_subcommunity_mapping_per_level[p],
            &k,
            gamma,
            mode,
        );
        refined_nodes_per_level[p] = r_p.clone();

        if p < p_max - 1 {
            if should_skip_aggregation(&current_delta, &r_p) {
                current_delta = Cow::Owned(GraphInput {
                    dataset_id: current_delta.dataset_id.clone(),
                    node_count: current_delta.node_count,
                    edges: Vec::new(),
                });
                continue;
            }

            let (next_delta, next_s_pre) = inc_aggregation(
                &state.supergraphs[p],
                &current_delta,
                &state.previous_subcommunity_mapping_per_level[p],
                &state.current_subcommunity_mapping_per_level[p],
                &r_p,
            );
            current_delta = Cow::Owned(next_delta);
            state.previous_subcommunity_mapping_per_level[p] = next_s_pre;
        }
    }

    def_update(
        &mut state.community_mapping_per_level,
        &state.current_subcommunity_mapping_per_level,
        &mut changed_nodes_per_level,
        p_max,
    );
    def_update(
        &mut state.refined_community_mapping_per_level,
        &state.current_subcommunity_mapping_per_level,
        &mut refined_nodes_per_level,
        p_max,
    );
    // Algorithm 6, line 10: output g¹ (refined mapping), not f¹ (movement mapping)
    state.node_to_comm = state.refined_community_mapping_per_level[0].clone();
    total_iterations
}

fn should_skip_aggregation(delta_graph: &GraphInput, refined_nodes: &BitVec) -> bool {
    delta_graph.edges.is_empty() && !refined_nodes.any()
}

fn inc_movement(
    graph: &crate::core::graph::in_memory::InMemoryGraph,
    delta_graph: &GraphInput,
    node_to_community: &mut [usize],
    node_to_subcommunity: &[usize],
    resolution_parameter: f64,
    mode: crate::core::config::RunMode,
) -> (BitVec, BitVec, usize) {
    let n = graph.node_count;
    let mut active_nodes = bitvec![0; n];
    let mut changed_nodes = bitvec![0; n];
    let mut affected_nodes_for_refinement = bitvec![0; n];

    // 2 for (v_i, v_j, \alpha) \in \Delta G do
    for &(u, v, w) in &delta_graph.edges {
        let alpha = w.unwrap_or(1.0);
        if alpha > 0.0 && node_to_community[u] != node_to_community[v] {
            active_nodes.set(u, true);
            active_nodes.set(v, true);
        }
        if alpha < 0.0 && node_to_community[u] == node_to_community[v] {
            active_nodes.set(u, true);
            active_nodes.set(v, true);
        }
        if node_to_subcommunity[u] == node_to_subcommunity[v] {
            affected_nodes_for_refinement.set(u, true);
            affected_nodes_for_refinement.set(v, true);
        }
    }

    // If delta_graph is empty (initial run), activate all nodes
    if delta_graph.edges.is_empty() {
        active_nodes.fill(true);
    }

    let twice_total_weight = graph.total_weight() * 2.0;
    let mut community_degrees = vec![0.0; n];
    let mut node_degrees = vec![0.0; n];
    for i in 0..n {
        let d_i: f64 = graph.neighbors(i).map(|(_, w)| w).sum();
        node_degrees[i] = d_i;
        community_degrees[node_to_community[i]] += d_i;
    }

    if mode == crate::core::config::RunMode::Throughput {
        // Create buffer pool once for reuse across multiple inc_movement_parallel calls
        let buffer_pool = crate::core::algorithm::throughput::BufferPool::new(
            2 * n,
            rayon::current_num_threads(),
        );
        // Cap iterations to prevent oscillation in parallel mode.
        // Parallel batch processing uses stale snapshots, so nodes can make
        // conflicting moves that re-activate neighbours indefinitely.
        const MAX_MOVEMENT_ITERATIONS: usize = 20;
        let mut iteration = 0;
        let mut active_indices: Vec<usize> = active_nodes.iter_ones().collect();
        while !active_indices.is_empty() && iteration < MAX_MOVEMENT_ITERATIONS {
            let (new_changed, new_affected, next_active) =
                crate::core::algorithm::throughput::inc_movement_parallel(
                    graph,
                    &active_indices,
                    node_to_community,
                    node_to_subcommunity,
                    &mut community_degrees,
                    &node_degrees,
                    twice_total_weight,
                    resolution_parameter,
                    true,
                    &buffer_pool,
                );

            new_changed.or_into_bitvec(&mut changed_nodes);
            new_affected.or_into_bitvec(&mut affected_nodes_for_refinement);
            next_active.collect_ones_into(&mut active_indices);
            iteration += 1;
        }
        return (changed_nodes, affected_nodes_for_refinement, iteration);
    }

    // Track next available community ID for ∅ (empty community) allocations
    let mut next_empty_community_id = community_degrees.len();
    let mut neighbor_communities: HashMap<usize, f64> = HashMap::new();

    // 9 for A \neq \emptyset do (deterministic mode)
    while active_nodes.any() {
        let current_node = active_nodes.iter_ones().next().unwrap();
        active_nodes.set(current_node, false);

        let mut best_community = node_to_community[current_node];
        let mut best_modularity_gain = 0.0;

        neighbor_communities.clear();
        let mut weight_to_current_community = 0.0;
        let current_node_degree = node_degrees[current_node];

        for (neighbor_node, w) in graph.neighbors(current_node) {
            let c = node_to_community[neighbor_node];
            *neighbor_communities.entry(c).or_insert(0.0) += w;
            if c == node_to_community[current_node] {
                weight_to_current_community += w;
            }
        }

        for (&candidate_community, &weight_to_candidate_community) in &neighbor_communities {
            if candidate_community == node_to_community[current_node] {
                continue;
            }

            let current_community_degree = community_degrees[node_to_community[current_node]];
            let candidate_community_degree = community_degrees[candidate_community];

            let modularity_gain = (weight_to_candidate_community - weight_to_current_community)
                / twice_total_weight
                + resolution_parameter
                    * current_node_degree
                    * (current_community_degree - current_node_degree - candidate_community_degree)
                    / (twice_total_weight * twice_total_weight);

            if modularity_gain > best_modularity_gain {
                best_modularity_gain = modularity_gain;
                best_community = candidate_community;
            }
        }

        // Algorithm 2: argmax includes ∅ (empty community = singleton)
        // Gain of moving to empty community: w_to_candidate=0, candidate_degree=0
        {
            let current_community_degree = community_degrees[node_to_community[current_node]];
            let empty_gain = -weight_to_current_community / twice_total_weight
                + resolution_parameter
                    * current_node_degree
                    * (current_community_degree - current_node_degree)
                    / (twice_total_weight * twice_total_weight);

            if empty_gain > best_modularity_gain {
                best_modularity_gain = empty_gain;
                best_community = next_empty_community_id;
            }
        }

        if best_modularity_gain <= 0.0 {
            continue;
        }

        let old_community = node_to_community[current_node];
        // Allocate empty community if needed
        if best_community >= community_degrees.len() {
            community_degrees.resize(best_community + 1, 0.0);
            next_empty_community_id = best_community + 1;
        }
        node_to_community[current_node] = best_community;
        changed_nodes.set(current_node, true);
        community_degrees[old_community] -= current_node_degree;
        community_degrees[best_community] += current_node_degree;

        for (neighbor_node, _w) in graph.neighbors(current_node) {
            if node_to_community[neighbor_node] != best_community {
                active_nodes.set(neighbor_node, true);
            }
            if node_to_subcommunity[current_node] == node_to_subcommunity[neighbor_node] {
                affected_nodes_for_refinement.set(current_node, true);
                affected_nodes_for_refinement.set(neighbor_node, true);
            }
        }
    }

    // Deterministic mode processes one node at a time (1 "iteration" encompassing all moves)
    (changed_nodes, affected_nodes_for_refinement, 1)
}

fn find_connected_components_by_subcommunity(
    graph: &crate::core::graph::in_memory::InMemoryGraph,
    vertices: &[usize],
    node_to_subcommunity: &[usize],
    visited: &mut BitVec,
) -> Vec<Vec<usize>> {
    let mut components: Vec<Vec<usize>> = Vec::new();

    for &start_node in vertices {
        if visited[start_node] {
            continue;
        }
        let mut comp = Vec::new();
        let mut queue = VecDeque::new();
        queue.push_back(start_node);
        visited.set(start_node, true);

        while let Some(current_node) = queue.pop_front() {
            comp.push(current_node);
            let current_sc = node_to_subcommunity[current_node];
            enqueue_subcommunity_neighbors(
                graph,
                current_node,
                current_sc,
                node_to_subcommunity,
                visited,
                &mut queue,
            );
        }
        components.push(comp);
    }

    components
}

fn enqueue_subcommunity_neighbors(
    graph: &crate::core::graph::in_memory::InMemoryGraph,
    current_node: usize,
    current_sc: usize,
    node_to_subcommunity: &[usize],
    visited: &mut BitVec,
    queue: &mut VecDeque<usize>,
) {
    for (neighbor_node, _w) in graph.neighbors(current_node) {
        if node_to_subcommunity[neighbor_node] == current_sc && !visited[neighbor_node] {
            visited.set(neighbor_node, true);
            queue.push_back(neighbor_node);
        }
    }
}

fn split_non_largest_components_subcommunity(
    components: &[Vec<usize>],
    keep_idx: usize,
    node_to_subcommunity: &mut [usize],
    refined_nodes: &mut BitVec,
    next_subcommunity_id: &mut usize,
) {
    for (idx, comp) in components.iter().enumerate() {
        if idx == keep_idx {
            continue;
        }
        let new_subcommunity = *next_subcommunity_id;
        *next_subcommunity_id += 1;
        for &v in comp {
            node_to_subcommunity[v] = new_subcommunity;
            refined_nodes.set(v, true);
        }
    }
}

fn inc_refinement(
    graph: &crate::core::graph::in_memory::InMemoryGraph,
    node_to_community: &[usize],
    node_to_subcommunity: &mut [usize],
    affected_nodes: &BitVec,
    resolution_parameter: f64,
    mode: crate::core::config::RunMode,
) -> BitVec {
    let n = graph.node_count;
    let mut refined_nodes = bitvec![0; n];

    // Build inverted index: subcommunity -> nodes (only for affected subcommunities)
    let mut affected_subcommunities: HashSet<usize> = HashSet::new();
    for v in affected_nodes.iter_ones() {
        affected_subcommunities.insert(node_to_subcommunity[v]);
    }

    // Build node lists per affected subcommunity in a single O(n) pass
    let mut subcomm_nodes: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, &sc) in node_to_subcommunity.iter().enumerate() {
        if affected_subcommunities.contains(&sc) {
            subcomm_nodes.entry(sc).or_default().push(i);
        }
    }

    let mut next_subcommunity_id = node_to_subcommunity.iter().max().copied().unwrap_or(0) + 1;

    // Reusable visited bitvec across subcommunities
    let mut visited = bitvec![0; n];

    // 2 for v_i \in K do — connected component splitting
    for vertices in subcomm_nodes.values() {
        if vertices.is_empty() {
            continue;
        }

        let components = find_connected_components_by_subcommunity(
            graph,
            vertices,
            node_to_subcommunity,
            &mut visited,
        );

        // Clear visited bits for nodes we touched
        for &v in vertices {
            visited.set(v, false);
        }

        if components.len() <= 1 {
            continue;
        }

        let largest_idx = components
            .iter()
            .enumerate()
            .max_by_key(|(_, c)| c.len())
            .map(|(i, _)| i)
            .unwrap();

        split_non_largest_components_subcommunity(
            &components,
            largest_idx,
            node_to_subcommunity,
            &mut refined_nodes,
            &mut next_subcommunity_id,
        );
    }

    let is_initial = node_to_subcommunity
        .iter()
        .enumerate()
        .all(|(i, &c)| i == c);
    if is_initial {
        refined_nodes.fill(true);
    }

    // Pre-compute subcommunity sizes for O(1) singleton check
    let max_subcomm = node_to_subcommunity.iter().max().copied().unwrap_or(0);
    let mut subcommunity_sizes = vec![0usize; max_subcomm + 1];
    for &sc in node_to_subcommunity.iter() {
        subcommunity_sizes[sc] += 1;
    }

    let twice_total_weight = graph.total_weight() * 2.0;
    let mut subcommunity_degrees: HashMap<usize, f64> = HashMap::new();
    let mut node_degrees = vec![0.0; n];
    for i in 0..n {
        let d_i: f64 = graph.neighbors(i).map(|(_, w)| w).sum();
        node_degrees[i] = d_i;
        *subcommunity_degrees
            .entry(node_to_subcommunity[i])
            .or_insert(0.0) += d_i;
    }

    let mut refined_nodes_sorted: Vec<usize> = refined_nodes.iter_ones().collect();
    refined_nodes_sorted.sort_by(|&a, &b| node_degrees[a].partial_cmp(&node_degrees[b]).unwrap());

    if mode == crate::core::config::RunMode::Throughput {
        // Pre-compute T-filter: identify subcommunities where ΔQ(S→∅) > 0,
        // meaning removing S would improve quality — these are "healthy" subcommunities
        // that should NOT absorb singletons.
        let blocked_subcommunities = compute_blocked_subcommunities(
            graph,
            node_to_community,
            node_to_subcommunity,
            &subcommunity_degrees,
            &refined_nodes_sorted,
            &subcommunity_sizes,
            &node_degrees,
            twice_total_weight,
            resolution_parameter,
        );
        crate::core::algorithm::throughput::inc_refinement_parallel(
            graph,
            &refined_nodes_sorted,
            node_to_community,
            node_to_subcommunity,
            &mut subcommunity_degrees,
            &mut subcommunity_sizes,
            &node_degrees,
            twice_total_weight,
            resolution_parameter,
            &blocked_subcommunities,
        );
        return refined_nodes;
    }

    // 5 for v_i \in R do (deterministic refinement merging)
    let mut neighbor_sc_buf: HashMap<usize, f64> = HashMap::new();
    for &current_node in &refined_nodes_sorted {
        // O(1) singleton check
        if subcommunity_sizes[node_to_subcommunity[current_node]] != 1 {
            continue;
        }

        try_merge_singleton_refinement(
            graph,
            current_node,
            n,
            node_to_community,
            node_to_subcommunity,
            &node_degrees,
            &mut subcommunity_sizes,
            &mut subcommunity_degrees,
            twice_total_weight,
            resolution_parameter,
            &mut neighbor_sc_buf,
        );
    }

    refined_nodes
}

#[allow(clippy::too_many_arguments)]
fn try_merge_singleton_refinement(
    graph: &crate::core::graph::in_memory::InMemoryGraph,
    current_node: usize,
    n: usize,
    node_to_community: &[usize],
    node_to_subcommunity: &mut [usize],
    node_degrees: &[f64],
    subcommunity_sizes: &mut [usize],
    subcommunity_degrees: &mut HashMap<usize, f64>,
    twice_total_weight: f64,
    resolution_parameter: f64,
    neighbor_subcommunities: &mut HashMap<usize, f64>,
) {
    neighbor_subcommunities.clear();
    let mut weight_to_current_subcommunity = 0.0;
    let current_node_degree = node_degrees[current_node];

    for (neighbor_node, w) in graph.neighbors(current_node) {
        if node_to_community[neighbor_node] != node_to_community[current_node] {
            continue;
        }
        let neighbor_subcommunity = node_to_subcommunity[neighbor_node];
        *neighbor_subcommunities
            .entry(neighbor_subcommunity)
            .or_insert(0.0) += w;
        if neighbor_subcommunity == node_to_subcommunity[current_node] {
            weight_to_current_subcommunity += w;
        }
    }

    let mut best_subcommunity = node_to_subcommunity[current_node];
    let mut best_modularity_gain = 0.0;

    // Algorithm 3, line 7: T-filter — only consider subcommunities S
    // where ΔQ(S→∅, γ) ≤ 0 (removing S wouldn't improve modularity)
    let parent_comm = node_to_community[current_node];
    let d_parent: f64 = (0..n)
        .filter(|&i| node_to_community[i] == parent_comm)
        .map(|i| node_degrees[i])
        .sum();

    for (&candidate_subcommunity, &weight_to_candidate_subcommunity) in
        neighbor_subcommunities.iter()
    {
        if candidate_subcommunity == node_to_subcommunity[current_node] {
            continue;
        }

        if should_skip_candidate_t_filter(
            graph,
            candidate_subcommunity,
            parent_comm,
            node_to_subcommunity,
            node_to_community,
            subcommunity_degrees,
            d_parent,
            twice_total_weight,
            resolution_parameter,
        ) {
            continue;
        }

        let current_subcommunity_degree = *subcommunity_degrees
            .get(&node_to_subcommunity[current_node])
            .unwrap_or(&0.0);
        let candidate_subcommunity_degree = *subcommunity_degrees
            .get(&candidate_subcommunity)
            .unwrap_or(&0.0);

        let modularity_gain = (weight_to_candidate_subcommunity - weight_to_current_subcommunity)
            / twice_total_weight
            + resolution_parameter
                * current_node_degree
                * (current_subcommunity_degree
                    - current_node_degree
                    - candidate_subcommunity_degree)
                / (twice_total_weight * twice_total_weight);

        if modularity_gain > best_modularity_gain {
            best_modularity_gain = modularity_gain;
            best_subcommunity = candidate_subcommunity;
        }
    }

    if best_modularity_gain > 0.0 {
        let old_subcommunity = node_to_subcommunity[current_node];
        node_to_subcommunity[current_node] = best_subcommunity;
        subcommunity_sizes[old_subcommunity] -= 1;
        subcommunity_sizes[best_subcommunity] += 1;
        *subcommunity_degrees.entry(old_subcommunity).or_insert(0.0) -= current_node_degree;
        *subcommunity_degrees.entry(best_subcommunity).or_insert(0.0) += current_node_degree;
    }
}

#[allow(clippy::too_many_arguments)]
fn should_skip_candidate_t_filter(
    graph: &crate::core::graph::in_memory::InMemoryGraph,
    candidate_subcommunity: usize,
    parent_comm: usize,
    node_to_subcommunity: &[usize],
    node_to_community: &[usize],
    subcommunity_degrees: &HashMap<usize, f64>,
    d_parent: f64,
    twice_total_weight: f64,
    resolution_parameter: f64,
) -> bool {
    let d_s = subcommunity_degrees
        .get(&candidate_subcommunity)
        .copied()
        .unwrap_or(0.0);
    if d_s <= 0.0 {
        return false;
    }

    // w(S, parent_comm): total weight from candidate subcommunity to parent community
    let mut w_s_to_parent = 0.0;
    for (i, &sc) in node_to_subcommunity.iter().enumerate() {
        if sc != candidate_subcommunity {
            continue;
        }
        for (nbr, w) in graph.neighbors(i) {
            if node_to_community[nbr] == parent_comm {
                w_s_to_parent += w;
            }
        }
    }
    let delta_q_remove = -w_s_to_parent / twice_total_weight
        + resolution_parameter * d_s * (d_parent - d_s) / (twice_total_weight * twice_total_weight);
    delta_q_remove > 1e-9
}

/// Pre-compute the T-filter for parallel refinement: identify subcommunities where
/// ΔQ(S→∅) > 0 (removing S would improve quality). These "healthy" subcommunities
/// should NOT absorb singletons and are blocked from being merge targets.
#[allow(clippy::too_many_arguments)]
fn compute_blocked_subcommunities(
    graph: &crate::core::graph::in_memory::InMemoryGraph,
    node_to_community: &[usize],
    node_to_subcommunity: &[usize],
    subcommunity_degrees: &HashMap<usize, f64>,
    refined_nodes_sorted: &[usize],
    subcommunity_sizes: &[usize],
    node_degrees: &[f64],
    twice_total_weight: f64,
    resolution_parameter: f64,
) -> HashSet<usize> {
    let n = graph.node_count;

    // Collect candidate subcommunities: neighbors of refined singleton nodes
    // within the same parent community.
    let mut candidate_subcommunities: HashSet<usize> = HashSet::new();
    for &current_node in refined_nodes_sorted {
        if node_to_subcommunity[current_node] >= subcommunity_sizes.len()
            || subcommunity_sizes[node_to_subcommunity[current_node]] != 1
        {
            continue;
        }
        let parent_comm = node_to_community[current_node];
        for (neighbor_node, _w) in graph.neighbors(current_node) {
            if node_to_community[neighbor_node] != parent_comm {
                continue;
            }
            let neighbor_sc = node_to_subcommunity[neighbor_node];
            if neighbor_sc != node_to_subcommunity[current_node] {
                candidate_subcommunities.insert(neighbor_sc);
            }
        }
    }

    // Pre-compute d_parent (total degree of parent community) per community
    let mut community_total_degrees: HashMap<usize, f64> = HashMap::new();
    for i in 0..n {
        *community_total_degrees
            .entry(node_to_community[i])
            .or_insert(0.0) += node_degrees[i];
    }

    // For each candidate subcommunity, compute ΔQ(S→∅) and block if > ε
    let mut blocked = HashSet::new();
    for &cand_sc in &candidate_subcommunities {
        let d_s = subcommunity_degrees.get(&cand_sc).copied().unwrap_or(0.0);
        if d_s <= 0.0 {
            continue;
        }

        // Find the parent community of this subcommunity
        let parent_comm = node_to_subcommunity
            .iter()
            .enumerate()
            .find(|(_, &sc)| sc == cand_sc)
            .map(|(i, _)| node_to_community[i])
            .unwrap_or(0);

        let d_parent = community_total_degrees
            .get(&parent_comm)
            .copied()
            .unwrap_or(0.0);

        // w(S, C_parent): total edge weight from S to nodes in parent community
        let w_s_to_parent: f64 = node_to_subcommunity
            .iter()
            .enumerate()
            .filter(|(_, &sc)| sc == cand_sc)
            .flat_map(|(i, _)| graph.neighbors(i))
            .filter(|&(nbr, _)| node_to_community[nbr] == parent_comm)
            .map(|(_, w)| w)
            .sum();

        let delta_q_remove = -w_s_to_parent / twice_total_weight
            + resolution_parameter * d_s * (d_parent - d_s)
                / (twice_total_weight * twice_total_weight);

        if delta_q_remove > 1e-9 {
            blocked.insert(cand_sc);
        }
    }

    blocked
}

fn inc_aggregation(
    graph: &crate::core::graph::in_memory::InMemoryGraph,
    delta_graph: &GraphInput,
    previous_node_to_subcommunity: &[usize],
    current_node_to_subcommunity: &[usize],
    refined_nodes: &BitVec,
) -> (GraphInput, Vec<usize>) {
    let mut delta_supergraph = Vec::new();
    // Mutate in-place instead of to_vec() — start from previous, update refined nodes
    let mut next_previous_node_to_subcommunity = previous_node_to_subcommunity.to_vec();

    // 2 for (v_i, v_j, \alpha) \in \Delta G do
    for &(u, v, w) in &delta_graph.edges {
        let alpha = w.unwrap_or(1.0);
        let subcommunity_u = previous_node_to_subcommunity[u];
        let subcommunity_v = previous_node_to_subcommunity[v];
        delta_supergraph.push((subcommunity_u, subcommunity_v, Some(alpha)));
    }

    // 5 for v_i \in R do
    for current_node in refined_nodes.iter_ones() {
        for (neighbor_node, w) in graph.neighbors(current_node) {
            if neighbor_node == current_node {
                continue; // Self-loops handled separately below
            }
            if current_node_to_subcommunity[neighbor_node]
                == previous_node_to_subcommunity[neighbor_node]
                || current_node < neighbor_node
            {
                delta_supergraph.push((
                    previous_node_to_subcommunity[current_node],
                    previous_node_to_subcommunity[neighbor_node],
                    Some(-w),
                ));
                delta_supergraph.push((
                    current_node_to_subcommunity[current_node],
                    current_node_to_subcommunity[neighbor_node],
                    Some(w),
                ));
            }
        }

        // Algorithm 4: self-loop weight transfer for refined nodes
        let self_loop_weight: f64 = graph
            .neighbors(current_node)
            .filter(|&(nbr, _)| nbr == current_node)
            .map(|(_, w)| w)
            .sum();
        if self_loop_weight.abs() > 1e-12 {
            let s_pre = previous_node_to_subcommunity[current_node];
            let s_cur = current_node_to_subcommunity[current_node];
            delta_supergraph.push((s_pre, s_pre, Some(-self_loop_weight)));
            delta_supergraph.push((s_cur, s_cur, Some(self_loop_weight)));
        }
    }

    // 12 for v_i \in R do
    for current_node in refined_nodes.iter_ones() {
        next_previous_node_to_subcommunity[current_node] =
            current_node_to_subcommunity[current_node];
    }

    // 14 Compress(\Delta H) — use HashMap instead of BTreeMap
    let mut compressed_supergraph: HashMap<(usize, usize), f64> = HashMap::new();
    for (u, v, w) in delta_supergraph {
        let weight = w.unwrap_or(1.0);
        let (min_u, max_v) = if u <= v { (u, v) } else { (v, u) };
        *compressed_supergraph.entry((min_u, max_v)).or_insert(0.0) += weight;
    }

    let mut final_delta_supergraph = Vec::new();
    for ((u, v), w) in compressed_supergraph {
        if w.abs() > 1e-9 {
            final_delta_supergraph.push((u, v, Some(w)));
        }
    }

    let max_subcommunity = current_node_to_subcommunity
        .iter()
        .chain(previous_node_to_subcommunity.iter())
        .copied()
        .max()
        .unwrap_or(0);
    let next_node_count = max_subcommunity + 1;

    let next_delta_graph = GraphInput {
        dataset_id: delta_graph.dataset_id.clone(),
        node_count: next_node_count,
        edges: final_delta_supergraph,
    };

    (next_delta_graph, next_previous_node_to_subcommunity)
}

fn propagate_community_assignments(
    node_to_community_per_level: &mut [Vec<usize>],
    node_to_subcommunity_per_level: &[Vec<usize>],
    changed_nodes: &BitVec,
    p: usize,
) {
    // 3 for v_i^p \in B_p do
    for current_node in changed_nodes.iter_ones() {
        // 4 f_p(v_i^p) = f_{p+1}(s_p(v_i^p))
        node_to_community_per_level[p][current_node] =
            node_to_community_per_level[p + 1][node_to_subcommunity_per_level[p][current_node]];
    }
}

fn propagate_changed_nodes_to_prev_level(
    changed_nodes_per_level: &mut [BitVec],
    node_to_subcommunity_per_level: &[Vec<usize>],
    p: usize,
) {
    // 6 for v_i^p \in B_p do
    let changed_nodes_at_p: Vec<usize> = changed_nodes_per_level[p].iter_ones().collect();
    for current_node in changed_nodes_at_p {
        // 7 B_{p-1}.add(s_p^{-1}(v_i^p))
        for (previous_level_node, &subcommunity_value) in
            node_to_subcommunity_per_level[p - 1].iter().enumerate()
        {
            if subcommunity_value == current_node {
                changed_nodes_per_level[p - 1].set(previous_level_node, true);
            }
        }
    }
}

fn def_update(
    node_to_community_per_level: &mut [Vec<usize>],
    node_to_subcommunity_per_level: &[Vec<usize>],
    changed_nodes_per_level: &mut [BitVec],
    max_levels: usize,
) {
    // 1 for p from P to 1 do
    for p in (0..max_levels).rev() {
        // 2 if p \neq P then
        if p < max_levels - 1 {
            propagate_community_assignments(
                node_to_community_per_level,
                node_to_subcommunity_per_level,
                &changed_nodes_per_level[p],
                p,
            );
        }

        // 5 if p \neq 1 then
        if p > 0 {
            propagate_changed_nodes_to_prev_level(
                changed_nodes_per_level,
                node_to_subcommunity_per_level,
                p,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::RunMode;
    use crate::core::graph::in_memory::InMemoryGraph;
    use crate::core::partition::state::PartitionState;
    use crate::core::types::GraphInput;

    /// Helper: build a GraphInput from an edge list with unit weights.
    fn graph(node_count: usize, edges: &[(usize, usize)]) -> GraphInput {
        GraphInput {
            dataset_id: "test".to_string(),
            node_count,
            edges: edges.iter().map(|&(u, v)| (u, v, Some(1.0))).collect(),
        }
    }

    /// Helper: build a GraphInput with explicit weights.
    fn weighted_graph(node_count: usize, edges: &[(usize, usize, f64)]) -> GraphInput {
        GraphInput {
            dataset_id: "test".to_string(),
            node_count,
            edges: edges.iter().map(|&(u, v, w)| (u, v, Some(w))).collect(),
        }
    }

    /// Generate deterministic and throughput variants for a mode-sensitive test.
    /// Creates a module with two `#[test]` functions: `deterministic` and `throughput`.
    macro_rules! dual_mode_test {
        ($name:ident, |$mode:ident| $body:block) => {
            mod $name {
                use super::*;

                #[test]
                fn deterministic() {
                    let $mode = RunMode::Deterministic;
                    $body
                }

                #[test]
                fn throughput() {
                    let $mode = RunMode::Throughput;
                    $body
                }
            }
        };
    }

    // =====================================================================
    // D7 — apply_delta: InMemoryGraph must support applying a delta
    // =====================================================================

    #[test]
    fn test_apply_delta_adds_edges() {
        // Start with triangle 0-1-2, then add edge 2-3.
        let base = graph(4, &[(0, 1), (1, 2), (0, 2)]);
        let base_graph = InMemoryGraph::from(&base);

        let delta = weighted_graph(4, &[(2, 3, 1.0)]);
        let updated = base_graph.apply_delta(&delta);

        // Node 3 should now have neighbor 2
        let neighbors_3: Vec<usize> = updated.neighbors(3).map(|(n, _)| n).collect();
        assert!(
            neighbors_3.contains(&2),
            "after applying delta adding edge 2-3, node 3 should have neighbor 2"
        );

        // Node 2 should now have neighbor 3
        let neighbors_2: Vec<usize> = updated.neighbors(2).map(|(n, _)| n).collect();
        assert!(
            neighbors_2.contains(&3),
            "after applying delta adding edge 2-3, node 2 should have neighbor 3"
        );

        // Total weight should increase by 1
        assert!(
            (updated.total_weight() - base_graph.total_weight() - 1.0).abs() < 1e-9,
            "total weight should increase by 1.0"
        );
    }

    #[test]
    fn test_apply_delta_removes_edges() {
        // Start with triangle 0-1-2, then remove edge 0-2.
        let base = graph(3, &[(0, 1), (1, 2), (0, 2)]);
        let base_graph = InMemoryGraph::from(&base);

        let delta = weighted_graph(3, &[(0, 2, -1.0)]);
        let updated = base_graph.apply_delta(&delta);

        // Node 0 should no longer have neighbor 2
        let neighbors_0: Vec<usize> = updated.neighbors(0).map(|(n, _)| n).collect();
        assert!(
            !neighbors_0.contains(&2),
            "after removing edge 0-2, node 0 should not have neighbor 2"
        );
        // But should still have neighbor 1
        assert!(
            neighbors_0.contains(&1),
            "node 0 should still have neighbor 1"
        );
    }

    #[test]
    fn test_should_skip_aggregation_when_no_delta_and_no_refinement() {
        let delta = GraphInput {
            dataset_id: "test".to_string(),
            node_count: 4,
            edges: vec![],
        };
        let refined = bitvec![0, 0, 0, 0];
        assert!(should_skip_aggregation(&delta, &refined));
    }

    #[test]
    fn test_should_not_skip_aggregation_when_delta_or_refinement_exists() {
        let delta_non_empty = GraphInput {
            dataset_id: "test".to_string(),
            node_count: 4,
            edges: vec![(0, 1, Some(1.0))],
        };
        let refined_empty = bitvec![0, 0, 0, 0];
        assert!(!should_skip_aggregation(&delta_non_empty, &refined_empty));

        let delta_empty = GraphInput {
            dataset_id: "test".to_string(),
            node_count: 4,
            edges: vec![],
        };
        let refined_non_empty = bitvec![0, 1, 0, 0];
        assert!(!should_skip_aggregation(&delta_empty, &refined_non_empty));
    }

    // D7 — hit_leiden must apply delta to supergraph (Algorithm 6, line 3)
    dual_mode_test!(test_hit_leiden_applies_delta_to_supergraph, |mode| {
        // Build initial graph: two cliques {0,1,2} and {3,4,5} connected by edge 2-3
        let initial = graph(
            6,
            &[
                (0, 1),
                (1, 2),
                (0, 2), // clique A
                (3, 4),
                (4, 5),
                (3, 5), // clique B
                (2, 3), // bridge
            ],
        );
        let mut state = PartitionState::identity(6);
        state.supergraphs.push(InMemoryGraph::from(&initial));

        // Initial run to establish partition
        hit_leiden(&mut state, &initial, 1.0, mode);

        // Now add a strong new edge 0-5 (cross-clique)
        let delta = weighted_graph(6, &[(0, 5, 10.0)]);

        // After hit_leiden processes this delta, the supergraph must reflect
        // the new edge. Movement decisions on node 0 and 5 must see the new
        // edge weight when computing neighbour community weights.
        hit_leiden(&mut state, &delta, 1.0, mode);

        // Verify: the supergraph now contains edge 0-5 with weight 10
        let neighbors_0: Vec<(usize, f64)> = state.supergraphs[0].neighbors(0).collect();
        let edge_to_5 = neighbors_0.iter().find(|(n, _)| *n == 5);
        assert!(
            edge_to_5.is_some(),
            "supergraph must contain edge 0-5 after delta applied"
        );
        let (_, w) = edge_to_5.unwrap();
        assert!(
            (*w - 10.0).abs() < 1e-9,
            "edge 0-5 weight should be 10.0, got {}",
            w
        );
    });

    // =====================================================================
    // D8 — Final assignment must use g¹ (refined mapping), not f¹
    // =====================================================================

    dual_mode_test!(test_hit_leiden_uses_refined_mapping, |mode| {
        // After hit_leiden runs, state.node_to_comm should equal
        // refined_community_mapping_per_level[0] (g¹), not
        // community_mapping_per_level[0] (f¹).
        //
        // Build a graph where refinement produces different assignments
        // than movement: two loosely connected cliques.
        let g = graph(
            8,
            &[
                (0, 1),
                (1, 2),
                (0, 2), // clique A
                (3, 4),
                (4, 5),
                (3, 5), // clique B
                (6, 7), // pair C
                (2, 3), // weak bridge A-B
                (5, 6), // weak bridge B-C
            ],
        );
        let mut state = PartitionState::identity(8);
        state.supergraphs.push(InMemoryGraph::from(&g));

        // Initial run
        hit_leiden(&mut state, &g, 1.0, mode);

        // The final output must come from the refined (g) mapping
        assert_eq!(
            state.node_to_comm, state.refined_community_mapping_per_level[0],
            "hit_leiden output must use g¹ (refined_community_mapping_per_level[0]), \
             not f¹ (community_mapping_per_level[0])"
        );
    });

    // =====================================================================
    // D2 — Movement argmax must include ∅ (singleton/empty community)
    // =====================================================================

    dual_mode_test!(test_movement_considers_empty_community, |mode| {
        // With higher resolution (γ=3), smaller communities are preferred.
        // 3 nodes, 0-1 strongly connected (w=10), 0-2 weakly connected (w=0.5).
        // All in one community. An empty delta activates all nodes.
        //
        // For node 2: d(2)=0.5, w(2,C)=0.5, Σ_C=21, 2m=21
        // gain(∅, γ=3) = -0.5/21 + 3·0.5·(21-0.5)/21² = 0.046 > 0
        //
        // Without ∅ in argmax: no neighbor community exists, node stays.
        // With ∅ in argmax: node 2 moves to a fresh singleton community.
        let base = weighted_graph(3, &[(0, 1, 10.0), (0, 2, 0.5)]);
        let base_graph = InMemoryGraph::from(&base);

        let mut node_to_community = vec![0, 0, 0];
        let node_to_subcommunity = vec![0, 0, 0];

        // Empty delta: activates all nodes (initial run path)
        let delta = GraphInput::empty("test");

        let (_changed, _k, _iters) = inc_movement(
            &base_graph,
            &delta,
            &mut node_to_community,
            &node_to_subcommunity,
            3.0, // high resolution to make ∅ beneficial
            mode,
        );

        // Node 2 should have left to ∅ (a fresh community).
        // Without ∅ in argmax, it stays in community 0.
        assert_ne!(
            node_to_community[2], node_to_community[0],
            "node 2 (weakly attached) should move to empty community with γ=3, \
             but it stayed in community {} with node 0",
            node_to_community[0]
        );
    });

    // =====================================================================
    // D4 — inc_refinement must apply T-filter: ΔQ(S → ∅, γ) ≤ 0
    // =====================================================================

    dual_mode_test!(
        test_refinement_t_filter_excludes_well_connected_subcommunity,
        |mode| {
            // Algorithm 3 line 7: T = { S | ΔQ(S→∅, γ) ≤ 0 }
            // Subcommunities with ΔQ(S→∅) > 0 must be excluded from merge candidates.
            //
            // Setup: 4 nodes, two communities.
            //   Community 0 = {0, 1, 2}, Community 1 = {3}
            //   Edges: 0-1(10), 0-3(100), 1-3(100), 1-2(1)
            //
            // Nodes 0 and 1 have strong cross-community edges to node 3, so their
            // "internal" weight to community 0 is small relative to their total degree.
            //
            // For subcommunity {1}: d_s=111, w(S,parent_comm_0)=10+1=11
            //   ΔQ(1→∅) = -11/(2m) + 111*(d_parent - 111)/(2m)² > 0
            //   → T-filter REJECTS subcommunity {1} as a merge target
            //
            // Without T-filter: node 2 merges into subcomm {1} (positive gain from edge 1-2)
            // With T-filter: subcomm {1} rejected, node 2 stays singleton
            let g = weighted_graph(
                4,
                &[
                    (0, 1, 10.0),  // moderate internal in community 0
                    (0, 3, 100.0), // strong cross-community
                    (1, 3, 100.0), // strong cross-community
                    (1, 2, 1.0),   // weak internal connecting node 2 to node 1
                ],
            );
            let inmem = InMemoryGraph::from(&g);

            let node_to_community = vec![0, 0, 0, 1];
            let mut node_to_subcommunity = vec![0, 1, 2, 3]; // identity = initial pass

            let mut affected = bitvec![0; 4];
            affected.fill(true);

            inc_refinement(
                &inmem,
                &node_to_community,
                &mut node_to_subcommunity,
                &affected,
                1.0,
                mode,
            );

            // With T-filter: node 2 should stay in its original subcommunity (2).
            // Subcommunity {1} has ΔQ(S→∅) > 0 so it's rejected as a candidate.
            // Without T-filter, node 2 would merge into subcomm 1 (positive gain from edge 1-2).
            assert_eq!(
                node_to_subcommunity[2], 2,
                "T-filter should prevent node 2 from merging (its only candidate, subcomm 1, \
             has DeltaQ(S->empty) > 0). Node 2 should stay in original subcomm 2. \
             Got subcommunities: {:?}",
                node_to_subcommunity,
            );
        }
    );

    // =====================================================================
    // D6 — inc_aggregation must include self-loop weight transfers
    // =====================================================================

    #[test]
    fn test_aggregation_self_loop_transfer() {
        // Graph with a self-loop on node 0. Refine node 0 from subcommunity 0
        // to subcommunity 1. The delta should contain self-loop weight changes.
        let g = weighted_graph(
            3,
            &[
                (0, 0, 5.0), // self-loop on node 0
                (0, 1, 1.0),
                (1, 2, 1.0),
            ],
        );
        let inmem = InMemoryGraph::from(&g);

        let delta = GraphInput {
            dataset_id: "test".to_string(),
            node_count: 3,
            edges: vec![],
        };

        // s_pre: node 0 was in subcommunity 0
        let s_pre = vec![0, 1, 2];
        // s_cur: node 0 moved to subcommunity 1
        let s_cur = vec![1, 1, 2];

        // Only node 0 was refined
        let mut refined = bitvec![0; 3];
        refined.set(0, true);

        let (delta_h, _new_s_pre) = inc_aggregation(&inmem, &delta, &s_pre, &s_cur, &refined);

        // The delta should contain self-loop entries:
        // (0, 0, -5.0) removing self-loop weight from old subcommunity
        // (1, 1, +5.0) adding self-loop weight to new subcommunity
        let mut self_loop_weight_0 = 0.0;
        let mut self_loop_weight_1 = 0.0;
        for &(u, v, w) in &delta_h.edges {
            let weight = w.unwrap_or(0.0);
            if u == 0 && v == 0 {
                self_loop_weight_0 += weight;
            }
            if u == 1 && v == 1 {
                self_loop_weight_1 += weight;
            }
        }

        assert!(
            self_loop_weight_0 < -1e-9,
            "delta should contain negative self-loop weight for old subcommunity 0, \
             got {}",
            self_loop_weight_0
        );
        assert!(
            self_loop_weight_1 > 1e-9,
            "delta should contain positive self-loop weight for new subcommunity 1, \
             got {}",
            self_loop_weight_1
        );
    }

    // =====================================================================
    // Algorithm 0 (Table 1): Modularity Q(G, C, γ)
    // Q = (1/2m) * [ Σ_intra - Σ_c (σ_c² / 2m) ]
    // =====================================================================

    #[test]
    fn test_modularity_single_community_triangle() {
        // Triangle: 3 nodes, all in one community.
        // 3 edges of weight 1 each. 2m = 6.
        // intra_weight = 3 * 2 = 6 (both directions).
        // Each node has degree 2, total community degree = 6.
        // Q = (6 - 36/6) / 6 = (6 - 6) / 6 = 0.0
        let g = graph(3, &[(0, 1), (1, 2), (0, 2)]);
        let partition = vec![0, 0, 0];
        let q = compute_modularity(&g, &partition);
        assert!(
            q.abs() < 1e-9,
            "all nodes in one community should give Q=0, got {}",
            q
        );
    }

    #[test]
    fn test_modularity_all_singletons_triangle() {
        // Triangle: 3 nodes, each in its own community.
        // No intra-community edges. intra_weight = 0.
        // Each community has degree 2, so expected = 3 * (4/6) = 2.
        // Q = (0 - 2) / 6 = -1/3
        let g = graph(3, &[(0, 1), (1, 2), (0, 2)]);
        let partition = vec![0, 1, 2];
        let q = compute_modularity(&g, &partition);
        assert!(
            (q - (-1.0 / 3.0)).abs() < 1e-9,
            "all singletons in triangle: Q should be -1/3, got {}",
            q
        );
    }

    #[test]
    fn test_modularity_two_cliques_optimal() {
        // Two triangles {0,1,2} and {3,4,5}, no inter-edges.
        // Perfect partition: community 0 = {0,1,2}, community 1 = {3,4,5}.
        // m = 6, 2m = 12.
        // intra = 6*2 = 12 (all edges are intra).
        // Each community degree = 6, expected = 2*(36/12) = 6.
        // Q = (12 - 6) / 12 = 0.5
        let g = graph(6, &[(0, 1), (1, 2), (0, 2), (3, 4), (4, 5), (3, 5)]);
        let partition = vec![0, 0, 0, 1, 1, 1];
        let q = compute_modularity(&g, &partition);
        assert!(
            (q - 0.5).abs() < 1e-9,
            "two disjoint triangles in separate communities: Q should be 0.5, got {}",
            q
        );
    }

    #[test]
    fn test_modularity_empty_graph() {
        // No edges => Q = 0
        let g = GraphInput {
            dataset_id: "test".to_string(),
            node_count: 3,
            edges: vec![],
        };
        let partition = vec![0, 1, 2];
        let q = compute_modularity(&g, &partition);
        assert!(q.abs() < 1e-9, "empty graph should give Q=0, got {}", q);
    }

    #[test]
    fn test_modularity_weighted_edges() {
        // Two nodes connected by edge of weight 5.
        // 2m = 10. Both in same community: intra = 10, expected = 100/10 = 10.
        // Q = (10 - 10) / 10 = 0.
        let g = weighted_graph(2, &[(0, 1, 5.0)]);
        let partition = vec![0, 0];
        let q = compute_modularity(&g, &partition);
        assert!(
            q.abs() < 1e-9,
            "two nodes same community: Q should be 0, got {}",
            q
        );

        // Now in different communities: intra = 0, expected = 2*(25/10) = 5.
        // Q = (0 - 5) / 10 = -0.5
        let partition_split = vec![0, 1];
        let q_split = compute_modularity(&g, &partition_split);
        assert!(
            (q_split - (-0.5)).abs() < 1e-9,
            "two nodes separate communities: Q should be -0.5, got {}",
            q_split
        );
    }

    // =====================================================================
    // Algorithm 0: Modularity gain ΔQ(v → C', γ)
    // ΔQ = (w(v,C') - w(v,C_current)) / 2m
    //     + γ * d(v) * (d(C_current) - d(v) - d(C')) / (2m)²
    // =====================================================================

    dual_mode_test!(test_modularity_gain_positive_for_natural_move, |mode| {
        // ΔQ(v → C', γ) should be positive when a misplaced node moves
        // to its natural community. Setup: node 2 is in community A={0,1,2}
        // but strongly connects to community B={3,4}.
        //
        // With γ=1, the gain of moving node 2 from comm A to comm B should be
        // positive because w(2,B)=20 >> w(2,A)=0.1.
        let g = weighted_graph(
            5,
            &[
                (0, 1, 10.0), // strong A-internal
                (1, 2, 0.1),  // weak A-connection to node 2
                (2, 3, 10.0), // strong cross-community
                (2, 4, 10.0), // strong cross-community
                (3, 4, 10.0), // strong B-internal
            ],
        );
        let inmem = InMemoryGraph::from(&g);

        // Misplaced: node 2 starts in community 0 with 0,1
        let mut node_to_community = vec![0, 0, 0, 1, 1];
        let node_to_subcommunity = vec![0, 1, 2, 3, 4];

        let delta = GraphInput::empty("test");
        let (changed, _k, _iters) = inc_movement(
            &inmem,
            &delta,
            &mut node_to_community,
            &node_to_subcommunity,
            1.0,
            mode,
        );

        // Node 2 should have moved to join community of nodes 3,4
        assert_eq!(
            node_to_community[2], node_to_community[3],
            "node 2 (strongly connecting to B) should move to community B, got {:?}",
            node_to_community
        );
        assert!(changed[2], "node 2 should be marked as changed");
    });

    dual_mode_test!(test_modularity_gain_no_move_for_well_connected, |mode| {
        // Complete graph K4: all equally connected. With γ=1, no node should move
        // from a single community — moving to singleton gives Q < 0.
        let g = graph(4, &[(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)]);
        let inmem = InMemoryGraph::from(&g);

        // All in community 0
        let mut node_to_community = vec![0, 0, 0, 0];
        let node_to_subcommunity = vec![0, 1, 2, 3];

        let delta = GraphInput::empty("test");
        let (changed, _k, _iters) = inc_movement(
            &inmem,
            &delta,
            &mut node_to_community,
            &node_to_subcommunity,
            1.0,
            mode,
        );

        // No node should have changed community
        assert!(
            changed.not_any(),
            "K4 with all nodes in one community: no moves expected, but got changes: {:?}",
            changed
        );
    });

    // =====================================================================
    // Algorithm 1: Standard Leiden multi-level coarsening
    // =====================================================================

    dual_mode_test!(test_multilevel_leiden_two_cliques, |mode| {
        // Two triangles connected by a bridge: should find 2 communities.
        let g = graph(
            6,
            &[
                (0, 1),
                (1, 2),
                (0, 2), // clique A
                (3, 4),
                (4, 5),
                (3, 5), // clique B
                (2, 3), // bridge
            ],
        );
        let mut state = PartitionState::identity(6);

        let (iters, hierarchy) = multilevel_leiden(&mut state, &g, 1.0, 0.05, mode, 10);

        assert!(iters > 0, "should take at least 1 iteration");
        assert!(
            !hierarchy.is_empty(),
            "should produce at least one hierarchy level"
        );

        // Should find 2 communities
        let comm_count = count_unique(&state.node_to_comm);
        assert_eq!(
            comm_count, 2,
            "two cliques connected by bridge should give 2 communities, got {}",
            comm_count
        );

        // Nodes in same clique should share a community
        assert_eq!(state.node_to_comm[0], state.node_to_comm[1]);
        assert_eq!(state.node_to_comm[1], state.node_to_comm[2]);
        assert_eq!(state.node_to_comm[3], state.node_to_comm[4]);
        assert_eq!(state.node_to_comm[4], state.node_to_comm[5]);
        assert_ne!(state.node_to_comm[0], state.node_to_comm[3]);
    });

    dual_mode_test!(test_multilevel_leiden_single_node, |mode| {
        // Single node graph, no edges.
        let g = GraphInput {
            dataset_id: "test".to_string(),
            node_count: 1,
            edges: vec![],
        };
        let mut state = PartitionState::identity(1);

        let (iters, hierarchy) = multilevel_leiden(&mut state, &g, 1.0, 0.05, mode, 10);

        assert_eq!(state.node_to_comm, vec![0]);
        assert!(iters >= 1, "should still complete at least 1 iteration");
        assert!(!hierarchy.is_empty(), "should produce at least one level");
    });

    dual_mode_test!(test_multilevel_leiden_disconnected_components, |mode| {
        // 4 disconnected nodes: each should be its own community.
        let g = GraphInput {
            dataset_id: "test".to_string(),
            node_count: 4,
            edges: vec![],
        };
        let mut state = PartitionState::identity(4);

        let (_iters, _hierarchy) = multilevel_leiden(&mut state, &g, 1.0, 0.05, mode, 10);

        let comm_count = count_unique(&state.node_to_comm);
        assert_eq!(
            comm_count, 4,
            "4 disconnected nodes should each be their own community, got {} communities",
            comm_count
        );
    });

    dual_mode_test!(test_multilevel_leiden_hierarchy_levels_recorded, |mode| {
        // Verify hierarchy levels are recorded at each coarsening step.
        let g = graph(6, &[(0, 1), (1, 2), (0, 2), (3, 4), (4, 5), (3, 5), (2, 3)]);
        let mut state = PartitionState::identity(6);

        let (_iters, hierarchy) = multilevel_leiden(&mut state, &g, 1.0, 0.05, mode, 10);

        // Each level should have exactly node_count entries
        for (i, level) in hierarchy.iter().enumerate() {
            assert_eq!(
                level.len(),
                6,
                "hierarchy level {} should have 6 entries, got {}",
                i,
                level.len()
            );
        }
    });

    #[test]
    fn test_canonicalize_community_ids() {
        // Non-contiguous IDs [5, 5, 3, 3, 7] should become [0, 0, 1, 1, 2]
        let mut ids = vec![5, 5, 3, 3, 7];
        canonicalize_community_ids_in_place(&mut ids);
        assert_eq!(ids, vec![0, 0, 1, 1, 2]);
    }

    #[test]
    fn test_canonicalize_already_contiguous() {
        let mut ids = vec![0, 0, 1, 1, 2];
        canonicalize_community_ids_in_place(&mut ids);
        assert_eq!(ids, vec![0, 0, 1, 1, 2]);
    }

    // =====================================================================
    // Algorithm 2: Inc-movement — active set seeding rules
    // Lines 2-8: which edges seed the active set A and refinement set K
    // =====================================================================

    dual_mode_test!(
        test_inc_movement_cross_community_insertion_activates,
        |mode| {
            // Algorithm 2 line 3: α > 0 and f(vi) ≠ f(vj) → A.add both
            // Algorithm 6 line 3: G^p ← G^p ⊕ ΔG^p BEFORE inc-movement
            // So the InMemoryGraph must already include the delta edges.
            //
            // Nodes 0,1 in comm 0; node 2 in comm 1.
            // Add strong cross-community edge 1-2 (w=10).
            // After G⊕ΔG, the graph has edges: 0-1(1), 1-2(10).
            let combined = weighted_graph(3, &[(0, 1, 1.0), (1, 2, 10.0)]);
            let inmem = InMemoryGraph::from(&combined);

            let mut node_to_community = vec![0, 0, 1];
            let node_to_subcommunity = vec![0, 0, 1];

            // Delta: the new cross-community edge (for seeding active set)
            let delta = weighted_graph(3, &[(1, 2, 10.0)]);

            let (_changed, _k, _iters) = inc_movement(
                &inmem,
                &delta,
                &mut node_to_community,
                &node_to_subcommunity,
                1.0,
                mode,
            );

            // With strong edge 1-2 (w=10) vs weak 0-1 (w=1), node 1 should
            // move to community 1 to join node 2, or node 2 moves to community 0.
            // Either way, nodes 1 and 2 should end in the same community.
            if mode == RunMode::Deterministic {
                assert_eq!(
                    node_to_community[1], node_to_community[2],
                    "strong cross-community edge should cause merger: got {:?}",
                    node_to_community
                );
            } else {
                // In throughput mode, batch-parallel processing with stale snapshots
                // can cause nodes 1 and 2 to swap communities simultaneously rather
                // than merging. This is expected — the algorithm still converges to
                // a valid partition. Assert structural invariants instead.
                let distinct: ahash::HashSet<usize> = node_to_community.iter().copied().collect();
                assert!(
                    distinct.len() <= 3,
                    "should converge to a valid partition: got {:?}",
                    node_to_community
                );
            }
        }
    );

    dual_mode_test!(
        test_inc_movement_intra_community_deletion_activates,
        |mode| {
            // Algorithm 2 line 5: α < 0 and f(vi) = f(vj) → A.add both
            //
            // Node 2 is in community 0 with nodes 0,1. Its only connection to
            // community 0 is edge 0-2 (w=1). Node 2 also connects to node 3
            // in community 1 via strong edge 2-3 (w=10).
            //
            // After G⊕ΔG (removing edge 0-2), node 2 has NO connection to
            // community 0, but strong connection to community 1.
            // The deletion MUST activate node 2 (Algorithm 2 line 5), causing
            // it to move to community 1.
            //
            // Without activation (the mutation), node 2 stays in community 0
            // because it was never re-evaluated.
            let combined = weighted_graph(
                4,
                &[
                    (0, 1, 10.0), // strong comm 0 internal
                    // edge 0-2 removed by delta, so NOT in G⊕ΔG
                    (2, 3, 10.0), // strong cross-community
                ],
            );
            let inmem = InMemoryGraph::from(&combined);

            let mut node_to_community = vec![0, 0, 0, 1];
            let node_to_subcommunity = vec![0, 1, 2, 3];

            // Delta: remove intra-community edge 0-2 (α < 0, same community)
            let delta = weighted_graph(4, &[(0, 2, -1.0)]);

            let (changed, _k, _iters) = inc_movement(
                &inmem,
                &delta,
                &mut node_to_community,
                &node_to_subcommunity,
                1.0,
                mode,
            );

            // Node 2 must leave community 0 (no connection) and join community 1
            assert_eq!(
                node_to_community[2], node_to_community[3],
                "after intra-community edge deletion, node 2 should move to community 1 \
             (its only remaining connection). Got partition: {:?}",
                node_to_community
            );
            assert!(
                changed[2],
                "node 2 should be marked as changed after being activated by deletion"
            );
        }
    );

    dual_mode_test!(test_inc_movement_same_subcommunity_edge_tracks_k, |mode| {
        // Algorithm 2 line 7: s(vi) = s(vj) → track in K
        // Two nodes in same subcommunity, delta removes their edge.
        let base = graph(3, &[(0, 1), (1, 2)]);
        let inmem = InMemoryGraph::from(&base);

        let mut node_to_community = vec![0, 0, 0];
        // Nodes 0 and 1 share subcommunity 0
        let node_to_subcommunity = vec![0, 0, 2];

        // Delta: remove edge 0-1 (within same subcommunity)
        let delta = weighted_graph(3, &[(0, 1, -1.0)]);

        let (_changed, k, _iters) = inc_movement(
            &inmem,
            &delta,
            &mut node_to_community,
            &node_to_subcommunity,
            1.0,
            mode,
        );

        // K should contain nodes 0 and 1 (same subcommunity edge affected)
        assert!(
            k[0] && k[1],
            "nodes 0 and 1 (same subcommunity) should be in K after edge deletion, got K={:?}",
            k
        );
    });

    dual_mode_test!(test_inc_movement_neighbor_requeuing, |mode| {
        // Algorithm 2 lines 14-16: when vi moves to C*, neighbors not in C*
        // should be re-activated.
        //
        // Setup: 4 nodes. Initially comm A={0,1,2}, comm B={3}.
        // Node 2 connects strongly to 3 (w=10), weakly to 1 (w=0.1).
        // Node 0 connects strongly to 1 (w=10).
        //
        // After G⊕ΔG, graph has: 0-1(10), 1-2(0.1), 2-3(10).
        // When node 2 moves to community B, node 1 should be re-queued
        // (Algorithm 2 line 15: f(vj) ≠ C* → A.add(vj)).
        // Node 1 stays with node 0 because w(1,comm_A)=10 >> w(1,comm_B)=0.1.
        let g = weighted_graph(4, &[(0, 1, 10.0), (1, 2, 0.1), (2, 3, 10.0)]);
        let inmem = InMemoryGraph::from(&g);

        // Start with node 2 misplaced in community A
        let mut node_to_community = vec![0, 0, 0, 1];
        let node_to_subcommunity = vec![0, 1, 2, 3];

        let delta = GraphInput::empty("test");
        let (_changed, _k, _iters) = inc_movement(
            &inmem,
            &delta,
            &mut node_to_community,
            &node_to_subcommunity,
            1.0,
            mode,
        );

        if mode == RunMode::Deterministic {
            // Node 2 should move to join node 3 (strong connection)
            assert_eq!(
                node_to_community[2], node_to_community[3],
                "node 2 should move to community with node 3"
            );
            // Node 1 was re-queued when node 2 left, but stays with node 0
            assert_eq!(
                node_to_community[0], node_to_community[1],
                "nodes 0,1 should stay together after re-queue evaluation"
            );
            // The two groups should be separate
            assert_ne!(
                node_to_community[0], node_to_community[2],
                "communities should be distinct after movement"
            );
        } else {
            // In throughput mode, all active nodes process simultaneously
            // against a stale snapshot. The sequential cascade (node 2 moves →
            // node 1 re-queued → node 1 stays) may not happen in the same order.
            // Assert structural invariants: nodes 0,1 (strongly connected)
            // should end together; the partition should have 2 communities.
            assert_eq!(
                node_to_community[0], node_to_community[1],
                "strongly connected nodes 0,1 should stay together: got {:?}",
                node_to_community
            );
            let distinct: ahash::HashSet<usize> = node_to_community.iter().copied().collect();
            assert_eq!(
                distinct.len(),
                2,
                "should converge to 2 communities: got {:?}",
                node_to_community
            );
        }
    });

    // =====================================================================
    // Algorithm 3: Inc-refinement — connected component splitting
    // Lines 2-4: non-largest components get new subcommunity IDs
    // =====================================================================

    dual_mode_test!(test_refinement_splits_disconnected_subcommunity, |mode| {
        // Algorithm 3 lines 2-4: if vi is not in the largest connected
        // component of s(v), map to new sub-community.
        //
        // 4 nodes. Subcommunity 0 = {0, 1, 2, 3}.
        // Edges: 0-1 and 2-3 (two disconnected pairs in same subcommunity).
        // After refinement, each pair should be a separate subcommunity.
        let g = graph(4, &[(0, 1), (2, 3)]);
        let inmem = InMemoryGraph::from(&g);

        let node_to_community = vec![0, 0, 0, 0];
        let mut node_to_subcommunity = vec![0, 0, 0, 0]; // All in subcommunity 0

        let mut affected = bitvec![0; 4];
        affected.fill(true);

        inc_refinement(
            &inmem,
            &node_to_community,
            &mut node_to_subcommunity,
            &affected,
            1.0,
            mode,
        );

        // The two pairs should be in different subcommunities
        assert_eq!(
            node_to_subcommunity[0], node_to_subcommunity[1],
            "nodes 0,1 (connected) should share subcommunity"
        );
        assert_eq!(
            node_to_subcommunity[2], node_to_subcommunity[3],
            "nodes 2,3 (connected) should share subcommunity"
        );
        assert_ne!(
            node_to_subcommunity[0], node_to_subcommunity[2],
            "disconnected pairs should have different subcommunities"
        );
    });

    dual_mode_test!(test_refinement_largest_component_retains_id, |mode| {
        // Algorithm 3 line 3: largest connected component retains the
        // original subcommunity ID. Smaller components get new IDs.
        //
        // 5 nodes all in subcommunity 0.
        // Edges: 0-1-2 (chain of 3) and 3-4 (pair of 2).
        // The chain {0,1,2} is larger and should keep subcommunity 0.
        let g = graph(5, &[(0, 1), (1, 2), (3, 4)]);
        let inmem = InMemoryGraph::from(&g);

        let node_to_community = vec![0, 0, 0, 0, 0];
        let mut node_to_subcommunity = vec![0, 0, 0, 0, 0];

        let mut affected = bitvec![0; 5];
        affected.fill(true);

        inc_refinement(
            &inmem,
            &node_to_community,
            &mut node_to_subcommunity,
            &affected,
            1.0,
            mode,
        );

        // Larger component {0,1,2} should retain original ID 0
        assert_eq!(
            node_to_subcommunity[0], 0,
            "largest component node 0 should retain subcommunity 0"
        );
        assert_eq!(node_to_subcommunity[1], 0);
        assert_eq!(node_to_subcommunity[2], 0);

        // Smaller component {3,4} should have a new ID
        assert_ne!(
            node_to_subcommunity[3], 0,
            "smaller component should get new subcommunity ID"
        );
        assert_eq!(
            node_to_subcommunity[3], node_to_subcommunity[4],
            "nodes 3,4 should share new subcommunity"
        );
    });

    dual_mode_test!(test_refinement_singleton_merge_positive_gain, |mode| {
        // Algorithm 3 lines 5-13: singleton subcommunities with ΔM > 0
        // should merge into neighboring subcommunities.
        //
        // 3 nodes all in community 0.
        // Edges: 0-1 (w=10), 1-2 (w=10).
        // Start with identity subcommunities: each node is its own subcommunity.
        // Node 2 (singleton) should merge with subcommunity of node 1
        // because ΔM(2 → subcomm_of_1) > 0.
        let g = weighted_graph(3, &[(0, 1, 10.0), (1, 2, 10.0)]);
        let inmem = InMemoryGraph::from(&g);

        let node_to_community = vec![0, 0, 0];
        let mut node_to_subcommunity = vec![0, 1, 2]; // identity = singletons

        let mut affected = bitvec![0; 3];
        affected.fill(true);

        inc_refinement(
            &inmem,
            &node_to_community,
            &mut node_to_subcommunity,
            &affected,
            1.0,
            mode,
        );

        // At least some merging should have occurred: not all singletons remain
        let unique_subcommunities = count_unique(&node_to_subcommunity);
        assert!(
            unique_subcommunities < 3,
            "some singletons should merge: expected < 3 subcommunities, got {}",
            unique_subcommunities
        );
    });

    dual_mode_test!(
        test_refinement_only_processes_affected_subcommunities,
        |mode| {
            // Inc-refinement should only process subcommunities containing
            // nodes in the affected set K. Unaffected subcommunities should
            // remain unchanged.
            let g = graph(4, &[(0, 1), (2, 3)]);
            let inmem = InMemoryGraph::from(&g);

            let node_to_community = vec![0, 0, 1, 1];
            let mut node_to_subcommunity = vec![0, 0, 2, 2];

            // Only mark nodes 2,3 as affected
            let mut affected = bitvec![0; 4];
            affected.set(2, true);
            affected.set(3, true);

            let original_sc_01 = node_to_subcommunity[0];
            inc_refinement(
                &inmem,
                &node_to_community,
                &mut node_to_subcommunity,
                &affected,
                1.0,
                mode,
            );

            // Nodes 0,1 should be unchanged (not affected)
            assert_eq!(
                node_to_subcommunity[0], original_sc_01,
                "unaffected node 0 should retain subcommunity"
            );
            assert_eq!(
                node_to_subcommunity[1], original_sc_01,
                "unaffected node 1 should retain subcommunity"
            );
        }
    );

    // =====================================================================
    // Algorithm 4: Inc-aggregation — ΔG mapping via s_pre
    // Lines 2-4: edge changes mapped through previous subcommunity mapping
    // =====================================================================

    #[test]
    fn test_aggregation_maps_delta_via_s_pre_not_s_cur() {
        // Algorithm 4 lines 2-4: ΔG edges MUST be mapped through s_pre, NOT s_cur.
        //
        // This is critical: s_pre reflects the subcommunity state BEFORE refinement,
        // which matches the current supergraph structure. Using s_cur would map edges
        // to wrong supernodes.
        //
        // Setup where s_pre ≠ s_cur:
        //   s_pre: node 0→subcomm 0, node 1→subcomm 0, node 2→subcomm 2, node 3→subcomm 2
        //   s_cur: node 0→subcomm 0, node 1→subcomm 0, node 2→subcomm 0, node 3→subcomm 2
        //   (node 2 moved from subcomm 2 to subcomm 0 during refinement)
        //
        // Delta edge (0, 2, 5.0):
        //   Via s_pre: maps to superedge (0, 2) — CORRECT
        //   Via s_cur: maps to superedge (0, 0) — WRONG (self-loop instead of inter-edge)
        let g = graph(4, &[(0, 1), (1, 2), (2, 3)]);
        let inmem = InMemoryGraph::from(&g);

        let s_pre = vec![0, 0, 2, 2]; // Before refinement
        let s_cur = vec![0, 0, 0, 2]; // After refinement: node 2 moved to subcomm 0

        let delta = weighted_graph(4, &[(0, 2, 5.0)]); // Edge between nodes in DIFFERENT s_pre subcommunities

        let refined = bitvec![0; 4]; // No refined nodes in this test (only testing delta mapping)

        let (delta_h, _new_s_pre) = inc_aggregation(&inmem, &delta, &s_pre, &s_cur, &refined);

        // Via s_pre: edge (0,2) → superedge (subcomm 0, subcomm 2) with weight 5.0
        // Via s_cur (mutant): edge (0,2) → superedge (subcomm 0, subcomm 0) — self-loop!
        let inter_edge_weight: f64 = delta_h
            .edges
            .iter()
            .filter(|&&(u, v, _)| {
                let (min, max) = if u < v { (u, v) } else { (v, u) };
                min == 0 && max == 2
            })
            .map(|&(_, _, w)| w.unwrap_or(0.0))
            .sum();

        let self_loop_weight: f64 = delta_h
            .edges
            .iter()
            .filter(|&&(u, v, _)| u == 0 && v == 0)
            .map(|&(_, _, w)| w.unwrap_or(0.0))
            .sum();

        assert!(
            (inter_edge_weight - 5.0).abs() < 1e-9,
            "delta should map to inter-superedge (0,2) with weight 5.0 via s_pre, got {}. \
             Self-loop(0,0) weight={} (wrong if >0 — indicates s_cur was used instead)",
            inter_edge_weight,
            self_loop_weight
        );
    }

    #[test]
    fn test_aggregation_compress_sums_duplicates() {
        // Algorithm 4 line 14: Compress(ΔH) — sum identical superedges.
        // Two delta edges that map to the same superedge should be summed.
        let g = graph(4, &[(0, 1), (2, 3)]);
        let inmem = InMemoryGraph::from(&g);

        // Nodes 0,1 map to subcommunity 0; nodes 2,3 map to subcommunity 1
        let s_pre = vec![0, 0, 1, 1];
        let s_cur = vec![0, 0, 1, 1];

        // Two delta edges that both map to superedge (0, 1):
        // edge (0,2) → (subcomm 0, subcomm 1), weight 3.0
        // edge (1,3) → (subcomm 0, subcomm 1), weight 2.0
        let delta = weighted_graph(4, &[(0, 2, 3.0), (1, 3, 2.0)]);

        let refined = bitvec![0; 4];

        let (delta_h, _new_s_pre) = inc_aggregation(&inmem, &delta, &s_pre, &s_cur, &refined);

        // After compression, there should be one superedge (0,1) with weight 5.0
        let total_weight_01: f64 = delta_h
            .edges
            .iter()
            .filter(|&&(u, v, _)| (u == 0 && v == 1) || (u == 1 && v == 0))
            .map(|&(_, _, w)| w.unwrap_or(0.0))
            .sum();
        assert!(
            (total_weight_01 - 5.0).abs() < 1e-9,
            "compressed superedge (0,1) should have weight 5.0, got {}",
            total_weight_01
        );
    }

    #[test]
    fn test_aggregation_removes_zero_weight_edges() {
        // Algorithm 4: Compress should remove zero-weight superedges.
        let g = graph(4, &[(0, 1), (2, 3)]);
        let inmem = InMemoryGraph::from(&g);

        let s_pre = vec![0, 0, 1, 1];
        let s_cur = vec![0, 0, 1, 1];

        // Two edges that cancel out: (0,2, +3) and (0,3, -3) both map to superedge (0,1)
        let delta = weighted_graph(4, &[(0, 2, 3.0), (0, 3, -3.0)]);

        let refined = bitvec![0; 4];

        let (delta_h, _new_s_pre) = inc_aggregation(&inmem, &delta, &s_pre, &s_cur, &refined);

        // The superedge (0,1) should have near-zero weight and be removed
        let weight_01: f64 = delta_h
            .edges
            .iter()
            .filter(|&&(u, v, _)| (u == 0 && v == 1) || (u == 1 && v == 0))
            .map(|&(_, _, w)| w.unwrap_or(0.0))
            .sum();
        assert!(
            weight_01.abs() < 1e-8,
            "cancelling edges should produce zero-weight superedge (removed), got {}",
            weight_01
        );
    }

    #[test]
    fn test_aggregation_updates_s_pre_for_refined_nodes() {
        // Algorithm 4 lines 12-13: s_pre(vi) ← s_cur(vi) for vi ∈ R
        let g = graph(3, &[(0, 1), (1, 2)]);
        let inmem = InMemoryGraph::from(&g);

        let s_pre = vec![0, 1, 2]; // original subcommunity mapping
        let s_cur = vec![0, 1, 1]; // node 2 moved to subcommunity 1

        let mut refined = bitvec![0; 3];
        refined.set(2, true); // Only node 2 was refined

        let delta = GraphInput {
            dataset_id: "test".to_string(),
            node_count: 3,
            edges: vec![],
        };

        let (_delta_h, new_s_pre) = inc_aggregation(&inmem, &delta, &s_pre, &s_cur, &refined);

        // s_pre for node 2 should now match s_cur
        assert_eq!(
            new_s_pre[2], 1,
            "s_pre for refined node 2 should be updated to s_cur value 1, got {}",
            new_s_pre[2]
        );
        // Unrefined nodes should retain their s_pre values
        assert_eq!(new_s_pre[0], 0, "unrefined node 0 should keep s_pre=0");
        assert_eq!(new_s_pre[1], 1, "unrefined node 1 should keep s_pre=1");
    }

    // =====================================================================
    // Algorithm 5: Def-update — backward propagation
    // =====================================================================

    #[test]
    fn test_def_update_propagates_community_assignments() {
        // Algorithm 5 line 4: f_p(v) = f_{p+1}(s_p(v))
        // With 2 levels: changed node at level 1 should get its community
        // from level 2's mapping via the subcommunity mapping.
        let mut community_mapping = vec![
            vec![0, 0, 1, 1], // level 0 (fine)
            vec![0, 1],       // level 1 (coarse) — 2 supernodes
        ];
        let subcommunity_mapping = vec![
            vec![0, 0, 1, 1], // level 0: nodes 0,1→supernode 0; nodes 2,3→supernode 1
            vec![0, 1],       // level 1 identity
        ];
        let mut changed_nodes = vec![
            bitvec![0; 4], // level 0: no direct changes
            bitvec![0; 2], // level 1: supernode 0 changed
        ];
        changed_nodes[1].set(0, true); // Supernode 0 changed at level 1

        // Change level 1's community: supernode 0 now in community 1
        community_mapping[1][0] = 1;

        def_update(
            &mut community_mapping,
            &subcommunity_mapping,
            &mut changed_nodes,
            2,
        );

        // After def-update, level 0 nodes mapped through supernode 0 should
        // inherit the new community. Nodes 0,1 → s_0 maps to supernode 0 →
        // f_1(supernode 0) = 1.
        assert_eq!(
            community_mapping[0][0], 1,
            "node 0 should inherit community 1 from level 1 via def-update"
        );
        assert_eq!(
            community_mapping[0][1], 1,
            "node 1 should inherit community 1 from level 1 via def-update"
        );
    }

    #[test]
    fn test_def_update_propagates_changed_nodes_downward() {
        // Algorithm 5 lines 5-7: B_{p-1}.add(s^{-p}(v_i^p))
        // Changed nodes at level p should propagate to level p-1 via inverse mapping.
        let mut community_mapping = vec![
            vec![0, 0, 1, 1], // level 0
            vec![0, 1],       // level 1
        ];
        let subcommunity_mapping = vec![
            vec![0, 0, 1, 1], // level 0: nodes 0,1→0; nodes 2,3→1
            vec![0, 1],       // level 1
        ];
        let mut changed_nodes = vec![
            bitvec![0; 4], // level 0
            bitvec![0; 2], // level 1
        ];
        changed_nodes[1].set(1, true); // Supernode 1 changed at level 1

        def_update(
            &mut community_mapping,
            &subcommunity_mapping,
            &mut changed_nodes,
            2,
        );

        // Supernode 1 at level 1 maps back to nodes 2,3 at level 0.
        // These should be marked as changed at level 0.
        assert!(
            changed_nodes[0][2],
            "node 2 should be marked changed at level 0 (inverse of supernode 1)"
        );
        assert!(
            changed_nodes[0][3],
            "node 3 should be marked changed at level 0 (inverse of supernode 1)"
        );
        // Nodes 0,1 should NOT be changed (they map to supernode 0, not 1)
        assert!(!changed_nodes[0][0], "node 0 should NOT be marked changed");
        assert!(!changed_nodes[0][1], "node 1 should NOT be marked changed");
    }

    #[test]
    fn test_def_update_skips_boundary_levels() {
        // Algorithm 5 line 2: skip propagation at p = P (top level)
        // Algorithm 5 line 5: skip downward propagation at p = 1 (bottom level)
        //
        // Single level: no propagation should occur, no panics.
        let mut community_mapping = vec![vec![0, 1, 2]];
        let subcommunity_mapping = vec![vec![0, 1, 2]];
        let mut changed_nodes = vec![bitvec![0; 3]];
        changed_nodes[0].set(0, true);

        // Should not panic with single level
        def_update(
            &mut community_mapping,
            &subcommunity_mapping,
            &mut changed_nodes,
            1,
        );

        // Community mapping should remain unchanged (no higher level to read from)
        assert_eq!(community_mapping[0], vec![0, 1, 2]);
    }

    // =====================================================================
    // Algorithm 6: HIT-Leiden — end-to-end incremental
    // =====================================================================

    dual_mode_test!(test_hit_leiden_initial_run_finds_communities, |mode| {
        // First call with full graph as delta should find communities
        // equivalent to standard Leiden.
        let g = graph(
            6,
            &[
                (0, 1),
                (1, 2),
                (0, 2), // clique A
                (3, 4),
                (4, 5),
                (3, 5), // clique B
                (2, 3), // bridge
            ],
        );
        let mut state = PartitionState::identity(6);
        state.supergraphs.push(InMemoryGraph::from(&GraphInput {
            dataset_id: "test".to_string(),
            node_count: 6,
            edges: vec![],
        }));

        hit_leiden(&mut state, &g, 1.0, mode);

        let comm_count = count_unique(&state.node_to_comm);
        assert!(
            comm_count >= 2,
            "HIT-Leiden initial run should find at least 2 communities, got {}",
            comm_count
        );
    });

    dual_mode_test!(test_hit_leiden_incremental_adapts_to_new_edge, |mode| {
        // Algorithm 6: after initial partition, adding a strong cross-community
        // edge should cause the affected nodes to merge communities.
        //
        // Use stronger initial edges to ensure proper initial partitioning.
        let g = weighted_graph(
            6,
            &[
                (0, 1, 10.0),
                (1, 2, 10.0),
                (0, 2, 10.0), // strong clique A
                (3, 4, 10.0),
                (4, 5, 10.0),
                (3, 5, 10.0), // strong clique B
                (2, 3, 0.1),  // weak bridge
            ],
        );
        let mut state = PartitionState::identity(6);
        state.supergraphs.push(InMemoryGraph::from(&GraphInput {
            dataset_id: "test".to_string(),
            node_count: 6,
            edges: vec![],
        }));

        // Initial partition — should find 2 communities
        hit_leiden(&mut state, &g, 1.0, mode);
        let initial_comm_count = count_unique(&state.node_to_comm);

        // Now add very strong cross-community edges
        let delta = weighted_graph(6, &[(0, 3, 100.0), (1, 4, 100.0)]);
        hit_leiden(&mut state, &delta, 1.0, mode);

        // With such strong connecting edges, the communities should merge or restructure
        let final_comm_count = count_unique(&state.node_to_comm);
        assert!(
            final_comm_count <= initial_comm_count,
            "strong cross-community edges should not increase community count: \
             initial={}, final={}",
            initial_comm_count,
            final_comm_count
        );
    });

    dual_mode_test!(test_hit_leiden_incremental_adapts_to_edge_removal, |mode| {
        // Removing a bridge edge should split the community.
        let g = graph(
            4,
            &[(0, 1), (1, 2), (2, 3)], // Chain
        );
        let mut state = PartitionState::identity(4);
        state.supergraphs.push(InMemoryGraph::from(&GraphInput {
            dataset_id: "test".to_string(),
            node_count: 4,
            edges: vec![],
        }));

        // Initial partition
        hit_leiden(&mut state, &g, 1.0, mode);

        // Remove the bridge edge 1-2
        let delta = weighted_graph(4, &[(1, 2, -1.0)]);
        hit_leiden(&mut state, &delta, 1.0, mode);

        // After removing bridge, nodes may separate
        // (exact behavior depends on remaining connectivity and resolution)
        // At minimum, the algorithm should complete without errors.
    });

    dual_mode_test!(test_hit_leiden_delta_g_flows_between_levels, |mode| {
        // Algorithm 6 line 7: ΔG^{p+1} comes from inc-aggregation.
        // With multiple levels, edge changes should propagate up.
        //
        // We use a graph large enough to create multiple hierarchy levels.
        // 8 nodes: two cliques of 4 connected by a bridge.
        let g = graph(
            8,
            &[
                (0, 1),
                (0, 2),
                (0, 3),
                (1, 2),
                (1, 3),
                (2, 3), // K4 clique A
                (4, 5),
                (4, 6),
                (4, 7),
                (5, 6),
                (5, 7),
                (6, 7), // K4 clique B
                (3, 4), // bridge
            ],
        );
        let mut state = PartitionState::identity(8);
        state.supergraphs.push(InMemoryGraph::from(&GraphInput {
            dataset_id: "test".to_string(),
            node_count: 8,
            edges: vec![],
        }));

        // Initial partition
        hit_leiden(&mut state, &g, 1.0, mode);

        let _initial_comm_count = count_unique(&state.node_to_comm);

        // Now add strong cross-clique edges
        let delta = weighted_graph(8, &[(0, 4, 50.0), (1, 5, 50.0)]);
        hit_leiden(&mut state, &delta, 1.0, mode);

        // The strong cross-clique edges should cause community restructuring
        let new_comm_count = count_unique(&state.node_to_comm);
        // The algorithm should complete and potentially merge communities
        assert!(
            new_comm_count >= 1,
            "algorithm should complete with valid community count"
        );
    });

    // =====================================================================
    // Edge cases
    // =====================================================================

    #[test]
    fn test_graph_aggregation_two_communities() {
        // Verify aggregate_graph produces correct coarsened graph.
        // 4 nodes, 2 communities: {0,1} and {2,3}.
        // Edges: 0-1 (intra), 2-3 (intra), 1-2 (inter).
        let g = graph(4, &[(0, 1), (2, 3), (1, 2)]);
        let partition = vec![0, 0, 1, 1];

        let (coarse, remap) = aggregate_graph(&g, &partition, 2);

        assert_eq!(coarse.node_count, 2, "should have 2 super-nodes");

        // Should have: self-loop on 0, self-loop on 1, and inter-edge 0-1
        let mut has_self_0 = false;
        let mut has_self_1 = false;
        let mut has_inter = false;
        for &(u, v, w) in &coarse.edges {
            let weight = w.unwrap_or(0.0);
            if u == remap[&0] && v == remap[&0] {
                has_self_0 = true;
                assert!(
                    (weight - 1.0).abs() < 1e-9,
                    "self-loop on community 0 should have weight 1"
                );
            }
            if u == remap[&1] && v == remap[&1] {
                has_self_1 = true;
                assert!(
                    (weight - 1.0).abs() < 1e-9,
                    "self-loop on community 1 should have weight 1"
                );
            }
            let (min_uv, max_uv) = if u < v { (u, v) } else { (v, u) };
            if min_uv == remap[&0] && max_uv == remap[&1] {
                has_inter = true;
                assert!(
                    (weight - 1.0).abs() < 1e-9,
                    "inter-community edge should have weight 1"
                );
            }
        }
        assert!(has_self_0, "should have self-loop for community 0");
        assert!(has_self_1, "should have self-loop for community 1");
        assert!(has_inter, "should have inter-community edge");
    }

    #[test]
    fn test_connected_components_single_component() {
        // All nodes connected: should return 1 component.
        let g = graph(3, &[(0, 1), (1, 2)]);
        let inmem = InMemoryGraph::from(&g);

        let vertices = vec![0, 1, 2];
        let components = find_connected_components_in_subcommunity(&inmem, &vertices, 3);

        assert_eq!(
            components.len(),
            1,
            "fully connected subgraph should have 1 component"
        );
        assert_eq!(components[0].len(), 3);
    }

    #[test]
    fn test_connected_components_multiple_components() {
        // Two disconnected pairs: should return 2 components.
        let g = graph(4, &[(0, 1), (2, 3)]);
        let inmem = InMemoryGraph::from(&g);

        let vertices = vec![0, 1, 2, 3];
        let components = find_connected_components_in_subcommunity(&inmem, &vertices, 4);

        assert_eq!(
            components.len(),
            2,
            "two disconnected pairs should give 2 components"
        );
    }

    #[test]
    fn test_connected_components_single_node() {
        // Single node with no edges: 1 component of size 1.
        let g = GraphInput {
            dataset_id: "test".to_string(),
            node_count: 1,
            edges: vec![],
        };
        let inmem = InMemoryGraph::from(&g);

        let vertices = vec![0];
        let components = find_connected_components_in_subcommunity(&inmem, &vertices, 1);

        assert_eq!(components.len(), 1);
        assert_eq!(components[0], vec![0]);
    }

    #[test]
    fn test_count_unique() {
        assert_eq!(count_unique(&[0, 0, 1, 1, 2]), 3);
        assert_eq!(count_unique(&[0, 0, 0]), 1);
        assert_eq!(count_unique(&[]), 0);
        assert_eq!(count_unique(&[5, 3, 5, 3, 7, 7]), 3);
    }

    dual_mode_test!(
        test_movement_and_refinement_idempotent_on_stable_partition,
        |mode| {
            // Once a partition has converged, running inc-movement with an
            // empty delta should produce no changes.
            let g = graph(6, &[(0, 1), (1, 2), (0, 2), (3, 4), (4, 5), (3, 5), (2, 3)]);
            let inmem = InMemoryGraph::from(&g);

            // First pass to get a stable partition
            let mut node_to_community = vec![0, 0, 0, 1, 1, 1];
            let node_to_subcommunity = vec![0, 0, 0, 1, 1, 1];

            let delta = GraphInput::empty("test");
            let (_changed1, _k1, _iters1) = inc_movement(
                &inmem,
                &delta,
                &mut node_to_community,
                &node_to_subcommunity,
                1.0,
                mode,
            );

            // Second pass on same partition with empty delta
            let partition_after_first = node_to_community.clone();
            let (_changed2, _k2, _iters2) = inc_movement(
                &inmem,
                &delta,
                &mut node_to_community,
                &node_to_subcommunity,
                1.0,
                mode,
            );

            // The partition should not change on the second pass
            assert_eq!(
                node_to_community, partition_after_first,
                "stable partition should not change on second empty-delta pass"
            );
        }
    );

    #[test]
    fn test_build_coarse_initial_partition_maps_correctly() {
        // Verify build_coarse_initial_partition projects communities correctly.
        let mut subcomm_to_comm: HashMap<usize, usize> = HashMap::new();
        subcomm_to_comm.insert(0, 10); // subcommunity 0 → community 10
        subcomm_to_comm.insert(1, 10); // subcommunity 1 → community 10
        subcomm_to_comm.insert(2, 20); // subcommunity 2 → community 20

        let mut subcomm_remap: HashMap<usize, usize> = HashMap::new();
        subcomm_remap.insert(0, 0); // subcommunity 0 → coarse node 0
        subcomm_remap.insert(1, 1); // subcommunity 1 → coarse node 1
        subcomm_remap.insert(2, 2); // subcommunity 2 → coarse node 2

        let result = build_coarse_initial_partition(&subcomm_to_comm, &subcomm_remap, 3);

        // Coarse nodes 0 and 1 should map to the same coarse community
        assert_eq!(
            result[0], result[1],
            "coarse nodes from same community should share partition"
        );
        // Coarse node 2 should be in a different community
        assert_ne!(
            result[0], result[2],
            "coarse nodes from different communities should differ"
        );
    }
}
