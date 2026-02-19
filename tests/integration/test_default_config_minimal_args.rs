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

use hit_leiden::{cli::run::run_default, GraphInput};

#[test]
fn default_run_with_minimal_required_graph_source() {
    let graph = GraphInput {
        dataset_id: "min".into(),
        node_count: 1,
        edges: vec![],
    };
    let out = run_default(&graph).expect("default run should succeed");
    assert_eq!(out.partition.unwrap().node_to_community.len(), 1);
}
