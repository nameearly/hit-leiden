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

use hit_leiden::{run, validate, GraphInput, RunConfig};

#[test]
fn run_and_validate_contract() {
    let graph = GraphInput {
        dataset_id: "d1".to_string(),
        node_count: 3,
        edges: vec![(0, 1, None), (1, 2, None)],
    };
    let config = RunConfig::default();
    let outcome = run(&graph, &config).expect("run should succeed");
    let validation = validate(&outcome, &outcome, config.mode);
    assert!(validation.equivalence_passed);
}
