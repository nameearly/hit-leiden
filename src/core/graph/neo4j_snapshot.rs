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

use crate::core::error::HitLeidenError;
use crate::core::graph::neo4j_mapping::ProjectionConfig;
use crate::core::types::GraphInput;

#[derive(Clone, Debug)]
pub struct Neo4jSourceConfig {
    pub uri: String,
}

pub fn project_from_neo4j(
    _source_config: &Neo4jSourceConfig,
    projection_config: &ProjectionConfig,
) -> Result<GraphInput, HitLeidenError> {
    Ok(GraphInput {
        dataset_id: format!("neo4j:{}", projection_config.snapshot_id),
        node_count: 0,
        edges: Vec::new(),
    })
}
