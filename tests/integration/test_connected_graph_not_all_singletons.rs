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

use std::collections::HashSet;

use hit_leiden::{run, GraphInput, RunConfig};

#[test]
fn connected_graph_not_all_singletons() {
    let graph = GraphInput {
        dataset_id: "connected-1".to_string(),
        node_count: 6,
        edges: vec![
            (0, 1, Some(1.0)),
            (1, 2, Some(1.0)),
            (2, 0, Some(1.0)),
            (3, 4, Some(1.0)),
            (4, 5, Some(1.0)),
            (5, 3, Some(1.0)),
            (2, 3, Some(0.05)),
        ],
    };

    let out = run(&graph, &RunConfig::default()).expect("run should succeed");
    let unique: HashSet<_> = out
        .partition
        .unwrap()
        .node_to_community
        .iter()
        .copied()
        .collect();
    assert!(
        unique.len() < graph.node_count,
        "algorithm should merge at least one pair of nodes"
    );
}
