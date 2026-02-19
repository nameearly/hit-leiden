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

use hit_leiden::{
    core::graph::{neo4j_mapping::ProjectionConfig, neo4j_snapshot::Neo4jSourceConfig},
    project_from_neo4j,
};

#[test]
fn neo4j_projection_parity_shape() {
    let source = Neo4jSourceConfig {
        uri: "bolt://localhost".to_string(),
    };
    let proj = ProjectionConfig {
        snapshot_id: "s1".to_string(),
        batched: true,
    };
    let graph = project_from_neo4j(&source, &proj).expect("projection");
    assert!(graph.dataset_id.starts_with("neo4j:"));
}
