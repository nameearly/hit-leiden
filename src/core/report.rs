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

use crate::core::backend::ResolutionMetadata;

#[derive(Clone, Debug, PartialEq)]
pub struct RunOutcome {
    pub run_id: String,
    pub partition: Vec<usize>,
    pub quality_score: f64,
    pub resolution: ResolutionMetadata,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ValidationOutcome {
    pub hard_invariants_passed: bool,
    pub deterministic_identity_passed: Option<bool>,
    pub quality_delta_vs_reference: Option<f64>,
    pub equivalence_passed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BenchmarkOutcome {
    pub baseline_commit: String,
    pub candidate_commit: String,
    pub benchmark_suite: String,
    pub median_throughput_gain: f64,
    pub reproducible: bool,
    pub release_gate_eligible: bool,
    pub release_gate_reason: Option<String>,
}

pub mod writer;
