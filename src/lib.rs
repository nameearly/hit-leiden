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

pub mod benchmark;
pub mod cli;
pub mod core;

pub use core::backend::{AccelerationTarget, GraphSource};
pub use core::config::{RunConfig, RunMode};
pub use core::error::HitLeidenError;
pub use core::report::{BenchmarkOutcome, ValidationOutcome};
pub use core::types::{GraphInput, RunOutcome};

pub fn run(graph: &GraphInput, config: &RunConfig) -> Result<RunOutcome, HitLeidenError> {
    core::algorithm::hit_leiden::run(graph, config)
}

pub fn project_from_neo4j(
    source_config: &core::graph::neo4j_snapshot::Neo4jSourceConfig,
    projection_config: &core::graph::neo4j_mapping::ProjectionConfig,
) -> Result<GraphInput, HitLeidenError> {
    core::graph::neo4j_snapshot::project_from_neo4j(source_config, projection_config)
}

pub fn validate(
    reference: &RunOutcome,
    candidate: &RunOutcome,
    mode: core::config::RunMode,
) -> ValidationOutcome {
    core::validation::equivalence::validate(reference, candidate, mode)
}

pub fn compare_baseline(
    baseline_commit: &str,
    candidate_commit: &str,
    benchmark_suite: &str,
    profile: &benchmark::hardware_profile::HardwareProfile,
) -> BenchmarkOutcome {
    benchmark::compare::compare_baseline(
        baseline_commit,
        candidate_commit,
        benchmark_suite,
        profile,
    )
}
