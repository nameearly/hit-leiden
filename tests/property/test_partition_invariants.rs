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
use proptest::prelude::*;

proptest! {
    #[test]
    fn partition_len_matches_nodes(node_count in 0usize..50) {
        let graph = GraphInput {
            dataset_id: "p1".to_string(),
            node_count,
            edges: vec![],
        };
        let config = RunConfig::default();
        let out = run(&graph, &config).expect("run");
        prop_assert_eq!(out.partition.unwrap().node_to_community.len(), node_count);
    }
}
