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

use crate::core::algorithm::parallel_frontier::{
    execute_shard, MoveCandidate, ShardResult, SharedBitVec,
};
use crate::core::graph::in_memory::InMemoryGraph;
use ahash::{HashMap, HashMapExt, HashSet, HashSetExt};
use rayon::prelude::*;

/// Thread-safe wrapper around UnsafeCell for per-thread buffer access.
/// SAFETY: Each rayon worker thread accesses a unique index via current_thread_index(),
/// ensuring no concurrent access to the same UnsafeCell. This is safe because:
/// 1. Rayon maintains a persistent thread pool (no thread ID reuse during parallel work)
/// 2. Each thread accesses only buffers[current_thread_index()]
/// 3. No two threads share the same thread index value
struct SyncUnsafeCell<T>(std::cell::UnsafeCell<T>);

unsafe impl<T> Sync for SyncUnsafeCell<T> {}

impl<T> SyncUnsafeCell<T> {
    fn new(value: T) -> Self {
        SyncUnsafeCell(std::cell::UnsafeCell::new(value))
    }

    fn get(&self) -> *mut T {
        self.0.get()
    }
}

/// Reusable per-thread buffer pool for inc_movement_parallel.
/// Allocates once and reuses across multiple calls to avoid repeated allocations.
/// Safe to reuse because each thread accesses its own slot via current_thread_index().
pub struct BufferPool {
    buffers: Vec<SyncUnsafeCell<(Vec<f64>, Vec<usize>)>>,
}

impl BufferPool {
    /// Create a new buffer pool for `num_threads` workers processing graphs with `node_count` nodes.
    /// Pre-allocates full-size neighbor weight buffers to avoid growth reallocations.
    pub fn new(node_count: usize, num_threads: usize) -> Self {
        let buffers: Vec<_> = (0..num_threads)
            .map(|_| SyncUnsafeCell::new((vec![0.0; node_count], Vec::with_capacity(4096))))
            .collect();
        BufferPool { buffers }
    }

    /// Reset buffers for reuse (clears Vec contents but keeps allocations).
    /// Only clears dirty_communities - neighbor_buf doesn't need zeroing since
    /// execute_shard only reads from indices it previously wrote to.
    pub fn reset(&self) {
        for i in 0..self.buffers.len() {
            unsafe {
                let buf_pair = &mut *(*self.buffers)[i].get();
                buf_pair.1.clear(); // Only clear dirty_communities, not neighbor_buf
            }
        }
    }

    /// Get internal buffer references (internal use only via inc_movement_parallel).
    fn as_ref(&self) -> &[SyncUnsafeCell<(Vec<f64>, Vec<usize>)>] {
        &self.buffers
    }
}

#[allow(clippy::too_many_arguments)]
pub fn inc_movement_parallel(
    graph: &InMemoryGraph,
    active_nodes_vec: &[usize],
    node_to_community: &mut [usize],
    node_to_subcommunity: &[usize],
    community_degrees: &mut Vec<f64>,
    node_degrees: &[f64],
    twice_total_weight: f64,
    resolution_parameter: f64,
    allow_empty_community_moves: bool,
    buffer_pool: &BufferPool,
) -> (SharedBitVec, SharedBitVec, SharedBitVec) {
    let n = graph.node_count;

    // Shared atomic bitvecs — rayon worker threads write directly via fetch_or.
    // Rayon maintains a persistent thread pool so there is no spawn/join churn.
    let shared_changed = SharedBitVec::new(n);
    let shared_affected = SharedBitVec::new(n);
    let shared_next_active = SharedBitVec::new(n);

    // Reset buffer pool for reuse (keeps allocations, clears data)
    buffer_pool.reset();
    let buffers = buffer_pool.as_ref();
    let num_threads = buffers.len(); // Get thread count from buffer pool

    // Create immutable views for parallel access
    let node_to_community_view: &[usize] = node_to_community;
    let community_degrees_view: &[f64] = community_degrees;

    // Pre-chunk work by thread count for load balancing
    let chunk_size = active_nodes_vec.len().div_ceil(num_threads);
    let chunks: Vec<&[usize]> = active_nodes_vec.chunks(chunk_size).collect();

    // Per-chunk result storage: SyncUnsafeCell to eliminate Mutex syscalls (20% overhead).
    // Safe: each chunk is processed by one thread only; no concurrent access.
    let results: Vec<SyncUnsafeCell<Option<ShardResult>>> = (0..chunks.len())
        .map(|_| SyncUnsafeCell::new(None))
        .collect();

    // Spawn one task per chunk and let rayon's work-stealing handle load balancing.
    // This avoids the problem where pre-batching causes fast threads to idle waiting
    // for slower threads at the scope barrier. Graph structure is uneven (varying degrees),
    // so work-stealing is essential to keep all threads busy.

    rayon::scope(|s| {
        for (chunk_idx, chunk) in chunks.into_iter().enumerate() {
            // Capture references for borrowing in the move closure
            let buffers = &buffers;
            let results = &results;

            s.spawn(move |_| {
                // Each spawn gets a unique thread from rayon's persistent pool.
                // Direct UnsafeCell access - zero syscall overhead!
                let thread_idx = rayon::current_thread_index().unwrap_or(0);
                let buf_pair = unsafe { &mut *(*buffers)[thread_idx % buffers.len()].get() };
                let (neighbor_buf, dirty_buf) = buf_pair;

                let result = execute_shard(
                    graph,
                    chunk,
                    node_to_community_view,
                    node_to_subcommunity,
                    community_degrees_view,
                    node_degrees,
                    twice_total_weight,
                    resolution_parameter,
                    allow_empty_community_moves,
                    neighbor_buf,
                    dirty_buf,
                    n,
                );

                // Store result in per-chunk slot (no lock needed, no concurrent access)
                unsafe { *(*results)[chunk_idx].get() = Some(result) };
            });
        }
    });

    // Extract results in order - no sorting needed, indices match chunk order
    let results: Vec<_> = results
        .into_iter()
        .map(|cell| unsafe { (*cell.get()).take().unwrap() })
        .collect();

    // Collect candidates and decouple conflicting moves globally.
    let mut all_candidates: Vec<MoveCandidate> = results
        .into_iter()
        .flat_map(|r| r.move_candidates)
        .collect();
    let selected_moves = select_decoupled_moves(&mut all_candidates);

    // Empty community IDs (>= n) may have been allocated; resize community_degrees if needed.
    let max_comm = selected_moves.iter().map(|m| m.to_comm).max().unwrap_or(0);
    if max_comm >= community_degrees.len() {
        community_degrees.resize(max_comm + 1, 0.0);
    }

    let total_gain: f64 = selected_moves.iter().map(|m| m.gain).sum();
    if total_gain <= 0.0 {
        return (shared_changed, shared_affected, shared_next_active);
    }

    for mv in &selected_moves {
        node_to_community[mv.node] = mv.to_comm;
        community_degrees[mv.from_comm] -= mv.node_degree;
        community_degrees[mv.to_comm] += mv.node_degree;
        shared_changed.set(mv.node);
    }

    for mv in &selected_moves {
        for (neighbor_node, _w) in graph.neighbors(mv.node) {
            if node_to_community[neighbor_node] != mv.to_comm {
                shared_next_active.set(neighbor_node);
            }
            if node_to_subcommunity[mv.node] == node_to_subcommunity[neighbor_node] {
                shared_affected.set(mv.node);
                shared_affected.set(neighbor_node);
            }
        }
    }

    (shared_changed, shared_affected, shared_next_active)
}

fn select_decoupled_moves(candidates: &mut [MoveCandidate]) -> Vec<MoveCandidate> {
    candidates.sort_by(|a, b| {
        b.gain
            .partial_cmp(&a.gain)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.node.cmp(&b.node))
    });

    let mut emitters = HashSet::new();
    let mut acceptors = HashSet::new();
    let mut moved_nodes = HashSet::new();
    let mut selected = Vec::with_capacity(candidates.len());

    for mv in candidates.iter() {
        if mv.gain <= 0.0 {
            continue;
        }
        if moved_nodes.contains(&mv.node) {
            continue;
        }
        // Decoupling rule: no move may leave an acceptor or enter an emitter.
        if acceptors.contains(&mv.from_comm) || emitters.contains(&mv.to_comm) {
            continue;
        }

        selected.push(mv.clone());
        moved_nodes.insert(mv.node);
        emitters.insert(mv.from_comm);
        acceptors.insert(mv.to_comm);
    }

    selected
}

#[allow(clippy::too_many_arguments)]
pub fn inc_refinement_parallel(
    graph: &InMemoryGraph,
    refined_nodes_sorted: &[usize],
    node_to_community: &[usize],
    node_to_subcommunity: &mut [usize],
    subcommunity_degrees: &mut HashMap<usize, f64>,
    subcommunity_sizes: &mut [usize],
    node_degrees: &[f64],
    twice_total_weight: f64,
    resolution_parameter: f64,
    blocked_subcommunities: &HashSet<usize>,
) {
    let chunk_size = (refined_nodes_sorted.len() / rayon::current_num_threads()).max(1);
    let states: Vec<Vec<(usize, usize, usize, f64)>> = refined_nodes_sorted
        .par_chunks(chunk_size)
        .map(|shard| {
            let mut local_updates = Vec::new();
            for &current_node in shard {
                if subcommunity_sizes[node_to_subcommunity[current_node]] != 1 {
                    continue;
                }
                if let Some(update) = try_refinement_move(
                    graph,
                    current_node,
                    node_to_community,
                    node_to_subcommunity,
                    subcommunity_degrees,
                    node_degrees,
                    twice_total_weight,
                    resolution_parameter,
                    blocked_subcommunities,
                ) {
                    local_updates.push(update);
                }
            }
            local_updates
        })
        .collect();

    for state in states {
        for (node, old_subcomm, new_subcomm, degree) in state {
            node_to_subcommunity[node] = new_subcomm;
            *subcommunity_degrees.entry(old_subcomm).or_insert(0.0) -= degree;
            *subcommunity_degrees.entry(new_subcomm).or_insert(0.0) += degree;
            // Update subcommunity sizes to keep singleton check accurate across chunks
            subcommunity_sizes[old_subcomm] = subcommunity_sizes[old_subcomm].saturating_sub(1);
            subcommunity_sizes[new_subcomm] += 1;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn try_refinement_move(
    graph: &InMemoryGraph,
    current_node: usize,
    node_to_community: &[usize],
    node_to_subcommunity: &[usize],
    subcommunity_degrees: &HashMap<usize, f64>,
    node_degrees: &[f64],
    twice_total_weight: f64,
    resolution_parameter: f64,
    blocked_subcommunities: &HashSet<usize>,
) -> Option<(usize, usize, usize, f64)> {
    let current_comm = node_to_community[current_node];
    let current_sc = node_to_subcommunity[current_node];
    let current_node_degree = node_degrees[current_node];

    let mut neighbor_subcommunities: HashMap<usize, f64> = HashMap::new();
    let mut weight_to_current_subcommunity = 0.0;

    for (neighbor_node, w) in graph.neighbors(current_node) {
        if node_to_community[neighbor_node] != current_comm {
            continue;
        }
        let neighbor_sc = node_to_subcommunity[neighbor_node];
        *neighbor_subcommunities.entry(neighbor_sc).or_insert(0.0) += w;
        if neighbor_sc == current_sc {
            weight_to_current_subcommunity += w;
        }
    }

    let mut best_subcommunity = current_sc;
    let mut best_modularity_gain = 0.0;

    for (&candidate_sc, &weight_to_candidate) in &neighbor_subcommunities {
        if candidate_sc == current_sc {
            continue;
        }

        // T-filter: skip "healthy" subcommunities where ΔQ(S→∅) > 0
        if blocked_subcommunities.contains(&candidate_sc) {
            continue;
        }

        let current_sc_degree = *subcommunity_degrees.get(&current_sc).unwrap_or(&0.0);
        let candidate_sc_degree = *subcommunity_degrees.get(&candidate_sc).unwrap_or(&0.0);

        let modularity_gain = (weight_to_candidate - weight_to_current_subcommunity)
            / twice_total_weight
            + resolution_parameter
                * current_node_degree
                * (current_sc_degree - current_node_degree - candidate_sc_degree)
                / (twice_total_weight * twice_total_weight);

        if modularity_gain > best_modularity_gain {
            best_modularity_gain = modularity_gain;
            best_subcommunity = candidate_sc;
        }
    }

    if best_modularity_gain > 0.0 {
        Some((
            current_node,
            current_sc,
            best_subcommunity,
            current_node_degree,
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mc(node: usize, from_comm: usize, to_comm: usize, gain: f64) -> MoveCandidate {
        MoveCandidate {
            node,
            from_comm,
            to_comm,
            node_degree: 1.0,
            gain,
        }
    }

    #[test]
    fn decoupling_blocks_reverse_conflicts() {
        let mut candidates = vec![mc(1, 10, 20, 10.0), mc(2, 20, 10, 9.0), mc(3, 30, 40, 8.0)];

        let selected = select_decoupled_moves(&mut candidates);
        let selected_nodes: Vec<usize> = selected.iter().map(|m| m.node).collect();

        assert_eq!(selected_nodes, vec![1, 3]);
    }

    #[test]
    fn decoupling_allows_emit_and_accept_consistency() {
        let mut candidates = vec![
            mc(10, 1, 2, 10.0),
            mc(11, 1, 3, 9.0),
            mc(12, 4, 2, 8.0),
            mc(13, 2, 5, 7.0),
        ];

        let selected = select_decoupled_moves(&mut candidates);
        let selected_nodes: Vec<usize> = selected.iter().map(|m| m.node).collect();

        // 10 accepted first (1->2), 11 and 12 remain valid (emit from 1, accept to 2),
        // 13 must be rejected because it emits from acceptor community 2.
        assert_eq!(selected_nodes, vec![10, 11, 12]);
    }
}
