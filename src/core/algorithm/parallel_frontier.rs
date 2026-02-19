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

use crate::core::graph::in_memory::InMemoryGraph;
use bitvec::prelude::*;
use smallvec::SmallVec;
use std::sync::atomic::{AtomicU64, Ordering};

/// Cache-aligned atomic word to prevent false sharing.
/// Each word occupies its own cache line (64 bytes).
#[repr(align(64))]
struct CacheAligned<T>(T);

/// Lock-free bit vector backed by cache-aligned atomic 64-bit words.
///
/// Each word sits on its own cache line to prevent false sharing when
/// multiple threads write to adjacent bit ranges.
///
/// Supports concurrent `set` operations from multiple threads via `fetch_or`.
/// Iterate with `iter_ones()` or `any()` without conversion overhead.
pub struct SharedBitVec {
    words: Vec<CacheAligned<AtomicU64>>,
    len: usize,
}

impl SharedBitVec {
    pub fn new(len: usize) -> Self {
        let num_words = len.div_ceil(64);
        let words = (0..num_words)
            .map(|_| CacheAligned(AtomicU64::new(0)))
            .collect();
        Self { words, len }
    }

    /// Atomically set a single bit. Safe to call from multiple threads.
    #[inline(always)]
    pub fn set(&self, index: usize) {
        debug_assert!(index < self.len);
        let word_idx = index / 64;
        let bit_idx = index % 64;
        self.words[word_idx]
            .0
            .fetch_or(1u64 << bit_idx, Ordering::Relaxed);
    }

    /// Check if any bit is set (non-zero). Used for loop termination.
    #[inline]
    pub fn any(&self) -> bool {
        self.words.iter().any(|w| w.0.load(Ordering::Relaxed) != 0)
    }

    /// Iterate over set bit indices. Reads atomic words directly (no BitVec allocation).
    pub fn iter_ones(&self) -> impl Iterator<Item = usize> + '_ {
        self.words
            .iter()
            .enumerate()
            .flat_map(|(word_idx, aligned)| {
                let word = aligned.0.load(Ordering::Relaxed);
                extract_set_bits(word, word_idx, self.len)
            })
    }

    /// OR all set bits into an existing `BitVec` accumulator.
    /// Avoids allocating a new BitVec — only sets additional bits in `target`.
    /// Call only after all writing threads have joined.
    pub fn or_into_bitvec(&self, target: &mut BitVec) {
        for (word_idx, aligned) in self.words.iter().enumerate() {
            let word = aligned.0.load(Ordering::Relaxed);
            for idx in extract_set_bits(word, word_idx, self.len) {
                target.set(idx, true);
            }
        }
    }

    /// Collect set bit indices into an existing Vec, reusing its allocation.
    /// Call only after all writing threads have joined.
    pub fn collect_ones_into(&self, target: &mut Vec<usize>) {
        target.clear();
        target.extend(self.iter_ones());
    }
}

fn extract_set_bits(mut word: u64, word_idx: usize, len: usize) -> SmallVec<[usize; 8]> {
    let mut indices = SmallVec::<[usize; 8]>::new();
    while word != 0 {
        let bit = word.trailing_zeros() as usize;
        let global_idx = word_idx * 64 + bit;
        if global_idx < len {
            indices.push(global_idx);
        }
        word &= word.wrapping_sub(1);
    }
    indices
}

/// Per-shard results that must be applied sequentially after all threads join.
pub struct ShardResult {
    pub move_candidates: Vec<MoveCandidate>,
}

#[derive(Debug, Clone)]
pub struct MoveCandidate {
    pub node: usize,
    pub from_comm: usize,
    pub to_comm: usize,
    pub node_degree: f64,
    pub gain: f64,
}

/// Execute one shard of the incremental movement step.
///
/// Bits for `changed_nodes`, `affected_nodes`, and `next_active_nodes` are
/// written directly into shared atomic bitvecs (zero-copy merge).
/// Only the community-assignment and degree-delta updates are returned for
/// sequential application.
///
/// `neighbor_weight_buf` and `dirty_communities` are thread-local scratch
/// buffers reused across all nodes in the shard to avoid per-node allocation.
#[allow(clippy::too_many_arguments)]
pub fn execute_shard(
    graph: &InMemoryGraph,
    shard: &[usize],
    node_to_community: &[usize],
    _node_to_subcommunity: &[usize],
    community_degrees: &[f64],
    node_degrees: &[f64],
    twice_total_weight: f64,
    resolution_parameter: f64,
    allow_empty_community_moves: bool,
    neighbor_weight_buf: &mut [f64],
    dirty_communities: &mut Vec<usize>,
    graph_node_count: usize,
) -> ShardResult {
    // Pre-allocate result Vecs to avoid growth reallocations during execution.
    // Estimate ~15% of nodes will change community (conservative for modularity optimization).
    // Move candidates are later decoupled and applied centrally.
    let estimated_changes = (shard.len() * 15) / 100;
    let mut result = ShardResult {
        move_candidates: Vec::with_capacity(estimated_changes),
    };

    for &current_node in shard {
        let current_community = node_to_community[current_node];
        let current_node_degree = node_degrees[current_node];
        let mut best_community = current_community;
        let mut best_modularity_gain = 0.0;
        let mut weight_to_current_community = 0.0;

        // Accumulate neighbor weights by community in flat buffer (O(degree))
        for (neighbor_node, w) in graph.neighbors(current_node) {
            if neighbor_node == current_node {
                continue; // Skip self-loops (alignment with deterministic path)
            }
            let c = node_to_community[neighbor_node];
            if neighbor_weight_buf[c] == 0.0 {
                dirty_communities.push(c);
            }
            neighbor_weight_buf[c] += w;
            if c == current_community {
                weight_to_current_community += w;
            }
        }

        let current_community_degree = community_degrees[current_community];

        // Evaluate each neighbor community
        for &candidate_community in dirty_communities.iter() {
            if candidate_community == current_community {
                continue;
            }

            let weight_to_candidate = neighbor_weight_buf[candidate_community];
            let candidate_community_degree = community_degrees[candidate_community];

            let modularity_gain = (weight_to_candidate - weight_to_current_community)
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

        // Algorithm 2: argmax includes ∅ (empty community = singleton).
        // In multilevel throughput mode this can over-fragment communities due to
        // stale snapshots; callers can disable this path.
        if allow_empty_community_moves {
            // Use graph_node_count + current_node as a unique empty community ID
            // to avoid cross-thread ID conflicts.
            let empty_gain = -weight_to_current_community / twice_total_weight
                + resolution_parameter
                    * current_node_degree
                    * (current_community_degree - current_node_degree)
                    / (twice_total_weight * twice_total_weight);
            if empty_gain > best_modularity_gain {
                best_modularity_gain = empty_gain;
                best_community = graph_node_count + current_node;
            }
        }

        // Reset dirty entries (O(degree), not O(n))
        for &c in dirty_communities.iter() {
            neighbor_weight_buf[c] = 0.0;
        }
        dirty_communities.clear();

        // Skip if no beneficial move
        if best_modularity_gain <= 0.0 {
            continue;
        }

        result.move_candidates.push(MoveCandidate {
            node: current_node,
            from_comm: current_community,
            to_comm: best_community,
            node_degree: current_node_degree,
            gain: best_modularity_gain,
        });
    }

    result
}
