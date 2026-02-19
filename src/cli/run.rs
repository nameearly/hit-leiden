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

use crate::cli::options::{CliMode, CliOptions};
use crate::core::backend::{AccelerationTarget, GraphSource};
use crate::core::config::{RunConfig, RunMode};
use crate::core::types::GraphInput;

pub fn run_from_cli(
    options: &CliOptions,
    graph: &GraphInput,
) -> Result<crate::core::types::RunOutcome, crate::core::error::HitLeidenError> {
    let mode = match options.mode {
        CliMode::Deterministic => RunMode::Deterministic,
        CliMode::Throughput => RunMode::Throughput,
    };

    let config = RunConfig {
        mode,
        graph_source: GraphSource::File, // Assuming file for now
        acceleration: AccelerationTarget::PureRust,
        quality_tolerance: 0.001,
        max_iterations: 10,
        pinned_profile: None,
        resolution: 1.0,
        refinement_gamma: 0.05,
    };

    crate::run(graph, &config)
}

pub fn run_default(
    graph: &GraphInput,
) -> Result<crate::core::types::RunOutcome, crate::core::error::HitLeidenError> {
    crate::run(graph, &RunConfig::default())
}
