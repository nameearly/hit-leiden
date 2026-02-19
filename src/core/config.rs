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

use crate::core::backend::{AccelerationTarget, GraphSource};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunMode {
    Deterministic,
    Throughput,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunConfig {
    pub mode: RunMode,
    pub graph_source: GraphSource,
    pub acceleration: AccelerationTarget,
    pub quality_tolerance: f64,
    pub max_iterations: usize,
    pub pinned_profile: Option<String>,
    /// Leiden resolution parameter (gamma). Controls community granularity.
    /// Default 1.0 matches the HIT-Leiden paper (standard modularity).
    /// Used for both movement and refinement quality functions.
    pub resolution: f64,
    /// Refinement connectivity criterion gamma. Controls which nodes participate
    /// in refinement merging (must satisfy cut_size >= gamma * v_total * (S - v_total)).
    /// Default 0.05. NOT used for quality function.
    pub refinement_gamma: f64,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            mode: RunMode::Deterministic,
            graph_source: GraphSource::File,
            acceleration: AccelerationTarget::PureRust,
            quality_tolerance: 0.001,
            max_iterations: 10,
            pinned_profile: None,
            resolution: 1.0,
            refinement_gamma: 0.05,
        }
    }
}

impl RunConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_iterations == 0 {
            return Err("max_iterations must be > 0".to_string());
        }
        if self.quality_tolerance < 0.0 {
            return Err("quality_tolerance must be >= 0".to_string());
        }
        Ok(())
    }
}
