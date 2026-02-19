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

use crate::core::backend::{AccelerationTarget, GraphSource, ResolutionMetadata};
use crate::core::config::RunConfig;

pub fn resolve(config: &RunConfig) -> ResolutionMetadata {
    ResolutionMetadata {
        source_resolved: config.graph_source,
        accel_resolved: config.acceleration,
        fallback_reason: None,
    }
}

pub fn release_gate_eligible(source: GraphSource) -> (bool, Option<String>) {
    if source == GraphSource::LiveNeo4j {
        (
            false,
            Some("LIVE_QUERY_SOURCE_INELIGIBLE_FOR_RELEASE_GATE".to_string()),
        )
    } else {
        (true, None)
    }
}

pub fn fallback(source: GraphSource, accel: AccelerationTarget) -> ResolutionMetadata {
    ResolutionMetadata {
        source_resolved: source,
        accel_resolved: accel,
        fallback_reason: None,
    }
}
