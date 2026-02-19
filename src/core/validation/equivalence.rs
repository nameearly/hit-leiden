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

use crate::core::config::RunMode;
use crate::core::report::ValidationOutcome;
use crate::core::types::RunOutcome;

pub fn validate(
    reference: &RunOutcome,
    candidate: &RunOutcome,
    mode: RunMode,
) -> ValidationOutcome {
    let ref_part = reference.partition.as_ref().unwrap();
    let cand_part = candidate.partition.as_ref().unwrap();
    let same_partition = ref_part.node_to_community == cand_part.node_to_community;
    let quality_delta = (ref_part.quality_score - cand_part.quality_score).abs();
    match mode {
        RunMode::Deterministic => ValidationOutcome {
            hard_invariants_passed: true,
            deterministic_identity_passed: Some(same_partition),
            quality_delta_vs_reference: Some(quality_delta),
            equivalence_passed: same_partition,
        },
        RunMode::Throughput => ValidationOutcome {
            hard_invariants_passed: true,
            deterministic_identity_passed: None,
            quality_delta_vs_reference: Some(quality_delta),
            equivalence_passed: quality_delta <= 0.001,
        },
    }
}
