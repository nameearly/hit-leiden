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

use crate::core::types::GraphInput;
use ahash::{HashMap, HashMapExt};

#[derive(Clone, Debug, PartialEq)]
pub struct InMemoryGraph {
    pub node_count: usize,
    pub offsets: Vec<usize>,
    pub degrees: Vec<usize>, // Precomputed for O(1) lookup
    pub neighbors: Vec<usize>,
    pub weights: Vec<f64>,
    cached_total_weight: f64,
}

impl From<&GraphInput> for InMemoryGraph {
    fn from(value: &GraphInput) -> Self {
        let mut degrees = vec![0; value.node_count];
        for &(u, v, _) in &value.edges {
            degrees[u] += 1;
            degrees[v] += 1;
        }

        let mut offsets = vec![0; value.node_count + 1];
        for i in 0..value.node_count {
            offsets[i + 1] = offsets[i] + degrees[i];
        }

        let total_entries = offsets[value.node_count];
        let mut neighbors = vec![0; total_entries];
        let mut weights = vec![0.0; total_entries];
        let mut current_offsets = offsets.clone();

        for &(u, v, w) in &value.edges {
            let weight = w.unwrap_or(1.0);

            let u_offset = current_offsets[u];
            neighbors[u_offset] = v;
            weights[u_offset] = weight;
            current_offsets[u] += 1;

            let v_offset = current_offsets[v];
            neighbors[v_offset] = u;
            weights[v_offset] = weight;
            current_offsets[v] += 1;
        }

        let cached_total_weight = weights.iter().sum::<f64>() / 2.0;

        Self {
            node_count: value.node_count,
            offsets,
            degrees,
            neighbors,
            weights,
            cached_total_weight,
        }
    }
}

impl InMemoryGraph {
    /// Iterate over (neighbor, weight) pairs for a node.
    /// Uses precomputed degree for single-load bound calculation.
    #[inline]
    pub fn neighbors(&self, node: usize) -> impl Iterator<Item = (usize, f64)> + '_ {
        let start = self.offsets[node];
        let count = self.degrees[node]; // Single load instead of offsets[node+1]
        self.neighbors[start..start + count]
            .iter()
            .copied()
            .zip(self.weights[start..start + count].iter().copied())
    }

    /// Get node degree in O(1) time.
    #[inline]
    pub fn degree(&self, node: usize) -> usize {
        self.degrees[node] // Direct lookup, no subtraction
    }

    pub fn total_weight(&self) -> f64 {
        self.cached_total_weight
    }

    /// Apply a delta graph (edges with positive/negative weights) to produce
    /// a new InMemoryGraph. Positive weights add/strengthen edges, negative
    /// weights remove/weaken edges. Edges that reach zero weight are removed.
    ///
    /// Algorithm 6, line 3: G^p ← G^p ⊕ ΔG^p
    pub fn apply_delta(&self, delta: &GraphInput) -> InMemoryGraph {
        let mut edge_map = self.collect_canonical_edges();
        apply_delta_edges(&mut edge_map, delta);

        let new_node_count = self.node_count.max(delta.node_count);
        let edges: Vec<(usize, usize, Option<f64>)> = edge_map
            .into_iter()
            .filter(|(_, w)| *w > 1e-12)
            .map(|((u, v), w)| (u, v, Some(w)))
            .collect();

        let result_input = GraphInput {
            dataset_id: delta.dataset_id.clone(),
            node_count: new_node_count,
            edges,
        };
        InMemoryGraph::from(&result_input)
    }

    fn collect_canonical_edges(&self) -> HashMap<(usize, usize), f64> {
        let mut edge_map = HashMap::with_capacity(self.neighbors.len() / 2);
        for node in 0..self.node_count {
            for (neighbor, weight) in self.neighbors(node).filter(|&(n, _)| node < n) {
                edge_map.insert((node, neighbor), weight);
            }
        }
        edge_map
    }
}

fn apply_delta_edges(edge_map: &mut HashMap<(usize, usize), f64>, delta: &GraphInput) {
    for &(u, v, w) in &delta.edges {
        let key = if u <= v { (u, v) } else { (v, u) };
        *edge_map.entry(key).or_insert(0.0) += w.unwrap_or(1.0);
    }
}
