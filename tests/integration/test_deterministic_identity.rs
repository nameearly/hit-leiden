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

use hit_leiden::{run, GraphInput, RunConfig};

#[test]
fn deterministic_replay_identity() {
    let graph = GraphInput {
        dataset_id: "d2".to_string(),
        node_count: 4,
        edges: vec![(0, 1, None), (2, 3, None)],
    };
    let config = RunConfig::default();
    let a = run(&graph, &config).expect("a");
    let b = run(&graph, &config).expect("b");
    assert_eq!(a.partition, b.partition);
}
